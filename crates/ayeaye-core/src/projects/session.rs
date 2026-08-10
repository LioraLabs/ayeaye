//! What the tmux session for a project is called.

use super::rank::basename;

/// The session name a project's directory implies.
///
/// The ftz/ftn convention: the directory's own name with its dots replaced,
/// because tmux reads a dot as a pane address. A project that names its own
/// session in `.tmux.yaml` overrides that — which is how a repo whose
/// directory is `src` avoids being one of five sessions called `src`.
///
/// The file's text is an argument rather than a path: reading it is the
/// shell's, and whether it exists at all is the shell's answer to give.
pub fn name_for(path: &str, tmux_yaml: Option<&str>) -> String {
    let named = tmux_yaml.and_then(declared_name).unwrap_or_else(|| {
        let name = basename(path);
        if name == "/" { "root" } else { name }.to_string()
    });
    let named = if named.is_empty() {
        "root".to_string()
    } else {
        named
    };
    named.replace('.', "_")
}

/// The `session_name:` a `.tmux.yaml` declares, if it declares one.
///
/// At the start of a line and the first one only, exactly as the daemon's
/// line-by-line read of the file does it — an indented `session_name:` is a
/// key of something else, not the session's.
fn declared_name(yaml: &str) -> Option<String> {
    yaml.lines()
        .find_map(|line| line.strip_prefix("session_name:"))
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::name_for;

    // AYEAYE-50 — the picker shows this name and the spawn endpoint attaches
    // to it, so it has to be the same name the ftz/ftn convention produces or
    // starting an agent from the phone opens a second session beside the one
    // already running.
    #[test]
    fn a_session_is_named_after_the_directory_with_its_dots_replaced() {
        assert_eq!(name_for("/home/a/ayeaye", None), "ayeaye");
        assert_eq!(name_for("/home/a/my.project", None), "my_project");
        assert_eq!(name_for("/home/a/ayeaye/", None), "ayeaye");
        // tmux would read a dot as a pane address, so every one goes.
        assert_eq!(name_for("/home/a/a.b.c", None), "a_b_c");
        // The root has no basename to be named after.
        assert_eq!(name_for("/", None), "root");
        assert_eq!(name_for("", None), "root");
    }

    // AYEAYE-50 — a project that names its own session wins, which is how a
    // repo whose directory is `src` avoids being one of five sessions called
    // `src`.
    #[test]
    fn a_project_that_names_its_own_session_wins() {
        let yaml = "name: whatever\nsession_name: chosen\nwindows:\n  - one\n";
        assert_eq!(name_for("/home/a/src", Some(yaml)), "chosen");
        // Dots in the chosen name go too: it is still a tmux name.
        assert_eq!(
            name_for("/home/a/src", Some("session_name: my.chosen\n")),
            "my_chosen"
        );
        // Only at the start of a line, and only the first one.
        assert_eq!(
            name_for("/home/a/src", Some("  session_name: indented\n")),
            "src"
        );
        assert_eq!(
            name_for(
                "/home/a/src",
                Some("session_name: first\nsession_name: second\n")
            ),
            "first"
        );
        // An empty value is not a name, so the directory keeps deciding.
        assert_eq!(name_for("/home/a/src", Some("session_name:\n")), "src");
        assert_eq!(name_for("/home/a/src", Some("session_name:   \n")), "src");
        // And a file with nothing to say about it changes nothing.
        assert_eq!(name_for("/home/a/src", Some("windows:\n  - one\n")), "src");
    }
}
