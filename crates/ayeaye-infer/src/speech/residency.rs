//! Who owns the model's lifetime, and when it lets go.

use std::path::Path;

use ayeaye_core::Pcm16kMono;

use crate::backend::{self, Selection};

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
    /// The device decision every model this slot loads is put on.
    ///
    /// Held on the slot rather than made inside `load`, because a slot outlives
    /// the models in it: AYEAYE-56's idle policy unloads and reloads, and
    /// re-asking the machine each time means a card that goes away between two
    /// loads changes the answer under a report that has already been printed.
    /// One probe per slot, whatever the residency policy does.
    selection: Option<Selection>,
}

impl SpeechSlot {
    /// A slot with nothing in it, which will choose a device when first asked
    /// to load one.
    pub fn empty() -> Self {
        Self {
            model: None,
            selection: None,
        }
    }

    /// A slot bound to a device decision already made.
    ///
    /// The decision belongs to the process, not to the model: a daemon holding
    /// a speech slot and a language slot should probe the machine once and hand
    /// the answer to both, so that the acceleration it reported at startup is
    /// the acceleration its models are actually on. [`Self::empty`] makes the
    /// decision on first load instead, which is right for a caller that has
    /// only one slot and no report to keep honest.
    pub fn on(selection: Selection) -> Self {
        Self {
            model: None,
            selection: Some(selection),
        }
    }

    /// Why the resident model is not on the backend the build was compiled for.
    ///
    /// Read off the **model** when one is resident, and off the slot's own
    /// decision only when none is. That is deliberate: answering from the
    /// slot's decision either way would report what this slot intended rather
    /// than what the model in it actually got, and those are the same thing
    /// only for as long as `load` really uses the decision — which is the
    /// property worth being able to test.
    pub fn fallback(&self) -> Option<&str> {
        match &self.model {
            Some(model) => model.fallback(),
            None => self.selection.as_ref()?.fallback(),
        }
    }

    /// Load a model from `dir`, replacing whatever was resident.
    pub fn load(&mut self, dir: &Path) -> Result<(), SpeechError> {
        // Released before the new one is read, rather than after: holding two
        // models at once is how a reconfiguration doubles the memory of the
        // thing it was reconfiguring.
        self.unload();
        let selection = self.selection.get_or_insert_with(backend::select).clone();
        self.model = Some(SpeechModel::load_with(dir, selection)?);
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
