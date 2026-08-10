//! Starting an agent in a pane, and stopping one.
//!
//! Nothing here runs tmux, opens a directory or types anything. This is what
//! there is to decide — which agents may be started, what the session is
//! called, which subcommand makes the pane, and what gets typed into it — and
//! the shell above turns the answers into subprocesses.

use crate::peer::PaneId;
use crate::{json, shell};

/// One agent this daemon knows how to start.
///
/// There is no way to make one but [`agent`], and no way to make that answer
/// with something that is not on [`AGENTS`]. That is the whole design: this
/// endpoint starts processes on the user's machine at the word of anything
/// holding the token, so "which program" must not be a string any caller can
/// spell. A function taking `&str` would have left the allowlist a convention.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Agent {
    name: &'static str,
    command: &'static str,
}

impl Agent {
    /// What the panel calls it, which is also what the request asked for.
    pub fn name(&self) -> &'static str {
        self.name
    }

    /// The program that starts it.
    pub fn command(&self) -> &'static str {
        self.command
    }
}

/// The agents that may be started, and the command each one is.
pub const AGENTS: &[Agent] = &[
    Agent {
        name: "claude",
        command: "claude",
    },
    Agent {
        name: "codex",
        command: "codex",
    },
];

/// The agent this name asks for, if it is one we start at all.
pub fn agent(name: &str) -> Option<Agent> {
    AGENTS.iter().copied().find(|agent| agent.name == name)
}

/// What the tmux session for a project is called.
///
/// The `ftz`/`ftn` convention `bin/ayeaye` mirrors: the last path component,
/// unless the project names itself in `.tmux.yaml`, with dots folded to
/// underscores because a dot in a session name is a target tmux reads as a
/// window separator.
///
/// The file's *text* is the argument rather than the path, so this stays a
/// decision. Reading it is the shell's, and a project without one passes
/// `None` — which is not the same as passing an empty file, though both end up
/// at the directory's own name.
pub fn session_name(dir: &str, tmux_yaml: Option<&str>) -> String {
    // Split on the separator rather than asking `std::path`: the daemon uses
    // `os.path.basename`, which is POSIX-only wherever it runs, and a path that
    // came out of a JSON body should not be read one way here and another way
    // on the machine that will `cd` into it.
    let trimmed = dir.trim_end_matches('/');
    let from_dir = match trimmed.rsplit('/').next().unwrap_or_default() {
        "" => "root",
        last => last,
    };
    // A dot is a window separator to tmux — `work.1` is window 1 of `work` —
    // so it cannot survive into a session name whichever half named it.
    tmux_yaml
        .and_then(declared_name)
        .unwrap_or(from_dir)
        .replace('.', "_")
}

/// The `session_name:` a project declares, if it declares one.
///
/// The first such line and nothing else — this is not a YAML parser and must
/// not become one. `bin/ayeaye` reads the file the same way, one line at a
/// time, stopping at the first hit.
fn declared_name(tmux_yaml: &str) -> Option<&str> {
    tmux_yaml
        .lines()
        .find_map(|line| line.strip_prefix("session_name:"))
        .map(str::trim)
        .filter(|declared| !declared.is_empty())
}

/// What appeared when the pane was made.
///
/// Two values, so it is two values everywhere. The panel says "new {created}
/// in {session}", and a free-form label would let a caller announce a window it
/// did not make — which is the same failure [`Create`] bundles the argv and the
/// label to prevent, one layer further out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Created {
    /// The project had no session, so this made one.
    Session,
    /// The project's session was already there, so this made a window in it.
    Window,
}

impl Created {
    /// What the panel calls it.
    pub fn as_str(&self) -> &'static str {
        match self {
            Created::Session => "session",
            Created::Window => "window",
        }
    }
}

/// How to make the pane, and what to call what was made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Create {
    /// The tmux arguments, after the program itself.
    pub argv: Vec<String>,
    /// What the panel is told appeared.
    pub created: Created,
}

