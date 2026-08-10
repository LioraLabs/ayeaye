//! Finding a named agent below a pane's shell.
//!
//! tmux hands out the pane's shell, never the thing running in it, so every
//! caller starts one level too high. The agent is a child of that shell, and
//! sometimes a grandchild through a wrapper — an nvm shim, an interpreter, a
//! login shell somebody set up years ago — so the search goes down rather than
//! looking once.

/// The two questions a walk asks, and the whole of what a platform must answer.
///
/// Deliberately not the other five: those are per-process and are asked about a
/// pid the walk has already found, whereas these are asked about every node it
/// passes. Keeping them apart is what lets macOS answer both from one `ps`
/// snapshot while Linux answers each from a file.
pub trait Source {
    /// The pids whose parent is this one. Empty when there are none, and also
    /// when there was no way to find out — a walk cannot tell those apart and
    /// does not need to.
    fn children(&self, pid: u32) -> Vec<u32>;

    /// What this process is called, or `None` when that could not be read at
    /// all. `None` prunes: a process whose name is unknown may still be an
    /// agent, but nothing below it can be reached through a name.
    fn comm(&self, pid: u32) -> Option<String>;
}

/// How far below the pane's shell an agent is looked for.
///
/// Three levels covers a shell, a wrapper, and the agent under it. Deeper is
/// how a pane resolves to something a user started inside their agent rather
/// than to the agent.
pub const DEPTH: usize = 3;

/// The nearest process called `name` below `pid`, or `None`.
///
/// Breadth first, so the shallowest match wins: the agent sits directly under
/// the pane's shell in the common case, and anything deeper with the same name
/// is something that agent started.
pub fn descendant<S: Source + ?Sized>(
    source: &S,
    pid: u32,
    name: &str,
    depth: usize,
) -> Option<u32> {
    let mut frontier = vec![pid];
    for _ in 0..depth {
        let mut next = Vec::new();
        for parent in frontier {
            for child in source.children(parent) {
                // Gone, or not ours to look at. Either way there is no name to
                // compare and no name below it to reach.
                let Some(comm) = source.comm(child) else {
                    continue;
                };
                if comm == name {
                    return Some(child);
                }
                next.push(child);
            }
        }
        frontier = next;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{DEPTH, Source, descendant};
    use crate::machine::fixture;
    use crate::process::ps::Tree;
    use std::collections::BTreeMap;

    fn codex_tree() -> Tree {
        Tree::parse(fixture!("ps/darwin-codex-tree"))
    }

    /// A tree somebody wrote down, for the cases no captured output holds.
    struct Fake {
        kids: BTreeMap<u32, Vec<u32>>,
        names: BTreeMap<u32, Option<&'static str>>,
    }

    impl Fake {
        /// A chain of `(pid, name)`, each the child of the one before it.
        fn chain(nodes: &[(u32, Option<&'static str>)]) -> Fake {
            let mut fake = Fake {
                kids: BTreeMap::new(),
                names: BTreeMap::new(),
            };
            for pair in nodes.windows(2) {
                fake.kids.entry(pair[0].0).or_default().push(pair[1].0);
            }
            for &(pid, name) in nodes {
                fake.names.insert(pid, name);
            }
            fake
        }
    }

    impl Source for Fake {
        fn children(&self, pid: u32) -> Vec<u32> {
            self.kids.get(&pid).cloned().unwrap_or_default()
        }

        fn comm(&self, pid: u32) -> Option<String> {
            self.names.get(&pid).copied().flatten().map(str::to_string)
        }
    }

    // AYEAYE-44 — the ordinary case: tmux gives the shell, the agent is under
    // it.
    #[test]
    fn a_direct_child_is_found() {
        assert_eq!(descendant(&codex_tree(), 701, "codex", DEPTH), Some(702));
    }

    // AYEAYE-44 — an npm-installed agent runs under a node wrapper, which is
    // what the shell's child actually is.
    #[test]
    fn a_grandchild_is_found_through_a_wrapper() {
        assert_eq!(descendant(&codex_tree(), 801, "codex", DEPTH), Some(803));
    }

    // AYEAYE-44
    #[test]
    fn a_pane_with_no_agent_below_it_resolves_to_nothing() {
        assert_eq!(descendant(&codex_tree(), 901, "codex", DEPTH), None);
    }

    // AYEAYE-44 — deeper than the limit is something the agent started, not
    // the agent.
    #[test]
    fn the_search_stops_at_the_depth_limit() {
        let deep = Fake::chain(&[
            (100, Some("bash")),
            (101, Some("bash")),
            (102, Some("bash")),
            (103, Some("bash")),
            (104, Some("codex")),
        ]);
        assert_eq!(descendant(&deep, 100, "codex", DEPTH), None);
        assert_eq!(descendant(&deep, 100, "codex", 4), Some(104));
    }

    // AYEAYE-44 — a name that could not be read is not a name that is empty:
    // the first prunes, and the second is an ordinary row macOS `ps` prints
    // for a process it could not name but can still parent.
    #[test]
    fn an_unreadable_name_prunes_where_an_empty_one_does_not() {
        let unreadable = Fake::chain(&[(100, Some("bash")), (101, None), (102, Some("codex"))]);
        assert_eq!(descendant(&unreadable, 100, "codex", DEPTH), None);

        let unnamed = Fake::chain(&[(100, Some("bash")), (101, Some("")), (102, Some("codex"))]);
        assert_eq!(descendant(&unnamed, 100, "codex", DEPTH), Some(102));
    }

    // AYEAYE-44 — one odd process among the shell's children must not hide the
    // agent standing next to it.
    #[test]
    fn an_unreadable_sibling_does_not_hide_the_agent() {
        let mut tree = Fake::chain(&[(100, Some("bash")), (101, None)]);
        tree.kids.entry(100).or_default().push(102);
        tree.names.insert(102, Some("codex"));
        assert_eq!(descendant(&tree, 100, "codex", DEPTH), Some(102));
    }

    // AYEAYE-44 — breadth first, not depth first: the agent under the shell,
    // never the one that agent started. The wrapper is listed *first* here, so
    // a search that followed one branch to the bottom would return its child.
    #[test]
    fn the_shallowest_match_wins() {
        let mut tree = Fake::chain(&[(103, Some("bash")), (104, Some("codex"))]);
        tree.kids.insert(100, vec![103, 101]);
        tree.names.insert(100, Some("bash"));
        tree.names.insert(101, Some("codex"));
        assert_eq!(descendant(&tree, 100, "codex", DEPTH), Some(101));
    }
}
