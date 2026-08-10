//! A pane's capture, read as the two things it actually is.
//!
//! Nothing here runs `capture-pane`. The shell above captures the text and the
//! grid size and hands them over; this decides what they mean, which is what
//! lets the split and the diff be held to output really captured from a tmux
//! server rather than to output somebody imagined.
//!
//! The capture is two things with very different lives glued together: the
//! **scrollback** above the screen, which tmux never rewrites and only appends
//! to, and the **screen**, which a TUI repaints constantly but which is only
//! ever `rows` lines. Splitting there is what makes most polls cost almost
//! nothing — see [`crate::pane::Cache::diff`].

use crate::json;
use crate::sha1;

/// How much of the digest names a window or a screen.
///
/// Twelve hex characters, as `bin/ayeaye`'s `_pane_hash` takes. The token is
/// opaque to the client and lands in a query string, so its only jobs are to be
/// short and to change when the text does.
const TOKEN_LEN: usize = 12;

/// How much scrollback the terminal view carries above the screen.
///
/// Five hundred lines, as `bin/ayeaye`'s `PANE_LINES`. It is about five
/// milliseconds of `capture-pane`, and the diff protocol resends only what
/// changed since the client last asked — so depth costs the server memory, not
/// the phone's radio.
///
/// Fixed, where `bin/ayeaye:163` reads `_env("LINES", "500")`. The default is the
/// same, so only a deployment that overrode it diverges — and the option is not
/// carried over because a scrollback depth is a decision about the protocol
/// rather than about the machine, and nothing in the milestone needs it to move.
pub const LINES: usize = 500;

/// The token naming this text.
pub fn token(text: &str) -> String {
    let mut digest = sha1::hex(text.as_bytes());
    digest.truncate(TOKEN_LEN);
    digest
}

/// The lines of a capture, as they are held and sent.
///
/// A line carrying escapes keeps its trailing spaces — they may be painted
/// background, and trimming them would strip the colour off the right-hand end
/// of a TUI's panel. A line with no escapes in it is right-trimmed, because the
/// spaces tmux pads it to the grid width with are nothing but bytes on the
/// radio.
pub fn lines(raw: &str) -> Vec<String> {
    raw.lines()
        .map(|line| {
            if line.contains(ESC) {
                line.to_string()
            } else {
                line.trim_end().to_string()
            }
        })
        .collect()
}

/// The escape byte `capture-pane -e` writes colour with.
const ESC: char = '\u{1b}';

/// Whether a line would look empty on screen.
///
/// `capture-pane -e` emits SGR sequences and nothing cursor-y, so a line that is
/// blank apart from its colouring is blank. Written as a scan rather than a
/// pattern because there is no regex crate below the shell and there must not
/// be one: the core declares no dependencies at all.
pub fn blank(line: &str) -> bool {
    let mut rest = line;
    while let Some(start) = rest.find(ESC) {
        if !rest[..start].trim().is_empty() {
            return false;
        }
        // `\x1b[` then the parameter bytes then `m`. Anything else is not an SGR
        // sequence, and is left alone rather than guessed at — a line that is
        // not blank must never be mistaken for one.
        let Some(after) = rest[start..].strip_prefix("\u{1b}[") else {
            return false;
        };
        let end = after.find(|c: char| !matches!(c, '0'..='9' | ';' | ':'));
        match end {
            Some(index) if after[index..].starts_with('m') => rest = &after[index + 1..],
            _ => return false,
        }
    }
    rest.trim().is_empty()
}

/// A capture, split where its two halves live.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct View {
    /// The scrollback above the screen. Append-only, as far as tmux is
    /// concerned, which is the property the patch below relies on.
    pub hist: Vec<String>,
    /// The visible screen, with its trailing blank lines dropped.
    pub screen: Vec<String>,
}

