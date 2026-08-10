//! The binary: argument parsing, and the things it can be asked to do.
//!
//! Everything of substance lives in the library beside this file, so the
//! server can be driven by an integration test over a real socket rather than
//! only by starting the process and hoping.

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use ayeaye::config::{self, Settings};
use ayeaye::models;
use ayeaye::probe;
use ayeaye::server;
use ayeaye::service::{Runner, Services, Subprocess};
use ayeaye::setup;
use ayeaye_core::service::{Definition, Layout, manual_instructions};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("serve") => serve(&args[1..]),
        Some("service") => service_verb(args.get(1).map(String::as_str)),
        Some("model") => model_verb(&args[1..]),
        Some("setup") => setup_verb(&args[1..]),
        Some("check") => check_verb(),
        None => {
            println!("{}", banner());
            ExitCode::SUCCESS
        }
        Some("--version" | "-V") => {
            println!("{}", banner());
            ExitCode::SUCCESS
        }
        Some("--help" | "-h") => {
            println!("{}", USAGE);
            ExitCode::SUCCESS
        }
        Some(other) => {
            eprintln!("ayeaye: unknown command {other:?}\n\n{USAGE}");
            ExitCode::FAILURE
        }
    }
}

const USAGE: &str = "\
usage: ayeaye [serve [--bind ADDR] [--port N]]
       ayeaye setup [--yes] [--no-service] [--no-model] [--model ID]
                    [--bind ADDR] [--port N]
       ayeaye check
       ayeaye service <install|repair|enable|disable|start|stop|status|remove>
       ayeaye model <ls|pull ID|use ID|rm ID>

  serve      run the HTTP server
  setup      make this computer ready to run ayeaye, and check that it is
  check      re-run the health checks on their own
  service    manage the service this binary installs for itself
  model      fetch and choose the models this binary runs
  --version  print the version and what this build can do
  --help     this

setup asks before two things and nothing else: downloading a model, which
goes to the internet, and starting a service, which runs whenever you log
in. --yes answers both. With no terminal it does neither and prints the
command that would. Everything else it finds — how you reach this machine
from outside, a reverse proxy, a mesh network, your coding agents, tmux —
it checks and reports, and never configures.

a model ID is owner/name, as in openai/whisper-small.en, optionally
@revision. ayeaye ships the inference, not the weights.

environment (AYEAYE_*, or the legacy VOICE_REMOTE_*):
  AYEAYE_BIND           address to bind (default 127.0.0.1)
  AYEAYE_DEV_PORT       port to bind (default 8912)
  AYEAYE_ALLOWED_HOSTS  comma-separated extra Host values to answer to
  AYEAYE_TOKEN          the shared secret; otherwise read from the state file
  AYEAYE_CLIBAN         the cliban the board tab reads (legacy VOICE_CLIBAN);
                        otherwise the first on PATH, else ~/.cargo/bin/cliban
  AYEAYE_SPEECH_MODEL   which model transcribes; `ayeaye model use` writes it
  AYEAYE_CLEANUP_PROMPT what the cleanup model is told it is for
  AYEAYE_MODEL_IDLE     how long a model stays resident idle (default 5m, 0 keeps it)
  AYEAYE_MODEL_HUB      where models are fetched from";

/// One line naming the version and the capabilities compiled in.
fn banner() -> String {
    let backend = ayeaye_infer::backend::selected();
    ayeaye_core::Identity {
        version: ayeaye_core::VERSION,
        capabilities: &[backend.label()],
    }
    .banner()
}

