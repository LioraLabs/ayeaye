//! The one walk in flight, and what happens when a newer query arrives.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ayeaye_core::projects::rank::Candidate;

use super::walk::{Lister, Stopped, stopper, walk};

/// What a caller gets back.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// The candidates found — every one of them, or the prefix that arrived in
    /// time.
    Found(Vec<Candidate>),
    /// A newer query took this search over.
    Superseded,
}

/// How far a search has got.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Running,
    Finished,
    Cancelled,
}

/// The bounds a search runs under.
#[derive(Debug, Clone)]
pub struct Bounds {
    /// Where to look.
    pub roots: Vec<PathBuf>,
    /// How far below a root a project may sit.
    pub depth: usize,
    /// Wall clock one walk may spend. It runs on a thread of its own, so this
    /// bounds the work rather than the response.
    pub budget: Duration,
    /// Wall clock a request may wait for that walk before answering with
    /// whatever has arrived. This is the number that keeps the request loop
    /// moving.
    pub wait: Duration,
    /// How long a finished search stays usable.
    pub ttl: Duration,
}

/// How many finished searches are remembered at once. The daemon's own number.
const CACHE_MAX: usize = 64;

/// The searches this server is running.
pub struct Searches {
    lister: Arc<dyn Lister + 'static>,
    bounds: Bounds,
    shared: Arc<Mutex<State>>,
}

#[derive(Default)]
struct State {
    current: Option<Arc<Search>>,
    cache: HashMap<(String, usize), (Instant, Vec<Candidate>)>,
}

impl State {
    /// Forget every cached answer that has gone stale.
    ///
    /// Short enough that a repo cloned a minute ago shows up, and it is also
    /// what bounds the map: a daemon that ran for a month would otherwise
    /// remember every prefix anybody ever typed.
    fn prune(&mut self, now: Instant, ttl: Duration) {
        self.cache
            .retain(|_, (at, _)| now.duration_since(*at) < ttl);
        // And a hard cap, as the daemon has. The TTL alone bounds this by
        // "distinct completed walks per minute", which is small in practice
        // and is not a bound. Cleared wholesale rather than evicted: refilling
        // costs one bounded walk, and an eviction policy is more code than the
        // thing it serves.
        if self.cache.len() > CACHE_MAX {
            self.cache.clear();
        }
    }
}

/// The registry as the walking thread sees it.
///
/// A handle rather than the whole [`Searches`], because the walk finishes on a
/// thread that outlives the request and must not keep the lister alive with it.
struct Registry {
    state: Arc<Mutex<State>>,
    ttl: Duration,
}

impl Registry {
    /// Record that a walk ended, and cache it if it is worth caching.
    ///
    /// Cached here rather than by whoever was waiting, because a walk that
    /// finishes after its waiter gave up is the ordinary case on a cold cache
    /// — and throwing that away would mean the next keystroke starts again
    /// from nothing, for ever.
    fn finished(&self, search: &Arc<Search>, stopped: Stopped) {
        let mut state = self.state.lock().expect("the search registry");
        // A walk a clock or a cancellation cut short is a prefix, and caching a
        // prefix would answer the same question with it until the TTL ran out.
        if matches!(
            stopped,
            Stopped::Exhausted | Stopped::Depth | Stopped::Limit
        ) {
            let now = Instant::now();
            state.prune(now, self.ttl);
            state
                .cache
                .insert((search.query.clone(), search.cap), (now, search.rows()));
        }
        if state
            .current
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, search))
        {
            state.current = None;
        }
        search.finish();
    }
}

/// However a walk ends, the registry is told.
struct Ending {
    registry: Registry,
    search: Arc<Search>,
    stopped: Mutex<Stopped>,
}

impl Drop for Ending {
    fn drop(&mut self) {
        let stopped = self
            .stopped
            .lock()
            .map(|stopped| *stopped)
            .unwrap_or(Stopped::Cancel);
        self.registry.finished(&self.search, stopped);
    }
}

/// The roots that are there right now, absolute.
///
/// `walk_projects` does `abspath(r)` and then `isdir(r)`, and both halves
/// matter: a relative root yields relative candidate paths, which match no key
/// in the pick history and shorten to nothing when the picker draws them.
fn existing(roots: &[PathBuf]) -> Vec<PathBuf> {
    let here = std::env::current_dir().unwrap_or_default();
    roots
        .iter()
        .map(|root| {
            if root.is_absolute() {
                root.clone()
            } else {
                here.join(root)
            }
        })
        .filter(|root| root.is_dir())
        .collect()
}

