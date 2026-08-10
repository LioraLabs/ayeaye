//! The record of which directories you actually use.
//!
//! What the frecency tool the picker used to lean on was really providing was
//! not a directory list — the filesystem already has one — but a record of
//! which directories you actually use. So ayeaye keeps that record itself: no
//! second tool and nothing to train, the picker gets better because you used
//! it.
//!
//! The file is shared with the Python daemon for as long as both run, so the
//! shape here is its shape, down to the version field.

use std::collections::HashMap;

use crate::json::{Value, parse};

/// How long a pick takes to count for half.
///
/// A fortnight is what makes this a record of what you are working on rather
/// than what you once worked on: a directory opened three times this week
/// beats one opened ten times last spring.
pub const HALF_LIFE_DAYS: f64 = 14.0;

/// How many directories are worth remembering.
///
/// A ranking signal, not a history file.
pub const MAX: usize = 200;

const SECONDS_PER_DAY: f64 = 86400.0;

/// One directory's history: how often, and how long ago.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pick {
    /// How many times an agent was started here.
    ///
    /// Signed, because the daemon's reader takes whatever `int()` takes and a
    /// hand-edited `-1` is a row it keeps. Refusing it here would not merely
    /// ignore that row: [`Recents::render`] rewrites the whole shared file, so
    /// a row this reader drops is a row deleted out from under the Python
    /// daemon. A negative count simply ranks below every real pick.
    pub count: i64,
    /// When the last one was, in seconds since the epoch.
    pub at: f64,
}

/// The pick store.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Recents {
    picks: Vec<(String, Pick)>,
}

impl Recents {
    /// Read the store.
    ///
    /// Anything wrong with the text — truncated by a crash, hand-edited into
    /// nonsense, a `picks` that is not an object, a row missing its count or
    /// its timestamp — means *no ranking signal* rather than an error. This is
    /// read on the request path, and a picker without history is still a
    /// working picker.
    pub fn parse(text: &str) -> Recents {
        let Some(document) = parse(text) else {
            return Recents::default();
        };
        let Some(members) = document.get("picks").and_then(Value::as_object) else {
            return Recents::default();
        };
        // A key written twice is one directory: Python's dict keeps the first
        // position and the last value, and so does this. The index is what
        // keeps it linear — a scan of what has been seen, per member, is the
        // quadratic this module went out of its way to remove from the parser.
        let mut picks: Vec<(String, Pick)> = Vec::with_capacity(members.len());
        let mut seen: HashMap<&str, usize> = HashMap::with_capacity(members.len());
        for (path, row) in members {
            let Some(pick) = pick_of(row) else { continue };
            match seen.get(path.as_str()) {
                Some(&at) => picks[at].1 = pick,
                None => {
                    seen.insert(path.as_str(), picks.len());
                    picks.push((path.clone(), pick));
                }
            }
        }
        Recents { picks }
    }

    /// How strong a signal this directory is, now.
    ///
    /// Frecency: how often, discounted by how long ago. A directory nobody has
    /// picked scores nothing, so the ranker can ask this about every candidate
    /// rather than checking first. `now` is an argument because the core has
    /// no clock.
    pub fn score(&self, path: &str, now: f64) -> f64 {
        self.get(path).map_or(0.0, |pick| score_of(pick, now))
    }

    /// Record that an agent was started here.
    ///
    /// `path` is already the normalised key — see [`key`], which the shell
    /// calls because expanding `~` and resolving a relative path need a home
    /// and a working directory the core does not have.
    pub fn record(&mut self, path: &str, now: f64) {
        match self.picks.iter_mut().find(|(known, _)| known == path) {
            Some((_, pick)) => {
                pick.count = pick.count.saturating_add(1);
                pick.at = now;
            }
            None => self
                .picks
                .push((path.to_string(), Pick { count: 1, at: now })),
        }
        self.trim(now);
    }

