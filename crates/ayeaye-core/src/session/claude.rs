//! Reading a claude session out of what claude leaves behind.
//!
//! Two routes, and the order matters. Claude keeps one small file per live
//! session under `~/.claude/sessions`, named for the process that owns it: the
//! session id, the working directory and the start time, written at startup and
//! removed when the session exits. That is keyed by pid, so the descendant walk
//! the codex path already does is the whole lookup — no guessing from a working
//! directory and an mtime, which is ambiguous the moment two agents share a
//! repo, and nothing to install.
//!
//! The statusline marker is what a claude too old to write that file had
//! instead, and it is the fallback rather than the fast path. Claude paints the
//! statusline into the *live* screen and it reaches the scrollback only as
//! later output pushes it off; while a question dialog is up it paints no
//! statusline at all. So a session that asks something in its first screenful
//! has no marker in the pane anywhere — and a pane blocked on a question is the
//! exact thing this app exists to surface.

use crate::json;

/// How far the start time recorded in a session file may sit from the
/// process's own, in seconds.
///
/// Claude stamps it a moment after exec and macOS start times are truncated to
/// the second, so the gap is small but never zero. A wider one is a different
/// process wearing a recycled pid: a session file that outlived a crash names a
/// pid the system is free to hand out again, and the next holder of it must not
/// inherit a stranger's conversation.
pub const START_SKEW: f64 = 60.0;

/// What a transcript is called.
const TRANSCRIPT: &str = ".jsonl";

/// The corner brackets the statusline draws its marker between.
///
/// Written as code points rather than as the characters themselves, for the
/// same reason the daemon spells them with a `.replace(" ", "")`: the marker
/// must not appear literally in a file that could end up on somebody's screen
/// inside a pane this scans.
const OPEN: char = '\u{27ea}';
const CLOSE: char = '\u{27eb}';

/// What the marker says it is, between the brackets.
const TAG: &str = "cc:";

/// How many characters of the id the marker carries.
const MARKED: usize = 8;

/// How much of a session id must be plain hex before the dashes start.
///
/// The same eight as [`MARKED`] today, and a different fact: this is what a
/// *file* is found by and that is what a statusline chose to print. Spelling
/// them once would tie the security control below to the width of somebody
/// else's status bar.
const HEX_HEAD: usize = 8;

/// Whether `id` is a session id claude could have written.
///
/// **This is a security control, not a formatting nicety.** The id comes out of
/// a file this server did not write, and it goes on to select entries from a
/// directory scan; an id carrying a separator or a `..` is one that could aim
/// that scan outside the projects directory. Matched rather than trusted, the
/// way the daemon matches it: eight hex characters, then hex and dashes.
pub fn valid_id(id: &str) -> bool {
    let bytes = id.as_bytes();
    let Some((head, tail)) = bytes.split_at_checked(HEX_HEAD) else {
        return false;
    };
    head.iter()
        .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
        && tail
            .iter()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f' | b'-'))
}

/// The session id in one `~/.claude/sessions/<pid>.json`, if it is about the
/// process that was asked about.
///
/// `process_start` is the pid's real start time, in seconds since the epoch,
/// and the file's own `startedAt` is in milliseconds. They have to agree to
/// within [`START_SKEW`] or this is a file about somebody else.
pub fn session_id(file_text: &str, process_start: f64) -> Option<String> {
    let recorded = json::number_member(file_text, "startedAt")?;
    if (recorded / 1000.0 - process_start).abs() > START_SKEW {
        return None;
    }
    let id = json::string_member(file_text, "sessionId")?;
    valid_id(&id).then_some(id)
}

/// Whether `name` — one entry of one directory under the projects root — is
/// the transcript of session `id`.
///
/// A fixed prefix-and-suffix test rather than a pattern match, which is what
/// the daemon's `glob(projects/*/<id>*.jsonl)` comes to once the wildcard is
/// gone. The id is re-checked here rather than only at its source: this is the
/// function that turns an id into a file, so it is the one that must refuse an
/// id that is not one.
pub fn transcript_name(name: &str, id: &str) -> bool {
    valid_id(id) && name.starts_with(id) && name.ends_with(TRANSCRIPT)
}