struct Search {
    query: String,
    cap: usize,
    rows: Mutex<Vec<Candidate>>,
    cancel: Arc<AtomicBool>,
    phase: tokio::sync::watch::Sender<Phase>,
}

impl Search {
    fn new(query: &str, cap: usize) -> Search {
        Search {
            query: query.to_string(),
            cap,
            rows: Mutex::new(Vec::new()),
            cancel: Arc::new(AtomicBool::new(false)),
            phase: tokio::sync::watch::Sender::new(Phase::Running),
        }
    }

    /// Appended to by the walking thread and copied by whoever is waiting, so
    /// a waiter that gives up early still gets a coherent prefix.
    fn push(&self, candidate: Candidate) {
        if let Ok(mut rows) = self.rows.lock() {
            rows.push(candidate);
        }
    }

    fn rows(&self) -> Vec<Candidate> {
        self.rows
            .lock()
            .map(|rows| rows.clone())
            .unwrap_or_default()
    }

    /// Stop the thread that is walking, not merely the one that is waiting.
    ///
    /// A query the user has already typed past must stop costing syscalls, or
    /// every keystroke leaves another walk grinding away behind it.
    fn supersede(&self) {
        self.cancel.store(true, Ordering::SeqCst);
        let _ = self.phase.send(Phase::Cancelled);
    }

    /// Finished — unless it was already cancelled, which is the answer its
    /// waiters have been given and must not be taken back.
    fn finish(&self) {
        self.phase.send_if_modified(|phase| {
            if *phase == Phase::Running {
                *phase = Phase::Finished;
                return true;
            }
            false
        });
    }
}

impl Searches {
    /// A registry over a lister.
    pub fn new(lister: Arc<dyn Lister + 'static>, bounds: Bounds) -> Searches {
        Searches {
            lister,
            bounds,
            shared: Arc::new(Mutex::new(State::default())),
        }
    }

    /// Search, waiting no longer than the configured wait.
    ///
    /// The walk never runs on the calling task. This is reached from the
    /// request handler while somebody types, and a filesystem walk there would
    /// put the pane polls and the transcript stream behind it — so the walk
    /// goes to a blocking thread, this waits, and answers with whatever has
    /// arrived by then. Partial and immediate beats complete and late when the
    /// next keystroke is 150ms away.
    ///
    /// A different query supersedes the one before it: the older caller gives
    /// up its wait *at once* and says so, rather than racing the newer answer
    /// to the browser, and the walk behind it is cancelled rather than left
    /// grinding away. Two callers asking the same thing share one walk, so a
    /// second browser cannot starve the first.
    pub async fn search(&self, query: &str, cap: usize) -> Outcome {
        let key = (query.to_string(), cap);
        let search = {
            let mut state = self.shared.lock().expect("the search registry");
            state.prune(Instant::now(), self.bounds.ttl);
            if let Some((_, rows)) = state.cache.get(&key) {
                return Outcome::Found(rows.clone());
            }
            match &state.current {
                // Already being walked: wait on it rather than starting a
                // second walk over the same tree.
                Some(current) if (current.query.as_str(), current.cap) == (query, cap) => {
                    Arc::clone(current)
                }
                _ => {
                    if let Some(older) = state.current.take() {
                        older.supersede();
                    }
                    let search = Arc::new(Search::new(query, cap));
                    state.current = Some(Arc::clone(&search));
                    self.spawn(Arc::clone(&search));
                    search
                }
            }
        };

        // A watch rather than a notification: it carries the phase the search
        // is already in, so a walk that finished between the lock above and the
        // wait below cannot leave a caller waiting for a wakeup that has
        // already happened.
        //
        // And a `select` rather than a poll loop — this is the half of the
        // Python's event dance that the async runtime really does replace. A
        // superseded caller wakes the moment the newer query arrives, instead
        // of at the end of the next 20ms tick.
        let mut phase = search.phase.subscribe();
        let budget = tokio::time::sleep(self.bounds.wait);
        tokio::pin!(budget);
        let reached = loop {
            let seen = *phase.borrow_and_update();
            if seen != Phase::Running {
                break seen;
            }
            tokio::select! {
                changed = phase.changed() => {
                    // Unreachable while this caller holds the `Arc` the sender
                    // lives in. Answered rather than ignored, because the
                    // alternative is a loop spinning on a closed channel.
                    if changed.is_err() {
                        break Phase::Finished;
                    }
                }
                // Out of time. `Running` is the honest answer, and the caller
                // below takes whatever prefix has arrived.
                _ = &mut budget => break Phase::Running,
            }
        };

        if reached == Phase::Cancelled {
            return Outcome::Superseded;
        }
        // Whatever has arrived: the walk appends as it goes, so a caller that
        // gave up early still takes a coherent prefix of the answer.
        Outcome::Found(search.rows())
    }

