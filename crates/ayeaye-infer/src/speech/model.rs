//! A Whisper-family speech model, loaded from a directory of files.

use std::path::Path;

use candle_core::{DType, Device};
use candle_nn::VarBuilder;
use candle_transformers::models::whisper::{self, Config, model::Whisper};
use tokenizers::Tokenizer;

use super::error::SpeechError;
use crate::backend::{self, Backend};

/// The model's own description of its shape and vocabulary.
pub const CONFIG_FILE: &str = "config.json";
/// The vocabulary, in HuggingFace `tokenizers` form.
pub const TOKENIZER_FILE: &str = "tokenizer.json";
/// The weights.
pub const WEIGHTS_FILE: &str = "model.safetensors";

/// The sample rate the filterbank is built for.
///
/// `ayeaye-core` and candle each name this number, and the mel filters are
/// computed from one while the audio is validated against the other. If they
/// ever disagree the filterbank silently describes the wrong frequencies, so
/// they are made to agree at compile time rather than at transcription time.
const SAMPLE_RATE_HZ: u32 = ayeaye_core::audio::SAMPLE_RATE_HZ;
const _: () = assert!(SAMPLE_RATE_HZ as usize == whisper::SAMPLE_RATE);

/// The vocabulary size of OpenAI's English-only Whisper models.
///
/// The only thing that distinguishes an `.en` model from a multilingual one.
/// Their tokenizers are not a clue: `tiny.en` and `tiny` both carry
/// `<|en|>`, `<|fr|>` and 98 language tokens, and both have 1 608 added
/// tokens — verified against the published tokenizers, because the obvious
/// "does it have a language token" check quietly gets this wrong.
const ENGLISH_ONLY_VOCAB_SIZE: usize = 51_864;

/// A loaded speech model, resident in memory until it is dropped.
///
/// Loading is the expensive, explicit act — reading hundreds of megabytes and
/// laying it out for a device. Nothing here loads on demand; see
/// [`super::SpeechSlot`] for why that is the point rather than an omission.
pub struct SpeechModel {
    pub(crate) whisper: Whisper,
    pub(crate) tokenizer: Tokenizer,
    pub(crate) config: Config,
    pub(crate) device: Device,
    pub(crate) filters: Vec<f32>,
    pub(crate) tokens: SpecialTokens,
}

/// The token ids Whisper decoding is steered with.
///
/// Ids, not strings: they are looked up once at load rather than per decode
/// step, and a model whose tokenizer lacks one of them cannot be decoded at
/// all, which is a fact worth learning at load time.
pub(crate) struct SpecialTokens {
    /// `<|startoftranscript|>`, the first token of every decode.
    pub(crate) start: u32,
    /// `<|endoftext|>`, which is how a decode ends.
    pub(crate) end: u32,
    /// `<|transcribe|>`, as opposed to `<|translate|>`.
    pub(crate) transcribe: Option<u32>,
    /// `<|notimestamps|>`, since this ticket returns text and not timings.
    pub(crate) no_timestamps: Option<u32>,
    /// The language token, where the model is one that takes one.
    pub(crate) language: Option<u32>,
}

impl SpeechModel {
    /// Load the model in `dir`, which must hold `config.json`,
    /// `tokenizer.json` and `model.safetensors`.
    ///
    /// The directory is the caller's to choose and ayeaye's to read: acquiring
    /// what goes in it is AYEAYE-56's, and shipping weights is nobody's.
    pub fn load(dir: &Path) -> Result<Self, SpeechError> {
        let device = device_for(backend::selected())?;

        let config_path = dir.join(CONFIG_FILE);
        let config_text = std::fs::read_to_string(&config_path)
            .map_err(|e| SpeechError::read(&config_path, e))?;
        let config: Config = serde_json::from_str(&config_text)
            .map_err(|e| SpeechError::malformed(&config_path, e))?;

        let tokenizer_path = dir.join(TOKENIZER_FILE);
        let tokenizer_bytes =
            std::fs::read(&tokenizer_path).map_err(|e| SpeechError::read(&tokenizer_path, e))?;
        let tokenizer = Tokenizer::from_bytes(&tokenizer_bytes)
            .map_err(|e| SpeechError::malformed(&tokenizer_path, e))?;

        let weights_path = dir.join(WEIGHTS_FILE);
        // Read into memory rather than mmapped, deliberately. candle offers
        // `from_mmaped_safetensors`, which is `unsafe` and undefined if anyone
        // writes to the file while it is mapped — and this is a daemon that
        // may be transcribing while a model directory is being replaced.
        //
        // The price is peak memory: the file is buffered whole, and candle
        // converts each tensor to F32 on the way in, so an F16 checkpoint
        // peaks near three times its own size before settling at two. That is
        // fine for the models this runs on a CPU and is not fine for the
        // largest ones. AYEAYE-56 owns residency policy and is where the
        // trade-off should be revisited with a number attached.
        let weights =
            std::fs::read(&weights_path).map_err(|e| SpeechError::read(&weights_path, e))?;
        let vb = VarBuilder::from_buffered_safetensors(weights, DType::F32, &device)
            .map_err(|e| SpeechError::malformed(&weights_path, e))?;

        let whisper = Whisper::load(&vb, config.clone())
            .map_err(|e| SpeechError::malformed(&weights_path, e))?;

        let tokens = SpecialTokens::resolve(&tokenizer, &config, &tokenizer_path)?;
        check_shape(&config, &tokens, &config_path)?;
        let filters =
            ayeaye_core::mel::mel_filterbank(SAMPLE_RATE_HZ, whisper::N_FFT, config.num_mel_bins);

        Ok(Self {
            whisper,
            tokenizer,
            config,
            device,
            filters,
            tokens,
        })
    }

