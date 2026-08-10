//! `ayeaye setup` — doing what [`ayeaye_core::setup`] decided.
//!
//! The plan is the core's; this carries it out. Every step here writes a file,
//! fetches something, or asks a service manager to do something, and each one is
//! reached only if it is on the plan — which is what makes "records consent for
//! anything with a privacy or security consequence" a property of a data
//! structure rather than of remembering to ask.
//!
//! # What it will not do
//!
//! It does not configure network exposure, a reverse proxy, a mesh network, a
//! coding agent, or the terminal multiplexer. Every one of those is *detected
//! and verified* by [`crate::health`] and left alone. That is the milestone's
//! decision and not an unfinished corner: network placement is a judgement made
//! of assumptions, a conversation walks somebody through it better than a
//! branching wizard does, and the binary's job is to say whether the result
//! works.
//!
//! It also does not install packages. It prints the command for this platform,
//! from `machine::packages`, and stops.

use std::io::{BufRead, Read, Write};
use std::path::{Path, PathBuf};

use ayeaye_core::machine::Acceleration;
use ayeaye_core::model::{ModelId, settings};
use ayeaye_core::service::{Definition, Layout, Session, manual_instructions};
use ayeaye_core::setup::{Choices, Consent, Consequence, Existing, Plan, Step, Wanted, plan};

use crate::models::{self, Fetcher};
use crate::probe::Captured;
use crate::service::{Runner, Services};

/// How many bytes of entropy a key is made of.
///
/// Thirty-two, as `install.sh` mints it with `secrets.token_urlsafe(32)`. The
/// number is the input to the encoder and not the length of the result, which is
/// 43 characters.
const KEY_BYTES: usize = 32;

/// Where randomness comes from.
///
/// `/dev/urandom` rather than a crate. It is the kernel's CSPRNG on every
/// platform this ships to, it never blocks after boot, and reading a file is
/// something this crate is already allowed to do — where adding a dependency for
/// 32 bytes is a dependency the constitution's fourth rule would have to be
/// consulted about.
const ENTROPY: &str = "/dev/urandom";

/// Everything setup writes to, resolved once.
#[derive(Debug, Clone)]
pub struct Places {
    /// The settings file setup owns.
    pub config_file: PathBuf,
    /// Where this machine keeps ayeaye's state.
    pub state_dir: PathBuf,
    /// The key.
    pub token_file: PathBuf,
    /// Where model weights land.
    pub model_store: PathBuf,
}

impl Places {
    /// From the layout the environment describes.
    pub fn from(layout: &Layout, state_dir: &Path) -> Places {
        Places {
            config_file: PathBuf::from(&layout.env_file),
            state_dir: state_dir.to_path_buf(),
            token_file: state_dir.join("token"),
            model_store: state_dir.join("models"),
        }
    }
}

/// Asking a person a yes-or-no question.
///
/// A trait so that "with no terminal and no `--yes`, nothing consequential
/// happens" is a thing a test can assert rather than a thing a comment claims.
pub trait Ask {
    /// Ask, and take the answer. `default` is what a bare newline means.
    fn confirm(&self, question: &str, default: bool) -> bool;
}

/// A real terminal.
pub struct Tty;

impl Ask for Tty {
    fn confirm(&self, question: &str, default: bool) -> bool {
        let hint = if default { "Y/n" } else { "y/N" };
        print!("{question} [{hint}] ");
        let _ = std::io::stdout().flush();
        let mut line = String::new();
        if std::io::stdin().lock().read_line(&mut line).is_err() {
            return default;
        }
        match line.trim().to_ascii_lowercase().as_str() {
            "" => default,
            "y" | "yes" => true,
            _ => false,
        }
    }
}

/// Nobody to ask, so the answer is fixed.
///
/// `Assumed(false)` is what a run with no terminal gets, and it is the whole of
/// why `ayeaye setup` is safe for AYEAYE-63's downloader to hand off to blind:
/// the consequential steps become things it *tells you how to do* rather than
/// things it does.
pub struct Assumed(pub bool);

