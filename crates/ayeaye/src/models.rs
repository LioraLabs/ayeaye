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
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use ayeaye_core::cleanup::{Policy as CleanupPolicy, PolicyError};
use ayeaye_core::model::hub::{self, Wanted};
use ayeaye_core::model::residency::{self, Plan, Policy};
use ayeaye_core::model::settings::{self, BadSetting, ModelSettings};
use ayeaye_core::model::verify::{self, Unusable};
use ayeaye_core::model::{Architecture, ModelId, Role, Unsupported, architecture};
use ayeaye_core::model::{CONFIG_FILE, TOKENIZER_FILE, WEIGHTS_FILE};
use ayeaye_infer::language::model::{
    TOKENIZER_FILE as CLEANUP_TOKENIZER_FILE, WEIGHTS_FILE as CLEANUP_WEIGHTS_FILE,
};
use ayeaye_infer::{LanguageError, LanguageSlot, SpeechError, SpeechSlot};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchResult {
    pub id: String,
    pub role: Role,
    pub bytes: u64,
    pub gated: bool,
    pub evidence: &'static str,
    pub headroom: u64,
}

#[derive(Debug, Clone, Copy)]
pub struct SearchLimits {
    pub ram_bytes: u64,
    pub disk_bytes: u64,
    pub speech_bytes: u64,
    pub cleanup_bytes: u64,
}

fn hub_model_type(repo: &serde_json::Value) -> &str {
    repo["config"][architecture::MODEL_TYPE_FIELD].as_str().unwrap_or_default()
}

pub fn search_response(json: &str, limits: SearchLimits) -> Result<Vec<SearchResult>, String> {
    let repos: Vec<serde_json::Value> =
        serde_json::from_str(json).map_err(|why| format!("malformed Hub response: {why}"))?;
    let has_tokenizer = |id: &str| {
        repos.iter().any(|repo| {
            repo["id"].as_str() == Some(id)
                && repo["siblings"].as_array().is_some_and(|files| {
                    files.iter().any(|file| file["rfilename"] == TOKENIZER_FILE)
                })
        })
    };
    let tokenizer_size = |id: &str| {
        repos.iter().find(|repo| repo["id"] == id)?["siblings"].as_array()?
            .iter().find(|file| file["rfilename"] == TOKENIZER_FILE)
            .and_then(|file| file["size"].as_u64().or_else(|| file["lfs"]["size"].as_u64()))
    };
    let mut found = Vec::new();
    for repo in &repos {
        let Some(id) = repo["id"].as_str() else { continue };
        let files = repo["siblings"].as_array().map(Vec::as_slice).unwrap_or(&[]);
        let named = |name: &str| files.iter().find(|file| file["rfilename"] == name);
        let file_size = |file: &serde_json::Value| {
            file["size"].as_u64().or_else(|| file["lfs"]["size"].as_u64())
        };
        let speech = architecture::in_config(&repo["config"].to_string()).is_ok();
        let (role, bytes) = if speech {
            let Some(weights) = named(WEIGHTS_FILE).and_then(file_size) else { continue };
            let Some(config) = named(CONFIG_FILE).and_then(file_size) else { continue };
            let Some(tokenizer) = named(TOKENIZER_FILE).and_then(file_size) else { continue };
            (Role::Speech, weights + config + tokenizer)
        } else {
            let mut ggufs: Vec<_> = files.iter().filter(|file| {
                file["rfilename"].as_str().is_some_and(|name| {
                    name.ends_with(".gguf") && !name.contains('/') && !name.contains("-of-")
                })
            }).filter_map(|file| Some((file, file_size(file)?))).collect();
            ggufs.sort_unstable_by_key(|(file, bytes)| {
                (!file["rfilename"].as_str().unwrap_or_default().to_ascii_uppercase().contains("Q4_K_M"), *bytes)
            });
            let Some((_, bytes)) = ggufs.first() else { continue };
            let bases: Vec<&str> = repo["cardData"]["base_model"].as_str().into_iter()
                .chain(repo["cardData"]["base_model"].as_array().into_iter().flatten().filter_map(serde_json::Value::as_str))
                .collect();
            let architecture = if hub_model_type(repo).is_empty() {
                bases.iter().find_map(|id| repos.iter().find(|candidate| candidate["id"] == **id))
                    .map(hub_model_type).unwrap_or_default()
            } else { hub_model_type(repo) };
            if !ayeaye_infer::language::model::SUPPORTED.contains(&architecture) { continue; }
            let tokenizer = named(TOKENIZER_FILE).and_then(file_size)
                .or_else(|| bases.iter().find_map(|base| tokenizer_size(base)));
            let Some(tokenizer) = tokenizer else { continue };
            debug_assert!(named(TOKENIZER_FILE).is_some() || bases.iter().any(|base| has_tokenizer(base)));
            (Role::Cleanup, *bytes + tokenizer)
        };
        let resident = bytes + match role {
            Role::Speech => limits.cleanup_bytes,
            Role::Cleanup => limits.speech_bytes,
        };
        if bytes > limits.disk_bytes || resident > limits.ram_bytes { continue; }
        found.push(SearchResult {
            id: id.to_string(),
            role,
            bytes,
            gated: repo["gated"].as_bool().unwrap_or(false)
                || repo["gated"].as_str().is_some_and(|value| value != "false"),
            evidence: "metadata heuristic",
            headroom: limits.ram_bytes - resident,
        });
    }
    found.sort_by_key(|model| (std::cmp::Reverse(model.headroom), model.bytes, model.id.clone()));
    Ok(found)
}

