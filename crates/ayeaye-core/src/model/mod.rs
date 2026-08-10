//! Which models ayeaye will run, where they come from, and how long they stay.
//!
//! All of it pure. The network and the filesystem are the shell's; what lives
//! here is the deciding — is this an architecture this build implements, is
//! that a repository id a directory can safely be named after, which file is
//! fetched first, and when a resident model should be let go.
//!
//! The bound this module exists to express is the one that makes ayeaye
//! different from an open-ended model registry: `candle-transformers`
//! implements architectures **individually**, so "this build cannot run that"
//! is a fact knowable from a 2 KB `config.json` rather than a failure met at
//! the first transcription, after a download of hundreds of megabytes.

pub mod architecture;
pub mod config;

pub use architecture::{Architecture, Unsupported};
