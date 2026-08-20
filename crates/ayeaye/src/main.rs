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
use ayeaye_core::model::settings;
use ayeaye_core::service::{Definition, Layout, manual_instructions};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("serve") => serve(&args[1..]),
        Some("service") => service_verb(args.get(1).map(String::as_str)),
        Some("model") => model_verb(&args[1..]),
        Some("dictate") => dictate_verb(&args[1..]),
        Some("setup") => setup_verb(&args[1..]),
        Some("check") => check_verb(),
        None => report(),
        Some("--version" | "-V") => report(),
        Some("--help" | "-h") => {
            println!("{USAGE}");
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
       ayeaye setup [--yes] [--no-service] [--no-model]
                    [--bind ADDR] [--port N]
       ayeaye check
       ayeaye service <install|repair|enable|disable|start|stop|status|remove>
       ayeaye model <ls|choose [speech|cleanup]|use [speech|cleanup] NAME>
       ayeaye dictate <pane> [client-pid]

  serve      run the HTTP server
  setup      make this computer ready to run ayeaye, and check that it is
  check      re-run the health checks on their own
  service    manage the service this binary installs for itself
  model      choose which of the backend's models plays which part
  dictate    toggle dictation for one pane; bind it to a key in tmux
  --version  print the version
  --help     this

setup asks before one thing and nothing else: starting a service, which
runs whenever you log in. --yes answers it. With no terminal it does not,
and prints the command that would. Everything else it finds — how you
reach this machine from outside, a reverse proxy, a mesh network, your
coding agents, tmux — it checks and reports, and never configures.

ayeaye does not download, store or run models. speech and cleanup happen
in a llama-swap you run yourself, and a model name here is the key it goes
by in that config. --no-model skips choosing them.

environment (AYEAYE_*, or the legacy VOICE_REMOTE_*):
  AYEAYE_BIND           address to bind (default 127.0.0.1)
  AYEAYE_PORT           port to bind (default 8911)
  AYEAYE_ALLOWED_HOSTS  comma-separated extra Host values to answer to
  AYEAYE_TOKEN          the shared secret; otherwise read from the state file
  AYEAYE_CLIBAN         the cliban the board tab reads (legacy VOICE_CLIBAN);
                        otherwise the first on PATH, else ~/.cargo/bin/cliban
  AYEAYE_SPEECH_MODEL   which model transcribes; `ayeaye model use speech` writes it
  AYEAYE_CLEANUP_MODEL  which model cleans transcripts; `ayeaye model use cleanup` writes it
  AYEAYE_CLEANUP_PROMPT what the cleanup model is told it is for
  AYEAYE_LLAMA_SWAP     the backend serving both models (default http://127.0.0.1:8080)
  AYEAYE_PROJECT_ROOTS  colon-separated roots the picker walks (default ~)
  AYEAYE_PROJECT_DEPTH  how far below a root a project may sit (default 6)
  AYEAYE_PROJECT_BUDGET seconds one walk may spend (default 4)
  AYEAYE_PROJECT_WAIT   seconds a request waits for a walk (default 0.4)
  AYEAYE_PROJECT_TTL    seconds a finished search stays usable (default 60)
  AYEAYE_PROJECT_SKIP   extra directory names never walked, comma-separated
  AYEAYE_NOTIFY_EVERY   seconds between notification checks (default 10)
  AYEAYE_NOTIFY_STATES  states that notify (default blocked,waiting)
  VOICE_PORT            the port the recording agent listens on (default 8787)";

/// The one line a `--version`-style probe reads.
///
/// The capability list is where the acceleration used to be named, and it is now
/// empty on purpose rather than by omission: this binary runs no model, so it has
/// no acceleration to claim. What device transcription runs on is a fact about
/// llama-swap's process, and `ayeaye check` asks the thing that knows.
fn banner() -> String {
    ayeaye_core::Identity {
        version: ayeaye_core::VERSION,
        capabilities: &[],
    }
    .banner()
}

/// Say what this build is.
///
/// The banner goes to stdout and stays **one line**, because that is what a
/// `--version`-style probe reads and a second line would be a parsing change
/// that only appears on the artifacts hardest to test.
fn report() -> ExitCode {
    println!("{}", banner());
    ExitCode::SUCCESS
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
    // Voice is a progressive enhancement: a machine whose backend is not running
    // serves everything else, and the probe is what tells the page so. A
    // configuration file it cannot read is a different matter — that is a typo
    // somebody has to be told about rather than a feature to switch off.
    let config_file = PathBuf::from(layout(from_environment).env_file);
    let models = match models::settings(&config_file) {
        Ok(models) => models,
        Err(why) => {
            eprintln!("ayeaye: {why}");
            return ExitCode::FAILURE;
        }
    };
    let policy = match models::cleanup_policy(&config_file) {
        Ok(policy) => policy,
        Err(why) => {
            eprintln!("ayeaye: {why}");
            return ExitCode::FAILURE;
        }
    };
    // The backend address is read once, here, and refused loudly: a daemon that
    // fell back to a default because somebody typed `https://` would look
    // configured and fail at the first dictation.
    let backend = match ayeaye::swap::Swap::at(&models.backend) {
        Ok(backend) => backend,
        Err(why) => return complain(&format!("ayeaye: AYEAYE_LLAMA_SWAP: {why}")),
    };
    let voice = Arc::new(ayeaye::dictate::Voice::new(
        models,
        policy,
        ayeaye::audio::CONVERTER.to_string(),
        backend,
    ));
    // The environment first, then the settings file `ayeaye setup` owns, then
    // the defaults. The file layer is what `manual_instructions` promises with
    // "it reads the settings file by itself": the systemd unit injects the same
    // file into the environment, but launchd and a hand-started server inject
    // nothing, and on those paths a daemon reading only the environment would
    // ignore what setup just wrote.
    let settings = match Settings::resolve(
        args,
        config::env_then_file(&config_file),
        token,
        nodename(&Subprocess),
        cliban,
        voice,
    ) {
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
        let settings = Arc::new(settings);
        let notifications = ayeaye::notify::Config::resolve(|name| std::env::var(name).ok());
        eprintln!("ayeaye: {}", notifications.describe());
        ayeaye::notify::watcher(Arc::clone(&settings), notifications);
        if let Err(why) = server::serve(listener, settings).await {
            eprintln!("ayeaye: server stopped: {why}");
            return ExitCode::FAILURE;
        }
        ExitCode::SUCCESS
    })
}

/// `ayeaye model <verb>` — which of the backend's models plays which part.
///
/// Three verbs where there were seven. `search`, `pull`, `add` and `rm` all
/// existed because ayeaye kept a model store of its own, and AYEAYE-101 gave
/// that job to `llama-swap`: models arrive by being written into its config,
/// which is a file this binary has no business editing.
fn model_verb(args: &[String]) -> ExitCode {
    let config_file = PathBuf::from(layout(from_environment).env_file);

    // Resolved before the verb runs, so a configuration file with a typo in it
    // is one error rather than a different one per subcommand.
    let settings = match models::settings(&config_file) {
        Ok(settings) => settings,
        Err(why) => return complain(&format!("ayeaye: {why}")),
    };
    let backend = match ayeaye::swap::Swap::at(&settings.backend) {
        Ok(backend) => backend,
        Err(why) => return complain(&format!("ayeaye: AYEAYE_LLAMA_SWAP: {why}")),
    };
    let policy = match models::cleanup_policy(&config_file) {
        Ok(policy) => policy,
        Err(why) => return complain(&format!("ayeaye: {why}")),
    };

    match args.first().map(String::as_str) {
        Some("ls") => {
            let mut smoke = match models::RealSmoke::new(&backend, policy) {
                Ok(smoke) => smoke,
                Err(why) => return complain(&format!("ayeaye: {why}")),
            };
            let served = match models::Smoke::models(&mut smoke) {
                Ok(served) => served,
                Err(why) => {
                    return complain(&format!(
                        "ayeaye: could not ask {} what it serves: {why}",
                        backend.base()
                    ));
                }
            };
            if served.is_empty() {
                println!("{} is serving no models", backend.base());
                return ExitCode::SUCCESS;
            }
            for name in served {
                // The mark says which of these two names is in the config file,
                // which is the one question `ls` is actually asked.
                let mut roles = Vec::new();
                if settings.speech.as_deref() == Some(name.as_str()) {
                    roles.push("speech");
                }
                if settings.cleanup.as_deref() == Some(name.as_str()) {
                    roles.push("cleanup");
                }
                if roles.is_empty() {
                    println!("{name}");
                } else {
                    println!("{name}  <- {}", roles.join(", "));
                }
            }
            ExitCode::SUCCESS
        }
        Some("choose") => {
            let role = match args.get(1).map(String::as_str) {
                None => None,
                Some("speech") => Some(ayeaye_core::model::Role::Speech),
                Some("cleanup") => Some(ayeaye_core::model::Role::Cleanup),
                Some(_) => return complain("usage: ayeaye model choose [speech|cleanup]"),
            };
            if !std::io::IsTerminal::is_terminal(&std::io::stdin()) {
                return complain(
                    "ayeaye: model choose needs a terminal; script it with `ayeaye model ls` and `ayeaye model use speech|cleanup NAME`",
                );
            }
            let mut smoke = match models::RealSmoke::new(&backend, policy) {
                Ok(smoke) => smoke,
                Err(why) => return complain(&format!("ayeaye: {why}")),
            };
            match models::choose_interactive(&mut smoke, &setup::Tty, &config_file, role) {
                Err(why) => complain(&format!("ayeaye: {why}; previous configuration is intact")),
                Ok(result) => {
                    for selected in result.selected {
                        println!(
                            "{} selected; smoke output: {}",
                            selected.name,
                            selected.smoke.trim()
                        );
                    }
                    if result.cleanup_declined {
                        println!("cleanup left unset; dictation will use raw transcripts");
                    }
                    ExitCode::SUCCESS
                }
            }
        }
        Some("use") => {
            let (role, given) = match args.get(1).map(String::as_str) {
                Some("speech") => (ayeaye_core::model::Role::Speech, args.get(2)),
                Some("cleanup") => (ayeaye_core::model::Role::Cleanup, args.get(2)),
                _ => (ayeaye_core::model::Role::Speech, args.get(1)),
            };
            let Some(name) = given
                .map(String::as_str)
                .map(str::trim)
                .filter(|n| !n.is_empty())
            else {
                return complain(
                    "ayeaye: which model? give the name it has in llama-swap's config",
                );
            };
            // Warned about, not refused. The proxy may be stopped right now, and
            // a machine being configured before its backend is started is an
            // ordinary order to do things in — but a name that is a typo is
            // worth catching here rather than at somebody's first dictation.
            let mut smoke = match models::RealSmoke::new(&backend, policy) {
                Ok(smoke) => smoke,
                Err(why) => return complain(&format!("ayeaye: {why}")),
            };
            match models::Smoke::models(&mut smoke) {
                Ok(served) if !served.iter().any(|had| had == name) => {
                    eprintln!(
                        "ayeaye: warning: {} is not serving {name} — it serves {}",
                        backend.base(),
                        if served.is_empty() {
                            "nothing".to_string()
                        } else {
                            served.join(", ")
                        }
                    );
                }
                Ok(_) => {}
                Err(why) => eprintln!(
                    "ayeaye: warning: could not ask {} what it serves: {why}",
                    backend.base()
                ),
            }
            if let Err(why) = models::choose(&config_file, models::key_for(role), name) {
                return complain(&format!("ayeaye: {why}"));
            }
            match role {
                ayeaye_core::model::Role::Speech => {
                    println!("{name} is the model to transcribe with")
                }
                ayeaye_core::model::Role::Cleanup => println!("{name} is the cleanup model"),
            }
            println!("  written to {}", config_file.display());
            ExitCode::SUCCESS
        }
        _ => complain("usage: ayeaye model <ls|choose [speech|cleanup]|use [speech|cleanup] NAME>"),
    }
}

/// `ayeaye dictate <pane> [client-pid]` — one press of the dictation key.
///
/// Bound to a key in tmux, which is why it says everything on the status line
/// and nothing on stdout: `run-shell -b` discards both streams, so a message
/// written here would be a key that silently did nothing.
///
/// **The pane id must be qualified** — `desktop/%3`, like every id in this app —
/// and the host half has to be the name this machine goes by, because the pane
/// is looked up in the list under that name. A bare `#{pane_id}` is refused:
/// `PaneId::parse` has nothing to split, so there is no host to compare.
///
/// A tmux binding therefore spells it `#{host}/#{pane_id}`, and that only agrees
/// with this binary while nobody has set `AYEAYE_NAME` — tmux's `#{host}` is
/// `gethostname()`, and `AYEAYE_NAME` overrides it. A machine that sets it has
/// to write the same name into the binding, or every dictation is refused with
/// "is not a pane on this machine". `ayeaye::config::machine_name` is the one
/// rule; there is nothing that can reconcile it with a string in somebody's
/// `tmux.conf`, so it is said here instead.
fn dictate_verb(args: &[String]) -> ExitCode {
    let Some(pane) = args.first() else {
        return complain("usage: ayeaye dictate <pane> [client-pid]");
    };
    let Some(store) = config::state_dir() else {
        return complain(
            "cannot tell where this machine keeps its state: set HOME or XDG_STATE_HOME",
        );
    };
    let token = match std::fs::read_to_string(agent_token_path()) {
        Ok(token) => token.trim().to_string(),
        Err(why) => {
            return complain(&format!(
                "no recorder token at {}: {why}",
                agent_token_path().display()
            ));
        }
    };
    let config_file = PathBuf::from(layout(from_environment).env_file);
    let models = match models::settings(&config_file) {
        Ok(models) => models,
        Err(why) => return complain(&format!("ayeaye: {why}")),
    };
    let policy = match models::cleanup_policy(&config_file) {
        Ok(policy) => policy,
        Err(why) => return complain(&format!("ayeaye: {why}")),
    };
    let backend = match ayeaye::swap::Swap::at(&models.backend) {
        Ok(backend) => backend,
        Err(why) => return complain(&format!("ayeaye: AYEAYE_LLAMA_SWAP: {why}")),
    };

    // What this machine calls itself, in the same order `Settings::resolve`
    // reads it — the two have to agree, or a pane id written by the daemon is
    // one this toggle cannot find in its own pane list.
    // The same function the daemon names itself with, not a second spelling of
    // the same rule: the ids in the state file were qualified by the daemon, and
    // a toggle that disagreed about this name could not find any of them.
    let named = ayeaye::config::machine_name(config::env_var, nodename(&Subprocess));
    let here = match ayeaye_core::peer::HostName::new(&named) {
        Ok(here) => here,
        Err(why) => {
            return complain(&format!(
                "ayeaye: {named:?} cannot be a machine name: {why:?}"
            ));
        }
    };

    let runtime = match tokio::runtime::Runtime::new() {
        Ok(runtime) => runtime,
        Err(why) => return complain(&format!("ayeaye: could not start the async runtime: {why}")),
    };
    runtime.block_on(async move {
        let tmux = ayeaye::tmux::Tmux::new();
        let voice = ayeaye::dictate::Voice::new(
            models,
            policy,
            ayeaye::audio::CONVERTER.to_string(),
            backend,
        );
        let state = store.join(ayeaye::dictate::STATE_FILE);
        let toggle = ayeaye::dictate::Toggle {
            tmux: &tmux,
            here,
            voice: &voice,
            state: &state,
            token,
            port: agent_port(),
        };
        // Only asked on the way *in*. The second press talks to the machine the
        // first one recorded, which is what the state file is for: a client that
        // has since detached must not send the audio somewhere else.
        let peer = match ayeaye::dictate::read_state(&state) {
            Some(_) => None,
            None => recording_peer(&tmux, pane, args.get(1).map(String::as_str)).await,
        };

        let said = toggle.press(pane, peer.as_deref()).await;
        if said.is_empty() {
            return ExitCode::SUCCESS;
        }
        // The status line, because there is no window to use and nothing reads
        // this process's streams.
        //
        // `#` is doubled because `display-message` expands `#{...}` formats, and
        // this message can carry a converter's own stderr. tmux's escape for a
        // literal `#` is `##`, so this is the one place the text is quoted for
        // the thing that will interpret it — the same care `send-keys -l --`
        // takes next door, for the same reason.
        let _ = tmux
            .ask(&["display-message", &said.replace('#', "##")])
            .await;
        eprintln!("{said}");
        ExitCode::FAILURE
    })
}

/// The address the tmux client is connected from, or `None` if it is local.
///
/// This half is the asking: which clients the pane's session has. The deciding
/// is `ayeaye::dictate::recording_peer`, which takes the process backend as an
/// argument and is where the rule is written down and tested.
async fn recording_peer(
    tmux: &ayeaye::tmux::Tmux,
    pane: &str,
    passed: Option<&str>,
) -> Option<String> {
    // The methods below belong to `ayeaye::process::Processes`, and the trait is
    // not imported: `here()` hands back a `dyn Processes`, whose methods are
    // callable without it.
    let processes = ayeaye::process::here();
    let local = ayeaye_core::peer::PaneId::parse(pane)
        .map(|id| id.pane().to_string())
        .unwrap_or_else(|_| pane.to_string());

    let session = tmux
        .ask(&["display-message", "-p", "-t", &local, "#{session_name}"])
        .await
        .unwrap_or_default();
    let clients = tmux
        .ask(&["list-clients", "-t", session.trim(), "-F", "#{client_pid}"])
        .await
        .unwrap_or_default();
    let clients: Vec<u32> = clients
        .split_whitespace()
        .filter_map(|pid| pid.parse().ok())
        .collect();

    ayeaye::dictate::recording_peer(processes.as_ref(), passed, &clients)
}

/// Where the shared secret the recording agent checks lives.
///
/// `bin/voice-dictate`'s path, and deliberately the same file: the agent on the
/// phone is the Python one and stays that way, so the secret has to be the one
/// it was set up with.
fn agent_token_path() -> PathBuf {
    let base = from_environment("XDG_CONFIG_HOME")
        .unwrap_or_else(|| format!("{}/.config", from_environment("HOME").unwrap_or_default()));
    PathBuf::from(base).join("voice-dictate/token")
}

/// The port the recording agent listens on.
///
/// `VOICE_PORT`, plainly, because that is the name `bin/voice-agent` reads on
/// the phone and the two have to agree. Deliberately *not* through
/// `config::env_var`, which would also answer to `AYEAYE_VOICE_PORT` and
/// `VOICE_REMOTE_VOICE_PORT` — doubly-prefixed names that exist nowhere else,
/// are in no documentation, and would each be a second place this setting could
/// come from.
fn agent_port() -> u16 {
    from_environment("VOICE_PORT")
        .and_then(|port| port.trim().parse().ok())
        .unwrap_or(ayeaye::recorder::DEFAULT_PORT)
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
    let captured = probe::capture(&probe::System);
    println!("  {}", captured.machine().summary());
    // Out of the capture that already happened, rather than asking this machine
    // the same three questions a second time — and so setup cannot end up with
    // two ideas of what it is running on.
    let session = captured.session();

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

    let cleanup = match models::cleanup_policy(&places.config_file) {
        Ok(cleanup) => cleanup,
        Err(why) => return complain(&format!("ayeaye: {why}")),
    };
    let settings = match models::settings(&places.config_file) {
        Ok(settings) => settings,
        Err(why) => return complain(&format!("ayeaye: {why}")),
    };
    let backend = match ayeaye::swap::Swap::at(&settings.backend) {
        Ok(backend) => backend,
        Err(why) => return complain(&format!("ayeaye: AYEAYE_LLAMA_SWAP: {why}")),
    };
    let mut smoke = match models::RealSmoke::new(&backend, cleanup) {
        Ok(smoke) => smoke,
        Err(why) => return complain(&format!("ayeaye: {why}")),
    };
    let carried = if std::io::IsTerminal::is_terminal(&std::io::stdin()) {
        setup::carry_out(&plan, &run, Subprocess, &mut smoke, &setup::Tty, &stamp())
    } else {
        setup::carry_out(
            &plan,
            &run,
            Subprocess,
            &mut smoke,
            &setup::Assumed(flags.yes),
            &stamp(),
        )
    };
    let did = match carried {
        Ok(did) => did,
        Err(why) => return complain(&format!("ayeaye: {why}")),
    };
    for line in &did.lines {
        println!("  {line}");
    }
    if !did.declined.is_empty() {
        // "not done", not "not done because you did not ask for it". Since
        // AYEAYE-101 this list holds two kinds of line: a step declined for
        // want of consent, and one that ran and could not finish — a backend
        // that is not up is the common case, and telling somebody they declined
        // it would be a sentence about a choice they never made.
        println!("\nnot done:");
        for line in &did.declined {
            println!("  {line}");
        }
    }

    println!();
    // `setup` reports what is unfinished and still *finishes*. The shell's health
    // step answers PENDING for everything it can be unhappy about, and a pending
    // step never stops a run: a machine that needs tmux installed, or that has no
    // curl, is a machine setup did its job on. Only the lock being off is a
    // failure, and it is the same failure here as in `check`.
    match check(&captured, session.as_ref(), &places) {
        Report::Insecure => ExitCode::from(2),
        Report::Unfinished | Report::Fine => ExitCode::SUCCESS,
    }
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
    let captured = probe::capture(&probe::System);
    // Asked as a question, so the exit code is the answer: 0 for a machine with
    // nothing outstanding, 1 for one that has something, 2 for the lock being
    // off. `setup` treats the middle one differently on purpose — see there.
    match check(&captured, probe::session(&probe::System).as_ref(), &places) {
        Report::Fine => ExitCode::SUCCESS,
        Report::Unfinished => ExitCode::FAILURE,
        Report::Insecure => ExitCode::from(2),
    }
}

/// How a health run came out, for the two callers that read it differently.
enum Report {
    /// Everything asked for was checked and works.
    Fine,
    /// Something did not work, or could not be checked.
    Unfinished,
    /// The lock is off.
    Insecure,
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
) -> Report {
    let bind = setup::effective(&places.config_file, "BIND", config::DEFAULT_BIND);
    let asking = ayeaye::health::Asking {
        // Read the way the daemon reads it: the environment first, then the
        // file setup wrote, then the default — the same layers `serve` hands
        // `Settings::resolve`, so the check and the daemon cannot disagree
        // about which address is even being discussed.
        url: format!(
            "http://{}:{}",
            bind,
            setup::effective(
                &places.config_file,
                "PORT",
                &config::DEFAULT_PORT.to_string()
            )
        ),
        allowed_hosts: setup::effective(&places.config_file, "ALLOWED_HOSTS", "")
            .split(',')
            .map(str::trim)
            .filter(|host| !host.is_empty())
            .map(str::to_string)
            .collect(),
        // Anything but loopback is this machine on a network, which is the fact
        // the front-end checks are about. Detected, and never configured.
        loopback_only: bind == config::DEFAULT_BIND || bind == "localhost" || bind == "::1",
        token: config::load_token().ok(),
        state_dir: Some(places.state_dir.clone()),
        captured,
        session,
        // Read the same way, from the same file, so the check asks the address
        // the daemon will use rather than a second guess at it.
        backend: setup::effective(
            &places.config_file,
            settings::LLAMA_SWAP,
            settings::DEFAULT_BACKEND,
        )
        .trim_end_matches('/')
        .to_string(),
        // The core's own names for these, not a second spelling of them: a
        // check reading `SPEECH_MODEL` while the daemon reads something else
        // would report on a setting nobody set.
        wanted: [
            setup::effective(&places.config_file, settings::SPEECH_MODEL, ""),
            setup::effective(&places.config_file, settings::CLEANUP_MODEL, ""),
        ]
        .into_iter()
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty())
        .collect(),
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
        ayeaye_core::health::Outcome::Done => Report::Fine,
        ayeaye_core::health::Outcome::Unfinished => Report::Unfinished,
        ayeaye_core::health::Outcome::Insecure => {
            eprintln!();
            for line in ayeaye_core::health::insecure_warning(&asking.url) {
                eprintln!("{line}");
            }
            Report::Insecure
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
/// Not an apology and not a failure. This is the documented manual mode:
/// ayeaye works
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
    // service they are *finished*, because running ayeaye by hand is a supported
    // way to use it and not a fault to come back and fix. The six that address a
    // manager did not happen, so they say so and fail.
    #[test]
    fn with_no_manager_installing_is_finished_and_starting_is_not() {
        for verb in ["install", "repair"] {
            let (said, finished) =
                without_a_service_manager(Some(verb), "/opt/ayeaye", "/conf/env").expect("a verb");
            assert!(finished, "{verb} has nothing left to do");
            assert!(
                said.contains("run the server with: /opt/ayeaye serve"),
                "{said}"
            );
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
