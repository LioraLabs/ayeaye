//! A quantized language model small enough to build in a test, and structurally
//! real.
//!
//! ayeaye ships the inference interface and not weights, so there are no
//! weights to test against — and a 4 GB download is not a unit test. What is
//! testable is the whole path from a directory of files to a rewritten
//! sentence, and that path does not care whether the weights are any good.
//!
//! So this builds one. Every tensor `quantized_llama::ModelWeights::from_gguf`
//! asks for, at the shape it asks for, quantized with the real quantizer and
//! written by candle's own GGUF writer. The result is a real quantized llama of
//! a ridiculous size whose rewrites are noise — which is exactly enough to
//! prove that loading, prompting, decoding and detokenising are wired to each
//! other correctly, and exactly the right thing to point the degradation tests
//! at.

#![allow(dead_code)]

use std::path::{Path, PathBuf};

use candle_core::quantized::{GgmlDType, QTensor, gguf_file};
use candle_core::{Device, Tensor};

/// The width of the model. A multiple of 32 because that is the block size
/// every `Q8_0` row is quantized in, and `QTensor::quantize` refuses a row that
/// is not a whole number of blocks.
pub const EMBEDDING: usize = 32;
/// How many tokens the toy vocabulary holds, specials included.
pub const VOCAB: usize = 64;
/// The feed-forward width. Also a multiple of the block size.
const FEED_FORWARD: usize = 64;
/// Attention heads, and key/value heads: no grouped-query trickery to test.
const HEADS: usize = 2;
/// One transformer block. The wiring is what is under test, not the depth.
const BLOCKS: usize = 1;
/// How many positions the model can hold, where the architecture reads that
/// from its own metadata. Small on purpose: it is also the number a
/// too-long-prompt refusal has to be provoked past.
pub const WINDOW: usize = 128;

/// The first id used by a special token; everything below is a word.
const FIRST_SPECIAL: u32 = 60;

/// The special tokens, at ids [`FIRST_SPECIAL`] and up. ChatML's, because that
/// is the template the default policy renders.
const SPECIALS: [&str; 3] = ["<|im_start|>", "<|im_end|>", "<|endoftext|>"];

/// The words this toy vocabulary can emit.
const WORDS: [&str; 20] = [
    "run",
    "the",
    "tests",
    "and",
    "then",
    "tell",
    "me",
    "what",
    "broke",
    "in",
    "parser",
    "please",
    "system",
    "user",
    "assistant",
    "clean",
    "up",
    "speech",
    "yes",
    "no",
];

/// The id a special token is written at.
pub fn special_id(token: &str) -> u32 {
    let index = SPECIALS
        .iter()
        .position(|t| *t == token)
        .unwrap_or_else(|| panic!("{token} is not in the toy vocabulary"));
    FIRST_SPECIAL + index as u32
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

/// A distinct scratch directory per call.
fn scratch(label: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "ayeaye-55-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default()
    ));
    std::fs::create_dir_all(&path).expect("creating the model directory");
    path
}

/// Build a complete, loadable `llama` model directory.
pub fn tiny_model(label: &str) -> ModelDir {
    build(label, "llama", true)
}

/// Build a complete, loadable model directory of a named supported
/// architecture.
///
/// The two are not interchangeable files: `qwen2` carries attention biases that
/// `llama` does not, and reads its window out of its own metadata. Building
/// both here is what stops the second architecture being supported on paper.
pub fn tiny_model_named(label: &str, architecture: &str) -> ModelDir {
    build(label, architecture, true)
}

/// Build a model directory whose GGUF claims an architecture nothing runs.
pub fn tiny_model_of_architecture(label: &str, architecture: &str) -> ModelDir {
    build(label, architecture, true)
}

/// Build a model directory whose GGUF is well-formed and short a tensor the
/// architecture needs.
pub fn tiny_model_missing_a_tensor(label: &str) -> ModelDir {
    build(label, "llama", false)
}

fn build(label: &str, architecture: &str, complete: bool) -> ModelDir {
    let dir = ModelDir {
        path: scratch(label),
    };
    write_gguf(&dir.path().join("model.gguf"), architecture, complete);
    std::fs::write(dir.path().join("tokenizer.json"), tokenizer_json())
        .expect("writing the tokenizer");
    dir
}

