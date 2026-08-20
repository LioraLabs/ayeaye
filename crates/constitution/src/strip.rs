//! Reduce Rust source to the part of it that runs.
//!
//! Without this, the constitution convicts itself: the budget table names
//! every reach it forbids, the README quotes them, and a doc comment
//! explaining why `std::process` is out reads exactly like a reach to
//! `std::process`. Prose is not a reach.
//!
//! One lexer, three views. [`code_only`] blanks comments *and* literals, which
//! is what a rule comparing reaches wants. [`without_comments`] blanks only
//! the comments, which is what a rule comparing copied code wants — two runs
//! that differ only in their string literals are two different decisions, and
//! blanking the literals would convict them of agreeing. [`string_literals`]
//! is the literals alone, which is what the duplicate-literal rule reads.

/// One string literal, as written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Literal {
    /// The text between the quotes, escapes left as they were spelled.
    pub text: String,
    /// The 1-based line the literal opens on.
    pub line: usize,
}

/// What one span of source is, as the lexer sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// Code proper, including lifetimes.
    Code,
    /// A `//` comment, up to but not including its newline.
    LineComment,
    /// A `/* … */` comment, nesting included.
    BlockComment,
    /// A character or byte literal.
    CharLiteral,
    /// A string of any flavor. `inner` is the char range between the quotes.
    Str {
        inner_start: usize,
        inner_end: usize,
    },
}

/// One lexed span: its kind and the half-open char range it covers.
#[derive(Debug, Clone, Copy)]
struct Segment {
    kind: Kind,
    start: usize,
    end: usize,
}

/// Blank out comments and the contents of literals, leaving every other byte
/// where it was.
///
/// Newlines survive inside what is blanked, so a line number taken from the
/// result is a line number in the original.
pub fn code_only(source: &str) -> String {
    render(source, true)
}

/// Blank out comments only. String and character literals stay verbatim.
///
/// This is the view the duplicate-code rule compares: a copied run is copied
/// literals and all, and two runs that differ only inside their strings are
/// not copies of each other.
pub fn without_comments(source: &str) -> String {
    render(source, false)
}

/// Every string literal in the source, comments skipped, in order.
///
/// The text is the spelling between the quotes — escapes as written, raw
/// strings unwrapped of their hashes. A literal quoted in a comment is prose
/// and does not appear.
pub fn string_literals(source: &str) -> Vec<Literal> {
    let chars: Vec<char> = source.chars().collect();
    let mut literals = Vec::new();
    let mut line = 1usize;
    let mut cursor = 0usize;
    for segment in segments(&chars) {
        line += chars[cursor..segment.start]
            .iter()
            .filter(|c| **c == '\n')
            .count();
        cursor = segment.start;
        if let Kind::Str {
            inner_start,
            inner_end,
        } = segment.kind
        {
            literals.push(Literal {
                text: chars[inner_start..inner_end].iter().collect(),
                line,
            });
        }
    }
    literals
}

/// Blank out every `#[cfg(test)]`-attributed item, comments and code alike.
///
/// The duplication rules judge shipped code, and a `#[cfg(test)]` module is a
/// test that happens to live in `src/` — its literals are the independent
/// restatements a non-tautological test is *supposed* to make. The extent of
/// the item is found on the [`code_only`] view, so a `{` inside a string
/// cannot derail the brace count and a `#[cfg(test)]` quoted in a comment is
/// prose.
///
/// Only the exact spelling `#[cfg(test)]` is recognized. Test code reached
/// through `#[cfg(all(test, …))]` or `#[cfg_attr(test, …)]` stays in scope —
/// the loud direction: at worst a duplication rule fires on a test and
/// somebody waives it, never the other way round.
pub fn without_test_blocks(source: &str) -> String {
    let chars: Vec<char> = source.chars().collect();
    let stripped: Vec<char> = code_only(source).chars().collect();
    let marker: Vec<char> = "#[cfg(test)]".chars().collect();

    let mut ranges: Vec<(usize, usize)> = Vec::new();
    let mut i = 0;
    while i + marker.len() <= stripped.len() {
        if stripped[i..i + marker.len()] != marker[..] {
            i += 1;
            continue;
        }
        let end = item_end(&stripped, i + marker.len());
        ranges.push((i, end));
        i = end;
    }

    let mut out = String::with_capacity(source.len());
    let mut next_range = ranges.iter().peekable();
    let mut i = 0;
    while i < chars.len() {
        match next_range.peek() {
            Some((start, end)) if *start == i => {
                for c in &chars[*start..*end] {
                    blank(&mut out, *c);
                }
                i = *end;
                next_range.next();
            }
            _ => {
                out.push(chars[i]);
                i += 1;
            }
        }
    }
    out
}