    /// The strongest directories, strongest first.
    pub fn strongest(&self, now: f64, how_many: usize) -> Vec<&str> {
        // Scored once each rather than inside the comparator, which would ask
        // the same `powf` a few thousand times over for two hundred picks.
        let mut ranked: Vec<(f64, &str)> = self
            .picks
            .iter()
            .map(|(path, pick)| (score_of(*pick, now), path.as_str()))
            .collect();
        // By score, and by path where two scores are equal, so the answer does
        // not depend on what order the file happened to be written in.
        ranked.sort_by(|left, right| right.0.total_cmp(&left.0).then_with(|| left.1.cmp(right.1)));
        ranked
            .into_iter()
            // A directory whose count was hand-edited to zero or below is not
            // a signal, and this is what the ranker folds back in as a
            // candidate: offering one would put a directory nobody has picked
            // ahead of the directories they have.
            .filter(|(score, _)| *score > 0.0)
            .take(how_many)
            .map(|(_, path)| path)
            .collect()
    }

    /// How many directories are remembered.
    pub fn len(&self) -> usize {
        self.picks.len()
    }

    /// Whether anything is remembered at all.
    pub fn is_empty(&self) -> bool {
        self.picks.is_empty()
    }

    /// Drop everything past [`MAX`], weakest first.
    ///
    /// The store is a ranking signal rather than a history file, so what goes
    /// is what could never have reached the top of the list anyway.
    fn trim(&mut self, now: f64) {
        if self.picks.len() <= MAX {
            return;
        }
        let mut order: Vec<usize> = (0..self.picks.len()).collect();
        let score = |index: usize| score_of(self.picks[index].1, now);
        order.sort_by(|&left, &right| {
            score(right)
                .total_cmp(&score(left))
                .then_with(|| self.picks[left].0.cmp(&self.picks[right].0))
        });
        let mut keep = vec![false; self.picks.len()];
        for &index in order.iter().take(MAX) {
            keep[index] = true;
        }
        let mut index = 0;
        self.picks.retain(|_| {
            index += 1;
            keep[index - 1]
        });
    }

    /// This directory's history, if it has one.
    pub fn get(&self, path: &str) -> Option<Pick> {
        self.picks
            .iter()
            .find(|(known, _)| known == path)
            .map(|(_, pick)| *pick)
    }

    /// Write the store.
    pub fn render(&self) -> String {
        Value::Object(vec![
            ("version".to_string(), Value::Number(1.0)),
            (
                "picks".to_string(),
                Value::Object(
                    self.picks
                        .iter()
                        .map(|(path, pick)| {
                            (
                                path.clone(),
                                Value::Object(vec![
                                    ("n".to_string(), Value::Number(pick.count as f64)),
                                    ("t".to_string(), Value::Number(pick.at)),
                                ]),
                            )
                        })
                        .collect(),
                ),
            ),
        ])
        .render()
    }
}

/// The one spelling of a directory the store is keyed by.
///
/// Two spellings of one directory have to be one key, or a pick recorded as
/// `/a/b/` is invisible to a walk that reports `/a/b` and the history quietly
/// stops counting. Only the trailing-slash half is here: expanding `~` and
/// resolving a relative path need a home and a working directory, which are
/// the shell's, and the shell hands in what it resolved.
pub fn key(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        return "/".to_string();
    }
    trimmed.to_string()
}

/// How strong a pick is, now.
///
/// Frecency: how often, discounted by how long ago.
fn score_of(pick: Pick, now: f64) -> f64 {
    // A clock that went backwards, or a timestamp from the future, must not be
    // worth more than the present.
    let age_days = ((now - pick.at) / SECONDS_PER_DAY).max(0.0);
    pick.count as f64 * 0.5f64.powf(age_days / HALF_LIFE_DAYS)
}

/// One row of the store, or `None` if it is not one.
///
/// This is deliberately as permissive as `_recents_load`, and for a reason
/// beyond ranking: [`Recents::render`] rewrites the *whole* shared file, so a
/// row this reader refuses is a row **deleted** from the store the Python
/// daemon is still reading. So `n` takes what `int()` takes — a float, which
/// truncates, or a string of digits — and `t` takes what `float()` takes.
/// What is left over is exactly what the daemon drops too, and the daemon's
/// own `note_project_pick` deletes those on its next write for the same
/// reason.
///
/// The one departure: a non-finite number becomes 0 rather than travelling on.
/// Python would carry a `NaN` into its sort; here a `NaN` score would poison
/// the total order the picker repaints itself from on every keystroke.
fn pick_of(row: &Value) -> Option<Pick> {
    let count = as_int(row.get("n")?)?;
    let at = as_float(row.get("t")?)?;
    Some(Pick { count, at })
}

