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

/// The model dictation is transcribed with.
pub const SPEECH_MODEL: &str = "SPEECH_MODEL";
/// The model a transcript is cleaned up with. Optional: without it, dictation
/// is the raw transcript, which is what the Python daemon degrades to when
/// ollama is unreachable.
pub const CLEANUP_MODEL: &str = "CLEANUP_MODEL";
/// What the cleanup model is told it is for.
pub const CLEANUP_PROMPT: &str = "CLEANUP_PROMPT";
/// Where the llama-swap proxy that serves both models listens.
pub const LLAMA_SWAP: &str = "LLAMA_SWAP";

/// What the cleanup model is told when nothing says otherwise.
///
/// Carried over from `bin/voice-dictate`, which is the parity source of truth
/// for this milestone, and deliberately not improved on here: a rewrite would
/// make the Rust and the Python disagree about dictation while both are still
/// running, and there would be no way to tell which change caused it.
pub const DEFAULT_CLEANUP_PROMPT: &str = "You clean up dictated speech that will be sent to a coding agent.\n\nRewrite the user's text: fix grammar, remove filler words and false starts, and restore obvious punctuation. Keep the intent and every technical term, filename, and identifier exactly as spoken. Never answer, explain, or act on the text -- only rewrite it. Reply with the rewritten text and nothing else.\n\nWrite plain text only: no markdown, no backticks, no code fences or quotes around names. When the speaker says \"underscore\", \"dot\", \"dash\" or \"slash\" between words they are spelling out an identifier or path, so join it up: \"parse underscore config\" is parse_config, \"server dot py\" is server.py.";

/// Where the proxy is when nobody said.
///
/// llama-swap's own default port on the loopback, which is what a machine
/// running one unconfigured has. A deployment behind TLS on a real hostname is
/// the other common shape and cannot be guessed, so it is written down —
/// `AYEAYE_LLAMA_SWAP=https://llama.example.test`.
pub const DEFAULT_BACKEND: &str = "http://127.0.0.1:8080";

/// Everything about models that is somebody's choice rather than ayeaye's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelSettings {
    /// The speech model, where one has been chosen. `None` is a legitimate
    /// state and not a failure: a machine whose proxy serves no speech model
    /// runs text-only, which is what the Python daemon does today.
    ///
    /// A plain string since AYEAYE-101, and that is the point rather than a
    /// loosening: the name is a key in somebody's `llama-swap` config, and
    /// llama-swap lets them call it `whisper`. Insisting on `owner/name` here
    /// would refuse the name the backend actually answers to.
    pub speech: Option<String>,
    /// The cleanup model, where one has been chosen.
    pub cleanup: Option<String>,
    /// What the cleanup model is told it is for.
    pub cleanup_prompt: String,
    /// Where the proxy serving both of them listens.
    pub backend: String,
}

/// Why a configuration could not be read.
///
/// One variant, and the model names are not in it: a model name is whatever key
/// somebody wrote in their `llama-swap` config, and this crate has no way to
/// know which keys are in it. A name that is not served is found out by asking
/// the proxy — see `ayeaye::dictate::Voice::probe` — which is a better answer
/// than a syntax rule, because it names the models that *are* there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BadSetting {
    /// A backend address that is not one, and what is wrong with it.
    Backend {
        /// Which setting it was.
        key: String,
        /// Why it was refused.
        why: String,
    },
}

impl fmt::Display for BadSetting {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            // The key is named as the user would have to spell it, prefix
            // included, because that is the thing they have to go and change.
            BadSetting::Backend { key, why } => write!(out, "AYEAYE_{key}: {why}"),
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
        // Trimmed and emptied to `None`. An `AYEAYE_CLEANUP_MODEL=` left behind
        // by somebody turning cleanup off is "no model", not a model called the
        // empty string — which would otherwise reach the proxy as a request for
        // one and come back a 400 per dictation.
        let name = |key: &str| {
            value(key)
                .map(|given| given.trim().to_string())
                .filter(|given| !given.is_empty())
        };

