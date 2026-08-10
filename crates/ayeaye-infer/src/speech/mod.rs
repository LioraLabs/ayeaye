//! Speech to text, inside this process.
//!
//! No transcription server, no `whisper-cli` subprocess, no model runtime to
//! install: a directory of files goes in, a [`Transcript`] comes out, and the
//! only thing between them is arithmetic this binary is carrying.

pub mod error;
pub mod model;
pub mod residency;
pub mod transcribe;
pub mod transcript;

pub use error::SpeechError;
pub use model::SpeechModel;
pub use residency::SpeechSlot;
pub use transcript::{Segment, Transcript};