fn serve(args: &[String]) -> ExitCode {
    let token = match config::load_token() {
        Ok(token) => token,
        Err(why) => {
            eprintln!("ayeaye: {why}");
            return ExitCode::FAILURE;
        }
    };
    // Resolved before the arguments are parsed for the same reason the token
    // is: both are the machine's answer rather than the caller's, and neither
    // should depend on which flags were typed.
    let cliban = ayeaye::cliban::Cliban::new(config::locate_cliban());
    let settings =
        match Settings::resolve(args, config::env_var, token, nodename(&Subprocess), cliban) {
            Ok(settings) => settings,
            Err(why) => {
                eprintln!("ayeaye: {why}\n\n{USAGE}");
                return ExitCode::FAILURE;
            }
        };

    // The runtime is built here rather than with `#[tokio::main]` so that the
    // banner and the argument errors above cost nothing to reach: they are the
    // paths a misconfigured service unit takes, and they should not need a
    // thread pool to print one line.
    let runtime = match tokio::runtime::Runtime::new() {
        Ok(runtime) => runtime,
        Err(why) => {
            eprintln!("ayeaye: could not start the async runtime: {why}");
            return ExitCode::FAILURE;
        }
    };
    runtime.block_on(async move {
        let listener = match server::listen(&settings).await {
            Ok(listener) => listener,
            Err(why) => {
                eprintln!("ayeaye: cannot bind {}: {why}", settings.address());
                return ExitCode::FAILURE;
            }
        };
        eprintln!(
            "{} on {} · token auth on, browsers log in once via /?token=<token>",
            banner(),
            settings.address()
        );
        // Before the first request, and only here: a window a previous process
        // left at phone width goes back now, and the file it was recorded in is
        // consumed so a later start cannot restore it a second time.
        settings.fits.recover(&settings.tmux, &settings.peers).await;
        if let Err(why) = server::serve(listener, Arc::new(settings)).await {
            eprintln!("ayeaye: server stopped: {why}");
            return ExitCode::FAILURE;
        }
        ExitCode::SUCCESS
    })
}

/// `ayeaye model <verb>` — the models this binary runs.
///
/// ayeaye ships the inference and not the weights, so this is how weights get
/// onto a machine: a repository id in, a directory under the state directory
/// out, with the architecture checked before anything large is fetched.
fn model_verb(args: &[String]) -> ExitCode {
    let Some(store) = config::state_dir() else {
        return complain(
            "cannot tell where this machine keeps its state: set HOME or XDG_STATE_HOME",
        );
    };
    let config_file = PathBuf::from(layout(from_environment).env_file);

    // Resolved before the verb runs, so a configuration file with a typo in it
    // is one error rather than a different one per subcommand.
    let settings = match models::settings(&config_file) {
        Ok(settings) => settings,
        Err(why) => return complain(&format!("ayeaye: {why}")),
    };

    let id = |given: Option<&String>| match given {
        Some(given) => ayeaye_core::model::ModelId::parse(given).map_err(|why| why.to_string()),
        None => Err("which model? give it as owner/name".to_string()),
    };

    match args.first().map(String::as_str) {
        Some("ls") => {
            let installed = models::installed(&store);
            if installed.is_empty() {
                println!("no models yet — `ayeaye model pull openai/whisper-small.en` gets one");
                return ExitCode::SUCCESS;
            }
            let chosen = settings.speech.as_ref().map(ToString::to_string);
            for model in installed {
                // The chosen one is marked rather than listed separately: the
                // question somebody runs this to answer is usually "is the one
                // I configured actually here".
                let mark = if chosen.as_deref() == Some(&model.to_string()) {
                    " (in use)"
                } else {
                    ""
                };
                println!("{model}{mark}");
            }
            ExitCode::SUCCESS
        }
        Some("pull") => match id(args.get(1)) {
            Err(why) => complain(&format!("ayeaye: {why}")),
            Ok(id) => {
                eprintln!("fetching {id} from {}", settings.hub);
                match models::pull(&models::Curl, &store, &settings.hub, &id) {
                    Ok(pulled) => {
                        println!(
                            "{} is in {} ({}, {})",
                            pulled.id,
                            pulled.dir.display(),
                            pulled.architecture.hf_name(),
                            human(pulled.bytes)
                        );
                        ExitCode::SUCCESS
                    }
                    Err(why) => complain(&format!("ayeaye: {why}")),
                }
            }
        },
        Some("use") => match id(args.get(1)) {
            Err(why) => complain(&format!("ayeaye: {why}")),
            Ok(id) => {
                let key = ayeaye_core::model::settings::SPEECH_MODEL;
                if let Err(why) = models::choose(&config_file, key, &id.to_string()) {
                    return complain(&format!("ayeaye: {why}"));
                }
                println!("{} is the model to transcribe with", id);
                println!("  written to {}", config_file.display());
                // Said rather than refused: choosing a model before fetching it
                // is a reasonable order to do things in, and refusing it would
                // make the two commands care about each other for no reason.
                if !models::installed(&store).contains(&id) {
                    println!("  it is not on this machine yet — `ayeaye model pull {id}`");
                }
                ExitCode::SUCCESS
            }
        },
        Some("rm") => match id(args.get(1)) {
            Err(why) => complain(&format!("ayeaye: {why}")),
            Ok(id) => match models::remove(&store, &id) {
                Ok(true) => {
                    println!("removed {id}");
                    ExitCode::SUCCESS
                }
                Ok(false) => {
                    println!("there was no {id} to remove");
                    ExitCode::SUCCESS
                }
                Err(why) => complain(&format!("ayeaye: {why}")),
            },
        },
        _ => complain("usage: ayeaye model <ls|pull ID|use ID|rm ID>"),
    }
}

