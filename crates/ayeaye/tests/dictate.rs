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
#[tokio::test]
async fn the_probe_tells_a_model_that_is_here_from_one_that_was_only_chosen() {
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
        Policy::default(),
        converter.to_string(),
    )
    .probe()
    .await;
    assert!(here.speech_ready, "the model really is in the store");
    assert!(here.ok(), "{:?}", here.why());

    let chosen_only = dictate::Voice::new(
        store.0.clone(),
        settings(Some("openai/whisper-tiny.en"), None),
        Policy::default(),
        converter.to_string(),
    )
    .probe()
    .await;
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
        Policy::default(),
        "ayeaye-58-no-such-converter".to_string(),
    )
    .probe()
    .await;
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
        Policy::default(),
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
        Policy::default(),
        "true".to_string(),
    );

    assert!(!voice.sweep(std::time::Instant::now()).await);
    assert!(!voice.probe().await.speech_ready, "nothing was loaded to sweep");
}

// ------------------------------------------------------------- the tmux path

mod common;

use ayeaye_core::dictation::State;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// A stand-in for `bin/voice-agent`, on a port the kernel picked.
///
/// A real socket speaking real HTTP/1.1, because the thing under test is a
/// hand-rolled client and a substitute for the protocol would prove only that
/// the substitute agrees with it.
struct Agent {
    port: u16,
    stops: Arc<AtomicUsize>,
}

impl Agent {
    async fn started(healthy: bool, clip: &'static [u8]) -> Agent {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("a port");
        let port = listener.local_addr().expect("a bound address").port();
        let stops = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&stops);

        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    return;
                };
                let counted = Arc::clone(&counted);
                tokio::spawn(async move {
                    let mut raw = vec![0u8; 4096];
                    let read = stream.read(&mut raw).await.unwrap_or(0);
                    let request = String::from_utf8_lossy(&raw[..read]).into_owned();
                    // Every route is gated, exactly as the agent gates them:
                    // a peer on the tailnet without the secret must not be able
                    // to turn somebody's microphone on.
                    let authed = request.contains("X-Voice-Token: right-token");
                    let answer: Vec<u8> = if !authed {
                        b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 21\r\nConnection: close\r\n\r\n{\"error\":\"bad token\"}".to_vec()
                    } else if request.starts_with("GET /health") {
                        let body = if healthy {
                            r#"{"ok":true,"recorder":"ffmpeg"}"#
                        } else {
                            r#"{"ok":false,"recorder":"none"}"#
                        };
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        )
                        .into_bytes()
                    } else if request.starts_with("POST /start") {
                        b"HTTP/1.1 200 OK\r\nContent-Length: 11\r\nConnection: close\r\n\r\n{\"ok\":true}".to_vec()
                    } else {
                        counted.fetch_add(1, Ordering::Relaxed);
                        let mut out = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\n\
                             X-Audio-Ext: ogg\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            clip.len()
                        )
                        .into_bytes();
                        out.extend_from_slice(clip);
                        out
                    };
                    let _ = stream.write_all(&answer).await;
                    let _ = stream.shutdown().await;
                });
            }
        });

        Agent { port, stops }
    }
}

/// A dictation that answers with whatever it was told to, recording the clip.
struct Says {
    outcome: Outcome,
    heard: std::sync::Mutex<Vec<(Vec<u8>, String, String)>>,
}

impl Says {
    fn heard(text: &str) -> Says {
        Says {
            outcome: Outcome::Heard {
                raw: text.to_string(),
                cleaned: settle(&Policy::default(), text, None),
            },
            heard: std::sync::Mutex::new(Vec::new()),
        }
    }
}

impl dictate::Dictates for Says {
    async fn dictate(&self, clip: &[u8], ext: &str, names: &str) -> Outcome {
        self.heard.lock().expect("the lock").push((
            clip.to_vec(),
            ext.to_string(),
            names.to_string(),
        ));
        self.outcome.clone()
    }
}

fn state_path(what: &str) -> std::path::PathBuf {
    let path = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("dictate-state-{what}"))
        .join(ayeaye::dictate::STATE_FILE);
    let _ = std::fs::remove_dir_all(path.parent().expect("a directory"));
    path
}

