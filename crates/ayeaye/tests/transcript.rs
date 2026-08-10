//! The transcript endpoints, at the seam the server drives them.
//!
//! The mapping is unit-tested in the core and the tail against files of its
//! own in `src/transcript.rs`; what belongs here is the endpoint contract —
//! which path the module owns, and what it answers when the chain below it
//! has nothing. Both questions must be answerable on a machine with no tmux
//! and no agents, so both point every dependency at nowhere.

// The harness is shared with the other test binaries and each of them uses a
// different part of it; the `#![allow(dead_code)]` for that lives inside
// `common` itself, so it is stated once rather than once per binary.
mod common;

use ayeaye::session::Agents;

/// Settings pointed at nothing, for questions that are about paths.
fn settings(what: &str) -> ayeaye::config::Settings {
    let mut settings = ayeaye::config::Settings::resolve(
        &[],
        |_| None,
        "test-token-not-a-real-secret".to_string(),
        Some("desktop".to_string()),
        ayeaye::cliban::Cliban::new("/nonexistent/cliban".to_string()),
        std::sync::Arc::new(ayeaye::dictate::Voice::new(
            std::path::PathBuf::from("/nonexistent/store"),
            ayeaye_core::model::settings::ModelSettings::resolve(|_| None, "")
                .expect("the defaults resolve"),
            ayeaye_core::cleanup::Policy::default(),
            "ayeaye-46-no-such-converter".to_string(),
        )),
    )
    .expect("settings a test can drive");
    settings.tmux = common::nowhere(what);
    settings.agents = Agents::under("/nonexistent/home");
    settings
}

// AYEAYE-46 — nothing else in `/api/` belongs to this module, and saying so
// is what keeps the server's 404 reachable for a path nobody has written yet.
// `/api/stream` in particular is not this chain's: it answers through its own
// route, because its body never ends.
#[tokio::test]
async fn the_module_owns_one_path_and_no_other() {
    let settings = settings("transcript-paths");
    for path in ["/api/messages", "/api/message/", "/api/stream", "/api/session", "/", ""] {
        assert!(
            ayeaye::transcript::answer(&settings, path, None)
                .await
                .is_none(),
            "{path} is not this module's"
        );
    }
}

// AYEAYE-46 — every missing link is the daemon's one 404: no pane named, a
// pane nobody offered, a session that cannot be resolved. The panel renders
// the error text, and which link broke changes nothing it does.
#[tokio::test]
async fn a_message_nobody_can_have_is_the_daemons_404() {
    let settings = settings("transcript-nobody");
    for query in [None, Some("pane=desktop/%25999"), Some("ref=0:0"), Some("pane=desktop/%25999&ref=0:0")] {
        let (status, body) = ayeaye::transcript::answer(&settings, "/api/message", query)
            .await
            .expect("the message endpoint owns this path");
        assert_eq!(status, 404, "for {query:?}");
        assert_eq!(body, r#"{"error":"message not found"}"#, "for {query:?}");
    }
}
