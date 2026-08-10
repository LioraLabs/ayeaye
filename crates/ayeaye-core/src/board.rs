//! What the board tab is served, shaped from what cliban says.
//!
//! The page reads the same cliban the terminal does, through its `--json`
//! output rather than the SQLite file underneath, so list and show semantics —
//! archival, ordering, relations — stay cliban's problem rather than becoming
//! this daemon's. What is left here is the shaping: NDJSON in, one payload out,
//! and a stated reason when there is nothing to shape.

/// The rows in a stream of cliban's NDJSON.
///
/// Every line that is one well-formed JSON value, trimmed, in order; blank
/// lines and garbled ones are dropped. That last part is the daemon's
/// behaviour and worth keeping deliberately: a line cliban truncated costs one
/// row rather than the whole board.
///
/// A line ends at a newline and nothing else, where Python's `splitlines`
/// would also break on the vertical tab, the form feed and U+2028. Those are
/// legal raw inside a JSON string, so a ticket titled across one of them is a
/// row the daemon splits in half and drops, and one this keeps.
pub fn rows(stdout: &str) -> Vec<&str> {
    stdout
        .lines()
        .map(str::trim)
        .filter(|line| crate::json::is_value(line))
        .collect()
}

/// Everything the board renders, in one payload.
///
/// One fetch of all projects rather than per-project queries: the whole board
/// is a couple of hundred kilobytes without descriptions, and filtering
/// client-side means switching projects costs nothing on a phone.
///
/// The rows go out as the text cliban produced. They have already been checked
/// to parse; re-rendering them would risk handing back a number spelled
/// differently than the row it came from.
pub fn payload(projects: &[&str], milestones: &[&str], issues: &[&str]) -> String {
    let mut out = String::from("{");
    for (index, (name, rows)) in [
        ("projects", projects),
        ("milestones", milestones),
        ("issues", issues),
    ]
    .iter()
    .enumerate()
    {
        if index > 0 {
            out.push(',');
        }
        out.push('"');
        out.push_str(name);
        out.push_str("\":[");
        out.push_str(&rows.join(","));
        out.push(']');
    }
    out.push('}');
    out
}

