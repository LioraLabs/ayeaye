//! Enough JSON to read and write the daemon's own state.
//!
//! Hand rolled rather than pulled in, because the pure core's dependency
//! allowlist is empty and `serde_json` would have to be admitted to it for the
//! sake of one small state file and one small response body. Nothing here
//! streams and nothing derives: it reads the pick store the Python daemon
//! writes — while both daemons run, that file is shared — and it renders the
//! picker's answer back.

/// A JSON value.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    /// Every JSON number, integral or not. The store holds a count and a
    /// timestamp, and reading the count back as an integer is the caller's job.
    Number(f64),
    String(String),
    Array(Vec<Value>),
    /// Members in the order they were written, so rendering one back is stable.
    /// A map would sort or hash them, and a state file that reorders itself on
    /// every write is a file nobody can diff.
    Object(Vec<(String, Value)>),
}

/// How deeply a document may nest before it is refused.
///
/// This parser recurses, and the store is a file a crash can truncate and a
/// user can edit. Sixty-four is far past anything either daemon writes — the
/// store is three levels deep — and far short of the stack.
pub const MAX_DEPTH: usize = 64;

/// Read one JSON document.
///
/// `None` for anything that is not one — which is the whole vocabulary this
/// needs, because every caller answers a malformed file with "no history"
/// rather than with an error.
pub fn parse(text: &str) -> Option<Value> {
    let bytes = text.as_bytes();
    let mut at = 0;
    let value = read_value(bytes, &mut at, 0)?;
    skip_space(bytes, &mut at);
    // Trailing garbage is a refusal, not something to stop reading before: a
    // file truncated by a crash and then appended to must not read as the
    // prefix that happens to parse.
    (at == bytes.len()).then_some(value)
}

/// The escaped *body* of a JSON string, without the quotes around it.
///
/// ASCII out, always — non-ASCII leaves as `\uXXXX`, with a surrogate pair for
/// anything outside the basic plane, which is what Python's `json.dump` writes.
/// That is not tidiness: `bin/ayeaye` opens the shared store with the locale's
/// encoding, and on a machine whose locale is not UTF-8 a raw accented byte is
/// a `UnicodeDecodeError` — which `_recents_load` catches by throwing the whole
/// history away. A file that is pure ASCII cannot be misread by any locale.
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            c if c.is_ascii() && !c.is_ascii_control() => out.push(c),
            c => {
                let mut buffer = [0u16; 2];
                for unit in c.encode_utf16(&mut buffer) {
                    out.push_str(&format!("\\u{unit:04x}"));
                }
            }
        }
    }
    out
}

impl Value {
    /// Render this value as JSON text.
    pub fn render(&self) -> String {
        let mut out = String::new();
        self.write(&mut out);
        out
    }

    /// The member under `key`, if this is an object that has one.
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Object(members) => members
                .iter()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value),
            _ => None,
        }
    }

    /// The members, if this is an object.
    pub fn as_object(&self) -> Option<&[(String, Value)]> {
        match self {
            Value::Object(members) => Some(members),
            _ => None,
        }
    }

    /// The number, if this is one.
    pub fn as_number(&self) -> Option<f64> {
        match self {
            Value::Number(number) => Some(*number),
            _ => None,
        }
    }

    fn write(&self, out: &mut String) {
        match self {
            Value::Null => out.push_str("null"),
            Value::Bool(true) => out.push_str("true"),
            Value::Bool(false) => out.push_str("false"),
            // JSON has no spelling for a non-finite number, and `NaN` is what
            // Python's encoder writes and no other reader accepts. Null is the
            // one answer every reader understands.
            Value::Number(number) if !number.is_finite() => out.push_str("null"),
            Value::Number(number) => out.push_str(&number.to_string()),
            Value::String(text) => {
                out.push('"');
                out.push_str(&escape(text));
                out.push('"');
            }
            Value::Array(items) => {
                out.push('[');
                for (index, item) in items.iter().enumerate() {
                    if index > 0 {
                        out.push(',');
                    }
                    item.write(out);
                }
                out.push(']');
            }
            Value::Object(members) => {
                out.push('{');
                for (index, (name, value)) in members.iter().enumerate() {
                    if index > 0 {
                        out.push(',');
                    }
                    out.push('"');
                    out.push_str(&escape(name));
                    out.push_str("\":");
                    value.write(out);
                }
                out.push('}');
            }
        }
    }
}

