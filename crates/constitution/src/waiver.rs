//! The waiver ratchet: how a gate lands green on a tree that already breaks
//! its rules, without ever letting the breakage grow.
//!
//! A waiver names one existing violation by its [`Finding::key`] and says why
//! it is allowed to persist. The list can only shrink: a waiver whose
//! violation has gone is itself a violation, so nobody can bank an old entry
//! against a future copy — and a waiver with no written justification is a
//! violation too, because "someone once had a reason" is exactly the label
//! every duplication in this codebase already wore.

use crate::finding::{Finding, Rule};

/// One violation allowed to persist, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Waiver {
    /// The [`Finding::key`] of the violation being waived.
    pub key: String,
    /// Why this one may stay. Blank is a violation, not a default.
    pub justification: String,
}

/// Parse the waiver file: `[[waiver]]` tables, each with a `key` and a
/// `justification`.
///
/// A file that does not parse, or an entry missing either field, is an error
/// rather than an empty list — a half-read waiver file would un-waive
/// everything and fail the gate loudly, but an *ignored* one would waive
/// nothing and fail it confusingly. Say what is wrong instead.
pub fn parse(text: &str) -> Result<Vec<Waiver>, String> {
    let table = text
        .parse::<toml::Table>()
        .map_err(|e| format!("the waiver file does not parse: {e}"))?;
    let mut waivers = Vec::new();
    let Some(entries) = table.get("waiver") else {
        return Ok(waivers);
    };
    let entries = entries
        .as_array()
        .ok_or_else(|| "`waiver` should be an array of tables: use [[waiver]]".to_string())?;
    for (index, entry) in entries.iter().enumerate() {
        let field = |name: &str| -> Result<String, String> {
            entry
                .get(name)
                .and_then(|v| v.as_str())
                .map(str::to_string)
                .ok_or_else(|| format!("waiver {} has no `{name}` string", index + 1))
        };
        waivers.push(Waiver {
            key: field("key")?,
            justification: field("justification")?,
        });
    }
    Ok(waivers)
}

