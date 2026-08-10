//! What tmux said, read as panes.
//!
//! Nothing here runs tmux. The shell above captures the text and hands it over,
//! which is what lets the parse be held to output really captured from a tmux
//! server rather than to output somebody imagined.

use crate::peer::{HostName, PaneId};

/// The `list-panes -F` format the pane list is read out of.
///
/// Seven tab-separated fields, exactly as `bin/ayeaye`'s `list_panes` asks for
/// them. It lives beside the parser rather than beside the caller: the format
/// and the fields it produces are one decision, and splitting them across two
/// crates is how a field gets added in one place and dropped in the other.
pub const PANE_FORMAT: &str = concat!(
    "#{pane_id}\t#{session_name}\t#{window_index}\t#{window_name}",
    "\t#{pane_current_command}\t#{?pane_active,1,0}\t#{?pane_dead,1,0}"
);

/// A live pane, as the panel needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pane {
    /// Qualified with the machine it is on, always.
    pub id: PaneId,
    /// The tmux session.
    pub session: String,
    /// The window index, as text: it is a label here, never arithmetic.
    pub window: String,
    /// The window name.
    pub name: String,
    /// What the pane is running, by tmux's reckoning.
    pub cmd: String,
    /// Whether it is the active pane of its window.
    pub active: bool,
}

/// Read `list-panes -a -F PANE_FORMAT` output.
///
/// Two kinds of pane are dropped, both the way the daemon drops them:
///
/// - a session whose name starts with `_`, which is a floating scratch pane
///   somebody's tmux configuration made and not a target;
/// - a dead pane, which `remain-on-exit` keeps around as history. It is not a
///   live session and must disappear from the panel rather than sit there
///   looking answerable.
///
/// A line that does not carry seven fields is skipped rather than guessed at.
pub fn panes(host: &HostName, text: &str) -> Vec<Pane> {
    let mut panes = Vec::new();
    for line in text.lines() {
        let fields: Vec<&str> = line.split('\t').collect();
        let [id, session, window, name, cmd, active, dead] = fields[..] else {
            continue;
        };
        if dead == "1" || session.starts_with('_') {
            continue;
        }
        // An id tmux cannot have given us is a line we cannot use. It is
        // skipped for the same reason a short line is: one unreadable pane must
        // not take the other twenty with it.
        let Ok(id) = PaneId::new(host.clone(), id) else {
            continue;
        };
        panes.push(Pane {
            id,
            session: session.to_string(),
            window: window.to_string(),
            name: name.to_string(),
            cmd: cmd.to_string(),
            active: active == "1",
        });
    }
    panes
}

/// Whether what tmux printed on stderr means "there is no server here".
///
/// A machine with no sessions is not a broken daemon, and the difference is
/// worth keeping: "tmux is not installed" and "you have no panes open" call for
/// completely different things from whoever is reading the panel.
pub fn no_server_running(stderr: &str) -> bool {
    let said = stderr.to_lowercase();
    NO_SERVER.iter().any(|phrase| said.contains(phrase))
}

/// The ways tmux says there is no server to talk to.
///
/// Matched as substrings because each comes with the socket path attached, and
/// three spellings rather than one because they are three different tmux
/// answers: a socket that was never there, a server that has since gone, and
/// the wording older builds use.
const NO_SERVER: &[&str] = &[
    "no server running",
    "server exited unexpectedly",
    "error connecting to",
];

#[cfg(test)]
mod tests {
    use super::{PANE_FORMAT, no_server_running, panes};
    use crate::peer::HostName;

    /// Real output, captured from a private `tmux -L` server carrying a
    /// two-pane window, a second named window, a `_scratch` session and a pane
    /// left dead by `remain-on-exit`.
    const MIXED: &str = include_str!("../../../tests/fixtures/tmux/list-panes-mixed");

    fn host() -> HostName {
        HostName::new("desktop").expect("a host name")
    }

    // AYEAYE-43 — the pane list the panel renders, read out of tmux's own
    // output: every id qualified, the scratch session and the dead pane gone.
    #[test]
    fn the_live_panes_come_back_qualified_and_the_rest_do_not_come_back() {
        let panes = panes(&host(), MIXED);

        let ids: Vec<String> = panes.iter().map(|pane| pane.id.qualified()).collect();
        assert_eq!(ids, ["desktop/%0", "desktop/%1", "desktop/%2"]);

        let first = &panes[0];
        assert_eq!(first.session, "work");
        assert_eq!(first.window, "0");
        assert_eq!(first.name, "editor");
        assert_eq!(first.cmd, "sh");
        assert!(first.active);
        // The second pane of the same window is the one that is not active.
        assert!(!panes[1].active);
        assert_eq!(panes[2].name, "cook");
    }

    // AYEAYE-43 — the format is what produced the fixture above. Transcribed
    // from `bin/ayeaye`'s `list_panes` rather than derived from the parser, so
    // the two can disagree.
    #[test]
    fn the_format_is_the_seven_fields_the_daemon_asks_for() {
        assert_eq!(
            PANE_FORMAT,
            "#{pane_id}\t#{session_name}\t#{window_index}\t#{window_name}\
             \t#{pane_current_command}\t#{?pane_active,1,0}\t#{?pane_dead,1,0}"
        );
        assert_eq!(MIXED.lines().count(), 5, "the fixture is that format's output");
    }

    // AYEAYE-43 — a machine with no tmux server has no panes, which is not the
    // same answer as a machine whose tmux could not be asked. Every spelling
    // here is one a real tmux prints: the first from a socket that was never
    // there, the second from one whose server has gone, the third from tmux
    // before it changed the wording.
    #[test]
    fn no_server_running_is_told_from_a_real_failure() {
        for said in [
            "no server running on /tmp/tmux-1000/ayeaye-43",
            "server exited unexpectedly",
            "error connecting to /tmp/tmux-1000/ayeaye-43 (No such file or directory)",
        ] {
            assert!(no_server_running(said), "{said:?} means no server");
            assert!(
                no_server_running(&format!("  {}\n", said.to_uppercase())),
                "and says so however it is spaced or shouted"
            );
        }
        // A tmux that answered, and a tmux that refused for its own reasons,
        // are both something the panel should be told about.
        assert!(!no_server_running(""));
        assert!(!no_server_running("unknown option -- Z"));
        assert!(!no_server_running("can't find pane: %99"));
    }

    // AYEAYE-43 — a line that is not seven fields is skipped, not guessed at,
    // and it takes only itself with it. This is the shape of the failure the
    // spec names from the other direction: one unreadable pane must never empty
    // the list for every other pane.
    #[test]
    fn an_unreadable_line_costs_only_itself() {
        let text = "\
%0\twork\t0\teditor\tsh\t1\t0
this is not a pane
%1\twork\t0
\t\t\t\t\t\t
%2\twork\t1\tcook\tsh\t0\t0
";
        let ids: Vec<String> = panes(&host(), text)
            .iter()
            .map(|pane| pane.id.qualified())
            .collect();
        assert_eq!(ids, ["desktop/%0", "desktop/%2"]);
        assert!(panes(&host(), "").is_empty());
    }
}
