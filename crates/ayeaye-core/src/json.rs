//! Just enough JSON to answer with, to read cliban's output, and to be asked in.
//!
//! Writing this rather than depending on a serialiser is not thrift. This crate
//! declares no dependencies at all — the constitution's allowlist is empty and a
//! name goes on it only with an argument that its own transitive dependencies
//! reach nothing — and a pure crate whose whole job is "text and structs in,
//! text and structs out" can spell a string literal itself.
//!
//! **This file used to say there was no reader here and should not be, because
//! nothing the daemon was handed arrived as JSON.** Four tickets falsified that
//! in one wave, in four different ways, and they did not agree with each other.
//! What is here is the result, written down rather than tidied away — see
//! AYEAYE-64, which owns deciding how much of it should survive.
//!
//! [`is_value`] and [`string_member`] scan **borrowed text and build nothing**.
//! The board endpoints pass cliban's JSON through verbatim, so the only
//! questions asked of it are the two the Python daemon asks: "is this text one
//! well-formed value" — its `json.loads` per line, whose only observable effect
//! is dropping the lines that fail — and "what is this object's `key`". Parsing
//! those to a tree and re-rendering would answer both and add a risk nobody
//! asked for: a number that comes back out as different text than cliban put in.
//!
//! [`parse`] does build a tree, because `/api/answer` and `/api/send` are handed
//! a body a phone wrote and the fields have to be reached. It is deliberately
//! the smallest reader that can be trusted with a body from the network: it
//! allocates nothing it was not shown, it refuses rather than guesses, and it
//! will not recurse further than [`MAX_DEPTH`].
//!
//! **The spawn and kill endpoints disagree with that last decision**, and read
//! their bodies with `serde_json` in the shell instead, on the reasoning that a
//! hostile document is the one job worth a real parser and this crate is the one
//! place that cannot have one. Both positions are defensible and both are
//! currently true, which is the part that should not last.
//!
//! [`crate::model::config`] walks a model's `config.json` for one field before
//! any weights are downloaded beside it — narrow on purpose.
//!
//! The writer, [`string`], is shared by all of them.

/// How deep a value may nest before the scanner refuses it.
///
/// Distinct from [`MAX_DEPTH`], which bounds the tree parser below: this one
/// bounds a scan that allocates nothing, so it can afford to be looser.
///
/// Levels, not the index of the deepest: 128 nest, 129 is refused.
///
/// A recursive scanner with no bound is a stack overflow waiting for the first
/// oddly-shaped input, and an overflow is not a `false` — it takes the process
/// with it. cliban's rows nest two or three deep; anything near this is not a
/// board row.
const SCAN_DEPTH: usize = 128;

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
    if depth >= SCAN_DEPTH {
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

/// Scan an object, handing every member to `visit` as the indices
/// `(name_start, name_end, value_start)`.
///
/// The visitor is how [`string_member`] reads a member without a second scanner
/// that could disagree with this one about what an object is. Indices rather
/// than a decoded name because validating a board would otherwise allocate a
/// `String` for every member of every row, and the caller wants one of them.
fn scan_object(
    bytes: &[u8],
    at: usize,
    depth: usize,
    visit: &mut impl FnMut(usize, usize, usize),
) -> Option<usize> {
    let mut at = skip_space(bytes, at + 1);
    if bytes.get(at) == Some(&b'}') {
        return Some(at + 1);
    }
    loop {
        let name_start = at;
        at = read_string(bytes, at, None)?;
        let name_end = at;
        at = skip_space(bytes, at);
        if bytes.get(at) != Some(&b':') {
            return None;
        }
        let start = skip_space(bytes, at + 1);
        let end = scan_value(bytes, start, depth + 1)?;
        visit(name_start, name_end, start);
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
/// would otherwise allocate a `String` for every string in it, and the only one
/// ever wanted is the single value [`string_member`] was asked for. Scanning
/// Scanning bytes is safe on UTF-8 text because every byte this *decides* on is
/// ASCII, and a continuation byte can never be one of those; the walk below
/// steps over the continuations so that what is kept is whole characters.
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
    let end = scan_object(bytes, at, 0, &mut |name_start, name_end, value_start| {
        if named(bytes, name_start, name_end, name) {
            found = Some(value_start);
        }
    })?;
    if skip_space(bytes, end) != bytes.len() {
        return None;
    }
    let mut value = String::new();
    read_string(bytes, found?, Some(&mut value))?;
    Some(value)
}

/// The raw text of a top-level member's value, exactly as it was written.
///
/// Borrowed rather than decoded, because the two things this answers are not
/// strings: an agent's session file records when it started as a *number*, and
/// a codex rollout opens with a `payload` **object** whose members are the
/// answer. Handing the object's own text back means [`string_member`] reads it
/// with no second reader that could disagree with this one about what an object
/// is.
///
/// The same narrowing as [`string_member`] otherwise: `None` when the text is
/// not one well-formed object, or has no such member. A repeated member takes
/// the last, which is what `json.loads` does.
pub fn member<'a>(text: &'a str, name: &str) -> Option<&'a str> {
    let bytes = text.as_bytes();
    let at = skip_space(bytes, 0);
    if bytes.get(at) != Some(&b'{') {
        return None;
    }
    let mut found = None;
    let end = scan_object(bytes, at, 0, &mut |name_start, name_end, value_start| {
        if named(bytes, name_start, name_end, name) {
            found = Some(value_start);
        }
    })?;
    if skip_space(bytes, end) != bytes.len() {
        return None;
    }
    let start = found?;
    // The object scanned, so every value in it has an end. Indexed rather than
    // sliced by that reasoning alone would be a panic waiting for the day the
    // reasoning is wrong.
    let stop = scan_value(bytes, start, 0)?;
    text.get(start..stop)
}

