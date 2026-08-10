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
//! This is a port of `lib/steps/70-service.sh` and `lib/service.sh`, and the
//! golden files under `tests/fixtures/units/` are what makes it a port rather
//! than a rewrite: the renderers here are compared against them byte for byte.

/// The user-session service manager a machine has.
///
/// Which one this machine has is not decided here — detection is the shell's,
/// and it arrives as a value. "Neither" is spelled `Option<Manager>` by the
/// caller rather than as a variant, so that every function below is total.
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
    pub fn ayeaye(program: &str) -> Self {
        Definition {
            name: "ayeaye".to_string(),
            title: "voice remote for tmux (phone web UI)".to_string(),
            after: None,
            argv: vec![program.to_string()],
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
    /// The reverse-DNS prefix for a launchd label.
    pub launchd_prefix: String,
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
            launchd_prefix: DEFAULT_LAUNCHD_PREFIX.to_string(),
            launchd_path: DEFAULT_LAUNCHD_PATH.to_string(),
        }
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

/// A systemd user unit, whole.
pub fn render_systemd(definition: &Definition, layout: &Layout) -> String {
    let mut out = String::new();
    out.push_str("[Unit]\n");
    out.push_str("Description=");
    out.push_str(&definition.title);
    out.push('\n');
    if let Some(after) = definition.after.as_deref().filter(|a| !a.is_empty()) {
        out.push_str("After=");
        out.push_str(after);
        out.push('\n');
    }
    out.push_str(
        "\n[Service]\n\
         Type=simple\n\
         # Generated by setup, and replaced whole every time it runs. Nothing is\n\
         # configured here: every setting lives in the environment file below. Edit\n\
         # that, then `systemctl --user restart ",
    );
    out.push_str(&definition.name);
    out.push_str(
        "`.\n\
         #\n\
         # The leading \"-\" means a settings file that is not there yet is not a reason\n\
         # to refuse to start: ayeaye has a default for everything in it, and a restart\n\
         # loop would be a worse answer than running on the defaults.\n\
         EnvironmentFile=-",
    );
    out.push_str(&systemd_value(&layout.env_file));
    out.push_str("\nExecStart=");
    let argv: Vec<String> = definition.argv.iter().map(|arg| systemd_arg(arg)).collect();
    out.push_str(&argv.join(" "));
    out.push_str(
        "\nRestart=on-failure\n\
         RestartSec=5\n\
         \n\
         [Install]\n\
         WantedBy=default.target\n",
    );
    out
}

/// A launchd property list, whole.
///
/// The differences from the unit above are forced by the platform and are not
/// choices. launchd has no `EnvironmentFile`, so the agent is handed the
/// *location* of the settings rather than any value out of them. It has no
/// journal, so the agent names the file it writes. And it starts an agent with
/// almost no `PATH`, so one is supplied.
pub fn render_launchd(definition: &Definition, layout: &Layout) -> String {
    let log = format!("{}/{}.log", layout.log_dir, definition.name);
    let mut out = String::new();
    out.push_str(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \
         \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <plist version=\"1.0\">\n\
         <dict>\n\
         \x20 <!-- Generated by setup, and replaced whole every time it runs. Nothing is\n\
         \x20      configured here: every setting lives in the environment file this\n\
         \x20      agent reads. Edit that, then restart the agent. -->\n\
         \x20 <key>Label</key>\n\
         \x20 <string>",
    );
    out.push_str(&xml_text(&label(&definition.name, &layout.launchd_prefix)));
    out.push_str(
        "</string>\n\
         \x20 <key>ProgramArguments</key>\n\
         \x20 <array>\n",
    );
    for arg in &definition.argv {
        out.push_str("    <string>");
        out.push_str(&xml_text(arg));
        out.push_str("</string>\n");
    }
    out.push_str(
        "  </array>\n\
         \x20 <key>EnvironmentVariables</key>\n\
         \x20 <dict>\n\
         \x20   <key>PATH</key>\n\
         \x20   <string>",
    );
    out.push_str(&xml_text(&layout.launchd_path));
    out.push_str(
        "</string>\n\
         \x20   <key>XDG_CONFIG_HOME</key>\n\
         \x20   <string>",
    );
    out.push_str(&xml_text(&layout.config_home));
    out.push_str(
        "</string>\n\
         \x20   <key>XDG_STATE_HOME</key>\n\
         \x20   <string>",
    );
    out.push_str(&xml_text(&layout.state_home));
    out.push_str(
        "</string>\n\
         \x20 </dict>\n\
         \x20 <key>RunAtLoad</key>\n\
         \x20 <true/>\n\
         \x20 <key>KeepAlive</key>\n\
         \x20 <dict>\n\
         \x20   <key>SuccessfulExit</key>\n\
         \x20   <false/>\n\
         \x20 </dict>\n\
         \x20 <key>ThrottleInterval</key>\n\
         \x20 <integer>5</integer>\n\
         \x20 <key>StandardOutPath</key>\n\
         \x20 <string>",
    );
    out.push_str(&xml_text(&log));
    out.push_str(
        "</string>\n\
         \x20 <key>StandardErrorPath</key>\n\
         \x20 <string>",
    );
    out.push_str(&xml_text(&log));
    out.push_str(
        "</string>\n\
         </dict>\n\
         </plist>\n",
    );
    out
}

/// The launchd label for a logical name.
fn label(name: &str, prefix: &str) -> String {
    format!("{prefix}.{name}")
}

#[cfg(test)]
mod tests {
    use super::{Definition, Layout, render_launchd, render_systemd, unrepresentable_in_systemd};

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

    fn ayeaye() -> Definition {
        Definition::ayeaye(&format!("{REPO}/bin/ayeaye"))
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
                .contains("ExecStart=\"/Users/John Smith/ayeaye/bin/ayeaye\"\n"),
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
                .contains("ExecStart=\"/srv/100%% $$path/bin/ayeaye\"\n"),
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
            vec![path.to_string()],
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
}
