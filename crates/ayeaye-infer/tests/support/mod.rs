//! A speech model small enough to build in a test, and structurally real.
//!
//! ayeaye ships the inference interface and not weights, so there are no
//! weights to test against — and a 75 MB download is not a unit test. What is
//! testable is the whole path from a directory of files to a transcript, and
//! that path does not care whether the weights are any good.
//!
//! So this builds one. A `VarMap`-backed `VarBuilder` creates every tensor
//! `Whisper::load` asks for, at whatever shape it asks for it, and `VarMap`
//! writes them out as safetensors. The result is a real Whisper of a
//! ridiculous size whose transcripts are noise — which is exactly enough to
//! prove that loading, mel, encoding, decoding and detokenising are wired to
//! each other correctly.

use std::path::{Path, PathBuf};

use candle_core::{DType, Device};
use candle_nn::{VarBuilder, VarMap};
use candle_transformers::models::whisper::{Config, model::Whisper};

/// Real Whisper's mel bin count, so the real filterbank is exercised.
pub const MEL_BINS: usize = 80;

/// Encoder positions, which set the model's audio window.
///
/// 50 positions is 100 mel frames is 16 000 samples is **exactly one second**,
/// chosen so the windowing arithmetic in the tests is legible: a 2.5 second
/// clip is three windows, and the numbers in the assertions can be read
/// without computing anything.
pub const WINDOW_POSITIONS: usize = 50;

/// How many tokens the vocabulary holds, specials included.
pub const VOCAB: usize = 64;

/// The first id used by a special token; everything below is a word.
const FIRST_SPECIAL: u32 = 50;

/// The words this toy vocabulary can emit.
const WORDS: [&str; 49] = [
    "ayeaye", "hears", "you", "the", "quick", "brown", "fox", "jumps", "over", "lazy", "dog",
    "and", "then", "says", "something", "about", "a", "model", "that", "was", "never", "trained",
    "on", "anything", "at", "all", "which", "is", "fine", "because", "this", "test", "cares",
    "only", "about", "wiring", "not", "about", "words", "so", "here", "are", "some", "more", "of",
    "them", "to", "fill", "space",
];

/// The special tokens, at ids [`FIRST_SPECIAL`] and up.
const SPECIALS: [&str; 6] = [
    "<|startoftranscript|>",
    "<|endoftext|>",
    "<|transcribe|>",
    "<|translate|>",
    "<|notimestamps|>",
    "<|en|>",
];

/// The id a special token is written at.
pub fn special_id(token: &str) -> u32 {
    let index = SPECIALS
        .iter()
        .position(|t| *t == token)
        .unwrap_or_else(|| panic!("{token} is not in the toy vocabulary"));
    FIRST_SPECIAL + index as u32
}

/// A tiny Whisper config, optionally with tokens it must never emit.
pub fn tiny_config(suppress_tokens: Vec<u32>) -> Config {
    Config {
        num_mel_bins: MEL_BINS,
        max_source_positions: WINDOW_POSITIONS,
        d_model: 32,
        encoder_attention_heads: 2,
        encoder_layers: 1,
        vocab_size: VOCAB,
        max_target_positions: 24,
        decoder_attention_heads: 2,
        decoder_layers: 1,
        suppress_tokens,
    }
}

/// A directory holding a loadable model, removed when this is dropped.
pub struct ModelDir {
    path: PathBuf,
}

impl ModelDir {
    /// Where the model is.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Delete one of the model's files, to see what the loader says about it.
    pub fn remove(&self, name: &str) {
        std::fs::remove_file(self.path.join(name)).expect("removing a file that should be there");
    }

    /// Overwrite one of the model's files with something that is not it.
    pub fn corrupt(&self, name: &str) {
        std::fs::write(self.path.join(name), b"{ this is not the file you want")
            .expect("overwriting a file that should be there");
    }
}

impl Drop for ModelDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// Build a complete, loadable model directory.
///
/// The weights are made deterministic after the shapes are discovered — see
/// [`fill_deterministically`] — so a test that asserts on a decode gets the
/// same decode twice.
pub fn tiny_model(label: &str, config: &Config) -> ModelDir {
    let path = std::env::temp_dir().join(format!(
        "ayeaye-54-{label}-{}-{}",
        std::process::id(),
        // Distinct per call within a process, so two models in one test do not
        // land in the same directory.
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default()
    ));
    std::fs::create_dir_all(&path).expect("creating the model directory");
    let dir = ModelDir { path };

    let device = Device::Cpu;

    let varmap = VarMap::new();
    let vb = VarBuilder::from_varmap(&varmap, DType::F32, &device);
    // Loading against a VarMap *creates* every tensor it asks for, at the
    // shape it asks for. That is what makes this a structurally real model
    // rather than a guess at what Whisper's key names are.
    Whisper::load(&vb, config.clone()).expect("building the toy model");
    fill_deterministically(&varmap, &device);
    varmap
        .save(dir.path().join("model.safetensors"))
        .expect("writing the toy weights");

    std::fs::write(
        dir.path().join("config.json"),
        serde_json::to_string_pretty(&config_json(config)).expect("rendering the config"),
    )
    .expect("writing the config");

    std::fs::write(dir.path().join("tokenizer.json"), tokenizer_json())
        .expect("writing the tokenizer");

    dir
}

