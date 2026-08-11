//! The notification watcher: tell you when a session starts needing you,
//! once per change.
//!
//! The decision is `ayeaye_core::notify`'s; this is the loop around it and
//! the transport under it. The transport is `curl`, and that is the model
//! store's precedent applied for the model store's reason: an ntfy URL is
//! HTTPS, an in-process client for HTTPS needs TLS, every TLS stack in the
//! ecosystem reaches `ring` or `aws-lc-sys`, and both put `cc` in
//! `Cargo.lock`, which the constitution refuses. A notification is one small
//! POST every few minutes at most; a subprocess is the right price.
//!
//! Best effort throughout — a notification failure must never interfere with
//! the app itself. Every refusal goes to stderr and nowhere else.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use ayeaye_core::notify;
use ayeaye_core::session::status::State;

use crate::config::Settings;

/// How long one publish may hold a process.
///
/// The daemon gives `urlopen` ten seconds; curl gets the same deadline via
/// `--max-time` below, and this is the kill-on-drop backstop around it.
const PUBLISH_LIMIT: Duration = Duration::from_secs(15);

/// How often the board is swept when nothing says otherwise.
const DEFAULT_EVERY: u64 = 10;

/// Everything the watcher was told: where to publish, what to say is
/// tappable, how often to look, and which states are worth a message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// The server half of the configured URL — what curl POSTs to.
    base: String,
    /// The topic half — named in the body, ntfy's JSON API shape.
    topic: String,
    /// Opened when the notification is tapped, or empty.
    click: String,
    /// The gap between sweeps.
    every: Duration,
    /// The states that fire.
    wanted: BTreeSet<State>,
}

impl Config {
    /// The watcher's settings, or `None` when no URL is configured — and no
    /// URL disables the watcher entirely: the app works without it, this
    /// only saves you from having to look.
    ///
    /// `AYEAYE_NTFY_URL` first, then the daemon's own `VOICE_NTFY_URL` —
    /// the `choose_cliban` pattern, because these four never wore the
    /// `VOICE_REMOTE_` prefix the general lookup tries. An `EVERY` that
    /// does not parse falls back to the default where the daemon's `int()`
    /// would have refused to start: a mistyped nicety must not take the
    /// daemon down with it.
    pub fn resolve(look_up: impl Fn(&str) -> Option<String>) -> Option<Config> {
        let url = named(&look_up, "NTFY_URL")?;
        let (base, topic) = notify::split_url(&url);
        let every = named(&look_up, "NOTIFY_EVERY")
            .and_then(|value| value.parse().ok())
            .unwrap_or(DEFAULT_EVERY);
        let states = named(&look_up, "NOTIFY_STATES");
        Some(Config {
            base,
            topic,
            click: named(&look_up, "NTFY_CLICK").unwrap_or_default(),
            every: Duration::from_secs(every),
            wanted: notify::wanted(states.as_deref().unwrap_or(notify::DEFAULT_STATES)),
        })
    }

    /// One line for the startup journal: what fires, and where it goes.
    pub fn describe(&self) -> String {
        let mut states = self
            .wanted
            .iter()
            .map(|state| state.as_str())
            .collect::<Vec<_>>();
        states.sort_unstable();
        format!("notifying {} on {}/{}", states.join(","), self.base, self.topic)
    }
}

/// One name, `AYEAYE_` first, then the daemon's `VOICE_` spelling. Empty is
/// unset, as every other setting reads.
fn named(look_up: &impl Fn(&str) -> Option<String>, name: &str) -> Option<String> {
    for prefix in ["AYEAYE_", "VOICE_"] {
        if let Some(value) = look_up(&format!("{prefix}{name}")) {
            let value = value.trim().to_string();
            if !value.is_empty() {
                return Some(value);
            }
        }
    }
    None
}

/// The command line one publish runs.
///
/// `--fail` so an ntfy error page is a refusal rather than a success;
/// `--max-time` is the daemon's own ten-second deadline; the payload rides
/// as an argument because it never carries a secret — the token in this
/// process authenticates the *panel*, and none of it goes to ntfy. The `--`
/// ends the options, so a base URL somebody configured starting with a dash
/// arrives as data.
pub fn argv(base: &str, payload: &str) -> Vec<String> {
    [
        "curl",
        "--fail",
        "--silent",
        "--show-error",
        "--max-time",
        "10",
        "--header",
        "Content-Type: application/json",
        "--data-binary",
        payload,
        "--",
    ]
    .iter()
    .map(|arg| (*arg).to_string())
    .chain([format!("{base}/")])
    .collect()
}

