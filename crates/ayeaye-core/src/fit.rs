//! Auto-fit as a lease, not a latch.
//!
//! tmux keeps one grid per pane, so the only way to make a pane readable on a
//! phone is to resize the window — which reflows it for the desktop too. The
//! client asks for the size it wants and the pane poll renews the lease as a
//! side effect of watching; when the polls stop — view switched, tab closed,
//! phone locked, browser killed — the lease expires and the window goes back.
//! **So the browser dying abruptly can never leave the desktop stuck at phone
//! width**, which is the property the whole shape exists for.
//!
//! Nothing here runs tmux or reads a clock. Every entry point takes `now` as an
//! argument — the core may not reach `std::time::Instant`, and the same rule is
//! what lets expiry be tested without sleeping through it — and the commands
//! that restore a window come back as argv for the shell above to run.

/// How long a lease lives without being renewed, in milliseconds.
///
/// Twelve seconds, as `bin/ayeaye`'s `FIT_TTL`: long enough that a poll every
/// two seconds keeps it comfortably, short enough that a phone going into a
/// pocket gives the desktop its window back while somebody is still looking at
/// it.
pub const DEFAULT_TTL_MS: u64 = 12_000;

/// The narrowest and widest a client may ask a window to be.
///
/// `bin/ayeaye`'s clamp. A phone asks for something like 40 columns; the bounds
/// are there so a client that asks for one column, or for a million, cannot
/// make a window nobody can use out of a number nobody typed.
pub const MIN_COLS: u16 = 20;
/// The widest a window may be asked to be.
pub const MAX_COLS: u16 = 400;
/// The shortest a window may be asked to be.
pub const MIN_ROWS: u16 = 5;
/// The tallest a window may be asked to be.
pub const MAX_ROWS: u16 = 200;

/// How a window was sized before a lease took it.
///
/// Captured once per lease, so a window that was already sized by hand goes back
/// to *that* size and anything else goes back to tmux computing it. The two are
/// different tmux commands, and restoring with the wrong one leaves the desktop
/// either stuck at a size nobody chose or recomputed out of a size somebody did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Prev {
    /// tmux was computing the size from the attached clients.
    Auto,
    /// Somebody had already sized the window by hand.
    Manual {
        /// The width it was held at.
        cols: u16,
        /// The height it was held at.
        rows: u16,
    },
}

impl Prev {
    /// The arguments — after `tmux` itself — that put this window back.
    pub fn restore(&self, pane: &str) -> Vec<String> {
        match self {
            // Unset rather than set to something: tmux recomputes from the
            // attached clients the moment the option goes away, which is what
            // "automatic" means and what the desktop had before the phone
            // arrived.
            Prev::Auto => words(&["set-option", "-w", "-t", pane, "-u", "window-size"]),
            Prev::Manual { cols, rows } => resize(pane, *cols, *rows),
        }
    }

    /// How this is written to the state file: one field, or three.
    fn saved(&self) -> String {
        match self {
            Prev::Auto => "auto".to_string(),
            Prev::Manual { cols, rows } => format!("manual\t{cols}\t{rows}"),
        }
    }

    fn read(fields: &[&str]) -> Option<Prev> {
        match fields {
            ["auto"] => Some(Prev::Auto),
            ["manual", cols, rows] => Some(Prev::Manual {
                cols: cols.parse().ok()?,
                rows: rows.parse().ok()?,
            }),
            _ => None,
        }
    }
}

/// The arguments that resize a window to a size a client asked for.
pub fn resize(pane: &str, cols: u16, rows: u16) -> Vec<String> {
    words(&[
        "resize-window",
        "-t",
        pane,
        "-x",
        &cols.to_string(),
        "-y",
        &rows.to_string(),
    ])
}

fn words(args: &[&str]) -> Vec<String> {
    args.iter().map(|word| word.to_string()).collect()
}

/// Whether `show-options -w window-size` says the window is sized by hand.
///
/// tmux answers `window-size manual`, and older ones answer with the value
/// alone. The last word is the value either way, which is what `bin/ayeaye`
/// reads.
pub fn manual(shown: &str) -> bool {
    shown.split_whitespace().next_back() == Some("manual")
}

/// A `#{window_width}\t#{window_height}` answer, or `None` if it is not one.
///
/// Not a number is not a size, and a lease that recorded a size it invented
/// would restore the window to it later. `Prev::Auto` is the honest answer when
/// tmux would not say.
pub fn size(shown: &str) -> Option<(u16, u16)> {
    let (cols, rows) = shown.trim().split_once('\t')?;
    Some((cols.trim().parse().ok()?, rows.trim().parse().ok()?))
}

