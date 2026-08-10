//! The server, observed the way a browser observes it: over a socket.
//!
//! Every test here binds port 0, so the suite never collides with the Python
//! daemon or with itself, and speaks raw HTTP/1.1 with `Connection: close` so
//! the body ends at EOF and no HTTP client crate is needed to read it. That
//! keeps the seam the highest one that can see the behaviour — a route table
//! that resolves correctly and a server that never binds would pass a unit
//! test and fail a phone.

use std::collections::HashMap;
use std::sync::Arc;

use ayeaye::config::Settings;
use ayeaye_core::http::hosts::AllowedHosts;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// The token every test logs in with.
const TOKEN: &str = "test-token-not-a-real-secret";

/// A response, split into the parts a test wants to assert on.
struct Answer {
    status: u16,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

impl Answer {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(&name.to_lowercase()).map(String::as_str)
    }

    fn body_text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

/// A running server on a port the kernel picked.
struct Server {
    port: u16,
}

impl Server {
    /// Start one with the given settings, bound to port 0.
    async fn start(settings: Settings) -> Server {
        // Deliberately the server's own bind, not the test's: a harness that
        // binds for itself proves only that the harness can bind.
        let listener = ayeaye::server::listen(&settings)
            .await
            .expect("the settings should describe a bindable address");
        let port = listener.local_addr().expect("a bound address").port();
        let settings = Arc::new(settings);
        tokio::spawn(async move {
            let _ = ayeaye::server::serve(listener, settings).await;
        });
        Server { port }
    }

    /// Start one with the defaults every test wants: loopback, this port,
    /// the shared test token.
    async fn started() -> Server {
        Server::start(settings_on_port(0)).await
    }

    /// Send a raw request and read the whole answer.
    ///
    /// `Connection: close` means the server hangs up when it is done, so
    /// "read to EOF" is the whole body and no `Content-Length` parsing is
    /// needed to know where to stop.
    async fn request(&self, method: &str, path: &str, headers: &[(&str, &str)]) -> Answer {
        let mut stream = TcpStream::connect(("127.0.0.1", self.port))
            .await
            .expect("the server should be listening");

        let mut request = format!("{method} {path} HTTP/1.1\r\n");
        let mut has_host = false;
        for (name, value) in headers {
            if name.eq_ignore_ascii_case("host") {
                has_host = true;
            }
            request.push_str(&format!("{name}: {value}\r\n"));
        }
        if !has_host {
            request.push_str(&format!("Host: 127.0.0.1:{}\r\n", self.port));
        }
        request.push_str("Connection: close\r\n\r\n");

        stream
            .write_all(request.as_bytes())
            .await
            .expect("the request should be writable");
        let mut raw = Vec::new();
        stream
            .read_to_end(&mut raw)
            .await
            .expect("the response should be readable");
        parse(&raw)
    }

    async fn get(&self, path: &str) -> Answer {
        self.request("GET", path, &[]).await
    }
}

fn settings_on_port(port: u16) -> Settings {
    Settings {
        bind: "127.0.0.1".to_string(),
        port,
        allowed_hosts: AllowedHosts::new("127.0.0.1", port, ""),
        token: TOKEN.to_string(),
    }
}

fn parse(raw: &[u8]) -> Answer {
    let split = raw
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .expect("a response should have a header block");
    let head = String::from_utf8_lossy(&raw[..split]).into_owned();
    let body = raw[split + 4..].to_vec();

    let mut lines = head.lines();
    let status_line = lines.next().expect("a status line");
    let status = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .unwrap_or_else(|| panic!("no status code in {status_line:?}"));

    let mut headers = HashMap::new();
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            headers.insert(name.trim().to_lowercase(), value.trim().to_string());
        }
    }
    Answer {
        status,
        headers,
        body,
    }
}

// AYEAYE-42 — "a browser can load the panel from the Rust binary". Not the
// route table resolving correctly: an actual socket, an actual GET, and the
// bytes of app.html coming back.
#[tokio::test]
async fn a_browser_gets_the_panel_from_a_real_socket() {
    let server = Server::started().await;
    let answer = server.get("/").await;

    assert_eq!(answer.status, 200);
    assert_eq!(
        answer.header("content-type"),
        Some("text/html; charset=utf-8")
    );
    assert_eq!(
        answer.body,
        ayeaye::assets::bytes("app.html").expect("app.html is embedded"),
        "the panel served is not the panel compiled in"
    );
    assert!(
        answer.body_text().contains("<!doctype html")
            || answer.body_text().contains("<!DOCTYPE html"),
        "the body does not look like the panel"
    );
}

// AYEAYE-42 — "the port is configurable so both daemons can run side by side".
// The unit test proves which port wins the argument; this proves the server
// actually listens on the one that won, which is the half a browser cares
// about.
#[tokio::test]
async fn the_server_listens_on_the_port_it_was_configured_with() {
    // Borrow a port from the kernel and hand it back, so the number below is
    // one that was free a moment ago rather than one that was hoped for.
    let scout = TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("port 0 should always bind");
    let chosen = scout.local_addr().expect("a bound address").port();
    drop(scout);

    let server = Server::start(settings_on_port(chosen)).await;
    assert_eq!(server.port, chosen, "the server moved to a different port");
    assert_eq!(server.get("/").await.status, 200);
}

