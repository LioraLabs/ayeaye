//! What a long-running user service is, and what it looks like written down.
//!
//! One description of a service, two ways of writing it down. The facts about a
//! service — what it is called, what it runs, where its output goes — live in a
//! [`Definition`] and nowhere else; [`render_systemd`] and [`render_launchd`]
//! are two spellings of the same facts. A unit and a property list saying
//! slightly different things about the same program is exactly the bug this
//! arrangement cannot have.
//!
//! What a definition does *not* contain is any setting. The port, the address,
//! the allowed hosts live in the settings file and only there: the unit points
//! at it with `EnvironmentFile=`, and the agent — launchd has no such thing —
//! is told where it is instead. Somebody changing the port edits one file, and
//! a definition installed months ago can never disagree with the settings in
//! force.
//!
//! The service definitions and operations live here, and the
//! golden files under `tests/fixtures/units/` are what makes it a port rather
//! than a rewrite: the renderers here are compared against them byte for byte.

/// The user-session service manager a machine has.
///
/// Which one this machine has is not decided here — detection is the shell's,
/// and it arrives as a value. There is deliberately no "neither" variant: every
/// function below would have to answer for it, and the honest place for that
/// answer is above, where a machine with no user service manager is told so
/// rather than handed a command it cannot run. AYEAYE-60 is what will supply
/// it; until then the binary assumes one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Manager {
    /// systemd's *user* session. Never the system manager: this project
    /// installs a per-user service, and a root-owned one would keep running
    /// after the person who wanted it logged out.
    Systemd,
    /// launchd, addressed in the `gui/<uid>` domain.
    Launchd,
}

/// The reverse-DNS prefix a launchd label carries by default.
///
/// It matches the plist this project has always installed,
/// `~/Library/LaunchAgents/dev.ayeaye.plist`. Changing it changes every label
/// generated here.
pub const DEFAULT_LAUNCHD_PREFIX: &str = "dev";

/// What a launchd agent is given as its `PATH`.
///
/// Both Homebrew prefixes — Apple silicon and Intel — and then the four
/// directories launchd would have supplied on its own. A fixed list rather than
/// this machine's `PATH` on purpose: a definition is compared against a golden
/// file, and a copy of whatever happened to be exported the day setup ran is
/// not a description of the machine.
///
/// Without the Homebrew prefixes the agent starts, answers, passes its health
/// check, and shows an empty list of sessions forever, because tmux on a Mac
/// comes from Homebrew and ayeaye reads a tmux it cannot run as a tmux with
/// nothing in it.
pub const DEFAULT_LAUNCHD_PATH: &str =
    "/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin";

/// One service, said once, in terms neither platform owns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Definition {
    /// The logical name: `ayeaye`, not `ayeaye.service` and not `dev.ayeaye`.
    pub name: String,
    /// The one-line description a person reads in `systemctl status`.
    pub title: String,
    /// What has to be up first, under systemd. launchd has no equivalent and
    /// does not want one: an agent is started when the user logs in.
    pub after: Option<String>,
    /// The program and its arguments, absolute. A vector rather than a string
    /// because a path may contain a space, and this is the only representation
    /// both formats can be built from without guessing where an argument ends.
    pub argv: Vec<String>,
}

impl Definition {
    /// ayeaye itself.
    ///
    /// No `After=`: the unit used to order itself after `network.target`, which
    /// is a *system* target a user manager has never heard of, so the line was
    /// an ordering guarantee that did not exist. ayeaye binds when it starts and
    /// the restart policy covers the rest.
    /// **The `serve` is load-bearing.** The shell's `bin/ayeaye` is the server
    /// and running it bare starts one, which is why the captured unit says only
    /// `@REPO@/bin/ayeaye`. This binary is not: run bare it prints its banner and
    /// exits 0. A unit without the verb installs perfectly, starts, prints one
    /// line, exits successfully, and — because the policy is `on-failure` — is
    /// never restarted. Nothing is running and nothing anywhere says so, which
    /// is the failure mode this whole ticket's health checks exist to catch, so
    /// it would be a poor thing to ship.
    pub fn ayeaye(program: &str) -> Self {
        Definition {
            name: "ayeaye".to_string(),
            title: "voice remote for tmux (phone web UI)".to_string(),
            after: None,
            argv: vec![program.to_string(), "serve".to_string()],
        }
    }

    /// The local microphone recorder.
    ///
    /// It runs on the device a person is sitting at rather than on the server,
    /// which is why it is still a program of its own. Setup refreshes this
    /// definition when one is already installed and never installs a new one:
    /// somebody who copied the old hand-edited template still has the file, and
    /// a rerun should leave them with a correct one rather than a stale one
    /// naming a path the repository has since moved from.
    pub fn voice_agent(program: &str) -> Self {
        Definition {
            name: "voice-agent".to_string(),
            title: "voice-dictate mic recorder (local, for M-v when attached locally)".to_string(),
            after: Some("graphical-session.target".to_string()),
            argv: vec![program.to_string()],
        }
    }
}

/// Where this machine keeps the things a definition names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    /// The settings file a unit points at and an agent is told about.
    pub env_file: String,
    /// `XDG_CONFIG_HOME`, handed to a launchd agent so it reads the same
    /// settings the unit's `EnvironmentFile=` would have supplied.
    pub config_home: String,
    /// `XDG_STATE_HOME`, for the same reason.
    pub state_home: String,
    /// Where a systemd user unit is installed.
    pub unit_dir: String,
    /// Where a launchd agent is installed.
    pub agent_dir: String,
    /// Where an agent's output goes. launchd has no journal, so the agent names
    /// the file it writes.
    pub log_dir: String,
    /// The `PATH` a launchd agent is started with.
    pub launchd_path: String,
}

impl Layout {
    /// The documented defaults, derived from the three paths the shell knows.
    ///
    /// Every one of them is an argument rather than a lookup: reading `HOME` is
    /// the shell's job, and a pure function that consults the environment is not
    /// one.
    pub fn new(home: &str, config_home: &str, state_home: &str) -> Self {
        Layout {
            env_file: format!("{config_home}/ayeaye/env"),
            config_home: config_home.to_string(),
            state_home: state_home.to_string(),
            unit_dir: format!("{config_home}/systemd/user"),
            agent_dir: format!("{home}/Library/LaunchAgents"),
            log_dir: format!("{home}/Library/Logs/ayeaye"),
            launchd_path: DEFAULT_LAUNCHD_PATH.to_string(),
        }
    }
}

// ---------------------------------------------------------------- the session
//
// What the service manager is asked to do, and how it is addressed. Commands
// come back as argv rather than as a string, so nothing here needs a shell and
// nothing needs quoting: a plist path with a space in it is one element of a
// vector rather than a quoting rule somebody has to get right.

