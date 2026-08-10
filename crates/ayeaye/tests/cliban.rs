//! Running cliban, observed the way the request handler observes it.
//!
//! Every test here runs a `/bin/sh` stand-in written into the suite's own
//! temporary directory, never the real `cliban`. That is not only for speed:
//! the board this project is tracked on is the board the real binary would
//! answer about, and a test suite is no place to be one typo away from writing
//! to it. A stand-in also produces the answers a live board never would — a
//! non-zero exit, an empty stderr, a hang — which are the ones worth testing.

use std::io::Write;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;

use ayeaye::cliban::Cliban;

/// Where the stand-ins live, and the mark the hanging one would leave.
fn scratch() -> PathBuf {
    PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("cliban-stand-ins")
}

fn marker() -> PathBuf {
    scratch().join("still-running")
}

/// Every stand-in, written once before any of them is ever run.
///
/// Written up front rather than per test, and that is not tidiness: these
/// tests run in parallel threads in one process, and a `fork` in one thread
/// while another still holds a file open for writing hands the child an
/// inherited write handle to the very program it is about to exec. Linux
/// answers that with `ETXTBSY`, intermittently and only under load. `OnceLock`
/// makes every write finish before the first spawn.
fn stand_ins() -> &'static PathBuf {
    static WRITTEN: OnceLock<PathBuf> = OnceLock::new();
    WRITTEN.get_or_init(|| {
        use std::os::unix::fs::PermissionsExt;

        let directory = scratch();
        std::fs::create_dir_all(&directory).expect("a writable temporary directory");
        let _ = std::fs::remove_file(marker());
        for (name, body) in [
            ("says-its-arguments", r#"printf '%s\n' "$@""#.to_string()),
            (
                "complains",
                "echo 'error: no such project: NOPE' >&2\nexit 2".to_string(),
            ),
            ("fails-silently", "exit 3".to_string()),
            ("kills-itself", "kill -TERM $$".to_string()),
            (
                "hangs",
                format!("sleep 2\ntouch {}", marker().to_string_lossy()),
            ),
        ] {
            let path = directory.join(name);
            let mut file = std::fs::File::create(&path).expect("a written stand-in");
            write!(file, "#!/bin/sh\n{body}\n").expect("a written stand-in");
            drop(file);
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
                .expect("a runnable stand-in");
        }
        directory
    })
}

fn cliban(name: &str) -> Cliban {
    Cliban::new(stand_ins().join(name).to_string_lossy().into_owned())
}

// AYEAYE-53 — the arguments reach the program and its stdout comes back
// whole. Everything the board endpoints do is downstream of this one fact.
#[tokio::test]
async fn stdout_comes_back_and_the_arguments_got_there() {
    let cliban = cliban("says-its-arguments");
    assert_eq!(
        cliban.run(&["issue", "ls", "--json"]).await,
        Ok("issue\nls\n--json\n".to_string())
    );
}

// AYEAYE-53 — a failure states its reason, and the reason is cliban's own
// words where it has any. `stderr.strip() or "cliban exited %d"` is what the
// daemon says, and the reason is what the board page draws.
#[tokio::test]
async fn a_non_zero_exit_is_the_reason_it_gave() {
    let complains = cliban("complains");
    assert_eq!(
        complains.run(&[]).await,
        Err("error: no such project: NOPE".to_string())
    );

    // Nothing on stderr: the exit code is all there is to say.
    let silent = cliban("fails-silently");
    assert_eq!(silent.run(&[]).await, Err("cliban exited 3".to_string()));

    // Killed rather than exited, so there is no code to name.
    let killed = cliban("kills-itself");
    let why = killed
        .run(&[])
        .await
        .expect_err("a killed cliban is a failure");
    assert!(why.contains("killed"), "{why}");
}

// AYEAYE-53 — "a missing or failing board tool degrades to an empty board with
// a stated reason". A program that is not there is the missing half, and the
// reason has to name what was looked for or it explains nothing.
#[tokio::test]
async fn a_program_that_is_not_there_states_that_rather_than_panicking() {
    let absent = Cliban::new("/nonexistent/cargo/bin/cliban".to_string());
    let why = absent
        .run(&[])
        .await
        .expect_err("a missing cliban is a failure");
    assert!(why.contains("/nonexistent/cargo/bin/cliban"), "{why}");
    assert!(!why.is_empty());
}

// AYEAYE-53 — the daemon gives cliban 15 seconds and no longer. A request that
// waited on a wedged subprocess forever is a panel that never answers, which is
// the broken panel this ticket exists to avoid.
#[tokio::test]
async fn a_hang_becomes_a_stated_reason_and_the_child_does_not_outlive_it() {
    let mut hangs = cliban("hangs");
    hangs.timeout = Duration::from_millis(100);

    let why = hangs.run(&[]).await.expect_err("a hang is a failure");
    assert!(why.contains("did not answer"), "{why}");

    // And the child went with the request that abandoned it: were it still
    // running, it would leave its mark behind well before this returns.
    tokio::time::sleep(Duration::from_millis(2_500)).await;
    assert!(
        !marker().exists(),
        "the timed-out child outlived the request and finished anyway"
    );
}
