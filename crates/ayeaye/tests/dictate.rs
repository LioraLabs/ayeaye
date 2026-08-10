//! Dictation, observed from outside the process that does it.
//!
//! The converter is a real program on the machine, so every case that needs one
//! skips where it is not there rather than failing — a suite that fails on a
//! machine with no `ffmpeg` is telling you about the machine, not the code.

use std::process::Command;

use ayeaye::audio::{self, DecodeError};

/// Whether this machine has a converter for a test to ask at all.
fn have_converter() -> bool {
    Command::new(audio::CONVERTER)
        .arg("-version")
        .output()
        .is_ok()
}

/// A WAVE file, at whatever shape a recorder might have sent.
///
/// Deliberately *not* the shape the reader accepts: the point of the converter
/// is that it turns something else into that shape, and a test that handed it
/// audio already at 16 kHz mono would pass on a converter that copied its input.
fn stereo_44100(seconds: f32) -> Vec<u8> {
    let frames = (seconds * 44_100.0) as usize;
    let mut data = Vec::new();
    for frame in 0..frames {
        let t = frame as f32 / 44_100.0;
        let sample = (8_000.0 * (2.0 * std::f32::consts::PI * 440.0 * t).sin()) as i16;
        // Two channels, so a converter that ignored `-ac 1` produces twice the
        // samples and the assertion below notices.
        data.extend_from_slice(&sample.to_le_bytes());
        data.extend_from_slice(&sample.to_le_bytes());
    }

    let mut out = Vec::new();
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&2u16.to_le_bytes()); // stereo
    out.extend_from_slice(&44_100u32.to_le_bytes());
    out.extend_from_slice(&(44_100u32 * 4).to_le_bytes());
    out.extend_from_slice(&4u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(&data);
    out
}

// AYEAYE-58
//
// A clip in the shape a recorder sends comes back in the one shape a speech
// model reads. The duration is what is asserted rather than the sample count on
// its own, because that is the property a resample has to preserve and the one a
// converter told to do the wrong thing gets wrong.
#[tokio::test]
async fn a_recorded_clip_becomes_sixteen_kilohertz_mono_audio() {
    if !have_converter() {
        return;
    }

    let decoded = audio::decode(&stereo_44100(0.5), "wav")
        .await
        .expect("a converter should read a plain WAVE");

    assert!(
        (decoded.duration_secs() - 0.5).abs() < 0.05,
        "half a second of audio came back as {} seconds",
        decoded.duration_secs()
    );
    // And it is loud enough to be worth transcribing, which is the other half of
    // what this step is for.
    assert!(!decoded.is_silence(), "a tone is not room tone");
}

// AYEAYE-58
//
// The extension arrives from a request and becomes the name of a file this
// process writes, so it is checked against a list rather than reasoned about —
// and checked before anything is written, so a refused request costs no disk.
#[tokio::test]
async fn a_container_this_build_does_not_read_is_refused_by_name() {
    let refused = audio::decode(b"whatever", "wav.exe")
        .await
        .expect_err("an extension off the list cannot be accepted");

    assert_eq!(refused, DecodeError::BadExtension("wav.exe".to_string()));
    assert!(refused.to_string().contains("wav.exe"), "{refused}");
    // A path separator is not an extension either.
    assert!(matches!(
        audio::decode(b"whatever", "../../etc/passwd").await,
        Err(DecodeError::BadExtension(_))
    ));
}

// AYEAYE-58
//
// A machine with no converter is the state most machines are in until somebody
// installs one, and it has to be a sentence rather than a panic — the rest of
// the app keeps working without voice.
#[tokio::test]
async fn a_machine_with_no_converter_says_so_rather_than_failing_oddly() {
    let refused = audio::decode_with("ayeaye-58-no-such-converter", b"whatever", "webm")
        .await
        .expect_err("there is no converter by that name");

    assert!(
        matches!(refused, DecodeError::NoConverter(_)),
        "{refused:?}"
    );
    assert!(refused.to_string().contains(audio::CONVERTER), "{refused}");
}

// AYEAYE-58
//
// A clip that is not audio is the converter's answer, in the converter's own
// words. Whoever has to fix it is better served by what the program said than
// by any paraphrase of it.
#[tokio::test]
async fn something_that_is_not_audio_comes_back_as_the_converters_own_reason() {
    if !have_converter() {
        return;
    }

    let refused = audio::decode(b"<!DOCTYPE html><title>404</title>", "webm")
        .await
        .expect_err("a 404 page is not a recording");

    let DecodeError::Refused(said) = &refused else {
        panic!("expected the converter's refusal, got {refused:?}");
    };
    assert!(!said.is_empty(), "the converter's reason is the message");
}

// ------------------------------------------------------------- the pipeline

use ayeaye::dictate::{self, Cleanup, Speech};
use ayeaye_core::Pcm16kMono;
use ayeaye_core::cleanup::{Cleaned, Kept, Policy, settle};
use ayeaye_core::dictation::Outcome;