/// A byte count somebody can read at a glance.
fn human(bytes: u64) -> String {
    const STEP: u64 = 1024;
    for (limit, unit) in [(STEP.pow(3), "GB"), (STEP.pow(2), "MB"), (STEP, "kB")] {
        if bytes >= limit {
            return format!("{:.1} {unit}", bytes as f64 / limit as f64);
        }
    }
    format!("{bytes} bytes")
}

/// `ayeaye setup` — make this computer ready, and then check that it is.
///
/// The one command somebody with the binary and nothing else runs. It decides
/// before it does anything, says what it is about to do, asks about the two acts
/// with a consequence, carries the rest out, and finishes by checking what it
/// just built.
///
/// It ends on the health report rather than on "done", and that is the point of
/// the whole ticket: a run that installed everything perfectly and cannot reach
/// the result is not a successful run, and the only way to find that out is to
/// ask.
fn setup_verb(args: &[String]) -> ExitCode {
    let flags = match setup::parse(args) {
        Ok(flags) => flags,
        Err(why) => return complain(&format!("ayeaye: {why}\n\n{USAGE}")),
    };
    let layout = layout(from_environment);
    let Some(state_dir) = config::state_dir() else {
        return complain(
            "ayeaye: cannot tell where this machine keeps its state: set HOME or XDG_STATE_HOME",
        );
    };
    let places = setup::Places::from(&layout, &state_dir);
    let program = match std::env::current_exe() {
        Ok(path) => path.to_string_lossy().into_owned(),
        Err(error) => {
            return complain(&format!(
                "ayeaye: cannot tell where this binary is: {error}"
            ));
        }
    };

    println!("looking at this computer…");
    let captured = probe::capture(&probe::System, &places.model_store.to_string_lossy());
    let machine = captured.machine();
    println!("  {}", machine.summary());
    println!("  {}", machine.verdict.tier.as_str());
    if let Some(reason) = machine.verdict.reason {
        println!("  {reason}");
    }
    let session = probe::session(&probe::System);

    let run = setup::Run {
        captured: &captured,
        places: &places,
        session: session.as_ref(),
        layout: &layout,
        program: &program,
        flags: &flags,
    };

    // Asked only where there is somebody to ask. A pipe is not a person, and
    // taking silence for consent is the one thing this must never do.
    let plan = if std::io::IsTerminal::is_terminal(&std::io::stdin()) {
        setup::decide(&run, &setup::Tty)
    } else {
        setup::decide(&run, &setup::Assumed(false))
    };

    if let Err(why) = setup::record_consent(&places.state_dir, &plan, &stamp()) {
        return complain(&format!("ayeaye: {why}"));
    }

    let did = match setup::carry_out(&plan, &run, Subprocess, &models::Curl, &stamp()) {
        Ok(did) => did,
        Err(why) => return complain(&format!("ayeaye: {why}")),
    };
    for line in &did.lines {
        println!("  {line}");
    }
    if !did.declined.is_empty() {
        println!("\nnot done, because you did not ask for it:");
        for line in &did.declined {
            println!("  {line}");
        }
    }

    println!();
    check(&captured, session.as_ref(), &places)
}

/// `ayeaye check` — the health checks, on their own.
///
/// The same report `setup` ends on. Separate because the answer changes without
/// setup running again: a certificate expires, a proxy is reconfigured, somebody
/// stops the service. "Is this still working" is a question worth being able to
/// ask on its own.
fn check_verb() -> ExitCode {
    let layout = layout(from_environment);
    let Some(state_dir) = config::state_dir() else {
        return complain(
            "ayeaye: cannot tell where this machine keeps its state: set HOME or XDG_STATE_HOME",
        );
    };
    let places = setup::Places::from(&layout, &state_dir);
    let captured = probe::capture(&probe::System, &places.model_store.to_string_lossy());
    check(&captured, probe::session(&probe::System).as_ref(), &places)
}

