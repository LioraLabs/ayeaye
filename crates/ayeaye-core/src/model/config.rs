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
//!
//! **Skipping and reading are different jobs, and they are different functions.**
//! [`end_of_string`] finds where a string ends without decoding it; [`decode`]
//! turns one into text. Every string in the document is skipped and at most two
//! are decoded, so an escape this module cannot make sense of — a surrogate
//! pair, a `\x` — costs nothing unless it is in the name actually being read.
//! Sharing one function between the two jobs is how a smiley in a model's
//! `_name_or_path` came to make its architecture unreadable.

/// How deeply this will walk before giving up.
///
/// A bound, not a guess at a limit somebody might hit: `config.json` arrives
/// from a repository any stranger can publish, and [`skip_value`] recurses
/// once per alternation between `{` and `[`. Without this, a few hundred
/// kilobytes of `[{[{` ends the **process** — a stack overflow aborts and
/// cannot be caught — and it would do it inside the one function whose whole
/// purpose is to judge a stranger's file safely. Real configs nest three or
/// four deep.
const MAX_DEPTH: usize = 64;

/// What this model calls its architecture, if it says.
///
/// The `architectures` array first, since that is what HuggingFace's exporters
/// write and what names the class `candle-transformers` would have to
/// implement; `model_type` second, because some configs carry only that and
/// refusing one of those for the wrong reason is a worse answer than the right
/// one.
pub(crate) fn architecture_name(config_json: &str) -> Option<String> {
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
    // A byte-order mark is not whitespace and not `{`, and a file that opens
    // with one is still a config.
    let opens = json.strip_prefix('\u{feff}').map_or(0, |_| 3);
    let mut at = skip_space(bytes, opens);
    if bytes.get(at) != Some(&b'{') {
        return None;
    }
    at = skip_space(bytes, at + 1);

    loop {
        if bytes.get(at) == Some(&b'}') {
            return None;
        }
        let after_name = end_of_string(bytes, at)?;
        // Decoded rather than compared raw, so a key spelled with an escape is
        // still the key it names. A key this cannot decode is simply not one of
        // the two being looked for, which is a skip and not a failure.
        let matches = decode(&json[at..after_name]).is_some_and(|name| name == key);
        at = skip_space(bytes, after_name);
        if bytes.get(at) != Some(&b':') {
            return None;
        }
        let start = skip_space(bytes, at + 1);
        let end = skip_value(bytes, start, 0)?;
        if matches {
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
    let end = end_of_string(bytes, at)?;
    decode(&value[at..end])
}

/// A value that is a single string literal, decoded.
fn unquote(value: &str) -> Option<String> {
    let end = end_of_string(value.as_bytes(), 0)?;
    decode(&value[..end])
}

/// The index just past the closing quote of the string literal at `at`.
///
/// Structure only: a backslash always covers the byte after it, which is all
/// that is needed to find the end and is true of every escape JSON has. It
/// decodes nothing, so it cannot fail on an escape it does not recognise —
/// which is the point, because every string in the document goes through here
/// and only the one being read goes through [`decode`].
fn end_of_string(bytes: &[u8], at: usize) -> Option<usize> {
    if bytes.get(at) != Some(&b'"') {
        return None;
    }
    let mut i = at + 1;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => return Some(i + 1),
            b'\\' => i += 2,
            _ => i += 1,
        }
    }
    None
}

/// Turn a whole string literal, quotes included, into the text it names.
fn decode(literal: &str) -> Option<String> {
    let bytes = literal.as_bytes();
    if bytes.first() != Some(&b'"') || bytes.len() < 2 {
        return None;
    }
    let mut out = String::new();
    let mut i = 1;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => return Some(out),
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
            let code = hex4(bytes, at + 1)?;
            // A `\u` escape cannot name a character outside the basic plane on
            // its own; JSON spells those as a surrogate pair, which is what
            // `json.dumps` produces for every emoji it writes. Joining the two
            // halves is the difference between reading such a name and
            // refusing it.
            if (0xd800..0xdc00).contains(&code) {
                let low = (bytes.get(at + 5) == Some(&b'\\') && bytes.get(at + 6) == Some(&b'u'))
                    .then(|| hex4(bytes, at + 7))
                    .flatten()
                    .filter(|low| (0xdc00..0xe000).contains(low))?;
                let joined = 0x10000 + ((code - 0xd800) << 10) + (low - 0xdc00);
                return Some((char::from_u32(joined)?, at + 11));
            }
            Some((char::from_u32(code)?, at + 5))
        }
        _ => None,
    }
}

/// Four hex digits as a number.
///
/// `from_str_radix` accepts a leading sign, which would read `\u+041` as `A`.
/// That is not JSON, and accepting it means this disagrees with every other
/// reader about what the document says.
fn hex4(bytes: &[u8], at: usize) -> Option<u32> {
    let digits = bytes.get(at..at + 4)?;
    digits
        .iter()
        .all(|byte| byte.is_ascii_hexdigit())
        .then(|| u32::from_str_radix(std::str::from_utf8(digits).ok()?, 16).ok())
        .flatten()
}

