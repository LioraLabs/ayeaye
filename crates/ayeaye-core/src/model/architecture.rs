//! The architecture allowlist, and the refusal it produces.

use std::fmt;

/// An architecture this build implements.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Architecture {
    /// Whisper, the speech model AYEAYE-54 wired up.
    Whisper,
}

/// One entry in the allowlist.
pub struct Entry {
    /// The architecture itself.
    pub architecture: Architecture,
    /// What HuggingFace's `config.json` calls it in its `architectures` array.
    pub hf_name: &'static str,
    /// What the same file calls it in `model_type`.
    pub model_type: &'static str,
    /// What it is for, in the words a refusal will quote at somebody.
    pub what: &'static str,
}

/// Every architecture this build can run.
pub const SUPPORTED: &[Entry] = &[Entry {
    architecture: Architecture::Whisper,
    hf_name: "WhisperForConditionalGeneration",
    model_type: "whisper",
    what: "speech, transcription",
}];

/// An architecture this build does not implement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unsupported {
    /// What the model's own config called itself, where it said anything.
    pub found: Option<String>,
}

impl fmt::Display for Unsupported {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.found {
            // Quoted and shortened, because this string came out of a stranger's
            // `config.json` and lands in somebody's terminal. Raw it can carry
            // ANSI escapes, newlines that forge a second line of output, or a
            // couple of megabytes of nothing. `{:?}` escapes the control bytes;
            // `shorten` deals with the length. It also reads better for the
            // ordinary case, where the name differs from a supported one only
            // by whitespace nobody can see.
            Some(found) => write!(
                out,
                "{:?} is not an architecture this build can run",
                shorten(found)
            )?,
            None => write!(
                out,
                "this model's config.json names no architecture: it has neither an \
                 \"architectures\" array nor a \"model_type\""
            )?,
        }
        write!(
            out,
            ". ayeaye implements architectures one at a time rather than carrying a registry, \
             so this is refused now instead of at the first transcription. It runs: "
        )?;
        for (index, entry) in SUPPORTED.iter().enumerate() {
            if index > 0 {
                write!(out, ", ")?;
            }
            write!(out, "{} ({})", entry.hf_name, entry.what)?;
        }
        Ok(())
    }
}

impl std::error::Error for Unsupported {}

impl Architecture {
    /// What HuggingFace calls this architecture.
    pub fn hf_name(self) -> &'static str {
        self.entry().hf_name
    }

    fn entry(self) -> &'static Entry {
        // Matched rather than searched, so "an architecture with no entry" is
        // a shape the compiler refuses instead of a panic somebody has to
        // trust will not fire.
        match self {
            Architecture::Whisper => &SUPPORTED[0],
        }
    }
}

/// Judge a name against the allowlist.
///
/// The name may be either spelling a `config.json` uses — the entry in the
/// `architectures` array, or `model_type` — because a model that carries only
/// the second is still identifiable and refusing it for the wrong reason would
/// be a worse answer than the right one.
///
/// Compared exactly. These are class names written by an exporter, not
/// something a person types, so matching loosely would only ever admit a name
/// no real config carries — and it would do it silently.
pub fn judge(found: &str) -> Result<Architecture, Unsupported> {
    SUPPORTED
        .iter()
        .find(|entry| entry.hf_name == found || entry.model_type == found)
        .map(|entry| entry.architecture)
        .ok_or_else(|| Unsupported {
            found: Some(found.to_string()),
        })
}

/// As much of an untrusted name as is worth putting in a message.
///
/// A repository can call itself anything, including two megabytes of it. The
/// refusal has to name what was found, and naming the first eighty characters
/// of it does that; naming all of it turns an error message into the payload.
fn shorten(found: &str) -> String {
    const MOST: usize = 80;
    match found.char_indices().nth(MOST) {
        Some((at, _)) => format!("{}…", &found[..at]),
        None => found.to_string(),
    }
}