/// Make a pane for this project: a window in its session, or the session.
///
/// An existing session gets a window rather than a second session, so
/// everything started in one project stays under one name. The argv and the
/// label are one answer rather than two, because a caller that derived the
/// label separately could report a window it did not make.
pub fn create(session: &str, dir: &str, session_exists: bool) -> Create {
    // `=` forces an exact match. Without it tmux resolves `-t name` as a
    // prefix, and a project whose name is the start of another project's would
    // open its window in somebody else's session.
    let (mut argv, created) = if session_exists {
        (
            vec![
                "new-window".to_string(),
                "-t".to_string(),
                format!("={session}"),
            ],
            Created::Window,
        )
    } else {
        (
            vec![
                "new-session".to_string(),
                "-d".to_string(),
                "-s".to_string(),
                session.to_string(),
            ],
            Created::Session,
        )
    };
    // `-c` starts it in the project; `-P -F` makes tmux print the pane it made,
    // which is the only way to know which pane to type into afterwards.
    for word in ["-c", dir, "-P", "-F", "#{pane_id}"] {
        argv.push(word.to_string());
    }
    Create { argv, created }
}

/// The line typed into the pane's shell to start the agent.
///
/// Both agents take an opening prompt as an argument, which is what sidesteps
/// guessing how long a TUI takes to boot before anything can be pasted into
/// it. The prompt is collapsed to one line first — the daemon's
/// `" ".join(prompt.split())` — because this is a *command line*, and then
/// quoted, which is where a prompt that cannot be typed is refused.
///
/// The agent is an [`Agent`] rather than a command string: the program half of
/// this line is the half that must never be spellable by a caller.
pub fn command_line(agent: Agent, prompt: &str) -> Result<String, shell::Unquotable> {
    let collapsed = one_line(prompt);
    if collapsed.is_empty() {
        return Ok(agent.command.to_string());
    }
    Ok(format!("{} {}", agent.command, shell::quote(&collapsed)?))
}

/// Every run of whitespace becomes one space, and the ends lose theirs.
///
/// `bin/ayeaye`'s `" ".join(prompt.split())`, with one difference worth naming:
/// Python counts the C0 separators `\x1c`–`\x1f` as whitespace and Rust does
/// not, so those survive this and are refused by the quoting instead. That is
/// the safe direction — a byte nobody typed on purpose is refused rather than
/// silently becoming a space.
fn one_line(prompt: &str) -> String {
    prompt.split_whitespace().collect::<Vec<&str>>().join(" ")
}

/// What the panel is told after an agent has been started.
///
/// The pane id is **qualified** — `desktop/%7`, not `%7`. `bin/ayeaye` answers
/// with the bare id because it has only ever had one machine; here every id
/// `/api/panes` hands out carries its host, and `share/app.html` takes this
/// field as its selection (`sel = d.pane`) and then compares it against those.
/// A bare id would select a pane the panel could never find again.
///
/// Which is why the argument is a [`PaneId`] and not a string: the shell above
/// holds the bare `%7` tmux printed, and handing that straight through has to
/// be a thing it cannot do rather than a thing it must remember not to.
pub fn spawned_body(pane: &PaneId, session: &str, created: Created, agent: Agent) -> String {
    format!(
        "{{\"pane\":{},\"session\":{},\"created\":{},\"agent\":{}}}",
        json::string(&pane.qualified()),
        json::string(session),
        json::string(created.as_str()),
        json::string(agent.name()),
    )
}

/// What the panel is told after a pane has been killed.
///
/// That it happened, and nothing about what is left. The panel re-reads the
/// pane list afterwards, which is what makes "the panel reflects it" a fact
/// about tmux rather than a claim made here.
pub fn killed_body() -> &'static str {
    r#"{"ok":true}"#
}