impl View {
    /// Split a capture at the bottom `rows` lines.
    ///
    /// `rows` of zero means the grid size could not be read, and then the whole
    /// capture is screen: pretending the top of it is settled history would let
    /// a repaint be sent as an append, and the client would grow a transcript of
    /// every frame a TUI ever drew.
    pub fn split(mut lines: Vec<String>, rows: u16) -> View {
        let rows = rows as usize;
        let hist = if rows > 0 && lines.len() > rows {
            lines.drain(..lines.len() - rows).collect()
        } else {
            Vec::new()
        };
        let mut screen = lines;
        // Only the screen is trimmed. tmux pads the grid to `rows` whatever the
        // program drew, so the blank lines at the bottom are the terminal's, not
        // the program's — while a blank line in the scrollback is one something
        // really printed.
        while screen.last().is_some_and(|line| blank(line)) {
            screen.pop();
        }
        View { hist, screen }
    }

    /// The screen as one block of text, which is what a token names and what
    /// goes over the wire.
    pub fn screen_text(&self) -> String {
        self.screen.join("\n")
    }

    /// The history window as one block, which is what its token names.
    pub fn hist_text(&self) -> String {
        self.hist.join("\n")
    }
}

/// What a diff answer says beyond the grid and the two tokens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    /// Both tokens matched. The client renders nothing and the poll cost a
    /// header.
    Same,
    /// Shed `drop` lines from the front of the history window the client holds,
    /// append `add`, and replace the screen.
    Patch {
        /// How many lines have scrolled off the front of the window.
        drop: usize,
        /// The lines that have arrived at the back of it.
        add: Vec<String>,
        /// The screen, whole. It is only ever `rows` lines and a TUI rewrites
        /// all of it, so there is nothing to diff.
        screen: String,
    },
    /// Everything, because the client's token named a window this server no
    /// longer remembers.
    Full {
        /// The history window entire.
        hist: Vec<String>,
        /// The screen, whole.
        screen: String,
    },
}

/// One answer to `/api/pane?df=1`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diff {
    /// The grid the capture was taken at, so the client renders at the width
    /// tmux actually wrapped for.
    pub cols: u16,
    /// The grid's height.
    pub rows: u16,
    /// The token naming the history window this answer leaves the client
    /// holding.
    pub hh: String,
    /// The token naming the screen it leaves the client holding.
    pub sh: String,
    /// What changed.
    pub answer: Answer,
}

impl Diff {
    /// The body, in `bin/ayeaye`'s own field order.
    ///
    /// The order is not decoration: it is what lets one daemon's answer be
    /// compared to the other's byte for byte while both exist.
    pub fn body(&self) -> String {
        let mut body = format!(
            "{{\"cols\":{},\"rows\":{},\"hh\":{},\"sh\":{}",
            self.cols,
            self.rows,
            json::string(&self.hh),
            json::string(&self.sh)
        );
        match &self.answer {
            // `1` rather than `true`, because that is the value the daemon
            // sends and `share/app.html` tests it for truth either way.
            Answer::Same => body.push_str(",\"same\":1"),
            Answer::Patch { drop, add, screen } => {
                body.push_str(&format!(",\"drop\":{drop},\"add\":"));
                push_lines(&mut body, add);
                body.push_str(&format!(",\"screen\":{}", json::string(screen)));
            }
            Answer::Full { hist, screen } => {
                body.push_str(",\"hist\":");
                push_lines(&mut body, hist);
                body.push_str(&format!(",\"screen\":{}", json::string(screen)));
            }
        }
        body.push('}');
        body
    }
}

fn push_lines(body: &mut String, lines: &[String]) {
    body.push('[');
    for (index, line) in lines.iter().enumerate() {
        if index > 0 {
            body.push(',');
        }
        body.push_str(&json::string(line));
    }
    body.push(']');
}

/// The history windows recently served, so the next patch can be computed
/// against the exact lines a client says it holds.
///
/// **Keyed on the pane and the token the client supplies**, never on who asked.
/// That is what makes the endpoint free to proxy in the multi-host milestone:
/// there is no per-client state to move, a miss costs one full send, and a
/// restart costs the same. It is deliberately small for the same reason.
#[derive(Debug, Clone)]
pub struct Cache {
    /// Oldest first, so eviction is from the front. Thirty-two entries is a
    /// list, not a map: every operation here is one linear scan of it, and a
    /// hash map with an insertion order beside it would be more machinery than
    /// the thing it holds.
    held: Vec<((String, String), Vec<String>)>,
    max: usize,
}

impl Default for Cache {
    fn default() -> Cache {
        Cache::new(Cache::DEFAULT_MAX)
    }
}