impl Ask for Assumed {
    fn confirm(&self, _question: &str, _default: bool) -> bool {
        self.0
    }
}

/// What was typed on the command line.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Flags {
    /// Yes to everything with a consequence.
    pub yes: bool,
    /// Do not install a service.
    pub no_service: bool,
    /// Do not fetch any model.
    pub no_model: bool,
    /// Fetch this one rather than the one that suits this machine.
    pub model: Option<ModelId>,
    /// The address to listen on.
    pub bind: Option<String>,
    /// The port to listen on.
    pub port: Option<u16>,
}

/// Read the flags, or say which one was wrong.
pub fn parse(args: &[String]) -> Result<Flags, String> {
    let mut flags = Flags::default();
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        let mut value = |name: &str| {
            rest.next()
                .cloned()
                .ok_or_else(|| format!("{name} needs a value"))
        };
        match arg.as_str() {
            "--yes" | "-y" => flags.yes = true,
            "--no-service" => flags.no_service = true,
            "--no-model" => flags.no_model = true,
            "--model" => {
                let given = value("--model")?;
                flags.model = Some(ModelId::parse(&given).map_err(|why| why.to_string())?);
            }
            "--bind" => flags.bind = Some(value("--bind")?),
            "--port" => {
                let given = value("--port")?;
                flags.port = Some(
                    given
                        .parse()
                        .map_err(|_| format!("--port wants a number, not {given:?}"))?,
                );
            }
            other => return Err(format!("unknown option {other:?}")),
        }
    }
    if flags.no_model && flags.model.is_some() {
        return Err("--model and --no-model ask for opposite things".to_string());
    }
    Ok(flags)
}

/// Why setup could not finish.
#[derive(Debug)]
pub enum Failed {
    /// A file could not be read or written.
    Disk(String),
    /// The service manager refused.
    Service(String),
    /// A model could not be fetched.
    Model(String),
}

impl std::fmt::Display for Failed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Failed::Disk(what) | Failed::Service(what) | Failed::Model(what) => f.write_str(what),
        }
    }
}

impl std::error::Error for Failed {}

/// What setup found and did, for whatever prints it.
#[derive(Debug, Default)]
pub struct Did {
    /// One line per step, in the order they happened.
    pub lines: Vec<String>,
    /// The consequential steps nobody agreed to, and how to take them later.
    pub declined: Vec<String>,
}

/// Decide the plan for this machine.
///
/// Split from [`carry_out`] so that what setup *would* do can be asked for
/// without doing any of it — which is what the printed plan above the questions
/// is, and what makes the questions informed consent rather than a prompt.
pub fn decide(
    captured: &Captured,
    places: &Places,
    session: Option<&Session>,
    flags: &Flags,
    ask: &impl Ask,
) -> Plan {
    let machine = captured.machine();
    let existing = Existing {
        // A file with nothing in it is not a key. `config::load_token` reads it
        // the same way, and disagreeing would mean setup keeps a key the daemon
        // then refuses to use.
        key: std::fs::read_to_string(&places.token_file).is_ok_and(|held| !held.trim().is_empty()),
        models: models::installed(&places.model_store),
    };
    let choices = Choices {
        consent: Consent::default(),
        service: !flags.no_service,
        model: match (&flags.model, flags.no_model) {
            (Some(id), _) => Wanted::Named(id.clone()),
            (None, true) => Wanted::None,
            (None, false) => Wanted::Suited,
        },
    };

    // Asked once, having seen what there is to consent to. A plan built with no
    // consent names every consequential step in `declined`, which is exactly the
    // list of questions worth putting — and on a machine where there is nothing
    // to download and nowhere to install a service, that list is empty and
    // nobody is asked anything.
    let proposed = plan(&machine, session.is_some(), &existing, &choices);
    let mut consent = Consent::default();
    for step in &proposed.declined {
        let Some(consequence) = step.consequence() else {
            continue;
        };
        let agreed = flags.yes
            || ask.confirm(
                &format!("{}. {}", step.describe(), consequence.question()),
                false,
            );
        match consequence {
            Consequence::Network => consent.network = agreed,
            Consequence::RunsAtLogin => consent.run_at_login = agreed,
        }
    }

    plan(
        &machine,
        session.is_some(),
        &existing,
        &Choices { consent, ..choices },
    )
}