/// The sentences a spawn or a kill is refused with.
///
/// Transcribed from `bin/ayeaye` rather than reinvented. They are read by
/// whoever is holding the phone — `share/app.html` puts `d.error` on screen
/// unchanged — and the daemon's wording is what that person has been reading
/// until now. Keeping them here rather than at the call site is also what
/// makes them somebody's decision instead of a literal in a request handler.
pub mod refused {
    /// The agent named is not on the allowlist. `bin/ayeaye:1885`.
    pub const UNKNOWN_AGENT: &str = "unknown agent";
    /// The project path is not a directory. `bin/ayeaye:1887`.
    pub const NO_SUCH_DIRECTORY: &str = "no such directory";
    /// The body was not JSON, or was not an object. `bin/ayeaye:2686`.
    pub const BAD_JSON: &str = "bad json";
    /// A kill that named no pane at all. `bin/ayeaye:2690`.
    pub const NO_PANE: &str = "no pane";
    /// A kill naming a pane that is not in the list this server just read.
    /// `bin/ayeaye:1934`. Deliberately the same answer for a pane that does not
    /// exist, one that is hidden, and one on a machine we have never heard of:
    /// the panel offers only panes it was given, so anything else is a caller
    /// asking about a pane that is not theirs to ask about.
    pub const NO_SUCH_PANE: &str = "no such pane";

    /// tmux ran and made nothing. `bin/ayeaye:1902`.
    pub fn nothing_was_created(created: super::Created) -> String {
        format!("tmux would not create the {}", created.as_str())
    }

    /// tmux refused the kill, quoting what it said. `bin/ayeaye:1942`.
    pub fn could_not_kill(said: &str) -> String {
        let said = said.trim();
        let said = if said.is_empty() {
            "tmux refused the request"
        } else {
            said
        };
        format!("could not kill pane: {said}")
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Agent, Created, agent, command_line, create, killed_body, refused, session_name,
        spawned_body,
    };
    use crate::peer::{HostName, PaneId};
    use crate::shell::Unquotable;

    fn argv(session: &str, dir: &str, exists: bool) -> Vec<String> {
        create(session, dir, exists).argv
    }

    fn claude() -> Agent {
        agent("claude").expect("claude is an agent")
    }

    fn pane(host: &str, pane: &str) -> PaneId {
        PaneId::new(HostName::new(host).expect("a host name"), pane).expect("a pane id")
    }

    // AYEAYE-51 — the agent is matched against a list, never interpolated: this
    // endpoint starts a process, and `bin/ayeaye` says so where it declares the
    // same table. A name that is not on it is refused rather than tried, and
    // there is no other way to get an `Agent` — which is what makes the list a
    // rule rather than a habit.
    #[test]
    fn only_the_agents_on_the_list_may_be_started() {
        assert_eq!(claude().command(), "claude");
        assert_eq!(claude().name(), "claude");
        assert_eq!(agent("codex").map(|it| it.command()), Some("codex"));
        for spelling in ["", "bash", "rm", "claude; rm -rf ~", "CLAUDE", " claude"] {
            assert!(agent(spelling).is_none(), "{spelling:?} is not an agent");
        }
    }

    // AYEAYE-51 — the session a project's agents live in, transcribed from
    // `bin/ayeaye`'s `session_name_for` rather than derived from this code. Two
    // agents started in one directory have to land in one session, and they can
    // only do that if this agrees with what the daemon has been naming sessions
    // all along.
    #[test]
    fn a_project_is_named_the_way_the_daemon_names_it() {
        assert_eq!(session_name("/home/alex/dev/ayeaye", None), "ayeaye");
        // Trailing slashes are stripped first, however many there are.
        assert_eq!(session_name("/home/alex/dev/ayeaye/", None), "ayeaye");
        assert_eq!(session_name("/home/alex/dev/ayeaye///", None), "ayeaye");
        // A dot is a window separator to tmux, so it cannot be in the name.
        assert_eq!(session_name("/home/alex/.config/nvim.d", None), "nvim_d");
        // The root directory has no last component, and a session still needs
        // a name.
        assert_eq!(session_name("/", None), "root");
        assert_eq!(session_name("", None), "root");
        // A relative path is still a path.
        assert_eq!(session_name("ayeaye", None), "ayeaye");
    }

