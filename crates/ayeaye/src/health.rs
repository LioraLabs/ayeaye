//! Asking the questions [`ayeaye_core::health`] answers.
//!
//! The judgement is the core's; this makes the requests. Every one of them goes
//! through curl, which is AYEAYE-56's precedent and constitution rule 4's
//! consequence: no pure-Rust TLS client passes the dependency gate, and this
//! needs to reach an https front end somebody put in front of ayeaye.
//!
//! **A machine with no curl is not a machine that passed.** It is said once at
//! the top, and every network check that needed a request reports `unknown` —
//! while what nobody asked for stays `skipped`, because "could not be asked"
//! and "not asked for" are different facts and neither may wear the other's
//! mark. The shell says the same, in the same place, for the same reason.
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
    /// Whether the bind address keeps ayeaye on this computer.
    ///
    /// Network exposure, *detected and never configured* — the criterion's
    /// exact words. A machine bound to anything but loopback has been put on a
    /// network by somebody, and that is the fact the front-end checks are about.
    pub loopback_only: bool,
    /// The shared secret, where there is one to use.
    pub token: Option<String>,
    /// Where a temporary curl configuration may be written.
    pub state_dir: Option<PathBuf>,
    /// What this machine is.
    pub captured: &'a Captured,
    /// The session the service lives in, if this machine has one.
    pub session: Option<&'a Session>,
    /// Where the inference backend is, as a URL with no trailing slash.
    pub backend: String,
    /// The models dictation has been configured to ask it for.
    pub wanted: Vec<String>,
    /// How commands are run.
    pub runner: R,
}

/// What the curl configuration holding the key is called.
///
/// Named once so the sweep and the writer cannot disagree about which file is
/// the one holding a credential.
const SECRET_FILE: &str = "health-request";

/// The coding agents ayeaye knows how to show.
///
/// The same two `crate::process` recognises, and the list is here rather than
/// inline so that adding a third is one edit.
const AGENTS: &[&str] = &["claude", "codex"];

/// An address nobody would ever configure, used to prove ayeaye refuses one.
///
/// `.invalid` is reserved by RFC 2606 precisely so that it can never resolve, so
/// this cannot collide with a real deployment however unlucky somebody is.
const A_STRANGER: &str = "not-configured.invalid";

