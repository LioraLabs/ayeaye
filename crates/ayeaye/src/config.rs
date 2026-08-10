//! What this server was told to be.
//!
//! Resolution is split in two on purpose. [`Settings::resolve`] takes the
//! environment as a function, so every "which port wins" decision is testable
//! without setting a variable in a process shared with the other tests.
//! [`load_token`] is the part that genuinely has to open a file, and it is the
//! only part that does.

use std::fmt;
use std::path::{Path, PathBuf};

use ayeaye_core::http::hosts::AllowedHosts;
use ayeaye_core::peer::{BadName, HostName, Peer, Registry};

use crate::tmux::Tmux;

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

/// What this machine is called when nothing else says.
///
/// There is no unnamed machine: every pane id carries a host, so a machine that
/// will not say what it is called still needs something an id can be built from
/// and split back apart by.
pub const DEFAULT_NAME: &str = "localhost";

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
    /// The machines this deployment can see. Today that is this one, as a peer
    /// with no transport — which is the point: the local machine being a peer
    /// like any other is what a second machine slots into later.
    pub peers: Registry,
    /// The tmux the panes are read from. A field rather than a constant so the
    /// suite can point a whole server at its own private tmux server instead of
    /// at the one holding somebody's work.
    pub tmux: Tmux,
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
    /// This machine's name is not one a pane id could carry. Refused at startup
    /// rather than served: a name with the separator in it produces ids that
    /// split back into a different machine, and a deployment that cannot route
    /// its own ids should say so before it answers anything.
    BadName(BadName),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfigError::MissingValue(flag) => write!(out, "{flag} needs a value"),
            ConfigError::NotAPort(value) => write!(out, "{value:?} is not a port number"),
            ConfigError::UnknownArgument(arg) => write!(out, "unknown argument {arg:?}"),
            ConfigError::NoToken(why) => write!(out, "{why}"),
            ConfigError::BadName(BadName::Empty) => {
                write!(out, "this machine's name is empty")
            }
            ConfigError::BadName(BadName::HasSeparator(name)) => write!(
                out,
                "{name:?} cannot be this machine's name: pane ids are host/pane, \
                 so a name carrying '/' makes ids nothing can route"
            ),
            ConfigError::BadName(BadName::HasControl(name)) => write!(
                out,
                "{name:?} cannot be this machine's name: it carries a control character"
            ),
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
        nodename: Option<String>,
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
        // The daemon's own order: the configured name first, so a screenshot
        // need not show a real hostname; then what the machine calls itself;
        // then a name it can at least be routed by, because every pane id on
        // this machine is qualified with this and there is no "unnamed".
        let named = env("NAME")
            .or(nodename)
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_NAME.to_string());
        let here = HostName::new(&named).map_err(ConfigError::BadName)?;
        Ok(Settings {
            allowed_hosts: AllowedHosts::new(&bind, port, &extra),
            bind,
            port,
            token,
            peers: Registry::new(vec![Peer::here(here)]).expect("one peer, and it is this machine"),
            tmux: Tmux::new(),
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
    prefixed(name, |key| std::env::var(key).ok())
}

/// The prefix search itself, with the environment as an argument.
///
/// Separate from [`env_var`] because the *order* is the part that matters and
/// the part that can be got wrong, and a test that had to set a real variable
/// would be a test racing every other test in the process.
fn prefixed(name: &str, lookup: impl Fn(&str) -> Option<String>) -> Option<String> {
    for prefix in ["AYEAYE_", "VOICE_REMOTE_"] {
        if let Some(value) = lookup(&format!("{prefix}{name}"))
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
    let path = state_dir().map(|dir| dir.join("token"));
    if let Some(path) = &path
        && let Ok(contents) = std::fs::read_to_string(path)
    {
        let token = contents.trim();
        if !token.is_empty() {
            return Ok(token.to_string());
        }
    }
    Err(ConfigError::NoToken(format!(
        "no token: set AYEAYE_TOKEN, or start the Python daemon once so it writes {}",
        path.map(|path| path.display().to_string())
            .unwrap_or_else(|| "the state file".to_string())
    )))
}

/// Where the daemon keeps its state, or `None` if there is no home to look in.
///
/// Public because the model store lives under it and `ayeaye model` has to
/// agree with the token loader about which directory that is. Two answers to
/// "where does this machine keep ayeaye's things" is how a pull lands somewhere
/// a load does not look.
pub fn state_dir() -> Option<PathBuf> {
    let base = std::env::var("XDG_STATE_HOME")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from)
        // Not `std::env::home_dir`: the workspace declares an MSRV of 1.85 and
        // nothing here builds at it, so the un-deprecation is not worth
        // depending on for a daemon that only runs on Unix anyway.
        .or_else(|| {
            std::env::var("HOME")
                .ok()
                .filter(|value| !value.trim().is_empty())
                .map(|home| PathBuf::from(home).join(".local/state"))
        })?;
    Some(choose_state_dir(&base, |path| path.is_dir()))
}

/// Which of the two names the state lives under.
///
/// One directory, not two tried in turn — that is what `bin/ayeaye`'s
/// `_state_dir` does, and the difference is load-bearing while both daemons
/// run. If the legacy directory holds a token and the new one exists but is
/// empty, "try both" would authenticate against the stale secret while the
/// daemon mints a fresh one in the new directory, and the two would disagree
/// about the shared secret with nothing saying so.
///
/// `exists` is an argument so this decision is testable without laying out
/// directories on disk.
fn choose_state_dir(base: &Path, exists: impl Fn(&Path) -> bool) -> PathBuf {
    let current = base.join("ayeaye");
    let legacy = base.join("voice-remote");
    if exists(&legacy) && !exists(&current) {
        legacy
    } else {
        current
    }
}

#[cfg(test)]
mod tests {
    use super::{ConfigError, DEFAULT_DEV_PORT, Path, Settings};

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
        Settings::resolve(
            &args(list),
            env,
            "s3cret".to_string(),
            Some("box".to_string()),
        )
    }

    // AYEAYE-42 — the newer name wins, and the legacy one is only used when it
    // is the only one there. Trying both in turn would let this daemon
    // authenticate against a stale secret while the Python one mints a fresh
    // secret next door, and nothing would say the two had diverged.
    #[test]
    fn the_state_directory_is_chosen_the_way_the_daemon_chooses_it() {
        let base = Path::new("/state");
        let current = base.join("ayeaye");
        let legacy = base.join("voice-remote");

        // Neither exists yet: the new name, so a first run writes there.
        assert_eq!(super::choose_state_dir(base, |_| false), current);
        // Only the legacy one: keep using it.
        assert_eq!(super::choose_state_dir(base, |p| p == legacy), legacy);
        // Only the new one, or both: the new one.
        assert_eq!(super::choose_state_dir(base, |p| p == current), current);
        assert_eq!(super::choose_state_dir(base, |_| true), current);
    }

    // AYEAYE-42 — the AYEAYE_ prefix wins over the legacy VOICE_REMOTE_ one,
    // and an empty value is unset rather than the empty string: a service unit
    // that writes `AYEAYE_BIND=` must not bind nothing.
    #[test]
    fn the_current_prefix_wins_and_empty_is_unset() {
        let both = |key: &str| match key {
            "AYEAYE_BIND" => Some("new".to_string()),
            "VOICE_REMOTE_BIND" => Some("old".to_string()),
            _ => None,
        };
        assert_eq!(super::prefixed("BIND", both), Some("new".to_string()));

        let legacy_only = |key: &str| match key {
            "VOICE_REMOTE_BIND" => Some("old".to_string()),
            _ => None,
        };
        assert_eq!(
            super::prefixed("BIND", legacy_only),
            Some("old".to_string())
        );

        let blank_current = |key: &str| match key {
            "AYEAYE_BIND" => Some("   ".to_string()),
            "VOICE_REMOTE_BIND" => Some("old".to_string()),
            _ => None,
        };
        assert_eq!(
            super::prefixed("BIND", blank_current),
            Some("old".to_string()),
            "a blank current value must fall through, not win"
        );
        assert_eq!(super::prefixed("BIND", |_| None), None);
        // Values are trimmed, so a trailing newline in an env file is not a host.
        assert_eq!(
            super::prefixed("BIND", |_| Some(" 0.0.0.0\n".to_string())),
            Some("0.0.0.0".to_string())
        );
    }

    // AYEAYE-42 — "the port is configurable so both daemons can run side by
    // side". The default has to be a port the Python daemon is not already
    // holding, or the two cannot run at once at all.
    #[test]
    fn the_default_port_is_not_the_one_the_python_daemon_holds() {
        // The claim is about the constant, not about this call: asserting it
        // on `settings.port` after asserting that equals the constant is a
        // second assertion that can never fail on its own.
        assert_ne!(
            DEFAULT_DEV_PORT, 8911,
            "the daemon's port cannot be the default"
        );
        let settings = resolve(&[], no_env).expect("the defaults should resolve");
        assert_eq!(settings.port, DEFAULT_DEV_PORT);
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

    // AYEAYE-43 — what this machine calls itself, in the daemon's own order:
    // the configured name first, so a screenshot need not show a real hostname,
    // then what `uname -n` said, and only then a default. The name is a peer's
    // name now rather than a header label, so it is also what every pane id on
    // this machine is qualified with.
    #[test]
    fn the_configured_name_wins_over_the_nodename_and_something_always_answers() {
        let named = |name: &str| match name {
            "NAME" => Some("desktop".to_string()),
            _ => None,
        };
        let configured =
            Settings::resolve(&[], named, "s3cret".to_string(), Some("box".to_string()))
                .expect("a named machine");
        assert_eq!(configured.peers.here().name().as_str(), "desktop");
        assert!(
            configured.peers.here().is_here(),
            "this machine has no transport"
        );

        let from_uname =
            Settings::resolve(&[], no_env, "s3cret".to_string(), Some("box".to_string()))
                .expect("an unnamed machine still has a nodename");
        assert_eq!(from_uname.peers.here().name().as_str(), "box");

        // And a machine that will not say: a name it can be routed by beats no
        // name at all, since every pane id on it needs one.
        let anonymous =
            Settings::resolve(&[], no_env, "s3cret".to_string(), None).expect("a nameless machine");
        assert_eq!(anonymous.peers.here().name().as_str(), "localhost");
        let blank = Settings::resolve(&[], no_env, "s3cret".to_string(), Some("  ".to_string()))
            .expect("a blank nodename is no nodename");
        assert_eq!(blank.peers.here().name().as_str(), "localhost");
    }

    // AYEAYE-43 — a name a pane id cannot carry stops the server rather than
    // being escaped or quietly replaced. Ids built from it split back into a
    // different machine, and a deployment that cannot route its own ids should
    // say so before it answers anything.
    #[test]
    fn a_name_no_pane_id_could_carry_refuses_to_start() {
        let slashed = |name: &str| match name {
            "NAME" => Some("gpu/box".to_string()),
            _ => None,
        };
        let refused = Settings::resolve(&[], slashed, "s3cret".to_string(), None).unwrap_err();
        assert_eq!(
            refused,
            ConfigError::BadName(ayeaye_core::peer::BadName::HasSeparator(
                "gpu/box".to_string()
            ))
        );
        // And it says which name and why, because the fix is to change that
        // value and nothing else here can say which one it was.
        let said = refused.to_string();
        assert!(
            said.contains("gpu/box") && said.contains("host/pane"),
            "{said}"
        );
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
