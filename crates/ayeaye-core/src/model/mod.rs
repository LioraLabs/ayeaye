//! Which models ayeaye will run, and what they are told.
//!
//! All of it pure, and much less of it than there was. Acquisition, the
//! architecture check, the hub, residency and the on-disk verification all left
//! with AYEAYE-101: the weights live behind `llama-swap` now, so which files a
//! model needs, whether this build implements its architecture, and when to let
//! go of it are all questions for the process that loads it. What is left is
//! the part that was always ayeaye's own — which model plays which part, and
//! what the cleanup model is told it is for.

pub mod settings;

/// The job a model can do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// Turn audio into text.
    Speech,
    /// Rewrite a raw transcript.
    Cleanup,
}

impl std::fmt::Display for Role {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        out.write_str(match self {
            Self::Speech => "speech",
            Self::Cleanup => "cleanup",
        })
    }
}
