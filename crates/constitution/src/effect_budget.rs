//! Rule 1: what `ayeaye-core` is allowed to reach.

use crate::finding::{Finding, Rule};

/// How a budget entry is compared against a reach.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Match {
    /// The entry and everything under it. `std::fs` catches `std::fs::File`.
    Prefix,
    /// The entry and nothing else. `std::time` catches the bare module import
    /// that would let `t::Instant` in through an alias, while leaving
    /// `std::time::Duration` alone.
    Exact,
}

struct Budget {
    reach: &'static str,
    mode: Match,
    why: &'static str,
}

/// The reaches a pure crate may not have.
///
/// Ordered roughly by how often they are reached for. Each entry is a *reach*,
/// not a spelling: the scanner expands `use` trees before comparing, so
/// `use std::time::{Duration, Instant}` is two reaches and only one of them is
/// here.
const BUDGET: &[Budget] = &[
    Budget {
        reach: "std::fs",
        mode: Match::Prefix,
        why: "the filesystem is the shell's; take the text as an argument",
    },
    Budget {
        reach: "std::io",
        mode: Match::Prefix,
        why: "reading and writing streams is the shell's; return the value instead",
    },
    Budget {
        reach: "std::net",
        mode: Match::Prefix,
        why: "the network is the shell's",
    },
    Budget {
        reach: "std::process",
        mode: Match::Prefix,
        why: "starting processes is the shell's; parse the output it hands you",
    },
    Budget {
        reach: "std::env",
        mode: Match::Prefix,
        why: "the process environment is the shell's; env! is compile-time and fine",
    },
    Budget {
        reach: "std::thread",
        mode: Match::Prefix,
        why: "a pure function does not wait or fan out",
    },
    Budget {
        reach: "std::time::Instant",
        mode: Match::Prefix,
        why: "the clock is an input; take the instant as an argument",
    },
    Budget {
        reach: "std::time::SystemTime",
        mode: Match::Prefix,
        why: "the clock is an input; take the timestamp as an argument",
    },
    Budget {
        reach: "std::os",
        mode: Match::Prefix,
        why: "operating-system specifics belong above the core",
    },
    Budget {
        reach: "std::sync::mpsc",
        mode: Match::Prefix,
        why: "channels imply threads, which the core does not have",
    },
    Budget {
        reach: "std::time",
        mode: Match::Exact,
        why: "import the item, not the module: a module alias hides the clock",
    },
    Budget {
        reach: "std",
        mode: Match::Exact,
        why: "import the item, not the crate: a crate alias hides everything",
    },
    Budget {
        reach: "libc",
        mode: Match::Prefix,
        why: "the C library is the shell's",
    },
    Budget {
        reach: "tokio",
        mode: Match::Prefix,
        why: "async colouring stops at the shell boundary",
    },
];

/// Macros that need no import because the prelude already brought them, and
/// that write to a stream when they run. The reach scan cannot see these:
/// there is no path to expand.
const EFFECTFUL_MACROS: &[&str] = &["print", "println", "eprint", "eprintln", "dbg"];

/// Method names that are effectful whatever the receiver turns out to be.
/// Belt to the reach scan's braces — the import that made the receiver is
/// normally caught first, and this catches the receiver that arrived as an
/// argument.
const EFFECTFUL_METHODS: &[&str] = &[
    "read_to_string",
    "read_to_end",
    "write_all",
    "canonicalize",
    "create_dir_all",
    "remove_file",
    "spawn",
];

/// Judge one source file against the effect budget.
///
/// Takes the text rather than a path, so a planted violation is a string
/// literal in a test rather than a fixture on disk.
pub fn scan(path: &str, source: &str) -> Vec<Finding> {
    let mut code = crate::strip::code_only(source);
    let mut findings: Vec<Finding> = Vec::new();

    // `use` items first, because they are the only place a reach is written as
    // a tree rather than as a path. They are then blanked out, so the path
    // scan below cannot read `use std::time::{Duration, Instant}` as a bare
    // reach to `std::time` and convict the Duration along with the clock.
    for (start, end) in use_items(&code) {
        let line = line_of(&code, start);
        for reach in expand_use(&code[start + "use".len()..end]) {
            if let Some(budget) = judge(&reach) {
                push(&mut findings, path, &reach, budget, line);
            }
        }
        blank(&mut code, start, end);
    }

    for (offset, chain) in chains(&code) {
        if let Some(budget) = judge(&chain) {
            push(&mut findings, path, &chain, budget, line_of(&code, offset));
        }
    }

    for (offset, name) in calls(&code, b'!') {
        if EFFECTFUL_MACROS.contains(&name.as_str()) {
            let budget = &Budget {
                reach: "",
                mode: Match::Exact,
                why: "writing to a stream is the shell's; return the string instead",
            };
            let what = format!("{name}!");
            push(&mut findings, path, &what, budget, line_of(&code, offset));
        }
    }

    for (offset, name) in calls(&code, b'(') {
        if EFFECTFUL_METHODS.contains(&name.as_str()) {
            let budget = &Budget {
                reach: "",
                mode: Match::Exact,
                why: "the call is effectful whatever the receiver is; do it above the core",
            };
            let what = format!(".{name}()");
            push(&mut findings, path, &what, budget, line_of(&code, offset));
        }
    }

    findings
}

