//! Audio to text: mel, encode, decode, detokenise.

use ayeaye_core::Pcm16kMono;
use candle_core::Tensor;
use candle_transformers::models::whisper::{self, audio};

use super::error::SpeechError;
use super::model::SpeechModel;
use super::transcript::{Segment, Transcript};

/// The sample rate, as the float the segment timings are computed in.
const SAMPLE_RATE_F32: f32 = ayeaye_core::audio::SAMPLE_RATE_HZ as f32;

impl SpeechModel {
    /// Transcribe 16 kHz mono audio, in this process.
    ///
    /// Audio longer than the model's window is decoded window by window and
    /// comes back as one segment per window. Nothing is truncated silently:
    /// see [`Transcript`] for why that is the shape of the return value.
    ///
    /// `&mut self` because a transformer decode is not a pure function of its
    /// input — the attention blocks carry a key-value cache that this walks
    /// forward and resets between windows.
    pub fn transcribe(&mut self, audio: &Pcm16kMono) -> Result<Transcript, SpeechError> {
        // Nobody spoke. Padding silence out to a full window and running an
        // encoder over it costs real time to be told nothing, and a real model
        // asked to transcribe silence tends to invent something.
        if audio.is_empty() {
            return Ok(Transcript::default());
        }

        let window_samples = self.window_samples();
        let mut segments = Vec::new();

        for (index, window) in audio.samples().chunks(window_samples).enumerate() {
            let start = (index * window_samples) as f32 / SAMPLE_RATE_F32;
            let text = self.transcribe_window(window)?;
            segments.push(Segment {
                start_secs: start,
                // The last window ends where the audio does, not where the
                // window would have: the padding is ours, not the speaker's.
                end_secs: start + window.len() as f32 / SAMPLE_RATE_F32,
                text,
            });
        }

        Ok(Transcript { segments })
    }

    /// How many mel frames the encoder is fed.
    ///
    /// Read from the model's own config rather than assumed to be the 3 000
    /// that every released Whisper size happens to want, so a model with a
    /// different window is decoded correctly instead of confidently.
    fn window_frames(&self) -> usize {
        self.config.max_source_positions * 2
    }

    /// How many samples make up one window of audio.
    fn window_samples(&self) -> usize {
        self.window_frames() * whisper::HOP_LENGTH
    }

    /// Transcribe one window's worth of samples.
    fn transcribe_window(&mut self, window: &[f32]) -> Result<String, SpeechError> {
        let features = self.encode(window)?;
        let tokens = self.decode(&features)?;
        self.tokenizer
            .decode(&tokens, true)
            .map_err(SpeechError::inference)
    }

    /// Mel-spectrogram the window and run the audio encoder over it.
    fn encode(&mut self, window: &[f32]) -> Result<Tensor, SpeechError> {
        // Whisper always sees a full window; a short one is zero-padded to it.
        let mut padded = window.to_vec();
        padded.resize(self.window_samples(), 0.0);

        let mel = audio::pcm_to_mel(&self.config, &padded, &self.filters);
        let bins = self.config.num_mel_bins;
        let frames = mel.len() / bins;
        let mel = Tensor::from_vec(mel, (1, bins, frames), self.selection.device())
            .map_err(SpeechError::inference)?;
        // `pcm_to_mel` pads its output past the window; the encoder wants
        // exactly the window.
        let mel = mel
            .narrow(2, 0, self.window_frames())
            .map_err(SpeechError::inference)?;

        // Each window is a fresh utterance as far as the model is concerned.
        self.whisper.reset_kv_cache();
        self.whisper
            .encoder
            .forward(&mel, true)
            .map_err(SpeechError::inference)
    }

    /// Greedily decode tokens from the encoded audio.
    fn decode(&mut self, features: &Tensor) -> Result<Vec<u32>, SpeechError> {
        let prompt = self.tokens.prompt();
        let mut tokens = prompt.clone();

        // The decoder's positional embedding is only `max_target_positions`
        // long, so this bound is the model's, not a guess. It is also what
        // makes a model that will not emit an end token still return.
        while tokens.len() < self.config.max_target_positions {
            let step = tokens.len() - prompt.len();
            let input = Tensor::new(tokens.as_slice(), self.selection.device())
                .map_err(SpeechError::inference)?
                .unsqueeze(0)
                .map_err(SpeechError::inference)?;

            let hidden = self
                .whisper
                .decoder
                .forward(&input, features, step == 0)
                .map_err(SpeechError::inference)?;
            let (_, positions, _) = hidden.dims3().map_err(SpeechError::inference)?;
            let last = hidden
                .narrow(1, positions - 1, 1)
                .map_err(SpeechError::inference)?;
            let logits = self
                .whisper
                .decoder
                .final_linear(&last)
                .map_err(SpeechError::inference)?
                .squeeze(0)
                .map_err(SpeechError::inference)?
                .squeeze(0)
                .map_err(SpeechError::inference)?
                .to_vec1::<f32>()
                .map_err(SpeechError::inference)?;

            // Nothing legal left to say ends the decode, exactly as an end
            // token does. The alternative - a fallback token - is a token the
            // model was forbidden to emit.
            let Some(next) =
                ayeaye_core::logits::best_allowed(&logits, &self.config.suppress_tokens)
            else {
                break;
            };
            if next == self.tokens.end {
                break;
            }
            tokens.push(next);
        }

        Ok(tokens.split_off(prompt.len()))
    }
}
