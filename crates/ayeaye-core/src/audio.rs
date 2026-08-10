//! Audio in the one shape a speech model will accept.

/// The sample rate every speech model in ayeaye is trained on.
pub const SAMPLE_RATE_HZ: u32 = 16_000;

/// The scale factor between a signed 16-bit sample and a float one.
///
/// `32768`, not `32767`: it is `-i16::MIN`, so `i16::MIN` maps to exactly
/// `-1.0` and nothing can overflow the range in the other direction.
const I16_SCALE: f32 = 32_768.0;

/// Sixteen-kilohertz, single-channel audio, as floats in [-1, 1].
///
/// The name is the contract. A speech model is trained at one sample rate on
/// one channel, and handing it 44.1 kHz stereo does not fail — it produces a
/// confident transcript of nothing, which is far worse. So the rate is a
/// property of the type rather than a comment on a `Vec<f32>` that gets passed
/// through four functions before it reaches a model.
///
/// Resampling and channel mixing are deliberately *not* here: they are the job
/// of whatever accepted the audio, and this crate is the place that says what
/// the result has to be.
#[derive(Debug, Clone, PartialEq)]
pub struct Pcm16kMono {
    samples: Vec<f32>,
}

impl Pcm16kMono {
    /// Take samples that are already floats in [-1, 1].
    pub fn new(samples: Vec<f32>) -> Self {
        Self { samples }
    }

    /// Take signed 16-bit samples, the shape audio arrives in on the wire.
    pub fn from_i16(samples: &[i16]) -> Self {
        Self {
            samples: samples.iter().map(|&s| f32::from(s) / I16_SCALE).collect(),
        }
    }

    /// The samples themselves.
    pub fn samples(&self) -> &[f32] {
        &self.samples
    }

    /// How many samples there are.
    pub fn len(&self) -> usize {
        self.samples.len()
    }

    /// Whether there is any audio at all.
    ///
    /// An empty clip is legal input rather than an error: it is what "nobody
    /// spoke" sounds like, and the answer to it is an empty transcript.
    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    /// How long the clip runs, in seconds.
    pub fn duration_secs(&self) -> f32 {
        self.samples.len() as f32 / SAMPLE_RATE_HZ as f32
    }

    /// How loud the clip is, root mean square.
    ///
    /// Every sample, not one in seven. `bin/voice-dictate` strides because
    /// squaring two minutes of samples in Python is slow enough to notice; here
    /// it is a pass over a float slice, and a stride is a sampling error
    /// nobody would ever look for — a clip whose speech happened to land
    /// between the strides reads as silence.
    ///
    /// An empty clip is 0.0 rather than a division by zero. Nobody spoke is a
    /// legitimate input, and it is exactly as loud as it sounds.
    pub fn rms(&self) -> f32 {
        if self.samples.is_empty() {
            return 0.0;
        }
        // Summed as f64: two minutes at 16 kHz is nearly two million squares,
        // and an f32 accumulator stops growing long before the end of that.
        let total: f64 = self
            .samples
            .iter()
            .map(|s| f64::from(*s) * f64::from(*s))
            .sum();
        (total / self.samples.len() as f64).sqrt() as f32
    }
}

/// Whether a measured loudness is room tone rather than somebody speaking.
///
/// A free function over an already-measured loudness rather than a method on the
/// clip, because every caller has just measured it: a method would walk two
/// minutes of samples a second time to answer a question the number in hand
/// already answers.
pub fn is_silence(rms: f32) -> bool {
    rms < SILENCE_RMS
}

/// Below this, a clip is room tone rather than speech.
pub const SILENCE_RMS: f32 = 1_000.0 / I16_SCALE;

/// Why a WAVE file could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BadWav {
    /// Not a RIFF/WAVE file at all.
    NotWave,
    /// A chunk that claims more bytes than the file has.
    Truncated {
        /// Which chunk.
        chunk: String,
    },
    /// A WAVE this build does not read, and what about it.
    Unsupported {
        /// What was wrong — the rate, the channel count, the sample format.
        what: &'static str,
        /// What the file said.
        found: String,
    },
    /// A WAVE missing a chunk it cannot be one without.
    Incomplete {
        /// Which chunk is not there.
        missing: &'static str,
    },
}

