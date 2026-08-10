//! `/proc`, and nothing else.
//!
//! No subprocess anywhere in here. Every one of the seven questions is a file
//! or a symlink named after the pid being asked about, which is what keeps the
//! cost of resolving one pane proportional to that pane rather than to the
//! number of processes on the machine.
//!
//! The root is a field rather than a constant so a test can point it at a tree
//! it built itself. That is not only for convenience: the alternative is a
//! suite that inspects whatever else happens to be running on the machine it
//! is running on.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use ayeaye_core::process::{Source, procfs};

use super::Processes;

/// Everything Linux will say about a process.
#[derive(Debug, Clone)]
pub struct Linux {
    root: PathBuf,
    clk_tck: u64,
}

impl Linux {
    /// The real `/proc`, and this machine's clock tick.
    pub fn here() -> Linux {
        Linux::rooted("/proc", clock_tick())
    }

    /// A `/proc` somewhere else, for a test that would otherwise have to read
    /// the machine it is running on.
    pub fn rooted(root: impl Into<PathBuf>, clk_tck: u64) -> Linux {
        Linux {
            root: root.into(),
            clk_tck,
        }
    }

    fn of(&self, pid: u32) -> PathBuf {
        self.root.join(pid.to_string())
    }

    /// Who names this pid as their parent, asked of every process there is.
    ///
    /// The answer of last resort, and the thing the rest of this module exists
    /// to avoid: it is `pgrep`'s algorithm, and its cost is the machine's
    /// process count rather than the pane's. It runs only where the kernel
    /// publishes no children file at all, because the alternative there is
    /// resolving every pane on that machine to nothing without saying so.
    fn children_by_scan(&self, pid: u32) -> Vec<u32> {
        let Ok(entries) = fs::read_dir(&self.root) else {
            return Vec::new();
        };
        let mut found: Vec<u32> = entries
            .flatten()
            .filter_map(|entry| {
                let candidate: u32 = entry.file_name().to_str()?.parse().ok()?;
                let stat = text(&entry.path().join("stat"))?;
                (procfs::parent(&stat)? == pid).then_some(candidate)
            })
            .collect();
        found.sort_unstable();
        found
    }
}

impl Source for Linux {
    /// The kernel's own answer, unioned over the process's threads.
    ///
    /// Sorted, because the order directories come back in is arbitrary and the
    /// walk takes the first match at a level — an unsorted answer would resolve
    /// a shell with two agents under it to a different one on different runs.
    ///
    /// macOS deliberately does *not* sort: `ps` order is the order the machine
    /// reported, which is information, where directory order is noise. So the
    /// two platforms can pick different agents from a shell running two of the
    /// same name. That is a real divergence and it is the lesser one: the
    /// alternative is a platform that picks a different agent from the same
    /// machine twice in a row.
    fn children(&self, pid: u32) -> Vec<u32> {
        if !self.exists(pid) {
            return Vec::new();
        }
        let mut found = Vec::new();
        let mut published = false;
        if let Ok(threads) = fs::read_dir(self.of(pid).join("task")) {
            for thread in threads.flatten() {
                if let Some(listed) = text(&thread.path().join("children")) {
                    published = true;
                    found.extend(procfs::children(&listed));
                }
            }
        }
        if !published {
            return self.children_by_scan(pid);
        }
        found.sort_unstable();
        found
    }

    fn comm(&self, pid: u32) -> Option<String> {
        Some(text(&self.of(pid).join("comm"))?.trim().to_string())
    }
}

impl Processes for Linux {
    fn start_time(&self, pid: u32) -> Option<f64> {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?;
        procfs::start_time(
            &text(&self.of(pid).join("stat"))?,
            &text(&self.root.join("uptime"))?,
            self.clk_tck,
            now.as_secs_f64(),
        )
    }

    fn cwd(&self, pid: u32) -> Option<String> {
        Some(
            fs::read_link(self.of(pid).join("cwd"))
                .ok()?
                .to_string_lossy()
                .into_owned(),
        )
    }

    /// `/proc/<pid>/fd` is a directory of symlinks and the answer is what they
    /// point at.
    ///
    /// A descriptor closing while the directory is being walked is ordinary —
    /// the process is running — so a link that will not read is dropped rather
    /// than emptying the answer, which would otherwise say "holds nothing
    /// open" about a busy process.
    fn open_files(&self, pid: u32) -> Vec<String> {
        let Ok(entries) = fs::read_dir(self.of(pid).join("fd")) else {
            return Vec::new();
        };
        entries
            .flatten()
            .filter_map(|entry| {
                Some(
                    fs::read_link(entry.path())
                        .ok()?
                        .to_string_lossy()
                        .into_owned(),
                )
            })
            .collect()
    }