    /// Put the walk on a blocking thread.
    ///
    /// `spawn_blocking` rather than a task: `read_dir` blocks, and a blocking
    /// task cannot be aborted by the runtime. That is exactly why the cancel
    /// flag survives the port from Python and why the walk checks it between
    /// directories — what the runtime buys is the *waiting* side, where a
    /// superseded caller wakes at once instead of polling every 20ms.
    fn spawn(&self, search: Arc<Search>) {
        let lister = Arc::clone(&self.lister);
        let bounds = self.bounds.clone();
        let registry = self.handle();
        tokio::task::spawn_blocking(move || {
            // The Python wraps its walk in `try/finally` so `done` is set
            // however the walk ends. Without the same guarantee a panicking
            // walk would leave this search `Running` for ever, and every later
            // caller asking the same question would join a dead search, wait
            // the whole budget and get its partial rows — never re-walked and
            // never cached. This guard is that `finally`.
            let ending = Ending {
                registry,
                search: Arc::clone(&search),
                // Unless the walk says otherwise it did not finish, so nothing
                // is cached.
                stopped: Mutex::new(Stopped::Cancel),
            };
            let stop = stopper(Instant::now() + bounds.budget, Arc::clone(&search.cancel));
            let stopped = walk(
                lister.as_ref(),
                // Re-checked here rather than once at construction, as the
                // daemon re-checks them: a root created after this server
                // started is still a root.
                &existing(&bounds.roots),
                bounds.depth,
                search.cap,
                &search.query,
                &stop,
                &mut |candidate| search.push(candidate),
            );
            if let Ok(mut ended) = ending.stopped.lock() {
                *ended = stopped;
            }
        });
    }

    fn handle(&self) -> Registry {
        Registry {
            state: Arc::clone(&self.shared),
            ttl: self.bounds.ttl,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Bounds, Candidate, Duration, Lister, Outcome, PathBuf, Searches};
    use crate::projects::walk::Listing;
    use std::path::Path;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Condvar, Mutex};

    /// A lister that blocks until it is let go, so a walk can genuinely be in
    /// flight while a second query arrives. Nothing else produces that state:
    /// a walk over a real tree finishes before a test can race it.
    struct Blocking {
        gate: Mutex<bool>,
        opened: Condvar,
        listings: AtomicUsize,
    }

    impl Blocking {
        fn new() -> Arc<Blocking> {
            Arc::new(Blocking {
                gate: Mutex::new(false),
                opened: Condvar::new(),
                listings: AtomicUsize::new(0),
            })
        }

        fn release(&self) {
            *self.gate.lock().expect("the gate") = true;
            self.opened.notify_all();
        }

        /// Release on the way out, however the test leaves.
        ///
        /// A blocked walk keeps a blocking thread parked, and the runtime will
        /// not shut down while one is — so a *failing* assertion would hang
        /// the suite instead of reporting it. The report is the point.
        fn released_on_unwind(self: &Arc<Blocking>) -> Guard {
            Guard(Arc::clone(self))
        }
    }

    struct Guard(Arc<Blocking>);

    impl Drop for Guard {
        fn drop(&mut self) {
            self.0.release();
        }
    }

    impl Lister for Blocking {
        fn list(&self, path: &Path) -> Listing {
            self.listings.fetch_add(1, Ordering::SeqCst);
            let mut open = self.gate.lock().expect("the gate");
            while !*open {
                open = self.opened.wait(open).expect("the gate");
            }
            // One child, one level down, and then the tree ends.
            if path.to_string_lossy().contains("child") {
                return Vec::new();
            }
            vec![(path.join("child"), true)]
        }
    }

    fn bounds(wait: Duration) -> Bounds {
        Bounds {
            roots: vec![PathBuf::from("/root")],
            depth: 6,
            budget: Duration::from_secs(30),
            wait,
            ttl: Duration::from_secs(60),
        }
    }

    fn paths(outcome: &Outcome) -> Vec<&str> {
        match outcome {
            Outcome::Found(rows) => rows
                .iter()
                .map(|row: &Candidate| row.path.as_str())
                .collect(),
            Outcome::Superseded => panic!("superseded, not found"),
        }
    }

