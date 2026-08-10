//! Text to better text, inside this process.
//!
//! No model server, no `ollama` to install, no runtime to keep running: a
//! directory of files goes in, a dictation is rewritten, and the only thing
//! between them is arithmetic this binary is carrying.
//!
//! Setup could never install the model server this replaces — on every platform
//! it refused outright and printed a URL — so cleanup was unavailable to
//! everyone who did not go and get one themselves. That is what moving it in
//! process buys: reach, not tidiness.

pub mod error;
pub mod model;

pub use error::LanguageError;
pub use model::{LanguageModel, SUPPORTED};
