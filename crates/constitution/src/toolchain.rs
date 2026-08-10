//! Rule 4: nothing the portable build needs may require a C, C++ or CUDA
//! compiler.
//!
//! Two halves, because no single input answers it. [`check`] reads `Cargo.lock`
//! and proves the graph carries no crate that vendors C for a C compiler to
//! build. [`gated`] reads the manifests and proves the cost `cc` cannot see —
//! a build script driving its own compiler, which is what the CUDA path does —
//! stays out of the build nobody asked for.

use crate::finding::{Finding, Rule};

/// Packages whose presence means the build is no longer pure Rust.
///
/// `cc` is the load-bearing one and the reason this half is cheap to trust:
/// **it is how a build script compiles C or C++ in this ecosystem.** A crate
/// that vendors native sources takes `cc` as a build dependency, so `cc`
/// absent from the lockfile is a mechanical proof that no such crate is in the
/// graph — including crates nobody read. `cmake` and `bindgen` are the same
/// argument for the two other shapes that cost takes, and `onig`/`onig_sys`
/// are named because they are how it very nearly arrived: see the note in
/// `crates/ayeaye-infer/Cargo.toml`.
///
/// **It is not proof that nothing anywhere compiles C.** A build script that
/// drives its own compiler needs no `cc`, and that is exactly the CUDA path:
/// `candle-kernels` runs nvcc itself. That cost is [`gated`]'s to catch, not
/// this table's.
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

/// An optional feature that legitimately needs a toolchain, and what it costs.
///
/// The second half of rule 4 exists because the first half cannot see this.
/// [`check`] reads names in `Cargo.lock`, and a lockfile is feature-blind in
/// **both** directions:
///
/// - It lists optional dependencies whether or not anything enables them, so
///   `bindgen_cuda` sitting in the lockfile is not evidence that any CUDA is
///   compiled. (AYEAYE-56 measured that from the other side.)
/// - And `cc` being absent is not evidence that none is. `candle-kernels`'
///   build script drives **nvcc** directly and links `stdc++`; `bindgen_cuda`
///   depends on `glob`, `num_cpus` and `rayon`, and on nothing that compiles C.
///   The whole CUDA toolchain is invisible to a table of package names.
///
/// So the question "is a toolchain in the default build" cannot be answered
/// from the lockfile at all. It can be answered from the manifests, which are
/// the only place that says what is *on* — and that is what [`gated`] reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Gated {
    /// The feature name, as our manifests spell it.
    pub feature: &'static str,
    /// Other feature names that turn the same cost on.
    ///
    /// A gated feature is rarely reachable only under its own name: candle
    /// defines `cudnn`, `nccl` and `flash-attn`, each listing `cuda` among its
    /// own, so `default = ["candle-core/cudnn"]` pays for nvcc while naming
    /// nothing called `cuda`.
    ///
    /// **This list is a courtesy, not the guarantee.** It cannot be complete:
    /// it describes an upstream's feature names, which the upstream may add to
    /// in any release, and a crate of ours can forward any of them under a name
    /// of its own. [`in_default_build`] is the half that actually holds — it
    /// reads what cargo resolved and does not care what anything is called.
    /// What this buys is a finding that names the feature and the manifest,
    /// which is what somebody needs in order to fix it.
    pub also: &'static [&'static str],
    /// What building it needs that a default build does not.
    pub cost: &'static str,
    /// Which release artifact stops being a static portable binary.
    pub artifact: &'static str,
}

/// The features allowed to need a toolchain, each with the price written down.
///
/// A name here is not permission to be on; it is permission to *exist*, on the
/// condition that nothing turns it on by default. `metal` is deliberately
/// absent: `candle-metal-kernels` declares `build = false` and every build
/// script in the Apple graph is pure Rust with no build-dependencies, so Metal
/// costs no toolchain and being on by default in an Apple build is the design
/// rather than a violation.
pub const GATED: &[Gated] = &[Gated {
    feature: "cuda",
    // candle-core, candle-nn and candle-transformers all define these, and
    // both of them list "cuda" among their own implied features.
    also: &["cudnn", "nccl", "flash-attn"],
    cost: "nvcc and a host C++ compiler at build time (candle-kernels' build \
           script compiles .cu sources with nvcc and links stdc++ and cudart)",
    artifact: "the x86_64 Linux NVIDIA build (glibc-dynamic rather than static musl)",
}];