/// Sweep the board on a clock, and publish each pane's arrival once.
///
/// Spawned from `main` beside the server rather than inside `serve`, so a
/// server driven by an integration test never notifies whoever runs the
/// suite. The first sweep only seeds — a restart is not news.
pub fn watcher(settings: Arc<Settings>, config: Config) {
    tokio::spawn(async move {
        let mut seen = notify::Seen::new();
        loop {
            tokio::time::sleep(config.every).await;
            let cards = match crate::overview::cards(&settings).await {
                Ok(cards) => cards,
                // `seen` is left untouched, so the next sweep still compares
                // against the last state actually observed rather than
                // reseeding over a blink.
                Err(trouble) => {
                    eprintln!("ayeaye: notify: {trouble}");
                    continue;
                }
            };
            let (fresh, next) = notify::changes(&cards, &seen, &config.wanted);
            for card in fresh {
                publish(&config, &notify::message(card)).await;
            }
            seen = next;
        }
    });
}

/// One message, published, best effort.
async fn publish(config: &Config, message: &notify::Message) {
    let payload = notify::payload(&config.topic, message, &config.click);
    match crate::command::run(&argv(&config.base, &payload), PUBLISH_LIMIT).await {
        Ok(ran) if ran.ok => {}
        Ok(ran) => eprintln!("ayeaye: notify failed: {}", ran.stderr.trim()),
        Err(why) => eprintln!("ayeaye: notify failed: {why}"),
    }
}

#[cfg(test)]
mod tests {
    use super::{Config, argv};
    use ayeaye_core::notify;
    use std::time::Duration;

    fn env(values: Vec<(&'static str, &'static str)>) -> impl Fn(&str) -> Option<String> {
        move |name| {
            values
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| value.to_string())
        }
    }

    // AYEAYE-49 — no URL disables the watcher entirely: the app works
    // without it, and an empty value is unset, as every other setting reads.
    #[test]
    fn no_url_means_no_watcher_at_all() {
        assert_eq!(Config::resolve(env(vec![])), None);
        assert_eq!(Config::resolve(env(vec![("VOICE_NTFY_URL", "  ")])), None);
    }

    // AYEAYE-49 — the daemon's own spelling still works, the new one wins,
    // and the defaults are the daemon's: ten seconds, blocked and waiting.
    #[test]
    fn the_daemons_spelling_works_and_the_new_one_wins() {
        let legacy = Config::resolve(env(vec![("VOICE_NTFY_URL", "https://ntfy.sh/agents")]))
            .expect("a configured watcher");
        assert_eq!(legacy.base, "https://ntfy.sh");
        assert_eq!(legacy.topic, "agents");
        assert_eq!(legacy.click, "");
        assert_eq!(legacy.every, Duration::from_secs(10));
        assert_eq!(legacy.wanted, notify::wanted("blocked,waiting"));

        let both = Config::resolve(env(vec![
            ("AYEAYE_NTFY_URL", "https://new.example/topic"),
            ("VOICE_NTFY_URL", "https://old.example/topic"),
        ]))
        .expect("a configured watcher");
        assert_eq!(both.base, "https://new.example");
    }

    // AYEAYE-49 — the sweep gap and the states narrow by configuration, and
    // an EVERY that does not parse falls back rather than taking the daemon
    // down: a mistyped nicety is not a reason to refuse to serve.
    #[test]
    fn the_gap_and_the_states_narrow_by_configuration() {
        let config = Config::resolve(env(vec![
            ("VOICE_NTFY_URL", "https://ntfy.sh/agents"),
            ("VOICE_NOTIFY_EVERY", "30"),
            ("VOICE_NOTIFY_STATES", "blocked"),
            ("VOICE_NTFY_CLICK", "https://app.example/"),
        ]))
        .expect("a configured watcher");
        assert_eq!(config.every, Duration::from_secs(30));
        assert_eq!(config.wanted, notify::wanted("blocked"));
        assert_eq!(config.click, "https://app.example/");

        let mistyped = Config::resolve(env(vec![
            ("VOICE_NTFY_URL", "https://ntfy.sh/agents"),
            ("VOICE_NOTIFY_EVERY", "soon"),
        ]))
        .expect("a configured watcher");
        assert_eq!(mistyped.every, Duration::from_secs(10));
    }

    // AYEAYE-49 — the startup line says what fires and where it goes, which
    // is the one place a misconfigured topic shows itself before the first
    // quiet hour is mistaken for peace.
    #[test]
    fn the_journal_line_names_the_states_and_the_destination() {
        let config = Config::resolve(env(vec![("VOICE_NTFY_URL", "https://ntfy.sh/agents")]))
            .expect("a configured watcher");
        assert_eq!(
            config.describe(),
            "notifying blocked,waiting on https://ntfy.sh/agents"
        );
    }

    // AYEAYE-49 — the publish command line: --fail so an error page is a
    // refusal, the daemon's ten-second deadline, the JSON as one argv
    // element, and -- so a hostile base cannot be read as a flag.
    #[test]
    fn the_publish_command_line_is_exactly_this() {
        assert_eq!(
            argv("https://ntfy.sh", r#"{"topic":"agents"}"#),
            [
                "curl",
                "--fail",
                "--silent",
                "--show-error",
                "--max-time",
                "10",
                "--header",
                "Content-Type: application/json",
                "--data-binary",
                r#"{"topic":"agents"}"#,
                "--",
                "https://ntfy.sh/",
            ]
        );
    }
}
