//! What an agent session is doing right now, read off the tail of its
//! transcript.
//!
//! Derived from the transcript rather than a hook, so it works the same for
//! claude and codex and needs nothing installed. Nothing here opens a file:
//! the shell reads the tail ([`TAIL_BYTES`] of it) and hands the text over
//! with the clock, which is what lets every decision below be tested without
//! a filesystem or a `now` that moves.
//!
//! Four answers, and the question behind all of them is who the session is
//! waiting on. Its own agents: [`State::Delegating`] — work is happening and
//! none of it is yours. You: [`State::Waiting`], because the last thing in
//! the transcript is the agent speaking, which is how a turn is handed back.
//! A tool it called: [`State::Running`] — the call is out and nothing has
//! come back. Itself: [`State::Working`], thinking with everything it needs
//! already in hand.
//!
//! `Running` is split out of `Working` for the same reason `Delegating` was:
//! both are the session waiting on something that is not you, and lumping
//! them under "working" hides the one number that matters when it goes
//! wrong. A tool call is the one place a session can hang for an hour, and
//! `running` next to `age` says that out loud.
//!
//! There is deliberately no state for "quiet for a while". How long it has
//! been quiet is what `age` is, and the card carries it next to the state: a
//! session that handed you the turn three hours ago is still waiting on you,
//! and calling that idle only hid which of the two it was.

use std::collections::BTreeSet;

use crate::json::{self, Value};
use crate::session::Kind;
use crate::transcript::{self, Row};

/// How much of a transcript's tail is read.
///
/// The daemon's `session_status(sess, tail_bytes=200_000)`. The tail is the
/// horizon, deliberately: a delegation old enough to have scrolled out of it
/// is forgotten — and a session busy enough to push it out is not one sitting
/// still waiting for an answer.
pub const TAIL_BYTES: u64 = 200_000;

/// How much of the last thing said is carried on a card.
pub const LAST_LIMIT: usize = 120;

/// What a card says a session is doing.
///
/// The vocabulary `share/app.html` styles, exactly: a state the page has no
/// label for renders as a bare word.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum State {
    /// A result landed, or you just sent something: the agent has the ball.
    Working,
    /// A tool call is out and nothing has come back.
    Running,
    /// The turn is yours — the agent spoke last, or was never spoken to.
    Waiting,
    /// Waiting on agents of its own, which outranks everything below.
    Delegating,
    /// Stopped at a prompt. Never returned by [`classify`]: a prompt is read
    /// off the screen, not the transcript, and `ayeaye_core::overview` is
    /// where it overrides the transcript's answer.
    Blocked,
    /// The transcript could not be read. Never returned by [`classify`]
    /// either — the shell answers it for a tail it could not fetch. The
    /// spec's third state: distinct from working and from blocked, because
    /// rendering "could not read" as "fine" is the failure this app exists
    /// to prevent.
    Gone,
}

impl State {
    /// The word the wire and the page use.
    pub fn as_str(self) -> &'static str {
        match self {
            State::Working => "working",
            State::Running => "running",
            State::Waiting => "waiting",
            State::Delegating => "delegating",
            State::Blocked => "blocked",
            State::Gone => "gone",
        }
    }

    /// The state this name means, or `None`.
    ///
    /// What `VOICE_NOTIFY_STATES` is read through: an unknown name in the
    /// setting configures nothing, exactly as it matched nothing when the
    /// daemon compared raw strings.
    pub fn of(name: &str) -> Option<State> {
        match name {
            "working" => Some(State::Working),
            "running" => Some(State::Running),
            "waiting" => Some(State::Waiting),
            "delegating" => Some(State::Delegating),
            "blocked" => Some(State::Blocked),
            "gone" => Some(State::Gone),
            _ => None,
        }
    }
}

/// What one session's tail came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    pub state: State,
    /// Seconds since the last thing that actually happened, or `None` for a
    /// transcript that is gone.
    pub age: Option<u64>,
    /// The most recent thing either of you actually said — more useful on a
    /// card than whatever tool happened to run last.
    pub last: String,
}

impl Status {
    /// The transcript could not be read.
    ///
    /// The daemon's `{"state": "gone", "age": None, "last": ""}` — an answer,
    /// not an error, because a session whose file has been cleaned up is an
    /// ordinary state of the world.
    pub fn gone() -> Status {
        Status {
            state: State::Gone,
            age: None,
            last: String::new(),
        }
    }
}