/// A board that could not be read, and why.
///
/// The page's own `load()` reads `d.error` and draws the reason with a retry
/// button, so the degraded answer is a stated reason on an empty board rather
/// than a broken one. That only holds while the body still parses, which is
/// the whole reason the message is escaped rather than interpolated: cliban's
/// stderr is where a stray quote or newline would come from.
pub fn failure(message: &str) -> String {
    format!(r#"{{"error":{}}}"#, crate::json::quote(message))
}

/// Just the project keys, for the main page to linkify ticket references with.
///
/// Its own payload rather than a slice of the board, because the app page wants
/// nothing else from cliban and the board is a couple of hundred kilobytes. A
/// row with no `key` is skipped rather than refused: the answer is a list of
/// what can be linkified, and one unusable row does not make the rest unusable.
///
/// The keys are escaped but not shape-checked, which is the daemon's
/// `[p["key"] for p in projs if p.get("key")]` and is on purpose: cliban names
/// its own projects, and unlike [`is_issue_key`] nothing here reaches an argv.
/// Escaping is not optional though — the page reads this with `.json()`, and a
/// body that does not parse is no links at all rather than one link fewer.
pub fn project_keys(rows: &[&str]) -> String {
    let keys: Vec<String> = rows
        .iter()
        .filter_map(|row| crate::json::string_member(row, "key"))
        .filter(|key| !key.is_empty())
        .map(|key| crate::json::quote(&key))
        .collect();
    format!(r#"{{"keys":[{}]}}"#, keys.join(","))
}

/// Whether `key` is shaped like an issue key: `^[A-Za-z][A-Za-z0-9]*-\d+$`.
///
/// The key a request names lands in a subprocess argv, and a shape check is
/// what keeps that endpoint from being a way to hand cliban arbitrary
/// arguments. Not a lookup — cliban decides whether the issue exists; this
/// decides only whether the question is one worth asking it.
///
/// Stricter than the daemon's `re.match` in two places, both deliberate.
/// Python's `$` also matches before a trailing newline, so `AYEAYE-53\n` passes
/// there and not here; and Python's `\d` matches every Unicode digit, so
/// `AYE-` followed by Arabic-Indic digits is a key there and not here. Nothing
/// legitimate sends either, and an argument carrying a newline is exactly what
/// this check exists to refuse.
pub fn is_issue_key(key: &str) -> bool {
    // The project part admits no dash, so a key has exactly one and either end
    // finds it. Splitting at the first keeps the failure legible: `AYE-AYE-53`
    // fails on the number rather than on a project name nobody would recognise.
    let Some((project, number)) = key.split_once('-') else {
        return false;
    };
    let mut letters = project.chars();
    letters
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic())
        && letters.all(|rest| rest.is_ascii_alphanumeric())
        && !number.is_empty()
        && number.chars().all(|digit| digit.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::{failure, is_issue_key, payload, project_keys, rows};
    use crate::json::is_value;

    /// Three lines of `cliban project ls --json`, as they come off the pipe.
    const PROJECTS: &str = concat!(
        r#"{"key":"AYEAYE","name":"AyeAye","updated_at":"2026-08-10T05:53:47Z"}"#,
        "\n",
        r#"{"key":"CLI","name":"Cliban","updated_at":"2026-08-10T01:41:44Z"}"#,
        "\n",
        // A row whose first member is a number, which is what a project with a
        // retention setting really looks like.
        r#"{"auto_archive_done_after_days":7,"key":"COOK","name":"Cook"}"#,
        "\n",
    );

    /// One line of `cliban issue ls --json`, nested `relations` and all: the
    /// shape that actually flows through `rows` on a real board.
    const ISSUE: &str = r#"{"key":"AYEAYE-16","labels":["feature"],"milestone":"Car mode","priority":"urgent","relations":[{"type":"blocks","target":"AYEAYE-18"}],"status":"backlog","title":"Server-side risk classification","updated_at":"2026-08-08T16:11:06Z"}"#;

    // AYEAYE-53 — the rows the board renders are the lines that parse, and the
    // ones that do not are dropped rather than carried: half an object in the
    // payload is a page that cannot render at all, where a missing row is a
    // page missing a row.
    #[test]
    fn the_lines_that_parse_are_rows_and_the_rest_are_dropped() {
        assert_eq!(
            rows(PROJECTS),
            vec![
                r#"{"key":"AYEAYE","name":"AyeAye","updated_at":"2026-08-10T05:53:47Z"}"#,
                r#"{"key":"CLI","name":"Cliban","updated_at":"2026-08-10T01:41:44Z"}"#,
                r#"{"auto_archive_done_after_days":7,"key":"COOK","name":"Cook"}"#,
            ]
        );
        assert!(rows("").is_empty());
        assert!(rows("\n  \n\t\n").is_empty());
        // A truncated line costs itself and nothing else.
        assert_eq!(
            rows("{\"key\":\"A\"}\n{\"key\":\"B\"\n{\"key\":\"C\"}\n"),
            vec![r#"{"key":"A"}"#, r#"{"key":"C"}"#]
        );
        // Trimmed, so the payload does not carry the pipe's whitespace.
        assert_eq!(rows("  {\"a\":1}  \n"), vec![r#"{"a":1}"#]);
        // A row with a nested array of objects in it, which every issue row
        // has and no fixture above did.
        assert_eq!(rows(&format!("{ISSUE}\n")), vec![ISSUE]);
        // Not NDJSON at all: a human-readable table, or an error on stdout.
        assert!(rows("KEY  NAME\nAYEAYE  AyeAye\n").is_empty());
    }

    // AYEAYE-53 — the three lists the page reads, under the three names it
    // reads them under. `load()` in board.html does `d.projects`, `d.milestones`
    // and `d.issues`; a payload that names them anything else renders an empty
    // board with no error to explain it.
    #[test]
    fn the_payload_carries_the_three_lists_the_page_reads() {
        assert_eq!(
            payload(
                &[r#"{"key":"AYEAYE"}"#],
                &[r#"{"name":"One binary","project":"AYEAYE"}"#],
                &[ISSUE],
            ),
            format!(
                concat!(
                    r#"{{"projects":[{{"key":"AYEAYE"}}],"#,
                    r#""milestones":[{{"name":"One binary","project":"AYEAYE"}}],"#,
                    r#""issues":[{}]}}"#,
                ),
                ISSUE
            )
        );
        // Every payload parses, populated or not: the page reads all of them
        // with `.json()`, and one that does not is a blank board with no error.
        assert!(is_value(&payload(
            &[r#"{"key":"AYEAYE"}"#],
            &[r#"{"name":"One binary"}"#],
            &[ISSUE]
        )));
        // An empty board is still a board, and still has to parse.
        assert_eq!(
            payload(&[], &[], &[]),
            r#"{"projects":[],"milestones":[],"issues":[]}"#
        );
        assert!(is_value(&payload(&[], &[], &[])));
    }

    // AYEAYE-53 — "never to a broken panel" is the claim, and the way a stated
    // reason becomes a broken panel is a message that ends its own string. The
    // text below is the shape of a real failure: an execvp error carrying a
    // path, and a multi-line usage dump on stderr.
    #[test]
    fn a_failure_states_its_reason_and_still_parses() {
        assert_eq!(failure("cliban exited 2"), r#"{"error":"cliban exited 2"}"#);
        let awkward = "No such file or directory (os error 2): \"cliban\"\nusage: cliban";
        assert!(is_value(&failure(awkward)));
        assert_eq!(
            crate::json::string_member(&failure(awkward), "error").as_deref(),
            Some(awkward)
        );
    }

    // AYEAYE-53 — the app page builds a regular expression out of these keys,
    // so what goes in the list has to be the keys and only the keys. A row
    // without one is skipped, because the endpoint's whole contract is that it
    // degrades to fewer links rather than to an error.
    #[test]
    fn the_project_keys_are_the_keys_and_the_rest_is_skipped() {
        assert_eq!(
            project_keys(&rows(PROJECTS)),
            r#"{"keys":["AYEAYE","CLI","COOK"]}"#
        );
        assert_eq!(project_keys(&[]), r#"{"keys":[]}"#);
        assert_eq!(
            project_keys(&[r#"{"name":"no key here"}"#, r#"{"key":"COOK"}"#]),
            r#"{"keys":["COOK"]}"#
        );
        // An empty key is skipped, which is `if p.get("key")` and not a
        // formality: app.html joins these with `|` into
        // `\b(?:AYEAYE||CLI)-\d+\b`, and the empty alternative matches the
        // empty string — so every `-123` in a terminal becomes a ticket link.
        assert_eq!(
            project_keys(&[r#"{"key":""}"#, r#"{"key":"CLI"}"#]),
            r#"{"keys":["CLI"]}"#
        );
        // The key is escaped on the way out, not interpolated. cliban names its
        // own projects so nothing here is hostile, but app.html reads this with
        // `.json()` and swallows the failure into a silent catch: a key that
        // ended its own string would cost every ticket link on the page, with
        // nothing anywhere saying why.
        let awkward = project_keys(&[r#"{"key":"a\"b\nc"}"#]);
        assert_eq!(awkward, r#"{"keys":["a\"b\nc"]}"#);
        assert!(is_value(&awkward));
        assert!(is_value(&project_keys(&rows(PROJECTS))));
        assert!(is_value(&project_keys(&[])));
    }

    // AYEAYE-53 — the key goes into a subprocess argv, so this is the check
    // that stops the endpoint being a way to hand cliban arbitrary arguments.
    // The refusals below are the ones that matter: a leading dash is a flag,
    // and whitespace or a newline is a second argument waiting to happen.
    #[test]
    fn only_something_shaped_like_an_issue_key_is_asked_about() {
        for key in ["AYEAYE-53", "A-1", "cli-2", "Ayeaye9-100"] {
            assert!(is_issue_key(key), "should be a key: {key:?}");
        }
        for key in [
            "",
            "AYEAYE",
            "AYEAYE-",
            "-53",
            "9AYE-53",
            "AYEAYE-53x",
            "AYEAYE_53",
            "AYE AYE-53",
            "AYEAYE-53 ",
            "AYEAYE-53\n",
            "--help",
            "AYEAYE-5.3",
            "AYE-AYE-53",
            // ASCII letters and digits only: the key is spliced into a regular
            // expression by the page and into an argv by the shell, and a
            // Unicode digit or a word character that merely looks like one has
            // no business in either.
            "A_1-2",
            "Aé-1",
            "AYE-١٢",
        ] {
            assert!(!is_issue_key(key), "should not be a key: {key:?}");
        }
    }
}
