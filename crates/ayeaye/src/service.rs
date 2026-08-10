//! Installing, repairing, starting and removing ayeaye's own service.
//!
//! This is the one thing setup still *does* to the machine. Everything the core
//! decided — what the definition says, what it is called, where it lands, which
//! command an operation is — arrives here as data; what is left is the part that
//! cannot be decided, only done: writing a file and running a program.
//!
//! Running a program goes through [`Runner`] rather than straight to
//! `std::process::Command`, and that is not ceremony. "Enable, start, stop and
//! status all work" has to be assertable on a machine with neither service
//! manager, and a recorded argv is the only evidence there is.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use ayeaye_core::service::{
    Definition, Install, Layout, Manager, Operation, Session, Unavailable, plan_install,
    unrepresentable_in_systemd,
};

/// What running a command came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    /// Whether it exited successfully.
    pub ok: bool,
    /// Everything it said, both streams together. At the moment something
    /// fails, the service manager's own words are worth more to whoever has to
    /// fix it than any paraphrase of them.
    pub output: String,
}

/// Something that can run a command.
pub trait Runner {
    /// Run it, and say what came of it. A command that could not be started at
    /// all is a command that failed.
    fn run(&self, argv: &[String]) -> Outcome;
}

/// A borrowed runner is a runner. AYEAYE-62 needs one runner to serve several
/// `Services` in a single `ayeaye setup`, and recording every command a whole
/// run made is only possible if they all reach the same recorder.
impl<R: Runner + ?Sized> Runner for &R {
    fn run(&self, argv: &[String]) -> Outcome {
        (**self).run(argv)
    }
}

/// The real one.
#[derive(Debug, Clone, Copy, Default)]
pub struct Subprocess;

impl Runner for Subprocess {
    fn run(&self, argv: &[String]) -> Outcome {
        let Some((program, rest)) = argv.split_first() else {
            return Outcome {
                ok: false,
                output: "no command to run".to_string(),
            };
        };
        match Command::new(program).args(rest).output() {
            Ok(output) => {
                let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
                text.push_str(&String::from_utf8_lossy(&output.stderr));
                Outcome {
                    ok: output.status.success(),
                    output: text,
                }
            }
            Err(error) => Outcome {
                ok: false,
                output: format!("{program}: {error}"),
            },
        }
    }
}

/// Why the service could not be put where it was asked to go.
#[derive(Debug)]
pub enum Failed {
    /// This machine cannot do it, or the caller asked for something that is not
    /// a thing. The core's own distinction, carried up unchanged.
    Unavailable(Unavailable),
    /// A file could not be read, written, copied or removed.
    Disk(String),
    /// systemd cannot name this path in a unit at all, and no escaping exists
    /// for it. Not a disk problem and not the manager refusing: a fact about
    /// where somebody put their files.
    Unrepresentable(String),
    /// The service manager ran and refused.
    Command {
        /// What was run.
        argv: Vec<String>,
        /// What it said about it.
        output: String,
    },
}

impl From<Unavailable> for Failed {
    fn from(why: Unavailable) -> Self {
        Failed::Unavailable(why)
    }
}

impl std::fmt::Display for Failed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Failed::Unavailable(why) => write!(f, "{why}"),
            Failed::Disk(what) => f.write_str(what),
            Failed::Unrepresentable(path) => write!(
                f,
                "systemd cannot name a path holding a quote, a backslash or a line break \
                 in a unit, and this one does: {path}"
            ),
            Failed::Command { argv, output } => {
                // Quoted where a word holds a space, because argv was chosen
                // precisely so a plist path with a space in it stays one
                // argument, and a message that joins on spaces gives that back.
                let said: Vec<String> = argv
                    .iter()
                    .map(|word| {
                        if word.contains(' ') {
                            format!("'{word}'")
                        } else {
                            word.clone()
                        }
                    })
                    .collect();
                write!(f, "{} failed: {}", said.join(" "), output.trim())
            }
        }
    }
}

impl std::error::Error for Failed {}

/// What installing a definition did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installed {
    /// Where the definition is now.
    pub path: PathBuf,
    /// Whether anything was written.
    pub action: Install,
    /// Where the definition that was there before was kept, if there was one.
    pub backup: Option<PathBuf>,
}

/// What taking a service off the machine did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Removed {
    /// The definition that is no longer there.
    pub path: PathBuf,
    /// Where a copy of it was kept.
    pub backup: PathBuf,
}

/// What bringing a service up to date did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repaired {
    /// What happened to the file.
    pub installed: Installed,
    /// Whether the service was already up when this started, asked before
    /// anything is written so that the answer describes the machine as it was
    /// found.
    pub was_running: bool,
    /// Whether it was restarted onto the new definition.
    pub restarted: bool,
}

/// A service manager, a place to put definitions, and a way to run commands.
#[derive(Debug, Clone)]
pub struct Services<R> {
    /// Which manager, and how it is addressed.
    pub session: Session,
    /// Where definitions and settings live.
    pub layout: Layout,
    /// How a command gets run.
    pub runner: R,
}

