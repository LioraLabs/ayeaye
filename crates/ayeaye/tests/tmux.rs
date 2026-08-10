//! The tmux layer, against a real tmux.
//!
//! Every test here asks a tmux server of the suite's own — see `common` for why
//! the default socket is off limits.

mod common;

use std::time::Duration;

use ayeaye::tmux::{Tmux, Trouble};
use ayeaye_core::peer::HostName;

use common::Private;

fn host() -> HostName {
    HostName::new("desktop").expect("a host name")
}

// AYEAYE-43 — the panes really come from tmux: the format is one tmux accepts,
// the fields land where the parser expects them, and the ids come back
// qualified. A unit test over captured text cannot say any of that.
#[tokio::test]
async fn the_panes_of_a_real_tmux_come_back_qualified() {
    let Some(server) = Private::named("live") else {
        eprintln!("skipped: no tmux on this machine");
        return;
    };
    server.tmux(&["new-window", "-t", "work", "-n", "cook", "-d", "/bin/sh"]);
    // A floating scratch session, which the panel must not offer as a target.
    server.tmux(&["new-session", "-d", "-s", "_scratch", "/bin/sh"]);

    let panes = server
        .layer()
        .panes(&host())
        .await
        .expect("a running tmux answers");

    let sessions: Vec<&str> = panes.iter().map(|pane| pane.session.as_str()).collect();
    assert_eq!(
        sessions,
        ["work", "work"],
        "the _scratch session is not a target"
    );
    for pane in &panes {
        let qualified = pane.id.qualified();
        assert!(
            qualified.starts_with("desktop/%"),
            "{qualified} should be this machine's, and a tmux pane id"
        );
    }
    let names: Vec<&str> = panes.iter().map(|pane| pane.name.as_str()).collect();
    assert_eq!(names, ["editor", "cook"]);
    assert!(panes.iter().all(|pane| pane.active), "one pane per window");
}

// AYEAYE-43 — a machine with no tmux server has no panes, and that is an
// answer. Most machines are in that state most of the time, and a failure there
// would put an error in the panel on every one of them.
#[tokio::test]
async fn a_socket_with_no_server_has_no_panes_and_no_complaint() {
    if !common::have_tmux() {
        eprintln!("skipped: no tmux on this machine");
        return;
    }
    assert_eq!(
        common::nowhere("nobody").panes(&host()).await,
        Ok(Vec::new())
    );
}

// AYEAYE-43 — and a tmux that could not be run at all is not silence. "I could
// not look" and "there is nothing to see" arriving as the same answer is the
// failure the pane list exists to avoid.
#[tokio::test]
async fn a_tmux_that_is_not_there_is_reported_rather_than_swallowed() {
    let missing = Tmux::spelled(&["ayeaye-43-no-such-tmux"], Duration::from_secs(10));
    let Err(trouble) = missing.panes(&host()).await else {
        panic!("a tmux that does not exist cannot have answered");
    };
    assert!(matches!(trouble, Trouble::NotRun(_)), "{trouble:?}");
    assert!(
        trouble.to_string().contains("ayeaye-43-no-such-tmux"),
        "the thing that could not be run is named: {trouble}"
    );
}

// AYEAYE-45 — the live-session list really comes from tmux: one name to a
// line, and a session name with a space in it stays one session. A unit test
// over text somebody typed cannot say that tmux prints it that way.
#[tokio::test]
async fn the_live_sessions_of_a_real_tmux_come_back_one_to_a_line() {
    let Some(server) = Private::named("sessions") else {
        eprintln!("skipped: no tmux on this machine");
        return;
    };
    server.tmux(&["new-session", "-d", "-s", "my work", "/bin/sh"]);

    let mut sessions = server
        .layer()
        .sessions()
        .await
        .expect("a running tmux answers");
    sessions.sort();
    assert_eq!(
        sessions,
        ["my work", "work"],
        "a name with a space in it is one session, not two"
    );
}

// AYEAYE-45 — a machine with no tmux server has no sessions, and that is an
// answer rather than a failure, for the same reason it is for panes.
#[tokio::test]
async fn a_socket_with_no_server_has_no_sessions_and_no_complaint() {
    if !common::have_tmux() {
        eprintln!("skipped: no tmux on this machine");
        return;
    }
    assert_eq!(
        common::nowhere("no-sessions").sessions().await,
        Ok(Vec::new())
    );
}
