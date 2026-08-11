//! Running a program, with a limit on how long it may take.
//!
//! The standard library has no wait-with-timeout, and no way to reach a child a
//! waiting thread already owns in order to kill it. This is the smallest thing
//! that does both: tokio is already this crate's runtime, so the wait is a
//! future with a deadline and the child dies when that future is dropped.
//!
//! It is separate from [`crate::service::Runner`] on purpose. That one is
//! synchronous and answers `systemctl` at startup, where there is nothing to
//! interleave with; this one is called from inside a request, where a program
//! that never returns would hold a connection open for as long as it liked.

use std::ffi::OsStr;
use std::fmt;
use std::time::Duration;

use tokio::io::AsyncWriteExt;
use tokio::process::Command;

/// What running a program came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ran {
    /// Whether it exited successfully.
    pub ok: bool,
    /// Everything it wrote to stdout, decoded lossily.
    pub stdout: String,
    /// Everything it wrote to stderr, decoded lossily.
    pub stderr: String,
}

/// Why there is no answer at all.
///
/// A program that exited unsuccessfully is not in here: that is an answer, and
/// what it said on the way out is usually the useful part of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failed {
    /// It outran its limit and was killed.
    TimedOut(Duration),
    /// It could not be started, or could not be waited for.
    NotStarted(String),
}

impl fmt::Display for Failed {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Failed::TimedOut(limit) => write!(out, "gave up after {limit:?}"),
            Failed::NotStarted(why) => write!(out, "{why}"),
        }
    }
}

/// Run a program and wait for it, for no longer than `limit`.
///
/// Both streams come back decoded **lossily**. That is not laziness about
/// encodings: a process whose name or output is not valid UTF-8 must cost that
/// one process, and the daemon this replaces answered an undecodable byte
/// anywhere in `list-panes` output by returning nothing at all — one pane with
/// an odd name emptied the panel for every pane.
///
/// stdin is `/dev/null`. A program run from inside a request has no business
/// reading the daemon's own stdin, and one that tries would otherwise block
/// until the limit rather than answering.
pub async fn run<S: AsRef<OsStr>>(argv: &[S], limit: Duration) -> Result<Ran, Failed> {
    run_inner(argv, None, limit).await
}

pub async fn run_with_input<S: AsRef<OsStr>>(
    argv: &[S],
    input: &[u8],
    limit: Duration,
) -> Result<Ran, Failed> {
    run_inner(argv, Some(input), limit).await
}