/// Classify one session from the tail of its transcript.
///
/// `chunk` is the tail the shell read, already cut to whole lines; `now` is
/// the clock; `touched` is the file's mtime, used only when no line carries a
/// timestamp of its own. That order is the daemon's hard-won rule: a live
/// claude touches its transcript long after it stops writing to it — four
/// days after, on the machine this was found on — so mtime dated every quiet
/// session to a few minutes ago and floated it to the top of the board. The
/// timestamp on the last thing that actually happened cannot say that.
pub fn classify(chunk: &str, kind: Kind, now: f64, touched: f64) -> Status {
    // Parsed once and read twice: the rows say whose turn it is, and the same
    // lines say what the session is waiting on.
    let mut rows: Vec<Row> = Vec::new();
    let mut delegation = Delegation::new(kind);
    let mut latest: Option<f64> = None;
    for raw in chunk.lines() {
        let Ok(line) = json::parse(raw) else {
            continue;
        };
        let fresh = transcript::rows_of(&line, kind);
        if !fresh.is_empty() {
            // Only a line that carries conversation dates the session. The
            // bookkeeping around it — modes, titles, hook summaries — is
            // written whether or not anything happened, and taking the newest
            // of those is how mtime got it wrong in the first place.
            //
            // Newest rather than last: claude interleaves an attachment line
            // timestamped before the turn it belongs to, and reading in file
            // order would let one of those wind the clock back.
            if let Some(at) = instant(&line)
                && latest.is_none_or(|seen| at > seen)
            {
                latest = Some(at);
            }
        }
        rows.extend(fresh);
        delegation.feed(&line);
    }

    // A clock that disagrees with the transcript must not read as the future.
    let age = (now - latest.unwrap_or(touched)).max(0.0) as u64;

    let state = if delegation.outstanding() {
        State::Delegating // its agents are the holdup
    } else {
        match rows.last().map(|row| row.cls) {
            // Turn handed back, or not begun: a transcript with nothing
            // conversational in it waits on you, because it has not been
            // asked anything.
            None | Some("assistant") => State::Waiting,
            Some("tool") => State::Running, // a call is out, nothing back yet
            _ => State::Working,            // a result landed, or you just sent
        }
    };

    let last = rows
        .iter()
        .rev()
        .find(|row| row.cls == "assistant" || row.cls == "user")
        .map(|row| {
            row.text
                .trim()
                .replace('\n', " ")
                .chars()
                .take(LAST_LIMIT)
                .collect()
        })
        .unwrap_or_default();

    Status {
        state,
        age: Some(age),
        last,
    }
}

/// Claude's subagent tools. `Task` is what `Agent` was called before;
/// transcripts written by either are still on disk and still worth reading.
const CLAUDE_AGENT_TOOLS: &[&str] = &["Agent", "Task"];

/// Codex tools whose entire purpose is to block on a thread it spawned.
const CODEX_WAIT_TOOLS: &[&str] = &["wait_agent", "wait"];

/// What comes back the instant a background agent is launched. It
/// acknowledges the launch and says nothing about the work, so the call that
/// produced it is still outstanding — unlike a synchronous agent, whose
/// result *is* the work.
fn async_launch(text: &str) -> bool {
    text.to_lowercase().contains("async agent launched")
}

/// Whether this session is waiting on agents it launched.
///
/// Fed every line of the tail in order, because pairing a launch with the
/// thing that ends it is the only honest way to answer. Each agent needs a
/// different pairing.
///
/// Claude gets an acknowledgement back the moment it launches a background
/// agent and the real answer much later, as a `<task-notification>` turn —
/// and that turn names the tool call it answers, so any number of agents in
/// flight stay matched up by id.
///
/// Codex blocks in a tool instead, on a short timeout, in a loop: whether a
/// wait call is outstanding *at this instant* flickers on and off every few
/// seconds, which is no use to a card someone is looking at. What does not
/// flicker is which tool it keeps reaching for, so the question asked here is
/// whether the last thing it called was a wait.
struct Delegation {
    kind: Kind,
    /// Claude: launches with nothing back yet.
    pending: BTreeSet<String>,
    /// Codex: the loop above.
    last_call_was_wait: bool,
}

impl Delegation {
    fn new(kind: Kind) -> Delegation {
        Delegation {
            kind,
            pending: BTreeSet::new(),
            last_call_was_wait: false,
        }
    }

    fn feed(&mut self, line: &Value) {
        match self.kind {
            Kind::Claude => self.claude(line),
            Kind::Codex => self.codex(line),
        }
    }

    fn outstanding(&self) -> bool {
        !self.pending.is_empty() || self.last_call_was_wait
    }

