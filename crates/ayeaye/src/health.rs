//! Asking the questions [`ayeaye_core::health`] answers.
//!
//! The judgement is the core's; this makes the requests. Every one of them goes
//! through curl, which is AYEAYE-56's precedent and constitution rule 4's
//! consequence: no pure-Rust TLS client passes the dependency gate, and this
//! needs to reach an https front end somebody put in front of ayeaye.
//!
//! **A machine with no curl is not a machine that passed.** It is said once at
//! the top and every network check reports `unknown`, which is the truth — the
//! shell says the same, in the same place, for the same reason.
//!
//! # The key never touches a command line
//!
//! Two of the checks need the shared secret. It goes into a curl configuration
//! file written under a private umask and deleted the moment the request is
//! done, because a token in an argument is a token in the process list, in the
//! shell's history, and in whatever log somebody pastes when the check fails.

use std::io::Write;
use std::path::{Path, PathBuf};

use ayeaye_core::health::{self, Check, Report, Verdict};
use ayeaye_core::machine::Acceleration;
use ayeaye_core::service::{Manager, Operation, Session};

use crate::probe::Captured;
use crate::service::Runner;

/// How long any one request is given.
///
/// Short, and deliberately: everything asked about is on this computer or on
/// this computer's own network, and a health check that hangs for a minute per
/// capability is one nobody will ever wait for.
const TIMEOUT: &str = "5";

/// Everything a health run needs to know before it asks anything.
pub struct Asking<'a, R: Runner> {
    /// Where ayeaye is listening, as a URL with no trailing slash.
    pub url: String,
    /// The addresses ayeaye was configured to answer to.
    pub allowed_hosts: Vec<String>,
    /// The shared secret, where there is one to use.
    pub token: Option<String>,
    /// Where a temporary curl configuration may be written.
    pub state_dir: Option<PathBuf>,
    /// What this machine is.
    pub captured: &'a Captured,
    /// The session the service lives in, if this machine has one.
    pub session: Option<&'a Session>,
    /// What this build can run inference on.
    pub build: Acceleration,
    /// How commands are run.
    pub runner: R,
}

/// An address nobody would ever configure, used to prove ayeaye refuses one.
///
/// `.invalid` is reserved by RFC 2606 precisely so that it can never resolve, so
/// this cannot collide with a real deployment however unlucky somebody is.
const A_STRANGER: &str = "not-configured.invalid";

