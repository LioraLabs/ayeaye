//! Acquiring models, and keeping them somewhere a load can find them.
//!
//! The decisions are all next door in `ayeaye_core::model` — what a model id
//! may be, which URL a file comes from, whether the architecture is one this
//! build implements, whether what arrived is the file it claims to be. What is
//! left here is the part that cannot be decided, only done: running a program,
//! writing bytes, and renaming a directory.
//!
//! **The transport is `curl`, and that is a decision with a measurement behind
//! it rather than a shrug.** An in-process HTTP client needs TLS, every TLS
//! stack in the ecosystem reaches `ring` or `aws-lc-sys`, and both of those put
//! `cc` in `Cargo.lock` — `ring` even when no feature enables it, because a
//! lockfile carries optional dependencies too. The constitution's rule 4 reads
//! that lockfile and refuses the build, and it is right to: the mechanical
//! proof that nothing in the graph compiles C *is* the single-binary milestone.
//! So the network stays where the effect budget already says it belongs. `curl`
//! is in the repo's package map already, and this is a setup-time act — the
//! daemon serves without it, and only acquiring needs it.

use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use ayeaye_core::cleanup::{Policy as CleanupPolicy, PolicyError};
use ayeaye_core::model::hub::{self, Wanted};
use ayeaye_core::model::residency::{self, Plan, Policy};
use ayeaye_core::model::settings::{self, BadSetting, ModelSettings};
use ayeaye_core::model::verify::{self, Unusable};
use ayeaye_core::model::{Architecture, ModelId, Unsupported, architecture};
use ayeaye_infer::{LanguageError, LanguageSlot, SpeechError, SpeechSlot};

/// Something that can fetch one URL into one file.
///
/// A trait with one operation, so a test can watch *which* URLs were asked for
/// and in what order — which is the only way to observe the property this whole
/// ticket is about, namely that nothing large is fetched before the
/// architecture has been judged.
pub trait Fetcher {
    /// Fetch `url` into `into`, or say why not.
    fn get(&self, url: &str, into: &Path) -> Result<(), String>;
}

/// The real one.
#[derive(Debug, Clone, Copy, Default)]
pub struct Curl;

impl Curl {
    /// The command line, as its own function so the flags are readable and so a
    /// test can be written against them without running anything.
    ///
    /// `--fail` matters more than it looks: without it curl writes the server's
    /// error page to the output file and exits 0, which is precisely how a 404
    /// page ends up saved as `model.safetensors`. `--location` matters too —
    /// the hub answers `/resolve/` with a redirect to its CDN, so without it
    /// every download is a redirect notice.
    ///
    /// The `--` is not decoration either. curl reads options at any position,
    /// not only before the first non-option, so a hub configured as
    /// `-o/somewhere` arrives as a *flag* rather than as a URL. Ending the
    /// options is what makes "the URL is the last argument" actually mean the
    /// URL is data.
    pub fn argv(url: &str, into: &Path) -> Vec<String> {
        [
            "curl",
            "--fail",
            "--location",
            "--silent",
            "--show-error",
            "--connect-timeout",
            "30",
            "--output",
        ]
        .iter()
        .map(|arg| (*arg).to_string())
        .chain([
            into.to_string_lossy().into_owned(),
            "--".to_string(),
            url.to_string(),
        ])
        .collect()
    }
}

impl Fetcher for Curl {
    fn get(&self, url: &str, into: &Path) -> Result<(), String> {
        let argv = Curl::argv(url, into);
        let ran = Command::new(&argv[0])
            .args(&argv[1..])
            .output()
            .map_err(|why| format!("could not run curl: {why}"))?;
        if ran.status.success() {
            return Ok(());
        }
        // curl's own words, because at the moment something fails they are
        // worth more to whoever has to fix it than any paraphrase.
        let said = String::from_utf8_lossy(&ran.stderr).trim().to_string();
        Err(if said.is_empty() {
            format!("curl exited {}", ran.status)
        } else {
            said
        })
    }
}

/// Why a model could not be acquired.
#[derive(Debug)]
pub enum PullError {
    /// The architecture is not one this build implements. **Raised before any
    /// weights are fetched**, which is the point of the whole arrangement.
    Unsupported(Unsupported),
    /// A file could not be fetched.
    Fetch {
        /// Where it was being fetched from.
        url: String,
        /// What the transport said.
        why: String,
    },
    /// A file arrived and is not what it claims to be.
    Unusable(Unusable),
    /// The filesystem refused something.
    Disk {
        /// What was being attempted.
        what: String,
        /// What the filesystem said.
        why: String,
    },
}

impl fmt::Display for PullError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PullError::Unsupported(why) => write!(out, "{why}"),
            PullError::Fetch { url, why } => write!(out, "could not fetch {url}: {why}"),
            PullError::Unusable(why) => write!(out, "{why}"),
            PullError::Disk { what, why } => write!(out, "could not {what}: {why}"),
        }
    }
}