/// The char index just past the item an attribute at `from` governs, on the
/// stripped view: through the matching `}` of its first brace, or through the
/// `;` of a braceless item, whichever comes first.
///
/// A `,` also ends it, and a `}` ends it *without* being consumed: the
/// attribute may sit on a struct field or an enum variant, and scanning past
/// the enclosing item's close brace would blank whatever shipped code comes
/// next — a silent exemption, which is the one failure shape this crate is
/// built to refuse. Both terminators count only outside `(…)` and `[…]`, so a
/// multi-argument test fn, a stacked `#[derive(…)]`, a tuple struct and a
/// `[u8; 2]` field all blank to their real extent. Angle brackets are *not*
/// tracked — `a < b` in an expression opens a depth that never closes — so a
/// comma inside bare generics (`fn t<T, U>`) still stops this early and
/// leaves a mangled tail in scope. The nesting count saturates rather than
/// underflows, so an attribute on the *last* field of a tuple struct sees the
/// struct's own `)` as a stray closer and consumes through the `;`, mangling
/// the struct it sits in. Both err loud: at worst a false positive somebody
/// can see, never a silent exemption.
fn item_end(stripped: &[char], from: usize) -> usize {
    let mut nesting = 0usize;
    let mut i = from;
    while i < stripped.len() {
        match stripped[i] {
            '(' | '[' => nesting += 1,
            ')' | ']' => nesting = nesting.saturating_sub(1),
            ';' | ',' if nesting == 0 => return i + 1,
            '}' if nesting == 0 => return i,
            '{' if nesting == 0 => {
                let mut depth = 0usize;
                while i < stripped.len() {
                    match stripped[i] {
                        '{' => depth += 1,
                        '}' => {
                            depth -= 1;
                            if depth == 0 {
                                return i + 1;
                            }
                        }
                        _ => {}
                    }
                    i += 1;
                }
                return stripped.len();
            }
            _ => {}
        }
        i += 1;
    }
    stripped.len()
}

/// Both blanking views, off one lexer pass.
fn render(source: &str, blank_literals: bool) -> String {
    let chars: Vec<char> = source.chars().collect();
    let mut out = String::with_capacity(source.len());
    for segment in segments(&chars) {
        let keep = match segment.kind {
            Kind::Code => true,
            Kind::LineComment | Kind::BlockComment => false,
            Kind::CharLiteral | Kind::Str { .. } => !blank_literals,
        };
        for c in &chars[segment.start..segment.end] {
            if keep {
                out.push(*c);
            } else {
                blank(&mut out, *c);
            }
        }
    }
    out
}

/// Lex the source into contiguous spans. Every char lands in exactly one span.
fn segments(chars: &[char]) -> Vec<Segment> {
    let mut segments = Vec::new();
    let mut code_start = 0usize;
    let mut i = 0usize;

    let close_code = |segments: &mut Vec<Segment>, code_start: usize, at: usize| {
        if at > code_start {
            segments.push(Segment {
                kind: Kind::Code,
                start: code_start,
                end: at,
            });
        }
    };

    while i < chars.len() {
        let c = chars[i];

        // Line comment, up to but not including its newline.
        if c == '/' && chars.get(i + 1) == Some(&'/') {
            close_code(&mut segments, code_start, i);
            let start = i;
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            segments.push(Segment {
                kind: Kind::LineComment,
                start,
                end: i,
            });
            code_start = i;
            continue;
        }

        // Block comment, which nests in Rust.
        if c == '/' && chars.get(i + 1) == Some(&'*') {
            close_code(&mut segments, code_start, i);
            let start = i;
            let mut depth = 0usize;
            while i < chars.len() {
                if chars[i] == '/' && chars.get(i + 1) == Some(&'*') {
                    depth += 1;
                    i += 2;
                    continue;
                }
                if chars[i] == '*' && chars.get(i + 1) == Some(&'/') {
                    depth -= 1;
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                    continue;
                }
                i += 1;
            }
            segments.push(Segment {
                kind: Kind::BlockComment,
                start,
                end: i,
            });
            code_start = i;
            continue;
        }

        // Raw string, with any number of hashes: r"…", br##"…"##.
        if let Some((inner_start, inner_end, next)) = raw_string(chars, i) {
            close_code(&mut segments, code_start, i);
            segments.push(Segment {
                kind: Kind::Str {
                    inner_start,
                    inner_end,
                },
                start: i,
                end: next,
            });
            i = next;
            code_start = i;
            continue;
        }

        // Ordinary string or byte string. The `b` of a byte string is left as
        // code, which blanks the same way it always has.
        if c == '"' {
            close_code(&mut segments, code_start, i);
            let start = i;
            let inner_start = i + 1;
            let mut closed = false;
            i += 1;
            while i < chars.len() {
                if chars[i] == '\\' {
                    i += 2;
                    continue;
                }
                if chars[i] == '"' {
                    i += 1;
                    closed = true;
                    break;
                }
                i += 1;
            }
            i = i.min(chars.len());
            let inner_end = if closed { i - 1 } else { i };
            segments.push(Segment {
                kind: Kind::Str {
                    inner_start,
                    inner_end,
                },
                start,
                end: i,
            });
            code_start = i;
            continue;
        }

        // A character literal, or a lifetime. `'a'` is a literal; `'a` is a
        // lifetime, and treating it as an unterminated literal would swallow
        // the rest of the file.
        if c == '\'' {
            if let Some(end) = char_literal(chars, i) {
                close_code(&mut segments, code_start, i);
                segments.push(Segment {
                    kind: Kind::CharLiteral,
                    start: i,
                    end,
                });
                i = end;
                code_start = i;
                continue;
            }
        }

        i += 1;
    }

    close_code(&mut segments, code_start, i.min(chars.len()));
    segments
}