/// Write the GGUF: the metadata `from_gguf` reads, then every tensor it asks
/// for.
fn write_gguf(path: &Path, architecture: &str, complete: bool) {
    let device = Device::Cpu;
    let head_dim = EMBEDDING / HEADS;

    // GGUF namespaces these by architecture, and each loader looks them up
    // under its own prefix. A file claiming something nothing implements is
    // refused before any of this is read, so an unsupported fixture gets the
    // `llama` keys and never has them looked at.
    let prefix = match architecture {
        "qwen2" => "qwen2",
        _ => "llama",
    };
    let mut metadata: Vec<(String, gguf_file::Value)> = vec![
        (
            "general.architecture".to_string(),
            gguf_file::Value::String(architecture.to_string()),
        ),
        (
            format!("{prefix}.attention.head_count"),
            gguf_file::Value::U32(HEADS as u32),
        ),
        (
            format!("{prefix}.attention.head_count_kv"),
            gguf_file::Value::U32(HEADS as u32),
        ),
        (
            format!("{prefix}.block_count"),
            gguf_file::Value::U32(BLOCKS as u32),
        ),
        (
            format!("{prefix}.embedding_length"),
            gguf_file::Value::U32(EMBEDDING as u32),
        ),
        (
            format!("{prefix}.attention.layer_norm_rms_epsilon"),
            gguf_file::Value::F32(1e-5),
        ),
        (
            "tokenizer.ggml.eos_token_id".to_string(),
            gguf_file::Value::U32(special_id("<|im_end|>")),
        ),
    ];
    if prefix == "qwen2" {
        // qwen2 builds its rotary table to exactly this many positions, so it
        // is the model's real window rather than a claim about it.
        metadata.push((
            "qwen2.context_length".to_string(),
            gguf_file::Value::U32(WINDOW as u32),
        ));
    } else {
        metadata.push((
            "llama.rope.dimension_count".to_string(),
            gguf_file::Value::U32(head_dim as u32),
        ));
    }

    let mut tensors: Vec<(String, QTensor)> = vec![
        (
            "token_embd.weight".to_string(),
            quantized(&device, "token_embd", VOCAB, EMBEDDING),
        ),
        (
            "output.weight".to_string(),
            quantized(&device, "output", VOCAB, EMBEDDING),
        ),
        (
            "output_norm.weight".to_string(),
            norm(&device, "output_norm", EMBEDDING),
        ),
    ];
    for block in 0..BLOCKS {
        let prefix = format!("blk.{block}");
        for name in ["attn_q", "attn_k", "attn_v", "attn_output"] {
            tensors.push((
                format!("{prefix}.{name}.weight"),
                quantized(&device, &format!("{prefix}.{name}"), EMBEDDING, EMBEDDING),
            ));
        }
        tensors.push((
            format!("{prefix}.ffn_gate.weight"),
            quantized(
                &device,
                &format!("{prefix}.ffn_gate"),
                FEED_FORWARD,
                EMBEDDING,
            ),
        ));
        tensors.push((
            format!("{prefix}.ffn_up.weight"),
            quantized(
                &device,
                &format!("{prefix}.ffn_up"),
                FEED_FORWARD,
                EMBEDDING,
            ),
        ));
        // The one that maps back down. Omitted when the caller asked for a file
        // that parses and cannot be built into a model.
        if complete {
            tensors.push((
                format!("{prefix}.ffn_down.weight"),
                quantized(
                    &device,
                    &format!("{prefix}.ffn_down"),
                    EMBEDDING,
                    FEED_FORWARD,
                ),
            ));
        }
        // qwen2's one structural difference from llama: the attention
        // projections carry biases, and `from_gguf` demands all three.
        if architecture == "qwen2" {
            for name in ["attn_q", "attn_k", "attn_v"] {
                tensors.push((
                    format!("{prefix}.{name}.bias"),
                    norm(&device, &format!("{prefix}.{name}.bias"), EMBEDDING),
                ));
            }
        }
        tensors.push((
            format!("{prefix}.attn_norm.weight"),
            norm(&device, &format!("{prefix}.attn_norm"), EMBEDDING),
        ));
        tensors.push((
            format!("{prefix}.ffn_norm.weight"),
            norm(&device, &format!("{prefix}.ffn_norm"), EMBEDDING),
        ));
    }

    let metadata: Vec<(&str, &gguf_file::Value)> =
        metadata.iter().map(|(k, v)| (k.as_str(), v)).collect();
    let tensors: Vec<(&str, &QTensor)> = tensors.iter().map(|(k, v)| (k.as_str(), v)).collect();

    let mut file = std::fs::File::create(path).expect("creating the toy GGUF");
    gguf_file::write(&mut file, &metadata, &tensors).expect("writing the toy GGUF");
}

/// A matmul weight, quantized the way a published model's is.
///
/// `Q8_0` rather than `F32`: the ticket says *quantized*, and the dequantizing
/// path is exactly the part a test with float weights would skip.
fn quantized(device: &Device, name: &str, rows: usize, columns: usize) -> QTensor {
    let tensor = deterministic(device, name, &[rows, columns]);
    QTensor::quantize(&tensor, GgmlDType::Q8_0).expect("quantizing a toy weight")
}

/// A normalisation weight. Left at `F32`, as llama.cpp leaves them: they are a
/// vector per layer and quantizing them buys nothing.
fn norm(device: &Device, name: &str, width: usize) -> QTensor {
    let tensor = deterministic(device, name, &[width]);
    QTensor::quantize(&tensor, GgmlDType::F32).expect("writing a toy norm")
}

/// A tensor filled from a hash of its own name.
///
/// The initialisers candle uses are random and `Device::Cpu` cannot be seeded
/// in this version, so a test that asserts two generations agree has to make
/// the weights itself. Values are scaled small enough that one block does not
/// saturate — and centred slightly off zero, because a `Q8_0` row of exact
/// zeros quantizes to a zero scale.
fn deterministic(device: &Device, name: &str, shape: &[usize]) -> Tensor {
    let mut state = name.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
    });
    let count: usize = shape.iter().product();
    let values: Vec<f32> = (0..count)
        .map(|_| {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            ((state >> 40) as f32 / 8_388_608.0 - 1.0) * 0.1 + 0.01
        })
        .collect();
    Tensor::from_vec(values, shape, device).expect("building a deterministic weight")
}

/// A word-level tokenizer carrying ChatML's markers.
fn tokenizer_json() -> String {
    let mut vocab = serde_json::Map::new();
    vocab.insert("[UNK]".to_string(), serde_json::json!(0));
    for (i, word) in WORDS.iter().enumerate() {
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
        "pre_tokenizer": { "type": "WhitespaceSplit" },
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
