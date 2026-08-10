//! Who owns the model's lifetime, and the method that cannot lose a dictation.

use std::path::Path;

use ayeaye_core::cleanup::{Cleaned, Policy, settle, worth_cleaning};

use crate::backend::{self, Selection};

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
    /// The device decision every model this slot loads is put on. See
    /// [`crate::speech::SpeechSlot`] for why it is held here.
    selection: Option<Selection>,
}

impl LanguageSlot {
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
    /// decision only when none is. See [`crate::speech::SpeechSlot::fallback`].
    pub fn fallback(&self) -> Option<&str> {
        match &self.model {
            Some(model) => model.fallback(),
            None => self.selection.as_ref()?.fallback(),
        }
    }

    /// Load a model from `dir`, replacing whatever was resident.
    pub fn load(&mut self, dir: &Path) -> Result<(), LanguageError> {
        // Released before the new one is read, rather than after: holding two
        // models at once is how a reconfiguration doubles the memory of the
        // thing it was reconfiguring.
        self.unload();
        let selection = self.selection.get_or_insert_with(backend::select).clone();
        self.model = Some(LanguageModel::load_with(dir, selection)?);
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