impl Cache {
    /// How many windows are remembered at once, as `bin/ayeaye` remembers them.
    pub const DEFAULT_MAX: usize = 32;

    /// One that holds `max` windows.
    pub fn new(max: usize) -> Cache {
        Cache {
            held: Vec::new(),
            max,
        }
    }

    /// How many windows are held. For the suite, and for anyone wondering
    /// whether the bound is doing anything.
    pub fn len(&self) -> usize {
        self.held.len()
    }

    /// Whether nothing is held.
    pub fn is_empty(&self) -> bool {
        self.held.is_empty()
    }

    /// What changed since the capture the client says it holds.
    ///
    /// The patch is found by sliding. The old window and the new one are both
    /// windows onto the same append-only scrollback, so the new one is the old
    /// one shifted by `k` with fresh lines behind it. Any `k` with
    /// `old[k..] == new[..old.len() - k]` reconstructs to exactly `new` when
    /// applied — **the equation is the guarantee, not the intent behind the
    /// match** — so a coincidental overlap is harmless and the smallest `k` is
    /// merely the smallest payload. `k = old.len()` always satisfies it, which
    /// is why this cannot fail: a resize that rewrapped every line degrades to a
    /// full send in patch clothing rather than to an error.
    pub fn diff(
        &mut self,
        pane: &str,
        view: &View,
        cols: u16,
        rows: u16,
        had_hist: &str,
        had_screen: &str,
    ) -> Diff {
        let screen = view.screen_text();
        let hh = token(&view.hist_text());
        let sh = token(&screen);

        // Looked up before the insert, because a client that is up to date names
        // the window this call is about to store and the two would be the same
        // entry. A client that names nothing gets no lookup at all.
        let prev = if had_hist.is_empty() {
            None
        } else {
            self.recall(pane, had_hist)
        };
        self.remember(pane, &hh, &view.hist);

        let answer = if had_hist == hh && had_screen == sh {
            Answer::Same
        } else if let Some(prev) = prev {
            let (drop, add) = patch(&prev, &view.hist);
            Answer::Patch { drop, add, screen }
        } else {
            Answer::Full {
                hist: view.hist.clone(),
                screen,
            }
        };
        Diff {
            cols,
            rows,
            hh,
            sh,
            answer,
        }
    }

    fn recall(&self, pane: &str, hh: &str) -> Option<Vec<String>> {
        self.held
            .iter()
            .find(|((held_pane, held_hh), _)| held_pane == pane && held_hh == hh)
            .map(|(_, lines)| lines.clone())
    }

    fn remember(&mut self, pane: &str, hh: &str, hist: &[String]) {
        self.held
            .retain(|((held_pane, held_hh), _)| held_pane != pane || held_hh != hh);
        self.held
            .push(((pane.to_string(), hh.to_string()), hist.to_vec()));
        while self.held.len() > self.max {
            self.held.remove(0);
        }
    }
}

/// How to get from the window the client holds to the one it should hold.
///
/// Returns how many lines to shed from the front and what to append. See
/// [`Cache::diff`] for why this cannot fail.
fn patch(prev: &[String], hist: &[String]) -> (usize, Vec<String>) {
    for shed in 0..=prev.len() {
        let kept = prev.len() - shed;
        if kept <= hist.len() && prev[shed..] == hist[..kept] {
            return (shed, hist[kept..].to_vec());
        }
    }
    (prev.len(), hist.to_vec())
}

/// The whole capture as one block, for a client that does not speak the diff
/// protocol.
///
/// The whole thing rather than the last `rows` of it: trimming decapitates a
/// full-screen TUI, whose top box is as important as its bottom.
pub fn whole(lines: &[String]) -> String {
    let mut kept = lines;
    while kept.last().is_some_and(|line| blank(line)) {
        kept = &kept[..kept.len() - 1];
    }
    kept.join("\n")
}

/// The body `/api/pane` answers with when the client did not ask for a diff.
///
/// The grid comes back with the text so the client can render at the width tmux
/// actually wrapped for.
pub fn whole_body(text: &str, cols: u16, rows: u16) -> String {
    format!(
        "{{\"text\":{},\"cols\":{cols},\"rows\":{rows}}}",
        json::string(text)
    )
}

