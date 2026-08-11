//! What this server was told to be.
//!
//! Resolution is split in two on purpose. [`Settings::resolve`] takes the
//! environment as a function, so every "which port wins" decision is testable
//! without setting a variable in a process shared with the other tests.
//! [`load_token`] is the part that genuinely has to open a file, and it is the
//! only part that does.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use ayeaye_core::http::hosts::AllowedHosts;
use ayeaye_core::pane;
use ayeaye_core::peer::{BadName, HostName, Peer, Registry};

use crate::dictate::Voice;
use crate::fit::{Fits, STATE_FILE};
use crate::session::Agents;
use crate::tmux::Tmux;

use crate::cliban::Cliban;

/// The port ayeaye listens on when nothing says otherwise.
pub const DEFAULT_PORT: u16 = 8911;

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
    /// Where the agents keep what they leave behind. A field for the same
    /// reason `tmux` is one: without it the suite reads whatever is in the
    /// home directory of whoever is running it.
    pub agents: Agents,
    /// The cliban the board endpoints ask. Resolved once, at startup, because
    /// that is when the daemon resolves it and because a PATH search per
    /// request is a directory walk per request.
    pub cliban: Cliban,
    /// The history windows the terminal view has recently served, so a poll can
    /// be answered with what changed. Shared across requests because that is
    /// what makes it a cache; keyed on what the *client* says it holds, which is
    /// what keeps it from being per-client state.
    pub pane_cache: Arc<Mutex<pane::Cache>>,
    /// The windows held at somebody's phone's size, and the file that outlives
    /// this process.
    pub fits: Arc<Fits>,
    /// The models a dictation runs through, and their lifetime. Shared because
    /// there is one of each per machine and inference is this process's own
    /// arithmetic: two requests must take turns rather than each load a model.
    pub voice: Arc<Voice>,
    /// Where spawn records a pick, so the picker has a history to rank by.
    ///
    /// The same file `projects::Settings::from_env` points the picker at —
    /// both derive it from [`state_dir`], which is what keeps the write where
    /// the read looks. `None` where there is no state directory, exactly as
    /// `fits` has no path there: a machine with no home loses ranking history,
    /// and nothing else. A field rather than a call inside spawn for the same
    /// reason `tmux` is one — without it, the suite writes into the state
    /// directory of whoever is running it.
    pub store: Option<PathBuf>,
}

/// Why a configuration could not be resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigError {
    /// A `--flag` that needs a value did not get one.
    MissingValue(String),
    /// A value was given but could not be read as what it has to be.
    NotAPort(String),
    /// An argument nobody recognises. Refused rather than ignored: a typo'd
    /// `--prot 9000` that silently binds 8911 is worse than a refusal.
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
    /// `token`, `cliban` and `voice` arrive resolved, for the same reason:
    /// finding them means reading a file and walking a PATH, and none of it is
    /// a decision.
    ///
    /// `env` is a lookup rather than `std::env` so this is a decision a test
    /// can drive. It is handed the *bare* name — `BIND`, `PORT` — and is
    /// expected to try `AYEAYE_<name>` before the legacy `VOICE_REMOTE_<name>`.
    /// The daemon hands it [`env_then_file`], which layers the settings file
    /// `ayeaye setup` owns beneath the environment; [`env_var`] is the
    /// environment half alone.
    ///
    /// Arguments win over the environment, which wins over the defaults.
    pub fn resolve(
        args: &[String],
        env: impl Fn(&str) -> Option<String>,
        token: String,
        nodename: Option<String>,
        cliban: Cliban,
        voice: Arc<Voice>,
    ) -> Result<Settings, ConfigError> {
        let mut bind = env("BIND").unwrap_or_else(|| DEFAULT_BIND.to_string());
        let mut port = match env("PORT") {
            Some(value) => parse_port(&value)?,
            None => DEFAULT_PORT,
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
        let here = HostName::new(&machine_name(&env, nodename)).map_err(ConfigError::BadName)?;
        Ok(Settings {
            allowed_hosts: AllowedHosts::new(&bind, port, &extra),
            bind,
            port,
            token,
            peers: Registry::new(vec![Peer::here(here)]).expect("one peer, and it is this machine"),
            tmux: Tmux::new(),
            // A location rather than a decision, exactly as `Tmux::new()`
            // above is: it is where this user's home is, and a test that cares
            // replaces the field.
            agents: Agents::here(),
            cliban,
            pane_cache: Arc::new(Mutex::new(pane::Cache::default())),
            fits: Arc::new(Fits::new(ayeaye_core::fit::DEFAULT_TTL_MS, fits_path())),
            voice,
            store: state_dir().map(|dir| dir.join(crate::projects::store::FILE)),
        })
    }

    /// The address to hand a listener.
    pub fn address(&self) -> String {
        format!("{}:{}", self.bind, self.port)
    }
}