impl<R: Runner> Asking<'_, R> {
    /// Run every check, in the shell's order, and report.
    pub fn run(&self) -> Report {
        let mut report = Report::default();
        let curl = self.captured.has("curl");

        report.record(self.service_check());
        report.record(self.tmux_check());
        report.record(health::acceleration(self.build, &self.captured.machine()));

        if !curl {
            // Said once, at the top, rather than five times underneath.
            for (name, claim) in [
                ("local", format!("ayeaye answering on {}", self.url)),
                (
                    "auth",
                    "ayeaye refusing anyone without your key".to_string(),
                ),
                ("authorised", "your key opens the page".to_string()),
                (
                    "hosts",
                    "ayeaye accepting the address you named".to_string(),
                ),
                ("https", "an https address your phone can open".to_string()),
            ] {
                report.record(Check {
                    name,
                    claim,
                    verdict: Verdict::Unknown,
                    detail: Some("this computer has no curl, so this could not be asked".into()),
                });
            }
            report.record(self.agents_check());
            return report;
        }

        let local = self.code(&format!("{}/", self.url), &[]);
        report.record(Check {
            name: "local",
            claim: format!("ayeaye answers on {}", self.url),
            verdict: health::local(local),
            detail: local.map(|code| format!("it answered {code}")),
        });

        report.record_auth(
            "ayeaye refuses anyone without your key",
            self.code(&format!("{}/api/overview", self.url), &[]),
        );

        report.record(self.authorised_check());
        report.record(self.hosts_check());
        report.record(self.https_check());
        report.record(self.agents_check());
        report
    }

    /// Is there a service, and is it up?
    fn service_check(&self) -> Check {
        let Some(session) = self.session else {
            return Check {
                name: "service",
                claim: "ayeaye starting when you log in".to_string(),
                verdict: health::service(false, None),
                detail: Some("this computer has no user service manager".to_string()),
            };
        };
        let launchd = session.manager == Manager::Launchd;
        let asked = session
            .command("ayeaye", Operation::Status, None)
            .ok()
            .map(|argv| self.runner.run(&argv).ok);
        Check {
            name: "service",
            claim: health::service_claim(launchd).to_string(),
            verdict: health::service(true, asked),
            detail: None,
        }
    }

    /// The terminal multiplexer: verified, and never installed.
    fn tmux_check(&self) -> Check {
        let present = self.captured.has("tmux");
        Check {
            name: "tmux",
            claim: "tmux, which ayeaye reads your agents through".to_string(),
            verdict: health::tmux(present),
            detail: (!present).then(|| self.captured.install_hint(&["tmux"]).join("; ")),
        }
    }

    /// And does the key this machine holds actually open it?
    ///
    /// The complement of the assertion above, and not a duplicate of it. That
    /// one proves the lock is on; this proves it is *your* lock — that the key
    /// in the state file and the key the daemon is checking against are the same
    /// key. They come apart in exactly one situation and it is not rare: a token
    /// file rewritten while a daemon is already running, at which point the
    /// phone is logged in and setup would report a perfectly healthy install.
    fn authorised_check(&self) -> Check {
        let claim = "your key opens the page".to_string();
        let (Some(token), Some(state_dir)) = (self.token.as_deref(), self.state_dir.as_deref())
        else {
            return Check {
                name: "authorised",
                claim,
                verdict: Verdict::Unknown,
                detail: Some("there is no key on this computer to try".to_string()),
            };
        };
        let secret = match SecretConfig::write(state_dir, token) {
            Ok(secret) => secret,
            Err(why) => {
                return Check {
                    name: "authorised",
                    claim,
                    verdict: Verdict::Unknown,
                    detail: Some(why),
                };
            }
        };
        let code = self.code(
            &format!("{}/api/overview", self.url),
            &["-K", &secret.path().to_string_lossy()],
        );
        Check {
            name: "authorised",
            claim,
            // The same shape as `local`: a 2xx is the answer, anything else is
            // not, and no answer is never a pass.
            verdict: health::local(code),
            detail: code.map(|code| format!("it answered {code}")),
        }
    }

    /// Host validation, asserted in both directions.
    fn hosts_check(&self) -> Check {
        let claim = if self.allowed_hosts.is_empty() {
            "ayeaye accepting an address you named".to_string()
        } else {
            format!(
                "ayeaye accepts {} and refuses anything else",
                self.allowed_hosts.join(", ")
            )
        };
        let configured: Vec<Option<u16>> = self
            .allowed_hosts
            .iter()
            .map(|host| self.code(&format!("{}/", self.url), &["-H", &format!("Host: {host}")]))
            .collect();
        // Asked even when nothing is configured would be a request whose answer
        // is never read, so the stranger is only asked where the first half was.
        let stranger = (!configured.is_empty())
            .then(|| {
                self.code(
                    &format!("{}/", self.url),
                    &["-H", &format!("Host: {A_STRANGER}")],
                )
            })
            .flatten();
        let verdict = health::hosts(&configured, stranger);
        Check {
            name: "hosts",
            claim,
            verdict,
            detail: (verdict == Verdict::Failed).then(|| {
                format!("an address nobody configured answered {stranger:?}; yours answered {configured:?}")
            }),
        }
    }

    /// The https front end, whichever one is in place — detected and verified,
    /// never configured.
    fn https_check(&self) -> Check {
        let host = self.allowed_hosts.first();
        // "Was one asked for" is the presence of a configured address: nothing
        // else in this binary knows about a proxy or a mesh, and a machine that
        // named none is a machine that stayed on loopback by choice.
        let asked_for = !self.allowed_hosts.is_empty();
        let code = host.and_then(|host| self.code(&format!("https://{host}/"), &[]));
        Check {
            name: "https",
            claim: match host {
                Some(host) => format!("https://{host}/ answers, which is what your phone opens"),
                None => "an https address your phone can open".to_string(),
            },
            verdict: health::https(asked_for, host.map(String::as_str), code),
            detail: code.map(|code| format!("it answered {code}")),
        }
    }

    /// The coding agents: detected and verified, never installed.
    fn agents_check(&self) -> Check {
        let found: Vec<(&str, bool)> = ["claude", "codex"]
            .into_iter()
            .filter(|name| self.captured.has(name))
            .map(|name| (name, true))
            .collect();
        let verdict = health::agents(&found);
        Check {
            name: "agents",
            claim: if found.is_empty() {
                "a coding agent for ayeaye to show you".to_string()
            } else {
                format!(
                    "the coding agents on this computer: {}",
                    found
                        .iter()
                        .map(|(name, _)| *name)
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            },
            verdict,
            detail: None,
        }
    }

    /// The HTTP status of one request, or `None` when it could not be made.
    ///
    /// `--output /dev/null` and `--write-out '%{http_code}'` rather than
    /// `--fail`, because the status *is* the answer here: a 401 is what the
    /// authentication check is hoping for, and a flag that turned it into a
    /// non-zero exit would throw away the thing being measured.
    fn code(&self, url: &str, extra: &[&str]) -> Option<u16> {
        let mut argv: Vec<String> = [
            "curl",
            "--silent",
            "--show-error",
            "--output",
            "/dev/null",
            "--write-out",
            "%{http_code}",
            "--max-time",
            TIMEOUT,
        ]
        .iter()
        .map(|word| (*word).to_string())
        .collect();
        argv.extend(extra.iter().map(|word| (*word).to_string()));
        // The URL is data, and `--` is what makes "the last argument" mean it:
        // curl reads options at any position, so a host configured as `-o/tmp/x`
        // would otherwise arrive as a flag.
        argv.push("--".to_string());
        argv.push(url.to_string());

        let asked = self.runner.run(&argv);
        if !asked.ok {
            return None;
        }
        asked.output.trim().parse().ok().filter(|code| *code != 0)
    }
}

