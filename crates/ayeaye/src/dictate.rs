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
use ayeaye_core::dictation::{self, Capability, Outcome, State, is_hallucination};
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

// ------------------------------------------------------------ the tmux path
//
// Dictation as a tmux state, toggled by one key:
//
//     M-v  ->  starts recording, the status bar shows ● REC
//     M-v  ->  stops, transcribes, cleans up, types it into the pane
//
// Recording happens on whichever device the tmux client is connected from;
// transcription and cleanup happen here. Nothing is ever submitted: the text is
// typed without Enter so it is reviewed first.

use std::path::Path;

use ayeaye_core::prompt;

use crate::recorder::Recorder;
use crate::tmux::Tmux;

/// Where the recording state lives, under the state directory.
///
/// Deliberately **not** `bin/voice-dictate`'s `voice-dictate.json`, for the same
/// reason `crate::fit` keeps its own file: the two implementations run side by
/// side until the cutover, and one toggle reading the other's state would stop a
/// recording the other started and hand the audio to the wrong pipeline.
pub const STATE_FILE: &str = "ayeaye-dictate.json";

/// Something that can turn a clip into words. [`Voice`] is the real one.
///
/// A trait so the toggle can be driven end to end without a model: the property
/// worth proving at this level is what reaches somebody's terminal, and that is
/// the same property whether the words came from a model or from a test.
pub trait Dictates {
    /// One dictation.
    fn dictate(
        &self,
        clip: &[u8],
        ext: &str,
        names: &str,
    ) -> impl std::future::Future<Output = Outcome> + Send;
}

impl Dictates for Voice {
    async fn dictate(&self, clip: &[u8], ext: &str, names: &str) -> Outcome {
        Voice::dictate(self, clip, ext, names).await
    }
}

/// One press of the dictation key.
pub struct Toggle<'a, D: Dictates> {
    /// The tmux holding the pane and the status bar.
    pub tmux: &'a Tmux,
    /// What turns a clip into words.
    pub voice: &'a D,
    /// Where the recording state is kept between the two presses.
    pub state: &'a Path,
    /// The secret the recording agent is presented with.
    pub token: String,
    /// The port the recording agent listens on.
    pub port: u16,
}

