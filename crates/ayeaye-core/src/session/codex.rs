//! Reading a codex session out of what codex leaves behind.
//!
//! Codex hooks would be the clean route, and they are not available: on 0.145.0
//! `SessionStart` never fires in an interactive session (openai/codex#17532) —
//! verified with a valid matcher and trust granted — and `SessionEnd` is dead
//! too. So there are two answers here, in the order they deserve.
//!
//! **What the process has open is asked first.** It is a fact about the process
//! rather than an inference about it: the kernel is asked which file it holds,
//! so nothing has to be assumed about how soon after starting the file
//! appeared. A *resumed* session is where that shows — its rollout was written
//! days before the process that reopened it, and no window around a start time
//! will ever contain it.
//!
//! **Then the timestamp match**, because older codex did not hold the rollout
//! reliably. A rollout is created within a second or two of the process
//! starting and its filename carries that moment, and pairing that with the
//! working directory keeps two codex agents in one directory distinct. Both
//! work however codex was launched, not just from this app.
//!
//! Codex holds its subagents' rollouts open too, so the ones it is keeping on
//! someone else's behalf are put aside here: a spawned thread records the parent
//! it came from, and the session sitting in the pane is the one that came from
//! nobody.

use crate::json;
use crate::paths::file_name;

/// What a rollout file is called, at both ends.
const ROLLOUT: &str = "rollout-";
const JSONL: &str = ".jsonl";

/// `YYYY-MM-DDTHH-MM-SS`: how much of a rollout's name is its moment.
const STAMPED: usize = 19;

/// The years `strptime` will read, and so the years this will.
const YEARS: std::ops::RangeInclusive<i32> = 1..=9999;

/// What `thread_source` says when a thread was spawned rather than started.
const SUBAGENT: &str = "subagent";

/// How early a rollout may appear relative to the process that owns it.
///
/// Negative, and small: the two clocks are the same clock, and this is only
/// granularity — a start time truncated to the second against a filename
/// rounded the other way.
const EARLIEST: f64 = -5.0;

/// How late it may appear. Generous, for a first turn that takes a while to
/// write anything.
const LATEST: f64 = 120.0;

/// What a rollout's first line says about the session it holds.
///
/// The narrow reading: everything below is what decides whether *this* rollout
/// belongs to *this* pane, and the rest of the payload — the model, the git
/// remote, the context window — belongs to whoever renders it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Meta {
    /// The thread id, which is the session id everywhere else.
    pub id: String,
    /// Where the session is running, or `None` if it did not say.
    pub cwd: Option<String>,
    /// The thread that spawned this one, as codex recorded it.
    pub parent_thread_id: Option<String>,
    /// What codex says this thread came from: `cli` for one somebody started,
    /// `subagent` for one another thread did.
    pub thread_source: Option<String>,
}

impl Meta {
    /// Whether this rollout belongs to a thread its parent spawned.
    ///
    /// Codex holds its subagents' rollouts open too, so the process in the pane
    /// has several and only one of them is its own. Two facts say so and either
    /// is enough — they arrived in different versions of codex — and neither is
    /// stored pre-judged: the raw values are what a transcript view will want to
    /// label a delegation with.
    ///
    /// A parent recorded as `null` or as nothing at all is no parent, which is
    /// what the daemon's truthiness test comes to.
    pub fn is_subagent(&self) -> bool {
        self.parent_thread_id
            .as_deref()
            .is_some_and(|parent| !parent.is_empty())
            || self.thread_source.as_deref() == Some(SUBAGENT)
    }
}

/// The `session_meta` a rollout opens with, or `None`.
///
/// Total over anything at all: these are files this server did not write, read
/// while the process that owns them is writing them. An unfinished first line,
/// a directory, bytes that are not JSON — all of it is `None`, which says "this
/// tells us nothing" and is not the same answer as a payload that happens to be
/// empty.
///
/// A rollout with no id is `None` too. The daemon carries an empty one through
/// its second route and would report a session labelled with nothing; there is
/// no id to find such a file by again, so it is not a session anyone can open.
pub fn meta(first_line: &str) -> Option<Meta> {
    let payload = json::member(first_line, "payload")?;
    let id = json::string_member(payload, "id")?;
    if id.is_empty() {
        return None;
    }
    Some(Meta {
        id,
        cwd: json::string_member(payload, "cwd"),
        parent_thread_id: json::string_member(payload, "parent_thread_id"),
        thread_source: json::string_member(payload, "thread_source"),
    })
}

/// Whether a name in a rollout directory is a rollout.
///
/// A fixed prefix-and-suffix test rather than a pattern match: this is what the
/// daemon's `rollout-*.jsonl` comes to once the wildcard is gone.
pub fn rollout_name(name: &str) -> bool {
    name.starts_with(ROLLOUT) && name.ends_with(JSONL)
}

