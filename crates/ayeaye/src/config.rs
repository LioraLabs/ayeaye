//! What this server was told to be.
//!
//! Resolution is split in two on purpose. [`Settings::resolve`] takes the
//! environment as a function, so every "which port wins" decision is testable
//! without setting a variable in a process shared with the other tests.
//! [`load_token`] is the part that genuinely has to open a file, and it is the
//! only part that does.

use std::fmt;

use ayeaye_core::http::hosts::AllowedHosts;

/// The port the Rust daemon listens on until the cutover.
///
/// Deliberately not 8911. The Python daemon keeps the real port for the rest
/// of the milestone, and the two have to be able to run side by side on one
/// machine — which is also why this reads `AYEAYE_DEV_PORT` and not
/// `AYEAYE_PORT`: a shell already configured for the daemon must not drag this
/// onto the port that daemon is already holding.
pub const DEFAULT_DEV_PORT: u16 = 8912;

/// The address to bind when nothing says otherwise.
pub const DEFAULT_BIND: &str = "127.0.0.1";

/// Everything the server needs to answer a request.
#[derive(Debug, Clone)]
pub struct Settings {
    /// The address to bind.
    pub bind: String,
    /// The port to bind.
    pub port: u16,
    /// The `Host` values this server answers to.
    pub allowed_hosts: AllowedHosts,
    /// The shared secret every gated request has to present.
    pub token: String,
}

/// Why a configuration could not be resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigError {
    /// A `--flag` that needs a value did not get one.
    MissingValue(String),
    /// A value was given but could not be read as what it has to be.
    NotAPort(String),
    /// An argument nobody recognises. Refused rather than ignored: a typo'd
    /// `--prot 9000` that silently binds 8912 is worse than a refusal.
    UnknownArgument(String),
    /// No token in the environment and none on disk.
    NoToken(String),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfigError::MissingValue(flag) => write!(out, "{flag} needs a value"),
            ConfigError::NotAPort(value) => write!(out, "{value:?} is not a port number"),
            ConfigError::UnknownArgument(arg) => write!(out, "unknown argument {arg:?}"),
            ConfigError::NoToken(why) => write!(out, "{why}"),
        }
    }
}

impl std::error::Error for ConfigError {}

impl Settings {
    /// Resolve the settings from arguments, the environment, and a token the
    /// caller has already found.
    ///
    /// `env` is a lookup rather than `std::env` so this is a decision a test
    /// can drive. It is handed the *bare* name — `BIND`, `DEV_PORT` — and is
    /// expected to try `AYEAYE_<name>` before the legacy `VOICE_REMOTE_<name>`,
    /// which is what [`env_var`] does.
    ///
    /// Arguments win over the environment, which wins over the defaults.
    pub fn resolve(
        args: &[String],
        env: impl Fn(&str) -> Option<String>,
        token: String,
    ) -> Result<Settings, ConfigError> {
        let mut bind = env("BIND").unwrap_or_else(|| DEFAULT_BIND.to_string());
        let mut port = match env("DEV_PORT") {
            Some(value) => parse_port(&value)?,
            None => DEFAULT_DEV_PORT,
        };

        let mut rest = args.iter();
        while let Some(arg) = rest.next() {
            match arg.as_str() {
                "--bind" => {
                    bind = rest
                        .next()
                        .ok_or_else(|| ConfigError::MissingValue("--bind".to_string()))?
                        .clone();
                }
                "--port" => {
                    let value = rest
                        .next()
                        .ok_or_else(|| ConfigError::MissingValue("--port".to_string()))?;
                    port = parse_port(value)?;
                }
                other => return Err(ConfigError::UnknownArgument(other.to_string())),
            }
        }

        let extra = env("ALLOWED_HOSTS").unwrap_or_default();
        Ok(Settings {
            allowed_hosts: AllowedHosts::new(&bind, port, &extra),
            bind,
            port,
            token,
        })
    }

    /// The address to hand a listener.
    pub fn address(&self) -> String {
        format!("{}:{}", self.bind, self.port)
    }
}

fn parse_port(value: &str) -> Result<u16, ConfigError> {
    value
        .trim()
        .parse()
        .map_err(|_| ConfigError::NotAPort(value.to_string()))
}

/// `AYEAYE_<name>`, falling back to the legacy `VOICE_REMOTE_<name>`.
///
/// Empty is treated as unset, so `AYEAYE_BIND=` in a service unit does not
/// bind the empty string.
pub fn env_var(name: &str) -> Option<String> {
    for prefix in ["AYEAYE_", "VOICE_REMOTE_"] {
        if let Ok(value) = std::env::var(format!("{prefix}{name}"))
            && !value.trim().is_empty()
        {
            return Some(value.trim().to_string());
        }
    }
    None
}

