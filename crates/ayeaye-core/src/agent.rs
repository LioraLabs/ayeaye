//! Starting an agent in a pane, and stopping one.
//!
//! Nothing here runs tmux, opens a directory or types anything. This is what
//! there is to decide — which agents may be started, what the session is
//! called, which subcommand makes the pane, and what gets typed into it — and
//! the shell above turns the answers into subprocesses.

/// The programs that may be started, and the command each one is.
///
/// An allowlist rather than a name interpolated out of the request. This
/// endpoint starts processes on the user's machine at the word of anything
/// holding the token, and the difference between a table and a string is the
/// difference between "start claude" and "start whatever this says".
pub const AGENTS: &[(&str, &str)] = &[("claude", "claude"), ("codex", "codex")];

/// The command that starts this agent, if it is one we start at all.
pub fn agent_command(name: &str) -> Option<&'static str> {
    AGENTS
        .iter()
        .find(|(agent, _)| *agent == name)
        .map(|(_, command)| *command)
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
    let mut name = match trimmed.rsplit('/').next().unwrap_or("") {
        "" => "root",
        last => last,
    };
    if let Some(declared) = tmux_yaml.and_then(declared_name) {
        name = declared;
    }
    name.replace('.', "_")
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

/// How to make the pane, and what to call what was made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Create {
    /// The tmux arguments, after the program itself.
    pub argv: Vec<String>,
    /// `"window"` or `"session"` — what the panel is told appeared.
    pub created: &'static str,
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
            "window",
        )
    } else {
        (
            vec![
                "new-session".to_string(),
                "-d".to_string(),
                "-s".to_string(),
                session.to_string(),
            ],
            "session",
        )
    };
    // `-c` starts it in the project; `-P -F` makes tmux print the pane it made,
    // which is the only way to know which pane to type into afterwards.
    for word in ["-c", dir, "-P", "-F", "#{pane_id}"] {
        argv.push(word.to_string());
    }
    Create { argv, created }
}

#[cfg(test)]
mod tests {
    use super::{agent_command, create, session_name};

    fn argv(session: &str, dir: &str, exists: bool) -> Vec<String> {
        create(session, dir, exists).argv
    }

    // AYEAYE-51 — the agent is matched against a list, never interpolated: this
    // endpoint starts a process, and `bin/ayeaye` says so where it declares the
    // same table. A name that is not on it is refused rather than tried.
    #[test]
    fn only_the_agents_on_the_list_may_be_started() {
        assert_eq!(agent_command("claude"), Some("claude"));
        assert_eq!(agent_command("codex"), Some("codex"));
        for refused in ["", "bash", "rm", "claude; rm -rf ~", "CLAUDE", " claude"] {
            assert_eq!(agent_command(refused), None, "{refused:?} is not an agent");
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
        assert_eq!(session_name("/home/alex/dev/ayeaye", Some(yaml)), "work_bench");
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
        assert_eq!(create("ayeaye", "/home/alex/dev/ayeaye", false).created, "session");
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
        assert_eq!(create("ayeaye", "/home/alex/dev/ayeaye", true).created, "window");
    }
}
