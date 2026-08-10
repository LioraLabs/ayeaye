//! The only part of the constitution that touches a disk.
//!
//! Every rule takes data. This is where the data comes from, kept apart so
//! that a rule can always be handed a synthetic violation instead.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::strata::CrateNode;

/// The manifest tables a dependency can be declared in. Development and build
/// tables count: see the note on [`crate::deps::check`].
const DEPENDENCY_TABLES: &[&str] = &["dependencies", "dev-dependencies", "build-dependencies"];

/// The directories under a member that hold its Rust source.
const SOURCE_DIRS: &[&str] = &["src", "tests", "benches", "examples"];

/// Rust source that sits at a member's root rather than under a directory.
/// A build script runs on whatever machine is doing the building, so a crate
/// that carries one is exactly as pure as that script is.
const SOURCE_FILES: &[&str] = &["build.rs"];

/// One Rust source file, as read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    /// Path relative to the workspace root, which is what a finding names.
    pub path: String,
    /// The file's text.
    pub text: String,
}

/// One workspace member.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    /// The package name from its manifest.
    pub name: String,
    /// Its directory, relative to the workspace root.
    pub dir: String,
    /// Every dependency it declares, in every dependency table, deduplicated.
    pub dependencies: Vec<String>,
    /// Every `.rs` file under its `src/` and `tests/`.
    pub sources: Vec<Source>,
    /// The manifest's own text.
    ///
    /// Kept because a manifest says two different kinds of thing and the rules
    /// need both: which crates are depended on (rule 2, already parsed out of
    /// it) and which features a build turns on (rule 4's second half, which
    /// takes the text so it can be handed a planted one instead).
    pub manifest: String,
}

/// The workspace as the constitution sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Corpus {
    /// The members, in the order the root manifest lists them.
    pub members: Vec<Member>,
    /// The root manifest's own text.
    ///
    /// The root is not a member and has no sources, so nothing else here would
    /// read it — and it declares dependencies, in `[workspace.dependencies]`.
    /// A rule over "every member's manifest" would silently skip the one file
    /// this workspace keeps candle in.
    pub root_manifest: String,
}

impl Corpus {
    /// Read the workspace rooted at `root`.
    pub fn walk(root: &Path) -> Result<Self, String> {
        let root_path = root.join("Cargo.toml");
        let root_manifest =
            fs::read_to_string(&root_path).map_err(|e| format!("{}: {e}", root_path.display()))?;
        let manifest = parse_text(&root_manifest, &root_path)?;
        let listed = manifest
            .get("workspace")
            .and_then(|w| w.get("members"))
            .and_then(|m| m.as_array())
            .ok_or_else(|| format!("{} declares no workspace members", root.display()))?;

        let mut members = Vec::new();
        for entry in listed {
            let dir = entry
                .as_str()
                .ok_or_else(|| "a workspace member is not a string".to_string())?;
            if dir.contains('*') {
                // Deliberately unsupported rather than quietly half-read: a
                // glob that matches nothing would empty the corpus, which is
                // the failure this whole module is built to make loud.
                return Err(format!(
                    "workspace member '{dir}' is a glob; the constitution reads literal paths"
                ));
            }
            members.push(read_member(root, dir)?);
        }

        Ok(Corpus {
            members,
            root_manifest,
        })
    }

    /// How many source files the walk found, across every member.
    pub fn file_count(&self) -> usize {
        self.members.iter().map(|m| m.sources.len()).sum()
    }

    /// One member by package name.
    pub fn member(&self, name: &str) -> Option<&Member> {
        self.members.iter().find(|m| m.name == name)
    }

    /// The dependency graph, restricted to edges between workspace members —
    /// which is the only kind the stratum rule has an opinion about.
    pub fn nodes(&self) -> Vec<CrateNode> {
        let names: Vec<&str> = self.members.iter().map(|m| m.name.as_str()).collect();
        self.members
            .iter()
            .map(|member| CrateNode {
                name: member.name.clone(),
                depends_on: member
                    .dependencies
                    .iter()
                    .filter(|dep| names.contains(&dep.as_str()))
                    .cloned()
                    .collect(),
            })
            .collect()
    }
}

