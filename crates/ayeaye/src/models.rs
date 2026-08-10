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

use ayeaye_core::model::hub::{self, Wanted};
use ayeaye_core::model::settings::{self, BadSetting, ModelSettings};
use ayeaye_core::model::verify::{self, Unusable};
use ayeaye_core::model::{Architecture, ModelId, Unsupported, architecture};

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
        .chain([into.to_string_lossy().into_owned(), url.to_string()])
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

    let displaced = dir.with_extension("replaced");
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
        ".pulling-{}-{}-{}",
        id.owner(),
        id.name(),
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

#[cfg(test)]
mod tests {
    use super::{Curl, Fetcher, PullError, install, installed, pull, remove};
    use ayeaye_core::model::{CONFIG_FILE, ModelId, TOKENIZER_FILE, WEIGHTS_FILE};
    use std::cell::RefCell;
    use std::path::{Path, PathBuf};

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

    // AYEAYE-56 — `--fail` and `--location` are not decoration. Without the
    // first, curl writes the server's error page to the output file and exits
    // 0, which is exactly how a 404 page comes to be saved as
    // `model.safetensors`; without the second, every download of a hub file is
    // a redirect notice, because `/resolve/` redirects to the CDN.
    #[test]
    fn the_command_line_refuses_an_error_page_and_follows_the_hubs_redirect() {
        let argv = Curl::argv(
            "https://hub.test/a/b/resolve/main/config.json",
            Path::new("/tmp/x"),
        );
        assert_eq!(argv[0], "curl");
        assert!(argv.iter().any(|arg| arg == "--fail"), "{argv:?}");
        assert!(argv.iter().any(|arg| arg == "--location"), "{argv:?}");
        // The URL is the last argument, so a URL beginning with `-` cannot be
        // read as a flag.
        assert_eq!(
            argv.last().unwrap(),
            "https://hub.test/a/b/resolve/main/config.json"
        );
    }
}