pub fn search(
    fetcher: &impl Fetcher,
    hub_host: &str,
    query: &str,
    limits: SearchLimits,
) -> Result<Vec<SearchResult>, String> {
    let query = form_urlencoded::Serializer::new(String::new())
        .append_pair("search", query)
        .append_pair("expand[]", "config")
        .append_pair("expand[]", "cardData")
        .append_pair("expand[]", "siblings")
        .append_pair("expand[]", "gated")
        .finish();
    let url = format!("{}/api/models?{query}", hub_host.trim_end_matches('/'));
    let file = std::env::temp_dir().join(format!(
        "ayeaye-model-search-{}-{}",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos()
    ));
    let result = fetcher
        .get(&url, &file)
        .map_err(|why| format!("could not search {url}: {why}"))
        .and_then(|()| std::fs::read_to_string(&file).map_err(|why| why.to_string()))
        .and_then(|json| {
            let mut repos: Vec<serde_json::Value> = serde_json::from_str(&json)
                .map_err(|why| format!("malformed Hub response: {why}"))?;
            for repo in &mut repos {
                let Some(id) = repo["id"].as_str() else { continue };
                let detail_url = format!("{}/api/models/{id}?expand%5B%5D=config&expand%5B%5D=cardData&expand%5B%5D=siblings&expand%5B%5D=gated", hub_host.trim_end_matches('/'));
                fetcher.get(&detail_url, &file)
                    .map_err(|why| format!("could not inspect {id}: {why}"))?;
                let json = std::fs::read_to_string(&file).map_err(|why| why.to_string())?;
                *repo = serde_json::from_str(&json)
                    .map_err(|why| format!("malformed Hub response for {id}: {why}"))?;
            }
            let bases: Vec<String> = repos.iter().flat_map(|repo| {
                repo["cardData"]["base_model"].as_str().into_iter()
                    .chain(repo["cardData"]["base_model"].as_array().into_iter().flatten().filter_map(serde_json::Value::as_str))
            }).map(str::to_string).filter(|base| !repos.iter().any(|repo| repo["id"] == *base)).collect();
            for base in bases {
                let base_url = format!("{}/api/models/{base}?expand%5B%5D=config&expand%5B%5D=siblings", hub_host.trim_end_matches('/'));
                fetcher.get(&base_url, &file)
                    .map_err(|why| format!("could not inspect {base}: {why}"))?;
                let json = std::fs::read_to_string(&file).map_err(|why| why.to_string())?;
                repos.push(serde_json::from_str(&json)
                    .map_err(|why| format!("malformed Hub response for {base}: {why}"))?);
            }
            search_response(&serde_json::to_string(&repos).expect("Hub values serialize"), limits)
        });
    let _ = std::fs::remove_file(file);
    result
}

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

    fn authorization(url: &str, token: &str) -> Option<String> {
        url.starts_with(&format!("{}/", hub::DEFAULT_HOST))
            .then(|| format!("oauth2-bearer = \"{token}\""))
    }
}

