//! What a dictation came to, and what this machine can do about one.
//!
//! Every refusal in here was a real failure of the path this replaces, and every
//! one of them is a sentence somebody reads on a phone while wondering why the
//! microphone did nothing. So they are values with bodies rather than early
//! returns with string literals scattered across a handler.

use crate::cleanup::Cleaned;
use crate::json;
use crate::model::ModelId;

/// What a speech model writes when it is handed near-silence.
///
/// `bin/voice-dictate`'s list, which was collected from real clips rather than
/// imagined: a Whisper handed room tone reaches for the most common thing in its
/// training data, and this is what that turns out to be.
///
/// Matched only against a whole transcription. "Thank you" is a legitimate
/// dictation, and a substring match would throw away every sentence containing
/// it.
pub const HALLUCINATIONS: &[&str] = &[
    "thank you",
    "thanks for watching",
    "thank you very much",
    "you",
    "bye",
    "[blank_audio]",
    "(silence)",
    "(buzzing)",
    "",
    "so",
    "okay",
    "peace",
];

/// The punctuation that is packaging rather than words.
const TRAILING: &[char] = &['.', '"', '!', ','];

/// Whether a transcription is one of a speech model's stock answers to silence.
///
/// Case and trailing punctuation are ignored on both sides, because the model
/// writes "Thank you." and "thank you" for the same non-event.
pub fn is_hallucination(text: &str) -> bool {
    let said = text.trim().to_lowercase();
    let said = said.trim_matches(|c| TRAILING.contains(&c));
    HALLUCINATIONS
        .iter()
        .any(|stock| stock.trim_matches(|c| TRAILING.contains(&c)) == said)
}

/// What this machine can and cannot do about a dictation.
///
/// Read rather than assumed. `bin/ayeaye` probes for the same reason: voice is a
/// progressive enhancement, the floor is a text-only panel that works with
/// nothing installed, and the page greys out the talk button on what this says.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Capability {
    /// Whether the audio converter is on this machine.
    pub converter: bool,
    /// The speech model somebody has chosen, if they have.
    pub speech: Option<ModelId>,
    /// Whether that model's files are really in the store.
    pub speech_ready: bool,
    /// The cleanup model somebody has chosen, if they have.
    pub cleanup: Option<ModelId>,
    /// Whether that model's files are really in the store.
    pub cleanup_ready: bool,
}

impl Capability {
    /// Whether a dictation could plausibly succeed right now.
    ///
    /// **Cleanup is deliberately not part of this.** Without it a dictation is
    /// the raw transcription, which is what the daemon degrades to when the
    /// model server it used is unreachable — a worse dictation, not no
    /// dictation. Refusing voice for a missing cleanup model would turn a
    /// working feature off.
    pub fn ok(&self) -> bool {
        self.converter && self.speech.is_some() && self.speech_ready
    }

    /// The first thing standing in the way, in words somebody can act on.
    ///
    /// One thing rather than all of them: it is a sentence on a phone, and the
    /// next thing to do is the first thing missing.
    pub fn why(&self) -> Option<String> {
        if !self.converter {
            return Some("there is no audio converter on this machine".to_string());
        }
        match &self.speech {
            None => Some("no speech model is configured — `ayeaye model use <id>`".to_string()),
            Some(id) if !self.speech_ready => Some(format!(
                "{id} is not on this machine yet — `ayeaye model pull {id}`"
            )),
            Some(_) => None,
        }
    }

    /// The body the capability probe answers with.
    ///
    /// `voice` is the field `share/app.html` reads to decide whether the talk
    /// button is live; the rest is what somebody needs in order to fix it.
    pub fn body(&self) -> String {
        let model = |chosen: &Option<ModelId>| match chosen {
            Some(id) => json::string(&id.to_string()),
            None => "null".to_string(),
        };
        format!(
            concat!(
                r#"{{"voice":{},"converter":{},"speech":{},"speech_ready":{},"#,
                r#""cleanup":{},"cleanup_ready":{},"why":{}}}"#
            ),
            self.ok(),
            self.converter,
            model(&self.speech),
            self.speech_ready,
            model(&self.cleanup),
            self.cleanup_ready,
            match self.why() {
                Some(why) => json::string(&why),
                None => "null".to_string(),
            }
        )
    }
}

