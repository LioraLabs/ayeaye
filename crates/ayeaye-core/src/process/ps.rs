//! What BSD `ps` prints, and what can be read out of it.

use std::collections::BTreeMap;

use super::Source;

/// Every process on a Mac, and who its parent is.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tree {
    children: BTreeMap<u32, Vec<u32>>,
    names: BTreeMap<u32, String>,
}

impl Tree {
    /// Read the output of `ps -axww -o pid= -o ppid= -o comm=`.
    ///
    /// Three columns, right-aligned numbers and then a path that may itself
    /// contain spaces — so only the first two are split on whitespace and the
    /// rest of the line is one field. A line that does not begin with two
    /// numbers is not a process: that is what makes a column header, a blank
    /// line and a warning `ps` printed to the same stream all disappear here
    /// rather than becoming a row nothing can walk.
    pub fn parse(text: &str) -> Tree {
        let mut tree = Tree::default();
        for line in text.lines() {
            let Some((pid, rest)) = field(line) else {
                continue;
            };
            let Some((ppid, rest)) = field(rest) else {
                continue;
            };
            let (Ok(pid), Ok(ppid)) = (pid.parse::<u32>(), ppid.parse::<u32>()) else {
                continue;
            };
            tree.names.insert(pid, name_of(rest.trim()).to_string());
            tree.children.entry(ppid).or_default().push(pid);
        }
        tree
    }

    /// The pids whose parent is this one, in the order `ps` listed them.
    pub fn children(&self, pid: u32) -> &[u32] {
        self.children.get(&pid).map_or(&[], Vec::as_slice)
    }

    /// What this process is called, or `None` if `ps` did not list it at all.
    pub fn comm(&self, pid: u32) -> Option<&str> {
        self.names.get(&pid).map(String::as_str)
    }
}

/// One `ps` snapshot answers a whole walk: three levels of ancestry would
/// otherwise cost a process per node, on the platform where spawning them is
/// slowest.
impl Source for Tree {
    fn children(&self, pid: u32) -> Vec<u32> {
        Tree::children(self, pid).to_vec()
    }

    fn comm(&self, pid: u32) -> Option<String> {
        Tree::comm(self, pid).map(str::to_string)
    }
}

/// The address a process was reached from, out of what `ps -ww -E` printed.
///
/// `-E` appends the environment to the command column, space separated — so
/// unlike Linux's NUL-separated block, a value with a space in it cannot be
/// recovered whole here. That is why this answers one narrow question rather
/// than offering a general environment: the field wanted is an address, an
/// address has no spaces, and both platforms can therefore return exactly the
/// same answer. A general reader would have had to truncate a value on one
/// platform and not the other, quietly.
///
/// The name is anchored to a token boundary, so neither `OLD_SSH_CONNECTION`
/// nor `SSH_CLIENT` answers for it, and the value stops at the first space, so
/// an `SSH_CONNECTION` that is set but empty does not hand back the variable
/// after it as an address — which the caller would go on to dial.
pub fn ssh_peer_in_command(text: &str) -> Option<String> {
    const NAME: &str = "SSH_CONNECTION=";
    let value = text
        .split_whitespace()
        .find_map(|word| word.strip_prefix(NAME))?;
    if value.is_empty() {
        return None;
    }
    Some(value.to_string())
}

/// The first whitespace-separated word, and everything after it.
///
/// `None` only when there is no word left at all, so a row `ps` printed with
/// two columns and nothing else still parses — it has a pid, and a pid can
/// have children worth walking.
fn field(text: &str) -> Option<(&str, &str)> {
    let text = text.trim_start();
    if text.is_empty() {
        return None;
    }
    match text.find(char::is_whitespace) {
        Some(end) => Some((&text[..end], &text[end..])),
        None => Some((text, "")),
    }
}

/// The last path component, which is what Linux would have called the process.
///
/// macOS reports the whole executable path and Linux reports a bare name
/// truncated to fifteen characters. The name being searched for is the bare
/// one, so this is where the two platforms are made to agree.
fn name_of(path: &str) -> &str {
    match path.rfind('/') {
        Some(slash) => &path[slash + 1..],
        None => path,
    }
}

#[cfg(test)]
mod tests {
    use super::{Tree, ssh_peer_in_command};
    use crate::machine::fixture;

    fn codex_tree() -> Tree {
        Tree::parse(fixture!("ps/darwin-codex-tree"))
    }

    // AYEAYE-44 — macOS `ps` reports /opt/homebrew/bin/codex where Linux
    // reports codex, and the name being looked for is the bare one.
    #[test]
    fn a_full_path_is_read_as_the_name_at_the_end_of_it() {
        let tree = codex_tree();
        assert_eq!(tree.comm(702), Some("codex"));
        assert_eq!(tree.children(701), [702]);
    }