/// Judge findings against the waiver list.
///
/// What comes back is everything still wrong: findings no waiver covers, plus
/// a [`Rule::Waiver`] finding for every entry that is itself a violation — a
/// blank justification, a key waiving nothing that exists, a key listed
/// twice. A covered finding is suppressed even when its waiver has a blank
/// justification: the failure should point at the waiver file, where the fix
/// is, not at code the entry already admits to.
///
/// `waiver_file` is the path findings about the file itself are filed under.
pub fn apply(findings: &[Finding], waivers: &[Waiver], waiver_file: &str) -> Vec<Finding> {
    let mut out = Vec::new();

    let waived = |key: &str| waivers.iter().any(|w| w.key == key);
    for finding in findings {
        if !waived(&finding.key()) {
            out.push(finding.clone());
        }
    }

    for (index, waiver) in waivers.iter().enumerate() {
        if waiver.justification.trim().is_empty() {
            out.push(Finding {
                rule: Rule::Waiver,
                subject: waiver_file.to_string(),
                what: waiver.key.clone(),
                why: "waived with no justification — write down why this violation is \
                      allowed to persist, or remove the entry and fix it"
                    .to_string(),
                line: None,
            });
        }
        if !findings.iter().any(|f| f.key() == waiver.key) {
            out.push(Finding {
                rule: Rule::Waiver,
                subject: waiver_file.to_string(),
                what: waiver.key.clone(),
                why: "waives nothing that exists — the violation is gone, so delete the \
                      entry; the list only shrinks"
                    .to_string(),
                line: None,
            });
        }
        if waivers[..index].iter().any(|w| w.key == waiver.key) {
            out.push(Finding {
                rule: Rule::Waiver,
                subject: waiver_file.to_string(),
                what: waiver.key.clone(),
                why: "listed twice — one violation gets one entry and one justification"
                    .to_string(),
                line: None,
            });
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::{Waiver, apply, parse};
    use crate::finding::{Finding, Rule};

    const FILE: &str = "crates/constitution/waivers.toml";

    fn finding(what: &str) -> Finding {
        Finding {
            rule: Rule::DuplicateLiteral,
            subject: "\"the decision\"".to_string(),
            what: what.to_string(),
            why: "planted".to_string(),
            line: None,
        }
    }

    fn waiver(key: &str, justification: &str) -> Waiver {
        Waiver {
            key: key.to_string(),
            justification: justification.to_string(),
        }
    }

    // AYEAYE-64 — the green path: every existing violation waived with a
    // reason, nothing new, nothing stale. This is the state the gate lands in.
    #[test]
    fn a_fully_waived_list_is_clean() {
        let f = finding("high+low");
        let out = apply(&[f.clone()], &[waiver(&f.key(), "predates the tiers")], FILE);
        assert!(out.is_empty(), "{out:?}");
    }

    // AYEAYE-64 — mutation: a new violation has no waiver and passes through
    // untouched. This is "fails only on what is new".
    #[test]
    fn an_unwaived_finding_passes_through() {
        let old = finding("high+low");
        let new = finding("high+mid");
        let out = apply(
            &[old.clone(), new.clone()],
            &[waiver(&old.key(), "predates the tiers")],
            FILE,
        );
        assert_eq!(out, vec![new]);
    }

    // AYEAYE-64 — mutation: a blank justification is itself a failure.
    #[test]
    fn a_waiver_without_a_justification_is_a_finding() {
        let f = finding("high+low");
        let out = apply(&[f.clone()], &[waiver(&f.key(), "   ")], FILE);
        assert_eq!(out.len(), 1, "{out:?}");
        assert_eq!(out[0].rule, Rule::Waiver);
        assert!(out[0].why.contains("no justification"), "{}", out[0].why);
    }

    // AYEAYE-64 — mutation: a waiver whose violation is gone is itself a
    // failure, so the list can only shrink.
    #[test]
    fn a_stale_waiver_is_a_finding() {
        let out = apply(
            &[],
            &[waiver("duplicate-literal/\"gone\"/high+low", "it left")],
            FILE,
        );
        assert_eq!(out.len(), 1, "{out:?}");
        assert_eq!(out[0].rule, Rule::Waiver);
        assert!(out[0].why.contains("only shrinks"), "{}", out[0].why);
    }

    // AYEAYE-64 — a key listed twice is one entry too many.
    #[test]
    fn a_duplicated_waiver_key_is_a_finding() {
        let f = finding("high+low");
        let out = apply(
            &[f.clone()],
            &[
                waiver(&f.key(), "predates the tiers"),
                waiver(&f.key(), "said twice"),
            ],
            FILE,
        );
        assert_eq!(out.len(), 1, "{out:?}");
        assert!(out[0].why.contains("listed twice"), "{}", out[0].why);
    }

    // AYEAYE-64 — nothing wrong, nothing waived, nothing reported.
    #[test]
    fn no_findings_and_no_waivers_is_clean() {
        assert!(apply(&[], &[], FILE).is_empty());
    }

    // AYEAYE-64 — the file format, round-tripped.
    #[test]
    fn a_waiver_file_parses_to_its_entries() {
        let waivers = parse(
            "[[waiver]]\nkey = \"duplicate-literal/\\\"x\\\"/a+b\"\njustification = \"why\"\n",
        )
        .expect("a well-formed file");
        assert_eq!(
            waivers,
            vec![waiver("duplicate-literal/\"x\"/a+b", "why")]
        );
        assert_eq!(parse("").expect("an empty file"), Vec::new());
    }

    // AYEAYE-64 — a malformed file is an error, never an empty list: an
    // ignored waiver file would fail the gate confusingly rather than loudly.
    #[test]
    fn a_malformed_waiver_file_is_refused() {
        assert!(parse("not toml at [[").is_err());
        let missing = parse("[[waiver]]\nkey = \"only-a-key\"\n");
        let error = missing.expect_err("a keyless entry should refuse");
        assert!(error.contains("justification"), "{error}");
    }
}
