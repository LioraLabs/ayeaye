//! The project picker's decisions.
//!
//! The picker walks the filesystem while somebody types on a phone, so every
//! part of that walk is bounded — and every bound, every ranking rung and the
//! pick history that feeds them are decisions rather than effects. They live
//! here, where a test reaches them without a machine; the walking itself, the
//! clock and the store on disk are the shell's.

pub mod json;
pub mod rank;
pub mod recents;
pub mod session;
pub mod skip;