async fn run_inner<S: AsRef<OsStr>>(
    argv: &[S],
    input: Option<&[u8]>,
    limit: Duration,
) -> Result<Ran, Failed> {
    let Some((program, rest)) = argv.split_first() else {
        return Err(Failed::NotStarted("no command to run".to_string()));
    };
    let named = program.as_ref().to_string_lossy().into_owned();
    let started = Command::new(program)
        .args(rest)
        .stdin(if input.is_some() {
            std::process::Stdio::piped()
        } else {
            std::process::Stdio::null()
        })
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        // The whole point. `wait_with_output` owns the child, so when the
        // deadline drops that future it drops the child with it — and this is
        // what makes that drop a kill rather than an orphan.
        .kill_on_drop(true)
        .spawn();
    let mut child = match started {
        Ok(child) => child,
        Err(why) => return Err(Failed::NotStarted(format!("{named}: {why}"))),
    };
    let waited = async {
        if let Some(input) = input {
            child
                .stdin
                .take()
                .expect("piped when input is present")
                .write_all(input)
                .await?;
        }
        child.wait_with_output().await
    };
    match tokio::time::timeout(limit, waited).await {
        Ok(Ok(output)) => Ok(Ran {
            ok: output.status.success(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        }),
        Ok(Err(why)) => Err(Failed::NotStarted(format!("{named}: {why}"))),
        Err(_) => Err(Failed::TimedOut(limit)),
    }
}

#[cfg(test)]
mod tests {
    use super::{Failed, run, run_with_input};
    use std::time::{Duration, Instant};

    // AYEAYE-43 — both streams and the exit status come back, and a program
    // that fails is an answer rather than an error: what tmux says when it
    // refuses is the part worth passing on.
    #[tokio::test]
    async fn a_program_answers_with_both_its_streams_and_its_status() {
        let ran = run(
            &["sh", "-c", "printf hello; printf oops >&2"],
            Duration::from_secs(5),
        )
        .await
        .expect("sh should run");
        assert!(ran.ok);
        assert_eq!(ran.stdout, "hello");
        assert_eq!(ran.stderr, "oops");

        let refused = run(
            &["sh", "-c", "printf no >&2; exit 3"],
            Duration::from_secs(5),
        )
        .await
        .expect("a failing program still answers");
        assert!(!refused.ok);
        assert_eq!(refused.stderr, "no");
    }

    // AYEAYE-84 — encrypted push bodies contain arbitrary bytes, including NUL.
    #[tokio::test]
    async fn binary_input_reaches_the_program_on_stdin() {
        let ran = run_with_input(
            &["sh", "-c", "od -An -tx1"],
            b"a\0b",
            Duration::from_secs(5),
        )
        .await
        .unwrap();
        assert_eq!(ran.stdout.split_whitespace().collect::<String>(), "610062");
    }

    // AYEAYE-43 — output that is not UTF-8 decodes lossily rather than failing.
    // The daemon this replaces returned nothing at all for an undecodable byte
    // anywhere in the output, so one process with an odd name emptied the pane
    // list for every pane.
    #[tokio::test]
    async fn output_that_is_not_utf_8_costs_only_its_own_bytes() {
        let ran = run(
            &["sh", "-c", r"printf 'be\376\377fore\tafter'"],
            Duration::from_secs(5),
        )
        .await
        .expect("sh should run");
        assert!(ran.ok);
        assert!(
            ran.stdout.starts_with("be") && ran.stdout.ends_with("fore\tafter"),
            "everything around the bad bytes survives: {:?}",
            ran.stdout
        );
    }

    // AYEAYE-43, AYEAYE-70 — a program that outruns its limit is killed and
    // said to have been. Abandoning it would leave the process running and the
    // caller waiting for a future that never resolves.
    #[tokio::test]
    async fn a_program_that_outruns_its_limit_is_killed() {
        // A duration nothing else on the machine would be sleeping for, and
        // long enough that no scheduler stall short of a minute can push a
        // 200 ms wait past it. AYEAYE-70: the bound used to be a fixed 5 s,
        // and heavy parallel load violated it — the elapsed assertion below
        // compares against this sleep instead, because "came back before the
        // child could possibly have exited on its own" is the actual claim.
        const SLEEP_SECS: &str = "67.31";
        let sleeps_for = Duration::from_secs_f64(SLEEP_SECS.parse().expect("a number"));

        let started = Instant::now();
        let outcome = run(&["sleep", SLEEP_SECS], Duration::from_millis(200)).await;
        assert_eq!(
            outcome.unwrap_err(),
            Failed::TimedOut(Duration::from_millis(200))
        );
        assert!(
            started.elapsed() < sleeps_for,
            "the limit is what was waited for, not the sleep"
        );

        // And it is dead, not merely let go of. `pgrep` is asked only when the
        // machine has it; where it does not, the timing above is all this can
        // prove.
        let pattern = format!("sleep {SLEEP_SECS}");
        let found = run(&["pgrep", "-f", pattern.as_str()], Duration::from_secs(5)).await;
        if let Ok(found) = found {
            assert!(
                found.stdout.trim().is_empty(),
                "the child outlived its limit: {:?}",
                found.stdout
            );
        }
    }

    // AYEAYE-43 — a program that cannot be started at all is reported as
    // itself. An empty answer would be indistinguishable from a machine with no
    // panes, which is the one distinction the pane list has to keep.
    #[tokio::test]
    async fn a_program_that_cannot_be_started_says_so() {
        let outcome = run(&["ayeaye-43-no-such-program"], Duration::from_secs(5)).await;
        let Err(Failed::NotStarted(why)) = outcome else {
            panic!("a missing program is not a running one: {outcome:?}");
        };
        assert!(
            why.contains("ayeaye-43-no-such-program"),
            "the program is named in {why:?}"
        );

        let nothing: [&str; 0] = [];
        assert!(matches!(
            run(&nothing, Duration::from_secs(5)).await,
            Err(Failed::NotStarted(_))
        ));
    }
}
