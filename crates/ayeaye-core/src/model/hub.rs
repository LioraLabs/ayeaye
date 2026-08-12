//! Where a model's files are fetched from, and in which order.
//!
//! Pure: this builds the URLs and says what has to arrive. Fetching them is the
//! shell's, and the transport is a `curl` subprocess rather than an in-process
//! HTTP client — see the ticket's `## Spec` amendment. In short: every rustls
//! edge puts `ring` and therefore `cc` in `Cargo.lock`, because a lockfile
//! carries optional dependencies whether or not a feature enables them, and the
//! constitution's rule 4 reads the lockfile. There is no feature combination
//! that gets round that, so the network stays where the effect budget already
//! says it belongs: outside the core, in the shell.

use super::id::ModelId;

/// The public model hub, when nothing says otherwise.
pub const DEFAULT_HOST: &str = "https://huggingface.co";

/// A file a model directory has to hold, in the order it is fetched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Wanted {
    /// The name it has in the repository and on disk.
    pub file: &'static str,
    /// Whether this is the small file whose contents decide the rest.
    pub decides: bool,
}

/// Everything a model directory needs, **`config.json` first**.
///
/// The order is the ticket. `config.json` is a couple of kilobytes and names
/// the architecture, so it is fetched, judged, and only then is anything large
/// asked for. Reordering this turns a refusal that costs nothing into one that
/// costs a download, which is the failure mode the allowlist exists to prevent.
pub const WANTED: &[Wanted] = &[
    Wanted {
        file: super::CONFIG_FILE,
        decides: true,
    },
    Wanted {
        file: super::TOKENIZER_FILE,
        decides: false,
    },
    Wanted {
        file: super::WEIGHTS_FILE,
        decides: false,
    },
];

/// The URL one of a model's files is fetched from.
///
/// `host` is an argument rather than a constant read from inside, so a test can
/// point the whole acquisition path at a directory on disk over `file://`
/// instead of at somebody else's server. A test that has to reach the real hub
/// to run is a test that does not run.
///
/// Trailing slashes on the host are absorbed: a host configured as
/// `https://example.test/` must not produce a URL with `//` in the middle of it.
///
/// `file` is a [`Wanted`] rather than a string, and the guarantee comes from
/// its `file` being `&'static str`: a name that arrived from the hub at runtime
/// cannot be one. `Wanted` is `Copy` with public fields, so it is that lifetime
/// doing the work and not the newtype. The id's segments are already
/// refused at [`ModelId::parse`] if they carry anything a path should not, so
/// every part of the URL below is something this crate has already judged.
pub fn url(host: &str, id: &ModelId, file: &Wanted) -> String {
    file_url(host, id, file.file)
}

/// The resolve URL for a filename already validated by the caller.
pub fn file_url(host: &str, id: &ModelId, file: &str) -> String {
    format!(
        "{}/{}/{}/resolve/{}/{}",
        host.trim_end_matches('/'),
        id.owner(),
        id.name(),
        id.revision(),
        file
    )
}

#[cfg(test)]
mod tests {
    use super::{DEFAULT_HOST, WANTED, url};
    use crate::model::id::ModelId;

    fn whisper() -> ModelId {
        ModelId::parse("openai/whisper-small.en").expect("a well-formed id")
    }

    /// The entry for a named file, so a test reads like the caller does.
    fn wanted(file: &str) -> &'static super::Wanted {
        WANTED
            .iter()
            .find(|wanted| wanted.file == file)
            .expect("the tests only ask for files a model needs")
    }

    // AYEAYE-56 — the hub's own URL shape, which is what `curl` is handed.
    #[test]
    fn a_file_is_fetched_from_the_hubs_resolve_path() {
        assert_eq!(
            url(DEFAULT_HOST, &whisper(), wanted("config.json")),
            "https://huggingface.co/openai/whisper-small.en/resolve/main/config.json"
        );

        let pinned = ModelId::parse("openai/whisper-small.en@a1b2c3d").expect("a pinned id");
        assert_eq!(
            url(DEFAULT_HOST, &pinned, wanted("model.safetensors")),
            "https://huggingface.co/openai/whisper-small.en/resolve/a1b2c3d/model.safetensors"
        );
    }

    // AYEAYE-56 — the host is an argument so a test can serve the files from a
    // directory, and a trailing slash on it must not produce `//` in the path.
    #[test]
    fn the_host_is_substitutable_and_a_trailing_slash_does_not_double_up() {
        assert_eq!(
            url("file:///tmp/hub/", &whisper(), wanted("config.json")),
            "file:///tmp/hub/openai/whisper-small.en/resolve/main/config.json"
        );
    }

    // AYEAYE-56 — the order is the ticket: the small file that names the
    // architecture is fetched first, so an unsupported model costs kilobytes
    // rather than a download. Exactly one file decides.
    #[test]
    fn the_file_that_decides_is_fetched_before_anything_large() {
        assert_eq!(WANTED[0].file, "config.json");
        assert!(WANTED[0].decides);
        assert_eq!(
            WANTED.iter().filter(|wanted| wanted.decides).count(),
            1,
            "one file decides, and everything after it is fetched on its say-so"
        );
        assert_eq!(
            WANTED.iter().map(|w| w.file).collect::<Vec<_>>(),
            vec!["config.json", "tokenizer.json", "model.safetensors"]
        );
    }
}
