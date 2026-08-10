//! Fit leases, against a real tmux.
//!
//! Every test here drives a tmux server of the suite's own — see `common` for
//! why the default socket is off limits. Resizing somebody's actual window is
//! precisely the damage these tests are about, so none of them may be able to.

mod common;

use std::path::PathBuf;

use ayeaye::fit::{Fits, STATE_FILE};
use ayeaye::tmux::Tmux;
use ayeaye_core::peer::{HostName, PaneId, Peer, Registry};

use common::Private;

fn host() -> HostName {
    HostName::new("desktop").expect("a host name")
}

fn here() -> Registry {
    Registry::new(vec![Peer::here(host())]).expect("one peer, and it is this machine")
}

/// A directory of this test's own to keep a state file in.
///
/// Never the daemon's state directory: a test that wrote there would be writing
/// into the leases of whoever is running the suite.
fn scratch(what: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ayeaye-47-{}-{what}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// The first pane of the server's `work` session, qualified.
async fn only_pane(server: &Private) -> PaneId {
    let panes = server
        .layer()
        .panes(&host())
        .await
        .expect("a running tmux answers");
    panes.first().expect("one pane at least").id.clone()
}

/// The window's size, as tmux reports it.
async fn window(tmux: &Tmux, pane: &str) -> (u16, u16) {
    let said = tmux
        .ask(&[
            "display-message",
            "-p",
            "-t",
            pane,
            "#{window_width}\t#{window_height}",
        ])
        .await
        .expect("tmux should describe a window it has");
    ayeaye_core::fit::size(&said).unwrap_or_else(|| panic!("not a size: {said:?}"))
}

/// Whether tmux says this window is sized by hand.
async fn is_manual(tmux: &Tmux, pane: &str) -> bool {
    let shown = tmux
        .ask(&["show-options", "-w", "-t", pane, "window-size"])
        .await
        .unwrap_or_default();
    ayeaye_core::fit::manual(&shown)
}

// AYEAYE-47 — a window tmux was sizing goes back to tmux sizing it. Releasing
// to *a size* would be indistinguishable from this in a test that only checked
// the window was no longer at phone width, and would leave the desktop pinned
// at whatever it happened to be when the phone arrived.
#[tokio::test]
async fn a_window_tmux_was_sizing_goes_back_to_tmux_sizing_it() {
    let Some(server) = Private::named("fit-auto") else {
        eprintln!("skipped: no tmux on this machine");
        return;
    };
    let tmux = server.layer();
    let id = only_pane(&server).await;
    assert!(
        !is_manual(&tmux, id.pane()).await,
        "a fresh window is not manual"
    );

    let fits = Fits::new(60_000, None);
    fits.acquire(&tmux, &id).await;
    ayeaye::fit::resize(&tmux, id.pane(), 40, 10).await;
    assert_eq!(window(&tmux, id.pane()).await, (40, 10));
    assert!(fits.holds(&id));
    assert!(
        is_manual(&tmux, id.pane()).await,
        "fitting a window is what pins it"
    );

    assert!(fits.release(&tmux, &id).await);
    // The claim is that tmux is in charge of the size again, not that the size
    // changed back: this session has no client attached, so there is nothing for
    // tmux to recompute from and it keeps the last size until one arrives. The
    // option is the whole difference between the two restores — release to *a
    // size* would leave this window pinned at whatever it happened to be when
    // the phone arrived, and would read identically to this if the test only
    // looked at the numbers.
    assert!(
        !is_manual(&tmux, id.pane()).await,
        "the window is still pinned to a size nobody chose"
    );
    assert!(!fits.holds(&id));
    // Releasing what is not held is not an error, and restores nothing: the
    // beacon the page sends on the way out can arrive after a sweep.
    assert!(!fits.release(&tmux, &id).await);
}

// AYEAYE-47 — a window somebody had already sized by hand goes back to *that*
// size. It is the other tmux command entirely, and getting it wrong hands a
// deliberately-sized window back to automatic sizing.
#[tokio::test]
async fn a_window_sized_by_hand_goes_back_to_the_size_it_was() {
    let Some(server) = Private::named("fit-manual") else {
        eprintln!("skipped: no tmux on this machine");
        return;
    };
    let tmux = server.layer();
    let id = only_pane(&server).await;
    ayeaye::fit::resize(&tmux, id.pane(), 100, 40).await;
    assert!(is_manual(&tmux, id.pane()).await);

    let fits = Fits::new(60_000, None);
    fits.acquire(&tmux, &id).await;
    ayeaye::fit::resize(&tmux, id.pane(), 40, 10).await;
    // A second acquire, as a client re-asserting its fit after a drift check
    // does. This is where re-recording the pre-fit state would save phone width
    // as the size to go back to.
    fits.acquire(&tmux, &id).await;

    assert!(fits.release(&tmux, &id).await);
    assert_eq!(window(&tmux, id.pane()).await, (100, 40));
    assert!(is_manual(&tmux, id.pane()).await);
}

// AYEAYE-47 — "a crashed client's window is restored without intervention".
// Nobody releases here: the polls simply stop, which is what a locked phone or
// a closed tab looks like from the server, and the sweeper is the only thing
// that puts the window back.
#[tokio::test]
async fn a_lease_nobody_renews_is_swept_and_the_window_comes_back() {
    let Some(server) = Private::named("fit-sweep") else {
        eprintln!("skipped: no tmux on this machine");
        return;
    };
    let tmux = server.layer();
    let id = only_pane(&server).await;
    ayeaye::fit::resize(&tmux, id.pane(), 100, 40).await;

    let fits = Fits::new(120, None);
    fits.acquire(&tmux, &id).await;
    ayeaye::fit::resize(&tmux, id.pane(), 40, 10).await;

    // A client that is still watching renews, and keeps its window.
    assert!(fits.touch(&id));
    assert_eq!(fits.sweep(&tmux).await, 0);
    assert_eq!(window(&tmux, id.pane()).await, (40, 10));

    // Then it stops. Slept rather than driven by a fake clock on purpose: the
    // claim is about a lease running out in real time while nothing happens.
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    assert_eq!(fits.sweep(&tmux).await, 1);
    assert_eq!(window(&tmux, id.pane()).await, (100, 40));
    assert!(!fits.holds(&id));
    assert_eq!(fits.sweep(&tmux).await, 0, "and nothing is swept twice");
}

// AYEAYE-47 — the sweeper looks often enough to matter. An interval longer than
// the lease would leave a window at phone width for a lease and a half, which is
// the failure "swept on expiry" is about.
#[test]
fn the_sweeper_looks_more_often_than_a_lease_lasts() {
    let production = Fits::new(ayeaye_core::fit::DEFAULT_TTL_MS, None);
    assert_eq!(production.sweep_every(), ayeaye::fit::MAX_SWEEP);
    for ttl in [120, 1_000, ayeaye_core::fit::DEFAULT_TTL_MS, 600_000] {
        let fits = Fits::new(ttl, None);
        let every = fits.sweep_every();
        assert!(
            every.as_millis() as u64 <= ttl.max(ayeaye::fit::MIN_SWEEP.as_millis() as u64),
            "a {ttl}ms lease is swept every {every:?}"
        );
        assert!(every >= ayeaye::fit::MIN_SWEEP, "the sweeper must not spin");
    }
}

// AYEAYE-47 — "leases survive a daemon restart, recovering from persisted
// state". The file is what one process leaves for the next; a window it lists
// goes back without anyone asking, and the file is consumed so a second start
// cannot re-restore a window somebody has since resized by hand.
#[tokio::test]
async fn a_window_a_dead_process_left_fitted_is_restored_at_the_next_start() {
    let Some(server) = Private::named("fit-recover") else {
        eprintln!("skipped: no tmux on this machine");
        return;
    };
    let tmux = server.layer();
    let id = only_pane(&server).await;
    let dir = scratch("recover");
    let path = dir.join(STATE_FILE);

    // One process takes a lease and dies with it held.
    ayeaye::fit::resize(&tmux, id.pane(), 100, 40).await;
    let doomed = Fits::new(60_000, Some(path.clone()));
    doomed.acquire(&tmux, &id).await;
    ayeaye::fit::resize(&tmux, id.pane(), 40, 10).await;
    drop(doomed);
    assert!(path.is_file(), "the lease was never written down");

    // The next one starts, finds the file, and gives the desktop its window
    // back.
    let restarted = Fits::new(60_000, Some(path.clone()));
    restarted.recover(&tmux, &here()).await;
    assert_eq!(window(&tmux, id.pane()).await, (100, 40));
    assert!(!path.exists(), "the file has to be consumed, or a later start restores again");

    // And a third start, after somebody has resized by hand, leaves it alone.
    ayeaye::fit::resize(&tmux, id.pane(), 90, 30).await;
    Fits::new(60_000, Some(path.clone()))
        .recover(&tmux, &here())
        .await;
    assert_eq!(window(&tmux, id.pane()).await, (90, 30));

    let _ = std::fs::remove_dir_all(&dir);
}

// AYEAYE-47 — a released lease leaves nothing behind for the next process to
// restore, and a state file naming another machine's pane is left for that
// machine's own daemon. Restoring it here would run a foreign pane id against
// this tmux.
#[tokio::test]
async fn nothing_is_left_behind_and_nothing_foreign_is_acted_on() {
    let Some(server) = Private::named("fit-elsewhere") else {
        eprintln!("skipped: no tmux on this machine");
        return;
    };
    let tmux = server.layer();
    let id = only_pane(&server).await;
    let dir = scratch("elsewhere");
    let path = dir.join(STATE_FILE);

    let fits = Fits::new(60_000, Some(path.clone()));
    fits.acquire(&tmux, &id).await;
    assert!(fits.release(&tmux, &id).await);
    assert_eq!(
        std::fs::read_to_string(&path).expect("the file is rewritten on release"),
        "",
        "a released lease is still written down as held"
    );

    // A file naming a machine this deployment does not know, and one naming a
    // peer that is not here: both are somebody else's window.
    std::fs::write(&path, "gpu-box/%0\tmanual\t1\t1\nnot-an-id\tauto\n")
        .expect("the scratch directory should be writable");
    Fits::new(60_000, Some(path.clone()))
        .recover(&tmux, &here())
        .await;
    // The local window is untouched, which is the observable half of "nothing
    // foreign was acted on".
    assert!(!is_manual(&tmux, id.pane()).await);
    assert!(!path.exists());

    let _ = std::fs::remove_dir_all(&dir);
}
