//! Which candidates the picker shows, and in what order.

use super::recents::Recents;
use crate::json::Value;

/// A directory the walk found, or a pick the walk could not reach.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    /// The absolute path.
    pub path: String,
    /// Whether it holds a `.git`.
    pub is_git: bool,
    /// How far below a root it sits. A recent pick folded back in is 0.
    pub level: usize,
}

/// Rank candidates and keep the best `limit`.
///
/// Git repositories first and everything else after: "does it hold a `.git`"
/// is the strongest signal available that a directory is a project rather than
/// a folder something was once unzipped into.
pub fn order(
    found: &[Candidate],
    recents: &Recents,
    now: f64,
    query: &str,
    limit: usize,
) -> Vec<Candidate> {
    // Scored and matched once each rather than inside the comparator, which
    // would ask for the same frecency and the same lowercasing a few thousand
    // times over for three hundred candidates.
    let mut ranked: Vec<(bool, f64, bool, usize, &Candidate)> = found
        .iter()
        .map(|candidate| {
            (
                !candidate.is_git,
                -recents.score(&candidate.path, now),
                !names_it(&candidate.path, query),
                candidate.level,
                candidate,
            )
        })
        .collect();
    ranked.sort_by(|left, right| {
        left.0
            .cmp(&right.0)
            .then_with(|| left.1.total_cmp(&right.1))
            .then_with(|| left.2.cmp(&right.2))
            .then_with(|| left.3.cmp(&right.3))
            .then_with(|| left.4.path.cmp(&right.4.path))
    });
    ranked
        .into_iter()
        .take(limit)
        .map(|(_, _, _, _, candidate)| candidate.clone())
        .collect()
}

/// Whether a path answers the query at all.
///
/// The whole path, folded to lower case, exactly as the daemon's
/// `q in child.lower()` has it — so typing `ayeaye` finds
/// `/home/a/src/ayeaye` and typing part of a parent finds everything under it.
/// The query is trimmed and folded too: on a phone the first letter is
/// auto-capitalised, so `Ayeaye` is the *common* input, not the exotic one.
///
/// This is the walk's filter as well as the fold-back's. The daemon filters
/// inside the walk and counts only matches against its cap; a walk that
/// counted every directory it *visited* would stop three hundred directories
/// in and answer a query with a handful of rows where the daemon answers with
/// a full list.
pub fn matches(path: &str, query: &str) -> bool {
    let query = query.trim();
    query.is_empty() || path.to_lowercase().contains(&query.to_lowercase())
}

/// How many of the strongest picks are worth offering back as candidates.
pub const RECENT_CANDIDATES: usize = 50;

/// Picks the walk did not reach that are worth checking for.
///
/// Somewhere you have started an agent is a candidate even when the walk
/// cannot reach it: outside the roots, or below the depth bound. The caller
/// still has to decide whether each one is a directory that exists, which is
/// two stats on the request thread — so only [`RECENT_CANDIDATES`] are offered,
/// and the filtering happens to what the history offered rather than to the
/// whole history.
pub fn recent_candidates<'a>(
    recents: &'a Recents,
    now: f64,
    query: &str,
    already: &[String],
) -> Vec<&'a str> {
    recents
        .strongest(now, RECENT_CANDIDATES)
        .into_iter()
        .filter(|path| !already.iter().any(|known| known == path))
        .filter(|path| matches(path, query))
        .collect()
}

/// The path as the picker shows it: `~` for home.
///
/// A *path* prefix, not a string prefix — `/home/alexander` is not inside
/// `/home/alex`, and a picker that says it is shows a path that reads as
/// somewhere else entirely. An empty home, or `HOME=/`, shortens nothing:
/// `~etc` helps nobody read where they are.
pub fn shorten(path: &str, home: &str) -> String {
    let home = home.trim_end_matches('/');
    if home.is_empty() {
        return path.to_string();
    }
    if path == home {
        return "~".to_string();
    }
    match path.strip_prefix(home) {
        Some(rest) if rest.starts_with('/') => format!("~{rest}"),
        _ => path.to_string(),
    }
}

/// One row of the picker, as the page reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// The absolute path, which is what the spawn endpoint is given back.
    pub dir: String,
    /// The path as it is shown.
    pub short: String,
    /// The tmux session this project would use.
    pub session: String,
    /// Whether that session is already running.
    pub exists: bool,
    /// Whether the directory holds a `.git`.
    pub git: bool,
}