/// Identifiers immediately followed by `terminator`, and preceded by nothing
/// that would make them part of a longer path. `!` finds macro invocations;
/// `(` finds calls, which are only interesting here when the call is a method
/// on something — hence the leading `.`.
fn calls(code: &str, terminator: u8) -> Vec<(usize, String)> {
    let bytes = code.as_bytes();
    let wants_dot = terminator == b'(';
    let mut found = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if !is_ident_start(bytes[i]) || (i > 0 && is_ident_byte(bytes[i - 1])) {
            i += 1;
            continue;
        }
        let end = ident_end(bytes, i);
        let preceded_by_dot = i > 0 && bytes[i - 1] == b'.';
        if end < bytes.len() && bytes[end] == terminator && (!wants_dot || preceded_by_dot) {
            found.push((i, code[i..end].to_string()));
        }
        i = end.max(i + 1);
    }
    found
}

/// The `use` items in the text, as `(start, end)` byte ranges covering
/// `use` up to but not including its terminating `;`.
fn use_items(code: &str) -> Vec<(usize, usize)> {
    let bytes = code.as_bytes();
    let mut items = Vec::new();
    let mut i = 0;
    while i + 3 <= bytes.len() {
        let is_boundary = i == 0 || !is_ident_byte(bytes[i - 1]);
        if !(is_boundary && bytes[i..].starts_with(b"use") && !is_ident_byte_at(bytes, i + 3)) {
            i += 1;
            continue;
        }
        let mut depth = 0usize;
        let mut j = i + 3;
        while j < bytes.len() {
            match bytes[j] {
                b'{' => depth += 1,
                b'}' => depth = depth.saturating_sub(1),
                b';' if depth == 0 => break,
                _ => {}
            }
            j += 1;
        }
        items.push((i, j.min(bytes.len())));
        i = j;
    }
    items
}

fn is_ident_byte_at(bytes: &[u8], at: usize) -> bool {
    at < bytes.len() && is_ident_byte(bytes[at])
}

/// Every reach a `use` tree names, brace groups expanded.
fn expand_use(tree: &str) -> Vec<String> {
    let mut out = Vec::new();
    walk_use("", tree, &mut out);
    out
}

fn walk_use(prefix: &str, item: &str, out: &mut Vec<String>) {
    let item = item.trim().trim_start_matches("::").trim();
    if item.is_empty() {
        return;
    }
    if let Some(open) = item.find('{') {
        let Some(close) = matching_brace(item, open) else {
            return;
        };
        let head = item[..open].trim().trim_end_matches("::");
        let inner = &item[open + 1..close];
        let nested = join(prefix, head);
        for part in split_top_level(inner) {
            walk_use(&nested, part, out);
        }
        return;
    }
    let leaf = item
        .split(" as ")
        .next()
        .unwrap_or(item)
        .trim()
        .trim_end_matches('*')
        .trim_end_matches("::")
        .trim();
    let reach = if leaf.is_empty() || leaf == "self" {
        prefix.to_string()
    } else {
        join(prefix, leaf)
    };
    if !reach.is_empty() {
        out.push(reach);
    }
}

fn join(prefix: &str, leaf: &str) -> String {
    match (prefix.is_empty(), leaf.is_empty()) {
        (true, _) => leaf.to_string(),
        (false, true) => prefix.to_string(),
        (false, false) => format!("{prefix}::{leaf}"),
    }
}

fn matching_brace(text: &str, open: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut depth = 0usize;
    for (offset, byte) in bytes.iter().enumerate().skip(open) {
        match byte {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(offset);
                }
            }
            _ => {}
        }
    }
    None
}

fn split_top_level(inner: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0usize;
    let mut start = 0usize;
    for (offset, byte) in inner.bytes().enumerate() {
        match byte {
            b'{' => depth += 1,
            b'}' => depth = depth.saturating_sub(1),
            b',' if depth == 0 => {
                parts.push(&inner[start..offset]);
                start = offset + 1;
            }
            _ => {}
        }
    }
    parts.push(&inner[start..]);
    parts
}

/// Replace a range with spaces, keeping newlines so line numbers survive.
fn blank(code: &mut String, start: usize, end: usize) {
    let replacement: String = code[start..end]
        .chars()
        .map(|c| if c == '\n' { '\n' } else { ' ' })
        .collect();
    code.replace_range(start..end, &replacement);
}

/// The budget entry a reach breaks, if it breaks one.
fn judge(reach: &str) -> Option<&'static Budget> {
    BUDGET.iter().find(|budget| match budget.mode {
        Match::Exact => reach == budget.reach,
        Match::Prefix => {
            reach == budget.reach
                || (reach.len() > budget.reach.len()
                    && reach.starts_with(budget.reach)
                    && reach[budget.reach.len()..].starts_with("::"))
        }
    })
}