/// Packages that exist only to build acceleration, and what each one costs.
///
/// This is the *exact* half of the feature question, and it is a table of
/// **packages** rather than of feature names on purpose. [`gated`] has to guess
/// how an upstream spells the features that reach a toolchain — candle alone
/// offers `cuda`, `cudnn`, `nccl` and `flash-attn`, and a crate of ours can
/// forward any of them under a name of its own — and every version of that
/// guess has been incomplete. A package cannot be spelled two ways. If any of
/// these is in the default build's resolved graph, something turned
/// acceleration on, whatever it called it.
pub const ACCELERATED: &[(&str, &str)] = &[
    (
        "candle-kernels",
        "candle's CUDA kernels; its build script runs nvcc over every .cu file",
    ),
    (
        "bindgen_cuda",
        "drives nvcc from a build script, which is why no `cc` ever appears",
    ),
    (
        "candle-flash-attn",
        "flash-attention CUDA kernels, compiled by nvcc at build time",
    ),
    (
        "candle-flash-attn-build",
        "the build script that compiles them",
    ),
    ("cudarc", "links the CUDA driver and runtime libraries"),
    ("ug-cuda", "CUDA code generation"),
];

/// Rule 4, the exact half: no acceleration package in the default build.
///
/// `packages` is what cargo resolved, from
/// [`crate::corpus::default_build_packages`] — feature resolution is cargo's,
/// and this rule reads the answer rather than restating the question. That is
/// what makes it immune to the spelling of a feature, to a feature forwarded
/// across crates, and to a crate cargo treats as a member that our own walk
/// never listed.
pub fn in_default_build(packages: &[String], accelerated: &[(&str, &str)]) -> Vec<Finding> {
    accelerated
        .iter()
        .filter(|(name, _)| packages.iter().any(|p| p == name))
        .map(|(name, why)| Finding {
            rule: Rule::PureRustGraph,
            subject: "the default build".to_string(),
            what: (*name).to_string(),
            why: format!(
                "{why}. A default build of this workspace resolves to it, so the \
                 portable binary needs a CUDA toolchain — whatever feature turned it \
                 on, and whichever manifest named it"
            ),
            line: None,
        })
        .collect()
}

/// The dependency tables a feature can be forced on from.
const DEPENDENCY_TABLES: &[&str] = &["dependencies", "dev-dependencies", "build-dependencies"];

/// Rule 4, second half: a gated feature may not be on in a default build.
///
/// Two shapes, because there are two ways in and closing one leaves the other
/// wide open:
///
/// 1. The transitive closure of `[features] default`. Transitive, because
///    `default = ["everything"]` with `everything = ["cuda"]` is the same
///    build, and a check that read one level would pass it.
/// 2. A `features = [...]` array on any dependency edge, target-conditional
///    ones included. This is not hypothetical: it is exactly the mechanism
///    `ayeaye-infer` uses to turn `metal` on for Apple builds, because a cargo
///    feature nobody passes is off. The way past a `default = []` check is to
///    not use `default`.
///
/// The manifest text is the argument, so a planted violation can be handed to
/// it — the real workspace has no example of one, which is the same reason
/// [`crate::strata::check`] and [`crate::deps::check`] take their tables.
pub fn gated(subject: &str, manifest: &str, gated: &[Gated]) -> Vec<Finding> {
    let Ok(parsed) = manifest.parse::<toml::Table>() else {
        return vec![Finding {
            rule: Rule::PureRustGraph,
            subject: subject.to_string(),
            what: "Cargo.toml".to_string(),
            why: "this manifest does not parse as TOML, so nothing about it was checked; \
                  a rule that cannot read its input must not report the input clean"
                .to_string(),
            line: None,
        }];
    };
    let manifest_value = toml::Value::Table(parsed);

    let mut findings = Vec::new();
    let mut seen = Vec::new();
    let mut note = |enabled: &str, how: &str, findings: &mut Vec<Finding>| {
        // The bare name, or the tail of a `dep/feature` entry.
        let spelling = enabled.rsplit('/').next().unwrap_or(enabled);
        let Some(entry) = gated
            .iter()
            .find(|g| spelling == g.feature || g.also.contains(&spelling))
        else {
            return;
        };
        if seen.contains(&spelling.to_string()) {
            return;
        }
        seen.push(spelling.to_string());
        findings.push(Finding {
            rule: Rule::PureRustGraph,
            subject: subject.to_string(),
            what: spelling.to_string(),
            why: format!(
                "{how}. Building it needs {}. That cost is why {} is the one release \
                 artifact that is not a static portable binary, and the portable build may \
                 not pay it: leave the feature off by default, or amend GATED and say out \
                 loud which artifact stops being static",
                entry.cost, entry.artifact
            ),
            line: None,
        });
    };

    for enabled in default_closure(&manifest_value) {
        note(&enabled, "a default build turns this on", &mut findings);
    }
    for enabled in forced_on_a_dependency(&manifest_value) {
        note(
            &enabled,
            "a dependency edge turns this on with no feature to pass",
            &mut findings,
        );
    }

    findings
}

