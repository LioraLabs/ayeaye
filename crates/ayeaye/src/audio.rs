//! Whatever a recorder sent, turned into the one shape a speech model reads.
//!
//! **The external converter stays, and that is a decision with a reason.**
//! Browsers record in Opus inside WebM, and the pure-Rust decoding crates do not
//! implement that codec; the only binding that does is to a C library, which
//! would put back exactly the toolchain cost the inference decision exists to
//! avoid. The converter is also packaged on every platform this ships to, unlike
//! the two runtimes being removed here, so the reach argument that justifies
//! embedding inference does not apply to it. AYEAYE-68 is where both recorders
//! change to send raw PCM and this goes.
//!
//! Nothing here decides anything about the audio. The bytes go to the converter
//! and come back as a WAVE, and what a WAVE means is `ayeaye_core::audio`.

use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use ayeaye_core::Pcm16kMono;
use ayeaye_core::audio::{self, BadWav};

use crate::command::{self, Failed};

/// The program that does the converting.
pub const CONVERTER: &str = "ffmpeg";

/// The containers a recorder is allowed to say it sent.
///
/// `bin/ayeaye`'s `AUDIO_EXTS`. It is an allowlist because the extension is
/// chosen by the client and ends up as the name of a file this process writes:
/// nothing here would follow a `../`, but a name arriving from a request is
/// worth checking against a list rather than reasoning about.
pub const EXTENSIONS: &[&str] = &["webm", "ogg", "mp4", "wav", "m4a"];

/// How long the converter gets.
///
/// Generous, because it is decoding an upload rather than answering a poll, and
/// bounded because this runs inside a request: a wedged converter must cost one
/// dictation rather than the connection.
pub const LIMIT: Duration = Duration::from_secs(60);

/// Why a clip could not be turned into audio.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    /// A container this build will not accept.
    BadExtension(String),
    /// There is no converter on this machine, or it could not be run.
    NoConverter(Failed),
    /// It ran and did not finish. Its own case rather than [`Self::NoConverter`],
    /// because "there is no ffmpeg on this machine" sends whoever reads it off to
    /// install a program they already have.
    TimedOut(Duration),
    /// It ran, and refused.
    Refused(String),
    /// It produced something that is not the audio it was asked for.
    NotAudio(BadWav),
    /// The filesystem refused something.
    Disk(String),
}

impl fmt::Display for DecodeError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            // Every one of these is a sentence somebody reads on a phone, so
            // each says what happened rather than which layer it happened in.
            DecodeError::BadExtension(ext) => write!(out, "{ext:?} is not audio this build reads"),
            DecodeError::NoConverter(why) => {
                write!(out, "there is no {CONVERTER} on this machine: {why}")
            }
            DecodeError::TimedOut(limit) => {
                write!(out, "{CONVERTER} did not finish within {limit:?}")
            }
            DecodeError::Refused(said) => {
                write!(out, "{CONVERTER} could not read the clip: {said}")
            }
            DecodeError::NotAudio(why) => write!(out, "{CONVERTER} produced {why}"),
            DecodeError::Disk(why) => write!(out, "{why}"),
        }
    }
}

impl std::error::Error for DecodeError {}

/// Whether this build will accept a clip in this container.
pub fn is_known_extension(ext: &str) -> bool {
    EXTENSIONS.contains(&ext)
}

/// The converter's command line.
///
/// Its own function so the flags are readable and can be held to without running
/// anything. Each one is load-bearing:
///
/// - `-nostdin`, because this runs inside a daemon and a converter that decided
///   to prompt would hold the request until the limit;
/// - `-ac 1 -ar 16000`, which is the whole job: the rate and channel count
///   `Pcm16kMono` promises;
/// - `-f wav` and an explicit `pcm_s16le`, so the output is the shape the reader
///   accepts rather than whatever the converter inferred from a file extension;
/// - `-y`, because the output path is one this process just made and the
///   alternative is a converter blocking on a question nobody can answer.
pub fn argv(converter: &str, source: &Path, into: &Path) -> Vec<String> {
    [converter, "-nostdin", "-loglevel", "error", "-i"]
        .iter()
        .map(|arg| (*arg).to_string())
        .chain([source.to_string_lossy().into_owned()])
        .chain(
            [
                "-ac",
                "1",
                "-ar",
                "16000",
                "-c:a",
                "pcm_s16le",
                "-f",
                "wav",
                "-y",
            ]
            .iter()
            .map(|arg| (*arg).to_string()),
        )
        .chain([into.to_string_lossy().into_owned()])
        .collect()
}

/// Turn a recorded clip into 16 kHz mono audio.
///
/// Through files rather than through pipes, and deliberately: the containers a
/// browser records in carry their index at the end, so a converter reading one
/// from a pipe has to buffer the whole thing before it can seek — and where it
/// cannot, it fails on exactly the WebM the phone produces.
pub async fn decode(clip: &[u8], ext: &str) -> Result<Pcm16kMono, DecodeError> {
    decode_with(CONVERTER, clip, ext).await
}

