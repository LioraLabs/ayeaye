//! File references and previews: the decisions, with no filesystem in them.
//!
//! A transcript mentions a file; the panel wants to show it. Everything that
//! *means* something in that trip lives here — what a reference is, which
//! tracked file it names, what kind of file that is, and how much of it a
//! preview may carry — as functions over text and bytes. Reading the tracked
//! list, opening the file and asking tmux where the pane is are the shell's
//! (`crates/ayeaye/src/files.rs`).
//!
//! The daemon this ports is `bin/ayeaye`'s `resolve_file_reference` and
//! `file_preview`, and the response shapes are `share/app.html`'s to read, so
//! none of them may drift.

use crate::json;
use crate::paths;

/// The most lines a text preview carries.
pub const MAX_PREVIEW_LINES: usize = 200;

/// The most bytes a text preview carries, across all its lines.
pub const MAX_TEXT_PREVIEW_BYTES: usize = 256 * 1024;

/// The most of a file that is read looking for a requested line. A distant
/// line number must not turn a preview into an unbounded file scan.
pub const MAX_TEXT_SCAN_BYTES: usize = 1024 * 1024;

/// The largest image the preview will serve. Bigger answers 413: the panel is
/// a phone, and an 8MiB ceiling is already generous for a screenshot.
pub const MAX_IMAGE_PREVIEW_BYTES: u64 = 8 * 1024 * 1024;

/// Extensions the preview renders as text, from the daemon's table.
const TEXT_EXTS: &[&str] = &[
    ".c", ".cc", ".cfg", ".conf", ".cpp", ".css", ".csv", ".go", ".h", ".hpp", ".html", ".ini",
    ".java", ".js", ".json", ".jsx", ".lua", ".md", ".py", ".rb", ".rs", ".sh", ".sql", ".toml",
    ".ts", ".tsx", ".txt", ".xml", ".yaml", ".yml",
];

/// Extensionless names that are text anyway.
const TEXT_NAMES: &[&str] = &["dockerfile", "gemfile", "license", "makefile", "readme"];

/// Extensions served as image bytes, with the type each is labelled as.
///
/// `.svg` is in the MIME table and not in the image set on purpose, matching
/// the daemon: an SVG is a document that can carry script, so it is its own
/// [`Kind`] and the shell serves it with a sandboxing `Content-Security-Policy`
/// the raster formats do not need.
const IMAGE_MIME: &[(&str, &str)] = &[
    (".gif", "image/gif"),
    (".jpeg", "image/jpeg"),
    (".jpg", "image/jpeg"),
    (".png", "image/png"),
    (".svg", "image/svg+xml"),
    (".webp", "image/webp"),
];

/// What a preview does with a file, decided from its name alone.
///
/// From the name rather than the bytes, deliberately: sniffing content is how
/// a "text" file turns out to be a terabyte of NULs, and the daemon decides
/// from the name too.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Served as its own bytes, labelled by extension.
    Image,
    /// An image that is also a document: served as bytes, but sandboxed.
    Svg,
    /// Served as bounded lines of text.
    Text,
    /// Not served: the answer is a stub saying so.
    Binary,
}

impl Kind {
    /// What kind of file this path names.
    pub fn of(path: &str) -> Kind {
        let name = paths::file_name(path).to_lowercase();
        let ext = extension(&name);
        if ext == ".svg" {
            return Kind::Svg;
        }
        if IMAGE_MIME.iter().any(|(image, _)| *image == ext) {
            return Kind::Image;
        }
        if TEXT_EXTS.contains(&ext) || TEXT_NAMES.contains(&name.as_str()) {
            return Kind::Text;
        }
        Kind::Binary
    }

    /// The word the JSON bodies carry for this kind.
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Image => "image",
            Kind::Svg => "svg",
            Kind::Text => "text",
            Kind::Binary => "binary",
        }
    }
}

