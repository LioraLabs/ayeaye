//! Rules 5 and 6: no decision is implemented twice.
//!
//! Every duplication found so far was once labelled a deliberate copy by
//! someone with a good reason that later expired. These rules do not ban
//! having had the reason — that is what the waiver file is for — they ban
//! acquiring a new one silently.
//!
//! **Scope: shipped code.** Each crate's `src/` minus its `#[cfg(test)]`
//! items, plus `build.rs`. Tests are deliberately out: a non-tautological
//! test *restates* the value it asserts on — an integration test in `ayeaye`
//! spelling out a route that `ayeaye-core` owns is the independence that
//! makes the test worth having, not a second implementation of the decision.
//! The `constitution` crate is out too: it is stratum-3 tooling that quotes
//! the workspace by design.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use crate::corpus::Corpus;
use crate::finding::{Finding, Rule};
use crate::strip;

/// How long a string literal has to be before two crates writing it is a
/// shared decision rather than a coincidence. Below this, short words and
/// format fragments dominate.
pub const LITERAL_FLOOR: usize = 8;

/// How many significant lines make a run of code. Trimmed, comment-stripped,
/// blank and punctuation-only lines dropped — six such lines agreeing across
/// two crates is a copy, not a coincidence.
pub const RUN_LINES: usize = 6;

/// The members the duplication rules do not compare. The constitution quotes
/// the workspace — planted violations, budget tables, this very file — so
/// judging it would convict the judge for reading the evidence aloud.
pub const EXEMPT: &[&str] = &["constitution"];

/// One file of shipped code, named by the crate that owns it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrateSource {
    /// The workspace member the file belongs to.
    pub crate_name: String,
    /// Path relative to the workspace root, which is what a finding names.
    pub path: String,
    /// The file's text, as read — the rules do their own stripping.
    pub text: String,
}

/// The corpus reduced to what the duplication rules judge: every non-exempt
/// member's `src/` and `build.rs`. Test directories never enter.
pub fn scope(corpus: &Corpus) -> Vec<CrateSource> {
    let mut sources = Vec::new();
    for member in &corpus.members {
        if EXEMPT.contains(&member.name.as_str()) {
            continue;
        }
        let src_prefix = format!("{}/src/", member.dir);
        let build_script = format!("{}/build.rs", member.dir);
        for source in &member.sources {
            if source.path.starts_with(&src_prefix) || source.path == build_script {
                sources.push(CrateSource {
                    crate_name: member.name.clone(),
                    path: source.path.clone(),
                    text: source.text.clone(),
                });
            }
        }
    }
    sources
}

/// Rule 5: a string literal written in more than one crate.
///
/// One finding per literal, keyed on the literal and the set of crates that
/// write it — never on a line number, so an edit above a known violation does
/// not re-fire it. A literal spreading to a *new* crate changes the key,
/// which staleness then surfaces: that is the ratchet noticing, not churn.
pub fn literals(sources: &[CrateSource], floor: usize) -> Vec<Finding> {
    // literal text -> crate -> the files that write it, all ordered so the
    // findings come out deterministic.
    let mut written: BTreeMap<String, BTreeMap<String, BTreeSet<String>>> = BTreeMap::new();
    for source in sources {
        let shipped = strip::without_test_blocks(&source.text);
        for literal in strip::string_literals(&shipped) {
            if literal.text.chars().count() < floor {
                continue;
            }
            written
                .entry(literal.text)
                .or_default()
                .entry(source.crate_name.clone())
                .or_default()
                .insert(source.path.clone());
        }
    }

    written
        .into_iter()
        .filter(|(_, crates)| crates.len() > 1)
        .map(|(literal, crates)| {
            let names: Vec<&str> = crates.keys().map(String::as_str).collect();
            let files: Vec<&str> = crates
                .values()
                .flat_map(|paths| paths.iter().map(String::as_str))
                .collect();
            Finding {
                rule: Rule::DuplicateLiteral,
                subject: format!("{literal:?}"),
                what: names.join("+"),
                why: format!(
                    "written in {} — one crate should own this decision; move it down or waive it",
                    files.join(", ")
                ),
                line: None,
            }
        })
        .collect()
}

