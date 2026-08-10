//! The project picker's effects.
//!
//! Every decision the picker makes is in `ayeaye_core::projects`; what is
//! here is the walking, the clock, the store on disk and the one search in
//! flight. The split is the point: the ranking rungs and the bounds are
//! testable without a machine, and this crate is where the machine is.

pub mod search;
pub mod store;
pub mod walk;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use ayeaye_core::projects::rank::{self, Candidate, Row};
use ayeaye_core::projects::{session, skip};

use search::{Bounds, Outcome, Searches};
use walk::{Cached, FsLister};

/// How many candidates the picker shows when nobody says otherwise.
pub const DEFAULT_LIMIT: usize = 40;

/// What the picker was told to be.
///
/// Every name here is one the Python daemon already reads, so a machine
/// configured for it is configured for this.
#[derive(Debug, Clone)]
pub struct Settings {
    /// Where to look. A root is scanned whatever it is called, which is the
    /// escape hatch for a directory the skip list would otherwise hide.
    pub roots: Vec<PathBuf>,
    /// How far below a root a project may sit.
    pub depth: usize,
    /// Wall clock one walk may spend.
    pub budget: Duration,
    /// Wall clock a request may wait for a walk.
    pub wait: Duration,
    /// How long a finished search and a directory listing stay trusted.
    pub ttl: Duration,
    /// Names this machine adds to the skip list.
    pub skip_extra: Vec<String>,
    /// The home directory, for shortening what the picker shows.
    pub home: String,
    /// The pick store.
    pub store: PathBuf,
}

impl Settings {
    /// Resolve from the environment, a home and a state directory.
    ///
    /// `env` is a lookup rather than `std::env` so every default is a decision
    /// a test can drive, and it is handed the bare name — `PROJECT_DEPTH` —
    /// exactly as [`crate::config::env_var`] expects it.
    pub fn resolve(env: impl Fn(&str) -> Option<String>, home: &str, state_dir: &Path) -> Settings {
        let roots = env("PROJECT_ROOTS").unwrap_or_else(|| "~".to_string());
        Settings {
            roots: roots
                .split(':')
                .map(str::trim)
                .filter(|root| !root.is_empty())
                .map(|root| expand(root, home))
                .collect(),
            depth: number(env("PROJECT_DEPTH"), 6.0) as usize,
            budget: seconds(env("PROJECT_BUDGET"), 4.0),
            wait: seconds(env("PROJECT_WAIT"), 0.4),
            ttl: seconds(env("PROJECT_TTL"), 60.0),
            skip_extra: skip::extra_names(&env("PROJECT_SKIP").unwrap_or_default()),
            home: home.to_string(),
            store: state_dir.join(store::FILE),
        }
    }

    /// Resolve from the process environment.
    pub fn from_env() -> Settings {
        let home = std::env::var("HOME").unwrap_or_default();
        // Beside the token the daemon already wrote, which is the same
        // directory `bin/ayeaye` keeps `projects.json` in.
        let state = crate::config::state_dir().unwrap_or_else(|| PathBuf::from("."));
        Settings::resolve(crate::config::env_var, &home, &state)
    }
}

/// A leading `~` made into the home directory, as the daemon expands it.
fn expand(root: &str, home: &str) -> PathBuf {
    match root.strip_prefix('~') {
        Some("") => PathBuf::from(home),
        Some(rest) if rest.starts_with('/') => PathBuf::from(format!("{home}{rest}")),
        _ => PathBuf::from(root),
    }
}

fn number(value: Option<String>, default: f64) -> f64 {
    value
        .and_then(|value| value.trim().parse().ok())
        .filter(|number: &f64| number.is_finite() && *number >= 0.0)
        .unwrap_or(default)
}

fn seconds(value: Option<String>, default: f64) -> Duration {
    Duration::from_secs_f64(number(value, default))
}

/// The picker.
pub struct Picker {
    settings: Settings,
    searches: Searches,
}

impl Picker {
    /// A picker over the real filesystem.
    pub fn new(settings: Settings) -> Picker {
        let lister = Cached::new(
            FsLister {
                skip_extra: settings.skip_extra.clone(),
                ..FsLister::default()
            },
            settings.ttl,
        );
        let searches = Searches::new(
            Arc::new(lister),
            Bounds {
                // Only the roots that are there: the walk takes its roots as
                // absolute and existing, and a root that is neither yields
                // candidate paths that match no pick and shorten to nothing.
                roots: settings
                    .roots
                    .iter()
                    .filter(|root| root.is_dir())
                    .cloned()
                    .collect(),
                depth: settings.depth,
                budget: settings.budget,
                wait: settings.wait,
                ttl: settings.ttl,
            },
        );
        Picker { settings, searches }
    }