/// A curl configuration file holding the key, deleted when it goes out of scope.
///
/// The key never reaches a command line. `Drop` rather than a call at the end of
/// the request, so that a panic or an early return between writing it and using
/// it cannot leave a credential on disk — which is the failure the shell's own
/// tripwire watches for.
pub struct SecretConfig {
    path: PathBuf,
}

impl SecretConfig {
    /// Write one, or say why not.
    pub fn write(dir: &Path, token: &str) -> Result<SecretConfig, String> {
        std::fs::create_dir_all(dir).map_err(|why| format!("create {}: {why}", dir.display()))?;
        let path = dir.join("health-request");
        let _ = std::fs::remove_file(&path);
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(&path)
            .map_err(|why| format!("write {}: {why}", path.display()))?;
        writeln!(file, "header = \"X-Voice-Token: {token}\"")
            .map_err(|why| format!("write {}: {why}", path.display()))?;
        Ok(SecretConfig { path })
    }

    /// Where it is, for `curl -K`.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for SecretConfig {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::{Asking, SecretConfig};
    use crate::probe::{Captured, Sources, capture};
    use crate::service::{Outcome, Runner};
    use ayeaye_core::health::{Outcome as StepOutcome, Verdict};
    use ayeaye_core::machine::Acceleration;
    use ayeaye_core::service::Session;
    use std::cell::RefCell;
    use std::collections::HashMap;

