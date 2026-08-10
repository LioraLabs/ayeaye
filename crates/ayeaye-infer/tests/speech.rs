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

// AYEAYE-54
//
// The acceptance criterion, end to end: audio in, text out, in this process.
// Nothing here starts a subprocess or opens a socket — the proof that CPU
// inference works with no external speech runtime is that this test passes on
// a machine with none installed.
#[test]
fn a_clip_shorter_than_the_window_transcribes_in_one_segment() {
    let dir = tiny_model("one-window", &tiny_config(vec![]));
    let mut model = SpeechModel::load(dir.path()).expect("the toy model should load");

    let transcript = model
        .transcribe(&support::tone(1.0))
        .expect("a second of audio should transcribe");

    assert_eq!(transcript.segments.len(), 1);
    assert_eq!(transcript.segments[0].start_secs, 0.0);
    assert_eq!(transcript.segments[0].end_secs, 1.0);
    assert_eq!(model.backend(), ayeaye_infer::Backend::Cpu);
    // Not an assertion about *what* it heard — these weights are noise — but
    // that it heard something. It is what makes the suppression test below
    // mean anything.
    assert!(
        !transcript.segments[0].text.is_empty(),
        "the model should have decoded something"
    );
}

// AYEAYE-54
//
// The window is one second in this toy model, so 2.5 seconds is three windows
// and the last one is short. Truncating at the window and returning anyway is
// the failure this guards: it is indistinguishable from a successful
// transcription of a shorter recording.
#[test]
fn audio_longer_than_the_window_is_decoded_window_by_window() {
    let dir = tiny_model("three-windows", &tiny_config(vec![]));
    let mut model = SpeechModel::load(dir.path()).expect("the toy model should load");

    let transcript = model
        .transcribe(&support::tone(2.5))
        .expect("two and a half seconds should transcribe");

    let spans: Vec<(f32, f32)> = transcript
        .segments
        .iter()
        .map(|s| (s.start_secs, s.end_secs))
        .collect();
    assert_eq!(spans, vec![(0.0, 1.0), (1.0, 2.0), (2.0, 2.5)]);
}

// AYEAYE-54
//
// A clip nobody spoke into is an empty transcript, and the model is never run:
// thirty seconds of zero-padding through an encoder to be told nothing was
// said is a cost with no answer attached.
#[test]
fn an_empty_clip_is_an_empty_transcript() {
    let dir = tiny_model("empty", &tiny_config(vec![]));
    let mut model = SpeechModel::load(dir.path()).expect("the toy model should load");

    let transcript = model
        .transcribe(&ayeaye_core::Pcm16kMono::new(vec![]))
        .expect("an empty clip is legal input");

    assert!(transcript.segments.is_empty());
    assert!(transcript.is_empty());
}

// AYEAYE-54
//
// `suppress_tokens` is the model's own list of things it must never say, and
// Whisper ships one. Suppressing everything except the end token is the
// version of that claim a toy model can be held to: whatever the weights would
// have produced, the only move left is to stop, so every segment comes back
// empty. A no-op suppression fails this, because the unsuppressed run of the
// same model on the same audio produces text — which the test above asserts.
#[test]
fn a_suppressed_token_is_never_decoded() {
    let end = support::special_id("<|endoftext|>");
    let everything_else: Vec<u32> = (0..support::VOCAB as u32).filter(|id| *id != end).collect();
    let dir = tiny_model("suppressed", &tiny_config(everything_else));
    let mut model = SpeechModel::load(dir.path()).expect("the toy model should load");

    let transcript = model
        .transcribe(&support::tone(1.0))
        .expect("a second of audio should transcribe");

    assert_eq!(transcript.segments.len(), 1);
    assert_eq!(transcript.segments[0].text, "");
    assert!(transcript.is_empty());
}

// AYEAYE-54
//
// Residency is the daemon's, not the slot's. Transcribing into an empty slot
// is the moment an implicit design would quietly load a model, so this is the
// test that says it must not: an error, and still empty afterwards.
#[test]
fn transcribing_an_empty_slot_is_an_error_and_loads_nothing() {
    let mut slot = ayeaye_infer::SpeechSlot::empty();

    let error = slot
        .transcribe(&support::tone(1.0))
        .expect_err("an empty slot has nothing to transcribe with");

    assert!(matches!(error, SpeechError::NotLoaded));
    assert!(!slot.is_loaded(), "it must not have loaded one to find out");
}

// AYEAYE-54
//
// Load and unload are both explicit, both repeatable, and unload really lets
// go — the sequence AYEAYE-56's residency policy will drive, from a place that
// cannot know which state it is in.
#[test]
fn a_model_can_be_loaded_unloaded_and_loaded_again() {
    let dir = tiny_model("residency", &tiny_config(vec![]));
    let mut slot = ayeaye_infer::SpeechSlot::empty();
    assert!(!slot.is_loaded());

    slot.load(dir.path()).expect("the toy model should load");
    assert!(slot.is_loaded());
    assert!(!slot.transcribe(&support::tone(1.0)).unwrap().is_empty());

    assert!(slot.unload(), "unload should say it released something");
    assert!(!slot.is_loaded());
    assert!(!slot.unload(), "unloading nothing releases nothing");
    assert!(slot.transcribe(&support::tone(1.0)).is_err());

    slot.load(dir.path()).expect("it should load a second time");
    assert!(slot.is_loaded());
    assert!(!slot.transcribe(&support::tone(1.0)).unwrap().is_empty());
}

// AYEAYE-54
//
// The window every released Whisper actually has. The other transcription
// tests use a one-second window to keep their arithmetic readable, which means
// none of them would notice if the 3 000-frame path were wrong — and 3 000 is
// exactly where `pcm_to_mel`'s padding rounds differently. A real 30-second
// window, and a clip that runs past it.
#[test]
fn a_thirty_second_window_is_the_shape_a_real_whisper_asks_for() {
    let dir = tiny_model("real-window", &support::real_window_config());
    let mut model = SpeechModel::load(dir.path()).expect("the toy model should load");

    let transcript = model
        .transcribe(&support::tone(31.0))
        .expect("thirty-one seconds should transcribe");

    let spans: Vec<(f32, f32)> = transcript
        .segments
        .iter()
        .map(|s| (s.start_secs, s.end_secs))
        .collect();
    assert_eq!(spans, vec![(0.0, 30.0), (30.0, 31.0)]);
}

// AYEAYE-54
//
// Loading releases the resident model before it reads the new one, so a load
// that fails leaves the slot empty rather than holding the old model. That is
// a deliberate trade and worth pinning: it is what stops a reconfiguration
// from briefly holding two models, and the price is that a bad path costs you
// the working one. AYEAYE-56's policy needs to know which of the two it gets.
#[test]
fn a_failed_load_leaves_the_slot_empty_rather_than_holding_the_old_model() {
    let dir = tiny_model("failed-reload", &tiny_config(vec![]));
    let mut slot = ayeaye_infer::SpeechSlot::empty();
    slot.load(dir.path()).expect("the toy model should load");
    assert!(slot.is_loaded());

    let error = slot
        .load(std::path::Path::new("/nowhere/no-such-model"))
        .expect_err("there is no model there");

    assert!(matches!(error, SpeechError::Missing { .. }));
    assert!(!slot.is_loaded(), "the old model was released, not retained");
    assert!(slot.transcribe(&support::tone(1.0)).is_err());
}