    fn claude(&mut self, line: &Value) {
        if !matches!(
            line.get("type").and_then(Value::text),
            Some("user" | "assistant")
        ) {
            return;
        }
        let Some(content) = line
            .get("message")
            .and_then(|message| message.get("content"))
        else {
            return;
        };
        if let Some(text) = content.text() {
            // A plain turn, which is the only thing a completion arrives as.
            // Tool output is a list of blocks and never reaches here, which is
            // what stops a transcript that merely quotes a notification —
            // this suite greps for them — from cancelling a live one.
            for id in notified_ids(text) {
                self.pending.remove(id);
            }
            return;
        }
        let Value::List(blocks) = content else {
            return;
        };
        for block in blocks {
            match block.get("type").and_then(Value::text) {
                Some("tool_use") => {
                    let name = block.get("name").and_then(Value::text).unwrap_or("");
                    if CLAUDE_AGENT_TOOLS.contains(&name)
                        && let Some(id) = block.get("id").and_then(Value::text)
                        && !id.is_empty()
                    {
                        self.pending.insert(id.to_string());
                    }
                }
                Some("tool_result") => {
                    let tid = block.get("tool_use_id").and_then(Value::text).unwrap_or("");
                    if self.pending.contains(tid) && !async_launch(&result_text(block)) {
                        // A synchronous agent, returned.
                        self.pending.remove(tid);
                    }
                }
                _ => {}
            }
        }
    }

    fn codex(&mut self, line: &Value) {
        let Some(payload) = line.get("payload") else {
            return;
        };
        if !matches!(
            payload.get("type").and_then(Value::text),
            Some("function_call" | "custom_tool_call")
        ) {
            return;
        }
        let name = payload.get("name").and_then(Value::text).unwrap_or("");
        self.last_call_was_wait = CODEX_WAIT_TOOLS.contains(&name);
    }
}

/// The text of a `tool_result` block, whichever shape it arrived in.
fn result_text(block: &Value) -> String {
    match block.get("content") {
        Some(Value::List(items)) => items
            .iter()
            .filter(|item| matches!(item, Value::Map(_)))
            .map(|item| item.get("text").and_then(Value::text).unwrap_or(""))
            .collect::<Vec<_>>()
            .join(" "),
        Some(Value::Text(text)) => text.clone(),
        _ => String::new(),
    }
}

/// How the real answer arrives, minutes or hours later: an ordinary user turn
/// naming the tool call it belongs to.
///
/// The daemon's `<tool-use-id>([^<>]+)</tool-use-id>`, hand-rolled because
/// there is no regex crate below the shell and there must not be one: an id
/// carrying an angle bracket is not an id, and the scan resumes after the
/// opening tag exactly as the regex engine would.
fn notified_ids(text: &str) -> Vec<&str> {
    const OPEN: &str = "<tool-use-id>";
    const CLOSE: &str = "</tool-use-id>";
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find(OPEN) {
        rest = &rest[start + OPEN.len()..];
        let Some(end) = rest.find(CLOSE) else {
            break;
        };
        let id = &rest[..end];
        if !id.is_empty() && !id.contains(['<', '>']) {
            out.push(id);
            rest = &rest[end + CLOSE.len()..];
        }
        // A bracket inside means no match starts at that opening tag; the
        // loop continues from just past it, where a nested tag may still
        // begin a real one.
    }
    out
}

/// When a transcript line says it happened, in epoch seconds, or `None`.
fn instant(line: &Value) -> Option<f64> {
    epoch(line.get("timestamp").and_then(Value::text)?)
}

/// An ISO-8601 UTC stamp's first nineteen characters as epoch seconds.
///
/// Both agents write `YYYY-MM-DDTHH:MM:SS` with a fractional part and a
/// trailing `Z`; only the whole seconds are read, as the daemon's
/// `strptime(ts[:19], ...)` reads them. Parsed as UTC rather than handed to
/// the local zone: this is compared against the clock, and a transcript
/// written in one timezone and read in another would otherwise be hours out —
/// in whichever direction made the session look freshest.
pub fn epoch(ts: &str) -> Option<f64> {
    let bytes = ts.as_bytes();
    if bytes.len() < 19 {
        return None;
    }
    let bytes = &bytes[..19];
    if bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
    {
        return None;
    }
    let number = |range: core::ops::Range<usize>| -> Option<i64> {
        let piece = &bytes[range];
        if !piece.iter().all(u8::is_ascii_digit) {
            return None;
        }
        core::str::from_utf8(piece).ok()?.parse().ok()
    };
    let year = number(0..4)?;
    let month = number(5..7)?;
    let day = number(8..10)?;
    let hour = number(11..13)?;
    let minute = number(14..16)?;
    let second = number(17..19)?;
    if !(1..=12).contains(&month)
        || day < 1
        || day > days_in_month(year, month)
        || hour > 23
        || minute > 59
        // 60 for a leap second, which `timegm` also accepts.
        || second > 60
    {
        return None;
    }
    Some((days_from_civil(year, month, day) * 86_400 + hour * 3_600 + minute * 60 + second) as f64)
}