    // AYEAYE-51 — a project that names itself wins over its directory, which is
    // what makes `ftz` and this app agree about where a project's windows go.
    #[test]
    fn a_project_that_names_itself_in_tmux_yaml_wins() {
        let yaml = "windows:\n  - editor\nsession_name: work.bench\nroot: .\n";
        assert_eq!(
            session_name("/home/alex/dev/ayeaye", Some(yaml)),
            "work_bench"
        );
        // Only at the start of a line, and only the first one.
        assert_eq!(
            session_name(
                "/dev/proj",
                Some("session_name: first\nsession_name: second\n")
            ),
            "first"
        );
        assert_eq!(
            session_name("/dev/proj", Some("  session_name: indented\n")),
            "proj",
            "an indented key is not the key"
        );
        // A key with nothing after it leaves the directory's name standing,
        // rather than naming the session the empty string.
        assert_eq!(session_name("/dev/proj", Some("session_name:\n")), "proj");
        assert_eq!(session_name("/dev/proj", Some("")), "proj");
    }

    // AYEAYE-51 — the two subcommands, transcribed from `bin/ayeaye`'s
    // `spawn_agent`. `-P -F #{pane_id}` is what makes tmux print the pane it
    // made, which is the only way to know which pane to type into; `-c` is what
    // starts it in the project rather than wherever the daemon happens to be.
    #[test]
    fn a_project_with_no_session_gets_one_and_a_project_with_one_gets_a_window() {
        assert_eq!(
            argv("ayeaye", "/home/alex/dev/ayeaye", false),
            [
                "new-session",
                "-d",
                "-s",
                "ayeaye",
                "-c",
                "/home/alex/dev/ayeaye",
                "-P",
                "-F",
                "#{pane_id}"
            ]
        );
        assert_eq!(
            create("ayeaye", "/home/alex/dev/ayeaye", false).created,
            Created::Session
        );
    }

    // AYEAYE-51 — and the `=`. Without it tmux resolves `-t ayeaye` as a
    // prefix, so starting an agent in `ayeaye` would open a window in
    // `ayeaye-one-binary` if that session sorted first — someone else's project
    // gaining a window nobody asked for.
    #[test]
    fn an_existing_session_is_named_exactly_rather_than_by_prefix() {
        assert_eq!(
            argv("ayeaye", "/home/alex/dev/ayeaye", true),
            [
                "new-window",
                "-t",
                "=ayeaye",
                "-c",
                "/home/alex/dev/ayeaye",
                "-P",
                "-F",
                "#{pane_id}"
            ]
        );
        assert_eq!(
            create("ayeaye", "/home/alex/dev/ayeaye", true).created,
            Created::Window
        );
    }

    // AYEAYE-51 — "spawn takes a project path, an agent, and an **optional**
    // prompt". No prompt is the agent on its own; a prompt is one argument
    // after it, whatever the prompt happens to contain.
    #[test]
    fn a_prompt_becomes_one_argument_and_no_prompt_becomes_none() {
        assert_eq!(command_line(claude(), "").unwrap(), "claude");
        // A prompt of nothing but whitespace is no prompt. `bin/ayeaye` tests
        // the *raw* prompt (`if prompt:`) and so types `claude ''` here — an
        // empty argument the agent then has to interpret. This tests what is
        // left after collapsing, which is the thing that was actually asked
        // for. A deliberate divergence, and the better half of it.
        assert_eq!(command_line(claude(), "   \n  ").unwrap(), "claude");
        assert_eq!(
            command_line(claude(), "fix the tests").unwrap(),
            "claude 'fix the tests'"
        );
        assert_eq!(
            command_line(
                agent("codex").unwrap(),
                "run $(id) && rm -rf ~; echo 'done'"
            )
            .unwrap(),
            r"codex 'run $(id) && rm -rf ~; echo '\''done'\'''"
        );
    }

    // AYEAYE-51 — a prompt arrives from a phone keyboard, so it arrives with
    // line breaks in it. This is a command line: the collapse is what stops the
    // first newline submitting half a prompt, and it happens before the quoting
    // rather than after, or the quoting would be refusing text the user never
    // meant as separate lines.
    #[test]
    fn a_prompt_written_over_several_lines_is_collapsed_into_one() {
        assert_eq!(
            command_line(claude(), "  fix\n  the\ttests \r\n please  ").unwrap(),
            "claude 'fix the tests please'"
        );
    }

