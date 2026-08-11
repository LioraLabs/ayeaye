//! The notification watcher: tell you when a session starts needing you,
//! once per change.
//!
//! The decision is `ayeaye_core::notify`'s; this is the loop around it and
//! the transport under it. The transport is `curl`, and that is the model
//! store's precedent applied for the model store's reason: a push endpoint is
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
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use rand_core::{OsRng, RngCore};

use crate::config::Settings;

/// How long one publish may hold a process.
///
/// The daemon gives `urlopen` ten seconds; curl gets the same deadline via
/// `--max-time` below, and this is the kill-on-drop backstop around it.
const PUBLISH_LIMIT: Duration = Duration::from_secs(15);

/// How often the board is swept when nothing says otherwise.
const DEFAULT_EVERY: u64 = 10;

/// How often to look and which states are worth a message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// The gap between sweeps.
    every: Duration,
    /// The states that fire.
    wanted: BTreeSet<State>,
}

impl Config {
    /// The watcher's settings. An `EVERY` that
    /// does not parse falls back to the default where the daemon's `int()`
    /// would have refused to start: a mistyped nicety must not take the
    /// daemon down with it.
    pub fn resolve(look_up: impl Fn(&str) -> Option<String>) -> Config {
        let every = named(&look_up, "NOTIFY_EVERY")
            .and_then(|value| value.parse().ok())
            .unwrap_or(DEFAULT_EVERY);
        let states = named(&look_up, "NOTIFY_STATES");
        Config {
            every: Duration::from_secs(every),
            wanted: notify::wanted(states.as_deref().unwrap_or(notify::DEFAULT_STATES)),
        }
    }

    /// One line for the startup journal: what fires, and where it goes.
    pub fn describe(&self) -> String {
        let mut states = self
            .wanted
            .iter()
            .map(|state| state.as_str())
            .collect::<Vec<_>>();
        states.sort_unstable();
        format!("notifying {} with Web Push", states.join(","))
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
                publish(&settings, &notify::message(card)).await;
            }
            seen = next;
        }
    });
}

/// One message, published, best effort.
async fn publish(settings: &Settings, message: &notify::Message) {
    let Some(store) = &settings.push else {
        return;
    };
    deliver(
        store,
        message,
        "curl",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
    )
    .await;
}

pub async fn deliver(
    store: &Arc<std::sync::Mutex<crate::push::Store>>,
    message: &notify::Message,
    program: &str,
    now: u64,
) {
    let (private_key, subscriptions) = {
        let store = store
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let private_key = match store.private_key() {
            Ok(key) => key,
            Err(why) => {
                eprintln!("ayeaye: notify failed: {why}");
                return;
            }
        };
        (private_key, store.subscriptions().to_vec())
    };
    let payload = notify::payload(message);
    for subscription in subscriptions {
        let result = push(
            &subscription,
            &private_key,
            payload.as_bytes(),
            message.priority,
            program,
            now,
        )
        .await;
        match result {
            Ok(404 | 410) => {
                let removed = store
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .remove_if_current(&subscription);
                if let Err(why) = removed {
                    eprintln!("ayeaye: notify failed: {why}");
                }
            }
            Ok(200..=299) => {}
            Ok(status) => eprintln!("ayeaye: notify failed: HTTP {status}"),
            Err(why) => eprintln!("ayeaye: notify failed: {why}"),
        }
    }
}

