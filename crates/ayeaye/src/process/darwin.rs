//! `ps`, `lsof`, and one look at the process table.
//!
//! macOS has no `/proc` at all, so five of the seven questions are a tool run
//! and its output parsed. What each answer *means* is in
//! `ayeaye_core::process` — including the command lines, which are data and
//! encode three BSD `ps` traps that all fail silently.
//!
//! The sixth, `start_time`, is the one that is neither: it comes from the
//! process information structure the kernel keeps, through `sysinfo`, rather
//! than through `ps -o lstart=`. That is not tidiness. `lstart` prints a local
//! wall-clock string, and turning it back into a moment in time needs the
//! machine's timezone — which the core may not read and which nothing in this
//! crate should be reinventing. **There is deliberately no `lstart` command
//! line**, and restoring one in the name of parity with the Python would be
//! restoring the round trip this replaces.
//!
//! It is whole seconds, where Linux keeps the fraction. So does `lstart`, and
//! in the same direction: a start time that can only be earlier than the true
//! one moves a session away from the lower edge of the window it is matched
//! inside rather than off it. Worth knowing before anyone narrows that window.

use ayeaye_core::process::{Source, Tree, lsof, ps};
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

use super::Processes;
use super::tool::{Subprocess, Tool};

/// Everything macOS will say about a process.
#[derive(Debug, Clone, Copy, Default)]
pub struct Darwin<T> {
    tool: T,
}

impl Darwin<Subprocess> {
    /// The real `ps` and `lsof`.
    pub fn here() -> Darwin<Subprocess> {
        Darwin {
            tool: Subprocess::default(),
        }
    }
}

impl<T: Tool> Darwin<T> {
    /// One of these, answering with whatever that tool says.
    pub fn asking(tool: T) -> Darwin<T> {
        Darwin { tool }
    }

    /// Every process on the machine, as of now.
    ///
    /// Taken per walk and carried on the stack, never kept on the backend: one
    /// of these is shared by every task of the server, and a half-replaced tree
    /// resolves a pane to the wrong agent or to none at all.
    fn snapshot(&self) -> Tree {
        Tree::parse(&self.tool.run(&ps::snapshot_argv()).unwrap_or_default())
    }
}

impl<T: Tool> Source for Darwin<T> {
    fn children(&self, pid: u32) -> Vec<u32> {
        self.snapshot().children(pid)
    }

    fn comm(&self, pid: u32) -> Option<String> {
        self.snapshot().comm(pid)
    }
}

impl<T: Tool> Processes for Darwin<T> {
    /// Straight out of the kernel's own record of the process, in seconds since
    /// the epoch — no tool, no string, no timezone.
    ///
    /// One pid is refreshed, not the machine: the tutorial call walks every
    /// process there is, which is the cost this whole module is arranged to
    /// avoid, and none of the rest of it is wanted here.
    fn start_time(&self, pid: u32) -> Option<f64> {
        let pid = Pid::from_u32(pid);
        Some(only(pid).process(pid)?.start_time() as f64)
    }

    fn cwd(&self, pid: u32) -> Option<String> {
        lsof::cwd(&self.tool.run(&lsof::cwd_argv(pid))?)
    }

    fn open_files(&self, pid: u32) -> Vec<String> {
        self.tool
            .run(&lsof::names_argv(pid))
            .map(|said| lsof::names(&said))
            .unwrap_or_default()
    }

    fn exists(&self, pid: u32) -> bool {
        ps::says_alive(self.tool.run(&ps::liveness_argv(pid)).as_deref())
    }

    fn ssh_peer(&self, pid: u32) -> Option<String> {
        ps::ssh_peer_in_command(&self.tool.run(&ps::environment_argv(pid))?)
    }

    /// One `ps` for the whole walk.
    ///
    /// Three levels of ancestry asked one process at a time would cost a
    /// process per node, on the platform where starting them is slowest.
    fn descendant(&self, pid: u32, name: &str, depth: usize) -> Option<u32> {
        ayeaye_core::process::descendant(&self.snapshot(), pid, name, depth)
    }
}

/// A process table holding one process, and nothing else about it.
///
/// Its own function so a test can count what it holds. The tutorial call
/// refreshes every process on the machine, which is the cost this whole module
/// is arranged to avoid, and none of the rest of it — memory, disk, threads,
/// the command line — is wanted for a start time.
fn only(pid: Pid) -> System {
    let mut table = System::new();
    table.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[pid]),
        true,
        ProcessRefreshKind::nothing(),
    );
    table
}

