//! Just enough JSON to answer with, and just enough to be asked in.
//!
//! Writing this rather than depending on a serialiser is not thrift. This crate
//! declares no dependencies at all — the constitution's allowlist is empty and a
//! name goes on it only with an argument that its own transitive dependencies
//! reach nothing — and a pure crate whose whole job is "text and structs in,
//! text and structs out" can spell a string literal itself.
//!
//! **This file used to say there was no reader here and should not be, because
//! nothing the daemon was handed arrived as JSON.** That stopped being true with
//! the first write endpoint: `/api/answer` and `/api/send` are handed a body a
//! phone wrote, and reading it is a decision about text — which is this crate's
//! work and not the socket's. So there is a reader now, and it is deliberately
//! the smallest one that can be trusted with a body from the network: it
//! allocates nothing it was not shown, it refuses rather than guesses, and it
//! will not recurse further than [`MAX_DEPTH`].

/// One JSON string literal, quotes included.
///
/// Escapes what the grammar requires — the quote and the backslash — and every
/// control character, which is the half that is usually got wrong. A window
/// name really can carry a newline, and a body that carries it raw is not JSON
/// at all.
///
/// Everything else is passed through as the UTF-8 it already is. `\u` escaping
/// the rest would be legal and would turn every accented directory name into
/// six bytes of noise for no reader's benefit.
pub fn string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for character in value.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
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
        // slightly wider grammar than JSON's — it would take `1.` — and that is
        // a deliberate trade: nothing here routes on a number, and a hand-rolled
        // float grammar is a much better place for a bug than a laxity.
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
                // A raw control character is not legal inside a JSON string, and
                // is refused rather than carried: the strings here end up in a
                // pane id and in text typed at somebody's terminal.
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

#[cfg(test)]
mod tests {
    use super::{BadJson, MAX_DEPTH, Value, parse, string};

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
    // it would take the whole daemon down, from a request an unauthenticated
    // caller can shape. The limit turns that into a refusal.
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
}