impl<R: Runner> Services<R> {
    /// Put a definition on disk, and say what that did.
    ///
    /// An unchanged definition is left alone: no new timestamp, and no backup
    /// of a file identical to its replacement. That is the difference between
    /// repairing a service and disturbing one, and it also means a directory
    /// somebody has made read-only that already holds the right definition is
    /// not reported as a failure.
    ///
    /// `stamp` names the copy kept of a definition that was there and differed.
    /// It is an argument because reading a clock is not this function's to do,
    /// and because a test that cannot pin the name cannot assert on it.
    pub fn install(&self, definition: &Definition, stamp: &str) -> Result<Installed, Failed> {
        let path = PathBuf::from(
            self.session
                .definition_path(&definition.name, &self.layout)?,
        );
        let rendered = self.session.render(definition, &self.layout)?;

        // systemd unquotes the executable word and then refuses the unit if
        // what comes out holds a quote, a backslash or a control character.
        // There is no escaping for that, so the only honest answers are to name
        // the path or to install a unit that can never start.
        if self.session.manager == Manager::Systemd {
            let mut paths: Vec<&str> = definition.argv.iter().map(String::as_str).collect();
            paths.push(&self.layout.env_file);
            if let Some(refused) = unrepresentable_in_systemd(&paths) {
                return Err(Failed::Unrepresentable(refused.to_string()));
            }
        }

        // Whether a copy is kept turns on whether something is *there*, and
        // reading it only decides whether it needs replacing. Collapsing the two
        // - `read_to_string(&path).ok()` - answers "nothing is there" for a unit
        // that is there and unreadable, and then the rename below destroys it
        // with no copy taken. Bytes that are not text, and a file somebody has
        // made unreadable, are both that case.
        let action = match fs::read_to_string(&path) {
            Ok(text) => plan_install(Some(&text), &rendered),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                plan_install(None, &rendered)
            }
            Err(_) => Install::Replace,
        };
        if action == Install::Unchanged {
            return Ok(Installed {
                path,
                action,
                backup: None,
            });
        }

        let dir = path.parent().unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(dir).map_err(|e| Failed::Disk(format!("{}: {e}", dir.display())))?;

        // The agent names the files it writes, and launchd refuses to start a
        // job whose StandardOutPath cannot be opened. Created rather than hoped
        // for: this is the difference between an agent that runs and one that is
        // installed, reported as installed, and never starts.
        if self.session.manager == Manager::Launchd {
            fs::create_dir_all(&self.layout.log_dir)
                .map_err(|e| Failed::Disk(format!("{}: {e}", self.layout.log_dir)))?;
        }

        // Written first, so that a directory that cannot be written leaves the
        // definition already there untouched rather than backed up and lost.
        // Appended rather than `with_extension`, which would turn
        // ayeaye.service into ayeaye.tmp.N and lose the name being installed.
        let temporary = PathBuf::from(format!("{}.tmp.{}", path.display(), std::process::id()));
        fs::write(&temporary, &rendered)
            .map_err(|e| Failed::Disk(format!("{}: {e}", temporary.display())))?;
        if let Err(why) = set_readable(&temporary) {
            let _ = fs::remove_file(&temporary);
            return Err(why);
        }

        // A definition that is there and different is somebody's — an older
        // version of this program wrote it, or a person edited it. This run is
        // going to replace it either way, but the bytes are cheap to keep.
        let mut backup = None;
        if action == Install::Replace {
            let kept = free_backup_path(&path, stamp);
            if let Err(error) = fs::copy(&path, &kept) {
                let _ = fs::remove_file(&temporary);
                return Err(Failed::Disk(format!(
                    "could not keep a copy of {} at {}: {error}",
                    path.display(),
                    kept.display()
                )));
            }
            backup = Some(kept);
        }

