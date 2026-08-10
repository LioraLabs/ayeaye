//! Starting an agent in a pane, and stopping one.
//!
//! Nothing here runs tmux, opens a directory or types anything. This is what
//! there is to decide — which agents may be started, what the session is
//! called, which subcommand makes the pane, and what gets typed into it — and
//! the shell above turns the answers into subprocesses.

use crate::peer::PaneId;
use crate::{json, quoting};

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
    pub fn name(self) -> &'static str {
        self.name
    }

    /// The program that starts it.
    pub fn command(self) -> &'static str {
        self.command
    }
}

/// The agents that may be started, and the command each one is.
///
/// `bin/ayeaye:1387`. A line added here is a program this daemon will start on
/// somebody's machine when a request asks it to, so
/// `the_list_is_exactly_these_two_agents` pins the whole table rather than the
/// entries somebody thought to check.
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

/// How much of a prompt is kept. `bin/ayeaye:2701`.
///
/// Characters, not bytes, because that is what the daemon counts and because a
/// cut in the middle of a character is not text. The number lives here rather
/// than beside the request that applies it: it is a decision about what this
/// app does with a prompt, and a decision that exists only as a literal in a
/// request handler is one that disappears the next time the handler is
/// rewritten.
pub const PROMPT_LIMIT: usize = 4000;

/// The prompt, cut to [`PROMPT_LIMIT`].
pub fn within_limit(prompt: &str) -> &str {
    match prompt.char_indices().nth(PROMPT_LIMIT) {
        Some((end, _)) => &prompt[..end],
        None => prompt,
    }
}

/// What the tmux session for a project is called.
///
/// The `ftz`/`ftn` convention `bin/ayeaye` mirrors: the last path component,
/// unless the project names itself in `.tmux.yaml`, with the two characters
/// tmux reads as target separators folded to underscores.
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
    tmux_yaml
        .and_then(declared_name)
        .unwrap_or(from_dir)
        .replace(SEPARATORS, "_")
}

/// The characters tmux reads as target separators, which a session name may not
/// carry.
///
/// A tmux target is `session:window.pane`, so both of these turn a name into a
/// *different* name plus a coordinate. `bin/ayeaye:1403` folds only the dot,
/// and the colon is the one that does real damage: tmux accepts a session
/// literally called `work:9`, and `new-window -t "=work:9"` then opens window 9
/// of the session `work` — someone else's project gaining a window nobody asked
/// for, which is exactly what the `=` is there to prevent. The `=` forces an
/// exact match on the *session* half and cannot help once the colon has ended
/// it.
///
/// A deliberate departure from the daemon, in the same direction as
/// `Tmux::sessions` splitting on lines: a project directory can be called
/// anything, and this is the only place that can stop what it is called from
/// naming somebody else's session.
const SEPARATORS: &[char] = &[':', '.'];

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
/// did not make — which is the same failure [`Creation`] bundles the argv and the
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
    pub fn as_str(self) -> &'static str {
        match self {
            Created::Session => "session",
            Created::Window => "window",
        }
    }
}

/// How to make the pane, and what to call what was made.
///
/// The two halves are read back through methods rather than taken from fields,
/// so the label a caller reports is the label that belongs to the argv it ran.
/// A `pub` field would have let a caller run [`Creation::argv`] and then hand
/// [`spawned_body`] the other variant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Creation {
    argv: Vec<String>,
    created: Created,
}

impl Creation {
    /// The tmux arguments, after the program itself.
    pub fn argv(&self) -> &[String] {
        &self.argv
    }

    /// What the panel is told appeared.
    pub fn created(&self) -> Created {
        self.created
    }
}

