//! Just enough JSON to hand cliban's own output back out again.
//!
//! Not a parser. The board endpoints pass cliban's JSON through verbatim, so
//! the only questions asked of it are the two the Python daemon asks: "is this
//! text one well-formed value" — its `json.loads` per line, whose only
//! observable effect is dropping the lines that fail — and "what is this
//! object's `key`". Parsing to a tree and re-rendering would answer both and
//! add a risk nobody asked for: a number that comes back out as different text
//! than cliban put in.
//!
//! So this is a scanner over borrowed text, a reader for one string member, and
//! a writer for the few strings this crate does author.

/// How deep a value may nest before it is refused.
///
/// A recursive scanner with no bound is a stack overflow waiting for the first
/// oddly-shaped input, and an overflow is not a `false` — it takes the process
/// with it. cliban's rows nest two or three deep; anything near this is not a
/// board row.
const MAX_DEPTH: usize = 128;

/// Whether `text` is exactly one well-formed JSON value.
///
/// Leading and trailing whitespace is allowed and nothing else is: this is the
/// verdict `json.loads` gives, which is what decides whether a line of cliban's
/// NDJSON becomes a row or is dropped.
pub fn is_value(text: &str) -> bool {
    let bytes = text.as_bytes();
    let Some(end) = scan_value(bytes, skip_space(bytes, 0), 0) else {
        return false;
    };
    skip_space(bytes, end) == bytes.len()
}

fn skip_space(bytes: &[u8], mut at: usize) -> usize {
    while matches!(bytes.get(at), Some(b' ' | b'\t' | b'\n' | b'\r')) {
        at += 1;
    }
    at
}

/// Scan one value, returning the index just past it.
fn scan_value(bytes: &[u8], at: usize, depth: usize) -> Option<usize> {
    if depth > MAX_DEPTH {
        return None;
    }
    match *bytes.get(at)? {
        b'{' => scan_object(bytes, at, depth, &mut |_, _, _| {}),
        b'[' => scan_array(bytes, at, depth),
        b'"' => read_string(bytes, at, None),
        b't' => word(bytes, at, b"true"),
        b'f' => word(bytes, at, b"false"),
        b'n' => word(bytes, at, b"null"),
        b'-' | b'0'..=b'9' => scan_number(bytes, at),
        _ => None,
    }
}

fn word(bytes: &[u8], at: usize, expected: &[u8]) -> Option<usize> {
    let end = at + expected.len();
    (bytes.get(at..end)? == expected).then_some(end)
}

/// Scan an object, handing every member to `visit` as `(name, value_start, value_end)`.
///
/// The visitor is how [`string_member`] reads a member without a second scanner
/// that could disagree with this one about what an object is.
fn scan_object(
    bytes: &[u8],
    at: usize,
    depth: usize,
    visit: &mut impl FnMut(String, usize, usize),
) -> Option<usize> {
    let mut at = skip_space(bytes, at + 1);
    if bytes.get(at) == Some(&b'}') {
        return Some(at + 1);
    }
    loop {
        let mut name = String::new();
        at = read_string(bytes, at, Some(&mut name))?;
        at = skip_space(bytes, at);
        if bytes.get(at) != Some(&b':') {
            return None;
        }
        let start = skip_space(bytes, at + 1);
        let end = scan_value(bytes, start, depth + 1)?;
        visit(name, start, end);
        at = skip_space(bytes, end);
        match bytes.get(at) {
            Some(b',') => at = skip_space(bytes, at + 1),
            Some(b'}') => return Some(at + 1),
            _ => return None,
        }
    }
}

fn scan_array(bytes: &[u8], at: usize, depth: usize) -> Option<usize> {
    let mut at = skip_space(bytes, at + 1);
    if bytes.get(at) == Some(&b']') {
        return Some(at + 1);
    }
    loop {
        at = skip_space(bytes, scan_value(bytes, at, depth + 1)?);
        match bytes.get(at) {
            Some(b',') => at = skip_space(bytes, at + 1),
            Some(b']') => return Some(at + 1),
            _ => return None,
        }
    }
}

/// `-? (0 | [1-9][0-9]*) (. [0-9]+)? ([eE] [+-]? [0-9]+)?`
///
/// The grammar, not Python's superset: `json.loads` also accepts `NaN`,
/// `Infinity` and `-Infinity`, which no JSON encoder emits and cliban has no
/// way to produce. Refusing them costs a row that could not have existed.
fn scan_number(bytes: &[u8], at: usize) -> Option<usize> {
    let mut at = at;
    if bytes.get(at) == Some(&b'-') {
        at += 1;
    }
    match *bytes.get(at)? {
        // A leading zero admits no more digits: `01` is two tokens, not a
        // number, and accepting it would let `{"a":01}` through as a row.
        b'0' => at += 1,
        b'1'..=b'9' => at = digits(bytes, at),
        _ => return None,
    }
    if bytes.get(at) == Some(&b'.') {
        at += 1;
        let after = digits(bytes, at);
        if after == at {
            return None;
        }
        at = after;
    }
    if matches!(bytes.get(at), Some(b'e' | b'E')) {
        at += 1;
        if matches!(bytes.get(at), Some(b'+' | b'-')) {
            at += 1;
        }
        let after = digits(bytes, at);
        if after == at {
            return None;
        }
        at = after;
    }
    Some(at)
}