/// What Python's `int()` would make of this value, if it would make anything.
///
/// Known and deliberate departures, all of them in the refusing direction and
/// none of them producible by the daemon itself: Python takes `int("1_0")` and
/// non-ASCII digits, and its integers have no width, so a count of `10**30`
/// saturates here rather than surviving. A saturated count is still a row; a
/// refused one would be deleted from the shared file.
fn as_int(value: &Value) -> Option<i64> {
    let number = match value {
        Value::Number(number) => *number,
        // `int(True)` is 1. Absurd in a store, and absurd is what a
        // hand-edited file is made of.
        Value::Bool(flag) => return Some(i64::from(*flag)),
        // `int("3")` is 3; `int("3.5")` raises, and so does this. A run of
        // digits too long for an `i64` saturates rather than being refused.
        Value::String(text) => {
            let text = text.trim();
            return text.parse().ok().or_else(|| saturated(text));
        }
        _ => return None,
    };
    if !number.is_finite() {
        return Some(0);
    }
    // `int(3.7)` truncates towards zero rather than rounding.
    Some(number.trunc() as i64)
}

/// An integer literal too long for an `i64`, clamped to the end it ran off.
fn saturated(text: &str) -> Option<i64> {
    let digits = text.strip_prefix(['+', '-']).unwrap_or(text);
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    Some(if text.starts_with('-') {
        i64::MIN
    } else {
        i64::MAX
    })
}

/// What Python's `float()` would make of this value, if it would make anything.
fn as_float(value: &Value) -> Option<f64> {
    let number = match value {
        Value::Number(number) => *number,
        // `float(True)` is 1.0, for the same reason as above.
        Value::Bool(flag) => return Some(if *flag { 1.0 } else { 0.0 }),
        Value::String(text) => text.trim().parse().ok()?,
        _ => return None,
    };
    // The one place this refuses to follow Python: a `NaN` timestamp would
    // poison the total order the picker repaints itself from, so it reads as
    // the epoch — ancient, ranked last, and still a row in the file.
    Some(if number.is_finite() { number } else { 0.0 })
}

#[cfg(test)]
mod tests {
    use super::Recents;

    // AYEAYE-50 — the Python daemon writes this file and this binary rewrites
    // it while both run, so a store that went through here has to be the store
    // the other one reads.
    #[test]
    fn a_store_survives_being_read_and_written_back() {
        let text = concat!(
            r#"{"version":1,"picks":{"#,
            r#""/home/a/src/one":{"n":3,"t":1700000000.5},"#,
            r#""/home/a/src/two":{"n":1,"t":1699000000}"#,
            r#"}}"#
        );
        let store = Recents::parse(text);
        assert_eq!(store.render(), text);
    }

    // AYEAYE-50 — this is read on the request path, and a store that cannot be
    // read has to mean no ranking signal rather than an error: a picker
    // without history is still a working picker, and a daemon that refuses to
    // list projects because a state file was truncated is not.
    #[test]
    fn anything_wrong_with_the_file_means_no_history_rather_than_an_error() {
        let empty = Recents::default();
        assert_eq!(Recents::parse(""), empty, "an empty file");
        assert_eq!(Recents::parse("{\"version\":1,\"pick"), empty, "truncated");
        assert_eq!(Recents::parse("not json at all"), empty, "nonsense");
        assert_eq!(Recents::parse("{\"version\":1}"), empty, "no picks at all");
        assert_eq!(
            Recents::parse("{\"picks\":[\"/a\"]}"),
            empty,
            "picks that is not an object"
        );
        // A row that is genuinely not a row is dropped; the rest of the store
        // survives, because one hand-edited line must not cost every other
        // pick.
        let mixed = Recents::parse(concat!(
            "{\"version\":1,\"picks\":{",
            "\"/a\":{\"n\":2},",
            "\"/b\":{\"t\":5},",
            "\"/c\":\"nonsense\",",
            "\"/good\":{\"n\":4,\"t\":9}",
            "}}"
        ));
        assert_eq!(
            mixed.render(),
            "{\"version\":1,\"picks\":{\"/good\":{\"n\":4,\"t\":9}}}"
        );
    }

