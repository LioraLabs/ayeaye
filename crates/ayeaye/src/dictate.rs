//! One dictation, from a clip of audio to the words to type.
//!
//! This is the file that wires the milestone's three halves of voice together:
//! `crate::audio` decodes, `ayeaye_infer` transcribes and rewrites, and
//! `ayeaye_core::dictation` says what any of it came to. Nothing here decides
//! anything a test would have to start a model to observe — the two model steps
//! arrive as traits, for the same reason `models::Slot` is one.
//!
//! The order is the design. The energy gate comes before transcription because
//! transcription is seconds of a model's time and silence is knowable for free;
//! the stock-answer check comes after it because a cough clears the gate and
//! still transcribes to "thank you"; and cleanup comes last because it is the
//! one step that can make things worse, and the words the speaker said are the
//! floor under it.

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use ayeaye_core::Pcm16kMono;
use ayeaye_core::cleanup::{Cleaned, Policy, settle};
use ayeaye_core::dictation::{Capability, Outcome, is_hallucination};
use ayeaye_core::model::residency::Policy as Residency;
use ayeaye_core::model::settings::ModelSettings;
use ayeaye_infer::{LanguageSlot, SpeechSlot};

use crate::audio;
use crate::models::{self, Residents};

/// Something that can turn audio into words.
pub trait Speech {
    /// Transcribe, or say why not.
    fn transcribe(&mut self, audio: &Pcm16kMono) -> Result<String, String>;
}

/// Something that can tidy a transcription up.
pub trait Cleanup {
    /// Clean up, and **never fail**: the words the speaker said are the floor.
    fn clean(&mut self, raw: &str, names: &str, policy: &Policy) -> Cleaned;
}

impl Speech for SpeechSlot {
    fn transcribe(&mut self, audio: &Pcm16kMono) -> Result<String, String> {
        SpeechSlot::transcribe(self, audio)
            .map(|transcript| transcript.text())
            .map_err(|why| why.to_string())
    }
}

impl Cleanup for LanguageSlot {
    fn clean(&mut self, raw: &str, names: &str, policy: &Policy) -> Cleaned {
        LanguageSlot::clean_with(self, raw, names, policy)
    }
}

/// Cleanup on a machine that has no cleanup model.
///
/// A value rather than an `Option` at every call site: "there is no model" and
/// "the model refused" are the same outcome, which is the dictation as spoken,
/// and `settle` already says so in one place.
pub struct AsSpoken;

impl Cleanup for AsSpoken {
    fn clean(&mut self, raw: &str, _names: &str, policy: &Policy) -> Cleaned {
        settle(policy, raw, None)
    }
}

/// Turn one clip of audio into what should be typed.
pub fn hear(
    speech: &mut impl Speech,
    cleanup: &mut impl Cleanup,
    audio: &Pcm16kMono,
    names: &str,
    policy: &Policy,
) -> Outcome {
    let rms = audio.rms();
    let seconds = audio.duration_secs();

    if audio.is_silence() {
        return Outcome::Silence { rms, seconds };
    }

    let raw = match speech.transcribe(audio) {
        Ok(raw) => raw,
        // Not `Empty`. "I could not listen" and "you said nothing" are opposite
        // answers, and answering the first as the second sends somebody looking
        // at their microphone for a model that never loaded.
        Err(why) => return Outcome::Unavailable(why),
    };
    // The backstop: a door or a cough clears the energy gate and still
    // transcribes to one of a speech model's stock answers to silence, and
    // typing that into a terminal is worse than typing nothing.
    if raw.trim().is_empty() || is_hallucination(&raw) {
        return Outcome::Empty { rms, seconds };
    }

    let cleaned = cleanup.clean(&raw, names, policy);
    Outcome::Heard { raw, cleaned }
}

/// How long the probe's answer is worth reusing.
///
/// `bin/ayeaye`'s `VOICE_PROBE_TTL`. The panel polls, so a probe per request
/// would be a process started per request; and the answer is allowed to change,
/// which is the whole point of it being cached rather than resolved once — a
/// converter installed while the daemon runs must light the talk button up
/// without anybody restarting anything.
pub const PROBE_TTL: Duration = Duration::from_secs(30);

/// How often the sweeper asks whether a model is still worth its memory.
pub const SWEEP_EVERY: Duration = Duration::from_secs(30);

/// Everything a dictation needs that outlives one request.
///
/// The models are behind one lock rather than two, and that is deliberate:
/// inference here is one process's arithmetic, and two dictations running at
/// once would be two models' worth of device memory and half the speed each.
/// One at a time is also what the daemon this replaces got for free by talking
/// to a server that serialised them.
pub struct Voice {
    store: PathBuf,
    settings: ModelSettings,
    policy: Policy,
    converter: String,
    resident: tokio::sync::Mutex<Resident>,
    probed: Mutex<Option<(Instant, bool)>>,
}

impl std::fmt::Debug for Voice {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        out.debug_struct("Voice")
            .field("store", &self.store)
            .field("speech", &self.settings.speech)
            .field("cleanup", &self.settings.cleanup)
            .finish()
    }
}

/// What is loaded, and when it was last wanted.
struct Resident {
    speech: Residents<SpeechSlot>,
    language: Residents<LanguageSlot>,
    /// When a model was last asked for anything. `None` until one is.
    used: Option<Instant>,
}