    // AYEAYE-51 — and what the collapse does not reach is refused rather than
    // typed. An escape byte is not whitespace, so it survives into the quoting,
    // and there it has no spelling that a terminal reads as text.
    #[test]
    fn a_prompt_that_cannot_be_typed_is_refused_with_its_reason() {
        assert_eq!(
            command_line(claude(), "fix \u{1b}[31mthe\u{1b}[0m tests").unwrap_err(),
            Unquotable::Control('\u{1b}')
        );
        assert_eq!(
            command_line(claude(), r"escape the \n in the regex").unwrap_err(),
            Unquotable::Backslash
        );
        // The one place the collapse and Python's `split()` disagree: Python
        // counts the C0 separators as whitespace and would quietly turn this
        // into a space. Here it survives the collapse and is refused, which is
        // the safe direction and the reason the difference is acceptable.
        assert_eq!(
            command_line(claude(), "fix\u{1c}the tests").unwrap_err(),
            Unquotable::Control('\u{1c}')
        );
    }

    // AYEAYE-51 — the refusals, transcribed from `bin/ayeaye` rather than
    // reinvented. They are put on screen unchanged by `share/app.html`, so they
    // are the wording somebody has been reading until now; a fresh set would be
    // a change to the product nobody asked for.
    #[test]
    fn a_refusal_says_what_the_daemon_says() {
        assert_eq!(refused::UNKNOWN_AGENT, "unknown agent");
        assert_eq!(refused::NO_SUCH_DIRECTORY, "no such directory");
        assert_eq!(refused::BAD_JSON, "bad json");
        assert_eq!(refused::NO_PANE, "no pane");
        assert_eq!(refused::NO_SUCH_PANE, "no such pane");
        assert_eq!(
            refused::nothing_was_created(Created::Session),
            "tmux would not create the session"
        );
        assert_eq!(
            refused::nothing_was_created(Created::Window),
            "tmux would not create the window"
        );
        assert_eq!(
            refused::could_not_kill("can't find pane: %99\n"),
            "could not kill pane: can't find pane: %99"
        );
        // tmux that failed and said nothing still has to produce a sentence,
        // or the panel shows "could not kill pane: " and trails off.
        assert_eq!(
            refused::could_not_kill("  \n"),
            "could not kill pane: tmux refused the request"
        );
    }

    // AYEAYE-51 — the body `share/app.html` reads after a spawn: it announces
    // "new {created} in {session}" and then selects `d.pane`. The id is
    // qualified, because that is what the pane list it will compare against
    // hands out — a bare one would select a pane the panel can never find.
    #[test]
    fn a_spawn_answers_with_a_qualified_pane_and_what_was_made() {
        assert_eq!(
            spawned_body(&pane("desktop", "%7"), "ayeaye", Created::Window, claude()),
            concat!(
                r#"{"pane":"desktop/%7","session":"ayeaye","#,
                r#""created":"window","agent":"claude"}"#
            )
        );
        // A session name comes from a directory somebody made, and a directory
        // name can hold a quote. It goes through the escaping like any other
        // text, or one oddly-named project answers with a body nothing parses.
        assert_eq!(
            spawned_body(
                &pane("Alex's Mac", "%1"),
                r#"say "hi""#,
                Created::Session,
                agent("codex").expect("codex is an agent")
            ),
            concat!(
                r#"{"pane":"Alex's Mac/%1","session":"say \"hi\"","#,
                r#""created":"session","agent":"codex"}"#
            )
        );
    }

    // AYEAYE-51 — a kill says only that it happened. The panel re-reads the
    // pane list afterwards rather than being told what is left, which is what
    // makes "the panel reflects it" a fact about tmux rather than a claim in
    // this body.
    #[test]
    fn a_kill_answers_that_it_happened_and_nothing_else() {
        assert_eq!(killed_body(), r#"{"ok":true}"#);
    }
}
