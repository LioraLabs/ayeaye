//! The constitution: the workspace's rules, in a form that can refuse a build.
//!
//! Every rule takes its input as data — source text, a list of dependency
//! names, a graph of crates — and returns findings. Nothing here reads a file
//! except [`corpus`], which exists precisely so that every other module can be
//! handed a synthetic violation and asked whether it notices. A rule that
//! cannot be shown a planted violation is a rule nobody has tested.
//!
//! The rules themselves, and how to amend one, are written out in this crate's
//! `README.md`. That file is normative; this one is the implementation of it.

pub mod effect_budget;
pub mod finding;
pub mod strip;

pub use finding::{Finding, Rule, report};
