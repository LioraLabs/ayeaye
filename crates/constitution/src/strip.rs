//! Reduce Rust source to the part of it that runs.
//!
//! Without this, the constitution convicts itself: the budget table names
//! every reach it forbids, the README quotes them, and a doc comment
//! explaining why `std::process` is out reads exactly like a reach to
//! `std::process`. Prose is not a reach.

/// Blank out comments and the contents of literals, leaving every other byte
/// where it was.
///
/// Newlines survive inside what is blanked, so a line number taken from the
/// result is a line number in the original.
pub fn code_only(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    let chars: Vec<char> = source.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        let c = chars[i];

        // Line comment.
        if c == '/' && chars.get(i + 1) == Some(&'/') {
            while i < chars.len() && chars[i] != '\n' {
                out.push(' ');
                i += 1;
            }
            continue;
        }

        // Block comment, which nests in Rust.
        if c == '/' && chars.get(i + 1) == Some(&'*') {
            let mut depth = 0usize;
            while i < chars.len() {
                if chars[i] == '/' && chars.get(i + 1) == Some(&'*') {
                    depth += 1;
                    blank(&mut out, chars[i]);
                    blank(&mut out, chars[i + 1]);
                    i += 2;
                    continue;
                }
                if chars[i] == '*' && chars.get(i + 1) == Some(&'/') {
                    depth -= 1;
                    blank(&mut out, chars[i]);
                    blank(&mut out, chars[i + 1]);
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                    continue;
                }
                blank(&mut out, chars[i]);
                i += 1;
            }
            continue;
        }

        // Raw string, with any number of hashes: r"…", br##"…"##.
        if let Some(next) = raw_string(&chars, i) {
            for c in &chars[i..next] {
                blank(&mut out, *c);
            }
            i = next;
            continue;
        }

        // Ordinary string or byte string.
        if c == '"' {
            out.push(' ');
            i += 1;
            while i < chars.len() {
                if chars[i] == '\\' {
                    blank(&mut out, chars[i]);
                    if i + 1 < chars.len() {
                        blank(&mut out, chars[i + 1]);
                    }
                    i += 2;
                    continue;
                }
                if chars[i] == '"' {
                    out.push(' ');
                    i += 1;
                    break;
                }
                blank(&mut out, chars[i]);
                i += 1;
            }
            continue;
        }

        // A character literal, or a lifetime. `'a'` is a literal; `'a` is a
        // lifetime, and treating it as an unterminated literal would swallow
        // the rest of the file.
        if c == '\'' {
            if let Some(end) = char_literal(&chars, i) {
                for c in &chars[i..end] {
                    blank(&mut out, *c);
                }
                i = end;
            } else {
                out.push(c);
                i += 1;
            }
            continue;
        }

        out.push(c);
        i += 1;
    }

    out
}

/// Push a space for anything but a newline, which is kept so that line
/// numbers taken from the stripped text still mean something.
fn blank(out: &mut String, c: char) {
    out.push(if c == '\n' { '\n' } else { ' ' });
}

/// The index just past a raw string starting at `i`, if one starts there.
fn raw_string(chars: &[char], i: usize) -> Option<usize> {
    let mut j = i;
    if chars.get(j) == Some(&'b') {
        j += 1;
    }
    if chars.get(j) != Some(&'r') {
        return None;
    }
    j += 1;
    let hash_start = j;
    while chars.get(j) == Some(&'#') {
        j += 1;
    }
    let hashes = j - hash_start;
    if chars.get(j) != Some(&'"') {
        return None;
    }
    // A raw string may not be preceded by an identifier byte — `for_r"x"` is
    // not a thing, but `br` inside a longer word would otherwise match.
    if i > 0 && (chars[i - 1].is_alphanumeric() || chars[i - 1] == '_') {
        return None;
    }
    j += 1;
    while j < chars.len() {
        if chars[j] == '"' && chars[j + 1..].iter().take(hashes).all(|c| *c == '#') {
            let close = j + 1 + hashes;
            return Some(close.min(chars.len()));
        }
        j += 1;
    }
    Some(chars.len())
}

/// The index just past a character literal starting at `i`, if that quote
/// opens a literal rather than a lifetime.
fn char_literal(chars: &[char], i: usize) -> Option<usize> {
    if chars.get(i + 1) == Some(&'\\') {
        let mut j = i + 2;
        while j < chars.len() && chars[j] != '\'' {
            j += 1;
        }
        return Some((j + 1).min(chars.len()));
    }
    if chars.get(i + 2) == Some(&'\'') {
        return Some(i + 3);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::code_only;

    // AYEAYE-41
    #[test]
    fn a_line_comment_becomes_blank_and_the_line_count_survives() {
        assert_eq!(
            code_only("let a = 1; // std::fs\nlet b = 2;\n")
                .lines()
                .count(),
            2
        );
        assert!(!code_only("let a = 1; // std::fs\n").contains("std"));
    }

    // AYEAYE-41
    #[test]
    fn a_nested_block_comment_closes_where_it_should() {
        let stripped = code_only("/* outer /* inner */ still */ let a = 1;");
        assert!(!stripped.contains("inner"));
        assert!(stripped.contains("let a = 1;"));
    }

    // AYEAYE-41
    #[test]
    fn a_raw_string_with_hashes_is_emptied_without_ending_early() {
        let stripped = code_only("let s = r#\"a \" b std::fs\"#; let t = 1;");
        assert!(!stripped.contains("std"));
        assert!(stripped.contains("let t = 1;"));
    }

    // AYEAYE-41
    #[test]
    fn an_escaped_quote_does_not_end_the_string() {
        let stripped = code_only("let s = \"a \\\" std::fs\"; let t = 1;");
        assert!(!stripped.contains("std"));
        assert!(stripped.contains("let t = 1;"));
    }

    // AYEAYE-41
    #[test]
    fn a_lifetime_is_not_an_unterminated_character_literal() {
        let stripped = code_only("fn f<'a>(x: &'a str) { let c = '\\''; g(x); }");
        assert!(stripped.contains("g(x);"));
    }
}
