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
use crate::models::{self, Residents, Slot};

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
///
/// **A dictation does hold both models at once**, and that is a real change from
/// `Residents`' own promise, which is about one holder never holding two of *its*
/// models. It is the shape the pipeline needs — a transcription is rewritten
/// while the speech model is still resident — and the cost is stated here rather
/// than discovered: a machine sized for one of these two is a machine that
/// should configure the other away, and `AYEAYE_CLEANUP_MODEL` unset is exactly
/// that, giving the raw transcription and no second model.
///
/// Generic over the two slots, with the real ones as defaults so nothing above
/// this file mentions the parameters. That is the same boundary `models::Slot`
/// already is, and it is what lets the suite watch a cleanup model *fail to
/// load* and a sweep *let go* — two branches that would otherwise need a
/// gigabyte of weights to reach.
pub struct Voice<S: Slot = SpeechSlot, L: Slot = LanguageSlot> {
    store: PathBuf,
    settings: ModelSettings,
    policy: Policy,
    converter: String,
    resident: tokio::sync::Mutex<Resident<S, L>>,
    probed: Mutex<Option<(Instant, Capability)>>,
}

impl<S: Slot, L: Slot> std::fmt::Debug for Voice<S, L> {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        out.debug_struct("Voice")
            .field("store", &self.store)
            .field("speech", &self.settings.speech)
            .field("cleanup", &self.settings.cleanup)
            .finish()
    }
}

/// What is loaded, and when it was last wanted.
struct Resident<S: Slot, L: Slot> {
    speech: Residents<S>,
    language: Residents<L>,
    /// When a model was last asked for anything. `None` until one is.
    used: Option<Instant>,
}

impl Voice {
    /// A voice on this machine.
    ///
    /// `policy` arrives resolved rather than being assembled here, for the same
    /// reason the token and cliban do on [`crate::config::Settings`]: it is read
    /// from the environment and a file, neither of which this should touch. It
    /// also matters more than it looks — a policy built by hand out of
    /// `ModelSettings::cleanup_prompt` pairs somebody's own prompt with the
    /// default prompt's echo phrases, which `Policy::resolve` exists to prevent,
    /// and leaves `CLEANUP_TEMPLATE` and `CLEANUP_MAX_TOKENS` reading nothing at
    /// all.
    pub fn new(
        store: PathBuf,
        settings: ModelSettings,
        policy: Policy,
        converter: String,
    ) -> Voice {
        Voice::with_slots(
            store,
            settings,
            policy,
            converter,
            SpeechSlot::empty(),
            LanguageSlot::empty(),
        )
    }
}