#[cfg(test)]
mod tests {
    use super::{Darwin, only};
    use crate::process::Processes;
    use crate::process::tool::Tool;
    use ayeaye_core::process::Source;
    use std::cell::RefCell;

    const CODEX_TREE: &str = include_str!("../../../../tests/fixtures/ps/darwin-codex-tree");
    const OPEN_FILES: &str = include_str!("../../../../tests/fixtures/lsof/darwin-open-files");
    const CWD: &str = include_str!("../../../../tests/fixtures/lsof/darwin-cwd-spaces");
    const DENIED: &str = include_str!("../../../../tests/fixtures/lsof/darwin-denied");
    const ALIVE: &str = include_str!("../../../../tests/fixtures/ps/darwin-pid-alive");
    const SSH_ENVIRON: &str = include_str!("../../../../tests/fixtures/ps/darwin-ssh-environ");

    /// A `ps` and an `lsof` that never ran, answering with what a real Mac
    /// printed and remembering what it was asked.
    ///
    /// Answers are matched on a prefix of the joined argv, which is exactly why
    /// the argv itself is asserted separately: a wrong flag past the prefix
    /// would otherwise go unnoticed, on the platform that cannot be run here.
    #[derive(Default)]
    struct Canned {
        answers: Vec<(&'static str, &'static str)>,
        asked: RefCell<Vec<Vec<String>>>,
    }

    impl Canned {
        fn new(answers: &[(&'static str, &'static str)]) -> Canned {
            Canned {
                answers: answers.to_vec(),
                asked: RefCell::new(Vec::new()),
            }
        }

        fn asked(&self) -> Vec<Vec<String>> {
            self.asked.borrow().clone()
        }
    }

    impl Tool for Canned {
        fn run(&self, argv: &[String]) -> Option<String> {
            self.asked.borrow_mut().push(argv.to_vec());
            let joined = argv.join(" ");
            self.answers
                .iter()
                .filter(|(prefix, _)| joined.starts_with(prefix))
                .max_by_key(|(prefix, _)| prefix.len())
                .map(|(_, answer)| (*answer).to_string())
        }
    }

    fn darwin(answers: &[(&'static str, &'static str)]) -> Darwin<Canned> {
        Darwin::asking(Canned::new(answers))
    }

    fn tree() -> Darwin<Canned> {
        darwin(&[("ps -axww", CODEX_TREE)])
    }

    // AYEAYE-44 — the pane's shell in, the agent under it out, through a `ps`
    // that never ran.
    #[test]
    fn an_agent_is_found_in_the_ps_snapshot() {
        assert_eq!(tree().descendant(701, "codex", 3), Some(702));
        assert_eq!(tree().descendant(801, "codex", 3), Some(803));
        assert_eq!(tree().descendant(901, "codex", 3), None);
    }

    // AYEAYE-44 — the two questions a walk asks, answerable on their own, each
    // out of a snapshot of its own. A caller that wants both for many pids
    // should walk instead, and the cost says so.
    #[test]
    fn a_single_question_takes_a_snapshot_of_its_own() {
        let backend = tree();
        assert_eq!(backend.comm(702).as_deref(), Some("codex"));
        assert_eq!(backend.children(701), [702]);
        assert_eq!(backend.tool.asked().len(), 2);
    }

    // AYEAYE-44 — three levels of ancestry asked one process at a time would
    // cost a process per node, on the platform where starting them is slowest.
    #[test]
    fn the_whole_walk_costs_one_ps() {
        let backend = tree();
        backend.descendant(801, "codex", 3);
        assert_eq!(backend.tool.asked().len(), 1);
    }

    // AYEAYE-44 — one backend is shared by every task of the server, so a tree
    // kept on it is a tree another request can be halfway through replacing,
    // and the pane that lands on it resolves to the wrong agent or to none.
    #[test]
    fn a_second_walk_takes_a_fresh_snapshot() {
        let backend = tree();
        backend.descendant(801, "codex", 3);
        backend.descendant(801, "codex", 3);
        assert_eq!(backend.tool.asked().len(), 2);
    }

    // AYEAYE-44 — the paths, not a count: a resumed session is only resolvable
    // because the file it holds open can be found by name.
    #[test]
    fn open_files_are_the_names_lsof_reported() {
        let backend = darwin(&[("lsof -p", OPEN_FILES)]);
        let open = backend.open_files(702);
        assert!(open.contains(&"/Users/someone/My Sessions/notes.jsonl".to_string()));
        assert!(open.iter().any(|path| path.contains("rollout-2026-03-04")));
        assert_eq!(backend.tool.asked()[0], ["lsof", "-p", "702", "-Fn"]);
    }