/// A size a client asked for, brought inside the bounds.
///
/// Clamped rather than refused: a phone in an odd rotation asking for eighteen
/// columns wants the narrowest window it can have, not an error.
pub fn clamp(cols: i64, rows: i64) -> (u16, u16) {
    (
        cols.clamp(MIN_COLS as i64, MAX_COLS as i64) as u16,
        rows.clamp(MIN_ROWS as i64, MAX_ROWS as i64) as u16,
    )
}

/// The windows currently held at somebody's phone's size.
///
/// Held as a list because a deployment has a handful of these at most — one per
/// pane somebody is looking at — and every operation is one scan of it.
#[derive(Debug, Clone)]
pub struct Leases {
    held: Vec<Lease>,
    ttl_ms: u64,
}

#[derive(Debug, Clone)]
struct Lease {
    pane: String,
    expires: u64,
    prev: Prev,
}

impl Default for Leases {
    fn default() -> Leases {
        Leases::new(DEFAULT_TTL_MS)
    }
}

impl Leases {
    /// Leases that expire `ttl_ms` after they were last renewed.
    pub fn new(ttl_ms: u64) -> Leases {
        Leases {
            held: Vec::new(),
            ttl_ms,
        }
    }

    /// How long an unrenewed lease lives.
    pub fn ttl_ms(&self) -> u64 {
        self.ttl_ms
    }

    /// Whether this pane's window is held.
    pub fn holds(&self, pane: &str) -> bool {
        self.held.iter().any(|lease| lease.pane == pane)
    }

    /// How many windows are held.
    pub fn len(&self) -> usize {
        self.held.len()
    }

    /// Whether none are.
    pub fn is_empty(&self) -> bool {
        self.held.is_empty()
    }

    /// Renew a lease, and say whether there was one.
    ///
    /// `false` is the caller's cue to go and read the pre-fit state, which is a
    /// tmux call worth not making on every poll of every watched pane.
    pub fn touch(&mut self, pane: &str, now: u64) -> bool {
        match self.held.iter_mut().find(|lease| lease.pane == pane) {
            Some(lease) => {
                lease.expires = now + self.ttl_ms;
                true
            }
            None => false,
        }
    }

    /// Record the pre-fit state, once, and start the lease.
    ///
    /// A pane already held is **renewed, not re-recorded**. That is the whole
    /// point of "once per lease": the second acquire happens after the window is
    /// already at phone width, so overwriting would record phone width as the
    /// state to restore, and the desktop would never get its window back.
    pub fn acquire(&mut self, pane: &str, prev: Prev, now: u64) -> bool {
        if self.touch(pane, now) {
            return false;
        }
        self.held.push(Lease {
            pane: pane.to_string(),
            expires: now + self.ttl_ms,
            prev,
        });
        true
    }

    /// End a lease, giving back the state to restore the window to.
    pub fn release(&mut self, pane: &str) -> Option<Prev> {
        let at = self.held.iter().position(|lease| lease.pane == pane)?;
        Some(self.held.remove(at).prev)
    }

    /// Drop a lease **without** restoring anything.
    ///
    /// The manual escape hatch: somebody has asked for sizing to go back to tmux
    /// outright, whatever the window was before, so the recorded state is no
    /// longer what they want and holding the lease would only undo their choice
    /// twelve seconds later.
    pub fn forget(&mut self, pane: &str) -> bool {
        let before = self.held.len();
        self.held.retain(|lease| lease.pane != pane);
        before != self.held.len()
    }

    /// Take every lease nobody has renewed, for the sweeper to restore.
    ///
    /// They are removed here rather than after the restore, so a slow tmux
    /// cannot have the same window swept twice by two passes.
    pub fn due(&mut self, now: u64) -> Vec<(String, Prev)> {
        let mut due = Vec::new();
        let mut kept = Vec::with_capacity(self.held.len());
        for lease in self.held.drain(..) {
            if lease.expires <= now {
                due.push((lease.pane, lease.prev));
            } else {
                kept.push(lease);
            }
        }
        self.held = kept;
        due
    }

    /// The leases as the state file holds them: one per line, `pane` then the
    /// state to restore it to.
    ///
    /// Expiry is not written. A file is read once, at startup, by a process that
    /// has just proved the previous one is gone — every lease in it is over
    /// however long it had left.
    ///
    /// Tab-separated rather than JSON because the core has no JSON reader and
    /// should not grow one for a file only this daemon writes. A pane id cannot
    /// carry a tab: `peer::PaneId` refuses control characters, so nothing here
    /// needs escaping.
    pub fn saved(&self) -> String {
        let mut out = String::new();
        for lease in &self.held {
            out.push_str(&lease.pane);
            out.push('\t');
            out.push_str(&lease.prev.saved());
            out.push('\n');
        }
        out
    }

