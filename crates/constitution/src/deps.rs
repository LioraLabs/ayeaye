//! Rule 2: what `ayeaye-core` is allowed to depend on.

use crate::finding::{Finding, Rule};

/// What the pure core may depend on.
///
/// Empty, and that is the honest starting state: the core has no dependencies,
/// and a name here that nothing uses is permission granted in advance to
/// something nobody has looked at.
///
/// Admission is not "does it look harmless". A crate added here must be
/// effect-free in its own non-optional transitive dependencies, because a
/// crate that reaches the filesystem inside itself is invisible to any scan of
/// *our* source — which is exactly why this rule is not redundant with the
/// effect budget. It is also why timestamp and cookie handling are hand-rolled
/// rather than pulled from crates that carry a clock along with them.
pub const ALLOWED: &[&str] = &[];

/// The crate this rule is about. Nothing else in the workspace is held to it:
/// `ayeaye-infer` and `ayeaye` exist in order to have effects.
pub const GOVERNED: &str = "ayeaye-core";

/// Judge a crate's declared dependencies against an allowlist.
///
/// The allowlist is an argument rather than read from inside, for the same
/// reason [`crate::strata::check`] takes its table: the real one is empty, and
/// a rule that cannot be shown an admitted dependency alongside a refused one
/// has only half of it tested. [`ALLOWED`] is what the real workspace passes.
///
/// Every dependency table counts, development ones included. The effect budget
/// already scans `#[cfg(test)]` code, so exempting test dependencies would
/// leave the core one `dev-dependencies` line away from opening a file.
/// Fixtures reach a pure crate through `include_str!`.
pub fn check(crate_name: &str, declared: &[String], allowed: &[&str]) -> Vec<Finding> {
    if crate_name != GOVERNED {
        return Vec::new();
    }
    let list = if allowed.is_empty() {
        "the core's allowlist is empty".to_string()
    } else {
        format!("not on the core's allowlist ({})", allowed.join(", "))
    };
    declared
        .iter()
        .filter(|name| !allowed.contains(&name.as_str()))
        .map(|name| Finding {
            rule: Rule::DependencyAllowlist,
            subject: crate_name.to_string(),
            what: name.clone(),
            why: format!(
                "{list}; add it to ALLOWED with a reason, or depend on it from a crate above the core"
            ),
            line: None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::check;

    const ALLOWED: &[&str] = &["serde"];

    // AYEAYE-41 — mutation: a crate that reaches the world internally is
    // invisible to any scan of our own source, which is the whole reason this
    // rule is not redundant with the effect budget.
    #[test]
    fn a_dependency_that_is_not_on_the_list_is_found() {
        let findings = check("ayeaye-core", &["tokio".to_string()], ALLOWED);
        assert_eq!(
            findings.iter().map(|f| f.what.as_str()).collect::<Vec<_>>(),
            vec!["tokio"]
        );
    }

    // AYEAYE-41
    #[test]
    fn an_allowlisted_dependency_passes() {
        assert!(check("ayeaye-core", &["serde".to_string()], ALLOWED).is_empty());
    }

    // AYEAYE-41 — with nothing admitted, everything declared is a finding, and
    // the message has to say so rather than trailing an empty bracket.
    #[test]
    fn an_empty_allowlist_admits_nothing_and_says_why() {
        let findings = check("ayeaye-core", &["serde".to_string()], &[]);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].why.contains("empty"), "{}", findings[0].why);
    }

    // AYEAYE-41 — the rule is about the pure core, not about every crate.
    #[test]
    fn a_crate_above_the_core_may_depend_on_what_it_likes() {
        assert!(check("ayeaye-infer", &["tokio".to_string()], &[]).is_empty());
    }
}