impl Fetcher for Curl {
    fn get(&self, url: &str, into: &Path) -> Result<(), String> {
        let mut argv = Curl::argv(url, into);
        let authorization = hf_token()?.and_then(|token| Self::authorization(url, &token));
        if authorization.is_some() {
            let before_separator = argv.len() - 2;
            argv.splice(
                before_separator..before_separator,
                ["--config".to_string(), "-".to_string()],
            );
        }
        let mut command = Command::new(&argv[0]);
        command.args(&argv[1..]);
        if authorization.is_some() {
            command.stdin(Stdio::piped());
        }
        let mut child = command
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|why| format!("could not run curl: {why}"))?;
        if let Some(authorization) = authorization {
            writeln!(child.stdin.take().expect("piped stdin"), "{authorization}")
                .map_err(|why| format!("could not authorize curl: {why}"))?;
        }
        let ran = child
            .wait_with_output()
            .map_err(|why| format!("could not wait for curl: {why}"))?;
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

fn hf_token() -> Result<Option<String>, String> {
    let token = std::env::var("HF_TOKEN").ok().or_else(|| {
        let home = std::env::var_os("HF_HOME").map(PathBuf::from).or_else(|| {
            std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache/huggingface"))
        })?;
        std::fs::read_to_string(home.join("token")).ok()
    });
    let token = token
        .map(|token| token.trim().to_string())
        .filter(|token| !token.is_empty());
    if token.as_deref().is_some_and(|token| {
        !token
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    }) {
        return Err(
            "HF_TOKEN or the Hugging Face CLI token file contains invalid characters".to_string(),
        );
    }
    Ok(token)
}

pub fn token_available() -> Result<bool, String> {
    hf_token().map(|token| token.is_some())
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
    /// A local path is not a model this build can import.
    Invalid(String),
}

impl fmt::Display for PullError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PullError::Unsupported(why) => write!(out, "{why}"),
            PullError::Fetch { url, why } => write!(out, "could not fetch {url}: {why}"),
            PullError::Unusable(why) => write!(out, "{why}"),
            PullError::Disk { what, why } => write!(out, "could not {what}: {why}"),
            PullError::Invalid(why) => out.write_str(why),
        }
    }
}

/// A local model that is now in the managed store.
pub struct Added {
    pub id: ModelId,
    pub dir: PathBuf,
    pub role: Role,
    pub already: bool,
}

/// Validate and import a local speech directory or GGUF model.
pub fn add(store: &Path, source: &Path) -> Result<Added, PullError> {
    let (name, role, files): (String, Role, Vec<(&str, PathBuf)>) = if source.is_dir() {
        let role = role(source).map_err(PullError::Invalid)?;
        let files = match role {
            Role::Speech => {
                let config = std::fs::read_to_string(source.join(CONFIG_FILE)).map_err(|why| {
                    PullError::Invalid(format!(
                        "could not read {}: {why}",
                        source.join(CONFIG_FILE).display()
                    ))
                })?;
                architecture::in_config(&config).map_err(PullError::Unsupported)?;
                vec![CONFIG_FILE, TOKENIZER_FILE, WEIGHTS_FILE]
            }
            Role::Cleanup => {
                check_cleanup_architecture(&source.join(CLEANUP_WEIGHTS_FILE))?;
                vec![CLEANUP_TOKENIZER_FILE, CLEANUP_WEIGHTS_FILE]
            }
        }
        .into_iter()
        .map(|file| (file, source.join(file)))
        .collect();
        (
            source
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            role,
            files,
        )
    } else if source.is_file() && source.extension().is_some_and(|ext| ext == "gguf") {
        let tokenizer = source.with_file_name(CLEANUP_TOKENIZER_FILE);
        if !tokenizer.is_file() {
            return Err(PullError::Invalid(format!(
                "missing companion {} beside {}",
                CLEANUP_TOKENIZER_FILE,
                source.display()
            )));
        }
        valid_json(&tokenizer).map_err(PullError::Invalid)?;
        check_cleanup_architecture(source)?;
        (
            source
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            Role::Cleanup,
            vec![
                (CLEANUP_TOKENIZER_FILE, tokenizer),
                (CLEANUP_WEIGHTS_FILE, source.to_path_buf()),
            ],
        )
    } else {
        return Err(PullError::Invalid(format!(
            "{} is not a model directory or a GGUF file",
            source.display()
        )));
    };

    let revision = content_hash(&files)?;
    let safe_name: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '-'
            }
        })
        .collect();
    let id = ModelId::parse(&format!("local/{safe_name}@{revision}"))
        .map_err(|why| PullError::Invalid(why.to_string()))?;
    let dir = store.join(id.relative_dir());
    if dir.is_dir() {
        return Ok(Added {
            id,
            dir,
            role,
            already: true,
        });
    }
    let staging = staging_dir(store, &id);
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging).map_err(|why| PullError::Disk {
        what: format!("create {}", staging.display()),
        why: why.to_string(),
    })?;
    let imported = files.into_iter().try_for_each(|(name, from)| {
        if !from
            .symlink_metadata()
            .is_ok_and(|metadata| metadata.file_type().is_file())
        {
            return Err(PullError::Invalid(format!(
                "{} is not a regular file",
                from.display()
            )));
        }
        if std::fs::hard_link(&from, staging.join(name)).is_ok() {
            Ok(())
        } else {
            std::fs::copy(&from, staging.join(name))
                .map(|_| ())
                .map_err(|why| PullError::Disk {
                    what: format!("import {}", from.display()),
                    why: why.to_string(),
                })
        }
    });
    if let Err(why) = imported {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(why);
    }
    install(&staging, &dir)?;
    Ok(Added {
        id,
        dir,
        role,
        already: false,
    })
}

