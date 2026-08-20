//! Which model plays which part, and where that is written down.
//!
//! **This file used to acquire models.** It searched a hub, judged an
//! architecture off a two-kilobyte `config.json` before fetching hundreds of
//! megabytes, staged a download and renamed it into place, verified what landed,
//! and held the resident weights. All of it went with AYEAYE-101: the weights
//! live behind `llama-swap` now, which downloads nothing — somebody points it at
//! files they already have and gives each one a name.
//!
//! What is left is the part that was always ayeaye's own, and the reason this is
//! still a file rather than two functions in `config.rs`: *which* of the models
//! the backend serves is the one dictation is transcribed with, and which one
//! rewrites it. That is a choice, it belongs in `~/.config/ayeaye/env`, and
//! writing one key of a file without disturbing the rest of it is the fiddly
//! part.

use std::io::Write;
use std::path::Path;

use ayeaye_core::cleanup::{Policy as CleanupPolicy, PolicyError};
use ayeaye_core::model::Role;
use ayeaye_core::model::settings::{self, BadSetting, ModelSettings};

use crate::dictate::{Cleanup, Lists, Speech};

/// Why a choice could not be written down.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Disk {
    /// What was being attempted, in the imperative.
    pub what: String,
    /// What the system said.
    pub why: String,
}

impl std::fmt::Display for Disk {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(out, "could not {}: {}", self.what, self.why)
    }
}

impl std::error::Error for Disk {}

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
/// The same precedence [`settings`] reads under, and for the same reason: under
/// the service unit the file has *become* the environment by the time this runs,
/// and run by hand it has not.
///
/// Separate from [`settings`] because it answers a different question and fails
/// for different reasons. Reading it through `Policy::resolve` rather than
/// assembling a `Policy` by hand is what keeps `CLEANUP_ECHOES` paired with the
/// prompt it belongs to, and what makes `CLEANUP_MAX_TOKENS` mean anything.
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
pub fn choose(config_file: &Path, key: &str, value: &str) -> Result<(), Disk> {
    choose_with(
        config_file,
        key,
        value,
        |from, to| std::fs::rename(from, to),
        |dir| std::fs::File::open(dir).and_then(|dir| dir.sync_all()),
    )
}

fn choose_with(
    config_file: &Path,
    key: &str,
    value: &str,
    rename: impl FnOnce(&Path, &Path) -> std::io::Result<()>,
    sync_parent: impl FnOnce(&Path) -> std::io::Result<()>,
) -> Result<(), Disk> {
    let disk = |what: String| {
        move |why: std::io::Error| Disk {
            what,
            why: why.to_string(),
        }
    };
    if let Some(parent) = config_file.parent() {
        std::fs::create_dir_all(parent).map_err(disk(format!("create {}", parent.display())))?;
    }
    let before = match std::fs::read_to_string(config_file) {
        Ok(text) => text,
        Err(why) if why.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(why) => return Err(disk(format!("read {}", config_file.display()))(why)),
    };
    let after = settings::upsert(&before, key, value);
    let temporary = config_file.with_extension(format!("new-{}", std::process::id()));
    let result = (|| {
        let mut file = std::fs::File::create(&temporary)
            .map_err(disk(format!("write {}", temporary.display())))?;
        file.write_all(after.as_bytes())
            .and_then(|()| file.sync_all())
            .map_err(disk(format!("write {}", temporary.display())))?;
        rename(&temporary, config_file)
            .map_err(disk(format!("replace {}", config_file.display())))?;
        // Rename is the commit point. A directory sync is still attempted for
        // crash durability, but a failure after commit cannot honestly be
        // reported as though the prior selection survived.
        let _ = sync_parent(config_file.parent().unwrap_or(Path::new(".")));
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temporary);
    }
    result
}

/// The setting a role is written to.
pub fn key_for(role: Role) -> &'static str {
    match role {
        Role::Speech => settings::SPEECH_MODEL,
        Role::Cleanup => settings::CLEANUP_MODEL,
    }
}

/// Something that can prove a model actually answers.
///
/// A trait so the setup suite can run the whole wizard without a proxy on a
/// port. The real one is [`RealSmoke`], and what it proves is worth being
/// precise about: that llama-swap will start this model and it will say
/// something. That is exactly the failure a name in a config file cannot rule
/// out — a `cmd:` with a path that moved is a model that lists fine and dies on
/// first use, which without this is discovered during somebody's first
/// dictation.
pub trait Smoke {
    /// Every model the backend will serve, or why it could not be asked.
    fn models(&mut self) -> Result<Vec<String>, String>;

    /// Ask one of them to do its job once, and hand back what it said.
    fn run(&mut self, role: Role, model: &str) -> Result<String, String>;
}

/// The words the cleanup model is asked to tidy, when proving it works.
///
/// Deliberately full of the things it is for — a filler word, a false start, and
/// an identifier spelled out loud — so the line the wizard prints back is
/// evidence rather than a greeting.
pub const SMOKE_DICTATION: &str = "um so like run the tests in parse underscore config";

