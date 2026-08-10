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
    pub count: u32,
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
        Recents {
            picks: members
                .iter()
                .filter_map(|(path, row)| Some((path.clone(), pick_of(row)?)))
                .collect(),
        }
    }

    /// How strong a signal this directory is, now.
    ///
    /// Frecency: how often, discounted by how long ago. A directory nobody has
    /// picked scores nothing, so the ranker can ask this about every candidate
    /// rather than checking first. `now` is an argument because the core has
    /// no clock.
    pub fn score(&self, path: &str, now: f64) -> f64 {
        let Some(pick) = self.get(path) else {
            return 0.0;
        };
        // A clock that went backwards, or a timestamp from the future, must
        // not be worth more than the present.
        let age_days = ((now - pick.at) / SECONDS_PER_DAY).max(0.0);
        f64::from(pick.count) * 0.5f64.powf(age_days / HALF_LIFE_DAYS)
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
        let mut ranked: Vec<&(String, Pick)> = self.picks.iter().collect();
        // By score, and by path where two scores are equal, so the answer does
        // not depend on what order the file happened to be written in.
        ranked.sort_by(|left, right| {
            self.score(&right.0, now)
                .total_cmp(&self.score(&left.0, now))
                .then_with(|| left.0.cmp(&right.0))
        });
        ranked
            .into_iter()
            .take(how_many)
            .map(|(path, _)| path.as_str())
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
        let keep: Vec<String> = self
            .strongest(now, MAX)
            .into_iter()
            .map(str::to_string)
            .collect();
        self.picks.retain(|(path, _)| keep.contains(path));
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
                                    ("n".to_string(), Value::Number(f64::from(pick.count))),
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

/// One row of the store, or `None` if it is not one.
///
/// A count that is negative or not whole is not a count. The daemon's reader
/// takes whatever `int()` accepts; refusing the row here costs one directory's
/// history and keeps a negative count from outranking every real pick.
fn pick_of(row: &Value) -> Option<Pick> {
    let count = row.get("n")?.as_number()?;
    let at = row.get("t")?.as_number()?;
    if !count.is_finite() || count < 0.0 || count.fract() != 0.0 || !at.is_finite() {
        return None;
    }
    Some(Pick {
        count: count.min(f64::from(u32::MAX)) as u32,
        at,
    })
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
        // A row that is not a row is dropped; the rest of the store survives,
        // because one hand-edited line must not cost every other pick.
        let mixed = Recents::parse(concat!(
            "{\"version\":1,\"picks\":{",
            "\"/a\":{\"n\":2},",
            "\"/b\":{\"t\":5},",
            "\"/c\":\"nonsense\",",
            "\"/d\":{\"n\":-1,\"t\":5},",
            "\"/e\":{\"n\":1.5,\"t\":5},",
            "\"/good\":{\"n\":4,\"t\":9}",
            "}}"
        ));
        assert_eq!(
            mixed.render(),
            "{\"version\":1,\"picks\":{\"/good\":{\"n\":4,\"t\":9}}}"
        );
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
