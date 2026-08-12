//! Model choice and system prompts, as configuration.
//!
//! Two halves, both pure. [`ModelSettings::resolve`] decides what wins, and
//! takes both the environment and the configuration file as arguments so every
//! "which one of these three answers" decision is testable without a test
//! mutating a process environment it shares with every other test. [`upsert`]
//! is the writing half: one key changed, the file otherwise byte-for-byte what
//! it was.
//!
//! The file is `~/.config/ayeaye/env`, which is already the file the systemd
//! unit points `EnvironmentFile=` at. That is the whole reason the precedence
//! below is the right way round: under the service the file has *become* the
//! environment by the time this runs, and run by hand it has not, so reading
//! both and letting the real environment win is what makes the two agree.

use std::fmt;
use std::time::Duration;

use super::id::{BadModelId, ModelId};

/// The model dictation is transcribed with.
pub const SPEECH_MODEL: &str = "SPEECH_MODEL";
/// The model a transcript is cleaned up with. Optional: without it, dictation
/// is the raw transcript, which is what the Python daemon degrades to when
/// ollama is unreachable.
pub const CLEANUP_MODEL: &str = "CLEANUP_MODEL";
/// What the cleanup model is told it is for.
pub const CLEANUP_PROMPT: &str = "CLEANUP_PROMPT";
/// How long a model stays resident with nothing to do.
pub const MODEL_IDLE: &str = "MODEL_IDLE";
/// Where models are fetched from.
pub const MODEL_HUB: &str = "MODEL_HUB";

/// What the cleanup model is told when nothing says otherwise.
///
/// Carried over from `bin/voice-dictate`, which is the parity source of truth
/// for this milestone, and deliberately not improved on here: a rewrite would
/// make the Rust and the Python disagree about dictation while both are still
/// running, and there would be no way to tell which change caused it.
pub const DEFAULT_CLEANUP_PROMPT: &str = "You clean up dictated speech that will be sent to a coding agent.\n\nRewrite the user's text: fix grammar, remove filler words and false starts, and restore obvious punctuation. Keep the intent and every technical term, filename, and identifier exactly as spoken. Never answer, explain, or act on the text -- only rewrite it. Reply with the rewritten text and nothing else.\n\nWrite plain text only: no markdown, no backticks, no code fences or quotes around names. When the speaker says \"underscore\", \"dot\", \"dash\" or \"slash\" between words they are spelling out an identifier or path, so join it up: \"parse underscore config\" is parse_config, \"server dot py\" is server.py.";

/// How long a model stays resident with nothing to do, by default.
///
/// Five minutes, matching the shape of `VOICE_KEEP_ALIVE` rather than its
/// value: ollama held a model for an hour because reloading it meant talking to
/// another process. Here a reload is this process reading its own files, and
/// the memory is this process's to give back.
pub const DEFAULT_IDLE: Duration = Duration::from_secs(300);

/// Everything about models that is somebody's choice rather than ayeaye's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelSettings {
    /// The speech model, where one has been chosen. `None` is a legitimate
    /// state and not a failure: a machine that has not pulled a model yet runs
    /// text-only, which is what the Python daemon does today.
    pub speech: Option<ModelId>,
    /// The cleanup model, where one has been chosen.
    pub cleanup: Option<ModelId>,
    /// What the cleanup model is told it is for.
    pub cleanup_prompt: String,
    /// How long a resident model is kept with nothing to do. `None` means it is
    /// never released on time alone.
    pub idle: Option<Duration>,
    /// Where models are fetched from.
    pub hub: String,
}

/// Why a configuration could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BadSetting {
    /// A model id that is not one.
    Model {
        /// Which setting it was.
        key: String,
        /// Why the id was refused.
        why: BadModelId,
    },
    /// A duration that is not one.
    Duration {
        /// Which setting it was.
        key: String,
        /// What was given.
        given: String,
    },
}

impl fmt::Display for BadSetting {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            // The key is named as the user would have to spell it, prefix
            // included, because that is the thing they have to go and change.
            BadSetting::Model { key, why } => write!(out, "AYEAYE_{key}: {why}"),
            BadSetting::Duration { key, given } => write!(
                out,
                "AYEAYE_{key}: {given:?} is not a length of time. Write seconds, \
                 or a number with s, m or h after it, or 0 to keep the model \
                 loaded until something else releases it"
            ),
        }
    }
}