/// Something a service manager can be asked to do.
///
/// `start` and `stop` are deliberately the "leave enablement alone" pair on
/// both platforms, so that stop-then-start behaves the same either side. Under
/// launchd that makes `stop` a signal rather than a bootout — bootout is the
/// unregistering one, and it is what `disable` does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    /// Re-read the definition files. systemd only.
    Reload,
    /// Install and start, and keep it that way across logins. Under launchd
    /// this is bootstrap, which needs the path of the plist.
    Enable,
    /// The inverse of enable: stop it, and do not bring it back.
    Disable,
    /// Start it now, leaving enablement alone.
    Start,
    /// Stop it now, leaving enablement alone.
    Stop,
    /// What is it doing.
    Status,
}

/// Why there is no command to give.
///
/// The two are worth telling apart. Reporting a caller's mistake to a user as
/// "your platform is unsupported" would send them looking in the wrong place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unavailable {
    /// A fact about this machine: an operation that does not exist here, or
    /// something needed to address it that is not known.
    Machine(&'static str),
    /// The call was wrong — a bug above this layer.
    Caller(&'static str),
}

impl core::fmt::Display for Unavailable {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Unavailable::Machine(why) | Unavailable::Caller(why) => f.write_str(why),
        }
    }
}

impl core::error::Error for Unavailable {}

/// Who the service manager is, and how it is addressed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    /// The service manager this machine's user session has.
    pub manager: Manager,
    /// The uid that addresses a launchd domain. systemd never needs one; on a
    /// Mac where `id` could not be run this is `None`, and every launchd
    /// command refuses rather than addressing the malformed `gui//label`.
    pub uid: Option<String>,
    /// The reverse-DNS prefix for a launchd label.
    pub launchd_prefix: String,
}

impl Session {
    /// A systemd user session.
    pub fn systemd() -> Self {
        Session {
            manager: Manager::Systemd,
            uid: None,
            launchd_prefix: DEFAULT_LAUNCHD_PREFIX.to_string(),
        }
    }

    /// A launchd session in the `gui/<uid>` domain.
    pub fn launchd(uid: &str) -> Self {
        Session {
            manager: Manager::Launchd,
            uid: Some(uid.to_string()),
            launchd_prefix: DEFAULT_LAUNCHD_PREFIX.to_string(),
        }
    }

    /// The session for the manager [`crate::machine`] detected, if there is one.
    ///
    /// This is where the detector and the verbs meet, and `None` is the third
    /// answer [`Manager`] refuses to carry: a machine with neither manager. It
    /// is not a failure — a container and a stripped-down Linux both really are
    /// one, and starting ayeaye by hand remains a
    /// supported way to use it — so the caller is expected to reach for
    /// [`manual_instructions`] rather than to give up.
    ///
    /// The uid is only ever launchd's. systemd's user manager is addressed as
    /// whoever is calling, so handing it one would be describing a thing that
    /// does not exist.
    pub fn for_manager(manager: crate::machine::ServiceManager, uid: Option<&str>) -> Option<Self> {
        match manager {
            crate::machine::ServiceManager::None => None,
            crate::machine::ServiceManager::Systemd => Some(Session::systemd()),
            crate::machine::ServiceManager::Launchd => Some(Session {
                manager: Manager::Launchd,
                uid: uid.map(str::to_string),
                launchd_prefix: DEFAULT_LAUNCHD_PREFIX.to_string(),
            }),
        }
    }

    /// What this platform calls that service.
    ///
    /// A name is reduced to the bare logical one first and then dressed in the
    /// local convention, so that one name works everywhere: `ayeaye`,
    /// `ayeaye.service` and `dev.ayeaye` all come out as `ayeaye.service` under
    /// systemd and `dev.ayeaye` under launchd. Shared code holds one name; it
    /// must not matter which one.
    pub fn unit_name(&self, name: &str) -> Result<String, Unavailable> {
        if name.is_empty() {
            return Err(Unavailable::Caller("a service with no name"));
        }
        // A name is a name, not a path. `definition_path` joins it onto a
        // directory, so a separator in it is a definition written outside the
        // one place this program installs into.
        if name.contains('/') || name.contains(std::path::MAIN_SEPARATOR) {
            return Err(Unavailable::Caller(
                "a service name may not contain a path separator",
            ));
        }
        Ok(match self.manager {
            Manager::Systemd => {
                let bare = undress(name, &self.launchd_prefix);
                if bare.ends_with(".socket")
                    || bare.ends_with(".timer")
                    || bare.ends_with(".target")
                {
                    bare.to_string()
                } else {
                    format!("{bare}.service")
                }
            }
            Manager::Launchd => self.label(name),
        })
    }

    /// The launchd label for a service.
    ///
    /// The plist's `Label`, its filename and every `gui/<uid>/…` target are all
    /// this one string. They were two strings once, and a definition labelled
    /// one thing living in a file named another is an agent that bootstraps a
    /// target which does not exist.
    pub fn label(&self, name: &str) -> String {
        format!(
            "{}.{}",
            self.launchd_prefix,
            undress(name, &self.launchd_prefix)
        )
    }

    /// The definition, written down the way this platform writes one.
    ///
    /// A method rather than two free functions because the launchd agent
    /// carries its own label: rendering an agent without the session that names
    /// it is how the label and the filename came apart.
    pub fn render(&self, definition: &Definition, layout: &Layout) -> Result<String, Unavailable> {
        // A definition whose program cannot be named is not a definition.
        // Asked before a byte is written, because a unit with an empty
        // `ExecStart` installs perfectly and starts nothing.
        if definition.argv.is_empty() {
            return Err(Unavailable::Caller("a definition with no program to run"));
        }
        Ok(match self.manager {
            Manager::Systemd => render_systemd(definition, layout),
            Manager::Launchd => render_launchd(definition, layout, &self.label(&definition.name)),
        })
    }

    /// Where this platform keeps that service's definition.
    pub fn definition_path(&self, name: &str, layout: &Layout) -> Result<String, Unavailable> {
        let unit = self.unit_name(name)?;
        Ok(match self.manager {
            Manager::Systemd => format!("{}/{unit}", layout.unit_dir),
            Manager::Launchd => format!("{}/{unit}.plist", layout.agent_dir),
        })
    }

    /// The command for an operation, as argv, executing nothing.
    ///
    /// `unit_path` is only ever read by launchd's `enable`, which is a
    /// bootstrap and needs the plist itself. Guessing one would be worse than
    /// refusing.
    pub fn command(
        &self,
        name: &str,
        operation: Operation,
        unit_path: Option<&str>,
    ) -> Result<Vec<String>, Unavailable> {
        let unit = self.unit_name(name)?;
        match self.manager {
            Manager::Systemd => Ok(systemd_command(operation, &unit)),
            Manager::Launchd => {
                // Every launchd command addresses a domain by uid. Without one
                // the target would be the malformed `gui//label`, which is
                // worse than saying this cannot be done.
                let uid = self.uid.as_deref().ok_or(Unavailable::Machine(
                    "no uid, so no launchd domain to address",
                ))?;
                launchd_command(operation, &unit, uid, unit_path)
            }
        }
    }
}

