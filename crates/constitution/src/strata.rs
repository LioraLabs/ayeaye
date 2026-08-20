//! Rule 3: which way a dependency is allowed to point.

use crate::finding::{Finding, Rule};

/// A workspace member and the workspace members it depends on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrateNode {
    /// The package name, as the manifest spells it.
    pub name: String,
    /// The names of the workspace members it declares a dependency on, in any
    /// dependency table.
    pub depends_on: Vec<String>,
}

/// Every workspace member, and how high it sits.
///
/// A dependency may only point at a strictly lower number. Upwards is the edge
/// that dissolves the split; sideways is the edge that makes two crates one
/// crate with extra manifests.
///
/// `constitution` sits above everything on purpose: it must be able to read
/// every crate, and nothing may be built on it.
pub const STRATA: &[(&str, u8)] = &[("ayeaye-core", 0), ("ayeaye", 1), ("constitution", 2)];

/// Judge a set of crates against a table of strata.
///
/// The table is an argument rather than a constant read from inside, because
/// the violations this rule exists to catch — an edge sideways between two
/// crates at the same height — cannot be built out of a table that has no two
/// crates at the same height. A rule that cannot be shown a planted violation
/// is a rule nobody has tested. [`STRATA`] is what the real workspace passes.
pub fn check(crates: &[CrateNode], strata: &[(&str, u8)]) -> Vec<Finding> {
    let mut findings = Vec::new();

    for node in crates {
        let Some(here) = stratum_of(&node.name, strata) else {
            // A crate nobody placed is a crate this rule cannot judge, so
            // adding one has to be a deliberate act rather than a way past.
            findings.push(Finding {
                rule: Rule::Stratum,
                subject: node.name.clone(),
                what: node.name.clone(),
                why: "not placed in the stratum table; add it to STRATA and say where it sits"
                    .to_string(),
                line: None,
            });
            continue;
        };

        for dep in &node.depends_on {
            let Some(there) = stratum_of(dep, strata) else {
                continue;
            };
            if there < here {
                continue;
            }
            let direction = if there == here { "sideways" } else { "upwards" };
            findings.push(Finding {
                rule: Rule::Stratum,
                subject: node.name.clone(),
                what: format!("{} -> {}", node.name, dep),
                why: format!(
                    "{direction}: stratum {here} may only depend on something lower than {here}, and {dep} is {there}"
                ),
                line: None,
            });
        }
    }

    findings
}

/// Where a crate sits in a table, if it has been placed at all.
pub fn stratum_of(name: &str, strata: &[(&str, u8)]) -> Option<u8> {
    strata
        .iter()
        .find(|(crate_name, _)| *crate_name == name)
        .map(|(_, stratum)| *stratum)
}

#[cfg(test)]
mod tests {
    use super::{CrateNode, check};

    /// A synthetic table with two crates side by side, which the real one has
    /// no example of.
    const TABLE: &[(&str, u8)] = &[("low", 0), ("also-low", 0), ("high", 1)];

    fn node(name: &str, depends_on: &[&str]) -> CrateNode {
        CrateNode {
            name: name.to_string(),
            depends_on: depends_on.iter().map(|d| d.to_string()).collect(),
        }
    }

    fn what(findings: &[crate::Finding]) -> Vec<&str> {
        findings.iter().map(|f| f.what.as_str()).collect()
    }

    // AYEAYE-41 — mutation: a crate reaching up into the one above it is the
    // edge that would quietly dissolve the whole split.
    #[test]
    fn an_upward_edge_is_found() {
        let findings = check(&[node("low", &["high"])], TABLE);
        assert_eq!(what(&findings), vec!["low -> high"]);
        assert!(findings[0].why.contains("upwards"), "{}", findings[0].why);
    }

    // AYEAYE-41 — mutation: sideways is the edge that turns two crates into
    // one crate with two manifests.
    #[test]
    fn a_sideways_edge_is_found() {
        let findings = check(&[node("low", &["also-low"])], TABLE);
        assert_eq!(what(&findings), vec!["low -> also-low"]);
        assert!(findings[0].why.contains("sideways"), "{}", findings[0].why);
    }

    // AYEAYE-41 — mutation: a crate nobody placed must not slip through as
    // "no rule applies to me".
    #[test]
    fn an_unplaced_crate_is_found() {
        assert_eq!(
            what(&check(&[node("newcomer", &[])], TABLE)),
            vec!["newcomer"]
        );
    }

    // AYEAYE-41
    #[test]
    fn a_downward_edge_passes() {
        assert!(check(&[node("high", &["low", "also-low"])], TABLE).is_empty());
    }
}
