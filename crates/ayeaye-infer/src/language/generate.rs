//! Prompt to text: render, tokenise, decode greedily, detokenise.

use ayeaye_core::cleanup::Policy;
use candle_core::Tensor;

use super::error::LanguageError;
use super::model::LanguageModel;

impl LanguageModel {
    /// Rewrite one dictation, in this process.
    ///
    /// Fallible on purpose, and not the method a dictation path should call on
    /// its own: an `Err` here is a diagnostic, not an outcome. `LanguageSlot`
    /// pairs it with `ayeaye_core::cleanup::settle`, which is what turns any
    /// failure back into the words the speaker said.
    ///
    /// `&mut self` because a transformer decode is not a pure function of its
    /// input: the attention blocks carry a key-value cache that this walks
    /// forward, and drops by starting each generation at position zero.
    pub fn rewrite(&mut self, raw: &str, policy: &Policy) -> Result<String, LanguageError> {
        let prompt = policy.prompt(raw);
        // `false`: the template already spells every special token it wants,
        // and a post-processor that added its own would put a second
        // beginning-of-text marker in front of the one Llama-3's prefix wrote.
        // Added tokens inside the text are recognised either way.
        let encoded = self
            .tokenizer
            .encode(prompt, false)
            .map_err(LanguageError::inference)?;
        let prompt_tokens = encoded.get_ids().to_vec();
        if prompt_tokens.is_empty() {
            return Err(LanguageError::inference(
                "the prompt tokenised to nothing at all",
            ));
        }
        // A position past the rotary table is an out-of-bounds index rather
        // than a worse answer. Refused here, where the number can be named,
        // instead of met as a panic in the middle of somebody's dictation.
        if prompt_tokens.len() >= self.window {
            return Err(LanguageError::inference(format!(
                "the prompt is {} tokens and this model's window is {}",
                prompt_tokens.len(),
                self.window
            )));
        }

        let stops = self.stop_tokens(policy);
        let window = self.window;
        let budget = policy.max_new_tokens;

        let mut fed = prompt_tokens;
        let mut written: Vec<u32> = Vec::new();
        // Zero, and it matters: this is the only thing that clears the previous
        // dictation's attention cache. See `Weights::forward`.
        let mut index = 0usize;

        while written.len() < budget && index + fed.len() < window {
            let input = Tensor::new(fed.as_slice(), self.selection.device())
                .map_err(LanguageError::inference)?
                .unsqueeze(0)
                .map_err(LanguageError::inference)?;
            let logits = self
                .weights
                .forward(&input, index)
                .map_err(LanguageError::inference)?
                .squeeze(0)
                .map_err(LanguageError::inference)?
                .to_vec1::<f32>()
                .map_err(LanguageError::inference)?;

            index += fed.len();

            // Greedy, through the same chooser transcription decodes with.
            // Nothing is suppressed — an instruct model has no equivalent of
            // Whisper's forbidden tokens — but `None` still means the model has
            // nothing legal to say, which ends the generation exactly as a stop
            // token does. Greedy rather than sampled because a rewrite wants the
            // same answer twice; a dictation is not a place for variety.
            let Some(next) = ayeaye_core::logits::best_allowed(&logits, &[]) else {
                break;
            };
            if stops.contains(&next) {
                break;
            }
            written.push(next);
            fed = vec![next];
        }

        self.tokenizer
            .decode(&written, true)
            .map_err(LanguageError::inference)
    }

    /// The token ids that end a generation.
    ///
    /// Resolved per call rather than at load, because the stop markers belong
    /// to the *policy's* template and the policy is configuration: one loaded
    /// model may be prompted through more than one of them.
    ///
    /// A marker the tokenizer does not know is skipped rather than refused. A
    /// ChatML template against a vocabulary with no `<|endoftext|>` is not
    /// broken, it just has one stop instead of two — and the token budget is
    /// the backstop either way.
    fn stop_tokens(&self, policy: &Policy) -> Vec<u32> {
        let mut stops: Vec<u32> = policy
            .template
            .stop
            .iter()
            .filter_map(|marker| self.tokenizer.token_to_id(marker))
            .collect();
        // What the file itself says ends a generation, which is the one stop
        // that does not depend on somebody having configured the right template.
        stops.extend(self.eos);
        stops
    }
}