/// Read one member: its name, its declared dependencies, and its sources.
fn read_member(root: &Path, dir: &str) -> Result<Member, String> {
    let manifest_path = root.join(dir).join("Cargo.toml");
    let manifest_text = fs::read_to_string(&manifest_path)
        .map_err(|e| format!("{}: {e}", manifest_path.display()))?;
    let manifest = parse_text(&manifest_text, &manifest_path)?;

    let name = manifest
        .get("package")
        .and_then(|p| p.get("name"))
        .and_then(|n| n.as_str())
        .ok_or_else(|| format!("{} names no package", manifest_path.display()))?
        .to_string();

    let mut dependencies = Vec::new();
    collect_dependencies(&manifest, &mut dependencies);
    // Target-specific tables are dependencies too, and a rule that cannot see
    // them is one `[target.'cfg(unix)'.dependencies]` from being bypassed.
    if let Some(targets) = manifest.get("target").and_then(|t| t.as_table()) {
        for (_, table) in targets {
            collect_dependencies(table, &mut dependencies);
        }
    }
    dependencies.sort();
    dependencies.dedup();

    let mut sources = Vec::new();
    for source_dir in SOURCE_DIRS {
        read_sources(root, &root.join(dir).join(source_dir), &mut sources)?;
    }
    for source_file in SOURCE_FILES {
        read_source(root, &root.join(dir).join(source_file), &mut sources)?;
    }
    sources.sort_by(|a, b| a.path.cmp(&b.path));

    Ok(Member {
        name,
        dir: dir.to_string(),
        dependencies,
        sources,
        manifest: manifest_text,
    })
}

/// Every dependency name in `table`'s dependency tables, renames resolved.
fn collect_dependencies(table: &toml::Value, out: &mut Vec<String>) {
    for kind in DEPENDENCY_TABLES {
        let Some(entries) = table.get(kind).and_then(|d| d.as_table()) else {
            continue;
        };
        for (key, value) in entries {
            // `serde_alias = { package = "serde" }` depends on serde; the key
            // is the local name, not the crate.
            let name = value
                .get("package")
                .and_then(|p| p.as_str())
                .unwrap_or(key.as_str());
            out.push(name.to_string());
        }
    }
}

/// Every `.rs` file under `dir`, recursively. A directory that is not there is
/// not an error — most crates have no `benches`.
fn read_sources(root: &Path, dir: &Path, out: &mut Vec<Source>) -> Result<(), String> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Ok(());
    };
    for entry in entries {
        let entry = entry.map_err(|e| format!("{}: {e}", dir.display()))?;
        let path = entry.path();
        if path.is_dir() {
            read_sources(root, &path, out)?;
            continue;
        }
        if path.extension().is_none_or(|ext| ext != "rs") {
            continue;
        }
        read_source(root, &path, out)?;
    }
    Ok(())
}

/// One file, if it is there. A crate without a build script is the normal
/// case, not an error.
fn read_source(root: &Path, path: &Path, out: &mut Vec<Source>) -> Result<(), String> {
    if !path.is_file() {
        return Ok(());
    }
    let text = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let relative = path.strip_prefix(root).unwrap_or(path);
    out.push(Source {
        path: relative.to_string_lossy().into_owned(),
        text,
    });
    Ok(())
}

/// Parse a manifest whose text the caller already has, and keeps.
///
/// Every caller keeps it: rule 4's second half reads features out of the text
/// rather than out of the parse, so reading and parsing are two steps.
fn parse_text(text: &str, path: &Path) -> Result<toml::Value, String> {
    // Parsed as a document rather than as a value: a manifest is a table, and
    // `Value`'s own FromStr wants a single TOML value.
    let table = text
        .parse::<toml::Table>()
        .map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(toml::Value::Table(table))
}

