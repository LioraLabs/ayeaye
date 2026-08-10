//! What tmux said, read as panes.
//!
//! Nothing here runs tmux. The shell above captures the text and hands it over,
//! which is what lets the parse be held to output really captured from a tmux
//! server rather than to output somebody imagined.

use crate::json;
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

/// The body `/api/panes` answers with.
///
/// The daemon's shape — the host this list came from, then the panes — with the
/// six fields `share/app.html` reads off each card. The id is qualified, and
/// the client carries it opaquely: nothing in the page splits it, which is what
/// lets a federated id and a local one be the same string to everything above
/// this line.
///
/// `failed` is what to say when tmux could not be asked at all. It is a field
/// rather than a status code because the panel must keep rendering; an empty
/// list with no explanation is the specific lie this app exists to prevent,
/// since "no agents need you" and "I could not look" are opposite answers.
pub fn panes_body(host: &HostName, panes: &[Pane], failed: Option<&str>) -> String {
    let mut body = format!("{{\"host\":{},\"panes\":[", json::string(host.as_str()));
    for (index, pane) in panes.iter().enumerate() {
        if index > 0 {
            body.push(',');
        }
        body.push_str(&format!(
            "{{\"id\":{},\"session\":{},\"window\":{},\"name\":{},\"cmd\":{},\"active\":{}}}",
            json::string(&pane.id.qualified()),
            json::string(&pane.session),
            json::string(&pane.window),
            json::string(&pane.name),
            json::string(&pane.cmd),
            pane.active,
        ));
    }
    body.push(']');
    if let Some(failed) = failed {
        body.push_str(&format!(",\"error\":{}", json::string(failed)));
    }
    body.push('}');
    body
}

/// Whether what tmux printed on stderr means "there is no server here".
///
/// A machine with no sessions is not a broken daemon, and the difference is
/// worth keeping: "tmux is not installed" and "you have no panes open" call for
/// completely different things from whoever is reading the panel.
pub fn no_server_running(stderr: &str) -> bool {
    let said = stderr.to_lowercase();
    if NO_SERVER.iter().any(|phrase| said.contains(phrase)) {
        return true;
    }
    // `error connecting to <socket> (<reason>)` is the older wording, and the
    // reason is the whole message: a socket that is not there and one that is
    // there but refuses are no server, while one we are not allowed to open is
    // a machine with panes on it we cannot see. Reporting that as "no panes"
    // would be the panel saying nothing needs you when it does not know.
    said.contains("error connecting to")
        && (said.contains("no such file or directory") || said.contains("connection refused"))
}

/// The ways tmux says there is no server to talk to on its own.
///
/// Matched as substrings because each comes with the socket path attached: a
/// socket that was never there, and a server that has since gone.
const NO_SERVER: &[&str] = &["no server running", "server exited unexpectedly"];

#[cfg(test)]
mod tests {
    use super::{PANE_FORMAT, no_server_running, panes, panes_body};
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
        // And the fixture really is that format's output: seven fields on
        // every line. A line count would pass over a fixture whose columns had
        // drifted, which is the only way this could quietly stop being a test
        // of the format at all.
        assert_eq!(MIXED.lines().count(), 5);
        for line in MIXED.lines() {
            assert_eq!(
                line.split('\t').count(),
                PANE_FORMAT.matches('\t').count() + 1,
                "one field per format specifier in {line:?}"
            );
        }
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
        // A socket that is there and will not open for us is a machine with
        // panes we cannot see. Calling that "no panes" is the panel saying
        // nothing needs you when it does not know.
        assert!(!no_server_running(
            "error connecting to /tmp/tmux-0/default (Permission denied)"
        ));
        // A stale socket nobody is listening on is no server, though.
        assert!(no_server_running(
            "error connecting to /tmp/tmux-1000/default (Connection refused)"
        ));
        // And the reason only means that *about a socket*. Read on its own it
        // appears in failures that have nothing to do with one, which would be
        // the same lie from the other side.
        assert!(!no_server_running(
            "tmux: open terminal failed: No such file or directory"
        ));
    }

    // AYEAYE-43 — a line with *more* than seven fields is dropped too. A window
    // renamed with a tab in it produces one, and reading the first seven fields
    // out of it shifts every field past the tab: the command is read out of the
    // window name and the flags are read out of the command. The pane would
    // come back as a different pane, which is worse than not coming back.
    // `bin/ayeaye`'s `len(parts) != 7` drops it, and so must this.
    #[test]
    fn a_line_with_an_extra_field_is_dropped_like_a_short_one() {
        // A window named "two\tnames", on an inactive pane: eight fields, and
        // the seventh is not the dead flag it would be mistaken for.
        let text = "\
%0\twork\t0\ttwo\tnames\tsh\t0\t0
%2\twork\t1\tcook\tsh\t0\t0
";
        let ids: Vec<String> = panes(&host(), text)
            .iter()
            .map(|pane| pane.id.qualified())
            .collect();
        assert_eq!(ids, ["desktop/%2"]);
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

    // AYEAYE-43 — the body the panel reads: the host it came from, and a card
    // per pane carrying the six fields `share/app.html` renders. Written out in
    // full rather than field by field, because the shape is the contract.
    #[test]
    fn the_body_carries_the_host_and_a_card_per_pane() {
        let body = panes_body(&host(), &panes(&host(), MIXED), None);
        assert_eq!(
            body,
            concat!(
                r#"{"host":"desktop","panes":["#,
                r#"{"id":"desktop/%0","session":"work","window":"0","name":"editor","cmd":"sh","active":true},"#,
                r#"{"id":"desktop/%1","session":"work","window":"0","name":"editor","cmd":"sh","active":false},"#,
                r#"{"id":"desktop/%2","session":"work","window":"1","name":"cook","cmd":"sh","active":true}"#,
                r#"]}"#,
            )
        );
        assert_eq!(
            panes_body(&host(), &[], None),
            r#"{"host":"desktop","panes":[]}"#
        );
    }

    // AYEAYE-43 — a window name is whatever somebody typed. A quote in it must
    // not end the string that carries it, or one badly-named window empties the
    // panel for every pane — the same failure the spec names about decoding.
    #[test]
    fn a_window_name_cannot_break_the_body() {
        let awkward = "%0\twork\t0\tsay \"hi\"\tsh\t1\t0\n";
        assert_eq!(
            panes_body(&host(), &panes(&host(), awkward), None),
            concat!(
                r#"{"host":"desktop","panes":[{"id":"desktop/%0","session":"work",""#,
                r#"window":"0","name":"say \"hi\"","cmd":"sh","active":true}]}"#,
            )
        );
    }

    // AYEAYE-43 — tmux that could not be asked is said out loud. An empty list
    // with no reason reads as "nothing needs you", which is the one thing this
    // app must never say when it does not know.
    #[test]
    fn a_failure_is_stated_rather_than_served_as_an_empty_list() {
        assert_eq!(
            panes_body(&host(), &[], Some("tmux: No such file or directory")),
            r#"{"host":"desktop","panes":[],"error":"tmux: No such file or directory"}"#
        );
    }
}
