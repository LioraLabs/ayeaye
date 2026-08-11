//! When a push notification is worth sending, and what it says.
//!
//! One rule, and it is a rule about transitions rather than states: a
//! notification marks the moment a session *became* your problem. Not the
//! fact that it is your problem, which stays true for as long as you leave
//! it, and not anything it does on its own afterwards. A board that notified
//! on states would tell you about the same waiting session every ten
//! seconds; one that notified on every change would tell you that you had
//! just replied, which you know.
//!
//! Nothing here keeps time or opens a socket. The shell sweeps the overview
//! on its clock and publishes over its transport; this decides which sweep
//! notifies ([`changes`]) and what the notification says ([`message`],
//! [`payload`]) — so the decision the daemon makes inside `notify_changes`
//! and `watch_and_notify` is a set of facts a test can hold without either.

use std::collections::{BTreeMap, BTreeSet};

use crate::json;
use crate::overview::Card;
use crate::session::status::State;

/// Which states are worth a notification, when nothing says otherwise.
///
/// The ones that mean it is your turn: `blocked` is an agent stopped at a
/// prompt; `waiting` is an agent that has handed the turn back. Both fire on
/// the way in and only on the way in, so a session that has been waiting for
/// an hour is silent, and so is one you have just replied to.
pub const DEFAULT_STATES: &str = "blocked,waiting";

/// How much of a blocked pane's question takes part in change detection.
const QUESTION_KEY: usize = 120;

/// How much of the question a notification body carries.
const QUESTION_BODY: usize = 300;

/// How many options ride along, and how much of each label.
const OPTIONS: usize = 4;
const OPTION_LABEL: usize = 60;

/// The states a comma-separated setting names.
///
/// An unknown or empty piece configures nothing, exactly as it matched
/// nothing when the daemon compared raw strings.
/// Trimmed per piece, where the daemon compares raw: `"blocked, waiting"`
/// configured only `blocked` there, and that was a trap rather than a rule
/// worth keeping.
pub fn wanted(setting: &str) -> BTreeSet<State> {
    setting
        .split(',')
        .filter_map(|name| State::of(name.trim()))
        .collect()
}

/// What makes this pane worth one notification.
///
/// The state, plus the question while blocked. Including the question means a
/// second prompt in a pane already blocked is fresh news; excluding it
/// everywhere else means the sentences an agent writes while it works are not
/// mistaken for changes of state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Key {
    state: State,
    question: String,
}

/// This pane's key, or `None` for a pane that can never be worth any — a
/// plain shell has no state to change.
pub fn key(card: &Card) -> Option<Key> {
    let state = card.state?;
    let question = match state {
        State::Blocked => card
            .prompt
            .as_ref()
            .map(|prompt| prompt.question.chars().take(QUESTION_KEY).collect())
            .unwrap_or_default(),
        _ => String::new(),
    };
    Some(Key { state, question })
}

/// The state each pane was last seen in, keyed by qualified pane id.
pub type Seen = BTreeMap<String, Key>;

/// The panes that just became your problem, and the state to compare next.
///
/// A notification marks a transition *into* a state and never the state
/// itself. A pane with no previous state is recorded and left alone, which is
/// what makes the first sweep after a restart silent rather than a broadside
/// of everything already on the board. A pane that has gone is dropped from
/// `Seen`, so its id coming back later seeds afresh.
pub fn changes<'cards>(
    cards: &'cards [Card],
    seen: &Seen,
    wanted: &BTreeSet<State>,
) -> (Vec<&'cards Card>, Seen) {
    let mut fresh = Vec::new();
    let mut current = Seen::new();
    for card in cards {
        let Some(key) = key(card) else {
            continue;
        };
        let id = card.pane.id.qualified();
        if seen.get(&id).is_some_and(|was| *was != key) && wanted.contains(&key.state) {
            fresh.push(card);
        }
        current.insert(id, key);
    }
    (fresh, current)
}

/// One notification, worded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    /// `session:window · agent note` — where, who, and what just happened.
    pub title: String,
    /// The question or the last words, with up to four options under it.
    pub body: String,
    /// ntfy's icon tags.
    pub tags: &'static [&'static str],
    /// ntfy's 1–5 priority.
    pub priority: u8,
}

