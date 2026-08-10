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