/// A speech model that says what it was told to, and remembers being asked.
///
/// The point of substituting it is the point of `models::Slot`: a real one is a
/// directory of weights and hundreds of megabytes of device memory, and the
/// properties worth asserting here are about the order things happen in.
struct Heard {
    said: Result<String, String>,
    asked: usize,
}

impl Heard {
    fn saying(text: &str) -> Heard {
        Heard {
            said: Ok(text.to_string()),
            asked: 0,
        }
    }
}

impl Speech for Heard {
    fn transcribe(&mut self, _audio: &Pcm16kMono) -> Result<String, String> {
        self.asked += 1;
        self.said.clone()
    }
}

/// A cleanup model that rewrites to a fixed answer, remembering the names.
struct Rewrites {
    into: Option<String>,
    names: Vec<String>,
}

impl Rewrites {
    fn into(text: &str) -> Rewrites {
        Rewrites {
            into: Some(text.to_string()),
            names: Vec::new(),
        }
    }
}

impl Cleanup for Rewrites {
    fn clean(&mut self, raw: &str, names: &str, policy: &Policy) -> Cleaned {
        self.names.push(names.to_string());
        settle(policy, raw, self.into.as_deref())
    }
}

/// Audio loud enough to be worth transcribing.
fn speech(seconds: f32) -> Pcm16kMono {
    let samples = (seconds * 16_000.0) as usize;
    Pcm16kMono::new(
        (0..samples)
            .map(|i| if i % 2 == 0 { 0.3 } else { -0.3 })
            .collect(),
    )
}

// AYEAYE-58
//
// The whole path in one case: audio in, words out, cleaned up, with the names
// off the pane handed to the cleanup step.
#[test]
fn a_clip_of_speech_becomes_a_cleaned_up_line_primed_by_the_pane() {
    let mut speech_model = Heard::saying("um so run the parse config tests");
    let mut cleanup = Rewrites::into("Run the parse_config tests.");

    let outcome = dictate::hear(
        &mut speech_model,
        &mut cleanup,
        &speech(1.0),
        "parse_config server.py",
        &Policy::default(),
    );

    let Outcome::Heard { raw, cleaned } = &outcome else {
        panic!("expected words, got {outcome:?}");
    };
    assert_eq!(raw, "um so run the parse config tests");
    assert_eq!(cleaned.text(), "Run the parse_config tests.");
    assert!(cleaned.was_rewritten());
    assert_eq!(
        cleanup.names,
        vec!["parse_config server.py".to_string()],
        "the names on the pane have to reach the cleanup step"
    );
}

