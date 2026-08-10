//! Just enough JSON to answer with.
//!
//! Writing this rather than depending on a serialiser is not thrift. This crate
//! declares no dependencies at all — the constitution's allowlist is empty and a
//! name goes on it only with an argument that its own transitive dependencies
//! reach nothing — and a pure crate whose whole job is "text and structs in,
//! text and structs out" can spell a string literal itself.
//!
//! There is no general reader here, and there should not be: nothing this
//! daemon is *handed* arrives as JSON it has to parse. The one exception is a
//! model's `config.json`, which AYEAYE-56 has to read one field of before it
//! will download the weights beside it — and that lives in
//! [`crate::model::config`], narrow enough to stay an exception rather than
//! becoming a parser by increments.

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

#[cfg(test)]
mod tests {
    use super::string;

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
}