fn systemd_command(operation: Operation, unit: &str) -> Vec<String> {
    let words: Vec<&str> = match operation {
        Operation::Reload => vec!["daemon-reload"],
        Operation::Enable => vec!["enable", "--now", unit],
        Operation::Disable => vec!["disable", "--now", unit],
        Operation::Start => vec!["start", unit],
        Operation::Stop => vec!["stop", unit],
        Operation::Status => vec!["status", unit],
    };
    let mut argv = vec!["systemctl".to_string(), "--user".to_string()];
    argv.extend(words.into_iter().map(str::to_string));
    argv
}

fn launchd_command(
    operation: Operation,
    unit: &str,
    uid: &str,
    unit_path: Option<&str>,
) -> Result<Vec<String>, Unavailable> {
    let target = format!("gui/{uid}/{unit}");
    let words: Vec<String> = match operation {
        // launchd has no daemon-reload: a changed plist is re-read by being
        // bootstrapped again, which is the enable step. Saying so beats
        // inventing a command that does nothing.
        Operation::Reload => {
            return Err(Unavailable::Machine(
                "launchd re-reads a plist by being bootstrapped again, not by reloading",
            ));
        }
        Operation::Enable => {
            let path = unit_path.ok_or(Unavailable::Machine(
                "bootstrapping needs the path of the plist, and guessing one would be worse",
            ))?;
            vec![
                "bootstrap".to_string(),
                format!("gui/{uid}"),
                path.to_string(),
            ]
        }
        Operation::Disable => vec!["bootout".to_string(), target],
        Operation::Start => vec!["kickstart".to_string(), target],
        // A signal, not a bootout: stop must leave the service registered, the
        // way `systemctl --user stop` does, or stop-then-start works on Linux
        // and fails on a Mac with "Could not find service".
        Operation::Stop => vec!["kill".to_string(), "SIGTERM".to_string(), target],
        Operation::Status => vec!["print".to_string(), target],
    };
    let mut argv = vec!["launchctl".to_string()];
    argv.extend(words);
    Ok(argv)
}

// ------------------------------------------------------------- what to write

/// What installing a rendered definition would do to the disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Install {
    /// The definition on disk is already the generated one.
    Unchanged,
    /// There is nothing there yet.
    Create,
    /// Something is there and it differs. It is a generated file and this run
    /// is going to replace it, but it may be an older version's or a person's
    /// — which is why the layer that acts on this keeps a copy first.
    Replace,
}

/// What to do about a definition, given what is on disk and what was rendered.
///
/// Leaving an unchanged file alone is not an optimisation: it is the difference
/// between repairing a service and disturbing one. No new timestamp, no backup
/// of a file identical to its replacement, and a directory somebody has made
/// read-only that already holds the right definition is not a failure.
pub fn plan_install(existing: Option<&str>, rendered: &str) -> Install {
    match existing {
        None => Install::Create,
        Some(text) if text == rendered => Install::Unchanged,
        Some(_) => Install::Replace,
    }
}

// ------------------------------------------------------------------ quoting
//
// Each format mangles a path in its own way, and both of them are silent about
// it: a unit with a split `ExecStart` installs cleanly and never starts, and a
// plist with a bare ampersand in it is not XML at all.

/// The bytes systemd treats as themselves inside an `ExecStart` word.
fn is_bare(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '/' | ':' | '=' | '@' | '+' | ',' | '-')
}

/// One `ExecStart` argument, as systemd parses it.
///
/// Left bare when it is made only of characters systemd treats as themselves,
/// which keeps the ordinary case readable. Otherwise double-quoted, and inside
/// those quotes a backslash, a double quote, a dollar (systemd expands `$NAME`
/// there) and a percent (systemd's own `%h`-style specifiers) all have to be
/// doubled or escaped to arrive at `execve` intact.
fn systemd_arg(arg: &str) -> String {
    if arg.is_empty() {
        return "\"\"".to_string();
    }
    if arg.chars().all(is_bare) {
        return arg.to_string();
    }
    let mut out = String::with_capacity(arg.len() + 2);
    out.push('"');
    for c in arg.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '$' => out.push_str("$$"),
            '%' => out.push_str("%%"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A directive value that is not a command.
///
/// systemd runs specifier expansion over `EnvironmentFile=` just as it does
/// over `ExecStart=`, and an unrecognised specifier is not an error: the whole
/// directive is dropped with a warning nobody reads. A settings file under a
/// path containing a percent sign would leave a unit that starts perfectly and
/// runs with none of the user's settings.
fn systemd_value(text: &str) -> String {
    text.replace('%', "%%")
}

/// Text safe to put between two XML tags.
///
/// The ampersand first, or the entities the other two introduce would be
/// escaped a second time.
fn xml_text(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// What to tell somebody whose machine has nowhere to install a service.
///
/// The port of install.sh's `_manual_instructions` and the line
/// `_service_manual_fallback` adds after it. Three sentences, and each one
/// answers a question the person is about to have: what do I run, where do its
/// settings come from, and where does its output go.
///
/// Deliberately not phrased as an apology. `tests/cases/service_fallback_test.sh`
/// is explicit that a machine with no user service manager is a machine where
/// setup has nothing left to do, and that telling somebody to come back to it
/// later would be telling them to fix something that is not broken.
/// The `serve` is the same load-bearing word as in [`Definition::ayeaye`], and
/// for the same reason. `install.sh` says `run the server with: $REPO/bin/ayeaye`
/// because that script *is* the server; telling somebody to run this binary bare
/// would have them watch it print one line and exit, on the one path where there
/// is no service manager to notice.
pub fn manual_instructions(program: &str, env_file: &str) -> [String; 3] {
    [
        format!("run the server with: {program} serve"),
        format!("(it reads {env_file} by itself)"),
        "its log is whatever the terminal you start it in shows.".to_string(),
    ]
}

/// Whether systemd can name every one of these paths in a unit, and which one
/// it cannot.
///
/// There is no escaping for these. systemd unquotes the executable word and
/// then rejects the result if it contains a quote, a backslash or a control
/// character — "Executable path contains special characters", a fatal unit
/// error — so the only honest answers are to say which path it is or to write a
/// unit that can never start.
pub fn unrepresentable_in_systemd<'a>(paths: &[&'a str]) -> Option<&'a str> {
    paths
        .iter()
        .find(|path| path.contains(['"', '\'', '\\', '\n']))
        .copied()
}

// ---------------------------------------------------------------- renderers