/// Whether one path a process has open is a rollout under `sessions_root`.
///
/// Not depth-checked, deliberately: a descriptor is a fact about the process,
/// and how deep codex chose to file it is codex's business. What is checked is
/// that it is *inside* the sessions directory, so a `.jsonl` the agent happens
/// to be editing in somebody's repository is not read as a session.
pub fn held(path: &str, sessions_root: &str) -> bool {
    let root = sessions_root.trim_end_matches('/');
    // The separator is part of the prefix, or `/…/.codex/sessions-backup/x`
    // would count as being inside `/…/.codex/sessions`.
    path.len() > root.len()
        && path.starts_with(root)
        && path.as_bytes().get(root.len()) == Some(&b'/')
        && rollout_name(file_name(path))
}

/// A wall-clock moment, as a rollout's name spells it.
///
/// Fields rather than an instant, and that is the whole point: codex writes
/// this stamp in the machine's **local** time, and turning local civil time
/// into an instant needs the machine's timezone — a reach the pure core does
/// not have. The shell converts, with the same `mktime` the daemon uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stamp {
    /// The full year.
    pub year: i32,
    /// 1–12.
    pub month: u32,
    /// 1–31, and a day that month really has: February the 31st is refused
    /// here rather than normalised into March, because the daemon's `strptime`
    /// refuses it too and the `mktime` the shell converts with would not.
    pub day: u32,
    /// 0–23.
    pub hour: u32,
    /// 0–59.
    pub minute: u32,
    /// 0–61, which is the range `strptime`'s `%S` accepts. Two leap seconds in
    /// one minute has never happened; the point is not to differ from the
    /// daemon over a second nobody will ever see.
    pub second: u32,
}

/// The moment in a rollout's name, or `None`.
///
/// `rollout-YYYY-MM-DDTHH-MM-SS-<id>.jsonl`. It is a filename, so it can be
/// anything: a name shaped like this and meaning nothing is refused rather than
/// guessed at.
pub fn stamp(name: &str) -> Option<Stamp> {
    let after = name.strip_prefix(ROLLOUT)?;
    let stamped = after.get(..STAMPED)?;
    let bytes = stamped.as_bytes();
    // Fixed offsets, because the format is fixed. A split on '-' would not do:
    // the separator between the date and the id is the same character.
    for (at, separator) in [(4, b'-'), (7, b'-'), (10, b'T'), (13, b'-'), (16, b'-')] {
        if bytes.get(at) != Some(&separator) {
            return None;
        }
    }
    let field = |range: std::ops::Range<usize>| -> Option<u32> {
        let text = stamped.get(range)?;
        text.bytes()
            .all(|byte| byte.is_ascii_digit())
            .then(|| text.parse().ok())
            .flatten()
    };
    let stamp = Stamp {
        year: i32::try_from(field(0..4)?).ok()?,
        month: field(5..7)?,
        day: field(8..10)?,
        hour: field(11..13)?,
        minute: field(14..16)?,
        second: field(17..19)?,
    };
    (YEARS.contains(&stamp.year)
        && (1..=12).contains(&stamp.month)
        && (1..=days_in(stamp.year, stamp.month)).contains(&stamp.day)
        && stamp.hour < 24
        && stamp.minute < 60
        && stamp.second <= 61)
        .then_some(stamp)
}

/// How many days that month has.
///
/// The check the daemon gets from `strptime` and the shell's `mktime` would
/// not: `mktime` normalises February the 31st into March the 3rd and answers
/// with an instant, where `strptime` raises and the daemon skips the file. A
/// normalised instant is a *different moment* being compared against a process
/// start, which is the one thing this whole comparison exists to get right.
fn days_in(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if year.rem_euclid(4) == 0
            && (year.rem_euclid(100) != 0 || year.rem_euclid(400) == 0) =>
        {
            29
        }
        2 => 28,
        _ => 0,
    }
}

/// How long after a process started its rollout appeared, if that is close
/// enough for the two to be the same session.
///
/// The delta rather than a yes: two rollouts can both fall inside the window,
/// and the one that appeared closest to the process starting is the one that
/// process wrote.
pub fn delay(rollout_at: f64, process_start: f64) -> Option<f64> {
    let delta = rollout_at - process_start;
    (EARLIEST..=LATEST).contains(&delta).then_some(delta)
}

#[cfg(test)]
mod tests {
    use super::{Stamp, delay, held, meta, rollout_name, stamp};
    use crate::machine::fixture;

