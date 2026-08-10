//! The bounded walk.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ayeaye_core::projects::rank::Candidate;
use ayeaye_core::projects::skip;

/// What ended the walk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stopped {
    /// Nothing did: the tree ran out.
    Exhausted,
    /// The depth bound.
    Depth,
    /// Enough candidates.
    Limit,
    /// The wall clock.
    Deadline,
    /// A newer query took this walk over.
    Cancel,
}

/// One directory listing: a sub-directory, and whether it holds a `.git`.
pub type Listing = Vec<(PathBuf, bool)>;

/// Where the filesystem enters the walk.
pub trait Lister: Send + Sync {
    /// The interesting sub-directories of `path`, and whether each holds a
    /// `.git`.
    fn list(&self, path: &Path) -> Listing;
}

/// Walk `roots`, breadth first, handing each candidate to `found`.
///
/// Breadth first because a project sits a couple of levels below home, so the
/// answers worth having arrive before any bound can bite — which is what lets
/// a request answer from a walk that is still running.
///
/// The bound that ended it comes back, and callers care which: a walk that
/// finished is worth caching and one that was cut short is not.
pub fn walk(
    lister: &dyn Lister,
    roots: &[PathBuf],
    depth: usize,
    limit: usize,
    stop: &dyn Fn() -> Option<Stopped>,
    found: &mut dyn FnMut(Candidate),
) -> Stopped {
    let mut frontier: Vec<PathBuf> = roots.to_vec();
    // A directory reached twice is walked once. A symlink the lister did
    // follow, or two roots that overlap, would otherwise be a walk with no
    // end to it.
    let mut seen: HashSet<PathBuf> = frontier.iter().cloned().collect();
    let mut level = 0;
    let mut count = 0;
    while !frontier.is_empty() {
        if level >= depth {
            return Stopped::Depth;
        }
        let mut next = Vec::new();
        for path in &frontier {
            // Before each listing, which is the only place the walk can be
            // interrupted at all: the case this cannot cover — one directory
            // holding a hundred thousand children — is bounded inside the
            // lister instead.
            if let Some(bound) = stop() {
                return bound;
            }
            for (child, is_git) in lister.list(path) {
                if !seen.insert(child.clone()) {
                    continue;
                }
                found(Candidate {
                    path: child.to_string_lossy().into_owned(),
                    is_git,
                    level: level + 1,
                });
                count += 1;
                if limit > 0 && count >= limit {
                    return Stopped::Limit;
                }
                next.push(child);
            }
        }
        frontier = next;
        level += 1;
    }
    Stopped::Exhausted
}

/// How many entries are examined in any one directory.
///
/// Every other bound is checked *between* directories, which is no help at all
/// inside a single one holding a hundred thousand children — a package cache,
/// a maildir, a downloads folder nobody ever emptied. Truncating here is what
/// keeps the clock and the cancel flag reachable.
pub const PER_DIR_MAX: usize = 5000;

/// How many directory listings are remembered.
pub const LIST_MAX: usize = 4000;

/// The bound-checker the daemon runs with.
///
/// Cancel is asked first: a walk the user has already typed past must stop
/// costing syscalls whether or not its clock has run out, and answering
/// "deadline" for a walk that was superseded would have the caller cache a
/// result nobody asked for.
///
/// A closure rather than two arguments to [`walk`], so the walk has one seam
/// for "should I stop" and a test can fire either bound without a clock.
pub fn stopper(deadline: Instant, cancel: Arc<AtomicBool>) -> impl Fn() -> Option<Stopped> {
    move || {
        if cancel.load(Ordering::SeqCst) {
            return Some(Stopped::Cancel);
        }
        (Instant::now() >= deadline).then_some(Stopped::Deadline)
    }
}

/// The lister that reads the real filesystem.
pub struct FsLister {
    /// Names this machine adds to the skip list.
    pub skip_extra: Vec<String>,
}

impl Lister for FsLister {
    /// The sub-directories worth walking, and whether each is a repository.
    ///
    /// A symlinked directory is skipped outright: a symlink is either a second
    /// name for something the walk will reach anyway, or a way out of the
    /// bounded region entirely, and following one is how a walk finds a cycle.
    ///
    /// A directory that cannot be read is no directories, not an error. This
    /// runs over somebody's whole home directory, where a handful of
    /// unreadable ones is ordinary.
    fn list(&self, path: &Path) -> Listing {
        let Ok(entries) = std::fs::read_dir(path) else {
            return Vec::new();
        };
        let mut rows = Vec::new();
        for entry in entries.take(PER_DIR_MAX) {
            let Ok(entry) = entry else { continue };
            let name = entry.file_name();
            if skip::is_skipped(&name.to_string_lossy(), &self.skip_extra) {
                continue;
            }
            // `file_type` does not follow the link, which is what makes this
            // both the directory test and the symlink test at once.
            if !entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                continue;
            }
            let child = entry.path();
            // A worktree and a submodule keep `.git` as a *file* rather than a
            // directory, and both are somewhere you would start an agent.
            let is_git = child.join(".git").exists();
            rows.push((child, is_git));
        }
        rows
    }
}