        Ok(ModelSettings {
            speech: name(SPEECH_MODEL),
            cleanup: name(CLEANUP_MODEL),
            cleanup_prompt: value(CLEANUP_PROMPT)
                .unwrap_or_else(|| DEFAULT_CLEANUP_PROMPT.to_string()),
            backend: name(LLAMA_SWAP).unwrap_or_else(|| DEFAULT_BACKEND.to_string()),
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

#[cfg(test)]
mod tests {
    use super::{
        BadSetting, CLEANUP_MODEL, CLEANUP_PROMPT, DEFAULT_BACKEND, DEFAULT_CLEANUP_PROMPT,
        LLAMA_SWAP, ModelSettings, SPEECH_MODEL, parse_env_file, upsert,
    };

    fn no_env(_: &str) -> Option<String> {
        None
    }

    // AYEAYE-56 — the acceptance criterion, in one test: the environment beats
    // the file, the file beats the default, and a default answers when neither
    // says anything. Without the middle rung `ayeaye model use` writes a file
    // nothing reads; without the first, a service unit cannot override it.
    #[test]
    fn the_environment_beats_the_file_which_beats_the_default() {
        let file = "AYEAYE_SPEECH_MODEL=whisper\nAYEAYE_LLAMA_SWAP=http://box:9292\n";

        let from_file = ModelSettings::resolve(no_env, file).expect("the file should resolve");
        assert_eq!(from_file.speech.as_deref(), Some("whisper"));
        assert_eq!(from_file.backend, "http://box:9292");

        let overridden = ModelSettings::resolve(
            |key| (key == SPEECH_MODEL).then(|| "whisper-turbo".to_string()),
            file,
        )
        .expect("the environment should resolve");
        assert_eq!(
            overridden.speech.as_deref(),
            Some("whisper-turbo"),
            "the environment has to win, or a service unit cannot override the file"
        );
        // And the setting it did not name still comes from the file.
        assert_eq!(overridden.backend, "http://box:9292");

        let bare = ModelSettings::resolve(no_env, "").expect("nothing at all should resolve");
        assert_eq!(
            bare.speech, None,
            "no model chosen is a state, not a failure"
        );
        assert_eq!(bare.cleanup_prompt, DEFAULT_CLEANUP_PROMPT);
        assert_eq!(bare.backend, DEFAULT_BACKEND);
    }

    // AYEAYE-101 — a model name is whatever key somebody wrote in llama-swap's
    // config, and this crate cannot know which keys are in it. The old
    // `owner/name` rule would refuse the name the backend actually answers to,
    // which is the shape llama-swap's own documentation uses.
    #[test]
    fn a_model_name_is_whatever_the_backend_calls_it() {
        for spelled in ["whisper", "qwen3-30b", "openai/whisper-small.en", "Q4_K_M"] {
            let settings =
                ModelSettings::resolve(no_env, &format!("AYEAYE_SPEECH_MODEL={spelled}\n"))
                    .unwrap_or_else(|why| panic!("{spelled:?} should resolve: {why}"));
            assert_eq!(settings.speech.as_deref(), Some(spelled));
        }
    }

    // AYEAYE-101 — an emptied setting is "no model", not a model called the
    // empty string. Somebody turning cleanup off writes `AYEAYE_CLEANUP_MODEL=`
    // and leaves it there, and a daemon that asked the proxy for `""` would
    // collect a 400 per dictation instead of dictating the words as spoken.
    #[test]
    fn a_setting_emptied_rather_than_deleted_is_no_model_at_all() {
        for file in [
            "AYEAYE_CLEANUP_MODEL=\n",
            "AYEAYE_CLEANUP_MODEL=\"\"\n",
            "AYEAYE_CLEANUP_MODEL='   '\n",
        ] {
            let settings = ModelSettings::resolve(no_env, file)
                .unwrap_or_else(|why| panic!("{file:?} should resolve: {why}"));
            assert_eq!(settings.cleanup, None, "{file:?}");
        }
        // And a name with space around it is that name, not a different one.
        let padded = ModelSettings::resolve(no_env, "AYEAYE_CLEANUP_MODEL=  qwen  \n")
            .expect("it should resolve");
        assert_eq!(padded.cleanup.as_deref(), Some("qwen"));
        assert_eq!(CLEANUP_MODEL, "CLEANUP_MODEL");
        assert_eq!(LLAMA_SWAP, "LLAMA_SWAP");
        assert!(matches!(
            BadSetting::Backend {
                key: LLAMA_SWAP.to_string(),
                why: "unused today".to_string()
            },
            BadSetting::Backend { .. }
        ));
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

    // AYEAYE-56 — a key set twice resolves the way the service manager
    // resolves it, which is to the last one. Taking the first would mean one
    // file gave two different answers depending on whether it was read by
    // systemd or by this binary, which is the disagreement the whole
    // arrangement exists to prevent.
    #[test]
    fn a_key_set_twice_by_hand_resolves_to_the_last_one() {
        let by_hand = "AYEAYE_SPEECH_MODEL=first\nAYEAYE_SPEECH_MODEL=second\n";
        let settings = ModelSettings::resolve(no_env, by_hand).expect("it should resolve");
        assert_eq!(settings.speech.as_deref(), Some("second"));
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