/// What this machine calls itself.
///
/// The daemon's own order: the configured name first, so a screenshot need not
/// show a real hostname; then what the machine calls itself; then a name it can
/// at least be routed by, because every pane id on this machine is qualified
/// with this and there is no "unnamed".
///
/// **Public, and shared, because two programs have to agree about it.** The
/// daemon qualifies every pane id it issues with this name, and `ayeaye dictate`
/// compares the id in its state file against the pane list under the same name —
/// so a second spelling of this rule anywhere is a toggle that cannot find the
/// panes the daemon named. It was written out twice once; this is that mistake
/// removed rather than commented on.
pub fn machine_name(env: impl Fn(&str) -> Option<String>, nodename: Option<String>) -> String {
    env("NAME")
        .or(nodename)
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_NAME.to_string())
}

/// Where the `cliban` the board endpoints run lives.
///
/// The daemon resolves this once at startup rather than per call, and the
/// comment on its own line says why: the systemd user PATH does not include
/// `~/.cargo/bin`, which is where cargo puts it. So a PATH search alone is not
/// enough, and neither is the fallback alone.
///
/// `look_up` and `executable` are arguments so the order is testable without a
/// process environment or a directory of stand-ins.
pub fn choose_cliban(
    look_up: impl Fn(&str) -> Option<String>,
    executable: impl Fn(&Path) -> bool,
) -> String {
    // `AYEAYE_CLIBAN` first for the name this daemon will be configured under,
    // then the one the Python daemon reads today. Not `VOICE_REMOTE_CLIBAN`:
    // that prefix belongs to the server's own settings and nothing has ever
    // set this under it.
    for name in ["AYEAYE_CLIBAN", "VOICE_CLIBAN"] {
        if let Some(named) = look_up(name).filter(|value| !value.trim().is_empty()) {
            return named.trim().to_string();
        }
    }
    if let Some(path) = look_up("PATH") {
        for directory in path.split(':').filter(|entry| !entry.is_empty()) {
            let candidate = Path::new(directory).join("cliban");
            if executable(&candidate) {
                return candidate.to_string_lossy().into_owned();
            }
        }
    }
    // Left as a literal `~` when there is no home to expand it against, which
    // is what `os.path.expanduser` does: a name that cannot be spawned states
    // its reason, where an empty one would spawn the current directory.
    match look_up("HOME").filter(|home| !home.trim().is_empty()) {
        Some(home) => format!("{}/.cargo/bin/cliban", home.trim_end_matches('/')),
        None => "~/.cargo/bin/cliban".to_string(),
    }
}

/// [`choose_cliban`], against the real environment and the real filesystem.
///
/// The one effectful wrapper, beside [`load_token`], and called once at startup
/// rather than per request.
pub fn locate_cliban() -> String {
    choose_cliban(|name| std::env::var(name).ok(), runnable)
}

/// Whether a path names something this process could run.
///
/// Two small differences from `shutil.which`, both narrowing. An empty `PATH`
/// entry is skipped rather than read as the current directory, so a stray
/// colon cannot make `./cliban` the board tool; and the bit is the file's own
/// executable bit rather than `access(X_OK)`, so a file only another user may
/// run is not chosen and then refused at spawn.
#[cfg(unix)]
fn runnable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .is_ok_and(|found| found.is_file() && found.permissions().mode() & 0o111 != 0)
}