#[cfg(test)]
mod tests {
    use super::{Answer, Cache, Diff, View, blank, lines, token, whole, whole_body};

    /// A real capture, from a private `tmux -L` server: `capture-pane -p -e` over
    /// a pane carrying colour, a blank line, a line with a quote and a backslash
    /// in it, and a shell prompt at the bottom.
    ///
    /// Held to output really captured rather than to output somebody imagined —
    /// which is the module's whole claim, and which a hand-written `["h1","h2"]`
    /// cannot make. Its shape is load-bearing below: line 7 is blank and line 9
    /// carries the quote, so a split at four rows ends the history on a blank
    /// line and a split at two rows puts the quote inside the history array.
    const CAPTURE: &str = include_str!("../../../tests/fixtures/tmux/capture-pane-coloured");

    fn owned(list: &[&str]) -> Vec<String> {
        list.iter().map(|line| line.to_string()).collect()
    }

    /// A view with a history window and a one-line screen.
    fn view(hist: &[&str], screen: &[&str]) -> View {
        View {
            hist: owned(hist),
            screen: owned(screen),
        }
    }

    /// What a client holding `hist` would send back after being given `diff`,
    /// and what it would then hold.
    ///
    /// This is `share/app.html`'s own rule — `outHist.slice(drop).concat(add)`
    /// — written once so every case below can be asserted by *applying* the
    /// patch rather than by reading `drop` and `add` back out of it. A test
    /// that only checked the numbers would agree with a patch that reconstructs
    /// the wrong window.
    fn applied(held: &[String], diff: &Diff) -> Vec<String> {
        match &diff.answer {
            Answer::Same => held.to_vec(),
            Answer::Full { hist, .. } => hist.clone(),
            Answer::Patch { drop, add, .. } => {
                let mut next = held[(*drop).min(held.len())..].to_vec();
                next.extend(add.iter().cloned());
                next
            }
        }
    }

    // AYEAYE-47 — a line that is blank apart from its colouring is blank. tmux
    // pads a repainted screen with them, and a screen whose trailing lines were
    // kept would change its token every time a TUI repainted the same frame.
    #[test]
    fn a_line_is_blank_when_nothing_but_colour_is_left_of_it() {
        assert!(blank(""));
        assert!(blank("   "));
        assert!(blank("\u{1b}[0m"));
        assert!(blank("\u{1b}[38;5;244m   \u{1b}[0m"));
        // The colon form is real: `38:2:255:0:0` is the direct-colour spelling.
        assert!(blank("\u{1b}[38:2:255:0:0m \u{1b}[m"));
        assert!(!blank("x"));
        assert!(!blank("\u{1b}[31mx\u{1b}[0m"));
        assert!(!blank("  text  "));
    }

    // AYEAYE-47 — only SGR. `capture-pane -e` emits colour and nothing
    // cursor-y, so anything else in a line is content: a sequence this does not
    // understand must leave the line looking non-blank rather than being
    // skipped over as decoration.
    #[test]
    fn an_escape_that_is_not_colour_is_not_treated_as_decoration() {
        assert!(!blank("\u{1b}[2J"));
        assert!(!blank("\u{1b}]0;title\u{7}"));
        assert!(!blank("\u{1b}"));
        assert!(!blank("\u{1b}[31"));
    }

    // AYEAYE-47 — a line with escapes keeps its trailing spaces, because they
    // may be painted background; a line without them is right-trimmed, because
    // tmux padded it to the grid and those bytes are nothing.
    #[test]
    fn a_coloured_line_keeps_its_padding_and_a_plain_one_does_not() {
        let raw = "plain   \n\u{1b}[44mpainted   \u{1b}[0m   \nlast";
        assert_eq!(
            lines(raw),
            owned(&["plain", "\u{1b}[44mpainted   \u{1b}[0m   ", "last"])
        );
        assert!(lines("").is_empty());
        // A trailing newline is a line ending, not an extra line — `str::lines`
        // and Python's `splitlines` agree, and the daemon relies on it.
        assert_eq!(lines("one\n"), owned(&["one"]));
    }