    const ROOT: &str = "/Users/someone/.codex/sessions";

    // AYEAYE-45 — the ordinary case, read off a rollout's real opening line.
    #[test]
    fn a_rollout_says_which_session_it_holds_and_where_it_runs() {
        let read = meta(fixture!("codex/rollout-meta")).expect("a session_meta");
        assert_eq!(read.id, "0123abcd-dead-beef-0000-111122223333");
        assert_eq!(read.cwd.as_deref(), Some("/Users/someone/dev/thing"));
        assert_eq!(read.thread_source.as_deref(), Some("cli"));
        assert_eq!(read.parent_thread_id, None);
        assert!(!read.is_subagent(), "nobody spawned this one");
    }

    // AYEAYE-45 — codex holds its subagents' rollouts open too, so the process
    // in the pane has several and only one of them is its own. A spawned thread
    // records the parent it came from; the session sitting in the pane is the
    // one that came from nobody.
    #[test]
    fn a_rollout_codex_is_holding_for_a_subagent_says_so() {
        let read = meta(fixture!("codex/rollout-meta-subagent")).expect("a session_meta");
        assert_eq!(
            read.parent_thread_id.as_deref(),
            Some("0123abcd-dead-beef-0000-111122223333"),
            "the parent is kept rather than pre-judged: a transcript view is \
             what labels a delegation with it"
        );
        assert!(
            read.is_subagent(),
            "this one has a parent and says it is a subagent"
        );

        // Either fact alone is enough: the two arrived in different versions.
        let parented = r#"{"payload":{"id":"aaaa","parent_thread_id":"bbbb"}}"#;
        assert!(meta(parented).expect("a payload").is_subagent());
        let sourced = r#"{"payload":{"id":"aaaa","thread_source":"subagent"}}"#;
        assert!(meta(sourced).expect("a payload").is_subagent());

        // And a parent recorded as nothing is no parent, which is what the
        // daemon's truthiness test comes to.
        let orphan = r#"{"payload":{"id":"aaaa","parent_thread_id":null,"thread_source":"cli"}}"#;
        assert!(!meta(orphan).expect("a payload").is_subagent());
        let blank = r#"{"payload":{"id":"aaaa","parent_thread_id":""}}"#;
        assert!(!meta(blank).expect("a payload").is_subagent());
    }

    // AYEAYE-45 — these are files this server did not write, read while the
    // process that owns them is writing them. A half-written first line is the
    // ordinary sight, not the exotic one.
    #[test]
    fn anything_that_is_not_a_session_meta_tells_us_nothing() {
        for line in [
            "",
            "{",
            r#"{"timestamp":"2026-03-04T09:00:02.123Z","type":"session_meta","payload":{"id":"#,
            r#"{"type":"session_meta"}"#,
            r#"{"payload":{}}"#,
            r#"{"payload":{"id":""}}"#,
            r#"{"payload":{"cwd":"/Users/someone/dev/thing"}}"#,
            "not json at all",
        ] {
            assert_eq!(meta(line), None, "read a session out of {line:?}");
        }
    }