fn read_value(bytes: &[u8], at: &mut usize, depth: usize) -> Option<Value> {
    if depth >= MAX_DEPTH {
        return None;
    }
    skip_space(bytes, at);
    match *bytes.get(*at)? {
        b'{' => read_object(bytes, at, depth),
        b'[' => read_array(bytes, at, depth),
        b'"' => read_string(bytes, at).map(Value::String),
        b't' => literal(bytes, at, b"true", Value::Bool(true)),
        b'f' => literal(bytes, at, b"false", Value::Bool(false)),
        b'n' => literal(bytes, at, b"null", Value::Null),
        _ => read_number(bytes, at),
    }
}

fn literal(bytes: &[u8], at: &mut usize, word: &[u8], value: Value) -> Option<Value> {
    if bytes[*at..].starts_with(word) {
        *at += word.len();
        return Some(value);
    }
    None
}

fn read_object(bytes: &[u8], at: &mut usize, depth: usize) -> Option<Value> {
    *at += 1;
    let mut members = Vec::new();
    skip_space(bytes, at);
    if bytes.get(*at) == Some(&b'}') {
        *at += 1;
        return Some(Value::Object(members));
    }
    loop {
        skip_space(bytes, at);
        let name = read_string(bytes, at)?;
        skip_space(bytes, at);
        if bytes.get(*at) != Some(&b':') {
            return None;
        }
        *at += 1;
        members.push((name, read_value(bytes, at, depth + 1)?));
        skip_space(bytes, at);
        match bytes.get(*at) {
            Some(b',') => *at += 1,
            Some(b'}') => {
                *at += 1;
                return Some(Value::Object(members));
            }
            _ => return None,
        }
    }
}

fn read_array(bytes: &[u8], at: &mut usize, depth: usize) -> Option<Value> {
    *at += 1;
    let mut items = Vec::new();
    skip_space(bytes, at);
    if bytes.get(*at) == Some(&b']') {
        *at += 1;
        return Some(Value::Array(items));
    }
    loop {
        items.push(read_value(bytes, at, depth + 1)?);
        skip_space(bytes, at);
        match bytes.get(*at) {
            Some(b',') => *at += 1,
            Some(b']') => {
                *at += 1;
                return Some(Value::Array(items));
            }
            _ => return None,
        }
    }
}

fn read_string(bytes: &[u8], at: &mut usize) -> Option<String> {
    if bytes.get(*at) != Some(&b'"') {
        return None;
    }
    *at += 1;
    let mut out = String::new();
    loop {
        let byte = *bytes.get(*at)?;
        match byte {
            b'"' => {
                *at += 1;
                return Some(out);
            }
            b'\\' => {
                *at += 1;
                let escape = *bytes.get(*at)?;
                *at += 1;
                match escape {
                    b'"' => out.push('"'),
                    b'\\' => out.push('\\'),
                    b'/' => out.push('/'),
                    b'b' => out.push('\u{08}'),
                    b'f' => out.push('\u{0c}'),
                    b'n' => out.push('\n'),
                    b'r' => out.push('\r'),
                    b't' => out.push('\t'),
                    b'u' => out.push(read_escaped_char(bytes, at)?),
                    _ => return None,
                }
            }
            // A raw control character is not allowed inside a JSON string, and
            // accepting one would let a truncated file read as a longer one.
            byte if byte < 0x20 => return None,
            // ASCII is a character on its own, and taking it directly is what
            // keeps this linear: validating from here to the end of the
            // document once per character made a 2000-key store cost 50ms on
            // the request path.
            byte if byte < 0x80 => {
                out.push(byte as char);
                *at += 1;
            }
            _ => {
                // A multi-byte character arrives whole. Only its own bytes are
                // examined — four at the most — never the rest of the text.
                let width = utf8_width(byte);
                let end = (*at + width).min(bytes.len());
                let ch = core_str(bytes, *at, end)?.chars().next()?;
                out.push(ch);
                *at += ch.len_utf8();
            }
        }
    }
}

/// The character a `\u` escape names, the `\u` itself already consumed.
///
/// A high surrogate takes the `\uXXXX` after it as its pair. A surrogate with
/// no pair is not a character at all — it stands for a byte the filesystem had
/// and UTF-8 does not, which is what Python's `surrogateescape` puts in a path
/// — and it becomes the replacement character rather than refusing the file:
/// one unreadable path must not cost every other pick in the store.
fn read_escaped_char(bytes: &[u8], at: &mut usize) -> Option<char> {
    let first = read_hex4(bytes, at)?;
    if !(0xd800..0xdc00).contains(&first) {
        return Some(char::from_u32(first).unwrap_or('\u{fffd}'));
    }
    let mut after = *at;
    if bytes.get(after) == Some(&b'\\') && bytes.get(after + 1) == Some(&b'u') {
        after += 2;
        if let Some(low) = read_hex4(bytes, &mut after)
            && (0xdc00..0xe000).contains(&low)
        {
            *at = after;
            let combined = 0x10000 + ((first - 0xd800) << 10) + (low - 0xdc00);
            return Some(char::from_u32(combined).unwrap_or('\u{fffd}'));
        }
    }
    Some('\u{fffd}')
}