// AYEAYE-58
//
// The whole tmux path in one case: the first press starts the recorder and puts
// the indicator up, the second retrieves the audio, dictates it, and types the
// words into the pane — **without submitting them**. That last assertion is the
// point of the feature: a person reads what a model wrote before an agent acts
// on it.
#[tokio::test]
async fn a_second_press_types_the_words_into_the_pane_and_never_submits_them() {
    if !common::have_tmux() {
        return;
    }
    let Some(server) = common::Private::named("toggle") else {
        return;
    };
    let agent = Agent::started(true, b"pretend this is ogg").await;
    let voice = Says::heard("run the tests");
    let state = state_path("typed");
    let tmux = server.layer();
    let toggle = ayeaye::dictate::Toggle {
        tmux: &tmux,
        voice: &voice,
        state: &state,
        token: "right-token".to_string(),
        port: agent.port,
    };

    // The pane the words go into, named the way every id in this app is.
    let pane = tmux
        .ask(&["display-message", "-p", "-t", "work", "#{pane_id}"])
        .await
        .expect("tmux should name its own pane");
    let qualified = format!("desktop/{}", pane.trim());

    let said = toggle.press(&qualified, None).await;
    assert_eq!(said, "", "starting a recording is not news: {said}");
    assert!(state.exists(), "the recording has to be remembered");
    assert_eq!(
        ayeaye::dictate::read_state(&state).map(|s| s.pane),
        Some(qualified.clone())
    );

    let said = toggle.press(&qualified, None).await;
    assert_eq!(said, "", "a dictation that worked says nothing: {said}");
    assert!(!state.exists(), "the recording is over");
    assert_eq!(agent.stops.load(Ordering::Relaxed), 1);

    // The audio really came from the agent, and the container it named is the
    // one the dictation was told about.
    let heard = voice.heard.lock().expect("the lock");
    assert_eq!(heard.len(), 1);
    assert_eq!(heard[0].0, b"pretend this is ogg");
    assert_eq!(heard[0].1, "ogg");
    drop(heard);

    // And the pane holds the words, with no submission: `send-keys -l` writes
    // the text and nothing else, so the shell has not run anything.
    let screen = server.captured(pane.trim());
    assert!(
        screen.contains("run the tests"),
        "the words are not in the pane: {screen:?}"
    );
    assert!(
        !screen.contains("command not found") && !screen.contains("not found"),
        "the text was submitted: {screen:?}"
    );
}

// AYEAYE-58
//
// A recorder that will not take the token is a sentence naming the machine, not
// a recording nobody started and a state file claiming otherwise.
#[tokio::test]
async fn a_recorder_that_refuses_the_token_leaves_no_recording_behind() {
    if !common::have_tmux() {
        return;
    }
    let Some(server) = common::Private::named("toggle-token") else {
        return;
    };
    let agent = Agent::started(true, b"never reached").await;
    let voice = Says::heard("never reached");
    let state = state_path("token");
    let tmux = server.layer();
    let toggle = ayeaye::dictate::Toggle {
        tmux: &tmux,
        voice: &voice,
        state: &state,
        token: "wrong-token".to_string(),
        port: agent.port,
    };

    let said = toggle.press("desktop/%0", None).await;

    assert!(said.contains("rejected the token"), "{said}");
    assert!(
        !state.exists(),
        "a recording that never started must not be remembered"
    );
}

// AYEAYE-58
//
// No agent at all is the common case — a machine where nobody has installed the
// recorder — and it has to name the device rather than fail oddly.
#[tokio::test]
async fn no_recorder_on_that_device_is_a_sentence_naming_it() {
    if !common::have_tmux() {
        return;
    }
    let Some(server) = common::Private::named("toggle-absent") else {
        return;
    };
    // A port nothing is listening on: bound, then dropped.
    let port = {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("a port");
        listener.local_addr().expect("an address").port()
    };
    let voice = Says::heard("never reached");
    let state = state_path("absent");
    let tmux = server.layer();
    let toggle = ayeaye::dictate::Toggle {
        tmux: &tmux,
        voice: &voice,
        state: &state,
        token: "right-token".to_string(),
        port,
    };

    let said = toggle.press("desktop/%0", Some("100.101.102.103")).await;

    assert!(
        said.starts_with("voice: no recorder on 100.101.102.103"),
        "{said}"
    );
    assert!(!state.exists());
}

// AYEAYE-58
//
// The state is written to a temporary name and renamed over, so a crash leaves
// either the old state or the new one. Asserted by watching the directory: the
// real name never holds a partial file, and no temporary is left behind.
#[test]
fn the_recording_state_is_renamed_into_place_and_leaves_nothing_behind() {
    let state = state_path("atomic");
    let directory = state.parent().expect("a directory").to_path_buf();
    let one = State {
        host: "100.101.102.103".to_string(),
        label: "phone".to_string(),
        pane: "desktop/%0".to_string(),
    };

    ayeaye::dictate::write_state(&state, &one).expect("it should be writable");
    let left: Vec<String> = std::fs::read_dir(&directory)
        .expect("the directory")
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        left,
        vec![ayeaye::dictate::STATE_FILE.to_string()],
        "a temporary file was left beside the state"
    );
    assert_eq!(ayeaye::dictate::read_state(&state), Some(one));

    // A file torn off half-written is not a recording in progress, so the next
    // press starts one rather than refusing forever.
    std::fs::write(&state, "{\"host\":\"1.2.3").expect("a torn file");
    assert_eq!(ayeaye::dictate::read_state(&state), None);

    ayeaye::dictate::clear_state(&state);
    assert!(!state.exists());
    // Clearing a state that is not there is not an error.
    ayeaye::dictate::clear_state(&state);
}