/// Do it.
pub fn carry_out<R: Runner, F: Fetcher>(
    plan: &Plan,
    captured: &Captured,
    places: &Places,
    session: Option<&Session>,
    layout: &Layout,
    program: &str,
    flags: &Flags,
    runner: R,
    fetcher: &F,
    stamp: &str,
) -> Result<Did, Failed> {
    let mut did = Did::default();
    for step in &plan.steps {
        match step {
            Step::MintKey => {
                let key = mint(&places.token_file)?;
                did.lines.push(format!(
                    "generated the key at {}",
                    places.token_file.display()
                ));
                debug_assert!(!key.is_empty());
            }
            Step::WriteSettings => {
                let written = write_settings(places, flags)?;
                did.lines.push(written);
            }
            Step::FetchModel(id) => {
                let pulled = models::pull(
                    fetcher,
                    &places.model_store,
                    ayeaye_core::model::hub::DEFAULT_HOST,
                    id,
                )
                .map_err(|why| Failed::Model(why.to_string()))?;
                models::choose(&places.config_file, settings::SPEECH_MODEL, &id.to_string())
                    .map_err(|why| Failed::Model(why.to_string()))?;
                did.lines.push(format!(
                    "fetched {} into {}, and set it as the model to transcribe with",
                    pulled.id,
                    pulled.dir.display()
                ));
            }
            Step::InstallService => {
                let session = session.expect("a service step needs a session");
                let services = Services {
                    session: session.clone(),
                    layout: layout.clone(),
                    runner: &runner,
                };
                let installed = services
                    .install(&Definition::ayeaye(program), stamp)
                    .map_err(|why| Failed::Service(why.to_string()))?;
                did.lines
                    .push(format!("wrote {}", installed.path.display()));
                if let Some(backup) = installed.backup {
                    did.lines.push(format!(
                        "kept a copy of what was there at {}",
                        backup.display()
                    ));
                }
            }
            Step::EnableService => {
                let session = session.expect("a service step needs a session");
                let services = Services {
                    session: session.clone(),
                    layout: layout.clone(),
                    runner: &runner,
                };
                services
                    .enable("ayeaye")
                    .map_err(|why| Failed::Service(why.to_string()))?;
                did.lines
                    .push("ayeaye is running, and will start when you log in".to_string());
            }
        }
    }

    for step in &plan.declined {
        if let Some(by_hand) = step.by_hand() {
            did.declined.push(format!(
                "{} — `{by_hand}` when you want it",
                step.describe()
            ));
        }
    }

    // The third answer, reached from here as well as from `ayeaye service`.
    if session.is_none() {
        did.lines
            .extend(manual_instructions(program, &layout.env_file));
    }

    // Verified, never installed — and said whether or not anything else went
    // wrong, because it is the one thing without which none of the rest works.
    if !captured.has("tmux") {
        did.lines
            .push("tmux is not installed, and ayeaye reads your agents through it.".to_string());
        did.lines.extend(captured.install_hint(&["tmux"]));
    }

    Ok(did)
}

