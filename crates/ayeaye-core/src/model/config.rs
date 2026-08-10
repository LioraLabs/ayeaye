//! The one field of a model's `config.json` that decides whether it can run.
//!
//! Deliberately not a JSON reader. `ayeaye_core::json` writes JSON and says out
//! loud that it holds no parser, and this does not turn it into one: it answers
//! a single question — what does this model call its architecture — and answers
//! `None` to everything else, including to text that is not JSON at all.
//!
//! It still has to walk the document properly rather than search it for a
//! substring. `"architectures"` occurring inside a *string value*, or inside a
//! nested object such as a config's `decoder`, is not the top-level key, and a
//! scanner that cannot tell those apart would read a model's description as its
//! architecture and refuse something it can run.

/// What this model calls its architecture, if it says.
///
/// The `architectures` array first, since that is what HuggingFace's exporters
/// write and what names the class `candle-transformers` would have to
/// implement; `model_type` second, because some configs carry only that and
/// refusing one of those for the wrong reason is a worse answer than the right
/// one.
pub fn architecture_name(config_json: &str) -> Option<String> {
    if let Some(array) = top_level(config_json, "architectures")
        && let Some(first) = first_string(array)
    {
        return Some(first);
    }
    top_level(config_json, "model_type").and_then(unquote)
}

/// The raw text of a top-level key's value.
///
/// Top-level meaning a key of the root object and nothing deeper — the whole
/// point of walking rather than searching.
fn top_level<'a>(json: &'a str, key: &str) -> Option<&'a str> {
    let bytes = json.as_bytes();
    let mut at = skip_space(bytes, 0);
    if bytes.get(at) != Some(&b'{') {
        return None;
    }
    at = skip_space(bytes, at + 1);

    loop {
        if bytes.get(at) == Some(&b'}') {
            return None;
        }
        let (name, after_name) = read_string(bytes, at)?;
        at = skip_space(bytes, after_name);
        if bytes.get(at) != Some(&b':') {
            return None;
        }
        let start = skip_space(bytes, at + 1);
        let end = skip_value(bytes, start)?;
        if name == key {
            return Some(json[start..end].trim());
        }
        at = skip_space(bytes, end);
        match bytes.get(at) {
            Some(b',') => at = skip_space(bytes, at + 1),
            _ => return None,
        }
    }
}

/// The first string in an array, where the value really is an array of them.
fn first_string(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    if bytes.first() != Some(&b'[') {
        return None;
    }
    let at = skip_space(bytes, 1);
    read_string(bytes, at).map(|(text, _)| text)
}

/// A value that is a single string literal, unescaped.
fn unquote(value: &str) -> Option<String> {
    read_string(value.as_bytes(), 0).map(|(text, _)| text)
}

/// Read a JSON string literal starting at `at`, returning it and the index
/// just past its closing quote.
fn read_string(bytes: &[u8], at: usize) -> Option<(String, usize)> {
    if bytes.get(at) != Some(&b'"') {
        return None;
    }
    let mut out = String::new();
    let mut i = at + 1;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => return Some((out, i + 1)),
            b'\\' => {
                let (character, next) = read_escape(bytes, i + 1)?;
                out.push(character);
                i = next;
            }
            // Everything else is passed through as the bytes it already is, so
            // a UTF-8 name survives. A lone continuation byte cannot appear
            // here: the input arrived as `&str`.
            _ => {
                let start = i;
                while i < bytes.len() && bytes[i] != b'"' && bytes[i] != b'\\' {
                    i += 1;
                }
                out.push_str(std::str::from_utf8(&bytes[start..i]).ok()?);
            }
        }
    }
    None
}

/// The character a backslash escape names, and the index just past it.
fn read_escape(bytes: &[u8], at: usize) -> Option<(char, usize)> {
    let simple = |c: char| Some((c, at + 1));
    match bytes.get(at)? {
        b'"' => simple('"'),
        b'\\' => simple('\\'),
        b'/' => simple('/'),
        b'b' => simple('\u{8}'),
        b'f' => simple('\u{c}'),
        b'n' => simple('\n'),
        b'r' => simple('\r'),
        b't' => simple('\t'),
        b'u' => {
            let hex = bytes.get(at + 1..at + 5)?;
            let code = u32::from_str_radix(std::str::from_utf8(hex).ok()?, 16).ok()?;
            // A surrogate half is not a character. Nothing in an architecture
            // name needs one, and refusing beats inventing a replacement that
            // would then fail to match the allowlist for a second reason.
            Some((char::from_u32(code)?, at + 5))
        }
        _ => None,
    }
}

