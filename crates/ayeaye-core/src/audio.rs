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
}

#[cfg(test)]
mod tests {
    use super::{Pcm16kMono, SAMPLE_RATE_HZ};

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
