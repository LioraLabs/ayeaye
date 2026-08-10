//! What can go wrong between a directory of files and a rewritten sentence.

use std::fmt;
use std::path::{Path, PathBuf};

use super::model::SUPPORTED;

/// A failure loading or running a language model.
///
/// The same shape as [`crate::speech::SpeechError`] and for the same reason:
/// every variant that concerns a file carries the path, because "the model did
/// not load" at two in the morning is the error message that costs an evening.
///
/// Note what having one of these does *not* mean. A cleanup pass that fails
/// still returns the dictation — see [`super::LanguageSlot::clean`] — so these
/// are for the log and for the person configuring a model, never for a caller
/// deciding whether the user gets their words back.
#[derive(Debug)]
pub enum LanguageError {
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
    /// The file is a valid GGUF describing an architecture nothing here
    /// implements.
    ///
    /// Refused before a single tensor is read, and named, because this is a
    /// fact about the download rather than about the machine: knowing it at
    /// load time is what lets AYEAYE-56 know it at *pull* time and refuse the
    /// gigabytes instead of the sentence.
    UnsupportedArchitecture {
        /// What the file's own metadata called itself.
        found: String,
    },
    /// The model loaded and inference failed.
    Inference {
        /// What the tensor library said.
        cause: String,
    },
    /// A rewrite was asked of a slot holding no model.
    ///
    /// Deliberately an error rather than an implicit load: see
    /// [`super::LanguageSlot`].
    NotLoaded,
}

impl LanguageError {
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

impl fmt::Display for LanguageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing { file } => {
                write!(f, "no language model file at {}", file.display())
            }
            Self::Unreadable { file, cause } => {
                write!(f, "could not read {}: {cause}", file.display())
            }
            Self::Malformed { file, cause } => {
                write!(f, "{} is not what it claims to be: {cause}", file.display())
            }
            Self::UnsupportedArchitecture { found } => write!(
                f,
                "{found:?} is not an architecture this build can run; it has {}",
                SUPPORTED.join(", ")
            ),
            Self::Inference { cause } => write!(f, "inference failed: {cause}"),
            Self::NotLoaded => write!(f, "no language model is loaded"),
        }
    }
}

impl std::error::Error for LanguageError {}