impl std::error::Error for BadSetting {}

impl ModelSettings {
    /// Resolve from the environment, falling back to the configuration file,
    /// falling back to the defaults.
    ///
    /// `env` is a lookup rather than `std::env`, and `file` is text rather than
    /// a path, for the same reason: both make the precedence a decision a test
    /// can drive. `env` is handed the *bare* name and is expected to have
    /// already tried `AYEAYE_<name>` and the legacy `VOICE_REMOTE_<name>`,
    /// which is what the binary's `env_var` does.
    pub fn resolve(
        env: impl Fn(&str) -> Option<String>,
        file: &str,
    ) -> Result<ModelSettings, BadSetting> {
        let from_file = parse_env_file(file);
        let value = |key: &str| {
            env(key).or_else(|| {
                // The *last* occurrence, not the first. A key set twice is what
                // systemd's `EnvironmentFile=` resolves to the last one, so
                // taking the first here would mean the same file said one thing
                // under the service unit and the opposite run by hand — which
                // is the disagreement this module exists to prevent rather than
                // to introduce. `upsert` never writes a duplicate, so this only
                // ever arises in a hand-edited file, which is exactly the file
                // whose author expects it to behave like every other env file.
                from_file
                    .iter()
                    .rev()
                    .find(|(name, _)| name == key)
                    .map(|(_, value)| value.clone())
            })
        };
        let model = |key: &str| match value(key) {
            Some(given) => ModelId::parse(&given)
                .map(Some)
                .map_err(|why| BadSetting::Model {
                    key: key.to_string(),
                    why,
                }),
            None => Ok(None),
        };

        Ok(ModelSettings {
            speech: model(SPEECH_MODEL)?,
            cleanup: model(CLEANUP_MODEL)?,
            cleanup_prompt: value(CLEANUP_PROMPT)
                .unwrap_or_else(|| DEFAULT_CLEANUP_PROMPT.to_string()),
            idle: match value(MODEL_IDLE) {
                Some(given) => parse_duration(&given).ok_or(BadSetting::Duration {
                    key: MODEL_IDLE.to_string(),
                    given,
                })?,
                None => Some(DEFAULT_IDLE),
            },
            hub: value(MODEL_HUB).unwrap_or_else(|| super::hub::DEFAULT_HOST.to_string()),
        })
    }
}

/// Read an environment file into the pairs it sets.
///
/// The shape systemd's `EnvironmentFile=` accepts and the shape `bin/ayeaye`
/// already writes: `KEY=value`, blank lines, and `#` comments. `export KEY=` is
/// taken too, because a file somebody has also been sourcing from a shell is
/// the common case and refusing it would be a distinction without a difference.
///
/// One pair of surrounding quotes is removed. Anything else is the value: a
/// system prompt with an apostrophe in it must survive, and a `#` after a value
/// is part of it rather than the start of a comment, which is what systemd does
/// too.
pub fn parse_env_file(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                return None;
            }
            let line = line.strip_prefix("export ").unwrap_or(line);
            let (key, value) = line.split_once('=')?;
            let key = key.trim();
            if key.is_empty() {
                return None;
            }
            Some((
                key.trim_start_matches("AYEAYE_").to_string(),
                unquote(value),
            ))
        })
        .collect()
}

