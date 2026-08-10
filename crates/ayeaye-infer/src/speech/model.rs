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
    /// The language token, where the model has one.
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
        // `metadata` first so an absent file is `Missing` naming the file,
        // rather than whatever the tensor library says about a path it cannot
        // open — which is the difference between a fixable error and a puzzle.
        std::fs::metadata(&weights_path).map_err(|e| SpeechError::read(&weights_path, e))?;
        let weights = std::fs::read(&weights_path).map_err(|e| SpeechError::read(&weights_path, e))?;
        let vb = VarBuilder::from_buffered_safetensors(weights, DType::F32, &device)
            .map_err(|e| SpeechError::malformed(&weights_path, e))?;

        let whisper = Whisper::load(&vb, config.clone())
            .map_err(|e| SpeechError::malformed(&weights_path, e))?;

        let tokens = SpecialTokens::resolve(&tokenizer)?;
        let filters = ayeaye_core::mel::mel_filterbank(
            u32::try_from(whisper::SAMPLE_RATE).unwrap_or(16_000),
            whisper::N_FFT,
            config.num_mel_bins,
        );

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
    /// says when to stop. The rest are optional because the English-only
    /// models do not all carry them.
    fn resolve(tokenizer: &Tokenizer) -> Result<Self, SpeechError> {
        let id = |token: &str| tokenizer.token_to_id(token);
        let required = |token: &str| {
            id(token).ok_or_else(|| SpeechError::MissingToken {
                token: token.to_string(),
            })
        };
        Ok(Self {
            start: required(whisper::SOT_TOKEN)?,
            end: required(whisper::EOT_TOKEN)?,
            transcribe: id(whisper::TRANSCRIBE_TOKEN),
            no_timestamps: id(whisper::NO_TIMESTAMPS_TOKEN),
            // English, because that is what the rest of ayeaye assumes today.
            // Language as configuration belongs with model choice, which is
            // AYEAYE-56's.
            language: id("<|en|>"),
        })
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

/// The device this build's backend runs on.
///
/// A one-line mapping today because a CPU build has one answer. AYEAYE-57 owns
/// turning this into a real selection — asking whether the device is actually
/// there and falling back to the CPU with a stated reason — and this is the
/// function it replaces.
fn device_for(backend: Backend) -> Result<Device, SpeechError> {
    match backend {
        Backend::Cpu => Ok(Device::Cpu),
        Backend::Cuda => Device::new_cuda(0).map_err(SpeechError::inference),
        Backend::Metal => Device::new_metal(0).map_err(SpeechError::inference),
    }
}
