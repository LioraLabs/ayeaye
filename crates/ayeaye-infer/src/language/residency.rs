//! Who owns the model's lifetime, and the method that cannot lose a dictation.

use std::path::Path;

use ayeaye_core::cleanup::{Cleaned, Policy, settle, worth_cleaning};

use super::error::LanguageError;
use super::model::LanguageModel;

/// A place a language model may or may not be resident.
///
/// The same shape as [`crate::speech::SpeechSlot`], and refusing the same
/// thing: [`LanguageSlot::rewrite`] on an empty slot is an error, **not** a
/// load. A slot that quietly loads on first use has taken out a lifetime nobody
/// wrote down — gigabytes, held until the process exits, acquired at whatever
/// moment a request happened to arrive — and it never gives it back, because
/// nothing ever said it should. Making both ends explicit is what lets the
/// daemon above decide when a model is worth its memory, which is AYEAYE-56's
/// policy to write.
///
/// `unload` is idempotent and reload after unload works, because that policy
/// will be calling both from somewhere that cannot know the current state.
#[derive(Debug, Default)]
pub struct LanguageSlot {
    model: Option<LanguageModel>,
}

impl LanguageSlot {
    /// A slot with nothing in it.
    pub fn empty() -> Self {
        Self { model: None }
    }

    /// Load a model from `dir`, replacing whatever was resident.
    pub fn load(&mut self, dir: &Path) -> Result<(), LanguageError> {
        // Released before the new one is read, rather than after: holding two
        // models at once is how a reconfiguration doubles the memory of the
        // thing it was reconfiguring.
        self.unload();
        self.model = Some(LanguageModel::load(dir)?);
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

    /// Rewrite with the resident model, reporting what went wrong.
    ///
    /// [`LanguageError::NotLoaded`] when the slot is empty. It does not load
    /// one. This is the diagnostic half of the pair — the half worth putting in
    /// a log — and a dictation path should call [`LanguageSlot::clean`] instead,
    /// or pair this with `ayeaye_core::cleanup::settle` itself.
    pub fn rewrite(&mut self, raw: &str, policy: &Policy) -> Result<String, LanguageError> {
        match self.model.as_mut() {
            Some(model) => model.rewrite(raw, policy),
            None => Err(LanguageError::NotLoaded),
        }
    }

    /// Clean up a dictation. **This cannot fail.**
    ///
    /// There is no error type to ignore and no `Result` to unwrap the wrong
    /// way. An empty slot, a corrupt model, a decode that diverged, a model
    /// that answered the question instead of rewriting it — every one of them
    /// arrives at the same place, which is the text the speaker said.
    ///
    /// The one thing it does before reaching for the model is ask whether there
    /// is anything to clean: a blank transcription is seconds of inference to be
    /// told what `settle` already knows, and a model handed a blank dictation
    /// writes one.
    pub fn clean(&mut self, raw: &str, policy: &Policy) -> Cleaned {
        if !worth_cleaning(raw) {
            return settle(policy, raw, None);
        }
        let candidate = self.rewrite(raw, policy).ok();
        settle(policy, raw, candidate.as_deref())
    }
}