/// The `Content-Type` an image or SVG preview is served with.
///
/// `None` for everything else, which no caller should reach: the shell asks
/// this only after [`Kind::of`] said the file was one of the two.
pub fn mime(path: &str) -> Option<&'static str> {
    let name = paths::file_name(path).to_lowercase();
    let ext = extension(&name);
    IMAGE_MIME
        .iter()
        .find(|(image, _)| *image == ext)
        .map(|(_, mime)| *mime)
}

/// The `.ext` of a lowercased file name, or `""`.
///
/// A leading dot is not an extension: `.bashrc` is a name, which is how the
/// daemon's `splitext` reads it too.
fn extension(name: &str) -> &str {
    match name.rfind('.') {
        Some(at) if at > 0 => &name[at..],
        _ => "",
    }
}

/// A transcript reference, split into the path it names and the line it
/// points at.
///
/// Markup comes off first — a whole-string markdown link keeps only its
/// target, surrounding backticks are trimmed — and then one trailing
/// `:<line>` is split, only if the line is a positive number with no leading
/// zero. `file.txt:0` is a file called `file.txt:0`, because that is the only
/// reading that never loses a real path.
pub fn reference(text: &str) -> (String, Option<usize>) {
    let mut text = text.trim();
    if let Some(target) = markdown_target(text) {
        text = target.trim();
    }
    if text.len() >= 2 && text.starts_with('`') && text.ends_with('`') {
        text = text[1..text.len() - 1].trim();
    }
    if let Some((path, line)) = text.rsplit_once(':')
        && is_line_number(line)
        && let Ok(number) = line.parse::<usize>()
    {
        return (path.to_string(), Some(number));
    }
    (text.to_string(), None)
}

/// The target of a reference that is one whole markdown link, `[label](target)`.
///
/// The label may not contain `]`, which is the daemon's `[^\]]*` — splitting
/// at the *first* `]` is that same rule, so a reference that is a link *plus*
/// anything else is left alone.
fn markdown_target(text: &str) -> Option<&str> {
    let rest = text.strip_prefix('[')?;
    let (_, rest) = rest.split_once(']')?;
    rest.strip_prefix('(')?.strip_suffix(')')
}

/// Whether this is the daemon's `[1-9][0-9]*`: digits, no leading zero.
fn is_line_number(text: &str) -> bool {
    let mut digits = text.chars();
    digits.next().is_some_and(|first| ('1'..='9').contains(&first))
        && digits.all(|digit| digit.is_ascii_digit())
}

/// A `line=` query parameter, validated the way the daemon validates it.
///
/// Empty is no line, which is fine; anything else must be a positive number of
/// at most ten digits, or the request is bad. Ten digits is the daemon's cap,
/// and it is what keeps a parse from being handed a numeral longer than any
/// file has lines.
pub fn line_param(text: &str) -> Result<Option<usize>, BadLine> {
    if text.is_empty() {
        return Ok(None);
    }
    if text.len() > 10 || !is_line_number(text) {
        return Err(BadLine);
    }
    text.parse().map(Some).map_err(|_| BadLine)
}

/// A `line=` that is not a positive number.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BadLine;