/// A lister that remembers what it was told, for a while.
pub struct Cached<L> {
    inner: L,
    ttl: Duration,
    entries: Mutex<HashMap<PathBuf, (Instant, Listing)>>,
}

impl<L: Lister> Cached<L> {
    /// Wrap a lister.
    pub fn new(inner: L, ttl: Duration) -> Cached<L> {
        Cached {
            inner,
            ttl,
            entries: Mutex::new(HashMap::new()),
        }
    }

    /// List, from memory if it was listed recently enough.
    ///
    /// `now` is an argument so the expiry is a test rather than a sleep. The
    /// memo is bounded by clearing it wholesale: an eviction policy would be
    /// more code than the walk it serves, and refilling costs one bounded
    /// walk.
    pub fn list_at(&self, path: &Path, now: Instant) -> Listing {
        if let Ok(entries) = self.entries.lock()
            && let Some((at, rows)) = entries.get(path)
            && now.duration_since(*at) < self.ttl
        {
            return rows.clone();
        }
        let rows = self.inner.list(path);
        if let Ok(mut entries) = self.entries.lock() {
            if entries.len() >= LIST_MAX {
                entries.clear();
            }
            entries.insert(path.to_path_buf(), (now, rows.clone()));
        }
        rows
    }
}

impl<L: Lister> Lister for Cached<L> {
    fn list(&self, path: &Path) -> Listing {
        self.list_at(path, Instant::now())
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Arc, AtomicBool, Cached, Candidate, Duration, FsLister, Instant, Lister, Listing, Ordering,
        Path, PathBuf, Stopped, stopper, walk,
    };
    use crate::projects::TempTree;
    use std::collections::HashMap;

    /// A tree with no filesystem under it.
    struct Tree {
        children: HashMap<String, Vec<(String, bool)>>,
    }

    impl Tree {
        fn of(rows: &[(&str, &[(&str, bool)])]) -> Tree {
            Tree {
                children: rows
                    .iter()
                    .map(|(parent, kids)| {
                        (
                            parent.to_string(),
                            kids.iter()
                                .map(|(name, git)| (name.to_string(), *git))
                                .collect(),
                        )
                    })
                    .collect(),
            }
        }
    }

    impl Lister for Tree {
        fn list(&self, path: &Path) -> Listing {
            self.children
                .get(path.to_str().unwrap_or_default())
                .map(|kids| {
                    kids.iter()
                        .map(|(child, git)| (PathBuf::from(child), *git))
                        .collect()
                })
                .unwrap_or_default()
        }
    }

    fn never() -> Option<Stopped> {
        None
    }

    fn collect(
        tree: &Tree,
        roots: &[&str],
        depth: usize,
        limit: usize,
        stop: &dyn Fn() -> Option<Stopped>,
    ) -> (Vec<String>, Stopped) {
        let roots: Vec<PathBuf> = roots.iter().map(PathBuf::from).collect();
        let mut found = Vec::new();
        let stopped = walk(
            tree,
            &roots,
            depth,
            limit,
            stop,
            &mut |candidate: Candidate| found.push(candidate.path),
        );
        (found, stopped)
    }

    // AYEAYE-50 — breadth first, because a project sits a couple of levels
    // below home: the answers worth having arrive before any bound can bite.
    // And the depth bound is reported, because a walk that was cut short is
    // not worth caching and one that finished is.
    #[test]
    fn the_walk_is_breadth_first_and_stops_at_the_depth_bound() {
        let tree = Tree::of(&[
            ("/root", &[("/root/a", false), ("/root/b", true)]),
            ("/root/a", &[("/root/a/deep", false)]),
            ("/root/a/deep", &[("/root/a/deep/deeper", false)]),
        ]);
        let (found, stopped) = collect(&tree, &["/root"], 2, 0, &never);
        assert_eq!(found, vec!["/root/a", "/root/b", "/root/a/deep"]);
        assert_eq!(stopped, Stopped::Depth);
    }

    // AYEAYE-50 — a limit is an early stop, and an exhausted tree is the one
    // answer worth caching without qualification.
    #[test]
    fn a_limit_stops_it_early_and_an_exhausted_tree_says_so() {
        let tree = Tree::of(&[(
            "/root",
            &[("/root/a", false), ("/root/b", false), ("/root/c", false)],
        )]);
        let (found, stopped) = collect(&tree, &["/root"], 6, 2, &never);
        assert_eq!(found, vec!["/root/a", "/root/b"]);
        assert_eq!(stopped, Stopped::Limit);

        let (found, stopped) = collect(&tree, &["/root"], 6, 0, &never);
        assert_eq!(found.len(), 3, "a limit of zero is no limit at all");
        assert_eq!(stopped, Stopped::Exhausted);

        // A root that is not there at all is not an error, and not a bound.
        assert_eq!(
            collect(&tree, &["/nowhere"], 6, 0, &never).1,
            Stopped::Exhausted
        );
    }