/// The numeric value of a top-level member, if there is one.
///
/// `None` when the member is absent or is not a number — a string there is not
/// a number that happens to be quoted, and the one caller compares it against a
/// clock.
///
/// Read through Rust's own float parser, which accepts every JSON number and
/// several things that are not one; [`member`] has already established that
/// this text *is* a JSON number, so the difference cannot arise.
pub fn number_member(text: &str, name: &str) -> Option<f64> {
    member(text, name)?.parse().ok()
}

/// Whether the member name at `start..end` — quotes included, escapes intact —
/// is `name`.
///
/// The raw bytes answer it for every name cliban has ever emitted; only a name
/// carrying a backslash costs a decode, and `{"key":…}` is still `key`.
fn named(bytes: &[u8], start: usize, end: usize, name: &str) -> bool {
    // Indexed through `get` rather than sliced: the caller's guarantee that
    // this span is a whole `"…"` lives in a doc comment, and a doc comment is
    // not a reason for the one place in this module that could panic.
    let Some(raw) = end
        .checked_sub(1)
        .and_then(|last| bytes.get(start + 1..last))
    else {
        return false;
    };
    if !raw.contains(&b'\\') {
        return raw == name.as_bytes();
    }
    let mut decoded = String::new();
    read_string(bytes, start, Some(&mut decoded)).is_some() && decoded == name
}

/// Render `text` as a JSON string literal, quotes included.
///
/// Every control character is escaped, not just the ones with short names: an
/// error message from cliban goes through here on its way into a response body,
/// and a raw control character there is a body the browser's `.json()` refuses
/// — which would turn a stated reason into a broken panel.
pub fn string(text: &str) -> String {
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
            // Not only the ASCII ones: `char::is_control` also covers the C1
            // block, which a mis-decoded byte can leave behind and which no
            // parser has to accept raw either.
            control if control.is_control() => {
                out.push_str(&format!("\\u{:04x}", control as u32));
            }
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

/// A JSON value, as far as this daemon needs one.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// `null`.
    Null,
    /// `true` or `false`.
    Bool(bool),
    /// Any number, as the double every JSON reader agrees on.
    Number(f64),
    /// A string, with its escapes already resolved.
    Text(String),
    /// An array.
    List(Vec<Value>),
    /// An object, in the order its members were written. Order is kept rather
    /// than sorted or de-duplicated: a body carrying `pane` twice is a body
    /// somebody built to be read two ways, and [`Value::get`] answers with the
    /// first — which is what `parse_qs(...)[0]` does everywhere else here.
    Map(Vec<(String, Value)>),
}

/// How deeply a body may nest before it is refused.
///
/// A recursive-descent reader recurses once per bracket, so a body of nothing
/// but brackets is a stack overflow — and a stack overflow is an abort, not an
/// error, which would take the whole daemon down from an unauthenticated
/// request-shaped distance. Nothing this reader is handed is more than two deep,
/// so the limit costs nothing real.
pub const MAX_DEPTH: usize = 32;

/// Why some text is not JSON.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BadJson {
    /// Not what the grammar needs, at this byte offset.
    Malformed(usize),
    /// Nested deeper than [`MAX_DEPTH`], at this byte offset.
    TooDeep(usize),
    /// A complete value, and then more text, at this byte offset.
    Trailing(usize),
    /// It ended in the middle of something.
    Ended,
}

