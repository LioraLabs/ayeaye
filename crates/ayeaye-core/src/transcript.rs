//! Transcript rows: two agents' JSONL, one shape the panel can draw.
//!
//! The whole row-mapping layer, and nothing but mapping: a line of somebody
//! else's transcript in, zero or more display rows out, plus the framing the
//! event stream and the single-message page put around them. Nothing here
//! opens a file or knows what a socket is — the shell tails the file and hands
//! lines down, which is what lets a test feed this module transcripts nobody
//! ever wrote to disk.
//!
//! Rows are read as untyped [`json::Value`]s rather than typed structs, and
//! that is a decision on the ticket rather than a shortcut: these are two
//! evolving formats whose fields turn up as objects and arrays as well as
//! strings, and a struct would turn every field the agents grow into a parse
//! failure. A line that is not JSON, or is JSON this module does not
//! recognise, maps to no rows — bookkeeping lines outnumber conversation in
//! both formats, and skipping them *is* the correct reading.
//!
//! `bin/ayeaye`'s `rows_claude` / `rows_codex` / `_clip` / `transcript_rows`
//! are the reference. Both agents' formats are the input; matching the
//! daemon's exact output is not the contract, and where the daemon would have
//! crashed on a field of the wrong shape this maps to no row instead.

use crate::json::{self, Value};
use crate::session::Kind;

/// How many characters of one row the stream sends before clipping.
///
/// The daemon's `MAX_BLOCK`. A row longer than this is clipped for the stream
/// and flagged, and the single-message page is where the rest is read.
pub const MAX_BLOCK: usize = 4000;

/// How many past rows to send when a client attaches.
///
/// The daemon's `VOICE_TX_ROWS` default. Everything after the backlog arrives
/// live; the shell may widen or narrow this from the environment.
pub const BACKLOG: usize = 200;

/// The comment frame that keeps an idle stream warm.
///
/// A comment rather than an event: the browser's `EventSource` ignores it, the
/// proxy sees traffic, and a dead link is noticed rather than hung on forever.
pub const PING: &str = ": ping\n\n";

/// One display row: what the panel draws a card from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// The card's class: `user`, `assistant`, `tool`, or `result`.
    pub cls: &'static str,
    /// The clock part of the line's timestamp, `HH:MM:SS`, or empty.
    pub ts: String,
    /// Who or what the row is from: `you`, `claude`, `codex`, `agent`, a tool
    /// name, `result`, or `error`.
    pub label: String,
    /// The row's full text, unclipped. Never a non-string: a value that
    /// arrived as an object or an array is already rendered back to JSON.
    pub text: String,
}

/// Display rows for one JSONL line, unclipped.
///
/// Zero rows is the ordinary answer: bookkeeping lines, empty turns, and text
/// that is not JSON at all are all skipped rather than refused. This is the
/// one entry point for both agents, and `kind` is what decides the reading —
/// never the line's own shape, which the two formats overlap on.
pub fn rows(line: &str, kind: Kind) -> Vec<Row> {
    let Ok(parsed) = json::parse(line) else {
        return Vec::new();
    };
    match kind {
        Kind::Claude => rows_claude(&parsed),
        Kind::Codex => rows_codex(&parsed),
    }
}

/// How a finished background agent reports in: a plain turn, but addressed to
/// the agent rather than sent by you. Left as a user row it becomes "the last
/// thing either of you said", which puts raw markup on the card of every
/// session that delegates.
const TASK_NOTIFICATION: &str = "<task-notification>";

