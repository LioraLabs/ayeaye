//! A quantized instruct model, loaded from a directory of files.

use std::path::Path;

use candle_core::Tensor;
use candle_core::quantized::gguf_file;
use candle_transformers::models::{quantized_llama, quantized_qwen2};
use tokenizers::Tokenizer;

use super::error::LanguageError;
use crate::backend::{self, Backend, Selection};

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
/// list. Two entries covering most of what a person would point this at: in
/// GGUF terms `llama` is a family rather than a model — the Llama and Mistral
/// lineages, and most community fine-tunes of either, convert to it — and
/// `qwen2` is what the cleanup model this replaces an external server for is
/// published as.
///
/// The list is short because `candle-transformers` implements architectures one
/// at a time, and that is the property which makes an allowlist honest rather
/// than bureaucratic: an unsupported architecture is knowable from a file's
/// header instead of discovered at the first dictation.
pub const SUPPORTED: &[&str] = &["llama", "qwen2"];

/// The GGUF metadata key naming the architecture.
const ARCHITECTURE_KEY: &str = "general.architecture";

/// The GGUF metadata key naming the token that ends a generation.
const EOS_KEY: &str = "tokenizer.ggml.eos_token_id";

/// The weights, behind whichever loader understands them.
///
/// An enum rather than a trait object: there are two, the workspace owns
/// neither, and `forward` is the only thing either is ever asked for.
pub(crate) enum Weights {
    /// Llama, Mistral, and the fine-tunes that convert to their layout.
    Llama(quantized_llama::ModelWeights),
    /// Qwen 2 and 2.5, which differ by carrying attention biases.
    Qwen2(quantized_qwen2::ModelWeights),
}

impl Weights {
    /// Logits for the last position, given tokens and where they start.
    ///
    /// **`index_pos == 0` is what clears the attention cache**, in both of
    /// these. That is read out of candle 0.9's source rather than assumed:
    /// every layer keeps a key-value cache and concatenates onto it *except*
    /// when the index is zero, where it drops what was there. There is no
    /// `reset` to call, so starting each generation from zero is the whole of
    /// what stops one dictation attending to the last — and the test that two
    /// dictations through one loaded model agree is what would notice if that
    /// behaviour ever changed under us.
    pub(crate) fn forward(
        &mut self,
        input: &Tensor,
        index_pos: usize,
    ) -> candle_core::Result<Tensor> {
        match self {
            Self::Llama(weights) => weights.forward(input, index_pos),
            Self::Qwen2(weights) => weights.forward(input, index_pos),
        }
    }
}