impl core::fmt::Display for BadWav {
    fn fmt(&self, out: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NotWave => write!(out, "this is not a RIFF/WAVE file"),
            Self::Truncated { chunk } => {
                write!(out, "the {chunk} chunk runs past the end of the file")
            }
            Self::Unsupported { what, found } => {
                write!(out, "the {what} is {found}, which this build does not read")
            }
            Self::Incomplete { missing } => {
                write!(out, "there is no {missing} chunk, so this is not audio")
            }
        }
    }
}

impl core::error::Error for BadWav {}

/// How big a `fmt ` chunk has to be before it says anything.
const FORMAT_FIELDS: usize = 16;

/// The `wFormatTag` that means uncompressed integer samples.
const PCM: u16 = 1;

/// Read 16 kHz mono 16-bit PCM out of a WAVE file.
///
/// **The one shape, and everything else refused by name.** A speech model
/// handed 44.1 kHz stereo does not fail: it produces a confident transcript of
/// nothing, which is far worse than an error and is the exact failure
/// [`Pcm16kMono`] exists to make unstateable. So this reads the file's own
/// claims about itself and refuses any that are not the shape the type promises,
/// rather than resampling — resampling is the caller's business, and on this
/// path it is the converter's.
///
/// Chunks are walked rather than read at fixed offsets. A converter writes what
/// it likes between `fmt ` and `data`, and reading `data` at offset 36 finds
/// somebody's `LIST` chunk instead and transcribes the encoder's name.
pub fn from_wav(bytes: &[u8]) -> Result<Pcm16kMono, BadWav> {
    if bytes.len() < 12 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(BadWav::NotWave);
    }

    let mut format: Option<()> = None;
    let mut at = 12;
    // The RIFF size field is deliberately not trusted: files are written with a
    // placeholder in it when the length was not known in advance, and a reader
    // that believes it either stops early or runs off the end. What is in the
    // buffer is what there is.
    while at + 8 <= bytes.len() {
        let id = &bytes[at..at + 4];
        let size = u32::from_le_bytes([bytes[at + 4], bytes[at + 5], bytes[at + 6], bytes[at + 7]])
            as usize;
        let body = at + 8;
        let named = || String::from_utf8_lossy(id).trim_end().to_string();
        let end = body
            .checked_add(size)
            .ok_or_else(|| BadWav::Truncated { chunk: named() })?;
        if end > bytes.len() {
            return Err(BadWav::Truncated { chunk: named() });
        }

        match id {
            b"fmt " => {
                check_format(&bytes[body..end])?;
                format = Some(());
            }
            b"data" => {
                if format.is_none() {
                    return Err(BadWav::Incomplete { missing: "fmt " });
                }
                return Ok(Pcm16kMono::from_i16(&little_endian_i16(&bytes[body..end])));
            }
            _ => {}
        }

        // Chunks are word-aligned: an odd-sized one carries a pad byte that is
        // not part of it and is not the start of the next chunk either.
        at = end + end % 2;
    }

    Err(BadWav::Incomplete {
        missing: if format.is_some() { "data" } else { "fmt " },
    })
}

/// Judge a `fmt ` chunk against the one shape this reads.
fn check_format(chunk: &[u8]) -> Result<(), BadWav> {
    if chunk.len() < FORMAT_FIELDS {
        return Err(BadWav::Truncated {
            chunk: "fmt".to_string(),
        });
    }
    let short = |at: usize| u16::from_le_bytes([chunk[at], chunk[at + 1]]);
    let long =
        |at: usize| u32::from_le_bytes([chunk[at], chunk[at + 1], chunk[at + 2], chunk[at + 3]]);

    let unsupported = |what, found: String| Err(BadWav::Unsupported { what, found });
    // In the order somebody would want to be told about them: what kind of
    // audio, then how much of it, then how it is written down.
    if short(0) != PCM {
        return unsupported("sample format", format!("{} rather than PCM", short(0)));
    }
    if short(2) != 1 {
        return unsupported("channel count", short(2).to_string());
    }
    if long(4) != SAMPLE_RATE_HZ {
        return unsupported("sample rate", format!("{} Hz", long(4)));
    }
    if short(14) != 16 {
        return unsupported("sample size", format!("{} bits", short(14)));
    }
    Ok(())
}

