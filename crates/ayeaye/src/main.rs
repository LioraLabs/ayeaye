//! The binary: the only crate allowed to touch the world.
//!
//! Subprocesses, the filesystem, sockets, and model lifetime live here or
//! below in `ayeaye-infer`. Anything that is a decision rather than an effect
//! belongs in `ayeaye-core`, where a test can reach it without a machine.

mod service;

use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use ayeaye_core::service::{DEFAULT_LAUNCHD_PREFIX, Definition, Layout, Manager, Session};
use service::{Runner, Services, Subprocess};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("service") => service_verb(args.get(1).map(String::as_str)),
        _ => {
            let backend = ayeaye_infer::backend::selected();
            let identity = ayeaye_core::Identity {
                version: ayeaye_core::VERSION,
                capabilities: &[backend.label()],
            };
            println!("{}", identity.banner());
            ExitCode::SUCCESS
        }
    }
}

/// `ayeaye service <verb>` — the service this binary installs for itself.
///
/// `ayeaye setup` is AYEAYE-62's and will drive the same verbs; this is the
/// door that proves they work and gives somebody a way to repair an install by
/// hand without one.
fn service_verb(verb: Option<&str>) -> ExitCode {
    let services = Services {
        session: session(&Subprocess),
        layout: layout(),
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
fn layout() -> Layout {
    let home = std::env::var("HOME").unwrap_or_default();
    let config_home =
        std::env::var("XDG_CONFIG_HOME").unwrap_or_else(|_| format!("{home}/.config"));
    let state_home =
        std::env::var("XDG_STATE_HOME").unwrap_or_else(|_| format!("{home}/.local/state"));
    Layout::new(&home, &config_home, &state_home)
}

/// Which service manager to talk to.
///
/// Provisional, and deliberately the smallest thing that is not a guess:
/// AYEAYE-60 detects the user session properly, including the answer this
/// cannot give — that a machine has *neither* manager, which a container and a
/// stripped-down Linux both really are. AYEAYE-62 is where the two meet, and
/// this should be replaced by that detector there rather than grown here.
fn session(runner: &impl Runner) -> Session {
    if !cfg!(target_os = "macos") {
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