impl Value {
    /// The first member under this name, if this is an object that has one.
    pub fn get(&self, name: &str) -> Option<&Value> {
        match self {
            Value::Map(members) => members
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value),
            _ => None,
        }
    }

    /// The string this is, or `None` for every other kind of value.
    ///
    /// Deliberately not "the string this could be rendered as": a client that
    /// sends `{"pane": 12}` has sent something that is not a pane id, and
    /// turning it into `"12"` here would invent an id nobody wrote.
    pub fn text(&self) -> Option<&str> {
        match self {
            Value::Text(text) => Some(text),
            _ => None,
        }
    }

    /// Whether this value means yes, by python's rule.
    ///
    /// The daemon this replaces writes `if req.get("enter")`, so `0`, `""`, `[]`
    /// and a missing member are all "no" there. Matching that exactly is worth
    /// more than a stricter rule nobody can predict from the other daemon.
    pub fn truthy(&self) -> bool {
        match self {
            Value::Null => false,
            Value::Bool(yes) => *yes,
            Value::Number(number) => *number != 0.0,
            Value::Text(text) => !text.is_empty(),
            Value::List(items) => !items.is_empty(),
            Value::Map(members) => !members.is_empty(),
        }
    }
}

/// Read one JSON value out of some text, or say why it is not one.
pub fn parse(text: &str) -> Result<Value, BadJson> {
    let mut reader = Reader {
        bytes: text.as_bytes(),
        at: 0,
    };
    let value = reader.value(1)?;
    reader.space();
    if reader.at < reader.bytes.len() {
        return Err(BadJson::Trailing(reader.at));
    }
    Ok(value)
}

