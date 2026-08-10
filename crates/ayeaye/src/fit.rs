//! Holding a window at a phone's size, and letting go of it.
//!
//! The policy is [`ayeaye_core::fit`] and is pure. This is the half that has
//! effects: it asks tmux what a window's sizing was, resizes it, puts it back,
//! and keeps the leases on disk so a process that dies mid-fit cannot strand a
//! desktop at phone width.
//!
//! **The state file is not `bin/ayeaye`'s `fits.json`.** Both daemons run side
//! by side for the rest of this milestone, and each rewrites its file from its
//! own map — so sharing one means either daemon's save silently drops the
//! other's leases, which is the exact stranding this feature exists to prevent.
//! Draining the Python daemon's file once, at the cutover, belongs to the ticket
//! that retires it.

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use ayeaye_core::fit::{Leases, Prev};
use ayeaye_core::peer::{PaneId, Registry};

use crate::tmux::Tmux;

/// What the state file is called under the state directory.
pub const STATE_FILE: &str = "fits.tsv";

/// The longest the sweeper waits between passes.
///
/// Two seconds, as `bin/ayeaye`'s sweeper sleeps. At the production TTL that is
/// what [`Fits::sweep_every`] comes to; a shorter TTL sweeps proportionally
/// faster, which is what lets a test prove expiry in milliseconds rather than
/// waiting out twelve seconds of it.
pub const MAX_SWEEP: Duration = Duration::from_secs(2);

/// The shortest it waits, so a very short TTL cannot turn the sweeper into a
/// spin.
pub const MIN_SWEEP: Duration = Duration::from_millis(50);

/// The windows this daemon is holding, and the file that outlives it.
#[derive(Debug)]
pub struct Fits {
    leases: Mutex<Leases>,
    /// Held for the whole of an acquire, a release, a forget or a sweep.
    ///
    /// The lock above guards the *map* and is dropped before any tmux call, as
    /// it must be — an `await` under a blocking lock is a deadlock waiting to
    /// happen. But that leaves a gap with teeth in it: a sweep takes the lease
    /// out of the map and then awaits the restore, and an `/api/resize` landing
    /// in that gap sees no lease, reads the pre-fit state off a window that is
    /// **still at phone width**, and records phone width as the state to go back
    /// to. The desktop would then be pinned there for good — and the page
    /// re-asserts its fit on drift every five seconds, so the trigger is
    /// ordinary rather than exotic.
    ///
    /// So the four operations that pair a map change with a tmux call take their
    /// turn here. `touch` does not: it is the per-poll hot path, it changes no
    /// window, and a renewal that loses a race with a sweep is a lease that had
    /// already run out.
    ///
    /// `bin/ayeaye` has the same gap — `del _fits[p]` and then `_fit_restore`
    /// outside the lock — so this is a deliberate divergence rather than a port.
    turn: tokio::sync::Mutex<()>,
    /// Where the leases are written, or `None` for a deployment with no state
    /// directory to write to. `None` is a working daemon that cannot survive its
    /// own restart, which is better than one that will not start.
    path: Option<PathBuf>,
    /// The clock, as a monotonic origin rather than a wall time. A wall clock
    /// that steps backwards — an NTP correction, a laptop waking up — would
    /// leave every lease looking fresh for as long as the step, and the desktop
    /// would keep its phone width for that long.
    started: Instant,
}

impl Fits {
    /// Leases that expire `ttl_ms` after their last renewal, persisted to
    /// `path`.
    pub fn new(ttl_ms: u64, path: Option<PathBuf>) -> Fits {
        Fits {
            leases: Mutex::new(Leases::new(ttl_ms)),
            turn: tokio::sync::Mutex::new(()),
            path,
            started: Instant::now(),
        }
    }

    /// How often the sweeper should look.
    ///
    /// Derived from the TTL rather than fixed, so the interval and the lifetime
    /// cannot drift apart: a sweeper slower than the TTL leaves a window at
    /// phone width for a lease and a half.
    pub fn sweep_every(&self) -> Duration {
        Duration::from_millis(self.ttl_ms() / 6).clamp(MIN_SWEEP, MAX_SWEEP)
    }

    /// How long an unrenewed lease lives.
    pub fn ttl_ms(&self) -> u64 {
        self.locked().ttl_ms()
    }

    /// Whether this window is held.
    pub fn holds(&self, id: &PaneId) -> bool {
        self.locked().holds(&id.qualified())
    }

    /// Renew a lease, as a side effect of the pane being watched.
    ///
    /// Never creates one: a lease with no recorded pre-fit state would be swept
    /// into restoring a window to nothing.
    pub fn touch(&self, id: &PaneId) -> bool {
        let now = self.now();
        self.locked().touch(&id.qualified(), now)
    }

    /// Take the lease, recording what the window was before it.
    ///
    /// The read of the pre-fit state happens **outside** the lock and only when
    /// there is no lease yet, which is both a tmux call saved on every renewal
    /// and the reason a second acquire cannot record phone width as the state to
    /// go back to.
    pub async fn acquire(&self, tmux: &Tmux, id: &PaneId) {
        if self.touch(id) {
            return;
        }
        let _turn = self.turn.lock().await;
        // Again, under the turn: a restore may have finished while this call was
        // waiting for it, and the window it was about to record has moved.
        if self.touch(id) {
            return;
        }
        let prev = pre_fit(tmux, id.pane()).await;
        let now = self.now();
        // `Leases::acquire` keeps whichever state was recorded first, so two
        // clients arriving together cost one extra read rather than a wrong
        // restore.
        self.locked().acquire(&id.qualified(), prev, now);
        self.save();
    }

