//! Who is running under a pane, and where — the half of that question that is
//! text rather than machinery.
//!
//! Seven questions are ever asked of a process: who are its children, what is
//! each one called, when did it start, where is it running, what has it got
//! open, is it still there, and what address was it reached from. Answering
//! them means reading `/proc` on Linux and running `ps` and `lsof` on macOS,
//! and none of that may happen here. What lives here is everything that is
//! decided rather than done: the shapes those two platforms print, turned into
//! a tree and a handful of values, and the walk that finds a named agent below
//! a pane's shell.
//!
//! That split is the point. The platform half cannot be executed on the host
//! that runs this suite — nobody here can boot a Mac — so the captured output
//! under `tests/fixtures` is the contract, and it is exercised in this crate,
//! where a test needs no machine at all.
//!
//! `None` means "could not find out", never an error: this answers inside a
//! request handler, and a pane whose agent cannot be identified is an ordinary
//! state of the world.

pub mod lsof;
pub mod procfs;
pub mod ps;
pub mod walk;

pub use ps::Tree;
pub use walk::{DEPTH, Source, descendant};