impl std::error::Error for PullError {}

/// A model that is now on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pulled {
    /// Which model.
    pub id: ModelId,
    /// Where its files are.
    pub dir: PathBuf,
    /// What it turned out to be.
    pub architecture: Architecture,
    /// How much was fetched, in bytes.
    pub bytes: u64,
}

/// Fetch a model into `store`, checking the architecture before the weights.
///
/// The order is the ticket. `config.json` is a couple of kilobytes; it is
/// fetched, verified, and judged, and only if it is something this build can
/// run is anything large asked for. An unsupported model therefore costs a
/// round trip rather than a download, and that bound is what makes this
/// different from an open-ended registry.
///
/// Files land in a staging directory and are renamed into place at the end, so
/// an interrupted pull never leaves a half-model somewhere a load would find
/// it and fail halfway through.
pub fn pull(
    fetcher: &impl Fetcher,
    store: &Path,
    hub_host: &str,
    id: &ModelId,
) -> Result<Pulled, PullError> {
    let staging = staging_dir(store, id);
    let disk = |what: String| {
        move |why: std::io::Error| PullError::Disk {
            what,
            why: why.to_string(),
        }
    };

    // A leftover from a previous interrupted run is not something to resume:
    // there is no way to tell which of its files are whole.
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging).map_err(disk(format!("create {}", staging.display())))?;

    let outcome = fetch_all(fetcher, &staging, hub_host, id);
    let (architecture, bytes) = match outcome {
        Ok(both) => both,
        Err(why) => {
            // Nothing half-fetched is left behind for a later run to mistake
            // for a model.
            let _ = std::fs::remove_dir_all(&staging);
            return Err(why);
        }
    };

    let dir = store.join(id.relative_dir());
    install(&staging, &dir)?;
    Ok(Pulled {
        id: id.clone(),
        dir,
        architecture,
        bytes,
    })
}

/// Fetch every wanted file into `staging`, stopping at the one that decides.
fn fetch_all(
    fetcher: &impl Fetcher,
    staging: &Path,
    hub_host: &str,
    id: &ModelId,
) -> Result<(Architecture, u64), PullError> {
    let mut architecture = None;
    let mut bytes = 0u64;

    for wanted in hub::WANTED {
        let url = hub::url(hub_host, id, wanted);
        let into = staging.join(wanted.file);
        fetcher
            .get(&url, &into)
            .map_err(|why| PullError::Fetch { url, why })?;

        let contents = std::fs::read(&into).map_err(|why| PullError::Disk {
            what: format!("read back {}", into.display()),
            why: why.to_string(),
        })?;
        verify::check(wanted.file, &contents).map_err(PullError::Unusable)?;
        bytes += contents.len() as u64;

        if wanted.decides {
            architecture = Some(judge(&contents, wanted)?);
        }
    }

    // Belt to the ordering in `WANTED`: if nothing decided, nothing judged the
    // architecture, and a pull that skipped the whole point of itself must not
    // quietly succeed.
    let architecture = architecture.ok_or(PullError::Unsupported(Unsupported { found: None }))?;
    Ok((architecture, bytes))
}

/// Judge the file that decides.
fn judge(contents: &[u8], wanted: &Wanted) -> Result<Architecture, PullError> {
    let text = std::str::from_utf8(contents).map_err(|why| {
        PullError::Unusable(Unusable::Malformed {
            file: wanted.file.to_string(),
            why: format!("is not text: {why}"),
        })
    })?;
    architecture::in_config(text).map_err(PullError::Unsupported)
}

/// Move a finished staging directory into its final place.
///
/// The old one is moved aside before the new one is moved in, and only removed
/// once that has worked. Removing first would mean a failed rename leaves the
/// machine with no model where it used to have a working one.
///
/// `pub(crate)` so the suite can reach it: the restore path only runs when a
/// rename fails, which no fetch-level fault injection can produce, and a branch
/// whose whole purpose is to save somebody's working model is not one to ship
/// having never watched it run.
pub(crate) fn install(staging: &Path, dir: &Path) -> Result<(), PullError> {
    let disk = |what: String| {
        move |why: std::io::Error| PullError::Disk {
            what,
            why: why.to_string(),
        }
    };
    if let Some(parent) = dir.parent() {
        std::fs::create_dir_all(parent).map_err(disk(format!("create {}", parent.display())))?;
    }

    // Not `with_extension`, which truncates at the last dot and would turn the
    // revision `v1.0` into `v1.replaced`. The `~` matters too: it is outside
    // the characters `ModelId::parse` admits, so a crash between the two
    // renames leaves something `ayeaye model ls` will not show as a phantom
    // model.
    let displaced = match dir.file_name() {
        Some(name) => dir.with_file_name(format!("{}~replaced", name.to_string_lossy())),
        None => {
            return Err(PullError::Disk {
                what: format!("replace {}", dir.display()),
                why: "it has no name".to_string(),
            });
        }
    };
    let had_one = dir.exists();
    if had_one {
        let _ = std::fs::remove_dir_all(&displaced);
        std::fs::rename(dir, &displaced).map_err(disk(format!("move {} aside", dir.display())))?;
    }
    match std::fs::rename(staging, dir) {
        Ok(()) => {
            let _ = std::fs::remove_dir_all(&displaced);
            Ok(())
        }
        Err(why) => {
            // Put back what was there, so a failure costs nothing rather than
            // the model that was working a moment ago.
            if had_one {
                let _ = std::fs::rename(&displaced, dir);
            }
            Err(PullError::Disk {
                what: format!("move {} into {}", staging.display(), dir.display()),
                why: why.to_string(),
            })
        }
    }
}

