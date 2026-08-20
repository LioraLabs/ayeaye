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
    assert!(
        !ayeaye_core::audio::is_silence(decoded.rms()),
        "a tone is not room tone"
    );
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

use ayeaye::dictate::{self, Cleanup, Lists, Speech};
use ayeaye_core::Pcm16kMono;
use ayeaye_core::cleanup::{Cleaned, Kept, Policy, settle};
use ayeaye_core::dictation::Outcome;
use ayeaye_core::model::settings::ModelSettings;
use std::sync::Mutex;

/// A backend that answers what it was told to, and remembers being asked.
///
/// One double for all three traits, because since AYEAYE-101 there is one thing
/// behind them: a proxy. Substituting it is the same boundary the two model
/// slots used to be — a real one is a process holding hundreds of megabytes of
/// device memory, and the properties worth asserting here are about the order
/// things happen in and what survives a failure.
struct Fake {
    /// What a transcription answers.
    said: Mutex<Result<String, String>>,
    /// What a rewrite answers. `None` is a model that declined to say anything
    /// usable, which is what an unreachable proxy looks like from here.
    rewrite: Mutex<Option<String>>,
    /// The models it will admit to serving.
    serving: Vec<String>,
    /// How many transcriptions were asked for.
    asked: Mutex<usize>,
    /// The names handed to each cleanup, in order.
    names: Mutex<Vec<String>>,
    /// The model each cleanup was asked of, in order.
    cleaned: Mutex<Vec<String>>,
}

impl Default for Fake {
    fn default() -> Fake {
        Fake {
            // An empty transcript rather than an error: a backend nobody told
            // what to say is one that heard nothing, which is a state, not a
            // fault.
            said: Mutex::new(Ok(String::new())),
            rewrite: Mutex::new(None),
            serving: Vec::new(),
            asked: Mutex::new(0),
            names: Mutex::new(Vec::new()),
            cleaned: Mutex::new(Vec::new()),
        }
    }
}

impl Fake {
    fn saying(text: &str) -> Fake {
        Fake {
            said: Mutex::new(Ok(text.to_string())),
            ..Fake::default()
        }
    }

    fn rewriting_to(self, text: &str) -> Fake {
        *self.rewrite.lock().expect("a lock") = Some(text.to_string());
        self
    }

    fn serving(mut self, models: &[&str]) -> Fake {
        self.serving = models.iter().map(|name| (*name).to_string()).collect();
        self
    }

    fn asked(&self) -> usize {
        *self.asked.lock().expect("a lock")
    }

    fn names(&self) -> Vec<String> {
        self.names.lock().expect("a lock").clone()
    }
}

impl Speech for Fake {
    async fn transcribe(&self, _model: &str, _audio: &Pcm16kMono) -> Result<String, String> {
        *self.asked.lock().expect("a lock") += 1;
        self.said.lock().expect("a lock").clone()
    }
}

impl Cleanup for Fake {
    async fn clean(&self, model: &str, raw: &str, names: &str, policy: &Policy) -> Cleaned {
        self.names.lock().expect("a lock").push(names.to_string());
        self.cleaned.lock().expect("a lock").push(model.to_string());
        let rewrite = self.rewrite.lock().expect("a lock").clone();
        settle(policy, raw, rewrite.as_deref())
    }
}

