//! The binary's argument handling, run as the binary.
//!
//! `main` is thin on purpose, but "thin" is not "free": it owns the exit codes
//! a service manager reads, and nothing else in the suite starts the process.

use std::process::Command;

fn ayeaye(args: &[&str]) -> (i32, String, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_ayeaye"))
        .args(args)
        // A stray AYEAYE_* in the developer's shell must not change what these
        // assertions see.
        .env_remove("AYEAYE_TOKEN")
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
    assert!(
        err.contains("--prot") || err.contains("no token"),
        "stderr was {err:?}"
    );
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
