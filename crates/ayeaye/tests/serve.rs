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
        let raw: Vec<(&str, &[u8])> = headers
            .iter()
            .map(|(name, value)| (*name, value.as_bytes()))
            .collect();
        self.request_raw(method, path, &raw).await
    }

    /// The same, with header *values* as bytes rather than text.
    ///
    /// A header value is bytes on the wire and only sometimes text, and the
    /// difference is load-bearing for the origin gate: a value a Rust `String`
    /// cannot hold is exactly the one that must not read as "no header sent".
    /// A test that could only send `&str` could never send that request.
    async fn request_raw(&self, method: &str, path: &str, headers: &[(&str, &[u8])]) -> Answer {
        let mut stream = TcpStream::connect(("127.0.0.1", self.port))
            .await
            .expect("the server should be listening");

        let mut request = format!("{method} {path} HTTP/1.1\r\n").into_bytes();
        let mut has_host = false;
        for (name, value) in headers {
            if name.eq_ignore_ascii_case("host") {
                has_host = true;
            }
            request.extend_from_slice(name.as_bytes());
            request.extend_from_slice(b": ");
            request.extend_from_slice(value);
            request.extend_from_slice(b"\r\n");
        }
        if !has_host {
            request.extend_from_slice(format!("Host: 127.0.0.1:{}\r\n", self.port).as_bytes());
        }
        request.extend_from_slice(b"Connection: close\r\n\r\n");

        stream
            .write_all(&request)
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

// AYEAYE-69 — the CSRF gate, over the wire. A browser labels a request from
// somebody else's page `Sec-Fetch-Site: cross-site`, and that is refused at
// every path and by every method that is not a read — before the token is
// looked at, so a stolen or guessed token buys nothing, and with the same 403
// the Host gate gives so a refused write cannot be told apart from a refused
// host.
#[tokio::test]
async fn a_cross_site_write_is_refused_before_the_token_is_looked_at() {
    let server = Server::started().await;
    for path in ["/", "/board", "/api/answer", "/nope"] {
        for headers in [
            vec![("Sec-Fetch-Site", "cross-site")],
            // With the right token, which must not buy past this gate.
            vec![("Sec-Fetch-Site", "cross-site"), ("X-Voice-Token", TOKEN)],
        ] {
            let answer = server.request("POST", path, &headers).await;
            assert_eq!(answer.status, 403, "POST {path} with {headers:?}");
            assert_eq!(answer.body_text(), r#"{"error":"forbidden"}"#);
        }
    }
    // Not only POST: the router sends every method to the one handler, so the
    // gate covers a verb nobody has mounted an endpoint on yet.
    for method in ["PUT", "PATCH", "DELETE", "OPTIONS"] {
        let answer = server
            .request(method, "/api/answer", &[("Sec-Fetch-Site", "cross-site")])
            .await;
        assert_eq!(answer.status, 403, "{method} /api/answer");
    }
}

// AYEAYE-69 — the other half of the label: an `Origin` naming a host this
// daemon does not answer to, and the opaque `null` a sandboxed frame sends.
// The same request from this server's own origin has to reach the token gate,
// or the test above would pass on a server that refused every write.
#[tokio::test]
async fn a_write_from_a_foreign_origin_is_refused_and_one_from_here_is_not() {
    let server = Server::started().await;
    let ours = format!("http://127.0.0.1:{}", server.port);

    for origin in [
        "https://evil.example",
        "null",
        "http://127.0.0.1.evil.example",
    ] {
        let answer = server
            .request("POST", "/api/answer", &[("Origin", origin)])
            .await;
        assert_eq!(answer.status, 403, "POST from {origin}");
        assert_eq!(answer.body_text(), r#"{"error":"forbidden"}"#);
    }

    // 401, not 403: this origin passed the CSRF gate and was stopped by the
    // token gate behind it, which is the order the daemon refuses in.
    let anonymous = server
        .request("POST", "/api/answer", &[("Origin", ours.as_str())])
        .await;
    assert_eq!(anonymous.status, 401);
    assert_eq!(anonymous.body_text(), r#"{"error":"unauthorized"}"#);

    // And with the token it is a 404, because nothing writes here yet: both
    // gates let it through.
    let authorized = server
        .request(
            "POST",
            "/api/answer",
            &[("Origin", ours.as_str()), ("X-Voice-Token", TOKEN)],
        )
        .await;
    assert_eq!(authorized.status, 404);
}

// AYEAYE-69 — a read is exempt, which is not a detail: a page on any origin
// may link to this one, and a link preview or a PWA install check fetches an
// icon with `Sec-Fetch-Site: cross-site` on it. Gating reads would break that
// for no gain, since a read changes nothing.
#[tokio::test]
async fn a_cross_site_read_is_still_answered() {
    let server = Server::started().await;
    let labelled: &[(&str, &str)] = &[
        ("Sec-Fetch-Site", "cross-site"),
        ("Origin", "https://evil.example"),
    ];

    for path in ["/", "/favicon.ico", "/manifest.webmanifest"] {
        assert_eq!(
            server.request("GET", path, labelled).await.status,
            200,
            "a cross-site GET of {path} is a read"
        );
    }
    assert_eq!(
        server
            .request("HEAD", "/favicon.ico", labelled)
            .await
            .status,
        200
    );
}

// AYEAYE-69 — an `Origin` whose bytes are not text, sent as bytes. `to_str()`
// cannot render these, and `to_str().ok()` would call them `None` — the same
// answer as a client that sent no `Origin`, which is allowed. Nobody is kept
// out by refusing them who could not simply omit the header instead; what this
// pins is that the server can still tell the two apart, which is the property
// the next caller of this gate will need.
//
// It is also the only test here that can send a request a Rust `String` cannot
// hold, which is why the harness grew `request_raw`.
#[tokio::test]
async fn an_origin_whose_bytes_are_not_text_is_refused() {
    let server = Server::started().await;
    // 0xFF is legal in a header value (RFC 9110 obs-text) and is not UTF-8.
    let hostile: &[u8] = b"https://\xff\xfe.example";

    let write = server
        .request_raw("POST", "/api/answer", &[("Origin", hostile)])
        .await;
    assert_eq!(
        write.status, 403,
        "an Origin the server cannot read must not read as no Origin at all"
    );
    assert_eq!(write.body_text(), r#"{"error":"forbidden"}"#);

    // A read is still a read, and the server is still answering afterwards.
    let read = server
        .request_raw("GET", "/favicon.ico", &[("Origin", hostile)])
        .await;
    assert_eq!(read.status, 200);
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

// AYEAYE-42 — regression, found at the final gate. `next` is percent-decoded
// before it is used, so a CRLF survived into the `Location:` header: the
// response could not be built, the worker panicked, and the client got a
// dropped connection instead of an answer. An authenticated caller could do it
// to itself, which is not a breach, but a network-reachable panic is not an
// answer either.
#[tokio::test]
async fn control_characters_in_next_do_not_kill_the_response() {
    let server = Server::started().await;
    for hostile in [
        "%2Fa%0d%0aX-Injected:%20yes",
        "%2Fa%0aX-Injected:%20yes",
        "%2Fa%00b",
    ] {
        let answer = server
            .get(&format!("/login?token={TOKEN}&next={hostile}"))
            .await;
        assert_eq!(answer.status, 303, "status for {hostile}");
        assert_eq!(
            answer.header("location"),
            Some("/"),
            "{hostile} must not reach the Location header"
        );
        assert_eq!(
            answer.header("x-injected"),
            None,
            "{hostile} spliced a header into the response"
        );
    }
    // The server is still answering afterwards, which a panicked worker that
    // took the runtime down with it would not be.
    assert_eq!(server.get("/").await.status, 200);
}

// AYEAYE-42 — HEAD is the ticket's one deliberate widening over the daemon
// (which 501s it). The pure test says the gate is open; this says the server
// actually answers, with headers and no body.
#[tokio::test]
async fn head_is_answered_where_get_is_and_gated_where_get_is() {
    let server = Server::started().await;

    let open = server.request("HEAD", "/favicon.ico", &[]).await;
    assert_eq!(open.status, 200);
    assert_eq!(open.header("content-type"), Some("image/x-icon"));
    assert!(open.body.is_empty(), "HEAD must not return a body");

    let gated = server.request("HEAD", "/api/panes", &[]).await;
    assert_eq!(gated.status, 401, "HEAD is gated exactly where GET is");
}

// AYEAYE-42 — the login handshake is a GET. The daemon's do_POST has no route
// for /login and falls through to 404; answering a POST with a Set-Cookie
// would be a divergence nobody asked for.
#[tokio::test]
async fn the_login_handshake_is_not_reachable_by_post() {
    let server = Server::started().await;
    let answer = server
        .request(
            "POST",
            &format!("/login?token={TOKEN}"),
            &[("X-Voice-Token", TOKEN)],
        )
        .await;
    assert_eq!(answer.status, 404);
    assert_eq!(answer.header("set-cookie"), None);
}