fn digits(bytes: &[u8], mut at: usize) -> usize {
    while matches!(bytes.get(at), Some(b'0'..=b'9')) {
        at += 1;
    }
    at
}

/// Read a string, returning the index just past its closing quote.
///
/// `out` decides whether the characters are kept: validating a whole board
/// would otherwise allocate a `String` for every string in it, and only the
/// member names and the one value [`string_member`] is asked for are ever
/// wanted. Scanning bytes is safe on UTF-8 text because every byte this looks
/// at is ASCII, and a continuation byte can never be one of them.
fn read_string(bytes: &[u8], at: usize, mut out: Option<&mut String>) -> Option<usize> {
    if bytes.get(at) != Some(&b'"') {
        return None;
    }
    let mut at = at + 1;
    loop {
        match *bytes.get(at)? {
            b'"' => return Some(at + 1),
            // Strict, as `json.loads` is by default: a raw newline or tab
            // inside a string is a malformed line, not a row.
            0x00..=0x1f => return None,
            b'\\' => {
                let (next, decoded) = read_escape(bytes, at)?;
                if let Some(out) = out.as_deref_mut() {
                    out.push(decoded);
                }
                at = next;
            }
            _ => {
                let start = at;
                at += 1;
                // Walk to the end of one UTF-8 character so the slice pushed
                // below is always a character boundary.
                while matches!(bytes.get(at), Some(0x80..=0xbf)) {
                    at += 1;
                }
                if let Some(out) = out.as_deref_mut() {
                    out.push_str(std::str::from_utf8(&bytes[start..at]).ok()?);
                }
            }
        }
    }
}

/// One `\…` escape, returning the index just past it and the character it means.
fn read_escape(bytes: &[u8], at: usize) -> Option<(usize, char)> {
    let simple = |c| Some((at + 2, c));
    match *bytes.get(at + 1)? {
        b'"' => simple('"'),
        b'\\' => simple('\\'),
        b'/' => simple('/'),
        b'b' => simple('\u{8}'),
        b'f' => simple('\u{c}'),
        b'n' => simple('\n'),
        b'r' => simple('\r'),
        b't' => simple('\t'),
        b'u' => {
            let first = hex4(bytes, at + 2)?;
            // A high surrogate takes the low one that follows it. A lone one
            // is kept as the replacement character rather than refused:
            // `json.loads` accepts it, Rust's `char` cannot hold it, and the
            // difference between the two is not worth dropping a row over.
            if (0xd800..0xdc00).contains(&first)
                && bytes.get(at + 6) == Some(&b'\\')
                && bytes.get(at + 7) == Some(&b'u')
                && let Some(second) = hex4(bytes, at + 8)
                && (0xdc00..0xe000).contains(&second)
            {
                let combined = 0x10000 + ((first - 0xd800) << 10) + (second - 0xdc00);
                return Some((at + 12, char::from_u32(combined)?));
            }
            Some((at + 6, char::from_u32(first).unwrap_or('\u{fffd}')))
        }
        _ => None,
    }
}

fn hex4(bytes: &[u8], at: usize) -> Option<u32> {
    let digits = bytes.get(at..at + 4)?;
    let mut value = 0;
    for digit in digits {
        value = value * 16 + (*digit as char).to_digit(16)?;
    }
    Some(value)
}

/// The decoded string value of a top-level member, if there is one.
///
/// `None` when the text is not an object, does not parse, has no such member,
/// or has one whose value is not a string. The last is a deliberate narrowing:
/// the only member ever asked for is a project `key`, which is compared and
/// echoed as text, and a number there would be a row cliban could not emit.
///
/// A repeated member takes the last one, which is what `json.loads` does.
pub fn string_member(text: &str, name: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let at = skip_space(bytes, 0);
    if bytes.get(at) != Some(&b'{') {
        return None;
    }
    let mut found = None;
    let end = scan_object(bytes, at, 0, &mut |member, start, stop| {
        if member == name {
            found = Some((start, stop));
        }
    })?;
    if skip_space(bytes, end) != bytes.len() {
        return None;
    }
    let (start, _) = found?;
    let mut value = String::new();
    read_string(bytes, start, Some(&mut value))?;
    Some(value)
}

