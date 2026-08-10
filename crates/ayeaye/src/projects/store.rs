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
/// **Nothing in this binary calls this yet.** The daemon's call site is inside
/// spawn (`note_project_pick` in `bin/ayeaye`), and spawning is AYEAYE-51 —
/// which was written against a base where this function did not exist. Until
/// its handler calls this, the picker reads a history nothing writes and
/// "recent picks are recorded" holds in the tests and not on a running
/// machine. AYEAYE-71 is the wiring.
///
/// Best effort in every direction: a store that cannot be written costs
/// ranking quality and nothing else, so no failure here may reach the
/// response. The write is atomic, because a store half-replaced by a crash
/// would read as corrupt and throw the whole history away — and the daemon
/// beside us may be reading it at this moment.
pub fn note_pick(at: &Path, path: &str, now: f64) {
    let mut store = load(at);
    store.record(&key_for(path), now);

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

/// The store's key for a directory somebody names.
///
/// The daemon's `_recents_key` is `abspath(expanduser(path)).rstrip("/")`, and
/// every part of it is load-bearing while the two share one file: a pick
/// written under `~/src/thing` or under a relative path is a key the Python
/// daemon will never produce, so the history quietly splits in a file both are
/// writing. `recents::key` is the `rstrip` — pure, and the core's. Expanding a
/// `~` needs a home and resolving a relative path needs a working directory,
/// and both of those are here.
///
/// **What is not here.** `abspath` is `normpath(join(cwd, path))`, and the
/// `normpath` half is not implemented: `~/src/../other` and `./src` key
/// differently from the daemon. Nor is `expanduser` of *another* user's home,
/// which needs the password database — `~root/x` lands under the working
/// directory here, as it does in Python when the name cannot be looked up.
/// Both are bounded to a path somebody typed: the picker's own candidates
/// arrive absolute and already normal, straight from the walk.
pub fn key_for(path: &str) -> String {
    let expanded = match path.strip_prefix('~') {
        Some("") => home(),
        Some(rest) if rest.starts_with('/') => format!("{}{rest}", home()),
        _ => path.to_string(),
    };
    let absolute = if expanded.starts_with('/') {
        PathBuf::from(expanded)
    } else {
        std::env::current_dir().unwrap_or_default().join(expanded)
    };
    recents::key(&absolute.to_string_lossy())
}

/// The home directory, or the empty string when there is none.
///
/// With no `HOME` there is no right answer: `~` keys as the working directory
/// and `~/x` as `/x`. Neither is the daemon's answer either — `expanduser`
/// falls back to the password database, which this does not read — and a
/// daemon running without a `HOME` has worse problems than its picker's
/// history.
fn home() -> String {
    std::env::var("HOME").unwrap_or_default()
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

    // AYEAYE-50 — the daemon keys its store by
    // `abspath(expanduser(path)).rstrip("/")`, and while the two share one
    // file every part of that matters: a pick written under `~/src/thing` is a
    // key the Python daemon will never produce, so the history splits in a
    // file both of them are writing.
    #[test]
    fn a_key_is_expanded_and_absolute_before_it_is_written() {
        let tree = TempTree::named("store-key");
        let at = tree.path.join("projects.json");
        // SAFETY: HOME is read by `key_for` and by nothing else running here.
        unsafe { std::env::set_var("HOME", "/home/tester") };

        assert_eq!(super::key_for("~/src/thing"), "/home/tester/src/thing");
        assert_eq!(super::key_for("~"), "/home/tester");
        assert_eq!(super::key_for("~/"), "/home/tester");
        // Not a home reference: `~other` is somebody else's home, and reading
        // it needs a password database. Python's `expanduser` gives up on a
        // name it cannot find and hands the path to `abspath` unchanged, so it
        // ends up under the working directory — and so does this.
        assert!(super::key_for("~other/x").ends_with("/~other/x"));
        assert_eq!(super::key_for("/already/there/"), "/already/there");

        let relative = super::key_for("src/thing");
        assert!(relative.starts_with('/'), "{relative}");
        assert!(relative.ends_with("/src/thing"), "{relative}");

        // And the two halves of `abspath` that are *not* implemented, pinned
        // as divergences rather than left to be discovered: `normpath` is not
        // here, so a path somebody typed with a `..` in it keys differently
        // from the daemon. Bounded to typed paths — the walk's own candidates
        // arrive absolute and already normal.
        assert_eq!(
            super::key_for("~/src/../other"),
            "/home/tester/src/../other",
            "the daemon would say /home/tester/other"
        );

        // And the key that lands in the file is the resolved one.
        note_pick(&at, "~/src/thing", 5.0);
        assert!(
            load(&at).get("/home/tester/src/thing").is_some(),
            "{}",
            std::fs::read_to_string(&at).unwrap_or_default()
        );
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