/// Whether a path may be previewed at all, before anything looks for it.
///
/// Relative, no NUL, and every component a real name: `.` and `..` are
/// refused, and so is the empty component a doubled or trailing separator
/// leaves. The descriptor walk would refuse most of these too — this exists so
/// a traversal attempt is a 400 rather than a 404, and so the walk only ever
/// sees paths of the shape it was written for.
pub fn preview_path_ok(path: &str) -> bool {
    !path.is_empty()
        && !path.contains('\0')
        && !path.starts_with('/')
        && path
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

/// The pane's directory relative to the repository root, as tracked paths are.
///
/// `""` for the root itself. A `cwd` outside `root` walks up — `..` becomes a
/// component, which [`paths::distance`] counts as a step, and that is the
/// right price for a pane working outside the repository it references.
pub fn relative(root: &str, cwd: &str) -> String {
    let root: Vec<&str> = root.split('/').filter(|part| !part.is_empty()).collect();
    let cwd: Vec<&str> = cwd.split('/').filter(|part| !part.is_empty()).collect();
    let shared = root
        .iter()
        .zip(cwd.iter())
        .take_while(|(here, there)| here == there)
        .count();
    let mut parts: Vec<&str> = vec![".."; root.len() - shared];
    parts.extend(&cwd[shared..]);
    parts.join("/")
}

/// The tracked files a reference could mean, best first.
///
/// Three ways to match, strongest first: the whole tracked path, a trailing
/// `dir/…` of it, or just the file's name. Ties are broken by how far the file
/// sits from the pane's own directory — [`paths::distance`], which is what
/// tells two files with one name apart: the agent meant the one beside it.
/// Then by shorter path, then by the path itself, so the answer is the same on
/// every machine holding the same repository.
///
/// The caller caps the list; this ranks all matches, because the cap belongs
/// with the stat that can drop candidates below it.
pub fn candidates<'a>(
    wanted: &str,
    pane_dir: &str,
    tracked: impl IntoIterator<Item = &'a str>,
) -> Vec<&'a str> {
    let mut wanted = wanted;
    while let Some(rest) = wanted.strip_prefix("./") {
        wanted = rest;
    }
    if wanted.is_empty() {
        return Vec::new();
    }
    let wanted_name = paths::file_name(wanted);
    let mut ranked: Vec<(u8, usize, usize, &str)> = tracked
        .into_iter()
        .filter_map(|tracked| {
            let rank = if tracked == wanted {
                0
            } else if wanted.contains('/') && suffixed(tracked, wanted) {
                1
            } else if paths::file_name(tracked) == wanted_name {
                2
            } else {
                return None;
            };
            let directory = tracked.rsplit_once('/').map_or("", |(dir, _)| dir);
            Some((
                rank,
                paths::distance(pane_dir, directory),
                tracked.len(),
                tracked,
            ))
        })
        .collect();
    ranked.sort();
    ranked.into_iter().map(|(_, _, _, path)| path).collect()
}

/// Whether `tracked` ends in `/wanted` — the same file, named from deeper in.
fn suffixed(tracked: &str, wanted: &str) -> bool {
    tracked
        .strip_suffix(wanted)
        .is_some_and(|rest| rest.ends_with('/'))
}

/// One line of a text preview.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// One-based, as editors and compilers number.
    pub number: usize,
    /// The line's text, terminator included — that is what the daemon sends,
    /// and `share/app.html` renders it inside a `<pre>` where it is layout.
    pub text: String,
}

/// The bounded excerpt a text preview carries.
///
/// `bytes` is the first [`MAX_TEXT_SCAN_BYTES`] of the file at most, and
/// `file_len` is how long the file really is — the difference is how this
/// knows a scan was cut short rather than complete.
///
/// With no requested line: the beginning, to [`MAX_PREVIEW_LINES`] lines and
/// [`MAX_TEXT_PREVIEW_BYTES`] bytes. With one: a window that runs to half the
/// line budget past the requested line, keeping at most the full budget before
/// it — so the line sits about two thirds of the way down what is shown, with
/// the context that led to it above. A line beyond the scan is answered with
/// the bounded beginning rather than a longer read; a line beyond the end of a
/// fully-scanned file is answered with its tail, which is as near as the file
/// can offer.
pub fn excerpt(bytes: &[u8], file_len: u64, requested: Option<usize>) -> Vec<Row> {
    let cut_short = file_len > bytes.len() as u64;
    let capped = capped_lines(bytes);
    let raw = match requested {
        None => head(capped),
        Some(line) => window(capped, line, cut_short),
    };
    decoded(raw)
}

