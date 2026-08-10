//! In-process transcript cleanup, observed only through its public API.

mod gguf;

use ayeaye_infer::language::{LanguageError, LanguageModel, SUPPORTED};
use gguf::{tiny_model, tiny_model_missing_a_tensor, tiny_model_of_architecture};

// AYEAYE-55
//
// "The model did not load" is the error message that costs an evening. Each
// file gets its own test because each is read at a different point, and a
// loader that named the first file whatever went missing would pass one of
// these and not the other.
#[test]
fn absent_weights_are_an_error_naming_the_weights() {
    let dir = tiny_model("no-weights");
    dir.remove("model.gguf");

    let error = LanguageModel::load(dir.path()).expect_err("a model with no weights cannot load");

    assert!(
        matches!(error, LanguageError::Missing { .. }),
        "expected a Missing error, got {error:?}"
    );
    assert!(
        error.to_string().contains("model.gguf"),
        "the error should name the file it wanted: {error}"
    );
}

// AYEAYE-55
#[test]
fn an_absent_tokenizer_is_an_error_naming_the_tokenizer() {
    let dir = tiny_model("no-tokenizer");
    dir.remove("tokenizer.json");

    let error = LanguageModel::load(dir.path()).expect_err("a model with no tokenizer cannot load");

    assert!(
        matches!(error, LanguageError::Missing { .. }),
        "expected a Missing error, got {error:?}"
    );
    assert!(
        error.to_string().contains("tokenizer.json"),
        "the error should name the file it wanted: {error}"
    );
}

// AYEAYE-55
#[test]
fn weights_that_are_not_a_gguf_are_a_malformed_error_naming_them() {
    let dir = tiny_model("corrupt-weights");
    dir.corrupt("model.gguf");

    let error = LanguageModel::load(dir.path()).expect_err("a file that is not a GGUF cannot load");

    assert!(
        matches!(error, LanguageError::Malformed { .. }),
        "expected a Malformed error, got {error:?}"
    );
    assert!(error.to_string().contains("model.gguf"));
}

// AYEAYE-55
#[test]
fn a_tokenizer_that_is_not_json_is_a_malformed_error_naming_it() {
    let dir = tiny_model("corrupt-tokenizer");
    dir.corrupt("tokenizer.json");

    let error =
        LanguageModel::load(dir.path()).expect_err("a tokenizer that is not JSON cannot load");

    assert!(
        matches!(error, LanguageError::Malformed { .. }),
        "expected a Malformed error, got {error:?}"
    );
    assert!(error.to_string().contains("tokenizer.json"));
}

// AYEAYE-55
//
// The architecture allowlist, at load time. `candle-transformers` implements
// architectures one at a time, so an unsupported one is a fact about the file
// rather than about the machine — knowable from the header, before a gigabyte
// of tensors is read. AYEAYE-56 asks the same question at pull time, against
// the same list.
#[test]
fn an_architecture_this_build_cannot_run_is_refused_by_name() {
    let dir = tiny_model_of_architecture("unsupported", "mamba");

    let error =
        LanguageModel::load(dir.path()).expect_err("an unimplemented architecture cannot load");

    assert!(
        matches!(error, LanguageError::UnsupportedArchitecture { .. }),
        "expected an UnsupportedArchitecture error, got {error:?}"
    );
    let message = error.to_string();
    assert!(
        message.contains("mamba"),
        "the error should name what it found: {message}"
    );
    assert!(
        message.contains("llama"),
        "and what it can run instead: {message}"
    );
}

// AYEAYE-55
//
// The list is public so AYEAYE-56 can refuse a download rather than restating
// it. `qwen2` is off it deliberately: candle 0.9's `quantized_qwen2` weights
// neither derive `Clone` nor expose a cache reset, so one dictation would bleed
// into the next.
#[test]
fn the_supported_architectures_are_published_for_pull_time() {
    assert_eq!(SUPPORTED, &["llama"]);
}

// AYEAYE-55
//
// A GGUF that parses and is short a tensor the architecture needs is a
// malformed *file*, not a panic and not an unsupported architecture.
#[test]
fn a_gguf_missing_a_tensor_is_malformed_rather_than_a_panic() {
    let dir = tiny_model_missing_a_tensor("short-a-tensor");

    let error = LanguageModel::load(dir.path()).expect_err("an incomplete model cannot load");

    assert!(
        matches!(error, LanguageError::Malformed { .. }),
        "expected a Malformed error, got {error:?}"
    );
    assert!(error.to_string().contains("model.gguf"));
}

// AYEAYE-55
//
// Criterion 1's first half, and criterion 4 by construction: this runs on the
// CPU and nothing else.
#[test]
fn a_complete_quantized_model_loads_from_the_directory_it_was_pointed_at() {
    let dir = tiny_model("loads");

    let model = LanguageModel::load(dir.path()).expect("a complete model loads");

    assert_eq!(model.architecture(), "llama");
    assert_eq!(model.backend().label(), "cpu");
    // The Debug line is what ends up in a log, and it must describe the model
    // rather than print gigabytes of weights.
    let described = format!("{model:?}");
    assert!(described.contains("llama"), "{described}");
    assert!(described.contains("cpu"), "{described}");
}