/// The bytes, and how far through them we are.
struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.at).copied()
    }

    /// JSON's whitespace, which is four characters and not `char::is_whitespace`.
    fn space(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.at += 1;
        }
    }

    /// Step over one byte we have already looked at.
    fn take(&mut self, byte: u8) -> Result<(), BadJson> {
        if self.peek() == Some(byte) {
            self.at += 1;
            return Ok(());
        }
        Err(self.here())
    }

    fn here(&self) -> BadJson {
        if self.at >= self.bytes.len() {
            BadJson::Ended
        } else {
            BadJson::Malformed(self.at)
        }
    }

    fn value(&mut self, depth: usize) -> Result<Value, BadJson> {
        if depth > MAX_DEPTH {
            return Err(BadJson::TooDeep(self.at));
        }
        self.space();
        match self.peek().ok_or(BadJson::Ended)? {
            b'{' => self.map(depth),
            b'[' => self.list(depth),
            b'"' => self.text().map(Value::Text),
            b't' => self.word("true").map(|()| Value::Bool(true)),
            b'f' => self.word("false").map(|()| Value::Bool(false)),
            b'n' => self.word("null").map(|()| Value::Null),
            _ => self.number(),
        }
    }

    fn word(&mut self, word: &str) -> Result<(), BadJson> {
        if self.bytes[self.at..].starts_with(word.as_bytes()) {
            self.at += word.len();
            return Ok(());
        }
        Err(self.here())
    }

    fn number(&mut self) -> Result<Value, BadJson> {
        let from = self.at;
        while matches!(
            self.peek(),
            Some(b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9')
        ) {
            self.at += 1;
        }
        // Delegated to the standard library rather than spelled out. It is a
        // wider grammar than JSON's: `1.`, `.5`, `00`, `+1` and `-0.` are all
        // taken here and all refused by a strict reader. That is a deliberate
        // trade, and it is safe only because nothing in this daemon routes on
        // a number — every decision is made on a string or a boolean. A
        // hand-rolled float grammar is a much better place for a bug than a
        // laxity nobody reads.
        let digits = core::str::from_utf8(&self.bytes[from..self.at]).map_err(|_| self.here())?;
        match digits.parse::<f64>() {
            Ok(number) if number.is_finite() => Ok(Value::Number(number)),
            _ => Err(BadJson::Malformed(from)),
        }
    }

    fn list(&mut self, depth: usize) -> Result<Value, BadJson> {
        self.take(b'[')?;
        let mut items = Vec::new();
        self.space();
        if self.peek() == Some(b']') {
            self.at += 1;
            return Ok(Value::List(items));
        }
        loop {
            items.push(self.value(depth + 1)?);
            self.space();
            match self.peek().ok_or(BadJson::Ended)? {
                b',' => self.at += 1,
                b']' => {
                    self.at += 1;
                    return Ok(Value::List(items));
                }
                _ => return Err(self.here()),
            }
        }
    }

    fn map(&mut self, depth: usize) -> Result<Value, BadJson> {
        self.take(b'{')?;
        let mut members = Vec::new();
        self.space();
        if self.peek() == Some(b'}') {
            self.at += 1;
            return Ok(Value::Map(members));
        }
        loop {
            self.space();
            let name = self.text()?;
            self.space();
            self.take(b':')?;
            members.push((name, self.value(depth + 1)?));
            self.space();
            match self.peek().ok_or(BadJson::Ended)? {
                b',' => self.at += 1,
                b'}' => {
                    self.at += 1;
                    return Ok(Value::Map(members));
                }
                _ => return Err(self.here()),
            }
        }
    }

    /// One string, with its escapes resolved.
    ///
    /// Cut only at `"` and `\`, both ASCII, so a slice never lands inside a
    /// multi-byte character and the text between them is the UTF-8 it already
    /// was.
    fn text(&mut self) -> Result<String, BadJson> {
        self.take(b'"')?;
        let mut out = String::new();
        let mut from = self.at;
        loop {
            match self.peek().ok_or(BadJson::Ended)? {
                b'"' => {
                    out.push_str(self.slice(from, self.at)?);
                    self.at += 1;
                    return Ok(out);
                }
                b'\\' => {
                    out.push_str(self.slice(from, self.at)?);
                    self.at += 1;
                    self.escape(&mut out)?;
                    from = self.at;
                }
                // A raw control character is not legal inside a JSON string,
                // so it is refused here. **This is grammar conformance and
                // not a safety check**, and the difference matters: a
                // backslash-u escape of the same code point is legal JSON and
                // decodes to that very byte a moment later. What may be typed
                // at somebody's terminal is a decision about the text and not
                // about its spelling, and it belongs to whoever does the
                // typing: see `prompt::typed`.
                byte if byte < 0x20 => return Err(BadJson::Malformed(self.at)),
                _ => self.at += 1,
            }
        }
    }

    fn slice(&self, from: usize, until: usize) -> Result<&str, BadJson> {
        core::str::from_utf8(&self.bytes[from..until]).map_err(|_| BadJson::Malformed(from))
    }

    fn escape(&mut self, out: &mut String) -> Result<(), BadJson> {
        let escaped = self.peek().ok_or(BadJson::Ended)?;
        self.at += 1;
        let plain = match escaped {
            b'"' => '"',
            b'\\' => '\\',
            b'/' => '/',
            b'b' => '\u{8}',
            b'f' => '\u{c}',
            b'n' => '\n',
            b'r' => '\r',
            b't' => '\t',
            b'u' => return self.escaped_char(out),
            _ => return Err(BadJson::Malformed(self.at - 1)),
        };
        out.push(plain);
        Ok(())
    }

    /// A `\u` escape, and the surrogate pair a character above the basic plane
    /// arrives as.
    ///
    /// A surrogate with no partner becomes the replacement character rather than
    /// a refusal. A rust `String` cannot hold one at all, so the choice is
    /// between refusing the whole request and carrying the rest of the text, and
    /// one bad escape in a label must not cost the pane its answer.
    fn escaped_char(&mut self, out: &mut String) -> Result<(), BadJson> {
        let first = self.hex4()?;
        let code = match first {
            0xD800..=0xDBFF => match self.low_surrogate()? {
                Some(second) => 0x1_0000 + ((first - 0xD800) << 10) + (second - 0xDC00),
                None => 0xFFFD,
            },
            0xDC00..=0xDFFF => 0xFFFD,
            other => other,
        };
        out.push(char::from_u32(code).unwrap_or('\u{fffd}'));
        Ok(())
    }

    /// The `\uDCxx` half of a pair, if that is really what comes next.
    fn low_surrogate(&mut self) -> Result<Option<u32>, BadJson> {
        if !self.bytes[self.at..].starts_with(b"\\u") {
            return Ok(None);
        }
        let resume = self.at;
        self.at += 2;
        let second = self.hex4()?;
        if (0xDC00..=0xDFFF).contains(&second) {
            return Ok(Some(second));
        }
        // Not a pair after all: leave it to be read as its own escape.
        self.at = resume;
        Ok(None)
    }

    fn hex4(&mut self) -> Result<u32, BadJson> {
        let from = self.at;
        let digits = self
            .bytes
            .get(from..from + 4)
            .ok_or(BadJson::Ended)?
            .iter()
            .try_fold(0u32, |value, byte| {
                let digit = (*byte as char).to_digit(16)?;
                Some(value * 16 + digit)
            })
            .ok_or(BadJson::Malformed(from))?;
        self.at += 4;
        Ok(digits)
    }
}

