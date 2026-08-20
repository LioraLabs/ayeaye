//! The overview endpoint's effectful half: the pane list, each pane's
//! transcript tail, and each pane's screen, gathered and handed to the core.
//!
//! What any of it *means* — which state wins, which pane sorts first, what
//! the body says — is `ayeaye_core::overview`'s. This module is the reading:
//! one `list-panes`, then per pane the session resolution, a bounded tail of
//! its transcript, and one `capture-pane` for the prompt. That is the
//! daemon's own cost per poll, and `Agents::behind` was put on the blocking
//! pool in advance of exactly this caller.

use std::io::Read;
use std::time::SystemTime;

use ayeaye_core::overview::{self, Card};
use ayeaye_core::prompt;
use ayeaye_core::session::Session;
use ayeaye_core::session::status::{self, Status, TAIL_BYTES};
use ayeaye_core::tmux::Pane;

use crate::config::Settings;
use crate::tmux::Trouble;

/// The `/api/overview` body.
///
/// A tmux that could not be asked is an empty list *carrying the reason*, at
/// 200 — the same shape `/api/panes` answers with, and for the same reason:
/// the panel polls this and has to keep rendering, and what it must never do
/// is show an empty board that means "nothing needs you" when the truth is
/// "I could not look".
pub async fn body(settings: &Settings) -> String {
    let here = settings.peers.here().name();
    // The one capability probe, cached behind its TTL — the daemon reads
    // `voice_available()` here for the same field, and a second probe would
    // be a second answer to disagree with `/api/voice`.
    let voice = settings.voice.probe().await.ok();
    match cards(settings).await {
        Ok(cards) => overview::body(here, &cards, voice, None),
        Err(trouble) => {
            eprintln!("ayeaye: {trouble}");
            overview::body(here, &[], voice, Some(&trouble.to_string()))
        }
    }
}

/// Every pane's card, ordered so the ones needing you come first.
///
/// Public to the crate for the notification watcher, which sweeps the same
/// board on a clock: one assembly means the panel and the notifier cannot
/// disagree about what a pane is doing.
pub(crate) async fn cards(settings: &Settings) -> Result<Vec<Card>, Trouble> {
    let here = settings.peers.here().name();
    let panes = settings.tmux.panes(here).await?;
    let mut cards = Vec::with_capacity(panes.len());
    for pane in panes {
        cards.push(card(settings, pane).await);
    }
    overview::order(&mut cards);
    Ok(cards)
}

/// One pane's three answers, gathered.
async fn card(settings: &Settings, pane: Pane) -> Card {
    let session = match settings.agents.behind(&settings.tmux, &pane).await {
        Some(found) => {
            let status = classified(&found).await;
            Some((found.kind, found.id, status))
        }
        None => None,
    };
    // Checked for every pane, not just resolved agents: anything can stop to
    // ask. A capture that failed is no prompt rather than a refusal — the
    // daemon's answer too — and the pane's own state still says what the
    // transcript knows.
    let prompt = match settings.tmux.capture(&pane).await {
        Ok(screen) => prompt::read(&screen),
        Err(trouble) => {
            eprintln!("ayeaye: {trouble}");
            None
        }
    };
    Card::of(pane, session, prompt)
}

/// One session's status, off the tail of its transcript.
///
/// On the blocking pool: the read is file IO and the classification walks up
/// to [`TAIL_BYTES`] of text, and this runs once per agent pane per poll.
async fn classified(session: &Session) -> Status {
    let path = session.path.clone();
    let kind = session.kind;
    tokio::task::spawn_blocking(move || match tail(&path, TAIL_BYTES) {
        Some((chunk, touched)) => status::classify(&chunk, kind, now(), touched),
        // The spec's third state: a transcript that could not be read is
        // `gone`, distinct from working and from blocked, because rendering
        // "could not read" as "fine" is the failure this app exists to
        // prevent.
        None => Status::gone(),
    })
    .await
    .unwrap_or_else(|_| Status::gone())
}