/// The answer to a search that a newer query took over.
///
/// Distinguishable from an honest empty result on purpose: `share/app.html`
/// reads this as "leave the list alone and ask again", where an empty list
/// would repaint the picker with "nothing matching" for a question the user
/// has already typed past.
pub const SUPERSEDED: &str = r#"{"projects":[],"superseded":true}"#;

/// The picker's answer.
pub fn body(rows: &[Row]) -> String {
    let projects = rows
        .iter()
        .map(|row| {
            Value::Object(vec![
                ("dir".to_string(), Value::String(row.dir.clone())),
                ("short".to_string(), Value::String(row.short.clone())),
                ("session".to_string(), Value::String(row.session.clone())),
                ("exists".to_string(), Value::Bool(row.exists)),
                ("git".to_string(), Value::Bool(row.git)),
            ])
        })
        .collect();
    Value::Object(vec![("projects".to_string(), Value::Array(projects))]).render()
}

/// Whether the query matches the directory's own name rather than something it
/// happens to sit under.
///
/// An empty query names everything, so it cannot separate two candidates —
/// which is what leaves the level and the path to decide when nobody is
/// typing.
fn names_it(path: &str, query: &str) -> bool {
    matches(basename(path), query)
}

/// The last component of a path, with any trailing slashes ignored.
pub fn basename(path: &str) -> &str {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        // Every slash: the root, which has no component to be named after.
        return "/";
    }
    // The trailing slashes are gone, so whatever follows the last one is a
    // component.
    trimmed.rsplit_once('/').map_or(trimmed, |(_, name)| name)
}

#[cfg(test)]
mod tests {
    use super::{Candidate, Row, SUPERSEDED, body, matches, order, recent_candidates, shorten};
    use crate::projects::recents::Recents;

    fn candidate(path: &str, is_git: bool, level: usize) -> Candidate {
        Candidate {
            path: path.to_string(),
            is_git,
            level,
        }
    }

    fn paths(rows: &[Candidate]) -> Vec<&str> {
        rows.iter().map(|row| row.path.as_str()).collect()
    }

    // AYEAYE-50 — "does it hold a .git" is the strongest signal available that
    // a directory is a project rather than a folder something was once
    // unzipped into, so it outranks everything else there is to know.
    #[test]
    fn a_repository_outranks_a_directory_that_is_not_one() {
        let store = Recents::parse("{\"picks\":{\"/plain\":{\"n\":50,\"t\":100}}}");
        let found = vec![candidate("/plain", false, 1), candidate("/repo", true, 6)];
        assert_eq!(
            paths(&order(&found, &store, 100.0, "", 10)),
            vec!["/repo", "/plain"],
            "a repository comes first even when the other is deeper in the history"
        );
    }

    // AYEAYE-50 — "recent picks influence ranking": inside a tier the pick
    // history decides, which is the whole reason the store exists.
    #[test]
    fn the_pick_history_decides_inside_a_tier() {
        let store = Recents::parse(concat!(
            "{\"picks\":{",
            "\"/repo/often\":{\"n\":10,\"t\":100},",
            "\"/repo/once\":{\"n\":1,\"t\":100}",
            "}}"
        ));
        let found = vec![
            candidate("/repo/never", true, 1),
            candidate("/repo/once", true, 1),
            candidate("/repo/often", true, 1),
        ];
        assert_eq!(
            paths(&order(&found, &store, 100.0, "", 10)),
            vec!["/repo/often", "/repo/once", "/repo/never"]
        );
    }

    // AYEAYE-50 — "query ranking": a query that matches the directory's own
    // name is a better answer than one that matches something it happens to
    // sit under, or typing "ayeaye" would surface every file in the checkout's
    // parent before the checkout.
    #[test]
    fn a_match_on_the_name_beats_one_buried_in_the_path() {
        let store = Recents::default();
        let found = vec![
            candidate("/home/ayeaye/notes", true, 2),
            candidate("/home/src/ayeaye", true, 2),
        ];
        assert_eq!(
            paths(&order(&found, &store, 100.0, "ayeaye", 10)),
            vec!["/home/src/ayeaye", "/home/ayeaye/notes"]
        );
    }

