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
pub mod hub;
pub mod id;

pub use architecture::{Architecture, Unsupported};
pub use id::{BadModelId, ModelId};

/// The model's own description of its shape and vocabulary.
///
/// These three names are what acquisition writes and what
/// `ayeaye_infer::speech::model` reads back. They are spelled in both places
/// because the core may not depend on the crate above it — so
/// `crates/ayeaye/tests/models.rs` asserts the two agree, since a disagreement
/// would be a pull that lands files no load can find, with nothing saying so.
pub const CONFIG_FILE: &str = "config.json";
/// The vocabulary, in HuggingFace `tokenizers` form.
pub const TOKENIZER_FILE: &str = "tokenizer.json";
/// The weights.
pub const WEIGHTS_FILE: &str = "model.safetensors";
