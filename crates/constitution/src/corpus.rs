//! The only part of the constitution that touches a disk.
//!
//! Every rule takes data. This is where the data comes from, kept apart so
//! that a rule can always be handed a synthetic violation instead.

use std::fs;
use std::path::{Path, PathBuf};

use crate::strata::CrateNode;

/// The manifest tables a dependency can be declared in. Development and build
/// tables count: see the note on [`crate::deps::check`].
const DEPENDENCY_TABLES: &[&str] = &["dependencies", "dev-dependencies", "build-dependencies"];

/// The directories under a member that hold its Rust source.
const SOURCE_DIRS: &[&str] = &["src", "tests", "benches", "examples"];

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
}

/// The workspace as the constitution sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Corpus {
    /// The members, in the order the root manifest lists them.
    pub members: Vec<Member>,
}

impl Corpus {
    /// Read the workspace rooted at `root`.
    pub fn walk(root: &Path) -> Result<Self, String> {
        let manifest = parse(&root.join("Cargo.toml"))?;
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

        Ok(Corpus { members })
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
    let manifest = parse(&manifest_path)?;

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
    sources.sort_by(|a, b| a.path.cmp(&b.path));

    Ok(Member {
        name,
        dir: dir.to_string(),
        dependencies,
        sources,
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
        let text = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let relative = path.strip_prefix(root).unwrap_or(&path);
        out.push(Source {
            path: relative.to_string_lossy().into_owned(),
            text,
        });
    }
    Ok(())
}

fn parse(path: &Path) -> Result<toml::Value, String> {
    let text = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    // Parsed as a document rather than as a value: a manifest is a table, and
    // `Value`'s own FromStr wants a single TOML value.
    let table = text
        .parse::<toml::Table>()
        .map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(toml::Value::Table(table))
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