/// The index just past the value starting at `at`, whatever kind it is.
fn skip_value(bytes: &[u8], at: usize) -> Option<usize> {
    match bytes.get(at)? {
        b'"' => read_string(bytes, at).map(|(_, next)| next),
        open @ (b'{' | b'[') => {
            let close = if *open == b'{' { b'}' } else { b']' };
            let mut depth = 0usize;
            let mut i = at;
            while i < bytes.len() {
                match bytes[i] {
                    b'"' => i = read_string(bytes, i)?.1,
                    byte if byte == *open => {
                        depth += 1;
                        i += 1;
                    }
                    byte if byte == close => {
                        depth -= 1;
                        i += 1;
                        if depth == 0 {
                            return Some(i);
                        }
                    }
                    // A nested container of the other kind, whose own braces
                    // must not be counted against this one's depth.
                    b'{' | b'[' => i = skip_value(bytes, i)?,
                    _ => i += 1,
                }
            }
            None
        }
        // A number, `true`, `false` or `null`: it ends where the object does.
        _ => {
            let mut i = at;
            while i < bytes.len() && !matches!(bytes[i], b',' | b'}' | b']') {
                i += 1;
            }
            (i > at).then_some(i)
        }
    }
}

fn skip_space(bytes: &[u8], from: usize) -> usize {
    let mut i = from;
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    i
}

#[cfg(test)]
mod tests {
    use super::architecture_name;

    /// The shape a real Whisper `config.json` has, cut down to the keys that
    /// matter here plus enough of the others to be a fair test.
    const WHISPER: &str = r#"{
      "architectures": ["WhisperForConditionalGeneration"],
      "d_model": 384,
      "decoder_layers": 4,
      "is_encoder_decoder": true,
      "model_type": "whisper",
      "suppress_tokens": [1, 2, 7],
      "begin_suppress_tokens": [220, 50256]
    }"#;

    // AYEAYE-56 — the field is read out of a real-shaped config.
    #[test]
    fn the_architecture_is_read_from_a_real_shaped_config() {
        assert_eq!(
            architecture_name(WHISPER).as_deref(),
            Some("WhisperForConditionalGeneration")
        );
    }

    // AYEAYE-56 — a config that carries only model_type is still identifiable.
    #[test]
    fn model_type_answers_when_there_is_no_architectures_array() {
        assert_eq!(
            architecture_name(r#"{"model_type": "whisper"}"#).as_deref(),
            Some("whisper")
        );
    }

    // AYEAYE-56 — the reason this walks the document instead of searching it.
    // A substring search would read a model's own description as its
    // architecture and refuse something this build can run.
    #[test]
    fn the_key_inside_a_string_value_does_not_answer() {
        let sneaky = r#"{
          "description": "exported with \"architectures\": [\"LlamaForCausalLM\"]",
          "model_type": "whisper"
        }"#;
        assert_eq!(architecture_name(sneaky).as_deref(), Some("whisper"));
    }

    // AYEAYE-56 — nor does the same key nested inside another object, which a
    // composite config really does carry.
    #[test]
    fn the_key_nested_in_another_object_does_not_answer() {
        let nested = r#"{
          "decoder": {"architectures": ["LlamaForCausalLM"], "layers": 4},
          "encoder": [{"architectures": ["AlsoNotIt"]}],
          "model_type": "whisper"
        }"#;
        assert_eq!(architecture_name(nested).as_deref(), Some("whisper"));
    }

    // AYEAYE-56 — escapes are decoded, so a name is compared as itself.
    #[test]
    fn escapes_in_the_name_are_decoded() {
        assert_eq!(
            architecture_name(r#"{"architectures": ["A\"BC"]}"#).as_deref(),
            Some("A\"BC")
        );
    }

    // AYEAYE-56 — a config that says nothing, and text that is not JSON at
    // all, both answer "nothing" rather than guessing. The HTML error page is
    // the realistic one: it is what a hub serves when a repository is private
    // or misspelled, and it is saved under the name `config.json`.
    #[test]
    fn a_config_that_names_nothing_and_a_page_that_is_not_one_both_answer_nothing() {
        assert_eq!(architecture_name(r#"{"d_model": 384}"#), None);
        assert_eq!(architecture_name("<!DOCTYPE html><title>404</title>"), None);
        assert_eq!(architecture_name(""), None);
        assert_eq!(architecture_name(r#"{"architectures": [] }"#), None);
        assert_eq!(architecture_name(r#"{"architectures": [1, 2]}"#), None);
        // Truncated: the array never closes, so there is no value to read.
        assert_eq!(architecture_name(r#"{"architectures": ["Wh"#), None);
    }
}
