//! Whether what arrived is the file it was supposed to be.
//!
//! **Structural, not cryptographic, and that is a claim about honesty rather
//! than about effort.** There is no trusted source for a per-file checksum
//! here: the only digest available comes down the same connection as the bytes,
//! so checking one against the other proves that a download agrees with itself.
//! Writing that would look like assurance and be none.
//!
//! What this catches is what actually goes wrong. A download is interrupted and
//! the file is short. A repository is private, or the name was mistyped, and
//! the hub answers with a page of HTML which is saved under the name
//! `model.safetensors`. Both are caught here, before the file is moved
//! somewhere a load will find it, and both are caught from the bytes alone.

use std::fmt;

/// Why a file that arrived cannot be kept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unusable {
    /// Nothing arrived at all.
    Empty {
        /// The file it was supposed to be.
        file: String,
    },
    /// It arrived, and it is not what it claims to be.
    Malformed {
        /// The file it was supposed to be.
        file: String,
        /// What is wrong with it, in the words the refusal will carry.
        why: String,
    },
}

impl fmt::Display for Unusable {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Unusable::Empty { file } => write!(out, "{file} came back empty"),
            Unusable::Malformed { file, why } => write!(out, "{file} {why}"),
        }
    }
}

impl std::error::Error for Unusable {}

/// Judge one file that has just been fetched.
///
/// The check is chosen by name, because the three files a model needs are three
/// different formats and "is this a plausible file" is not a question with one
/// answer. A name this does not recognise is checked for being non-empty and
/// nothing more — refusing an unknown file outright would make adding one to
/// [`super::hub::WANTED`] silently impossible.
pub fn check(file: &str, bytes: &[u8]) -> Result<(), Unusable> {
    if bytes.is_empty() {
        return Err(Unusable::Empty {
            file: file.to_string(),
        });
    }
    match file {
        super::WEIGHTS_FILE => safetensors(file, bytes),
        super::CONFIG_FILE | super::TOKENIZER_FILE => json_object(file, bytes),
        _ => Ok(()),
    }
}

/// A safetensors file opens with its own length, which is what makes a
/// truncated one detectable without reading it all.
///
/// The format is: eight bytes of little-endian length, then that many bytes of
/// JSON header, then the tensor data. So a file that was cut short says it is
/// longer than it is, and a page of HTML read as a length says it is
/// astronomically longer than it is. One comparison catches both.
fn safetensors(file: &str, bytes: &[u8]) -> Result<(), Unusable> {
    let refuse = |why: String| {
        Err(Unusable::Malformed {
            file: file.to_string(),
            why,
        })
    };

    let Some(length) = bytes.get(..8) else {
        return refuse(format!(
            "is {} bytes, too short to be safetensors at all",
            bytes.len()
        ));
    };
    let declared = u64::from_le_bytes(length.try_into().expect("eight bytes"));
    if declared == 0 {
        return refuse("declares an empty header, so it describes no tensors".to_string());
    }
    // Compared in u128 so the arithmetic cannot wrap: an HTML page read as a
    // length really is near u64::MAX, and `8 + declared` would overflow.
    if u128::from(declared) + 8 > bytes.len() as u128 {
        return refuse(format!(
            "declares a {declared}-byte header but is {} bytes long: it is truncated, \
             or it is not safetensors",
            bytes.len()
        ));
    }
    if bytes.get(8) != Some(&b'{') {
        return refuse(
            "has a header that does not open with '{', so it is not safetensors".to_string(),
        );
    }
    Ok(())
}