/// A systemd user unit, whole. Reached through [`Session::render`].
///
/// Written as one template rather than assembled, so that what a reader sees
/// here is the shape of the file that lands on somebody's disk.
fn render_systemd(definition: &Definition, layout: &Layout) -> String {
    let after = match definition.after.as_deref().filter(|a| !a.is_empty()) {
        Some(after) => format!("After={after}\n"),
        None => String::new(),
    };
    let program = definition
        .argv
        .iter()
        .map(|arg| systemd_arg(arg))
        .collect::<Vec<String>>()
        .join(" ");
    let title = &definition.title;
    let name = &definition.name;
    let env_file = systemd_value(&layout.env_file);
    format!(
        r#"[Unit]
Description={title}
{after}
[Service]
Type=simple
# Generated by setup, and replaced whole every time it runs. Nothing is
# configured here: every setting lives in the environment file below. Edit
# that, then `systemctl --user restart {name}`.
#
# The leading "-" means a settings file that is not there yet is not a reason
# to refuse to start: ayeaye has a default for everything in it, and a restart
# loop would be a worse answer than running on the defaults.
EnvironmentFile=-{env_file}
ExecStart={program}
Restart=on-failure
RestartSec=5

[Install]
WantedBy=default.target
"#
    )
}

/// A launchd property list, whole. Reached through [`Session::render`].
///
/// The differences from the unit above are forced by the platform and are not
/// choices. launchd has no `EnvironmentFile`, so the agent is handed the
/// *location* of the settings rather than any value out of them. It has no
/// journal, so the agent names the file it writes. And it starts an agent with
/// almost no `PATH`, so one is supplied.
///
/// The label is an argument rather than something worked out here: it is the
/// same string as the agent's filename and as every `gui/<uid>/…` target, and
/// [`Session`] is where that string lives.
fn render_launchd(definition: &Definition, layout: &Layout, label: &str) -> String {
    let log = xml_text(&format!("{}/{}.log", layout.log_dir, definition.name));
    let label = xml_text(label);
    let path = xml_text(&layout.launchd_path);
    let config_home = xml_text(&layout.config_home);
    let state_home = xml_text(&layout.state_home);
    let program = definition
        .argv
        .iter()
        .map(|arg| format!("    <string>{}</string>\n", xml_text(arg)))
        .collect::<String>();
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <!-- Generated by setup, and replaced whole every time it runs. Nothing is
       configured here: every setting lives in the environment file this
       agent reads. Edit that, then restart the agent. -->
  <key>Label</key>
  <string>{label}</string>
  <key>ProgramArguments</key>
  <array>
{program}  </array>
  <key>EnvironmentVariables</key>
  <dict>
    <key>PATH</key>
    <string>{path}</string>
    <key>XDG_CONFIG_HOME</key>
    <string>{config_home}</string>
    <key>XDG_STATE_HOME</key>
    <string>{state_home}</string>
  </dict>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <dict>
    <key>SuccessfulExit</key>
    <false/>
  </dict>
  <key>ThrottleInterval</key>
  <integer>5</integer>
  <key>StandardOutPath</key>
  <string>{log}</string>
  <key>StandardErrorPath</key>
  <string>{log}</string>
</dict>
</plist>
"#
    )
}

/// A name with whichever platform's clothes it arrived in taken off.
///
/// `ayeaye`, `ayeaye.service` and `dev.ayeaye` are one service. Shared code
/// holds one name; it must not matter which one.
fn undress<'a>(name: &'a str, launchd_prefix: &str) -> &'a str {
    let bare = name
        .strip_prefix(&format!("{launchd_prefix}."))
        .unwrap_or(name);
    bare.strip_suffix(".service").unwrap_or(bare)
}

#[cfg(test)]
mod tests {
    use super::{
        DEFAULT_LAUNCHD_PREFIX, Definition, Install, Layout, Manager, Operation, Session,
        Unavailable, manual_instructions, plan_install, systemd_arg, unrepresentable_in_systemd,
    };
    use crate::machine::ServiceManager;

    // AYEAYE-62 — the third answer AYEAYE-61 recorded as owed. A machine with
    // neither manager is not a broken machine: a container and a stripped-down
    // Linux both really are one, and the shell treats running ayeaye by hand as
    // a supported way to use it. It stays out of `Manager` deliberately — every
    // function taking a `Manager` would otherwise have to invent a reply — so
    // the answer is the absence of a session.
    #[test]
    fn a_machine_with_no_service_manager_has_no_session_to_address() {
        assert_eq!(Session::for_manager(ServiceManager::None, None), None);
        assert_eq!(
            Session::for_manager(ServiceManager::None, Some("501")),
            None,
            "a uid does not conjure a manager to address with it"
        );
    }

    // AYEAYE-62 — and where there is one, it is the one the detector named,
    // addressed the way that platform is addressed.
    #[test]
    fn the_detected_manager_is_the_one_the_session_addresses() {
        let systemd = Session::for_manager(ServiceManager::Systemd, Some("1000"))
            .expect("systemd is a manager");
        assert_eq!(systemd.manager, Manager::Systemd);
        assert_eq!(
            systemd.uid, None,
            "systemd --user addresses the caller's own session and never a uid"
        );

        let launchd =
            Session::for_manager(ServiceManager::Launchd, Some("501")).expect("launchd is one too");
        assert_eq!(launchd.manager, Manager::Launchd);
        assert_eq!(launchd.uid.as_deref(), Some("501"));
        assert_eq!(launchd.launchd_prefix, DEFAULT_LAUNCHD_PREFIX);

        // A Mac whose `id` would not answer. The session exists — launchctl is
        // there — and every command through it refuses rather than addressing
        // the malformed `gui//label`, which is the pre-existing behaviour.
        let unaddressable =
            Session::for_manager(ServiceManager::Launchd, None).expect("launchctl is still here");
        assert_eq!(unaddressable.uid, None);
        assert_eq!(
            unaddressable.command("ayeaye", Operation::Start, None),
            Err(Unavailable::Machine(
                "no uid, so no launchd domain to address"
            ))
        );
    }

    // AYEAYE-62 — the port of install.sh's `_manual_instructions` and the third
    // line `_service_manual_fallback` adds, pinned to the sentences
    // tests/cases/service_fallback_test.sh asserts on. Not an apology and not a
    // failure: it is the whole of the right answer on a machine with nothing to
    // install into.
    #[test]
    fn a_machine_with_no_manager_is_told_how_to_run_it() {
        let said = manual_instructions("/opt/ayeaye/bin/ayeaye", "/home/tester/.config/ayeaye/env");
        assert_eq!(
            said,
            [
                "run the server with: /opt/ayeaye/bin/ayeaye serve",
                "(it reads /home/tester/.config/ayeaye/env by itself)",
                "its log is whatever the terminal you start it in shows.",
            ]
        );
    }

    /// The unit this session would install for that definition.
    ///
    /// Every renderer test goes through [`Session::render`], because that is
    /// the only door: an agent cannot be rendered without the session that
    /// names it.
    fn render(session: &Session, definition: &Definition, layout: &Layout) -> String {
        session.render(definition, layout).expect("a definition")
    }

    fn render_systemd(definition: &Definition, layout: &Layout) -> String {
        render(&Session::systemd(), definition, layout)
    }

