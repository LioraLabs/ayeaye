//! The pick store, on disk.
//!
//! Two effects, and nothing else: read the file, and rewrite it. Every
//! decision about what is in it — what a row means, what to do with one that
//! is not a row, how strong a pick is now — is `ayeaye_core::projects::recents`.

use std::path::{Path, PathBuf};

use ayeaye_core::projects::recents::{self, Recents};

/// What the file is called, inside the daemon's state directory.
pub const FILE: &str = "projects.json";

/// Read the store.
///
/// Anything wrong with it — absent, unreadable, a directory where the file
/// should be — is no ranking signal rather than an error. This is on the
/// request path, and a picker without history is still a working picker.
pub fn load(at: &Path) -> Recents {
    std::fs::read_to_string(at)
        .map(|text| Recents::parse(&text))
        .unwrap_or_default()
}

/// Record that an agent was started here.
///
/// Best effort in every direction: a store that cannot be written costs
/// ranking quality and nothing else, so no failure here may reach the
/// response. The write is atomic, because a store half-replaced by a crash
/// would read as corrupt and throw the whole history away — and the daemon
/// beside us may be reading it at this moment.
pub fn note_pick(at: &Path, path: &str, now: f64) {
    let mut store = load(at);
    store.record(&recents::key(path), now);

    if let Some(parent) = at.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    // Named for this process, so two daemons rewriting the store at once
    // cannot each half-write the other's temporary file.
    let temporary: PathBuf = at.with_extension(format!("tmp{}", std::process::id()));
    if std::fs::write(&temporary, store.render()).is_ok()
        && std::fs::rename(&temporary, at).is_err()
    {
        let _ = std::fs::remove_file(&temporary);
    }
}

#[cfg(test)]
mod tests {
    use super::{Recents, load, note_pick};
    use crate::projects::TempTree;

    // AYEAYE-50 — "recent picks are recorded". The file is the one the Python
    // daemon reads while both are running, so what lands on disk has to be the
    // shape it reads — and reading it back through the same door is what says
    // the two halves agree.
    #[test]
    fn a_pick_is_written_where_the_python_daemon_reads_it() {
        let tree = TempTree::named("store");
        let at = tree.path.join("state").join("projects.json");

        note_pick(&at, "/home/a/src/ayeaye", 1_700_000_000.0);
        let written = std::fs::read_to_string(&at).expect("the store exists");
        assert_eq!(
            written,
            r#"{"version":1,"picks":{"/home/a/src/ayeaye":{"n":1,"t":1700000000}}}"#
        );

        note_pick(&at, "/home/a/src/ayeaye", 1_700_000_100.0);
        assert_eq!(
            load(&at).get("/home/a/src/ayeaye").map(|pick| pick.count),
            Some(2),
            "a second pick is a bump, not a second row"
        );

        // A trailing slash is the same directory, or the history quietly
        // stops counting the moment something spells it differently.
        note_pick(&at, "/home/a/src/ayeaye/", 1_700_000_200.0);
        assert_eq!(load(&at).len(), 1);
    }

    // AYEAYE-50 — best effort in every direction: a store that cannot be read
    // or written costs ranking quality and nothing else, and none of it may
    // reach the response.
    #[test]
    fn a_store_that_cannot_be_used_costs_ranking_and_nothing_else() {
        let tree = TempTree::named("store-broken");
        let missing = tree.path.join("nowhere").join("projects.json");
        assert_eq!(load(&missing), Recents::default());

        // A directory where the file should be: unreadable, and unwritable.
        let blocked = tree.path.join("blocked");
        std::fs::create_dir_all(&blocked).expect("a directory");
        assert_eq!(load(&blocked), Recents::default());
        note_pick(&blocked, "/home/a/src", 1.0);

        // Nonsense on disk is no history rather than an error, and the next
        // pick replaces it.
        let at = tree.path.join("nonsense.json");
        std::fs::write(&at, "{ truncated").expect("a file");
        assert_eq!(load(&at), Recents::default());
        note_pick(&at, "/home/a/src", 5.0);
        assert_eq!(load(&at).len(), 1);
    }
}