/// What one dictation came to.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// Voice is not configured on this machine.
    Unavailable(String),
    /// The clip could not be turned into audio.
    Undecodable(String),
    /// Nobody spoke.
    Silence {
        /// How loud it was, as floats in [-1, 1].
        rms: f32,
        /// How long the clip ran.
        seconds: f32,
    },
    /// Somebody spoke and nothing was recognised.
    Empty {
        /// How loud it was.
        rms: f32,
        /// How long the clip ran.
        seconds: f32,
    },
    /// Words.
    Heard {
        /// What the speech model heard.
        raw: String,
        /// What is to be typed, cleaned up or deliberately not.
        cleaned: Cleaned,
    },
}

impl Outcome {
    /// The body this answers with.
    ///
    /// `raw` and `final` are the two fields `share/app.html` reads: it takes
    /// `final` for the draft and compares it against `raw` to decide whether to
    /// say "cleaned up". Everything else is an `error`, which is the one other
    /// field that page looks for — including the two that are answered at 200,
    /// because "you did not say anything" is an answer rather than a fault.
    pub fn body(&self) -> String {
        let measured = |error: &str, rms: f32, seconds: f32| {
            format!(
                r#"{{"error":{},"rms":{:.1},"seconds":{:.1}}}"#,
                json::string(error),
                on_sixteen_bit_scale(rms),
                seconds
            )
        };
        match self {
            Outcome::Heard { raw, cleaned } => format!(
                r#"{{"raw":{},"final":{}}}"#,
                json::string(raw),
                json::string(cleaned.text())
            ),
            Outcome::Silence { rms, seconds } => {
                measured("that sounded like silence", *rms, *seconds)
            }
            Outcome::Empty { rms, seconds } => measured("nothing recognised", *rms, *seconds),
            Outcome::Undecodable(why) => format!(
                r#"{{"error":{}}}"#,
                json::string(&format!("could not decode the audio: {why}"))
            ),
            Outcome::Unavailable(why) => format!(
                r#"{{"error":{}}}"#,
                json::string(&format!("voice is not configured on this server: {why}"))
            ),
        }
    }

    /// One line for a log a person will read at two in the morning.
    ///
    /// The point is to tell a bad microphone from a bad transcription from a bad
    /// rewrite, which is the same reason `bin/voice-dictate` writes one line per
    /// dictation carrying the loudness, the duration, and both texts.
    pub fn why(&self) -> String {
        match self {
            Outcome::Unavailable(why) => format!("voice unavailable: {why}"),
            Outcome::Undecodable(why) => format!("undecodable: {why}"),
            Outcome::Silence { rms, seconds } => format!(
                "silence: {:.1} over {seconds:.1}s",
                on_sixteen_bit_scale(*rms)
            ),
            Outcome::Empty { rms, seconds } => format!(
                "nothing recognised: {:.1} over {seconds:.1}s",
                on_sixteen_bit_scale(*rms)
            ),
            Outcome::Heard { raw, cleaned } => match cleaned.kept() {
                None => format!("heard {raw:?}, cleaned up to {:?}", cleaned.text()),
                Some(kept) => format!("heard {raw:?}, kept as spoken: {}", kept.why()),
            },
        }
    }
}

/// The loudness scale the daemon's thresholds are quoted on.
///
/// `bin/voice-dictate` measures 16-bit samples and compares against 1000, and
/// those numbers are in its log lines and in this project's notes — a quiet room
/// is about 490 and speech about 3 400. The audio here is floats, so a loudness
/// that reaches a person is put back on the scale the numbers were written on,
/// rather than quietly meaning something different in one of the two
/// implementations of one feature.
pub fn on_sixteen_bit_scale(rms: f32) -> f32 {
    rms * 32_768.0
}

/// What tmux is holding while a recording is in progress.
///
/// The state lives in a file rather than in a process, and that is the whole
/// design of the tmux path: there is deliberately no popup. A popup has to own
/// the terminal to read a keystroke, and that fought everything — mouse
/// reporting delivered escape sequences that read as "cancel", and toggling
/// cbreak mode per keypress lost input entirely. Holding the state in a file and
/// the indicator in a tmux option means there is nothing to close and no
/// keyboard to capture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct State {
    /// The address the recorder is answering on.
    pub host: String,
    /// What to call that machine in a status-line message.
    pub label: String,
    /// The qualified id of the pane the words are going into.
    pub pane: String,
}

impl State {
    /// The file's contents.
    pub fn encode(&self) -> String {
        format!(
            r#"{{"host":{},"label":{},"pane":{}}}"#,
            json::string(&self.host),
            json::string(&self.label),
            json::string(&self.pane)
        )
    }

