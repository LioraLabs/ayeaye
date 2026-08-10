//! The mel filterbank a Whisper-family model expects its audio through.
//!
//! Computed rather than shipped. The alternative is a binary blob per bin
//! count — 80 for the models most people run, 128 for `large-v3`, and a third
//! the day a model wants one — and this is thirty lines of arithmetic with no
//! effects in it, which is what makes it the core's rather than the inference
//! crate's.

/// The frequency below which the mel scale is linear, in hertz.
const LINEAR_BELOW_HZ: f64 = 1000.0;

/// Hertz per mel in the linear region: 200/3, so 1000 Hz is exactly 15 mel.
const HZ_PER_MEL: f64 = 200.0 / 3.0;

/// Where the logarithmic region begins, in mel.
const LOG_ABOVE_MEL: f64 = LINEAR_BELOW_HZ / HZ_PER_MEL;

/// Mel-scale frequency for a frequency in hertz.
///
/// This is the Slaney scale — linear below 1 kHz, logarithmic above — and not
/// the HTK one. Which of the two is used is not a matter of taste: it decides
/// where every filter sits, and a model trained through one and fed through the
/// other hears a spectrum that has been quietly bent.
fn hz_to_mel(hz: f64) -> f64 {
    if hz < LINEAR_BELOW_HZ {
        hz / HZ_PER_MEL
    } else {
        LOG_ABOVE_MEL + (hz / LINEAR_BELOW_HZ).ln() / (6.4f64.ln() / 27.0)
    }
}

/// Frequency in hertz for a mel-scale frequency. The inverse of [`hz_to_mel`].
fn mel_to_hz(mel: f64) -> f64 {
    if mel < LOG_ABOVE_MEL {
        mel * HZ_PER_MEL
    } else {
        LINEAR_BELOW_HZ * ((mel - LOG_ABOVE_MEL) * (6.4f64.ln() / 27.0)).exp()
    }
}

/// How many FFT bins a real-valued transform of `n_fft` samples produces.
pub const fn bins_for(n_fft: usize) -> usize {
    1 + n_fft / 2
}

/// The mel filterbank: `n_mels` triangular filters over `1 + n_fft/2` FFT bins.
///
/// Laid out row-major, filter by filter — `weights[m * bins_for(n_fft) + k]` is
/// the weight filter `m` gives bin `k`. That is the layout the inference side
/// indexes it with, and it is the layout OpenAI's `mel_filters.npz` is stored
/// in, so the two agree by construction rather than by a transpose somebody
/// has to remember.
///
/// The filters are area-normalised (Slaney), meaning each is scaled by the
/// reciprocal of its own width so a wide high-frequency filter does not simply
/// out-shout a narrow low-frequency one.
pub fn mel_filterbank(sample_rate: u32, n_fft: usize, n_mels: usize) -> Vec<f32> {
    let bins = bins_for(n_fft);
    let mut weights = vec![0f32; n_mels * bins];
    if n_mels == 0 || bins == 0 {
        return weights;
    }

    let nyquist = f64::from(sample_rate) / 2.0;

    // The centre frequency of each FFT bin: bin 0 is DC, the last is Nyquist.
    let bin_hz = |k: usize| k as f64 * nyquist / (bins - 1) as f64;

    // n_mels + 2 band edges evenly spaced *on the mel scale*, so each filter
    // spans from the previous edge, through its own, to the next.
    let top_mel = hz_to_mel(nyquist);
    let edge_hz = |i: usize| mel_to_hz(i as f64 * top_mel / (n_mels + 1) as f64);

    for m in 0..n_mels {
        let (low, centre, high) = (edge_hz(m), edge_hz(m + 1), edge_hz(m + 2));
        let area = 2.0 / (high - low);
        for k in 0..bins {
            let hz = bin_hz(k);
            let rising = (hz - low) / (centre - low);
            let falling = (high - hz) / (high - centre);
            weights[m * bins + k] = (rising.min(falling).max(0.0) * area) as f32;
        }
    }

    weights
}

#[cfg(test)]
mod tests {
    use super::{bins_for, mel_filterbank};

    /// The real Whisper filterbanks, as `mel_index fft_bin weight` lines.
    ///
    /// `include_str!` is compile-time, which is how a fixture reaches a crate
    /// that may not open a file. See `crates/constitution/README.md`.
    const FILTERS_80: &str = include_str!("../tests/fixtures/whisper-melfilters-80.txt");
    const FILTERS_128: &str = include_str!("../tests/fixtures/whisper-melfilters-128.txt");

    /// Expand a sparse fixture into the dense filterbank it describes.
    fn expected(fixture: &str, n_mels: usize, n_fft: usize) -> Vec<f32> {
        let bins = bins_for(n_fft);
        let mut dense = vec![0f32; n_mels * bins];
        for line in fixture.lines().filter(|l| !l.starts_with('#')) {
            let mut field = line.split_whitespace();
            let mel: usize = field.next().unwrap().parse().unwrap();
            let bin: usize = field.next().unwrap().parse().unwrap();
            let weight: f32 = field.next().unwrap().parse().unwrap();
            dense[mel * bins + bin] = weight;
        }
        dense
    }

    /// Compare every weight, so a filterbank that is right in the places the
    /// fixture happens to name and wrong elsewhere still fails.
    fn assert_matches(got: &[f32], want: &[f32]) {
        assert_eq!(got.len(), want.len(), "filterbank is the wrong size");
        for (i, (&g, &w)) in got.iter().zip(want).enumerate() {
            assert!(
                (g - w).abs() < 1e-6,
                "weight {i} is {g}, and OpenAI's filterbank says {w}"
            );
        }
    }

    // AYEAYE-54
    //
    // The expected values are the filterbank real Whisper weights were trained
    // through, not this formula written twice: it is OpenAI's mel_filters.npz,
    // by way of candle's melfilters.bytes. Nothing else in this ticket can tell
    // a subtly wrong filterbank from a right one — the model would load, run,
    // and transcribe confident nonsense.
    #[test]
    fn the_eighty_bin_filterbank_is_the_one_whisper_was_trained_through() {
        let got = mel_filterbank(16_000, 400, 80);

        assert_matches(&got, &expected(FILTERS_80, 80, 400));
    }

    // AYEAYE-54
    #[test]
    fn the_hundred_and_twenty_eight_bin_filterbank_large_v3_wants_agrees_too() {
        let got = mel_filterbank(16_000, 400, 128);

        assert_matches(&got, &expected(FILTERS_128, 128, 400));
    }

    // AYEAYE-54
    //
    // Most of a filterbank is zero — 391 non-zero weights out of 16 080 — so a
    // function that returned zeros everywhere would pass a spot check. This is
    // what makes the comparison above mean something.
    #[test]
    fn the_filterbank_is_mostly_zero_but_not_entirely() {
        let got = mel_filterbank(16_000, 400, 80);

        assert_eq!(got.len(), 80 * 201);
        assert_eq!(got.iter().filter(|w| **w != 0.0).count(), 391);
    }
}
