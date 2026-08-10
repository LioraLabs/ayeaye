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

// AYEAYE-46 — the stream endpoint over a socket: gated like the rest of the
// API, and a pane with no session behind it — including no pane at all — is
// the daemon's 404 `{"kind":null}`, so the panel can tell "not an agent" from
// a broken server without parsing an error string. Over the wire rather than
// through the handler, because the 401 lives above the route and only a real
// request passes through it.
#[tokio::test]
async fn the_stream_is_gated_and_a_sessionless_pane_is_the_daemons_404() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let mut settings = settings("transcript-stream");
    settings.port = 0;
    let listener = ayeaye::server::listen(&settings)
        .await
        .expect("the settings should describe a bindable address");
    let port = listener.local_addr().expect("a bound address").port();
    tokio::spawn(ayeaye::server::serve(listener, std::sync::Arc::new(settings)));

    // `Connection: close` so the whole answer is "read to EOF" — which is
    // also the proof these are complete responses, not streams.
    let ask = |headers: &'static str| async move {
        let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .expect("the server should be listening");
        let request = format!(
            "GET /api/stream?pane=desktop/%25999 HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n{headers}Connection: close\r\n\r\n"
        );
        stream
            .write_all(request.as_bytes())
            .await
            .expect("the request should be writable");
        let mut raw = Vec::new();
        stream
            .read_to_end(&mut raw)
            .await
            .expect("the response should be readable");
        String::from_utf8_lossy(&raw).into_owned()
    };

    let denied = ask("").await;
    assert!(
        denied.starts_with("HTTP/1.1 401"),
        "without a token the stream must be refused: {denied}"
    );

    let asked = ask("X-Voice-Token: test-token-not-a-real-secret\r\n").await;
    assert!(
        asked.starts_with("HTTP/1.1 404"),
        "a pane nobody can resolve is a 404: {asked}"
    );
    assert!(
        asked.ends_with(r#"{"kind":null}"#),
        "and the body is the daemon's, not an error string: {asked}"
    );
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