    /// Read a state file back.
    ///
    /// A line that is not a lease is skipped rather than guessed at, and takes
    /// only itself with it — one truncated line at the end of a file a process
    /// died while writing must not strand every other window it lists.
    pub fn read(text: &str) -> Vec<(String, Prev)> {
        let mut leases = Vec::new();
        for line in text.lines() {
            let fields: Vec<&str> = line.split('\t').collect();
            let Some((pane, rest)) = fields.split_first() else {
                continue;
            };
            if pane.is_empty() {
                continue;
            }
            if let Some(prev) = Prev::read(rest) {
                leases.push((pane.to_string(), prev));
            }
        }
        leases
    }
}

#[cfg(test)]
mod tests {
    use super::{DEFAULT_TTL_MS, Leases, Prev, clamp, manual, resize, size};

    // AYEAYE-47 — the two restores are different tmux commands, and which one
    // runs is the difference between a desktop getting its window back and a
    // desktop stuck at a size nobody chose. Automatic is *unset*, not set to
    // anything: tmux recomputes from the attached clients when the option goes.
    #[test]
    fn each_pre_fit_state_restores_with_its_own_command() {
        assert_eq!(
            Prev::Auto.restore("%3"),
            ["set-option", "-w", "-t", "%3", "-u", "window-size"]
        );
        assert_eq!(
            Prev::Manual {
                cols: 100,
                rows: 40
            }
            .restore("%3"),
            ["resize-window", "-t", "%3", "-x", "100", "-y", "40"]
        );
        assert_eq!(
            resize("%3", 40, 24),
            ["resize-window", "-t", "%3", "-x", "40", "-y", "24"]
        );
    }

    // AYEAYE-47 — what tmux says about a window's sizing. The last word is the
    // value, whichever way the option is printed; anything that is not a pair of
    // numbers is not a size, and a lease that recorded an invented one would
    // restore the window to it later.
    #[test]
    fn the_pre_fit_state_is_read_out_of_what_tmux_says() {
        assert!(manual("window-size manual"));
        assert!(manual("manual\n"));
        assert!(!manual("window-size latest"));
        assert!(!manual("window-size largest"));
        assert!(!manual(""));

        assert_eq!(size("100\t40"), Some((100, 40)));
        assert_eq!(size(" 100\t40 \n"), Some((100, 40)));
        assert_eq!(size("100 40"), None);
        assert_eq!(size(""), None);
        assert_eq!(size("wide\ttall"), None);
        assert_eq!(size("-1\t40"), None);
    }

    // AYEAYE-47 — a size is clamped rather than refused: a phone in an odd
    // rotation asking for eighteen columns wants the narrowest window it can
    // have. The bounds are what stop a number nobody typed making a window
    // nobody can use.
    #[test]
    fn a_size_is_brought_inside_the_bounds_rather_than_refused() {
        assert_eq!(clamp(40, 24), (40, 24));
        assert_eq!(clamp(1, 1), (20, 5));
        assert_eq!(clamp(100_000, 100_000), (400, 200));
        assert_eq!(clamp(-5, -5), (20, 5));
        // Bigger than a u16, which is the one that would wrap rather than clamp
        // if the arithmetic happened in the narrow type.
        assert_eq!(clamp(70_000, 70_000), (400, 200));
    }

    // AYEAYE-47 — the pre-fit state is recorded **once**. A second acquire
    // happens when the window is already at phone width, so re-recording would
    // save phone width as the state to restore and the desktop would never get
    // its window back. It renews instead, which is what a client re-asserting a
    // fit after a drift check is doing.
    #[test]
    fn acquiring_a_held_lease_renews_it_rather_than_re_recording_it() {
        let mut leases = Leases::new(1_000);
        assert!(leases.acquire("desktop/%1", Prev::Manual { cols: 100, rows: 40 }, 0));
        assert!(leases.holds("desktop/%1"));

        // The window is at phone width now, so this is what a second acquire
        // would be handed.
        assert!(!leases.acquire("desktop/%1", Prev::Manual { cols: 40, rows: 24 }, 500));
        assert_eq!(leases.len(), 1);
        // Renewed: it would have expired at 1000 and does not.
        assert!(leases.due(1_200).is_empty());
        assert_eq!(
            leases.release("desktop/%1"),
            Some(Prev::Manual { cols: 100, rows: 40 }),
            "the state recorded first is the state restored"
        );
        assert!(!leases.holds("desktop/%1"));
        assert_eq!(leases.release("desktop/%1"), None);
    }