/// The last `tail_bytes` of a file as text, and the file's mtime.
///
/// A tail that starts mid-file discards its partial first line, as the
/// daemon's `fh.readline()` does after the seek: half a JSON line parses as
/// nothing and must not cost the line it collided with. Lossily decoded, so
/// one odd byte in one agent's file costs that byte rather than the whole
/// answer. `None` is "could not read", never an error.
fn tail(path: &str, tail_bytes: u64) -> Option<(String, f64)> {
    use std::io::{BufRead, BufReader, Seek, SeekFrom};

    let meta = std::fs::metadata(path).ok()?;
    let size = meta.len();
    let touched = meta
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs_f64();
    let mut file = std::fs::File::open(path).ok()?;
    let mut bytes = Vec::new();
    if size > tail_bytes {
        file.seek(SeekFrom::Start(size - tail_bytes)).ok()?;
        let mut reader = BufReader::new(file);
        let mut partial = Vec::new();
        reader.read_until(b'\n', &mut partial).ok()?;
        reader.read_to_end(&mut bytes).ok()?;
    } else {
        file.read_to_end(&mut bytes).ok()?;
    }
    Some((String::from_utf8_lossy(&bytes).into_owned(), touched))
}

/// The clock, as the seconds `classify` compares stamps against.
fn now() -> f64 {
    SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_secs_f64())
        .unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::tail;
    use ayeaye_core::session::Kind;
    use ayeaye_core::session::status::{State, Status, classify};
    use std::fs;
    use std::path::PathBuf;

    /// A transcript of one's own, cleaned up on drop.
    struct Scratch(PathBuf);

    impl Scratch {
        fn holding(what: &str, text: &str) -> Scratch {
            let path = std::env::temp_dir().join(format!(
                "ayeaye-overview-{}-{what}.jsonl",
                std::process::id()
            ));
            fs::write(&path, text).expect("a scratch transcript");
            Scratch(path)
        }

        fn path(&self) -> &str {
            self.0.to_str().expect("a utf-8 temp path")
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.0);
        }
    }

    fn said(kind: &str, text: &str) -> String {
        format!(
            r#"{{"type":"{kind}","timestamp":"2026-03-04T09:00:00.000Z","message":{{"role":"{kind}","content":[{{"type":"text","text":"{text}"}}]}}}}"#
        )
    }

    // AYEAYE-49 — a small file is read whole, and its mtime rides along for
    // the stampless case.
    #[test]
    fn a_small_file_is_read_whole() {
        let scratch = Scratch::holding("whole", "line one\nline two\n");
        let (chunk, touched) = tail(scratch.path(), 200_000).expect("a readable file");
        assert_eq!(chunk, "line one\nline two\n");
        assert!(touched > 0.0, "the mtime came along");
    }

    // AYEAYE-49 — a tail that starts mid-file discards its partial first
    // line: half a JSON line parses as nothing and must not cost the line it
    // collided with.
    #[test]
    fn a_tail_discards_its_partial_first_line() {
        let mut text = String::new();
        for index in 0..100 {
            text.push_str(&format!(
                "line number {index:03} padded out {}\n",
                "x".repeat(40)
            ));
        }
        let scratch = Scratch::holding("partial", &text);
        let (chunk, _) = tail(scratch.path(), 1_000).expect("a readable file");
        assert!(chunk.len() < 1_000);
        assert!(
            chunk.starts_with("line number "),
            "the chunk starts at a line start: {:?}",
            &chunk[..40]
        );
        assert!(chunk.ends_with(&format!("line number 099 padded out {}\n", "x".repeat(40))));
    }

    // AYEAYE-49 — the spec's third state: a transcript that is not there is
    // "could not read", never an empty transcript that would classify as a
    // fine, waiting session.
    #[test]
    fn a_transcript_that_is_gone_is_gone() {
        assert_eq!(tail("/nonexistent/ayeaye/transcript.jsonl", 200_000), None);
        assert_eq!(Status::gone().state, State::Gone);
    }

    // AYEAYE-49 — the tail is the horizon, deliberately: a launch old enough
    // to have scrolled out of it is forgotten, and a session busy enough to
    // push it out is not one sitting still waiting for an answer.
    // Transcribed from session_state.py's tail_bytes=2000 case.
    #[test]
    fn a_launch_that_scrolled_out_of_the_tail_is_forgotten() {
        let mut lines = vec![format!(
            r#"{{"type":"assistant","timestamp":"2026-03-04T09:00:00.000Z","message":{{"role":"assistant","content":[{{"type":"tool_use","id":"toolu_A","name":"Agent","input":{{}}}}]}}}}"#
        )];
        for index in 0..200 {
            lines.push(said(
                "assistant",
                &format!("line {index} {}", "x".repeat(200)),
            ));
        }
        let scratch = Scratch::holding("horizon", &(lines.join("\n") + "\n"));
        let (chunk, touched) = tail(scratch.path(), 2_000).expect("a readable file");
        let got = classify(&chunk, Kind::Claude, touched + 60.0, touched);
        assert_eq!(
            got.state,
            State::Waiting,
            "the launch is out of the horizon"
        );
    }
}