async fn push(
    subscription: &crate::push::Subscription,
    vapid_private: &[u8; 32],
    payload: &[u8],
    priority: u8,
    program: &str,
    now: u64,
) -> Result<u16, String> {
    let user_public = URL_SAFE_NO_PAD
        .decode(&subscription.p256dh)
        .map_err(|why| why.to_string())?;
    let auth = URL_SAFE_NO_PAD
        .decode(&subscription.auth)
        .map_err(|why| why.to_string())?;
    let ephemeral = p256::SecretKey::random(&mut OsRng);
    let ephemeral: [u8; 32] = ephemeral.to_bytes().into();
    let mut salt = [0; 16];
    OsRng.fill_bytes(&mut salt);
    let encrypted = ayeaye_core::web_push::encrypt(payload, &user_public, &auth, &salt, &ephemeral)
        .map_err(|why| format!("encryption: {why:?}"))?;
    let signed = ayeaye_core::web_push::vapid(
        &subscription.endpoint,
        now + 43_200,
        now,
        "https://github.com/LioraLabs/ayeaye",
        vapid_private,
    )
    .map_err(|why| format!("VAPID: {why:?}"))?;
    let urgency = if priority >= 4 { "high" } else { "normal" };
    let argv = vec![
        program.to_string(),
        "--silent".to_string(),
        "--show-error".to_string(),
        "--max-time".to_string(),
        "10".to_string(),
        "--output".to_string(),
        "/dev/null".to_string(),
        "--write-out".to_string(),
        "%{http_code}".to_string(),
        "--request".to_string(),
        "POST".to_string(),
        "--header".to_string(),
        format!("Content-Encoding: {}", encrypted.content_encoding),
        "--header".to_string(),
        "Content-Type: application/octet-stream".to_string(),
        "--header".to_string(),
        "TTL: 2419200".to_string(),
        "--header".to_string(),
        format!("Urgency: {urgency}"),
        "--header".to_string(),
        format!("Authorization: {}", signed.authorization),
        "--data-binary".to_string(),
        "@-".to_string(),
        "--".to_string(),
        subscription.endpoint.clone(),
    ];
    let ran = crate::command::run_with_input(&argv, &encrypted.body, PUBLISH_LIMIT)
        .await
        .map_err(|why| why.to_string())?;
    if !ran.ok {
        return Err(ran.stderr.trim().to_string());
    }
    ran.stdout
        .trim()
        .parse()
        .map_err(|_| format!("bad HTTP status {:?}", ran.stdout.trim()))
}

#[cfg(test)]
mod tests {
    use super::{Config, deliver};
    use ayeaye_core::notify;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::Arc;
    use std::time::Duration;

