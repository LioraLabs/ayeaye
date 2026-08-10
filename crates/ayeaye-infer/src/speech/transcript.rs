//! What comes back from a transcription.

/// One window of audio and what was heard in it.
#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    /// Where this window starts in the clip, in seconds.
    pub start_secs: f32,
    /// Where it ends. The last segment ends where the audio does, not where
    /// the window would have.
    pub end_secs: f32,
    /// What was heard, with the steering tokens stripped.
    pub text: String,
}

/// Everything heard in a clip, window by window.
///
/// Segmented rather than one string because a model sees a fixed window —
/// thirty seconds for every released Whisper size — and a clip longer than
/// that is decoded in several passes. Returning only the first pass, which is
/// what a single `String` invites, looks exactly like a successful
/// transcription of a shorter recording.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Transcript {
    /// One per window of audio, in order.
    pub segments: Vec<Segment>,
}

impl Transcript {
    /// Everything heard, as one line.
    pub fn text(&self) -> String {
        let mut text = String::new();
        for segment in &self.segments {
            let piece = segment.text.trim();
            if piece.is_empty() {
                continue;
            }
            if !text.is_empty() {
                text.push(' ');
            }
            text.push_str(piece);
        }
        text
    }

    /// Whether anything was heard at all.
    pub fn is_empty(&self) -> bool {
        self.segments.iter().all(|s| s.text.trim().is_empty())
    }
}
