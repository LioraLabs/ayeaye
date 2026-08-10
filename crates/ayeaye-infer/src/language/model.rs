//! A quantized instruct model, loaded from a directory of files.

use std::path::Path;

use candle_core::Device;
use candle_core::quantized::gguf_file;
use candle_transformers::models::quantized_llama::ModelWeights;
use tokenizers::Tokenizer;

use super::error::LanguageError;
use crate::backend::{self, Backend};

/// The weights, quantized, in the format llama.cpp publishes.
///
/// GGUF rather than safetensors, unlike the speech model, because that is the
/// format a *quantized* instruct model is published in — and quantization is
/// not an optimisation here. An F32 7B is 28 GB; the same model at four bits is
/// under five, which is the difference between a CPU build that can hold a
/// cleanup model and one that cannot.
pub const WEIGHTS_FILE: &str = "model.gguf";

/// The vocabulary, in HuggingFace `tokenizers` form.
///
/// Beside the GGUF rather than inside it. A GGUF does carry its vocabulary, as
/// a flat token list plus merges, but rebuilding a byte-level BPE from those
/// pieces means reconstructing the pre-tokeniser and the decoder by hand, and
/// getting either subtly wrong produces a model that runs and mis-spells.
/// Reading the tokenizer the publisher shipped is the honest option. AYEAYE-56
/// owns acquisition and has to fetch both files; note that quantized GGUF
/// repositories often carry only the weights, with `tokenizer.json` in the
/// unquantized repository they were converted from.
pub const TOKENIZER_FILE: &str = "tokenizer.json";

/// The architectures this build can run, as GGUF's `general.architecture`
/// spells them.
///
/// Public so AYEAYE-56 can check it at pull time rather than restating the
/// list. In GGUF terms `llama` is a family and not a model: the Llama and
/// Mistral lineages, and most community fine-tunes of either, convert to it.
///
/// `qwen2` is deliberately absent even though `candle-transformers` implements
/// it. Its `ModelWeights` does not derive `Clone` and exposes no way to clear
/// the attention cache, so a second dictation would decode with the first one's
/// cache still in it — a wrong answer that looks like a working model. See
/// [`LanguageModel::fresh`] for what `llama` gets instead. An architecture that
/// cannot be reset between dictations does not belong on this list.
pub const SUPPORTED: &[&str] = &["llama"];

/// The GGUF metadata key naming the architecture.
const ARCHITECTURE_KEY: &str = "general.architecture";

/// The GGUF metadata key naming the token that ends a generation.
const EOS_KEY: &str = "tokenizer.ggml.eos_token_id";

/// A loaded language model, resident in memory until it is dropped.
///
/// Loading is the expensive, explicit act. Nothing here loads on demand; see
/// `LanguageSlot` for why that is the point rather than an omission.
pub struct LanguageModel {
    /// The weights as they were loaded, never generated through.
    pristine: ModelWeights,
    pub(crate) tokenizer: Tokenizer,
    pub(crate) device: Device,
    pub(crate) architecture: String,
    /// The end-of-generation token the file itself names, where it names one.
    pub(crate) eos: Option<u32>,
}

impl LanguageModel {
    /// Load the model in `dir`, which must hold [`WEIGHTS_FILE`] and
    /// [`TOKENIZER_FILE`].
    ///
    /// The directory is the caller's to choose and ayeaye's to read: acquiring
    /// what goes in it is AYEAYE-56's, and shipping weights is nobody's.
    pub fn load(dir: &Path) -> Result<Self, LanguageError> {
        let device = backend::device(backend::selected()).map_err(LanguageError::inference)?;

        let weights_path = dir.join(WEIGHTS_FILE);
        // The file stays open and is read from as the tensors are pulled out,
        // rather than being buffered whole: a quantized model is the largest
        // thing this process touches, and the point of quantizing it was to fit.
        let mut file = std::fs::File::open(&weights_path)
            .map_err(|e| LanguageError::read(&weights_path, e))?;
        let content = gguf_file::Content::read(&mut file)
            .map_err(|e| LanguageError::malformed(&weights_path, e))?;

        let architecture = content
            .metadata
            .get(ARCHITECTURE_KEY)
            .and_then(|value| value.to_string().ok())
            .cloned()
            .ok_or_else(|| {
                LanguageError::malformed(
                    &weights_path,
                    format!("no {ARCHITECTURE_KEY} in the file's metadata"),
                )
            })?;
        // Before a single tensor is read. The architecture decides which code
        // could run the file at all, so asking afterwards would mean paying for
        // gigabytes to be told the answer was knowable from the header.
        if !SUPPORTED.contains(&architecture.as_str()) {
            return Err(LanguageError::UnsupportedArchitecture {
                found: architecture,
            });
        }

        let eos = content.metadata.get(EOS_KEY).and_then(|v| v.to_u32().ok());

        let pristine = ModelWeights::from_gguf(content, &mut file, &device)
            .map_err(|e| LanguageError::malformed(&weights_path, e))?;

        let tokenizer_path = dir.join(TOKENIZER_FILE);
        let tokenizer_bytes =
            std::fs::read(&tokenizer_path).map_err(|e| LanguageError::read(&tokenizer_path, e))?;
        let tokenizer = Tokenizer::from_bytes(&tokenizer_bytes)
            .map_err(|e| LanguageError::malformed(&tokenizer_path, e))?;

        Ok(Self {
            pristine,
            tokenizer,
            device,
            architecture,
            eos,
        })
    }

    /// Where this model is running.
    pub fn backend(&self) -> Backend {
        backend::selected()
    }

    /// What the file called itself.
    pub fn architecture(&self) -> &str {
        &self.architecture
    }

    /// A copy of the weights with no attention cache in it.
    ///
    /// candle 0.9's `quantized_llama` keeps a key-value cache inside every
    /// layer and offers no way to clear it, so a second generation through the
    /// same weights would attend to the first one's tokens — one dictation
    /// bleeding into the next, with no symptom but a worse answer.
    ///
    /// Cloning is the way out and is nearly free: every tensor in there is
    /// behind an `Arc`, so this copies pointers rather than weights. It is why
    /// [`SUPPORTED`] holds `llama` and not the architectures whose weights do
    /// not derive `Clone`.
    pub(crate) fn fresh(&self) -> ModelWeights {
        self.pristine.clone()
    }
}

impl std::fmt::Debug for LanguageModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The weights are gigabytes and nobody wants them in a log line.
        f.debug_struct("LanguageModel")
            .field("architecture", &self.architecture)
            .field("vocab_size", &self.tokenizer.get_vocab_size(true))
            .field("backend", &self.backend().label())
            .finish()
    }
}