/// Rows for one parsed Claude transcript line.
///
/// Only `user`/`assistant` lines carry conversation; the rest (`summary`,
/// `file-history-snapshot`, …) is bookkeeping. A `user` line holding
/// `tool_result` blocks is tool output echoed back, not something you typed.
fn rows_claude(line: &Value) -> Vec<Row> {
    let cls = match line.get("type").and_then(Value::text) {
        Some(t @ ("user" | "assistant")) => {
            if t == "assistant" { "assistant" } else { "user" }
        }
        _ => return Vec::new(),
    };
    if line.get("isMeta").is_some_and(Value::truthy) {
        return Vec::new();
    }
    let ts = stamp(line);
    let Some(content) = line.get("message").and_then(|m| m.get("content")) else {
        return Vec::new();
    };

    if let Some(text) = content.text() {
        if text.trim().is_empty() {
            return Vec::new();
        }
        // An answer coming back to the agent, so: the same class as any other
        // result. It also means the turn is the agent's again, which is
        // exactly what has just become true.
        if text.trim_start().starts_with(TASK_NOTIFICATION) {
            return vec![row("result", &ts, "agent", text)];
        }
        return vec![row("user", &ts, "you", text)];
    }

    let Value::List(blocks) = content else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for block in blocks {
        match block.get("type").and_then(Value::text) {
            Some("text") => {
                let text = block.get("text").and_then(Value::text).unwrap_or("");
                if !text.trim().is_empty() {
                    let label = if cls == "assistant" { "claude" } else { "you" };
                    out.push(row(cls, &ts, label, text));
                }
            }
            Some("tool_use") => {
                let label = block.get("name").and_then(Value::text).unwrap_or("tool");
                // The daemon renders a missing input as `null`, and so does
                // this: `json.dumps(None)` is what its `rows_claude` does.
                let text = match block.get("input") {
                    Some(Value::Text(text)) => text.clone(),
                    Some(other) => dump(other),
                    None => "null".to_string(),
                };
                out.push(Row {
                    cls: "tool",
                    ts: ts.clone(),
                    label: label.to_string(),
                    text,
                });
            }
            Some("tool_result") => {
                let text = match block.get("content") {
                    Some(Value::List(items)) => joined(items, |item| {
                        Some(item.get("text").and_then(Value::text).unwrap_or(""))
                    }),
                    other => coerced(other),
                };
                let label = if block.get("is_error").is_some_and(Value::truthy) {
                    "error"
                } else {
                    "result"
                };
                out.push(row("result", &ts, label, &text));
            }
            // `thinking` blocks arrive redacted (signature, empty text) —
            // nothing worth showing.
            _ => {}
        }
    }
    out
}

/// Rows for one parsed codex rollout line.
fn rows_codex(line: &Value) -> Vec<Row> {
    let ts = stamp(line);
    let Some(payload) = line.get("payload") else {
        return Vec::new();
    };
    match payload.get("type").and_then(Value::text) {
        Some("agent_message") => match payload.get("message") {
            Some(message) if message.truthy() => {
                vec![row("assistant", &ts, "codex", &coerced(Some(message)))]
            }
            _ => Vec::new(),
        },
        Some("message") => {
            let role = payload.get("role").and_then(Value::text);
            let Some(Value::List(content)) = payload.get("content") else {
                return Vec::new();
            };
            match role {
                Some("assistant") => {
                    // Only the spoken part: reasoning and other block kinds are
                    // filtered out before the join, so they cost no piece.
                    let text = joined(content, |block| {
                        (block.get("type").and_then(Value::text) == Some("output_text"))
                            .then(|| block.get("text").and_then(Value::text).unwrap_or(""))
                    });
                    if text.trim().is_empty() {
                        return Vec::new();
                    }
                    vec![row("assistant", &ts, "codex", &text)]
                }
                Some("user") => {
                    let text = joined(content, |block| {
                        Some(block.get("text").and_then(Value::text).unwrap_or(""))
                    });
                    if text.trim().is_empty() {
                        return Vec::new();
                    }
                    vec![row("user", &ts, "you", &text)]
                }
                _ => Vec::new(),
            }
        }
        Some("function_call" | "custom_tool_call") => {
            let label = payload.get("name").and_then(Value::text).unwrap_or("tool");
            vec![Row {
                cls: "tool",
                ts,
                label: label.to_string(),
                text: coerced(payload.get("arguments")),
            }]
        }
        Some("function_call_output" | "custom_tool_call_output") => {
            let text = match payload.get("output") {
                // Structured output carries its text under `content`; an
                // object with none is shown as the JSON it is.
                Some(output @ Value::Map(_)) => match output.get("content") {
                    Some(content) if content.truthy() => coerced(Some(content)),
                    _ => dump(output),
                },
                other => coerced(other),
            };
            vec![row("result", &ts, "result", &text)]
        }
        _ => Vec::new(),
    }
}

