//! The tmux layer, against a real tmux.
//!
//! Every test here asks a tmux server of the suite's own — see `common` for why
//! the default socket is off limits.

mod common;

use std::time::Duration;

use ayeaye::tmux::{Tmux, Trouble};
use ayeaye_core::peer::HostName;
use ayeaye_core::prompt;
use ayeaye_core::tmux::Pane;

use common::Private;

fn host() -> HostName {
    HostName::new("desktop").expect("a host name")
}

// AYEAYE-43 — the panes really come from tmux: the format is one tmux accepts,
// the fields land where the parser expects them, and the ids come back
// qualified. A unit test over captured text cannot say any of that.
#[tokio::test]
async fn the_panes_of_a_real_tmux_come_back_qualified() {
    let Some(server) = Private::named("live") else {
        eprintln!("skipped: no tmux on this machine");
        return;
    };
    server.tmux(&["new-window", "-t", "work", "-n", "cook", "-d", "/bin/sh"]);
    // A floating scratch session, which the panel must not offer as a target.
    server.tmux(&["new-session", "-d", "-s", "_scratch", "/bin/sh"]);

    let panes = server
        .layer()
        .panes(&host())
        .await
        .expect("a running tmux answers");

    let sessions: Vec<&str> = panes.iter().map(|pane| pane.session.as_str()).collect();
    assert_eq!(
        sessions,
        ["work", "work"],
        "the _scratch session is not a target"
    );
    for pane in &panes {
        let qualified = pane.id.qualified();
        assert!(
            qualified.starts_with("desktop/%"),
            "{qualified} should be this machine's, and a tmux pane id"
        );
    }
    let names: Vec<&str> = panes.iter().map(|pane| pane.name.as_str()).collect();
    assert_eq!(names, ["editor", "cook"]);
    assert!(panes.iter().all(|pane| pane.active), "one pane per window");
}

// AYEAYE-43 — a machine with no tmux server has no panes, and that is an
// answer. Most machines are in that state most of the time, and a failure there
// would put an error in the panel on every one of them.
#[tokio::test]
async fn a_socket_with_no_server_has_no_panes_and_no_complaint() {
    if !common::have_tmux() {
        eprintln!("skipped: no tmux on this machine");
        return;
    }
    assert_eq!(
        common::nowhere("nobody").panes(&host()).await,
        Ok(Vec::new())
    );
}

// AYEAYE-43 — and a tmux that could not be run at all is not silence. "I could
// not look" and "there is nothing to see" arriving as the same answer is the
// failure the pane list exists to avoid.
#[tokio::test]
async fn a_tmux_that_is_not_there_is_reported_rather_than_swallowed() {
    let missing = Tmux::spelled(&["ayeaye-43-no-such-tmux"], Duration::from_secs(10));
    let Err(trouble) = missing.panes(&host()).await else {
        panic!("a tmux that does not exist cannot have answered");
    };
    assert!(matches!(trouble, Trouble::NotRun(_)), "{trouble:?}");
    assert!(
        trouble.to_string().contains("ayeaye-43-no-such-tmux"),
        "the thing that could not be run is named: {trouble}"
    );
}

/// A pane of the private server's, running a program of our choosing.
///
/// **The safety check every test below leans on.** It asks the layer for the
/// pane list and refuses to go on unless that list is exactly the private
/// server's — one window per name we created, and nothing else. A `Tmux` whose
/// socket redirect had not taken effect would answer with the panes of whoever
/// is running the suite, and the very next line of these tests sends keys. So
/// the list is checked before a key is ever sent, and the pane that is sent to
/// is one taken *out of* that list rather than one named by hand.
async fn only_pane_of(server: &Private, window: &str, program: &str) -> Pane {
    server.tmux(&["new-window", "-t", "work", "-n", window, "-d", program]);
    let panes = server
        .layer()
        .panes(&host())
        .await
        .expect("the private server answers");

    let names: Vec<&str> = panes.iter().map(|pane| pane.name.as_str()).collect();
    assert!(
        names.contains(&window)
            && names
                .iter()
                .all(|name| *name == window || *name == "editor"),
        "this is not the suite's own tmux server: {names:?}"
    );
    assert!(
        panes.iter().all(|pane| pane.session == "work"),
        "this is not the suite's own tmux server: {panes:?}"
    );
    panes
        .into_iter()
        .find(|pane| pane.name == window)
        .expect("the window that was just created")
}