    fn exists(&self, pid: u32) -> bool {
        self.of(pid).exists()
    }

    fn ssh_peer(&self, pid: u32) -> Option<String> {
        procfs::ssh_peer(&fs::read(self.of(pid).join("environ")).ok()?)
    }
}

/// `SC_CLK_TCK`: the unit `/proc/<pid>/stat` counts a process's start in.
fn clock_tick() -> u64 {
    // The one call in this module that is not a file read. There is no file
    // that says this, and a wrong constant here moves every start time by a
    // factor rather than failing.
    let ticks = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    u64::try_from(ticks).unwrap_or(0)
}

/// Read a file as text, however it happens to be encoded.
///
/// Lossily, and that is load-bearing: no filesystem enforces valid UTF-8 in a
/// path, and a strict decode would turn one unrelated process into an empty
/// answer for every pane at once.
fn text(path: &Path) -> Option<String> {
    fs::read(path)
        .ok()
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
}

#[cfg(test)]
mod tests {
    use super::Linux;
    use crate::process::Processes;
    use ayeaye_core::process::Source;
    use std::fs;
    use std::os::unix::fs::symlink;
    use std::path::{Path, PathBuf};

    const CLK_TCK: u64 = 100;

    /// A `/proc` of one's own.
    ///
    /// Every question this backend answers is a file at a known name, so a
    /// directory of those files is the whole of what it needs — and building
    /// one is the only way to test the answers without inspecting whatever
    /// else is running on the machine.
    struct Tree(PathBuf);

