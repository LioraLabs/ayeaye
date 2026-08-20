//! One dictation, from a clip of audio to the words to type.
//!
//! This is the file that wires the milestone's three halves of voice together:
//! `crate::audio` decodes, `crate::swap` transcribes and rewrites, and
//! `ayeaye_core::dictation` says what any of it came to. Nothing here decides
//! anything a test would have to start a model to observe — the two model steps
//! arrive as traits, for the same reason they did when the models were in this
//! process.
//!
//! The order is the design. The energy gate comes before transcription because
//! transcription is a round trip and a model's time and silence is knowable for
//! free; the stock-answer check comes after it because a cough clears the gate
//! and still transcribes to "thank you"; and cleanup comes last because it is
//! the one step that can make things worse, and the words the speaker said are
//! the floor under it.
//!
//! **No residency here any more.** Loading, unloading and letting go of an idle
//! model is what `llama-swap` is for, and it does it better than a sweeper in
//! this process could: it can see every model on the machine, including the ones
//! ayeaye is not asking for.

use std::future::Future;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use ayeaye_core::Pcm16kMono;
use ayeaye_core::cleanup::{Cleaned, Policy, settle, worth_cleaning};
use ayeaye_core::dictation::{self, Capability, Outcome, State, is_hallucination};
use ayeaye_core::model::settings::ModelSettings;

use crate::audio;
use crate::swap::Swap;

/// Something that can turn audio into words.
///
/// `impl Future + Send` rather than `async fn`, because these are awaited inside
/// axum handlers and a bare `async fn` in a trait promises nothing about the
/// future being sendable — which shows up as an unspellable error at the
/// handler, a long way from here.
pub trait Speech {
    /// Transcribe with the named model, or say why not.
    fn transcribe(
        &self,
        model: &str,
        audio: &Pcm16kMono,
    ) -> impl Future<Output = Result<String, String>> + Send;
}

/// Something that can tidy a transcription up.
pub trait Cleanup {
    /// Clean up, and **never fail**: the words the speaker said are the floor.
    fn clean(
        &self,
        model: &str,
        raw: &str,
        names: &str,
        policy: &Policy,
    ) -> impl Future<Output = Cleaned> + Send;
}

/// Something that can say which models are there to be asked for.
pub trait Lists {
    /// Every model the backend will serve, or why it could not be asked.
    fn available(&self) -> impl Future<Output = Result<Vec<String>, String>> + Send;
}

impl Speech for Swap {
    async fn transcribe(&self, model: &str, audio: &Pcm16kMono) -> Result<String, String> {
        Swap::transcribe(self, model, audio)
            .await
            .map_err(|why| why.to_string())
    }
}

impl Cleanup for Swap {
    /// **This cannot fail.** An unreachable proxy, a model that will not load, a
    /// reply that is not a rewrite — every one of them arrives at the same
    /// place, which is the text the speaker said.
    ///
    /// The one thing it does before reaching for the network is ask whether
    /// there is anything to clean: a blank transcription is a round trip to be
    /// told what `settle` already knows.
    async fn clean(&self, model: &str, raw: &str, names: &str, policy: &Policy) -> Cleaned {
        if !worth_cleaning(raw) {
            return settle(policy, raw, None);
        }
        // The system turn, not a rendered chat template. Which template these
        // weights were trained with is the server's business now, and it is the
        // one that loaded them.
        let system = policy.system_with(names);
        let candidate = Swap::complete(self, model, &system, raw, policy.max_new_tokens)
            .await
            .inspect_err(|why| eprintln!("ayeaye: the cleanup model said nothing usable: {why}"))
            .ok();
        settle(policy, raw, candidate.as_deref())
    }
}

impl Lists for Swap {
    async fn available(&self) -> Result<Vec<String>, String> {
        Swap::models(self).await.map_err(|why| why.to_string())
    }
}