/// Everywhere else there is no executable bit to read, so being a file is the
/// whole of what can be asked. Present for the same reason `service.rs` keeps
/// its pair: the daemon is a Unix daemon, and a crate that only compiles on
/// one platform hides that behind a build failure rather than stating it.
#[cfg(not(unix))]
fn runnable(path: &Path) -> bool {
    std::fs::metadata(path).is_ok_and(|found| found.is_file())
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

/// The settings file's answer for one bare name, or `None`.
///
/// `parse_env_file` hands keys back with the `AYEAYE_` prefix stripped, so an
/// `AYEAYE_BIND=` line and a bare `BIND=` line both answer to `BIND` — and a
/// legacy `VOICE_REMOTE_BIND=` line answers only when neither does, which is
/// [`prefixed`]'s order applied to the file. Within each name the *last* line
/// wins, as it does when systemd or a shell loads the same file, and an empty
/// value is unset rather than the empty string, as everywhere else here.
fn file_var(text: &str, name: &str) -> Option<String> {
    let pairs = ayeaye_core::model::settings::parse_env_file(text);
    let legacy = format!("VOICE_REMOTE_{name}");
    for wanted in [name, legacy.as_str()] {
        let answer = pairs
            .iter()
            .rev()
            .find(|(key, _)| key == wanted)
            .map(|(_, value)| value.trim().to_string())
            .filter(|value| !value.is_empty());
        if answer.is_some() {
            return answer;
        }
    }
    None
}

/// One bare name, resolved the way the running daemon resolves every setting:
/// the environment first, then the settings file `ayeaye setup` owns.
///
/// The environment as an argument, so the order — the part that can be got
/// wrong — is a decision a test can drive without setting a real variable.
pub(crate) fn layered(
    name: &str,
    env: impl Fn(&str) -> Option<String>,
    file: &str,
) -> Option<String> {
    prefixed(name, env).or_else(|| file_var(file, name))
}

/// A lookup for [`Settings::resolve`]: the environment, then the settings file.
///
/// This is what makes "configuration is a file the binary writes and reads"
/// true on every path the daemon starts from. The systemd unit also injects the
/// same file with `EnvironmentFile=`, so there the two layers agree about
/// everything; the launchd agent and a server started by hand inject nothing,
/// and without this the daemon on those paths would ignore the file setup had
/// just written — while `ayeaye check` read it, and the two would disagree
/// about which port was even being discussed.
///
/// The file is read once, here, so four settings do not cost four reads — and a
/// file that is not there is simply no second layer.
pub fn env_then_file(config_file: &Path) -> impl Fn(&str) -> Option<String> {
    let file = std::fs::read_to_string(config_file).unwrap_or_default();
    move |name| layered(name, |key| std::env::var(key).ok(), &file)
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

/// Where the fit leases are written between runs.
///
/// `None` where there is no state directory to write to: a daemon that cannot
/// survive its own restart is better than one that will not start. Nothing is
/// written until a lease is actually taken, so resolving settings on a machine
/// with no leases touches no file.
///
/// Deliberately **not** `bin/ayeaye`'s `fits.json` — see `crate::fit`.
fn fits_path() -> Option<PathBuf> {
    Some(state_dir()?.join(STATE_FILE))
}

/// Where the daemon keeps its state, or `None` if there is no home to look in.
///
/// Public because the model store and the pick history both live under it, and
/// `ayeaye model` has to agree with the token loader about which directory that
/// is. Two answers to "where does this machine keep ayeaye's things" is how a
/// pull lands somewhere a load does not look.
pub fn state_dir() -> Option<PathBuf> {
    let base = std::env::var("XDG_STATE_HOME")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from)
        // Not `std::env::home_dir`: on Unix — the only place this daemon
        // runs — it consults `$HOME` first anyway, so the un-deprecated
        // function would add a platform-generality layer this read of the
        // variable already matches.
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
    use super::{ConfigError, DEFAULT_PORT, Path, Settings};

    fn no_env(_: &str) -> Option<String> {
        None
    }

    /// A voice with no models and a converter that is not there, so nothing in
    /// this file can reach a model or start a process by accident.
    fn silent() -> std::sync::Arc<super::Voice> {
        std::sync::Arc::new(super::Voice::new(
            std::path::PathBuf::from("/nonexistent/store"),
            ayeaye_core::model::settings::ModelSettings::resolve(|_| None, "")
                .expect("the defaults resolve"),
            ayeaye_core::cleanup::Policy::default(),
            "ayeaye-58-no-such-converter".to_string(),
            ayeaye_infer::backend::select(),
        ))
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
            super::Cliban::new("/nonexistent/cliban".to_string()),
            silent(),
        )
    }

    // AYEAYE-53 — the daemon resolves cliban once at startup as
    // `VOICE_CLIBAN or shutil.which("cliban") or ~/.cargo/bin/cliban`, and the
    // third clause is the load-bearing one: the systemd user PATH does not
    // include `~/.cargo/bin`, so a service that only searched PATH would find
    // nothing on the very machine this runs on.
    #[test]
    fn cliban_is_found_where_the_daemon_looks_for_it() {
        let env = |values: Vec<(&'static str, &'static str)>| {
            move |name: &str| {
                values
                    .iter()
                    .find(|(key, _)| *key == name)
                    .map(|(_, value)| value.to_string())
            }
        };
        let nothing_runs = |_: &Path| false;

        // The variable wins outright, even over something on PATH.
        assert_eq!(
            super::choose_cliban(
                env(vec![("VOICE_CLIBAN", "/opt/cliban"), ("PATH", "/bin")]),
                |_| true
            ),
            "/opt/cliban"
        );
        // And the current name wins over the legacy one, which is the order
        // every other setting here is read in.
        assert_eq!(
            super::choose_cliban(
                env(vec![
                    ("AYEAYE_CLIBAN", "/opt/new"),
                    ("VOICE_CLIBAN", "/opt/old")
                ]),
                |_| true
            ),
            "/opt/new"
        );
        // Then PATH, in order, taking the first entry that is executable.
        assert_eq!(
            super::choose_cliban(env(vec![("PATH", "/empty:/bin:/usr/bin")]), |path| path
                == Path::new("/bin/cliban")
                || path == Path::new("/usr/bin/cliban")),
            "/bin/cliban"
        );
        // A PATH entry that holds the name but cannot be run is not the answer.
        assert_eq!(
            super::choose_cliban(
                env(vec![("PATH", "/notexec:/bin"), ("HOME", "/home/tester")]),
                |path| path == Path::new("/bin/cliban")
            ),
            "/bin/cliban"
        );
        // Nothing on PATH: the cargo directory, which is where it usually is
        // and where the service manager's PATH will not look.
        assert_eq!(
            super::choose_cliban(
                env(vec![("PATH", "/bin"), ("HOME", "/home/tester")]),
                nothing_runs
            ),
            "/home/tester/.cargo/bin/cliban"
        );
        // No PATH and no HOME at all: still a name, so the failure is a stated
        // reason from the spawn rather than an empty argv.
        assert_eq!(
            super::choose_cliban(env(vec![]), nothing_runs),
            "~/.cargo/bin/cliban"
        );
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

    // AYEAYE-62 — the daemon reads the settings file setup writes, underneath
    // the environment. Without the file layer, `ayeaye setup --port 9000` on a
    // machine whose service manager injects nothing — launchd, or no manager at
    // all — writes configuration the daemon then ignores, while `ayeaye check`
    // reads it and probes a port nothing is listening on.
    #[test]
    fn the_daemon_reads_the_file_setup_writes_and_the_environment_still_wins() {
        let silent = |_: &str| None;
        // The file answers when the environment is silent, whichever spelling
        // the line uses — setup's own, a bare name, or the legacy prefix.
        assert_eq!(
            super::layered("PORT", silent, "AYEAYE_PORT=9005\n"),
            Some("9005".to_string())
        );
        assert_eq!(
            super::layered("PORT", silent, "PORT=9006\n"),
            Some("9006".to_string())
        );
        assert_eq!(
            super::layered("BIND", silent, "VOICE_REMOTE_BIND=10.0.0.9\n"),
            Some("10.0.0.9".to_string())
        );
        // The current spelling beats the legacy one wherever it sits in the
        // file, which is the same order the environment is read in.
        assert_eq!(
            super::layered(
                "BIND",
                silent,
                "VOICE_REMOTE_BIND=10.0.0.9\nAYEAYE_BIND=10.0.0.1\n"
            ),
            Some("10.0.0.1".to_string())
        );
        // The environment wins over the file, so a unit that injects the file
        // with EnvironmentFile= and this layer cannot disagree.
        let unit = |key: &str| (key == "AYEAYE_PORT").then(|| "9100".to_string());
        assert_eq!(
            super::layered("PORT", unit, "AYEAYE_PORT=9005\n"),
            Some("9100".to_string())
        );
        // The last line wins, as it would when a shell sourced the file; a key
        // mentioned with nothing after it is not an answer; no file, no layer.
        assert_eq!(
            super::layered("BIND", silent, "AYEAYE_BIND=first\nAYEAYE_BIND=second\n"),
            Some("second".to_string())
        );
        assert_eq!(super::layered("BIND", silent, "AYEAYE_BIND=\n"), None);
        assert_eq!(super::layered("BIND", silent, ""), None);
    }

    #[test]
    fn the_default_port_is_the_production_port() {
        assert_eq!(DEFAULT_PORT, 8911);
        let settings = resolve(&[], no_env).expect("the defaults should resolve");
        assert_eq!(settings.port, DEFAULT_PORT);
        assert_eq!(settings.bind, "127.0.0.1");
        assert_eq!(settings.address(), format!("127.0.0.1:{DEFAULT_PORT}"));
    }

    // AYEAYE-42 — an argument beats the environment, and the environment beats
    // the default; without the middle rung a service unit cannot set the port
    // at all.
    #[test]
    fn an_argument_beats_the_environment_which_beats_the_default() {
        let env = |name: &str| match name {
            "PORT" => Some("9101".to_string()),
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
    // 8911 refuses every request it receives.
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
        let configured = Settings::resolve(
            &[],
            named,
            "s3cret".to_string(),
            Some("box".to_string()),
            super::Cliban::new("/nonexistent/cliban".to_string()),
            silent(),
        )
        .expect("a named machine");
        assert_eq!(configured.peers.here().name().as_str(), "desktop");
        assert!(
            configured.peers.here().is_here(),
            "this machine has no transport"
        );

        let from_uname = Settings::resolve(
            &[],
            no_env,
            "s3cret".to_string(),
            Some("box".to_string()),
            super::Cliban::new("/nonexistent/cliban".to_string()),
            silent(),
        )
        .expect("an unnamed machine still has a nodename");
        assert_eq!(from_uname.peers.here().name().as_str(), "box");

        // And a machine that will not say: a name it can be routed by beats no
        // name at all, since every pane id on it needs one.
        let anonymous = Settings::resolve(
            &[],
            no_env,
            "s3cret".to_string(),
            None,
            super::Cliban::new("/nonexistent/cliban".to_string()),
            silent(),
        )
        .expect("a nameless machine");
        assert_eq!(anonymous.peers.here().name().as_str(), "localhost");
        let blank = Settings::resolve(
            &[],
            no_env,
            "s3cret".to_string(),
            Some("  ".to_string()),
            super::Cliban::new("/nonexistent/cliban".to_string()),
            silent(),
        )
        .expect("a blank nodename is no nodename");
        assert_eq!(blank.peers.here().name().as_str(), "localhost");
    }

    // AYEAYE-58 — the daemon qualifies every pane id it issues with this name,
    // and `ayeaye dictate` compares the ids in its state file against the pane
    // list under the same name. Two spellings of this rule is a toggle that
    // cannot find any pane the daemon named, and the symptom is a dictation that
    // records, transcribes, runs the cleanup model and then throws the words
    // away — so it is one function, and this is the test that says so.
    #[test]
    fn the_daemon_and_the_toggle_name_this_machine_the_same_way() {
        let configured = |name: &str| (name == "NAME").then(|| "desktop".to_string());

        // The configured name first, so a screenshot need not show a hostname.
        assert_eq!(
            super::machine_name(configured, Some("box".to_string())),
            "desktop"
        );
        // Then what the machine calls itself.
        assert_eq!(super::machine_name(no_env, Some("box".to_string())), "box");
        // Then something a pane id can at least be routed by.
        assert_eq!(super::machine_name(no_env, None), super::DEFAULT_NAME);
        assert_eq!(
            super::machine_name(no_env, Some("   ".to_string())),
            super::DEFAULT_NAME
        );

        // And the settings really are built from it, rather than from a second
        // copy of the same three lines.
        let settings = resolve(&[], configured).expect("a named machine");
        assert_eq!(
            settings.peers.here().name().as_str(),
            super::machine_name(configured, Some("box".to_string()))
        );
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
        let refused = Settings::resolve(
            &[],
            slashed,
            "s3cret".to_string(),
            None,
            super::Cliban::new("/nonexistent/cliban".to_string()),
            silent(),
        )
        .unwrap_err();
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