    fn render_launchd(definition: &Definition, layout: &Layout) -> String {
        render(&Session::launchd("501"), definition, layout)
    }

    /// The golden files the shell suite already compares against, brought in
    /// through the compiler because a pure crate may not open a file.
    const AYEAYE_SYSTEMD: &str = include_str!("../../../tests/fixtures/units/ayeaye-systemd");
    const AYEAYE_LAUNCHD: &str = include_str!("../../../tests/fixtures/units/ayeaye-launchd");
    const VOICE_AGENT_SYSTEMD: &str =
        include_str!("../../../tests/fixtures/units/voice-agent-systemd");
    const VOICE_AGENT_LAUNCHD: &str =
        include_str!("../../../tests/fixtures/units/voice-agent-launchd");
    const WHISPER_SYSTEMD: &str = include_str!("../../../tests/fixtures/units/whisper-systemd");
    const WHISPER_LAUNCHD: &str = include_str!("../../../tests/fixtures/units/whisper-launchd");

    const HOME: &str = "/home/tester";
    const CONFIG_HOME: &str = "/home/tester/.config";
    const STATE_HOME: &str = "/home/tester/.local/state";
    const REPO: &str = "/srv/ayeaye";
    const WHISPER_BIN: &str = "/opt/whisper/bin/whisper-server";

    fn layout() -> Layout {
        Layout::new(HOME, CONFIG_HOME, STATE_HOME)
    }

    /// A golden file with its markers filled in, exactly as
    /// `tests/cases/service_units_test.sh` fills them.
    fn golden(text: &str) -> String {
        let layout = layout();
        text.replace("@REPO@", REPO)
            .replace("@ENV@", &layout.env_file)
            .replace("@CONF@", CONFIG_HOME)
            .replace("@STATE@", STATE_HOME)
            .replace("@LOGS@", &layout.log_dir)
            .replace("@WHISPER@", WHISPER_BIN)
    }

    /// The `<string>` elements of a plist's `ProgramArguments`, unescaped.
    ///
    /// It is how a rendered agent is read back — the analogue of the shell
    /// suite handing its plist to `plistlib` — and it is also where the whisper
    /// corpus gets its input. The program that service runs is a page of shell
    /// carrying quotes, dollars, percent signs, tabs and angle brackets all at
    /// once; transcribing it into this file is the one thing nobody could
    /// review. Reading it back out of the golden makes the round trip itself an
    /// assertion, and the *systemd* golden — which it did not come from — is
    /// what stops an escaper and a faulty inverse cancelling each other out.
    fn program_arguments(plist: &str) -> Vec<String> {
        let array = plist
            .split_once("<key>ProgramArguments</key>")
            .expect("the agent should declare its program")
            .1;
        let body = array
            .split_once("<array>")
            .expect("ProgramArguments should be an array")
            .1
            .split_once("</array>")
            .expect("the array should be closed")
            .0;
        body.split("<string>")
            .skip(1)
            .map(|chunk| {
                let text = chunk
                    .split_once("</string>")
                    .expect("every string should be closed")
                    .0;
                text.replace("&lt;", "<")
                    .replace("&gt;", ">")
                    .replace("&amp;", "&")
            })
            .collect()
    }

    /// whisper.cpp's server, as the shell installs it today.
    ///
    /// Not a [`Definition`] constructor, because this milestone deletes the
    /// service: transcription moves in process. It is here as the escaping
    /// corpus and nothing else — the only captured input carrying `$`, `%`,
    /// `"`, `\`, a tab, `<`, `>` and `&` at the same time.
    fn whisper() -> Definition {
        Definition {
            name: "whisper-server".to_string(),
            title: "whisper.cpp server, kept resident for dictation".to_string(),
            after: None,
            argv: program_arguments(&golden(WHISPER_LAUNCHD)),
        }
    }

    /// The definition the *shell* installs, which is what the golden files
    /// captured: its `bin/ayeaye` is the server, so its `ExecStart` is the
    /// program and nothing else.
    ///
    /// Written out here rather than taken from [`Definition::ayeaye`] because
    /// AYEAYE-62 gave the real one a `serve`, without which the unit starts a
    /// process that prints a banner and exits. The goldens pin the *renderer*,
    /// and the test below pins the difference between the two on purpose.
    fn ayeaye() -> Definition {
        Definition {
            name: "ayeaye".to_string(),
            title: "voice remote for tmux (phone web UI)".to_string(),
            after: None,
            argv: vec![format!("{REPO}/bin/ayeaye")],
        }
    }

    // AYEAYE-62 — the one place the port deliberately differs from the file it
    // was captured from, so that the difference is a decision rather than a
    // drift. The shell's `bin/ayeaye` serves when run bare; this binary prints
    // its banner and exits 0, and `Restart=on-failure` never restarts a process
    // that exited successfully. A unit without the verb is a service that
    // installs cleanly, runs once, stops, and says nothing.
    #[test]
    fn the_installed_unit_tells_this_binary_to_serve() {
        let real = Definition::ayeaye("/opt/ayeaye");
        assert_eq!(real.argv, ["/opt/ayeaye", "serve"]);
        let unit = render_systemd(&real, &layout());
        assert!(
            unit.contains("ExecStart=/opt/ayeaye serve"),
            "a unit that starts nothing: {unit}"
        );
        // And it differs from the shell's unit in that line and nothing else.
        let shell = render_systemd(&ayeaye(), &layout());
        let differing: Vec<(&str, &str)> = shell
            .lines()
            .zip(unit.lines())
            .filter(|(before, after)| before != after)
            .collect();
        assert_eq!(differing.len(), 1, "{differing:?}");
        assert!(differing[0].0.starts_with("ExecStart="));
    }

    // AYEAYE-61 — the port is only a port if it agrees with the file the shell
    // installs today, to the byte and including the trailing newline.
    #[test]
    fn the_ayeaye_unit_is_what_the_golden_file_says() {
        assert_eq!(render_systemd(&ayeaye(), &layout()), golden(AYEAYE_SYSTEMD));
    }

    // AYEAYE-61
    #[test]
    fn the_ayeaye_agent_is_what_the_golden_file_says() {
        assert_eq!(render_launchd(&ayeaye(), &layout()), golden(AYEAYE_LAUNCHD));
    }

    // AYEAYE-61 — the one definition with an `After=`, which is why it is in
    // the corpus: without it nothing would prove the line is rendered at all.
    #[test]
    fn the_voice_agent_unit_is_what_the_golden_file_says() {
        let definition = Definition::voice_agent(&format!("{REPO}/bin/voice-agent"));
        assert_eq!(
            render_systemd(&definition, &layout()),
            golden(VOICE_AGENT_SYSTEMD)
        );
    }