/// Every line of the scan, numbered, each capped to the whole preview budget.
fn capped_lines(bytes: &[u8]) -> Vec<(usize, &[u8])> {
    let mut lines = Vec::new();
    let mut rest = bytes;
    let mut number = 0;
    while !rest.is_empty() {
        number += 1;
        let line = match rest.iter().position(|byte| *byte == b'\n') {
            Some(at) => {
                let (line, after) = rest.split_at(at + 1);
                rest = after;
                line
            }
            None => std::mem::take(&mut rest),
        };
        lines.push((number, &line[..line.len().min(MAX_TEXT_PREVIEW_BYTES)]));
    }
    lines
}

/// The bounded beginning: the first lines that fit both budgets.
fn head(lines: Vec<(usize, &[u8])>) -> Vec<(usize, &[u8])> {
    let mut kept = Vec::new();
    let mut budget = MAX_TEXT_PREVIEW_BYTES;
    for (number, line) in lines.into_iter().take(MAX_PREVIEW_LINES) {
        if budget == 0 {
            break;
        }
        let take = line.len().min(budget);
        kept.push((number, &line[..take]));
        budget -= take;
    }
    kept
}

/// The window around one requested line, or the fallback beginning.
fn window(lines: Vec<(usize, &[u8])>, requested: usize, cut_short: bool) -> Vec<(usize, &[u8])> {
    let scanned = lines.last().map_or(0, |(number, _)| *number);
    if cut_short && scanned < requested {
        // The line is further in than the scan goes. The stable answer is the
        // bounded beginning, not a bigger read.
        return head(lines);
    }
    // To half the line budget past the requested line, so the line shown has
    // the context that led to it above and some of what follows below.
    let end = requested + (MAX_PREVIEW_LINES / 2 - 1);
    let mut kept: std::collections::VecDeque<(usize, &[u8])> = std::collections::VecDeque::new();
    let mut retained = 0;
    for (number, line) in lines {
        if number > end {
            break;
        }
        kept.push_back((number, line));
        retained += line.len();
        // Older context pays for newer: the requested line must fit, whatever
        // sat above it.
        while kept.len() > MAX_PREVIEW_LINES || retained > MAX_TEXT_PREVIEW_BYTES {
            let (_, dropped) = kept.pop_front().expect("retained lines exist to drop");
            retained -= dropped.len();
        }
    }
    kept.into_iter().collect()
}

/// Raw rows as the rows a body carries: lossily decoded, still inside the
/// byte budget after decoding.
///
/// The re-check is not paranoia — a byte that would not decode becomes a
/// three-byte replacement character, so a window that fit in raw bytes can
/// outgrow the budget in UTF-8.
fn decoded(raw: Vec<(usize, &[u8])>) -> Vec<Row> {
    let mut rows = Vec::new();
    let mut budget = MAX_TEXT_PREVIEW_BYTES;
    for (number, line) in raw {
        if budget == 0 {
            break;
        }
        let mut text = String::from_utf8_lossy(line).into_owned();
        if text.len() > budget {
            let mut cut = budget;
            while !text.is_char_boundary(cut) {
                cut -= 1;
            }
            text.truncate(cut);
        }
        budget -= text.len();
        rows.push(Row { number, text });
    }
    rows
}

/// One resolved candidate, as the resolve body carries it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    /// The tracked path, exactly as `git ls-files` prints it.
    pub path: String,
    /// How big the file is on disk right now.
    pub size: u64,
}

/// The body `/api/files/resolve` answers with.
///
/// `line` rides along even when there are no candidates: the page keeps it to
/// pass back to the preview, and the daemon sends it the same way.
pub fn resolve_body(candidates: &[Candidate], line: Option<usize>) -> String {
    let mut body = String::from("{\"candidates\":[");
    for (index, candidate) in candidates.iter().enumerate() {
        if index > 0 {
            body.push(',');
        }
        body.push_str(&format!(
            "{{\"path\":{},\"kind\":{},\"size\":{}}}",
            json::string(&candidate.path),
            json::string(Kind::of(&candidate.path).as_str()),
            candidate.size,
        ));
    }
    body.push_str("],\"line\":");
    match line {
        Some(line) => body.push_str(&line.to_string()),
        None => body.push_str("null"),
    }
    body.push('}');
    body
}

