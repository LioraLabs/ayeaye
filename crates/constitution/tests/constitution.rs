//! The constitution, run against the workspace it governs.
//!
//! The rules themselves are unit-tested against planted violations beside
//! their implementations; these are the same rules pointed at the real tree.
//! Both halves are needed: a rule with no mutation test may be blind, and a
//! rule that is never run against the tree is decoration.

use constitution::corpus::{Corpus, workspace_root};
use constitution::{deps, effect_budget, finding::report, strata, toolchain};

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
/// is the number that says the governed crates were really read. It sits below
/// today's count on purpose: it is a floor against a walk that found nothing,
/// not a ratchet against anybody merging two modules into one.
const NON_TRIVIAL_SHIPPED: usize = 3;

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

// AYEAYE-41 — the build system's probe over the Rust sources is a glob under
// `crates/`. A member outside it would be read by this walk and invisible to
// that probe, which is a green release gate over source nothing rebuilt on.
#[test]
fn every_workspace_member_lives_under_crates() {
    for member in &corpus().members {
        assert!(
            member.dir.starts_with("crates/"),
            "{} sits at {}, outside the Cookfile's `rust` probe — move it under crates/, \
             or widen the probe in the same commit",
            member.name,
            member.dir
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

// AYEAYE-54
//
// The milestone's defining property, as a test rather than a comment. It very
// nearly left without anyone noticing: candle-core 0.10 names `tokenizers`
// with the `onig` feature itself, so upgrading candle — a routine-looking
// bump — would have put a C compiler back into every native build.
#[test]
fn nothing_in_the_dependency_graph_needs_a_c_compiler() {
    let lockfile_path = workspace_root().join("Cargo.lock");
    let lockfile = std::fs::read_to_string(&lockfile_path)
        .unwrap_or_else(|e| panic!("reading {}: {e}", lockfile_path.display()));
    // A walk that read nothing would pass this vacuously.
    assert!(
        lockfile
            .lines()
            .filter(|l| l.starts_with("name = "))
            .count()
            > 50,
        "the lockfile looks empty; the rule would pass over nothing"
    );

    // ...and a rule pointed at the real lockfile that cannot find anything in
    // it would also pass vacuously. Planting a package to prove otherwise does
    // not work: cargo rewrites Cargo.lock before the test runs and removes any
    // entry nothing depends on. So the proof is the other way round - hand the
    // rule a table naming something the lockfile really does carry, and it has
    // to say so.
    let planted = toolchain::check(&lockfile, &[("libc", "a package this graph really has")]);
    assert!(
        !planted.is_empty(),
        "the rule found nothing in a lockfile that contains libc; it is not reading it"
    );

    let findings = toolchain::check(&lockfile, toolchain::FORBIDDEN);

    assert!(
        findings.is_empty(),
        "the build is no longer pure Rust:\n{}",
        report(&findings)
    );
}

// AYEAYE-57
//
// Rule 4's second half, against the real manifests. The first half proves the
// graph compiles no C; this proves the cost that `cc` cannot see — nvcc, driven
// by candle-kernels' build script — stays out of the build nobody asks for.
#[test]
fn no_default_build_turns_on_a_feature_that_needs_a_toolchain() {
    let corpus = corpus();

    // A rule that finds nothing in manifests it is not really reading would
    // pass this vacuously, and unlike the lockfile there is nothing to plant:
    // the real tree has no violation. So the proof runs the other way round —
    // hand it a table naming `metal`, which `ayeaye-infer` really does turn on
    // for Apple builds through a target table, and it has to say so.
    let probe = &[toolchain::Gated {
        feature: "metal",
        also: &[],
        cost: "nothing at all, which is why it is not really gated",
        artifact: "no artifact",
    }];
    let infer = corpus
        .member("ayeaye-infer")
        .expect("ayeaye-infer should be a workspace member");
    assert!(
        !toolchain::gated(&infer.dir, &infer.manifest, probe).is_empty(),
        "the rule found nothing in a manifest that turns metal on for macOS; \
         it is not reading the manifests"
    );

    // The root manifest as well as every member's. It is not a member, it has
    // no sources, and it is where this workspace keeps candle — so "every
    // member" would skip exactly the file a future acceleration edit lands in.
    let findings: Vec<_> = corpus
        .members
        .iter()
        .map(|member| (member.dir.as_str(), member.manifest.as_str()))
        .chain([("Cargo.toml", corpus.root_manifest.as_str())])
        .flat_map(|(subject, manifest)| toolchain::gated(subject, manifest, toolchain::GATED))
        .collect();

    assert!(
        findings.is_empty(),
        "a default build would need a toolchain:\n{}",
        report(&findings)
    );
}