    // AYEAYE-44 — /Applications/My Editor.app/... is not exotic on a Mac, and
    // a snapshot that splits on every space mangles the tree from there on.
    #[test]
    fn a_path_with_a_space_in_it_stays_whole() {
        let tree = codex_tree();
        assert_eq!(tree.comm(951), Some("My Editor"));
        assert_eq!(tree.children(950), [951]);
        assert_eq!(tree.children(951), [952]);
    }

    // AYEAYE-44 — an nvm install puts codex 95 columns deep, which is why the
    // wide-output flag is on the command line the shell runs.
    #[test]
    fn a_long_interpreter_path_still_ends_in_the_agent_name() {
        assert_eq!(codex_tree().comm(961), Some("codex"));
    }

    // AYEAYE-44 — a row with no third column still has a pid, and a pid can
    // have children worth walking, so it is not dropped.
    #[test]
    fn a_row_ps_gave_no_name_for_is_kept_with_an_empty_one() {
        assert_eq!(codex_tree().comm(111), Some(""));
    }

    // AYEAYE-44 — most rows on a real Mac look like this: `ps` cannot read the
    // argument buffer of a process it does not own and falls back to the
    // parenthesised accounting name.
    #[test]
    fn the_accounting_names_of_other_users_processes_parse() {
        let tree = codex_tree();
        assert_eq!(tree.comm(222), Some("(logd)"));
        assert_eq!(tree.comm(333), Some("(mdworker_shared)"));
    }

    // AYEAYE-44
    #[test]
    fn a_pid_ps_never_mentioned_has_no_name_and_no_children() {
        let tree = codex_tree();
        assert_eq!(tree.comm(4242), None);
        assert_eq!(tree.children(4242), []);
    }

    // AYEAYE-44 — a column header is not a process, and the tree it would
    // otherwise contribute a row to is the pane's own ancestry.
    #[test]
    fn a_header_line_is_not_mistaken_for_a_process() {
        let tree = Tree::parse(fixture!("ps/darwin-with-header"));
        assert_eq!(tree.children(0), [1]);
        assert_eq!(tree.children(690), [701]);
        assert_eq!(tree.children(701), [702]);
        assert_eq!(tree.comm(702), Some("codex"));
    }

    // AYEAYE-44 — no filesystem enforces valid UTF-8 in a name, so the shell
    // decodes lossily; one odd process anywhere must not empty the answer for
    // every pane at once.
    #[test]
    fn an_undecodable_path_elsewhere_does_not_empty_the_tree() {
        let raw = fixture!("ps/darwin-codex-tree")
            .replace("/usr/bin/vim", "/Users/someone/b\u{fffd}d/vim");
        let tree = Tree::parse(&raw);
        assert_eq!(tree.comm(702), Some("codex"));
        assert_eq!(tree.comm(902), Some("vim"));
    }

    // AYEAYE-44 — the same answer Linux gives, out of the block `ps -E`
    // appends to the command column.
    #[test]
    fn the_peer_comes_out_of_the_process_environment() {
        assert_eq!(
            ssh_peer_in_command(fixture!("ps/darwin-ssh-environ")).as_deref(),
            Some("100.101.102.103")
        );
    }

    // AYEAYE-44 — a client sitting at the machine.
    #[test]
    fn a_local_client_has_no_peer() {
        assert_eq!(
            ssh_peer_in_command(fixture!("ps/darwin-local-environ")),
            None
        );
        assert_eq!(ssh_peer_in_command(""), None);
    }

    // AYEAYE-44 — the fixture carries an OLD_SSH_CONNECTION with a different
    // address ahead of the real one, and a loose match returns the wrong one.
    // SSH_CLIENT is there for the same reason.
    #[test]
    fn a_variable_whose_name_merely_ends_in_it_is_not_it() {
        let text = fixture!("ps/darwin-ssh-environ");
        assert!(text.contains("OLD_SSH_CONNECTION=10.0.0.1"));
        assert_eq!(
            ssh_peer_in_command(text).as_deref(),
            Some("100.101.102.103"),
            "10.0.0.1 is OLD_SSH_CONNECTION's, and dialling it is the bug"
        );
    }

    // AYEAYE-44 — this block is space separated, so an SSH_CONNECTION with
    // nothing in it is followed immediately by the next variable; a hungrier
    // read hands back SSH_TTY's value as an address.
    #[test]
    fn an_ssh_connection_with_no_value_is_not_the_next_variable() {
        assert_eq!(ssh_peer_in_command(fixture!("ps/darwin-ssh-empty")), None);
    }
}