    // AYEAYE-47 — watching the pane is what holds the fit, so a poll renews.
    // Only a lease that exists, though: a touch that created one would record no
    // pre-fit state and the sweeper would later restore the window to nothing.
    #[test]
    fn a_poll_renews_a_lease_and_never_creates_one() {
        let mut leases = Leases::new(1_000);
        assert!(!leases.touch("desktop/%1", 0));
        assert!(leases.is_empty());

        leases.acquire("desktop/%1", Prev::Auto, 0);
        assert!(leases.touch("desktop/%1", 900));
        assert!(leases.due(1_500).is_empty(), "renewed at 900, lives to 1900");
        assert_eq!(leases.due(1_900).len(), 1, "and no further");
    }

    // AYEAYE-47 — "a crashed client's window is restored without intervention".
    // The sweeper takes exactly the leases nobody renewed, and takes them out of
    // the list as it does, so a slow tmux cannot have one window swept twice by
    // two passes.
    #[test]
    fn the_sweeper_takes_exactly_the_leases_nobody_renewed() {
        let mut leases = Leases::new(1_000);
        leases.acquire("desktop/%1", Prev::Auto, 0);
        leases.acquire(
            "desktop/%2",
            Prev::Manual {
                cols: 100,
                rows: 40,
            },
            0,
        );
        leases.touch("desktop/%2", 800);

        let due = leases.due(1_100);
        assert_eq!(due, vec![("desktop/%1".to_string(), Prev::Auto)]);
        assert!(leases.holds("desktop/%2"), "the renewed one is untouched");
        assert!(
            leases.due(1_100).is_empty(),
            "a second pass finds nothing to restore twice"
        );
        assert_eq!(leases.due(1_800).len(), 1);
        assert!(leases.is_empty());
    }

    // AYEAYE-47 — the manual escape hatch drops a lease without restoring
    // anything: somebody has asked for sizing to go back to tmux outright, and a
    // lease still holding the old state would undo their choice twelve seconds
    // later.
    #[test]
    fn forgetting_a_lease_leaves_nothing_to_restore() {
        let mut leases = Leases::new(1_000);
        leases.acquire("desktop/%1", Prev::Manual { cols: 100, rows: 40 }, 0);
        assert!(leases.forget("desktop/%1"));
        assert!(!leases.forget("desktop/%1"));
        assert!(leases.due(9_999).is_empty());
    }

    // AYEAYE-47 — "leases survive a daemon restart, recovering from persisted
    // state". A round trip has to give back both shapes exactly, or the window a
    // crash stranded is restored to the wrong size — which is the failure the
    // file exists to prevent, arriving by a different route.
    #[test]
    fn the_persisted_form_round_trips_both_states() {
        let mut leases = Leases::default();
        assert_eq!(leases.ttl_ms(), DEFAULT_TTL_MS);
        leases.acquire("desktop/%1", Prev::Auto, 0);
        leases.acquire(
            "Alex's Mac/%2",
            Prev::Manual {
                cols: 100,
                rows: 40,
            },
            0,
        );

        let saved = leases.saved();
        assert_eq!(saved, "desktop/%1\tauto\nAlex's Mac/%2\tmanual\t100\t40\n");
        assert_eq!(
            Leases::read(&saved),
            vec![
                ("desktop/%1".to_string(), Prev::Auto),
                (
                    "Alex's Mac/%2".to_string(),
                    Prev::Manual {
                        cols: 100,
                        rows: 40
                    }
                ),
            ]
        );
        assert!(Leases::read("").is_empty());
        assert!(Leases::default().saved().is_empty());
    }

    // AYEAYE-47 — a file a process died halfway through writing must strand only
    // the line it was writing. Every other window it lists still goes back.
    #[test]
    fn an_unreadable_line_costs_only_its_own_window() {
        let leases = Leases::read(
            "desktop/%1\tauto\n\
             desktop/%2\tmanual\n\
             desktop/%3\tmanual\tten\t40\n\
             \tauto\n\
             nonsense\n\
             \n\
             desktop/%4\tmanual\t80\t25\n\
             desktop/%5\tma",
        );
        assert_eq!(
            leases,
            vec![
                ("desktop/%1".to_string(), Prev::Auto),
                (
                    "desktop/%4".to_string(),
                    Prev::Manual { cols: 80, rows: 25 }
                ),
            ]
        );
    }
}
