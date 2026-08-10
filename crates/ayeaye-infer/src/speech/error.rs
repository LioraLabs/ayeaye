//! What can go wrong between a directory of files and a transcript.

use std::fmt;
use std::path::{Path, PathBuf};

/// A failure loading or running a speech model.
///
/// Every variant that concerns a file carries the path, because the error a
/// user actually meets is "the model did not load" at two in the morning, and
/// the difference between "there is no `tokenizer.json` in that directory" and
/// "`config.json` is not valid JSON" is the difference between a fix and an
/// evening.
#[derive(Debug)]
pub enum SpeechError {
    /// A file the model needs is not in the directory it was pointed at.
    Missing {
        /// The full path that was looked for.
        file: PathBuf,
    },
    /// The file is there and could not be read.
    Unreadable {
        /// The file being read.
        file: PathBuf,
        /// What the operating system said.
        cause: String,
    },
    /// The file is there, was read, and is not what it claims to be.
    Malformed {
        /// The file being parsed.
        file: PathBuf,
        /// What the parser said.
        cause: String,
    },
    /// The tokenizer loaded but does not carry a token Whisper decoding needs.
    MissingToken {
        /// The special token that is absent, e.g. `<|startoftranscript|>`.
        token: String,
    },
    /// The model loaded and inference failed.
    Inference {
        /// What the tensor library said.
        cause: String,
    },
    /// A transcription was asked of a slot holding no model.
    ///
    /// Deliberately an error rather than an implicit load: see
    /// [`crate::speech::SpeechSlot`].
    NotLoaded,
}

impl SpeechError {
    /// The error for a file that is absent, or for reading one that is not.
    pub(crate) fn read(file: &Path, cause: std::io::Error) -> Self {
        if cause.kind() == std::io::ErrorKind::NotFound {
            Self::Missing {
                file: file.to_path_buf(),
            }
        } else {
            Self::Unreadable {
                file: file.to_path_buf(),
                cause: cause.to_string(),
            }
        }
    }

    /// The error for a file that read cleanly and did not parse.
    pub(crate) fn malformed(file: &Path, cause: impl fmt::Display) -> Self {
        Self::Malformed {
            file: file.to_path_buf(),
            cause: cause.to_string(),
        }
    }

    /// The error for a tensor operation that failed.
    pub(crate) fn inference(cause: impl fmt::Display) -> Self {
        Self::Inference {
            cause: cause.to_string(),
        }
    }
}

impl fmt::Display for SpeechError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing { file } => {
                write!(f, "no speech model file at {}", file.display())
            }
            Self::Unreadable { file, cause } => {
                write!(f, "could not read {}: {cause}", file.display())
            }
            Self::Malformed { file, cause } => {
                write!(f, "{} is not what it claims to be: {cause}", file.display())
            }
            Self::MissingToken { token } => {
                write!(f, "the tokenizer has no {token} token")
            }
            Self::Inference { cause } => write!(f, "inference failed: {cause}"),
            Self::NotLoaded => write!(f, "no speech model is loaded"),
        }
    }
}

impl std::error::Error for SpeechError {}