    /// The picker's answer, as the page reads it.
    pub async fn body(&self, query: &str, limit: usize) -> String {
        // Ranking wants more candidates than it returns, because the first
        // `limit` directories a breadth-first walk meets are not the best
        // `limit`. Still an early stop, just not a myopic one.
        let cap = (limit * 8).max(200);
        match self.searches.search(query, cap).await {
            Outcome::Superseded => rank::SUPERSEDED.to_string(),
            Outcome::Found(found) => self.rows(found, query, limit).await,
        }
    }

    /// Rank what was found and shape it into what the picker renders.
    ///
    /// On a blocking thread: this reads the pick store, stats the handful of
    /// recent picks the walk could not reach, reads a `.tmux.yaml` per row and
    /// asks tmux what is running. None of that is much, and all of it would be
    /// in front of the pane polls and the transcript stream on the same
    /// connection if it ran here.
    async fn rows(&self, found: Vec<Candidate>, query: &str, limit: usize) -> String {
        let settings = self.settings.clone();
        let query = query.to_string();
        tokio::task::spawn_blocking(move || {
            assemble(&settings, found, &query, limit, &live_sessions())
        })
        .await
        .unwrap_or_else(|_| rank::body(&[]))
    }
}

/// Everything from "here are the candidates" to "here is the body".
///
/// `live` is an argument rather than something asked of tmux inside, because
/// which sessions are running is the one input a test cannot arrange: a
/// machine that happens to have a session named after the fixture would
/// otherwise decide the answer.
fn assemble(
    settings: &Settings,
    found: Vec<Candidate>,
    query: &str,
    limit: usize,
    live: &[String],
) -> String {
    let recents = store::load(&settings.store);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_secs_f64())
        .unwrap_or(0.0);

    let mut candidates = found;
    let already: Vec<String> = candidates
        .iter()
        .map(|candidate| candidate.path.clone())
        .collect();
    // Somewhere you have started an agent is a candidate even when the walk
    // cannot reach it. Each one costs two stats here, which is why the core
    // offers only the strongest handful.
    for path in rank::recent_candidates(&recents, now, query, &already) {
        let at = Path::new(path);
        if !at.is_dir() {
            continue;
        }
        candidates.push(Candidate {
            path: path.to_string(),
            is_git: at.join(".git").exists(),
            level: 0,
        });
    }

    let rows: Vec<Row> = rank::order(&candidates, &recents, now, query, limit)
        .into_iter()
        .map(|candidate| {
            let yaml = std::fs::read_to_string(Path::new(&candidate.path).join(".tmux.yaml")).ok();
            let name = session::name_for(&candidate.path, yaml.as_deref());
            Row {
                short: rank::shorten(&candidate.path, &settings.home),
                dir: candidate.path,
                exists: live.contains(&name),
                session: name,
                git: candidate.is_git,
            }
        })
        .collect();
    rank::body(&rows)
}