    /// A machine that answers whatever it was told to answer.
    #[derive(Default)]
    struct Answers {
        /// Keyed on a substring of the joined argv.
        replies: Vec<(String, Outcome)>,
        ran: RefCell<Vec<Vec<String>>>,
    }

    impl Answers {
        fn to(mut self, matching: &str, ok: bool, output: &str) -> Self {
            self.replies.push((
                matching.to_string(),
                Outcome {
                    ok,
                    output: output.to_string(),
                },
            ));
            self
        }
    }

    impl Runner for Answers {
        fn run(&self, argv: &[String]) -> Outcome {
            self.ran.borrow_mut().push(argv.to_vec());
            let said = argv.join(" ");
            self.replies
                .iter()
                .find(|(matching, _)| said.contains(matching.as_str()))
                .map(|(_, outcome)| outcome.clone())
                .unwrap_or(Outcome {
                    ok: false,
                    output: "nothing answered".to_string(),
                })
        }
    }

    /// The probes of a plain Linux machine with the named commands on `PATH`.
    struct Bare {
        path: Vec<String>,
        files: HashMap<String, String>,
    }

    impl Sources for Bare {
        fn run(&self, argv: &[String]) -> Outcome {
            let ok = argv.first().map(String::as_str) == Some("uname");
            Outcome {
                ok,
                output: if ok {
                    "Linux\n".to_string()
                } else {
                    String::new()
                },
            }
        }
        fn read(&self, path: &str) -> Option<String> {
            self.files.get(path).cloned()
        }
        fn is_dir(&self, _path: &str) -> bool {
            false
        }
        fn is_executable(&self, _path: &str) -> bool {
            false
        }
        fn env(&self, _name: &str) -> Option<String> {
            None
        }
        fn which(&self, name: &str) -> Option<String> {
            self.path
                .iter()
                .any(|on| on == name)
                .then(|| format!("/usr/bin/{name}"))
        }
    }

    fn machine_with(commands: &[&str]) -> Captured {
        capture(
            &Bare {
                path: commands.iter().map(|name| (*name).to_string()).collect(),
                files: HashMap::new(),
            },
            "/tmp/models",
        )
    }

