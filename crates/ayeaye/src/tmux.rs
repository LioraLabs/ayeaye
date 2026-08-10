//! Asking tmux.
//!
//! The only part of the pane list that touches the machine: it runs `tmux`,
//! decodes what came back, and hands the text to `ayeaye_core::tmux`, which
//! decides what it means. Nothing here parses and nothing there runs anything.

use std::fmt;
use std::time::Duration;

use ayeaye_core::peer::HostName;
use ayeaye_core::tmux::{PANE_FORMAT, Pane, no_server_running};

use crate::command::{self, Failed};

/// How long tmux gets to answer.
///
/// Five seconds, which is what `bin/ayeaye`'s `tmux()` allows. It is generous
/// for a `list-panes` and short enough that a wedged tmux costs one poll rather
/// than the connection.
pub const LIMIT: Duration = Duration::from_secs(5);

/// A tmux to ask.
#[derive(Debug, Clone)]
pub struct Tmux {
    argv: Vec<String>,
    limit: Duration,
}

impl Default for Tmux {
    fn default() -> Tmux {
        Tmux::new()
    }
}

impl Tmux {
    /// The tmux this machine's panes are on.
    pub fn new() -> Tmux {
        Tmux {
            argv: vec!["tmux".to_string()],
            limit: LIMIT,
        }
    }

    /// A tmux spelled some other way.
    ///
    /// This exists for the suite, and the reason is worth stating: a test that
    /// asked the *real* tmux anything would be reading somebody's actual work,
    /// and one mistyped subcommand away from changing it. Tests pass
    /// `tmux -f /dev/null -L <their own socket>`, which is a private server
    /// with none of the user's configuration in it.
    pub fn spelled(argv: &[&str], limit: Duration) -> Tmux {
        Tmux {
            argv: argv.iter().map(|word| word.to_string()).collect(),
            limit,
        }
    }

    /// Run one tmux command and give back what it printed.
    pub async fn ask(&self, args: &[&str]) -> Result<String, Trouble> {
        let mut argv = self.argv.clone();
        argv.extend(args.iter().map(|word| word.to_string()));
        match command::run(&argv, self.limit).await {
            Ok(ran) if ran.ok => Ok(ran.stdout),
            // What tmux said when it refused is the whole message. It goes to
            // whoever is looking at the panel, and every paraphrase of it is
            // worth less than the sentence tmux wrote.
            Ok(ran) => Err(Trouble::Refused(if ran.stderr.trim().is_empty() {
                ran.stdout
            } else {
                ran.stderr
            })),
            Err(why) => Err(Trouble::NotRun(why)),
        }
    }

    /// Every live pane on this machine, qualified with the name it goes by.
    ///
    /// A machine with no tmux server has no panes, and that is an answer rather
    /// than a failure — most machines are in that state most of the time.
    /// Anything else tmux says is carried up as itself, because "I could not
    /// look" and "there is nothing to see" must not arrive as the same thing.
    pub async fn panes(&self, host: &HostName) -> Result<Vec<Pane>, Trouble> {
        match self.ask(&["list-panes", "-a", "-F", PANE_FORMAT]).await {
            Ok(text) => Ok(ayeaye_core::tmux::panes(host, &text)),
            Err(Trouble::Refused(said)) if no_server_running(&said) => Ok(Vec::new()),
            Err(trouble) => Err(trouble),
        }
    }
}

/// Why tmux gave no answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Trouble {
    /// It could not be run at all, or did not finish inside the limit.
    NotRun(Failed),
    /// It ran, and refused.
    Refused(String),
}

impl fmt::Display for Trouble {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Trouble::NotRun(why) => write!(out, "tmux: {why}"),
            Trouble::Refused(said) => write!(out, "tmux: {}", said.trim()),
        }
    }
}