/// How much silence the speech model is handed to prove it answers.
///
/// Half a second. It is not asked to *hear* anything: a transcript of silence is
/// empty or one of the stock hallucinations, and both are fine. What is being
/// proved is that the proxy started the model and it returned a transcript
/// rather than a five-hundred.
pub const SMOKE_SECONDS: f32 = 0.5;

/// The real one: a request each, through the proxy.
pub struct RealSmoke<'a, B> {
    backend: &'a B,
    policy: CleanupPolicy,
    runtime: tokio::runtime::Runtime,
}

impl<'a, B: Lists + Speech + Cleanup> RealSmoke<'a, B> {
    /// A smoke test against `backend`.
    ///
    /// It owns a runtime because `setup::carry_out` is synchronous and should
    /// stay that way: the wizard is a sequence of prompts on a terminal, and
    /// colouring the whole of it async to make two requests would be the tail
    /// wagging the dog.
    pub fn new(backend: &'a B, policy: CleanupPolicy) -> Result<Self, String> {
        Ok(RealSmoke {
            backend,
            policy,
            runtime: tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|why| format!("could not start a runtime to test the model: {why}"))?,
        })
    }
}

impl<B: Lists + Speech + Cleanup> Smoke for RealSmoke<'_, B> {
    fn models(&mut self) -> Result<Vec<String>, String> {
        self.runtime.block_on(self.backend.available())
    }

    fn run(&mut self, role: Role, model: &str) -> Result<String, String> {
        match role {
            Role::Speech => {
                let samples =
                    vec![0.0; (SMOKE_SECONDS * ayeaye_core::audio::SAMPLE_RATE_HZ as f32) as usize];
                let audio = ayeaye_core::Pcm16kMono::new(samples);
                self.runtime
                    .block_on(self.backend.transcribe(model, &audio))
                    .map(|said| {
                        if said.trim().is_empty() {
                            "(silence, as expected)".to_string()
                        } else {
                            said
                        }
                    })
            }
            Role::Cleanup => {
                let cleaned = self.runtime.block_on(self.backend.clean(
                    model,
                    SMOKE_DICTATION,
                    "",
                    &self.policy,
                ));
                // `clean` cannot fail — it falls back to the words as spoken —
                // so "it came back unchanged" is the failure to report. A model
                // that is not there and a model that declined to rewrite reach
                // this the same way, and both mean the same thing to somebody
                // choosing: do not pick this one.
                if cleaned.was_rewritten() {
                    Ok(cleaned.text().to_string())
                } else {
                    Err(format!(
                        "it did not rewrite anything — {}",
                        cleaned
                            .kept()
                            .map(|why| why.why().to_string())
                            .unwrap_or_else(|| "the backend said nothing usable".to_string())
                    ))
                }
            }
        }
    }
}

/// A model that has been chosen and shown to work.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selected {
    /// The name it goes by in the backend's config.
    pub name: String,
    /// What it said when asked to prove it.
    pub smoke: String,
}

/// What one pass of the chooser came to.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct InteractiveResult {
    /// The models chosen, in the order they were chosen.
    pub selected: Vec<Selected>,
    /// Whether cleanup was deliberately left unset.
    pub cleanup_declined: bool,
}

/// Choose models out of what the backend is serving, and write the choice down.
///
/// Much shorter than it was, and the reason is the whole ticket: there is
/// nothing to search, nothing to size against this machine's memory, and nothing
/// to download. The backend's `/v1/models` is the catalogue, and whoever
/// configured llama-swap already decided what fits.
///
/// `role` picks one part; `None` does both in order and offers to skip cleanup,
/// which is the wizard's path.
pub fn choose_interactive(
    smoke: &mut impl Smoke,
    ask: &impl crate::setup::Ask,
    config_file: &Path,
    role: Option<Role>,
) -> Result<InteractiveResult, String> {
    let served = smoke.models()?;
    if served.is_empty() {
        return Err("the backend is not serving any models".to_string());
    }

    let mut result = InteractiveResult::default();
    let roles: &[Role] = match role {
        Some(Role::Speech) => &[Role::Speech],
        Some(Role::Cleanup) => &[Role::Cleanup],
        None => &[Role::Speech, Role::Cleanup],
    };
    let paired = role.is_none();

    for &role in roles {
        if role == Role::Cleanup
            && paired
            && !ask.confirm(
                "add cleanup too? Without it, dictation uses raw transcripts.",
                false,
            )
        {
            result.cleanup_declined = true;
            break;
        }
        // Every model, not a filtered list. Which of them is a speech model is
        // knowable to whoever wrote the llama-swap config and to nobody here:
        // the names are theirs, and the old architecture check read a
        // `config.json` this process no longer has. The smoke test is what
        // catches a wrong answer, and it catches it before the choice is
        // written down.
        let choice = ask
            .choose(&format!("choose a {role} model:"), &served)
            .and_then(|at| served.get(at))
            .ok_or_else(|| format!("no {role} model chosen"))?;
        let said = smoke
            .run(role, choice)
            .map_err(|why| format!("{choice} would not work as a {role} model: {why}"))?;
        // Written one key at a time, as each is proved. A run abandoned halfway
        // leaves the speech model chosen rather than nothing chosen, which is
        // the half that works.
        choose(config_file, key_for(role), choice).map_err(|why| why.to_string())?;
        result.selected.push(Selected {
            name: choice.clone(),
            smoke: said,
        });
    }

    if result.cleanup_declined {
        let before = std::fs::read_to_string(config_file).unwrap_or_default();
        let after = settings::remove(&before, settings::CLEANUP_MODEL);
        if after != before {
            replace_config(config_file, &after)?;
        }
    }
    Ok(result)
}