    // AYEAYE-50 — render() rewrites the whole file, and that file is shared
    // with the Python daemon for as long as both run. So a row this reader
    // refuses is a row *deleted* out from under it, and the reader has to take
    // everything `int(rec["n"])` and `float(rec["t"])` take: a float, which
    // truncates, a string of digits, and a negative count.
    #[test]
    fn a_row_the_python_daemon_would_keep_is_not_deleted_from_the_shared_file() {
        let store = Recents::parse(concat!(
            "{\"version\": 1, \"picks\": {",
            "\"/truncates\": {\"n\": 1.7, \"t\": 5},",
            "\"/stringy\": {\"n\": \"3\", \"t\": \"5.5\"},",
            "\"/negative\": {\"n\": -1, \"t\": 5}",
            "}}"
        ));
        assert_eq!(store.len(), 3, "every one of these survives `int()`");
        assert_eq!(store.get("/truncates").map(|pick| pick.count), Some(1));
        assert_eq!(store.get("/stringy").map(|pick| pick.count), Some(3));
        assert_eq!(store.get("/stringy").map(|pick| pick.at), Some(5.5));
        assert_eq!(store.get("/negative").map(|pick| pick.count), Some(-1));
        // And a negative count ranks below a directory nobody has picked at
        // all, rather than above every real one.
        assert!(store.score("/negative", 5.0) < store.score("/unknown", 5.0));
        // A string `int()` would refuse is refused here too, which is the one
        // direction where dropping the row matches the daemon dropping it.
        assert_eq!(
            Recents::parse("{\"picks\":{\"/x\":{\"n\":\"3.5\",\"t\":5}}}").len(),
            0
        );
    }

    // AYEAYE-50 — the rest of what `int()` and `float()` take, and the one
    // place this deliberately does not follow them. Every case here is a row
    // the daemon keeps, so every case here is a row this must not delete.
    #[test]
    fn the_odd_corners_of_int_and_float_still_leave_a_row_behind() {
        // `int(True)` is 1 and `float(True)` is 1.0. Absurd in a store, and
        // absurd is what a hand-edited file is made of.
        let flags =
            Recents::parse("{\"picks\":{\"/a\":{\"n\":true,\"t\":5},\"/b\":{\"n\":1,\"t\":true}}}");
        assert_eq!(flags.len(), 2);
        assert_eq!(flags.get("/a").map(|pick| pick.count), Some(1));
        assert_eq!(flags.get("/b").map(|pick| pick.at), Some(1.0));

        // Python's integers have no width. A count too big for an `i64`
        // saturates rather than being refused, because a saturated row is
        // still a row and a refused one is deleted.
        let huge = Recents::parse("{\"picks\":{\"/a\":{\"n\":\"99999999999999999999\",\"t\":5}}}");
        assert_eq!(huge.get("/a").map(|pick| pick.count), Some(i64::MAX));

        // `json.load` reads the three literals its own encoder writes, so a
        // store carrying one must not be refused *whole* — that would rewrite
        // the file with nothing in it. The timestamp reads as the epoch:
        // ancient, ranked last, and still there.
        let poisoned =
            Recents::parse("{\"picks\":{\"/a\":{\"n\":1,\"t\":NaN},\"/b\":{\"n\":1,\"t\":5}}}");
        assert_eq!(
            poisoned.len(),
            2,
            "one bad timestamp must not empty the store"
        );
        assert_eq!(poisoned.get("/a").map(|pick| pick.at), Some(0.0));
        assert!(poisoned.render().is_ascii());
    }

    // AYEAYE-50 — a count hand-edited to zero or below is not a signal, and
    // `strongest` is what the ranker folds back in as candidates: offering one
    // would put a directory nobody has picked ahead of the ones they have.
    #[test]
    fn a_directory_with_no_signal_left_is_not_offered_as_a_candidate() {
        let store = Recents::parse(concat!(
            "{\"picks\":{",
            "\"/negative\":{\"n\":-3,\"t\":5},",
            "\"/zero\":{\"n\":0,\"t\":5},",
            "\"/real\":{\"n\":1,\"t\":5}",
            "}}"
        ));
        assert_eq!(store.strongest(5.0, 10), vec!["/real"]);
        // They are still *in* the store, because deleting them would delete
        // them from the file the Python daemon reads.
        assert_eq!(store.len(), 3);
    }

