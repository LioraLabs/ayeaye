//! What names a model, and what a directory may be named after.

use std::fmt;
use std::path::PathBuf;

/// The revision fetched when nobody says which.
pub const DEFAULT_REVISION: &str = "main";

/// A model on the hub: an owner, a name, and a revision.
///
/// Parsed rather than taken as three strings, because the thing a user types
/// is one string — `openai/whisper-tiny.en`, optionally `@` a revision — and
/// the refusals below only mean anything if there is one place they happen.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ModelId {
    owner: String,
    name: String,
    revision: String,
}

/// Why a string is not a model id.
///
/// These are not pedantry. The id becomes a *directory name* under the state
/// directory, so `..` in a segment is a path that climbs out of it, and a
/// segment carrying `/` is one that lands somewhere else entirely. Refusing at
/// the parse is what makes every later `join` safe by construction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BadModelId {
    /// Nothing, or only whitespace.
    Empty,
    /// No `owner/name` at all, or more than one separator.
    NotOwnerSlashName(String),
    /// A segment that is empty.
    EmptySegment(String),
    /// A segment a directory must not be named after.
    Traversal(String),
    /// A character no segment may carry.
    BadCharacter { segment: String, character: char },
}

impl fmt::Display for BadModelId {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BadModelId::Empty => write!(out, "a model id cannot be empty"),
            BadModelId::NotOwnerSlashName(given) => write!(
                out,
                "{given:?} is not a model id: it has to be owner/name, as in \
                 openai/whisper-small.en, with an optional @revision"
            ),
            BadModelId::EmptySegment(given) => {
                write!(out, "{given:?} has an empty owner, name or revision")
            }
            BadModelId::Traversal(segment) => write!(
                out,
                "{segment:?} cannot name part of a model id: a model is stored in a \
                 directory named after it, and that would climb out of it"
            ),
            BadModelId::BadCharacter { segment, character } => write!(
                out,
                "{segment:?} carries {character:?}, which a model id may not: \
                 letters, digits, '.', '-' and '_' only"
            ),
        }
    }
}

impl std::error::Error for BadModelId {}

impl ModelId {
    /// Read a model id, refusing anything a directory must not be named after.
    pub fn parse(given: &str) -> Result<ModelId, BadModelId> {
        let given = given.trim();
        if given.is_empty() {
            return Err(BadModelId::Empty);
        }
        let (repo, revision) = match given.split_once('@') {
            Some((repo, revision)) => (repo, revision),
            None => (given, DEFAULT_REVISION),
        };
        let Some((owner, name)) = repo.split_once('/') else {
            return Err(BadModelId::NotOwnerSlashName(given.to_string()));
        };
        if name.contains('/') {
            return Err(BadModelId::NotOwnerSlashName(given.to_string()));
        }
        for segment in [owner, name, revision] {
            check_segment(segment, given)?;
        }
        Ok(ModelId {
            owner: owner.to_string(),
            name: name.to_string(),
            revision: revision.to_string(),
        })
    }

    /// The account or organisation the repository sits under.
    pub fn owner(&self) -> &str {
        &self.owner
    }

    /// The repository's own name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The revision: a branch, a tag, or a commit.
    ///
    /// A revision carrying `/` is refused, because it is a path segment here
    /// like the other two. That really does put some of the hub's names out of
    /// reach — `refs/pr/1`, or a branch called `feature/x` — and pinning a
    /// commit is the way to reach any of them. Admitting them would mean
    /// letting a separator into a name a directory is created from, which is
    /// the one thing [`BadModelId`] exists to stop.
    pub fn revision(&self) -> &str {
        &self.revision
    }

    /// `owner/name`, which is what the hub calls the repository.
    pub fn repo(&self) -> String {
        format!("{}/{}", self.owner, self.name)
    }

    /// Where this model's files live, relative to the state directory.
    ///
    /// The revision is part of the path rather than something overwritten in
    /// place: two revisions of one model are two models as far as anything
    /// loading them is concerned, and a pull that silently replaced the weights
    /// under a resident model would be the worst kind of surprise.
    pub fn relative_dir(&self) -> PathBuf {
        PathBuf::from("models")
            .join(&self.owner)
            .join(&self.name)
            .join(&self.revision)
    }
}

impl fmt::Display for ModelId {
    /// Round-trips through [`ModelId::parse`]. The default revision is left
    /// off, so what a user typed is what they are shown back.
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(out, "{}/{}", self.owner, self.name)?;
        if self.revision != DEFAULT_REVISION {
            write!(out, "@{}", self.revision)?;
        }
        Ok(())
    }
}