/// Rule 6: a run of code copied across crates.
///
/// Files are reduced to their significant lines (comments and `#[cfg(test)]`
/// items stripped, lines trimmed, blank and punctuation-only lines dropped —
/// string contents kept, because two runs that differ only inside their
/// literals are two different decisions). A window of `window` such lines
/// appearing in two crates seeds a match, which is then extended to the
/// maximal shared run and reported once, keyed on a hash of the run's text.
pub fn runs(sources: &[CrateSource], window: usize) -> Vec<Finding> {
    assert!(window > 0, "a zero-line run would match everything");
    let files: Vec<NormalizedFile> = sources.iter().map(NormalizedFile::new).collect();

    // Seed index: window hash -> every (file, window start) it occurs at.
    let mut seeds: HashMap<u64, Vec<(usize, usize)>> = HashMap::new();
    for (file_index, file) in files.iter().enumerate() {
        if file.lines.len() < window {
            continue;
        }
        for start in 0..=(file.lines.len() - window) {
            let hash = fnv64_lines(&file.lines[start..start + window]);
            seeds.entry(hash).or_default().push((file_index, start));
        }
    }

    // Extend every cross-crate seed pair to its maximal run. `covered` keeps
    // one run from being rediscovered once per window it contains.
    let mut covered: HashSet<(usize, usize, usize, usize)> = HashSet::new();
    let mut found: BTreeMap<String, BTreeSet<(String, String, usize)>> = BTreeMap::new();
    let mut hashes: Vec<&u64> = seeds.keys().collect();
    hashes.sort();
    for hash in hashes {
        let occurrences = &seeds[hash];
        for (a, &(file_a, start_a)) in occurrences.iter().enumerate() {
            for &(file_b, start_b) in &occurrences[a + 1..] {
                if files[file_a].crate_name == files[file_b].crate_name {
                    continue;
                }
                if covered.contains(&(file_a, start_a, file_b, start_b)) {
                    continue;
                }
                // A hash agreeing is a seed, not a proof: confirm the lines
                // really match before reporting, so a 64-bit collision cannot
                // convict two crates of a copy neither of them made.
                if files[file_a].lines[start_a..start_a + window]
                    .iter()
                    .zip(&files[file_b].lines[start_b..start_b + window])
                    .any(|(line_a, line_b)| line_a.text != line_b.text)
                {
                    continue;
                }
                let (run_a, run_b, length) =
                    extend(&files[file_a], start_a, &files[file_b], start_b, window);
                for offset in 0..=(length - window) {
                    covered.insert((file_a, run_a + offset, file_b, run_b + offset));
                }
                let text = files[file_a].lines[run_a..run_a + length]
                    .iter()
                    .map(|line| line.text.as_str())
                    .collect::<Vec<_>>()
                    .join("\n");
                let entry = found.entry(text).or_default();
                entry.insert(location(&files[file_a], run_a));
                entry.insert(location(&files[file_b], run_b));
            }
        }
    }

    found
        .into_iter()
        .map(|(text, locations)| {
            let crates: BTreeSet<&str> =
                locations.iter().map(|(name, _, _)| name.as_str()).collect();
            let names: Vec<&str> = crates.into_iter().collect();
            let places: Vec<String> = locations
                .iter()
                .map(|(_, path, line)| format!("{path}:{line}"))
                .collect();
            let lines = text.lines().count();
            let first = text.lines().next().unwrap_or_default();
            Finding {
                rule: Rule::DuplicateCode,
                subject: format!("{:016x}", fnv64(&text)),
                what: names.join("+"),
                why: format!(
                    "{lines} lines starting {first:?} copied at {} — one crate should own this \
                     code; move it down or waive it",
                    places.join(", ")
                ),
                line: None,
            }
        })
        .collect()
}

/// One file reduced to the lines the run rule compares.
struct NormalizedFile {
    crate_name: String,
    path: String,
    lines: Vec<NormalizedLine>,
}

/// One significant line: its trimmed text and where it was.
struct NormalizedLine {
    text: String,
    original_line: usize,
}