/// Generate the key, and write it where only its owner can read it.
///
/// Through a temporary file and a rename, so a key is never half-written; and
/// created with mode 600 rather than chmod-ed afterwards, because the window
/// between the two is a window in which the secret is world-readable.
fn mint(path: &Path) -> Result<String, Failed> {
    let disk = |what: String| move |why: std::io::Error| Failed::Disk(format!("{what}: {why}"));
    // `read_exact` into a fixed buffer, and emphatically not `fs::read`:
    // `/dev/urandom` is an endless stream with no EOF, so reading it to the end
    // never returns. It hangs rather than failing, which is the worst shape a
    // bug can have — `ayeaye setup` would simply stop, with the key half-made
    // and nothing said.
    let mut bytes = [0u8; KEY_BYTES];
    std::fs::File::open(ENTROPY)
        .and_then(|mut source| source.read_exact(&mut bytes))
        .map_err(disk(format!("read {KEY_BYTES} bytes from {ENTROPY}")))?;
    let key = ayeaye_core::setup::urlsafe(&bytes);

    let dir = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir).map_err(disk(format!("create {}", dir.display())))?;
    let temporary = dir.join(format!(
        ".{}.new",
        path.file_name().unwrap_or_default().to_string_lossy()
    ));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&temporary)
        .map_err(disk(format!("write {}", temporary.display())))?;
    writeln!(file, "{key}").map_err(disk(format!("write {}", temporary.display())))?;
    drop(file);
    std::fs::rename(&temporary, path).map_err(disk(format!("rename to {}", path.display())))?;
    Ok(key)
}

/// Write the settings setup owns, leaving everything else in the file alone.
///
/// A merge and never a rewrite: anything in there setup did not ask about
/// belongs to whoever put it there. The keys it does own are written every run,
/// so a settings file and a service definition installed months apart cannot
/// disagree about the port.
fn write_settings(places: &Places, flags: &Flags) -> Result<String, Failed> {
    let mut wanted: Vec<(&str, String)> = Vec::new();
    if let Some(bind) = &flags.bind {
        wanted.push(("BIND", bind.clone()));
    }
    if let Some(port) = flags.port {
        wanted.push(("DEV_PORT", port.to_string()));
    }

    let dir = places
        .config_file
        .parent()
        .unwrap_or(Path::new("."))
        .to_path_buf();
    std::fs::create_dir_all(&dir)
        .map_err(|why| Failed::Disk(format!("create {}: {why}", dir.display())))?;
    let before = std::fs::read_to_string(&places.config_file).unwrap_or_default();
    let after = wanted.iter().fold(before.clone(), |text, (key, value)| {
        settings::upsert(&text, key, value)
    });
    if after == before && !before.is_empty() {
        return Ok(format!(
            "{} was already correct",
            places.config_file.display()
        ));
    }
    std::fs::write(&places.config_file, after)
        .map_err(|why| Failed::Disk(format!("write {}: {why}", places.config_file.display())))?;
    Ok(format!("wrote {}", places.config_file.display()))
}

/// Write down what was agreed to, and when.
///
/// The record is the point. "Records consent for anything with a privacy or
/// security consequence" is not met by asking and forgetting: somebody coming
/// back to a machine months later has to be able to find out what they said yes
/// to, and so does anybody auditing what this program did to a computer.
pub fn record_consent(state_dir: &Path, plan: &Plan, stamp: &str) -> Result<(), Failed> {
    let agreed: Vec<String> = plan
        .steps
        .iter()
        .filter(|step| step.consequence().is_some())
        .map(|step| format!("{stamp} agreed {}", step.describe()))
        .collect();
    let declined: Vec<String> = plan
        .declined
        .iter()
        .map(|step| format!("{stamp} declined {}", step.describe()))
        .collect();
    if agreed.is_empty() && declined.is_empty() {
        return Ok(());
    }
    std::fs::create_dir_all(state_dir)
        .map_err(|why| Failed::Disk(format!("create {}: {why}", state_dir.display())))?;
    let path = state_dir.join("consent");
    // Appended rather than replaced: a record that only ever holds the last
    // answer is not a record of what somebody agreed to, and a run that declines
    // what an earlier run agreed to is exactly the history worth keeping.
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|why| Failed::Disk(format!("write {}: {why}", path.display())))?;
    for line in agreed.iter().chain(declined.iter()) {
        writeln!(file, "{line}")
            .map_err(|why| Failed::Disk(format!("write {}: {why}", path.display())))?;
    }
    Ok(())
}