    // AYEAYE-50 — a key written twice is one directory, and Python's decoder
    // keeps the last one. Two rows for one path would otherwise be scored off
    // the first and both written back.
    #[test]
    fn a_repeated_key_is_the_last_one_written() {
        let store =
            Recents::parse("{\"picks\":{\"/a\":{\"n\":1,\"t\":1},\"/a\":{\"n\":9,\"t\":9}}}");
        assert_eq!(store.len(), 1);
        assert_eq!(store.get("/a").map(|pick| pick.count), Some(9));
        assert_eq!(
            store.render(),
            "{\"version\":1,\"picks\":{\"/a\":{\"n\":9,\"t\":9}}}"
        );
    }

    // AYEAYE-50 — what `json.dump` actually writes: spaces after the
    // separators, and every non-ASCII character escaped. A reader that only
    // took this module's own compact output would agree with itself and with
    // nothing else.
    #[test]
    fn a_store_python_wrote_is_read_and_written_back_as_ascii() {
        let store = Recents::parse(
            "{\"version\": 1, \"picks\": {\"/home/a/caf\\u00e9\": {\"n\": 3, \"t\": 1699000000.0}}}",
        );
        assert_eq!(
            store.get("/home/a/caf\u{e9}").map(|pick| pick.count),
            Some(3)
        );
        let written = store.render();
        assert!(written.is_ascii(), "{written}");
        assert_eq!(
            written,
            "{\"version\":1,\"picks\":{\"/home/a/caf\\u00e9\":{\"n\":3,\"t\":1699000000}}}"
        );
        // And back again, unchanged: the two daemons agree about this file.
        assert_eq!(Recents::parse(&written), store);
    }

    // AYEAYE-50 — frecency: how often, discounted by how long ago. Halving
    // every fortnight is what makes this a record of what you are working on
    // rather than what you once worked on.
    #[test]
    fn a_pick_counts_for_half_after_a_fortnight() {
        let now = 1_700_000_000.0;
        let store = Recents::parse(&format!(
            "{{\"picks\":{{\"/fresh\":{{\"n\":4,\"t\":{now}}},\"/old\":{{\"n\":4,\"t\":{}}}}}}}",
            now - 14.0 * 86400.0
        ));
        assert_eq!(store.score("/fresh", now), 4.0);
        assert_eq!(store.score("/old", now), 2.0);
        // A directory nobody has ever picked scores nothing at all, which is
        // what lets the ranker ask about every candidate.
        assert_eq!(store.score("/never", now), 0.0);
    }

    // AYEAYE-50 — the claim the half life exists to make: three picks this
    // week beat ten last spring. Worked out by hand from the spec's own rule
    // rather than recomputed the way the code computes it.
    #[test]
    fn three_picks_this_week_beat_ten_last_spring() {
        let now = 1_700_000_000.0;
        let store = Recents::parse(&format!(
            "{{\"picks\":{{\"/now\":{{\"n\":3,\"t\":{}}},\"/then\":{{\"n\":10,\"t\":{}}}}}}}",
            now - 7.0 * 86400.0,
            now - 140.0 * 86400.0
        ));
        // 3 * 2^-0.5 is about 2.12; 10 * 2^-10 is about 0.0098.
        assert!(store.score("/now", now) > 2.1);
        assert!(store.score("/then", now) < 0.01);
        // A clock that went backwards is not a pick from the future worth
        // more than the present: age is floored at zero, as the daemon floors
        // it.
        let ahead = Recents::parse("{\"picks\":{\"/ahead\":{\"n\":1,\"t\":9999999999}}}");
        assert_eq!(ahead.score("/ahead", now), 1.0);
    }

    // AYEAYE-50 — "recent picks are recorded": starting an agent somewhere is
    // what teaches the picker, so a first pick appears and a repeat pick both
    // counts for more and moves to now.
    #[test]
    fn a_pick_is_recorded_and_a_repeat_bumps_it() {
        let mut store = Recents::default();
        store.record("/home/a/one", 1_000.0);
        assert_eq!(
            store.get("/home/a/one"),
            Some(super::Pick {
                count: 1,
                at: 1_000.0
            })
        );
        store.record("/home/a/one", 2_000.0);
        assert_eq!(
            store.get("/home/a/one"),
            Some(super::Pick {
                count: 2,
                at: 2_000.0
            })
        );
        assert_eq!(store.len(), 1, "a repeat is the same directory");
    }

