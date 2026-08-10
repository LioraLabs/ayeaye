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

use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use axum::http::StatusCode;
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio::sync::mpsc;

use ayeaye_core::json;
use ayeaye_core::session::Kind;
use ayeaye_core::transcript;

use crate::config::Settings;
use crate::session;

/// The endpoint this module answers on the plain-JSON chain.
const MESSAGE: &str = "/api/message";

/// How often the tail looks at the file — and, when nothing has landed, how
/// often the stream says `: ping` instead. The daemon's one second.
pub const POLL: Duration = Duration::from_secs(1);

/// How many frames may sit between the tail and the socket.
///
/// Small on purpose: the sender awaits when it is full, so a client that
/// stopped reading stops the tail rather than growing a queue of its
/// transcript in this process's memory.
const IN_FLIGHT: usize = 16;

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

/// How many backlog rows a client is sent when it attaches.
///
/// The daemon's `VOICE_TX_ROWS`, behind the `AYEAYE_` spelling first like
/// every other knob. A value that is not a number falls back to the default
/// where the daemon would have died on import — that is the "do the right
/// thing instead" clause, not a drift.
pub fn backlog_limit(env: impl Fn(&str) -> Option<String>) -> usize {
    for name in ["AYEAYE_TX_ROWS", "VOICE_TX_ROWS"] {
        if let Some(value) = env(name)
            && let Ok(limit) = value.trim().parse()
        {
            return limit;
        }
    }
    transcript::BACKLOG
}

/// The frames of one session's event stream, as they should reach the wire.
///
/// The tail runs as its own task and the frames come back over a bounded
/// channel, which is the disconnect story in one mechanism: when the client
/// goes away the response body is dropped, the receiver with it, the next
/// send fails, and the tail ends. Nothing polls a dead socket forever.
///
/// `poll` and `stop_after` are parameters for the same reason the daemon's
/// `stream_transcript` takes `stop_after`: a test drives this against a file
/// of its own, and a test that needed a wall-clock second per poll would
/// never be run.
pub fn frames(
    kind: Kind,
    id: String,
    path: String,
    backlog: usize,
    poll: Duration,
    stop_after: Option<u32>,
) -> Frames {
    let (sender, receiver) = mpsc::channel(IN_FLIGHT);
    tokio::spawn(tail(kind, id, path, backlog, poll, stop_after, sender));
    Frames(receiver)
}

/// The stream `Body::from_stream` is handed: frames, in order, until the tail
/// ends.
pub struct Frames(mpsc::Receiver<String>);

impl futures_core::Stream for Frames {
    /// Infallible: a frame either arrives or the stream is over. The error
    /// type exists because a body stream must name one.
    type Item = Result<String, std::convert::Infallible>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.0.poll_recv(cx).map(|frame| frame.map(Ok))
    }
}

impl Frames {
    /// The next frame, for a test that wants to watch the stream without a
    /// socket. `None` is the stream ending.
    pub async fn next(&mut self) -> Option<String> {
        self.0.recv().await
    }
}

/// Backlog first, then whatever lands next.
///
/// Tails by byte offset and only ever advances past the last complete line
/// terminator — the ticket's correctness rule, decided in the core's
/// `complete` and obeyed here. An agent mid-write leaves a partial line;
/// advancing past it would parse half a line now and drop the event when the
/// rest landed, so the partial line is read again, whole, on a later poll.
///
/// Every exit is a return: the file going away, the client hanging up, and
/// the test harness's `stop_after` all end the task, the channel closes, and
/// the body ends — which is what tells the browser to reconnect.
async fn tail(
    kind: Kind,
    id: String,
    path: String,
    backlog: usize,
    poll: Duration,
    stop_after: Option<u32>,
    sender: mpsc::Sender<String>,
) {
    let Ok(data) = tokio::fs::read(&path).await else {
        return;
    };
    let mut offset = transcript::complete(&data);
    // Lossily, like every read of an agent's file: one odd byte must cost at
    // most the row it is in.
    let text = String::from_utf8_lossy(&data[..offset]);

    let mut rows = Vec::new();
    let mut next_line = 0;
    for (line, raw) in text.split_terminator('\n').enumerate() {
        for (item, row) in transcript::rows(raw, kind).into_iter().enumerate() {
            rows.push(transcript::row_event(&row, line, item));
        }
        next_line = line + 1;
    }

    let total = rows.len();
    let shown = total.min(backlog);
    if sender
        .send(transcript::init_event(kind, &id, total, shown))
        .await
        .is_err()
    {
        return;
    }
    for event in rows.into_iter().skip(total - shown) {
        if sender.send(event).await.is_err() {
            return;
        }
    }
    if sender.send(transcript::ready_event()).await.is_err() {
        return;
    }

    let mut idle = 0;
    while stop_after.is_none_or(|limit| idle < limit) {
        let Ok(grown) = tokio::fs::metadata(&path).await else {
            // The file went away: a cleared session, a deleted project. The
            // stream ends and the client's reconnect finds out what is true.
            return;
        };
        let size = usize::try_from(grown.len()).unwrap_or(usize::MAX);
        if size > offset {
            let Some(chunk) = read_from(&path, offset, size - offset).await else {
                return;
            };
            let cut = transcript::complete(&chunk);
            if cut > 0 {
                let text = String::from_utf8_lossy(&chunk[..cut]);
                for (line, raw) in text.split_terminator('\n').enumerate() {
                    for (item, row) in transcript::rows(raw, kind).into_iter().enumerate() {
                        let event = transcript::row_event(&row, next_line + line, item);
                        if sender.send(event).await.is_err() {
                            return;
                        }
                    }
                }
                next_line += text.split_terminator('\n').count();
                offset += cut;
            }
            // New bytes are life, complete or not: an agent mid-write is the
            // opposite of an idle stream, so the ping counter starts over.
            idle = 0;
        } else {
            idle += 1;
            if sender.send(transcript::PING.to_string()).await.is_err() {
                return;
            }
        }
        tokio::time::sleep(poll).await;
    }
}

