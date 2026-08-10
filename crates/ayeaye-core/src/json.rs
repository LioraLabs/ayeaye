//! Just enough JSON to answer with.
//!
//! Writing this rather than depending on a serialiser is not thrift. This crate
//! declares no dependencies at all — the constitution's allowlist is empty and a
//! name goes on it only with an argument that its own transitive dependencies
//! reach nothing — and a pure crate whose whole job is "text and structs in,
//! text and structs out" can spell a string literal itself.
//!
//! There is no reader here, and there should not be. The daemon *is* handed
//! JSON now — a spawn or a kill arrives as a request body — but reading a
//! hostile document is the one job worth a real parser, and this crate is the
//! one place that cannot have one. So the shell parses, and hands this crate
//! the strings it found.

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
    use super::{error, string};

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