fn check_cleanup_architecture(path: &Path) -> Result<String, PullError> {
    let found = ayeaye_infer::language::model::architecture(path)
        .map_err(|why| PullError::Invalid(why.to_string()))?;
    if ayeaye_infer::language::model::SUPPORTED.contains(&found.as_str()) {
        Ok(found)
    } else {
        Err(PullError::Invalid(format!(
            "{found:?} is not a GGUF architecture this build can run; it runs {}",
            ayeaye_infer::language::model::SUPPORTED.join(", ")
        )))
    }
}

fn content_hash(files: &[(&str, PathBuf)]) -> Result<String, PullError> {
    // FNV-1a is enough here: the revision is a stable content pin, not a security boundary.
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    let mut buffer = [0u8; 64 * 1024];
    for (name, path) in files {
        for byte in name.as_bytes() {
            hash = (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3);
        }
        let mut file = std::fs::File::open(path).map_err(|why| PullError::Disk {
            what: format!("read {}", path.display()),
            why: why.to_string(),
        })?;
        loop {
            let read = file.read(&mut buffer).map_err(|why| PullError::Disk {
                what: format!("read {}", path.display()),
                why: why.to_string(),
            })?;
            if read == 0 {
                break;
            }
            for byte in &buffer[..read] {
                hash = (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
    }
    Ok(format!("{hash:016x}"))
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
    pub architecture: String,
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
) -> Result<(String, u64), PullError> {
    if let Some(cleanup) = cleanup_plan(fetcher, staging, hub_host, id)? {
        return fetch_cleanup(fetcher, staging, hub_host, id, cleanup);
    }
    let mut architecture = None;
    let mut bytes = 0u64;

    for wanted in hub::WANTED {
        let url = hub::url(hub_host, id, wanted);
        let into = staging.join(wanted.file);
        fetcher
            .get(&url, &into)
            .map_err(|why| PullError::Fetch {
                url,
                why: format!(
                    "{why}. The repository may be gated; provide HF_TOKEN or the standard Hugging Face CLI token file"
                ),
            })?;

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
    Ok((architecture.hf_name().to_string(), bytes))
}

struct CleanupPlan {
    weights: String,
    base: ModelId,
}

fn cleanup_plan(
    fetcher: &impl Fetcher,
    staging: &Path,
    hub_host: &str,
    id: &ModelId,
) -> Result<Option<CleanupPlan>, PullError> {
    let url = format!(
        "{}/api/models/{}/{}/revision/{}",
        hub_host.trim_end_matches('/'),
        id.owner(),
        id.name(),
        id.revision()
    );
    let file = staging.join(".hub.json");
    if fetcher.get(&url, &file).is_err() {
        return Ok(None);
    }
    let bytes = std::fs::read(&file).map_err(|why| PullError::Disk {
        what: format!("read back {}", file.display()),
        why: why.to_string(),
    })?;
    let metadata: serde_json::Value = serde_json::from_slice(&bytes).map_err(|why| {
        PullError::Unusable(Unusable::Malformed {
            file: "Hub metadata".to_string(),
            why: why.to_string(),
        })
    })?;
    let mut candidates: Vec<&str> = metadata["siblings"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|file| file["rfilename"].as_str())
        .filter(|file| file.ends_with(".gguf") && !file.contains('/') && !file.contains("-of-"))
        .collect();
    if candidates.is_empty() {
        return Ok(None);
    }
    candidates.sort_unstable_by_key(|file| (!file.to_ascii_uppercase().contains("Q4_K_M"), *file));
    let base = metadata["cardData"]["base_model"]
        .as_str()
        .or_else(|| {
            metadata["cardData"]["base_model"]
                .as_array()?
                .first()?
                .as_str()
        })
        .ok_or_else(|| {
            PullError::Unusable(Unusable::Malformed {
                file: "Hub metadata".to_string(),
                why: "names no base_model for the tokenizer".to_string(),
            })
        })?;
    let base = ModelId::parse(base).map_err(|why| {
        PullError::Unusable(Unusable::Malformed {
            file: "Hub metadata".to_string(),
            why: format!("has an invalid base_model: {why}"),
        })
    })?;
    Ok(Some(CleanupPlan {
        weights: candidates[0].to_string(),
        base,
    }))
}

fn fetch_cleanup(
    fetcher: &impl Fetcher,
    staging: &Path,
    hub_host: &str,
    id: &ModelId,
    plan: CleanupPlan,
) -> Result<(String, u64), PullError> {
    let weights_url = hub::file_url(hub_host, id, &plan.weights);
    let weights = staging.join(CLEANUP_WEIGHTS_FILE);
    fetcher
        .get(&weights_url, &weights)
        .map_err(|why| PullError::Fetch {
            url: weights_url,
            why,
        })?;
    let architecture = check_cleanup_architecture(&weights)?;
    let tokenizer_wanted = Wanted {
        file: CLEANUP_TOKENIZER_FILE,
        decides: false,
    };
    let tokenizer_url = hub::url(hub_host, &plan.base, &tokenizer_wanted);
    let tokenizer = staging.join(CLEANUP_TOKENIZER_FILE);
    fetcher.get(&tokenizer_url, &tokenizer).map_err(|why| PullError::Fetch {
        url: tokenizer_url,
        why: format!("{why}. The base repository may be gated; provide HF_TOKEN or the standard Hugging Face CLI token file"),
    })?;
    let tokenizer_bytes = std::fs::read(&tokenizer).map_err(|why| PullError::Disk {
        what: format!("read back {}", tokenizer.display()),
        why: why.to_string(),
    })?;
    verify::check(CLEANUP_TOKENIZER_FILE, &tokenizer_bytes).map_err(PullError::Unusable)?;
    let bytes = std::fs::metadata(&weights)
        .map_err(|why| PullError::Disk {
            what: format!("inspect {}", weights.display()),
            why: why.to_string(),
        })?
        .len()
        + tokenizer_bytes.len() as u64;
    let _ = std::fs::remove_file(staging.join(".hub.json"));
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

/// What one directory in the model store can serve.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledModel {
    pub id: ModelId,
    pub role: Result<Role, String>,
    pub bytes: u64,
}

/// Inspect every model-shaped directory in stable ID order.
pub fn inspect(store: &Path) -> Vec<InstalledModel> {
    installed(store)
        .into_iter()
        .map(|id| {
            let dir = store.join(id.relative_dir());
            InstalledModel {
                role: role(&dir),
                bytes: directory_bytes(&dir),
                id,
            }
        })
        .collect()
}

fn role(dir: &Path) -> Result<Role, String> {
    let speech = [CONFIG_FILE, TOKENIZER_FILE, WEIGHTS_FILE];
    let cleanup = [CLEANUP_TOKENIZER_FILE, CLEANUP_WEIGHTS_FILE];
    let has_speech = speech.iter().all(|file| dir.join(file).is_file());
    let has_cleanup = cleanup.iter().all(|file| dir.join(file).is_file());

    match (has_speech, has_cleanup) {
        (true, true) => Err("matches both speech and cleanup layouts".to_string()),
        (true, false) => {
            valid_json(&dir.join(CONFIG_FILE))?;
            valid_json(&dir.join(TOKENIZER_FILE))?;
            valid_safetensors(&dir.join(WEIGHTS_FILE))?;
            Ok(Role::Speech)
        }
        (false, true) => {
            valid_json(&dir.join(CLEANUP_TOKENIZER_FILE))?;
            valid_gguf(&dir.join(CLEANUP_WEIGHTS_FILE))?;
            Ok(Role::Cleanup)
        }
        (false, false) => Err("missing a complete speech or cleanup file layout".to_string()),
    }
}

fn prefix(path: &Path, length: usize) -> Result<Vec<u8>, String> {
    let mut file = std::fs::File::open(path).map_err(|why| format!("{}: {why}", path.display()))?;
    let mut bytes = vec![0; length];
    let read = file
        .read(&mut bytes)
        .map_err(|why| format!("{}: {why}", path.display()))?;
    bytes.truncate(read);
    Ok(bytes)
}

fn valid_json(path: &Path) -> Result<(), String> {
    let bytes = prefix(path, 4_096)?;
    let bytes = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&bytes);
    if bytes.iter().find(|byte| !byte.is_ascii_whitespace()) == Some(&b'{') {
        Ok(())
    } else {
        Err(format!("{} is not a JSON object", path.display()))
    }
}

fn valid_safetensors(path: &Path) -> Result<(), String> {
    let bytes = prefix(path, 9)?;
    let length = bytes
        .get(..8)
        .and_then(|length| length.try_into().ok())
        .map(u64::from_le_bytes)
        .ok_or_else(|| format!("{} is too short for safetensors", path.display()))?;
    let file_length = std::fs::metadata(path)
        .map_err(|why| format!("{}: {why}", path.display()))?
        .len();
    if length > 0 && length <= file_length.saturating_sub(8) && bytes.get(8) == Some(&b'{') {
        Ok(())
    } else {
        Err(format!(
            "{} has an invalid safetensors header",
            path.display()
        ))
    }
}

fn valid_gguf(path: &Path) -> Result<(), String> {
    let bytes = prefix(path, 8)?;
    let version = bytes
        .get(4..8)
        .and_then(|version| version.try_into().ok())
        .map(u32::from_le_bytes);
    if bytes.starts_with(b"GGUF") && matches!(version, Some(2 | 3)) {
        Ok(())
    } else {
        Err(format!("{} has an invalid GGUF header", path.display()))
    }
}

fn directory_bytes(dir: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    entries
        .flatten()
        .map(|entry| match entry.file_type() {
            Ok(kind) if kind.is_dir() => directory_bytes(&entry.path()),
            Ok(kind) if kind.is_file() => entry.metadata().map(|meta| meta.len()).unwrap_or(0),
            _ => 0,
        })
        .sum()
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
    /// Why what is resident is not on the backend the build was compiled for.
    ///
    /// `None` when nothing was given up. This is how the daemon's load-time
    /// report reaches the real slots through the residency seam — AYEAYE-73's
    /// "a model loaded an hour later says so" — and it is on the trait rather
    /// than read around it because the trait's contract is the real slots'
    /// exact shape, and both real slots carry the answer.
    fn fallback(&self) -> Option<&str>;
}

impl Slot for SpeechSlot {
    type Error = SpeechError;

    fn load(&mut self, dir: &Path) -> Result<(), SpeechError> {
        SpeechSlot::load(self, dir)
    }

    fn unload(&mut self) -> bool {
        SpeechSlot::unload(self)
    }

    fn fallback(&self) -> Option<&str> {
        SpeechSlot::fallback(self)
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

    fn fallback(&self) -> Option<&str> {
        LanguageSlot::fallback(self)
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

    /// Make the resident model be the one that is wanted, saying whether a
    /// load actually happened.
    ///
    /// This is the only thing that loads. `wanted` comes from the configuration
    /// as it stands *now*, so a reconfiguration is not an event anything has to
    /// be told about: the next request notices that what is resident is not
    /// what is chosen, and the plan says to let go of it first.
    ///
    /// `true` exactly when a model went in just now. That answer is what a
    /// degradation report hangs off — AYEAYE-73's operator sees the fallback at
    /// each load, an hour into the daemon's life as much as at startup, and
    /// without this bool a caller could only say it once at startup or on every
    /// dictation, which are respectively the bug and the spam.
    pub fn ensure(&mut self, wanted: Option<&ModelId>) -> Result<bool, S::Error> {
        match residency::on_demand(self.loaded.as_ref(), wanted) {
            Plan::Keep => Ok(false),
            Plan::Release => {
                self.release();
                Ok(false)
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

    fn take(&mut self, wanted: Option<&ModelId>) -> Result<bool, S::Error> {
        let Some(wanted) = wanted else {
            return Ok(false);
        };
        self.slot.load(&self.store.join(wanted.relative_dir()))?;
        // Recorded only once the load has worked. Recording first would leave
        // the holder claiming a model it does not have, and the next request
        // would decide to keep something that is not there.
        self.loaded = Some(wanted.clone());
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Curl, Fetcher, Policy, PullError, Residents, SearchLimits, cleanup_policy, install,
        installed, pull, remove, search, search_response,
    };
    use ayeaye_core::model::{CONFIG_FILE, ModelId, TOKENIZER_FILE, WEIGHTS_FILE};

    struct RecordedHub;

    impl Fetcher for RecordedHub {
        fn get(&self, url: &str, into: &Path) -> Result<(), String> {
            let response = if url.contains("/api/models/good/speech?") {
                r#"{"id":"good/speech","config":{"model_type":"whisper"},"siblings":[
                    {"rfilename":"config.json","size":10},{"rfilename":"tokenizer.json","size":10},
                    {"rfilename":"model.safetensors","size":300}]}"#
            } else {
                r#"[{"id":"good/speech","siblings":[{"rfilename":"config.json"},
                    {"rfilename":"tokenizer.json"},{"rfilename":"model.safetensors"}]}]"#
            };
            std::fs::write(into, response).map_err(|why| why.to_string())
        }
    }

    #[test]
    fn search_fetches_per_repository_metadata_when_the_list_has_no_sizes() {
        let found = search(&RecordedHub, "https://hub.test", "speech", SearchLimits {
            ram_bytes: 1_000,
            disk_bytes: 1_000,
            speech_bytes: 0,
            cleanup_bytes: 0,
        }).unwrap();
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].bytes, 320);
    }

    #[test]
    fn hub_search_keeps_only_loadable_models_that_fit_beside_the_other_role() {
        let response = r#"[
          {"id":"base/tokenizer","config":{"model_type":"qwen2"},"siblings":[{"rfilename":"tokenizer.json","size":10}]},
          {"id":"good/speech","config":{"model_type":"whisper"},"siblings":[
            {"rfilename":"config.json","size":10},{"rfilename":"tokenizer.json","size":10},
            {"rfilename":"model.safetensors","size":300}]},
          {"id":"good/cleanup","gated":"manual",
            "cardData":{"base_model":["base/tokenizer"]},"siblings":[{"rfilename":"model-q4.gguf","size":200}]},
          {"id":"split/cleanup","config":{"model_type":"llama"},"siblings":[
            {"rfilename":"model-00001-of-00002.gguf","size":100},{"rfilename":"model-00002-of-00002.gguf","size":100},{"rfilename":"tokenizer.json","size":10}]},
          {"id":"too-big/speech","config":{"model_type":"whisper"},"siblings":[
            {"rfilename":"config.json"},{"rfilename":"tokenizer.json"},{"rfilename":"model.safetensors","size":901}]},
          {"id":"wrong/model","config":{"model_type":"bert"},"siblings":[{"rfilename":"model.gguf","size":20}]}
        ]"#;
        let found = search_response(response, SearchLimits {
            ram_bytes: 1_000,
            disk_bytes: 800,
            speech_bytes: 700,
            cleanup_bytes: 600,
        }).unwrap();
        assert_eq!(found.len(), 2, "{found:?}");
        assert_eq!(found[0].id, "good/cleanup");
        assert_eq!(found[0].bytes, 210, "GGUF plus its base tokenizer");
        assert!(found[0].gated);
        assert_eq!(found[1].id, "good/speech");
        assert_eq!(found[1].bytes, 320, "weights plus both required companions");
        assert!(found.iter().all(|model| model.evidence == "metadata heuristic"));
    }
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

    fn gguf(architecture: &str) -> Vec<u8> {
        let key = b"general.architecture";
        let value = architecture.as_bytes();
        let mut bytes = b"GGUF".to_vec();
        bytes.extend_from_slice(&3u32.to_le_bytes());
        bytes.extend_from_slice(&0u64.to_le_bytes());
        bytes.extend_from_slice(&1u64.to_le_bytes());
        bytes.extend_from_slice(&(key.len() as u64).to_le_bytes());
        bytes.extend_from_slice(key);
        bytes.extend_from_slice(&8u32.to_le_bytes());
        bytes.extend_from_slice(&(value.len() as u64).to_le_bytes());
        bytes.extend_from_slice(value);
        bytes
    }

    struct CleanupHub {
        metadata: Vec<u8>,
        architecture: &'static str,
        asked: RefCell<Vec<String>>,
    }

    struct Refuses;

    impl Fetcher for Refuses {
        fn get(&self, _: &str, _: &Path) -> Result<(), String> {
            Err("HTTP 401".to_string())
        }
    }

    impl Fetcher for CleanupHub {
        fn get(&self, url: &str, into: &Path) -> Result<(), String> {
            self.asked.borrow_mut().push(url.to_string());
            let bytes = if url.contains("/api/models/") {
                self.metadata.clone()
            } else if url.ends_with("tokenizer.json") {
                br#"{"model":{"vocab":{}}}"#.to_vec()
            } else if url.ends_with(".gguf") {
                gguf(self.architecture)
            } else {
                return Err(format!("nothing here answers {url}"));
            };
            std::fs::write(into, bytes).map_err(|why| why.to_string())
        }
    }

    #[test]
    fn a_gguf_repository_pulls_one_quantization_and_its_base_tokenizer() {
        let scratch = Scratch::named("cleanup-pull");
        let hub = CleanupHub {
            metadata: br#"{"siblings":[{"rfilename":"model-Q8_0-00001-of-00002.gguf"},{"rfilename":"model-Q8_0-00002-of-00002.gguf"},{"rfilename":"model-Q5_K_M.gguf"},{"rfilename":"model-Q4_K_M.gguf"}],"cardData":{"base_model":"Qwen/Qwen2.5-1.5B-Instruct"}}"#.to_vec(),
            architecture: "qwen2",
            asked: RefCell::new(Vec::new()),
        };
        let id = ModelId::parse("bartowski/Qwen2.5-GGUF").expect("an id");

        let pulled = pull(&hub, &scratch.0, "https://hub.test", &id).expect("cleanup pull");

        assert!(pulled.dir.join("model.gguf").is_file());
        assert!(pulled.dir.join("tokenizer.json").is_file());
        assert_eq!(
            hub.asked.borrow().as_slice(),
            [
                "https://hub.test/api/models/bartowski/Qwen2.5-GGUF/revision/main",
                "https://hub.test/bartowski/Qwen2.5-GGUF/resolve/main/model-Q4_K_M.gguf",
                "https://hub.test/Qwen/Qwen2.5-1.5B-Instruct/resolve/main/tokenizer.json",
            ]
        );
    }

    #[test]
    fn an_unsupported_gguf_architecture_never_replaces_the_working_model() {
        let scratch = Scratch::named("cleanup-unsupported");
        let id = ModelId::parse("someone/model-GGUF").expect("an id");
        let dir = scratch.0.join(id.relative_dir());
        std::fs::create_dir_all(&dir).expect("old model");
        std::fs::write(dir.join("model.gguf"), b"working").expect("old weights");
        let hub = CleanupHub {
            metadata: br#"{"siblings":[{"rfilename":"model.gguf"}],"cardData":{"base_model":"someone/model"}}"#.to_vec(),
            architecture: "gemma",
            asked: RefCell::new(Vec::new()),
        };

        let error = pull(&hub, &scratch.0, "https://hub.test", &id).unwrap_err();

        assert!(error.to_string().contains("gemma"), "{error}");
        assert_eq!(std::fs::read(dir.join("model.gguf")).unwrap(), b"working");
        assert_eq!(hub.asked.borrow().len(), 2, "tokenizer must not be fetched");
    }

    #[test]
    fn a_gated_repository_names_both_supported_token_sources() {
        let scratch = Scratch::named("gated");
        let id = ModelId::parse("private/model").expect("an id");
        let error = pull(&Refuses, &scratch.0, "https://hub.test", &id).unwrap_err();
        let message = error.to_string();
        assert!(message.contains("gated"), "{message}");
        assert!(message.contains("HF_TOKEN"), "{message}");
        assert!(message.contains("CLI token file"), "{message}");
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

        fn fallback(&self) -> Option<&str> {
            None
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

    // AYEAYE-73 — `ensure` says when a load actually happened, because that is
    // the moment a degradation report belongs to. The last third is the case
    // the ticket names: a model swept for idleness and loaded again an hour
    // later is a fresh load, and an operator scrolled an hour past the startup
    // banner gets the fallback said again where they are looking.
    #[test]
    fn ensure_says_when_it_actually_loaded_so_a_late_load_can_be_reported() {
        let small = ModelId::parse("openai/whisper-small.en").expect("an id");
        let tiny = ModelId::parse("openai/whisper-tiny.en").expect("an id");
        let mut residents = Residents::new(
            Recording::default(),
            PathBuf::from("/state"),
            Policy {
                idle: Some(Duration::from_secs(300)),
            },
        );

        assert!(
            residents.ensure(Some(&small)).expect("it should load"),
            "the first load is a load"
        );
        assert!(
            !residents.ensure(Some(&small)).expect("it should keep"),
            "a keep must not claim a load, or the report becomes per-dictation spam"
        );
        assert!(
            residents.ensure(Some(&tiny)).expect("it should reload"),
            "a reconfiguration puts a model in, so it is a load"
        );
        assert!(
            !residents.ensure(None).expect("it should release"),
            "a release loads nothing"
        );

        // The hour-later case: resident, swept idle, wanted again.
        assert!(residents.ensure(Some(&tiny)).expect("it should load again"));
        assert!(
            residents.sweep(Duration::from_secs(600)),
            "swept for idleness"
        );
        assert!(
            residents.ensure(Some(&tiny)).expect("the late load"),
            "a load after a sweep is a load, which is what lets the daemon \
             repeat the fallback an hour after the startup banner scrolled away"
        );
    }

    // AYEAYE-73 — the real slots answer the trait's question with the device
    // decision they hold, which is what the daemon's load-time report reads
    // through the residency seam. The selection is built by asking for cuda
    // and opening the processor — a fallback by definition, deterministic on
    // every build row, no card involved.
    #[test]
    fn the_real_slots_report_their_device_fallback_through_the_trait() {
        use ayeaye_infer::backend::{self, Backend};

        let selection = backend::choose(Backend::Cuda, |_| backend::open(Backend::Cpu));
        let why = selection
            .fallback()
            .expect("a cpu opened for a cuda ask is a fallback")
            .to_string();

        let speech = ayeaye_infer::SpeechSlot::on(selection.clone());
        let language = ayeaye_infer::LanguageSlot::on(selection);

        assert_eq!(super::Slot::fallback(&speech), Some(why.as_str()));
        assert_eq!(super::Slot::fallback(&language), Some(why.as_str()));
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
            fn fallback(&self) -> Option<&str> {
                None
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
            hub.urls().last().unwrap(),
            "https://hub.test/meta-llama/Llama-3.2-1B/resolve/main/config.json"
        );
        assert!(hub.urls().iter().all(|url| !url.ends_with(WEIGHTS_FILE)));
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
        assert_eq!(hub.urls().len(), 4);
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

    #[test]
    fn a_hugging_face_token_is_protected_and_never_sent_to_a_custom_hub() {
        assert_eq!(
            Curl::authorization(
                "https://huggingface.co/org/model/resolve/main/file",
                "hf_secret"
            ),
            Some("oauth2-bearer = \"hf_secret\"".to_string())
        );
        assert_eq!(
            Curl::authorization(
                "https://hub.example/org/model/resolve/main/file",
                "hf_secret"
            ),
            None
        );
    }
}
