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
    use super::{layout, session, stamp};
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