/// The body a text preview answers with.
pub fn text_body(path: &str, size: u64, rows: &[Row], line: Option<usize>) -> String {
    let start = rows.first().map_or(1, |row| row.number);
    let mut body = format!(
        "{{\"path\":{},\"size\":{size},\"kind\":\"text\",\"start\":{start},\"lines\":[",
        json::string(path),
    );
    for (index, row) in rows.iter().enumerate() {
        if index > 0 {
            body.push(',');
        }
        body.push_str(&format!(
            "{{\"number\":{},\"text\":{}}}",
            row.number,
            json::string(&row.text),
        ));
    }
    body.push(']');
    if let Some(line) = line {
        body.push_str(&format!(",\"line\":{line}"));
    }
    body.push('}');
    body
}

/// The body a binary file answers with: the fact of it, and no bytes.
pub fn binary_body(path: &str, size: u64) -> String {
    format!(
        "{{\"path\":{},\"size\":{size},\"kind\":\"binary\",\"preview\":\"unavailable\"}}",
        json::string(path),
    )
}

#[cfg(test)]
mod tests {
    use super::{
        BadLine, Candidate, Kind, MAX_PREVIEW_LINES, MAX_TEXT_PREVIEW_BYTES, MAX_TEXT_SCAN_BYTES,
        Row, binary_body, candidates, excerpt, line_param, mime, preview_path_ok, reference,
        relative, resolve_body, text_body,
    };

    // AYEAYE-52 — a reference resolves to a path and an optional line. The
    // colon split takes only a trailing positive number: a Windows-ish or
    // odd path with colons in it stays a path.
    #[test]
    fn a_reference_is_a_path_and_maybe_a_line() {
        assert_eq!(reference("src/main.rs:42"), ("src/main.rs".into(), Some(42)));
        assert_eq!(reference("src/main.rs"), ("src/main.rs".into(), None));
        assert_eq!(reference("  src/main.rs:7  "), ("src/main.rs".into(), Some(7)));
        // No leading zero and no zero: those are names, not line numbers.
        assert_eq!(reference("a.txt:0"), ("a.txt:0".into(), None));
        assert_eq!(reference("a.txt:007"), ("a.txt:007".into(), None));
        assert_eq!(reference("a.txt:12x"), ("a.txt:12x".into(), None));
        // The *last* colon splits, so a stranger path keeps its earlier ones.
        assert_eq!(reference("a:b/c.txt:3"), ("a:b/c.txt".into(), Some(3)));
    }

    // AYEAYE-52 — transcript markup comes off: a markdown link keeps its
    // target, backticks are trimmed, and both can happen to one reference.
    #[test]
    fn transcript_markup_is_stripped() {
        assert_eq!(
            reference("[the config](src/config.rs)"),
            ("src/config.rs".into(), None)
        );
        assert_eq!(reference("`src/main.rs:9`"), ("src/main.rs".into(), Some(9)));
        assert_eq!(
            reference("[see](`src/lib.rs:2`)"),
            ("src/lib.rs".into(), Some(2))
        );
        // A link with a `]` in the label is not the daemon's link shape, and a
        // sentence mentioning brackets is not a link at all.
        assert_eq!(
            reference("[a][b](c.txt)"),
            ("[a][b](c.txt)".into(), None)
        );
        // One lone backtick is text, not a wrapper.
        assert_eq!(reference("`a.txt"), ("`a.txt".into(), None));
    }