/// Replace every weight with something derived from its own name.
///
/// The initialisers candle uses are random, and `Device::Cpu` cannot be seeded
/// in this version — `set_seed` refuses outright. A test that asserts anything
/// about a decode has to get the same decode twice, so the weights are made
/// deterministic here rather than left to an RNG: each tensor is filled from a
/// small linear congruential sequence started from a hash of its own key, and
/// scaled small enough that a one-layer model does not saturate.
fn fill_deterministically(varmap: &VarMap, device: &Device) {
    let data = varmap.data().lock().expect("the varmap lock");
    for (name, var) in data.iter() {
        let mut state = name.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| {
            (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
        });
        let values: Vec<f32> = (0..var.elem_count())
            .map(|_| {
                state = state
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                // Uniform in [-0.1, 0.1]: big enough that the logits differ
                // from each other, small enough that one residual block does
                // not run off to infinity.
                ((state >> 40) as f32 / 8_388_608.0 - 1.0) * 0.1
            })
            .collect();
        let tensor = candle_core::Tensor::from_vec(values, var.shape(), device)
            .expect("building a deterministic weight");
        var.set(&tensor).expect("setting a deterministic weight");
    }
}

/// The config, in the JSON shape a real model ships it in.
fn config_json(config: &Config) -> serde_json::Value {
    serde_json::json!({
        "num_mel_bins": config.num_mel_bins,
        "max_source_positions": config.max_source_positions,
        "d_model": config.d_model,
        "encoder_attention_heads": config.encoder_attention_heads,
        "encoder_layers": config.encoder_layers,
        "vocab_size": config.vocab_size,
        "max_target_positions": config.max_target_positions,
        "decoder_attention_heads": config.decoder_attention_heads,
        "decoder_layers": config.decoder_layers,
        "suppress_tokens": config.suppress_tokens,
    })
}

/// A word-level tokenizer carrying Whisper's steering tokens.
fn tokenizer_json() -> String {
    let mut vocab = serde_json::Map::new();
    vocab.insert("[UNK]".to_string(), serde_json::json!(0));
    for (i, word) in WORDS.iter().enumerate() {
        // Later duplicates of a word keep the first id, which is fine: the
        // vocabulary only has to be a vocabulary, not a good one.
        vocab
            .entry(word.to_string())
            .or_insert(serde_json::json!(i + 1));
    }

    let added: Vec<serde_json::Value> = SPECIALS
        .iter()
        .enumerate()
        .map(|(i, token)| {
            serde_json::json!({
                "id": FIRST_SPECIAL + i as u32,
                "content": token,
                "single_word": false,
                "lstrip": false,
                "rstrip": false,
                "normalized": false,
                "special": true,
            })
        })
        .collect();

    serde_json::json!({
        "version": "1.0",
        "truncation": null,
        "padding": null,
        "added_tokens": added,
        "normalizer": null,
        "pre_tokenizer": null,
        "post_processor": null,
        "decoder": null,
        "model": {
            "type": "WordLevel",
            "vocab": vocab,
            "unk_token": "[UNK]",
        },
    })
    .to_string()
}

/// A tone, as 16 kHz mono audio.
///
/// Not silence: a model fed nothing but zeros is being asked a question with a
/// degenerate answer, and the mel of pure silence is a constant. A tone
/// exercises the filterbank the way speech would.
pub fn tone(secs: f32) -> ayeaye_core::Pcm16kMono {
    let samples = (secs * 16_000.0) as usize;
    ayeaye_core::Pcm16kMono::new(
        (0..samples)
            .map(|i| {
                let t = i as f32 / 16_000.0;
                0.4 * (2.0 * std::f32::consts::PI * 440.0 * t).sin()
            })
            .collect(),
    )
}

/// A config with the audio window a real Whisper has.
///
/// Every other model here uses a one-second window so the windowing arithmetic
/// is legible. That leaves the number every released Whisper actually uses —
/// 1 500 encoder positions, 3 000 mel frames, thirty seconds — exercised by
/// nothing, and the mel padding behaves differently there: `pcm_to_mel` rounds
/// its output up to a multiple of 1 500 and then adds 1 500 more, so 3 000 is
/// the one width where the amount of padding is not what a small window would
/// lead you to expect. The rest of the model stays tiny.
pub fn real_window_config() -> Config {
    Config {
        max_source_positions: 1_500,
        max_target_positions: 8,
        ..tiny_config(vec![])
    }
}