/// Render `text` as a JSON string literal, quotes included.
///
/// Everything below a space is escaped, not just the ones with short names: an
/// error message from cliban goes through here on its way into a response body,
/// and a raw control character there is a body the browser's `.json()` refuses
/// — which would turn a stated reason into a broken panel.
pub fn quote(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for character in text.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            control if control < ' ' => {
                out.push_str(&format!("\\u{:04x}", control as u32));
            }
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::{is_value, quote, string_member};

    /// One row of `cliban project ls --json`, as it comes off the pipe.
    const ROW: &str = r#"{"key":"AYEAYE","name":"AyeAye","updated_at":"2026-08-10T05:53:47Z"}"#;

    // AYEAYE-53 — the row filter is `json.loads` per line, and the only thing
    // that makes it a filter is what it refuses. A scanner that accepted a
    // truncated line would put a half-object into the board payload and take
    // the whole page down with it.
    #[test]
    fn one_well_formed_value_is_accepted_and_nothing_else_is() {
        for text in [
            ROW,
            "{}",
            "[]",
            r#""just a string""#,
            "3",
            "-1.5e10",
            "0",
            "true",
            "false",
            "null",
            "  \n\t {\"a\": [1, {\"b\": null}]}  \n",
            r#"{"description":"a \"quoted\" word\nand a line break"}"#,
            r#"{"emoji":"😀 and é"}"#,
        ] {
            assert!(is_value(text), "should parse: {text}");
        }

        for text in [
            "",
            "   ",
            r#"{"key":"AYEAYE""#,
            "{} {}",
            "nope",
            "tru",
            r#"{"a":}"#,
            r#"{"a" 1}"#,
            r#"{a:1}"#,
            "[1,]",
            "[1 2]",
            "01",
            "1.",
            "1e",
            "-",
            r#""unterminated"#,
            "\"a raw \n newline\"",
            r#""a bad \x escape""#,
            r#"{"a":1,}"#,
        ] {
            assert!(!is_value(text), "should not parse: {text}");
        }
    }

    // AYEAYE-53 — an unbounded recursive scanner does not return `false` on
    // deep nesting, it takes the process with it. The cap is what makes the
    // refusal a verdict rather than a crash.
    #[test]
    fn nesting_past_the_cap_is_refused_rather_than_recursed() {
        let shallow = format!("{}{}", "[".repeat(64), "]".repeat(64));
        assert!(is_value(&shallow));
        let deep = format!("{}{}", "[".repeat(5_000), "]".repeat(5_000));
        assert!(!is_value(&deep));
    }

    // AYEAYE-53 — the project keys the main page linkifies come from here, so
    // a member read out of a nested object, or a number read as text, would
    // put something into a regular expression that no project is named.
    #[test]
    fn a_top_level_string_member_is_read_and_nothing_else_is() {
        assert_eq!(string_member(ROW, "key").as_deref(), Some("AYEAYE"));
        assert_eq!(string_member(ROW, "name").as_deref(), Some("AyeAye"));
        assert_eq!(string_member(ROW, "nope"), None);
        // Not a string, not an object, and not parseable at all.
        assert_eq!(string_member(r#"{"key":7}"#, "key"), None);
        assert_eq!(string_member(r#"{"key":null}"#, "key"), None);
        assert_eq!(string_member("[1]", "key"), None);
        assert_eq!(string_member(r#"{"key":"A""#, "key"), None);
        // A member of a nested object is not this object's member.
        assert_eq!(string_member(r#"{"inner":{"key":"NO"}}"#, "key"), None);
        // Escapes are decoded, because the value is compared and echoed.
        assert_eq!(
            string_member(r#"{"key":"a\/bA\n"}"#, "key").as_deref(),
            Some("a/bA\n")
        );
        // A repeated member takes the last, as `json.loads` does.
        assert_eq!(
            string_member(r#"{"key":"FIRST","key":"LAST"}"#, "key").as_deref(),
            Some("LAST")
        );
    }

    // AYEAYE-53 — cliban's stderr goes into a response body through here. A
    // quote or a newline that escaped its own string would make the degraded
    // answer unreadable, which is the one thing the degradation must not do.
    #[test]
    fn a_quoted_string_is_a_json_string() {
        assert_eq!(quote("plain"), r#""plain""#);
        assert_eq!(
            quote("he said \"no\" \\ then\nstopped\tthere"),
            r#""he said \"no\" \\ then\nstopped\tthere""#
        );
        assert_eq!(quote("bell\u{7}"), r#""bell\u0007""#);
        assert_eq!(quote("é"), "\"é\"");
        // And what comes out is something this crate's own scanner accepts,
        // which is the property the response body actually needs.
        assert!(is_value(&quote("cliban: no such file\u{0}\"")));
    }
}