    /// Where this model is running.
    pub fn backend(&self) -> Backend {
        backend::selected()
    }

    /// How many mel bins this model's config asks for.
    pub fn mel_bins(&self) -> usize {
        self.config.num_mel_bins
    }
}

impl std::fmt::Debug for SpeechModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The weights are hundreds of megabytes and nobody wants them in a log
        // line; the shape is what identifies the model.
        f.debug_struct("SpeechModel")
            .field("mel_bins", &self.config.num_mel_bins)
            .field("vocab_size", &self.config.vocab_size)
            .field("encoder_layers", &self.config.encoder_layers)
            .field("decoder_layers", &self.config.decoder_layers)
            .field("backend", &self.backend().label())
            .finish()
    }
}

impl SpecialTokens {
    /// Look up the steering tokens in a tokenizer.
    ///
    /// `<|startoftranscript|>` and `<|endoftext|>` are required: without the
    /// first there is nothing to decode from, and without the second nothing
    /// says when to stop.
    ///
    /// The language and task tokens are taken only for a multilingual model.
    /// An `.en` model is prompted with `<|startoftranscript|>` and
    /// `<|notimestamps|>` alone — it was never trained with the other two — and
    /// since its tokenizer carries them anyway, looking them up and using what
    /// is found puts the most likely CPU model off its own distribution.
    ///
    /// Ids are checked against the vocabulary here rather than met at the
    /// first transcription, which is the whole reason this resolves at load.
    fn resolve(
        tokenizer: &Tokenizer,
        config: &Config,
        tokenizer_path: &Path,
    ) -> Result<Self, SpeechError> {
        let id = |token: &str| tokenizer.token_to_id(token);
        let required = |token: &str| {
            id(token).ok_or_else(|| SpeechError::MissingToken {
                token: token.to_string(),
            })
        };
        let multilingual = config.vocab_size > ENGLISH_ONLY_VOCAB_SIZE;

        let tokens = Self {
            start: required(whisper::SOT_TOKEN)?,
            end: required(whisper::EOT_TOKEN)?,
            transcribe: multilingual
                .then(|| id(whisper::TRANSCRIBE_TOKEN))
                .flatten(),
            no_timestamps: id(whisper::NO_TIMESTAMPS_TOKEN),
            // English. Which language, and whether to translate instead, are
            // configuration — AYEAYE-56's, along with model choice.
            language: multilingual.then(|| id("<|en|>")).flatten(),
        };

        if let Some(out_of_range) = tokens
            .prompt()
            .into_iter()
            .chain([tokens.end])
            .find(|id| *id as usize >= config.vocab_size)
        {
            return Err(SpeechError::malformed(
                tokenizer_path,
                format!(
                    "token {out_of_range} is outside the {} the config declares",
                    config.vocab_size
                ),
            ));
        }

        Ok(tokens)
    }

    /// The prompt every decode starts from.
    pub(crate) fn prompt(&self) -> Vec<u32> {
        let mut prompt = vec![self.start];
        prompt.extend(self.language);
        prompt.extend(self.transcribe);
        prompt.extend(self.no_timestamps);
        prompt
    }
}

/// Refuse a config that parses but describes nothing transcribable.
///
/// Valid JSON is not a valid model. A zero-length audio window would divide
/// the audio into chunks of nothing — which panics rather than errors — and a
/// target length no longer than the prompt could never emit a token. Both are
/// caught here, where the file that said so can still be named, instead of at
/// the first dictation.
fn check_shape(
    config: &Config,
    tokens: &SpecialTokens,
    config_path: &Path,
) -> Result<(), SpeechError> {
    let prompt = tokens.prompt().len();
    let refuse = |what: &str| Err(SpeechError::malformed(config_path, what.to_string()));

    if config.max_source_positions == 0 {
        return refuse("max_source_positions is 0, so the model has no audio window");
    }
    if config.num_mel_bins == 0 {
        return refuse("num_mel_bins is 0, so there is no spectrogram to encode");
    }
    if config.vocab_size == 0 {
        return refuse("vocab_size is 0, so there is nothing the model could say");
    }
    if config.max_target_positions <= prompt {
        return refuse(
            "max_target_positions leaves no room past the prompt, so no token could be decoded",
        );
    }

    Ok(())
}

/// The device this build's backend runs on.
///
/// The mapping itself moved to [`backend::device`] when a second model started
/// needing it; this is the error type it wears here. AYEAYE-57 owns turning the
/// mapping into a real selection.
fn device_for(backend: Backend) -> Result<Device, SpeechError> {
    backend::device(backend).map_err(SpeechError::inference)
}

#[cfg(test)]
mod tests {
    use super::SpecialTokens;

    fn tokens(language: Option<u32>, transcribe: Option<u32>) -> SpecialTokens {
        SpecialTokens {
            start: 1,
            end: 2,
            transcribe,
            no_timestamps: Some(4),
            language,
        }
    }

    // AYEAYE-54
    //
    // A multilingual model is told the language and the task.
    #[test]
    fn a_multilingual_prompt_names_the_language_and_the_task() {
        assert_eq!(tokens(Some(3), Some(5)).prompt(), vec![1, 3, 5, 4]);
    }

    // AYEAYE-54
    //
    // An `.en` model gets neither, even though its tokenizer carries both.
    // This is the shape no toy-model test can see and no real model forgives.
    #[test]
    fn an_english_only_prompt_is_the_start_token_and_no_timestamps() {
        assert_eq!(tokens(None, None).prompt(), vec![1, 4]);
    }
}