/// Where a pull assembles a model before it is anything.
///
/// Under the store rather than in `/tmp`, deliberately: the rename at the end
/// has to be a rename and not a copy, and across filesystems it would be a
/// copy — of hundreds of megabytes, with a window in the middle where the
/// directory is half a model.
fn staging_dir(store: &Path, id: &ModelId) -> PathBuf {
    store.join("models").join(format!(
        ".pulling-{}-{}-{}-{}",
        id.owner(),
        id.name(),
        id.revision(),
        std::process::id()
    ))
}

/// Every model in the store, in a stable order.
///
/// Sorted, because this is printed: an order that depends on what the
/// filesystem felt like returning makes two runs disagree for no reason.
pub fn installed(store: &Path) -> Vec<ModelId> {
    let root = store.join("models");
    let mut found = Vec::new();
    let Ok(owners) = std::fs::read_dir(&root) else {
        return found;
    };
    for owner in owners.flatten() {
        let Ok(names) = std::fs::read_dir(owner.path()) else {
            continue;
        };
        for name in names.flatten() {
            let Ok(revisions) = std::fs::read_dir(name.path()) else {
                continue;
            };
            for revision in revisions.flatten() {
                let spelled = format!(
                    "{}/{}@{}",
                    owner.file_name().to_string_lossy(),
                    name.file_name().to_string_lossy(),
                    revision.file_name().to_string_lossy()
                );
                // Parsed rather than trusted: a directory somebody made by hand
                // is not a model id, and this is the one place the two meet.
                if let Ok(id) = ModelId::parse(&spelled) {
                    found.push(id);
                }
            }
        }
    }
    found.sort();
    found
}

/// Remove a model from the store, saying whether there was one.
pub fn remove(store: &Path, id: &ModelId) -> Result<bool, PullError> {
    let dir = store.join(id.relative_dir());
    if !dir.exists() {
        return Ok(false);
    }
    std::fs::remove_dir_all(&dir).map_err(|why| PullError::Disk {
        what: format!("remove {}", dir.display()),
        why: why.to_string(),
    })?;
    Ok(true)
}

/// The configuration file, read into settings, with the environment on top.
///
/// A file that is not there is not an error: it is a machine nobody has
/// configured yet, which is every machine the first time.
pub fn settings(config_file: &Path) -> Result<ModelSettings, BadSetting> {
    let text = std::fs::read_to_string(config_file).unwrap_or_default();
    ModelSettings::resolve(crate::config::env_var, &text)
}

/// How a cleanup pass is configured, from the file with the environment on top.
///
/// The same precedence `settings` reads under, and for the same reason: under
/// the service unit the file has *become* the environment by the time this runs,
/// and run by hand it has not.
///
/// Separate from [`settings`] because it answers a different question and fails
/// for different reasons — a template nobody implements is refused by name,
/// where an absent model is simply a machine nobody has configured yet. Reading
/// it through `Policy::resolve` rather than assembling a `Policy` by hand is
/// what keeps `CLEANUP_ECHOES` paired with the prompt it belongs to, and what
/// makes `CLEANUP_TEMPLATE` and `CLEANUP_MAX_TOKENS` mean anything at all.
pub fn cleanup_policy(config_file: &Path) -> Result<CleanupPolicy, PolicyError> {
    let text = std::fs::read_to_string(config_file).unwrap_or_default();
    let from_file = settings::parse_env_file(&text);
    CleanupPolicy::resolve(|name| {
        crate::config::env_var(name).or_else(|| {
            // The last occurrence, as systemd's `EnvironmentFile=` resolves it.
            from_file
                .iter()
                .rev()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.clone())
        })
    })
}

/// Write one setting into the configuration file, leaving the rest alone.
///
/// Read, change one key, write the whole file back. Not an append: appending
/// leaves the old value above the new one, and the file then says two things.
pub fn choose(config_file: &Path, key: &str, value: &str) -> Result<(), PullError> {
    let disk = |what: String| {
        move |why: std::io::Error| PullError::Disk {
            what,
            why: why.to_string(),
        }
    };
    if let Some(parent) = config_file.parent() {
        std::fs::create_dir_all(parent).map_err(disk(format!("create {}", parent.display())))?;
    }
    let before = std::fs::read_to_string(config_file).unwrap_or_default();
    let after = settings::upsert(&before, key, value);
    std::fs::write(config_file, after).map_err(disk(format!("write {}", config_file.display())))
}

