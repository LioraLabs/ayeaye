use std::io::Write;
use std::path::{Path, PathBuf};

use base64::Engine;
use p256::elliptic_curve::sec1::ToEncodedPoint;
use serde::{Deserialize, Serialize};

pub const SUBSCRIPTIONS_FILE: &str = "push-subscriptions.json";
pub const VAPID_KEY_FILE: &str = "vapid-private-key";

pub fn public_key(state_dir: &Path) -> std::io::Result<String> {
    let path = state_dir.join(VAPID_KEY_FILE);
    let secret = match std::fs::read(&path)
        .ok()
        .and_then(|bytes| p256::SecretKey::from_slice(&bytes).ok())
    {
        Some(secret) => secret,
        None => {
            std::fs::create_dir_all(state_dir)?;
            let secret = p256::SecretKey::random(&mut rand_core::OsRng);
            let temporary = path.with_extension(format!("tmp{}", std::process::id()));
            atomic_write(&temporary, secret.to_bytes().as_ref())?;
            std::fs::rename(temporary, &path)?;
            secret
        }
    };
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(secret.public_key().to_encoded_point(false).as_bytes()))
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct Subscription {
    pub endpoint: String,
    pub p256dh: String,
    pub auth: String,
}

pub struct Store {
    state_dir: PathBuf,
    path: PathBuf,
    subscriptions: Vec<Subscription>,
}

impl Store {
    pub fn load(state_dir: &Path) -> Store {
        let path = state_dir.join(SUBSCRIPTIONS_FILE);
        let subscriptions = std::fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        Store {
            state_dir: state_dir.to_path_buf(),
            path,
            subscriptions,
        }
    }

    pub fn subscriptions(&self) -> &[Subscription] {
        &self.subscriptions
    }

    pub fn public_key(&self) -> std::io::Result<String> {
        public_key(&self.state_dir)
    }

    pub fn upsert(&mut self, subscription: Subscription) -> std::io::Result<()> {
        match self
            .subscriptions
            .iter_mut()
            .find(|held| held.endpoint == subscription.endpoint)
        {
            Some(held) => *held = subscription,
            None => self.subscriptions.push(subscription),
        }
        self.save()
    }

    pub fn remove(&mut self, endpoint: &str) -> std::io::Result<()> {
        self.subscriptions
            .retain(|subscription| subscription.endpoint != endpoint);
        self.save()
    }

    fn save(&self) -> std::io::Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let temporary = self
            .path
            .with_extension(format!("tmp{}", std::process::id()));
        let bytes = serde_json::to_vec(&self.subscriptions).map_err(std::io::Error::other)?;
        atomic_write(&temporary, &bytes)?;
        std::fs::rename(temporary, &self.path)
    }
}

fn atomic_write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

#[derive(Deserialize)]
pub struct Unsubscribe {
    pub endpoint: String,
}

#[derive(Deserialize)]
pub struct SubscriptionInput {
    pub endpoint: String,
    pub keys: Keys,
}

#[derive(Deserialize)]
pub struct Keys {
    pub p256dh: String,
    pub auth: String,
}

impl From<SubscriptionInput> for Subscription {
    fn from(input: SubscriptionInput) -> Subscription {
        Subscription {
            endpoint: input.endpoint,
            p256dh: input.keys.p256dh,
            auth: input.keys.auth,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Store, Subscription, public_key};

    fn scratch(name: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("push-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn subscription(endpoint: &str, key: &str) -> Subscription {
        Subscription {
            endpoint: endpoint.to_string(),
            p256dh: key.to_string(),
            auth: "auth".to_string(),
        }
    }

    // AYEAYE-83 — subscriptions coexist, and the endpoint is their identity.
    #[test]
    fn subscriptions_persist_and_upsert_by_endpoint() {
        let state = scratch("store");
        let mut store = Store::load(&state);
        store.upsert(subscription("https://one", "old")).unwrap();
        store.upsert(subscription("https://two", "other")).unwrap();
        store.upsert(subscription("https://one", "new")).unwrap();

        let loaded = Store::load(&state);
        assert_eq!(loaded.subscriptions().len(), 2);
        assert_eq!(loaded.subscriptions()[0].p256dh, "new");
        assert_eq!(loaded.subscriptions()[1].endpoint, "https://two");
    }

    // AYEAYE-83 — corrupt state is empty, and removing an endpoint is retry-safe.
    #[test]
    fn corrupt_state_recovers_and_remove_is_retry_safe() {
        let state = scratch("remove");
        std::fs::write(state.join(super::SUBSCRIPTIONS_FILE), "{ broken").unwrap();
        let mut store = Store::load(&state);
        assert!(store.subscriptions().is_empty());
        store.upsert(subscription("https://one", "key")).unwrap();
        store.remove("https://one").unwrap();
        store.remove("https://one").unwrap();
        assert!(Store::load(&state).subscriptions().is_empty());
    }

    // AYEAYE-83 — the browser receives one stable, uncompressed P-256 point.
    #[test]
    fn vapid_public_key_survives_a_restart_in_browser_shape() {
        let state = scratch("vapid");
        let first = public_key(&state).unwrap();
        let second = public_key(&state).unwrap();
        assert_eq!(first, second);
        assert_eq!(first.len(), 87);
        assert!(!first.contains(['=', '+', '/']));
        assert!(state.join(super::VAPID_KEY_FILE).is_file());
    }
}
