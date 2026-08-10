//! What BSD `ps` is asked, what it prints, and what can be read out of it.
//!
//! The command lines live here rather than beside the code that runs them for
//! the same reason the parsing does: an argv is data, it is decided rather than
//! done, and every one of the three mistakes below fails *silently* — an empty
//! process tree looks exactly like a pane with no agent under it. A test can
//! only stand in front of them where they can be read without a Mac.

use std::collections::BTreeMap;

use super::Source;

/// Ask for every process, its parent, and what it is running.
///
/// Three things about BSD `ps` that a Linux reflex gets wrong:
///
/// - `-o pid=,ppid=,comm=` — an `=` makes the *rest* of the argument the column
///   header, so that asks for one column named `,ppid=,comm=`. One keyword per
///   `-o` is the form that means the same thing to both implementations.
/// - `-ww` — the last column is otherwise clipped to the output width, which
///   `ps` takes from `$COLUMNS` or from whichever of its three streams is still
///   a terminal. With the output captured that is stdin, so the answer would
///   depend on how the server was started, and the clipped part is the
///   interpreter path being matched.
/// - `comm` is the full executable path here, where Linux gives a bare name
///   truncated to fifteen characters.
pub fn snapshot_argv() -> Vec<String> {
    argv(&["ps", "-axww", "-o", "pid=", "-o", "ppid=", "-o", "comm="])
}

/// Ask whether one pid is still there.
///
/// Deliberately its own question rather than a lookup in the snapshot above:
/// the snapshot is taken per walk and this is asked outside one, and a pid that
/// has been gone for a whole snapshot is exactly the pid worth catching.
pub fn liveness_argv(pid: u32) -> Vec<String> {
    argv(&["ps", "-p", &pid.to_string(), "-o", "pid="])
}

/// Ask for one process's environment.
///
/// `-E` is what appends it to the command column; without it this reads a
/// command line and finds nothing, on every Mac, silently. `-ww` because an
/// environment is far past any terminal width.
pub fn environment_argv(pid: u32) -> Vec<String> {
    argv(&["ps", "-ww", "-E", "-p", &pid.to_string(), "-o", "command="])
}

/// Whether [`liveness_argv`]'s output means the process is still there.
///
/// `None` is `ps` not having been asked at all — it is not installed, it could
/// not be started, it timed out. That is not "gone", and the two get opposite
/// answers. The only caller is checking a pid something else handed it, and its
/// alternative to that pid is a guess; when nothing can be learned, the fact
/// already in hand is the better of the two.
pub fn says_alive(output: Option<&str>) -> bool {
    output.is_none_or(|text| !text.trim().is_empty())
}

/// Every process on a Mac, and who its parent is.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tree {
    children: BTreeMap<u32, Vec<u32>>,
    names: BTreeMap<u32, String>,
}

impl Tree {
    /// Read the output of [`snapshot_argv`].
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
}

/// One `ps` snapshot answers a whole walk: three levels of ancestry would
/// otherwise cost a process per node, on the platform where spawning them is
/// slowest.
///
/// These are the tree's only readers, so there is one way to ask it a question
/// and no inherent method quietly shadowing the trait's.
impl Source for Tree {
    /// The pids whose parent is this one, **in the order `ps` listed them** —
    /// which is the order a walk considers them in, and therefore which agent a
    /// shell with two of them resolves to.
    fn children(&self, pid: u32) -> Vec<u32> {
        self.children.get(&pid).cloned().unwrap_or_default()
    }

    /// What this process is called, or `None` if `ps` did not list it at all.
    fn comm(&self, pid: u32) -> Option<String> {
        self.names.get(&pid).cloned()
    }
}

/// The address a process was reached from, out of what [`environment_argv`]
/// printed.
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
    text.split_whitespace()
        .filter_map(|word| word.strip_prefix(NAME))
        .find(|value| !value.is_empty())
        .map(str::to_string)
}