impl Voice {
    /// A voice on this machine.
    pub fn new(store: PathBuf, settings: ModelSettings, converter: String) -> Voice {
        let residency = Residency {
            idle: settings.idle,
        };
        let resident = Resident {
            speech: Residents::new(SpeechSlot::empty(), store.clone(), residency.clone()),
            language: Residents::new(LanguageSlot::empty(), store.clone(), residency),
            used: None,
        };
        Voice {
            policy: Policy {
                system_prompt: settings.cleanup_prompt.clone(),
                ..Policy::default()
            },
            store,
            settings,
            converter,
            resident: tokio::sync::Mutex::new(resident),
            probed: Mutex::new(None),
        }
    }

    /// What this machine can do about a dictation, right now.
    ///
    /// The converter answer is cached for [`PROBE_TTL`], because the panel polls
    /// this and starting a process per poll is what the cache in `bin/ayeaye`
    /// exists to avoid. A benign race between two threads costs one extra probe.
    pub fn probe(&self) -> Capability {
        let installed = models::installed(&self.store);
        let ready = |chosen: &Option<ayeaye_core::model::ModelId>| {
            chosen.as_ref().is_some_and(|id| installed.contains(id))
        };
        Capability {
            converter: self.have_converter(),
            speech_ready: ready(&self.settings.speech),
            cleanup_ready: ready(&self.settings.cleanup),
            speech: self.settings.speech.clone(),
            cleanup: self.settings.cleanup.clone(),
        }
    }

    /// Whether the audio converter is on this machine, asked at most every
    /// [`PROBE_TTL`].
    fn have_converter(&self) -> bool {
        let mut probed = self.probed.lock().unwrap_or_else(|held| held.into_inner());
        if let Some((at, answer)) = *probed
            && at.elapsed() < PROBE_TTL
        {
            return answer;
        }
        let answer = std::process::Command::new(&self.converter)
            .arg("-version")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok();
        *probed = Some((Instant::now(), answer));
        answer
    }

    /// One dictation.
    pub async fn dictate(&self, clip: &[u8], ext: &str, names: &str) -> Outcome {
        // Before the clip is decoded, because decoding is a process and a
        // machine with no speech model has nothing that could transcribe the
        // result of it.
        let Some(speech) = self.settings.speech.clone() else {
            return Outcome::Unavailable(
                "no speech model is configured — `ayeaye model use <id>`".to_string(),
            );
        };
        let audio = match audio::decode_with(&self.converter, clip, ext).await {
            Ok(audio) => audio,
            Err(why) => return Outcome::Undecodable(why.to_string()),
        };

        let mut resident = self.resident.lock().await;
        resident.used = Some(Instant::now());
        if let Err(why) = resident.speech.ensure(Some(&speech)) {
            return Outcome::Unavailable(format!("{speech} would not load: {why}"));
        }
        // Loaded on the same demand, and released the same way. A cleanup model
        // nobody configured is not an error: it is a machine that dictates the
        // words the speaker said.
        let cleanup = self.settings.cleanup.clone();
        let loaded = match resident.language.ensure(cleanup.as_ref()) {
            Ok(()) => cleanup.is_some(),
            Err(why) => {
                // Said out loud and stepped over. Losing the rewrite is a worse
                // dictation; losing the dictation is no dictation.
                eprintln!("ayeaye: the cleanup model would not load: {why}");
                false
            }
        };

        let Resident {
            speech: speech_models,
            language,
            ..
        } = &mut *resident;
        if loaded {
            hear(
                speech_models.slot_mut(),
                language.slot_mut(),
                &audio,
                names,
                &self.policy,
            )
        } else {
            hear(
                speech_models.slot_mut(),
                &mut AsSpoken,
                &audio,
                names,
                &self.policy,
            )
        }
    }

    /// Let go of a model nobody has used for a while, saying whether it did.
    ///
    /// `now` is an argument rather than a clock read inside, so the caller owns
    /// the one reading of the time — which is what `Residents::sweep` already
    /// asks for and what keeps the policy testable.
    pub async fn sweep(&self, now: Instant) -> bool {
        let mut resident = self.resident.lock().await;
        let Some(used) = resident.used else {
            return false;
        };
        let idle = now.saturating_duration_since(used);
        // Both, and the `|` rather than `||`: a sweep that stopped at the first
        // model to let go would leave the other resident for another cycle.
        let speech = resident.speech.sweep(idle);
        let language = resident.language.sweep(idle);
        speech | language
    }
}

/// Ask every [`SWEEP_EVERY`] whether a resident model is still worth its memory.
///
/// Started with the server rather than left to a request, and that is the whole
/// point: a model is released because nobody is dictating, so a policy that only
/// ran *during* a dictation would never fire. The residency decision itself is
/// `ayeaye_core::model::residency`, which AYEAYE-56 wrote and this is the first
/// thing to call.
pub fn sweeper(settings: std::sync::Arc<crate::config::Settings>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(SWEEP_EVERY).await;
            if settings.voice.sweep(Instant::now()).await {
                eprintln!("ayeaye: let go of an idle model");
            }
        }
    })
}