impl<R: Runner> Asking<'_, R> {
    /// Run every check and report.
    ///
    /// The order is the shell's where the shell has one — service, then the
    /// local answer, then the lock — with the checks this ticket added placed
    /// beside the ones they belong with. The agents come last rather than
    /// fourth because nothing else depends on them.
    pub fn run(&self) -> Report {
        let mut report = Report::default();
        let curl = self.captured.has("curl");
        // Whatever a crashed run left behind holds a key. The shell sweeps this
        // at both ends for that reason, and its own test harness has a tripwire
        // watching for the leftover; `SecretConfig`'s `Drop` covers the run that
        // finishes, and this covers the one that did not.
        self.sweep_stale_secret();

        report.record(self.service_check());
        report.record(self.tmux_check());
        report.record(self.backend_check(curl));

        if !curl {
            // Said once, at the top, rather than underneath every request — but
            // only for the questions that *needed* one. What is decidable
            // without a request keeps its real verdict: nothing configured is
            // still "you did not ask for this", because "could not be asked"
            // and "not asked for" are different facts and neither may wear the
            // other's mark. The verdicts stay the core's, fed the evidence
            // there is — which is none.
            let could_not = "this computer has no curl, so this could not be asked";
            report.record(Check {
                name: "local",
                claim: format!("ayeaye answers on {}", self.url),
                verdict: health::local(None),
                detail: Some(could_not.to_string()),
                explains_itself: false,
            });
            report.record_auth("ayeaye refuses anyone without your key", None);
            if let Some(auth) = report.checks.last_mut() {
                // The fused call owns the verdict and the alarm; only the
                // explanation is this path's to give.
                auth.detail = Some(could_not.to_string());
            }
            report.record(Check {
                name: "authorised",
                claim: "your key opens the page".to_string(),
                verdict: Verdict::Unknown,
                detail: Some(
                    if self.token.is_some() && self.state_dir.is_some() {
                        could_not
                    } else {
                        "there is no key on this computer to try"
                    }
                    .to_string(),
                ),
                explains_itself: false,
            });
            let unasked: Vec<Option<u16>> = self.allowed_hosts.iter().map(|_| None).collect();
            report.record(Check {
                name: "hosts",
                claim: self.hosts_claim(),
                verdict: health::hosts(&unasked, None),
                detail: (!unasked.is_empty()).then(|| could_not.to_string()),
                explains_itself: false,
            });
            let host = self.allowed_hosts.first().map(String::as_str);
            let asked_for = !self.allowed_hosts.is_empty() || !self.loopback_only;
            report.record(Check {
                name: "https",
                claim: Self::https_claim(host),
                verdict: health::https(asked_for, host, None),
                detail: asked_for.then(|| could_not.to_string()),
                explains_itself: false,
            });
            report.record(self.mesh_check());
            // The board needs the key over HTTP, so a cliban that is here could
            // not be asked about — and one that is not here is skipped exactly
            // as it is with curl, rather than a line that quietly vanishes.
            let cliban = self.captured.has("cliban");
            report.record(Check {
                name: "board",
                claim: "your ticket board on the phone".to_string(),
                verdict: health::board(cliban, None),
                detail: cliban.then(|| could_not.to_string()),
                explains_itself: false,
            });
            report.record(self.agents_check());
            return report;
        }

        let local = self.code(&format!("{}/", self.url), &[]);
        report.record(Check {
            name: "local",
            claim: format!("ayeaye answers on {}", self.url),
            verdict: health::local(local),
            detail: local.map(|code| format!("it answered {code}")),
            explains_itself: false,
        });

        report.record_auth(
            "ayeaye refuses anyone without your key",
            self.code(&format!("{}/api/overview", self.url), &[]),
        );

        report.record(self.authorised_check());
        report.record(self.hosts_check());
        report.record(self.https_check());
        report.record(self.mesh_check());
        report.record(self.board_check());
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
                explains_itself: false,
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
            explains_itself: false,
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
            explains_itself: false,
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
                explains_itself: false,
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
                    explains_itself: false,
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
            explains_itself: false,
        }
    }

    /// What the host check claims — one wording, whether or not it can be asked.
    fn hosts_claim(&self) -> String {
        if self.allowed_hosts.is_empty() {
            "ayeaye accepting an address you named".to_string()
        } else {
            format!(
                "ayeaye accepts {} and refuses anything else",
                self.allowed_hosts.join(", ")
            )
        }
    }

    /// What the https check claims, for whichever address there is.
    fn https_claim(host: Option<&str>) -> String {
        match host {
            Some(host) => format!("https://{host}/ answers, which is what your phone opens"),
            None => "an https address your phone can open".to_string(),
        }
    }

    /// Host validation, asserted in both directions.
    fn hosts_check(&self) -> Check {
        let claim = self.hosts_claim();
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
                let said = |code: Option<u16>| match code {
                    Some(code) => code.to_string(),
                    None => "nothing".to_string(),
                };
                format!(
                    "an address nobody configured answered {}; yours answered {}",
                    said(stranger),
                    configured
                        .iter()
                        .copied()
                        .map(said)
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            }),
            explains_itself: false,
        }
    }

    /// The https front end, whichever one is in place — detected and verified,
    /// never configured.
    fn https_check(&self) -> Check {
        let host = self.allowed_hosts.first();
        // Two signals, and the second is what makes "setup could not tell" a
        // reachable answer rather than a branch nothing takes. A configured
        // address is the obvious one. The other is a bind address that is not
        // loopback: a machine listening on the network has been *exposed* — by a
        // proxy, a mesh, or a hand-edited settings file — and that exposure is
        // the thing a front-end check is about. Exposed with no address to try is
        // exactly "you asked for this and setup could not check it", which is the
        // shell's answer where its `answer.access.mode` was set and its host list
        // was empty. Treating it as "you did not ask" would be the criterion's
        // floor broken from the other side.
        let exposed = !self.loopback_only;
        let asked_for = !self.allowed_hosts.is_empty() || exposed;
        let code = host.and_then(|host| self.code(&format!("https://{host}/"), &[]));
        Check {
            name: "https",
            claim: Self::https_claim(host.map(String::as_str)),
            verdict: health::https(asked_for, host.map(String::as_str), code),
            detail: code.map(|code| format!("it answered {code}")),
            explains_itself: false,
        }
    }

    /// A mesh network, if this machine is on one.
    ///
    /// Detected and verified, never configured. `tailscale status` is a read —
    /// it reports and changes nothing — which is the only kind of question this
    /// binary is allowed to ask about somebody's network.
    fn mesh_check(&self) -> Check {
        let installed = self.captured.has("tailscale");
        let up = installed.then(|| {
            self.runner
                .run(&["tailscale".to_string(), "status".to_string()])
                .ok
        });
        Check {
            name: "mesh",
            claim: "the mesh network you reach this machine over".to_string(),
            verdict: health::mesh(installed, up),
            detail: None,
            explains_itself: false,
        }
    }

    /// The ticket board, when there is a cliban to reach.
    ///
    /// Two things have to be true and they fail differently: the program has to
    /// be here, and the app has to be able to get an answer out of it. The second
    /// needs the key, so it goes through the same self-deleting config file.
    fn board_check(&self) -> Check {
        let installed = self.captured.has("cliban");
        let claim = "your ticket board on the phone".to_string();
        if !installed {
            return Check {
                name: "board",
                claim,
                verdict: health::board(false, None),
                detail: None,
                explains_itself: false,
            };
        }
        let answered = self.with_key(&format!("{}/api/cliban/projects", self.url));
        Check {
            name: "board",
            claim,
            verdict: health::board(true, answered.as_deref()),
            detail: answered
                .is_none()
                .then(|| "cliban is here and the app would not answer about it".to_string()),
            explains_itself: false,
        }
    }

    /// Remove a curl configuration a previous run left behind.
    ///
    /// It holds the key, and a run that was killed between writing it and using
    /// it leaves it on disk with nothing to clean it up.
    fn sweep_stale_secret(&self) {
        if let Some(dir) = self.state_dir.as_deref() {
            let _ = std::fs::remove_file(dir.join(SECRET_FILE));
        }
    }

    /// The body of a request that carried the key, or `None` when it could not
    /// be made at all.
    fn with_key(&self, url: &str) -> Option<String> {
        let secret =
            SecretConfig::write(self.state_dir.as_deref()?, self.token.as_deref()?).ok()?;
        let mut argv: Vec<String> = ["curl", "--silent", "--show-error", "--max-time", TIMEOUT]
            .iter()
            .map(|word| (*word).to_string())
            .collect();
        argv.push("-K".to_string());
        argv.push(secret.path().to_string_lossy().into_owned());
        argv.push("--".to_string());
        argv.push(url.to_string());
        let asked = self.runner.run(&argv);
        asked.ok.then_some(asked.output)
    }

    /// The inference backend: reachable, and serving what was chosen.
    ///
    /// This replaced the acceleration check, and the replacement is the ticket
    /// in one function. "Which device is this build compiled for" was a question
    /// about this binary, and this binary no longer runs a model — llama-swap
    /// does, in its own process, on whatever it decided to use. What is worth
    /// checking now is the thing that actually breaks a dictation: the proxy is
    /// not running, or it is running and has never heard of the model named in
    /// `~/.config/ayeaye/env`.
    ///
    /// Through curl for the same reason every other request here is: the shell
    /// makes requests with the program the machine already has, and this check
    /// must be answerable from `ayeaye check`, which is synchronous.
    fn backend_check(&self, curl: bool) -> Check {
        let wanted: Vec<&str> = self.wanted.iter().map(String::as_str).collect();
        let url = format!("{}/v1/models", self.backend);
        let served = curl.then(|| self.served(&url)).flatten();
        let verdict = health::backend(served.as_deref(), &wanted);
        let detail = match (&served, &verdict) {
            (None, _) if !curl => {
                Some("this computer has no curl, so this could not be asked".to_string())
            }
            (None, _) => Some(format!(
                "nothing answered at {url} — is llama-swap running?"
            )),
            (Some(served), Verdict::Failed) => {
                let missing: Vec<&str> = wanted
                    .iter()
                    .filter(|name| !served.iter().any(|had| had == *name))
                    .copied()
                    .collect();
                Some(format!(
                    "it is not serving {}; it serves {}",
                    missing.join(", "),
                    if served.is_empty() {
                        "nothing".to_string()
                    } else {
                        served.join(", ")
                    }
                ))
            }
            _ => None,
        };
        Check {
            name: "backend",
            claim: if wanted.is_empty() {
                format!("an inference backend at {}", self.backend)
            } else {
                format!("{} is serving {}", self.backend, wanted.join(" and "))
            },
            verdict,
            detail,
            explains_itself: false,
        }
    }

    /// The model names the proxy answered with, or `None` if it did not answer.
    ///
    /// No key: llama-swap is not ayeaye and does not have ayeaye's secret. A
    /// proxy that wants one is behind something that terminates TLS, which this
    /// client could not reach either — see `crate::swap`.
    fn served(&self, url: &str) -> Option<Vec<String>> {
        let argv: Vec<String> = [
            "curl",
            "--silent",
            "--show-error",
            "--max-time",
            TIMEOUT,
            "--",
            url,
        ]
        .iter()
        .map(|word| (*word).to_string())
        .collect();
        let asked = self.runner.run(&argv);
        if !asked.ok {
            return None;
        }
        let body = ayeaye_core::json::parse(&asked.output).ok()?;
        let ayeaye_core::json::Value::List(models) = body.get("data")? else {
            return None;
        };
        Some(
            models
                .iter()
                .filter_map(|model| Some(model.get("id")?.text()?.to_string()))
                .collect(),
        )
    }

    /// The coding agents: detected and verified, never installed.
    fn agents_check(&self) -> Check {
        // Every candidate with whether it is here, and not only the ones that
        // are: an absent agent that never reaches the core is an agent the core
        // can never report on, which is how this check became one that could
        // not fail.
        let candidates: Vec<(&str, bool)> = AGENTS
            .iter()
            .map(|name| (*name, self.captured.has(name)))
            .collect();
        let here: Vec<&str> = candidates
            .iter()
            .filter(|(_, present)| *present)
            .map(|(name, _)| *name)
            .collect();
        Check {
            name: "agents",
            claim: if here.is_empty() {
                "a coding agent for ayeaye to show you".to_string()
            } else {
                format!("the coding agents on this computer: {}", here.join(", "))
            },
            verdict: health::agents(&candidates),
            detail: here
                .is_empty()
                .then(|| format!("none of {} is on PATH", AGENTS.join(", "))),
            explains_itself: false,
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
        let path = dir.join(SECRET_FILE);
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
    use ayeaye_core::service::{Manager, Session};
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
        capture(&Bare {
            path: commands.iter().map(|name| (*name).to_string()).collect(),
            files: HashMap::new(),
        })
    }

    fn asking<'a>(captured: &'a Captured, runner: Answers) -> Asking<'a, Answers> {
        Asking {
            url: "http://127.0.0.1:8912".to_string(),
            allowed_hosts: Vec::new(),
            loopback_only: true,
            token: None,
            state_dir: None,
            captured,
            session: None,
            backend: "http://127.0.0.1:8080".to_string(),
            wanted: Vec::new(),
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
    // check that needed a request reports unknown, said once at the top — and
    // what nobody asked for stays skipped, because "could not be asked" and
    // "not asked for" are different facts and neither may wear the other's
    // mark, from either side.
    #[test]
    fn a_machine_with_no_curl_reports_unknown_and_never_ok() {
        let captured = machine_with(&["tmux"]);
        let report = asking(&captured, Answers::default()).run();
        for name in ["local", "auth", "authorised"] {
            assert_eq!(
                verdict(&report, name),
                Verdict::Unknown,
                "{name} could not be asked"
            );
        }
        // Nothing configured and loopback only: the very verdicts the same
        // machine gets with curl present, not a blanket unknown.
        for name in ["hosts", "https", "board"] {
            assert_eq!(
                verdict(&report, name),
                Verdict::Skipped,
                "{name} was never asked for, curl or no curl"
            );
        }
        assert_eq!(report.outcome(), StepOutcome::Unfinished);
        assert_eq!(
            verdict(&report, "tmux"),
            Verdict::Passed,
            "that one is a PATH lookup"
        );

        // And the other side of the same coin: what *was* asked for on a
        // machine with no curl is unknown, never skipped, and never absent.
        let with_board = machine_with(&["tmux", "cliban"]);
        let mut exposed = asking(&with_board, Answers::default());
        exposed.allowed_hosts = vec!["box.example".to_string()];
        exposed.loopback_only = false;
        let owed = exposed.run();
        for name in ["hosts", "https", "board"] {
            assert_eq!(
                verdict(&owed, name),
                Verdict::Unknown,
                "{name} was asked for and could not be checked"
            );
        }
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

    // AYEAYE-62 — a curl that could not run is not a status. Parsing one out of
    // a failed request would turn "nothing answered" into whatever digits were
    // lying in the output, and the whole floor of this step is that no answer is
    // never a pass.
    #[test]
    fn a_curl_that_failed_is_not_a_status() {
        let captured = machine_with(&["curl"]);
        // Exit non-zero, and still print something that parses as a status.
        let lying = asking(&captured, Answers::default().to("curl", false, "200"));
        let report = lying.run();
        assert_eq!(verdict(&report, "local"), Verdict::Unknown);
        assert_eq!(verdict(&report, "auth"), Verdict::Unknown);
        assert!(!report.insecure, "a failed request cannot raise the alarm");

        // And curl's own "could not connect" answer, which is `000`.
        let refused = asking(&captured, Answers::default().to("curl", true, "000"));
        assert_eq!(verdict(&refused.run(), "local"), Verdict::Unknown);
    }

    // AYEAYE-62 — a service that could not be asked is never a service that is
    // up. This is reachable rather than theoretical: a Mac whose `id` will not
    // answer has a launchd and no domain to address, so `Session::command`
    // refuses and there is no status to read.
    #[test]
    fn a_service_that_could_not_be_asked_is_never_reported_as_running() {
        let captured = machine_with(&["curl"]);
        // launchd with no uid: the core refuses to build a command at all.
        let unaddressable = Session {
            manager: Manager::Launchd,
            uid: None,
            launchd_prefix: "dev".to_string(),
        };
        let mut asked = asking(&captured, Answers::default().to("curl", true, "200"));
        asked.session = Some(&unaddressable);
        let report = asked.run();
        assert_eq!(verdict(&report, "service"), Verdict::Unknown);
        assert_ne!(verdict(&report, "service"), Verdict::Passed);
        assert!(
            !asked
                .runner
                .ran
                .borrow()
                .iter()
                .any(|argv| argv[0] == "launchctl"),
            "there was no domain to address, so nothing should have been addressed"
        );
    }

    // AYEAYE-62 — and with no key on this computer there is nothing to try, so
    // the check that proves your key opens the page cannot pass. A machine with
    // no key at all reporting "your key opens the page" is the exact shape of
    // failure this ticket exists to prevent.
    #[test]
    fn with_no_key_the_authorised_check_cannot_pass() {
        let captured = machine_with(&["curl"]);
        let no_key = asking(&captured, Answers::default().to("curl", true, "200"));
        assert_eq!(verdict(&no_key.run(), "authorised"), Verdict::Unknown);

        // A key, and a daemon that refuses it: that is a failure, not an unknown.
        let root = std::env::temp_dir().join(format!("ayeaye-authorised-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let mut wrong_key = asking(&captured, Answers::default().to("curl", true, "401"));
        wrong_key.token = Some("not-the-daemons-key".to_string());
        wrong_key.state_dir = Some(root.clone());
        assert_eq!(verdict(&wrong_key.run(), "authorised"), Verdict::Failed);
        let _ = std::fs::remove_dir_all(&root);
    }

    // AYEAYE-62 — every check this module knows how to make is actually made. A
    // check that is never recorded is a capability nobody is told about, and it
    // is invisible: the report simply has one fewer line and nothing says which.
    #[test]
    fn every_check_reaches_the_report_on_both_paths() {
        let expected = [
            "service",
            "tmux",
            "backend",
            "local",
            "auth",
            "authorised",
            "hosts",
            "https",
            "mesh",
            "board",
            "agents",
        ];
        let with_curl = machine_with(&["curl"]);
        let full = asking(&with_curl, Answers::default().to("curl", true, "200")).run();
        for name in expected {
            assert!(
                full.checks.iter().any(|check| check.name == name),
                "{name} was never recorded"
            );
        }
        assert_eq!(full.checks.len(), expected.len(), "and nothing twice");

        // The no-curl path reports on every one of them too — a check that
        // vanishes is a capability nobody is told about, which was exactly how
        // the board line went missing here once.
        let without = machine_with(&[]);
        let bare = asking(&without, Answers::default()).run();
        for name in expected {
            assert!(
                bare.checks.iter().any(|check| check.name == name),
                "{name} vanished when there was no curl"
            );
        }
        assert_eq!(bare.checks.len(), expected.len(), "and nothing twice");
    }

    // AYEAYE-62 — a mesh network and a ticket board are detected and verified and
    // never configured, and neither is a failure for being absent.
    #[test]
    fn a_mesh_and_a_board_are_verified_where_they_exist_and_never_set_up() {
        let neither = machine_with(&["curl"]);
        let report = asking(&neither, Answers::default().to("curl", true, "200")).run();
        assert_eq!(verdict(&report, "mesh"), Verdict::Skipped);
        assert_eq!(verdict(&report, "board"), Verdict::Skipped);

        let both = machine_with(&["curl", "tailscale", "cliban"]);
        let asked = asking(
            &both,
            Answers::default()
                .to("tailscale status", true, "100.64.0.1 box")
                .to("api/cliban/projects", true, "{\"keys\": [\"AYEAYE\"]}")
                .to("curl", true, "200"),
        );
        let report = asked.run();
        assert_eq!(verdict(&report, "mesh"), Verdict::Passed);
        for argv in asked.runner.ran.borrow().iter() {
            if argv[0] == "tailscale" {
                assert_eq!(argv[1], "status", "only ever a read: {argv:?}");
            }
        }

        // A mesh client that is here and down is a failure: something was set up
        // and is not working.
        let down = asking(&both, Answers::default().to("curl", true, "200"));
        assert_eq!(verdict(&down.run(), "mesh"), Verdict::Failed);
    }

    // AYEAYE-62 — the fourth mark means "you did not ask for this", and a check
    // must only wear it when somebody really did decline something.
    //
    // AYEAYE-101 moved this from the acceleration check to the backend one, and
    // the reasoning transferred whole: a machine with no model configured did
    // not ask for dictation, so `skipped` is honest there — but a *configured*
    // model the proxy is not serving is a failure, and rendering that as "you
    // did not ask" would be telling somebody they declined the thing they wrote
    // into their config file.
    #[test]
    fn a_check_only_says_you_did_not_ask_when_somebody_declined_something() {
        let captured = machine_with(&["curl"]);
        let report = asking(&captured, Answers::default().to("curl", true, "200")).run();
        let backend = report
            .checks
            .iter()
            .find(|check| check.name == "backend")
            .expect("recorded");
        // Nothing configured in this fixture, so nothing was asked for.
        assert_eq!(backend.verdict, Verdict::Skipped);

        // And a machine that did name a model gets a real verdict rather than a
        // shrug, whichever way it goes.
        let mut configured = asking(&captured, Answers::default().to("curl", true, "200"));
        configured.wanted = vec!["whisper".to_string()];
        let named = configured.run();
        let backend = named
            .checks
            .iter()
            .find(|check| check.name == "backend")
            .expect("recorded");
        assert_ne!(backend.verdict, Verdict::Skipped, "{}", backend.line());
        assert!(
            !backend.line().contains("you did not ask"),
            "{}",
            backend.line()
        );

        // While a check that really is about a choice still says so.
        let hosts = report
            .checks
            .iter()
            .find(|check| check.name == "hosts")
            .expect("recorded");
        assert_eq!(hosts.verdict, Verdict::Skipped);
        assert!(hosts.line().contains("you did not ask"), "{}", hosts.line());
    }

    // AYEAYE-62 — a machine put on a network by somebody, with no address to
    // check it at, is "you asked for this and setup could not tell" and never
    // "you did not ask for this". That is the criterion's floor, from the side
    // that is easy to miss.
    #[test]
    fn an_exposed_machine_with_no_address_is_unknown_and_not_skipped() {
        let captured = machine_with(&["curl"]);
        let mut exposed = asking(&captured, Answers::default().to("curl", true, "200"));
        exposed.loopback_only = false;
        assert_eq!(verdict(&exposed.run(), "https"), Verdict::Unknown);

        // Still on this computer only: nothing was asked for and nothing is owed.
        let private = asking(&captured, Answers::default().to("curl", true, "200"));
        assert_eq!(verdict(&private.run(), "https"), Verdict::Skipped);
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