/// Run the checks and print them, four marks and all.
///
/// The exit code is the report's outcome, and the three are not two. A failed or
/// unfinished check leaves work outstanding and exits 1; the lock being off
/// exits 2, because "anybody who can reach this address can run commands on this
/// computer" is not the same news as "your https certificate has expired" and a
/// script reading exit codes should not have to guess which happened.
fn check(
    captured: &ayeaye::probe::Captured,
    session: Option<&ayeaye_core::service::Session>,
    places: &setup::Places,
) -> ExitCode {
    let asking = ayeaye::health::Asking {
        url: format!(
            "http://{}:{}",
            config::env_var("BIND").unwrap_or_else(|| config::DEFAULT_BIND.to_string()),
            config::env_var("DEV_PORT").unwrap_or_else(|| config::DEFAULT_DEV_PORT.to_string())
        ),
        allowed_hosts: config::env_var("ALLOWED_HOSTS")
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|host| !host.is_empty())
            .map(str::to_string)
            .collect(),
        token: config::load_token().ok(),
        state_dir: Some(places.state_dir.clone()),
        captured,
        session,
        build: setup::build_acceleration(),
        runner: Subprocess,
    };

    println!("checking what is here. each line is one of:");
    println!("  ok       it works             FAILED   it does not");
    println!("  skipped  you did not ask      unknown  could not tell\n");
    let report = asking.run();
    for check in &report.checks {
        println!("{}", check.line());
        if let Some(detail) = &check.detail {
            println!("         {detail}");
        }
    }
    println!("\n{}", report.summary());

    match report.outcome() {
        ayeaye_core::health::Outcome::Done => ExitCode::SUCCESS,
        ayeaye_core::health::Outcome::Unfinished => ExitCode::FAILURE,
        ayeaye_core::health::Outcome::Insecure => {
            eprintln!();
            for line in ayeaye_core::health::insecure_warning(&asking.url) {
                eprintln!("{line}");
            }
            ExitCode::from(2)
        }
    }
}

/// `ayeaye service <verb>` — the service this binary installs for itself.
///
/// `ayeaye setup` is AYEAYE-62's and will drive the same verbs; this is the
/// door that proves they work and gives somebody a way to repair an install by
/// hand without one.
fn service_verb(verb: Option<&str>) -> ExitCode {
    let layout = layout(from_environment);
    let program = match std::env::current_exe() {
        Ok(path) => path.to_string_lossy().into_owned(),
        Err(error) => return complain(&format!("cannot tell where this binary is: {error}")),
    };
    // The third answer, and the whole of what AYEAYE-61 recorded as owed here:
    // a machine with neither service manager. Asked before anything is rendered
    // or run, because every verb below would otherwise exec a `systemctl` that
    // is not there.
    let Some(session) = probe::session(&probe::System) else {
        return no_service_manager(verb, &program, &layout.env_file);
    };
    let services = Services {
        session,
        layout,
        runner: Subprocess,
    };
    let definition = Definition::ayeaye(&program);
    let name = definition.name.clone();

    let said = match verb {
        Some("install" | "repair") => services.repair(&definition, &stamp()).map(|repaired| {
            let mut said = format!("{}", repaired.installed.path.display());
            if let Some(backup) = repaired.installed.backup {
                said.push_str(&format!(
                    "\nkept a copy of what was there at {}",
                    backup.display()
                ));
            }
            if repaired.restarted {
                said.push_str(
                    "\nit was already running, so it has been restarted on the new definition",
                );
            }
            said
        }),
        Some("enable") => services
            .enable(&name)
            .map(|()| format!("{name} will start when you log in")),
        Some("disable") => services
            .disable(&name)
            .map(|()| format!("{name} will not start again")),
        Some("start") => services.start(&name).map(|()| format!("{name} started")),
        Some("stop") => services.stop(&name).map(|()| format!("{name} stopped")),
        // Reported as the manager reported it, exit code included: `systemctl
        // status` exits 3 for a unit that is simply stopped, and answering
        // "the command failed" to a question about a stopped service is not an
        // answer at all.
        Some("status") => match services.status(&name) {
            Ok(outcome) => {
                println!("{}", outcome.output.trim_end());
                return if outcome.ok {
                    ExitCode::SUCCESS
                } else {
                    ExitCode::FAILURE
                };
            }
            Err(why) => Err(why),
        },
        Some("remove") => services
            .remove(&name, &stamp())
            .map(|removed| match removed {
                Some(removed) => format!(
                    "removed {}\nkept a copy at {}",
                    removed.path.display(),
                    removed.backup.display()
                ),
                None => format!("there was no {name} definition to remove"),
            }),
        _ => {
            return complain(
                "usage: ayeaye service <install|repair|enable|disable|start|stop|status|remove>",
            );
        }
    };

    match said {
        Ok(said) => {
            println!("{}", said.trim_end());
            ExitCode::SUCCESS
        }
        Err(why) => complain(&why.to_string()),
    }
}