/// Strip one surrounding pair of quotes, and undo what [`upsert`] escaped.
///
/// The two halves have to agree or the file is write-only: a system prompt is
/// the setting most likely to carry a newline and a quote, and one written out
/// escaped and read back raw comes home with visible backslashes in it. That is
/// exactly what happened here, and the round-trip test is what said so.
///
/// Double quotes interpret the escapes, single quotes do not — which is what
/// systemd's `EnvironmentFile=` does, and what a shell does.
fn unquote(value: &str) -> String {
    let value = value.trim();
    if value.len() >= 2 && value.starts_with('\'') && value.ends_with('\'') {
        return value[1..value.len() - 1].to_string();
    }
    if !(value.len() >= 2 && value.starts_with('"') && value.ends_with('"')) {
        return value.to_string();
    }

    let mut out = String::with_capacity(value.len());
    let mut rest = value[1..value.len() - 1].chars();
    while let Some(character) = rest.next() {
        if character != '\\' {
            out.push(character);
            continue;
        }
        match rest.next() {
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('t') => out.push('\t'),
            // A backslash before anything else is that thing, which is what
            // leaves a Windows-shaped path alone rather than eating it.
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

/// Set one key in an environment file, leaving everything else exactly as it
/// was.
///
/// Rewriting the whole file from a struct would be less code and would throw
/// away every comment in it. This is a file people edit by hand — the template
/// it grows from is all comments — so a `ayeaye model use` that ate them would
/// be a worse tool than the one it replaced.
///
/// The key is written with its `AYEAYE_` prefix, and an existing line is
/// replaced **in place** rather than removed and appended, so the file does not
/// slowly reorder itself every time something is set.
pub fn upsert(text: &str, key: &str, value: &str) -> String {
    let prefixed = format!("AYEAYE_{key}");
    let line = format!("{prefixed}={}", quote_if_needed(value));

    let mut out = String::with_capacity(text.len() + line.len() + 1);
    let mut replaced = false;
    for existing in text.lines() {
        let names_it = parse_env_file(existing)
            .first()
            .is_some_and(|(name, _)| name == key);
        if names_it && !replaced {
            out.push_str(&line);
            replaced = true;
        } else if names_it {
            // A second line setting the same key is dropped rather than left
            // behind: the last one would win at load, so leaving it would mean
            // the file says one thing and the process does another.
            continue;
        } else {
            out.push_str(existing);
        }
        out.push('\n');
    }
    if !replaced {
        out.push_str(&line);
        out.push('\n');
    }
    out
}

/// Remove every occurrence of one setting, leaving everything else in place.
pub fn remove(text: &str, key: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for line in text.lines() {
        if parse_env_file(line)
            .first()
            .is_some_and(|(name, _)| name == key)
        {
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// Quote a value that would not survive the round trip unquoted.
fn quote_if_needed(value: &str) -> String {
    let plain = !value.is_empty()
        && value.trim() == value
        && !value.contains(['\n', '"', '\''])
        && !value.starts_with('#');
    if plain {
        value.to_string()
    } else {
        // Newlines are escaped rather than written raw: an environment file is
        // line-oriented, and a raw newline in a value produces a second line
        // that reads as a different setting.
        format!(
            "\"{}\"",
            value
                .replace('\\', r"\\")
                .replace('"', "\\\"")
                .replace('\n', "\\n")
        )
    }
}

/// Read a length of time: bare seconds, or a number with `s`, `m` or `h`.
///
/// `Some(None)` for zero, which means "do not release it on time alone" rather
/// than "release it immediately" — an idle timeout of nothing would unload a
/// model between two sentences.
fn parse_duration(given: &str) -> Option<Option<Duration>> {
    let given = given.trim();
    let (number, scale) = match given.strip_suffix(['s', 'S']) {
        Some(rest) => (rest, 1),
        None => match given.strip_suffix(['m', 'M']) {
            Some(rest) => (rest, 60),
            None => match given.strip_suffix(['h', 'H']) {
                Some(rest) => (rest, 3600),
                None => (given, 1),
            },
        },
    };
    let seconds: u64 = number.trim().parse().ok()?;
    // Checked, because this is a number a person typed and `18446744073709551h`
    // would otherwise panic in a debug build and wrap silently in a release
    // one — into an arbitrary idle timeout. `verify.rs` is careful about
    // exactly this for the same reason; a pure function judging a stranger's
    // text does not get to assume the text is reasonable.
    let total = seconds.checked_mul(scale)?;
    Some((total > 0).then(|| Duration::from_secs(total)))
}

#[cfg(test)]
mod tests {
    use super::{
        BadSetting, CLEANUP_PROMPT, DEFAULT_CLEANUP_PROMPT, DEFAULT_IDLE, MODEL_IDLE,
        ModelSettings, SPEECH_MODEL, parse_env_file, upsert,
    };
    use std::time::Duration;

    fn no_env(_: &str) -> Option<String> {
        None
    }

    // AYEAYE-56 — the acceptance criterion, in one test: the environment beats
    // the file, the file beats the default, and a default answers when neither
    // says anything. Without the middle rung `ayeaye model use` writes a file
    // nothing reads; without the first, a service unit cannot override it.
    #[test]
    fn the_environment_beats_the_file_which_beats_the_default() {
        let file = "AYEAYE_SPEECH_MODEL=openai/whisper-small.en\nAYEAYE_MODEL_IDLE=1h\n";

        let from_file = ModelSettings::resolve(no_env, file).expect("the file should resolve");
        assert_eq!(
            from_file
                .speech
                .as_ref()
                .map(ToString::to_string)
                .as_deref(),
            Some("openai/whisper-small.en")
        );
        assert_eq!(from_file.idle, Some(Duration::from_secs(3600)));

        let overridden = ModelSettings::resolve(
            |key| (key == SPEECH_MODEL).then(|| "openai/whisper-tiny.en".to_string()),
            file,
        )
        .expect("the environment should resolve");
        assert_eq!(
            overridden
                .speech
                .as_ref()
                .map(ToString::to_string)
                .as_deref(),
            Some("openai/whisper-tiny.en"),
            "the environment has to win, or a service unit cannot override the file"
        );
        // And the setting it did not name still comes from the file.
        assert_eq!(overridden.idle, Some(Duration::from_secs(3600)));

        let bare = ModelSettings::resolve(no_env, "").expect("nothing at all should resolve");
        assert_eq!(
            bare.speech, None,
            "no model chosen is a state, not a failure"
        );
        assert_eq!(bare.idle, Some(DEFAULT_IDLE));
        assert_eq!(bare.cleanup_prompt, DEFAULT_CLEANUP_PROMPT);
        assert_eq!(bare.hub, "https://huggingface.co");
    }

    // AYEAYE-56 — a system prompt is configuration, and it is the setting most
    // likely to carry the characters an environment file is worst at.
    #[test]
    fn a_system_prompt_survives_the_file_it_is_written_in() {
        let prompt = "Rewrite it. Don't \"answer\" it.\nKeep names as spoken.";
        let file = upsert("", CLEANUP_PROMPT, prompt);

        let settings = ModelSettings::resolve(no_env, &file).expect("it should resolve");
        assert_eq!(
            settings.cleanup_prompt, prompt,
            "a prompt that does not survive being written and read back is not \
             configuration, whatever the file says"
        );
    }

    // AYEAYE-56 — a bad value is refused, naming the setting as the user would
    // have to spell it. Silently falling back to the default would leave
    // somebody who typed the model name wrong wondering why it never loaded.
    #[test]
    fn a_setting_that_is_not_one_is_refused_and_names_itself() {
        let refused = ModelSettings::resolve(no_env, "AYEAYE_SPEECH_MODEL=whisper\n").unwrap_err();
        assert!(matches!(refused, BadSetting::Model { .. }), "{refused:?}");
        let said = refused.to_string();
        assert!(said.contains("AYEAYE_SPEECH_MODEL"), "{said}");
        assert!(said.contains("owner/name"), "{said}");

        let bad_time = ModelSettings::resolve(no_env, "AYEAYE_MODEL_IDLE=soon\n").unwrap_err();
        assert_eq!(
            bad_time,
            BadSetting::Duration {
                key: MODEL_IDLE.to_string(),
                given: "soon".to_string()
            }
        );
    }

    // AYEAYE-56 — zero is "keep it", not "drop it immediately". An idle
    // timeout of nothing would unload the model between two sentences, which
    // is the opposite of what somebody setting it to zero wants.
    #[test]
    fn an_idle_time_of_zero_keeps_the_model_rather_than_dropping_it_at_once() {
        let kept = ModelSettings::resolve(no_env, "AYEAYE_MODEL_IDLE=0\n").expect("zero resolves");
        assert_eq!(kept.idle, None);

        for (given, expected) in [
            ("90", 90),
            ("90s", 90),
            ("5m", 300),
            ("2h", 7200),
            (" 5m ", 300),
        ] {
            let settings = ModelSettings::resolve(no_env, &format!("AYEAYE_MODEL_IDLE={given}\n"))
                .unwrap_or_else(|why| panic!("{given:?} should resolve: {why}"));
            assert_eq!(
                settings.idle,
                Some(Duration::from_secs(expected)),
                "{given:?}"
            );
        }
    }

    // AYEAYE-56 — a length of time nobody could mean must be refused rather
    // than wrapped. In a release build the multiplication has no overflow
    // check, so a wrapped value would become an arbitrary idle timeout; in a
    // debug build it panics. Neither is an answer.
    #[test]
    fn a_length_of_time_too_large_to_be_one_is_refused() {
        let refused =
            ModelSettings::resolve(no_env, "AYEAYE_MODEL_IDLE=18446744073709551h\n").unwrap_err();
        assert_eq!(
            refused,
            BadSetting::Duration {
                key: MODEL_IDLE.to_string(),
                given: "18446744073709551h".to_string()
            }
        );
        // And the largest one that *is* a length of time still resolves.
        let vast = ModelSettings::resolve(no_env, "AYEAYE_MODEL_IDLE=100000h\n")
            .expect("a big but representable idle time");
        assert_eq!(vast.idle, Some(Duration::from_secs(360_000_000)));
    }

    // AYEAYE-56 — a key set twice resolves the way the service manager
    // resolves it, which is to the last one. Taking the first would mean one
    // file gave two different answers depending on whether it was read by
    // systemd or by this binary, which is the disagreement the whole
    // arrangement exists to prevent.
    #[test]
    fn a_key_set_twice_by_hand_resolves_to_the_last_one() {
        let by_hand = "AYEAYE_SPEECH_MODEL=openai/first\nAYEAYE_SPEECH_MODEL=openai/second\n";
        let settings = ModelSettings::resolve(no_env, by_hand).expect("it should resolve");
        assert_eq!(
            settings.speech.map(|id| id.to_string()).as_deref(),
            Some("openai/second")
        );
    }

    // AYEAYE-56 — the file's own grammar, including the parts that would
    // otherwise be read as settings.
    #[test]
    fn the_file_is_read_the_way_the_service_manager_reads_it() {
        let file = "\
# a comment
   # an indented comment

AYEAYE_SPEECH_MODEL=openai/whisper-small.en
export AYEAYE_MODEL_HUB=https://example.test
AYEAYE_CLEANUP_PROMPT='keep # this'
not a setting
=novalue
";
        let pairs = parse_env_file(file);
        assert_eq!(
            pairs,
            vec![
                (
                    "SPEECH_MODEL".to_string(),
                    "openai/whisper-small.en".to_string()
                ),
                ("MODEL_HUB".to_string(), "https://example.test".to_string()),
                // A `#` after a value is part of it, which is what systemd does.
                ("CLEANUP_PROMPT".to_string(), "keep # this".to_string()),
            ]
        );
    }

    // AYEAYE-56 — the writing half. This file is one people edit by hand, so a
    // write that ate their comments or reordered their settings would be a
    // worse tool than the one it replaces.
    #[test]
    fn writing_a_setting_replaces_it_in_place_and_keeps_everything_else() {
        let before = "\
# which model listens
AYEAYE_SPEECH_MODEL=openai/whisper-tiny.en
# how long it stays
AYEAYE_MODEL_IDLE=5m
";
        let after = upsert(before, SPEECH_MODEL, "openai/whisper-small.en");
        assert_eq!(
            after,
            "\
# which model listens
AYEAYE_SPEECH_MODEL=openai/whisper-small.en
# how long it stays
AYEAYE_MODEL_IDLE=5m
"
        );

        // A key the file does not have is appended rather than losing the rest.
        let added = upsert(&after, "MODEL_HUB", "https://example.test");
        assert!(added.starts_with("# which model listens\n"), "{added}");
        assert!(
            added.ends_with("AYEAYE_MODEL_HUB=https://example.test\n"),
            "{added}"
        );
        assert!(added.contains("AYEAYE_MODEL_IDLE=5m\n"), "{added}");

        // And writing into an empty file produces a file, not a stray line.
        assert_eq!(
            upsert("", SPEECH_MODEL, "openai/whisper-tiny.en"),
            "AYEAYE_SPEECH_MODEL=openai/whisper-tiny.en\n"
        );
    }

    // AYEAYE-56 — a key set twice would have the last one win at load, so
    // leaving the first behind would mean the file says one thing and the
    // process does another.
    #[test]
    fn a_key_set_twice_ends_up_set_once() {
        let doubled =
            "AYEAYE_SPEECH_MODEL=a/one\nAYEAYE_MODEL_IDLE=5m\nAYEAYE_SPEECH_MODEL=a/two\n";
        let after = upsert(doubled, SPEECH_MODEL, "a/three");
        assert_eq!(after, "AYEAYE_SPEECH_MODEL=a/three\nAYEAYE_MODEL_IDLE=5m\n");
    }
}
