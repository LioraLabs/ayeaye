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