/// One row, spelled once.
fn row(cls: &'static str, ts: &str, label: &str, text: &str) -> Row {
    Row {
        cls,
        ts: ts.to_string(),
        label: label.to_string(),
        text: text.to_string(),
    }
}

/// The clock part of a line's timestamp: characters 11..19 of the ISO-8601
/// text, which is `HH:MM:SS`, or as much of it as is there.
///
/// By characters rather than bytes, and tolerant of a short or missing stamp:
/// this is a label on a card, and a line without one gets an empty label
/// rather than costing the row.
fn stamp(line: &Value) -> String {
    line.get("timestamp")
        .and_then(Value::text)
        .unwrap_or("")
        .chars()
        .skip(11)
        .take(8)
        .collect()
}

/// The text members of a list, joined with spaces.
///
/// The daemon's `" ".join(...)`, with `text_of` standing for its generator's
/// filter: `None` is a block the filter dropped, `Some("")` is a block that
/// passed and contributes an empty piece. The difference is visible — the
/// daemon's tool-result join keeps every object block, so two texted blocks
/// with an untexted one between them end up two spaces apart.
fn joined(items: &[Value], text_of: impl Fn(&Value) -> Option<&str>) -> String {
    items
        .iter()
        .filter(|item| matches!(item, Value::Map(_)))
        .filter_map(text_of)
        .collect::<Vec<_>>()
        .join(" ")
}

/// A value as row text: a string as itself, nothing as nothing, and anything
/// else rendered back to JSON.
///
/// The falsy cases go to empty deliberately — the daemon writes `c or ""`, so
/// `false`, `0` and `[]` are an empty row there and stay one here.
fn coerced(value: Option<&Value>) -> String {
    match value {
        None => String::new(),
        Some(value) if !value.truthy() => String::new(),
        Some(Value::Text(text)) => text.clone(),
        Some(other) => dump(other),
    }
}

/// Render a [`Value`] back to JSON text.
///
/// Private to this module on purpose: `json.rs` is a file every ticket in this
/// milestone reaches for, and rows are the one place a parsed value has to be
/// shown again. The daemon renders with `json.dumps`, whose default separators
/// put a space after `,` and `:` — kept, so a tool call renders the same from
/// either server.
fn dump(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(true) => "true".to_string(),
        Value::Bool(false) => "false".to_string(),
        Value::Number(number) => number_text(*number),
        Value::Text(text) => json::string(text),
        Value::List(items) => {
            let pieces: Vec<String> = items.iter().map(dump).collect();
            format!("[{}]", pieces.join(", "))
        }
        Value::Map(members) => {
            let pieces: Vec<String> = members
                .iter()
                .map(|(name, value)| format!("{}: {}", json::string(name), dump(value)))
                .collect();
            format!("{{{}}}", pieces.join(", "))
        }
    }
}

/// A JSON number as text: whole numbers without a fraction, the way both
/// agents wrote them.
fn number_text(number: f64) -> String {
    if number.fract() == 0.0 && number.abs() < 9e15 {
        format!("{}", number as i64)
    } else {
        format!("{number}")
    }
}

/// A row's text, cut for the stream, and whether it was cut.
///
/// Counted in characters, not bytes: cutting on a byte count could land inside
/// a character, and a panic in the middle of an event stream is too high a
/// price for somebody's emoji.
pub fn clip(text: &str) -> (String, bool) {
    let count = text.chars().count();
    if count <= MAX_BLOCK {
        return (text.to_string(), false);
    }
    let head: String = text.chars().take(MAX_BLOCK).collect();
    (
        format!("{head}\n… ({} more chars)", count - MAX_BLOCK),
        true,
    )
}