impl<D: Dictates> Toggle<'_, D> {
    /// Press the key: start a recording, or finish one.
    ///
    /// Returns the line to put on the status bar, which is also what a test
    /// reads: there is no window to report into, so every outcome is one
    /// sentence there.
    pub async fn press(&self, pane: &str, peer: Option<&str>) -> String {
        match read_state(self.state) {
            Some(state) => self.stop(state).await,
            None => self.start(pane, peer).await,
        }
    }

    /// Begin.
    async fn start(&self, pane: &str, peer: Option<&str>) -> String {
        let (host, label) = dictation::recording_device(peer);
        let recorder = Recorder::at(&host, self.port, &self.token);

        match recorder.health().await {
            Err(why) => return self.say(format!("voice: no recorder on {label} — {why}")),
            Ok(reply) if reply.status == 401 => {
                return self.say(format!("voice: {label} rejected the token"));
            }
            // A 200 that does not say `ok` is an agent running on a machine with
            // no microphone it knows how to use, which is a different thing to
            // fix and has to say so.
            Ok(reply) if !reply.healthy() => {
                return self.say(format!("voice: {label} has no recording backend"));
            }
            Ok(_) => {}
        }
        match recorder.start().await {
            Ok(reply) if reply.status == 200 => {}
            Ok(reply) => {
                return self.say(format!(
                    "voice: {label} would not start recording ({})",
                    reply.status
                ));
            }
            Err(why) => return self.say(format!("voice: lost contact with {label} — {why}")),
        }

        // Written *after* the recorder is running. The other order leaves a
        // machine that believes it is recording when nothing is, and the next
        // press stops a recording that never started.
        if let Err(why) = write_state(
            self.state,
            &State {
                host,
                label: label.clone(),
                pane: pane.to_string(),
            },
        ) {
            return self.say(format!("voice: could not remember the recording: {why}"));
        }
        self.indicate(dictation::RECORDING).await;
        String::new()
    }

    /// Finish.
    async fn stop(&self, state: State) -> String {
        // Cleared first, and the indicator moved on, so that a failure below
        // leaves a machine that can start another recording rather than one
        // stuck believing it is still taping.
        clear_state(self.state);
        self.indicate(dictation::WORKING).await;

        let recorder = Recorder::at(&state.host, self.port, &self.token);
        let reply = match recorder.stop().await {
            Ok(reply) if reply.status == 200 => reply,
            Ok(reply) => {
                return self
                    .finish(format!(
                        "voice: recording failed on {} ({})",
                        state.label, reply.status
                    ))
                    .await;
            }
            Err(why) => {
                return self
                    .finish(format!("voice: lost contact with {} — {why}", state.label))
                    .await;
            }
        };

        let ext = reply.extension.as_deref().unwrap_or("wav");
        let names = self.vocabulary(&state.pane).await;
        let outcome = self.voice.dictate(&reply.body, ext, &names).await;
        eprintln!("ayeaye: dictation: {}", outcome.why());

        match &outcome {
            Outcome::Heard { cleaned, .. } => {
                let said = self.type_into(&state.pane, cleaned.text()).await;
                self.finish(said).await
            }
            // Everything else is a sentence and nothing typed. A pane that got
            // "that sounded like silence" typed into it would be worse than one
            // that got nothing.
            other => self.finish(format!("voice: {}", other.why())).await,
        }
    }

    /// The identifiers on the pane being dictated into.
    async fn vocabulary(&self, pane: &str) -> String {
        let Ok(id) = ayeaye_core::peer::PaneId::parse(pane) else {
            return String::new();
        };
        let captured = self
            .tmux
            .ask(&[
                "capture-pane",
                "-p",
                "-t",
                id.pane(),
                "-S",
                &format!("-{}", ayeaye_core::vocab::LINES),
            ])
            .await
            .unwrap_or_default();
        ayeaye_core::vocab::on_screen(&captured)
    }

    /// Type the words, and **never submit them**.
    ///
    /// `-l` is literal, so nothing in the text is read as the name of a key, and
    /// `--` ends the options so text beginning with a dash is text. No Enter is
    /// sent, ever: the point of the whole feature is that a person reads what a
    /// model wrote before an agent acts on it.
    async fn type_into(&self, pane: &str, text: &str) -> String {
        let Ok(id) = ayeaye_core::peer::PaneId::parse(pane) else {
            return format!("voice: {pane} is not a pane any more");
        };
        let Some(typed) = prompt::typed(text) else {
            return "voice: the rewrite carried something that cannot be typed".to_string();
        };
        match self
            .tmux
            .ask(&["send-keys", "-t", id.pane(), "-l", "--", typed.text()])
            .await
        {
            Ok(_) => String::new(),
            Err(why) => format!("voice: could not type it into {pane}: {why}"),
        }
    }

    /// Clear the indicator, then say whatever there is to say.
    async fn finish(&self, said: String) -> String {
        self.indicate("").await;
        if said.is_empty() {
            return said;
        }
        self.say(said)
    }

    /// Put one line on the status line.
    ///
    /// The message is returned as well as displayed, because a caller — and a
    /// test — has no other way to know what happened: `run-shell -b` discards
    /// everything this process writes.
    fn say(&self, said: String) -> String {
        said
    }

    /// Drive the status bar. Empty text clears it.
    async fn indicate(&self, text: &str) {
        let _ = self
            .tmux
            .ask(&["set-option", "-g", "@voice_rec", text])
            .await;
        let _ = self.tmux.ask(&["refresh-client", "-S"]).await;
    }
}

/// The recording in progress, or `None`.
pub fn read_state(path: &Path) -> Option<State> {
    State::decode(&std::fs::read_to_string(path).ok()?)
}

/// Record that a recording is in progress, **atomically**.
///
/// Written to a temporary name and renamed over the real one, because a crash
/// between the two presses is not hypothetical — the whole feature is a person
/// holding a key down on a laptop that sleeps. A partial write under the real
/// name would be a state nothing can read, and while `State::decode` treats that
/// as "not recording" rather than wedging the toggle, the recording it named
/// would be lost along with which pane it was for.
///
/// The temporary name carries this process's id, so two daemons writing at once
/// do not rename each other's half-written file into place.
pub fn write_state(path: &Path, state: &State) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|why| format!("{}: {why}", parent.display()))?;
    }
    let temporary = path.with_extension(format!("tmp-{}", std::process::id()));
    std::fs::write(&temporary, state.encode())
        .map_err(|why| format!("{}: {why}", temporary.display()))?;
    std::fs::rename(&temporary, path).map_err(|why| {
        // Not left behind for somebody to find later and wonder about.
        let _ = std::fs::remove_file(&temporary);
        format!("{}: {why}", path.display())
    })
}

/// Forget the recording. A state that is not there is already forgotten.
pub fn clear_state(path: &Path) {
    let _ = std::fs::remove_file(path);
}
