//! `ayeaye model`, run as the binary, against a hub made of files.
//!
//! **No network, and nothing large.** The hub host is configuration, so this
//! lays out a directory shaped exactly like the hub's `/resolve/` paths and
//! points the binary at it over `file://`. That runs the real `curl`, with the
//! real flags, writing real bytes through the real staging-and-rename — the
//! whole acquisition path — using three files totalling a few hundred bytes.
//!
//! The alternative would be downloading a model in a test, which is a 75 MB
//! download on somebody's machine and a third-party service being asked to
//! serve it every time the suite runs.

use std::path::{Path, PathBuf};
use std::process::Command;

/// A directory of this test's own, removed when it goes out of scope.
struct Scratch(PathBuf);

impl Scratch {
    fn named(what: &str) -> Scratch {
        let path = std::env::temp_dir().join(format!(
            "ayeaye-56-cli-{what}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("a scratch directory");
        Scratch(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A safetensors file of the smallest shape that is a real one: a
/// little-endian header length, then a header of exactly that length.
fn weights() -> Vec<u8> {
    let header = br#"{"__metadata__":{}}"#;
    let mut bytes = (header.len() as u64).to_le_bytes().to_vec();
    bytes.extend_from_slice(header);
    bytes
}

fn gguf(architecture: &str) -> Vec<u8> {
    let mut bytes = b"GGUF".to_vec();
    bytes.extend_from_slice(&3u32.to_le_bytes());
    bytes.extend_from_slice(&0u64.to_le_bytes());
    bytes.extend_from_slice(&1u64.to_le_bytes());
    for text in ["general.architecture", architecture] {
        bytes.extend_from_slice(&(text.len() as u64).to_le_bytes());
        bytes.extend_from_slice(text.as_bytes());
        if text == "general.architecture" {
            bytes.extend_from_slice(&8u32.to_le_bytes());
        }
    }
    bytes
}

/// Lay out one repository the way the hub serves it, and answer with the URL
/// that reaches it.
fn hub(at: &Path, repo: &str, config: &[u8]) -> String {
    let dir = at.join("hub").join(repo).join("resolve/main");
    std::fs::create_dir_all(&dir).expect("a hub of our own");
    std::fs::write(dir.join("config.json"), config).expect("its config");
    std::fs::write(dir.join("tokenizer.json"), br#"{"model":{"vocab":{}}}"#).expect("its vocab");
    std::fs::write(dir.join("model.safetensors"), weights()).expect("its weights");
    format!("file://{}", at.join("hub").display())
}

/// Run the binary with a home of its own.
///
/// Every path the binary reads hangs off `HOME` and the two XDG variables, so
/// pointing all three at the scratch directory is what keeps this out of the
/// state of whoever is running the suite. Nothing here talks to a daemon, a
/// bus, or a service manager — it fetches files and writes a directory.
fn ayeaye(scratch: &Path, hub: &str, args: &[&str]) -> (i32, String, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_ayeaye"))
        .args(args)
        .env("HOME", scratch)
        .env("XDG_STATE_HOME", scratch.join("state"))
        .env("XDG_CONFIG_HOME", scratch.join("config"))
        .env("AYEAYE_MODEL_HUB", hub)
        .env("AYEAYE_TOKEN", "test-token-not-a-real-secret")
        .env_remove("AYEAYE_SPEECH_MODEL")
        .env_remove("VOICE_REMOTE_SPEECH_MODEL")
        .env_remove("AYEAYE_CLEANUP_MODEL")
        .env_remove("VOICE_REMOTE_CLEANUP_MODEL")
        .output()
        .expect("the binary should be runnable");
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn listing_reports_roles_sizes_both_selections_and_unusable_models() {
    let scratch = Scratch::named("role-list");
    let store = scratch.0.join("state/ayeaye/models");
    let speech = store.join("openai/whisper-tiny.en/main");
    std::fs::create_dir_all(&speech).expect("speech model directory");
    std::fs::write(speech.join("config.json"), br#"{}"#).expect("speech config");
    std::fs::write(speech.join("tokenizer.json"), br#"{}"#).expect("speech tokenizer");
    std::fs::write(speech.join("model.safetensors"), weights()).expect("speech weights");

    let cleanup = store.join("local/cleanup/main");
    std::fs::create_dir_all(&cleanup).expect("cleanup model directory");
    let mut gguf = b"GGUF".to_vec();
    gguf.extend_from_slice(&3u32.to_le_bytes());
    gguf.extend_from_slice(&0u64.to_le_bytes());
    gguf.extend_from_slice(&0u64.to_le_bytes());
    std::fs::write(cleanup.join("model.gguf"), gguf).expect("cleanup weights");
    std::fs::write(cleanup.join("tokenizer.json"), br#"{}"#).expect("cleanup tokenizer");

    let unusable = store.join("local/broken/main");
    std::fs::create_dir_all(&unusable).expect("unusable model directory");
    std::fs::write(unusable.join("notes.txt"), b"not a model").expect("junk file");

    let config = scratch.0.join("config/ayeaye/env");
    std::fs::create_dir_all(config.parent().expect("config parent")).expect("config directory");
    std::fs::write(
        config,
        "AYEAYE_SPEECH_MODEL=openai/whisper-tiny.en\nAYEAYE_CLEANUP_MODEL=local/cleanup\n",
    )
    .expect("model selections");

    let (code, out, err) = ayeaye(&scratch.0, "file:///unused", &["model", "ls"]);
    assert_eq!(code, 0, "stdout {out:?} stderr {err:?}");
    assert!(out.contains("openai/whisper-tiny.en  speech"), "{out:?}");
    assert!(out.contains("local/cleanup  cleanup"), "{out:?}");
    assert_eq!(out.matches("(in use)").count(), 2, "{out:?}");
    assert!(out.contains("local/broken  unusable:"), "{out:?}");
    assert!(out.lines().all(|line| line.contains(" bytes")), "{out:?}");
}

#[test]
fn local_models_are_validated_imported_once_and_removed_uniformly() {
    let scratch = Scratch::named("add");
    let source = scratch.0.join("my-whisper");
    std::fs::create_dir_all(&source).expect("source directory");
    std::fs::write(
        source.join("config.json"),
        br#"{"architectures":["WhisperForConditionalGeneration"]}"#,
    )
    .expect("config");
    std::fs::write(source.join("tokenizer.json"), br#"{"model":{}}"#).expect("tokenizer");
    std::fs::write(source.join("model.safetensors"), weights()).expect("weights");

    let (code, first, err) = ayeaye(
        &scratch.0,
        "file:///unused",
        &["model", "add", source.to_str().expect("utf-8 path")],
    );
    assert_eq!(code, 0, "stdout {first:?} stderr {err:?}");
    let id = first.split_whitespace().next().expect("the imported id");
    assert!(id.starts_with("local/my-whisper@"), "{first:?}");
    let imported = scratch.0.join("state/ayeaye/models").join(
        ayeaye_core::model::ModelId::parse(id)
            .expect("a model id")
            .relative_dir()
            .strip_prefix("models")
            .expect("a relative model path"),
    );
    assert_eq!(
        std::fs::metadata(source.join("model.safetensors"))
            .expect("source metadata")
            .len(),
        std::fs::metadata(imported.join("model.safetensors"))
            .expect("import metadata")
            .len()
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        assert_eq!(
            std::fs::metadata(source.join("model.safetensors"))
                .expect("source metadata")
                .ino(),
            std::fs::metadata(imported.join("model.safetensors"))
                .expect("import metadata")
                .ino(),
            "same-filesystem imports should hardlink"
        );
    }

    let (code, second, err) = ayeaye(
        &scratch.0,
        "file:///unused",
        &["model", "add", source.to_str().expect("utf-8 path")],
    );
    assert_eq!(code, 0, "{err:?}");
    assert!(
        second.contains(id) && second.contains("already"),
        "{second:?}"
    );

    let (code, listed, err) = ayeaye(&scratch.0, "file:///unused", &["model", "ls"]);
    assert_eq!(code, 0, "{err:?}");
    assert!(listed.contains(&format!("{id}  speech")), "{listed:?}");

    let (code, _, err) = ayeaye(&scratch.0, "file:///unused", &["model", "use", id]);
    assert_eq!(code, 0, "{err:?}");
    let (_, listed, _) = ayeaye(&scratch.0, "file:///unused", &["model", "ls"]);
    assert!(listed.contains(&format!("{id}  speech")) && listed.contains("(in use)"));

    let (code, removed, err) = ayeaye(&scratch.0, "file:///unused", &["model", "rm", id]);
    assert_eq!(code, 0, "{err:?}");
    assert!(removed.contains("removed"), "{removed:?}");
    assert!(!imported.exists());
}

#[test]
fn cleanup_import_names_missing_companions_and_unsupported_architectures() {
    let scratch = Scratch::named("add-cleanup");
    let source = scratch.0.join("cleanup.gguf");
    std::fs::write(&source, gguf("llama")).expect("gguf");

    let (code, _, missing) = ayeaye(
        &scratch.0,
        "file:///unused",
        &["model", "add", source.to_str().expect("utf-8 path")],
    );
    assert_eq!(code, 1);
    assert!(missing.contains("tokenizer.json"), "{missing:?}");

    std::fs::write(source.with_file_name("tokenizer.json"), br#"{"model":{}}"#).expect("tokenizer");
    std::fs::write(&source, gguf("mamba")).expect("unsupported gguf");
    let (code, _, unsupported) = ayeaye(
        &scratch.0,
        "file:///unused",
        &["model", "add", source.to_str().expect("utf-8 path")],
    );
    assert_eq!(code, 1);
    assert!(unsupported.contains("mamba"), "{unsupported:?}");

    std::fs::write(&source, gguf("llama")).expect("supported gguf");
    let (code, out, err) = ayeaye(
        &scratch.0,
        "file:///unused",
        &["model", "add", source.to_str().expect("utf-8 path")],
    );
    assert_eq!(code, 0, "stdout {out:?} stderr {err:?}");
    let id = out.split_whitespace().next().expect("the imported id");
    let (_, listed, _) = ayeaye(&scratch.0, "file:///unused", &["model", "ls"]);
    assert!(listed.contains(&format!("{id}  cleanup")), "{listed:?}");
}

// AYEAYE-56 — the whole acquisition path, end to end, through the real
// transport: a model is fetched, verified, stored under the state directory,
// listed, chosen, and removed.
#[test]
fn a_model_is_pulled_into_the_state_directory_and_can_then_be_chosen() {
    let scratch = Scratch::named("pull");
    let hub = hub(
        &scratch.0,
        "openai/whisper-tiny.en",
        br#"{"architectures": ["WhisperForConditionalGeneration"], "model_type": "whisper"}"#,
    );

    let (code, out, err) = ayeaye(
        &scratch.0,
        &hub,
        &["model", "pull", "openai/whisper-tiny.en"],
    );
    assert_eq!(code, 0, "stdout {out:?} stderr {err:?}");
    assert!(out.contains("WhisperForConditionalGeneration"), "{out:?}");

    let dir = scratch
        .0
        .join("state/ayeaye/models/openai/whisper-tiny.en/main");
    for file in ["config.json", "tokenizer.json", "model.safetensors"] {
        assert!(dir.join(file).is_file(), "{file} did not land: {out:?}");
    }
    // The staging directory is gone: a pull leaves the store holding models and
    // nothing else.
    let models = scratch.0.join("state/ayeaye/models");
    let strays: Vec<_> = std::fs::read_dir(&models)
        .expect("the store")
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with('.'))
        .collect();
    assert!(strays.is_empty(), "staging was left behind: {strays:?}");

    let (code, listed, _) = ayeaye(&scratch.0, &hub, &["model", "ls"]);
    assert_eq!(code, 0);
    assert!(listed.contains("openai/whisper-tiny.en"), "{listed:?}");
    assert!(
        !listed.contains("(in use)"),
        "nothing has been chosen yet: {listed:?}"
    );

    let (code, chosen, err) = ayeaye(
        &scratch.0,
        &hub,
        &["model", "use", "openai/whisper-tiny.en"],
    );
    assert_eq!(code, 0, "{err:?}");
    assert!(chosen.contains("transcribe"), "{chosen:?}");

    // The choice is in the file the service unit already reads, which is what
    // makes it configuration rather than a flag somebody has to remember.
    let written = std::fs::read_to_string(scratch.0.join("config/ayeaye/env"))
        .expect("the configuration file should have been written");
    assert!(
        written.contains("AYEAYE_SPEECH_MODEL=openai/whisper-tiny.en"),
        "{written:?}"
    );

    let (_, listed, _) = ayeaye(&scratch.0, &hub, &["model", "ls"]);
    assert!(listed.contains("(in use)"), "{listed:?}");

    let (code, removed, _) = ayeaye(&scratch.0, &hub, &["model", "rm", "openai/whisper-tiny.en"]);
    assert_eq!(code, 0);
    assert!(removed.contains("removed"), "{removed:?}");
    assert!(!dir.exists());
}

// AYEAYE-56 — the bound, proved where it counts: through the real binary and
// the real transport, an unsupported architecture is refused and the weights
// beside it are never fetched. The unit test watches the URLs; this watches
// the disk, which is the thing a user would actually notice.
#[test]
fn an_unsupported_model_is_refused_without_the_weights_being_fetched() {
    let scratch = Scratch::named("unsupported");
    let hub = hub(
        &scratch.0,
        "meta-llama/Llama-3.2-1B",
        br#"{"architectures": ["LlamaForCausalLM"], "model_type": "llama"}"#,
    );

    let (code, out, err) = ayeaye(
        &scratch.0,
        &hub,
        &["model", "pull", "meta-llama/Llama-3.2-1B"],
    );

    assert_eq!(code, 1, "stdout {out:?}");
    assert!(err.contains("LlamaForCausalLM"), "{err:?}");
    assert!(
        err.contains("WhisperForConditionalGeneration"),
        "the refusal has to say what this build does run: {err:?}"
    );
    // Nothing was kept, and in particular no weights were fetched.
    let models = scratch.0.join("state/ayeaye/models");
    let kept: Vec<PathBuf> = walk(&models);
    assert!(
        kept.iter().all(|path| !path.ends_with("model.safetensors")),
        "weights were fetched for a model this build cannot run: {kept:?}"
    );
    assert!(
        !scratch
            .0
            .join("state/ayeaye/models/meta-llama/Llama-3.2-1B/main")
            .exists(),
        "a model that was refused must not be left on disk"
    );
}

// AYEAYE-56 — a repository id that is not one is refused before anything is
// fetched, and the message says what one looks like.
#[test]
fn an_id_that_is_not_one_is_refused_with_an_example() {
    let scratch = Scratch::named("badid");
    let (code, _, err) = ayeaye(&scratch.0, "file:///nowhere", &["model", "pull", "whisper"]);
    assert_eq!(code, 1);
    assert!(err.contains("owner/name"), "{err:?}");

    let (code, _, err) = ayeaye(
        &scratch.0,
        "file:///nowhere",
        &["model", "pull", "../../etc"],
    );
    assert_eq!(code, 1);
    assert!(
        !err.is_empty(),
        "a traversal has to be refused, not attempted"
    );
}

// AYEAYE-56 — the two crates spell the model's filenames separately, because
// the pure core may not depend on the crate above it. If they ever disagree, a
// pull writes files a load cannot find and nothing else would say so. This is
// the only place both names are in scope at once.
#[test]
fn the_names_acquisition_writes_are_the_names_inference_reads() {
    assert_eq!(
        ayeaye_core::model::CONFIG_FILE,
        ayeaye_infer::speech::model::CONFIG_FILE
    );
    assert_eq!(
        ayeaye_core::model::TOKENIZER_FILE,
        ayeaye_infer::speech::model::TOKENIZER_FILE
    );
    assert_eq!(
        ayeaye_core::model::WEIGHTS_FILE,
        ayeaye_infer::speech::model::WEIGHTS_FILE
    );
}

/// Every file under a directory, or nothing if it is not there.
fn walk(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let Ok(entries) = std::fs::read_dir(root) else {
        return found;
    };
    for entry in entries.flatten() {
        if entry.path().is_dir() {
            found.extend(walk(&entry.path()));
        } else {
            found.push(entry.path());
        }
    }
    found
}