/// One `row` event, framed for the stream.
///
/// The ref `line:item` is what the "view full message" link carries back to
/// `/api/message`, so it names the line in the *file*, not the row's place in
/// the stream — a clipped row must stay findable after two hundred more land.
pub fn row_event(row: &Row, line: usize, item: usize) -> String {
    let (text, clipped) = clip(&row.text);
    format!(
        "event: row\ndata: {{\"cls\":{},\"ts\":{},\"label\":{},\"text\":{},\"ref\":{},\"clipped\":{}}}\n\n",
        json::string(row.cls),
        json::string(&row.ts),
        json::string(&row.label),
        json::string(&text),
        json::string(&format!("{line}:{item}")),
        clipped,
    )
}

/// The `init` event: what the client is about to be shown, and how much of the
/// whole that is, so it can say "showing the last N of M".
pub fn init_event(kind: Kind, id: &str, total: usize, shown: usize) -> String {
    format!(
        "event: init\ndata: {{\"kind\":{},\"id\":{},\"total\":{total},\"shown\":{shown}}}\n\n",
        json::string(kind.as_str()),
        json::string(id),
    )
}

/// The `ready` event: the backlog is done, everything after this is live.
pub fn ready_event() -> String {
    "event: ready\ndata: {}\n\n".to_string()
}

/// How many bytes of `data` are complete lines: everything up to and including
/// the last line terminator, or nothing if there is none.
///
/// This is the ticket's correctness rule rather than an optimisation. An agent
/// mid-write leaves a partial line, and a tail that advanced past it would
/// parse half a line now, fail, and *drop the event* when the rest landed —
/// the stream would silently miss a row. Advancing only past the last `\n`
/// means the partial line is read again, whole, on the poll that completes it.
pub fn complete(data: &[u8]) -> usize {
    data.iter()
        .rposition(|&byte| byte == b'\n')
        .map_or(0, |at| at + 1)
}

/// A `line:item` reference, read strictly, or `None`.
///
/// The daemon's `(0|[1-9][0-9]*):(0|[1-9][0-9]*)`, full-match: no signs, no
/// leading zeros, no whitespace, nothing after. The reference arrives from a
/// query string and names a line in a file this server will open, so anything
/// but exactly a reference is refused.
pub fn reference(text: &str) -> Option<(usize, usize)> {
    let (line, item) = text.split_once(':')?;
    Some((counted(line)?, counted(item)?))
}