fn complain(why: &str) -> ExitCode {
    eprintln!("{why}");
    ExitCode::FAILURE
}

/// Where this machine keeps things, from the environment the shell hands over.
///
/// The lookup is an argument so the defaulting can be tested without a test
/// mutating the process environment, which in this edition is `unsafe` and, with
/// tests running in threads, wrong as well.
fn layout(look_up: impl Fn(&str) -> Option<String>) -> Layout {
    let home = look_up("HOME").unwrap_or_default();
    let config_home = look_up("XDG_CONFIG_HOME").unwrap_or_else(|| format!("{home}/.config"));
    let state_home = look_up("XDG_STATE_HOME").unwrap_or_else(|| format!("{home}/.local/state"));
    Layout::new(&home, &config_home, &state_home)
}

/// The process environment, as [`layout`] wants to ask about it. An empty
/// value is not an answer: `HOME=` would put a unit at `/.config`.
fn from_environment(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.is_empty())
}

/// What this machine calls itself, from `uname -n`.
///
/// Asked through the same [`Runner`] the launchd uid is asked through, rather
/// than through the timeout helper the tmux calls use: this happens once,
/// before the runtime exists, with nothing to interleave with. `None` when
/// `uname` is not there or says nothing, which leaves the name to fall through
/// to a default rather than to the empty string.
fn nodename(runner: &impl Runner) -> Option<String> {
    let asked = runner.run(&["uname".to_string(), "-n".to_string()]);
    asked
        .ok
        .then(|| asked.output.trim().to_string())
        .filter(|name| !name.is_empty())
}

/// What to say on a machine with nowhere to install a service.
///
/// Not an apology and not a failure. `lib/steps/70-service.sh` calls this the
/// documented manual mode and returns a *finished* run from it: ayeaye works
/// perfectly well started by hand, and a container, a stripped-down Linux or a
/// Mac without launchctl is a machine where saying so plainly is the whole of
/// the right answer.
///
/// So `install` and `repair` — the two verbs setup itself drives — succeed,
/// because there is nothing left for them to do. The six that address a manager
/// fail, because they were asked to change or report the state of a service and
/// no such thing happened. Reporting either as the other would be the lie this
/// path exists to avoid.
fn no_service_manager(verb: Option<&str>, program: &str, env_file: &str) -> ExitCode {
    let Some((said, finished)) = without_a_service_manager(verb, program, env_file) else {
        return complain(
            "usage: ayeaye service <install|repair|enable|disable|start|stop|status|remove>",
        );
    };
    if finished {
        println!("{said}");
        ExitCode::SUCCESS
    } else {
        complain(&said)
    }
}

/// What that verb comes to on such a machine, and whether it is a finished run.
///
/// Split out from the printing so the exit codes can be asserted, which is the
/// only part of this a service manager or a script reads.
fn without_a_service_manager(
    verb: Option<&str>,
    program: &str,
    env_file: &str,
) -> Option<(String, bool)> {
    let (kind, finished) = match verb? {
        "install" | "repair" => ("install a service into", true),
        "enable" | "disable" | "start" | "stop" | "status" | "remove" => {
            ("start services for you from", false)
        }
        _ => return None,
    };
    Some((
        format!(
            "this computer has no user service manager, so there is nothing to {kind}.\n{}",
            manual_instructions(program, env_file).join("\n")
        ),
        finished,
    ))
}

/// What a kept copy of a replaced definition is named after.
///
/// Seconds since the epoch rather than a readable date: turning one into the
/// other needs a calendar, and a calendar is a dependency the core's allowlist
/// would have to admit for the sake of a backup filename.
fn stamp() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_secs().to_string())
        .unwrap_or_else(|_| "backup".to_string())
}

#[cfg(test)]
mod tests {
    use super::{layout, nodename, stamp, without_a_service_manager};
    use ayeaye::service::{Outcome, Runner};
    use std::cell::RefCell;

    /// A stand-in for `id`.
    struct Answers {
        outcome: Outcome,
        asked: RefCell<Vec<Vec<String>>>,
    }

    impl Answers {
        fn with(ok: bool, output: &str) -> Self {
            Answers {
                outcome: Outcome {
                    ok,
                    output: output.to_string(),
                },
                asked: RefCell::new(Vec::new()),
            }
        }
    }