/// A loaded language model, resident in memory until it is dropped.
///
/// Loading is the expensive, explicit act. Nothing here loads on demand; see
/// [`super::LanguageSlot`] for why that is the point rather than an omission.
pub struct LanguageModel {
    pub(crate) weights: Weights,
    pub(crate) tokenizer: Tokenizer,
    /// The device, and how it came to be that one.
    ///
    /// The whole answer is kept rather than the device alone, so that
    /// [`Self::backend`] and [`Self::fallback`] are two readings of one fact
    /// instead of two fields that can drift.
    pub(crate) selection: Selection,
    pub(crate) architecture: String,
    /// How many positions this model's rotary table was built for.
    pub(crate) window: usize,
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
        Self::load_with(dir, backend::select())
    }

    /// Load the model in `dir` onto a device already chosen.
    ///
    /// [`Self::load`] is this with [`backend::select`] called for you. This
    /// exists because the device decision is a property of the process and not
    /// of the model: it is what [`crate::LanguageSlot`] hands in so that a
    /// model unloaded by the idle policy and loaded again lands on the same
    /// device, and it is how a test names a selection this machine cannot
    /// produce.
    pub fn load_with(dir: &Path, selection: Selection) -> Result<Self, LanguageError> {
        let device = selection.device.clone();

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
        let window = window_of(&architecture, &content, &weights_path)?;

        // Before the weights, for the same reason the architecture is: a
        // tokenizer is a few hundred kB and the weights are gigabytes, so
        // reading it second means a missing or corrupt one costs a full
        // `from_gguf` before anybody is told. Every fact knowable cheaply is
        // established while it is still cheap.
        let tokenizer_path = dir.join(TOKENIZER_FILE);
        let tokenizer_bytes =
            std::fs::read(&tokenizer_path).map_err(|e| LanguageError::read(&tokenizer_path, e))?;
        let tokenizer = Tokenizer::from_bytes(&tokenizer_bytes)
            .map_err(|e| LanguageError::malformed(&tokenizer_path, e))?;

        let weights = match architecture.as_str() {
            "qwen2" => quantized_qwen2::ModelWeights::from_gguf(content, &mut file, &device)
                .map(Weights::Qwen2),
            // `llama`. The allowlist above is what makes that the only other
            // value able to reach here.
            _ => quantized_llama::ModelWeights::from_gguf(content, &mut file, &device)
                .map(Weights::Llama),
        }
        .map_err(|e| LanguageError::malformed(&weights_path, e))?;

        Ok(Self {
            weights,
            tokenizer,
            selection,
            architecture,
            window,
            eos,
        })
    }

    /// Where this model is really running.
    ///
    /// Read off the device it is holding, not off the build: on an artifact
    /// with acceleration compiled in these differ exactly when [`Self::fallback`]
    /// has something to say.
    pub fn backend(&self) -> Backend {
        self.selection.got()
    }

    /// Why this model is not on the backend the build was compiled for.
    ///
    /// `None` when it is — which is every CPU build, where there was nothing
    /// to give up.
    pub fn fallback(&self) -> Option<&str> {
        self.selection.fallback.as_deref()
    }

    /// What the file called itself.
    pub fn architecture(&self) -> &str {
        &self.architecture
    }

    /// How many positions this model can be prompted and answered within.
    pub fn window(&self) -> usize {
        self.window
    }
}

/// How far the loader will build a rotary table for this file.
///
/// Not the same question as "what context length does the file claim", and the
/// difference is why this is a function rather than a metadata read.
/// `quantized_llama` ignores the file's context length and precomputes a fixed
/// 4 096 positions; `quantized_qwen2` reads `qwen2.context_length` and builds
/// exactly that many. Either way a position past the table is an out-of-bounds
/// index rather than a degradation, so the table is what a generation has to be
/// bounded by.
///
/// The two ways this fails are different facts and get different sentences.
/// A file that never says how long its context is has an absent key; a file that
/// says zero has a nonsensical one. Reporting the second for the first sends
/// somebody looking for a number that is not in their file.
fn window_of(
    architecture: &str,
    content: &gguf_file::Content,
    weights_path: &Path,
) -> Result<usize, LanguageError> {
    const QWEN2_CONTEXT_KEY: &str = "qwen2.context_length";

    let window = match architecture {
        "qwen2" => content
            .metadata
            .get(QWEN2_CONTEXT_KEY)
            .and_then(|v| v.to_u32().ok())
            .ok_or_else(|| {
                LanguageError::malformed(
                    weights_path,
                    format!("no {QWEN2_CONTEXT_KEY} in the file's metadata, so there is no window"),
                )
            })? as usize,
        _ => quantized_llama::MAX_SEQ_LEN,
    };
    if window == 0 {
        return Err(LanguageError::malformed(
            weights_path,
            "the file's context length is 0, so there is no position to decode at",
        ));
    }
    Ok(window)
}

impl std::fmt::Debug for LanguageModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The weights are gigabytes and nobody wants them in a log line.
        f.debug_struct("LanguageModel")
            .field("architecture", &self.architecture)
            .field("window", &self.window)
            .field("vocab_size", &self.tokenizer.get_vocab_size(true))
            .field("backend", &self.backend().label())
            .finish()
    }
}