/// The session id the statusline printed into a pane, if it is there.
///
/// The **first** one, as the daemon's `search` takes it. A pane holds one
/// session's output, so every marker in it names the same id and which one is
/// taken does not arise — except in the case where it does: a pane where claude
/// exited and was started again without the screen being cleared has both ids
/// in its scrollback, oldest first, and this takes the stale one.
///
/// That is a known cost of the marker route and it is why the marker is the
/// *fallback*. The route above it asks the process, which cannot name a session
/// that is not running.
pub fn marker(pane_text: &str) -> Option<String> {
    for (at, _) in pane_text.match_indices(OPEN) {
        let after = &pane_text[at + OPEN.len_utf8()..];
        let Some(rest) = after.strip_prefix(TAG) else {
            continue;
        };
        // Bytes, having established they are all ASCII: only then is `MARKED`
        // an index into this string that could not split a character.
        let Some(marked) = rest.as_bytes().get(..MARKED) else {
            continue;
        };
        if !marked
            .iter()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
        {
            continue;
        }
        if rest[MARKED..].starts_with(CLOSE) {
            return Some(rest[..MARKED].to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{START_SKEW, marker, session_id, transcript_name, valid_id};

    /// What claude writes to `~/.claude/sessions/<pid>.json`, as it was found
    /// on 2.1.226.
    fn session_file(id: &str, started_ms: u64) -> String {
        format!(
            r#"{{"sessionId":"{id}","cwd":"/home/someone/dev/thing","startedAt":{started_ms}}}"#
        )
    }

    const ID: &str = "0123abcd-dead-beef-0000-111122223333";
    const STARTED: f64 = 1_772_614_802.0;

    // AYEAYE-45 — the whole reason this file is preferred over the marker: it
    // names the session id outright, keyed by the pid the walk already found.
    #[test]
    fn a_session_file_names_the_session_its_process_is_running() {
        assert_eq!(
            session_id(&session_file(ID, 1_772_614_802_123), STARTED).as_deref(),
            Some(ID)
        );
    }

    // AYEAYE-45 — a file that outlived a crash names a pid the system is free
    // to hand out again, and the next holder of it must not inherit a
    // stranger's conversation. Claude stamps the time a moment after exec and
    // macOS truncates start times to the second, so the window is small and
    // never zero.
    #[test]
    fn a_file_left_behind_by_a_crash_is_not_this_process() {
        let just_inside = STARTED + START_SKEW - 1.0;
        assert!(
            session_id(&session_file(ID, (just_inside * 1000.0) as u64), STARTED).is_some(),
            "a stamp a moment after exec is this process"
        );

        let yesterday = STARTED - 86_400.0;
        assert_eq!(
            session_id(&session_file(ID, (yesterday * 1000.0) as u64), STARTED),
            None,
            "a day apart is a recycled pid, not a slow startup"
        );
        let just_outside = STARTED + START_SKEW + 1.0;
        assert_eq!(
            session_id(&session_file(ID, (just_outside * 1000.0) as u64), STARTED),
            None
        );

        // Closed at both ends, and on both sides of the start: the daemon
        // compares an absolute difference against `>`, so exactly the skew is
        // still this process.
        for edge in [STARTED + START_SKEW, STARTED - START_SKEW] {
            assert!(
                session_id(&session_file(ID, (edge * 1000.0) as u64), STARTED).is_some(),
                "exactly the skew is inside the window"
            );
        }
        assert_eq!(
            session_id(
                &session_file(ID, ((STARTED - START_SKEW - 1.0) * 1000.0) as u64),
                STARTED
            ),
            None,
            "a file stamped before the process it claims to be about"
        );
    }

    // AYEAYE-45 — anything at all can be sitting in a file this server did not
    // write, including a half-written one: claude writes it at startup, which
    // is exactly when a pane is first looked at.
    #[test]
    fn a_file_that_says_nothing_useful_resolves_to_nothing() {
        assert_eq!(session_id("", STARTED), None);
        assert_eq!(session_id("{", STARTED), None);
        assert_eq!(session_id(r#"{"sessionId":"0123abcd"}"#, STARTED), None);
        assert_eq!(
            session_id(r#"{"startedAt":1772614802123}"#, STARTED),
            None,
            "a start time and no session is not a session"
        );
        assert_eq!(
            session_id(r#"{"sessionId":42,"startedAt":1772614802123}"#, STARTED),
            None
        );
    }

    // AYEAYE-45 — the id is pasted into a directory scan, and this is what
    // stops a file that says something else about itself from aiming that scan
    // somewhere else. The pattern is the control; the scan is not.
    #[test]
    fn an_id_that_could_aim_a_scan_elsewhere_is_refused() {
        for hostile in [
            "../../../../etc/passwd",
            "0123abcd/../../secrets",
            "0123abcd/..",
            "/etc/passwd",
            "0123ABCD",
            "0123abc",
            "",
            "0123abcz",
            "0123abcd*",
        ] {
            assert!(!valid_id(hostile), "{hostile} is not a session id");
            assert_eq!(
                session_id(&session_file(hostile, 1_772_614_802_123), STARTED),
                None,
                "{hostile} reached the caller"
            );
        }
        assert!(
            valid_id("0123abcd"),
            "eight hex is the shortest one there is"
        );
        assert!(valid_id(ID));
    }

    // AYEAYE-45 — a fixed prefix-and-suffix test is what the daemon's
    // `<id>*.jsonl` comes to, and the id is re-checked here because this is the
    // function that turns an id into a file.
    #[test]
    fn a_transcript_is_named_for_its_session() {
        assert!(transcript_name(&format!("{ID}.jsonl"), ID));
        assert!(transcript_name(&format!("{ID}-1.jsonl"), ID));
        assert!(!transcript_name(&format!("{ID}.json"), ID));
        assert!(!transcript_name("something-else.jsonl", ID));
        assert!(!transcript_name(".jsonl", ""));
        assert!(
            !transcript_name("passwd.jsonl", "../../etc/passwd"),
            "an id that is not one names no transcript"
        );
    }

    // AYEAYE-45 — the fallback for a claude too old to write a session file.
    // The marker is mid-scrollback by the time it is readable at all: claude
    // paints it into the live screen and later output is what pushes it into
    // the history this searches.
    #[test]
    fn the_statusline_marker_is_found_in_the_scrollback() {
        let text = format!(
            "$ claude\nsome output\n{}cc:0123abcd{} ~/dev/thing\nmore output\n",
            '\u{27ea}', '\u{27eb}'
        );
        assert_eq!(marker(&text).as_deref(), Some("0123abcd"));
    }

    // AYEAYE-45 — the daemon's `search` takes the first, and a pane normally
    // holds one session's output so every marker in it names the same id.
    #[test]
    fn the_first_marker_wins() {
        let text = format!(
            "{}cc:0123abcd{}\n{}cc:99999999{}\n",
            '\u{27ea}', '\u{27eb}', '\u{27ea}', '\u{27eb}'
        );
        assert_eq!(marker(&text).as_deref(), Some("0123abcd"));
    }

    // AYEAYE-45 — a pane is arbitrary text somebody else's program drew, and
    // most of it is not a marker. Nothing here may index into the middle of a
    // character either: a pane full of box drawing is the ordinary case.
    #[test]
    fn what_is_not_a_marker_is_not_read_as_one() {
        for text in [
            "",
            "no marker here at all",
            "cc:0123abcd",
            &format!("{}cc:0123abcd", '\u{27ea}'),
            &format!("{}cc:0123abc{}", '\u{27ea}', '\u{27eb}'),
            &format!("{}cc:0123ABCD{}", '\u{27ea}', '\u{27eb}'),
            &format!("{}xx:0123abcd{}", '\u{27ea}', '\u{27eb}'),
            &format!("{}cc:{}{}", '\u{27ea}', "─────────", '\u{27eb}'),
        ] {
            assert_eq!(marker(text), None, "read a marker out of {text:?}");
        }
    }

    // AYEAYE-45 — one thing that looks like a marker and is not must not stop
    // the search: the pane is a whole screen of somebody else's output.
    #[test]
    fn a_near_miss_does_not_hide_the_real_marker() {
        let text = format!(
            "{}cc:zzzzzzzz{} and later {}cc:0123abcd{}",
            '\u{27ea}', '\u{27eb}', '\u{27ea}', '\u{27eb}'
        );
        assert_eq!(marker(&text).as_deref(), Some("0123abcd"));
    }
}