/// Turn one clip of audio into what should be typed.
///
/// `cleanup` is an `Option` rather than a second implementation to stand in for
/// the empty case: a machine with no cleanup model configured dictates the words
/// the speaker said, and `settle` already says so in one place.
pub async fn hear<B: Speech + Cleanup>(
    backend: &B,
    speech: &str,
    cleanup: Option<&str>,
    audio: &Pcm16kMono,
    names: &str,
    policy: &Policy,
) -> Outcome {
    // Measured once. `is_silence` walks every sample again to answer the same
    // question, and a clip is up to two minutes at sixteen kilohertz.
    let rms = audio.rms();
    let seconds = audio.duration_secs();

    if ayeaye_core::audio::is_silence(rms) {
        return Outcome::Silence { rms, seconds };
    }

    let raw = match backend.transcribe(speech, audio).await {
        Ok(raw) => raw,
        // Not `Empty`. "I could not listen" and "you said nothing" are opposite
        // answers, and answering the first as the second sends somebody looking
        // at their microphone for a proxy that never answered.
        Err(why) => return Outcome::Unavailable(why),
    };
    // The backstop: a door or a cough clears the energy gate and still
    // transcribes to one of a speech model's stock answers to silence, and
    // typing that into a terminal is worse than typing nothing.
    if raw.trim().is_empty() || is_hallucination(&raw) {
        return Outcome::Empty { rms, seconds };
    }

    let cleaned = match cleanup {
        Some(model) => backend.clean(model, &raw, names, policy).await,
        None => settle(policy, &raw, None),
    };
    Outcome::Heard { raw, cleaned }
}

/// How long the probe's answer is worth reusing.
///
/// `bin/ayeaye`'s `VOICE_PROBE_TTL`. The panel polls, so a probe per request
/// would be a process started and a request sent per request; and the answer is
/// allowed to change, which is the whole point of it being cached rather than
/// resolved once — a converter installed while the daemon runs, or a model added
/// to llama-swap's config, must light the talk button up without anybody
/// restarting anything.
pub const PROBE_TTL: Duration = Duration::from_secs(30);

/// Everything a dictation needs that outlives one request.
///
/// Generic over the backend, with the real one as the default so nothing above
/// this file mentions the parameter. That is the same boundary the two model
/// slots used to be, and it is what lets the suite watch a cleanup model *fail*
/// and a transcription come back empty without a gigabyte of weights or a proxy
/// on a port.
///
/// **No lock around the backend.** Two dictations at once are two HTTP requests,
/// and what happens to them is llama-swap's decision to make — it is the thing
/// that knows whether the second one needs a model swap. Serialising them here
/// would be this process guessing on its behalf.
pub struct Voice<B = Swap> {
    settings: ModelSettings,
    policy: Policy,
    converter: String,
    backend: B,
    probed: Mutex<Option<(Instant, Capability)>>,
}

impl<B> std::fmt::Debug for Voice<B> {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        out.debug_struct("Voice")
            .field("speech", &self.settings.speech)
            .field("cleanup", &self.settings.cleanup)
            .finish()
    }
}

impl Voice {
    /// A voice on this machine, talking to the proxy at `backend`.
    ///
    /// `policy` arrives resolved rather than being assembled here, for the same
    /// reason the token and cliban do on [`crate::config::Settings`]: it is read
    /// from the environment and a file, neither of which this should touch. It
    /// also matters more than it looks — a policy built by hand out of
    /// `ModelSettings::cleanup_prompt` pairs somebody's own prompt with the
    /// default prompt's echo phrases, which `Policy::resolve` exists to prevent,
    /// and leaves `CLEANUP_MAX_TOKENS` reading nothing at all.
    pub fn new(settings: ModelSettings, policy: Policy, converter: String, backend: Swap) -> Voice {
        Voice::with_backend(settings, policy, converter, backend)
    }
}

impl<B: Speech + Cleanup + Lists> Voice<B> {
    /// A voice talking to something other than a real proxy.
    pub fn with_backend(
        settings: ModelSettings,
        policy: Policy,
        converter: String,
        backend: B,
    ) -> Voice<B> {
        Voice {
            settings,
            policy,
            converter,
            backend,
            probed: Mutex::new(None),
        }
    }