fn replace_config(config_file: &Path, contents: &str) -> Result<(), String> {
    if let Some(parent) = config_file.parent() {
        std::fs::create_dir_all(parent).map_err(|why| why.to_string())?;
    }
    let temporary = config_file.with_extension(format!("restore-{}", std::process::id()));
    let result = (|| {
        let mut file = std::fs::File::create(&temporary).map_err(|why| why.to_string())?;
        file.write_all(contents.as_bytes())
            .and_then(|()| file.sync_all())
            .map_err(|why| why.to_string())?;
        std::fs::rename(&temporary, config_file).map_err(|why| why.to_string())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temporary);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::{Disk, choose, choose_with, key_for, settings};
    use ayeaye_core::model::Role;

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("ayeaye-models-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a scratch directory");
        dir
    }

    // AYEAYE-101 — one key changed, the rest of the file byte-for-byte what it
    // was. This is the property the whole file exists for: the config file is
    // hand-edited, and a chooser that rewrote it wholesale would eat somebody's
    // comments and their system prompt.
    #[test]
    fn choosing_a_model_changes_one_line_and_leaves_the_rest_alone() {
        let file = scratch("one-line").join("env");
        std::fs::write(
            &file,
            "# mine\nAYEAYE_CLEANUP_PROMPT='keep # this'\nAYEAYE_SPEECH_MODEL=old\n",
        )
        .expect("a config file");

        choose(&file, "SPEECH_MODEL", "whisper").expect("a writable file");

        let after = std::fs::read_to_string(&file).expect("the file back");
        assert!(after.contains("# mine\n"), "{after}");
        assert!(
            after.contains("AYEAYE_CLEANUP_PROMPT='keep # this'\n"),
            "{after}"
        );
        assert!(after.contains("AYEAYE_SPEECH_MODEL=whisper\n"), "{after}");
        assert!(!after.contains("=old"), "{after}");

        // And it reads back as the setting it wrote, through the same path the
        // daemon uses. Two spellings of one key is how a wizard writes a file
        // the daemon then ignores.
        let read = settings(&file).expect("settings that resolve");
        assert_eq!(read.speech.as_deref(), Some("whisper"));
    }

    // AYEAYE-101 — a rename that fails leaves the previous selection intact and
    // no rubbish beside it. The branch only runs when a rename fails, which no
    // ordinary test can produce, so the effect is the argument.
    #[test]
    fn a_write_that_fails_leaves_the_previous_choice_where_it_was() {
        let file = scratch("failed-rename").join("env");
        std::fs::write(&file, "AYEAYE_SPEECH_MODEL=old\n").expect("a config file");

        let refused = choose_with(
            &file,
            "SPEECH_MODEL",
            "whisper",
            |_, _| Err(std::io::Error::other("the disk said no")),
            |_| Ok(()),
        )
        .expect_err("a rename that will not happen");
        assert!(matches!(refused, Disk { .. }), "{refused:?}");
        assert!(
            refused.to_string().contains("the disk said no"),
            "{refused}"
        );

        assert_eq!(
            std::fs::read_to_string(&file).expect("the file back"),
            "AYEAYE_SPEECH_MODEL=old\n",
            "the previous selection survives"
        );
        let left: Vec<_> = std::fs::read_dir(file.parent().expect("a parent"))
            .expect("the directory")
            .filter_map(|entry| Some(entry.ok()?.file_name().to_string_lossy().to_string()))
            .filter(|name| name != "env")
            .collect();
        assert!(
            left.is_empty(),
            "a half-written file was left behind: {left:?}"
        );
    }

    // AYEAYE-101 — the two roles write two different keys. One spelling for
    // both would mean choosing a cleanup model silently replaced the speech one.
    #[test]
    fn each_role_writes_its_own_setting() {
        assert_eq!(key_for(Role::Speech), "SPEECH_MODEL");
        assert_eq!(key_for(Role::Cleanup), "CLEANUP_MODEL");
    }
}