impl NormalizedFile {
    fn new(source: &CrateSource) -> Self {
        let shipped = strip::without_comments(&strip::without_test_blocks(&source.text));
        let lines = shipped
            .lines()
            .enumerate()
            .filter_map(|(index, line)| {
                let trimmed = line.trim();
                trimmed.chars().any(char::is_alphanumeric).then(|| {
                    NormalizedLine {
                        text: trimmed.to_string(),
                        original_line: index + 1,
                    }
                })
            })
            .collect();
        NormalizedFile {
            crate_name: source.crate_name.clone(),
            path: source.path.clone(),
            lines,
        }
    }
}

/// Grow a seeded match in both directions while the lines agree. Returns the
/// two run starts and the shared length.
fn extend(
    a: &NormalizedFile,
    seed_a: usize,
    b: &NormalizedFile,
    seed_b: usize,
    window: usize,
) -> (usize, usize, usize) {
    let mut start_a = seed_a;
    let mut start_b = seed_b;
    while start_a > 0 && start_b > 0 && a.lines[start_a - 1].text == b.lines[start_b - 1].text {
        start_a -= 1;
        start_b -= 1;
    }
    let mut length = window + (seed_a - start_a);
    while start_a + length < a.lines.len()
        && start_b + length < b.lines.len()
        && a.lines[start_a + length].text == b.lines[start_b + length].text
    {
        length += 1;
    }
    (start_a, start_b, length)
}

/// Where a run begins, for the finding's message.
fn location(file: &NormalizedFile, start: usize) -> (String, String, usize) {
    (
        file.crate_name.clone(),
        file.path.clone(),
        file.lines[start].original_line,
    )
}