/// Refuse a segment a directory must not be named after.
fn check_segment(segment: &str, given: &str) -> Result<(), BadModelId> {
    if segment.is_empty() {
        return Err(BadModelId::EmptySegment(given.to_string()));
    }
    if segment == "." || segment == ".." {
        return Err(BadModelId::Traversal(segment.to_string()));
    }
    if let Some(character) = segment
        .chars()
        .find(|c| !(c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_')))
    {
        return Err(BadModelId::BadCharacter {
            segment: segment.to_string(),
            character,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{BadModelId, DEFAULT_REVISION, ModelId};
    use std::path::PathBuf;

    // AYEAYE-56 — what a user types, with and without a revision.
    #[test]
    fn a_well_formed_id_parses_and_defaults_its_revision() {
        let plain = ModelId::parse("openai/whisper-small.en").expect("a well-formed id");
        assert_eq!(plain.owner(), "openai");
        assert_eq!(plain.name(), "whisper-small.en");
        assert_eq!(plain.revision(), DEFAULT_REVISION);
        assert_eq!(plain.repo(), "openai/whisper-small.en");

        let pinned = ModelId::parse("openai/whisper-small.en@a1b2c3d").expect("a pinned id");
        assert_eq!(pinned.revision(), "a1b2c3d");

        // Shown back the way it was typed, and it parses again from that.
        assert_eq!(plain.to_string(), "openai/whisper-small.en");
        assert_eq!(pinned.to_string(), "openai/whisper-small.en@a1b2c3d");
        assert_eq!(ModelId::parse(&pinned.to_string()), Ok(pinned));

        // Surrounding whitespace is what a config file leaves behind, not part
        // of the name.
        assert_eq!(
            ModelId::parse("  openai/whisper-small.en\n"),
            Ok(plain.clone())
        );

        assert_eq!(
            plain.relative_dir(),
            PathBuf::from("models/openai/whisper-small.en/main")
        );
    }

    // AYEAYE-56 — the refusals that matter: the id names a directory under the
    // state directory, so a segment that climbs out of it, or that carries a
    // separator, must never reach a `join`. These are the inputs that would
    // otherwise write outside the model store.
    #[test]
    fn an_id_that_would_escape_the_state_directory_is_refused() {
        assert_eq!(
            ModelId::parse("../../etc/passwd").unwrap_err(),
            BadModelId::NotOwnerSlashName("../../etc/passwd".to_string()),
            "more than one separator is not owner/name"
        );
        assert_eq!(
            ModelId::parse("openai/..").unwrap_err(),
            BadModelId::Traversal("..".to_string())
        );
        assert_eq!(
            ModelId::parse("../whisper").unwrap_err(),
            BadModelId::Traversal("..".to_string())
        );
        assert_eq!(
            ModelId::parse("openai/whisper@..").unwrap_err(),
            BadModelId::Traversal("..".to_string()),
            "the revision is a path segment too"
        );
        assert_eq!(
            ModelId::parse("openai/whisper@../../x").unwrap_err(),
            BadModelId::BadCharacter {
                segment: "../../x".to_string(),
                character: '/'
            }
        );
    }

    // AYEAYE-56 — and the rest of the shapes that are not an id.
    #[test]
    fn the_other_malformed_ids_are_refused_with_a_reason() {
        assert_eq!(ModelId::parse("").unwrap_err(), BadModelId::Empty);
        assert_eq!(ModelId::parse("   ").unwrap_err(), BadModelId::Empty);
        assert_eq!(
            ModelId::parse("whisper-small").unwrap_err(),
            BadModelId::NotOwnerSlashName("whisper-small".to_string()),
            "a bare name has no owner"
        );
        assert_eq!(
            ModelId::parse("/whisper").unwrap_err(),
            BadModelId::EmptySegment("/whisper".to_string())
        );
        assert_eq!(
            ModelId::parse("openai/").unwrap_err(),
            BadModelId::EmptySegment("openai/".to_string())
        );
        assert_eq!(
            ModelId::parse("openai/whisper@").unwrap_err(),
            BadModelId::EmptySegment("openai/whisper@".to_string())
        );
        assert_eq!(
            ModelId::parse("openai/whis per").unwrap_err(),
            BadModelId::BadCharacter {
                segment: "whis per".to_string(),
                character: ' '
            }
        );
        assert_eq!(
            ModelId::parse("openai/whisper\u{7}").unwrap_err(),
            BadModelId::BadCharacter {
                segment: "whisper\u{7}".to_string(),
                character: '\u{7}'
            },
            "a control character in a name is a directory nothing can address"
        );

        // And the message says which value and why, because the fix is to
        // change that value and nothing else can say which one it was.
        let said = ModelId::parse("openai/..").unwrap_err().to_string();
        assert!(said.contains("..") && said.contains("directory"), "{said}");
    }
}