    // AYEAYE-50 — the last two rungs. A project sits a couple of levels below
    // home, so the shallower of two equal answers is the likelier one; and the
    // path itself breaks the final tie, because the list is repainted on every
    // keystroke and two identical queries must produce an identical list.
    #[test]
    fn the_shallower_path_wins_and_the_path_itself_breaks_the_last_tie() {
        let store = Recents::default();
        let found = vec![
            candidate("/a/b/c/deep", true, 4),
            candidate("/a/shallow", true, 2),
        ];
        assert_eq!(
            paths(&order(&found, &store, 100.0, "", 10)),
            vec!["/a/shallow", "/a/b/c/deep"]
        );

        let tied = vec![
            candidate("/z/one", true, 1),
            candidate("/a/one", true, 1),
            candidate("/m/one", true, 1),
        ];
        assert_eq!(
            paths(&order(&tied, &store, 100.0, "", 10)),
            vec!["/a/one", "/m/one", "/z/one"],
            "nothing else can decide, so the path does"
        );
    }

    // AYEAYE-50 — the picker asks for more candidates than it shows, because
    // the first forty a breadth-first walk meets are not the best forty.
    #[test]
    fn only_the_best_limit_come_back() {
        let store = Recents::default();
        let found: Vec<Candidate> = (0..10)
            .map(|index| candidate(&format!("/dir{index}"), index % 2 == 0, 1))
            .collect();
        let best = order(&found, &store, 100.0, "", 3);
        assert_eq!(paths(&best), vec!["/dir0", "/dir2", "/dir4"]);
    }

    // AYEAYE-50 — the one matching rule, used by the walk to decide what to
    // return and by the fold-back to decide what to offer. The query is
    // trimmed and folded to lower case because on a phone the first letter is
    // auto-capitalised: "Ayeaye" is the common input, not the exotic one, and
    // a rule that only matched the exact keys typed would find nothing for
    // most of the people using this.
    #[test]
    fn matching_is_the_whole_path_folded_and_the_query_trimmed() {
        assert!(matches("/home/a/src/ayeaye", "ayeaye"));
        assert!(matches("/home/a/src/ayeaye", "Ayeaye"), "auto-capitalised");
        assert!(matches("/home/a/src/ayeaye", "  ayeaye  "), "trimmed");
        assert!(
            matches("/home/a/AyeAye", "ayeaye"),
            "and the path folds too"
        );
        assert!(matches("/home/a/src/ayeaye", "src"), "part of a parent");
        assert!(!matches("/home/a/src/ayeaye", "codex"));
        // Nothing typed matches everything, which is the picker's first paint.
        assert!(matches("/anything", ""));
        assert!(matches("/anything", "   "));
    }

    // AYEAYE-50 — and the rung reads the same query the same way, so a
    // capitalised search still prefers the directory that is named for it.
    #[test]
    fn the_name_rung_reads_a_capitalised_query_too() {
        let store = Recents::default();
        let found = vec![
            candidate("/home/ayeaye/notes", true, 2),
            candidate("/home/src/ayeaye", true, 2),
        ];
        assert_eq!(
            paths(&order(&found, &store, 100.0, "  AyeAye ", 10)),
            vec!["/home/src/ayeaye", "/home/ayeaye/notes"]
        );
    }

    // AYEAYE-50 — somewhere you have started an agent is a candidate even when
    // the walk cannot reach it: outside the roots, or below the depth bound.
    // Only the strongest handful are worth offering, because each one the
    // shell accepts costs it two stats on the request thread.
    #[test]
    fn a_pick_the_walk_could_not_reach_is_offered_back_as_a_candidate() {
        let store = Recents::parse(concat!(
            "{\"picks\":{",
            "\"/elsewhere/ayeaye\":{\"n\":5,\"t\":100},",
            "\"/elsewhere/other\":{\"n\":4,\"t\":100},",
            "\"/already/ayeaye\":{\"n\":9,\"t\":100}",
            "}}"
        ));
        let already = vec!["/already/ayeaye".to_string()];
        assert_eq!(
            recent_candidates(&store, 100.0, "ayeaye", &already),
            vec!["/elsewhere/ayeaye"],
            "the walk already has one, and the other does not match the query"
        );
        // With nothing typed, everything the walk missed is worth checking.
        assert_eq!(
            recent_candidates(&store, 100.0, "", &already),
            vec!["/elsewhere/ayeaye", "/elsewhere/other"],
            "strongest first"
        );
    }