/// FNV-1a, 64-bit. A run's identity is its text, not its address: the hash
/// moves only when the run itself changes, never when code above it does.
fn fnv64(text: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// The hash of a window of normalized lines, joined the same way the run text
/// is, so a window hash and a run hash of the same lines agree.
fn fnv64_lines(lines: &[NormalizedLine]) -> u64 {
    let text = lines
        .iter()
        .map(|line| line.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    fnv64(&text)
}

#[cfg(test)]
mod tests {
    use super::{CrateSource, literals, runs};

    fn source(crate_name: &str, path: &str, text: &str) -> CrateSource {
        CrateSource {
            crate_name: crate_name.to_string(),
            path: path.to_string(),
            text: text.to_string(),
        }
    }

    fn keys(findings: &[crate::Finding]) -> Vec<String> {
        findings.iter().map(|f| f.key()).collect()
    }

    // AYEAYE-64 — mutation: the planted violation the rule exists for. One
    // literal, two crates, one finding.
    #[test]
    fn a_literal_written_in_two_crates_is_found() {
        let findings = literals(
            &[
                source("low", "crates/low/src/lib.rs", "const A: &str = \"the decision\";"),
                source("high", "crates/high/src/lib.rs", "let a = \"the decision\";"),
            ],
            8,
        );
        assert_eq!(
            keys(&findings),
            vec!["duplicate-literal/\"the decision\"/high+low"]
        );
        let why = &findings[0].why;
        assert!(why.contains("crates/low/src/lib.rs"), "{why}");
        assert!(why.contains("crates/high/src/lib.rs"), "{why}");
    }

    // AYEAYE-64 — the rule is about crates, not files: a literal repeated
    // inside one crate is that crate's own business.
    #[test]
    fn the_same_literal_twice_in_one_crate_is_clean() {
        let findings = literals(
            &[
                source("low", "crates/low/src/a.rs", "let a = \"the decision\";"),
                source("low", "crates/low/src/b.rs", "let b = \"the decision\";"),
            ],
            8,
        );
        assert!(findings.is_empty(), "{findings:?}");
    }

    // AYEAYE-64 — below the floor, coincidence dominates decision.
    #[test]
    fn a_literal_below_the_floor_is_clean() {
        let findings = literals(
            &[
                source("low", "crates/low/src/lib.rs", "let a = \"decide\";"),
                source("high", "crates/high/src/lib.rs", "let a = \"decide\";"),
            ],
            8,
        );
        assert!(findings.is_empty(), "{findings:?}");
    }

    // AYEAYE-64 — prose is not a literal.
    #[test]
    fn a_literal_quoted_in_a_comment_is_clean() {
        let findings = literals(
            &[
                source("low", "crates/low/src/lib.rs", "let a = \"the decision\";"),
                source("high", "crates/high/src/lib.rs", "// says \"the decision\"\n"),
            ],
            8,
        );
        assert!(findings.is_empty(), "{findings:?}");
    }

    // AYEAYE-64 — a test restating another crate's literal is the
    // independence that makes the test worth having.
    #[test]
    fn a_literal_in_a_cfg_test_module_is_clean() {
        let findings = literals(
            &[
                source("low", "crates/low/src/lib.rs", "let a = \"the decision\";"),
                source(
                    "high",
                    "crates/high/src/lib.rs",
                    "#[cfg(test)]\nmod tests {\n    const A: &str = \"the decision\";\n}\n",
                ),
            ],
            8,
        );
        assert!(findings.is_empty(), "{findings:?}");
    }

    // AYEAYE-64 — the key must not move when the line does: an unrelated
    // edit above a known violation is not a new violation.
    #[test]
    fn a_literal_key_survives_an_edit_above_it() {
        let before = literals(
            &[
                source("low", "crates/low/src/lib.rs", "let a = \"the decision\";"),
                source("high", "crates/high/src/lib.rs", "let a = \"the decision\";"),
            ],
            8,
        );
        let after = literals(
            &[
                source(
                    "low",
                    "crates/low/src/lib.rs",
                    "fn unrelated() {}\nfn also() {}\nlet a = \"the decision\";",
                ),
                source("high", "crates/high/src/lib.rs", "let a = \"the decision\";"),
            ],
            8,
        );
        assert_eq!(keys(&before), keys(&after));
    }

    /// Six significant lines that read like code, with a knob to vary one.
    fn run_of_code(marker: &str) -> String {
        format!(
            "fn handle() {{\n    let a = read({marker:?});\n    let b = parse(a);\n    \
             let c = rank(b);\n    let d = emit(c);\n    finish(d);\n}}\n"
        )
    }

    // AYEAYE-64 — mutation: the planted violation. The same run in two
    // crates is one finding naming both.
    #[test]
    fn a_run_copied_across_crates_is_found() {
        let findings = runs(
            &[
                source("low", "crates/low/src/lib.rs", &run_of_code("x")),
                source("high", "crates/high/src/lib.rs", &run_of_code("x")),
            ],
            6,
        );
        assert_eq!(findings.len(), 1, "{findings:?}");
        assert_eq!(findings[0].what, "high+low");
        let why = &findings[0].why;
        assert!(why.contains("crates/low/src/lib.rs:"), "{why}");
        assert!(why.contains("crates/high/src/lib.rs:"), "{why}");
    }

    // AYEAYE-64 — fewer lines than the window is not a run.
    #[test]
    fn a_shorter_overlap_than_the_window_is_clean() {
        let a = "fn handle() {\n    let a = read(\"x\");\n    let b = parse(a);\n}\n";
        let b = "fn other() {\n    let a = read(\"x\");\n    let b = parse(a);\n}\n";
        let findings = runs(
            &[
                source("low", "crates/low/src/lib.rs", a),
                source("high", "crates/high/src/lib.rs", b),
            ],
            6,
        );
        assert!(findings.is_empty(), "{findings:?}");
    }

    // AYEAYE-64 — the boundary itself, in files longer than the window: a
    // 5-line overlap with window 6 is clean, and growing it by one line is
    // the whole difference. An off-by-one in the window loop, or a rule that
    // only skipped short files, survives the test above and dies here.
    #[test]
    fn five_shared_lines_are_clean_and_six_are_found_at_window_six() {
        let shared_five = "    let a = read(input);\n    let b = parse(a);\n    \
                           let c = rank(b);\n    let d = emit(c);\n    finish(d);\n";
        let in_low = |shared: &str| {
            format!("fn low_one() {{}}\nfn low_two() {{}}\nfn low_go() {{\n{shared}}}\n")
        };
        let in_high = |shared: &str| {
            format!("fn high_one() {{}}\nfn high_go() {{\n{shared}}}\nfn high_two() {{}}\n")
        };
        let five = runs(
            &[
                source("low", "crates/low/src/lib.rs", &in_low(shared_five)),
                source("high", "crates/high/src/lib.rs", &in_high(shared_five)),
            ],
            6,
        );
        assert!(five.is_empty(), "{five:?}");

        let shared_six = format!("{shared_five}    close(d);\n");
        let six = runs(
            &[
                source("low", "crates/low/src/lib.rs", &in_low(&shared_six)),
                source("high", "crates/high/src/lib.rs", &in_high(&shared_six)),
            ],
            6,
        );
        assert_eq!(six.len(), 1, "{six:?}");
    }

    // AYEAYE-64 — a run copied twice within one crate is that crate's own
    // business; the rule is about crates.
    #[test]
    fn a_run_copied_within_one_crate_is_clean() {
        let findings = runs(
            &[
                source("low", "crates/low/src/a.rs", &run_of_code("x")),
                source("low", "crates/low/src/b.rs", &run_of_code("x")),
            ],
            6,
        );
        assert!(findings.is_empty(), "{findings:?}");
    }

    // AYEAYE-64 — string contents are part of the code: two runs that differ
    // only inside their literals are two different decisions, and blanking
    // the strings would convict them of agreeing.
    #[test]
    fn runs_differing_only_in_a_string_literal_are_clean() {
        let findings = runs(
            &[
                source("low", "crates/low/src/lib.rs", &run_of_code("one")),
                source("high", "crates/high/src/lib.rs", &run_of_code("two")),
            ],
            6,
        );
        assert!(findings.is_empty(), "{findings:?}");
    }

    // AYEAYE-64 — comments are not code: a copy with different commentary is
    // still a copy.
    #[test]
    fn a_copy_with_different_comments_is_still_found() {
        let commented = format!("// entirely different prose\n{}", run_of_code("x"));
        let findings = runs(
            &[
                source("low", "crates/low/src/lib.rs", &commented),
                source("high", "crates/high/src/lib.rs", &run_of_code("x")),
            ],
            6,
        );
        assert_eq!(findings.len(), 1, "{findings:?}");
    }

    // AYEAYE-64 — one copied block is one finding, not one per window it
    // contains; and its key survives an unrelated edit above it.
    #[test]
    fn a_long_run_is_one_finding_with_a_stable_key() {
        let long = format!("{}{}", run_of_code("x"), run_of_code("y"));
        let before = runs(
            &[
                source("low", "crates/low/src/lib.rs", &long),
                source("high", "crates/high/src/lib.rs", &long),
            ],
            6,
        );
        assert_eq!(before.len(), 1, "{before:?}");
        let moved = format!("fn unrelated() {{}}\nfn more() {{}}\n{long}");
        let after = runs(
            &[
                source("low", "crates/low/src/lib.rs", &moved),
                source("high", "crates/high/src/lib.rs", &long),
            ],
            6,
        );
        assert_eq!(keys(&before), keys(&after));
    }

    // AYEAYE-64 — the scope filter: only src/ and build.rs enter, only
    // non-exempt members, so a tests/ file restating a literal never fires.
    #[test]
    fn scope_reads_src_and_build_rs_and_skips_tests_and_the_constitution() {
        use crate::corpus::{Corpus, Member, Source};
        let member = |name: &str, dir: &str, paths: &[&str]| Member {
            name: name.to_string(),
            dir: dir.to_string(),
            dependencies: Vec::new(),
            sources: paths
                .iter()
                .map(|p| Source {
                    path: p.to_string(),
                    text: String::new(),
                })
                .collect(),
            manifest: String::new(),
        };
        let corpus = Corpus {
            members: vec![
                member(
                    "low",
                    "crates/low",
                    &[
                        "crates/low/src/lib.rs",
                        "crates/low/build.rs",
                        "crates/low/tests/integration.rs",
                        "crates/low/benches/bench.rs",
                    ],
                ),
                member("constitution", "crates/constitution", &[
                    "crates/constitution/src/lib.rs",
                ]),
            ],
            root_manifest: String::new(),
        };
        let paths: Vec<String> = super::scope(&corpus).into_iter().map(|s| s.path).collect();
        assert_eq!(
            paths,
            vec![
                "crates/low/src/lib.rs".to_string(),
                "crates/low/build.rs".to_string()
            ]
        );
    }
}