/// A place a speech model is resident, and the thing that decides.
///
/// The deciding is `ayeaye_core::model::residency`, which is pure; this carries
/// the decision out. The split is what makes "released before the new one is
/// loaded" assertable at all — the plan says `Reload` and this is the two lines
/// that honour the order.
///
/// Generic over [`Slot`] rather than holding a `SpeechSlot` directly, because a
/// real slot's load is a directory of weights and hundreds of megabytes of
/// device memory. That is a system boundary, and substituting it is what lets
/// the suite assert the property that matters — that two models are never
/// resident at once — without a machine and without a download.
pub struct Residents<S: Slot> {
    slot: S,
    loaded: Option<ModelId>,
    store: PathBuf,
    policy: Policy,
}

/// Somewhere a model can be resident.
///
/// [`SpeechSlot`] and [`LanguageSlot`] are the real ones. The trait exists for
/// the reason above and has exactly their shape, so it is a boundary rather than
/// an abstraction.
///
/// The error is associated rather than fixed, because the two slots fail for
/// different reasons and neither should have to be described in the other's
/// words. A single error type here would mean a corrupt GGUF arriving at a
/// caller as a `SpeechError`, which is a sentence nobody could act on.
pub trait Slot {
    /// Why this kind of model would not load.
    type Error;

    /// Load the model in `dir`, replacing whatever was resident.
    fn load(&mut self, dir: &Path) -> Result<(), Self::Error>;
    /// Release the resident model, saying whether there was one.
    fn unload(&mut self) -> bool;
}

impl Slot for SpeechSlot {
    type Error = SpeechError;

    fn load(&mut self, dir: &Path) -> Result<(), SpeechError> {
        SpeechSlot::load(self, dir)
    }

    fn unload(&mut self) -> bool {
        SpeechSlot::unload(self)
    }
}

impl Slot for LanguageSlot {
    type Error = LanguageError;

    fn load(&mut self, dir: &Path) -> Result<(), LanguageError> {
        LanguageSlot::load(self, dir)
    }

    fn unload(&mut self) -> bool {
        LanguageSlot::unload(self)
    }
}

impl<S: Slot> Residents<S> {
    /// A holder with nothing resident.
    pub fn new(slot: S, store: PathBuf, policy: Policy) -> Self {
        Residents {
            slot,
            loaded: None,
            store,
            policy,
        }
    }

    /// Which model is resident, if any.
    pub fn loaded(&self) -> Option<&ModelId> {
        self.loaded.as_ref()
    }

    /// The slot, to look at rather than to drive.
    pub fn slot(&self) -> &S {
        &self.slot
    }

    /// The slot itself, for the one caller that has something to ask the model
    /// in it.
    ///
    /// Deliberately narrow. Everything about *lifetime* goes through
    /// [`Residents::ensure`] and [`Residents::sweep`]; this is only how a
    /// request reaches the model those two decided should be resident, and a
    /// caller that used it to load or unload would be taking the decision back
    /// out of the one place that makes it.
    pub fn slot_mut(&mut self) -> &mut S {
        &mut self.slot
    }

    /// Make the resident model be the one that is wanted.
    ///
    /// This is the only thing that loads. `wanted` comes from the configuration
    /// as it stands *now*, so a reconfiguration is not an event anything has to
    /// be told about: the next request notices that what is resident is not
    /// what is chosen, and the plan says to let go of it first.
    pub fn ensure(&mut self, wanted: Option<&ModelId>) -> Result<(), S::Error> {
        match residency::on_demand(self.loaded.as_ref(), wanted) {
            Plan::Keep => Ok(()),
            Plan::Release => {
                self.release();
                Ok(())
            }
            Plan::Load => self.take(wanted),
            Plan::Reload => {
                // Released first, and this ordering is the acceptance
                // criterion rather than a preference. `SpeechSlot::load`
                // happens to unload first as well; doing it here too is not
                // redundant, because the thing being promised is a property of
                // this decision and not of that implementation.
                self.release();
                self.take(wanted)
            }
        }
    }

    /// Let go of a model that has been idle too long, saying whether it did.
    ///
    /// `idle_for` is an argument rather than a clock read inside, so the caller
    /// owns the one reading of the time and the whole thing stays testable.
    pub fn sweep(&mut self, idle_for: Duration) -> bool {
        match residency::on_idle(self.loaded.as_ref(), idle_for, &self.policy) {
            Plan::Release => self.release(),
            _ => false,
        }
    }

    fn release(&mut self) -> bool {
        self.loaded = None;
        self.slot.unload()
    }

