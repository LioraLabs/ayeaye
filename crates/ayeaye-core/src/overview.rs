//! The main panel view: every pane with its state, ordered so the ones
//! needing you come first.
//!
//! Nothing here runs tmux, reads a transcript, or captures a screen. The
//! shell gathers those three answers per pane and hands them over; this
//! decides what they add up to — which state wins, which pane sorts first,
//! and what the body says — so the merge the daemon does inside `overview()`
//! is a set of facts a test can hold without a machine.

use crate::json;
use crate::peer::HostName;
use crate::prompt::{self, Prompt};
use crate::session::Kind;
use crate::session::status::{State, Status};
use crate::tmux::Pane;

/// One pane's card: the pane itself and everything known about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Card {
    pub pane: Pane,
    /// The agent behind it, or `None` for a plain shell.
    pub kind: Option<Kind>,
    /// The session's short id. `agent_id`, not `id`: `id` is the pane id the
    /// UI selects with, and overwriting it with the session id makes every
    /// card unselectable — the daemon learned that the hard way.
    pub agent_id: Option<String>,
    /// What the transcript said, with the prompt's override already applied.
    pub state: Option<State>,
    /// Seconds since the last thing that actually happened.
    pub age: Option<u64>,
    /// The most recent thing either of you said.
    pub last: String,
    /// The question the pane is stopped on, if it is stopped on one.
    pub prompt: Option<Prompt>,
}

impl Card {
    /// Merge one pane's three answers into its card.
    ///
    /// The prompt is checked for every pane, not just resolved agents:
    /// anything can stop to ask. And it overrides the transcript-derived
    /// state, which would otherwise report `working` for a pane that is
    /// blocked — the worst thing to get wrong on a dashboard whose job is
    /// "who needs me".
    pub fn of(pane: Pane, session: Option<(Kind, String, Status)>, prompt: Option<Prompt>) -> Card {
        let (kind, agent_id, state, age, last) = match session {
            Some((kind, id, status)) => (
                Some(kind),
                Some(id),
                Some(status.state),
                status.age,
                status.last,
            ),
            None => (None, None, None, None, String::new()),
        };
        let state = if prompt.is_some() {
            Some(State::Blocked)
        } else {
            state
        };
        Card {
            pane,
            kind,
            agent_id,
            state,
            age,
            last,
            prompt,
        }
    }
}

/// The age a card with no age sorts as: after every card that has one.
///
/// The daemon's `1 << 30` — a session whose age cannot be read must not float
/// to the top as though it just spoke.
const AGELESS: u64 = 1 << 30;

/// Blocked first, then agents by recency, plain shells last.
///
/// A stable sort, so ties keep the pane list's own order — which is tmux's,
/// and the same twice running.
pub fn order(cards: &mut [Card]) {
    cards.sort_by_key(|card| {
        (
            card.state != Some(State::Blocked),
            card.kind.is_none(),
            card.age.unwrap_or(AGELESS),
        )
    });
}

/// The body `/api/overview` answers with.
///
/// The daemon's `{"host": …, "panes": [...], "voice": …}`, one card per pane
/// in the order [`order`] put them in. `failed` is `/api/panes`'s
/// could-not-look field carried over rather than reinvented: the panel polls
/// this and has to keep rendering, and what it must never do is show an empty
/// list that means "nothing needs you" when the truth is "I could not look".
pub fn body(host: &HostName, cards: &[Card], voice: bool, failed: Option<&str>) -> String {
    let mut body = format!("{{\"host\":{},\"panes\":[", json::string(host.as_str()));
    for (index, card) in cards.iter().enumerate() {
        if index > 0 {
            body.push(',');
        }
        body.push_str(&card_object(card));
    }
    body.push_str(&format!("],\"voice\":{voice}"));
    if let Some(failed) = failed {
        body.push_str(&format!(",\"error\":{}", json::string(failed)));
    }
    body.push('}');
    body
}