    impl Runner for Answers {
        fn run(&self, argv: &[String]) -> Outcome {
            self.asked.borrow_mut().push(argv.to_vec());
            self.outcome.clone()
        }
    }

    // The three tests that stood here — a Mac addressed by the uid `id` reports,
    // a Mac whose `id` will not answer, and everything else being systemd — moved
    // to `probe::tests` with the detector they were about. AYEAYE-62 replaced
    // main's provisional `session()` with AYEAYE-60's detector, which can also
    // answer that a machine has *neither* manager, and the tests followed it.

    // AYEAYE-43 — what this machine calls itself is `uname -n`, which is what
    // the daemon reads it from, and it is asked through the same runner the
    // launchd uid is asked through. A machine that will not answer has no name
    // here rather than an empty one: the empty string is not a host name and
    // the defaulting above it is what turns "no answer" into a usable name.
    #[test]
    fn this_machine_is_named_by_uname_n_and_nothing_is_not_a_name() {
        let runner = Answers::with(true, "desktop\n");
        assert_eq!(nodename(&runner), Some("desktop".to_string()));
        assert_eq!(
            runner.asked.borrow().as_slice(),
            [vec!["uname".to_string(), "-n".to_string()]]
        );
        assert_eq!(nodename(&Answers::with(false, "desktop")), None);
        assert_eq!(nodename(&Answers::with(true, " \n")), None);
    }

    // AYEAYE-61 — the XDG defaults, which decide where a unit lands and which
    // settings file it names.
    #[test]
    fn the_xdg_variables_win_and_their_defaults_hang_off_home() {
        let bare = layout(|name| (name == "HOME").then(|| "/home/tester".to_string()));
        assert_eq!(bare.env_file, "/home/tester/.config/ayeaye/env");
        assert_eq!(bare.unit_dir, "/home/tester/.config/systemd/user");
        assert_eq!(bare.state_home, "/home/tester/.local/state");
        assert_eq!(bare.agent_dir, "/home/tester/Library/LaunchAgents");

        let moved = layout(|name| {
            Some(
                match name {
                    "HOME" => "/home/tester",
                    "XDG_CONFIG_HOME" => "/elsewhere/config",
                    _ => "/elsewhere/state",
                }
                .to_string(),
            )
        });
        assert_eq!(moved.env_file, "/elsewhere/config/ayeaye/env");
        assert_eq!(moved.unit_dir, "/elsewhere/config/systemd/user");
        assert_eq!(moved.state_home, "/elsewhere/state");
    }

    // AYEAYE-62 — the third answer, at the door. `install` and `repair` are the
    // verbs setup itself drives, and on a machine with nowhere to install a
    // service they are *finished*: lib/steps/70-service.sh returns SKIP here and
    // ends the run successfully, because running ayeaye by hand is a supported
    // way to use it and not a fault to come back and fix. The six that address a
    // manager did not happen, so they say so and fail.
    #[test]
    fn with_no_manager_installing_is_finished_and_starting_is_not() {
        for verb in ["install", "repair"] {
            let (said, finished) =
                without_a_service_manager(Some(verb), "/opt/ayeaye", "/conf/env").expect("a verb");
            assert!(finished, "{verb} has nothing left to do");
            assert!(said.contains("run the server with: /opt/ayeaye"), "{said}");
            assert!(said.contains("/conf/env"), "{said}");
        }
        for verb in ["enable", "disable", "start", "stop", "status", "remove"] {
            let (said, finished) =
                without_a_service_manager(Some(verb), "/opt/ayeaye", "/conf/env").expect("a verb");
            assert!(!finished, "{verb} was asked for and did not happen");
            assert!(said.contains("no user service manager"), "{said}");
            // Still told what to do instead — the point of the path is the
            // instructions, not the refusal.
            assert!(said.contains("run the server with:"), "{said}");
        }
        assert_eq!(
            without_a_service_manager(Some("polish"), "/opt/ayeaye", "/conf/env"),
            None,
            "a verb that is not a verb is a usage error, not a fact about this machine"
        );
        assert_eq!(
            without_a_service_manager(None, "/opt/ayeaye", "/conf/env"),
            None
        );
    }

    // AYEAYE-61 — a stamp that is empty, or that a filename cannot hold, would
    // take the naming of a kept copy with it.
    #[test]
    fn a_stamp_is_something_a_backup_can_be_named_after() {
        let stamp = stamp();
        assert!(!stamp.is_empty());
        assert!(
            stamp.chars().all(|c| c.is_ascii_digit()),
            "a stamp goes in a filename: {stamp}"
        );
    }
}
