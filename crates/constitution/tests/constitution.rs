//! The constitution, run against the workspace it governs.
//!
//! The rules themselves are unit-tested against planted violations beside
//! their implementations; these are the same rules pointed at the real tree.
//! Both halves are needed: a rule with no mutation test may be blind, and a
//! rule that is never run against the tree is decoration.

use constitution::corpus::{self, Corpus, workspace_root};
use constitution::{deps, duplication, effect_budget, finding::report, strata, toolchain, waiver};

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
    // The root manifest is its own guard: an empty string parses as an empty
    // table and reports clean, so a `root_manifest` that regressed to `""`
    // would look exactly like a workspace with nothing to say.
    assert!(
        corpus.root_manifest.contains("[workspace]"),
        "the root manifest was not read, so judging it proves nothing"
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

// AYEAYE-57
//
// The exact half of rule 4's feature question, and the one that holds. Three
// separate bypasses of the manifest half were found by planting them — a
// feature spelled `flash-attn`, a feature of ours forwarding one across a crate
// boundary, and a crate cargo treats as a member that our own walk never listed
// — and all three are the same shape: a rule that restates cargo's feature
// resolution will keep being incomplete. This one reads cargo's answer.
#[test]
fn the_default_build_resolves_to_no_acceleration_package() {
    let root = workspace_root();
    let packages =
        corpus::default_build_packages(&root).expect("cargo tree should describe the workspace");

    // A resolution that returned nothing would pass every table it is handed.
    assert!(
        packages.len() > 50,
        "cargo tree named {} packages, which is not a real workspace",
        packages.len()
    );
    // ...and a rule that cannot find a package the default build certainly has
    // is not reading the list it was given.
    assert!(
        !toolchain::in_default_build(&packages, &[("candle-core", "the graph really has this")])
            .is_empty(),
        "the rule found nothing in a resolution containing candle-core"
    );

    let findings = toolchain::in_default_build(&packages, toolchain::ACCELERATED);

    assert!(
        findings.is_empty(),
        "the portable build needs a CUDA toolchain:\n{}",
        report(&findings)
    );
}

// AYEAYE-64
//
// The duplication tiers, held green by the waiver ratchet: every violation
// the tree already had is waived with a written reason, so what this test
// refuses is only what is new. The waiver file is read from beside this test
// by include_str! — and the non-empty waiver list doubles as the canary
// against a scanner gone blind, because a scanner that stops finding the
// waived violations turns every entry stale at once, loudly.
#[test]
fn no_decision_is_implemented_twice() {
    let corpus = corpus();
    let sources = duplication::scope(&corpus);

    // A scope that emptied out would pass both rules vacuously: prove it
    // still covers the three shipped crates, with a real number of files.
    let crates: std::collections::BTreeSet<&str> =
        sources.iter().map(|s| s.crate_name.as_str()).collect();
    assert!(
        crates.len() >= 3,
        "the duplication scope covers {crates:?}, not the three shipped crates"
    );
    assert!(
        sources.len() >= NON_TRIVIAL,
        "the duplication scope holds {} files, which is not a real workspace",
        sources.len()
    );

    // ...and a rule that finds nothing in sources it is not really reading
    // would also pass vacuously. Hand it a planted pair and it has to fire.
    let planted = duplication::literals(
        &[
            duplication::CrateSource {
                crate_name: "one".to_string(),
                path: "crates/one/src/lib.rs".to_string(),
                text: "const A: &str = \"a planted decision\";".to_string(),
            },
            duplication::CrateSource {
                crate_name: "two".to_string(),
                path: "crates/two/src/lib.rs".to_string(),
                text: "const B: &str = \"a planted decision\";".to_string(),
            },
        ],
        duplication::LITERAL_FLOOR,
    );
    assert!(
        !planted.is_empty(),
        "the literal rule found nothing in a planted two-crate duplication; it is blind"
    );

    let mut findings = duplication::literals(&sources, duplication::LITERAL_FLOOR);
    findings.extend(duplication::runs(&sources, duplication::RUN_LINES));

    let waivers =
        waiver::parse(include_str!("../waivers.toml")).expect("waivers.toml should parse");
    let remaining = waiver::apply(&findings, &waivers, "crates/constitution/waivers.toml");

    let rendered: String = remaining
        .iter()
        .map(|finding| format!("  {finding}\n    waiver key: {}\n", finding.key()))
        .collect();
    assert!(
        remaining.is_empty(),
        "a decision is implemented twice (or the waiver file is wrong):\n{rendered}\n\
         Either remove the duplication, or add a [[waiver]] with a written justification \
         to crates/constitution/waivers.toml — `cargo run -p constitution --example \
         ratchet` prints paste-ready entries."
    );
}

// AYEAYE-57 — the twin of `every_workspace_member_lives_under_crates`, and the
// half that was missing.
//
// **Cargo treats a path dependency inside the workspace directory as a member
// whether or not `[workspace] members` lists it.** The corpus walks the list,
// so a crate under `crates/` that nobody added to it was built, resolved
// features, and had rules 1, 2 and 3 applied to it not at all — the effect
// budget never scanned it, the strata never placed it, the allowlist never read
// it. Rule 4's package check catches what such a crate *pulls in*; nothing
// caught the crate itself.
#[test]
fn every_crate_under_crates_is_a_listed_workspace_member() {
    let root = workspace_root();
    let listed: Vec<String> = corpus().members.iter().map(|m| m.dir.clone()).collect();

    let entries = std::fs::read_dir(root.join("crates")).expect("crates/ should be readable");
    let mut unlisted = Vec::new();
    for entry in entries {
        let path = entry.expect("a directory entry").path();
        if !path.join("Cargo.toml").is_file() {
            continue;
        }
        let name = path
            .file_name()
            .expect("a directory name")
            .to_string_lossy()
            .into_owned();
        let dir = format!("crates/{name}");
        if !listed.contains(&dir) {
            unlisted.push(dir);
        }
    }

    assert!(
        unlisted.is_empty(),
        "these crates are under crates/ but not in [workspace] members, so cargo builds \
         them and the constitution never reads them: {unlisted:?}. Add them to the member \
         list, or move them out of the workspace directory"
    );
}