/// The tmux sessions running right now.
///
/// A provisional home. AYEAYE-43 is building the tmux layer this belongs in,
/// and it is three lines here rather than a dependency on a branch that has
/// not landed; the picker needs it to say whether choosing a project attaches
/// to a session or starts one. Fold it into that layer at integration.
fn live_sessions() -> Vec<String> {
    std::process::Command::new("tmux")
        .args(["list-sessions", "-F", "#{session_name}"])
        .output()
        .ok()
        .filter(|asked| asked.status.success())
        .map(|asked| {
            String::from_utf8_lossy(&asked.stdout)
                .split_whitespace()
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// A real directory tree, for the tests that need one.
///
/// The walk's rules — the skip list, a symlink not followed, `.git` as a file
/// — are claims about a filesystem, and the only way to test a claim about a
/// filesystem is against one. Everything else in this module is driven through
/// an in-memory lister instead.
#[cfg(test)]
pub(crate) struct TempTree {
    /// The root, which this owns and removes.
    pub path: std::path::PathBuf,
}

#[cfg(test)]
impl TempTree {
    /// A directory of this run's own, under the system temporary directory.
    pub fn named(label: &str) -> TempTree {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "ayeaye-50-{label}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("a temporary directory");
        TempTree { path }
    }

    /// Make a directory, and everything above it.
    pub fn dir(&self, relative: &str) {
        std::fs::create_dir_all(self.path.join(relative)).expect("a directory");
    }

    /// Write a file, making everything above it.
    pub fn file(&self, relative: &str, contents: &str) {
        let at = self.path.join(relative);
        if let Some(parent) = at.parent() {
            std::fs::create_dir_all(parent).expect("a parent");
        }
        std::fs::write(at, contents).expect("a file");
    }

    /// Point `link` at `target`, both relative to the root.
    pub fn link(&self, target: &str, link: &str) {
        std::os::unix::fs::symlink(self.path.join(target), self.path.join(link))
            .expect("a symlink");
    }
}

#[cfg(test)]
impl Drop for TempTree {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::{Candidate, Settings, TempTree, assemble, expand};
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    fn no_env(_: &str) -> Option<String> {
        None
    }

    fn settings(tree: &TempTree) -> Settings {
        Settings::resolve(
            no_env,
            &tree.path.to_string_lossy(),
            &tree.path.join("state"),
        )
    }

    fn candidate(path: &Path, is_git: bool) -> Candidate {
        Candidate {
            path: path.to_string_lossy().into_owned(),
            is_git,
            level: 1,
        }
    }

    // AYEAYE-50 — the daemon's own defaults, so a machine configured for it is
    // configured for this, and `~` expanded because a root that is not
    // absolute yields candidate paths that match no pick and shorten to
    // nothing.
    #[test]
    fn the_defaults_are_the_ones_the_daemon_carries() {
        let plain = Settings::resolve(no_env, "/home/tester", Path::new("/state"));
        assert_eq!(plain.roots, vec![PathBuf::from("/home/tester")]);
        assert_eq!(plain.depth, 6);
        assert_eq!(plain.budget, Duration::from_secs(4));
        assert_eq!(plain.wait, Duration::from_millis(400));
        assert_eq!(plain.ttl, Duration::from_secs(60));
        assert_eq!(plain.store, PathBuf::from("/state/projects.json"));

        let configured = Settings::resolve(
            |name| {
                Some(
                    match name {
                        "PROJECT_ROOTS" => "~/src:~:/opt/things: ",
                        "PROJECT_DEPTH" => "3",
                        "PROJECT_BUDGET" => "1.5",
                        "PROJECT_WAIT" => "0.05",
                        "PROJECT_TTL" => "10",
                        _ => "scratch, archive",
                    }
                    .to_string(),
                )
            },
            "/home/tester",
            Path::new("/state"),
        );
        assert_eq!(
            configured.roots,
            vec![
                PathBuf::from("/home/tester/src"),
                PathBuf::from("/home/tester"),
                PathBuf::from("/opt/things"),
            ]
        );
        assert_eq!(configured.depth, 3);
        assert_eq!(configured.budget, Duration::from_millis(1500));
        assert_eq!(configured.wait, Duration::from_millis(50));
        assert_eq!(configured.skip_extra, vec!["scratch", "archive"]);

        // Nonsense falls back to the default rather than to zero, because a
        // depth of zero is a picker that finds nothing at all.
        let nonsense = Settings::resolve(
            |name| (name == "PROJECT_DEPTH").then(|| "deep".to_string()),
            "/home/tester",
            Path::new("/state"),
        );
        assert_eq!(nonsense.depth, 6);
        assert_eq!(expand("~", ""), PathBuf::from(""));
    }

    // AYEAYE-50 — the field the page draws an arrow next to. Which sessions
    // are running is an argument, because a machine that happens to have a
    // session named after the fixture would otherwise decide the answer — and
    // this machine does.
    #[test]
    fn a_project_whose_session_is_running_says_so() {
        let tree = TempTree::named("assemble");
        tree.dir("ayeaye");
        tree.dir("other");
        let settings = settings(&tree);
        let found = vec![
            candidate(&tree.path.join("ayeaye"), true),
            candidate(&tree.path.join("other"), true),
        ];

        let running = ["ayeaye".to_string()];
        let body = assemble(&settings, found.clone(), "", 10, &running);
        let read = ayeaye_core::json::parse(&body).expect("a body");
        let ayeaye_core::json::Value::Array(rows) = read.get("projects").expect("rows").clone()
        else {
            panic!("not an array: {body}");
        };
        let exists: Vec<bool> = rows
            .iter()
            .map(|row| row.get("exists") == Some(&ayeaye_core::json::Value::Bool(true)))
            .collect();
        assert_eq!(exists, vec![true, false], "{body}");

        // And with nothing running, nothing is attached to.
        let quiet = assemble(&settings, found, "", 10, &[]);
        assert!(!quiet.contains("\"exists\":true"), "{quiet}");
    }

    // AYEAYE-50 — "recent picks influence ranking", through the store on disk
    // and out to the wire: the directory you have started an agent in comes
    // first, whatever the alphabet says.
    #[test]
    fn a_pick_recorded_on_disk_reaches_the_answer() {
        let tree = TempTree::named("assemble-recents");
        tree.dir("aaa");
        tree.dir("zzz");
        let settings = settings(&tree);
        let found = vec![
            candidate(&tree.path.join("aaa"), true),
            candidate(&tree.path.join("zzz"), true),
        ];

        let alphabetical = assemble(&settings, found.clone(), "", 10, &[]);
        assert!(
            alphabetical.find("/aaa").unwrap() < alphabetical.find("/zzz").unwrap(),
            "{alphabetical}"
        );

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("a clock")
            .as_secs_f64();
        super::store::note_pick(
            &settings.store,
            &tree.path.join("zzz").to_string_lossy(),
            now,
        );
        let ranked = assemble(&settings, found, "", 10, &[]);
        assert!(
            ranked.find("/zzz").unwrap() < ranked.find("/aaa").unwrap(),
            "the pick history outranks the alphabet: {ranked}"
        );
    }
}