/// One side of a reference: a canonical decimal number.
fn counted(text: &str) -> Option<usize> {
    if text.is_empty() || (text.len() > 1 && text.starts_with('0')) {
        return None;
    }
    if !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

/// The single-message body: one conversational row, unclipped, or `None` for
/// a row that is not conversation.
///
/// Only `assistant` and `user` rows have a full-message page — tool traffic is
/// already shown whole in the stream, and the daemon answers 404 for it too.
pub fn message_body(row: &Row) -> Option<String> {
    if row.cls != "assistant" && row.cls != "user" {
        return None;
    }
    Some(format!(
        "{{\"cls\":{},\"ts\":{},\"label\":{},\"text\":{}}}",
        json::string(row.cls),
        json::string(&row.ts),
        json::string(&row.label),
        json::string(&row.text),
    ))
}

#[cfg(test)]
mod tests {
    use super::{
        BACKLOG, MAX_BLOCK, PING, Row, clip, complete, init_event, message_body, ready_event,
        reference, row_event, rows,
    };
    use crate::session::Kind;

    fn claude(line: &str) -> Vec<Row> {
        rows(line, Kind::Claude)
    }

    fn codex(line: &str) -> Vec<Row> {
        rows(line, Kind::Codex)
    }

    // AYEAYE-46 — a typed turn and a spoken turn, the two rows a conversation
    // is made of, each with the clock part of its timestamp.
    #[test]
    fn claude_conversation_becomes_user_and_assistant_rows() {
        let typed = claude(
            r#"{"type":"user","timestamp":"2026-08-10T12:34:56.789Z","message":{"content":"fix the bug"}}"#,
        );
        assert_eq!(
            typed,
            vec![Row {
                cls: "user",
                ts: "12:34:56".to_string(),
                label: "you".to_string(),
                text: "fix the bug".to_string(),
            }]
        );

        let spoken = claude(
            r#"{"type":"assistant","timestamp":"2026-08-10T12:35:01.000Z","message":{"content":[{"type":"text","text":"done"}]}}"#,
        );
        assert_eq!(
            spoken,
            vec![Row {
                cls: "assistant",
                ts: "12:35:01".to_string(),
                label: "claude".to_string(),
                text: "done".to_string(),
            }]
        );
    }

    // AYEAYE-46 — bookkeeping is not conversation: other types, meta lines,
    // empty turns, redacted thinking, and text that is not JSON all map to no
    // rows rather than to an error.
    #[test]
    fn claude_bookkeeping_maps_to_no_rows() {
        for line in [
            r#"{"type":"summary","summary":"a title"}"#,
            r#"{"type":"file-history-snapshot","messageId":"x"}"#,
            r#"{"type":"user","isMeta":true,"message":{"content":"caveat: ..."}}"#,
            r#"{"type":"user","message":{"content":"   "}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"thinking","thinking":"","signature":"xx"}]}}"#,
            r#"{"type":"assistant"}"#,
            r#"not json at all"#,
            r#"{"type":"user","message":{"content":[]}}"#,
        ] {
            assert_eq!(claude(line), Vec::new(), "for {line}");
        }
    }

    // AYEAYE-46 — the delegation result. A finished background agent reports
    // in as a plain user turn addressed to the agent; shown as "you" it would
    // put raw markup on the card of every session that delegates, so it is a
    // result from the agent instead.
    #[test]
    fn a_task_notification_is_a_delegation_result_not_a_user_turn() {
        let done = claude(
            r#"{"type":"user","timestamp":"2026-08-10T13:00:00Z","message":{"content":"  <task-notification>agent finished</task-notification>"}}"#,
        );
        assert_eq!(done.len(), 1);
        assert_eq!(done[0].cls, "result");
        assert_eq!(done[0].label, "agent");
        assert_eq!(
            done[0].text,
            "  <task-notification>agent finished</task-notification>",
            "the text is kept whole, markup included — the page renders it"
        );
    }

    // AYEAYE-46 — a tool call and what came back, including the failure case.
    // The input arrives as an object and is rendered back to JSON rather than
    // refused: rows are untyped on purpose.
    #[test]
    fn claude_tool_traffic_becomes_tool_and_result_rows() {
        let called = claude(
            r#"{"type":"assistant","timestamp":"2026-08-10T14:00:00Z","message":{"content":[{"type":"tool_use","name":"Bash","input":{"command":"ls","n":2}}]}}"#,
        );
        assert_eq!(called.len(), 1);
        assert_eq!(called[0].cls, "tool");
        assert_eq!(called[0].label, "Bash");
        assert_eq!(called[0].text, r#"{"command": "ls", "n": 2}"#);

        let answered = claude(
            r#"{"type":"user","message":{"content":[{"type":"tool_result","content":[{"type":"text","text":"file-a"},{"type":"text","text":"file-b"}]}]}}"#,
        );
        assert_eq!(answered.len(), 1);
        assert_eq!(answered[0].cls, "result");
        assert_eq!(answered[0].label, "result");
        assert_eq!(answered[0].text, "file-a file-b");

        let failed = claude(
            r#"{"type":"user","message":{"content":[{"type":"tool_result","is_error":true,"content":"no such file"}]}}"#,
        );
        assert_eq!(failed[0].label, "error");
        assert_eq!(failed[0].text, "no such file");

        // A tool_use with no input renders as the daemon renders it: null.
        let bare = claude(
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Probe"}]}}"#,
        );
        assert_eq!(bare[0].text, "null");
    }

    // AYEAYE-46 — codex's two spellings of a spoken turn, and a typed one.
    #[test]
    fn codex_conversation_becomes_rows() {
        let said = codex(
            r#"{"timestamp":"2026-08-10T15:00:09Z","payload":{"type":"agent_message","message":"hello"}}"#,
        );
        assert_eq!(
            said,
            vec![Row {
                cls: "assistant",
                ts: "15:00:09".to_string(),
                label: "codex".to_string(),
                text: "hello".to_string(),
            }]
        );

        let blocks = codex(
            r#"{"payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"part one"},{"type":"reasoning","text":"hidden"},{"type":"output_text","text":"part two"}]}}"#,
        );
        assert_eq!(blocks.len(), 1);
        assert_eq!(
            blocks[0].text, "part one part two",
            "reasoning blocks are filtered before the join, costing no piece"
        );

        let typed = codex(
            r#"{"payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"do it"}]}}"#,
        );
        assert_eq!(typed[0].cls, "user");
        assert_eq!(typed[0].label, "you");
        assert_eq!(typed[0].text, "do it");
    }

    // AYEAYE-46 — codex tool traffic: the call carries its arguments as they
    // were written, and structured output answers with its content — or, when
    // it has none, with the JSON it is.
    #[test]
    fn codex_tool_traffic_becomes_rows() {
        let called = codex(
            r#"{"payload":{"type":"function_call","name":"shell","arguments":"{\"cmd\":\"ls\"}"}}"#,
        );
        assert_eq!(called[0].cls, "tool");
        assert_eq!(called[0].label, "shell");
        assert_eq!(called[0].text, r#"{"cmd":"ls"}"#);

        let answered = codex(
            r#"{"payload":{"type":"function_call_output","output":{"content":"it ran","status":"ok"}}}"#,
        );
        assert_eq!(answered[0].cls, "result");
        assert_eq!(answered[0].text, "it ran");

        let opaque = codex(r#"{"payload":{"type":"custom_tool_call_output","output":"plain"}}"#);
        assert_eq!(opaque[0].text, "plain");

        let contentless = codex(r#"{"payload":{"type":"function_call_output","output":{"ok":true}}}"#);
        assert_eq!(contentless[0].text, r#"{"ok": true}"#);
    }

    // AYEAYE-46 — codex bookkeeping (session_meta, token_count, …) maps to no
    // rows, and so do empty turns.
    #[test]
    fn codex_bookkeeping_maps_to_no_rows() {
        for line in [
            r#"{"type":"session_meta","payload":{"id":"x","cwd":"/y"}}"#,
            r#"{"payload":{"type":"token_count","total":123}}"#,
            r#"{"payload":{"type":"agent_message","message":""}}"#,
            r#"{"payload":{"type":"message","role":"assistant","content":[{"type":"reasoning","text":"hidden"}]}}"#,
            r#"{"payload":{"type":"message","role":"tool","content":[{"type":"text","text":"x"}]}}"#,
            r#"{}"#,
        ] {
            assert_eq!(codex(line), Vec::new(), "for {line}");
        }
    }

    // AYEAYE-46 — a row longer than MAX_BLOCK is clipped for the stream, told
    // how much is missing, and flagged so the panel offers the full page. The
    // count is characters, not bytes, so a multi-byte transcript cannot panic
    // the stream.
    #[test]
    fn a_long_row_is_clipped_and_flagged() {
        let (kept, cut) = clip("short");
        assert_eq!((kept.as_str(), cut), ("short", false));

        let long = "é".repeat(MAX_BLOCK + 7);
        let (kept, cut) = clip(&long);
        assert!(cut);
        assert_eq!(kept.chars().count(), MAX_BLOCK + "\n… (7 more chars)".chars().count());
        assert!(kept.ends_with("\n… (7 more chars)"));

        // Exactly at the limit is not clipped: nothing would be saved.
        let (_, cut) = clip(&"x".repeat(MAX_BLOCK));
        assert!(!cut);
    }

    // AYEAYE-46 — the stream's frames, exactly as `EventSource` parses them:
    // named event, one data line, blank line. The ref names the line in the
    // file so the full-message link still works after the stream moves on.
    #[test]
    fn the_frames_are_what_an_event_source_parses() {
        let row = Row {
            cls: "user",
            ts: "12:00:00".to_string(),
            label: "you".to_string(),
            text: "hi".to_string(),
        };
        assert_eq!(
            row_event(&row, 41, 1),
            "event: row\ndata: {\"cls\":\"user\",\"ts\":\"12:00:00\",\"label\":\"you\",\"text\":\"hi\",\"ref\":\"41:1\",\"clipped\":false}\n\n"
        );
        assert_eq!(
            init_event(Kind::Codex, "0123abcd", 250, BACKLOG),
            "event: init\ndata: {\"kind\":\"codex\",\"id\":\"0123abcd\",\"total\":250,\"shown\":200}\n\n"
        );
        assert_eq!(ready_event(), "event: ready\ndata: {}\n\n");
        assert_eq!(PING, ": ping\n\n");

        // A row whose text carries a newline must still be one data line.
        let multiline = Row {
            text: "two\nlines".to_string(),
            ..row
        };
        assert!(!row_event(&multiline, 0, 0).contains("\ndata: two"));
    }

    // AYEAYE-46 — the tailing correctness rule: only complete lines are ever
    // consumed. An agent mid-write leaves a partial line, and advancing past
    // it would drop the event when the rest landed.
    #[test]
    fn only_complete_lines_are_consumed() {
        assert_eq!(complete(b"one line\n"), 9);
        assert_eq!(complete(b"one line\npartial"), 9);
        assert_eq!(complete(b"partial"), 0);
        assert_eq!(complete(b""), 0);
        assert_eq!(complete(b"a\nb\nc"), 4);
    }

    // AYEAYE-46 — a reference is exactly `line:item`, no leading zeros, no
    // signs, nothing else: it names a line in a file this server will open.
    #[test]
    fn a_reference_is_read_strictly() {
        assert_eq!(reference("0:0"), Some((0, 0)));
        assert_eq!(reference("41:1"), Some((41, 1)));
        for bad in ["", ":", "1:", ":2", "01:2", "1:02", "-1:2", "1:2:3", "1 :2", "x:y", "1.0:2"] {
            assert_eq!(reference(bad), None, "for {bad:?}");
        }
    }

    // AYEAYE-46 — the single message page shows conversation only, unclipped:
    // tool traffic is already whole in the stream, and the daemon answers 404
    // for it too.
    #[test]
    fn only_conversation_has_a_full_message_page() {
        let long = "x".repeat(MAX_BLOCK + 10);
        let spoken = Row {
            cls: "assistant",
            ts: "12:00:00".to_string(),
            label: "claude".to_string(),
            text: long.clone(),
        };
        let body = message_body(&spoken).expect("conversation has a page");
        assert!(
            body.contains(&long),
            "the page is the unclipped text — that is its whole point"
        );
        assert_eq!(
            body,
            format!(
                "{{\"cls\":\"assistant\",\"ts\":\"12:00:00\",\"label\":\"claude\",\"text\":\"{long}\"}}"
            )
        );

        let tool = Row {
            cls: "tool",
            ts: String::new(),
            label: "Bash".to_string(),
            text: "ls".to_string(),
        };
        assert_eq!(message_body(&tool), None);
    }

    // AYEAYE-46 — a timestamp that is short, missing, or not text labels the
    // row with what there is rather than costing it.
    #[test]
    fn a_broken_timestamp_costs_the_label_not_the_row() {
        let short = claude(r#"{"type":"user","timestamp":"2026-08-10T12","message":{"content":"hi"}}"#);
        assert_eq!(short[0].ts, "12");
        let none = claude(r#"{"type":"user","message":{"content":"hi"}}"#);
        assert_eq!(none[0].ts, "");
        let wrong = claude(r#"{"type":"user","timestamp":123,"message":{"content":"hi"}}"#);
        assert_eq!(wrong[0].ts, "");
    }
}