    // AYEAYE-61 — and the case that proves the log file is named per service
    // rather than after ayeaye.
    #[test]
    fn the_voice_agent_agent_is_what_the_golden_file_says() {
        let definition = Definition::voice_agent(&format!("{REPO}/bin/voice-agent"));
        assert_eq!(
            render_launchd(&definition, &layout()),
            golden(VOICE_AGENT_LAUNCHD)
        );
    }

    // AYEAYE-61 — the escaping torture case: `$`, `%`, `"`, `\` and a tab, all
    // in one `ExecStart` word.
    #[test]
    fn the_whisper_unit_is_what_the_golden_file_says() {
        assert_eq!(
            render_systemd(&whisper(), &layout()),
            golden(WHISPER_SYSTEMD)
        );
    }

    // AYEAYE-61 — and the same word through the other escaper, where `<`, `>`
    // and `&` are what would otherwise stop the file being XML at all.
    #[test]
    fn the_whisper_agent_is_what_the_golden_file_says() {
        assert_eq!(
            render_launchd(&whisper(), &layout()),
            golden(WHISPER_LAUNCHD)
        );
    }

    // AYEAYE-61 — the corpus has to be a corpus. An `include_str!` that
    // resolved to an empty file, or an extraction that found no arguments,
    // would leave every golden above comparing nothing against nothing.
    #[test]
    fn the_corpus_really_holds_the_captured_units() {
        for golden_file in [
            AYEAYE_SYSTEMD,
            AYEAYE_LAUNCHD,
            VOICE_AGENT_SYSTEMD,
            VOICE_AGENT_LAUNCHD,
            WHISPER_SYSTEMD,
            WHISPER_LAUNCHD,
        ] {
            assert!(
                golden_file.len() > 200,
                "a golden file is suspiciously short"
            );
        }
        assert_eq!(
            whisper().argv.len(),
            6,
            "the whisper program is a shell, its script, and four arguments"
        );
        assert!(
            whisper().argv[2].contains("VOICE_WHISPER_MODEL"),
            "the extracted script is not the program the service runs"
        );
    }

    // ------------------------------------------------------------ properties
    //
    // Named properties beside the goldens, deliberately: a golden diff must
    // never be the only thing that fails, or a fixture updated without being
    // read would sail through.

    // AYEAYE-61 — a unit whose `ExecStart` was split on a space installs
    // cleanly and never starts, and macOS paths routinely have spaces in them.
    #[test]
    fn both_formats_run_one_absolute_path_however_it_is_spelled() {
        let unit = render_systemd(&ayeaye(), &layout());
        let agent = render_launchd(&ayeaye(), &layout());
        assert!(unit.contains(&format!("ExecStart={REPO}/bin/ayeaye\n")));
        assert!(agent.contains(&format!("<string>{REPO}/bin/ayeaye</string>")));

        let spaced = Definition::ayeaye("/Users/John Smith/ayeaye/bin/ayeaye");
        assert!(
            render_systemd(&spaced, &layout())
                .contains("ExecStart=\"/Users/John Smith/ayeaye/bin/ayeaye\" serve\n"),
            "systemd splits an unquoted argument on the space"
        );
        assert!(
            render_launchd(&spaced, &layout())
                .contains("<string>/Users/John Smith/ayeaye/bin/ayeaye</string>"),
            "an XML string element needs no quoting for a space"
        );
    }

    // AYEAYE-61 — systemd expands `$NAME` and its own `%h`-style specifiers
    // inside `ExecStart`, so both have to be doubled to arrive at `execve`
    // intact. Neither is special in XML.
    #[test]
    fn a_dollar_or_a_percent_in_the_path_survives_both_formats() {
        let awkward = Definition::ayeaye("/srv/100% $path/bin/ayeaye");
        assert!(
            render_systemd(&awkward, &layout())
                .contains("ExecStart=\"/srv/100%% $$path/bin/ayeaye\" serve\n"),
        );
        assert!(
            render_launchd(&awkward, &layout())
                .contains("<string>/srv/100% $path/bin/ayeaye</string>"),
        );
    }

    // AYEAYE-61 — a bare ampersand is not XML, and a plist that is not XML is
    // an agent that never loads.
    #[test]
    fn xml_special_characters_are_escaped_and_come_back_out_unchanged() {
        let path = "/srv/rock & roll <b> \"quoted\"/bin/ayeaye";
        let agent = render_launchd(&Definition::ayeaye(path), &layout());
        assert!(agent.contains("rock &amp; roll &lt;b&gt;"));
        assert!(
            !agent.contains("rock & roll"),
            "a bare ampersand is not XML"
        );
        assert_eq!(
            program_arguments(&agent),
            vec![path.to_string(), "serve".to_string()],
            "the path has to come back out of the XML exactly as it went in"
        );
    }

    // AYEAYE-61 — there is no escaping for these, so the only honest answers
    // are to name the path or to write a unit that can never start.
    #[test]
    fn a_quote_a_backslash_or_a_newline_is_something_systemd_cannot_express() {
        assert_eq!(unrepresentable_in_systemd(&["/home/me/repo"]), None);
        assert_eq!(
            unrepresentable_in_systemd(&["/home/o\"d/repo"]),
            Some("/home/o\"d/repo"),
            "a double quote"
        );
        assert_eq!(
            unrepresentable_in_systemd(&["/home/o'd/repo"]),
            Some("/home/o'd/repo"),
            "a single quote"
        );
        assert_eq!(
            unrepresentable_in_systemd(&["/home/o\\d/repo"]),
            Some("/home/o\\d/repo"),
            "a backslash"
        );
        assert_eq!(
            unrepresentable_in_systemd(&["/home/ok", "/home/o\nd"]),
            Some("/home/o\nd"),
            "a newline, in any of the paths given, and it says which"
        );
    }

    // AYEAYE-61 — the whole of the wiring, in one line, and a settings file
    // that is not there yet must not become a restart loop.
    #[test]
    fn the_settings_file_is_referenced_exactly_once_and_may_be_missing() {
        let unit = render_systemd(&ayeaye(), &layout());
        let references: Vec<&str> = unit
            .lines()
            .filter(|line| line.starts_with("EnvironmentFile="))
            .collect();
        assert_eq!(
            references,
            vec![format!("EnvironmentFile=-{CONFIG_HOME}/ayeaye/env")],
            "one reference, and the leading `-` is what keeps a missing file \
             from becoming a restart loop under Restart=on-failure"
        );
    }

    // AYEAYE-61 — systemd runs specifier expansion over `EnvironmentFile=` as
    // well, and an unknown specifier silently drops the whole directive: the
    // unit would start perfectly and run with none of the user's settings.
    #[test]
    fn a_percent_sign_in_the_settings_path_is_escaped_for_systemd() {
        let mut layout = layout();
        layout.env_file = "/home/tester/100%/ayeaye/env".to_string();
        let unit = render_systemd(&ayeaye(), &layout);
        assert!(unit.contains("EnvironmentFile=-/home/tester/100%%/ayeaye/env\n"));
        assert!(!unit.contains("EnvironmentFile=-/home/tester/100%/ayeaye/env\n"));
    }