impl<S, L> Voice<S, L>
where
    S: Slot + Speech,
    L: Slot + Cleanup,
    // A slot that cannot say why it would not load is a slot whose failure
    // reaches somebody as nothing at all, and "the model did not load" is the
    // error that costs an evening.
    S::Error: std::fmt::Display,
    L::Error: std::fmt::Display,
{
    /// A voice holding somewhere other than the real slots.
    pub fn with_slots(
        store: PathBuf,
        settings: ModelSettings,
        policy: Policy,
        converter: String,
        speech: S,
        language: L,
    ) -> Voice<S, L> {
        let residency = Residency {
            idle: settings.idle,
        };
        let resident = Resident {
            speech: Residents::new(speech, store.clone(), residency.clone()),
            language: Residents::new(language, store.clone(), residency),
            used: None,
        };
        Voice {
            policy,
            store,
            settings,
            converter,
            resident: tokio::sync::Mutex::new(resident),
            probed: Mutex::new(None),
        }
    }

    /// What this machine can do about a dictation, right now.
    ///
    /// Cached whole for [`PROBE_TTL`], not just the converter answer: the panel
    /// polls this, and the store walk behind `speech_ready` is a directory
    /// listing per poll on top of the process. The answer is *allowed* to change,
    /// which is the point of a cache with a lifetime rather than a value resolved
    /// once — a model pulled or a converter installed while the daemon runs must
    /// light the talk button up without anybody restarting anything.
    ///
    /// A benign race between two callers costs one extra probe. The lock is never
    /// held across the `await`, because a probe that blocked every other request
    /// on one machine's slow filesystem would be worse than probing twice.
    pub async fn probe(&self) -> Capability {
        if let Some(fresh) = self.probed_recently() {
            return fresh;
        }
        let installed = models::installed(&self.store);
        let ready = |chosen: &Option<ayeaye_core::model::ModelId>| {
            chosen.as_ref().is_some_and(|id| installed.contains(id))
        };
        let capability = Capability {
            converter: self.have_converter().await,
            speech_ready: ready(&self.settings.speech),
            cleanup_ready: ready(&self.settings.cleanup),
            speech: self.settings.speech.clone(),
            cleanup: self.settings.cleanup.clone(),
        };
        *self.probed.lock().unwrap_or_else(|held| held.into_inner()) =
            Some((Instant::now(), capability.clone()));
        capability
    }

    /// The last answer, if it is still worth reusing.
    fn probed_recently(&self) -> Option<Capability> {
        let probed = self.probed.lock().unwrap_or_else(|held| held.into_inner());
        probed
            .as_ref()
            .filter(|(at, _)| at.elapsed() < PROBE_TTL)
            .map(|(_, capability)| capability.clone())
    }

    /// Whether the audio converter is on this machine.
    ///
    /// Through [`crate::command::run`], which has a deadline: this is asked from
    /// inside a request, and a converter that hangs on `-version` — a stale
    /// network mount, a wedged interpreter — would otherwise hold the request
    /// forever and, worse, hold it while a lock was in hand.
    async fn have_converter(&self) -> bool {
        crate::command::run(&[self.converter.as_str(), "-version"], PROBE_LIMIT)
            .await
            .is_ok()
    }

    /// One dictation.
    pub async fn dictate(&self, clip: &[u8], ext: &str, names: &str) -> Outcome {
        // Before the clip is decoded, because decoding is a process and a
        // machine with no speech model has nothing that could transcribe the
        // result of it.
        if self.settings.speech.is_none() {
            return Outcome::Unavailable(
                "no speech model is configured — `ayeaye model use <id>`".to_string(),
            );
        }
        let audio = match audio::decode_with(&self.converter, clip, ext).await {
            Ok(audio) => audio,
            Err(why) => return Outcome::Undecodable(why.to_string()),
        };
        self.hear_decoded(&audio, names).await
    }

    /// The same, given audio that has already been decoded.
    ///
    /// Split from [`Voice::dictate`] at the one place the external converter
    /// stops being involved, so the suite can drive residency and the pipeline
    /// without a program on the machine — and so AYEAYE-68, which removes the
    /// converter entirely by having both recorders send raw PCM, has the seam it
    /// needs already cut.
    pub async fn hear_decoded(&self, audio: &Pcm16kMono, names: &str) -> Outcome {
        let Some(speech) = self.settings.speech.clone() else {
            return Outcome::Unavailable(
                "no speech model is configured — `ayeaye model use <id>`".to_string(),
            );
        };

        let mut resident = self.resident.lock().await;
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
            used,
            ..
        } = &mut *resident;
        let policy = &self.policy;
        // Seconds of arithmetic, on a thread that is otherwise answering
        // requests. `block_in_place` tells the runtime to move the rest of this
        // worker's work elsewhere first; without it one dictation stalls every
        // poll, every pane capture and every prompt on this machine for as long
        // as the model takes.
        let outcome = in_place(|| {
            if loaded {
                hear(speech_models.slot_mut(), language.slot_mut(), audio, names, policy)
            } else {
                hear(speech_models.slot_mut(), &mut AsSpoken, audio, names, policy)
            }
        });
        // Stamped *after* the work, not before. A model is idle from the moment
        // it stopped being used, and a dictation that took thirty seconds would
        // otherwise start its idle clock thirty seconds in the past.
        *used = Some(Instant::now());
        outcome
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

/// How long the converter gets to say it exists.
///
/// Short: this is a probe behind a poll, and a converter that cannot answer in a
/// couple of seconds is one the dictation below would not survive either.
pub const PROBE_LIMIT: Duration = Duration::from_secs(2);

/// Run something that will not yield, without stalling the runtime.
///
/// `block_in_place` only exists on the multi-threaded runtime and panics on the
/// current-thread one, which is what `#[tokio::test]` builds by default. So the
/// flavour is asked rather than assumed: a test that reaches inference must not
/// abort for want of a second thread, and the daemon — whose runtime is
/// multi-threaded — must not stall for want of the call.
fn in_place<T>(work: impl FnOnce() -> T) -> T {
    match tokio::runtime::Handle::try_current().map(|handle| handle.runtime_flavor()) {
        Ok(tokio::runtime::RuntimeFlavor::MultiThread) => tokio::task::block_in_place(work),
        _ => work(),
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