/// Push a space for anything but a newline, which is kept so that line
/// numbers taken from the stripped text still mean something.
fn blank(out: &mut String, c: char) {
    out.push(if c == '\n' { '\n' } else { ' ' });
}

/// The inner range and the index just past a raw string starting at `i`, if
/// one starts there.
fn raw_string(chars: &[char], i: usize) -> Option<(usize, usize, usize)> {
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
    let inner_start = j;
    while j < chars.len() {
        if chars[j] == '"' && chars[j + 1..].iter().take(hashes).all(|c| *c == '#') {
            let close = j + 1 + hashes;
            return Some((inner_start, j, close.min(chars.len())));
        }
        j += 1;
    }
    Some((inner_start, chars.len(), chars.len()))
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
    use super::{code_only, string_literals, without_comments, without_test_blocks};

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

    // AYEAYE-64 — the duplicate-code rule compares literals as part of the
    // code, so this view must keep them while still refusing prose.
    #[test]
    fn without_comments_keeps_the_string_and_drops_the_comment() {
        let out = without_comments("serve(\"/api/panes\"); // the panes route\n");
        assert!(out.contains("\"/api/panes\""), "{out}");
        assert!(!out.contains("panes route"), "{out}");
    }

    // AYEAYE-64 — a `//` inside a string is not a comment.
    #[test]
    fn a_slash_slash_inside_a_string_is_not_a_comment() {
        let out = without_comments("let u = \"http://example\"; let t = 1;");
        assert!(out.contains("http://example"), "{out}");
        assert!(out.contains("let t = 1;"), "{out}");
    }

    // AYEAYE-64 — a literal quoted in a comment is prose, not a literal.
    #[test]
    fn a_literal_in_a_comment_is_not_collected() {
        let literals = string_literals("// says \"hello there\"\nlet s = \"general kenobi\";\n");
        assert_eq!(literals.len(), 1);
        assert_eq!(literals[0].text, "general kenobi");
        assert_eq!(literals[0].line, 2);
    }

    // AYEAYE-64 — raw strings unwrap to the text between the delimiters.
    #[test]
    fn a_raw_string_yields_its_inner_text() {
        let literals = string_literals("let s = r#\"a \" b\"#;");
        assert_eq!(literals.len(), 1);
        assert_eq!(literals[0].text, "a \" b");
    }

    // AYEAYE-64 — escapes are compared as spelled, not decoded; two crates
    // that copied the same spelling should compare equal, which is all the
    // duplicate-literal rule needs.
    #[test]
    fn escapes_stay_as_written() {
        let literals = string_literals("let s = \"a\\n\\\"b\";");
        assert_eq!(literals.len(), 1);
        assert_eq!(literals[0].text, "a\\n\\\"b");
    }

    // AYEAYE-64 — a cfg(test) module in src/ is a test, and its contents are
    // not shipped code.
    #[test]
    fn a_cfg_test_module_is_blanked_and_the_code_around_it_survives() {
        let source = "fn shipped() {}\n#[cfg(test)]\nmod tests {\n    fn t() { let s = \"copied\"; }\n}\nfn also_shipped() {}\n";
        let out = without_test_blocks(source);
        assert!(out.contains("fn shipped()"), "{out}");
        assert!(out.contains("fn also_shipped()"), "{out}");
        assert!(!out.contains("copied"), "{out}");
        assert_eq!(out.lines().count(), source.lines().count());
    }

    // AYEAYE-64 — a `{` inside a string must not derail the brace count.
    #[test]
    fn a_brace_inside_a_string_does_not_derail_the_blanking() {
        let source = "#[cfg(test)]\nmod tests {\n    const S: &str = \"{\";\n}\nfn shipped() {}\n";
        let out = without_test_blocks(source);
        assert!(out.contains("fn shipped()"), "{out}");
        assert!(!out.contains("mod tests"), "{out}");
    }

    // AYEAYE-64 — an attributed item with no braces ends at its semicolon.
    #[test]
    fn a_braceless_cfg_test_item_ends_at_the_semicolon() {
        let source = "#[cfg(test)]\nuse helper::thing;\nfn shipped() {}\n";
        let out = without_test_blocks(source);
        assert!(!out.contains("helper"), "{out}");
        assert!(out.contains("fn shipped()"), "{out}");
    }

    // AYEAYE-64 — `#[cfg(test)]` in a comment is prose; blanking from it
    // would swallow shipped code.
    #[test]
    fn a_cfg_test_named_in_a_comment_blanks_nothing() {
        let source = "// mentions #[cfg(test)] in prose\nfn shipped() {}\n";
        let out = without_test_blocks(source);
        assert!(out.contains("fn shipped()"), "{out}");
    }

    // AYEAYE-64 — the attribute may sit on a struct field, which ends at a
    // comma. Scanning past the struct's close brace would blank whatever
    // shipped item comes next: a silent exemption, the failure shape this
    // whole crate exists to refuse.
    #[test]
    fn a_cfg_test_field_does_not_swallow_the_item_after_the_struct() {
        let source =
            "struct S {\n    #[cfg(test)]\n    probe: u8,\n}\nfn shipped() { real_work(); }\n";
        let out = without_test_blocks(source);
        assert!(out.contains("real_work"), "{out}");
        assert!(!out.contains("probe"), "{out}");
    }

    // AYEAYE-64 — a last field with no trailing comma ends at the struct's
    // own `}`, which belongs to the struct and must survive.
    #[test]
    fn a_cfg_test_field_without_a_trailing_comma_leaves_the_brace() {
        let source = "struct S {\n    #[cfg(test)]\n    probe: u8\n}\nfn shipped() {}\n";
        let out = without_test_blocks(source);
        assert!(out.contains("fn shipped()"), "{out}");
        assert!(!out.contains("probe"), "{out}");
        assert_eq!(out.matches('}').count(), source.matches('}').count());
    }

    // AYEAYE-64 — a comma inside an argument list is not a field's comma: a
    // multi-argument test fn blanks whole, or its body's literals leak back
    // into the duplication scope.
    #[test]
    fn a_cfg_test_fn_with_two_arguments_is_blanked_whole() {
        let source =
            "#[cfg(test)]\nfn helper(a: u8, b: u8) { let s = \"copied\"; }\nfn shipped() {}\n";
        let out = without_test_blocks(source);
        assert!(out.contains("fn shipped()"), "{out}");
        assert!(!out.contains("copied"), "{out}");
        assert!(!out.contains("b: u8"), "{out}");
    }

    // AYEAYE-64 — a stacked #[derive(Debug, Clone)] carries a comma of its
    // own; stopping inside it would leave the whole item in scope.
    #[test]
    fn a_cfg_test_item_with_a_stacked_derive_is_blanked_whole() {
        let source =
            "#[cfg(test)]\n#[derive(Debug, Clone)]\nstruct Probe { x: u8 }\nfn shipped() {}\n";
        let out = without_test_blocks(source);
        assert!(out.contains("fn shipped()"), "{out}");
        assert!(!out.contains("Probe"), "{out}");
        assert!(!out.contains("Clone"), "{out}");
    }

    // AYEAYE-64 — a tuple struct's commas sit inside its parens; it ends at
    // the semicolon.
    #[test]
    fn a_cfg_test_tuple_struct_is_blanked_through_its_semicolon() {
        let source = "#[cfg(test)]\nstruct Probe(u8, u8);\nfn shipped() {}\n";
        let out = without_test_blocks(source);
        assert!(out.contains("fn shipped()"), "{out}");
        assert!(!out.contains("Probe"), "{out}");
        assert!(!out.contains("u8);"), "{out}");
    }

    // AYEAYE-64 — an array-typed field's `;` sits inside its brackets; the
    // field blanks to its comma, and the next field survives.
    #[test]
    fn a_cfg_test_array_field_blanks_to_its_own_comma() {
        let source = "struct S {\n    #[cfg(test)]\n    probe: [u8; 2],\n    kept: u8,\n}\nfn shipped() {}\n";
        let out = without_test_blocks(source);
        assert!(out.contains("kept: u8"), "{out}");
        assert!(out.contains("fn shipped()"), "{out}");
        assert!(!out.contains("probe"), "{out}");
        // The whole type blanks, not just up to the `;` inside the brackets —
        // this is the assertion that dies on a scan with no nesting count.
        assert!(!out.contains("2]"), "{out}");
    }
}
