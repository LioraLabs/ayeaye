//! The server, observed the way a browser observes it: over a socket.
//!
//! Every test here binds port 0, so the suite never collides with the Python
//! daemon or with itself, and speaks raw HTTP/1.1 with `Connection: close` so
//! the body ends at EOF and no HTTP client crate is needed to read it. That
//! keeps the seam the highest one that can see the behaviour — a route table
//! that resolves correctly and a server that never binds would pass a unit
//! test and fail a phone.

mod common;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use ayeaye::config::Settings;
use ayeaye_core::http::hosts::AllowedHosts;
use ayeaye_core::peer::{HostName, Peer, Registry};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use common::Private;

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
        Server::serving(listener, settings)
    }

    /// Serve on a listener the caller has already had the server bind.
    ///
    /// Split out for the one test that has to know the port *before* the server
    /// starts: it cannot ask afterwards, and it cannot bind for itself without
    /// racing every other test in this binary for the port it borrowed.
    fn serving(listener: TcpListener, settings: Settings) -> Server {
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

    /// A write, presented the way the panel presents one.
    ///
    /// The `Origin` is this server's own and the token is the test token,
    /// because AYEAYE-69's CSRF gate judges every non-GET before anything reads
    /// a body: a write test that sent neither would be refused at that gate and
    /// would look exactly like the endpoint being broken.
    async fn write(&self, path: &str, body: &str) -> Answer {
        let origin = format!("http://127.0.0.1:{}", self.port);
        let length = body.len().to_string();
        self.request_with_body(
            "POST",
            path,
            &[
                ("Origin", origin.as_str()),
                ("X-Voice-Token", TOKEN),
                ("Content-Type", "application/json"),
                ("Content-Length", length.as_str()),
            ],
            body,
        )
        .await
    }

    /// The same as [`Server::request`], with a body after the headers.
    async fn request_with_body(
        &self,
        method: &str,
        path: &str,
        headers: &[(&str, &str)],
        body: &str,
    ) -> Answer {
        let mut stream = TcpStream::connect(("127.0.0.1", self.port))
            .await
            .expect("the server should be listening");

        let mut request = format!("{method} {path} HTTP/1.1\r\n").into_bytes();
        for (name, value) in headers {
            request.extend_from_slice(format!("{name}: {value}\r\n").as_bytes());
        }
        request.extend_from_slice(format!("Host: 127.0.0.1:{}\r\n", self.port).as_bytes());
        request.extend_from_slice(b"Connection: close\r\n\r\n");
        request.extend_from_slice(body.as_bytes());

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
}

/// One value, percent-encoded the way `encodeURIComponent` encodes it.
///
/// Every tmux pane id starts with `%`, so an id in a query string is always
/// encoded on the way here — and a server that compared the raw text would be
/// looking at different text than the panel sent.
fn encoded(value: &str) -> String {
    let mut out = String::new();
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

/// One JSON string literal, for building a body in a test.
fn quoted(value: &str) -> String {
    ayeaye_core::json::string(value)
}

fn settings_on_port(port: u16) -> Settings {
    Settings {
        bind: "127.0.0.1".to_string(),
        port,
        allowed_hosts: AllowedHosts::new("127.0.0.1", port, ""),
        token: TOKEN.to_string(),
        peers: registry("desktop"),
        // Pointed at a socket no server is on, so a test that does not care
        // about panes still cannot read the panes of whoever is running the
        // suite. The cases that do care point it at a server of their own.
        tmux: common::nowhere("serve-nobody"),
    }
}

/// A deployment of one machine, under the name a test wants to see.
fn registry(name: &str) -> Registry {
    Registry::new(vec![Peer::here(HostName::new(name).expect("a host name"))])
        .expect("one peer, and it is this machine")
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
    //
    // Between handing it back and the server taking it, any other test in this
    // binary asking for port 0 can be given the same number — so this is tried
    // a few times rather than once. The claim is about which port the server
    // binds; losing a race for a port is not that claim failing, and a test
    // that cannot tell the two apart goes red for reasons nobody can act on.
    let mut refusals = Vec::new();
    for _ in 0..8 {
        let scout = TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("port 0 should always bind");
        let chosen = scout.local_addr().expect("a bound address").port();
        drop(scout);

        let settings = settings_on_port(chosen);
        // The server's own bind, as `Server::start` uses — the port has to be
        // known before it starts, and this is the only way to know it.
        let listener = match ayeaye::server::listen(&settings).await {
            Ok(listener) => listener,
            Err(why) => {
                refusals.push(format!("{chosen}: {why}"));
                continue;
            }
        };
        let server = Server::serving(listener, settings);
        assert_eq!(server.port, chosen, "the server moved to a different port");
        assert_eq!(server.get("/").await.status, 200);
        return;
    }
    panic!("no borrowed port was still free when the server reached for it: {refusals:?}");
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
    for path in ["/", "/board", "/favicon.ico", "/api/overview"] {
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

    let anonymous = server.get("/api/overview").await;
    assert_eq!(anonymous.status, 401);
    assert_eq!(anonymous.body_text(), r#"{"error":"unauthorized"}"#);

    let wrong = server
        .request(
            "GET",
            "/api/overview",
            &[("X-Voice-Token", "not-the-token")],
        )
        .await;
    assert_eq!(wrong.status, 401, "a wrong token is not a token");

    // 404, not 401: the gate let it through and nothing answers there yet.
    let by_header = server
        .request("GET", "/api/overview", &[("X-Voice-Token", TOKEN)])
        .await;
    assert_eq!(by_header.status, 404);
    assert_eq!(by_header.body_text(), r#"{"error":"not found"}"#);

    let by_cookie = server
        .request(
            "GET",
            "/api/overview",
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

    // And with the token both gates let it through and the endpoint itself
    // answers. AYEAYE-48 gave this path a handler, so what a bodiless POST gets
    // is that handler refusing an empty body — which is still the proof this
    // test was written for: neither gate stopped it.
    let authorized = server
        .request(
            "POST",
            "/api/answer",
            &[("Origin", ours.as_str()), ("X-Voice-Token", TOKEN)],
        )
        .await;
    assert_eq!(authorized.status, 400);
    assert_eq!(authorized.body_text(), r#"{"error":"bad json"}"#);
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

    let gated = server.request("HEAD", "/api/overview", &[]).await;
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

// AYEAYE-43 — the whole of it, from the seam a phone reads: a real socket, a
// real tmux, and a pane list whose ids are already federation-shaped. The unit
// tests prove the parse and the body; only this proves they are wired to the
// route, with this machine's name on them.
#[tokio::test]
async fn the_pane_list_comes_back_over_a_socket_with_every_id_qualified() {
    let Some(tmux) = common::Private::named("serve-live") else {
        eprintln!("skipped: no tmux on this machine");
        return;
    };
    tmux.tmux(&["new-window", "-t", "work", "-n", "cook", "-d", "/bin/sh"]);

    let mut settings = settings_on_port(0);
    settings.tmux = tmux.layer();
    let server = Server::start(settings).await;

    let answer = server
        .request("GET", "/api/panes", &[("X-Voice-Token", TOKEN)])
        .await;
    assert_eq!(answer.status, 200);
    assert_eq!(answer.header("content-type"), Some("application/json"));
    // Live state, so never cached — the same rule every other answer follows.
    assert_eq!(answer.header("cache-control"), Some("no-store"));

    let body = answer.body_text();
    assert!(
        body.starts_with(r#"{"host":"desktop","panes":[{"id":"desktop/%"#),
        "the panel needs the host and qualified ids: {body}"
    );
    assert_eq!(body.matches(r#""session":"work""#).count(), 2, "{body}");
    assert!(body.contains(r#""name":"cook""#), "{body}");
    assert!(!body.contains("error"), "nothing failed: {body}");
}

// AYEAYE-43 — a machine with no tmux server answers an empty list rather than
// an error. Most machines are in that state most of the time, and the panel has
// to render on all of them.
#[tokio::test]
async fn a_machine_with_no_tmux_server_answers_an_empty_list() {
    if !common::have_tmux() {
        eprintln!("skipped: no tmux on this machine");
        return;
    }
    let server = Server::started().await;
    let answer = server
        .request("GET", "/api/panes", &[("X-Voice-Token", TOKEN)])
        .await;
    assert_eq!(answer.status, 200);
    assert_eq!(answer.body_text(), r#"{"host":"desktop","panes":[]}"#);
}

// AYEAYE-43 — and it is gated like everything else under /api/, by header or by
// the login cookie. A pane list names every session on the machine.
#[tokio::test]
async fn the_pane_list_needs_a_token() {
    let server = Server::started().await;
    assert_eq!(server.get("/api/panes").await.status, 401);
    assert_eq!(
        server
            .request("GET", "/api/panes", &[("X-Voice-Token", "not-the-token")])
            .await
            .status,
        401
    );
    assert_eq!(
        server
            .request(
                "GET",
                "/api/panes",
                &[("Cookie", &format!("voice_token={TOKEN}"))]
            )
            .await
            .status,
        200,
        "the login cookie should authenticate"
    );
}

/// A server whose tmux is a private one of the test's own, and the pane it is
/// pointed at.
///
/// **The safety check every case below leans on.** Nothing here may reach the
/// tmux the person running the suite keeps their work in, and the very next
/// thing these tests do is press keys. So the pane is taken out of the pane list
/// the server itself answers with — over the socket, through the route under
/// test — and that list is refused unless it is exactly the private server's.
/// A `Tmux` whose `-L` had not taken effect would answer with somebody's real
/// sessions here, and the test would stop rather than type into one.
async fn server_on_its_own_tmux(what: &str, program: &str) -> Option<(Private, Server, String)> {
    let server = Private::named(what)?;
    server.tmux(&["new-window", "-t", "work", "-n", "answering", "-d", program]);

    let mut settings = settings_on_port(0);
    // Submitting immediately: the 400ms gap exists so a TUI does not read the
    // Enter as part of the same burst, and `cat` is not a TUI.
    settings.tmux = server.layer().submitting_after(Duration::from_millis(1));
    let serving = Server::start(settings).await;

    let listed = serving
        .request("GET", "/api/panes", &[("X-Voice-Token", TOKEN)])
        .await
        .body_text();
    assert!(
        listed.contains(r#""name":"answering""#) && listed.contains(r#""session":"work""#),
        "this is not the suite's own tmux server: {listed}"
    );
    assert!(
        !listed.contains(r#""session":"_"#),
        "this is not the suite's own tmux server: {listed}"
    );
    let pane = id_of(&listed, "answering");
    Some((server, serving, pane))
}

/// The qualified id of the pane with this window name, out of a `/api/panes`
/// body. Read out of the answer rather than built, so what is targeted below is
/// what the panel would have been given.
fn id_of(body: &str, window: &str) -> String {
    let card = body
        .split("{\"id\":\"")
        .find(|card| card.contains(&format!(r#""name":"{window}""#)))
        .unwrap_or_else(|| panic!("no card for {window} in {body}"));
    card.split('"')
        .next()
        .expect("the id ends at its closing quote")
        .to_string()
}

/// What the pane says, once it says it — or what it last said, at the deadline.
async fn settles(tmux: &Private, pane: &str, until: impl Fn(&str) -> bool) -> String {
    let bare = pane.split_once('/').expect("a qualified id").1.to_string();
    for _ in 0..100 {
        let said = tmux.captured(&bare);
        if until(&said) {
            return said;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    tmux.captured(&bare)
}

// AYEAYE-48 — the whole ticket, from the seam a phone reads: a real socket, a
// real tmux, a real screen with a question drawn on it, and the options coming
// back as JSON. The parser tests prove the reading and the tmux tests prove the
// capture; only this proves they are wired to a route.
#[tokio::test]
async fn a_pane_at_a_prompt_shows_its_options_and_can_be_answered() {
    let Some((tmux, server, pane)) = server_on_its_own_tmux("prompt", "cat").await else {
        eprintln!("skipped: no tmux on this machine");
        return;
    };
    let bare = pane.split_once('/').expect("a qualified id").1;
    tmux.tmux(&[
        "send-keys",
        "-t",
        bare,
        "-l",
        "--",
        "Pick one?\n 1. First\n 2. Second\n Enter to select . Esc to cancel\n",
    ]);
    settles(&tmux, &pane, |said| said.contains("Esc to cancel")).await;

    let asked = server
        .request(
            "GET",
            &format!("/api/prompt?pane={}", encoded(&pane)),
            &[("X-Voice-Token", TOKEN)],
        )
        .await;
    assert_eq!(asked.status, 200);
    assert_eq!(asked.header("content-type"), Some("application/json"));
    assert_eq!(asked.header("cache-control"), Some("no-store"));
    let body = asked.body_text();
    assert!(body.contains(r#""question":"Pick one?""#), "{body}");
    assert!(body.contains(r#"{"key":"1","label":"First"}"#), "{body}");
    assert!(body.contains(r#"{"key":"2","label":"Second"}"#), "{body}");

    // And answering it presses that key in that pane, which the pane itself is
    // the witness for.
    let answered = server
        .write("/api/answer", &format!(r#"{{"pane":"{pane}","key":"2"}}"#))
        .await;
    assert_eq!(answered.status, 200, "{}", answered.body_text());
    assert_eq!(answered.body_text(), r#"{"ok":true,"sent":"2"}"#);
    let screen = settles(&tmux, &pane, |said| said.contains("Esc to cancel\n2")).await;
    assert!(
        screen.contains("Esc to cancel\n2"),
        "the key never reached the pane: {screen:?}"
    );
}

// AYEAYE-48 — a pane with no question on it answers a null prompt, which is not
// an error and is not silence. The page clears its card on a null and keeps the
// last one on a failure, and those are opposite instructions.
#[tokio::test]
async fn a_pane_with_no_question_on_it_answers_a_null_prompt() {
    let Some((_tmux, server, pane)) = server_on_its_own_tmux("quiet", "cat").await else {
        eprintln!("skipped: no tmux on this machine");
        return;
    };
    let asked = server
        .request(
            "GET",
            &format!("/api/prompt?pane={}", encoded(&pane)),
            &[("X-Voice-Token", TOKEN)],
        )
        .await;
    assert_eq!(asked.status, 200);
    assert_eq!(asked.body_text(), r#"{"prompt":null}"#);
}

// AYEAYE-48 — a pane that is not there has no question on it, and that is an
// answer rather than a refusal. `share/app.html`'s pollPrompt keeps its last
// card on a failed request, so refusing here would leave a dead pane's question
// on screen for as long as the transcript view stayed open. Naming no pane at
// all is still a refusal, because that is a caller mistake and not a fact about
// a pane — which is exactly the daemon's split too.
#[tokio::test]
async fn a_pane_that_is_not_there_has_no_question_on_it() {
    let Some((_tmux, server, pane)) = server_on_its_own_tmux("gone", "cat").await else {
        eprintln!("skipped: no tmux on this machine");
        return;
    };
    let bare = pane.split_once('/').expect("a qualified id").1.to_string();
    for missing in [
        "desktop/%9999".to_string(),
        bare.clone(),
        format!("gpu-box/{bare}"),
        format!("desktop/{bare} ; kill-server"),
    ] {
        let asked = server
            .request(
                "GET",
                &format!("/api/prompt?pane={}", encoded(&missing)),
                &[("X-Voice-Token", TOKEN)],
            )
            .await;
        assert_eq!(asked.status, 200, "for {missing:?}");
        assert_eq!(asked.body_text(), r#"{"prompt":null}"#, "for {missing:?}");
    }
    for blank in ["", "%20%20"] {
        let asked = server
            .request(
                "GET",
                &format!("/api/prompt?pane={blank}"),
                &[("X-Voice-Token", TOKEN)],
            )
            .await;
        assert_eq!(asked.status, 400, "for {blank:?}");
        assert_eq!(asked.body_text(), r#"{"error":"no pane"}"#);
    }
}

// AYEAYE-48 — **the check between a request and somebody's terminal.** A pane id
// that parses is not a pane. Every id here is refused, and the pane that really
// exists is untouched afterwards — which is the assertion that matters, since a
// 400 with the key already sent would look exactly like a pass.
#[tokio::test]
async fn a_pane_id_that_is_not_a_live_pane_sends_no_key_at_all() {
    let Some((tmux, server, pane)) = server_on_its_own_tmux("forged", "cat").await else {
        eprintln!("skipped: no tmux on this machine");
        return;
    };
    let bare = pane.split_once('/').expect("a qualified id").1.to_string();

    for forged in [
        // Well formed, and not there. This is what a forged id looks like.
        "desktop/%9999".to_string(),
        // The real pane, un-qualified. Ids go out qualified and come back
        // qualified; a bare one is not an id this deployment issued.
        bare.clone(),
        // The real pane, on a machine that is not this one.
        format!("gpu-box/{bare}"),
        // The real pane, with something after it. tmux would take `%0` out of
        // `%0 ; kill-server` as a target; membership never gets that far.
        format!("desktop/{bare} ; kill-server"),
        format!("desktop/{bare}\t"),
        // A tmux target that is not a pane id at all: `-a` is every pane there
        // is, and `.+` is the last pane of a window.
        "desktop/-a".to_string(),
        "desktop/.+".to_string(),
        "desktop/%".to_string(),
        // Not an id at all.
        "desktop/".to_string(),
        "/".to_string(),
        String::new(),
    ] {
        let refusal = server
            .write(
                "/api/answer",
                &format!(r#"{{"pane":{},"key":"1"}}"#, quoted(&forged)),
            )
            .await;
        assert_eq!(refusal.status, 400, "{forged:?} was not refused");
        assert!(
            refusal.body_text().contains("pane"),
            "{forged:?}: {}",
            refusal.body_text()
        );

        let typing = server
            .write(
                "/api/send",
                &format!(r#"{{"pane":{},"text":"whoops"}}"#, quoted(&forged)),
            )
            .await;
        assert_eq!(
            typing.status, 400,
            "{forged:?} was not refused for /api/send"
        );
    }

    // Nothing arrived anywhere. The pane that exists is still blank, and the
    // server that would have been killed is still answering.
    let screen = tmux.captured(&bare);
    assert!(
        screen.trim().is_empty(),
        "a refused request still reached the pane: {screen:?}"
    );
    let still_there = server
        .request("GET", "/api/panes", &[("X-Voice-Token", TOKEN)])
        .await;
    assert_eq!(still_there.status, 200);
    assert!(still_there.body_text().contains(r#""name":"answering""#));
}

// AYEAYE-48 — "typing sends text without submitting it". The pane is the witness
// worth having: `cat` repeats a line the moment a newline ends it, so one copy
// on the screen is the proof that nothing was submitted and two is the proof
// that the separate Enter arrived.
#[tokio::test]
async fn typing_sends_the_text_and_only_submits_when_asked() {
    let Some((tmux, server, pane)) = server_on_its_own_tmux("typing", "cat").await else {
        eprintln!("skipped: no tmux on this machine");
        return;
    };

    let typed = server
        .write(
            "/api/send",
            &format!(r#"{{"pane":"{pane}","text":"ship it"}}"#),
        )
        .await;
    assert_eq!(typed.status, 200, "{}", typed.body_text());
    assert_eq!(typed.body_text(), r#"{"ok":true}"#);
    let screen = settles(&tmux, &pane, |said| said.contains("ship it")).await;
    assert_eq!(
        screen.matches("ship it").count(),
        1,
        "typing submitted the text: {screen:?}"
    );

    let submitted = server
        .write(
            "/api/send",
            &format!(r#"{{"pane":"{pane}","text":"!","enter":true}}"#),
        )
        .await;
    assert_eq!(submitted.status, 200);
    let screen = settles(&tmux, &pane, |said| said.contains("ship it!\nship it!")).await;
    assert!(
        screen.contains("ship it!\nship it!"),
        "the Enter never arrived: {screen:?}"
    );
}

// AYEAYE-48 — text that would submit itself is refused before it reaches the
// pane. `send-keys -l` writes the bytes straight to the terminal, so a newline
// in the text is a submit nobody asked for and nobody saw — and an escape byte
// is the start of a sequence the agent's TUI acts on rather than shows.
#[tokio::test]
async fn text_that_would_submit_itself_never_reaches_the_pane() {
    let Some((tmux, server, pane)) = server_on_its_own_tmux("control", "cat").await else {
        eprintln!("skipped: no tmux on this machine");
        return;
    };
    let bare = pane.split_once('/').expect("a qualified id").1.to_string();

    for hostile in [
        r"rm -rf ~\n",
        r"a\rb",
        r"a\u001b[2Jb",
        r"a\u0000b",
        r"\t",
        "",
    ] {
        let refusal = server
            .write(
                "/api/send",
                &format!(r#"{{"pane":"{pane}","text":"{hostile}"}}"#),
            )
            .await;
        assert_eq!(refusal.status, 400, "{hostile:?} was typed into a pane");
    }
    assert!(
        tmux.captured(&bare).trim().is_empty(),
        "something reached the pane: {:?}",
        tmux.captured(&bare)
    );
}

// AYEAYE-48 — the key allow-list, at the seam it protects. This endpoint is
// reachable over the network and must not become a general send-keys hole, so
// what is refused is refused here and not only in the table.
#[tokio::test]
async fn only_a_key_from_the_allow_list_can_be_pressed() {
    let Some((tmux, server, pane)) = server_on_its_own_tmux("keys", "cat").await else {
        eprintln!("skipped: no tmux on this machine");
        return;
    };
    let bare = pane.split_once('/').expect("a qualified id").1.to_string();

    for hostile in [
        r#""C-c""#,
        r#""C-d""#,
        r#""Enter""#,
        r#""Escape""#,
        r#""0""#,
        r#""10""#,
        r#""05""#,
        r#""kill-server""#,
        r#""""#,
        "1",
        "null",
        "true",
        "[]",
    ] {
        let refusal = server
            .write(
                "/api/answer",
                &format!(r#"{{"pane":"{pane}","key":{hostile}}}"#),
            )
            .await;
        assert_eq!(refusal.status, 400, "key {hostile} was pressed");
        assert_eq!(refusal.body_text(), r#"{"error":"bad key"}"#, "{hostile}");
    }
    assert!(
        tmux.captured(&bare).trim().is_empty(),
        "a refused key still reached the pane: {:?}",
        tmux.captured(&bare)
    );
}

// AYEAYE-48 — a body off a socket is whatever somebody wrote. Every way it can
// be wrong is an answer rather than a panic, and the server is still serving
// afterwards, which a worker that took the runtime down with it would not be.
#[tokio::test]
async fn a_body_that_is_not_a_request_is_answered_rather_than_crashed_into() {
    let server = Server::started().await;
    let deep = format!("{}{}", "[".repeat(5000), "]".repeat(5000));
    for body in [
        "",
        "{",
        "not json at all",
        r#"["desktop/%0","1"]"#,
        r#"{"pane":12,"key":"1"}"#,
        r#"{"key":"1"}"#,
        deep.as_str(),
    ] {
        for path in ["/api/answer", "/api/send"] {
            let answer = server.write(path, body).await;
            assert_eq!(answer.status, 400, "{path} with {body:.40}");
            assert!(
                answer.body_text().starts_with(r#"{"error":"#),
                "{path}: {}",
                answer.body_text()
            );
        }
    }
    assert_eq!(server.get("/").await.status, 200);
}

// AYEAYE-48 — the three routes are gated like everything else under `/api/`,
// and the two that write are refused cross-site before their token is looked at.
// A POST here presses a key in somebody's terminal; nothing about it may be
// reachable from another origin's page.
#[tokio::test]
async fn the_prompt_routes_need_a_token_and_refuse_a_cross_site_write() {
    let server = Server::started().await;
    for path in ["/api/prompt?pane=desktop/%0", "/api/answer", "/api/send"] {
        assert_eq!(server.get(path).await.status, 401, "GET {path}");
        assert_eq!(
            server.request("POST", path, &[]).await.status,
            401,
            "POST {path}"
        );
    }
    for path in ["/api/answer", "/api/send"] {
        let cross = server
            .request(
                "POST",
                path,
                &[("Sec-Fetch-Site", "cross-site"), ("X-Voice-Token", TOKEN)],
            )
            .await;
        assert_eq!(cross.status, 403, "POST {path} cross-site");
        let foreign = server
            .request(
                "POST",
                path,
                &[("Origin", "https://evil.example"), ("X-Voice-Token", TOKEN)],
            )
            .await;
        assert_eq!(foreign.status, 403, "POST {path} from a foreign origin");
    }
    // And the methods nothing is mounted on are still a 404 rather than an
    // answer, which is what the daemon says too.
    assert_eq!(
        server
            .request("POST", "/api/prompt", &[("X-Voice-Token", TOKEN)])
            .await
            .status,
        404
    );
    assert_eq!(
        server
            .request("GET", "/api/answer", &[("X-Voice-Token", TOKEN)])
            .await
            .status,
        404
    );
}

// AYEAYE-48 — the query is percent-decoded before the id is compared, because
// that is the id the panel encoded. Every tmux pane id starts with `%`, so the
// encoded form and the raw form are never the same text — a server comparing
// the raw text would be looking at something the panel never sent, and this
// request would be refused.
#[tokio::test]
async fn a_pane_id_in_the_query_is_decoded_before_it_is_matched() {
    let Some((_tmux, server, pane)) = server_on_its_own_tmux("encoded", "cat").await else {
        eprintln!("skipped: no tmux on this machine");
        return;
    };
    let sent = encoded(&pane);
    assert!(sent.contains("%25") && sent != pane, "{sent}");
    let asked = server
        .request(
            "GET",
            &format!("/api/prompt?pane={sent}"),
            &[("X-Voice-Token", TOKEN)],
        )
        .await;
    assert_eq!(asked.status, 200, "{}", asked.body_text());
    assert_eq!(asked.body_text(), r#"{"prompt":null}"#);
}