    // AYEAYE-47 — the split is the whole point: the bottom `rows` lines are the
    // screen a TUI repaints, everything above is scrollback tmux only appends
    // to. Getting it wrong turns every repaint into an append and grows the
    // client a transcript of every frame ever drawn.
    #[test]
    fn the_bottom_rows_are_the_screen_and_the_rest_is_scrollback() {
        let view = View::split(owned(&["h1", "h2", "s1", "s2", "s3"]), 3);
        assert_eq!(view.hist, owned(&["h1", "h2"]));
        assert_eq!(view.screen, owned(&["s1", "s2", "s3"]));
        assert_eq!(view.screen_text(), "s1\ns2\ns3");

        // A capture no longer than the grid is all screen: there is nothing
        // above it yet.
        let short = View::split(owned(&["s1", "s2"]), 3);
        assert!(short.hist.is_empty());
        assert_eq!(short.screen, owned(&["s1", "s2"]));
        assert_eq!(
            View::split(owned(&["a", "b", "c"]), 3).hist,
            Vec::<String>::new()
        );
    }

    // AYEAYE-47 — a grid size that could not be read means the whole capture is
    // screen. The alternative — guessing a split — would send a repaint as an
    // append, and the client trusts appends blindly.
    #[test]
    fn a_grid_of_no_rows_is_all_screen() {
        let view = View::split(owned(&["a", "b", "c"]), 0);
        assert!(view.hist.is_empty());
        assert_eq!(view.screen, owned(&["a", "b", "c"]));
    }

    // AYEAYE-47 — the screen's trailing blanks come off and the scrollback's do
    // not. A blank line in the scrollback is one something really printed; a
    // blank line at the bottom of the screen is tmux padding the grid.
    #[test]
    fn trailing_blanks_come_off_the_screen_only() {
        let view = View::split(owned(&["h1", "", "h2", "s1", "", "\u{1b}[0m  "]), 3);
        assert_eq!(view.hist, owned(&["h1", "", "h2"]));
        assert_eq!(view.screen, owned(&["s1"]));

        // A screen that is entirely blank empties, and the history stays where
        // it is: the daemon pops out of the screen and no further.
        let empty = View::split(owned(&["h1", "", ""]), 2);
        assert_eq!(empty.hist, owned(&["h1"]));
        assert!(empty.screen.is_empty());
        assert_eq!(empty.screen_text(), "");
    }

    // AYEAYE-47 — a real capture, and the half of the trim rule a handwritten
    // fixture kept leaving unproven: the scrollback's trailing blank line stays.
    // A blank line in the scrollback is one something really printed, and
    // trimming it would shift the split, change `hh`, and cost a full send every
    // time a program printed a blank line at just the wrong moment.
    #[test]
    fn the_scrollback_keeps_a_blank_line_the_screen_would_lose() {
        let captured = lines(CAPTURE);
        assert_eq!(captured.len(), 11, "the fixture has drifted: {captured:?}");
        assert!(captured[6].is_empty(), "line 7 is the blank one");

        // Four rows: the history ends on that blank line, and keeps it.
        let view = View::split(captured.clone(), 4);
        assert_eq!(view.hist.len(), 7);
        assert_eq!(view.hist.last().map(String::as_str), Some(""));
        assert_eq!(view.screen.len(), 4, "and the screen ends non-blank");

        // The same capture with the blank line at the bottom of the screen
        // instead: now it goes, because down there it is tmux padding the grid.
        let padded = View::split(captured[..7].to_vec(), 3);
        assert_eq!(padded.hist, captured[..4].to_vec());
        assert_eq!(padded.screen, captured[4..6].to_vec());
    }

    // AYEAYE-47 — the whole-text shape is the whole capture, not the last
    // screenful. Trimming to the screen decapitates a full-screen TUI, whose top
    // box is as important as its bottom.
    #[test]
    fn the_whole_shape_is_the_whole_capture_without_its_trailing_blanks() {
        assert_eq!(whole(&owned(&["a", "", "b", "", "  "])), "a\n\nb");
        assert_eq!(whole(&owned(&["", " "])), "");
        assert!(whole(&[]).is_empty());
    }

