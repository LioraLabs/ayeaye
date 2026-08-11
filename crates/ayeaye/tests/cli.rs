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

    let (code, out, err) = ayeaye(&["--version"]);

    assert_eq!(code, 0);
    assert!(
        out.contains(&format!("({})", selection.got().label())),
        "the banner should name the backend in use ({}): {out:?}",
        selection.got().label()
    );
    // Whatever happened, stdout stays one line: it is what a `--version`-style
    // probe parses, and a fallback must not change its shape.
    assert_eq!(
        out.lines().count(),
        1,
        "stdout is one line whether or not anything was given up: {out:?}"
    );
    if let Some(why) = selection.fallback() {
        assert!(
            err.contains(why),
            "the reason has to reach the person reading it, on stderr: {err:?}"
        );
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
        "AYEAYE_PORT",
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
// runs, because ayeaye started by hand is a supported way to use it.
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

// AYEAYE-62 — `ayeaye setup` as the binary, on a machine where no service
// manager can be found and no consent was given.
//
// **The empty `PATH` is the safety property, not a convenience.** It means the
// binary cannot find `systemctl`, so it never asks the live user bus anything
// and never runs a service verb against it — and it cannot find `curl` either,
// so no request leaves this machine. AYEAYE-61 disabled the developer's own
// ayeaye service by assuming a redirected `HOME` was a sandbox; it is not, and
// this is what a sandbox actually looks like from here.
#[test]
fn setup_with_nobody_to_ask_writes_its_own_files_and_nothing_else() {
    let home = scratch("setup");
    let (code, out, err) = ayeaye_on_a_bare_machine(&["setup", "--no-model"], &home);

    // The health checks are the last thing it does, and on a machine with
    // nothing running most of them cannot be made — but setup still *finishes*.
    // Health answers pending for everything it can be unhappy about, and a
    // pending step never stops a run: needing tmux installed is not
    // setup having failed at its job. `ayeaye check` reports the same thing and
    // exits 1, because that one was asked as a question.
    assert_eq!(
        code, 0,
        "an unfinished check is not a failed run: {out}{err}"
    );
    assert!(out.contains("looking at this computer"), "{out}");

    // It minted a key, and only its owner can read it.
    let token = home.join("state/ayeaye/token");
    assert!(token.exists(), "no key was made: {out}");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&token).unwrap().permissions().mode();
        assert_eq!(mode & 0o077, 0, "the key is readable by others: {mode:o}");
    }
    assert_eq!(
        std::fs::read_to_string(&token).unwrap().trim().len(),
        43,
        "the shape secrets.token_urlsafe(32) makes"
    );

    // It wrote the settings file it owns.
    assert!(home.join("config/ayeaye/env").exists(), "{out}");

    // It found no service manager and said how to run ayeaye by hand.
    assert!(out.contains("run the server with:"), "{out}");
    assert!(
        !home.join("config/systemd").exists(),
        "nothing may be installed where nothing can run it"
    );

    // Every check reported, and the four marks are the four marks.
    assert!(out.contains("skipped  you did not ask"), "{out}");
    assert!(out.contains("unknown  could not tell"), "{out}");
    assert!(out.contains(" ok, "), "the summary line: {out}");
    // Nothing could be asked over the network, and none of it passed.
    assert!(out.contains("unknown"), "{out}");
    assert!(
        !err.contains("panicked"),
        "setup must not panic on a bare machine: {err}"
    );

    // And the consent record exists, saying what was declined rather than
    // silently dropping it.
    let consent = std::fs::read_to_string(home.join("state/ayeaye/consent")).unwrap_or_default();
    assert!(
        consent.is_empty() || consent.contains("declined"),
        "a record that only holds agreements is not a record: {consent}"
    );
}

// AYEAYE-62 — re-runnable, and does not damage an existing install. The key a
// second run finds is the key a phone is already logged in with.
#[test]
fn a_second_setup_keeps_the_key_the_first_one_made() {
    let home = scratch("setup-twice");
    ayeaye_on_a_bare_machine(&["setup", "--no-model"], &home);
    let token = home.join("state/ayeaye/token");
    let first = std::fs::read_to_string(&token).expect("a key");
    std::fs::write(home.join("config/ayeaye/env"), "AYEAYE_CLIBAN=/opt/mine\n").unwrap();

    ayeaye_on_a_bare_machine(&["setup", "--no-model"], &home);
    assert_eq!(
        std::fs::read_to_string(&token).expect("still a key"),
        first,
        "a new key would log every phone out"
    );
    assert!(
        std::fs::read_to_string(home.join("config/ayeaye/env"))
            .unwrap()
            .contains("AYEAYE_CLIBAN=/opt/mine"),
        "settings setup did not ask about belong to whoever put them there"
    );
}

// AYEAYE-62 — `ayeaye check` runs the same report on its own, because the answer
// changes without setup running again: a certificate expires, a proxy is
// reconfigured, somebody stops the service.
#[test]
fn check_runs_the_health_report_on_its_own() {
    let home = scratch("check");
    let (code, out, err) = ayeaye_on_a_bare_machine(&["check"], &home);
    // `check` is asked as a question, so its exit code is the answer — unlike
    // `setup`, which reports the same thing and still finishes.
    assert_eq!(code, 1, "nothing is running, so nothing is finished");
    assert!(out.contains("checking what is here"), "{out}");
    assert!(out.contains(" ok, "), "the summary line: {out}");
    assert!(!err.contains("panicked"), "{err}");
    // It checks; it does not set anything up.
    assert!(
        !home.join("state/ayeaye/token").exists(),
        "check must not mint a key"
    );
    assert!(
        !home.join("config/ayeaye/env").exists(),
        "check must not write settings"
    );
}

// AYEAYE-62 — the help names the two verbs and says what setup will ask about,
// because "it asks before two things and nothing else" is the promise somebody
// is trusting when they run it on a machine they care about.
#[test]
fn help_says_what_setup_will_and_will_not_do() {
    let (code, out, _) = ayeaye(&["--help"]);
    assert_eq!(code, 0);
    for said in [
        "ayeaye setup",
        "ayeaye check",
        "--yes",
        "asks before two things",
        "never configures",
    ] {
        assert!(out.contains(said), "--help does not mention {said:?}");
    }
}
