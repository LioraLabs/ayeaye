//! In-process speech transcription, observed only through its public API.

mod support;

use ayeaye_infer::speech::{SpeechError, SpeechModel};
use support::{tiny_config, tiny_model};

// AYEAYE-54
//
// "The model did not load" is the error message that costs an evening. Each of
// the three files gets its own test because each is read at a different point,
// and a loader that names the first file whatever went missing would pass a
// single one of these.
#[test]
fn an_absent_config_is_an_error_naming_the_config() {
    let dir = tiny_model("no-config", &tiny_config(vec![]));
    dir.remove("config.json");

    let error = SpeechModel::load(dir.path()).expect_err("a model with no config cannot load");

    assert!(
        matches!(error, SpeechError::Missing { .. }),
        "expected a Missing error, got {error:?}"
    );
    assert!(
        error.to_string().contains("config.json"),
        "the error should name the file it wanted: {error}"
    );
}

// AYEAYE-54
#[test]
fn an_absent_tokenizer_is_an_error_naming_the_tokenizer() {
    let dir = tiny_model("no-tokenizer", &tiny_config(vec![]));
    dir.remove("tokenizer.json");

    let error = SpeechModel::load(dir.path()).expect_err("a model with no tokenizer cannot load");

    assert!(matches!(error, SpeechError::Missing { .. }));
    assert!(
        error.to_string().contains("tokenizer.json"),
        "the error should name the file it wanted: {error}"
    );
}

// AYEAYE-54
#[test]
fn absent_weights_are_an_error_naming_the_weights() {
    let dir = tiny_model("no-weights", &tiny_config(vec![]));
    dir.remove("model.safetensors");

    let error = SpeechModel::load(dir.path()).expect_err("a model with no weights cannot load");

    assert!(matches!(error, SpeechError::Missing { .. }));
    assert!(
        error.to_string().contains("model.safetensors"),
        "the error should name the file it wanted: {error}"
    );
}

// AYEAYE-54
//
// A file that is present and wrong is a different problem from one that is
// absent, and collapsing the two sends someone looking for a missing download
// that is sitting right there.
#[test]
fn a_config_that_is_not_json_is_malformed_rather_than_missing() {
    let dir = tiny_model("bad-config", &tiny_config(vec![]));
    dir.corrupt("config.json");

    let error = SpeechModel::load(dir.path()).expect_err("a model with a broken config cannot load");

    assert!(
        matches!(error, SpeechError::Malformed { .. }),
        "expected a Malformed error, got {error:?}"
    );
    assert!(error.to_string().contains("config.json"));
}

// AYEAYE-54
//
// The whole point of the support module: a structurally real Whisper, built in
// the test, loads from a directory the caller named. No weights are shipped
// and none are downloaded.
#[test]
fn a_model_loads_from_the_directory_it_was_pointed_at() {
    let dir = tiny_model("loads", &tiny_config(vec![]));

    let model = SpeechModel::load(dir.path()).expect("the toy model should load");

    assert_eq!(model.mel_bins(), support::MEL_BINS);
}