    // AYEAYE-50 — two callers asking the same thing share one walk, so a
    // second browser cannot start a second walk over somebody's whole home
    // directory and starve the first.
    #[tokio::test]
    async fn two_callers_asking_the_same_thing_share_one_walk() {
        let lister = Blocking::new();
        let _release = lister.released_on_unwind();
        let searches = Arc::new(Searches::new(
            lister.clone(),
            bounds(Duration::from_secs(5)),
        ));

        let one = tokio::spawn({
            let searches = searches.clone();
            async move { searches.search("child", 100).await }
        });
        let two = tokio::spawn({
            let searches = searches.clone();
            async move { searches.search("child", 100).await }
        });
        // Let both reach the wait, then let the walk run.
        tokio::time::sleep(Duration::from_millis(50)).await;
        let started = lister.listings.load(Ordering::SeqCst);
        lister.release();

        let (one, two) = (one.await.expect("one"), two.await.expect("two"));
        assert_eq!(paths(&one), vec!["/root/child"]);
        assert_eq!(paths(&two), vec!["/root/child"]);
        assert_eq!(started, 1, "one walk was started, not two");
    }

    // AYEAYE-50 — "an in-flight search is superseded cleanly when a newer
    // query arrives". The older caller answers *at once* rather than at the
    // end of its wait, and its walk is cancelled rather than left grinding
    // away behind every keystroke.
    #[tokio::test]
    async fn a_newer_query_supersedes_the_one_before_it() {
        let lister = Blocking::new();
        let _release = lister.released_on_unwind();
        let searches = Arc::new(Searches::new(
            lister.clone(),
            // A wait far longer than this test: if supersession did not cut
            // the wait short, this would take five seconds rather than being
            // quick and wrong.
            bounds(Duration::from_secs(5)),
        ));

        let older = tokio::spawn({
            let searches = searches.clone();
            async move { searches.search("chi", 100).await }
        });
        tokio::time::sleep(Duration::from_millis(50)).await;

        let started = std::time::Instant::now();
        let newer = tokio::spawn({
            let searches = searches.clone();
            async move { searches.search("child", 100).await }
        });
        assert_eq!(
            older.await.expect("the older search"),
            Outcome::Superseded,
            "the older caller is told, rather than racing the newer answer"
        );
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "it answered at once, not at the end of its wait: {:?}",
            started.elapsed()
        );

        lister.release();
        assert_eq!(
            paths(&newer.await.expect("the newer search")),
            vec!["/root/child"]
        );

        // And the older walk was cancelled rather than left to finish: a walk
        // a cancellation cut short is not cached, so asking for it again walks
        // again. This is the half a waiter cannot observe, and the half that
        // costs syscalls behind every keystroke when it is missing.
        let before = lister.listings.load(Ordering::SeqCst);
        assert_eq!(
            paths(&searches.search("chi", 100).await),
            vec!["/root/child"]
        );
        assert!(
            lister.listings.load(Ordering::SeqCst) > before,
            "a cancelled walk must not be cached as though it had finished"
        );
    }

    // AYEAYE-50 — a walk that finished is worth caching; one that a bound cut
    // short is not, because caching a prefix would answer the same question
    // with it until the TTL ran out.
    #[tokio::test]
    async fn a_walk_that_finished_is_served_from_memory_next_time() {
        let lister = Blocking::new();
        let _release = lister.released_on_unwind();
        lister.release();
        let searches = Searches::new(lister.clone(), bounds(Duration::from_secs(5)));

        assert_eq!(
            paths(&searches.search("child", 100).await),
            vec!["/root/child"]
        );
        let after_first = lister.listings.load(Ordering::SeqCst);
        assert_eq!(
            paths(&searches.search("child", 100).await),
            vec!["/root/child"]
        );
        assert_eq!(
            lister.listings.load(Ordering::SeqCst),
            after_first,
            "the second ask walked nothing"
        );

        // A different cap is a different question, and is walked.
        assert_eq!(
            paths(&searches.search("child", 200).await),
            vec!["/root/child"]
        );
        assert!(lister.listings.load(Ordering::SeqCst) > after_first);
    }

    // AYEAYE-50 — partial and immediate beats complete and late when the next
    // keystroke is 150ms away: the request answers with the prefix that
    // arrived, and the walk carries on behind it.
    #[tokio::test]
    async fn a_wait_shorter_than_the_walk_answers_with_what_arrived() {
        let lister = Blocking::new();
        let _release = lister.released_on_unwind();
        let searches = Searches::new(lister.clone(), bounds(Duration::from_millis(30)));
        assert_eq!(
            searches.search("child", 100).await,
            Outcome::Found(Vec::new()),
            "nothing yet, and no error"
        );
    }
}