/// The shared secret, from the environment or from the file the Python daemon
/// wrote.
///
/// This does not *generate* a token when it finds none, and that is deliberate
/// while both daemons run: a second generated token would be a second secret,
/// and the phone is logged in with the first. Minting one belongs to
/// `ayeaye setup`, once there is only one daemon left to own it.
pub fn load_token() -> Result<String, ConfigError> {
    if let Some(token) = env_var("TOKEN") {
        return Ok(token);
    }
    for dir in state_dirs() {
        let path = dir.join("token");
        if let Ok(contents) = std::fs::read_to_string(&path) {
            let token = contents.trim();
            if !token.is_empty() {
                return Ok(token.to_string());
            }
        }
    }
    Err(ConfigError::NoToken(format!(
        "no token: set AYEAYE_TOKEN, or start the Python daemon once so it writes {}",
        state_dirs()
            .first()
            .map(|dir| dir.join("token").display().to_string())
            .unwrap_or_default()
    )))
}

/// Where the daemon keeps its state, newest naming first.
fn state_dirs() -> Vec<std::path::PathBuf> {
    let base = std::env::var("XDG_STATE_HOME")
        .ok()
        .filter(|value| !value.is_empty())
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::home_dir().map(|home| home.join(".local/state")));
    match base {
        Some(base) => vec![base.join("ayeaye"), base.join("voice-remote")],
        None => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::{ConfigError, DEFAULT_DEV_PORT, Settings};

    fn no_env(_: &str) -> Option<String> {
        None
    }

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|arg| arg.to_string()).collect()
    }

    fn resolve(
        list: &[&str],
        env: impl Fn(&str) -> Option<String>,
    ) -> Result<Settings, ConfigError> {
        Settings::resolve(&args(list), env, "s3cret".to_string())
    }

    // AYEAYE-42 — "the port is configurable so both daemons can run side by
    // side". The default has to be a port the Python daemon is not already
    // holding, or the two cannot run at once at all.
    #[test]
    fn the_default_port_is_not_the_one_the_python_daemon_holds() {
        let settings = resolve(&[], no_env).expect("the defaults should resolve");
        assert_eq!(settings.port, DEFAULT_DEV_PORT);
        assert_ne!(
            settings.port, 8911,
            "the daemon's port cannot be the default"
        );
        assert_eq!(settings.bind, "127.0.0.1");
        assert_eq!(settings.address(), format!("127.0.0.1:{DEFAULT_DEV_PORT}"));
    }

    // AYEAYE-42 — an argument beats the environment, and the environment beats
    // the default; without the middle rung a service unit cannot set the port
    // at all.
    #[test]
    fn an_argument_beats_the_environment_which_beats_the_default() {
        let env = |name: &str| match name {
            "DEV_PORT" => Some("9101".to_string()),
            "BIND" => Some("0.0.0.0".to_string()),
            _ => None,
        };
        let from_env = resolve(&[], env).expect("the environment should resolve");
        assert_eq!(from_env.port, 9101);
        assert_eq!(from_env.bind, "0.0.0.0");

        let overridden = resolve(&["--port", "9202", "--bind", "127.0.0.2"], env)
            .expect("arguments should resolve");
        assert_eq!(overridden.port, 9202);
        assert_eq!(overridden.bind, "127.0.0.2");
    }

    // AYEAYE-42 — the allow-list is built from the port that won, not the one
    // that was asked for: a server on 9202 that only trusts Host values naming
    // 8912 refuses every request it receives.
    #[test]
    fn the_allow_list_is_built_from_the_port_that_won() {
        let settings = resolve(&["--port", "9202"], no_env).expect("arguments should resolve");
        assert!(settings.allowed_hosts.allows("127.0.0.1:9202"));
        assert!(!settings.allowed_hosts.allows("evil.example:9202"));
    }

    // AYEAYE-42 — a typo has to be refused rather than absorbed: `--prot 9000`
    // that silently binds the default is a server nobody can find.
    #[test]
    fn a_bad_argument_is_refused_rather_than_ignored() {
        assert_eq!(
            resolve(&["--prot", "9000"], no_env).unwrap_err(),
            ConfigError::UnknownArgument("--prot".to_string())
        );
        assert_eq!(
            resolve(&["--port"], no_env).unwrap_err(),
            ConfigError::MissingValue("--port".to_string())
        );
        assert_eq!(
            resolve(&["--port", "http"], no_env).unwrap_err(),
            ConfigError::NotAPort("http".to_string())
        );
    }
}