/// Make a pane for this project: a window in its session, or the session.
///
/// An existing session gets a window rather than a second session, so
/// everything started in one project stays under one name. The argv and the
/// label are one answer rather than two, because a caller that derived the
/// label separately could report a window it did not make.
pub fn create(session: &str, dir: &str, session_exists: bool) -> Creation {
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
    Creation { argv, created }
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
/// this line is the half that must never be spellable by a caller. The prompt
/// is expected to have been cut to [`PROMPT_LIMIT`] already — that is a fact
/// about the request rather than about the command line, so it belongs to
/// whoever read the body.
pub fn command_line(agent: Agent, prompt: &str) -> Result<String, quoting::Unquotable> {
    let collapsed = one_line(prompt);
    if collapsed.is_empty() {
        return Ok(agent.command.to_string());
    }
    Ok(format!("{} {}", agent.command, quoting::quote(&collapsed)?))
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
pub fn spawned_body(pane: &PaneId, session: &str, made: &Creation, agent: Agent) -> String {
    format!(
        "{{\"pane\":{},\"session\":{},\"created\":{},\"agent\":{}}}",
        json::string(&pane.qualified()),
        json::string(session),
        json::string(made.created().as_str()),
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
    /// The agent named is not on the allowlist. `bin/ayeaye:1889`.
    pub const UNKNOWN_AGENT: &str = "unknown agent";
    /// The project path is not a directory. `bin/ayeaye:1891`.
    pub const NO_SUCH_DIRECTORY: &str = "no such directory";
    /// A kill naming a pane that is not in the list this server just read.
    /// `bin/ayeaye:1934`. Deliberately the same answer for a pane that does not
    /// exist, one that is hidden, and one on a machine we have never heard of:
    /// the panel offers only panes it was given, so anything else is a caller
    /// asking about a pane that is not theirs to ask about.
    pub const NO_SUCH_PANE: &str = "no such pane";

    /// A request naming a machine this deployment has never heard of.
    ///
    /// One of the two refusals here with no counterpart in `bin/ayeaye`, which
    /// has never had a second machine to be asked about. The name is quoted
    /// back because the caller is the only one who can correct it, and it came
    /// from the caller in the first place.
    pub fn no_such_machine(named: &str) -> String {
        format!("no machine here is called {named:?}")
    }

    /// A request naming a machine we know but cannot reach.
    ///
    /// Unreachable today — the registry holds one peer and it is this one — and
    /// written anyway, because it is what the `host` field means. The federated
    /// case arrives as this branch being taken rather than as a new one.
    pub fn not_reachable_yet(host: &str) -> String {
        format!("{host} is another machine, and reaching one is not built yet")
    }

    /// tmux ran and made nothing. `bin/ayeaye:1904`.
    pub fn nothing_was_created(created: super::Created) -> String {
        format!("tmux would not create the {}", created.as_str())
    }

    /// The pane was made and the agent could not be started in it.
    ///
    /// The daemon has no counterpart because it never checks: it types and
    /// moves on. This one names the pane on purpose. By the time it can happen
    /// the pane really exists, so a refusal that said only "it did not work"
    /// would leave a pane running on somebody's machine that nothing had told
    /// them about — and the panel's next poll would show it with no explanation
    /// of where it came from.
    pub fn started_but_could_not_type(
        created: super::Created,
        pane: &crate::peer::PaneId,
        why: &str,
    ) -> String {
        format!(
            "started the {} as {} but could not type into it: {why}",
            created.as_str(),
            pane.qualified()
        )
    }

    /// tmux refused the kill, quoting what it said. `bin/ayeaye:1939` for a
    /// tmux that could not be run at all, `:1941` for the empty-stderr
    /// fallback, `:1942` for a tmux that ran and said no.
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
        AGENTS, Agent, Created, PROMPT_LIMIT, agent, command_line, create, killed_body, refused,
        session_name, spawned_body, within_limit,
    };
    use crate::peer::{HostName, PaneId};
    use crate::quoting::Unquotable;

    fn argv(session: &str, dir: &str, exists: bool) -> Vec<String> {
        create(session, dir, exists).argv().to_vec()
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

    // AYEAYE-51 — and the list is *these two*, whole. Asserting that claude and
    // codex are on it and that six other spellings are not leaves the table
    // open at the top: adding `("sh", "sh")` would keep every one of those
    // assertions true. This is the security boundary of the endpoint, so the
    // test closes the world rather than sampling it.
    #[test]
    fn the_list_is_exactly_these_two_agents() {
        let listed: Vec<(&str, &str)> = AGENTS
            .iter()
            .map(|agent| (agent.name(), agent.command()))
            .collect();
        assert_eq!(listed, [("claude", "claude"), ("codex", "codex")]);
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
        // A dot is a pane separator to tmux, so it cannot be in the name.
        assert_eq!(session_name("/home/alex/.config/nvim.d", None), "nvim_d");
        // The root directory has no last component, and a session still needs
        // a name.
        assert_eq!(session_name("/", None), "root");
        assert_eq!(session_name("", None), "root");
        // A relative path is still a path.
        assert_eq!(session_name("ayeaye", None), "ayeaye");
    }

    // AYEAYE-51 — found at the final gate, and the reason `create`'s `=` is not
    // enough on its own. A tmux target is `session:window.pane`, and tmux will
    // happily hold a session called `work:9` — so a project directory of that
    // name produces `new-window -t "=work:9"`, which opens **window 9 of the
    // session `work`**. Verified against a real tmux on a private socket: the
    // window lands in `work`, and the reply would name a session it is not in.
    //
    // `bin/ayeaye:1403` folds only the dot and has this hole. Folding both is a
    // departure from it, and the safe direction: a project can be called
    // anything, and this is the only place that can stop what it is called from
    // naming somebody else's session.
    #[test]
    fn a_name_cannot_carry_a_character_tmux_reads_as_a_target_separator() {
        assert_eq!(session_name("/dev/work:9", None), "work_9");
        assert_eq!(session_name("/dev/a:b.c", None), "a_b_c");
        assert_eq!(
            session_name("/dev/proj", Some("session_name: work:9\n")),
            "work_9",
            "a project that names itself does not get to skip this"
        );
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
        assert_eq!(
            session_name("/dev/proj", Some("   session_name:  \n")),
            "proj"
        );
        assert_eq!(session_name("/dev/proj", Some("")), "proj");
        // A file written on Windows, or by an editor that thinks it was. The
        // carriage return is trimmed with the rest of the whitespace, as
        // Python's `.strip()` trims it.
        assert_eq!(
            session_name("/dev/proj", Some("session_name: work\r\n")),
            "work"
        );
    }

    // AYEAYE-51 — a prompt arrives from the network, so its length is somebody
    // else's decision until this is applied. The cut is by character rather
    // than by byte: `bin/ayeaye`'s `[:4000]` counts characters, and a cut in
    // the middle of one is not text at all.
    #[test]
    fn a_prompt_is_cut_to_the_length_the_daemon_keeps() {
        assert_eq!(PROMPT_LIMIT, 4000);
        assert_eq!(within_limit("fix the tests"), "fix the tests");

        let long = "é".repeat(PROMPT_LIMIT + 500);
        let kept = within_limit(&long);
        assert_eq!(kept.chars().count(), PROMPT_LIMIT);
        // And it is still a string: cutting 4000 *bytes* off this would land
        // inside a character and could not be one.
        assert!(long.starts_with(kept));

        let exactly = "a".repeat(PROMPT_LIMIT);
        assert_eq!(within_limit(&exactly), exactly);
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
            create("ayeaye", "/home/alex/dev/ayeaye", false).created(),
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
            create("ayeaye", "/home/alex/dev/ayeaye", true).created(),
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
        // And the one refusal that has to name the pane: by the time it can
        // happen the pane exists, so a refusal that did not say which one would
        // leave a pane running that nothing had told anybody about.
        assert_eq!(
            refused::started_but_could_not_type(
                Created::Window,
                &pane("desktop", "%7"),
                "tmux: gave up after 5s"
            ),
            "started the window as desktop/%7 but could not type into it: \
             tmux: gave up after 5s"
        );
    }

    // AYEAYE-51 — the body `share/app.html` reads after a spawn: it announces
    // "new {created} in {session}" and then selects `d.pane`. The id is
    // qualified, because that is what the pane list it will compare against
    // hands out — a bare one would select a pane the panel can never find.
    #[test]
    fn a_spawn_answers_with_a_qualified_pane_and_what_was_made() {
        assert_eq!(
            spawned_body(
                &pane("desktop", "%7"),
                "ayeaye",
                &create("ayeaye", "/home/alex/dev/ayeaye", true),
                claude()
            ),
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
                &create("say.hi", "/dev/say.hi", false),
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