    // AYEAYE-45 — a session with no cwd is still a session; it just cannot be
    // matched by the second route.
    #[test]
    fn a_rollout_with_no_working_directory_is_still_a_rollout() {
        let read = meta(r#"{"payload":{"id":"aaaa","thread_source":"cli"}}"#).expect("a payload");
        assert_eq!(read.cwd, None);
        assert_eq!(read.id, "aaaa");
    }

    // AYEAYE-45 — the fixed prefix-and-suffix test that replaces the daemon's
    // `rollout-*.jsonl`.
    #[test]
    fn only_a_rollout_is_a_rollout() {
        assert!(rollout_name("rollout-2026-03-04T09-00-02-0123abcd.jsonl"));
        assert!(!rollout_name(
            "rollout-2026-03-04T09-00-02-0123abcd.jsonl.tmp"
        ));
        assert!(!rollout_name("notes.jsonl"));
        assert!(!rollout_name("rollout-2026-03-04.json"));
        assert!(!rollout_name(""));
    }

    // AYEAYE-45 — a descriptor is a fact about the process, and the only thing
    // that has to be checked about it is that it is inside the sessions
    // directory: a `.jsonl` the agent is editing in somebody's repository is
    // not a session, and neither is one in a directory whose name merely starts
    // the same way.
    #[test]
    fn a_held_rollout_is_one_inside_the_sessions_directory() {
        assert!(held(
            "/Users/someone/.codex/sessions/2026/03/04/rollout-2026-03-04T09-00-02-0123abcd.jsonl",
            ROOT
        ));
        assert!(
            held("/Users/someone/.codex/sessions/rollout-x.jsonl", ROOT),
            "how deep codex files it is codex's business"
        );
        assert!(
            held(
                "/Users/someone/.codex/sessions/2026/03/04/rollout-x.jsonl",
                "/Users/someone/.codex/sessions/"
            ),
            "a trailing separator on the root changes nothing"
        );

        for other in [
            "/Users/someone/dev/thing/rollout-notes.jsonl",
            "/Users/someone/.codex/sessions-backup/2026/rollout-x.jsonl",
            "/Users/someone/.codex/sessions/2026/03/04/notes.jsonl",
            "/Users/someone/.codex/sessions",
            "/dev/pts/3",
        ] {
            assert!(!held(other, ROOT), "{other} is not a held rollout");
        }
    }

    // AYEAYE-45 — the second route reads the moment out of the filename, which
    // is the only place a rollout records when it was created.
    #[test]
    fn a_rollout_name_carries_the_moment_it_was_created() {
        assert_eq!(
            stamp("rollout-2026-03-04T09-00-02-0123abcd-dead-beef.jsonl"),
            Some(Stamp {
                year: 2026,
                month: 3,
                day: 4,
                hour: 9,
                minute: 0,
                second: 2,
            })
        );
    }

    // AYEAYE-45 — it is a filename, so it can be anything. A name shaped like a
    // moment and meaning nothing is refused rather than guessed at.
    #[test]
    fn a_name_that_is_not_a_moment_is_refused() {
        for name in [
            "rollout-2026-13-04T09-00-02-x.jsonl",
            "rollout-2026-03-32T09-00-02-x.jsonl",
            "rollout-2026-03-04T24-00-02-x.jsonl",
            "rollout-2026-03-04T09-60-02-x.jsonl",
            "rollout-2026-03-04 09-00-02-x.jsonl",
            "rollout-2026-03-04T09:00:02-x.jsonl",
            "rollout-202x-03-04T09-00-02-x.jsonl",
            "rollout-2026-03-04T09-00.jsonl",
            "rollout-.jsonl",
            "notes.jsonl",
            "rollout-2026-03-04T09-00-+2-x.jsonl",
        ] {
            assert_eq!(stamp(name), None, "read a moment out of {name}");
        }
        // A day that month does not have is refused here rather than left for
        // the shell's `mktime`, which would normalise it into March and hand
        // back a *different moment* to compare against a process start. The
        // daemon's `strptime` raises on it and skips the file.
        assert_eq!(stamp("rollout-2026-02-31T09-00-02-x.jsonl"), None);
        assert_eq!(stamp("rollout-2026-02-29T09-00-02-x.jsonl"), None);
        assert_eq!(stamp("rollout-2026-04-31T09-00-02-x.jsonl"), None);
        assert_eq!(stamp("rollout-2026-01-00T09-00-02-x.jsonl"), None);
        assert!(
            stamp("rollout-2028-02-29T09-00-02-x.jsonl").is_some(),
            "2028 is a leap year and the 29th is a day it has"
        );
        assert!(stamp("rollout-2000-02-29T09-00-02-x.jsonl").is_some());
        assert_eq!(stamp("rollout-1900-02-29T09-00-02-x.jsonl"), None);
        assert_eq!(
            stamp("rollout-0000-02-29T09-00-02-x.jsonl"),
            None,
            "the daemon's strptime refuses a year outside 1..9999, and mktime \
             would not survive it either"
        );
        assert!(
            stamp("rollout-2026-03-04T09-00-61-x.jsonl").is_some(),
            "61 is what strptime's %S accepts, and differing over it is a \
             divergence for a second nobody will ever see"
        );
    }

    // AYEAYE-45 — a rollout belongs to a process only if it appeared just after
    // that process started. The delta rather than a yes: two can fall inside
    // the window, and the closest is the one that process wrote.
    #[test]
    fn a_rollout_belongs_to_the_process_it_appeared_just_after() {
        let started = 1_772_614_802.0;
        assert_eq!(delay(started + 1.0, started), Some(1.0));
        assert_eq!(delay(started, started), Some(0.0));
        assert_eq!(
            delay(started - 1.0, started),
            Some(-1.0),
            "the two clocks are the same clock; a second early is granularity"
        );
        assert_eq!(delay(started + 120.0, started), Some(120.0));
        assert_eq!(
            delay(started - 5.0, started),
            Some(-5.0),
            "the window is closed at both ends"
        );

        assert_eq!(delay(started + 121.0, started), None);
        assert_eq!(delay(started - 6.0, started), None);
        assert_eq!(
            delay(started - 86_400.0, started),
            None,
            "yesterday's rollout is what the held-open route exists for"
        );
    }
}