/// Record a finding, unless the same violation is already recorded. One reach
/// written twice in a file is one thing to fix.
fn push(findings: &mut Vec<Finding>, path: &str, what: &str, budget: &Budget, line: usize) {
    let finding = Finding {
        rule: Rule::EffectBudget,
        subject: path.to_string(),
        what: what.to_string(),
        why: budget.why.to_string(),
        line: Some(line),
    };
    if findings.iter().any(|seen| seen.key() == finding.key()) {
        return;
    }
    findings.push(finding);
}

/// Every `a::b::c` chain in the text, with the offset it starts at.
fn chains(code: &str) -> Vec<(usize, String)> {
    let bytes = code.as_bytes();
    let mut found = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if !is_ident_start(bytes[i]) || (i > 0 && is_ident_byte(bytes[i - 1])) {
            i += 1;
            continue;
        }
        let start = i;
        let mut end = i;
        let mut segments = 0;
        loop {
            let seg_end = ident_end(bytes, end);
            if seg_end == end {
                break;
            }
            segments += 1;
            end = seg_end;
            if bytes[end..].starts_with(b"::") && ident_end(bytes, end + 2) > end + 2 {
                end += 2;
            } else {
                break;
            }
        }
        if segments > 1 {
            found.push((start, code[start..end].to_string()));
        }
        i = end.max(start + 1);
    }
    found
}

fn ident_end(bytes: &[u8], from: usize) -> usize {
    if from >= bytes.len() || !is_ident_start(bytes[from]) {
        return from;
    }
    let mut end = from;
    while end < bytes.len() && is_ident_byte(bytes[end]) {
        end += 1;
    }
    end
}

fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_'
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn line_of(code: &str, offset: usize) -> usize {
    code[..offset].bytes().filter(|b| *b == b'\n').count() + 1
}

#[cfg(test)]
mod tests {
    use super::scan;

    // AYEAYE-41 — mutation: a deliberate violation must produce a finding.
    #[test]
    fn a_fully_qualified_filesystem_call_is_found_with_no_import_at_all() {
        let source = r#"
            pub fn load(name: &str) -> String {
                std::fs::read_to_string(name).unwrap()
            }
        "#;
        let findings = scan("planted.rs", source);
        assert_eq!(what(&findings), vec!["std::fs::read_to_string"]);
    }

    // AYEAYE-41 — a reach, not a spelling: two items in one brace group are
    // two reaches, and only one of them is outside the budget.
    #[test]
    fn a_brace_group_convicts_the_clock_and_acquits_the_duration() {
        let source = "use std::time::{Duration, Instant};\n";
        assert_eq!(
            what(&scan("planted.rs", source)),
            vec!["std::time::Instant"]
        );
    }

    // AYEAYE-41 — the same reach spelled two ways is the same reach.
    #[test]
    fn a_single_import_and_a_brace_group_of_one_are_the_same_reach() {
        let single = scan("a.rs", "use std::time::Instant;\n");
        let braced = scan("a.rs", "use std::time::{Instant};\n");
        assert_eq!(what(&single), what(&braced));
        assert_eq!(what(&single), vec!["std::time::Instant"]);
    }

    // AYEAYE-41 — the failure mode that made an earlier version of this rule
    // green and blind: a call that reaches a stream with nothing to import.
    #[test]
    fn a_prelude_macro_is_found_although_there_is_no_path_to_expand() {
        let source = "fn shout() { println!(\"hi\"); }\n";
        assert_eq!(what(&scan("planted.rs", source)), vec!["println!"]);
    }

    // AYEAYE-41
    #[test]
    fn an_effectful_method_on_a_receiver_that_arrived_as_an_argument_is_found() {
        let source = "fn slurp(f: &mut impl Anything) { f.read_to_string(&mut s); }\n";
        assert_eq!(what(&scan("planted.rs", source)), vec![".read_to_string()"]);
    }

    // AYEAYE-41 — the rule has to be able to describe itself. A doc comment
    // or a message naming a forbidden reach is prose, not a reach.
    #[test]
    fn prose_naming_a_forbidden_reach_is_not_a_reach() {
        let source = r####"
            /// Never use std::process::Command here; println! is out too.
            // std::fs::read_to_string is likewise not allowed
            fn explain() -> &'static str {
                "std::net::TcpStream is forbidden, and so is eprintln!"
            }
        "####;
        assert!(
            scan("prose.rs", source).is_empty(),
            "{:?}",
            scan("prose.rs", source)
        );
    }

    // AYEAYE-41 — a lifetime is not a character literal, and mistaking one for
    // the other swallows the rest of the file.
    #[test]
    fn a_lifetime_does_not_swallow_the_code_after_it() {
        let source = "fn f<'a>(x: &'a str) { std::fs::write(x); }\n";
        assert_eq!(what(&scan("planted.rs", source)), vec!["std::fs::write"]);
    }

    fn what(findings: &[crate::Finding]) -> Vec<&str> {
        findings.iter().map(|f| f.what.as_str()).collect()
    }
}
