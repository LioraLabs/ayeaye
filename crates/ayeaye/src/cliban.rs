//! Running cliban.
//!
//! The board page reads the same cliban the terminal does, through its `--json`
//! output rather than the SQLite file underneath, so list and show semantics —
//! archival, ordering, relations — stay cliban's problem. This is the whole of
//! the effect: a program, some arguments, and a bounded wait. What the output
//! means is `ayeaye_core::board`'s.

use std::time::Duration;

/// How long cliban gets to answer.
///
/// The daemon's own `timeout=15`. Long enough that a cold SQLite open on a
/// spinning disk is not a failure, short enough that a wedged subprocess is a
/// stated reason rather than a request that never returns.
pub const TIMEOUT: Duration = Duration::from_secs(15);

/// The cliban this daemon runs.
#[derive(Debug, Clone)]
pub struct Cliban {
    /// The program to run, already resolved — see `config::choose_cliban`.
    pub program: String,
    /// How long it gets. A field rather than a constant read inside, so a test
    /// can observe the timeout without waiting out the real one.
    pub timeout: Duration,
}

impl Cliban {
    pub fn new(program: String) -> Cliban {
        Cliban {
            program,
            timeout: TIMEOUT,
        }
    }

    /// Run it, and return its stdout or a reason.
    ///
    /// The reason is cliban's own stderr where it has any, its exit code where
    /// it does not, and the spawn error where it never started — the daemon's
    /// three cases, in its order. Every one of them ends up drawn on the board
    /// page, so a reason that says nothing is a panel that explains nothing.
    ///
    /// Output decodes lossily rather than failing. A single undecodable byte
    /// somewhere in a ticket title is not a reason to answer nothing about the
    /// whole board.
    pub async fn run(&self, args: &[&str]) -> Result<String, String> {
        let mut command = tokio::process::Command::new(&self.program);
        command
            .args(args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            // Without this, a request that gave up waiting would leave the
            // child it started running behind it, and a page that retries
            // would stack another one on top.
            .kill_on_drop(true);

        let finished = match tokio::time::timeout(self.timeout, command.output()).await {
            Ok(Ok(finished)) => finished,
            // The program is named because the reason is read by someone
            // finding out that this machine's PATH is not the one they meant.
            Ok(Err(why)) => return Err(format!("{}: {why}", self.program)),
            Err(_) => {
                return Err(format!(
                    "cliban did not answer within {} seconds",
                    self.timeout.as_secs_f32()
                ));
            }
        };

        if !finished.status.success() {
            let complaint = String::from_utf8_lossy(&finished.stderr).trim().to_string();
            if !complaint.is_empty() {
                return Err(complaint);
            }
            return Err(match finished.status.code() {
                Some(code) => format!("cliban exited {code}"),
                // No code means a signal, which is the shape a `kill` or an
                // out-of-memory takes; "exited 0" would be a lie about both.
                None => "cliban was killed before it answered".to_string(),
            });
        }
        Ok(String::from_utf8_lossy(&finished.stdout).into_owned())
    }
}
