//! Rule 4: nothing in the dependency graph may need a C or C++ compiler.

use crate::finding::{Finding, Rule};

/// Packages whose presence means the build is no longer pure Rust.
///
/// `cc` is the load-bearing one and the reason this rule is cheap to trust:
/// **it is how a build script compiles C or C++ in this ecosystem.** A crate
/// that vendors native sources takes `cc` as a build dependency, so `cc`
/// absent from the lockfile is a mechanical proof that no native source is
/// compiled anywhere in the graph — including inside crates nobody read.
/// `cmake` and `bindgen` are the same argument for the two other shapes the
/// cost takes, and `onig`/`onig_sys` are named because they are how it very
/// nearly arrived: see the note in `crates/ayeaye-infer/Cargo.toml`.
pub const FORBIDDEN: &[(&str, &str)] = &[
    (
        "cc",
        "compiles C or C++ from a build script; a pure Rust graph has no use for it",
    ),
    ("cmake", "runs CMake at build time"),
    ("bindgen", "needs libclang at build time"),
    (
        "onig",
        "oniguruma is a C library; take tokenizers with `fancy-regex` instead",
    ),
    ("onig_sys", "the C sources of oniguruma"),
];

/// Judge a lockfile against a table of packages that must not be in it.
///
/// The table is an argument rather than read from inside, for the same reason
/// [`crate::strata::check`] takes its table: a rule that has only ever been
/// run against a graph with no violation in it is a rule nobody has tested.
/// [`FORBIDDEN`] is what the real workspace passes.
///
/// It reads `Cargo.lock` rather than the manifests because the manifests only
/// say what *we* asked for. The cost this rule exists to catch arrives through
/// somebody else's dependency — which is exactly how it did arrive: candle-core
/// 0.10 names `tokenizers` with the `onig` feature itself, and no amount of
/// `default-features = false` on our own line has any effect on that.
///
/// **What it cannot catch:** the lockfile lists every optional dependency in
/// the graph, including ones no feature enables, so a name here is not proof
/// that anything was compiled. That makes the rule conservative in the safe
/// direction — it can refuse a build that would have been fine, and it cannot
/// pass one that would not. `bindgen_cuda` is the standing example: it sits in
/// the lockfile today and builds only under the `cuda` feature, which the
/// milestone has already accepted is the one artifact that is not portable.
pub fn check(lockfile: &str, forbidden: &[(&str, &str)]) -> Vec<Finding> {
    let mut findings = Vec::new();

    for (line, text) in lockfile.lines().enumerate() {
        let Some(name) = package_name(text) else {
            continue;
        };
        let Some((_, why)) = forbidden.iter().find(|(candidate, _)| *candidate == name) else {
            continue;
        };
        findings.push(Finding {
            rule: Rule::PureRustGraph,
            subject: "Cargo.lock".to_string(),
            what: name.to_string(),
            why: format!(
                "{why}. The single portable binary is the milestone, not a preference: \
                 drop the dependency, or amend toolchain::FORBIDDEN and say out loud \
                 which artifact stops being static"
            ),
            line: Some(line + 1),
        });
    }

    findings
}

/// The package a `name = "x"` line names, if it is one.
fn package_name(line: &str) -> Option<&str> {
    let rest = line.trim().strip_prefix("name")?.trim_start();
    let rest = rest.strip_prefix('=')?.trim();
    rest.strip_prefix('"')?.strip_suffix('"')
}

#[cfg(test)]
mod tests {
    use super::{FORBIDDEN, check, package_name};

    const CLEAN: &str = r#"
[[package]]
name = "candle-core"
version = "0.9.2"

[[package]]
name = "tokenizers"
version = "0.23.1"
"#;

    // AYEAYE-54 — the mutation test. This is the exact shape the graph had
    // before candle was pinned back to 0.9.
    #[test]
    fn a_lockfile_carrying_oniguruma_is_a_finding() {
        let planted = format!("{CLEAN}\n[[package]]\nname = \"onig_sys\"\nversion = \"69.9.3\"\n");

        let findings = check(&planted, FORBIDDEN);

        assert_eq!(findings.len(), 1, "{findings:?}");
        assert_eq!(findings[0].what, "onig_sys");
        assert_eq!(findings[0].key(), "pure-rust-graph/Cargo.lock/onig_sys");
    }

    // AYEAYE-54 — `cc` is the general case: whatever vendors the C, this is
    // what compiles it.
    #[test]
    fn a_lockfile_carrying_a_c_compiler_driver_is_a_finding() {
        let planted = format!("{CLEAN}\n[[package]]\nname = \"cc\"\nversion = \"1.2.0\"\n");

        let findings = check(&planted, FORBIDDEN);

        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].what, "cc");
    }

    // AYEAYE-54
    #[test]
    fn a_pure_rust_lockfile_has_nothing_to_say() {
        assert!(check(CLEAN, FORBIDDEN).is_empty());
    }

    // AYEAYE-54 — the finding has to point at the line, because a lockfile is
    // two thousand lines long and nobody is reading it by eye.
    #[test]
    fn a_finding_names_the_line_the_package_is_on() {
        let planted = "[[package]]\nname = \"cmake\"\n";

        let findings = check(planted, FORBIDDEN);

        assert_eq!(findings[0].line, Some(2));
    }

    // AYEAYE-54 — a version or a checksum that happens to contain the word is
    // not a package, and neither is a dependency list entry.
    #[test]
    fn only_a_package_name_counts() {
        let noise = "version = \"cc\"\nchecksum = \"cc0000\"\n dependencies = [\n \"cc\",\n]\n";

        assert!(check(noise, FORBIDDEN).is_empty());
    }

    // AYEAYE-54
    #[test]
    fn a_name_line_is_read_whatever_the_spacing() {
        assert_eq!(package_name("name = \"cc\""), Some("cc"));
        assert_eq!(package_name("name=\"cc\""), Some("cc"));
        assert_eq!(package_name("  name  =  \"cc\"  "), Some("cc"));
        assert_eq!(package_name("nameless = \"cc\""), None);
        assert_eq!(package_name("version = \"1.0\""), None);
    }
}