    // AYEAYE-61 — the same promise on both platforms, spelled two ways.
    #[test]
    fn both_formats_carry_a_restart_policy_and_start_at_login() {
        let unit = render_systemd(&ayeaye(), &layout());
        let agent = render_launchd(&ayeaye(), &layout());
        assert!(unit.contains("Restart=on-failure\n"));
        assert!(unit.contains("RestartSec=5\n"));
        assert!(unit.contains("WantedBy=default.target\n"));
        // launchd spells the same intention as "bring it back unless it exited
        // cleanly", plus a floor on how fast it may be restarted.
        assert!(agent.contains("<key>KeepAlive</key>"));
        assert!(agent.contains("<key>SuccessfulExit</key>\n    <false/>"));
        assert!(agent.contains("<key>ThrottleInterval</key>\n  <integer>5</integer>"));
        assert!(agent.contains("<key>RunAtLoad</key>\n  <true/>"));
    }

    // AYEAYE-61 — launchd has no EnvironmentFile, so the agent is told where
    // the settings are rather than what they say. That is the same wiring by
    // another name, and it is why no value can go stale in a definition.
    #[test]
    fn the_agent_is_told_where_the_settings_are_not_what_they_say() {
        let agent = render_launchd(&ayeaye(), &layout());
        assert!(agent.contains(&format!(
            "<key>XDG_CONFIG_HOME</key>\n    <string>{CONFIG_HOME}</string>"
        )));
        assert!(agent.contains(&format!(
            "<key>XDG_STATE_HOME</key>\n    <string>{STATE_HOME}</string>"
        )));
        // Nothing else could leak: a Definition and a Layout carry paths and a
        // title, and there is no way for a setting's *value* to reach either.
        // The shell had to assert this because its renderers could read the
        // settings file; these cannot read anything at all.
        assert!(!agent.contains(&layout().env_file));
    }

    // AYEAYE-61 — launchd hands a GUI agent `/usr/bin:/bin:/usr/sbin:/sbin` and
    // nothing else. tmux on a Mac comes from Homebrew, under one prefix on
    // Apple silicon and another on Intel, and neither is on that list.
    #[test]
    fn the_agent_can_find_tmux_where_a_mac_actually_keeps_it() {
        let agent = render_launchd(&ayeaye(), &layout());
        assert!(agent.contains("<key>PATH</key>"));
        assert!(agent.contains("/opt/homebrew/bin"));
        assert!(agent.contains("/usr/local/bin"));
    }

    // --------------------------------------------------------- names and ops
    //
    // The expected strings below are the ones
    // `tests/cases/platform_service_test.sh` already asserts, so the port can
    // be read line against line. argv is joined with a space only to compare
    // against the shell's spelling; nothing here is handed to a shell.

    fn spelled(session: &Session, name: &str, operation: Operation) -> String {
        session
            .command(name, operation, None)
            .expect("a command")
            .join(" ")
    }

    // AYEAYE-61 — shared code holds one name for the service. It must not
    // matter whether it was spelled the systemd way, the launchd way, or
    // neither.
    #[test]
    fn one_name_works_whichever_platform_it_was_written_for() {
        let systemd = Session::systemd();
        for spelling in ["ayeaye", "ayeaye.service", "dev.ayeaye"] {
            assert_eq!(
                systemd.unit_name(spelling).expect("a unit"),
                "ayeaye.service",
                "{spelling} must not become anything else under systemd"
            );
        }
        let launchd = Session::launchd("501");
        for spelling in ["ayeaye", "ayeaye.service", "dev.ayeaye"] {
            assert_eq!(
                launchd.unit_name(spelling).expect("a label"),
                "dev.ayeaye",
                "{spelling} must not become anything else under launchd"
            );
        }
    }

    // AYEAYE-61 — a unit that is not a service keeps its own kind.
    #[test]
    fn a_unit_that_is_not_a_service_keeps_its_own_kind() {
        let systemd = Session::systemd();
        assert_eq!(
            systemd.unit_name("ayeaye.socket").expect("a unit"),
            "ayeaye.socket"
        );
        assert_eq!(
            systemd.unit_name("ayeaye.timer").expect("a unit"),
            "ayeaye.timer"
        );
        assert_eq!(
            systemd.unit_name("ayeaye.target").expect("a unit"),
            "ayeaye.target"
        );
    }

    // AYEAYE-61 — the prefix is a tunable, and a name already carrying the new
    // one is left alone rather than given it twice.
    #[test]
    fn the_launchd_prefix_is_a_tunable_and_really_is_used() {
        let mut session = Session::launchd("501");
        session.launchd_prefix = "com.example".to_string();
        assert_eq!(
            session.unit_name("ayeaye").expect("a label"),
            "com.example.ayeaye"
        );
        assert_eq!(
            session.unit_name("com.example.ayeaye").expect("a label"),
            "com.example.ayeaye"
        );
        assert_eq!(
            spelled(&session, "ayeaye", Operation::Start),
            "launchctl kickstart gui/501/com.example.ayeaye"
        );
    }

    // AYEAYE-61 — every systemd command, exactly as the shell spells it.
    #[test]
    fn systemd_generates_its_user_scoped_commands() {
        let session = Session::systemd();
        assert_eq!(
            spelled(&session, "ayeaye", Operation::Reload),
            "systemctl --user daemon-reload"
        );
        assert_eq!(
            spelled(&session, "ayeaye", Operation::Enable),
            "systemctl --user enable --now ayeaye.service"
        );
        assert_eq!(
            spelled(&session, "ayeaye", Operation::Disable),
            "systemctl --user disable --now ayeaye.service"
        );
        assert_eq!(
            spelled(&session, "ayeaye", Operation::Start),
            "systemctl --user start ayeaye.service"
        );
        assert_eq!(
            spelled(&session, "ayeaye", Operation::Stop),
            "systemctl --user stop ayeaye.service"
        );
        assert_eq!(
            spelled(&session, "ayeaye", Operation::Status),
            "systemctl --user status ayeaye.service"
        );
    }

    // AYEAYE-61 — this project installs a per-user service. Root is never
    // right, and the system manager is a different thing entirely.
    #[test]
    fn every_systemd_command_is_user_scoped_and_none_asks_for_root() {
        let session = Session::systemd();
        for operation in [
            Operation::Reload,
            Operation::Enable,
            Operation::Disable,
            Operation::Start,
            Operation::Stop,
            Operation::Status,
        ] {
            let argv = session
                .command("ayeaye", operation, None)
                .expect("a command");
            assert!(
                argv.contains(&"--user".to_string()),
                "{operation:?} left the user session"
            );
            assert!(
                !argv.contains(&"sudo".to_string()),
                "{operation:?} reached for root"
            );
        }
    }

