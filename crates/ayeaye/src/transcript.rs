//! The transcript endpoints' effects: opening the file the session named.
//!
//! What a line *means* is `ayeaye_core::transcript`'s; this module is the
//! reading. `/api/message` answers here through the server's endpoint chain;
//! the event stream's tailing joins it in this file so both readers of a
//! session's transcript sit side by side.
//!
//! Every file read runs on the blocking pool, the same decision
//! `crate::session` records: a transcript grows to megabytes over a long
//! session, and a read that stalls under a hung mount must cost its request
//! rather than a runtime worker.

use axum::http::StatusCode;

use ayeaye_core::json;
use ayeaye_core::session::Kind;
use ayeaye_core::transcript;

use crate::config::Settings;
use crate::session;

/// The endpoint this module answers on the plain-JSON chain.
const MESSAGE: &str = "/api/message";

/// Answer `/api/message`, or `None` if this is not it.
///
/// Joined to the server's endpoint chain rather than mounted on the router,
/// so it inherits the Host gate, the CSRF gate and the token gate the one
/// handler has already applied.
pub async fn answer(
    settings: &Settings,
    path: &str,
    query: Option<&str>,
) -> Option<(StatusCode, String)> {
    (path == MESSAGE).then_some(())?;
    Some(message(settings, query).await)
}

/// One original, unclipped conversational transcript row.
///
/// Everything that can be missing — the pane, the session, the line, the item,
/// a row that is not conversation — is the same 404 with the daemon's exact
/// body. The panel shows the error text either way, and which link in the
/// chain broke is not something a caller does anything differently about.
async fn message(settings: &Settings, query: Option<&str>) -> (StatusCode, String) {
    let found = match first(query, "pane") {
        Some(pane) => session::resolve(settings, &pane).await,
        None => None,
    };
    let Some(found) = found else {
        return not_found();
    };
    let reference = first(query, "ref").unwrap_or_default();
    let read = tokio::task::spawn_blocking(move || one_row(&found.path, found.kind, &reference))
        .await
        .ok()
        .flatten();
    match read {
        Some(body) => (StatusCode::OK, body),
        None => not_found(),
    }
}

/// The row a reference names, read out of the file, as the response body.
///
/// The seam a test drives: a path, a kind and a reference in, a body or
/// nothing out, with no tmux and no session resolution anywhere near it.
/// Public for exactly that reason.
pub fn one_row(path: &str, kind: Kind, reference: &str) -> Option<String> {
    let (line, item) = transcript::reference(reference)?;
    let bytes = std::fs::read(path).ok()?;
    // Lossily, exactly as the stream reads: one odd byte in one line must not
    // cost the row it is in, let alone the file.
    let text = String::from_utf8_lossy(&bytes);
    let raw = text.split('\n').nth(line)?;
    let rows = transcript::rows(raw, kind);
    transcript::message_body(rows.get(item)?)
}

/// The daemon's answer when any link in the chain is missing.
fn not_found() -> (StatusCode, String) {
    (StatusCode::NOT_FOUND, json::error("message not found"))
}

/// The first non-empty value of a query parameter.
///
/// The same reading `crate::session` does, spelled here rather than shared:
/// four lines, and a helper exported for them would be a public seam two
/// modules then have to agree on forever.
fn first(query: Option<&str>, name: &str) -> Option<String> {
    form_urlencoded::parse(query.unwrap_or("").as_bytes())
        .find(|(key, value)| key == name && !value.is_empty())
        .map(|(_, value)| value.into_owned())
}

#[cfg(test)]
mod tests {
    use super::one_row;
    use ayeaye_core::session::Kind;
    use std::fs;
    use std::path::PathBuf;

    /// A transcript of one's own, cleaned up on the way out.
    struct File(PathBuf);

    impl File {
        fn holding(what: &str, lines: &str) -> File {
            let path = std::env::temp_dir().join(format!(
                "ayeaye-46-message-{}-{what}.jsonl",
                std::process::id()
            ));
            fs::write(&path, lines).expect("a transcript");
            File(path)
        }

        fn path(&self) -> &str {
            self.0.to_str().expect("a utf-8 temp path")
        }
    }

    impl Drop for File {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.0);
        }
    }

    // AYEAYE-46 — the acceptance criterion: a single message renders from a
    // reference, original and unclipped, found by the line the ref names in
    // the file rather than by its place in any stream.
    #[test]
    fn a_reference_names_one_unclipped_row_in_the_file() {
        let long = "x".repeat(5000);
        let spoken = format!(
            r#"{{"type":"assistant","timestamp":"2026-08-10T12:00:05Z","message":{{"content":[{{"type":"text","text":"{long}"}}]}}}}"#
        );
        let file = File::holding(
            "claude",
            &format!(
                "{}\n{spoken}\n",
                r#"{"type":"summary","summary":"bookkeeping"}"#,
            ),
        );

        let body = one_row(file.path(), Kind::Claude, "1:0").expect("the row is there");
        assert!(
            body.contains(&long),
            "the whole text, unclipped — that is the page's whole point"
        );
        assert!(body.starts_with(r#"{"cls":"assistant","ts":"12:00:05","label":"claude""#));

        // The bookkeeping line maps to no rows, so item 0 of line 0 is nothing.
        assert_eq!(one_row(file.path(), Kind::Claude, "0:0"), None);
    }

    // AYEAYE-46 — the same, through codex's format.
    #[test]
    fn a_codex_reference_resolves_too() {
        let file = File::holding(
            "codex",
            "{\"timestamp\":\"2026-08-10T09:30:00Z\",\"payload\":{\"type\":\"agent_message\",\"message\":\"hello\"}}\n",
        );
        assert_eq!(
            one_row(file.path(), Kind::Codex, "0:0"),
            Some(r#"{"cls":"assistant","ts":"09:30:00","label":"codex","text":"hello"}"#.to_string())
        );
    }

    // AYEAYE-46 — everything that can be missing is the same nothing: a line
    // past the end, an item past the row count, a ref that is not a ref, a
    // file that is not there, and a row that is not conversation.
    #[test]
    fn every_missing_link_is_the_same_nothing() {
        let file = File::holding(
            "missing",
            &format!(
                "{}\n{}\n",
                r#"{"type":"user","message":{"content":"hi"}}"#,
                r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Bash","input":{"command":"ls"}}]}}"#,
            ),
        );

        assert!(one_row(file.path(), Kind::Claude, "0:0").is_some());
        // Tool traffic has no full-message page.
        assert_eq!(one_row(file.path(), Kind::Claude, "1:0"), None);
        // Past the end, both ways.
        assert_eq!(one_row(file.path(), Kind::Claude, "9:0"), None);
        assert_eq!(one_row(file.path(), Kind::Claude, "0:1"), None);
        // Not a reference at all.
        for bad in ["", "0", "0:", "01:0", "0:0:0", "x:y"] {
            assert_eq!(one_row(file.path(), Kind::Claude, bad), None, "for {bad:?}");
        }
        // No file, no row.
        assert_eq!(one_row("/nonexistent/transcript.jsonl", Kind::Claude, "0:0"), None);
    }
}