// AYEAYE-58
//
// The gate comes first, and that is the assertion: transcription is seconds of
// a model's time, and a clip nobody spoke into is knowable for free.
#[test]
fn a_clip_nobody_spoke_into_is_refused_before_a_model_is_asked() {
    let mut speech_model = Heard::saying("thank you");
    let mut cleanup = Rewrites::into("Thank you.");
    let quiet = Pcm16kMono::from_i16(&[400, -400, 400, -400]);

    let outcome = dictate::hear(
        &mut speech_model,
        &mut cleanup,
        &quiet,
        "",
        &Policy::default(),
    );

    assert!(matches!(outcome, Outcome::Silence { .. }), "{outcome:?}");
    assert_eq!(
        speech_model.asked, 0,
        "silence must not cost a transcription"
    );
    // And the loudness rides along, because it is the number that tells somebody
    // their microphone is muted rather than their voice quiet.
    assert!(outcome.body().contains(r#""rms":"#), "{}", outcome.body());
}

// AYEAYE-58
//
// A clip can clear the energy gate — a door, a cough — and still transcribe to
// one of a speech model's stock answers to silence. Typing that into somebody's
// terminal is worse than typing nothing.
#[test]
fn a_stock_answer_to_silence_is_nothing_recognised_rather_than_a_dictation() {
    for said in ["", "   ", "Thank you.", "[BLANK_AUDIO]"] {
        let mut speech_model = Heard::saying(said);
        let mut cleanup = Rewrites::into("Something else entirely.");

        let outcome = dictate::hear(
            &mut speech_model,
            &mut cleanup,
            &speech(1.0),
            "",
            &Policy::default(),
        );

        assert!(
            matches!(outcome, Outcome::Empty { .. }),
            "{said:?}: {outcome:?}"
        );
        assert!(
            cleanup.names.is_empty(),
            "{said:?} is not worth a cleanup model's time"
        );
    }
}

// AYEAYE-58
//
// A speech model that failed is not a dictation that failed differently: there
// are no words, and there is nothing to type.
#[test]
fn a_transcription_that_failed_is_said_out_loud() {
    let mut speech_model = Heard {
        said: Err("the model is not loaded".to_string()),
        asked: 0,
    };
    let mut cleanup = Rewrites::into("anything");

    let outcome = dictate::hear(
        &mut speech_model,
        &mut cleanup,
        &speech(1.0),
        "",
        &Policy::default(),
    );

    let Outcome::Unavailable(why) = &outcome else {
        panic!("expected an unavailable model, got {outcome:?}");
    };
    assert!(why.contains("not loaded"), "{why}");
}

// AYEAYE-58
//
// The rule the whole cleanup design exists for, at the level that matters: a
// machine with no cleanup model dictates the words the speaker said.
#[test]
fn a_machine_with_no_cleanup_model_still_types_what_was_said() {
    let mut speech_model = Heard::saying("um so run the tests");

    let outcome = dictate::hear(
        &mut speech_model,
        &mut dictate::AsSpoken,
        &speech(1.0),
        "",
        &Policy::default(),
    );

    let Outcome::Heard { raw, cleaned } = &outcome else {
        panic!("expected words, got {outcome:?}");
    };
    assert_eq!(cleaned.text(), raw);
    assert_eq!(cleaned.kept(), Some(Kept::Unavailable));
    assert_eq!(
        outcome.body(),
        r#"{"raw":"um so run the tests","final":"um so run the tests"}"#
    );
}

// ---------------------------------------------------------------- the holder

use ayeaye_core::model::ModelId;
use ayeaye_core::model::settings::ModelSettings;

/// A store of this test's own, removed when it goes out of scope.
struct Store(std::path::PathBuf);

impl Store {
    fn named(what: &str) -> Store {
        let path = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("dictate-{what}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("a store");
        Store(path)
    }

    /// Put a model in it, as a pull would have.
    fn holding(self, id: &ModelId) -> Store {
        std::fs::create_dir_all(self.0.join(id.relative_dir())).expect("a model directory");
        self
    }
}

impl Drop for Store {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn settings(speech: Option<&str>, cleanup: Option<&str>) -> ModelSettings {
    let mut file = String::new();
    if let Some(speech) = speech {
        file.push_str(&format!("AYEAYE_SPEECH_MODEL={speech}\n"));
    }
    if let Some(cleanup) = cleanup {
        file.push_str(&format!("AYEAYE_CLEANUP_MODEL={cleanup}\n"));
    }
    ModelSettings::resolve(|_| None, &file).expect("a readable configuration")
}

// AYEAYE-58
//
// The probe answers about the store as it is, not about the configuration file.
// A model chosen and never pulled is the state a machine is in between two
// commands, and calling it ready lights up a talk button that cannot work.
#[test]
fn the_probe_tells_a_model_that_is_here_from_one_that_was_only_chosen() {
    let speech = ModelId::parse("openai/whisper-small.en").expect("an id");
    let store = Store::named("probe").holding(&speech);
    let converter = if have_converter() {
        audio::CONVERTER
    } else {
        "true"
    };

    let here = dictate::Voice::new(
        store.0.clone(),
        settings(Some("openai/whisper-small.en"), None),
        converter.to_string(),
    )
    .probe();
    assert!(here.speech_ready, "the model really is in the store");
    assert!(here.ok(), "{:?}", here.why());

    let chosen_only = dictate::Voice::new(
        store.0.clone(),
        settings(Some("openai/whisper-tiny.en"), None),
        converter.to_string(),
    )
    .probe();
    assert!(!chosen_only.speech_ready);
    assert!(!chosen_only.ok());
    assert!(
        chosen_only
            .why()
            .expect("a reason")
            .contains("whisper-tiny.en"),
        "{:?}",
        chosen_only.why()
    );

    // And a machine with no converter cannot dictate whatever it has pulled.
    let no_converter = dictate::Voice::new(
        store.0.clone(),
        settings(Some("openai/whisper-small.en"), None),
        "ayeaye-58-no-such-converter".to_string(),
    )
    .probe();
    assert!(!no_converter.converter);
    assert!(!no_converter.ok());
}

// AYEAYE-58
//
// A machine with no speech model refuses before the converter is started: there
// is nothing that could transcribe the result, so decoding is a process spent to
// reach a refusal that was knowable for free.
#[tokio::test]
async fn a_machine_with_no_speech_model_refuses_before_decoding_anything() {
    let store = Store::named("no-model");
    let voice = dictate::Voice::new(
        store.0.clone(),
        settings(None, None),
        "ayeaye-58-no-such-converter".to_string(),
    );

    let outcome = voice.dictate(&stereo_44100(0.2), "wav", "").await;

    // Not `Undecodable`, which is what a decode-first implementation would
    // answer here given a converter that is not on this machine.
    let Outcome::Unavailable(why) = &outcome else {
        panic!("expected an unavailable voice, got {outcome:?}");
    };
    assert!(why.contains("model"), "{why}");
}

// AYEAYE-58
//
// The sweeper never loads and never trips over a machine that has not dictated
// anything yet, which is every machine until somebody speaks.
#[tokio::test]
async fn sweeping_a_voice_that_has_never_been_used_lets_go_of_nothing() {
    let store = Store::named("sweep");
    let voice = dictate::Voice::new(
        store.0.clone(),
        settings(Some("openai/whisper-small.en"), None),
        "true".to_string(),
    );

    assert!(!voice.sweep(std::time::Instant::now()).await);
    assert!(!voice.probe().speech_ready, "nothing was loaded to sweep");
}