fn read_hex4(bytes: &[u8], at: &mut usize) -> Option<u32> {
    let mut value = 0u32;
    for _ in 0..4 {
        let digit = (*bytes.get(*at)? as char).to_digit(16)?;
        value = value * 16 + digit;
        *at += 1;
    }
    Some(value)
}

/// A slice of the document as text.
///
/// Bounded on both sides rather than running to the end: validating from here
/// to the end of the document once per character is how a linear parser
/// becomes a quadratic one, and this is read on the request path.
fn core_str(bytes: &[u8], from: usize, to: usize) -> Option<&str> {
    std::str::from_utf8(bytes.get(from..to)?).ok()
}

/// How many bytes the character starting with this one occupies.
///
/// A byte that starts no character at all answers 1, so the slice handed to
/// `from_utf8` is one invalid byte and the parse refuses it — rather than
/// running off the end looking for a continuation that is not there.
fn utf8_width(first: u8) -> usize {
    match first {
        0xc0..=0xdf => 2,
        0xe0..=0xef => 3,
        0xf0..=0xf7 => 4,
        _ => 1,
    }
}

/// A number, read a little more loosely than JSON spells one.
///
/// A leading zero and a trailing `.` are accepted, and an exponent too large
/// for an `f64` becomes infinity, which [`Value::write`] renders as `null`.
/// Every one of those is in the accepting direction, which is the right
/// direction for a reader of a file another program wrote — but it means a
/// document this refuses is a strict subset of what JSON refuses.
fn read_number(bytes: &[u8], at: &mut usize) -> Option<Value> {
    let start = *at;
    if bytes.get(*at) == Some(&b'-') {
        *at += 1;
    }
    let digits_from = *at;
    while matches!(bytes.get(*at), Some(byte) if byte.is_ascii_digit()) {
        *at += 1;
    }
    if *at == digits_from {
        return None;
    }
    if bytes.get(*at) == Some(&b'.') {
        *at += 1;
        while matches!(bytes.get(*at), Some(byte) if byte.is_ascii_digit()) {
            *at += 1;
        }
    }
    if matches!(bytes.get(*at), Some(b'e' | b'E')) {
        *at += 1;
        if matches!(bytes.get(*at), Some(b'+' | b'-')) {
            *at += 1;
        }
        while matches!(bytes.get(*at), Some(byte) if byte.is_ascii_digit()) {
            *at += 1;
        }
    }
    core_str(bytes, start, *at)?.parse().ok().map(Value::Number)
}