/// How each state reads once it is the thing that just happened.
///
/// Both mean the turn is yours; `blocked` is louder because a session at a
/// prompt cannot make progress without you, where one that has handed back
/// has at least finished.
fn note(state: State) -> (&'static str, &'static [&'static str], u8) {
    match state {
        State::Blocked => ("needs you", &["warning"], 4),
        State::Waiting => ("finished its turn", &["bell"], 3),
        // A state somebody configured in beyond the defaults still reads as
        // something on a lock screen.
        _ => ("needs you", &["bell"], 3),
    }
}

/// The words one card's arrival is announced with.
pub fn message(card: &Card) -> Message {
    let pane = &card.pane;
    let agent = match card.kind {
        Some(kind) => kind.as_str(),
        None if !pane.cmd.is_empty() => &pane.cmd,
        None => "agent",
    };
    // A card with no state cannot reach here through [`changes`] — it never
    // gets a key — but a direct caller gets the daemon's own default, which
    // is `note`'s fallback arm, not the waiting wording.
    let (said, tags, priority) = note(card.state.unwrap_or(State::Working));
    let question: String = card
        .prompt
        .as_ref()
        .map(|prompt| prompt.question.chars().take(QUESTION_BODY).collect())
        .unwrap_or_default();
    let mut body = if !question.is_empty() {
        question
    } else if !card.last.is_empty() {
        card.last.clone()
    } else {
        "is waiting for you".to_string()
    };
    if let Some(prompt) = &card.prompt
        && !prompt.options.is_empty()
    {
        body.push_str("\n\n");
        body.push_str(
            &prompt
                .options
                .iter()
                .take(OPTIONS)
                .map(|choice| {
                    format!(
                        "{}. {}",
                        choice.key,
                        choice.label.chars().take(OPTION_LABEL).collect::<String>()
                    )
                })
                .collect::<Vec<_>>()
                .join("\n"),
        );
    }
    Message {
        title: format!("{}:{} · {} {}", pane.session, pane.window, agent, said),
        body,
        tags,
        priority,
    }
}

/// The JSON one publish sends to ntfy.
///
/// The JSON API rather than ntfy's header form, exactly as the daemon
/// chooses: HTTP headers are latin-1, so a non-ASCII character in a title
/// arrives mangled — and agent questions are full of them.
pub fn payload(topic: &str, message: &Message, click: &str) -> String {
    let tags = message
        .tags
        .iter()
        .map(|tag| json::string(tag))
        .collect::<Vec<_>>()
        .join(",");
    let mut out = format!(
        "{{\"topic\":{},\"title\":{},\"message\":{},\"priority\":{},\"tags\":[{}]",
        json::string(topic),
        json::string(&message.title),
        json::string(&message.body),
        message.priority,
        tags,
    );
    if !click.is_empty() {
        out.push_str(&format!(",\"click\":{}", json::string(click)));
    }
    out.push('}');
    out
}

/// Where to POST and which topic to name: the configured URL's last path
/// piece is the topic, the rest is the server.
///
/// The daemon's `NTFY_URL.rstrip("/").rpartition("/")` — the URL somebody
/// configures is `https://ntfy.example/topic`, and the JSON API posts to the
/// server root with the topic in the body.
pub fn split_url(url: &str) -> (String, String) {
    let trimmed = url.trim_end_matches('/');
    match trimmed.rfind('/') {
        Some(at) => (trimmed[..at].to_string(), trimmed[at + 1..].to_string()),
        None => (String::new(), trimmed.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::{DEFAULT_STATES, Seen, changes, message, payload, split_url, wanted};
    use crate::overview::Card;
    use crate::peer::{HostName, PaneId};
    use crate::prompt::{Choice, Prompt};
    use crate::session::Kind;
    use crate::session::status::{State, Status};
    use crate::tmux::Pane;

    fn pane(id: &str) -> Pane {
        Pane {
            id: PaneId::new(HostName::new("desktop").expect("a host"), id).expect("a pane id"),
            session: "ppu-toys".to_string(),
            window: "1".to_string(),
            name: "ppu-toys".to_string(),
            cmd: "claude".to_string(),
            active: false,
        }
    }

    fn question(text: &str) -> Prompt {
        Prompt {
            question: text.to_string(),
            options: vec![Choice {
                key: "1".to_string(),
                label: "Yes".to_string(),
            }],
        }
    }

    /// One row of the board, as `overview` builds it.
    fn card(id: &str, state: Option<State>, asked: Option<&str>) -> Card {
        Card::of(
            pane(id),
            state.map(|state| {
                (
                    Kind::Claude,
                    "abcd1234".to_string(),
                    Status {
                        state,
                        age: Some(12),
                        last: String::new(),
                    },
                )
            }),
            asked.map(question),
        )
    }

    /// Poll once per step, returning the states that notified each sweep.
    ///
    /// A `Some((state, question))` is a pane at a prompt; `Some((state,
    /// None))`… is a pane in that state; `None` is no pane at all.
    fn sweep(steps: &[Option<(Option<State>, Option<&str>)>]) -> Vec<Vec<State>> {
        let wanted = wanted(DEFAULT_STATES);
        let mut seen = Seen::new();
        let mut fired = Vec::new();
        for step in steps {
            let cards: Vec<Card> = match step {
                None => Vec::new(),
                Some((state, asked)) => vec![card("%1", *state, *asked)],
            };
            let (fresh, next) = changes(&cards, &seen, &wanted);
            fired.push(
                fresh
                    .iter()
                    .map(|card| card.state.expect("a notified card has a state"))
                    .collect(),
            );
            seen = next;
        }
        fired
    }

    fn plain(state: State) -> Option<(Option<State>, Option<&'static str>)> {
        Some((Some(state), None))
    }

    // Transcribed from tests/python/notify_watch.py rather than derived from
    // the port, so the two can disagree.

    // AYEAYE-49 — the turn coming back notifies, whichever state it came
    // back from: working, delegating, or running.
    #[test]
    fn the_turn_handed_back_notifies() {
        for from in [State::Working, State::Delegating, State::Running] {
            assert_eq!(
                sweep(&[plain(from), plain(State::Waiting)]),
                [vec![], vec![State::Waiting]],
                "from {from:?}"
            );
        }
    }

    // AYEAYE-49 — idempotent across polls: a session left waiting notifies
    // once and then shuts up.
    #[test]
    fn a_session_left_waiting_notifies_once_and_then_shuts_up() {
        assert_eq!(
            sweep(&[
                plain(State::Working),
                plain(State::Waiting),
                plain(State::Waiting),
                plain(State::Waiting),
                plain(State::Waiting),
            ]),
            [vec![], vec![State::Waiting], vec![], vec![], vec![]]
        );
    }

    // AYEAYE-49 — the case that prompted all of this: your own message moves
    // the pane to working, and being told about your own typing is noise.
    #[test]
    fn replying_to_a_waiting_agent_does_not_notify() {
        assert_eq!(
            sweep(&[
                plain(State::Working),
                plain(State::Waiting),
                plain(State::Working),
            ]),
            [vec![], vec![State::Waiting], vec![]]
        );
    }

    // AYEAYE-49 — a full round trip notifies only on the way back.
    #[test]
    fn a_full_round_trip_notifies_only_on_the_way_back() {
        assert_eq!(
            sweep(&[
                plain(State::Working),
                plain(State::Waiting),
                plain(State::Working),
                plain(State::Running),
                plain(State::Waiting),
            ]),
            [vec![], vec![State::Waiting], vec![], vec![], vec![State::Waiting]]
        );
    }

    // AYEAYE-49 — the working/running/delegating flicker of an agent doing
    // its job is silent.
    #[test]
    fn the_working_running_flicker_is_silent() {
        assert_eq!(
            sweep(&[
                plain(State::Working),
                plain(State::Running),
                plain(State::Working),
                plain(State::Running),
                plain(State::Delegating),
            ]),
            vec![Vec::<State>::new(); 5]
        );
    }

    // AYEAYE-49 — a plain shell has no state, so it can never change into
    // one.
    #[test]
    fn no_state_at_all_is_never_a_notification() {
        assert_eq!(
            sweep(&[Some((None, None)), Some((None, None))]),
            vec![Vec::<State>::new(); 2]
        );
    }

    // AYEAYE-49 — arriving at a prompt notifies; the same prompt left
    // unanswered notifies once.
    #[test]
    fn arriving_at_a_prompt_notifies_once() {
        assert_eq!(
            sweep(&[
                plain(State::Working),
                Some((Some(State::Working), Some("Hook range?"))),
                Some((Some(State::Working), Some("Hook range?"))),
                Some((Some(State::Working), Some("Hook range?"))),
            ]),
            [vec![], vec![State::Blocked], vec![], vec![]]
        );
    }

    // AYEAYE-49 — the state never leaves blocked, so only the question can
    // say that the first one was answered and another has taken its place.
    #[test]
    fn a_second_question_in_the_same_pane_is_fresh_news() {
        assert_eq!(
            sweep(&[
                plain(State::Working),
                Some((Some(State::Working), Some("Hook range?"))),
                Some((Some(State::Working), Some("Which scanline?"))),
            ]),
            [vec![], vec![State::Blocked], vec![State::Blocked]]
        );
    }

    // AYEAYE-49 — answering a prompt does not notify: that is a move to
    // working, which nothing subscribes to.
    #[test]
    fn answering_a_prompt_does_not_notify() {
        assert_eq!(
            sweep(&[
                plain(State::Working),
                Some((Some(State::Working), Some("Hook range?"))),
                plain(State::Working),
            ]),
            [vec![], vec![State::Blocked], vec![]]
        );
    }

    // AYEAYE-49 — every pane looks new after a restart, and a restart is not
    // news: the first sweep only seeds.
    #[test]
    fn the_first_sweep_only_seeds() {
        let wanted = wanted(DEFAULT_STATES);
        let cards = vec![
            card("%1", Some(State::Waiting), None),
            card("%2", Some(State::Working), Some("Q?")),
        ];
        let (fresh, seen) = changes(&cards, &Seen::new(), &wanted);
        assert!(fresh.is_empty());
        assert_eq!(seen.len(), 2);
    }

    // AYEAYE-49 — a pane that goes away is forgotten, so its id coming back
    // seeds afresh rather than comparing against a dead pane's state.
    #[test]
    fn a_pane_that_goes_away_is_forgotten() {
        let wanted = wanted(DEFAULT_STATES);
        let cards = vec![card("%1", Some(State::Working), None)];
        let (_, seen) = changes(&cards, &Seen::new(), &wanted);
        let (_, seen) = changes(&[], &seen, &wanted);
        assert!(seen.is_empty());
    }

    // AYEAYE-49 — panes are tracked apart: one pane's transition does not
    // speak for its neighbour.
    #[test]
    fn panes_are_tracked_apart() {
        let wanted = wanted(DEFAULT_STATES);
        let first = vec![
            card("%1", Some(State::Working), None),
            card("%2", Some(State::Working), None),
        ];
        let (_, seen) = changes(&first, &Seen::new(), &wanted);
        let second = vec![
            card("%1", Some(State::Waiting), None),
            card("%2", Some(State::Working), None),
        ];
        let (fresh, _) = changes(&second, &seen, &wanted);
        assert_eq!(
            fresh
                .iter()
                .map(|card| card.pane.id.qualified())
                .collect::<Vec<_>>(),
            ["desktop/%1"]
        );
    }

    // AYEAYE-49 — narrowing the setting silences the rest, and an unknown
    // name in it configures nothing.
    #[test]
    fn narrowing_the_setting_silences_the_rest() {
        let only_blocked = wanted("blocked");
        let mut seen = Seen::new();
        let working = [card("%1", Some(State::Working), None)];
        let (_, next) = changes(&working, &seen, &only_blocked);
        seen = next;
        let waiting = [card("%1", Some(State::Waiting), None)];
        let (fresh, next) = changes(&waiting, &seen, &only_blocked);
        assert!(fresh.is_empty(), "waiting is not subscribed to");
        seen = next;
        let blocked = [card("%1", Some(State::Working), Some("Q?"))];
        let (fresh, _) = changes(&blocked, &seen, &only_blocked);
        assert_eq!(fresh.len(), 1, "blocked still is");

        assert_eq!(wanted("blocked,waiting"), wanted(DEFAULT_STATES));
        assert!(wanted("idle,,nonsense").is_empty());
    }

    // AYEAYE-49 — the wording, worth reading on a lock screen: where, who,
    // and what just happened, louder for a pane that cannot proceed at all.
    #[test]
    fn a_blocked_pane_is_announced_loudly_with_its_question_and_options() {
        let mut asked = question("Hook range?");
        asked.options.push(Choice {
            key: "2".to_string(),
            label: "x".repeat(80),
        });
        let card = Card::of(pane("%1"), None, Some(asked));
        let said = message(&card);
        assert_eq!(said.title, "ppu-toys:1 · claude needs you");
        assert_eq!(
            said.body,
            format!("Hook range?\n\n1. Yes\n2. {}", "x".repeat(60)),
            "the labels are capped at sixty characters"
        );
        assert_eq!(said.tags, ["warning"]);
        assert_eq!(said.priority, 4);
    }

    // AYEAYE-49 — a turn handed back reads as finished, with the agent's
    // last words for a body, and "is waiting for you" when there are none.
    #[test]
    fn a_waiting_pane_reads_as_finished_its_turn() {
        let mut waiting = card("%1", Some(State::Waiting), None);
        waiting.last = "done — two bugs, both in the lexer".to_string();
        let said = message(&waiting);
        assert_eq!(said.title, "ppu-toys:1 · claude finished its turn");
        assert_eq!(said.body, "done — two bugs, both in the lexer");
        assert_eq!(said.tags, ["bell"]);
        assert_eq!(said.priority, 3);

        let silent = card("%1", Some(State::Waiting), None);
        assert_eq!(message(&silent).body, "is waiting for you");
    }

    // AYEAYE-49 — a shell at a prompt is announced by what it runs, and a
    // pane that runs nothing nameable is still "agent".
    #[test]
    fn a_pane_is_named_by_its_agent_then_its_command() {
        let mut shell = pane("%1");
        shell.cmd = "sh".to_string();
        let said = message(&Card::of(shell, None, Some(question("Q?"))));
        assert!(said.title.contains("· sh needs you"), "{}", said.title);

        let mut bare = pane("%1");
        bare.cmd = String::new();
        let said = message(&Card::of(bare, None, Some(question("Q?"))));
        assert!(said.title.contains("· agent needs you"), "{}", said.title);
    }

    // AYEAYE-49 — every state that fires by default has wording of its own:
    // a state announced by the fallback says "needs you" whatever actually
    // happened, and the two messages worth having must not read the same.
    #[test]
    fn every_notified_state_has_wording_of_its_own() {
        let fallback = message(&card("%1", Some(State::Working), None));
        let blocked = message(&card("%1", Some(State::Working), Some("Q?")));
        let waiting = message(&card("%1", Some(State::Waiting), None));
        assert!(blocked.title.ends_with("needs you"), "{}", blocked.title);
        assert!(
            waiting.title.ends_with("finished its turn"),
            "{}",
            waiting.title
        );
        assert_ne!(
            (blocked.tags, blocked.priority),
            (fallback.tags, fallback.priority),
            "blocked is louder than the fallback"
        );
        assert_ne!(blocked.title, waiting.title);
    }

    // AYEAYE-49 — the payload is ntfy's JSON API, not its header form:
    // headers are latin-1 and agent questions are full of what latin-1
    // mangles. Click rides along only when it is configured.
    #[test]
    fn the_payload_is_json_so_a_non_ascii_title_survives() {
        let mut blocked = card("%1", Some(State::Working), Some("café ouvert ?"));
        blocked.last = String::new();
        let said = message(&blocked);
        assert_eq!(
            payload("mytopic", &said, "https://app.example/"),
            concat!(
                r#"{"topic":"mytopic","title":"ppu-toys:1 · claude needs you","#,
                r#""message":"café ouvert ?\n\n1. Yes","priority":4,"tags":["warning"],"#,
                r#""click":"https://app.example/"}"#
            )
        );
        assert!(!payload("t", &said, "").contains("click"));
    }

    // AYEAYE-49 — the configured URL's last piece is the topic, the rest the
    // server, exactly as the daemon splits it.
    #[test]
    fn the_url_splits_into_server_and_topic() {
        assert_eq!(
            split_url("https://ntfy.sh/my-agents"),
            ("https://ntfy.sh".to_string(), "my-agents".to_string())
        );
        assert_eq!(
            split_url("https://ntfy.example/team/agents/"),
            ("https://ntfy.example/team".to_string(), "agents".to_string())
        );
        assert_eq!(split_url("topic-only"), (String::new(), "topic-only".to_string()));
    }
}