    // AYEAYE-50 — a ranking signal, not a history file. Trimming keeps the
    // strongest, so the directories dropped are the ones that could never have
    // reached the top of the list anyway. Picks arrive in time order, which is
    // the only order they can arrive in: the trim happens as each one lands.
    #[test]
    fn the_store_is_trimmed_to_the_strongest_it_is_allowed_to_keep() {
        let now = 1_000_000_000.0;
        let count = super::MAX + 5;
        let mut store = Recents::default();
        for index in 0..count {
            let at = now - ((count - 1 - index) as f64) * 86400.0;
            store.record(&format!("/dir{index}"), at);
        }
        assert_eq!(store.len(), super::MAX);
        assert!(
            store.get(&format!("/dir{}", count - 1)).is_some(),
            "the most recent pick is kept"
        );
        assert!(store.get("/dir0").is_none(), "the weakest is dropped");
        assert_eq!(
            store.strongest(now, 3),
            vec![
                format!("/dir{}", count - 1),
                format!("/dir{}", count - 2),
                format!("/dir{}", count - 3)
            ],
            "strongest first"
        );
    }

    // AYEAYE-50 — "strongest", not "most recent". A directory picked fifty
    // times a fortnight ago is a stronger signal than a directory opened once
    // today, and a trim that kept the newest would drop exactly the one the
    // picker exists to surface. This is the mutation the first version of the
    // test above could not see, because there strength and recency agreed.
    #[test]
    fn a_well_used_directory_survives_a_flood_of_directories_seen_once() {
        let now = 1_000_000_000.0;
        let fortnight_ago = now - 14.0 * 86400.0;
        let mut store = Recents::default();
        for _ in 0..50 {
            store.record("/home/a/the-one", fortnight_ago);
        }
        // 25 after halving, against a score of 1 each for the newcomers.
        for index in 0..super::MAX {
            store.record(&format!("/home/a/seen-once{index}"), now);
        }
        assert_eq!(store.len(), super::MAX);
        assert!(
            store.get("/home/a/the-one").is_some(),
            "fifty picks a fortnight ago outrank one pick today"
        );
        assert_eq!(store.strongest(now, 1), vec!["/home/a/the-one"]);
    }

    // AYEAYE-50 — the picker repaints on every keystroke, so two equal scores
    // have to come back in the same order every time. Insertion order would
    // make the answer depend on what order the file happened to be written in.
    #[test]
    fn two_equal_scores_are_broken_by_the_path() {
        let store = Recents::parse(concat!(
            "{\"picks\":{",
            "\"/b\":{\"n\":1,\"t\":5},",
            "\"/a\":{\"n\":1,\"t\":5},",
            "\"/c\":{\"n\":1,\"t\":5}",
            "}}"
        ));
        assert_eq!(store.strongest(5.0, 3), vec!["/a", "/b", "/c"]);
    }

    // AYEAYE-50 — and the same tie-break where it decides what is *dropped*.
    // `strongest` has this pinned; `trim` sorts on its own, and a stable sort
    // with no tie-break would silently keep whichever the file happened to
    // list first — so the row that must go is deliberately the one that was
    // recorded *earliest*, where insertion order and the path disagree.
    #[test]
    fn two_equal_scores_are_broken_by_the_path_when_one_has_to_go() {
        let now = 1_000.0;
        let mut store = Recents::default();
        store.record("/zzz-earliest", now);
        for index in 0..super::MAX {
            store.record(&format!("/keep{index:04}"), now);
        }
        assert_eq!(store.len(), super::MAX, "the last one displaced somebody");
        assert!(
            store.get("/zzz-earliest").is_none(),
            "a tie goes to the lower path, and every /keep is lower than /zzz"
        );
    }

    // AYEAYE-50 — the store is keyed by path, and two spellings of one
    // directory are one directory: a pick recorded as `/a/b/` must be the pick
    // the ranker finds when the walk reports `/a/b`, or the history silently
    // stops counting.
    #[test]
    fn a_key_is_one_spelling_of_a_directory() {
        assert_eq!(super::key("/a/b/"), "/a/b");
        assert_eq!(super::key("/a/b///"), "/a/b");
        assert_eq!(super::key("/a/b"), "/a/b");
        // The root is the one path whose trailing slash is the whole of it.
        assert_eq!(super::key("/"), "/");
        assert_eq!(super::key(""), "/");

        let mut store = Recents::default();
        store.record(&super::key("/a/b/"), 1.0);
        assert!(store.get(&super::key("/a/b")).is_some());
    }
}