fn skip_space(bytes: &[u8], at: &mut usize) {
    while matches!(bytes.get(*at), Some(b' ' | b'\t' | b'\n' | b'\r')) {
        *at += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::{Value, escape, parse};

    fn string(text: &str) -> String {
        match parse(text) {
            Some(Value::String(read)) => read,
            other => panic!("{text} parsed as {other:?}, not a string"),
        }
    }

    // AYEAYE-50 — Python's encoder writes non-ASCII as `\uXXXX` by default, so
    // every path with an accent in it arrives escaped, and one outside the
    // basic plane arrives as a surrogate pair. A reader that mishandles either
    // turns a key into a key that can never match the directory it names.
    #[test]
    fn the_escapes_are_read_and_written() {
        assert_eq!(
            string(r#""a\"b\\c\/d\be\ff\ng\rh\ti""#),
            "a\"b\\c/d\u{08}e\u{0c}f\ng\rh\ti"
        );
        assert_eq!(string(r#""café""#), "café");
        assert_eq!(string(r#""😀""#), "😀");
        assert_eq!(string(r#""\ud83d\ude00""#), "😀");
        // A lone surrogate cannot be a Rust `char`. It stands for a byte the
        // filesystem had and UTF-8 does not, and refusing the file over one
        // would throw away every other pick in it — so it becomes the
        // replacement character and simply never matches a real directory.
        assert_eq!(string(r#""x\udcffy""#), "x\u{fffd}y");

        assert_eq!(escape("a\"b\\c\nd\te"), r#"a\"b\\c\nd\te"#);
        assert_eq!(escape("\u{08}\u{0c}\r\u{1}"), r#"\b\f\r\u0001"#);
        // ASCII out, always: a store this binary rewrote must be readable by
        // the Python daemon whatever the machine's locale is.
        assert_eq!(escape("caf\u{e9}"), r"caf\u00e9");
        assert_eq!(escape("\u{1f600}"), r"\ud83d\ude00");
        assert!(escape("naïve/日本/😀").is_ascii());
        // And what it writes, it reads back.
        assert_eq!(
            string(&format!("\"{}\"", escape("naïve/日本/😀"))),
            "naïve/日本/😀"
        );
    }

    // AYEAYE-50 — the store is read on the request path and a malformed one
    // has to mean "no history" rather than a half-read one. A parser that
    // stops at the first thing it cannot read would take a file truncated by
    // a crash and answer with the prefix that happened to parse, which is a
    // ranking signal built out of whatever survived.
    #[test]
    fn a_document_that_is_not_one_is_refused_rather_than_half_read() {
        assert_eq!(parse(r#"{"a":1} trailing"#), None);
        assert_eq!(parse(r#"{"a":1}{"b":2}"#), None);
        assert_eq!(parse(r#"{"a":1"#), None);
        assert_eq!(parse(r#""unterminated"#), None);
        assert_eq!(parse(","), None);
        assert_eq!(parse(r#"[1,]"#), None);
        assert_eq!(parse(r#"{"a":}"#), None);
        assert_eq!(parse(""), None);
        // What Python's own encoder writes for a non-finite number, and what
        // no other reader accepts. Refused here, and rendered as null.
        assert_eq!(parse("NaN"), None);
        assert_eq!(parse("Infinity"), None);
        // A raw control character inside a string: refused, or a file
        // truncated mid-line reads as a longer one that happens to close.
        assert_eq!(parse("\"a\nb\""), None);
        assert_eq!(parse("\"a\u{0}b\""), None);
        assert_eq!(Value::Number(f64::NAN).render(), "null");

        // Python's `repr` of a float is free to use an exponent, and the
        // timestamps in the store are floats it wrote.
        assert_eq!(parse("1e3"), Some(Value::Number(1000.0)));
        assert_eq!(parse("1E+3"), Some(Value::Number(1000.0)));
        assert_eq!(parse("-2.5e-2"), Some(Value::Number(-0.025)));
        // A well-formed document with room around it is still well formed.
        assert_eq!(
            parse(" {\"a\" : 1 } \n"),
            Some(super::Value::Object(vec![(
                "a".to_string(),
                Value::Number(1.0)
            )]))
        );
    }

    // AYEAYE-50 — the store is a file on disk that a user can edit and a crash
    // can truncate, and this parser recurses. Without a bound, a file that is
    // nothing but open brackets takes the daemon down with a stack overflow —
    // which is not a parse failure a caller can answer "no history" to.
    #[test]
    fn nesting_past_the_bound_is_refused_rather_than_recursed() {
        let deep = "[".repeat(super::MAX_DEPTH + 1) + &"]".repeat(super::MAX_DEPTH + 1);
        assert_eq!(parse(&deep), None);
        let brackets = "[".repeat(super::MAX_DEPTH) + &"]".repeat(super::MAX_DEPTH);
        assert!(
            parse(&brackets).is_some(),
            "the bound itself must still parse"
        );
        // Depth is nesting, not length: a flat document of any size is fine.
        let flat = format!("[{}]", vec!["1"; 5000].join(","));
        assert!(parse(&flat).is_some());
    }

    // AYEAYE-50 — the store is read and rewritten by this binary while the
    // Python daemon writes the same file, so what comes out has to be what
    // went in.
    #[test]
    fn a_nested_document_round_trips() {
        let text = r#"{"version":1,"picks":{"/home/a/src":{"n":3,"t":1700000000.5}}}"#;
        let value = parse(text).expect("a well-formed document should parse");
        assert_eq!(value.render(), text);

        // The rest of the vocabulary, which the picker's body uses even though
        // the store does not.
        let mixed = r#"{"a":[true,false,null,[],{}],"b":-2.5}"#;
        assert_eq!(
            parse(mixed).expect("arrays and literals parse").render(),
            mixed
        );
        assert_eq!(
            value,
            Value::Object(vec![
                ("version".to_string(), Value::Number(1.0)),
                (
                    "picks".to_string(),
                    Value::Object(vec![(
                        "/home/a/src".to_string(),
                        Value::Object(vec![
                            ("n".to_string(), Value::Number(3.0)),
                            ("t".to_string(), Value::Number(1700000000.5)),
                        ])
                    )])
                ),
            ])
        );
    }
}
