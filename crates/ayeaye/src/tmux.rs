//! Asking tmux.
//!
//! The only part of the pane list that touches the machine: it runs `tmux`,
//! decodes what came back, and hands the text to `ayeaye_core::tmux`, which
//! decides what it means. Nothing here parses and nothing there runs anything.

use std::fmt;
use std::time::Duration;

use ayeaye_core::peer::HostName;
use ayeaye_core::prompt::Press;
use ayeaye_core::tmux::{PANE_FORMAT, Pane, no_server_running};

use crate::command::{self, Failed};

/// How long tmux gets to answer.
///
/// Five seconds, which is what `bin/ayeaye`'s `tmux()` allows. It is generous
/// for a `list-panes` and short enough that a wedged tmux costs one poll rather
/// than the connection.
pub const LIMIT: Duration = Duration::from_secs(5);

/// The gap between typing a message and submitting it.
///
/// `bin/ayeaye`'s `SEND_ENTER_DELAY`, which defaults to 0.4 seconds. See
/// [`Tmux::submit`] for what it is for.
pub const SUBMIT_DELAY: Duration = Duration::from_millis(400);

/// A tmux to ask.
#[derive(Debug, Clone)]
pub struct Tmux {
    argv: Vec<String>,
    limit: Duration,
    /// Held rather than read from [`SUBMIT_DELAY`] at the call site so a test
    /// can prove the Enter arrives without waiting out the gap that exists for
    /// a TUI's benefit.
    submit_delay: Duration,
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
            submit_delay: SUBMIT_DELAY,
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
            submit_delay: SUBMIT_DELAY,
        }
    }

    /// The same, submitting after a gap of the caller's choosing.
    ///
    /// Also for the suite: the 400ms gap exists so a TUI does not read the Enter
    /// as part of the same input burst, and a test that has to wait it out on
    /// every case pays for a property it is not asserting.
    pub fn submitting_after(mut self, delay: Duration) -> Tmux {
        self.submit_delay = delay;
        self
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

    /// What one pane's screen says right now.
    ///
    /// The pane is a [`Pane`] rather than an id, and that is the whole defence:
    /// a `Pane` comes back from [`Tmux::panes`] and nowhere else, so the target
    /// of every method below is one this tmux itself named a moment ago. The
    /// daemon this replaces checks membership by hand at each call site — three
    /// of them, and `/api/answer` and `/api/send` are not among the three.
    pub async fn capture(&self, pane: &Pane) -> Result<String, Trouble> {
        self.ask(&["capture-pane", "-p", "-t", pane.id.pane()])
            .await
    }

    /// Press one key in a pane.
    ///
    /// A [`Press`] can only be built from `ayeaye_core::prompt`'s table, so what
    /// arrives at `send-keys` is a `&'static str` from that file rather than
    /// anything a request carried.
    pub async fn press(&self, pane: &Pane, key: Press) -> Result<(), Trouble> {
        let target = pane.id.pane();
        let mut argv = vec!["send-keys", "-t", target];
        if key.literal() {
            // `-l` is "this is text, not the name of a key", and `--` ends the
            // options so nothing beginning with a dash is read as one.
            argv.extend(["-l", "--"]);
        }
        argv.push(key.key());
        self.ask(&argv).await.map(|_| ())
    }

    /// Type some text into a pane, without submitting it.
    ///
    /// `-l` is literal, so nothing in the text is read as a key name, and `--`
    /// ends the options so text beginning with a dash is text. Nothing is
    /// shell-quoted, because nothing here crosses a shell: `send-keys` is handed
    /// the text as one element of an argument vector and writes those bytes to
    /// the pane's terminal.
    ///
    /// The text has to have come through `ayeaye_core::prompt::typed`, which is
    /// what keeps a newline — a submit — out of it.
    pub async fn type_text(&self, pane: &Pane, text: &str) -> Result<(), Trouble> {
        self.ask(&["send-keys", "-t", pane.id.pane(), "-l", "--", text])
            .await
            .map(|_| ())
    }

    /// Wait a moment, then press Enter.
    ///
    /// The wait is not politeness. Codex's composer swallows an Enter that
    /// arrives in the same input burst as the text — it reads as a newline
    /// rather than a submit, so the message just sits there. Claude tolerates
    /// it; a short gap makes both behave.
    pub async fn submit(&self, pane: &Pane) -> Result<(), Trouble> {
        tokio::time::sleep(self.submit_delay).await;
        self.ask(&["send-keys", "-t", pane.id.pane(), "Enter"])
            .await
            .map(|_| ())
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