    impl Tree {
        fn new(name: &str) -> Tree {
            let dir =
                std::env::temp_dir().join(format!("ayeaye-proc-{}-{name}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).expect("a temporary directory");
            fs::write(dir.join("uptime"), "1000.0 9876.54\n").expect("an uptime");
            Tree(dir)
        }

        fn backend(&self) -> Linux {
            Linux::rooted(&self.0, CLK_TCK)
        }

        fn path(&self, pid: u32) -> PathBuf {
            self.0.join(pid.to_string())
        }

        /// One process, shaped the way the kernel shapes it.
        fn process(&self, pid: u32, name: &str, ppid: u32, start_ticks: u64) -> &Tree {
            let dir = self.path(pid);
            fs::create_dir_all(&dir).expect("a process directory");
            let mut fields = vec!["S".to_string(), ppid.to_string()];
            fields.extend(std::iter::repeat_n("0".to_string(), 17));
            fields.push(start_ticks.to_string());
            fields.extend(std::iter::repeat_n("7".to_string(), 30));
            fs::write(
                dir.join("stat"),
                format!("{pid} ({name}) {}\n", fields.join(" ")),
            )
            .expect("a stat file");
            fs::write(dir.join("comm"), format!("{name}\n")).expect("a comm file");
            self
        }

        /// What the kernel publishes for one of a process's threads.
        fn thread_children(&self, pid: u32, tid: u32, kids: &str) -> &Tree {
            let dir = self.path(pid).join("task").join(tid.to_string());
            fs::create_dir_all(&dir).expect("a task directory");
            fs::write(dir.join("children"), format!("{kids}\n")).expect("a children file");
            self
        }

        fn cwd(&self, pid: u32, target: &str) -> &Tree {
            symlink(target, self.path(pid).join("cwd")).expect("a cwd link");
            self
        }

        fn open(&self, pid: u32, targets: &[&str]) -> &Tree {
            let dir = self.path(pid).join("fd");
            fs::create_dir_all(&dir).expect("a descriptor directory");
            // Numbered from where a process's first open file really is.
            for (offset, target) in targets.iter().enumerate() {
                symlink(target, dir.join((offset + 3).to_string())).expect("a descriptor");
            }
            self
        }

        fn environ(&self, pid: u32, entries: &[&str]) -> &Tree {
            let mut block = Vec::new();
            for entry in entries {
                block.extend_from_slice(entry.as_bytes());
                block.push(0);
            }
            fs::write(self.path(pid).join("environ"), block).expect("an environ file");
            self
        }

        fn write(&self, path: impl AsRef<Path>, bytes: impl AsRef<[u8]>) {
            fs::write(self.0.join(path), bytes).expect("a file");
        }
    }

    impl Drop for Tree {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    // AYEAYE-44 — the kernel's own answer, and it is per *thread*: whichever
    // thread called fork is the one that lists the child, so a backend that
    // reads only the first loses the rest silently.
    #[test]
    fn children_are_unioned_across_every_thread() {
        let tree = Tree::new("children");
        tree.process(100, "bash", 1, 0);
        tree.thread_children(100, 100, "101 102");
        tree.thread_children(100, 140, "103");
        let mut got = tree.backend().children(100);
        got.sort_unstable();
        assert_eq!(got, [101, 102, 103], "no thread's children were missed");
    }

    // AYEAYE-44 — the walk takes the first match at a level, so the order
    // decides which agent a shell running two of the same name resolves to. The
    // kernel's own order across threads is directory order, which is arbitrary,
    // so leaving it alone means answering differently on different runs of the
    // same machine. One thread, listing out of order, because two threads would
    // make this test agree by luck half the time.
    #[test]
    fn children_come_back_in_a_stable_order() {
        let tree = Tree::new("stable");
        tree.process(100, "bash", 1, 0);
        tree.thread_children(100, 100, "103 101 102");
        assert_eq!(tree.backend().children(100), [101, 102, 103]);
    }

    // AYEAYE-44 — a process with no children is the common case, and the
    // fallback must not be what answers for it: a scan per childless pid, once
    // per node per level of a walk, is the whole-machine cost this backend is
    // arranged to avoid. An empty children file is an answer, not a silence.
    #[test]
    fn an_empty_children_file_is_an_answer_and_not_a_reason_to_scan() {
        let tree = Tree::new("leaf");
        tree.process(100, "bash", 1, 0);
        tree.thread_children(100, 100, "");
        // Only a scan could find this one, and its stat says 100 is its parent.
        tree.process(101, "codex", 100, 0);
        assert_eq!(
            tree.backend().children(100),
            [],
            "the kernel said none, and nothing went looking anyway"
        );
    }

    // AYEAYE-44 — the targeted read is the point of this backend: it costs two
    // file reads rather than a walk of every process on the machine. The tree
    // here contains a process whose stat claims 100 as its parent and which
    // the children file does not list; a scan would find it and the read does
    // not.
    #[test]
    fn the_children_file_answers_rather_than_a_scan_of_every_process() {
        let tree = Tree::new("targeted");
        tree.process(100, "bash", 1, 0);
        tree.process(101, "codex", 100, 0);
        tree.process(102, "other", 100, 0);
        tree.thread_children(100, 100, "101");
        assert_eq!(
            tree.backend().children(100),
            [101],
            "102 is only reachable by scanning, and nothing scanned"
        );
    }

    // AYEAYE-44 — a kernel built without CONFIG_PROC_CHILDREN publishes no
    // such file, and answering "no children" there would resolve every pane on
    // that machine to nothing, silently.
    #[test]
    fn a_kernel_that_publishes_no_children_file_falls_back_to_a_scan() {
        let tree = Tree::new("fallback");
        tree.process(100, "bash", 1, 0);
        tree.process(101, "codex", 100, 0);
        tree.process(102, "vim", 999, 0);
        assert_eq!(tree.backend().children(100), [101]);
    }

    // AYEAYE-44 — a pid that is not there is answered without looking at
    // anything else. The tree here holds a process whose stat still names the
    // gone pid as its parent, which a scan would find and report as a child of
    // a process that no longer exists.
    #[test]
    fn a_process_that_is_gone_has_no_children_and_no_name() {
        let tree = Tree::new("gone");
        tree.process(101, "codex", 4242, 0);
        assert_eq!(tree.backend().children(4242), []);
        assert_eq!(tree.backend().comm(4242), None);
    }

    // AYEAYE-44 — comm is the tail of whatever path was exec'd, bytes and all.
    // The blast radius is not one pane: a whole board is resolved in one
    // request, so one odd process anywhere would turn it into an error.
    #[test]
    fn a_name_that_is_not_utf8_does_not_end_the_request() {
        let tree = Tree::new("comm");
        tree.process(42, "codex", 1, 0);
        assert_eq!(tree.backend().comm(42).as_deref(), Some("codex"));
        tree.write("42/comm", b"co\xffdex\n");
        let got = tree.backend().comm(42).expect("a name, however odd");
        assert_ne!(got, "codex");
        assert!(got.contains("co"), "the readable part survives: {got}");
    }

    // AYEAYE-44 — up for 1000s, started 400s after boot, so it began 600s ago.
    #[test]
    fn a_start_time_is_read_from_stat_and_uptime() {
        let tree = Tree::new("start");
        tree.process(42, "codex", 1, 400 * CLK_TCK);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("a clock")
            .as_secs_f64();
        let got = tree.backend().start_time(42).expect("a start time");
        assert!((got - (now - 600.0)).abs() < 1.0, "{got} is not 600s ago");
        assert_eq!(tree.backend().start_time(4242), None);
    }

    // AYEAYE-44
    #[test]
    fn a_working_directory_is_the_symlink_it_points_at() {
        let tree = Tree::new("cwd");
        tree.process(42, "codex", 1, 0);
        tree.cwd(42, "/home/someone/My Projects/thing");
        assert_eq!(
            tree.backend().cwd(42).as_deref(),
            Some("/home/someone/My Projects/thing")
        );
        assert_eq!(tree.backend().cwd(4242), None);
    }

    // AYEAYE-44 — the paths, not a count.
    #[test]
    fn open_files_are_every_descriptor_symlink() {
        let tree = Tree::new("open");
        tree.process(42, "codex", 1, 0);
        tree.open(
            42,
            &[
                "/home/someone/one.jsonl",
                "/home/someone/My Files/two.jsonl",
            ],
        );
        let mut got = tree.backend().open_files(42);
        got.sort();
        assert_eq!(
            got,
            [
                "/home/someone/My Files/two.jsonl",
                "/home/someone/one.jsonl"
            ]
        );
        assert_eq!(tree.backend().open_files(4242), Vec::<String>::new());
    }

    // AYEAYE-44 — the process under inspection is running, so descriptors close
    // while the directory is being read. Answering "holds nothing open" there
    // sends the pane to a fallback for no reason.
    #[test]
    fn a_descriptor_that_cannot_be_read_does_not_empty_the_answer() {
        let tree = Tree::new("closing");
        tree.process(42, "codex", 1, 0);
        tree.open(42, &["/home/someone/one.jsonl"]);
        // Not a symlink at all, which is what a descriptor that went away
        // between the listing and the read looks like from here.
        tree.write("42/fd/9", b"");
        assert_eq!(
            tree.backend().open_files(42),
            ["/home/someone/one.jsonl"],
            "one unreadable descriptor costs only itself"
        );
    }

    // AYEAYE-44
    #[test]
    fn a_process_exists_when_it_has_a_directory() {
        let tree = Tree::new("exists");
        tree.process(42, "codex", 1, 0);
        assert!(tree.backend().exists(42));
        assert!(!tree.backend().exists(4242));
    }

    // AYEAYE-44 — the block is NUL-separated, which is what makes a value with
    // spaces in it readable whole.
    #[test]
    fn the_peer_is_read_out_of_the_environment_block() {
        let tree = Tree::new("peer");
        tree.process(42, "codex", 1, 0);
        tree.environ(
            42,
            &[
                "OLD_SSH_CONNECTION=10.0.0.1 51000 10.0.0.2 22",
                "SSH_CONNECTION=100.101.102.103 54321 100.64.0.1 22",
            ],
        );
        assert_eq!(
            tree.backend().ssh_peer(42).as_deref(),
            Some("100.101.102.103")
        );

        tree.process(43, "codex", 1, 0);
        tree.environ(43, &["HOME=/home/someone"]);
        assert_eq!(tree.backend().ssh_peer(43), None);
        assert_eq!(tree.backend().ssh_peer(4242), None);
    }

    // AYEAYE-44 — the whole point of the two questions above: a pane's shell
    // in, the agent under it out.
    #[test]
    fn an_agent_is_found_below_a_pane_shell() {
        let tree = Tree::new("walk");
        tree.process(100, "bash", 1, 0);
        tree.process(101, "node", 100, 0);
        tree.process(102, "codex", 101, 0);
        tree.thread_children(100, 100, "101");
        tree.thread_children(101, 101, "102");
        assert_eq!(tree.backend().descendant(100, "codex", 3), Some(102));
        assert_eq!(tree.backend().descendant(100, "claude", 3), None);
    }
}
