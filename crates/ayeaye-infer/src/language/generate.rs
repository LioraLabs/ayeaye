//! Prompt to text: render, tokenise, decode greedily, detokenise.

use ayeaye_core::cleanup::Policy;
use candle_core::Tensor;
use candle_transformers::models::quantized_llama::MAX_SEQ_LEN;

use super::error::LanguageError;
use super::model::LanguageModel;

impl LanguageModel {
    /// Rewrite one dictation, in this process.
    ///
    /// Fallible on purpose, and never the method a dictation path should call
    /// on its own: an `Err` here is a diagnostic, not an outcome. `LanguageSlot`
    /// pairs it with `ayeaye_core::cleanup::settle`, which is what turns any
    /// failure back into the words the speaker said.
    ///
    /// `&self` rather than `&mut self`, unlike transcription: each generation
    /// runs on its own copy of the weights, so nothing about the model changes.
    /// See [`LanguageModel::fresh`] for why that copy exists.
    pub fn rewrite(&self, raw: &str, policy: &Policy) -> Result<String, LanguageError> {
        let prompt = policy.prompt(raw);
        // `false`: the template already spells every special token it wants,
        // and a post-processor that adds its own would put a second
        // beginning-of-text marker in front of the one Llama-3's prefix wrote.
        // Added tokens inside the text are still recognised either way.
        let encoded = self
            .tokenizer
            .encode(prompt, false)
            .map_err(LanguageError::inference)?;
        let prompt_tokens = encoded.get_ids();
        if prompt_tokens.is_empty() {
            return Err(LanguageError::inference(
                "the prompt tokenised to nothing at all",
            ));
        }
        // candle's rope table for this architecture is `MAX_SEQ_LEN` long
        // whatever the file's own context length says, so a longer prompt does
        // not degrade — it indexes past the end. Refused here, where the number
        // can be named, rather than met as a panic mid-dictation.
        if prompt_tokens.len() >= MAX_SEQ_LEN {
            return Err(LanguageError::inference(format!(
                "the prompt is {} tokens and this architecture's window is {MAX_SEQ_LEN}",
                prompt_tokens.len()
            )));
        }

        let stops = self.stop_tokens(policy);
        // The whole reason this is a copy: the layers carry a key-value cache
        // and candle 0.9 offers no way to clear it, so generating twice through
        // one set of weights would let the first dictation attend to the second.
        let mut weights = self.fresh();

        let mut fed = prompt_tokens.to_vec();
        let mut written: Vec<u32> = Vec::new();
        let mut index = 0usize;

        while written.len() < policy.max_new_tokens && index + fed.len() < MAX_SEQ_LEN {
            let input = Tensor::new(fed.as_slice(), &self.device)
                .map_err(LanguageError::inference)?
                .unsqueeze(0)
                .map_err(LanguageError::inference)?;
            let logits = weights
                .forward(&input, index)
                .map_err(LanguageError::inference)?
                .squeeze(0)
                .map_err(LanguageError::inference)?
                .to_vec1::<f32>()
                .map_err(LanguageError::inference)?;

            index += fed.len();

            // Greedy, through the same chooser transcription decodes with.
            // Nothing is suppressed here — an instruct model has no equivalent
            // of Whisper's forbidden tokens — but `None` still means the model
            // has nothing legal to say, which ends the generation exactly as a
            // stop token does.
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
    /// to the *policy's* template and the policy is configuration: a model
    /// loaded once may be prompted through more than one of them.
    ///
    /// A marker the tokenizer does not know is skipped rather than refused. A
    /// ChatML template against a model whose vocabulary has no `<|endoftext|>`
    /// is not broken, it just has one stop instead of two — and the token
    /// budget is the backstop either way.
    fn stop_tokens(&self, policy: &Policy) -> Vec<u32> {
        let mut stops: Vec<u32> = policy
            .template
            .stop
            .iter()
            .filter_map(|marker| self.tokenizer.token_to_id(marker))
            .collect();
        // What the file itself says ends a generation, which is the one stop
        // that does not depend on anybody having configured the right template.
        stops.extend(self.eos);
        stops
    }
}
