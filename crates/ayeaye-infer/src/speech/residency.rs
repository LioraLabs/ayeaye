//! Who owns the model's lifetime, and when it lets go.

use std::path::Path;

use ayeaye_core::Pcm16kMono;

use super::error::SpeechError;
use super::model::SpeechModel;
use super::transcript::Transcript;

/// A place a speech model may or may not be resident.
///
/// The whole point is what it refuses to do: [`SpeechSlot::transcribe`] on an
/// empty slot is an error, **not** a load. A slot that quietly loads on first
/// use has taken out a lifetime nobody wrote down — hundreds of megabytes,
/// held until the process exits, acquired at whatever moment a request
/// happened to arrive — and it never gives it back, because nothing ever said
/// it should. Making both ends explicit is what lets the daemon above decide
/// when a model is worth its memory, which is AYEAYE-56's policy to write.
///
/// `unload` is idempotent and reload after unload works, because that policy
/// will be calling both from somewhere that cannot know the current state.
#[derive(Debug, Default)]
pub struct SpeechSlot {
    model: Option<SpeechModel>,
}

impl SpeechSlot {
    /// A slot with nothing in it.
    pub fn empty() -> Self {
        Self { model: None }
    }

    /// Load a model from `dir`, replacing whatever was resident.
    pub fn load(&mut self, dir: &Path) -> Result<(), SpeechError> {
        // Released before the new one is read, rather than after: holding two
        // models at once is how a reconfiguration doubles the memory of the
        // thing it was reconfiguring.
        self.unload();
        self.model = Some(SpeechModel::load(dir)?);
        Ok(())
    }

    /// Release the resident model, if there is one.
    ///
    /// Returns whether anything was actually released, so a caller can log the
    /// truth rather than an intention.
    pub fn unload(&mut self) -> bool {
        self.model.take().is_some()
    }

    /// Whether a model is resident right now.
    pub fn is_loaded(&self) -> bool {
        self.model.is_some()
    }

    /// Transcribe with the resident model.
    ///
    /// [`SpeechError::NotLoaded`] when the slot is empty. It does not load one.
    pub fn transcribe(&mut self, audio: &Pcm16kMono) -> Result<Transcript, SpeechError> {
        match self.model.as_mut() {
            Some(model) => model.transcribe(audio),
            None => Err(SpeechError::NotLoaded),
        }
    }
}