// AYEAYE-58
//
// The half that says the write really is a rename rather than a write to the
// real name. The temporary this process would use is occupied by a directory,
// so the write cannot be made — and the recording that was there survives whole.
//
// A write straight to the real name would have succeeded instead, replacing it.
// That is the failure this is here to catch, and it is why the test knows the
// temporary's name: the name carrying this process's id *is* the contract, both
// for atomicity and so two daemons cannot rename each other's half-written file
// into place.
#[test]
fn a_write_that_cannot_be_staged_leaves_the_recording_that_was_there() {
    let state = state_path("staging");
    let before = State {
        host: "100.101.102.103".to_string(),
        label: "phone".to_string(),
        pane: "desktop/%0".to_string(),
    };
    ayeaye::dictate::write_state(&state, &before).expect("the first one is writable");

    let temporary = state.with_extension(format!("tmp-{}", std::process::id()));
    std::fs::create_dir_all(&temporary).expect("something in the way of the temporary");

    let refused = ayeaye::dictate::write_state(
        &state,
        &State {
            host: "10.0.0.1".to_string(),
            label: "laptop".to_string(),
            pane: "desktop/%9".to_string(),
        },
    );

    assert!(refused.is_err(), "the write could not have been staged");
    assert_eq!(
        ayeaye::dictate::read_state(&state),
        Some(before),
        "a write that could not be staged must leave the recording that was there"
    );
}

// --------------------------------------------------- residency, without models

/// A speech slot that loads instantly and says what it was told to.
///
/// The real one is a directory of weights and hundreds of megabytes of device
/// memory. Substituting it is the same boundary `models::Slot` already is, and
/// it is what lets the two branches below be watched at all.
#[derive(Default)]
struct StubSpeech {
    resident: bool,
    releases: usize,
}

impl ayeaye::models::Slot for StubSpeech {
    type Error = String;

    fn load(&mut self, _: &std::path::Path) -> Result<(), String> {
        self.resident = true;
        Ok(())
    }

    fn unload(&mut self) -> bool {
        self.releases += usize::from(self.resident);
        std::mem::take(&mut self.resident)
    }
}

impl Speech for StubSpeech {
    fn transcribe(&mut self, _: &Pcm16kMono) -> Result<String, String> {
        assert!(self.resident, "a slot that is not loaded cannot transcribe");
        Ok("um so run the tests".to_string())
    }
}

/// A cleanup slot whose weights are not there.
struct NoWeights;

impl ayeaye::models::Slot for NoWeights {
    type Error = String;

    fn load(&mut self, _: &std::path::Path) -> Result<(), String> {
        Err("model.gguf: No such file or directory".to_string())
    }

    fn unload(&mut self) -> bool {
        false
    }
}

impl Cleanup for NoWeights {
    fn clean(&mut self, _: &str, _: &str, _: &Policy) -> Cleaned {
        panic!("a cleanup model that would not load must never be asked to clean");
    }
}

/// A cleanup slot that loads and rewrites.
#[derive(Default)]
struct StubCleanup {
    resident: bool,
    releases: usize,
}

impl ayeaye::models::Slot for StubCleanup {
    type Error = String;

    fn load(&mut self, _: &std::path::Path) -> Result<(), String> {
        self.resident = true;
        Ok(())
    }

    fn unload(&mut self) -> bool {
        self.releases += usize::from(self.resident);
        std::mem::take(&mut self.resident)
    }
}

impl Cleanup for StubCleanup {
    fn clean(&mut self, raw: &str, _: &str, policy: &Policy) -> Cleaned {
        settle(policy, raw, Some("Run the tests."))
    }
}

// AYEAYE-58
//
// A cleanup model that is configured and will not load costs the rewrite and
// nothing else: the dictation comes back as the words the speaker said, with the
// reason recorded. That is the acceptance criterion in the one place it is
// easiest to get wrong — the model *was* asked for, so a naive implementation
// reaches for a slot that is empty and loses the dictation.
#[tokio::test]
async fn a_cleanup_model_that_will_not_load_costs_the_rewrite_and_not_the_words() {
    let store = Store::named("cleanup-broken");
    let voice = ayeaye::dictate::Voice::with_slots(
        store.0.clone(),
        settings(
            Some("openai/whisper-small.en"),
            Some("Qwen/Qwen2.5-7B-Instruct-GGUF"),
        ),
        Policy::default(),
        "true".to_string(),
        StubSpeech::default(),
        NoWeights,
    );

    let outcome = voice.hear_decoded(&speech(1.0), "").await;

    let Outcome::Heard { raw, cleaned } = &outcome else {
        panic!("expected the words anyway, got {outcome:?}");
    };
    assert_eq!(raw, "um so run the tests");
    assert_eq!(cleaned.text(), raw, "the dictation has to survive");
    assert_eq!(cleaned.kept(), Some(Kept::Unavailable));
}