    /// What this machine can do about a dictation, right now.
    ///
    /// Cached whole for [`PROBE_TTL`], not just the converter answer: the panel
    /// polls this, and the model list behind `speech_ready` is now a request to
    /// the proxy on top of the process. The answer is *allowed* to change, which
    /// is the point of a cache with a lifetime rather than a value resolved once.
    ///
    /// A benign race between two callers costs one extra probe. The lock is never
    /// held across the `await`, because a probe that blocked every other request
    /// on one slow round trip would be worse than probing twice.
    pub async fn probe(&self) -> Capability {
        if let Some(fresh) = self.probed_recently() {
            return fresh;
        }
        // A proxy that cannot be reached serves no models, which is the honest
        // answer: every name is unready, and `Capability::blocker` says so with
        // the model's own name rather than a network error nobody asked about.
        let served = self.backend.available().await.unwrap_or_default();
        let ready =
            |chosen: &Option<String>| chosen.as_ref().is_some_and(|name| served.contains(name));
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
    /// forever.
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
            return Outcome::Unavailable(dictation::NO_SPEECH_MODEL.to_string());
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
    /// stops being involved, so the suite can drive the pipeline without a
    /// program on the machine — and so AYEAYE-68, which removes the converter
    /// entirely by having both recorders send raw PCM, has the seam it needs
    /// already cut.
    pub async fn hear_decoded(&self, audio: &Pcm16kMono, names: &str) -> Outcome {
        let Some(speech) = self.settings.speech.as_deref() else {
            return Outcome::Unavailable(dictation::NO_SPEECH_MODEL.to_string());
        };
        hear(
            &self.backend,
            speech,
            self.settings.cleanup.as_deref(),
            audio,
            names,
            &self.policy,
        )
        .await
    }

    /// The backend, for a test that has something to ask it.
    ///
    /// `cfg(test)`-free because the integration suite is a separate crate and
    /// cannot see a `#[cfg(test)]` item.
    pub fn backend(&self) -> &B {
        &self.backend
    }
}

/// How long the converter gets to say it exists.
///
/// Short: this is a probe behind a poll, and a converter that cannot answer in a
/// couple of seconds is one the dictation below would not survive either.
pub const PROBE_LIMIT: Duration = Duration::from_secs(2);

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
    /// What this machine calls itself, so a qualified pane id can be checked
    /// against the panes this machine actually has.
    pub here: ayeaye_core::peer::HostName,
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
        // Before the microphone is touched. A target this machine does not have
        // is knowable now, and finding out at the *second* press costs somebody
        // the recording, the transcription and the cleanup model as well as the
        // words.
        if let Err(why) = self.pane_of(pane).await {
            return match why {
                NoPane::NotThere => format!("voice: {pane} is not a pane on this machine"),
                NoPane::CouldNotLook(why) => {
                    format!("voice: could not ask tmux about {pane}: {why}")
                }
            };
        }
        let (host, label) = dictation::recording_device(peer);
        let recorder = Recorder::at(&host, self.port, &self.token);

