//! The binary: argument parsing, and the things it can be asked to do.
//!
//! Everything of substance lives in the library beside this file, so the
//! server can be driven by an integration test over a real socket rather than
//! only by starting the process and hoping.

mod service;

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use ayeaye::config::{self, Settings};
use ayeaye::models;
use ayeaye::server;
use ayeaye_core::service::{DEFAULT_LAUNCHD_PREFIX, Definition, Layout, Manager, Session};
use service::{Runner, Services, Subprocess};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("serve") => serve(&args[1..]),
        Some("service") => service_verb(args.get(1).map(String::as_str)),
        Some("model") => model_verb(&args[1..]),
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
       ayeaye service <install|repair|enable|disable|start|stop|status|remove>
       ayeaye model <ls|pull ID|use ID|rm ID>

  serve      run the HTTP server
  service    manage the service this binary installs for itself
  model      fetch and choose the models this binary runs
  --version  print the version and what this build can do
  --help     this

a model ID is owner/name, as in openai/whisper-small.en, optionally
@revision. ayeaye ships the inference, not the weights.

environment (AYEAYE_*, or the legacy VOICE_REMOTE_*):
  AYEAYE_BIND           address to bind (default 127.0.0.1)
  AYEAYE_DEV_PORT       port to bind (default 8912)
  AYEAYE_ALLOWED_HOSTS  comma-separated extra Host values to answer to
  AYEAYE_TOKEN          the shared secret; otherwise read from the state file
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
    let settings = match Settings::resolve(args, config::env_var, token, nodename(&Subprocess)) {
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

/// `ayeaye service <verb>` — the service this binary installs for itself.
///
/// `ayeaye setup` is AYEAYE-62's and will drive the same verbs; this is the
/// door that proves they work and gives somebody a way to repair an install by
/// hand without one.
fn service_verb(verb: Option<&str>) -> ExitCode {
    let services = Services {
        session: session(&Subprocess, cfg!(target_os = "macos")),
        layout: layout(from_environment),
        runner: Subprocess,
    };
    let program = match std::env::current_exe() {
        Ok(path) => path.to_string_lossy().into_owned(),
        Err(error) => return complain(&format!("cannot tell where this binary is: {error}")),
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

/// Which service manager to talk to.
///
/// Provisional, and deliberately the smallest thing that is not a guess:
/// AYEAYE-60 detects the user session properly, including the answer this
/// cannot give — that a machine has *neither* manager, which a container and a
/// stripped-down Linux both really are. AYEAYE-62 is where the two meet, and
/// this should be replaced by that detector there rather than grown here.
///
/// Which platform this is arrives as an argument rather than as a `cfg!` read
/// inside, so that both answers can be asked for on one machine. A branch that
/// only exists on a Mac is a branch nobody here can test.
fn session(runner: &impl Runner, macos: bool) -> Session {
    if !macos {
        return Session::systemd();
    }
    // Every launchd command addresses a domain by uid, and `id` is where the
    // shell has always read it from.
    let asked = runner.run(&["id".to_string(), "-u".to_string()]);
    Session {
        manager: Manager::Launchd,
        uid: asked
            .ok
            .then(|| asked.output.trim().to_string())
            .filter(|uid| !uid.is_empty()),
        launchd_prefix: DEFAULT_LAUNCHD_PREFIX.to_string(),
    }
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
    use super::{layout, nodename, session, stamp};
    use crate::service::{Outcome, Runner};
    use ayeaye_core::service::Manager;
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

    // AYEAYE-61 — every launchd command addresses a domain by uid, and `id` is
    // where the shell has always read it from.
    #[test]
    fn a_mac_is_launchd_addressed_by_the_uid_id_reports() {
        let runner = Answers::with(true, "501\n");
        let session = session(&runner, true);
        assert_eq!(session.manager, Manager::Launchd);
        assert_eq!(session.uid.as_deref(), Some("501"));
        assert_eq!(
            runner.asked.borrow().as_slice(),
            [vec!["id".to_string(), "-u".to_string()]]
        );
    }

    // AYEAYE-61 — without a uid the target would be the malformed `gui//label`.
    // No uid is better than a wrong one; the core refuses on it rather than
    // addressing nothing.
    #[test]
    fn a_mac_with_no_answer_from_id_carries_no_uid() {
        assert_eq!(session(&Answers::with(false, ""), true).uid, None);
        assert_eq!(session(&Answers::with(true, " \n"), true).uid, None);
    }

    // AYEAYE-61 — and nothing is asked of `id` where nothing addresses a domain.
    #[test]
    fn anything_else_is_systemd_and_asks_nothing() {
        let runner = Answers::with(true, "1000\n");
        let session = session(&runner, false);
        assert_eq!(session.manager, Manager::Systemd);
        assert!(runner.asked.borrow().is_empty());
    }

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
