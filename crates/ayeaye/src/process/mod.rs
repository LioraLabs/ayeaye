//! Who is running under a pane, and where — the half that touches the machine.
//!
//! Everything below a pane is platform-specific: Linux answers out of `/proc`,
//! macOS has no `/proc` at all and answers out of `ps` and `lsof`. This module
//! is the whole of that difference. What each platform's text *means* is not
//! here — it is in `ayeaye_core::process`, where it can be read against the
//! captured output of a machine nobody here can boot.
//!
//! Seven questions are ever asked, so that is the whole interface: who are this
//! process's children, what is each one called, when did it start, where is it
//! running, what has it got open, is it still there, and what address was it
//! reached from.
//!
//! Nothing here signals, stops, or waits on a process. Every one of these reads
//! and none of them touches.
//!
//! `None` is always "could not find out", never an error: this answers inside a
//! request handler, and a pane whose agent cannot be identified is an ordinary
//! state of the world.

pub mod darwin;
pub mod linux;
pub mod tool;

use ayeaye_core::process::Source;

/// The seven questions, whichever platform is answering them.
///
/// It extends [`Source`] rather than repeating its two, so the walk that finds
/// an agent below a pane is the core's one walk over any backend, and no
/// implementation gets to have its own.
pub trait Processes: Source {
    /// When the process began, in seconds since the epoch.
    fn start_time(&self, pid: u32) -> Option<f64>;

    /// Where it is running.
    fn cwd(&self, pid: u32) -> Option<String>;

    /// Every path it has open, in no particular order.
    ///
    /// Paths rather than a count: a resumed agent session is only resolvable
    /// because the session file it holds open can be found by name.
    fn open_files(&self, pid: u32) -> Vec<String>;

    /// Whether it is still there.
    fn exists(&self, pid: u32) -> bool;

    /// The address it was reached from, or `None` if it is local.
    fn ssh_peer(&self, pid: u32) -> Option<String>;

    /// The nearest process called `name` below `pid`.
    ///
    /// tmux hands out the pane's shell, never the thing running in it, so every
    /// caller starts one level too high.
    fn descendant(&self, pid: u32, name: &str, depth: usize) -> Option<u32> {
        ayeaye_core::process::descendant(self, pid, name, depth)
    }
}

/// The backend for the machine this was built for.
///
/// Chosen by target rather than by trying something and catching what happens:
/// the Linux backend on a Mac would find no `/proc` and answer nothing about
/// every pane, silently, which is the failure mode this whole module exists to
/// remove.
pub fn here() -> Box<dyn Processes> {
    #[cfg(target_os = "macos")]
    {
        Box::new(darwin::Darwin::here())
    }
    #[cfg(not(target_os = "macos"))]
    {
        Box::new(linux::Linux::here())
    }
}

#[cfg(test)]
mod tests {
    use super::here;

    // AYEAYE-44 — the seven questions, asked of the one process this suite is
    // allowed to inspect: itself. Everything else here is a fixture or a tree
    // the test built, so this is the only place the backend this platform
    // actually selected is observed answering about a real process. Read-only,
    // and about nothing it did not start.
    #[test]
    fn this_platforms_backend_answers_about_this_process() {
        let processes = here();
        let ours = std::process::id();

        assert!(processes.exists(ours), "the test process is running");
        assert!(!processes.exists(u32::MAX), "and that pid is not");

        let name = processes.comm(ours).expect("this process has a name");
        assert!(
            !name.is_empty(),
            "a name, whatever the test binary is called"
        );

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("a clock")
            .as_secs_f64();
        let started = processes.start_time(ours).expect("this process began");
        assert!(
            started > 0.0 && started <= now + 1.0,
            "{started} is not a moment at or before {now}"
        );

        let cwd = processes.cwd(ours).expect("this process is somewhere");
        assert!(cwd.starts_with('/'), "an absolute path: {cwd}");

        assert!(
            !processes.open_files(ours).is_empty(),
            "every process holds at least its own streams open"
        );

        // Nothing to assert about the address except that asking is safe: a
        // suite run over SSH has one and a suite run at the machine does not.
        let _ = processes.ssh_peer(ours);

        assert_eq!(
            processes.descendant(ours, "ayeaye-no-such-agent", 3),
            None,
            "nothing by that name is running under this test"
        );
    }
}