    // AYEAYE-50 — the deadline and the cancel flag are one seam, so a test can
    // fire either without sleeping and without a clock. What was found before
    // the bound fired is kept: partial and immediate is the whole design, and
    // throwing the prefix away would make a cancelled walk cost everything it
    // had already spent.
    #[test]
    fn a_bound_that_fires_mid_walk_ends_it_and_keeps_what_was_found() {
        let tree = Tree::of(&[
            ("/root", &[("/root/a", false), ("/root/b", false)]),
            ("/root/a", &[("/root/a/one", false)]),
            ("/root/b", &[("/root/b/one", false)]),
        ]);
        for bound in [Stopped::Deadline, Stopped::Cancel] {
            let listings = std::cell::Cell::new(0);
            let stop = || {
                listings.set(listings.get() + 1);
                // Let the root be listed, then refuse the level below it.
                (listings.get() > 1).then_some(bound)
            };
            let (found, stopped) = collect(&tree, &["/root"], 6, 0, &stop);
            assert_eq!(stopped, bound);
            assert_eq!(found, vec!["/root/a", "/root/b"], "the prefix is kept");
        }
    }

    // AYEAYE-50 — two directories naming each other, which is what a symlink
    // the lister did follow would look like from up here. Without this the
    // walk never ends.
    #[test]
    fn a_directory_reached_twice_is_visited_once() {
        let tree = Tree::of(&[
            ("/root", &[("/a", false)]),
            ("/a", &[("/b", false)]),
            ("/b", &[("/a", false)]),
        ]);
        let (found, stopped) = collect(&tree, &["/root"], 6, 0, &never);
        assert_eq!(found, vec!["/a", "/b"]);
        assert_eq!(stopped, Stopped::Exhausted);
    }

    // AYEAYE-50 — the bounds the daemon actually runs with, behind the same
    // seam the tests above drive. Cancel is asked first, because a walk the
    // user has typed past must stop costing syscalls whether or not its clock
    // has run out.
    #[test]
    fn the_production_bounds_are_the_deadline_and_the_cancel_flag() {
        let cancel = Arc::new(AtomicBool::new(false));
        let far = Instant::now() + Duration::from_secs(60);
        let plenty = stopper(far, Arc::clone(&cancel));
        assert_eq!(plenty(), None);

        cancel.store(true, Ordering::SeqCst);
        assert_eq!(plenty(), Some(Stopped::Cancel));

        let past = Instant::now() - Duration::from_secs(1);
        let expired = stopper(past, Arc::new(AtomicBool::new(false)));
        assert_eq!(expired(), Some(Stopped::Deadline));
    }

    // AYEAYE-50 — listing the same directory once per keystroke is what would
    // make searching while typing unaffordable, and the clock is an argument
    // so the expiry is a test rather than a sleep.
    #[test]
    fn a_directory_listed_twice_inside_the_ttl_is_read_once() {
        struct Counting(std::sync::atomic::AtomicUsize);
        impl Lister for Counting {
            fn list(&self, path: &Path) -> Listing {
                self.0.fetch_add(1, Ordering::SeqCst);
                vec![(path.join("child"), false)]
            }
        }

        let ttl = Duration::from_secs(60);
        let cache = Cached::new(Counting(std::sync::atomic::AtomicUsize::new(0)), ttl);
        let start = Instant::now();
        let path = Path::new("/root");

        let first = cache.list_at(path, start);
        assert_eq!(cache.list_at(path, start + ttl / 2), first);
        assert_eq!(cache.inner.0.load(Ordering::SeqCst), 1, "read once");

        assert_eq!(cache.list_at(path, start + ttl * 2), first);
        assert_eq!(
            cache.inner.0.load(Ordering::SeqCst),
            2,
            "and read again once it is stale, so a repo cloned a minute ago shows up"
        );
    }

    // AYEAYE-50 — the rules that need a real disk under them: the skip list
    // and its dot rule, a symlinked directory not followed, a plain file not
    // offered, and `.git` as a *file*, which is what a worktree and a
    // submodule keep and both are somewhere you would start an agent.
    #[test]
    fn the_filesystem_lister_reads_the_rules_off_a_real_tree() {
        let root = TempTree::named("lister");
        root.dir("proj/.git");
        root.dir("worktree");
        root.file("worktree/.git", "gitdir: /elsewhere\n");
        root.dir("plain");
        root.dir(".hidden");
        root.dir("node_modules");
        root.dir("thing.egg-info");
        root.dir("machine-hidden");
        root.file("a-plain-file", "");
        root.link("proj", "a-link");

        let lister = FsLister {
            skip_extra: vec!["machine-hidden".to_string()],
        };
        let mut rows: Vec<(String, bool)> = lister
            .list(&root.path)
            .into_iter()
            .map(|(path, git)| {
                (
                    path.file_name().unwrap().to_string_lossy().into_owned(),
                    git,
                )
            })
            .collect();
        rows.sort();
        assert_eq!(
            rows,
            vec![
                ("plain".to_string(), false),
                ("proj".to_string(), true),
                ("worktree".to_string(), true),
            ]
        );
        // A directory that is not there is not an error.
        assert!(lister.list(&root.path.join("nowhere")).is_empty());
    }
}
