//! In-process speech and language inference.
//!
//! Effectful by nature: model files, device memory, and a good deal of time.
//! It sits above `ayeaye-core` and below the binary, and everything pure it
//! discovers belongs in the core rather than here.

pub mod backend;
pub mod language;
pub mod speech;

pub use backend::Backend;
pub use language::{LanguageError, LanguageModel};
pub use speech::{Segment, SpeechError, SpeechModel, SpeechSlot, Transcript};