/// One card, in the daemon's field order: the pane's own fields first, then
/// what is known about it, `null` where nothing is.
fn card_object(card: &Card) -> String {
    let pane = &card.pane;
    format!(
        concat!(
            "{{\"id\":{},\"session\":{},\"window\":{},\"name\":{},\"cmd\":{},\"active\":{},",
            "\"kind\":{},\"agent_id\":{},\"state\":{},\"age\":{},\"last\":{},\"prompt\":{}}}"
        ),
        json::string(&pane.id.qualified()),
        json::string(&pane.session),
        json::string(&pane.window),
        json::string(&pane.name),
        json::string(&pane.cmd),
        pane.active,
        named(card.kind.map(Kind::as_str)),
        named(card.agent_id.as_deref()),
        named(card.state.map(State::as_str)),
        match card.age {
            Some(age) => age.to_string(),
            None => "null".to_string(),
        },
        json::string(&card.last),
        match &card.prompt {
            Some(prompt) => prompt::object(prompt),
            None => "null".to_string(),
        },
    )
}

/// A string field that may be absent: quoted, or `null`.
fn named(value: Option<&str>) -> String {
    match value {
        Some(value) => json::string(value),
        None => "null".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::{Card, body, order};
    use crate::peer::{HostName, PaneId};
    use crate::prompt::{Choice, Prompt};
    use crate::session::Kind;
    use crate::session::status::{State, Status};
    use crate::tmux::Pane;

    fn host() -> HostName {
        HostName::new("desktop").expect("a host name")
    }

    fn pane(id: &str, cmd: &str) -> Pane {
        Pane {
            id: PaneId::new(host(), id).expect("a pane id"),
            session: "work".to_string(),
            window: "1".to_string(),
            name: "editor".to_string(),
            cmd: cmd.to_string(),
            active: false,
        }
    }

    fn status(state: State, age: u64) -> Status {
        Status {
            state,
            age: Some(age),
            last: String::new(),
        }
    }

    fn agent(id: &str, state: State, age: u64) -> Card {
        Card::of(
            pane(id, "claude"),
            Some((Kind::Claude, "abcd1234".to_string(), status(state, age))),
            None,
        )
    }

    fn question() -> Prompt {
        Prompt {
            question: "Hook range?".to_string(),
            options: vec![Choice {
                key: "1".to_string(),
                label: "Yes".to_string(),
            }],
        }
    }

    fn ids(cards: &[Card]) -> Vec<String> {
        cards.iter().map(|card| card.pane.id.qualified()).collect()
    }

    // AYEAYE-49 — a prompt overrides the transcript-derived state, which
    // would otherwise report `working` for a pane that is blocked: the worst
    // thing to get wrong on a dashboard whose job is "who needs me", and the
    // bug that motivated the prompt fixtures.
    #[test]
    fn a_prompt_makes_a_working_agent_blocked() {
        let card = Card::of(
            pane("%1", "claude"),
            Some((
                Kind::Claude,
                "abcd1234".to_string(),
                status(State::Working, 10),
            )),
            Some(question()),
        );
        assert_eq!(card.state, Some(State::Blocked));
        assert_eq!(card.kind, Some(Kind::Claude));
        assert_eq!(card.prompt, Some(question()));
    }

    // AYEAYE-49 — checked for every pane, not just resolved agents: anything
    // can stop to ask, a shell included.
    #[test]
    fn a_prompt_blocks_a_plain_shell_too() {
        let card = Card::of(pane("%1", "sh"), None, Some(question()));
        assert_eq!(card.state, Some(State::Blocked));
        assert_eq!(card.kind, None);
    }

    // AYEAYE-49 — the ordering: blocked first, then agents by recency, plain
    // shells last. Transcribed from the daemon's sort key rather than derived
    // from this module's.
    #[test]
    fn blocked_first_then_agents_by_recency_then_shells() {
        let mut cards = vec![
            Card::of(pane("%0", "sh"), None, None),
            agent("%1", State::Waiting, 900),
            agent("%2", State::Working, 5),
            Card::of(
                pane("%3", "claude"),
                Some((
                    Kind::Claude,
                    "abcd1234".to_string(),
                    status(State::Working, 3_600),
                )),
                Some(question()),
            ),
        ];
        order(&mut cards);
        assert_eq!(
            ids(&cards),
            ["desktop/%3", "desktop/%2", "desktop/%1", "desktop/%0"]
        );
    }

    // AYEAYE-49 — a blocked shell outranks the freshest agent: the first key
    // is the state, and only then whether it is an agent at all.
    #[test]
    fn a_blocked_shell_outranks_a_fresh_agent() {
        let mut cards = vec![
            agent("%1", State::Working, 0),
            Card::of(pane("%2", "sh"), None, Some(question())),
        ];
        order(&mut cards);
        assert_eq!(ids(&cards), ["desktop/%2", "desktop/%1"]);
    }

    // AYEAYE-49 — a gone session has no age, and sorts after every agent
    // that has one rather than floating to the top: could-not-read must not
    // read as just-spoke.
    #[test]
    fn an_ageless_agent_sorts_after_the_aged_and_before_the_shells() {
        let gone = Card::of(
            pane("%1", "claude"),
            Some((Kind::Claude, "abcd1234".to_string(), Status::gone())),
            None,
        );
        let mut cards = vec![
            Card::of(pane("%0", "sh"), None, None),
            gone,
            agent("%2", State::Waiting, 90_000),
        ];
        order(&mut cards);
        assert_eq!(ids(&cards), ["desktop/%2", "desktop/%1", "desktop/%0"]);
    }

    // AYEAYE-49 — ties keep the pane list's own order, so the board does not
    // reshuffle between two polls that changed nothing.
    #[test]
    fn ties_keep_the_pane_lists_order() {
        let mut cards = vec![
            Card::of(pane("%5", "sh"), None, None),
            Card::of(pane("%2", "sh"), None, None),
            agent("%3", State::Waiting, 60),
            agent("%4", State::Waiting, 60),
        ];
        order(&mut cards);
        assert_eq!(
            ids(&cards),
            ["desktop/%3", "desktop/%4", "desktop/%5", "desktop/%2"]
        );
    }

    // AYEAYE-49 — the body, field by field: the pane's own fields, then what
    // is known, null where nothing is, and the voice verdict the page greys
    // the talk button by.
    #[test]
    fn the_body_carries_every_field_the_daemon_sends() {
        let mut with_prompt = agent("%1", State::Waiting, 42);
        with_prompt.last = "done — two bugs".to_string();
        let cards = vec![with_prompt, Card::of(pane("%0", "sh"), None, None)];
        assert_eq!(
            body(&host(), &cards, true, None),
            concat!(
                r#"{"host":"desktop","panes":["#,
                r#"{"id":"desktop/%1","session":"work","window":"1","name":"editor","cmd":"claude","active":false,"#,
                r#""kind":"claude","agent_id":"abcd1234","state":"waiting","age":42,"last":"done — two bugs","prompt":null},"#,
                r#"{"id":"desktop/%0","session":"work","window":"1","name":"editor","cmd":"sh","active":false,"#,
                r#""kind":null,"agent_id":null,"state":null,"age":null,"last":"","prompt":null}"#,
                r#"],"voice":true}"#
            )
        );
    }

    // AYEAYE-49 — a blocked card carries its question, in the shape
    // `share/app.html` reads off `/api/prompt`.
    #[test]
    fn a_blocked_card_carries_its_question() {
        let cards = vec![Card::of(pane("%1", "sh"), None, Some(question()))];
        let text = body(&host(), &cards, false, None);
        assert!(
            text.contains(
                r#""state":"blocked","age":null,"last":"","prompt":{"question":"Hook range?","options":[{"key":"1","label":"Yes"}]}"#
            ),
            "{text}"
        );
        assert!(text.ends_with(r#""voice":false}"#), "{text}");
    }

    // AYEAYE-49 — a tmux that could not be asked is an empty list carrying
    // the reason, never an empty list that reads as "nothing needs you".
    #[test]
    fn a_failed_look_says_so_rather_than_reading_as_nothing_needs_you() {
        assert_eq!(
            body(&host(), &[], false, Some("no tmux on PATH")),
            r#"{"host":"desktop","panes":[],"voice":false,"error":"no tmux on PATH"}"#
        );
    }
}