    // AYEAYE-52 — file-kind detection is pure and from the name alone. The
    // table is the daemon's: images by extension, svg its own kind because it
    // is served sandboxed, known text extensions and names, everything else
    // binary rather than guessed at.
    #[test]
    fn a_files_kind_comes_from_its_name() {
        assert_eq!(Kind::of("shot.png"), Kind::Image);
        assert_eq!(Kind::of("a/b/photo.JPG"), Kind::Image);
        assert_eq!(Kind::of("diagram.svg"), Kind::Svg);
        assert_eq!(Kind::of("src/main.rs"), Kind::Text);
        assert_eq!(Kind::of("Makefile"), Kind::Text);
        assert_eq!(Kind::of("README"), Kind::Text);
        assert_eq!(Kind::of("target/release/ayeaye"), Kind::Binary);
        assert_eq!(Kind::of("archive.tar.gz"), Kind::Binary);
        // A leading dot is a name, not an extension.
        assert_eq!(Kind::of(".bashrc"), Kind::Binary);
    }

    // AYEAYE-52 — the label an image is served under. `.jpg` is `image/jpg`
    // because that is what the daemon sends, and the body shape is the page's.
    #[test]
    fn an_image_is_labelled_by_its_extension() {
        assert_eq!(mime("shot.png"), Some("image/png"));
        assert_eq!(mime("shot.jpeg"), Some("image/jpeg"));
        assert_eq!(mime("shot.jpg"), Some("image/jpeg"));
        assert_eq!(mime("art.svg"), Some("image/svg+xml"));
        assert_eq!(mime("main.rs"), None);
    }

    // AYEAYE-52 — path scoring: an exact tracked path beats a suffix match
    // beats a bare-name match, whatever order the tracked list arrived in.
    #[test]
    fn an_exact_path_beats_a_suffix_beats_a_name() {
        let tracked = ["docs/src/main.rs", "src/main.rs", "main.rs", "lib.rs"];
        assert_eq!(
            candidates("src/main.rs", "", tracked),
            vec!["src/main.rs", "docs/src/main.rs", "main.rs"],
        );
    }

    // AYEAYE-52 — the spec's scoring claim: two files with one name are told
    // apart by which sits nearer the session's working directory.
    #[test]
    fn the_file_beside_the_pane_wins() {
        let tracked = ["deep/away/notes.md", "crates/core/notes.md"];
        assert_eq!(
            candidates("notes.md", "crates/core", tracked),
            vec!["crates/core/notes.md", "deep/away/notes.md"],
        );
        // From the other side of the tree the other file is the near one.
        assert_eq!(
            candidates("notes.md", "deep/away", tracked),
            vec!["deep/away/notes.md", "crates/core/notes.md"],
        );
    }

    // AYEAYE-52 — at equal rank and equal distance, the shorter path and then
    // the path itself decide, so two machines with one repository agree.
    #[test]
    fn ties_break_by_length_then_path() {
        let tracked = ["b/x/a.txt", "a/x/a.txt", "aa/xx/a.txt"];
        assert_eq!(
            candidates("a.txt", "", tracked),
            vec!["a/x/a.txt", "b/x/a.txt", "aa/xx/a.txt"],
        );
    }

    // AYEAYE-52 — a `./` prefix names the same file, and a name-only wanted
    // never matches by suffix: `main.rs` must not suffix-match `domain.rs`.
    #[test]
    fn wanted_paths_are_matched_as_components() {
        assert_eq!(
            candidates("./src/main.rs", "", ["src/main.rs"]),
            vec!["src/main.rs"],
        );
        assert_eq!(candidates("main.rs", "", ["src/domain.rs"]), Vec::<&str>::new());
        // And a suffix match is whole components: `b/c.txt` is not `ab/c.txt`.
        assert_eq!(candidates("b/c.txt", "", ["ab/c.txt"]), Vec::<&str>::new());
        assert_eq!(candidates("b/c.txt", "", ["a/b/c.txt"]), vec!["a/b/c.txt"]);
        assert_eq!(candidates("", "", ["a.txt"]), Vec::<&str>::new());
    }