        match recorder.health().await {
            Err(why) => return format!("voice: no recorder on {label} — {why}"),
            Ok(reply) if reply.status == 401 => {
                return format!("voice: {label} rejected the token");
            }
            // A 200 that does not say `ok` is an agent running on a machine with
            // no microphone it knows how to use, which is a different thing to
            // fix and has to say so.
            Ok(reply) if !reply.healthy() => {
                return format!("voice: {label} has no recording backend");
            }
            Ok(_) => {}
        }
        match recorder.start().await {
            Ok(reply) if reply.status == 200 => {}
            Ok(reply) => {
                return format!(
                    "voice: {label} would not start recording ({})",
                    reply.status
                );
            }
            Err(why) => return format!("voice: lost contact with {label} — {why}"),
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
            return format!("voice: could not remember the recording: {why}");
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

    /// The pane this id names, if this machine really has it.
    ///
    /// **Membership, not syntax**, which is the rule everywhere else a pane is
    /// acted on — `server::pane_of` makes the same check for the same reason.
    /// `PaneId::parse` only says an id *could* name a pane; it compares no host
    /// and asks no tmux, so `elsewhere/%0` would parse and then act on the local
    /// `%0`. That matters more here than it looks: the id `type_into` uses is
    /// read back out of a **state file on disk**, which outlives the process
    /// that wrote it and the pane it named.
    async fn pane_of(&self, qualified: &str) -> Result<ayeaye_core::tmux::Pane, NoPane> {
        // Not `.ok()?`. `Tmux::panes` is careful to tell a tmux that could not be
        // asked from a machine with no panes, and collapsing the two here would
        // report a wedged tmux as "that pane is gone" — after the recording, the
        // transcription and the cleanup model have all run, which is the most
        // expensive possible moment to be told the wrong thing.
        let panes = self
            .tmux
            .panes(&self.here)
            .await
            .map_err(|why| NoPane::CouldNotLook(why.to_string()))?;
        panes
            .into_iter()
            .find(|pane| pane.id.qualified() == qualified)
            .ok_or(NoPane::NotThere)
    }

    /// The identifiers on the pane being dictated into.
    ///
    /// Every way of failing is the same answer — no names — because the hint is
    /// an improvement to the spelling and losing it must not cost the dictation.
    async fn vocabulary(&self, pane: &str) -> String {
        let Ok(pane) = self.pane_of(pane).await else {
            return String::new();
        };
        let captured = self
            .tmux
            .ask(&[
                "capture-pane",
                "-p",
                "-t",
                pane.id.pane(),
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
        let target = match self.pane_of(pane).await {
            Ok(target) => target,
            // Told apart, because they are opposite instructions to whoever
            // reads them: one is a pane that has gone, the other is a tmux that
            // would not answer and will answer again in a moment.
            Err(NoPane::NotThere) => {
                return format!("voice: {pane} is not a pane on this machine any more");
            }
            Err(NoPane::CouldNotLook(why)) => {
                return format!("voice: could not ask tmux about {pane}: {why}");
            }
        };
        let Some(typed) = prompt::typed(text) else {
            return "voice: the rewrite carried something that cannot be typed".to_string();
        };
        // Through `Tmux::type_text`, which takes a `Pane` — and a `Pane` comes
        // back from `Tmux::panes` and nowhere else, so the target of the
        // `send-keys` is one this tmux itself named a moment ago. `-l --` and no
        // Enter live in that method, which is why they are not spelled here.
        match self.tmux.type_text(&target, typed).await {
            Ok(()) => String::new(),
            Err(why) => format!("voice: could not type it into {pane}: {why}"),
        }
    }

    /// Clear the indicator, then hand back whatever there is to say.
    async fn finish(&self, said: String) -> String {
        self.indicate("").await;
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

/// Which tmux client's machine should record, as an address.
///
/// **Prefer the pid tmux handed the binding; then the pane's own session's
/// clients, taking the first with an address.** A remote client is the
/// interesting one: it is the machine the person is sitting at, and therefore
/// the machine with the microphone. `None` means every client is on this
/// machine, and so is the microphone.
///
/// `processes` is an argument rather than `crate::process::here()` read inside,
/// which is what makes this decision testable at all — the real backend can only
/// answer about processes that exist on the machine running the suite, and "the
/// pid tmux passed is dead, so ask the session's clients instead" is not a state
/// a test can arrange out of real pids without racing the machine.
///
/// Both questions go through `crate::process` rather than `/proc`, which exists
/// on exactly one of the two platforms this supports: reading it directly on a
/// Mac was not an error but an empty answer, and every client looked local.
pub fn recording_peer(
    processes: &dyn crate::process::Processes,
    passed: Option<&str>,
    clients: &[u32],
) -> Option<String> {
    if let Some(pid) = passed.and_then(|pid| pid.parse::<u32>().ok())
        && processes.exists(pid)
    {
        // Asked once, and its answer taken whatever it is. A live client that is
        // local is a local recording, not a reason to go looking for a remote
        // one — tmux told us which client pressed the key.
        return processes.ssh_peer(pid);
    }
    clients.iter().find_map(|pid| processes.ssh_peer(*pid))
}

/// Why there is no pane to dictate into.
///
/// Two cases and not one, because they are opposite instructions to whoever
/// reads the status line: a pane that has gone is somebody's own doing, and a
/// tmux that would not answer will answer again in a moment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoPane {
    /// The id is not in the list tmux just gave us.
    NotThere,
    /// tmux could not be asked at all.
    CouldNotLook(String),
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
    // Written *and flushed* before the rename. Without the sync the rename can
    // reach the disk before the bytes do, which is the one ordering that turns a
    // power loss into a state file that exists, is named correctly, and is
    // empty — the case the temporary name was supposed to make impossible.
    if let Err(why) = write_and_sync(&temporary, state.encode().as_bytes()) {
        // Not left behind for somebody to find later and wonder about, whichever
        // of the write, the flush or the create was the one that failed.
        let _ = std::fs::remove_file(&temporary);
        return Err(format!("{}: {why}", temporary.display()));
    }
    std::fs::rename(&temporary, path).map_err(|why| {
        // Not left behind for somebody to find later and wonder about.
        let _ = std::fs::remove_file(&temporary);
        format!("{}: {why}", path.display())
    })
}

/// Write a file and make sure the bytes have really landed.
fn write_and_sync(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;

    let mut file = std::fs::File::create(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

/// Forget the recording. A state that is not there is already forgotten.
pub fn clear_state(path: &Path) {
    let _ = std::fs::remove_file(path);
}