    // AYEAYE-47 — the body the panel reads when it did not ask for a diff. The
    // grid comes with it, or the page cannot render at the width tmux wrapped
    // for; and the text is a JSON string, so an escape byte in a coloured line
    // cannot end it.
    #[test]
    fn the_whole_body_carries_the_text_and_the_grid() {
        assert_eq!(
            whole_body("hi\nthere", 80, 24),
            r#"{"text":"hi\nthere","cols":80,"rows":24}"#
        );
        assert_eq!(
            whole_body("\u{1b}[31mred\"", 0, 0),
            r#"{"text":"\u001b[31mred\"","cols":0,"rows":0}"#
        );
    }

    // AYEAYE-47 — a client that is up to date is told so and nothing else. This
    // is the common case by a long way: a pane nobody is typing into is polled
    // every two seconds for as long as it is watched.
    #[test]
    fn matching_tokens_answer_same_and_carry_no_text() {
        let mut cache = Cache::default();
        let now = view(&["h1", "h2"], &["s1"]);
        let first = cache.diff("desktop/%1", &now, 80, 24, "", "");
        assert!(matches!(first.answer, Answer::Full { .. }));

        let again = cache.diff("desktop/%1", &now, 80, 24, &first.hh, &first.sh);
        assert_eq!(again.answer, Answer::Same);
        assert_eq!(again.hh, first.hh);
        assert_eq!(again.sh, first.sh);
        assert_eq!(
            again.body(),
            format!(
                r#"{{"cols":80,"rows":24,"hh":"{}","sh":"{}","same":1}}"#,
                first.hh, first.sh
            )
        );
    }

    // AYEAYE-47 — a TUI repainting itself changes the screen and nothing above
    // it, and that is the case the split exists for: the answer carries the
    // screen alone and sheds nothing.
    #[test]
    fn a_repaint_carries_the_screen_alone() {
        let mut cache = Cache::default();
        let before = view(&["h1", "h2"], &["frame one"]);
        let first = cache.diff("desktop/%1", &before, 80, 24, "", "");

        let after = view(&["h1", "h2"], &["frame two"]);
        let diff = cache.diff("desktop/%1", &after, 80, 24, &first.hh, &first.sh);
        assert_eq!(
            diff.answer,
            Answer::Patch {
                drop: 0,
                add: Vec::new(),
                screen: "frame two".to_string(),
            }
        );
        assert_eq!(applied(&before.hist, &diff), after.hist);
        assert_eq!(diff.hh, first.hh, "the history window did not move");
    }

    // AYEAYE-47 — a scroll carries only the lines that crossed into history.
    // The window the client holds is capped, so old lines leave the front of it
    // as new ones arrive at the back, and both ends have to be right or the
    // client's transcript silently loses or repeats a line.
    #[test]
    fn a_scroll_carries_only_the_lines_that_crossed_into_history() {
        let mut cache = Cache::default();
        let before = view(&["h1", "h2", "h3"], &["s"]);
        let first = cache.diff("desktop/%1", &before, 80, 24, "", "");

        // Two lines scrolled off the top and two arrived at the bottom.
        let after = view(&["h3", "h4", "h5"], &["s"]);
        let diff = cache.diff("desktop/%1", &after, 80, 24, &first.hh, &first.sh);
        assert_eq!(
            diff.answer,
            Answer::Patch {
                drop: 2,
                add: owned(&["h4", "h5"]),
                screen: "s".to_string(),
            }
        );
        assert_eq!(applied(&before.hist, &diff), after.hist);
    }