    // AYEAYE-52 — the pane's place in the repository, as tracked paths spell
    // it: relative, and `""` at the root.
    #[test]
    fn the_panes_directory_is_repo_relative() {
        assert_eq!(relative("/home/a/repo", "/home/a/repo"), "");
        assert_eq!(relative("/home/a/repo", "/home/a/repo/src/http"), "src/http");
        // Outside the root is a walk up, which distance prices as steps.
        assert_eq!(relative("/home/a/repo", "/home/a/other"), "../other");
    }

    // AYEAYE-52 — line bounding with no line asked for: the beginning, capped
    // in lines. Terminators ride along, as the daemon sends them.
    #[test]
    fn no_requested_line_previews_the_bounded_beginning() {
        let text: String = (1..=300).map(|n| format!("line {n}\n")).collect();
        let rows = excerpt(text.as_bytes(), text.len() as u64, None);
        assert_eq!(rows.len(), MAX_PREVIEW_LINES);
        assert_eq!(rows[0], Row { number: 1, text: "line 1\n".into() });
        assert_eq!(rows[199].number, 200);
    }

    // AYEAYE-52 — the acceptance criterion: bounded context *around* the
    // requested line. The window runs half the budget past it, so the line is
    // inside what is shown with its lead-up above it.
    #[test]
    fn a_requested_line_sits_inside_its_window() {
        let text: String = (1..=1000).map(|n| format!("line {n}\n")).collect();
        let rows = excerpt(text.as_bytes(), text.len() as u64, Some(500));
        assert_eq!(rows.len(), MAX_PREVIEW_LINES);
        // Ends 99 past the request, starts 100 before it: 400..=599.
        assert_eq!(rows.first().map(|row| row.number), Some(400));
        assert_eq!(rows.last().map(|row| row.number), Some(599));
        assert!(rows.iter().any(|row| row.number == 500));
        assert_eq!(rows[100], Row { number: 500, text: "line 500\n".into() });
    }

    // AYEAYE-52 — a line near the top does not push the window below it: the
    // excerpt starts at the file's start and still reaches past the line.
    #[test]
    fn an_early_line_keeps_the_files_start() {
        let text: String = (1..=1000).map(|n| format!("line {n}\n")).collect();
        let rows = excerpt(text.as_bytes(), text.len() as u64, Some(3));
        assert_eq!(rows.first().map(|row| row.number), Some(1));
        // To half the budget past line 3, and no further.
        assert_eq!(rows.last().map(|row| row.number), Some(3 + 99));
    }

    // AYEAYE-52 — a line beyond a fully-scanned file answers with the tail:
    // the nearest context the file can offer, not an error and not the top.
    #[test]
    fn a_line_past_the_end_answers_with_the_tail() {
        let text: String = (1..=50).map(|n| format!("line {n}\n")).collect();
        let rows = excerpt(text.as_bytes(), text.len() as u64, Some(4000));
        assert_eq!(rows.first().map(|row| row.number), Some(1));
        assert_eq!(rows.last().map(|row| row.number), Some(50));
    }

    // AYEAYE-52 — a line beyond the scan cap must not turn the preview into
    // an unbounded read: the answer is the bounded beginning.
    #[test]
    fn a_line_beyond_the_scan_falls_back_to_the_beginning() {
        // A scan-sized prefix of a "file" claiming to be much longer.
        let line = "x".repeat(99).to_string() + "\n";
        let scanned: String = line.repeat(MAX_TEXT_SCAN_BYTES / 100);
        let rows = excerpt(
            scanned.as_bytes(),
            (MAX_TEXT_SCAN_BYTES as u64) * 4,
            Some(4_000_000),
        );
        assert_eq!(rows.first().map(|row| row.number), Some(1));
        assert_eq!(rows.len(), MAX_PREVIEW_LINES);
    }