fn argv(words: &[&str]) -> Vec<String> {
    words.iter().map(|word| (*word).to_string()).collect()
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
    use super::{
        Tree, environment_argv, liveness_argv, says_alive, snapshot_argv, ssh_peer_in_command,
    };
    use crate::machine::fixture;
    use crate::process::Source;

    fn codex_tree() -> Tree {
        Tree::parse(fixture!("ps/darwin-codex-tree"))
    }

    // AYEAYE-44 — macOS `ps` reports /opt/homebrew/bin/codex where Linux
    // reports codex, and the name being looked for is the bare one.
    #[test]
    fn a_full_path_is_read_as_the_name_at_the_end_of_it() {
        let tree = codex_tree();
        assert_eq!(tree.comm(702).as_deref(), Some("codex"));
        assert_eq!(tree.children(701), [702]);
    }

    // AYEAYE-44 — /Applications/My Editor.app/... is not exotic on a Mac, and
    // a snapshot that splits on every space mangles the tree from there on.
    #[test]
    fn a_path_with_a_space_in_it_stays_whole() {
        let tree = codex_tree();
        assert_eq!(tree.comm(951).as_deref(), Some("My Editor"));
        assert_eq!(tree.children(950), [951]);
        assert_eq!(tree.children(951), [952]);
    }

    // AYEAYE-44 — an nvm install puts codex 95 columns deep, which is why the
    // wide-output flag is on the command line the shell runs.
    #[test]
    fn a_long_interpreter_path_still_ends_in_the_agent_name() {
        assert_eq!(codex_tree().comm(961).as_deref(), Some("codex"));
    }

    // AYEAYE-44 — a row with no third column still has a pid, and a pid can
    // have children worth walking, so it is not dropped.
    #[test]
    fn a_row_ps_gave_no_name_for_is_kept_with_an_empty_one() {
        assert_eq!(codex_tree().comm(111).as_deref(), Some(""));
    }

    // AYEAYE-44 — most rows on a real Mac look like this: `ps` cannot read the
    // argument buffer of a process it does not own and falls back to the
    // parenthesised accounting name.
    #[test]
    fn the_accounting_names_of_other_users_processes_parse() {
        let tree = codex_tree();
        assert_eq!(tree.comm(222).as_deref(), Some("(logd)"));
        assert_eq!(tree.comm(333).as_deref(), Some("(mdworker_shared)"));
    }

    // AYEAYE-44
    #[test]
    fn a_pid_ps_never_mentioned_has_no_name_and_no_children() {
        let tree = codex_tree();
        assert_eq!(tree.comm(4242), None);
        assert_eq!(tree.children(4242), []);
    }

    // AYEAYE-44 — the walk takes the first match at a level, so the order the
    // children come back in decides which agent a shell with two of them
    // resolves to. `ps` order, not sorted order: the corpus lists pid 1's
    // children out of numeric order precisely so the two can be told apart.
    #[test]
    fn children_come_back_in_the_order_ps_listed_them() {
        assert_eq!(
            codex_tree().children(1),
            [512, 690, 800, 901, 950, 960, 111, 222, 333]
        );
    }

    // AYEAYE-44 — a column header is not a process, and the tree it would
    // otherwise contribute a row to is the pane's own ancestry.
    #[test]
    fn a_header_line_is_not_mistaken_for_a_process() {
        let tree = Tree::parse(fixture!("ps/darwin-with-header"));
        assert_eq!(tree.children(0), [1]);
        assert_eq!(tree.children(690), [701]);
        assert_eq!(tree.children(701), [702]);
        assert_eq!(tree.comm(702).as_deref(), Some("codex"));
    }

    // AYEAYE-44 — the shell decodes `ps` output lossily, so a process whose
    // name is not valid UTF-8 arrives here as a replacement character in the
    // middle of a row. Every row after it still has to parse: one odd process
    // anywhere would otherwise empty the answer for every pane at once.
    #[test]
    fn an_undecodable_name_costs_nothing_after_it() {
        let raw = fixture!("ps/darwin-codex-tree")
            .replace("/usr/bin/vim", "/Users/someone/b\u{fffd}d/v\u{fffd}m");
        let tree = Tree::parse(&raw);
        assert_eq!(tree.comm(902).as_deref(), Some("v\u{fffd}m"));
        assert_eq!(tree.children(950), [951], "the rows after it still parse");
        assert_eq!(tree.comm(961).as_deref(), Some("codex"));
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
    // read hands back SSH_TTY's value as an address. A later one that does
    // have an address is still the answer, which is what the Python's `\S+`
    // does by refusing to match an empty one at all.
    #[test]
    fn an_ssh_connection_with_no_value_is_not_the_next_variable() {
        assert_eq!(ssh_peer_in_command(fixture!("ps/darwin-ssh-empty")), None);
        assert_eq!(
            ssh_peer_in_command(
                "SSH_CONNECTION= SSH_TTY=/dev/ttys004 SSH_CONNECTION=10.0.0.9 1 2 3"
            )
            .as_deref(),
            Some("10.0.0.9")
        );
    }

    // AYEAYE-44 — an `=` makes the rest of a BSD -o argument the column header,
    // so a comma-joined list asks for one column with a silly name and yields a
    // tree with no parents in it.
    #[test]
    fn no_output_keyword_carries_a_comma_joined_list() {
        for word in snapshot_argv()
            .iter()
            .chain(&liveness_argv(802))
            .chain(&environment_argv(802))
        {
            assert!(!word.contains(','), "{word} joins keywords with a comma");
        }
    }

    // AYEAYE-44 — without -ww the last column is clipped to the terminal width,
    // and with the output captured there is no terminal to ask; the clipped
    // part is the interpreter path being matched.
    #[test]
    fn every_query_whose_answer_is_wide_asks_for_unlimited_width() {
        assert_eq!(
            snapshot_argv(),
            ["ps", "-axww", "-o", "pid=", "-o", "ppid=", "-o", "comm="]
        );
        assert_eq!(
            environment_argv(802),
            ["ps", "-ww", "-E", "-p", "802", "-o", "command="],
            "-E is what appends the environment; without it this reads a command line"
        );
    }

    // AYEAYE-44
    #[test]
    fn liveness_asks_about_one_pid_and_nothing_else() {
        assert_eq!(liveness_argv(802), ["ps", "-p", "802", "-o", "pid="]);
    }

    // AYEAYE-44 — "could not tell" and "gone" get opposite answers. The caller
    // is checking a pid something else handed it, and its alternative is a
    // guess at the first client in a list.
    #[test]
    fn a_pid_is_believed_when_ps_could_not_be_asked_at_all() {
        assert!(says_alive(Some(fixture!("ps/darwin-pid-alive"))));
        assert!(says_alive(None), "no answer is not the same as no process");
        assert!(!says_alive(Some("")));
        assert!(!says_alive(Some("\n \n")));
    }
}