    /// Read it back, or `None`.
    ///
    /// **`None` for anything that is not a whole state**, including a file torn
    /// off half-written by a crash. That is a decision rather than leniency: the
    /// alternative is a toggle that refuses forever, because the state that says
    /// "you are recording" is also the state that has to be readable in order to
    /// stop. Unreadable is treated as not recording, so the next keypress starts
    /// one.
    pub fn decode(text: &str) -> Option<State> {
        let value = json::parse(text).ok()?;
        let field = |name: &str| value.get(name).and_then(json::Value::text);
        Some(State {
            host: field("host")?.to_string(),
            label: field("label")?.to_string(),
            pane: field("pane")?.to_string(),
        })
    }
}

/// Which machine records, and what to call it.
///
/// **Where the person is, not where tmux is.** Recording happens on the device
/// the tmux client is connected *from*, so an address means somebody is SSH'd in
/// and the microphone is at the other end of it; no address means the client is
/// on this machine and so is the microphone.
pub fn recording_device(peer: Option<&str>) -> (String, String) {
    match peer {
        Some(address) => (address.to_string(), address.to_string()),
        None => ("127.0.0.1".to_string(), "local".to_string()),
    }
}

/// The status bar while a recording is in progress.
pub const RECORDING: &str = "#[fg=#e46876,bold] ● REC #[default]";

/// The status bar while the words are being worked out.
pub const WORKING: &str = "#[fg=#98bb6c,bold] … #[default]";

#[cfg(test)]
mod tests {
    use super::{
        Capability, Outcome, State, is_hallucination, on_sixteen_bit_scale, recording_device,
    };
    use crate::cleanup::{Kept, Policy, settle};
    use crate::model::ModelId;

    fn id(spelled: &str) -> ModelId {
        ModelId::parse(spelled).expect("a well-formed id")
    }

    fn ready() -> Capability {
        Capability {
            converter: true,
            speech: Some(id("openai/whisper-small.en")),
            speech_ready: true,
            cleanup: Some(id("Qwen/Qwen2.5-7B-Instruct-GGUF")),
            cleanup_ready: true,
        }
    }

    // AYEAYE-58
    //
    // The probe answers what is really here, not what has been configured. A
    // model somebody chose and never pulled is the state a machine is in
    // between two commands, and reporting it as ready lights up a talk button
    // that cannot work.
    #[test]
    fn the_probe_is_honest_about_what_is_missing_and_names_it() {
        assert!(ready().ok());
        assert_eq!(ready().why(), None);

        for (broken, expected) in [
            (
                Capability {
                    converter: false,
                    ..ready()
                },
                "converter",
            ),
            (
                Capability {
                    speech: None,
                    ..ready()
                },
                "no speech model",
            ),
            (
                Capability {
                    speech_ready: false,
                    ..ready()
                },
                "whisper-small.en",
            ),
        ] {
            assert!(!broken.ok(), "{broken:?} should not be usable");
            let why = broken.why().expect("something is in the way");
            assert!(
                why.contains(expected),
                "{why:?} should mention {expected:?}"
            );
        }
    }