/// Judge the architecture a model's `config.json` declares.
///
/// This is the whole bound, in one call: a `config.json` is a couple of
/// kilobytes, so an unsupported model costs that and not the weights. A
/// config that names nothing is refused too — a file ayeaye cannot identify
/// is not a model it should go on to download half a gigabyte for.
pub fn in_config(config_json: &str) -> Result<Architecture, Unsupported> {
    match super::config::architecture_name(config_json) {
        Some(name) => judge(&name),
        None => Err(Unsupported { found: None }),
    }
}

#[cfg(test)]
mod tests {
    use super::{Architecture, Unsupported, in_config, judge};

    // AYEAYE-56 — the two halves joined: a config this build can run is
    // admitted, and one it cannot is refused by name, both from the small file
    // that is fetched before any weights are.
    #[test]
    fn a_config_is_judged_by_the_architecture_it_declares() {
        assert_eq!(
            in_config(r#"{"architectures": ["WhisperForConditionalGeneration"]}"#),
            Ok(Architecture::Whisper)
        );
        assert_eq!(
            in_config(r#"{"architectures": ["LlamaForCausalLM"]}"#).unwrap_err(),
            Unsupported {
                found: Some("LlamaForCausalLM".to_string())
            }
        );
        // A file that identifies nothing is not a model either. The realistic
        // case is a hub serving an error page under the name `config.json`.
        let nameless = in_config("<!DOCTYPE html><title>404</title>").unwrap_err();
        assert_eq!(nameless, Unsupported { found: None });

        // And that refusal has to read as something a person can act on, since
        // "unsupported" about a file that is not a config at all sends them
        // looking for the wrong problem.
        let said = nameless.to_string();
        assert!(said.contains("names no architecture"), "{said}");
        assert!(said.contains("architectures"), "{said}");
        assert!(said.contains("WhisperForConditionalGeneration"), "{said}");
    }

    // AYEAYE-56 — the refusal has to name what was found *and* what this build
    // runs. "unsupported architecture" alone leaves somebody who typed a Llama
    // repository id with no way to find out what they should have typed.
    #[test]
    fn an_unsupported_architecture_is_refused_and_the_message_names_both_sides() {
        let refused = judge("LlamaForCausalLM").unwrap_err();
        assert_eq!(
            refused,
            Unsupported {
                found: Some("LlamaForCausalLM".to_string())
            }
        );

        let said = refused.to_string();
        assert!(said.contains("LlamaForCausalLM"), "{said}");
        assert!(said.contains("WhisperForConditionalGeneration"), "{said}");
    }

    // AYEAYE-56 — the name in a refusal came out of a stranger's config.json
    // and is printed to somebody's terminal. Raw, it can repaint the screen,
    // forge a second line of output, or run to megabytes.
    #[test]
    fn a_hostile_name_cannot_reach_a_terminal_intact() {
        let nasty = judge("\u{1b}[2J\u{1b}[31mfake error\nayeaye: all good")
            .unwrap_err()
            .to_string();
        assert!(
            !nasty.contains('\u{1b}'),
            "an escape byte survived: {nasty:?}"
        );
        assert!(
            !nasty.contains('\n'),
            "a newline survived, so the message can forge a line: {nasty:?}"
        );

        let huge = judge(&"a".repeat(2_000)).unwrap_err().to_string();
        assert!(
            huge.len() < 400,
            "a refusal became the payload: {} bytes",
            huge.len()
        );
        // Still says enough of it to be recognisable.
        assert!(huge.contains("aaaa"), "{huge}");

        // And an ordinary name is still readable rather than mangled.
        let plain = judge("LlamaForCausalLM").unwrap_err().to_string();
        assert!(plain.contains("LlamaForCausalLM"), "{plain}");
    }

    // AYEAYE-56 — and the one architecture AYEAYE-54 shipped is admitted, by
    // either of the two names a config.json can call it.
    #[test]
    fn the_architecture_this_build_implements_is_admitted_by_either_name() {
        assert_eq!(
            judge("WhisperForConditionalGeneration"),
            Ok(Architecture::Whisper)
        );
        assert_eq!(judge("whisper"), Ok(Architecture::Whisper));
        assert_eq!(
            Architecture::Whisper.hf_name(),
            "WhisperForConditionalGeneration"
        );
    }
}