// AYEAYE-58
//
// The other side: both models load, and the rewrite is what gets typed. Asserted
// beside the case above because the two differ by one branch, and a mistake in
// that branch would make one of them pass on its own.
#[tokio::test]
async fn both_models_loading_gives_the_rewrite_rather_than_the_raw_words() {
    let store = Store::named("cleanup-good");
    let voice = ayeaye::dictate::Voice::with_slots(
        store.0.clone(),
        settings(
            Some("openai/whisper-small.en"),
            Some("Qwen/Qwen2.5-7B-Instruct-GGUF"),
        ),
        Policy::default(),
        "true".to_string(),
        StubSpeech::default(),
        StubCleanup::default(),
    );

    let outcome = voice.hear_decoded(&speech(1.0), "").await;

    let Outcome::Heard { cleaned, .. } = &outcome else {
        panic!("expected words, got {outcome:?}");
    };
    assert_eq!(cleaned.text(), "Run the tests.");
    assert!(cleaned.was_rewritten());
}

// AYEAYE-58
//
// The sweeper's whole point: a model is released because nobody is dictating,
// and both of them go. The idle clock starts when the dictation *finished*, so a
// sweep inside the window keeps them and one past it does not.
#[tokio::test]
async fn a_model_nobody_has_dictated_through_is_let_go_of() {
    let store = Store::named("sweep-release");
    let idle = std::time::Duration::from_secs(60);
    let voice = ayeaye::dictate::Voice::with_slots(
        store.0.clone(),
        ModelSettings::resolve(
            |_| None,
            "AYEAYE_SPEECH_MODEL=openai/whisper-small.en\n\
             AYEAYE_CLEANUP_MODEL=Qwen/Qwen2.5-7B-Instruct-GGUF\n\
             AYEAYE_MODEL_IDLE=60s\n",
        )
        .expect("a readable configuration"),
        Policy::default(),
        "true".to_string(),
        StubSpeech::default(),
        StubCleanup::default(),
    );

    voice.hear_decoded(&speech(1.0), "").await;
    let finished = std::time::Instant::now();

    assert!(
        !voice.sweep(finished + idle / 2).await,
        "a model somebody dictated through a moment ago is still wanted"
    );
    assert!(
        voice.sweep(finished + idle * 2).await,
        "a model nobody has used for twice its idle time is not worth the memory"
    );
    // And sweeping again lets go of nothing, because there is nothing left.
    assert!(!voice.sweep(finished + idle * 4).await);
}

// AYEAYE-58
//
// The converter's flags, held to without running anything. Each is load-bearing:
// `-ac 1 -ar 16000` is the whole job, `pcm_s16le` and `-f wav` are what make the
// output the one shape the reader accepts rather than whatever the converter
// inferred from a file name, and `-nostdin` is what stops a converter that
// decided to prompt from holding a request until its deadline.
#[test]
fn the_converter_is_asked_for_the_one_shape_a_speech_model_reads() {
    let argv = audio::argv(
        "ffmpeg",
        std::path::Path::new("/tmp/x/clip.webm"),
        std::path::Path::new("/tmp/x/clip16k.wav"),
    );

    assert_eq!(argv[0], "ffmpeg");
    for flag in ["-nostdin", "-ac", "-ar", "-f", "-y"] {
        assert!(argv.iter().any(|arg| arg == flag), "{flag} is missing: {argv:?}");
    }
    let after = |flag: &str| {
        let at = argv.iter().position(|arg| arg == flag).expect("the flag");
        argv[at + 1].clone()
    };
    assert_eq!(after("-ac"), "1", "one channel: {argv:?}");
    assert_eq!(after("-ar"), "16000", "sixteen kilohertz: {argv:?}");
    assert_eq!(after("-c:a"), "pcm_s16le", "sixteen-bit samples: {argv:?}");
    assert_eq!(after("-f"), "wav", "a WAVE, whatever the file is called");
    // The input is named as data after `-i`, and the output is last.
    assert_eq!(after("-i"), "/tmp/x/clip.webm");
    assert_eq!(argv.last().unwrap(), "/tmp/x/clip16k.wav");
}