fn leap_year(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap_year(year) => 29,
        _ => 28,
    }
}

/// Days since 1970-01-01 for a civil date. Howard Hinnant's `days_from_civil`,
/// which is exact over the whole proleptic Gregorian calendar.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let day_of_year = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

#[cfg(test)]
mod tests {
    use super::{LAST_LIMIT, State, Status, classify, epoch};
    use crate::session::Kind;

    /// 2026-03-04T09:00:00Z, checked against `date -u +%s` rather than
    /// against `epoch`, so the two can disagree.
    const NINE: f64 = 1_772_614_800.0;

    fn classified(lines: &[String], kind: Kind, now: f64, touched: f64) -> Status {
        classify(&lines.join("\n"), kind, now, touched)
    }

    fn state(lines: &[String]) -> State {
        classified(lines, Kind::Claude, NINE + 60.0, NINE).state
    }

    // ------------------------------------------------- transcript writing

    /// One claude line carrying content blocks, stamped at 09:00 unless told
    /// otherwise.
    fn claude(kind: &str, blocks: &str, ts: Option<&str>) -> String {
        let stamp = match ts {
            Some(ts) => format!(r#""timestamp":"{ts}","#),
            None => String::new(),
        };
        format!(r#"{{"type":"{kind}",{stamp}"message":{{"role":"{kind}","content":{blocks}}}}}"#)
    }

    fn said(kind: &str, text: &str) -> String {
        said_at(kind, text, Some("2026-03-04T09:00:00.000Z"))
    }

    fn said_at(kind: &str, text: &str, ts: Option<&str>) -> String {
        claude(kind, &format!(r#"[{{"type":"text","text":"{text}"}}]"#), ts)
    }

    fn launched(tool_id: &str, name: &str) -> String {
        claude(
            "assistant",
            &format!(
                r#"[{{"type":"tool_use","id":"{tool_id}","name":"{name}","input":{{"description":"go and look"}}}}]"#
            ),
            Some("2026-03-04T09:00:00.000Z"),
        )
    }

    /// What comes straight back from a background launch: not the work.
    fn acknowledged(tool_id: &str) -> String {
        claude(
            "user",
            &format!(
                r#"[{{"type":"tool_result","tool_use_id":"{tool_id}","content":[{{"type":"text","text":"Async agent launched successfully. (This tool result is internal metadata — never quote or paste any part of it)"}}]}}]"#
            ),
            Some("2026-03-04T09:00:00.000Z"),
        )
    }

    /// What a synchronous agent gives back: the work itself.
    fn returned(tool_id: &str, text: &str) -> String {
        claude(
            "user",
            &format!(
                r#"[{{"type":"tool_result","tool_use_id":"{tool_id}","content":[{{"type":"text","text":"{text}"}}]}}]"#
            ),
            Some("2026-03-04T09:00:00.000Z"),
        )
    }

    /// The completion turn, hours later, naming the call it answers.
    fn notified(tool_id: &str) -> String {
        format!(
            r#"{{"type":"user","timestamp":"2026-03-04T11:00:00.000Z","message":{{"role":"user","content":"<task-notification> <task-id>a711c9edb2fa4fd7c</task-id> <tool-use-id>{tool_id}</tool-use-id> <output-file>/tmp/x/a711c9edb2fa4fd7c.output</output-file>"}}}}"#
        )
    }

    fn tool_use(tool_id: &str, name: &str) -> String {
        format!(r#"{{"type":"tool_use","id":"{tool_id}","name":"{name}","input":{{}}}}"#)
    }

    fn codex(payload: &str) -> String {
        format!(r#"{{"timestamp":"2026-03-04T09:00:00.000Z","payload":{payload}}}"#)
    }

    fn call(name: &str, call_id: &str) -> String {
        codex(&format!(
            r#"{{"type":"function_call","name":"{name}","call_id":"{call_id}","arguments":"{{}}"}}"#
        ))
    }

    fn output(call_id: &str, out: &str) -> String {
        codex(&format!(
            r#"{{"type":"function_call_output","call_id":"{call_id}","output":{out}}}"#
        ))
    }

    // ------------------------------------------------------------- claude
    // Transcribed from tests/python/session_state.py rather than derived
    // from the port, so the two can disagree.

    // AYEAYE-49 — whose turn it is comes from the last thing in the
    // transcript: the agent spoke last, so the turn is yours.
    #[test]
    fn an_agent_that_spoke_last_is_waiting_on_you() {
        let lines = [
            said("user", "have a look at the parser"),
            said("assistant", "done — two bugs, both in the lexer"),
        ];
        assert_eq!(state(&lines), State::Waiting);
    }

    // AYEAYE-49 — the call is out and nothing has come back: the holdup is
    // the tool, not the model, which is the difference `age` then makes
    // readable.
    #[test]
    fn an_agent_mid_tool_call_is_running() {
        let lines = [
            said("user", "have a look at the parser"),
            claude(
                "assistant",
                &format!("[{}]", tool_use("t1", "Bash")),
                Some("2026-03-04T09:00:00.000Z"),
            ),
        ];
        assert_eq!(state(&lines), State::Running);
    }

    // AYEAYE-49 — the agent has the ball back and has not spoken yet.
    #[test]
    fn a_tool_result_hands_the_work_back_to_the_agent() {
        let lines = [
            claude(
                "assistant",
                &format!("[{}]", tool_use("t1", "Bash")),
                Some("2026-03-04T09:00:00.000Z"),
            ),
            returned("t1", "ok"),
        ];
        assert_eq!(state(&lines), State::Working);
    }

    // AYEAYE-49 — parallel calls: one message, several tool_use blocks, and
    // the results back together. Anything outstanding means a tool is
    // running.
    #[test]
    fn the_last_call_of_a_batch_decides_it() {
        let batch = claude(
            "assistant",
            &format!("[{},{}]", tool_use("t1", "Read"), tool_use("t2", "Grep")),
            Some("2026-03-04T09:00:00.000Z"),
        );
        assert_eq!(state(std::slice::from_ref(&batch)), State::Running);
        let answered = [
            batch,
            claude(
                "user",
                r#"[{"type":"tool_result","tool_use_id":"t1","content":"ok"},{"type":"tool_result","tool_use_id":"t2","content":"ok"}]"#,
                Some("2026-03-04T09:00:00.000Z"),
            ),
        ];
        assert_eq!(state(&answered), State::Working);
    }

    // AYEAYE-49 — your own message is the agent's turn.
    #[test]
    fn what_you_just_sent_is_the_agents_turn() {
        let lines = [
            said("assistant", "anything else?"),
            said("user", "yes — ship it"),
        ];
        assert_eq!(state(&lines), State::Working);
    }

    // AYEAYE-49 — started, never spoken to. The turn has to be someone's, and
    // it is not the agent's: it has not been asked anything.
    #[test]
    fn a_transcript_with_nothing_in_it_waits_on_you() {
        let lines = [r#"{"type":"mode","mode":"normal"}"#.to_string()];
        assert_eq!(state(&lines), State::Waiting);
    }

    // AYEAYE-49 — the spec's third state, as a value: distinct from working
    // and from blocked, with no age to sort by.
    #[test]
    fn a_transcript_that_is_gone_is_gone() {
        let gone = Status::gone();
        assert_eq!(gone.state, State::Gone);
        assert_eq!(gone.age, None);
        assert_eq!(gone.last, "");
    }

    // AYEAYE-49 — the whole point of dropping idle: a turn handed back a week
    // ago is still a turn handed back, and the age says how long.
    #[test]
    fn the_age_is_reported_however_old_and_never_becomes_a_state() {
        let lines = [said_at(
            "assistant",
            "done",
            Some("2026-03-04T09:00:00.000Z"),
        )];
        let got = classified(&lines, Kind::Claude, NINE + 7.0 * 86_400.0, NINE);
        assert_eq!(got.state, State::Waiting);
        assert_eq!(got.age, Some(7 * 86_400));
    }

    // ---------------------------------------------------------------- age

    // AYEAYE-49 — the conversation dates the session, not the file: a live
    // claude touches its transcript long after it stops writing to it, so a
    // fresh mtime under an old turn must not make the session look fresh.
    #[test]
    fn the_conversation_dates_the_session_not_the_file() {
        let lines = [said("assistant", "done")];
        let touched_now = NINE + 7_200.0;
        let got = classified(&lines, Kind::Claude, NINE + 7_200.0, touched_now);
        assert_eq!(got.age, Some(7_200));
    }

    // AYEAYE-49 — modes, titles, hook summaries: written whether or not
    // anything happened, and none of them is anything happening.
    #[test]
    fn bookkeeping_cannot_make_an_old_session_look_fresh() {
        let mut lines = vec![said("assistant", "done")];
        for kind in ["mode", "ai-title", "system", "file-history-snapshot"] {
            lines.push(format!(
                r#"{{"type":"{kind}","timestamp":"2026-03-04T11:00:00.000Z"}}"#
            ));
        }
        let got = classified(&lines, Kind::Claude, NINE + 7_200.0, NINE + 7_200.0);
        assert_eq!(got.age, Some(7_200));
    }

    // AYEAYE-49 — claude interleaves lines timestamped before the turn they
    // belong to; reading in file order would let one of those wind the clock
    // back.
    #[test]
    fn the_newest_turn_dates_it_rather_than_the_last_line() {
        let lines = [
            said_at("assistant", "done", Some("2026-03-04T10:59:00.000Z")),
            said_at(
                "user",
                "an earlier line, written later",
                Some("2026-03-04T09:00:00.000Z"),
            ),
        ];
        let got = classified(&lines, Kind::Claude, NINE + 7_200.0, NINE);
        assert_eq!(got.age, Some(60));
    }

    // AYEAYE-49 — a tail carrying no timestamp falls back to the file's
    // mtime: the only answer left, and better than none.
    #[test]
    fn a_tail_carrying_no_timestamp_falls_back_to_the_file() {
        let lines = [said_at("assistant", "done", None)];
        let got = classified(&lines, Kind::Claude, NINE + 1_800.0, NINE);
        assert_eq!(got.age, Some(1_800));
    }

    // AYEAYE-49 — a clock behind the transcript does not read as the future.
    #[test]
    fn a_clock_behind_the_transcript_does_not_read_as_the_future() {
        let lines = [said_at(
            "assistant",
            "done",
            Some("2026-03-04T11:00:00.000Z"),
        )];
        let got = classified(&lines, Kind::Claude, NINE, NINE);
        assert_eq!(got.age, Some(0));
    }

    // AYEAYE-49 — a codex rollout is dated the same way.
    #[test]
    fn a_codex_rollout_is_dated_the_same_way() {
        let lines = [codex(r#"{"type":"agent_message","message":"shipped"}"#)];
        let got = classified(&lines, Kind::Codex, NINE + 4_000.0, NINE + 4_000.0);
        assert_eq!(got.age, Some(4_000));
        assert_eq!(got.state, State::Waiting);
    }

    // ----------------------------------------------------------- delegation

    // AYEAYE-49 — a launched agent is outstanding, and outranks the tool call
    // that launched it: what the session is waiting on is the agent.
    #[test]
    fn a_launched_agent_is_outstanding() {
        let lines = [
            said("user", "explore both of these"),
            launched("toolu_A", "Agent"),
        ];
        assert_eq!(state(&lines), State::Delegating);
    }

    // AYEAYE-49 — the acknowledgement comes back in milliseconds and says
    // only that the agent started. Treating it as the result is how the
    // orchestrator went quiet.
    #[test]
    fn the_launch_acknowledgement_is_not_the_answer() {
        let lines = [launched("toolu_A", "Agent"), acknowledged("toolu_A")];
        assert_eq!(state(&lines), State::Delegating);
    }

    // AYEAYE-49 — "I've launched two explorers, I'll report back": the agent
    // spoke last, but you are not what it is waiting on.
    #[test]
    fn handing_the_turn_back_does_not_end_the_delegation() {
        let lines = [
            launched("toolu_A", "Agent"),
            acknowledged("toolu_A"),
            said("assistant", "two explorers are running; standing by"),
        ];
        assert_eq!(state(&lines), State::Delegating);
    }

    // AYEAYE-49 — the notification ends it, into working rather than
    // waiting: the answer has just landed on the agent's side of the table.
    #[test]
    fn the_notification_ends_it() {
        let mut lines = vec![
            launched("toolu_A", "Agent"),
            acknowledged("toolu_A"),
            said("assistant", "two explorers are running; standing by"),
            notified("toolu_A"),
        ];
        assert_eq!(state(&lines), State::Working);
        lines.push(said("assistant", "both reported — here is the plan"));
        assert_eq!(state(&lines), State::Waiting);
    }

    // AYEAYE-49 — a notification is addressed to the agent. Left classed as
    // your turn, the card's "last thing either of you said" is a lump of
    // markup.
    #[test]
    fn a_notification_is_not_something_you_said() {
        let lines = [
            said("user", "explore both of these"),
            launched("toolu_A", "Agent"),
            acknowledged("toolu_A"),
            notified("toolu_A"),
        ];
        let got = classified(&lines, Kind::Claude, NINE + 60.0, NINE);
        assert_eq!(got.last, "explore both of these");
    }

    // AYEAYE-49 — agents are paired by id, so one answer is not all of them.
    #[test]
    fn agents_are_paired_by_id_so_one_answer_is_not_all_of_them() {
        let mut lines = vec![
            launched("toolu_A", "Agent"),
            launched("toolu_B", "Agent"),
            acknowledged("toolu_A"),
            acknowledged("toolu_B"),
            notified("toolu_A"),
        ];
        assert_eq!(state(&lines), State::Delegating);
        lines.push(notified("toolu_B"));
        assert_eq!(state(&lines), State::Working);
    }

    // AYEAYE-49 — no notification is coming for a synchronous agent: the
    // result IS the work.
    #[test]
    fn a_synchronous_agent_is_done_when_its_result_lands() {
        let mut lines = vec![launched("toolu_A", "Agent")];
        assert_eq!(state(&lines), State::Delegating);
        lines.push(returned("toolu_A", "found it in the lexer"));
        assert_eq!(state(&lines), State::Working);
    }

    // AYEAYE-49 — the older name for the tool counts too.
    #[test]
    fn the_older_name_for_the_tool_counts_too() {
        let lines = [launched("toolu_A", "Task")];
        assert_eq!(state(&lines), State::Delegating);
    }

    // AYEAYE-49 — an ordinary tool is running — a tool it is waiting on —
    // and not delegating, which is agents of its own.
    #[test]
    fn an_ordinary_tool_is_not_a_delegation() {
        let lines = [claude(
            "assistant",
            &format!("[{}]", tool_use("t1", "Bash")),
            Some("2026-03-04T09:00:00.000Z"),
        )];
        assert_eq!(state(&lines), State::Running);
    }

    // AYEAYE-49 — tool output is blocks, never a plain turn, which is what
    // keeps a session reading its own logs from cancelling a live
    // delegation.
    #[test]
    fn a_transcript_that_merely_quotes_a_notification_ends_nothing() {
        let lines = [
            launched("toolu_A", "Agent"),
            acknowledged("toolu_A"),
            returned("t9", "grep found: <tool-use-id>toolu_A</tool-use-id>"),
        ];
        assert_eq!(state(&lines), State::Delegating);
    }

    // ------------------------------------------------------------- codex

    // AYEAYE-49 — codex: the agent spoke last, so the turn is yours.
    #[test]
    fn a_codex_that_spoke_last_is_waiting_on_you() {
        let lines = [codex(
            r#"{"type":"agent_message","message":"that's landed on main"}"#,
        )];
        assert_eq!(
            classified(&lines, Kind::Codex, NINE + 60.0, NINE).state,
            State::Waiting
        );
    }

    // AYEAYE-49 — a codex mid tool call is running, and the output hands the
    // work back.
    #[test]
    fn a_codex_call_runs_and_its_output_hands_the_work_back() {
        let mut lines = vec![call("exec", "call_1")];
        assert_eq!(
            classified(&lines, Kind::Codex, NINE + 60.0, NINE).state,
            State::Running
        );
        lines.push(output("call_1", r#""ok""#));
        assert_eq!(
            classified(&lines, Kind::Codex, NINE + 60.0, NINE).state,
            State::Working
        );
    }

    // AYEAYE-49 — waiting on a spawned thread is delegating.
    #[test]
    fn waiting_on_a_spawned_thread_is_delegating() {
        let lines = [
            call("spawn_agent", "call_1"),
            output("call_1", r#""{\"task_name\":\"/root/review\"}""#),
            call("wait_agent", "call_2"),
        ];
        assert_eq!(
            classified(&lines, Kind::Codex, NINE + 60.0, NINE).state,
            State::Delegating
        );
    }

    // AYEAYE-49 — wait_agent returns on a 1s timeout and is called straight
    // back. The gap between the two is not the agent going back to work.
    #[test]
    fn the_wait_loop_does_not_flicker_between_polls() {
        let lines = [
            call("wait_agent", "call_1"),
            output("call_1", r#""{\"timed_out\":true}""#),
        ];
        assert_eq!(
            classified(&lines, Kind::Codex, NINE + 60.0, NINE).state,
            State::Delegating
        );
    }

    // AYEAYE-49 — going back to work ends it.
    #[test]
    fn going_back_to_work_ends_it() {
        let lines = [
            call("wait_agent", "call_1"),
            output("call_1", r#""{\"task_name\":\"/root/review\"}""#),
            call("exec", "call_2"),
        ];
        assert_eq!(
            classified(&lines, Kind::Codex, NINE + 60.0, NINE).state,
            State::Running
        );
    }

    // AYEAYE-49 — same rule as claude: an orchestrator narrating between
    // waits is not waiting on you.
    #[test]
    fn speaking_after_a_wait_still_reads_as_delegating() {
        let lines = [
            call("wait_agent", "call_1"),
            output("call_1", r#""{\"timed_out\":true}""#),
            codex(r#"{"type":"agent_message","message":"both reviewers still going"}"#),
        ];
        assert_eq!(
            classified(&lines, Kind::Codex, NINE + 60.0, NINE).state,
            State::Delegating
        );
    }

    // AYEAYE-49 — between spawning and waiting, codex is doing its own work.
    #[test]
    fn spawning_alone_is_not_waiting() {
        let lines = [
            call("spawn_agent", "call_1"),
            output("call_1", r#""{\"task_name\":\"/root/review\"}""#),
        ];
        assert_eq!(
            classified(&lines, Kind::Codex, NINE + 60.0, NINE).state,
            State::Working
        );
    }

    // ------------------------------------------------------------- the rest

    // AYEAYE-49 — the last thing said is clipped for the card, newlines to
    // spaces, and comes from conversation rather than tool traffic.
    #[test]
    fn the_last_words_are_clipped_and_flattened() {
        let long = "x".repeat(LAST_LIMIT + 40);
        let lines = [
            said("assistant", &format!("one\\ntwo {long}")),
            claude(
                "assistant",
                &format!("[{}]", tool_use("t1", "Bash")),
                Some("2026-03-04T09:00:00.000Z"),
            ),
        ];
        let got = classified(&lines, Kind::Claude, NINE + 60.0, NINE);
        assert!(got.last.starts_with("one two x"));
        assert_eq!(got.last.chars().count(), LAST_LIMIT);
    }

    // AYEAYE-49 — the vocabulary, as a whole: a state the client has no label
    // for renders as a bare word, so the page's label table has to carry
    // every state this can send — and `idle` has to be gone from both.
    #[test]
    fn every_state_the_server_can_send_has_a_label() {
        let app = include_str!("../../../../share/app.html");
        let labels = app
            .split("const label = {")
            .nth(1)
            .expect("the label table")
            .split("};")
            .next()
            .expect("the table ends");
        for state in [
            State::Working,
            State::Running,
            State::Waiting,
            State::Delegating,
            State::Blocked,
            State::Gone,
        ] {
            assert!(
                labels.contains(&format!("{}:", state.as_str())),
                "no label for {}",
                state.as_str()
            );
            assert_eq!(State::of(state.as_str()), Some(state));
        }
        assert!(!labels.contains("idle"), "idle is gone from the client");
        assert_eq!(State::of("idle"), None, "idle is gone from the server");
    }

    // AYEAYE-49 — the stamp is read as UTC, against values `date -u +%s`
    // computed, so the arithmetic disagrees with the code rather than
    // agreeing by construction.
    #[test]
    fn a_stamp_is_epoch_seconds_in_utc() {
        assert_eq!(epoch("2026-03-04T09:00:00.000Z"), Some(1_772_614_800.0));
        assert_eq!(epoch("2026-03-04T11:00:00.000Z"), Some(1_772_622_000.0));
        assert_eq!(epoch("1970-01-01T00:00:00.000Z"), Some(0.0));
        // A leap year's extra day, through the civil-date arithmetic.
        assert_eq!(epoch("2000-02-29T12:30:45.000Z"), Some(951_827_445.0));
        // And what is not a stamp answers nothing rather than something.
        for bad in [
            "",
            "2026-03-04",
            "2026-03-04 09:00:00",
            "2026-13-04T09:00:00Z",
            "2026-02-30T09:00:00Z",
            "2026-03-04T24:00:00Z",
            "not a stamp at all!!",
        ] {
            assert_eq!(epoch(bad), None, "{bad:?} is not a stamp");
        }
    }
}