    // AYEAYE-61 — every launchd command addresses a domain by uid.
    #[test]
    fn launchd_generates_domain_addressed_commands() {
        let session = Session::launchd("501");
        let plist = "/Users/John Smith/Library/LaunchAgents/dev.ayeaye.plist";
        assert_eq!(
            session
                .command("ayeaye", Operation::Enable, Some(plist))
                .expect("a command"),
            vec!["launchctl", "bootstrap", "gui/501", plist],
            "a path with a space in it is one element, not a quoting rule"
        );
        assert_eq!(
            spelled(&session, "ayeaye", Operation::Disable),
            "launchctl bootout gui/501/dev.ayeaye"
        );
        assert_eq!(
            spelled(&session, "ayeaye", Operation::Start),
            "launchctl kickstart gui/501/dev.ayeaye"
        );
        assert_eq!(
            spelled(&session, "ayeaye", Operation::Status),
            "launchctl print gui/501/dev.ayeaye"
        );
    }

    // AYEAYE-61 — bootout is the inverse of bootstrap, so mapping stop to it
    // would make stop-then-start work on Linux and fail on a Mac with "Could
    // not find service".
    #[test]
    fn stopping_leaves_the_service_registered_on_both_platforms() {
        let session = Session::launchd("501");
        assert_eq!(
            spelled(&session, "ayeaye", Operation::Stop),
            "launchctl kill SIGTERM gui/501/dev.ayeaye"
        );
        assert_ne!(
            session.command("ayeaye", Operation::Stop, None),
            session.command("ayeaye", Operation::Disable, None),
            "stop and disable are different questions"
        );
    }

    // AYEAYE-61 — the three refusals, and which kind each one is.
    #[test]
    fn a_refusal_says_whether_it_is_the_machine_or_the_caller() {
        let launchd = Session::launchd("501");
        assert!(
            matches!(
                launchd.command("ayeaye", Operation::Reload, None),
                Err(Unavailable::Machine(_))
            ),
            "launchd has no reload and must not invent one"
        );
        assert!(
            matches!(
                launchd.command("ayeaye", Operation::Enable, None),
                Err(Unavailable::Machine(_))
            ),
            "bootstrapping without a plist path would be a guess"
        );

        let no_uid = Session {
            manager: Manager::Launchd,
            uid: None,
            launchd_prefix: DEFAULT_LAUNCHD_PREFIX.to_string(),
        };
        assert!(
            matches!(
                no_uid.command("ayeaye", Operation::Start, None),
                Err(Unavailable::Machine(_))
            ),
            "gui//dev.ayeaye addresses nothing at all"
        );

        assert!(
            matches!(
                Session::systemd().command("", Operation::Start, None),
                Err(Unavailable::Caller(_))
            ),
            "a missing name is a bug above this layer, not a fact about the machine"
        );
    }

    // AYEAYE-61 — the path a definition lands at, which is also the path an
    // install from a previous version of this program wrote to.
    #[test]
    fn a_definition_lands_where_the_previous_install_put_it() {
        let layout = layout();
        assert_eq!(
            Session::systemd()
                .definition_path("ayeaye", &layout)
                .expect("a path"),
            format!("{CONFIG_HOME}/systemd/user/ayeaye.service")
        );
        assert_eq!(
            Session::launchd("501")
                .definition_path("ayeaye", &layout)
                .expect("a path"),
            format!("{HOME}/Library/LaunchAgents/dev.ayeaye.plist")
        );
    }

    // AYEAYE-61 — the label, the filename and the bootstrap target are one
    // string. They were two once: the plist's Label came from the layout and
    // its filename from the session, so changing the prefix installed
    // `com.example.ayeaye` into `dev.ayeaye.plist` and then bootstrapped a
    // target that did not exist.
    #[test]
    fn the_label_the_filename_and_the_target_are_one_string() {
        let mut session = Session::launchd("501");
        session.launchd_prefix = "com.example".to_string();
        let layout = layout();

        let agent = render(&session, &ayeaye(), &layout);
        assert!(
            agent.contains("<string>com.example.ayeaye</string>"),
            "the rendered agent still carries the default prefix"
        );
        let path = session.definition_path("ayeaye", &layout).expect("a path");
        assert!(path.ends_with("/com.example.ayeaye.plist"));
        assert_eq!(
            spelled(&session, "ayeaye", Operation::Status),
            "launchctl print gui/501/com.example.ayeaye"
        );
        // And the one string really is one: the label the agent declares is the
        // stem of the file it is installed as.
        assert!(path.ends_with(&format!("/{}.plist", session.label("ayeaye"))));
    }

    // AYEAYE-61 — a definition may hold its name in any of the three
    // spellings, and the label must be normalised the way the filename is or
    // the two come apart again.
    #[test]
    fn a_label_is_normalised_the_way_a_filename_is() {
        let session = Session::launchd("501");
        for spelling in ["ayeaye", "ayeaye.service", "dev.ayeaye"] {
            assert_eq!(session.label(spelling), "dev.ayeaye");
        }
        let mut definition = ayeaye();
        definition.name = "ayeaye.service".to_string();
        assert!(
            render(&session, &definition, &layout()).contains("<string>dev.ayeaye</string>"),
            "a label must not become dev.ayeaye.service"
        );
    }

    // AYEAYE-61 — a unit with an empty `ExecStart` installs perfectly and
    // starts nothing, so it is refused before a byte is rendered.
    #[test]
    fn a_definition_with_no_program_is_not_a_definition() {
        let mut definition = ayeaye();
        definition.argv.clear();
        for session in [Session::systemd(), Session::launchd("501")] {
            assert!(matches!(
                session.render(&definition, &layout()),
                Err(Unavailable::Caller(_))
            ));
        }
    }

    // AYEAYE-61 — systemd reads an empty word as the end of the arguments, so
    // an argument that is genuinely empty has to be spelled out.
    #[test]
    fn an_empty_argument_is_spelled_rather_than_dropped() {
        assert_eq!(systemd_arg(""), "\"\"");
        let mut definition = ayeaye();
        definition.argv.push(String::new());
        assert!(
            render_systemd(&definition, &layout())
                .contains(&format!("ExecStart={REPO}/bin/ayeaye \"\"\n"))
        );
    }

    // AYEAYE-61 — leaving an unchanged file alone is the difference between
    // repairing a service and disturbing one.
    #[test]
    fn an_identical_definition_is_left_alone() {
        let rendered = render_systemd(&ayeaye(), &layout());
        assert_eq!(plan_install(None, &rendered), Install::Create);
        assert_eq!(plan_install(Some(&rendered), &rendered), Install::Unchanged);
        assert_eq!(
            plan_install(Some("[Unit]\nDescription=something older\n"), &rendered),
            Install::Replace
        );
        assert_eq!(
            plan_install(
                Some(&rendered.replace("RestartSec=5", "RestartSec=9")),
                &rendered
            ),
            Install::Replace,
            "one line different is different"
        );
    }
}