/// What the pane says, once it says it — or what it last said, at the deadline.
///
/// A keystroke and the redraw that follows it are two events, and the second one
/// is the terminal's rather than ours. Polling for the answer keeps the test
/// about what arrived instead of about how fast this machine is.
async fn settles(tmux: &Tmux, pane: &Pane, until: impl Fn(&str) -> bool) -> String {
    let mut screen = String::new();
    for _ in 0..100 {
        screen = tmux.capture(pane).await.expect("a pane answers");
        if until(&screen) {
            return screen;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    screen
}

/// A `Tmux` that submits immediately: the gap is for a TUI, and `cat` has none.
fn prompt_layer(server: &Private) -> Tmux {
    server.layer().submitting_after(Duration::from_millis(1))
}

// AYEAYE-48 — capture-pane really comes back as the screen, and the parser
// really reads a prompt off what a real terminal drew. A unit test over a
// captured file cannot say the capture and the parse are wired together.
#[tokio::test]
async fn a_prompt_drawn_in_a_real_pane_is_read_back_off_it() {
    let Some(server) = Private::named("capture") else {
        eprintln!("skipped: no tmux on this machine");
        return;
    };
    let pane = only_pane_of(&server, "asking", "cat").await;
    let layer = server.layer();

    // Drawn by the pane's own program rather than typed by us, so what is parsed
    // is what a terminal put on a screen.
    layer
        .type_text(&pane, prompt::typed("Pick one? ").expect("typeable"))
        .await
        .expect("typing works");
    server.tmux(&[
        "send-keys",
        "-t",
        pane.id.pane(),
        "-l",
        "--",
        "\n 1. First\n 2. Second\n Enter to select . Esc to cancel\n",
    ]);

    let screen = settles(&layer, &pane, |said| said.contains("Esc to cancel")).await;
    let prompt = prompt::read(&screen).unwrap_or_else(|| panic!("no prompt on {screen:?}"));
    assert_eq!(prompt.question, "Pick one?");
    assert_eq!(
        prompt
            .options
            .iter()
            .map(|choice| choice.label.as_str())
            .collect::<Vec<_>>(),
        ["First", "Second"]
    );
}

// AYEAYE-48 — a key from the allow-list arrives as a key. `cat` echoes what it
// is typed and repeats a line when Enter ends it, so a second copy of the text
// is the pane saying the Enter was a submit and not a character.
#[tokio::test]
async fn a_named_key_arrives_as_a_key_and_a_number_arrives_as_a_number() {
    let Some(server) = Private::named("press") else {
        eprintln!("skipped: no tmux on this machine");
        return;
    };
    let pane = only_pane_of(&server, "pressing", "cat").await;
    let layer = server.layer();

    layer
        .press(&pane, prompt::press("3").expect("3 is an option"))
        .await
        .expect("a number can be pressed");
    let typed = settles(&layer, &pane, |said| said.contains('3')).await;
    assert!(typed.contains('3'), "the number never arrived: {typed:?}");
    // And it was not submitted: `cat` has not echoed a second line, which it
    // does the moment a newline ends the one it is reading.
    assert_eq!(typed.matches('3').count(), 1, "{typed:?}");

    layer
        .press(&pane, prompt::press("enter").expect("enter is a key"))
        .await
        .expect("enter can be pressed");
    let submitted = settles(&layer, &pane, |said| said.matches('3').count() >= 2).await;
    assert_eq!(
        submitted.matches('3').count(),
        2,
        "Enter did not end the line: {submitted:?}"
    );
}

// AYEAYE-48 — text is typed and nothing else happens. "Typing sends text
// without submitting it" is an acceptance criterion, and the pane is the only
// witness worth having: `cat` repeats a line the moment a newline ends it, so
// one copy on the screen is the proof that nothing was submitted.
#[tokio::test]
async fn typing_sends_the_text_and_does_not_submit_it() {
    let Some(server) = Private::named("typing") else {
        eprintln!("skipped: no tmux on this machine");
        return;
    };
    let pane = only_pane_of(&server, "typed", "cat").await;
    let layer = prompt_layer(&server);

    layer
        .type_text(&pane, prompt::typed("ship it").expect("typeable"))
        .await
        .expect("typing works");
    let typed = settles(&layer, &pane, |said| said.contains("ship it")).await;
    assert_eq!(
        typed.matches("ship it").count(),
        1,
        "typing submitted the text: {typed:?}"
    );

    // And submitting is the separate act. Only now does `cat` see a line.
    layer.submit(&pane).await.expect("submitting works");
    let submitted = settles(&layer, &pane, |said| said.matches("ship it").count() >= 2).await;
    assert_eq!(
        submitted.matches("ship it").count(),
        2,
        "the Enter never arrived: {submitted:?}"
    );
}

// AYEAYE-48 — the text arrives as itself. `-l` means nothing in it is read as
// the name of a key, and `--` means text beginning with a dash is text rather
// than an option to `send-keys`. Nothing is quoted on the way, because nothing
// crosses a shell — and `cat` is the pane's program precisely so that a shell
// cannot be what makes this pass.
#[tokio::test]
async fn awkward_text_arrives_verbatim() {
    let Some(server) = Private::named("awkward") else {
        eprintln!("skipped: no tmux on this machine");
        return;
    };
    let pane = only_pane_of(&server, "verbatim", "cat").await;
    let layer = server.layer();

    for awkward in [
        "--force",
        "-t %0",
        "$(id)",
        "`id`",
        "say \"hi\"",
        "a'b",
        "C-c",
        "Enter",
        "; kill-server",
        "cafe 100%",
    ] {
        layer
            .type_text(&pane, prompt::typed(awkward).expect("typeable"))
            .await
            .expect("typing works");
        let screen = settles(&layer, &pane, |said| said.contains(awkward)).await;
        assert!(
            screen.contains(awkward),
            "{awkward:?} did not arrive verbatim: {screen:?}"
        );
        // Wipe the line before the next one, so a later assertion cannot pass on
        // what an earlier one left behind.
        layer
            .press(&pane, prompt::press("esc").expect("esc is a key"))
            .await
            .expect("esc can be pressed");
        server.tmux(&["send-keys", "-t", pane.id.pane(), "C-u"]);
        server.tmux(&["clear-history", "-t", pane.id.pane()]);
        settles(&layer, &pane, |said| !said.contains(awkward)).await;
    }

    // The server is still there afterwards: `; kill-server` was typed at a pane
    // and not run.
    assert!(
        !layer
            .panes(&host())
            .await
            .expect("the server survived")
            .is_empty()
    );
}

// AYEAYE-45 — the live-session list really comes from tmux: one name to a
// line, and a session name with a space in it stays one session. A unit test
// over text somebody typed cannot say that tmux prints it that way.
#[tokio::test]
async fn the_live_sessions_of_a_real_tmux_come_back_one_to_a_line() {
    let Some(server) = Private::named("sessions") else {
        eprintln!("skipped: no tmux on this machine");
        return;
    };
    server.tmux(&["new-session", "-d", "-s", "my work", "/bin/sh"]);

    let mut sessions = server
        .layer()
        .sessions()
        .await
        .expect("a running tmux answers");
    sessions.sort();
    assert_eq!(
        sessions,
        ["my work", "work"],
        "a name with a space in it is one session, not two"
    );
}

// AYEAYE-45 — a machine with no tmux server has no sessions, and that is an
// answer rather than a failure, for the same reason it is for panes.
#[tokio::test]
async fn a_socket_with_no_server_has_no_sessions_and_no_complaint() {
    if !common::have_tmux() {
        eprintln!("skipped: no tmux on this machine");
        return;
    }
    assert_eq!(
        common::nowhere("no-sessions").sessions().await,
        Ok(Vec::new())
    );
}
