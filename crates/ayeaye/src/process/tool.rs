//! Running an inspection tool and taking what it said.
//!
//! Only the macOS backend needs this — Linux answers every question out of a
//! file — but the seam is what makes that backend testable at all on a machine
//! nobody can run it on: a substitute [`Tool`] answers with the captured output
//! of a real Mac, and records what it was asked for.
//!
//! A general subprocess helper with a timeout is arriving separately as part of
//! the tmux layer. When both have landed, one of these should go; this one is
//! deliberately the smallest thing that answers the four questions this module
//! asks.

use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Something that can be asked a question about the machine.
pub trait Tool {
    /// Everything it printed to standard output, or `None` if it could not be
    /// run at all.
    ///
    /// A non-zero exit is not a failure here: `lsof` exits 1 when it has a
    /// complaint about a file it could still report on, and the output is the
    /// answer either way.
    fn run(&self, argv: &[String]) -> Option<String>;
}

/// The real one.
#[derive(Debug, Clone, Copy)]
pub struct Subprocess {
    /// How long a tool is given before it is given up on.
    ///
    /// The standard library has no wait-with-timeout, and without one a single
    /// `lsof` blocked on an unresponsive mount holds the request that asked.
    pub patience: Duration,
}

impl Default for Subprocess {
    fn default() -> Self {
        Subprocess {
            patience: Duration::from_secs(5),
        }
    }
}

impl Tool for Subprocess {
    fn run(&self, argv: &[String]) -> Option<String> {
        let (program, arguments) = argv.split_first()?;
        let mut child = Command::new(program)
            .args(arguments)
            // The C locale, because a translated header or a decimal comma is a
            // parser looking at something it has never seen.
            .env("LC_ALL", "C")
            // Nothing to say to it, and a closed stdin also settles what BSD
            // `ps` takes its output width from when it looks for a terminal.
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;

        // Read on another thread: a tool that fills the pipe while nobody is
        // draining it blocks, and then so does the wait below.
        let mut pipe = child.stdout.take()?;
        let reading = std::thread::spawn(move || {
            let mut said = Vec::new();
            let _ = pipe.read_to_end(&mut said);
            said
        });

        let deadline = Instant::now() + self.patience;
        let finished = loop {
            match child.try_wait() {
                // The exit status is deliberately not read. A non-zero exit is
                // ordinary here and the output is the answer regardless.
                Ok(Some(_)) => break true,
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                // Out of patience, or the wait itself failed. The only process
                // this ever signals is one it started.
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break false;
                }
            }
        };

        let said = reading.join().ok()?;
        // Lossily: no filesystem enforces valid UTF-8 in a path, and a strict
        // decode would turn one unrelated process into an empty tree for every
        // pane at once.
        finished.then(|| String::from_utf8_lossy(&said).into_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::{Subprocess, Tool};
    use std::time::{Duration, Instant};

    fn argv(words: &[&str]) -> Vec<String> {
        words.iter().map(|word| (*word).to_string()).collect()
    }

    // AYEAYE-44
    #[test]
    fn what_a_tool_printed_is_what_comes_back() {
        let said = Subprocess::default().run(&argv(&["echo", "hello there"]));
        assert_eq!(said.as_deref(), Some("hello there\n"));
    }

    // AYEAYE-44 — `lsof` is not installed everywhere, and a machine without it
    // is a machine that answers less rather than one that errors.
    #[test]
    fn a_tool_that_is_not_there_is_not_an_error() {
        assert_eq!(
            Subprocess::default().run(&argv(&["ayeaye-no-such-tool", "-x"])),
            None
        );
        assert_eq!(Subprocess::default().run(&[]), None);
    }

    // AYEAYE-44 — a non-zero exit is not a failure: the output is the answer.
    #[test]
    fn a_complaint_does_not_throw_the_answer_away() {
        let said = Subprocess::default().run(&argv(&["sh", "-c", "echo said; exit 1"]));
        assert_eq!(said.as_deref(), Some("said\n"));
    }

    // AYEAYE-44 — the C locale, because a translated `ps` header or a decimal
    // comma is a parser looking at something it has never seen.
    #[test]
    fn a_tool_is_asked_in_the_c_locale() {
        let said = Subprocess::default().run(&argv(&["sh", "-c", "printf %s \"$LC_ALL\""]));
        assert_eq!(said.as_deref(), Some("C"));
    }

    // AYEAYE-44 — no filesystem enforces valid UTF-8 in a path, and a strict
    // decode would turn one unrelated process into an empty tree for every
    // pane at once. This is the pin the pure crate cannot hold: by the time
    // text reaches it, the decoding has already happened.
    #[test]
    fn output_that_is_not_utf8_comes_back_anyway() {
        let said = Subprocess::default()
            .run(&argv(&["sh", "-c", "printf 'co\\377dex\\n'"]))
            .expect("output, however it is encoded");
        assert!(said.starts_with("co"), "{said:?}");
        assert!(
            said.contains('\u{fffd}'),
            "the odd byte is replaced: {said:?}"
        );
    }

    // AYEAYE-44 — a tool blocked on an unresponsive mount otherwise holds the
    // request that asked, and the standard library has no wait-with-timeout.
    #[test]
    fn a_tool_that_never_finishes_is_given_up_on() {
        let impatient = Subprocess {
            patience: Duration::from_millis(200),
        };
        let started = Instant::now();
        let said = impatient.run(&argv(&["sleep", "30"]));
        assert_eq!(
            said, None,
            "nothing was learned, and nothing is what it says"
        );
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "it waited {:?}",
            started.elapsed()
        );
    }
}