/// Every feature a default build ends up with, following our own table.
fn default_closure(manifest: &toml::Value) -> Vec<String> {
    let Some(features) = manifest.get("features").and_then(|f| f.as_table()) else {
        return Vec::new();
    };
    let mut queue: Vec<String> = list(features.get("default"));
    let mut reached: Vec<String> = Vec::new();

    while let Some(name) = queue.pop() {
        if reached.contains(&name) {
            continue;
        }
        // `dep/feature` and `dep:name` name someone else's feature and are not
        // keys in our table; only a bare name can recurse.
        if let Some(next) = features.get(&name) {
            queue.extend(list(Some(next)));
        }
        reached.push(name);
    }

    reached
}

/// Every feature named on a dependency edge, in any table, target or not.
fn forced_on_a_dependency(manifest: &toml::Value) -> Vec<String> {
    let mut out = Vec::new();
    collect_dependency_features(manifest, &mut out);
    // The root manifest declares dependencies too, under a table spelled
    // differently from every other one — and it is where this workspace keeps
    // candle, which makes it the most likely place a future edit lands.
    if let Some(workspace) = manifest.get("workspace") {
        collect_dependency_features(workspace, &mut out);
    }
    if let Some(targets) = manifest.get("target").and_then(|t| t.as_table()) {
        for (_, table) in targets {
            collect_dependency_features(table, &mut out);
        }
    }
    out
}

fn collect_dependency_features(table: &toml::Value, out: &mut Vec<String>) {
    for kind in DEPENDENCY_TABLES {
        let Some(entries) = table.get(kind).and_then(|d| d.as_table()) else {
            continue;
        };
        for (_, value) in entries {
            out.extend(list(value.get("features")));
        }
    }
}

