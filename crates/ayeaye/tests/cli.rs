//! The binary's argument handling, run as the binary.
//!
//! `main` is thin on purpose, but "thin" is not "free": it owns the exit codes
//! a service manager reads, and nothing else in the suite starts the process.

use std::path::{Path, PathBuf};
use std::process::Command;

fn ayeaye(args: &[&str]) -> (i32, String, String) {
    // A token is supplied rather than removed. `serve` loads the token before
    // it parses arguments, so on a machine with no token file every `serve`
    // assertion below would otherwise be answered by the "no token" branch and
    // prove nothing about argument handling — passing for the wrong reason on
    // one machine and the right one on another.
    let output = Command::new(env!("CARGO_BIN_EXE_ayeaye"))
        .args(args)
        .env("AYEAYE_TOKEN", "test-token-not-a-real-secret")
        .env_remove("VOICE_REMOTE_TOKEN")
        .output()
        .expect("the binary should be runnable");
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

// AYEAYE-42 — with no arguments the binary still says what it is, which is
// what it did before this ticket and what a `--version`-style probe expects.
#[test]
fn the_bare_binary_prints_the_banner_and_succeeds() {
    let (code, out, _) = ayeaye(&[]);
    assert_eq!(code, 0);
    assert!(out.starts_with("ayeaye "), "banner was {out:?}");
    assert!(
        out.contains(env!("CARGO_PKG_VERSION")),
        "banner was {out:?}"
    );

    let (code, versioned, _) = ayeaye(&["--version"]);
    assert_eq!(code, 0);
    assert_eq!(versioned, out, "--version should be the same banner");
}

// AYEAYE-42 — an unrecognised command fails rather than starting something.
// A service unit with a typo in it should stop, not silently serve.
#[test]
fn an_unknown_command_fails_and_says_so() {
    let (code, _, err) = ayeaye(&["srve"]);
    assert_eq!(code, 1);
    assert!(err.contains("srve"), "stderr was {err:?}");
    assert!(err.contains("usage:"), "stderr was {err:?}");
}

// AYEAYE-42 — a bad argument to `serve` is refused before anything binds, and
// the message names the flag rather than a stack trace.
#[test]
fn a_bad_serve_argument_is_refused_before_binding() {
    let (code, _, err) = ayeaye(&["serve", "--prot", "9000"]);
    assert_eq!(code, 1);
    assert!(err.contains("--prot"), "stderr was {err:?}");
    assert!(err.contains("usage:"), "stderr was {err:?}");
    assert!(
        !err.contains("panicked"),
        "a bad flag must not panic: {err:?}"
    );
}

// AYEAYE-42 — the help text names every environment variable the server reads,
// so the one place a person looks is not missing the one they need.
#[test]
fn help_names_every_variable_the_server_reads() {
    let (code, out, _) = ayeaye(&["--help"]);
    assert_eq!(code, 0);
    for name in [
        "AYEAYE_BIND",
        "AYEAYE_DEV_PORT",
        "AYEAYE_ALLOWED_HOSTS",
        "AYEAYE_TOKEN",
    ] {
        assert!(out.contains(name), "--help does not mention {name}");
    }
}

/// The binary, run as a process on a machine that has nothing on its `PATH`.
///
/// **This is what makes the test below safe to run on a developer's computer.**
/// An empty `PATH` means `probe::session` cannot find `systemctl` or
/// `launchctl`, so it never asks the user bus anything and never runs a service
/// verb against it. AYEAYE-61 disabled this machine's own ayeaye service by
/// forgetting that `systemctl --user` addresses the live session whatever `HOME`
/// says; a redirected `HOME` is not a sandbox, and an empty `PATH` is the thing
/// that actually stops a command being found at all.
fn ayeaye_on_a_bare_machine(args: &[&str], home: &Path) -> (i32, String, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_ayeaye"))
        .args(args)
        .env("PATH", "")
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", home.join("config"))
        .env("XDG_STATE_HOME", home.join("state"))
        .env("AYEAYE_TOKEN", "test-token-not-a-real-secret")
        .env_remove("VOICE_REMOTE_TOKEN")
        .output()
        .expect("the binary should be runnable");
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

// AYEAYE-62 — the third answer, as the binary. AYEAYE-61 recorded that a machine
// with neither service manager got a `systemctl` that is not there; this is the
// door that proves it no longer does. `install` and `repair` are *finished*
// runs, because `lib/steps/70-service.sh` returns SKIP here and ends the run
// successfully: ayeaye started by hand is a supported way to use it.
#[test]
fn a_machine_with_no_service_manager_is_told_how_to_run_it_by_hand() {
    let home = scratch("no-manager");
    for verb in ["install", "repair"] {
        let (code, out, err) = ayeaye_on_a_bare_machine(&["service", verb], &home);
        assert_eq!(code, 0, "{verb} left nothing to do: {out}{err}");
        assert!(out.contains("no user service manager"), "{out}");
        assert!(out.contains("run the server with:"), "{out}");
        assert!(out.contains("by itself"), "{out}");
        assert!(
            out.contains("its log is whatever the terminal"),
            "the three sentences of the manual path, whole: {out}"
        );
        assert!(
            !err.contains("systemctl") && !err.contains("No such file"),
            "nothing should have tried to run a service manager: {err}"
        );
    }

    // The six that address a manager were asked for and did not happen, so they
    // fail — and still say what to do instead.
    for verb in ["enable", "disable", "start", "stop", "status", "remove"] {
        let (code, _, err) = ayeaye_on_a_bare_machine(&["service", verb], &home);
        assert_eq!(code, 1, "{verb} did not happen");
        assert!(err.contains("no user service manager"), "{err}");
        assert!(err.contains("run the server with:"), "{err}");
    }

    // Nothing was written anywhere on any of those paths.
    assert!(
        !home.join("config").exists() && !home.join("state").exists(),
        "the manual path writes nothing and runs nothing"
    );
}

/// A directory of this test's own, emptied first so a rerun starts clean.
fn scratch(what: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ayeaye-cli-{what}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    dir
}