// AYEAYE-42 — the board, the manifest and the icons are what turn the panel
// from a page into an installed app; each has to arrive with the type the
// daemon sends, and with the bytes that were compiled in.
#[tokio::test]
async fn every_page_manifest_and_icon_is_served_with_its_type() {
    let server = Server::started().await;
    let expected = [
        ("/board", "board.html", "text/html; charset=utf-8"),
        ("/board.html", "board.html", "text/html; charset=utf-8"),
        ("/index.html", "app.html", "text/html; charset=utf-8"),
        ("/message", "app.html", "text/html; charset=utf-8"),
        (
            "/manifest.webmanifest",
            "manifest.webmanifest",
            "application/manifest+json",
        ),
        ("/favicon.ico", "favicon.ico", "image/x-icon"),
        ("/icon-192.png", "icon-192.png", "image/png"),
        (
            "/icon-maskable-512.png",
            "icon-maskable-512.png",
            "image/png",
        ),
    ];

    for (path, file, content_type) in expected {
        let answer = server.get(path).await;
        assert_eq!(answer.status, 200, "status for {path}");
        assert_eq!(
            answer.header("content-type"),
            Some(content_type),
            "type for {path}"
        );
        assert_eq!(
            answer.body,
            ayeaye::assets::bytes(file).expect("the file is embedded"),
            "bytes for {path}"
        );
        // Every response the daemon sends is uncacheable, and these describe
        // live state as much as the API does.
        assert_eq!(
            answer.header("cache-control"),
            Some("no-store"),
            "cache for {path}"
        );
    }
}

// AYEAYE-42 — the Host gate applies to everything, pages included. A page on
// an attacker's origin that resolves their name to this address still sends
// their Host, and that is what has to be refused — before any route is even
// looked up.
#[tokio::test]
async fn a_foreign_host_is_refused_even_for_the_panel() {
    let server = Server::started().await;
    for path in ["/", "/board", "/favicon.ico", "/api/panes"] {
        let answer = server
            .request("GET", path, &[("Host", "evil.example")])
            .await;
        assert_eq!(answer.status, 403, "status for {path}");
        assert_eq!(
            answer.body_text(),
            r#"{"error":"forbidden"}"#,
            "body for {path}"
        );
    }
    // And the same request with the right Host is not refused, or the test
    // above would pass on a server that refused everything.
    assert_eq!(server.get("/").await.status, 200);
}

// AYEAYE-42 — the token gate, observed as the difference between 401 and 404:
// an /api/ path is refused before anyone learns whether it exists, and is
// answered once a token arrives, by header or by the login cookie.
#[tokio::test]
async fn the_api_needs_a_token_by_header_or_by_cookie() {
    let server = Server::started().await;

    let anonymous = server.get("/api/panes").await;
    assert_eq!(anonymous.status, 401);
    assert_eq!(anonymous.body_text(), r#"{"error":"unauthorized"}"#);

    let wrong = server
        .request("GET", "/api/panes", &[("X-Voice-Token", "not-the-token")])
        .await;
    assert_eq!(wrong.status, 401, "a wrong token is not a token");

    // 404, not 401: the gate let it through and nothing answers there yet.
    let by_header = server
        .request("GET", "/api/panes", &[("X-Voice-Token", TOKEN)])
        .await;
    assert_eq!(by_header.status, 404);
    assert_eq!(by_header.body_text(), r#"{"error":"not found"}"#);

    let by_cookie = server
        .request(
            "GET",
            "/api/panes",
            &[("Cookie", &format!("theme=dark; voice_token={TOKEN}"))],
        )
        .await;
    assert_eq!(
        by_cookie.status, 404,
        "the login cookie should authenticate"
    );
}

// AYEAYE-42 — anything that could write is gated whatever it names, which is
// what the daemon's do_POST does before it looks at the path.
#[tokio::test]
async fn a_post_needs_a_token_even_for_a_page() {
    let server = Server::started().await;
    for path in ["/", "/board", "/nope"] {
        let answer = server.request("POST", path, &[]).await;
        assert_eq!(answer.status, 401, "POST {path} must need a token");
    }
}

// AYEAYE-42 — the handshake a phone does once: a token in the query comes back
// as the cookie, and the browser is sent on to where it was going.
#[tokio::test]
async fn the_login_handshake_sets_the_cookie_and_redirects() {
    let server = Server::started().await;

    let answer = server
        .get(&format!("/login?token={TOKEN}&next=/board"))
        .await;
    assert_eq!(answer.status, 303);
    assert_eq!(answer.header("location"), Some("/board"));
    assert_eq!(
        answer.header("set-cookie"),
        Some(
            format!("voice_token={TOKEN}; Path=/; Max-Age=31536000; HttpOnly; SameSite=Strict")
                .as_str()
        )
    );

    // The app's own URL is the same handshake, so one bookmarked link works.
    let at_root = server.get(&format!("/?token={TOKEN}")).await;
    assert_eq!(at_root.status, 303);
    assert_eq!(at_root.header("location"), Some("/"));

    let wrong = server.get("/login?token=not-the-token").await;
    assert_eq!(wrong.status, 401);
    assert_eq!(
        wrong.header("set-cookie"),
        None,
        "a refusal must set nothing"
    );
}

// AYEAYE-42 — `next` is attacker-supplied, and it arrives percent-encoded. A
// check that looked at the raw text would pass `%2F%2Fevil.example` straight
// through to a browser that reads it as a host.
#[tokio::test]
async fn a_hostile_next_cannot_bounce_the_browser_off_this_origin() {
    let server = Server::started().await;
    for hostile in [
        "//evil.example",
        "%2F%2Fevil.example",
        "https://evil.example",
        "/%5Cevil.example",
    ] {
        let answer = server
            .get(&format!("/login?token={TOKEN}&next={hostile}"))
            .await;
        assert_eq!(answer.status, 303, "status for {hostile}");
        assert_eq!(
            answer.header("location"),
            Some("/"),
            "{hostile} must not survive as a redirect target"
        );
    }
}
