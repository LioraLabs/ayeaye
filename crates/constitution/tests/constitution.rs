//! The constitution, run against the workspace it governs.
//!
//! The rules themselves are unit-tested against planted violations beside
//! their implementations; these are the same rules pointed at the real tree.
//! Both halves are needed: a rule with no mutation test may be blind, and a
//! rule that is never run against the tree is decoration.

use constitution::corpus::{Corpus, workspace_root};
use constitution::{deps, effect_budget, finding::report, strata};

/// The floor the whole corpus walk has to clear.
///
/// It exists to catch a walk that found nothing — a mistyped directory, a
/// wrong root, a member list that stopped matching the tree. Raise it as the
/// crates fill up; a walk that returns two files where it used to return forty
/// is the failure this number is here to make loud.
const NON_TRIVIAL: usize = 8;

/// The floor for the crates that are actually shipped.
///
/// The constitution is most of the workspace's source today, so a total on its
/// own would still pass if the walk found nothing but the constitution. This
/// is the number that says the governed crates were really read.
const NON_TRIVIAL_SHIPPED: usize = 5;

fn corpus() -> Corpus {
    Corpus::walk(&workspace_root()).expect("the workspace should be readable")
}

// AYEAYE-41
#[test]
fn the_walk_finds_a_non_trivial_corpus() {
    let corpus = corpus();
    assert!(
        corpus.file_count() >= NON_TRIVIAL,
        "the walk found {} source files, which is fewer than the {NON_TRIVIAL} a real \
         workspace has — the walk is looking in the wrong place",
        corpus.file_count()
    );

    let shipped: usize = corpus
        .members
        .iter()
        .filter(|member| member.name != "constitution")
        .map(|member| member.sources.len())
        .sum();
    assert!(
        shipped >= NON_TRIVIAL_SHIPPED,
        "the walk found {shipped} source files outside the constitution itself, fewer than \
         the {NON_TRIVIAL_SHIPPED} the shipped crates have — the rules are judging almost nothing"
    );
}

// AYEAYE-41
#[test]
fn every_crate_the_strata_place_is_a_crate_the_walk_found() {
    let corpus = corpus();
    for (name, _) in strata::STRATA {
        let member = corpus.member(name).unwrap_or_else(|| {
            panic!("{name} is in the stratum table but the walk did not find it")
        });
        assert!(
            !member.sources.is_empty(),
            "{name} contributed no source files, so nothing about it was actually read"
        );
    }
}

// AYEAYE-41
#[test]
fn the_pure_core_stays_within_its_effect_budget() {
    let corpus = corpus();
    let core = corpus
        .member(deps::GOVERNED)
        .expect("the pure core should be a workspace member");
    let findings: Vec<_> = core
        .sources
        .iter()
        .flat_map(|source| effect_budget::scan(&source.path, &source.text))
        .collect();
    assert!(
        findings.is_empty(),
        "{} reaches outside the effect budget:\n{}",
        deps::GOVERNED,
        report(&findings)
    );
}

// AYEAYE-41
#[test]
fn the_pure_core_declares_only_allowlisted_dependencies() {
    let corpus = corpus();
    let core = corpus
        .member(deps::GOVERNED)
        .expect("the pure core should be a workspace member");
    let findings = deps::check(&core.name, &core.dependencies, deps::ALLOWED);
    assert!(
        findings.is_empty(),
        "{} declares dependencies that are not on its allowlist:\n{}",
        deps::GOVERNED,
        report(&findings)
    );
}

// AYEAYE-41
#[test]
fn no_crate_depends_upwards_or_sideways() {
    let corpus = corpus();
    let findings = strata::check(&corpus.nodes(), strata::STRATA);
    assert!(
        findings.is_empty(),
        "the strata are broken:\n{}",
        report(&findings)
    );
}