/// A TOML array of strings, or nothing.
fn list(value: Option<&toml::Value>) -> Vec<String> {
    value
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

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

#[cfg(test)]
mod gated_tests {
    use super::{GATED, Gated, gated};

    const CUDA: &[Gated] = &[Gated {
        feature: "cuda",
        also: &["cudnn", "nccl", "flash-attn"],
        cost: "nvcc and a host C++ compiler",
        artifact: "the x86_64 Linux NVIDIA build",
    }];

    // AYEAYE-57 — a gated feature is not only reachable under its own name.
    // candle-core defines `cudnn = ["cuda", …]` and `nccl = ["cuda", …]`, so
    // `default = ["candle-core/cudnn"]` puts nvcc in the portable build while
    // naming nothing this rule was watching for. The final gate found this by
    // planting it; the fix is that a gated feature carries the other spellings
    // that turn it on.
    #[test]
    fn a_feature_that_implies_a_gated_one_is_the_same_finding() {
        for spelling in ["cudnn", "nccl"] {
            let manifest = format!("[features]\ndefault = [\"candle-core/{spelling}\"]\n");

            let findings = gated("crates/x/Cargo.toml", &manifest, CUDA);

            assert_eq!(findings.len(), 1, "{spelling}: {findings:?}");
            assert_eq!(findings[0].what, spelling);
        }
    }

    // AYEAYE-57 — the root manifest declares dependencies too, in a table
    // spelled differently from every other one, and it is where this milestone
    // put candle. A rule that only knew `[dependencies]` would pass it.
    #[test]
    fn a_workspace_dependency_forcing_a_gated_feature_is_a_finding() {
        let manifest = "[workspace.dependencies]\n            candle-core = { version = \"0.9.1\", features = [\"cuda\"] }\n";

        let findings = gated("Cargo.toml", manifest, CUDA);

        assert_eq!(findings.len(), 1, "{findings:?}");
        assert_eq!(findings[0].what, "cuda");
    }

    // AYEAYE-57 — the mutation. This is the shape that would put nvcc into the
    // portable build, and the lockfile half of rule 4 cannot see it: the graph
    // carries `bindgen_cuda` either way, and nothing in the cuda path takes
    // `cc`.
    #[test]
    fn a_default_feature_list_naming_a_gated_feature_is_a_finding() {
        let manifest = "[features]\ndefault = [\"cuda\"]\ncuda = [\"candle-core/cuda\"]\n";

        let findings = gated("crates/x/Cargo.toml", manifest, CUDA);

        assert_eq!(findings.len(), 1, "{findings:?}");
        assert_eq!(findings[0].what, "cuda");
        assert!(
            findings[0].why.contains("nvcc"),
            "the cost belongs in the finding: {}",
            findings[0].why
        );
        assert!(
            findings[0].why.contains("NVIDIA build"),
            "and so does the artifact that stops being static: {}",
            findings[0].why
        );
    }

    // AYEAYE-57 — one level of indirection through our own feature table, which
    // a check that only read `default` would walk straight past.
    #[test]
    fn a_default_that_reaches_a_gated_feature_through_another_is_the_same_finding() {
        let manifest = "[features]\ndefault = [\"everything\"]\neverything = [\"fast\"]\n\
                        fast = [\"cuda\"]\ncuda = [\"candle-core/cuda\"]\n";

        let findings = gated("crates/x/Cargo.toml", manifest, CUDA);

        assert_eq!(findings.len(), 1, "{findings:?}");
        assert_eq!(findings[0].what, "cuda");
    }

    // AYEAYE-57 — the way past a `default = []` check is to not use `default`,
    // and this is the exact mechanism AYEAYE-57 uses legitimately for metal.
    // Both halves matter: the gated one is refused, the free one is not.
    #[test]
    fn a_target_table_forcing_a_gated_feature_is_a_finding_and_a_free_one_is_not() {
        let cuda_on_linux = "[features]\ndefault = []\n\
            [target.'cfg(target_os = \"linux\")'.dependencies]\n\
            candle-core = { version = \"0.9\", features = [\"cuda\"] }\n";

        let findings = gated("crates/x/Cargo.toml", cuda_on_linux, CUDA);
        assert_eq!(findings.len(), 1, "{findings:?}");
        assert_eq!(findings[0].what, "cuda");

        let metal_on_macos = "[features]\ndefault = []\n\
            [target.'cfg(target_os = \"macos\")'.dependencies]\n\
            candle-core = { version = \"0.9\", features = [\"metal\"] }\n";

        assert!(
            gated("crates/x/Cargo.toml", metal_on_macos, CUDA).is_empty(),
            "metal needs no toolchain, so turning it on by default is the point"
        );
    }

    // AYEAYE-57 — a plain dependency table is the same hole without the cfg.
    #[test]
    fn an_ordinary_dependency_table_forcing_a_gated_feature_is_a_finding() {
        let manifest =
            "[dependencies]\ncandle-core = { version = \"0.9\", features = [\"cuda\"] }\n";

        assert_eq!(gated("crates/x/Cargo.toml", manifest, CUDA).len(), 1);
    }

    // AYEAYE-57 — naming a gated feature without turning it on is the whole
    // design, not a violation.
    #[test]
    fn declaring_a_gated_feature_without_enabling_it_is_fine() {
        let manifest = "[features]\ndefault = []\ncuda = [\"candle-core/cuda\"]\n\
                        metal = [\"candle-core/metal\"]\n";

        assert!(gated("crates/x/Cargo.toml", manifest, CUDA).is_empty());
    }

    // AYEAYE-57 — the real table has to name something, or the rule is a
    // decoration nobody can trip.
    #[test]
    fn the_real_table_names_cuda_and_says_what_it_costs() {
        let cuda = GATED
            .iter()
            .find(|g| g.feature == "cuda")
            .expect("cuda is the gated feature this milestone accepted");
        assert!(cuda.cost.contains("nvcc"), "{}", cuda.cost);
        assert!(!cuda.artifact.is_empty());
        assert!(
            !GATED.iter().any(|g| g.feature == "metal"),
            "metal costs no toolchain; gating it would be a rule nobody could obey"
        );
    }

    // AYEAYE-57 — a manifest that does not parse must not read as clean.
    #[test]
    fn a_manifest_that_does_not_parse_is_a_finding_rather_than_a_pass() {
        assert!(!gated("crates/x/Cargo.toml", "[features\ndefault =", CUDA).is_empty());
    }
}