    fn take(&mut self, wanted: Option<&ModelId>) -> Result<(), S::Error> {
        let Some(wanted) = wanted else {
            return Ok(());
        };
        self.slot.load(&self.store.join(wanted.relative_dir()))?;
        // Recorded only once the load has worked. Recording first would leave
        // the holder claiming a model it does not have, and the next request
        // would decide to keep something that is not there.
        self.loaded = Some(wanted.clone());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Curl, Fetcher, Policy, PullError, Residents, cleanup_policy, install, installed, pull,
        remove,
    };
    use ayeaye_core::model::{CONFIG_FILE, ModelId, TOKENIZER_FILE, WEIGHTS_FILE};
    use std::cell::RefCell;
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    /// A directory of this test's own, removed when it goes out of scope.
    struct Scratch(PathBuf);

    impl Scratch {
        fn named(what: &str) -> Scratch {
            let path = std::env::temp_dir().join(format!(
                "ayeaye-56-{what}-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("a scratch directory");
            Scratch(path)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A safetensors file of the smallest shape that is a real one.
    fn weights() -> Vec<u8> {
        let header = br#"{"__metadata__":{}}"#;
        let mut bytes = (header.len() as u64).to_le_bytes().to_vec();
        bytes.extend_from_slice(header);
        bytes
    }

    /// A fetcher serving fixed bytes, recording every URL it was asked for.
    struct Serves {
        files: Vec<(&'static str, Vec<u8>)>,
        asked: RefCell<Vec<String>>,
    }

    impl Serves {
        fn whisper() -> Serves {
            Serves::of(br#"{"architectures": ["WhisperForConditionalGeneration"]}"#.to_vec())
        }

        fn of(config: Vec<u8>) -> Serves {
            Serves {
                files: vec![
                    (CONFIG_FILE, config),
                    (TOKENIZER_FILE, br#"{"model":{"vocab":{}}}"#.to_vec()),
                    (WEIGHTS_FILE, weights()),
                ],
                asked: RefCell::new(Vec::new()),
            }
        }

        fn urls(&self) -> Vec<String> {
            self.asked.borrow().clone()
        }
    }

    impl Fetcher for Serves {
        fn get(&self, url: &str, into: &Path) -> Result<(), String> {
            self.asked.borrow_mut().push(url.to_string());
            let (_, bytes) = self
                .files
                .iter()
                .find(|(name, _)| url.ends_with(name))
                .ok_or_else(|| format!("nothing here answers {url}"))?;
            std::fs::write(into, bytes).map_err(|why| why.to_string())
        }
    }

    /// A slot that records what it is holding, and how much at once.
    ///
    /// `peak` is the assertion this exists for: the acceptance criterion is
    /// that a model is never leaked across reconfiguration, and "two were
    /// resident at the same moment" is that failure stated as a number. A test
    /// that only checked the sequence of calls would pass on an implementation
    /// that held both and happened to call unload afterwards.
    #[derive(Default)]
    struct Recording {
        resident: Option<PathBuf>,
        at_once: usize,
        peak: usize,
        loads: usize,
    }

    impl super::Slot for Recording {
        type Error = ayeaye_infer::SpeechError;

        fn load(&mut self, dir: &Path) -> Result<(), ayeaye_infer::SpeechError> {
            self.resident = Some(dir.to_path_buf());
            self.at_once += 1;
            self.peak = self.peak.max(self.at_once);
            self.loads += 1;
            Ok(())
        }

        fn unload(&mut self) -> bool {
            self.at_once = self.at_once.saturating_sub(1);
            self.resident.take().is_some()
        }
    }

    // AYEAYE-56 — lifetime and residency owned by ayeaye: loaded on demand,
    // and never two at once across a reconfiguration.
    #[test]
    fn reconfiguring_never_holds_two_models_at_once() {
        let small = ModelId::parse("openai/whisper-small.en").expect("an id");
        let tiny = ModelId::parse("openai/whisper-tiny.en").expect("an id");
        let store = PathBuf::from("/state");
        let mut residents =
            Residents::new(Recording::default(), store.clone(), Policy { idle: None });

        // Nothing is resident until something wants it.
        assert_eq!(residents.loaded(), None);
        residents.ensure(Some(&small)).expect("it should load");
        assert_eq!(residents.loaded(), Some(&small));
        assert_eq!(
            residents.slot.resident.as_deref(),
            Some(store.join(small.relative_dir()).as_path()),
            "it has to load from where the pull put it"
        );

        // Asking again for the same one does not reload hundreds of megabytes.
        residents.ensure(Some(&small)).expect("it should keep");
        assert_eq!(residents.slot.loads, 1);

        // The reconfiguration itself.
        residents.ensure(Some(&tiny)).expect("it should reload");
        assert_eq!(residents.loaded(), Some(&tiny));
        assert_eq!(residents.slot.loads, 2);
        assert_eq!(
            residents.slot.peak, 1,
            "two models were resident at once, which is the leak this ticket \
             exists to prevent"
        );

        // And configuring the model away lets go of it.
        residents.ensure(None).expect("it should release");
        assert_eq!(residents.loaded(), None);
        assert_eq!(residents.slot.resident, None);
    }

    // AYEAYE-56 — released on a policy, and the sweep never loads.
    #[test]
    fn an_idle_model_is_swept_and_a_sweep_never_loads() {
        let model = ModelId::parse("openai/whisper-small.en").expect("an id");
        let mut residents = Residents::new(
            Recording::default(),
            PathBuf::from("/state"),
            Policy {
                idle: Some(Duration::from_secs(300)),
            },
        );
        residents.ensure(Some(&model)).expect("it should load");

        assert!(!residents.sweep(Duration::from_secs(60)), "still wanted");
        assert_eq!(residents.loaded(), Some(&model));

        assert!(residents.sweep(Duration::from_secs(600)), "long enough");
        assert_eq!(residents.loaded(), None, "and the holder knows it let go");
        assert_eq!(residents.slot.resident, None);

        // Sweeping an empty slot loads nothing and is not an error.
        assert!(!residents.sweep(Duration::from_secs(9_999)));
        assert_eq!(residents.slot.loads, 1);
    }

    // AYEAYE-56 — a load that fails leaves the holder claiming nothing.
    // Recording the model first would leave it insisting on a model it does
    // not have, and the next request would decide to keep it.
    #[test]
    fn a_load_that_fails_leaves_nothing_claimed() {
        struct Refuses;
        impl super::Slot for Refuses {
            type Error = ayeaye_infer::SpeechError;

            fn load(&mut self, _: &Path) -> Result<(), ayeaye_infer::SpeechError> {
                Err(ayeaye_infer::SpeechError::NotLoaded)
            }
            fn unload(&mut self) -> bool {
                false
            }
        }

        let model = ModelId::parse("openai/whisper-small.en").expect("an id");
        let mut residents = Residents::new(Refuses, PathBuf::from("/state"), Policy { idle: None });

        assert!(residents.ensure(Some(&model)).is_err());
        assert_eq!(
            residents.loaded(),
            None,
            "a holder that claims a model it failed to load would keep it forever"
        );
    }

    // AYEAYE-56 — the whole point of the ticket, as an observable outcome: an
    // unsupported architecture is refused having fetched **only** config.json.
    // Asserting the refusal alone would pass just as happily on an
    // implementation that downloaded half a gigabyte first and then read the
    // config, which is the thing this design exists to avoid.
    #[test]
    fn an_unsupported_model_is_refused_before_any_weights_are_fetched() {
        let scratch = Scratch::named("unsupported");
        let hub = Serves::of(br#"{"architectures": ["LlamaForCausalLM"]}"#.to_vec());
        let id = ModelId::parse("meta-llama/Llama-3.2-1B").expect("a well-formed id");

        let refused = pull(&hub, &scratch.0, "https://hub.test", &id).unwrap_err();

        assert!(matches!(refused, PullError::Unsupported(_)), "{refused:?}");
        assert!(
            refused.to_string().contains("LlamaForCausalLM"),
            "{refused}"
        );
        assert_eq!(
            hub.urls(),
            vec!["https://hub.test/meta-llama/Llama-3.2-1B/resolve/main/config.json"],
            "only the small file that decides may be fetched before the judgement"
        );
        // And nothing is left on disk for a later run to mistake for a model.
        assert_eq!(installed(&scratch.0), Vec::new());
        assert!(!scratch.0.join(id.relative_dir()).exists());
    }

    // AYEAYE-56 — the supported path: all three files land under the state
    // directory, in the place a load will look for them.
    #[test]
    fn a_supported_model_lands_whole_under_the_state_directory() {
        let scratch = Scratch::named("supported");
        let hub = Serves::whisper();
        let id = ModelId::parse("openai/whisper-tiny.en").expect("a well-formed id");

        let pulled = pull(&hub, &scratch.0, "https://hub.test", &id).expect("it should pull");

        assert_eq!(
            pulled.dir,
            scratch.0.join("models/openai/whisper-tiny.en/main")
        );
        for file in [CONFIG_FILE, TOKENIZER_FILE, WEIGHTS_FILE] {
            assert!(pulled.dir.join(file).is_file(), "{file} is missing");
        }
        assert_eq!(hub.urls().len(), 3);
        assert_eq!(installed(&scratch.0), vec![id.clone()]);

        // And it is findable and removable by the same id it was asked for.
        assert!(remove(&scratch.0, &id).expect("removable"));
        assert_eq!(installed(&scratch.0), Vec::new());
        assert!(
            !remove(&scratch.0, &id).expect("removing nothing is not an error"),
            "removing a model that is not there is not an error, and says so"
        );
    }

    // AYEAYE-56 — a file that arrives corrupt is refused rather than kept, and
    // the model that was already there survives it. A pull that half-replaced
    // a working model would be worse than one that failed.
    #[test]
    fn a_failed_pull_leaves_the_model_that_was_already_there_alone() {
        let scratch = Scratch::named("replace");
        let id = ModelId::parse("openai/whisper-tiny.en").expect("a well-formed id");
        pull(&Serves::whisper(), &scratch.0, "https://hub.test", &id).expect("the first pull");
        let dir = scratch.0.join(id.relative_dir());
        std::fs::write(dir.join("mine.txt"), b"the working one").expect("a marker");

        // The second pull serves a 404 page in place of the weights.
        let mut broken = Serves::whisper();
        broken.files[2].1 = b"<!DOCTYPE html><title>404</title>".to_vec();
        let refused = pull(&broken, &scratch.0, "https://hub.test", &id).unwrap_err();

        assert!(matches!(refused, PullError::Unusable(_)), "{refused:?}");
        assert_eq!(
            std::fs::read(dir.join("mine.txt")).expect("it should still be there"),
            b"the working one",
            "a failed pull must not disturb the model that was working"
        );
        assert_eq!(installed(&scratch.0), vec![id]);
    }

    // AYEAYE-56 — a pull that fails leaves the store holding models and
    // nothing else. A half-fetched staging directory left behind is a
    // directory the next run has no way to tell from a whole one, which is
    // why resuming one is not offered.
    #[test]
    fn a_failed_pull_leaves_no_staging_directory_behind() {
        let scratch = Scratch::named("staging");
        let mut broken = Serves::whisper();
        broken.files[2].1 = b"<!DOCTYPE html><title>404</title>".to_vec();
        let id = ModelId::parse("openai/whisper-tiny.en").expect("a well-formed id");

        pull(&broken, &scratch.0, "https://hub.test", &id).unwrap_err();

        let left: Vec<String> = std::fs::read_dir(scratch.0.join("models"))
            .expect("the store")
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        assert!(
            left.is_empty(),
            "a half-fetched model was left behind: {left:?}"
        );
    }

    // AYEAYE-56 — the store is a directory on somebody's disk, so what is in
    // it is not necessarily something ayeaye put there. Directory names are
    // parsed rather than trusted, and anything that is not a model id is not
    // reported as a model.
    #[test]
    fn a_directory_that_is_not_a_model_is_not_listed_as_one() {
        let scratch = Scratch::named("junk");
        let id = ModelId::parse("openai/whisper-tiny.en").expect("a well-formed id");
        pull(&Serves::whisper(), &scratch.0, "https://hub.test", &id).expect("the real one");

        // A name no model id could have — `~` is outside what `ModelId::parse`
        // admits, and it is what a displaced directory is named with.
        std::fs::create_dir_all(
            scratch
                .0
                .join("models/openai/whisper-tiny.en/main~replaced"),
        )
        .expect("something that is not a model");
        std::fs::create_dir_all(scratch.0.join("models/what is this/x y z/rev"))
            .expect("and something nobody meant to put there");

        assert_eq!(installed(&scratch.0), vec![id], "only the model is a model");
    }

    // AYEAYE-56 — the restore path, reached the only way it can be: by making
    // the rename itself fail. A pull that has already moved the old model
    // aside and then cannot move the new one in must put the old one back,
    // because the alternative is a machine with no model where it had a
    // working one a moment ago.
    //
    // `store/models` is made unwritable, which stops `rename(staging, dir)` —
    // staging lives directly under it — while leaving `rename(dir, displaced)`
    // alone, since those two share a different parent. No fault injection at
    // the fetcher can produce this, which is why the test reaches for `install`
    // rather than `pull`.
    #[test]
    #[cfg(unix)]
    fn a_rename_that_fails_puts_the_model_that_was_there_back() {
        use std::os::unix::fs::PermissionsExt;

        let scratch = Scratch::named("restore");
        let models = scratch.0.join("models");
        let dir = models.join("openai/whisper-tiny.en/main");
        std::fs::create_dir_all(&dir).expect("the model that is already there");
        std::fs::write(dir.join("mine.txt"), b"the working one").expect("a marker");
        let staging = models.join(".pulling-openai-whisper-tiny.en-1");
        std::fs::create_dir_all(&staging).expect("a finished staging directory");
        std::fs::write(staging.join(CONFIG_FILE), b"{}").expect("its contents");

        let locked = std::fs::Permissions::from_mode(0o555);
        let open = std::fs::Permissions::from_mode(0o755);
        std::fs::set_permissions(&models, locked).expect("lock the directory");
        let outcome = install(&staging, &dir);
        std::fs::set_permissions(&models, open).expect("unlock it again");

        let refused = outcome.expect_err("the rename should have been refused");
        assert!(matches!(refused, PullError::Disk { .. }), "{refused:?}");
        assert_eq!(
            std::fs::read(dir.join("mine.txt")).expect("it should have been put back"),
            b"the working one",
            "the model that was working has to survive a failed replacement"
        );
        assert!(
            !dir.with_extension("replaced").exists(),
            "and nothing may be left sitting beside it under another name"
        );
    }

    // AYEAYE-58 — the cleanup pass is configured through `Policy::resolve`, not
    // assembled by hand out of the prompt.
    //
    // The difference is not tidiness. A hand-built policy pairs somebody's own
    // prompt with the *default* prompt's echo phrases — guarding against
    // instructions nobody is giving, and refusing a legitimate rewrite that
    // happens to contain the words — and leaves the template and the token
    // budget reading nothing at all, which on a Llama-3 model is a cleanup pass
    // that quietly answers dictations instead of rewriting them.
    #[test]
    fn the_cleanup_pass_is_configured_rather_than_assembled() {
        let scratch = Scratch::named("cleanup-policy");
        let file = scratch.0.join("env");
        std::fs::write(
            &file,
            "AYEAYE_CLEANUP_PROMPT=Say it back in French.\n\
             AYEAYE_CLEANUP_ECHOES=say it back, en francais\n\
             AYEAYE_CLEANUP_TEMPLATE=llama3\n\
             AYEAYE_CLEANUP_MAX_TOKENS=64\n",
        )
        .expect("a configuration file");

        let policy = cleanup_policy(&file).expect("it should resolve");

        assert_eq!(policy.system_prompt, "Say it back in French.");
        assert_eq!(policy.echoes, vec!["say it back", "en francais"]);
        assert_eq!(policy.template, ayeaye_core::chat::Template::llama3());
        assert_eq!(policy.max_new_tokens, 64);
        // Which is observable rather than bookkeeping: the prompt really is
        // rendered in the family the file named.
        assert!(
            policy.prompt("bonjour").starts_with("<|begin_of_text|>"),
            "{}",
            policy.prompt("bonjour")
        );
    }

    // AYEAYE-58 — a prompt on its own drops the default prompt's tells with it,
    // which is the coupling `Policy::resolve` exists to keep and the one a
    // hand-built policy breaks silently.
    #[test]
    fn naming_only_a_prompt_leaves_the_old_prompts_tells_behind() {
        let scratch = Scratch::named("cleanup-echoes");
        let file = scratch.0.join("env");
        std::fs::write(&file, "AYEAYE_CLEANUP_PROMPT=Say it back in French.\n")
            .expect("a configuration file");

        let policy = cleanup_policy(&file).expect("it should resolve");

        assert_eq!(policy.system_prompt, "Say it back in French.");
        assert!(
            policy.echoes.is_empty(),
            "the default prompt's tells guard nothing on somebody else's prompt: {:?}",
            policy.echoes
        );
    }

    // AYEAYE-58 — a machine nobody has configured is not an error, and a
    // template nobody implements is. Falling back to the default there would
    // leave a model answering dictations with no symptom but worse output.
    #[test]
    fn no_file_is_the_default_pass_and_a_template_nobody_has_is_refused() {
        let scratch = Scratch::named("cleanup-absent");

        let bare = cleanup_policy(&scratch.0.join("nothing-here")).expect("no file resolves");
        assert_eq!(bare, super::CleanupPolicy::default());

        let file = scratch.0.join("env");
        std::fs::write(&file, "AYEAYE_CLEANUP_TEMPLATE=alpaca\n").expect("a configuration file");
        let refused = cleanup_policy(&file).expect_err("a template nobody implements");
        assert!(refused.to_string().contains("alpaca"), "{refused}");
    }

    // AYEAYE-56 — `--fail` and `--location` are not decoration. Without the
    // first, curl writes the server's error page to the output file and exits
    // 0, which is exactly how a 404 page comes to be saved as
    // `model.safetensors`; without the second, every download of a hub file is
    // a redirect notice, because `/resolve/` redirects to the CDN.
    #[test]
    fn the_command_line_carries_the_flags_that_stop_a_page_being_saved_as_a_model() {
        let argv = Curl::argv(
            "https://hub.test/a/b/resolve/main/config.json",
            Path::new("/tmp/x"),
        );
        assert_eq!(argv[0], "curl");
        assert!(argv.iter().any(|arg| arg == "--fail"), "{argv:?}");
        assert!(argv.iter().any(|arg| arg == "--location"), "{argv:?}");
        // curl reads options at any position, not only before the first
        // non-option, so the URL is data only because `--` ends the options
        // ahead of it. Verified against the real curl: without it, a hub
        // configured as `-o/somewhere` is consumed as a flag and curl answers
        // "no URL specified".
        assert_eq!(argv[argv.len() - 2], "--", "{argv:?}");
        assert_eq!(
            argv.last().unwrap(),
            "https://hub.test/a/b/resolve/main/config.json"
        );
    }
}
