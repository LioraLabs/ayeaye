//! The tmux layer, against a real tmux.
//!
//! Every test here starts its **own** tmux server on its own socket, with
//! `-f /dev/null` so none of the machine's configuration reaches it, and kills
//! that server on the way out whether the assertions passed or not. Nothing in
//! this file may touch the default socket: that is where the person running the
//! suite keeps their actual work, and a test that read it would be one mistyped
//! subcommand from changing it.

use std::process::Command;
use std::time::Duration;

use ayeaye::tmux::{Tmux, Trouble};
use ayeaye_core::peer::HostName;

/// A tmux server of our own, killed when it goes out of scope.
struct Private {
    socket: String,
}

impl Private {
    /// Start one, or say why the suite cannot have one.
    fn named(what: &str) -> Option<Private> {
        // The pid keeps two runs of the suite apart; the name keeps two tests
        // in one run apart.
        let socket = format!("ayeaye-43-{}-{what}", std::process::id());
        let server = Private { socket };
        server.tmux(&["new-session", "-d", "-s", "work", "-n", "editor", "/bin/sh"])?;
        Some(server)
    }

    /// One tmux command against this server, or `None` if tmux is not here.
    fn tmux(&self, args: &[&str]) -> Option<()> {
        let ran = Command::new("tmux")
            .args(["-f", "/dev/null", "-L", &self.socket])
            .args(args)
            .output()
            .ok()?;
        assert!(
            ran.status.success(),
            "the test's own tmux refused {args:?}: {}",
            String::from_utf8_lossy(&ran.stderr)
        );
        Some(())
    }

    /// The layer under test, pointed at this server.
    fn layer(&self) -> Tmux {
        Tmux::spelled(
            &["tmux", "-f", "/dev/null", "-L", &self.socket],
            Duration::from_secs(10),
        )
    }
}

impl Drop for Private {
    fn drop(&mut self) {
        // Not `self.tmux`: that asserts, and a panicking Drop during another
        // panic aborts the process. A server that is already gone is fine.
        let _ = Command::new("tmux")
            .args(["-f", "/dev/null", "-L", &self.socket, "kill-server"])
            .output();
    }
}

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
    let empty = Tmux::spelled(
        &[
            "tmux",
            "-f",
            "/dev/null",
            "-L",
            &format!("ayeaye-43-{}-nobody", std::process::id()),
        ],
        Duration::from_secs(10),
    );
    assert_eq!(empty.panes(&host()).await, Ok(Vec::new()));
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