        // A rename, so that a definition is never half-written: whatever is at
        // this path is a whole definition, before and after.
        if let Err(error) = fs::rename(&temporary, &path) {
            let _ = fs::remove_file(&temporary);
            return Err(Failed::Disk(format!("{}: {error}", path.display())));
        }
        Ok(Installed {
            path,
            action,
            backup,
        })
    }

    /// Bring a service up to date: install it, and make that take effect.
    ///
    /// A running service does not pick up a rewritten definition by itself, and
    /// `enable --now` on something already active does nothing at all — so
    /// without this, moving the binary and running setup again would report
    /// success while the old process carried on from the old path. This is the
    /// whole of what "repair" means, and it is the same code as an install.
    pub fn repair(&self, definition: &Definition, stamp: &str) -> Result<Repaired, Failed> {
        let was_running = self.is_running(&definition.name);
        let installed = self.install(definition, stamp)?;
        let changed = installed.action != Install::Unchanged;

        // systemd will not see a new unit file until it is told to look. launchd
        // has no such command and says so, which is not an error here.
        if changed
            && let Ok(argv) = self
                .session
                .command(&definition.name, Operation::Reload, None)
        {
            self.check(argv)?;
        }

        let restarted = if was_running && changed {
            self.restart(&definition.name)?;
            true
        } else {
            false
        };
        Ok(Repaired {
            installed,
            was_running,
            restarted,
        })
    }

    /// Install it, start it, and keep it that way across logins.
    pub fn enable(&self, name: &str) -> Result<(), Failed> {
        // launchd's bootstrap is the enable, and it needs the plist itself:
        // there is no equivalent of a unit name it could look up.
        let path = self.session.definition_path(name, &self.layout)?;
        // Booting out first, and only on launchd: bootstrapping a label launchd
        // already knows fails outright, which would turn every second run into
        // an error and make repairing a broken agent impossible. systemd needs
        // no such thing — `enable --now` on a unit that is already both is a
        // no-op — and disabling it first would stop a service somebody is using
        // in order to tell them it will start at login.
        if self.session.manager == Manager::Launchd {
            self.unregister(name);
        }
        self.check(self.session.command(name, Operation::Enable, Some(&path))?)
    }

    /// Stop it, and do not bring it back.
    pub fn disable(&self, name: &str) -> Result<(), Failed> {
        self.check(self.session.command(name, Operation::Disable, None)?)
    }

    /// Start it now, leaving enablement alone.
    pub fn start(&self, name: &str) -> Result<(), Failed> {
        self.check(self.session.command(name, Operation::Start, None)?)
    }

    /// Stop it now, leaving enablement alone.
    pub fn stop(&self, name: &str) -> Result<(), Failed> {
        self.check(self.session.command(name, Operation::Stop, None)?)
    }

    /// What the service manager says about it, in its own words.
    ///
    /// The outcome is handed back whole rather than turned into an error,
    /// because a non-zero exit here is an *answer*: `systemctl status` exits 3
    /// for a unit that is simply stopped. Wrapping that in "the command failed"
    /// would hide a perfectly good report behind a complaint about it.
    pub fn status(&self, name: &str) -> Result<Outcome, Failed> {
        let argv = self.session.command(name, Operation::Status, None)?;
        Ok(self.runner.run(&argv))
    }

    /// Whether the service manager says the service is up — or, on launchd,
    /// which has no finer answer, that it is registered.
    pub fn is_running(&self, name: &str) -> bool {
        match self.session.command(name, Operation::Status, None) {
            Ok(argv) => self.runner.run(&argv).ok,
            Err(_) => false,
        }
    }

    /// Make a rewritten definition take effect.
    ///
    /// Two platforms, two spellings of one idea. systemd stops and starts,
    /// which is deliberately the pair that leaves enablement alone. launchd has
    /// no restart at all: a property list is re-read by being handed to launchd
    /// again, and a label it already knows has to be booted out first or the
    /// bootstrap fails outright.
    pub fn restart(&self, name: &str) -> Result<(), Failed> {
        match self.session.manager {
            Manager::Systemd => {
                // A stop that fails is a service that was not running, which is
                // not a reason to refuse to start it.
                let _ = self.stop(name);
                self.start(name)
            }
            Manager::Launchd => self.enable(name),
        }
    }

    /// Take the service off this machine: unregister it, keep a copy of its
    /// definition, and delete it.
    ///
    /// The disable is unconditional. Doing it only when the service is *running*
    /// leaves an enabled-but-stopped unit deleted with its `default.target.wants`
    /// symlink still pointing at nothing, which is a service manager complaining
    /// at every login about a unit nobody can find. Disabling something already
    /// disabled is the state being asked for, not a failure.
    ///
    /// A copy is kept for the same reason installing over one keeps a copy: this
    /// may be a definition somebody wrote by hand, and deleting it is cheap to
    /// regret.
    pub fn remove(&self, name: &str, stamp: &str) -> Result<Option<Removed>, Failed> {
        let _ = self.disable(name);
        let path = PathBuf::from(self.session.definition_path(name, &self.layout)?);
        if !path.exists() {
            return Ok(None);
        }
        let kept = free_backup_path(&path, stamp);
        fs::copy(&path, &kept).map_err(|error| {
            Failed::Disk(format!(
                "could not keep a copy of {} at {}: {error}",
                path.display(),
                kept.display()
            ))
        })?;
        match fs::remove_file(&path) {
            Ok(()) => Ok(Some(Removed { path, backup: kept })),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(Failed::Disk(format!("{}: {error}", path.display()))),
        }
    }

    /// Boot a launchd job out if it is registered, and say nothing either way.
    fn unregister(&self, name: &str) {
        if self.is_running(name) {
            let _ = self.disable(name);
        }
    }

    /// Run a command the core built, and keep the manager's own words when it
    /// refuses.
    fn check(&self, argv: Vec<String>) -> Result<(), Failed> {
        let outcome = self.runner.run(&argv);
        if outcome.ok {
            Ok(())
        } else {
            Err(Failed::Command {
                argv,
                output: outcome.output,
            })
        }
    }
}