    // AYEAYE-47 — "a restart costs one full send". A client whose token names a
    // window this server never minted, or minted for a different pane, gets
    // everything rather than a patch computed against a guess.
    #[test]
    fn a_token_this_server_does_not_hold_costs_one_full_send() {
        let mut cache = Cache::default();
        let now = view(&["h1", "h2"], &["s"]);
        let stale = cache.diff("desktop/%1", &now, 80, 24, "deadbeefcafe", "0123456789ab");
        assert_eq!(
            stale.answer,
            Answer::Full {
                hist: owned(&["h1", "h2"]),
                screen: "s".to_string(),
            }
        );

        // A token minted for another pane is another pane's window: the key is
        // the pane *and* the token, or one pane's scrollback would be patched
        // into another's. Keyed on the token alone, the call below would find
        // %9's window and answer with a patch against it.
        let mut fresh = Cache::default();
        let theirs = fresh.diff("desktop/%9", &now, 80, 24, "", "");
        let grown = view(&["h1", "h2", "h3"], &["s"]);
        let mine = fresh.diff("desktop/%1", &grown, 80, 24, &theirs.hh, &theirs.sh);
        assert!(
            matches!(mine.answer, Answer::Full { .. }),
            "{:?} came from another pane's window",
            mine.answer
        );

        // And a restart costs *at most* one full send, then patches again: a
        // fresh cache is exactly what a restarted process has. It costs nothing
        // at all while the pane is unchanged, because both tokens are computed
        // from the capture rather than remembered — which is the same property
        // from the other side.
        assert_eq!(
            Cache::default()
                .diff("desktop/%1", &now, 80, 24, &stale.hh, &stale.sh)
                .answer,
            Answer::Same,
            "a restart with nothing changed costs nothing"
        );
        let restarted = &mut Cache::default();
        let later = view(&["h1", "h2", "h3"], &["s"]);
        let after_restart = restarted.diff("desktop/%1", &later, 80, 24, &stale.hh, &stale.sh);
        assert!(matches!(after_restart.answer, Answer::Full { .. }));
        let later_still = view(&["h1", "h2", "h3"], &["s2"]);
        let next = restarted.diff(
            "desktop/%1",
            &later_still,
            80,
            24,
            &after_restart.hh,
            &after_restart.sh,
        );
        assert!(matches!(next.answer, Answer::Patch { .. }), "{next:?}");
    }

    // AYEAYE-47 — the sliding match is an equation, not a guess: any shift that
    // reconstructs the new window is correct, and a window with no overlap at
    // all degrades to a full send in patch clothing rather than to an error.
    // A rewrap after a resize is exactly that case.
    #[test]
    fn a_window_with_nothing_in_common_still_reconstructs() {
        let mut cache = Cache::default();
        let before = view(&["a", "b", "c"], &["s"]);
        let first = cache.diff("desktop/%1", &before, 80, 24, "", "");

        let rewrapped = view(&["x", "y"], &["s"]);
        let diff = cache.diff("desktop/%1", &rewrapped, 40, 24, &first.hh, &first.sh);
        assert_eq!(applied(&before.hist, &diff), rewrapped.hist);
        assert_eq!(diff.cols, 40, "the new grid comes back with it");

        // Repeated lines are where a shorter shift than the true one exists.
        // It is still correct — applying it reconstructs the window — which is
        // the whole claim.
        let mut repeated = Cache::default();
        let held = view(&["x", "x", "x"], &["s"]);
        let one = repeated.diff("desktop/%1", &held, 80, 24, "", "");
        let grown = view(&["x", "x", "x", "x"], &["s"]);
        let two = repeated.diff("desktop/%1", &grown, 80, 24, &one.hh, &one.sh);
        assert_eq!(applied(&held.hist, &two), grown.hist);

        // An emptied window and an emptied client both reconstruct.
        let mut edges = Cache::default();
        let none = edges.diff("desktop/%1", &view(&[], &["s"]), 80, 24, "", "");
        let some = edges.diff(
            "desktop/%1",
            &view(&["a"], &["s"]),
            80,
            24,
            &none.hh,
            &none.sh,
        );
        assert_eq!(applied(&[], &some), owned(&["a"]));
        let gone = edges.diff("desktop/%1", &view(&[], &["s"]), 80, 24, &some.hh, &some.sh);
        assert_eq!(applied(&owned(&["a"]), &gone), Vec::<String>::new());
    }

    // AYEAYE-47 — the cache is bounded, and the bound is what keeps a server
    // watching many panes from remembering every window any of them ever had.
    // Oldest out first; a miss is one full send, which is the price of the
    // bound and the reason it can be this small.
    #[test]
    fn the_cache_is_bounded_and_evicts_the_oldest_first() {
        let mut cache = Cache::new(2);
        let first = cache.diff("desktop/%1", &view(&["a"], &["s"]), 80, 24, "", "");
        let second = cache.diff("desktop/%2", &view(&["b"], &["s"]), 80, 24, "", "");
        cache.diff("desktop/%3", &view(&["c"], &["s"]), 80, 24, "", "");
        assert_eq!(cache.len(), 2);

        // The second pane's window is still held; the first pane's is gone.
        // Asked in that order, because each of these calls stores a window of
        // its own and so evicts one more.
        let kept = cache.diff(
            "desktop/%2",
            &view(&["b", "e"], &["s"]),
            80,
            24,
            &second.hh,
            "",
        );
        assert!(matches!(kept.answer, Answer::Patch { .. }), "{kept:?}");
        let evicted = cache.diff(
            "desktop/%1",
            &view(&["a", "d"], &["s"]),
            80,
            24,
            &first.hh,
            "",
        );
        assert!(matches!(evicted.answer, Answer::Full { .. }), "{evicted:?}");
        assert_eq!(Cache::DEFAULT_MAX, 32);
        assert!(Cache::default().is_empty());
    }