/// The body every refusal answers with.
///
/// One shape for all of them, because `share/app.html` reads exactly one thing
/// off a failed call — `if(d.error) return msg(d.error, true)` — and a refusal
/// that spelled it differently would be a call the page thinks succeeded.
///
/// The reason is a *sentence*, not a code: it is put in front of whoever is
/// holding the phone, and "no such directory" is worth more to them than any
/// enumeration this could return instead.
pub fn error(said: &str) -> String {
    format!("{{\"error\":{}}}", string(said))
}

#[cfg(test)]
mod tests {
    use super::{
        BadJson, MAX_DEPTH, Value, is_value, member, number_member, parse, string, string_member,
    };

    /// The first line of a codex rollout, which is the shape `member` exists
    /// for: everything worth knowing is inside a nested object.
    const ROLLOUT: &str = r#"{"timestamp":"2026-03-04T09:00:02.123Z","type":"session_meta","payload":{"id":"0123abcd-dead-beef-0000-111122223333","cwd":"/Users/someone/dev/thing","originator":"codex_cli_rs"}}"#;

    // AYEAYE-45 — a codex rollout's answer is a whole object one level down,
    // and the point of handing its text back is that `string_member` reads it
    // rather than a second reader that could disagree about what an object is.
    #[test]
    fn a_nested_object_comes_back_whole_and_reads_again() {
        let payload = member(ROLLOUT, "payload").expect("a payload");
        assert!(
            payload.starts_with('{') && payload.ends_with('}'),
            "{payload}"
        );
        assert_eq!(
            string_member(payload, "cwd").as_deref(),
            Some("/Users/someone/dev/thing")
        );
        assert_eq!(string_member(payload, "parent_thread_id"), None);
    }

