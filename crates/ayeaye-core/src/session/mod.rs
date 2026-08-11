//! Which agent session is running in a pane — the half of that question that
//! is text rather than machinery.
//!
//! A pane is an agent session because an agent is running in it, so the
//! *process* is what settles it and the pane's text is only ever a fallback.
//! That order is worth more than the subprocess it saves: a statusline marker
//! outlives the session that printed it, so a pane whose agent has exited — or
//! that was cleared and started again — reads through the marker alone as an
//! agent, and as the wrong one. A process that is not there answers nothing.
//!
//! Nothing here opens a file or asks about a process. What lives here is
//! everything that is decided rather than done: whether a pane is running an
//! agent at all, whether a file the agent left behind is about the process
//! asking, whether a name in a directory is the transcript being looked for.
//! The shell above does the reading, in `ayeaye::session`, where a test can
//! build a directory of its own rather than read the machine it runs on.
//!
//! `None` means "could not find out", never an error: this answers inside a
//! request handler, and a pane whose session cannot be identified is an
//! ordinary state of the world.

pub mod claude;
pub mod codex;
pub mod status;

use crate::json;

/// How much of a session id is ever shown.
///
/// The daemon reports eight characters and the panel renders them; the whole id
/// is what a file is found by, and it is not what a card is labelled with.
pub const SHORT: usize = 8;

/// The agents this app can read a session out of.
///
/// A closed set on purpose, and the same one the spawn endpoint matches
/// against: a pane running something else resolves to nothing rather than to a
/// guess, and the name never comes from the request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Anthropic's `claude`.
    Claude,
    /// OpenAI's `codex`.
    Codex,
}

impl Kind {
    /// The agent a pane is running, by tmux's reckoning, or `None`.
    ///
    /// `pane_current_command` is the *foreground* process's name, which is what
    /// makes this a live check rather than a historical one: a pane whose agent
    /// has exited is back at a shell and answers here with nothing.
    pub fn of(pane_current_command: &str) -> Option<Kind> {
        match pane_current_command {
            "claude" => Some(Kind::Claude),
            "codex" => Some(Kind::Codex),
            _ => None,
        }
    }

    /// The name it goes by, in a response and on the command line alike.
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Claude => "claude",
            Kind::Codex => "codex",
        }
    }
}

/// The agent session behind one pane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    /// Which agent it is, which decides how its transcript is read.
    pub kind: Kind,
    /// The short id, as the panel shows it.
    pub id: String,
    /// The transcript on disk: a claude `.jsonl` under its projects directory,
    /// or a codex rollout.
    pub path: String,
}

impl Session {
    /// One session, with its id shortened the way the daemon shortens it.
    ///
    /// By characters rather than bytes. Every id either agent writes is hex and
    /// dashes, so the two agree — and slicing a string by a byte count that
    /// might land inside a character is a panic in a request handler, which is
    /// too high a price for an assumption about somebody else's file format.
    pub fn new(kind: Kind, id: &str, path: &str) -> Session {
        Session {
            kind,
            id: id.chars().take(SHORT).collect(),
            path: path.to_string(),
        }
    }
}

/// The body `/api/session` answers with.
///
/// The daemon's shape: `kind` is always present and is `null` when the pane is
/// not an agent, so the panel can tell "not an agent" from "could not look"
/// without a status code. `id` is there only when there is a session.
pub fn session_body(session: Option<&Session>) -> String {
    match session {
        None => "{\"kind\":null}".to_string(),
        Some(session) => format!(
            "{{\"kind\":{},\"id\":{}}}",
            json::string(session.kind.as_str()),
            json::string(&session.id),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::{Kind, Session, session_body};

    // AYEAYE-45 — a closed set: a pane running something else resolves to
    // nothing rather than to a guess, and `sh` is what a pane whose agent has
    // exited is running.
    #[test]
    fn only_the_two_agents_are_agents() {
        assert_eq!(Kind::of("claude"), Some(Kind::Claude));
        assert_eq!(Kind::of("codex"), Some(Kind::Codex));
        for other in ["sh", "bash", "vim", "Claude", "claude-code", "", "node"] {
            assert_eq!(Kind::of(other), None, "{other} is not an agent");
        }
    }

    // AYEAYE-45 — the panel shows eight characters and finds a file by the
    // whole id. Shortening by bytes would be a panic in a request handler the
    // first time an agent wrote an id this app did not expect.
    #[test]
    fn a_session_is_labelled_with_the_first_eight_characters() {
        let session = Session::new(
            Kind::Claude,
            "0123abcd-dead-beef-0000-111122223333",
            "/home/someone/.claude/projects/x/0123abcd.jsonl",
        );
        assert_eq!(session.id, "0123abcd");
        assert_eq!(
            session.path,
            "/home/someone/.claude/projects/x/0123abcd.jsonl"
        );

        assert_eq!(Session::new(Kind::Codex, "abc", "/p").id, "abc");
        assert_eq!(Session::new(Kind::Codex, "", "/p").id, "");
        // Not an id any agent writes, and not a panic either.
        assert_eq!(Session::new(Kind::Codex, "ααααααααα", "/p").id, "αααααααα");
    }

    // AYEAYE-45 — `kind` is always present so the panel can tell "this pane is
    // not an agent" from "something went wrong" without reading a status code.
    #[test]
    fn a_pane_that_is_not_an_agent_still_answers() {
        assert_eq!(session_body(None), r#"{"kind":null}"#);
        assert_eq!(
            session_body(Some(&Session::new(Kind::Codex, "0123abcd-dead", "/p"))),
            r#"{"kind":"codex","id":"0123abcd"}"#
        );
    }
}