    // AYEAYE-47 — the bodies, field for field, in the order `bin/ayeaye` writes
    // them and with every name `share/app.html` reads. A line is a JSON string,
    // so the escape bytes a coloured capture is full of cannot end one.
    #[test]
    fn the_diff_bodies_carry_the_names_the_panel_reads() {
        let mut cache = Cache::default();
        let before = view(&["h1"], &["\u{1b}[31ms\""]);
        let full = cache.diff("desktop/%1", &before, 80, 24, "", "");
        assert_eq!(
            full.body(),
            format!(
                concat!(
                    r#"{{"cols":80,"rows":24,"hh":"{}","sh":"{}","#,
                    r#""hist":["h1"],"screen":"\u001b[31ms\""}}"#
                ),
                full.hh, full.sh
            )
        );

        let after = view(&["h1", "h2"], &["s"]);
        let patch = cache.diff("desktop/%1", &after, 80, 24, &full.hh, &full.sh);
        assert_eq!(
            patch.body(),
            format!(
                concat!(
                    r#"{{"cols":80,"rows":24,"hh":"{}","sh":"{}","#,
                    r#""drop":0,"add":["h2"],"screen":"s"}}"#
                ),
                patch.hh, patch.sh
            )
        );
    }

    // AYEAYE-47 — and the *lines* are JSON strings too, not only the screen.
    // Scrollback is exactly where a quote or a backslash turns up — somebody's
    // shell history — and a `hist` array that emitted one raw would be invalid
    // JSON. `share/app.html:1536` catches the parse failure and does nothing
    // with it, so the terminal would simply stay blank with no error anywhere.
    #[test]
    fn a_quote_in_the_scrollback_cannot_break_the_arrays() {
        let captured = lines(CAPTURE);
        // Two rows, so the line carrying `"` and `\` lands in the history.
        let held = View::split(captured.clone(), 2);
        assert!(
            held.hist.iter().any(|line| line.contains('"')),
            "the fixture no longer puts a quote in the scrollback: {:?}",
            held.hist
        );

        let mut cache = Cache::default();
        let full = cache.diff("desktop/%1", &held, 40, 2, "", "");
        let Answer::Full { .. } = full.answer else {
            panic!("the first answer is a full send");
        };
        assert!(
            full.body().contains(r#"say \"hi\" and \\ back"#),
            "the quote and the backslash are escaped in `hist`: {}",
            full.body()
        );

        // And in `add`, which is the array a scroll travels in.
        let grown = View::split(captured, 1);
        let patch = cache.diff("desktop/%1", &grown, 40, 1, &full.hh, &full.sh);
        assert!(
            matches!(patch.answer, Answer::Patch { .. }),
            "{:?}",
            patch.answer
        );
        assert!(
            patch.body().contains(r#""add":["#) && !patch.body().contains("\u{1b}"),
            "a coloured line reaches `add` escaped: {}",
            patch.body()
        );
    }

    // AYEAYE-47 — the token is `sha1(text)[:12]`, which is what `bin/ayeaye`
    // mints. Held to the digest rather than to itself, so the two daemons can be
    // compared: a phone mid-session moved between them keeps its window.
    #[test]
    fn a_token_is_the_first_twelve_hex_of_the_daemons_own_hash() {
        assert_eq!(token(""), "da39a3ee5e6b");
        assert_eq!(token("abc"), "a9993e364706");
        assert_eq!(token("abc").len(), 12);
        assert_ne!(token("a\nb"), token("a\nc"));
    }
}