    // AYEAYE-52 — byte bounding: one enormous line cannot carry more than the
    // whole preview budget, and the budget is cumulative across lines.
    #[test]
    fn the_byte_budget_holds_however_long_the_lines_are() {
        let huge = vec![b'a'; MAX_TEXT_PREVIEW_BYTES * 3];
        let rows = excerpt(&huge, huge.len() as u64, None);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].text.len(), MAX_TEXT_PREVIEW_BYTES);

        let text: String = (1..=100).map(|_| "y".repeat(8191).to_string() + "\n").collect();
        let rows = excerpt(text.as_bytes(), text.len() as u64, None);
        let total: usize = rows.iter().map(|row| row.text.len()).sum();
        assert!(total <= MAX_TEXT_PREVIEW_BYTES, "{total} bytes kept");
        assert!(rows.len() == MAX_TEXT_PREVIEW_BYTES / 8192, "{} rows", rows.len());
    }

    // AYEAYE-52 — bytes that are not UTF-8 become replacement characters, and
    // the budget still holds after they grow in the decoding.
    #[test]
    fn undecodable_bytes_are_replaced_within_budget() {
        let mut bytes = vec![0xFF_u8; MAX_TEXT_PREVIEW_BYTES];
        bytes.push(b'\n');
        let rows = excerpt(&bytes, bytes.len() as u64, None);
        assert_eq!(rows.len(), 1);
        assert!(rows[0].text.len() <= MAX_TEXT_PREVIEW_BYTES);
        assert!(rows[0].text.starts_with('\u{FFFD}'));
    }

    // AYEAYE-52 — the preview path gate: relative, clean components, nothing
    // a walk should ever be handed.
    #[test]
    fn only_clean_relative_paths_may_be_previewed() {
        for good in ["a.txt", "src/main.rs", "a/b/c.d"] {
            assert!(preview_path_ok(good), "{good} should pass");
        }
        for bad in [
            "",
            "/etc/passwd",
            "../secrets",
            "a/../b",
            "./a.txt",
            "a//b",
            "a/b/",
            "a\0b",
        ] {
            assert!(!preview_path_ok(bad), "{bad:?} should be refused");
        }
    }

    // AYEAYE-52 — the line parameter: empty is no line, and everything else
    // is the daemon's ten-digit positive number or a bad request.
    #[test]
    fn the_line_parameter_is_a_bounded_positive_number() {
        assert_eq!(line_param(""), Ok(None));
        assert_eq!(line_param("1"), Ok(Some(1)));
        assert_eq!(line_param("1234567890"), Ok(Some(1_234_567_890)));
        for bad in ["0", "007", "-1", "12x", "12345678901", " 1"] {
            assert_eq!(line_param(bad), Err(BadLine), "{bad:?}");
        }
    }

    // AYEAYE-52 — the bodies are the page's to read: candidates with path,
    // kind and size; null for no line; text rows with start; the binary stub.
    #[test]
    fn the_bodies_carry_the_shapes_the_page_reads() {
        let found = [
            Candidate { path: "src/main.rs".into(), size: 120 },
            Candidate { path: "shot.png".into(), size: 4096 },
        ];
        assert_eq!(
            resolve_body(&found, Some(7)),
            r#"{"candidates":[{"path":"src/main.rs","kind":"text","size":120},{"path":"shot.png","kind":"image","size":4096}],"line":7}"#,
        );
        assert_eq!(resolve_body(&[], None), r#"{"candidates":[],"line":null}"#);

        let rows = [
            Row { number: 41, text: "fn main() {\n".into() },
            Row { number: 42, text: "}\n".into() },
        ];
        assert_eq!(
            text_body("src/main.rs", 999, &rows, Some(42)),
            r#"{"path":"src/main.rs","size":999,"kind":"text","start":41,"lines":[{"number":41,"text":"fn main() {\n"},{"number":42,"text":"}\n"}],"line":42}"#,
        );
        assert_eq!(
            text_body("empty.txt", 0, &[], None),
            r#"{"path":"empty.txt","size":0,"kind":"text","start":1,"lines":[]}"#,
        );
        assert_eq!(
            binary_body("a.bin", 5),
            r#"{"path":"a.bin","size":5,"kind":"binary","preview":"unavailable"}"#,
        );
    }
}