/// A file that is supposed to be JSON has to at least start like it.
///
/// Not a parse: `tokenizer.json` is megabytes and the loader will parse it
/// properly anyway. This catches the case that is worth catching here, which is
/// a page of HTML saved under a model file's name.
fn json_object(file: &str, bytes: &[u8]) -> Result<(), Unusable> {
    let text = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes);
    let first = text
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
        .map(|at| text[at]);
    if first == Some(b'{') {
        return Ok(());
    }
    Err(Unusable::Malformed {
        file: file.to_string(),
        why: format!(
            "does not open with '{{', so it is not the JSON it should be. \
             It starts {:?}",
            String::from_utf8_lossy(&text[..text.len().min(40)])
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::{Unusable, check};
    use crate::model::{CONFIG_FILE, TOKENIZER_FILE, WEIGHTS_FILE};

    /// A safetensors file of the smallest shape that is a real one: a length,
    /// a JSON header of exactly that length, and no tensor data.
    fn safetensors(header: &str) -> Vec<u8> {
        let mut bytes = (header.len() as u64).to_le_bytes().to_vec();
        bytes.extend_from_slice(header.as_bytes());
        bytes
    }

    /// What a hub serves when a repository is private or the name was mistyped.
    const PAGE: &[u8] = b"<!DOCTYPE html>\n<html><head><title>404</title></head></html>";

    // AYEAYE-56 — a real header passes, which is what stops this from being a
    // check that refuses everything and looks careful doing it.
    #[test]
    fn weights_with_a_header_that_describes_itself_are_kept() {
        assert_eq!(
            check(WEIGHTS_FILE, &safetensors(r#"{"__metadata__":{}}"#)),
            Ok(())
        );

        // With tensor data after the header, which is the real shape.
        let mut with_data =
            safetensors(r#"{"a":{"dtype":"F32","shape":[1],"data_offsets":[0,4]}}"#);
        with_data.extend_from_slice(&[0u8; 4]);
        assert_eq!(check(WEIGHTS_FILE, &with_data), Ok(()));
    }

    // AYEAYE-56 — the failure that actually happens: the connection dropped
    // and the file is short. The header says how long it is, so a truncated
    // file contradicts itself and can be caught without reading it all.
    #[test]
    fn truncated_weights_are_refused_because_they_contradict_their_own_header() {
        let whole = safetensors(r#"{"__metadata__":{"a":"bbbbbbbbbbbbbbbbbbbb"}}"#);
        let cut = &whole[..whole.len() - 10];

        let refused = check(WEIGHTS_FILE, cut).unwrap_err();
        let Unusable::Malformed { file, why } = &refused else {
            panic!("expected a malformed file, got {refused:?}");
        };
        assert_eq!(file, WEIGHTS_FILE);
        assert!(why.contains("truncated"), "{why}");
        // And it says both numbers, because "truncated" without them leaves
        // nobody able to tell a short download from the wrong file.
        assert!(why.contains(&cut.len().to_string()), "{why}");
    }

    // AYEAYE-56 — the other failure that actually happens. Read as a
    // little-endian length, the first eight bytes of a web page are an
    // astronomical number, so this must not overflow on the way to refusing it.
    #[test]
    fn a_page_of_html_saved_as_weights_is_refused_rather_than_overflowing() {
        let refused = check(WEIGHTS_FILE, PAGE).unwrap_err();
        assert!(matches!(refused, Unusable::Malformed { .. }), "{refused:?}");

        // The extreme of the same shape, which is what would wrap `8 + declared`.
        let mut worst = u64::MAX.to_le_bytes().to_vec();
        worst.push(b'{');
        assert!(check(WEIGHTS_FILE, &worst).is_err());
    }

    // AYEAYE-56 — and the degenerate shapes around the edge of the format.
    #[test]
    fn weights_that_are_too_short_or_describe_nothing_are_refused() {
        assert_eq!(
            check(WEIGHTS_FILE, &[1, 2, 3]).unwrap_err(),
            Unusable::Malformed {
                file: WEIGHTS_FILE.to_string(),
                why: "is 3 bytes, too short to be safetensors at all".to_string()
            }
        );
        assert!(
            check(WEIGHTS_FILE, &0u64.to_le_bytes()).is_err(),
            "a zero-length header"
        );
        // Exactly long enough, but the header is not JSON.
        let mut not_json = 1u64.to_le_bytes().to_vec();
        not_json.push(b'<');
        assert!(check(WEIGHTS_FILE, &not_json).is_err());
    }

    // AYEAYE-56 — an empty file is not a file, whichever of them it is.
    #[test]
    fn nothing_at_all_is_refused_for_every_file() {
        for file in [CONFIG_FILE, TOKENIZER_FILE, WEIGHTS_FILE] {
            assert_eq!(
                check(file, b"").unwrap_err(),
                Unusable::Empty {
                    file: file.to_string()
                }
            );
        }
    }

    // AYEAYE-56 — the JSON files get the same treatment, which is what stops a
    // 404 page being kept as a model's config and only failing at load.
    #[test]
    fn a_page_of_html_saved_as_a_json_file_is_refused_and_quoted_back() {
        for file in [CONFIG_FILE, TOKENIZER_FILE] {
            let refused = check(file, PAGE).unwrap_err();
            let Unusable::Malformed { why, .. } = &refused else {
                panic!("expected a malformed file, got {refused:?}");
            };
            // Quoted back, because "not JSON" about a file somebody cannot see
            // is not something they can act on. The first line of the page
            // usually says what the hub actually thought of the request.
            assert!(why.contains("DOCTYPE"), "{why}");
        }

        assert_eq!(check(CONFIG_FILE, br#"{"model_type":"whisper"}"#), Ok(()));
        // Leading whitespace, and a byte-order mark, are not a different file.
        assert_eq!(check(CONFIG_FILE, b"\n  {\"a\":1}"), Ok(()));
        assert_eq!(check(CONFIG_FILE, b"\xef\xbb\xbf{\"a\":1}"), Ok(()));
    }
}
