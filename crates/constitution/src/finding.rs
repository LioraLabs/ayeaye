//! What a broken rule looks like.

use std::fmt;

/// The rules this crate enforces. Tier 1 — the three that hold the pure core
/// pure and the strata apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rule {
    /// `ayeaye-core` may not reach outside itself.
    EffectBudget,
    /// `ayeaye-core` may not declare a dependency that is not on the list.
    DependencyAllowlist,
    /// A crate may depend only on a strictly lower stratum.
    Stratum,
    /// Nothing in the dependency graph may need a C or C++ compiler.
    PureRustGraph,
}

impl Rule {
    /// The short name the rule goes by in a finding key and in the README.
    pub fn slug(self) -> &'static str {
        match self {
            Rule::EffectBudget => "effect-budget",
            Rule::DependencyAllowlist => "dependency-allowlist",
            Rule::Stratum => "stratum",
            Rule::PureRustGraph => "pure-rust-graph",
        }
    }
}

impl fmt::Display for Rule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.slug())
    }
}

/// One violation, named well enough that a reader can act on it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    /// Which rule was broken.
    pub rule: Rule,
    /// What was being judged — a source path, or a crate name.
    pub subject: String,
    /// The offending thing itself — a reach, a dependency name, an edge.
    pub what: String,
    /// One line saying why it is a violation and what to do instead.
    pub why: String,
    /// Where in the subject, when the subject has a where. Never part of the
    /// key: an edit above a known violation must not rename it.
    pub line: Option<usize>,
}

impl Finding {
    /// The identity of this violation, stable under edits elsewhere in the
    /// file. This is what a waiver list keys on, so it must not move when an
    /// unrelated line is inserted above it.
    pub fn key(&self) -> String {
        format!("{}/{}/{}", self.rule.slug(), self.subject, self.what)
    }
}

impl fmt::Display for Finding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.line {
            Some(line) => write!(f, "{}:{}: {} — {}", self.subject, line, self.what, self.why),
            None => write!(f, "{}: {} — {}", self.subject, self.what, self.why),
        }
    }
}

/// Render a set of findings as the body of a test failure.
///
/// A constitution that fails with `assertion failed: findings.is_empty()` is a
/// constitution nobody can obey, so the message is the product here.
pub fn report(findings: &[Finding]) -> String {
    let mut out = String::new();
    for finding in findings {
        out.push_str("  ");
        out.push_str(&finding.to_string());
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{Finding, Rule, report};

    fn finding(line: Option<usize>) -> Finding {
        Finding {
            rule: Rule::EffectBudget,
            subject: "crates/ayeaye-core/src/lib.rs".to_string(),
            what: "std::fs".to_string(),
            why: "the filesystem is the shell's".to_string(),
            line,
        }
    }

    // AYEAYE-41 — the key is what a waiver list will hang on, so an edit that
    // moves a known violation down the file must not rename it.
    #[test]
    fn the_key_does_not_move_when_the_line_does() {
        assert_eq!(finding(Some(4)).key(), finding(Some(400)).key());
        assert_eq!(
            finding(None).key(),
            "effect-budget/crates/ayeaye-core/src/lib.rs/std::fs"
        );
    }

    // AYEAYE-41
    #[test]
    fn two_different_violations_have_two_different_keys() {
        let mut other = finding(Some(4));
        other.what = "std::net".to_string();
        assert_ne!(finding(Some(4)).key(), other.key());
    }

    // AYEAYE-41 — the failure message is the product: a reader has to be able
    // to find the violation and know what to do about it.
    #[test]
    fn a_report_names_the_file_the_line_the_reach_and_the_remedy() {
        let rendered = report(&[finding(Some(12))]);
        assert_eq!(
            rendered,
            "  crates/ayeaye-core/src/lib.rs:12: std::fs — the filesystem is the shell's\n"
        );
    }

    // AYEAYE-41 — a rule with nothing to point at still has to read.
    #[test]
    fn a_finding_with_no_line_leaves_the_colon_out() {
        assert_eq!(
            finding(None).to_string(),
            "crates/ayeaye-core/src/lib.rs: std::fs — the filesystem is the shell's"
        );
    }
}