/// Every package a default build of the workspace resolves to, on any target.
///
/// This is the one question about features that cannot be answered by reading
/// files, and the reason it is worth a subprocess: feature resolution is
/// cargo's, and every attempt to restate it here has been wrong. Rule 4's
/// manifest half has to know that `cudnn`, `nccl` and `flash-attn` all imply
/// `cuda`, and that a feature can be forwarded across crates; cargo knows.
///
/// Three flags, each load-bearing and each put there by a bypass someone
/// planted:
///
/// - **`--target all`**, because "the portable build" is a five-row release
///   matrix and the host is one row. A feature forwarded under
///   `[target.'cfg(target_os = "macos")'.dependencies]` puts nvcc into both
///   Apple rows and is invisible to a host resolution on Linux.
/// - **`--edges normal,build`**, because a build dependency is the one edge
///   kind that *definitionally* runs a compiler on the build machine, which is
///   the cost this rule exists for. It is also how two of [`
///   crate::toolchain::ACCELERATED`]'s entries are declared upstream —
///   `bindgen_cuda` and `candle-flash-attn-build` are build-only, so a normal
///   listing could never contain them. Dev edges stay out: a dev dependency is
///   not in the artifact.
/// - **`--locked`**, so asking a question can never rewrite the lockfile as a
///   side effect. A lockfile that no longer matches the manifests fails here
///   rather than being silently regenerated — the same thing
///   `cargo test --locked` says, one gate earlier.
pub fn default_build_packages(root: &Path) -> Result<Vec<String>, String> {
    let output = Command::new(env!("CARGO"))
        .args([
            "tree",
            "--workspace",
            "--edges",
            "normal,build",
            "--target",
            "all",
            "--locked",
            "--prefix",
            "none",
        ])
        .current_dir(root)
        .output()
        .map_err(|e| format!("running cargo tree in {}: {e}", root.display()))?;

    if !output.status.success() {
        return Err(format!(
            "cargo tree failed in {}: {}",
            root.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    let listing = String::from_utf8_lossy(&output.stdout);
    // The first token of each line. With `--prefix none` there are no tree
    // glyphs and no section headings; `(*)` marks a subtree cargo already
    // printed, but it is a line *suffix*, so the name is still the first token
    // and nothing needs filtering out.
    let mut packages: Vec<String> = listing
        .lines()
        .filter_map(|line| line.split_whitespace().next())
        .map(str::to_string)
        .collect();
    packages.sort();
    packages.dedup();
    Ok(packages)
}

/// The root of the workspace this crate is compiled inside.
pub fn workspace_root() -> PathBuf {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest
        .parent()
        .and_then(Path::parent)
        .unwrap_or(manifest)
        .to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::{Corpus, SOURCE_FILES, workspace_root};
    use std::fs;
    use std::path::{Path, PathBuf};

    /// A throwaway workspace on disk. The walk's whole job is reading a tree,
    /// so the only honest way to test it is to give it one.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("constitution-{}-{name}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).expect("a temporary directory");
            Scratch(dir)
        }

        fn write(&self, relative: &str, text: &str) -> &Self {
            let path = self.0.join(relative);
            fs::create_dir_all(path.parent().expect("a parent")).expect("the directory");
            fs::write(&path, text).expect("the file");
            self
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn one_member(name: &str, manifest_tail: &str) -> Scratch {
        let scratch = Scratch::new(name);
        scratch
            .write("Cargo.toml", "[workspace]\nmembers = [\"crates/one\"]\n")
            .write(
                "crates/one/Cargo.toml",
                &format!("[package]\nname = \"one\"\n{manifest_tail}"),
            )
            .write("crates/one/src/lib.rs", "pub fn f() {}\n");
        scratch
    }

    // AYEAYE-41 — a build script runs on the machine doing the building, so a
    // pure crate carrying one is not pure. It sits at the crate root rather
    // than under src/, which is exactly how it stayed invisible.
    #[test]
    fn a_build_script_is_part_of_the_corpus() {
        let scratch = one_member("build-script", "build = \"build.rs\"\n");
        scratch.write("crates/one/build.rs", "fn main() {}\n");
        let corpus = Corpus::walk(scratch.path()).expect("the walk");
        let paths: Vec<&str> = corpus.members[0]
            .sources
            .iter()
            .map(|s| s.path.as_str())
            .collect();
        assert!(
            paths.iter().any(|p| p.ends_with("build.rs")),
            "the build script should be read: {paths:?}"
        );
        assert!(SOURCE_FILES.contains(&"build.rs"));
    }

    // AYEAYE-41 — a rename means the local key is not the crate depended on.
    #[test]
    fn a_renamed_dependency_is_recorded_under_the_crate_it_really_is() {
        let scratch = one_member(
            "rename",
            "\n[dependencies]\nlocal_name = { package = \"tokio\", version = \"1\" }\n",
        );
        let corpus = Corpus::walk(scratch.path()).expect("the walk");
        assert_eq!(corpus.members[0].dependencies, vec!["tokio".to_string()]);
    }

    // AYEAYE-41 — a target-specific table is still a dependency table.
    #[test]
    fn a_target_specific_dependency_is_recorded() {
        let scratch = one_member(
            "target",
            "\n[target.'cfg(unix)'.dev-dependencies]\nlibc = \"0.2\"\n",
        );
        let corpus = Corpus::walk(scratch.path()).expect("the walk");
        assert_eq!(corpus.members[0].dependencies, vec!["libc".to_string()]);
    }

    // AYEAYE-41 — a glob that matched nothing would empty the corpus, which
    // is the silent pass this module exists to prevent.
    #[test]
    fn a_globbed_member_list_is_refused_rather_than_half_read() {
        let scratch = Scratch::new("glob");
        scratch.write("Cargo.toml", "[workspace]\nmembers = [\"crates/*\"]\n");
        let error = Corpus::walk(scratch.path()).expect_err("a glob should be refused");
        assert!(error.contains("glob"), "{error}");
    }

    // AYEAYE-41 — a crate manifest one directory too high would also be a
    // Cargo.toml, and would read as a root with no members at all.
    #[test]
    fn the_workspace_root_is_the_directory_holding_the_workspace_manifest() {
        let manifest = workspace_root().join("Cargo.toml");
        let text = fs::read_to_string(&manifest).expect("the root manifest");
        assert!(
            text.contains("[workspace]"),
            "{} is not the workspace manifest",
            manifest.display()
        );
    }
}