    fn env(values: Vec<(&'static str, &'static str)>) -> impl Fn(&str) -> Option<String> {
        move |name| {
            values
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| value.to_string())
        }
    }

    // AYEAYE-84 — the watcher exists without transport configuration; an
    // empty subscription store makes delivery the no-op.
    #[test]
    fn no_transport_setting_is_required() {
        let config = Config::resolve(env(vec![]));
        assert_eq!(config.every, Duration::from_secs(10));
        assert_eq!(config.wanted, notify::wanted("blocked,waiting"));
    }

    // AYEAYE-49 — the sweep gap and the states narrow by configuration, and
    // an EVERY that does not parse falls back rather than taking the daemon
    // down: a mistyped nicety is not a reason to refuse to serve.
    #[test]
    fn the_gap_and_the_states_narrow_by_configuration() {
        let config = Config::resolve(env(vec![
            ("VOICE_NOTIFY_EVERY", "30"),
            ("VOICE_NOTIFY_STATES", "blocked"),
        ]));
        assert_eq!(config.every, Duration::from_secs(30));
        assert_eq!(config.wanted, notify::wanted("blocked"));

        let mistyped = Config::resolve(env(vec![("VOICE_NOTIFY_EVERY", "soon")]));
        assert_eq!(mistyped.every, Duration::from_secs(10));
    }

    // AYEAYE-49 — the startup line says what fires and where it goes, which
    // is the one place a misconfigured topic shows itself before the first
    // quiet hour is mistaken for peace.
    #[test]
    fn the_journal_line_names_the_states_and_the_transport() {
        let config = Config::resolve(env(vec![]));
        assert_eq!(config.describe(), "notifying blocked,waiting with Web Push");
    }

    // AYEAYE-84 — every subscription is attempted, binary aes128gcm and the
    // required headers reach curl, and a gone endpoint alone is forgotten.
    #[tokio::test]
    async fn delivery_continues_and_permanently_drops_gone_subscriptions() {
        let state =
            std::env::temp_dir().join(format!("ayeaye-push-delivery-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&state);
        std::fs::create_dir_all(&state).unwrap();
        let script = state.join("curl");
        std::fs::write(
            &script,
            format!(
                r#"#!/bin/sh
for endpoint
do
  :
done
case "$endpoint" in
  *failed*) id=failed; status=503 ;;
  *gone*) id=gone; status=410 ;;
  *missing*) id=missing; status=404 ;;
  *) id=live; status=201 ;;
esac
cat > "{0}/$id.body"
printf '%s\n' "$@" > "{0}/$id.args"
printf '%s' "$status"
"#,
                state.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();

        let mut store = crate::push::Store::load(&state);
        for endpoint in ["failed", "gone", "missing", "live"] {
            store
                .upsert(crate::push::Subscription {
                    endpoint: format!("https://push.example/{endpoint}"),
                    p256dh: "BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4".to_string(),
                    auth: "BTBZMqHH6r4Tts7J_aSIgg".to_string(),
                })
                .unwrap();
        }
        let message = notify::Message {
            title: "agent needs you".to_string(),
            body: "Choose one".to_string(),
            priority: 4,
        };
        let store = Arc::new(std::sync::Mutex::new(store));
        deliver(&store, &message, script.to_str().unwrap(), 1_700_000_000).await;

        for endpoint in ["failed", "gone", "missing", "live"] {
            assert!(
                state
                    .join(format!("{endpoint}.body"))
                    .metadata()
                    .unwrap()
                    .len()
                    > 86
            );
        }
        let args = std::fs::read_to_string(state.join("live.args")).unwrap();
        for header in [
            "Content-Encoding: aes128gcm",
            "TTL: 2419200",
            "Urgency: high",
            "Authorization: vapid t=",
        ] {
            assert!(args.contains(header), "missing {header:?} in {args}");
        }
        let stored = crate::push::Store::load(&state);
        assert_eq!(
            stored
                .subscriptions()
                .iter()
                .map(|subscription| subscription.endpoint.as_str())
                .collect::<Vec<_>>(),
            ["https://push.example/failed", "https://push.example/live"]
        );
    }

    // AYEAYE-84 — a permanent response belongs to the keys that were sent;
    // it must not erase replacement keys registered while curl was in flight.
    #[tokio::test]
    async fn stale_gone_response_keeps_a_replacement_subscription() {
        let state = std::env::temp_dir().join(format!(
            "ayeaye-push-delivery-replacement-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&state);
        std::fs::create_dir_all(&state).unwrap();
        let script = state.join("curl");
        std::fs::write(
            &script,
            format!(
                r#"#!/bin/sh
touch "{0}/started"
while test ! -f "{0}/release"
do
  sleep 0.01
done
cat >/dev/null
printf 410
"#,
                state.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();

        let endpoint = "https://push.example/replaced";
        let old = crate::push::Subscription {
            endpoint: endpoint.to_string(),
            p256dh: "BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4".to_string(),
            auth: "BTBZMqHH6r4Tts7J_aSIgg".to_string(),
        };
        let replacement = crate::push::Subscription {
            endpoint: endpoint.to_string(),
            p256dh: "BPUt7QNVMuU5BT9xEOUDESWgf8_B0cDsIMZQVou-DVZe0T60XJQFDFkCYX9-n7R0tC7QpM-nOPzyQxUwDpOJQ-I".to_string(),
            auth: "6I3hFM3LWVox3SuX1mCK2A".to_string(),
        };
        let mut held = crate::push::Store::load(&state);
        held.upsert(old).unwrap();
        let store = Arc::new(std::sync::Mutex::new(held));
        let task = tokio::spawn({
            let store = Arc::clone(&store);
            let program = script.to_string_lossy().into_owned();
            async move {
                deliver(
                    &store,
                    &notify::Message {
                        title: "title".to_string(),
                        body: "body".to_string(),
                        priority: 3,
                    },
                    &program,
                    1_700_000_000,
                )
                .await;
            }
        });
        while !state.join("started").exists() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        store.lock().unwrap().upsert(replacement.clone()).unwrap();
        std::fs::write(state.join("release"), "").unwrap();
        task.await.unwrap();

        assert_eq!(store.lock().unwrap().subscriptions(), [replacement]);
    }
}