    /// End a lease and put the window back. `false` if there was none.
    pub async fn release(&self, tmux: &Tmux, id: &PaneId) -> bool {
        let _turn = self.turn.lock().await;
        let Some(prev) = self.locked().release(&id.qualified()) else {
            return false;
        };
        restore(tmux, id.pane(), &prev).await;
        self.save();
        true
    }

    /// Drop a lease without restoring anything, and hand sizing back to tmux.
    ///
    /// The manual escape hatch. Whatever the window was before, somebody has now
    /// asked for it to be automatic, and a lease still holding the old state
    /// would undo that the moment it expired.
    pub async fn forget(&self, tmux: &Tmux, id: &PaneId) {
        let _turn = self.turn.lock().await;
        self.locked().forget(&id.qualified());
        self.save();
        restore(tmux, id.pane(), &Prev::Auto).await;
    }

    /// Restore every window whose lease has run out, and say how many.
    ///
    /// This is the only cleanup path there is: nothing else expires a lease, so
    /// a client that stops polling for any reason at all — including having
    /// ceased to exist — is handled by exactly this and by nothing else.
    pub async fn sweep(&self, tmux: &Tmux) -> usize {
        let _turn = self.turn.lock().await;
        let now = self.now();
        let due = self.locked().due(now);
        for (pane, prev) in &due {
            // Every key in here arrived as a `PaneId` and was written back out
            // with `qualified()`, so this cannot fail — but the key is a
            // `String` by the time it comes back and the alternative is an
            // `expect` in a sweeper nobody is watching.
            if let Ok(id) = PaneId::parse(pane) {
                restore(tmux, id.pane(), prev).await;
            }
        }
        if !due.is_empty() {
            self.save();
        }
        due.len()
    }

    /// Restore the windows a previous process left fitted, then start clean.
    ///
    /// Called once, at startup. The file is consumed: a second start must not
    /// re-restore a window somebody has since resized by hand, and the leases in
    /// it are all long expired anyway — the process holding them is gone.
    ///
    /// Only this machine's panes. A qualified id naming a peer is left for
    /// whoever owns that machine's tmux, which is that machine's own daemon.
    pub async fn recover(&self, tmux: &Tmux, peers: &Registry) {
        let Some(path) = &self.path else {
            return;
        };
        let Ok(text) = std::fs::read_to_string(path) else {
            return;
        };
        for (pane, prev) in Leases::read(&text) {
            match peers.route(&pane) {
                Ok((peer, id)) if peer.is_here() => restore(tmux, id.pane(), &prev).await,
                _ => continue,
            }
        }
        let _ = std::fs::remove_file(path);
    }

    /// Write the leases out.
    ///
    /// Failures are swallowed on purpose, and this is the one place that is the
    /// right answer: a state directory that cannot be written is a daemon that
    /// cannot survive a restart, which is worth less than a daemon that refuses
    /// to resize a window.
    fn save(&self) {
        let Some(path) = &self.path else {
            return;
        };
        let text = self.locked().saved();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(path, text);
    }

    /// Milliseconds since this process started holding leases.
    fn now(&self) -> u64 {
        self.started.elapsed().as_millis() as u64
    }

    /// The leases, with a poisoned lock treated as the leases it still holds.
    ///
    /// A panic in one request must not leave every window on the machine stuck
    /// at phone width for good, which is what refusing a poisoned lock here
    /// would mean.
    fn locked(&self) -> std::sync::MutexGuard<'_, Leases> {
        self.leases.lock().unwrap_or_else(|held| held.into_inner())
    }
}

/// Run the sweeper for as long as the server does.
///
/// **This is the only cleanup path.** There is no second one, and the reason is
/// worth keeping: two paths that expire leases would each have to know what the
/// other had already restored, and the first disagreement would put a window
/// back to a size it was never at.
pub fn sweeper(settings: std::sync::Arc<crate::config::Settings>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let every = settings.fits.sweep_every();
        loop {
            tokio::time::sleep(every).await;
            settings.fits.sweep(&settings.tmux).await;
        }
    })
}

/// What tmux says this window's sizing was before anyone fitted it.
///
/// A window it will not describe is reported as automatic, which is what
/// `bin/ayeaye` does: automatic is the state tmux itself falls back to, so
/// restoring to it is the answer that leaves the desktop in charge.
async fn pre_fit(tmux: &Tmux, pane: &str) -> Prev {
    let shown = tmux
        .ask(&["show-options", "-w", "-t", pane, "window-size"])
        .await
        .unwrap_or_default();
    if !ayeaye_core::fit::manual(&shown) {
        return Prev::Auto;
    }
    let said = tmux
        .ask(&[
            "display-message",
            "-p",
            "-t",
            pane,
            "#{window_width}\t#{window_height}",
        ])
        .await
        .unwrap_or_default();
    match ayeaye_core::fit::size(&said) {
        Some((cols, rows)) => Prev::Manual { cols, rows },
        None => Prev::Auto,
    }
}

/// Put one window back the way it was.
async fn restore(tmux: &Tmux, pane: &str, prev: &Prev) {
    run(tmux, prev.restore(pane)).await;
}

/// Resize one window to a size a client asked for.
pub async fn resize(tmux: &Tmux, pane: &str, cols: u16, rows: u16) {
    run(tmux, ayeaye_core::fit::resize(pane, cols, rows)).await;
}

/// Run one of the core's argvs.
///
/// A refusal goes to the journal and no further. Every caller here is either a
/// sweeper with nobody to answer or a request whose useful work is already done,
/// and a pane that has closed since the lease was taken is the ordinary way this
/// fails.
async fn run(tmux: &Tmux, argv: Vec<String>) {
    let borrowed: Vec<&str> = argv.iter().map(String::as_str).collect();
    if let Err(trouble) = tmux.ask(&borrowed).await {
        eprintln!("ayeaye: {trouble}");
    }
}