    // AYEAYE-58
    //
    // Cleanup absent is a degradation, not an outage: without it a dictation is
    // the raw transcription, which is exactly what the daemon does when the
    // model server it used is unreachable. Refusing voice outright for it would
    // turn a working feature off.
    #[test]
    fn a_machine_with_no_cleanup_model_can_still_dictate() {
        let no_cleanup = Capability {
            cleanup: None,
            cleanup_ready: false,
            ..ready()
        };

        assert!(no_cleanup.ok());
        assert_eq!(no_cleanup.why(), None);
        // And the probe still says so, because the page has to be able to tell
        // "raw transcription" from "cleaned up".
        assert!(
            no_cleanup.body().contains(r#""cleanup":null"#),
            "{}",
            no_cleanup.body()
        );
        assert!(
            no_cleanup.body().contains(r#""voice":true"#),
            "{}",
            no_cleanup.body()
        );
    }

    // AYEAYE-58
    //
    // The shape `share/app.html` reads, which cannot be changed: it takes
    // `d.final` for the draft and compares it against `d.raw` to decide whether
    // to say "cleaned up".
    #[test]
    fn what_was_heard_answers_with_the_two_fields_the_page_reads() {
        let heard = Outcome::Heard {
            raw: "um so run the tests".to_string(),
            cleaned: settle(
                &Policy::default(),
                "um so run the tests",
                Some("Run the tests."),
            ),
        };

        assert_eq!(
            heard.body(),
            r#"{"raw":"um so run the tests","final":"Run the tests."}"#
        );
    }

    // AYEAYE-58
    //
    // Every refusal is an `error` field, because that is the one field the page
    // looks for. The loudness rides along with silence: it is the number that
    // tells somebody their microphone is muted rather than their voice quiet.
    #[test]
    fn every_refusal_is_a_sentence_the_page_can_draw() {
        let silence = Outcome::Silence {
            rms: 0.006,
            seconds: 1.5,
        };
        assert_eq!(
            silence.body(),
            r#"{"error":"that sounded like silence","rms":196.6,"seconds":1.5}"#
        );

        assert_eq!(
            Outcome::Empty {
                rms: 0.1,
                seconds: 2.0
            }
            .body(),
            r#"{"error":"nothing recognised","rms":3276.8,"seconds":2.0}"#
        );
        // A quote in a reason must not end the string that carries it.
        assert_eq!(
            Outcome::Undecodable("ffmpeg said \"no\"".to_string()).body(),
            r#"{"error":"could not decode the audio: ffmpeg said \"no\""}"#
        );
        assert_eq!(
            Outcome::Unavailable("no speech model is configured".to_string()).body(),
            r#"{"error":"voice is not configured on this server: no speech model is configured"}"#
        );
    }

    // AYEAYE-58
    //
    // The loudness numbers in this project's notes and in the daemon's log are
    // on the 16-bit scale — a quiet room is about 490 — so a number that reaches
    // a person is put back on it rather than quietly meaning something else in
    // one of the two implementations.
    #[test]
    fn a_loudness_is_reported_on_the_scale_the_thresholds_are_written_on() {
        assert_eq!(on_sixteen_bit_scale(0.5), 16_384.0);
        assert_eq!(on_sixteen_bit_scale(0.0), 0.0);
    }

    // AYEAYE-58
    //
    // A clip can clear the energy gate — a door, a cough — and still transcribe
    // to one of a speech model's stock answers to silence. Matched only when it
    // is the whole transcription, so a real sentence containing "thank you"
    // survives.
    #[test]
    fn a_stock_answer_to_silence_is_not_something_somebody_said() {
        for said in [
            "thank you",
            "Thank you.",
            "  THANKS FOR WATCHING! ",
            "[BLANK_AUDIO]",
        ] {
            assert!(
                is_hallucination(said),
                "{said:?} is what silence sounds like"
            );
        }
        for said in [
            "thank you for the review",
            "run the tests",
            "bye bye birdie",
        ] {
            assert!(!is_hallucination(said), "{said:?} is a sentence");
        }
    }

    // AYEAYE-58
    #[test]
    fn a_dictation_that_was_not_rewritten_says_so_in_one_line() {
        let kept = Outcome::Heard {
            raw: "run the tests".to_string(),
            cleaned: settle(&Policy::default(), "run the tests", None),
        };
        assert!(
            kept.why().contains(Kept::Unavailable.why()),
            "{}",
            kept.why()
        );
    }

    // AYEAYE-58
    //
    // The recording state survives being written and read back, awkward labels
    // included: a machine name is whatever tailscale or a person called it, and
    // a quote in one must not end the string that carries it.
    #[test]
    fn the_recording_state_round_trips_through_the_file_it_lives_in() {
        let state = State {
            host: "100.101.102.103".to_string(),
            label: "someone\'s \"phone\"".to_string(),
            pane: "desktop/%3".to_string(),
        };

        assert_eq!(State::decode(&state.encode()), Some(state));
    }

    // AYEAYE-58
    //
    // A file torn off half-written by a crash is *not recording*, and that is
    // the load-bearing half. The state that says "you are recording" is also the
    // state that has to be readable in order to stop, so treating an unreadable
    // one as recording would be a toggle that refuses forever — one keypress
    // away from a person who can never dictate again.
    #[test]
    fn a_state_nothing_could_read_is_not_a_recording_in_progress() {
        for torn in [
            "",
            "{",
            r#"{"host":"1.2.3.4","label":"phone""#,
            // Whole JSON, missing a field: half a state is not a state.
            r#"{"host":"1.2.3.4","label":"phone"}"#,
            r#"{"host":1,"label":"phone","pane":"desktop/%0"}"#,
        ] {
            assert_eq!(State::decode(torn), None, "{torn:?} is not a state");
        }
    }

    // AYEAYE-58
    //
    // Recording happens where the person is, not where tmux is. An address means
    // somebody is SSH\'d in and the microphone is at the other end of it.
    #[test]
    fn the_device_that_records_is_the_one_the_client_is_on() {
        assert_eq!(
            recording_device(Some("100.101.102.103")),
            ("100.101.102.103".to_string(), "100.101.102.103".to_string())
        );
        assert_eq!(
            recording_device(None),
            ("127.0.0.1".to_string(), "local".to_string())
        );
    }
}
