//! A tmux server of the suite's own.
//!
//! **Nothing in the suite may touch the default tmux socket.** That is where
//! the person running these tests keeps their actual work: reading it would be
//! a test whose result depends on what they happen to have open, and one
//! mistyped subcommand from changing it. So every test that wants a tmux starts
//! its own server, on its own socket, with `-f /dev/null` so none of the
//! machine's configuration reaches it, and kills that server on the way out
//! whether its assertions passed or not.
//!
//! Each test binary compiles this file for itself and uses the part of it that
//! binary needs, so what one leaves unused is not dead — it is in use next door.
#![allow(dead_code)]

use std::process::Command;
use std::time::Duration;

use ayeaye::tmux::Tmux;

/// How long the layer under test gets. Longer than production's, because a
/// loaded build machine is slower than a phone waiting for a poll.
const LIMIT: Duration = Duration::from_secs(10);

/// A tmux server of our own, killed when it goes out of scope.
pub struct Private {
    socket: String,
}

impl Private {
    /// Start one holding a single `work` session, or `None` where this machine
    /// has no tmux to start.
    pub fn named(what: &str) -> Option<Private> {
        let server = Private {
            // The pid keeps two runs of the suite apart; the name keeps two
            // tests of one run apart.
            socket: socket(what),
        };
        server.tmux(&["new-session", "-d", "-s", "work", "-n", "editor", "/bin/sh"])?;
        Some(server)
    }

    /// One tmux command against this server, or `None` if tmux is not here.
    pub fn tmux(&self, args: &[&str]) -> Option<()> {
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
    pub fn layer(&self) -> Tmux {
        on_socket(&self.socket)
    }
}

impl Drop for Private {
    fn drop(&mut self) {
        // Not `self.tmux`: that asserts, and a panicking `Drop` inside another
        // panic aborts the process. A server that is already gone is fine.
        //
        // The empty socket *file* outlives the server it named, in the same
        // per-user directory tmux puts every socket in. That is left alone: it
        // is nothing, and computing its path here would mean re-deriving a rule
        // — `TMUX_TMPDIR`, then the uid — that tmux owns and we would get
        // wrong on the machine where it mattered.
        let _ = Command::new("tmux")
            .args(["-f", "/dev/null", "-L", &self.socket, "kill-server"])
            .output();
    }
}

/// Whether this machine has a tmux for a test to ask at all.
///
/// Every case here skips rather than fails without one, and that has to be the
/// whole file's answer or it is nobody's: a suite where two cases skip and two
/// fail on the same machine is telling you two different things about the same
/// missing program.
pub fn have_tmux() -> bool {
    Command::new("tmux").arg("-V").output().is_ok()
}

/// A tmux pointed at a socket nobody has started a server on.
///
/// What a machine with no sessions looks like — and what a test that does not
/// care about panes should be pointed at, so that "does not care" cannot
/// quietly become "reads whatever the user has open".
pub fn nowhere(what: &str) -> Tmux {
    on_socket(&socket(what))
}

fn socket(what: &str) -> String {
    format!("ayeaye-43-{}-{what}", std::process::id())
}

fn on_socket(socket: &str) -> Tmux {
    Tmux::spelled(&["tmux", "-f", "/dev/null", "-L", socket], LIMIT)
}