    // AYEAYE-45 — the raw text, not a re-rendering: a member is borrowed so
    // nothing this crate writes can change what an agent wrote.
    #[test]
    fn a_member_is_the_text_that_was_written() {
        assert_eq!(member(ROW, "key"), Some(r#""AYEAYE""#));
        assert_eq!(member(r#"{"a":[1, 2 ,3]}"#, "a"), Some("[1, 2 ,3]"));
        assert_eq!(member(r#"{"a":1.50e2}"#, "a"), Some("1.50e2"));
        assert_eq!(member(r#"{"a":null}"#, "a"), Some("null"));
    }

    // AYEAYE-45 — the same narrowing `string_member` has. A truncated line is
    // the ordinary sight here: these files are read while an agent is writing
    // them.
    #[test]
    fn nothing_is_read_out_of_what_is_not_one_object() {
        assert_eq!(member(ROW, "missing"), None);
        assert_eq!(member(r#"{"key":"AYEAYE""#, "key"), None);
        assert_eq!(member(r#"{"a":1} {"a":2}"#, "a"), None);
        assert_eq!(member("[1,2]", "a"), None);
        assert_eq!(member("", "a"), None);
        assert_eq!(number_member(r#"{"startedAt":"#, "startedAt"), None);
    }

    // AYEAYE-45 — claude records when a session began as milliseconds since the
    // epoch, and that number is compared against the process's own start time.
    // A string there is not a number that happens to be quoted: reading one
    // would compare a clock against nothing.
    #[test]
    fn a_number_member_reads_and_a_string_one_does_not() {
        assert_eq!(
            number_member(r#"{"startedAt":1772614802123}"#, "startedAt"),
            Some(1_772_614_802_123.0)
        );
        assert_eq!(number_member(r#"{"a":-1.5e3}"#, "a"), Some(-1500.0));
        assert_eq!(
            number_member(r#"{"startedAt":"1772614802123"}"#, "startedAt"),
            None
        );
        assert_eq!(number_member(r#"{"startedAt":null}"#, "startedAt"), None);
    }

    // AYEAYE-45 — `json.loads` keeps the last of a repeated member, and so does
    // `string_member`. Two readers of the same text disagreeing about which one
    // won is a bug nobody would look for.
    #[test]
    fn a_repeated_member_takes_the_last_the_way_the_others_do() {
        assert_eq!(member(r#"{"a":1,"a":2}"#, "a"), Some("2"));
        assert_eq!(number_member(r#"{"a":1,"a":2}"#, "a"), Some(2.0));
        assert_eq!(
            string_member(r#"{"a":"first","a":"second"}"#, "a").as_deref(),
            Some("second")
        );
    }

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
            "  \n\t {\"a\": [1, {\"b\": null}]}  \r\n",
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
        // The cap is levels, not the index of the deepest: 128 nest, 129 does
        // not, and a constant that means one and says the other is a trap for
        // whoever next reads it as a budget.
        assert!(is_value(&format!("{}{}", "[".repeat(128), "]".repeat(128))));
        assert!(!is_value(&format!(
            "{}{}",
            "[".repeat(129),
            "]".repeat(129)
        )));
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
        // Junk after the object is not an object with a member; a reader that
        // stopped at the first match would answer a question about text that
        // never parsed. Nor is junk *before* it: the leading brace is what
        // makes this an object rather than a member found in loose text.
        assert_eq!(string_member(r#"{"key":"A"} and then junk"#, "key"), None);
        assert_eq!(string_member(r#"?"key":"A"}"#, "key"), None);
        // A name that merely starts with the one asked for is a different
        // member. `keys` and `keyboard` are both real cliban member names.
        assert_eq!(string_member(r#"{"keyboard":"X"}"#, "key"), None);
        assert_eq!(
            string_member(r#"{"keys":["A"],"key":"B"}"#, "key").as_deref(),
            Some("B")
        );
    }

    // AYEAYE-53 — cliban writes UTF-8 straight out, and a project name or a
    // ticket title is where the first non-ASCII byte arrives. A reader that
    // walked bytes without walking characters would answer `None` for every row
    // that had one, which is a board that empties itself on one accented title.
    #[test]
    fn a_member_survives_non_ascii_text_on_either_side_of_the_colon() {
        assert_eq!(
            string_member(r#"{"key":"Café — naïve 😀"}"#, "key").as_deref(),
            Some("Café — naïve 😀")
        );
        assert_eq!(
            string_member(r#"{"clé":"valeur","key":"AYEAYE"}"#, "clé").as_deref(),
            Some("valeur")
        );
        // And the member after a non-ASCII one is still found, so the scan did
        // not lose its place walking over it.
        assert_eq!(
            string_member(r#"{"clé":"valeur","key":"AYEAYE"}"#, "key").as_deref(),
            Some("AYEAYE")
        );
    }

    // AYEAYE-53 — `\uXXXX` is how a JSON encoder writes anything it would
    // rather not put in bytes, and a surrogate pair is how it writes an emoji.
    // Decoding either wrongly puts something in the app page's ticket-link
    // regular expression that no project is named.
    #[test]
    fn a_unicode_escape_decodes_and_a_surrogate_pair_is_one_character() {
        // \u00e9 is e-acute and \u0009 is a tab: an encoder writes both
        // that way, and a reader that took the four hex digits as two would
        // decode a different character.
        assert_eq!(
            string_member(r#"{"key":"A\u00e9\u0009B"}"#, "key").as_deref(),
            Some("A\u{e9}\tB")
        );
        // A surrogate pair is one character, not two replacement marks.
        assert_eq!(
            string_member(r#"{"key":"\ud83d\ude00"}"#, "key").as_deref(),
            Some("\u{1f600}")
        );
        // An escaped member name is the same member.
        assert_eq!(
            string_member(r#"{"\u006bey":"AYEAYE"}"#, "key").as_deref(),
            Some("AYEAYE")
        );
        // A high surrogate with nothing to pair with is kept rather than
        // refused: `json.loads` accepts it, and a dropped row is worse than a
        // replacement character in a title.
        assert_eq!(
            string_member(r#"{"key":"a\ud83dz"}"#, "key").as_deref(),
            Some("a\u{fffd}z")
        );
        // Four hex digits, not three and not "whatever follows".
        assert!(!is_value(r#"{"key":"\u00zz"}"#));
        assert!(!is_value(r#"{"key":"\u00e"}"#));
    }

    // AYEAYE-53 — cliban's stderr goes into a response body through here. A
    // quote or a newline that escaped its own string would make the degraded
    // answer unreadable, which is the one thing the degradation must not do.
    #[test]
    fn a_quoted_string_is_a_json_string() {
        assert_eq!(string("plain"), r#""plain""#);
        assert_eq!(
            string("he said \"no\" \\ then\nstopped\tthere"),
            r#""he said \"no\" \\ then\nstopped\tthere""#
        );
        assert_eq!(string("bell\u{7}"), r#""bell\u0007""#);
        assert_eq!(string("é"), "\"é\"");
        // And what comes out is something this crate's own scanner accepts,
        // which is the property the response body actually needs.
        assert!(is_value(&string("cliban: no such file\u{0}\"")));
    }
    use super::error;

    // AYEAYE-43 — a pane's window name is whatever somebody typed, and it lands
    // in a body the panel parses. A quote or a newline in it must escape rather
    // than end the string.
    #[test]
    fn a_string_escapes_what_would_otherwise_end_it() {
        assert_eq!(string("editor"), "\"editor\"");
        assert_eq!(string("say \"hi\""), r#""say \"hi\"""#);
        assert_eq!(string(r"C:\path"), r#""C:\\path""#);
        assert_eq!(string("two\nlines"), r#""two\nlines""#);
        assert_eq!(string("a\tb\r"), r#""a\tb\r""#);
        // Every other control character, including the escape byte an agent's
        // colouring leaves behind, gets the long form.
        assert_eq!(string("\u{1b}[31m"), r#""\u001b[31m""#);
        assert_eq!(string("\u{0}"), r#""\u0000""#);
        // And nothing else is touched: text that is already UTF-8 stays as it
        // is rather than becoming six bytes of noise per accent.
        assert_eq!(string("café ✓"), "\"café ✓\"");
    }

    fn text_of(body: &str, name: &str) -> Option<String> {
        parse(body)
            .expect("a body")
            .get(name)?
            .text()
            .map(str::to_string)
    }

    // AYEAYE-48 — the body a phone really sends to `/api/answer` and
    // `/api/send`, read back member by member. Whitespace between the tokens is
    // whitespace, and a member that is not there is not there.
    #[test]
    fn the_body_a_write_endpoint_is_handed_reads_back() {
        let body = r#" { "pane" : "desktop/%3" , "key": "1" } "#;
        assert_eq!(text_of(body, "pane").as_deref(), Some("desktop/%3"));
        assert_eq!(text_of(body, "key").as_deref(), Some("1"));
        assert_eq!(parse(body).unwrap().get("text"), None);

        let typed = r#"{"pane":"desktop/%3","text":"ls -la","enter":false}"#;
        assert_eq!(text_of(typed, "text").as_deref(), Some("ls -la"));
        assert!(!parse(typed).unwrap().get("enter").unwrap().truthy());
        assert!(
            parse(r#"{"enter":true}"#)
                .unwrap()
                .get("enter")
                .unwrap()
                .truthy()
        );
    }

    // AYEAYE-48 — `enter` decides whether a message is submitted, so what counts
    // as yes is worth pinning. The daemon writes `if req.get("enter")`, and
    // python's truthiness is what that means: nothing, zero, and empty are no.
    #[test]
    fn yes_means_what_it_means_to_the_daemon_this_replaces() {
        for yes in ["true", "1", "-0.5", r#""no""#, "[0]", r#"{"a":0}"#] {
            assert!(parse(yes).unwrap().truthy(), "{yes} should be yes");
        }
        for no in ["false", "null", "0", "0.0", r#""""#, "[]", "{}"] {
            assert!(!parse(no).unwrap().truthy(), "{no} should be no");
        }
    }

    // AYEAYE-48 — a string arrives escaped, and the escapes are what a pane id
    // or a line of typed text is really made of. A `\u` escape is a character,
    // and a character above the basic plane arrives as a surrogate pair.
    #[test]
    fn every_escape_a_string_can_carry_is_resolved() {
        assert_eq!(
            parse(r#""a\"b\\c\/d\n\r\t\b\f""#),
            Ok(Value::Text("a\"b\\c/d\n\r\t\u{8}\u{c}".to_string()))
        );
        assert_eq!(parse(r#""Aé""#), Ok(Value::Text("Aé".to_string())));
        // A surrogate pair is one character, and the emoji a phone's keyboard
        // sends is the reason this is not academic.
        assert_eq!(parse(r#""🚀""#), Ok(Value::Text("🚀".to_string())));
        // A surrogate with no partner cannot live in a rust String at all. It
        // becomes the replacement character rather than costing the pane its
        // whole answer — the text around it is what somebody meant to send.
        assert_eq!(
            parse(r#""a\ud83dz""#),
            Ok(Value::Text("a\u{fffd}z".to_string()))
        );
        assert_eq!(
            parse(r#""a\udc00""#),
            Ok(Value::Text("a\u{fffd}".to_string()))
        );
        // A high surrogate followed by an escape that is not its partner: the
        // second escape is still read as itself rather than swallowed.
        assert_eq!(
            parse(r#""\ud83dA""#),
            Ok(Value::Text("\u{fffd}A".to_string()))
        );
    }

    // AYEAYE-48 — the body comes off a socket, so every way it can be malformed
    // has to be an answer rather than a panic or a guess. A truncated string is
    // the shape a cut connection leaves behind.
    #[test]
    fn a_body_that_is_not_json_is_refused_rather_than_guessed_at() {
        for broken in [
            r#"{"pane": "desktop/%3""#,
            r#"{"pane": }"#,
            r#"{"pane" "desktop/%3"}"#,
            r#"{pane: 1}"#,
            r#""unterminated"#,
            r#""bad \q escape""#,
            r#""short \u12""#,
            r#""not hex \uzzzz""#,
            "",
            "   ",
            "tru",
            "01x",
        ] {
            assert!(parse(broken).is_err(), "{broken:?} is not JSON");
        }
        // A complete value with something after it is refused too: a body read
        // as its first half is a body read as something nobody sent.
        assert_eq!(parse(r#"{} {}"#), Err(BadJson::Trailing(3)));
        // A raw control character inside a string is not legal JSON, and these
        // strings become a pane id and text typed at somebody's terminal.
        assert_eq!(parse("\"a\nb\"").unwrap_err(), BadJson::Malformed(2));
        assert_eq!(parse("\"a\u{1b}[31m\"").unwrap_err(), BadJson::Malformed(2));
    }

    // AYEAYE-48 — a body of nothing but brackets. A recursive reader recurses
    // once per bracket, and a stack overflow is an abort rather than an error:
    // one request would take the whole daemon down, every pane with it, and
    // the panel would go dark for everybody. The reader is behind the token
    // gate, so this is a phone that has been logged in and then lost rather
    // than a stranger — which is a smaller blast radius and not a reason to
    // leave an abort reachable at all.
    #[test]
    fn a_body_of_nothing_but_brackets_is_refused_rather_than_recursed_into() {
        let deep = format!("{}{}", "[".repeat(5000), "]".repeat(5000));
        assert!(matches!(parse(&deep), Err(BadJson::TooDeep(_))));
        let deep_objects = format!(
            "{}null{}",
            r#"{"a":"#.repeat(MAX_DEPTH + 1),
            "}".repeat(MAX_DEPTH + 1)
        );
        assert!(matches!(parse(&deep_objects), Err(BadJson::TooDeep(_))));

        // And a body nested as deeply as anything real gets still reads.
        assert_eq!(
            parse(r#"{"a":[{"b":[1,2]}]}"#)
                .expect("four deep is not too deep")
                .get("a")
                .map(|value| matches!(value, Value::List(_))),
            Some(true)
        );
    }

    // AYEAYE-48 — a body that is not an object at all has no members, and asking
    // one for a pane must be `None` rather than anything else. This is the shape
    // `[]` and `"desktop/%3"` arrive in when somebody is probing.
    #[test]
    fn a_body_that_is_not_an_object_has_no_members() {
        for body in ["[]", r#""desktop/%3""#, "12", "null", "true"] {
            let value = parse(body).expect("still JSON");
            assert_eq!(value.get("pane"), None, "for {body}");
        }
        assert_eq!(parse("[]").unwrap().text(), None);
        assert_eq!(parse("12").unwrap().text(), None);
    }

    // AYEAYE-48 — a body carrying one name twice is a body built to be read two
    // ways. The first wins, which is what `parse_qs(...)[0]` does everywhere
    // else here, so the two never disagree about which value was meant.
    #[test]
    fn a_name_written_twice_is_read_as_the_first_of_them() {
        let body = r#"{"pane":"desktop/%3","pane":"desktop/%9"}"#;
        assert_eq!(text_of(body, "pane").as_deref(), Some("desktop/%3"));
    }

    // AYEAYE-51 — every refusal answers in the one shape the page reads. The
    // reason often quotes what tmux said, and what tmux said is not a string
    // literal somebody chose — so it goes through the escaping like any other
    // text, or the first refusal mentioning a quoted path is a body nothing
    // can parse.
    #[test]
    fn a_refusal_is_a_sentence_in_the_shape_the_page_reads() {
        assert_eq!(error("no such pane"), r#"{"error":"no such pane"}"#);
        assert_eq!(
            error("could not kill pane: can't find pane: \"%9\"\n"),
            r#"{"error":"could not kill pane: can't find pane: \"%9\"\n"}"#
        );
    }
}