/// The same, naming the converter.
///
/// The name is an argument so that "a machine with no converter degrades to a
/// stated reason" is a property the suite can observe. It is the one failure
/// here that cannot be produced any other way, and it is the one that happens
/// on somebody's laptop.
pub async fn decode_with(
    converter: &str,
    clip: &[u8],
    ext: &str,
) -> Result<Pcm16kMono, DecodeError> {
    if !is_known_extension(ext) {
        return Err(DecodeError::BadExtension(ext.to_string()));
    }
    // Checked before the clip is written anywhere: a refused request should cost
    // no disk at all.
    let scratch = Scratch::new()?;
    let source = scratch.path.join(format!("clip.{ext}"));
    let converted = scratch.path.join("clip16k.wav");

    std::fs::write(&source, clip)
        .map_err(|why| DecodeError::Disk(format!("could not write the clip: {why}")))?;

    let argv = argv(converter, &source, &converted);
    let ran = command::run(&argv, LIMIT).await.map_err(|why| match why {
        // Told apart, because they are opposite instructions to whoever reads
        // them: one is a program to install, the other is a clip or a machine
        // that will not finish.
        Failed::TimedOut(limit) => DecodeError::TimedOut(limit),
        not_started => DecodeError::NoConverter(not_started),
    })?;
    if !ran.ok {
        // The converter's own words. At the moment something fails they are
        // worth more to whoever has to fix it than any paraphrase.
        let said = ran.stderr.trim();
        return Err(DecodeError::Refused(if said.is_empty() {
            "it gave no reason".to_string()
        } else {
            said.to_string()
        }));
    }

    let wav = std::fs::read(&converted)
        .map_err(|why| DecodeError::Disk(format!("could not read back the audio: {why}")))?;
    audio::from_wav(&wav).map_err(DecodeError::NotAudio)
}

/// A directory of this dictation's own, removed when it goes out of scope.
///
/// Removed on the way out whatever happened, including a panic, because the
/// contents are a recording of somebody's voice and there is no reason for one
/// to outlive the request that carried it.
///
/// **Created, not opened, and readable only by this user.** The temporary
/// directory is shared, so a predictable name is a name somebody else can make
/// first: they would then own a directory this daemon writes a recording into,
/// and could swap the converted audio between the converter exiting and the read
/// below — choosing what gets transcribed and offered as a draft in somebody's
/// terminal. `create_dir` fails on a name that already exists, which is what
/// makes winning that race the whole of the defence; the random component is
/// what makes it unwinnable in the first place.
///
/// `bin/voice-dictate` gets both from `tempfile.TemporaryDirectory()`, which is
/// `mkdtemp`: random, exclusive, and `0700`. This is that, spelled out.
struct Scratch {
    path: PathBuf,
}

impl Scratch {
    fn new() -> Result<Scratch, DecodeError> {
        let mut refused = None;
        // A handful of attempts rather than one, because the name is random and
        // the only way `create_dir` fails on it twice is a collision somebody
        // arranged. Giving up after a few is what stops that being an infinite
        // loop inside a request.
        for _ in 0..8 {
            let path = std::env::temp_dir().join(format!("ayeaye-dictate-{}", name()));
            match create_private(&path) {
                Ok(()) => return Ok(Scratch { path }),
                Err(why) => refused = Some(format!("{}: {why}", path.display())),
            }
        }
        Err(DecodeError::Disk(format!(
            "could not make a private directory to decode in ({})",
            refused.unwrap_or_else(|| "no reason given".to_string())
        )))
    }
}

/// A name nobody can guess, from the pid, the clock, and a counter.
///
/// Not a cryptographic random: there is no source of one in this binary's
/// dependencies, and the property needed is that an attacker cannot create the
/// directory *first*, which the nanosecond clock alone makes impractical. The
/// counter keeps two dictations in one process apart when the clock does not
/// tick between them, and the pid keeps two processes apart.
fn name() -> String {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let ours = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_nanos())
        .unwrap_or_default();
    format!("{}-{now:x}-{ours:x}", std::process::id())
}

/// Create a directory this user alone may look in, and fail if it is there.
#[cfg(unix)]
fn create_private(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;

    // `create_dir`, not `create_dir_all`: the latter is happy with a directory
    // that already exists, which is exactly the one that is not ours. The mode
    // is set as the directory is made rather than afterwards, so there is no
    // moment where it is readable.
    std::fs::DirBuilder::new().mode(0o700).create(path)
}

/// Everywhere else there are no permission bits to ask for, so exclusive
/// creation is the whole of what can be promised. Present for the same reason
/// `config.rs` keeps its pair: the daemon is a Unix daemon, and a crate that
/// only compiles on one platform hides that behind a build failure.
#[cfg(not(unix))]
fn create_private(path: &Path) -> std::io::Result<()> {
    std::fs::DirBuilder::new().create(path)
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}