/// The index just past the value starting at `at`, whatever kind it is.
fn skip_value(bytes: &[u8], at: usize, depth: usize) -> Option<usize> {
    if depth > MAX_DEPTH {
        return None;
    }
    match bytes.get(at)? {
        b'"' => end_of_string(bytes, at),
        open @ (b'{' | b'[') => {
            let close = if *open == b'{' { b'}' } else { b']' };
            let mut nesting = 0usize;
            let mut i = at;
            while i < bytes.len() {
                match bytes[i] {
                    b'"' => i = end_of_string(bytes, i)?,
                    byte if byte == *open => {
                        nesting += 1;
                        i += 1;
                    }
                    byte if byte == close => {
                        nesting -= 1;
                        i += 1;
                        if nesting == 0 {
                            return Some(i);
                        }
                    }
                    // A nested container of the other kind, whose own braces
                    // must not be counted against this one's nesting.
                    b'{' | b'[' => i = skip_value(bytes, i, depth + 1)?,
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

    /// A real `config.json`, captured from `openai/whisper-tiny.en`.
    ///
    /// Captured rather than written out here because the shape is the test.
    /// `transformers` writes these with `sort_keys=True`, so `_name_or_path`
    /// (a string), `activation_dropout` (a number) and `activation_function`
    /// come *before* `architectures` — which means reading this file is the
    /// only test that exercises skipping a scalar to reach the key. A
    /// hand-written fixture with `architectures` first matches on the first
    /// key and skips nothing, and proves correspondingly less.
    const REAL: &str = include_str!("../../tests/fixtures/model/whisper-tiny.en-config.json");

    // AYEAYE-56 — the field is read out of a real config, with three keys and
    // two kinds of scalar to be walked past before it.
    #[test]
    fn the_architecture_is_read_from_a_real_captured_config() {
        assert!(
            REAL.starts_with("{\n  \"_name_or_path\""),
            "the fixture has to be a config whose first key is not the one being \
             looked for, or it stops testing the walk"
        );
        assert_eq!(
            architecture_name(REAL).as_deref(),
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
        assert_eq!(
            architecture_name(r#"{"architectures": ["café"]}"#).as_deref(),
            Some("café")
        );
        // A surrogate pair is how `json.dumps` writes anything outside the
        // basic plane, and it is two escapes naming one character.
        assert_eq!(
            architecture_name("{\"architectures\": [\"\\ud83d\\ude00 W\"]}").as_deref(),
            Some("😀 W")
        );
        // A high half with nothing after it is not a character, and inventing
        // a replacement for it would only fail to match the allowlist later,
        // for a second reason.
        assert_eq!(architecture_name(r#"{"architectures": ["\ud83d"]}"#), None);
        // `\u+041` is not JSON, and reading it as "A" would mean disagreeing
        // with every other reader about what the file says.
        assert_eq!(architecture_name(r#"{"architectures": ["\u+041"]}"#), None);
    }

    // AYEAYE-56 — an escape this cannot decode, in a string it is only
    // *skipping*, must not cost the model its architecture. `transformers`
    // writes configs with `ensure_ascii=True` and `sort_keys=True`, so an
    // emoji anywhere in the file arrives as a surrogate pair, in a key sorted
    // before `architectures`. Sharing one function between skipping and
    // decoding is what made this answer "names no architecture" about a config
    // that plainly does.
    #[test]
    fn an_escape_in_a_string_that_is_only_skipped_costs_nothing() {
        let smiley = r#"{
          "_name_or_path": "\\ud83d\\ude00 tiny",
          "_odd_escape": "\x41",
          "architectures": ["WhisperForConditionalGeneration"]
        }"#;
        assert_eq!(
            architecture_name(smiley).as_deref(),
            Some("WhisperForConditionalGeneration")
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

    // AYEAYE-56 — a byte-order mark is neither whitespace nor `{`, and a file
    // that opens with one is still a config.
    #[test]
    fn a_byte_order_mark_does_not_hide_the_config() {
        let marked = "\u{feff}".to_string() + r#"{"model_type": "whisper"}"#;
        assert_eq!(architecture_name(&marked).as_deref(), Some("whisper"));
    }

    // AYEAYE-56 — this file arrives from a repository any stranger can
    // publish, so nesting is bounded. Without the bound a few hundred
    // kilobytes of `[{[{` ends the *process*: a stack overflow aborts and
    // cannot be caught, inside the one function whose job is to judge a
    // stranger's file safely.
    #[test]
    fn nesting_deeper_than_the_bound_is_refused_rather_than_ending_the_process() {
        let deep = format!(
            "{{\"a\": {}{}, \"architectures\": [\"WhisperForConditionalGeneration\"]}}",
            "[{".repeat(5_000),
            "}]".repeat(5_000)
        );
        assert_eq!(architecture_name(&deep), None);

        // And the bound is well clear of what a real config nests, so nothing
        // legitimate is refused by it.
        let realistic = r#"{"a": {"b": [{"c": [1, 2]}]}, "model_type": "whisper"}"#;
        assert_eq!(architecture_name(realistic).as_deref(), Some("whisper"));
    }
}