    // AYEAYE-44
    #[test]
    fn a_working_directory_comes_from_its_own_narrowed_query() {
        let backend = darwin(&[("lsof -a", CWD)]);
        assert_eq!(
            backend.cwd(702).as_deref(),
            Some("/Users/someone/My Projects/thing")
        );
        assert_eq!(
            backend.tool.asked()[0],
            ["lsof", "-a", "-d", "cwd", "-p", "702", "-Fn"]
        );
    }

    // AYEAYE-44 — a process this user may not look into.
    #[test]
    fn nothing_learned_from_lsof_is_nothing_open_and_nowhere_to_be() {
        assert_eq!(darwin(&[("lsof", DENIED)]).cwd(702), None);
        assert_eq!(darwin(&[]).cwd(702), None);
        assert_eq!(
            darwin(&[("lsof", DENIED)]).open_files(702),
            Vec::<String>::new()
        );
        assert_eq!(darwin(&[]).open_files(702), Vec::<String>::new());
    }

    // AYEAYE-44 — "could not tell" and "gone" get opposite answers, and the
    // question is asked of `ps` rather than of a walk's snapshot: that one
    // carries no proof of liveness.
    #[test]
    fn a_pid_is_believed_unless_ps_looked_and_said_nothing() {
        let backend = darwin(&[("ps -p", ALIVE)]);
        assert!(backend.exists(802));
        assert_eq!(backend.tool.asked()[0], ["ps", "-p", "802", "-o", "pid="]);

        assert!(!darwin(&[("ps -p", "")]).exists(802));
        assert!(
            darwin(&[]).exists(802),
            "no answer at all is not the same as no process"
        );
    }

    // AYEAYE-44 — the address, out of the environment `ps -E` appends, and not
    // out of the walk's snapshot either: that one carries no environment.
    #[test]
    fn the_peer_comes_from_the_environment_of_that_one_process() {
        let backend = darwin(&[("ps -ww -E", SSH_ENVIRON)]);
        assert_eq!(backend.ssh_peer(802).as_deref(), Some("100.101.102.103"));
        assert_eq!(
            backend.tool.asked()[0],
            ["ps", "-ww", "-E", "-p", "802", "-o", "command="]
        );
        assert_eq!(darwin(&[]).ssh_peer(802), None);
    }

    // AYEAYE-44 — neither per-process question may quietly reuse the walk's
    // snapshot, which is both stale and silent about what they need.
    #[test]
    fn no_per_process_question_is_answered_from_the_walks_snapshot() {
        let backend = darwin(&[("ps", CODEX_TREE), ("lsof", OPEN_FILES)]);
        backend.exists(802);
        backend.ssh_peer(802);
        backend.cwd(802);
        backend.open_files(802);
        for argv in backend.tool.asked() {
            assert!(
                !argv.iter().any(|word| word == "-axww"),
                "a per-process question asked for every process: {argv:?}"
            );
        }
    }

    // AYEAYE-44 — the start time comes from the process information structure,
    // not from a wall-clock string that would have to be read back through the
    // machine's timezone. Asked here about the test process itself, which is
    // the one process this suite may inspect and the only way to observe that
    // the structure really answers.
    #[test]
    fn a_start_time_costs_no_ps_at_all() {
        let backend = darwin(&[]);
        let ours = std::process::id();
        let started = backend.start_time(ours).expect("this process has a start");
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("a clock")
            .as_secs_f64();
        assert!(
            started > 0.0 && started <= now + 1.0,
            "{started} is not a moment in the past"
        );
        assert!(
            backend.tool.asked().is_empty(),
            "no tool was run: {:?}",
            backend.tool.asked()
        );
        assert_eq!(backend.start_time(u32::MAX), None, "no such process");
    }

    // AYEAYE-44 — "targeted at the process being asked about" is the whole
    // difference between this and the call that walks the machine, and it is
    // one word in the argument rather than anything visible in the answer.
    #[test]
    fn the_process_table_is_refreshed_for_one_pid_and_no_others() {
        let table = only(sysinfo::Pid::from_u32(std::process::id()));
        assert_eq!(
            table.processes().len(),
            1,
            "every other process on this machine was refreshed too"
        );
    }
}
