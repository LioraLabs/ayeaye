//! The binary's argument handling, run as the binary.
//!
//! `main` is thin on purpose, but "thin" is not "free": it owns the exit codes
//! a service manager reads, and nothing else in the suite starts the process.

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

// AYEAYE-57 — the capability report names what this run actually got, and says
// why when that is not what the build asked for.
//
// The shape is asserted against a selection made in the test rather than
// against the literal `cpu`, so it holds on every row of the release matrix.
// The second line cannot be produced on a machine with no card — a build with
// no acceleration in it has nothing to fall back from — so what runs here is
// the negative half: exactly one line, and it names the backend in use.
#[test]
fn the_banner_reports_the_acceleration_this_run_actually_got() {
    let selection = ayeaye_infer::backend::select();

    let (code, out, _) = ayeaye(&["--version"]);

    assert_eq!(code, 0);
    assert!(
        out.contains(&format!("({})", selection.got().label())),
        "the banner should name the backend in use ({}): {out:?}",
        selection.got().label()
    );
    match &selection.fallback {
        None => assert_eq!(
            out.lines().count(),
            1,
            "nothing was given up, so there is nothing to explain: {out:?}"
        ),
        Some(why) => assert!(
            out.contains(why.as_str()),
            "the reason has to reach the person reading it: {out:?}"
        ),
    }
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