    // AYEAYE-50 — and no more than the strongest handful, whatever the query
    // does to them: the filter runs on what the history offered, not on the
    // whole history, or a store of two hundred picks would cost the request
    // thread four hundred stats.
    #[test]
    fn only_the_strongest_handful_are_ever_offered() {
        let mut store = Recents::default();
        for index in 0..super::RECENT_CANDIDATES + 20 {
            // Older with every step, so index order is strength order.
            store.record(&format!("/dir{index:03}"), 100.0 - (index as f64) * 86400.0);
        }
        assert_eq!(store.len(), super::RECENT_CANDIDATES + 20);
        let offered = recent_candidates(&store, 100.0, "", &[]);
        assert_eq!(offered.len(), super::RECENT_CANDIDATES);
        assert_eq!(offered[0], "/dir000");
        assert!(
            !offered.contains(&"/dir069"),
            "the weakest never got offered"
        );
    }

    // AYEAYE-50 — a path prefix, not a string prefix: `/home/alexander` is not
    // inside `/home/alex`, and a picker that says it is shows a path that
    // reads as somewhere else entirely.
    #[test]
    fn shortening_is_a_path_prefix_and_not_a_string_one() {
        assert_eq!(
            shorten("/home/alex/src/ayeaye", "/home/alex"),
            "~/src/ayeaye"
        );
        assert_eq!(shorten("/home/alex", "/home/alex"), "~");
        assert_eq!(
            shorten("/home/alexander/src", "/home/alex"),
            "/home/alexander/src"
        );
        assert_eq!(shorten("/opt/thing", "/home/alex"), "/opt/thing");
        // A trailing slash on home is not part of the prefix.
        assert_eq!(shorten("/home/alex/src", "/home/alex/"), "~/src");
        // HOME=/ shortens nothing: "~etc" helps nobody read where they are.
        assert_eq!(shorten("/etc", "/"), "/etc");
        assert_eq!(shorten("/etc", ""), "/etc");
    }

    // AYEAYE-50 — the fields the page reads off each row, under the names it
    // reads them by: `dir`, `short`, `session` and `exists` are what
    // `share/app.html` touches, and `git` is there because the daemon's JSON
    // has it. A renamed field is a picker that renders nothing, and nothing
    // else in the tree would notice.
    #[test]
    fn the_body_carries_the_five_fields_the_page_reads() {
        let rows = vec![
            Row {
                dir: "/home/a/ayeaye".to_string(),
                short: "~/ayeaye".to_string(),
                session: "ayeaye".to_string(),
                exists: true,
                git: true,
            },
            Row {
                dir: "/home/a/plain".to_string(),
                short: "~/plain".to_string(),
                session: "plain".to_string(),
                exists: false,
                git: false,
            },
        ];
        assert_eq!(
            body(&rows),
            concat!(
                r#"{"projects":["#,
                r#"{"dir":"/home/a/ayeaye","short":"~/ayeaye","session":"ayeaye","exists":true,"git":true},"#,
                r#"{"dir":"/home/a/plain","short":"~/plain","session":"plain","exists":false,"git":false}"#,
                r#"]}"#
            )
        );
        assert_eq!(body(&[]), r#"{"projects":[]}"#);
    }

    // AYEAYE-50 — a path can hold a quote, and a body that splices one in
    // unescaped is a page that stops rendering at that directory.
    #[test]
    fn a_path_that_holds_a_quote_does_not_break_the_body() {
        let rows = vec![Row {
            dir: "/home/a/od\"d".to_string(),
            short: "~/od\"d".to_string(),
            session: "od_d".to_string(),
            exists: false,
            git: true,
        }];
        let rendered = body(&rows);
        assert!(rendered.contains(r#""dir":"/home/a/od\"d""#), "{rendered}");
        assert!(crate::json::parse(&rendered).is_some(), "{rendered}");
    }

    // AYEAYE-50 — "an in-flight search is superseded cleanly": the page reads
    // this as "leave the list alone and ask again", which is a different thing
    // from an honest empty result. Answering the empty list instead would
    // repaint the picker with "nothing matching" for a question the user has
    // already typed past.
    #[test]
    fn a_superseded_search_is_not_an_empty_answer() {
        assert_ne!(SUPERSEDED, body(&[]));
        let read = crate::json::parse(SUPERSEDED).expect("the page parses this");
        assert_eq!(
            read.get("superseded"),
            Some(&crate::json::Value::Bool(true))
        );
        assert_eq!(
            read.get("projects"),
            Some(&crate::json::Value::Array(Vec::new()))
        );
    }
}