/// `length` bytes of the file starting at `offset`, or `None` if it could not
/// be read.
///
/// A short read is what it is: the tail only ever advances past complete
/// lines, so bytes that did not arrive are bytes a later poll reads again.
async fn read_from(path: &str, offset: usize, length: usize) -> Option<Vec<u8>> {
    let mut file = tokio::fs::File::open(path).await.ok()?;
    file.seek(std::io::SeekFrom::Start(offset as u64))
        .await
        .ok()?;
    let mut chunk = Vec::with_capacity(length);
    file.take(length as u64)
        .read_to_end(&mut chunk)
        .await
        .ok()?;
    Some(chunk)
}

#[cfg(test)]
mod tests {
    use super::{Frames, backlog_limit, frames, one_row};
    use ayeaye_core::session::Kind;
    use ayeaye_core::transcript::PING;
    use std::fs;
    use std::io::Write;
    use std::path::PathBuf;
    use std::time::Duration;

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

        /// More bytes, the way an agent writes them: at the end, as they are.
        fn append(&self, bytes: &str) {
            let mut file = fs::OpenOptions::new()
                .append(true)
                .open(&self.0)
                .expect("the transcript is there");
            file.write_all(bytes.as_bytes()).expect("an append");
        }
    }

    impl Drop for File {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.0);
        }
    }

    /// A tail over this file, polling fast enough for a test to watch.
    fn tailing(file: &File, backlog: usize, stop_after: Option<u32>) -> Frames {
        frames(
            Kind::Claude,
            "0123abcd".to_string(),
            file.path().to_string(),
            backlog,
            Duration::from_millis(10),
            stop_after,
        )
    }

    /// The next frame, or a panic naming the wait: a stream that has stopped
    /// streaming should fail the test rather than hang it.
    async fn next(frames: &mut Frames, waiting_for: &str) -> String {
        tokio::time::timeout(Duration::from_secs(10), frames.next())
            .await
            .unwrap_or_else(|_| panic!("timed out waiting for {waiting_for}"))
            .unwrap_or_else(|| panic!("the stream ended waiting for {waiting_for}"))
    }

    /// The next frame that is not a keep-alive.
    async fn next_event(frames: &mut Frames, waiting_for: &str) -> String {
        loop {
            let frame = next(frames, waiting_for).await;
            if frame != PING {
                return frame;
            }
        }
    }

    fn user_line(text: &str) -> String {
        format!(r#"{{"type":"user","message":{{"content":"{text}"}}}}"#)
    }

    fn user_row_event(text: &str, reference: &str) -> String {
        format!(
            "event: row\ndata: {{\"cls\":\"user\",\"ts\":\"\",\"label\":\"you\",\"text\":\"{text}\",\"ref\":\"{reference}\",\"clipped\":false}}\n\n"
        )
    }

    // AYEAYE-46 — the stream's opening: init says what is coming and how much
    // of the whole it is, the last N rows follow in file order, and ready
    // marks where live begins. One event per row, framed.
    #[tokio::test]
    async fn the_backlog_arrives_framed_and_then_the_stream_goes_live() {
        let file = File::holding(
            "backlog",
            &format!(
                "{}\n{}\n{}\n",
                user_line("one"),
                user_line("two"),
                user_line("three")
            ),
        );
        let mut stream = tailing(&file, 2, None);

        assert_eq!(
            next(&mut stream, "init").await,
            "event: init\ndata: {\"kind\":\"claude\",\"id\":\"0123abcd\",\"total\":3,\"shown\":2}\n\n"
        );
        // The last two, with refs naming their lines in the *file*: the first
        // row fell to the limit, not out of the file.
        assert_eq!(next(&mut stream, "the first shown row").await, user_row_event("two", "1:0"));
        assert_eq!(next(&mut stream, "the second shown row").await, user_row_event("three", "2:0"));
        assert_eq!(next(&mut stream, "ready").await, "event: ready\ndata: {}\n\n");

        // What lands after ready arrives live, numbered by its line.
        file.append(&format!("{}\n", user_line("four")));
        assert_eq!(
            next_event(&mut stream, "the live row").await,
            user_row_event("four", "3:0")
        );
    }

    // AYEAYE-46 — the ticket's correctness rule, watched from outside: an
    // agent mid-write leaves a partial line, and the tail must not consume it
    // until the terminator lands, or the event would be dropped. Until then
    // the stream is idle and says so with comment frames.
    #[tokio::test]
    async fn a_partial_line_is_not_an_event_until_it_is_whole() {
        let file = File::holding("partial", "");
        let mut stream = tailing(&file, 200, None);
        assert!(next(&mut stream, "init").await.starts_with("event: init"));
        assert_eq!(next(&mut stream, "ready").await, "event: ready\ndata: {}\n\n");

        // Half a line: everything the tail sees for a while is a keep-alive.
        let line = user_line("whole in the end");
        let (half, rest) = line.split_at(line.len() / 2);
        file.append(half);
        let deadline = tokio::time::Instant::now() + Duration::from_millis(150);
        while tokio::time::Instant::now() < deadline {
            match tokio::time::timeout(Duration::from_millis(30), stream.next()).await {
                Ok(Some(frame)) => {
                    assert_eq!(frame, PING, "half a line must not become an event");
                }
                Ok(None) => panic!("the stream ended under a partial line"),
                Err(_) => {}
            }
        }

        // The rest lands, and the event is the whole line — nothing dropped.
        file.append(rest);
        file.append("\n");
        assert_eq!(
            next_event(&mut stream, "the completed row").await,
            user_row_event("whole in the end", "0:0")
        );
    }

    // AYEAYE-46 — the keep-alive: an idle stream sends comment frames, and
    // `stop_after` ends the tail so a test can watch the ending too — the
    // channel closes and the stream reports it is over.
    #[tokio::test]
    async fn an_idle_stream_pings_and_an_ended_one_says_so() {
        let file = File::holding("idle", "");
        let mut stream = tailing(&file, 200, Some(2));
        assert!(next(&mut stream, "init").await.starts_with("event: init"));
        assert_eq!(next(&mut stream, "ready").await, "event: ready\ndata: {}\n\n");
        assert_eq!(next(&mut stream, "the first ping").await, PING);
        assert_eq!(next(&mut stream, "the second ping").await, PING);
        assert_eq!(stream.next().await, None, "two idle polls were the limit");
    }

    // AYEAYE-46 — a transcript that is not there is a stream that ends rather
    // than errors: the session endpoint said there was a file, and between
    // that answer and this open the agent may have been cleared away.
    #[tokio::test]
    async fn a_missing_file_is_a_stream_that_ends() {
        let mut stream = frames(
            Kind::Claude,
            "0123abcd".to_string(),
            "/nonexistent/transcript.jsonl".to_string(),
            200,
            Duration::from_millis(10),
            None,
        );
        assert_eq!(stream.next().await, None);
    }

    // AYEAYE-46 — the backlog knob: the AYEAYE_ spelling first, the daemon's
    // VOICE_TX_ROWS second, and a value that is not a number is the default
    // rather than a dead daemon.
    #[test]
    fn the_backlog_limit_reads_the_environment_in_order() {
        assert_eq!(backlog_limit(|_| None), 200);
        assert_eq!(
            backlog_limit(|name| (name == "VOICE_TX_ROWS").then(|| "50".to_string())),
            50
        );
        assert_eq!(
            backlog_limit(|name| match name {
                "AYEAYE_TX_ROWS" => Some("10".to_string()),
                "VOICE_TX_ROWS" => Some("50".to_string()),
                _ => None,
            }),
            10
        );
        assert_eq!(
            backlog_limit(|name| (name == "VOICE_TX_ROWS").then(|| "plenty".to_string())),
            200
        );
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