/// What this build runs inference on, as the core names it.
///
/// The one place `ayeaye_infer::Backend` is turned into `machine::Acceleration`,
/// so the health check can compare what this build has against what this machine
/// has without either crate learning the other's vocabulary.
pub fn build_acceleration() -> Acceleration {
    match ayeaye_infer::backend::selected() {
        ayeaye_infer::backend::Backend::Cpu => Acceleration::Cpu,
        ayeaye_infer::backend::Backend::Cuda => Acceleration::Cuda,
        ayeaye_infer::backend::Backend::Metal => Acceleration::Metal,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Assumed, Flags, Places, build_acceleration, carry_out, decide, mint, parse, record_consent,
        write_settings,
    };
    use crate::probe::{Captured, Sources, capture};
    use crate::service::{Outcome, Runner};
    use ayeaye_core::machine::Acceleration;
    use ayeaye_core::model::ModelId;
    use ayeaye_core::service::{Layout, Session};
    use ayeaye_core::setup::{Step, urlsafe};
    use std::cell::RefCell;
    use std::path::{Path, PathBuf};

    /// A runner that records and refuses nothing.
    #[derive(Default)]
    struct Recorder {
        ran: RefCell<Vec<Vec<String>>>,
    }

    impl Runner for Recorder {
        fn run(&self, argv: &[String]) -> Outcome {
            self.ran.borrow_mut().push(argv.to_vec());
            Outcome {
                ok: true,
                output: String::new(),
            }
        }
    }

    /// A fetcher that records and downloads nothing.
    #[derive(Default)]
    struct NoDownloads {
        asked: RefCell<Vec<String>>,
    }

    impl crate::models::Fetcher for NoDownloads {
        fn get(&self, url: &str, _into: &Path) -> Result<(), String> {
            self.asked.borrow_mut().push(url.to_string());
            Err("this test never reaches the network".to_string())
        }
    }

    /// A machine with the named commands and nothing else.
    struct Bare(Vec<String>);

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
        fn read(&self, _path: &str) -> Option<String> {
            None
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
            self.0
                .iter()
                .any(|on| on == name)
                .then(|| format!("/usr/bin/{name}"))
        }
    }

    fn machine_with(commands: &[&str]) -> Captured {
        capture(
            &Bare(commands.iter().map(|name| (*name).to_string()).collect()),
            "/tmp/models",
        )
    }

    /// A directory of this test's own.
    fn scratch(what: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "ayeaye-setup-{what}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a scratch directory");
        dir
    }

    fn places(root: &Path) -> Places {
        Places {
            config_file: root.join("config/ayeaye/env"),
            state_dir: root.join("state/ayeaye"),
            token_file: root.join("state/ayeaye/token"),
            model_store: root.join("state/ayeaye/models"),
        }
    }

    fn layout(root: &Path) -> Layout {
        Layout::new(
            &root.to_string_lossy(),
            &root.join("config").to_string_lossy(),
            &root.join("state").to_string_lossy(),
        )
    }

    // AYEAYE-62 — the flags, including the pair that ask for opposite things. A
    // typo in a flag is refused rather than half-applied: setup writes files, and
    // a misread option is a file written differently from what was asked.
    #[test]
    fn the_options_are_read_and_a_wrong_one_is_refused() {
        let all = parse(
            &[
                "--yes",
                "--no-service",
                "--bind",
                "0.0.0.0",
                "--port",
                "9000",
            ]
            .map(String::from),
        )
        .expect("valid");
        assert!(all.yes && all.no_service);
        assert_eq!(all.bind.as_deref(), Some("0.0.0.0"));
        assert_eq!(all.port, Some(9000));

        assert_eq!(
            parse(&["--model".to_string(), "openai/whisper-tiny.en".to_string()])
                .expect("valid")
                .model,
            ModelId::parse("openai/whisper-tiny.en").ok()
        );
        assert!(parse(&["--soop".to_string()]).is_err());
        assert!(parse(&["--port".to_string()]).is_err(), "no value");
        assert!(parse(&["--port".to_string(), "eight".to_string()]).is_err());
        assert!(parse(&["--model".to_string(), "not an id".to_string()]).is_err());
        assert!(
            parse(&[
                "--model".to_string(),
                "openai/whisper-tiny.en".to_string(),
                "--no-model".to_string()
            ])
            .is_err(),
            "opposite things"
        );
    }

    // AYEAYE-62 — the acceptance criterion, and the reason `ayeaye setup` is safe
    // for AYEAYE-63's downloader to hand off to with no terminal attached: with
    // nobody to ask, nothing with a consequence happens, and setup says what
    // would have taken each step.
    #[test]
    fn with_nobody_to_ask_nothing_consequential_happens() {
        let root = scratch("unattended");
        let captured = machine_with(&["tmux"]);
        let session = Session::systemd();
        let decided = decide(
            &captured,
            &places(&root),
            Some(&session),
            &Flags::default(),
            &Assumed(false),
        );
        assert!(!decided.consequential());
        assert!(
            decided.declined.contains(&Step::EnableService),
            "and it is owed, not forgotten"
        );

        let fetcher = NoDownloads::default();
        let runner = Recorder::default();
        let did = carry_out(
            &decided,
            &captured,
            &places(&root),
            Some(&session),
            &layout(&root),
            "/opt/ayeaye",
            &Flags::default(),
            &runner,
            &fetcher,
            "1",
        )
        .expect("it should finish");

        assert!(fetcher.asked.borrow().is_empty(), "nothing was downloaded");
        assert!(
            runner.ran.borrow().is_empty(),
            "no service manager was addressed: {:?}",
            runner.ran.borrow()
        );
        assert!(
            did.declined
                .iter()
                .any(|line| line.contains("ayeaye service enable")),
            "declining must never be a dead end: {:?}",
            did.declined
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    // AYEAYE-62 — and with consent, the definition is written and the manager is
    // asked to enable it. Asserted on the argv that reaches the runner, which is
    // the only evidence there is that the right thing was asked — and the reason
    // this test can exist at all without a systemd.
    #[test]
    fn with_consent_the_definition_is_written_and_the_manager_asked_to_enable_it() {
        let root = scratch("consented");
        let captured = machine_with(&["tmux"]);
        let session = Session::systemd();
        let flags = Flags {
            yes: true,
            no_model: true,
            ..Flags::default()
        };
        let decided = decide(
            &captured,
            &places(&root),
            Some(&session),
            &flags,
            &Assumed(false),
        );
        let runner = Recorder::default();
        let did = carry_out(
            &decided,
            &captured,
            &places(&root),
            Some(&session),
            &layout(&root),
            "/opt/ayeaye",
            &flags,
            &runner,
            &NoDownloads::default(),
            "1",
        )
        .expect("it should finish");

        let unit = root.join("config/systemd/user/ayeaye.service");
        assert!(unit.exists(), "{:?}", did.lines);
        let written = std::fs::read_to_string(&unit).expect("readable");
        assert!(written.contains("ExecStart=/opt/ayeaye"), "{written}");

        let ran = runner.ran.borrow();
        assert!(
            ran.iter().any(|argv| argv.contains(&"enable".to_string())),
            "{ran:?}"
        );
        for argv in ran.iter() {
            assert_eq!(argv[0], "systemctl", "only the manager is run: {argv:?}");
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    // AYEAYE-62 — the key is minted once, kept for ever, and only its owner can
    // read it. Keeping it is the whole of "does not damage an existing install"
    // for the one file somebody's phone is already logged in with.
    #[test]
    fn the_key_is_minted_once_and_then_kept() {
        let root = scratch("key");
        let places = places(&root);
        let first = mint(&places.token_file).expect("a key");
        assert_eq!(first.len(), 43, "the shape secrets.token_urlsafe(32) makes");
        assert_eq!(
            std::fs::read_to_string(&places.token_file)
                .expect("readable")
                .trim(),
            first
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&places.token_file)
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o077, 0, "nobody else may read the key: {mode:o}");
        }

        // A second run does not plan to mint one, because the bookmark already on
        // somebody's phone is logged in with the first.
        let decided = decide(
            &machine_with(&[]),
            &places,
            None,
            &Flags {
                no_model: true,
                ..Flags::default()
            },
            &Assumed(true),
        );
        assert!(!decided.steps.contains(&Step::MintKey));

        // And two keys are never the same key.
        let elsewhere = places.token_file.with_extension("second");
        assert_ne!(first, mint(&elsewhere).expect("another"));
        let _ = std::fs::remove_dir_all(&root);
    }

    // AYEAYE-62 — an empty token file is not a key. `config::load_token` reads it
    // that way too, and disagreeing would leave setup keeping a key the daemon
    // then refuses to start with.
    #[test]
    fn a_token_file_with_nothing_in_it_is_not_a_key() {
        let root = scratch("emptykey");
        let places = places(&root);
        std::fs::create_dir_all(&places.state_dir).unwrap();
        std::fs::write(&places.token_file, "  \n").unwrap();
        let decided = decide(
            &machine_with(&[]),
            &places,
            None,
            &Flags {
                no_model: true,
                ..Flags::default()
            },
            &Assumed(true),
        );
        assert!(decided.steps.contains(&Step::MintKey));
        let _ = std::fs::remove_dir_all(&root);
    }

    // AYEAYE-62 — the settings file is merged and never rewritten: anything in it
    // setup did not ask about belongs to whoever put it there.
    #[test]
    fn the_settings_file_keeps_what_setup_did_not_ask_about() {
        let root = scratch("settings");
        let places = places(&root);
        std::fs::create_dir_all(places.config_file.parent().unwrap()).unwrap();
        std::fs::write(
            &places.config_file,
            "# mine\nAYEAYE_CLIBAN=/opt/cliban\nAYEAYE_BIND=10.0.0.1\n",
        )
        .unwrap();

        write_settings(
            &places,
            &Flags {
                bind: Some("127.0.0.1".to_string()),
                port: Some(9001),
                ..Flags::default()
            },
        )
        .expect("written");
        let after = std::fs::read_to_string(&places.config_file).expect("readable");
        assert!(after.contains("AYEAYE_CLIBAN=/opt/cliban"), "{after}");
        assert!(
            after.contains("# mine"),
            "a comment is somebody's note: {after}"
        );
        assert!(after.contains("AYEAYE_BIND=127.0.0.1"), "{after}");
        assert!(
            !after.contains("10.0.0.1"),
            "the old value is gone: {after}"
        );
        assert!(after.contains("AYEAYE_DEV_PORT=9001"), "{after}");
        let _ = std::fs::remove_dir_all(&root);
    }

    // AYEAYE-62 — and a run that was told nothing leaves a settings file that
    // already exists exactly as it was. Rewriting it whole to say the same thing
    // would change its modification time and its line order for no reason.
    #[test]
    fn a_run_with_nothing_to_say_leaves_the_settings_alone() {
        let root = scratch("settings-idle");
        let places = places(&root);
        std::fs::create_dir_all(places.config_file.parent().unwrap()).unwrap();
        std::fs::write(&places.config_file, "AYEAYE_BIND=10.0.0.1\n").unwrap();
        let said = write_settings(&places, &Flags::default()).expect("fine");
        assert!(said.contains("already correct"), "{said}");
        assert_eq!(
            std::fs::read_to_string(&places.config_file).unwrap(),
            "AYEAYE_BIND=10.0.0.1\n"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    // AYEAYE-62 — consent is *recorded*, not merely asked. Somebody coming back
    // to a machine months later has to be able to find out what they said yes to,
    // and so does anybody auditing what this program did to a computer.
    #[test]
    fn what_was_agreed_to_is_written_down_and_never_overwritten() {
        let root = scratch("consent");
        let captured = machine_with(&[]);
        let session = Session::systemd();
        let flags = Flags {
            yes: true,
            no_model: true,
            ..Flags::default()
        };
        let agreed = decide(
            &captured,
            &places(&root),
            Some(&session),
            &flags,
            &Assumed(false),
        );
        record_consent(&places(&root).state_dir, &agreed, "1000").expect("recorded");

        let declined = decide(
            &captured,
            &places(&root),
            Some(&session),
            &Flags {
                no_model: true,
                ..Flags::default()
            },
            &Assumed(false),
        );
        record_consent(&places(&root).state_dir, &declined, "2000").expect("recorded");

        let held =
            std::fs::read_to_string(places(&root).state_dir.join("consent")).expect("a record");
        assert!(held.contains("1000 agreed start ayeaye now"), "{held}");
        assert!(
            held.contains("2000 declined start ayeaye now"),
            "the history is the record, and a later no does not erase an earlier yes: {held}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    // AYEAYE-62 — a machine with nowhere to install a service is told how to run
    // ayeaye by hand, from setup as well as from `ayeaye service`. AYEAYE-61
    // recorded this as owed, and setup is the door most people come through.
    #[test]
    fn setup_on_a_machine_with_no_manager_says_how_to_run_it_by_hand() {
        let root = scratch("no-manager");
        let captured = machine_with(&["tmux"]);
        let flags = Flags {
            yes: true,
            no_model: true,
            ..Flags::default()
        };
        let decided = decide(&captured, &places(&root), None, &flags, &Assumed(true));
        assert!(!decided.steps.contains(&Step::InstallService));
        let runner = Recorder::default();
        let did = carry_out(
            &decided,
            &captured,
            &places(&root),
            None,
            &layout(&root),
            "/opt/ayeaye",
            &flags,
            &runner,
            &NoDownloads::default(),
            "1",
        )
        .expect("a finished run");
        assert!(
            did.lines
                .iter()
                .any(|line| line.contains("run the server with: /opt/ayeaye")),
            "{:?}",
            did.lines
        );
        assert!(runner.ran.borrow().is_empty(), "nothing was addressed");
        let _ = std::fs::remove_dir_all(&root);
    }

    // AYEAYE-62 — the multiplexer is verified and never installed: setup prints
    // the exact command for this platform and stops there.
    #[test]
    fn a_missing_multiplexer_is_named_with_its_command_and_never_installed() {
        let root = scratch("tmux");
        let captured = machine_with(&[]);
        let flags = Flags {
            no_model: true,
            ..Flags::default()
        };
        let decided = decide(&captured, &places(&root), None, &flags, &Assumed(false));
        let runner = Recorder::default();
        let did = carry_out(
            &decided,
            &captured,
            &places(&root),
            None,
            &layout(&root),
            "/opt/ayeaye",
            &flags,
            &runner,
            &NoDownloads::default(),
            "1",
        )
        .expect("a finished run");
        assert!(
            did.lines
                .iter()
                .any(|line| line.contains("tmux is not installed")),
            "{:?}",
            did.lines
        );
        assert!(
            did.lines
                .iter()
                .any(|line| line.contains("packages needed: tmux")),
            "{:?}",
            did.lines
        );
        for argv in runner.ran.borrow().iter() {
            assert!(
                !argv.contains(&"install".to_string()),
                "setup verifies it and never installs it: {argv:?}"
            );
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    // AYEAYE-62 — the one place a compiled-in backend is turned into the core's
    // word for it, so the health check can compare the two without either crate
    // learning the other's vocabulary. The assertion is chosen at compile time
    // because so is the answer.
    #[test]
    fn the_build_names_its_acceleration_in_the_cores_words() {
        #[cfg(not(any(feature = "cuda", feature = "metal")))]
        assert_eq!(build_acceleration(), Acceleration::Cpu);
        #[cfg(feature = "cuda")]
        assert_eq!(build_acceleration(), Acceleration::Cuda);
        #[cfg(all(feature = "metal", not(feature = "cuda")))]
        assert_eq!(build_acceleration(), Acceleration::Metal);
        // And whatever it is, it is one the core can compare against a machine.
        assert_ne!(
            build_acceleration(),
            Acceleration::Rocm,
            "no ROCm backend exists"
        );
        assert_eq!(
            urlsafe(&[0, 0, 0]),
            "AAAA",
            "the core's encoder, not a second one"
        );
    }
}