/// A run of little-endian 16-bit samples.
///
/// An odd trailing byte is dropped rather than refused: it is half a sample,
/// it cannot be one, and the rest of the clip is real audio.
fn little_endian_i16(bytes: &[u8]) -> Vec<i16> {
    bytes
        .chunks_exact(2)
        .map(|pair| i16::from_le_bytes([pair[0], pair[1]]))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{BadWav, Pcm16kMono, SAMPLE_RATE_HZ, SILENCE_RMS, from_wav, is_silence};

    /// A WAVE file, built here so a test can say what is wrong with one.
    ///
    /// Written out byte by byte rather than through a library, because the
    /// bytes *are* what is under test: a reader held to a file some crate
    /// produced agrees with that crate, not with the converter whose output
    /// this really has to read.
    fn wav(channels: u16, rate: u32, bits: u16, format: u16, samples: &[i16]) -> Vec<u8> {
        let data: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
        let mut out = Vec::new();
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
        out.extend_from_slice(b"WAVE");
        out.extend_from_slice(b"fmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&format.to_le_bytes());
        out.extend_from_slice(&channels.to_le_bytes());
        out.extend_from_slice(&rate.to_le_bytes());
        let block_align = channels * bits / 8;
        out.extend_from_slice(&(rate * u32::from(block_align)).to_le_bytes());
        out.extend_from_slice(&block_align.to_le_bytes());
        out.extend_from_slice(&bits.to_le_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&data);
        out
    }

    /// The one shape this build reads.
    fn ours(samples: &[i16]) -> Vec<u8> {
        wav(1, SAMPLE_RATE_HZ, 16, 1, samples)
    }

    // AYEAYE-58
    #[test]
    fn sixteen_kilohertz_mono_pcm_decodes_to_the_samples_it_carries() {
        let decoded = from_wav(&ours(&[0, 16_384, -16_384, i16::MIN])).expect("a WAVE we can read");

        assert_eq!(
            decoded,
            Pcm16kMono::from_i16(&[0, 16_384, -16_384, i16::MIN])
        );
        assert_eq!(decoded.len(), 4);
    }

    // AYEAYE-58
    //
    // A converter writes what it likes between `fmt ` and `data` — `LIST`
    // carrying the encoder's name is the common one. Reading `data` at a fixed
    // offset finds that chunk's bytes instead and transcribes them, which is a
    // confident transcript of nothing rather than a failure.
    #[test]
    fn a_chunk_between_the_format_and_the_audio_is_walked_past() {
        let plain = ours(&[1_000, -1_000]);
        let mut with_list = plain[..36].to_vec();
        with_list.extend_from_slice(b"LIST");
        with_list.extend_from_slice(&4u32.to_le_bytes());
        with_list.extend_from_slice(b"INFO");
        with_list.extend_from_slice(&plain[36..]);

        assert_eq!(
            from_wav(&with_list).expect("a WAVE with a LIST chunk in it"),
            from_wav(&plain).expect("the same audio without one")
        );
    }

    // AYEAYE-58
    //
    // Every one of these is refused rather than read anyway. Handing a speech
    // model 44.1 kHz stereo does not fail — it produces a confident transcript
    // of nothing, which is the failure `Pcm16kMono` exists to make unstateable.
    #[test]
    fn audio_that_is_not_the_shape_a_speech_model_wants_is_refused_by_name() {
        for (bytes, what) in [
            (wav(2, SAMPLE_RATE_HZ, 16, 1, &[0, 0]), "channel count"),
            (wav(1, 44_100, 16, 1, &[0, 0]), "sample rate"),
            (wav(1, SAMPLE_RATE_HZ, 8, 1, &[0, 0]), "sample size"),
            (wav(1, SAMPLE_RATE_HZ, 16, 3, &[0, 0]), "sample format"),
        ] {
            let refused = from_wav(&bytes).unwrap_err();
            let BadWav::Unsupported { what: named, .. } = &refused else {
                panic!("{what} should be refused as unsupported, got {refused:?}");
            };
            assert_eq!(*named, what);
            // And it says what it found, because that is the number somebody
            // has to go and change.
            assert!(!refused.to_string().is_empty());
        }
    }

    // AYEAYE-58
    #[test]
    fn something_that_is_not_a_wave_is_refused_rather_than_read_as_one() {
        assert_eq!(from_wav(b"").unwrap_err(), BadWav::NotWave);
        assert_eq!(from_wav(b"<!DOCTYPE html>").unwrap_err(), BadWav::NotWave);
        // A RIFF that is not a WAVE — an AVI, say.
        let mut avi = ours(&[0]);
        avi[8..12].copy_from_slice(b"AVI ");
        assert_eq!(from_wav(&avi).unwrap_err(), BadWav::NotWave);

        // A header with nothing after it is not silence, it is a truncated
        // upload, and answering it as an empty clip would report "nothing
        // recognised" for a network that dropped.
        let whole = ours(&[1, 2, 3, 4]);
        assert!(matches!(
            from_wav(&whole[..whole.len() - 4]).unwrap_err(),
            BadWav::Truncated { .. }
        ));
        assert_eq!(
            from_wav(&whole[..36]).unwrap_err(),
            BadWav::Incomplete { missing: "data" }
        );
    }

    // AYEAYE-58
    //
    // The gate that stops a cough costing a transcription. The numbers are the
    // daemon's, measured on real hardware: a live microphone in a quiet room is
    // about 490 and actual speech about 3 400, on the 16-bit scale.
    #[test]
    fn room_tone_is_silence_and_speech_is_not() {
        let at = |level: i16| Pcm16kMono::from_i16(&[level, -level, level, -level]);

        assert!(
            is_silence(at(490).rms()),
            "a quiet room is not somebody talking"
        );
        assert!(!is_silence(at(3_400).rms()), "speech is not room tone");
        // Gated on energy rather than on length, deliberately: "run the tests"
        // is a legitimately short clip that a duration cutoff would throw away.
        assert!(!is_silence(Pcm16kMono::from_i16(&[3_400]).rms()));

        // The measurement itself, against arithmetic done by hand: a square
        // wave's RMS is its amplitude.
        assert!(
            (at(16_384).rms() - 0.5).abs() < 1e-6,
            "{}",
            at(16_384).rms()
        );
        assert_eq!(Pcm16kMono::from_i16(&[]).rms(), 0.0);
        assert!(is_silence(Pcm16kMono::from_i16(&[]).rms()));
        assert!((SILENCE_RMS - 0.030_517_578).abs() < 1e-6);
    }

    // AYEAYE-54
    #[test]
    fn sixteen_bit_samples_become_floats_in_range() {
        let pcm = Pcm16kMono::from_i16(&[i16::MIN, 0, i16::MAX]);

        assert_eq!(pcm.samples()[0], -1.0);
        assert_eq!(pcm.samples()[1], 0.0);
        // i16::MAX is 32767, one short of the scale, so it lands just inside.
        assert!(pcm.samples()[2] < 1.0);
        assert!(pcm.samples()[2] > 0.999);
    }

    // AYEAYE-54
    #[test]
    fn duration_is_the_sample_count_at_sixteen_kilohertz() {
        let pcm = Pcm16kMono::new(vec![0.0; 24_000]);

        assert_eq!(pcm.duration_secs(), 1.5);
        assert_eq!(SAMPLE_RATE_HZ, 16_000);
    }

    // AYEAYE-54
    #[test]
    fn a_clip_nobody_spoke_into_is_empty_rather_than_invalid() {
        let pcm = Pcm16kMono::from_i16(&[]);

        assert!(pcm.is_empty());
        assert_eq!(pcm.len(), 0);
        assert_eq!(pcm.duration_secs(), 0.0);
    }
}