    fn asking<'a>(captured: &'a Captured, runner: Answers) -> Asking<'a, Answers> {
        Asking {
            url: "http://127.0.0.1:8912".to_string(),
            allowed_hosts: Vec::new(),
            token: None,
            state_dir: None,
            captured,
            session: None,
            build: Acceleration::Cpu,
            runner,
        }
    }

    fn verdict(report: &ayeaye_core::health::Report, name: &str) -> Verdict {
        report
            .checks
            .iter()
            .find(|check| check.name == name)
            .unwrap_or_else(|| panic!("no {name} check"))
            .verdict
    }

    // AYEAYE-62 — a machine with no curl is not a machine that passed. Every
    // network check reports unknown, said once at the top, which is the truth.
    #[test]
    fn a_machine_with_no_curl_reports_unknown_and_never_ok() {
        let captured = machine_with(&["tmux"]);
        let report = asking(&captured, Answers::default()).run();
        for name in ["local", "auth", "hosts", "https"] {
            assert_eq!(
                verdict(&report, name),
                Verdict::Unknown,
                "{name} could not be asked"
            );
        }
        assert_eq!(report.outcome(), StepOutcome::Unfinished);
        assert_eq!(
            verdict(&report, "tmux"),
            Verdict::Passed,
            "that one is a PATH lookup"
        );
    }

    // AYEAYE-62 — and nothing is *run* to discover that. A run with no curl must
    // not start a curl to be told there is no curl.
    #[test]
    fn with_no_curl_no_request_is_attempted() {
        let captured = machine_with(&["tmux"]);
        let asked = asking(&captured, Answers::default());
        let ran = {
            let report = asked.run();
            assert!(!report.checks.is_empty());
            asked.runner.ran.borrow().clone()
        };
        assert!(
            !ran.iter().any(|argv| argv[0] == "curl"),
            "it should not have tried: {ran:?}"
        );
    }

    // AYEAYE-62 — the one assertion, end to end. A 200 to a request carrying no
    // key means anybody who can reach that address can run commands on this
    // computer, and it is the only thing that fails the step outright.
    #[test]
    fn a_200_to_an_unauthenticated_request_fails_the_whole_step() {
        let captured = machine_with(&["curl", "tmux"]);
        let report = asking(
            &captured,
            Answers::default().to("/api/overview", true, "200").to(
                "http://127.0.0.1:8912/",
                true,
                "200",
            ),
        )
        .run();
        assert_eq!(verdict(&report, "auth"), Verdict::Failed);
        assert!(
            report.insecure,
            "the alarm and the verdict are one decision"
        );
        assert_eq!(report.outcome(), StepOutcome::Insecure);

        // And the locked machine, which is the same run with one status changed.
        let locked = asking(
            &captured,
            Answers::default().to("/api/overview", true, "401").to(
                "http://127.0.0.1:8912/",
                true,
                "200",
            ),
        )
        .run();
        assert_eq!(verdict(&locked, "auth"), Verdict::Passed);
        assert!(!locked.insecure);
        assert_ne!(locked.outcome(), StepOutcome::Insecure);
    }

    // AYEAYE-62 — the status is the answer, so the request must not be made with
    // a flag that turns a 401 into a failure. `--fail` here would throw away the
    // one thing being measured.
    #[test]
    fn a_status_asking_request_never_carries_fail() {
        let captured = machine_with(&["curl"]);
        let asked = asking(&captured, Answers::default().to("curl", true, "401"));
        asked.run();
        let ran = asked.runner.ran.borrow();
        let curls: Vec<&Vec<String>> = ran.iter().filter(|argv| argv[0] == "curl").collect();
        assert!(!curls.is_empty());
        for argv in curls {
            assert!(
                !argv.iter().any(|word| word == "--fail"),
                "the status is the answer: {argv:?}"
            );
            assert!(
                argv.iter().any(|word| word == "--max-time"),
                "a check that hangs is one nobody waits for: {argv:?}"
            );
            // The URL is data, and `--` is what makes the last argument mean it.
            assert_eq!(argv[argv.len() - 2], "--", "{argv:?}");
        }
    }

    // AYEAYE-62 — host validation is asserted in both directions, and the second
    // request is the one that proves anything: a server that accepts everything
    // passes the first half perfectly.
    #[test]
    fn the_host_check_asks_about_an_address_nobody_configured() {
        let captured = machine_with(&["curl"]);
        let mut asked = asking(
            &captured,
            Answers::default()
                .to("not-configured.invalid", true, "403")
                .to("Host: box.example", true, "200")
                .to("curl", true, "200"),
        );
        asked.allowed_hosts = vec!["box.example".to_string()];
        let report = asked.run();
        assert_eq!(verdict(&report, "hosts"), Verdict::Passed);
        assert!(
            asked.runner.ran.borrow().iter().any(|argv| argv
                .iter()
                .any(|word| word.contains("not-configured.invalid"))),
            "without the second half the check proves nothing"
        );

        // The same machine, accepting everything.
        let mut permissive = asking(&captured, Answers::default().to("curl", true, "200"));
        permissive.allowed_hosts = vec!["box.example".to_string()];
        assert_eq!(verdict(&permissive.run(), "hosts"), Verdict::Failed);
    }

    // AYEAYE-62 — and where nothing was configured, nothing is asked. A request
    // whose answer is never read is a request that should not be made.
    #[test]
    fn with_no_configured_address_no_host_request_is_made() {
        let captured = machine_with(&["curl"]);
        let asked = asking(&captured, Answers::default().to("curl", true, "200"));
        let report = asked.run();
        assert_eq!(verdict(&report, "hosts"), Verdict::Skipped);
        assert_eq!(
            verdict(&report, "https"),
            Verdict::Skipped,
            "no front end was asked for"
        );
        assert!(
            !asked
                .runner
                .ran
                .borrow()
                .iter()
                .any(|argv| argv.iter().any(|word| word.starts_with("Host:"))),
        );
    }

    // AYEAYE-62 — a machine with no service manager is skipped and not failed,
    // and no service command is run at all. This is the check the live-bus
    // accident would have gone through.
    #[test]
    fn with_no_service_manager_the_check_is_skipped_and_nothing_is_addressed() {
        let captured = machine_with(&["curl"]);
        let asked = asking(&captured, Answers::default().to("curl", true, "200"));
        let report = asked.run();
        assert_eq!(verdict(&report, "service"), Verdict::Skipped);
        assert!(
            !asked
                .runner
                .ran
                .borrow()
                .iter()
                .any(|argv| argv[0] == "systemctl" || argv[0] == "launchctl"),
        );
    }

    // AYEAYE-62 — with a manager, the status command is run and nothing else is.
    // A health check reads; it does not start, stop or enable anything.
    #[test]
    fn a_health_check_only_ever_asks_the_manager_for_status() {
        let captured = machine_with(&["curl"]);
        let session = Session::systemd();
        let mut asked = asking(&captured, Answers::default().to("curl", true, "200"));
        asked.session = Some(&session);
        let report = asked.run();
        assert_eq!(
            verdict(&report, "service"),
            Verdict::Failed,
            "nothing answered"
        );
        let ran = asked.runner.ran.borrow();
        let manager: Vec<&Vec<String>> = ran.iter().filter(|argv| argv[0] == "systemctl").collect();
        assert_eq!(manager.len(), 1, "exactly one, and it is a read");
        assert!(
            manager[0].contains(&"status".to_string()),
            "{:?}",
            manager[0]
        );
        for verb in ["enable", "disable", "start", "stop", "restart"] {
            assert!(
                !manager[0].contains(&verb.to_string()),
                "a health check must not {verb} anything"
            );
        }
    }

    // AYEAYE-62 — a missing tmux is a failure with the command for this platform
    // attached, and the command is never run. That is the whole of the
    // acceptance criterion.
    #[test]
    fn a_missing_multiplexer_carries_the_command_and_never_runs_it() {
        let captured = machine_with(&["curl"]);
        let asked = asking(&captured, Answers::default().to("curl", true, "200"));
        let report = asked.run();
        let check = report
            .checks
            .iter()
            .find(|check| check.name == "tmux")
            .expect("a tmux check");
        assert_eq!(check.verdict, Verdict::Failed);
        assert!(
            check
                .detail
                .as_deref()
                .is_some_and(|detail| detail.contains("tmux")),
            "{check:?}"
        );
        for argv in asked.runner.ran.borrow().iter() {
            assert!(
                !argv.iter().any(|word| word == "install"),
                "setup verifies the multiplexer and never installs it: {argv:?}"
            );
        }
    }

    // AYEAYE-62 — the key never reaches a command line, and it never outlives
    // the request. A token in an argument is a token in the process list and in
    // whatever log somebody pastes when the check fails.
    #[test]
    fn the_key_goes_in_a_file_that_deletes_itself() {
        let dir = std::env::temp_dir().join(format!("ayeaye-health-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = {
            let secret = SecretConfig::write(&dir, "not-a-real-secret").expect("a config file");
            let held = secret.path().to_path_buf();
            let text = std::fs::read_to_string(&held).expect("readable");
            assert!(text.contains("X-Voice-Token: not-a-real-secret"), "{text}");
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mode = std::fs::metadata(&held).unwrap().permissions().mode();
                assert_eq!(mode & 0o077, 0, "nobody else may read it: {mode:o}");
            }
            held
        };
        assert!(
            !path.exists(),
            "a key left behind in a file is a leaked credential"
        );
        // Twice over, because the first write must not stop the second.
        let again = SecretConfig::write(&dir, "another").expect("a second one");
        assert!(again.path().exists());
        drop(again);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