/// A backup path nothing is using yet.
///
/// Two backups taken inside the same second must not silently replace each
/// other: the two versions are exactly what somebody comparing them needs.
fn free_backup_path(path: &Path, stamp: &str) -> PathBuf {
    let base = format!("{}.{stamp}.bak", path.display());
    let first = PathBuf::from(&base);
    if !first.exists() {
        return first;
    }
    for n in 2u32.. {
        let candidate = PathBuf::from(format!("{base}-{n}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    unreachable!("the range is unbounded")
}

/// A definition is read by the service manager, so it has to be readable.
#[cfg(unix)]
fn set_readable(path: &Path) -> Result<(), Failed> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o644))
        .map_err(|e| Failed::Disk(format!("{}: {e}", path.display())))
}

#[cfg(not(unix))]
fn set_readable(_path: &Path) -> Result<(), Failed> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{Failed, Outcome, Runner, Services};
    use ayeaye_core::service::{Definition, Install, Layout, Session};
    use std::cell::RefCell;
    use std::fs::{self, FileTimes};
    use std::path::{Path, PathBuf};
    use std::time::{Duration, SystemTime};

    /// The unit the shell installer leaves behind, as
    /// `tests/fixtures/units/ayeaye-systemd` has it. The migration case is
    /// "somebody already ran the old installer", so the old installer's own
    /// golden file is the only honest starting state.
    const PREVIOUS_INSTALL: &str = include_str!("../../../tests/fixtures/units/ayeaye-systemd");

    /// A recording service manager.
    ///
    /// It answers `status` with whatever `running` is set to, which is the one
    /// question every other operation branches on.
    #[derive(Default)]
    struct Fake {
        calls: RefCell<Vec<Vec<String>>>,
        running: RefCell<bool>,
        refuse: RefCell<Option<String>>,
    }

    impl Fake {
        fn running() -> Self {
            Fake {
                running: RefCell::new(true),
                ..Fake::default()
            }
        }

        /// Every command run, each joined with a space for legibility.
        fn calls(&self) -> Vec<String> {
            self.calls
                .borrow()
                .iter()
                .map(|argv| argv.join(" "))
                .collect()
        }

        fn ran(&self, wanted: &str) -> bool {
            self.calls().iter().any(|call| call == wanted)
        }
    }

    impl Runner for Fake {
        fn run(&self, argv: &[String]) -> Outcome {
            self.calls.borrow_mut().push(argv.to_vec());
            let joined = argv.join(" ");
            if let Some(refused) = self.refuse.borrow().as_deref()
                && joined.contains(refused)
            {
                return Outcome {
                    ok: false,
                    output: "Job for ayeaye.service failed".to_string(),
                };
            }
            let asked_whether_it_is_up =
                argv.contains(&"status".to_string()) || argv.contains(&"print".to_string());
            Outcome {
                ok: !asked_whether_it_is_up || *self.running.borrow(),
                output: String::new(),
            }
        }
    }

    /// A throwaway home directory. Writing definitions is the whole job, so the
    /// only honest way to test it is to give it somewhere to write them.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("ayeaye-service-{}-{name}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).expect("a temporary directory");
            Scratch(dir)
        }

        fn path(&self) -> &Path {
            &self.0
        }

        fn layout(&self) -> Layout {
            let home = self.0.to_string_lossy().into_owned();
            Layout::new(
                &home,
                &format!("{home}/.config"),
                &format!("{home}/.local/state"),
            )
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn definition() -> Definition {
        Definition::ayeaye("/opt/ayeaye/bin/ayeaye")
    }

    fn systemd(scratch: &Scratch, runner: Fake) -> Services<Fake> {
        Services {
            session: Session::systemd(),
            layout: scratch.layout(),
            runner,
        }
    }

    fn launchd(scratch: &Scratch, runner: Fake) -> Services<Fake> {
        Services {
            session: Session::launchd("501"),
            layout: scratch.layout(),
            runner,
        }
    }

    /// Where this session would install ayeaye's definition.
    fn unit_path(services: &Services<Fake>) -> PathBuf {
        PathBuf::from(
            services
                .session
                .definition_path("ayeaye", &services.layout)
                .expect("a path"),
        )
    }

    /// Every file in a directory, sorted.
    fn listing(dir: &Path) -> Vec<String> {
        let Ok(entries) = fs::read_dir(dir) else {
            return Vec::new();
        };
        let mut names: Vec<String> = entries
            .map(|e| {
                e.expect("an entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        names.sort();
        names
    }

    // AYEAYE-61 — the file that lands is the definition the core rendered, at
    // the path the core named.
    #[test]
    fn install_writes_the_rendered_definition_where_the_core_said_it_goes() {
        let scratch = Scratch::new("writes");
        let services = systemd(&scratch, Fake::default());
        let installed = services
            .install(&definition(), "20260810-120000")
            .expect("an install");

        assert_eq!(installed.action, Install::Create);
        assert_eq!(installed.backup, None);
        assert_eq!(
            installed.path,
            PathBuf::from(format!(
                "{}/.config/systemd/user/ayeaye.service",
                scratch.path().display()
            ))
        );
        assert_eq!(
            fs::read_to_string(&installed.path).expect("the unit"),
            services
                .session
                .render(&definition(), &services.layout)
                .expect("a definition")
        );
    }

    // AYEAYE-61 — a definition the service manager cannot read is a service
    // that does not start, for a reason nobody would guess.
    #[cfg(unix)]
    #[test]
    fn an_installed_definition_is_readable() {
        use std::os::unix::fs::PermissionsExt;
        let scratch = Scratch::new("mode");
        let services = systemd(&scratch, Fake::default());
        let installed = services
            .install(&definition(), "stamp")
            .expect("an install");
        let mode = fs::metadata(&installed.path)
            .expect("metadata")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o644, "mode was {:o}", mode & 0o777);
    }

    // AYEAYE-61 — leaving an unchanged definition alone is the difference
    // between repairing a service and disturbing one.
    #[test]
    fn installing_the_same_definition_twice_leaves_the_file_alone() {
        let scratch = Scratch::new("twice");
        let services = systemd(&scratch, Fake::default());
        let first = services
            .install(&definition(), "stamp")
            .expect("an install");

        // Asserted on the file's own timestamp rather than on the reported
        // action. `Unchanged` is what this returns whether or not it wrote the
        // same bytes over the top, and writing them is exactly what must not
        // happen: a rewritten unit is a new mtime, and somebody reading `ls -l`
        // to find out when their service last changed is told a lie.
        let long_ago = SystemTime::now() - Duration::from_secs(86_400);
        let handle = fs::File::options()
            .write(true)
            .open(&first.path)
            .expect("the unit");
        handle
            .set_times(
                FileTimes::new()
                    .set_accessed(long_ago)
                    .set_modified(long_ago),
            )
            .expect("an older timestamp");
        drop(handle);

        let again = services
            .install(&definition(), "stamp")
            .expect("an install");

        assert_eq!(again.action, Install::Unchanged);
        assert_eq!(again.backup, None);
        assert_eq!(
            fs::metadata(&first.path)
                .expect("metadata")
                .modified()
                .expect("an mtime"),
            long_ago,
            "an unchanged definition must not be rewritten"
        );
        assert_eq!(
            listing(first.path.parent().expect("a directory")),
            vec!["ayeaye.service"],
            "and must not leave a backup or a temporary file behind"
        );
    }

    // AYEAYE-61 — the migration case. Somebody ran the shell installer; the
    // binary must take that unit over rather than install a second one beside
    // it, because two units under one name is two ayeayes fighting for a port.
    #[test]
    fn a_definition_from_a_previous_install_is_replaced_not_duplicated() {
        let scratch = Scratch::new("migrate");
        let services = systemd(&scratch, Fake::default());
        let path = PathBuf::from(
            services
                .session
                .definition_path("ayeaye", &services.layout)
                .expect("a path"),
        );
        let previous = PREVIOUS_INSTALL
            .replace("@REPO@", "/home/someone/ayeaye")
            .replace(
                "@ENV@",
                &format!("{}/.config/ayeaye/env", scratch.path().display()),
            );
        fs::create_dir_all(path.parent().expect("a directory")).expect("the unit directory");
        fs::write(&path, &previous).expect("the previous install");

        let installed = services
            .install(&definition(), "20260810-120000")
            .expect("an install");

        assert_eq!(installed.action, Install::Replace);
        assert_eq!(installed.path, path, "the same unit name, at the same path");
        let units: Vec<String> = listing(path.parent().expect("a directory"))
            .into_iter()
            .filter(|name| name.ends_with(".service"))
            .collect();
        assert_eq!(units, vec!["ayeaye.service"], "one definition, not two");
        assert!(
            fs::read_to_string(&path)
                .expect("the unit")
                .contains("ExecStart=/opt/ayeaye/bin/ayeaye"),
            "the unit still names the path the old installer used"
        );
        let backup = installed.backup.expect("a copy of what was there");
        assert_eq!(fs::read_to_string(&backup).expect("the copy"), previous);
    }

    // AYEAYE-61 — two backups in the same second are two versions somebody
    // may need to compare, so neither may quietly replace the other.
    #[test]
    fn a_second_backup_in_the_same_second_does_not_replace_the_first() {
        let scratch = Scratch::new("backups");
        let services = systemd(&scratch, Fake::default());
        let path = PathBuf::from(
            services
                .session
                .definition_path("ayeaye", &services.layout)
                .expect("a path"),
        );
        fs::create_dir_all(path.parent().expect("a directory")).expect("the unit directory");

        fs::write(&path, "one\n").expect("a definition");
        let first = services
            .install(&definition(), "stamp")
            .expect("an install");
        fs::write(&path, "two\n").expect("another definition");
        let second = services
            .install(&definition(), "stamp")
            .expect("an install");

        assert_ne!(first.backup, second.backup);
        assert_eq!(
            fs::read_to_string(first.backup.expect("a copy")).expect("the copy"),
            "one\n"
        );
        assert_eq!(
            fs::read_to_string(second.backup.expect("a copy")).expect("the copy"),
            "two\n"
        );
    }

    // AYEAYE-61 — the five operations the ticket names, each reaching the
    // service manager as the command the core built.
    #[test]
    fn enable_start_stop_status_and_disable_reach_the_service_manager() {
        let scratch = Scratch::new("operations");
        let services = systemd(&scratch, Fake::running());
        services
            .install(&definition(), "stamp")
            .expect("an install");

        services.enable("ayeaye").expect("enable");
        services.start("ayeaye").expect("start");
        services.stop("ayeaye").expect("stop");
        services.status("ayeaye").expect("a status");
        services.disable("ayeaye").expect("disable");

        assert!(
            services
                .runner
                .ran("systemctl --user enable --now ayeaye.service")
        );
        assert!(services.runner.ran("systemctl --user start ayeaye.service"));
        assert!(services.runner.ran("systemctl --user stop ayeaye.service"));
        assert!(
            services
                .runner
                .ran("systemctl --user status ayeaye.service")
        );
        assert!(
            services
                .runner
                .ran("systemctl --user disable --now ayeaye.service")
        );
    }

    // AYEAYE-61 — `systemctl --user enable --now` on a unit that is already
    // enabled and running is a no-op. Booting it out first, the way launchd
    // has to be, would stop a service somebody is using in order to tell them
    // it will start at login.
    #[test]
    fn enabling_a_running_systemd_service_does_not_stop_it_first() {
        let scratch = Scratch::new("enable-running");
        let services = systemd(&scratch, Fake::running());
        services
            .install(&definition(), "stamp")
            .expect("an install");

        services.enable("ayeaye").expect("enable");

        assert!(
            services
                .runner
                .ran("systemctl --user enable --now ayeaye.service")
        );
        assert!(
            !services
                .runner
                .ran("systemctl --user disable --now ayeaye.service"),
            "enable must not disable first on systemd"
        );
        assert!(!services.runner.ran("systemctl --user stop ayeaye.service"));
    }

    // AYEAYE-61 — a running service does not pick up a rewritten definition by
    // itself. Without this, moving the binary and running setup again reports
    // success while the old process carries on from the old path.
    #[test]
    fn a_service_that_was_running_is_restarted_onto_the_new_definition() {
        let scratch = Scratch::new("repair");
        let services = systemd(&scratch, Fake::running());
        services
            .install(&Definition::ayeaye("/old/bin/ayeaye"), "stamp")
            .expect("an install");

        let repaired = services.repair(&definition(), "stamp").expect("a repair");

        assert!(repaired.was_running);
        assert!(repaired.restarted);
        assert_eq!(repaired.installed.action, Install::Replace);
        assert!(
            services.runner.ran("systemctl --user daemon-reload"),
            "systemd will not see a new unit file until it is told to look"
        );
        assert!(services.runner.ran("systemctl --user stop ayeaye.service"));
        assert!(services.runner.ran("systemctl --user start ayeaye.service"));
    }

    // AYEAYE-61 — and a definition that did not change is not an excuse to
    // interrupt somebody's running service.
    #[test]
    fn a_service_whose_definition_did_not_change_is_left_running() {
        let scratch = Scratch::new("undisturbed");
        let services = systemd(&scratch, Fake::running());
        services
            .install(&definition(), "stamp")
            .expect("an install");

        let repaired = services.repair(&definition(), "stamp").expect("a repair");

        assert!(repaired.was_running);
        assert!(!repaired.restarted);
        assert_eq!(repaired.installed.action, Install::Unchanged);
        assert!(!services.runner.ran("systemctl --user stop ayeaye.service"));
        assert!(
            !services.runner.ran("systemctl --user daemon-reload"),
            "nothing changed, so there is nothing to re-read"
        );
    }

    // AYEAYE-61 — launchd has no restart and no daemon-reload: a property list
    // is re-read by being handed to launchd again, and a label it already knows
    // has to be booted out first or the bootstrap fails outright.
    #[test]
    fn launchd_is_restarted_by_being_bootstrapped_again() {
        let scratch = Scratch::new("launchd");
        let services = launchd(&scratch, Fake::running());
        services
            .install(&Definition::ayeaye("/old/bin/ayeaye"), "stamp")
            .expect("an install");

        let repaired = services.repair(&definition(), "stamp").expect("a repair");

        assert!(repaired.restarted);
        let plist = format!(
            "{}/Library/LaunchAgents/dev.ayeaye.plist",
            scratch.path().display()
        );
        assert!(services.runner.ran("launchctl bootout gui/501/dev.ayeaye"));
        assert!(
            services
                .runner
                .ran(&format!("launchctl bootstrap gui/501 {plist}"))
        );
        let calls = services.runner.calls();
        let bootout = calls
            .iter()
            .position(|c| c.contains("bootout"))
            .expect("a bootout");
        let bootstrap = calls
            .iter()
            .position(|c| c.contains("bootstrap"))
            .expect("a bootstrap");
        assert!(
            bootout < bootstrap,
            "bootstrapping a known label fails outright"
        );
        assert!(
            !calls.iter().any(|c| c.contains("daemon-reload")),
            "launchd has no daemon-reload and must not be given one"
        );
    }

    // AYEAYE-61 — the binary can take itself off the machine again.
    #[test]
    fn remove_unregisters_the_service_and_deletes_its_definition() {
        let scratch = Scratch::new("remove");
        let services = systemd(&scratch, Fake::running());
        let installed = services
            .install(&definition(), "stamp")
            .expect("an install");

        let removed = services
            .remove("ayeaye", "stamp")
            .expect("a removal")
            .expect("a definition");

        assert_eq!(removed.path, installed.path);
        assert!(!installed.path.exists());
        assert!(
            services
                .runner
                .ran("systemctl --user disable --now ayeaye.service")
        );
        assert_eq!(
            fs::read_to_string(&removed.backup).expect("the copy"),
            services
                .session
                .render(&definition(), &services.layout)
                .expect("a definition"),
            "a definition somebody may have written by hand is cheap to keep"
        );
    }

    // AYEAYE-61 — disabling only when the service is *running* leaves an
    // enabled-but-stopped unit deleted with its default.target.wants symlink
    // still pointing at nothing, which the service manager complains about at
    // every login.
    #[test]
    fn remove_disables_a_service_that_is_installed_but_not_running() {
        let scratch = Scratch::new("remove-stopped");
        let services = systemd(&scratch, Fake::default());
        services
            .install(&definition(), "stamp")
            .expect("an install");

        services.remove("ayeaye", "stamp").expect("a removal");

        assert!(
            services
                .runner
                .ran("systemctl --user disable --now ayeaye.service"),
            "a stopped unit may still be enabled"
        );
    }

    // AYEAYE-61 — removing a service that is not there is the state being
    // asked for, not a failure.
    #[test]
    fn removing_a_service_that_is_not_installed_is_not_a_failure() {
        let scratch = Scratch::new("remove-absent");
        let services = systemd(&scratch, Fake::default());
        assert_eq!(services.remove("ayeaye", "stamp").expect("a removal"), None);
    }

    // AYEAYE-61 — the case that loses somebody's unit. Deciding whether a copy
    // is needed by whether the file could be *read* answers "nothing is there"
    // for a definition that is there and unreadable, and the rename then
    // destroys it with no copy taken.
    #[test]
    fn a_definition_that_is_there_but_not_text_is_still_replaced_with_a_copy_kept() {
        let scratch = Scratch::new("not-text");
        let services = systemd(&scratch, Fake::default());
        let path = unit_path(&services);
        fs::create_dir_all(path.parent().expect("a directory")).expect("the unit directory");
        let bytes: &[u8] = &[b'[', b'U', 0xff, 0xfe, b']', b'\n'];
        fs::write(&path, bytes).expect("a unit that is not UTF-8");

        let installed = services
            .install(&definition(), "stamp")
            .expect("an install");

        assert_eq!(installed.action, Install::Replace);
        let backup = installed.backup.expect("a copy of what was there");
        assert_eq!(fs::read(&backup).expect("the copy"), bytes);
    }

    // AYEAYE-61 — and when the copy cannot be taken at all, the definition
    // already there is left alone rather than replaced blind.
    #[cfg(unix)]
    #[test]
    fn a_definition_whose_copy_cannot_be_taken_is_left_alone() {
        use std::os::unix::fs::PermissionsExt;
        let scratch = Scratch::new("unreadable");
        let services = systemd(&scratch, Fake::default());
        let path = unit_path(&services);
        fs::create_dir_all(path.parent().expect("a directory")).expect("the unit directory");
        fs::write(&path, "somebody's unit\n").expect("a unit");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).expect("unreadable");

        let refused = services.install(&definition(), "stamp");

        // Running as root defeats the permission bit, and a test that quietly
        // passes because it could read the file after all is worse than none.
        if fs::read_to_string(&path).is_ok() {
            return;
        }
        assert!(matches!(refused, Err(Failed::Disk(_))), "{refused:?}");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).expect("readable again");
        assert_eq!(
            fs::read_to_string(&path).expect("the unit"),
            "somebody's unit\n",
            "the definition that was there must survive a refusal"
        );
    }

    // AYEAYE-61 — launchd refuses to start a job whose StandardOutPath cannot
    // be opened, so an agent installed into a home with no log directory is
    // installed, reported as installed, and never starts.
    #[test]
    fn installing_an_agent_creates_the_directory_it_writes_its_log_into() {
        let scratch = Scratch::new("logs");
        let services = launchd(&scratch, Fake::default());
        let installed = services
            .install(&definition(), "stamp")
            .expect("an install");

        assert!(
            Path::new(&services.layout.log_dir).is_dir(),
            "the agent names {}/ayeaye.log and nothing created it",
            services.layout.log_dir
        );
        assert!(
            fs::read_to_string(&installed.path)
                .expect("the agent")
                .contains(&format!(
                    "<string>{}/ayeaye.log</string>",
                    services.layout.log_dir
                ))
        );
    }

    // AYEAYE-61 — the launchd half of "migrated rather than duplicated". It is
    // where the risk actually lives: the label and the filename are one string,
    // and a definition installed under one name beside another under a second
    // is two agents where somebody has one.
    #[test]
    fn an_agent_from_a_previous_install_is_replaced_not_duplicated() {
        let scratch = Scratch::new("migrate-agent");
        let services = launchd(&scratch, Fake::default());
        let path = unit_path(&services);
        fs::create_dir_all(path.parent().expect("a directory")).expect("the agents directory");
        fs::write(&path, "<plist>an older agent</plist>\n").expect("the previous install");

        let installed = services
            .install(&definition(), "stamp")
            .expect("an install");

        assert_eq!(installed.action, Install::Replace);
        assert_eq!(installed.path, path);
        assert_eq!(
            listing(path.parent().expect("a directory"))
                .into_iter()
                .filter(|name| name.ends_with(".plist"))
                .collect::<Vec<String>>(),
            vec!["dev.ayeaye.plist"],
            "one agent, not two"
        );
        assert!(
            fs::read_to_string(&path)
                .expect("the agent")
                .contains("<string>dev.ayeaye</string>"),
            "the label is the stem of the file it is installed as"
        );
    }

    // AYEAYE-61 — systemd has no escaping for a quote, a backslash or a line
    // break in an executable path: it rejects the unit outright with
    // "Executable path contains special characters". Naming the path beats
    // installing something that can never start.
    #[test]
    fn a_path_systemd_cannot_express_is_named_rather_than_installed() {
        let scratch = Scratch::new("unrepresentable");
        let services = systemd(&scratch, Fake::default());
        let awkward = Definition::ayeaye("/opt/o\"d/bin/ayeaye");

        match services.install(&awkward, "stamp") {
            Err(Failed::Unrepresentable(path)) => assert_eq!(path, "/opt/o\"d/bin/ayeaye"),
            other => panic!("expected the path to be named, got {other:?}"),
        }
        assert!(listing(Path::new(&services.layout.unit_dir)).is_empty());
        // launchd has no such limit — XML says what it means about any of
        // these — so the refusal is systemd's rule and not this program's.
        launchd(&scratch, Fake::default())
            .install(&awkward, "stamp")
            .expect("an agent");
    }

    // AYEAYE-61 — `systemctl status` exits 3 for a unit that is simply
    // stopped. That is an answer, and turning it into "the command failed"
    // hides a perfectly good report behind a complaint about it.
    #[test]
    fn status_reports_a_stopped_service_rather_than_failing_to_ask() {
        let scratch = Scratch::new("status");
        let stopped = systemd(&scratch, Fake::default());
        assert!(
            !stopped.status("ayeaye").expect("an answer").ok,
            "a stopped unit is not a successful exit, and is still an answer"
        );
        assert!(
            systemd(&scratch, Fake::running())
                .status("ayeaye")
                .expect("an answer")
                .ok
        );
    }

    // AYEAYE-61 — the stamp is what keeps yesterday's copy from being
    // overwritten by today's.
    #[test]
    fn a_kept_copy_is_named_after_the_stamp_it_was_given() {
        let scratch = Scratch::new("stamped");
        let services = systemd(&scratch, Fake::default());
        let path = unit_path(&services);
        fs::create_dir_all(path.parent().expect("a directory")).expect("the unit directory");
        fs::write(&path, "an older unit\n").expect("a unit");

        let installed = services
            .install(&definition(), "20260810-120000")
            .expect("an install");

        let backup = installed.backup.expect("a copy");
        assert!(
            backup.to_string_lossy().contains("20260810-120000"),
            "the copy is at {}",
            backup.display()
        );
    }

    // AYEAYE-61 — at the moment something fails, the service manager's own
    // words are worth more to whoever has to fix it than any paraphrase.
    #[test]
    fn a_refusal_carries_the_service_managers_own_words() {
        let scratch = Scratch::new("refusal");
        let runner = Fake::default();
        *runner.refuse.borrow_mut() = Some("start".to_string());
        let services = systemd(&scratch, runner);

        match services.start("ayeaye") {
            Err(Failed::Command { argv, output }) => {
                assert_eq!(argv.join(" "), "systemctl --user start ayeaye.service");
                assert!(output.contains("Job for ayeaye.service failed"));
            }
            other => panic!("expected the manager's refusal, got {other:?}"),
        }
    }

    // AYEAYE-61 — a definition with no program is refused by the core, and the
    // refusal has to arrive here rather than becoming an empty file on disk.
    #[test]
    fn a_definition_the_core_refuses_never_reaches_the_disk() {
        let scratch = Scratch::new("refused");
        let services = systemd(&scratch, Fake::default());
        let mut broken = definition();
        broken.argv.clear();

        assert!(matches!(
            services.install(&broken, "stamp"),
            Err(Failed::Unavailable(_))
        ));
        assert!(listing(Path::new(&scratch.layout().unit_dir)).is_empty());
    }
}