impl Lists for Fake {
    async fn available(&self) -> Result<Vec<String>, String> {
        Ok(self.serving.clone())
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
#[tokio::test]
async fn a_clip_of_speech_becomes_a_cleaned_up_line_primed_by_the_pane() {
    let backend = Fake::saying("um so run the parse config tests")
        .rewriting_to("Run the parse_config tests.");

    let outcome = dictate::hear(
        &backend,
        "whisper",
        Some("qwen"),
        &speech(1.0),
        "parse_config server.py",
        &Policy::default(),
    )
    .await;

    let Outcome::Heard { raw, cleaned } = &outcome else {
        panic!("expected words, got {outcome:?}");
    };
    assert_eq!(raw, "um so run the parse config tests");
    assert_eq!(cleaned.text(), "Run the parse_config tests.");
    assert!(cleaned.was_rewritten());
    assert_eq!(
        backend.names(),
        vec!["parse_config server.py".to_string()],
        "the names on the pane have to reach the cleanup step"
    );
    // And each model is asked for by the name it goes by in the backend's
    // config. One name for both would be the bug this cannot otherwise see.
    assert_eq!(
        *backend.cleaned.lock().expect("a lock"),
        vec!["qwen".to_string()]
    );
}

// AYEAYE-58
//
// The gate comes first, and that is the assertion: transcription is a round trip
// and a model's time, and a clip nobody spoke into is knowable for free.
#[tokio::test]
async fn a_clip_nobody_spoke_into_is_refused_before_a_model_is_asked() {
    let backend = Fake::saying("thank you").rewriting_to("Thank you.");
    let quiet = Pcm16kMono::from_i16(&[400, -400, 400, -400]);

    let outcome = dictate::hear(
        &backend,
        "whisper",
        Some("qwen"),
        &quiet,
        "",
        &Policy::default(),
    )
    .await;

    assert!(matches!(outcome, Outcome::Silence { .. }), "{outcome:?}");
    assert_eq!(backend.asked(), 0, "silence must not cost a transcription");
    // And the loudness rides along, because it is the number that tells somebody
    // their microphone is muted rather than their voice quiet.
    assert!(outcome.body().contains(r#""rms":"#), "{}", outcome.body());
}

// AYEAYE-58
//
// A clip can clear the energy gate — a door, a cough — and still transcribe to
// one of a speech model's stock answers to silence. Typing that into somebody's
// terminal is worse than typing nothing.
#[tokio::test]
async fn a_stock_answer_to_silence_is_nothing_recognised_rather_than_a_dictation() {
    for said in ["", "   ", "Thank you.", "[BLANK_AUDIO]"] {
        let backend = Fake::saying(said).rewriting_to("Something else entirely.");

        let outcome = dictate::hear(
            &backend,
            "whisper",
            Some("qwen"),
            &speech(1.0),
            "",
            &Policy::default(),
        )
        .await;

        assert!(
            matches!(outcome, Outcome::Empty { .. }),
            "{said:?}: {outcome:?}"
        );
        assert!(
            backend.names().is_empty(),
            "{said:?} is not worth a cleanup model's time"
        );
    }
}

// AYEAYE-58
//
// A speech model that failed is not a dictation that failed differently: there
// are no words, and there is nothing to type.
#[tokio::test]
async fn a_transcription_that_failed_is_said_out_loud() {
    let backend = Fake {
        said: Mutex::new(Err("it answered HTTP 400: no such model".to_string())),
        ..Fake::default()
    };

    let outcome = dictate::hear(
        &backend,
        "whisper",
        Some("qwen"),
        &speech(1.0),
        "",
        &Policy::default(),
    )
    .await;

    let Outcome::Unavailable(why) = &outcome else {
        panic!("expected an unavailable model, got {outcome:?}");
    };
    assert!(why.contains("no such model"), "{why}");
}

// AYEAYE-58
//
// The rule the whole cleanup design exists for, at the level that matters: a
// machine with no cleanup model dictates the words the speaker said. `None`
// rather than a second implementation standing in for the empty case — the two
// used to be different types, and the branch is the thing worth watching.
#[tokio::test]
async fn a_machine_with_no_cleanup_model_still_types_what_was_said() {
    let backend = Fake::saying("um so run the tests").rewriting_to("Run the tests.");

    let outcome = dictate::hear(
        &backend,
        "whisper",
        None,
        &speech(1.0),
        "",
        &Policy::default(),
    )
    .await;

    let Outcome::Heard { raw, cleaned } = &outcome else {
        panic!("expected words, got {outcome:?}");
    };
    assert_eq!(cleaned.text(), raw);
    assert_eq!(cleaned.kept(), Some(Kept::Unavailable));
    assert_eq!(
        outcome.body(),
        r#"{"raw":"um so run the tests","final":"um so run the tests"}"#
    );
    assert!(
        backend.names().is_empty(),
        "no cleanup model configured is no request, not a request that was ignored"
    );
}

// AYEAYE-58
//
// The other side of the same branch: a cleanup model that says nothing usable
// costs the rewrite and nothing else. The dictation comes back as the words the
// speaker said, which is the acceptance criterion in the place it is easiest to
// get wrong — the model *was* asked for, so an implementation that treated a
// failed rewrite as a failed dictation would lose the words.
#[tokio::test]
async fn a_cleanup_model_that_says_nothing_usable_costs_the_rewrite_and_not_the_words() {
    let backend = Fake::saying("um so run the tests");

    let outcome = dictate::hear(
        &backend,
        "whisper",
        Some("qwen"),
        &speech(1.0),
        "",
        &Policy::default(),
    )
    .await;

    let Outcome::Heard { raw, cleaned } = &outcome else {
        panic!("expected the words anyway, got {outcome:?}");
    };
    assert_eq!(raw, "um so run the tests");
    assert_eq!(cleaned.text(), raw, "the dictation has to survive");
    assert_eq!(cleaned.kept(), Some(Kept::Unavailable));
    assert_eq!(
        backend.names().len(),
        1,
        "and it really was asked, rather than skipped"
    );
}

// ---------------------------------------------------------------- the holder

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

fn voice(backend: Fake, settings: ModelSettings, converter: &str) -> dictate::Voice<Fake> {
    dictate::Voice::with_backend(settings, Policy::default(), converter.to_string(), backend)
}

// AYEAYE-58, rewritten by AYEAYE-101.
//
// The probe answers about what the backend is *serving*, not about what the
// configuration file names. A model chosen and never added to llama-swap's
// config is the state a machine is in between two edits, and calling it ready
// lights up a talk button that cannot work. The store walk this used to be is
// now a request, and the fact it establishes is the same one.
#[tokio::test]
async fn the_probe_tells_a_model_the_backend_serves_from_one_that_was_only_chosen() {
    let converter = if have_converter() {
        audio::CONVERTER
    } else {
        "true"
    };

    let here = voice(
        Fake::default().serving(&["whisper", "qwen"]),
        settings(Some("whisper"), None),
        converter,
    )
    .probe()
    .await;
    assert!(here.speech_ready, "the backend really is serving it");
    assert!(here.ok(), "{:?}", here.why());

    let chosen_only = voice(
        Fake::default().serving(&["qwen"]),
        settings(Some("whisper"), None),
        converter,
    )
    .probe()
    .await;
    assert!(!chosen_only.speech_ready);
    assert!(!chosen_only.ok());
    assert!(
        chosen_only.why().expect("a reason").contains("whisper"),
        "{:?}",
        chosen_only.why()
    );

    // And a machine with no converter cannot dictate whatever is being served.
    let no_converter = voice(
        Fake::default().serving(&["whisper"]),
        settings(Some("whisper"), None),
        "ayeaye-58-no-such-converter",
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
    let voice = voice(
        Fake::default(),
        settings(None, None),
        "ayeaye-58-no-such-converter",
    );

    let outcome = voice.dictate(&stereo_44100(0.2), "wav", "").await;

    // Not `Undecodable`, which is what a decode-first implementation would
    // answer here given a converter that is not on this machine.
    let Outcome::Unavailable(why) = &outcome else {
        panic!("expected an unavailable voice, got {outcome:?}");
    };
    assert!(why.contains("model"), "{why}");
}

// AYEAYE-101 — a voice reaches the backend for the model each role names, and
// for nothing else. This is what replaced the residency tests: there is no slot
// to load, so what is worth watching is that the two names in the config file
// reach the two requests unmixed.
#[tokio::test]
async fn each_role_is_asked_of_the_model_its_own_setting_names() {
    let voice = voice(
        Fake::saying("um so run the tests").rewriting_to("Run the tests."),
        settings(Some("whisper"), Some("qwen")),
        "true",
    );

    let outcome = voice.hear_decoded(&speech(1.0), "").await;

    let Outcome::Heard { cleaned, .. } = &outcome else {
        panic!("expected words, got {outcome:?}");
    };
    assert_eq!(cleaned.text(), "Run the tests.");
    assert!(cleaned.was_rewritten());
    assert_eq!(
        *voice.backend().cleaned.lock().expect("a lock"),
        vec!["qwen".to_string()],
        "the cleanup request goes to the cleanup model"
    );
    assert_eq!(voice.backend().asked(), 1);
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
    async fn started(healthy: bool, clip: impl Into<Arc<[u8]>>) -> Agent {
        let clip: Arc<[u8]> = clip.into();
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("a port");
        let port = listener.local_addr().expect("a bound address").port();
        let stops = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&stops);

        let clip = Arc::clone(&clip);
        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    return;
                };
                let counted = Arc::clone(&counted);
                let clip = Arc::clone(&clip);
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
                        out.extend_from_slice(&clip);
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
    let agent = Agent::started(true, b"pretend this is ogg".to_vec()).await;
    let voice = Says::heard("run the tests");
    let state = state_path("typed");
    let tmux = server.layer();
    let toggle = ayeaye::dictate::Toggle {
        tmux: &tmux,
        here: ayeaye_core::peer::HostName::new("desktop").expect("a host name"),
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
    let agent = Agent::started(true, b"never reached".to_vec()).await;
    let voice = Says::heard("never reached");
    let state = state_path("token");
    let tmux = server.layer();
    let toggle = ayeaye::dictate::Toggle {
        tmux: &tmux,
        here: ayeaye_core::peer::HostName::new("desktop").expect("a host name"),
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
        here: ayeaye_core::peer::HostName::new("desktop").expect("a host name"),
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
        assert!(
            argv.iter().any(|arg| arg == flag),
            "{flag} is missing: {argv:?}"
        );
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

// AYEAYE-58
//
// **Membership, not syntax**, on the tmux path too. The pane the words go into
// is read back out of a state file on disk, which outlives the process that
// wrote it and the pane it named — so an id naming another machine, or a pane
// that has since gone, must not act on the local pane of the same number.
//
// `PaneId::parse` would accept `elsewhere/%0` and hand back `%0`, which is a
// real pane on this machine. This is the test that says the host half is
// compared and the pane list is asked.
#[tokio::test]
async fn a_pane_on_another_machine_is_never_typed_into() {
    if !common::have_tmux() {
        return;
    }
    let Some(server) = common::Private::named("toggle-elsewhere") else {
        return;
    };
    let agent = Agent::started(true, b"pretend this is ogg".to_vec()).await;
    let voice = Says::heard("this must not be typed anywhere");
    let state = state_path("elsewhere");
    let tmux = server.layer();
    let toggle = ayeaye::dictate::Toggle {
        tmux: &tmux,
        here: ayeaye_core::peer::HostName::new("desktop").expect("a host name"),
        voice: &voice,
        state: &state,
        token: "right-token".to_string(),
        port: agent.port,
    };

    let pane = tmux
        .ask(&["display-message", "-p", "-t", "work", "#{pane_id}"])
        .await
        .expect("tmux should name its own pane");
    let pane = pane.trim().to_string();

    // A recording in progress whose pane is on another machine — which is what
    // a state file written before the daemon was renamed looks like.
    ayeaye::dictate::write_state(
        &state,
        &State {
            host: "127.0.0.1".to_string(),
            label: "local".to_string(),
            pane: format!("elsewhere/{pane}"),
        },
    )
    .expect("a recording in progress");

    let said = toggle.press(&format!("elsewhere/{pane}"), None).await;

    assert!(
        said.contains("not a pane on this machine"),
        "it should have refused the target: {said:?}"
    );
    let screen = server.captured(&pane);
    assert!(
        !screen.contains("this must not be typed anywhere"),
        "another machine's pane id typed into this machine's pane: {screen:?}"
    );
    // And a pane that is simply not there any more is the same answer.
    ayeaye::dictate::write_state(
        &state,
        &State {
            host: "127.0.0.1".to_string(),
            label: "local".to_string(),
            pane: "desktop/%99".to_string(),
        },
    )
    .expect("a recording in progress");
    assert!(
        toggle
            .press("desktop/%99", None)
            .await
            .contains("not a pane"),
        "a pane that has gone is not a pane"
    );
}

// AYEAYE-58
//
// An agent sending more than a dictation could be is refused, not truncated.
// The cap has to exist — `read_to_end` on a socket is bounded only by the
// deadline — but stopping silently at it and decoding what arrived would hand
// the converter a truncated container, which it will usually decode into a
// recording that stops mid-sentence. A dictation quietly missing its ending is
// worse than one that failed, because nobody knows to say it again.
#[tokio::test]
async fn an_agent_sending_more_than_a_dictation_is_refused_rather_than_truncated() {
    let agent = Agent::started(true, vec![0u8; ayeaye::recorder::MAX_CLIP + 1]).await;
    let recorder = ayeaye::recorder::Recorder::at("127.0.0.1", agent.port, "right-token");

    let refused = recorder.stop().await.expect_err("that is not a dictation");

    assert_eq!(
        refused,
        ayeaye::recorder::Unreachable::TooMuch(ayeaye::recorder::MAX_CLIP)
    );
    assert!(refused.to_string().contains("32 MB"), "{refused}");

    // And a clip inside the cap still comes back whole, so the guard is a cap
    // rather than a ceiling everything bumps into.
    let small = Agent::started(true, b"a real recording".to_vec()).await;
    let recorder = ayeaye::recorder::Recorder::at("127.0.0.1", small.port, "right-token");
    let reply = recorder.stop().await.expect("a recording of a sane size");
    assert_eq!(reply.body, b"a real recording");
}

// AYEAYE-58
//
// A recording agent that is running on a machine with no microphone it knows
// how to use answers 200 and says `{"ok": false}`. Reading the status alone
// would start a recording on a device that cannot record, and the person would
// find out at the second press with nothing to show for it.
#[tokio::test]
async fn an_agent_with_no_microphone_is_told_from_one_with_a_microphone() {
    if !common::have_tmux() {
        return;
    }
    let Some(server) = common::Private::named("toggle-backend") else {
        return;
    };
    let agent = Agent::started(false, b"never recorded".to_vec()).await;
    let voice = Says::heard("never reached");
    let state = state_path("backend");
    let tmux = server.layer();
    let toggle = ayeaye::dictate::Toggle {
        tmux: &tmux,
        here: ayeaye_core::peer::HostName::new("desktop").expect("a host name"),
        voice: &voice,
        state: &state,
        token: "right-token".to_string(),
        port: agent.port,
    };

    let said = toggle.press("desktop/%0", None).await;

    assert!(said.contains("no recording backend"), "{said}");
    assert!(
        !state.exists(),
        "a recording that could never have started must not be remembered"
    );
}

// --------------------------------------------- which machine has the microphone

/// A process table this test wrote.
struct Machines {
    /// pid -> the address it was reached from, where there is one.
    peers: Vec<(u32, Option<&'static str>)>,
}

impl ayeaye_core::process::Source for Machines {
    fn children(&self, _: u32) -> Vec<u32> {
        Vec::new()
    }

    fn comm(&self, _: u32) -> Option<String> {
        None
    }
}

impl ayeaye::process::Processes for Machines {
    fn start_time(&self, _: u32) -> Option<f64> {
        None
    }

    fn cwd(&self, _: u32) -> Option<String> {
        None
    }

    fn open_files(&self, _: u32) -> Vec<String> {
        Vec::new()
    }

    fn exists(&self, pid: u32) -> bool {
        self.peers.iter().any(|(known, _)| *known == pid)
    }

    fn ssh_peer(&self, pid: u32) -> Option<String> {
        self.peers
            .iter()
            .find(|(known, _)| *known == pid)
            .and_then(|(_, peer)| peer.map(str::to_string))
    }
}

// AYEAYE-58
//
// Recording happens where the person is, not where tmux is, so picking the
// client is picking the room. The pid tmux handed the binding wins when it is
// alive; when it is not — a client that has since detached — the pane's own
// session's clients are asked, and a remote one is the interesting one.
#[test]
fn the_microphone_is_the_one_belonging_to_the_client_that_pressed_the_key() {
    let machines = Machines {
        peers: vec![
            (100, None),                    // a client sitting at this machine
            (200, Some("100.101.102.103")), // and one SSH'd in from a phone
        ],
    };

    // The pid tmux passed, alive and remote: that is the room.
    assert_eq!(
        ayeaye::dictate::recording_peer(&machines, Some("200"), &[100]),
        Some("100.101.102.103".to_string())
    );
    // Alive and local: also the room, and *not* a reason to go looking for a
    // remote client. tmux told us which client pressed the key.
    assert_eq!(
        ayeaye::dictate::recording_peer(&machines, Some("100"), &[200]),
        None,
        "a live local client must not hand the microphone to somebody else"
    );

    // A pid that is not there any more falls through to the session's clients,
    // preferring the remote one whatever order tmux listed them in.
    for clients in [vec![100, 200], vec![200, 100]] {
        assert_eq!(
            ayeaye::dictate::recording_peer(&machines, Some("999"), &clients),
            Some("100.101.102.103".to_string()),
            "{clients:?}"
        );
    }
    // Nothing passed at all is the same walk.
    assert_eq!(
        ayeaye::dictate::recording_peer(&machines, None, &[100, 200]),
        Some("100.101.102.103".to_string())
    );
    // Every client on this machine: record here.
    assert_eq!(
        ayeaye::dictate::recording_peer(&machines, None, &[100]),
        None
    );
    assert_eq!(ayeaye::dictate::recording_peer(&machines, None, &[]), None);
    // Something that is not a pid is not a pid.
    assert_eq!(
        ayeaye::dictate::recording_peer(&machines, Some("not-a-pid"), &[200]),
        Some("100.101.102.103".to_string())
    );
}

// AYEAYE-58
//
// "I could not ask tmux" and "that pane is gone" are opposite answers, and this
// is the most expensive possible place to confuse them: `type_into` runs after
// the recording, the transcription and the cleanup model. A wedged tmux reported
// as a vanished pane sends somebody looking at their window layout for a problem
// that will have fixed itself by the time they get there.
#[tokio::test]
async fn a_tmux_that_would_not_answer_is_told_from_a_pane_that_has_gone() {
    if !common::have_tmux() {
        return;
    }
    let Some(server) = common::Private::named("toggle-wedged") else {
        return;
    };
    let agent = Agent::started(true, b"pretend this is ogg".to_vec()).await;
    let voice = Says::heard("run the tests");
    let state = state_path("wedged");

    // A tmux that answers, and a pane it does not have: that pane has gone.
    let live = server.layer();
    let toggle = ayeaye::dictate::Toggle {
        tmux: &live,
        here: ayeaye_core::peer::HostName::new("desktop").expect("a host name"),
        voice: &voice,
        state: &state,
        token: "right-token".to_string(),
        port: agent.port,
    };
    let said = toggle.press("desktop/%99", None).await;
    assert!(
        said.contains("is not a pane on this machine"),
        "a tmux that answered should say the pane has gone: {said:?}"
    );

    // A tmux nobody can ask: not the same sentence, and not a claim about the
    // pane at all.
    let unreachable = ayeaye::tmux::Tmux::spelled(
        &["ayeaye-58-no-such-tmux"],
        std::time::Duration::from_secs(5),
    );
    let toggle = ayeaye::dictate::Toggle {
        tmux: &unreachable,
        here: ayeaye_core::peer::HostName::new("desktop").expect("a host name"),
        voice: &voice,
        state: &state,
        token: "right-token".to_string(),
        port: agent.port,
    };
    let said = toggle.press("desktop/%0", None).await;
    assert!(
        said.contains("could not ask tmux"),
        "a tmux that could not be asked must not be reported as a missing pane: {said:?}"
    );
    assert!(
        !state.exists(),
        "nothing was recorded, so nothing is remembered"
    );
}

// AYEAYE-58
//
// The target is judged on the way *in*. Finding out at the second press costs
// the recording, the transcription and the cleanup model as well as the words,
// and the microphone was turned on for nothing.
#[tokio::test]
async fn a_target_this_machine_does_not_have_is_refused_before_the_microphone() {
    if !common::have_tmux() {
        return;
    }
    let Some(server) = common::Private::named("toggle-early") else {
        return;
    };
    let agent = Agent::started(true, b"never recorded".to_vec()).await;
    let voice = Says::heard("never reached");
    let state = state_path("early");
    let tmux = server.layer();
    let toggle = ayeaye::dictate::Toggle {
        tmux: &tmux,
        here: ayeaye_core::peer::HostName::new("desktop").expect("a host name"),
        voice: &voice,
        state: &state,
        token: "right-token".to_string(),
        port: agent.port,
    };

    let said = toggle.press("desktop/%99", None).await;

    assert!(said.contains("is not a pane on this machine"), "{said}");
    assert!(
        !state.exists(),
        "a recording that was refused must not be remembered"
    );
    assert_eq!(
        agent.stops.load(Ordering::Relaxed),
        0,
        "the microphone was turned on for a pane that does not exist"
    );
}
