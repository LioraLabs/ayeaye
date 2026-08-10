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

use ayeaye::config::Settings;
use ayeaye_core::http::hosts::AllowedHosts;
use ayeaye_core::peer::{HostName, Peer, Registry};
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

    /// A POST carrying a JSON body, from this server's own origin.
    ///
    /// The `Origin` is not decoration. AYEAYE-69's CSRF gate refuses a write
    /// that a browser labelled cross-site, and it sits above route resolution —
    /// so a write test that presents no acceptable origin is refused with a 403
    /// that looks exactly like a bug in the endpoint under test.
    async fn post(&self, path: &str, body: &str, headers: &[(&str, &str)]) -> Answer {
        let origin = format!("http://127.0.0.1:{}", self.port);
        let length = body.len().to_string();
        let mut all: Vec<(&str, &str)> = vec![
            ("Origin", origin.as_str()),
            ("Content-Type", "application/json"),
            ("Content-Length", length.as_str()),
        ];
        all.extend_from_slice(headers);
        let raw: Vec<(&str, &[u8])> = all
            .iter()
            .map(|(name, value)| (*name, value.as_bytes()))
            .collect();
        self.request_with_body("POST", path, &raw, body.as_bytes())
            .await
    }

    /// The same, already carrying the token every gated call needs.
    async fn post_as_us(&self, path: &str, body: &str) -> Answer {
        self.post(path, body, &[("X-Voice-Token", TOKEN)]).await
    }

    /// The same, with header *values* as bytes rather than text.
    ///
    /// A header value is bytes on the wire and only sometimes text, and the
    /// difference is load-bearing for the origin gate: a value a Rust `String`
    /// cannot hold is exactly the one that must not read as "no header sent".
    /// A test that could only send `&str` could never send that request.
    async fn request_raw(&self, method: &str, path: &str, headers: &[(&str, &[u8])]) -> Answer {
        self.request_with_body(method, path, headers, b"").await
    }

    /// The same again, with a body after the headers.
    async fn request_with_body(
        &self,
        method: &str,
        path: &str,
        headers: &[(&str, &[u8])],
        body: &[u8],
    ) -> Answer {
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
        request.extend_from_slice(body);

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

/// Where this test binary keeps the files it has to put on a disk.
///
/// One directory per run of the suite, so two runs cannot read each other's,
/// and everything under it is written **once, before any test forks**. Rust
/// integration tests run as threads in one process: a `fork` in one thread
/// while another still holds a script open for writing hands the child an
/// inherited write handle to the file it is about to exec, and Linux answers
/// `ETXTBSY`. That failure reads as a bug in the code under test and only
/// appears under load, which is why the stand-in is written from a `OnceLock`
/// rather than per test.
fn scratch() -> &'static std::path::Path {
    static SCRATCH: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
    SCRATCH.get_or_init(|| {
        let root = std::env::temp_dir().join(format!("ayeaye-51-{}", std::process::id()));
        std::fs::create_dir_all(root.join("bin")).expect("a scratch directory");

        // A stand-in `claude`, which records the arguments it was given and
        // exits. A stand-in that *answers* would prove less: what this has to
        // show is that the prompt arrived as one argument, byte for byte, and
        // only the argv can show that.
        //
        // One argument per line, and the log is written whole in one `printf`,
        // so a reader either sees nothing or sees all of it.
        let claude = root.join("bin/claude");
        std::fs::write(
            &claude,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$#\" \"$@\" > {}\n",
                root.join("argv").display()
            ),
        )
        .expect("the stand-in agent should be writable");
        std::fs::set_permissions(
            &claude,
            <std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o755),
        )
        .expect("the stand-in agent should be executable");
        root
    })
}

/// What the stand-in agent recorded, once it has run.
///
/// Polled rather than read once: the command is *typed into a shell*, so
/// between the response and the argv landing there is a shell to read the line
/// and a process to start. Giving up is a failure with what was there.
async fn recorded_argv() -> Vec<String> {
    let log = scratch().join("argv");
    for _ in 0..200 {
        if let Ok(text) = std::fs::read_to_string(&log) {
            let lines: Vec<String> = text.lines().map(str::to_string).collect();
            // The first line is `$#`, so a log with that many lines after it is
            // a complete one rather than a half-written one.
            if lines.first().and_then(|count| count.parse::<usize>().ok())
                == Some(lines.len().saturating_sub(1))
            {
                return lines;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!(
        "the agent never recorded its arguments in {}",
        log.display()
    );
}

// AYEAYE-51 — the whole of a spawn, from the seam a phone reads: a real socket,
// a real tmux, a real project directory, and an agent that really starts in it.
// The prompt is the part only this can prove — it is typed into a shell, and
// what the shell made of it is visible nowhere but in the argv the agent was
// given.
#[tokio::test]
async fn spawning_starts_the_agent_in_the_project_with_the_prompt_as_one_argument() {
    let Some(tmux) = common::Private::named("serve-spawn") else {
        eprintln!("skipped: no tmux on this machine");
        return;
    };
    // Every pane this server opens runs `/bin/sh` with a `PATH` holding
    // *nothing but* the stand-in, and this option is set on a tmux server this
    // test started on a socket of its own.
    //
    // Both halves are load-bearing, and both were learned the hard way here:
    //
    // - `default-command` rather than `set-environment PATH`, because tmux
    //   starts a **login** shell by default and `/etc/profile` rewrites `PATH`.
    //   An earlier version of this test set the environment, watched the
    //   profile overwrite it, and started the machine's real `claude`.
    // - `PATH` holding only the stand-in, rather than the stand-in in front of
    //   the real one. This test asserts what an agent was given, and the agent
    //   it must never reach is the real one on the machine running the suite.
    //   Prepending would make that a matter of ordering; replacing makes
    //   "the stand-in is missing" a `claude: not found` and a failed test
    //   instead of somebody's actual agent starting in a temporary directory.
    tmux.tmux(&[
        "set-option",
        "-g",
        "default-command",
        &format!("PATH={} /bin/sh", scratch().join("bin").display()),
    ]);

    let project = scratch().join("ayeaye-51-project");
    std::fs::create_dir_all(&project).expect("a project directory");

    let mut settings = settings_on_port(0);
    settings.tmux = tmux.layer();
    let server = Server::start(settings).await;

    // An apostrophe, a double quote, a `$`, a backtick and a glob: everything
    // a shell would act on if the quoting were not doing its job.
    let prompt = "it's a \"test\" of $HOME and `id` and *.rs";
    let answer = server
        .post_as_us(
            "/api/spawn",
            &format!(
                r#"{{"dir":{},"agent":"claude","prompt":{}}}"#,
                serde_json::to_string(&project.to_string_lossy().into_owned()).unwrap(),
                serde_json::to_string(prompt).unwrap()
            ),
        )
        .await;

    assert_eq!(answer.status, 200, "{}", answer.body_text());
    let body = answer.body_text();
    assert!(
        body.contains(r#""pane":"desktop/%"#),
        "the pane has to be qualified, or the panel cannot select it: {body}"
    );
    assert!(
        body.contains(r#""session":"ayeaye-51-project""#),
        "the session is named after the project: {body}"
    );
    assert!(
        body.contains(r#""created":"session""#) && body.contains(r#""agent":"claude""#),
        "{body}"
    );

    // The agent really ran, in that directory, with the prompt as exactly one
    // argument — unsplit, unexpanded, and byte for byte what was sent.
    let argv = recorded_argv().await;
    assert_eq!(argv, ["1", prompt], "the agent was given {argv:?}");

    // And the panel can see the pane it was told about.
    let panes = server
        .request("GET", "/api/panes", &[("X-Voice-Token", TOKEN)])
        .await
        .body_text();
    let pane = body
        .split(r#""pane":""#)
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .expect("a pane id in the reply");
    assert!(
        panes.contains(&format!(r#""id":"{pane}""#)),
        "the spawned pane is not in the pane list: {panes}"
    );
}

// AYEAYE-51 — the prompt is the most attacker-influenceable string in the app,
// and the quoting can refuse it. A refusal is a stated 400 that says what to do
// about it, and nothing is started: a pane left behind by a request that failed
// is a pane nobody knows about.
#[tokio::test]
async fn a_prompt_that_cannot_be_typed_starts_nothing_and_says_why() {
    let Some(tmux) = common::Private::named("serve-unquotable") else {
        eprintln!("skipped: no tmux on this machine");
        return;
    };
    let project = scratch().join("ayeaye-51-unquotable");
    std::fs::create_dir_all(&project).expect("a project directory");

    let mut settings = settings_on_port(0);
    settings.tmux = tmux.layer();
    let server = Server::start(settings).await;

    let answer = server
        .post_as_us(
            "/api/spawn",
            &format!(
                r#"{{"dir":{},"agent":"claude","prompt":"escape the \\n in it"}}"#,
                serde_json::to_string(&project.to_string_lossy().into_owned()).unwrap()
            ),
        )
        .await;
    assert_eq!(answer.status, 400);
    let said = answer.body_text();
    assert!(said.contains("backslash"), "{said}");
    assert!(said.contains("take it out"), "{said}");

    // Nothing was made. The pane list is the only thing that can say so, and it
    // is what the panel would be reading a moment later. This server was
    // started with one session of one pane, so anything else is a pane a
    // refused request left behind.
    //
    // Asserted against that known state rather than against a reading taken
    // before the request: two live readings can differ because *either* of them
    // failed, and comparing them turns a tmux that was busy into a failure
    // claiming a pane was created. The `error` check is what keeps this test
    // honest about which of the two happened.
    let after = server
        .request("GET", "/api/panes", &[("X-Voice-Token", TOKEN)])
        .await
        .body_text();
    assert!(
        !after.contains(r#""error""#),
        "the pane list could not be read, so this test proves nothing: {after}"
    );
    assert_eq!(
        after.matches(r#""id""#).count(),
        1,
        "a refused spawn left a pane behind: {after}"
    );
    assert!(after.contains(r#""session":"work""#), "{after}");
}

// AYEAYE-51 — the agent is an allowlist and the directory has to exist. Both
// refusals are the daemon's own words, because they are put on screen unchanged
// by a page this ticket does not touch.
#[tokio::test]
async fn a_spawn_is_refused_for_an_unknown_agent_or_a_directory_that_is_not_there() {
    let Some(tmux) = common::Private::named("serve-spawn-refusals") else {
        eprintln!("skipped: no tmux on this machine");
        return;
    };
    let mut settings = settings_on_port(0);
    settings.tmux = tmux.layer();
    let server = Server::start(settings).await;

    for (body, expected) in [
        (r#"{"dir":"/tmp","agent":"bash"}"#, "unknown agent"),
        (r#"{"dir":"/tmp","agent":"sh -c id"}"#, "unknown agent"),
        (r#"{"dir":"/tmp"}"#, "unknown agent"),
        (
            r#"{"dir":"/nope/ayeaye-51","agent":"claude"}"#,
            "no such directory",
        ),
        // A file is not a directory, and neither is nothing.
        (
            r#"{"dir":"/etc/hostname","agent":"claude"}"#,
            "no such directory",
        ),
        (r#"{"agent":"claude"}"#, "no such directory"),
        (r#"{"dir":"","agent":"claude"}"#, "no such directory"),
    ] {
        let answer = server.post_as_us("/api/spawn", body).await;
        assert_eq!(answer.status, 400, "for {body}");
        assert_eq!(
            answer.body_text(),
            format!(r#"{{"error":"{expected}"}}"#),
            "for {body}"
        );
    }

    // A body that is not a request at all.
    for body in ["[1,2,3]", "{not json", r#""hello""#] {
        let answer = server.post_as_us("/api/spawn", body).await;
        assert_eq!(answer.status, 400, "for {body}");
        assert_eq!(answer.body_text(), r#"{"error":"bad json"}"#, "for {body}");
    }
}

// AYEAYE-51 — "the spawn request body carries an optional host field defaulting
// to this machine, so the shape is right for federation without implementing
// it". Optional is the half the panel depends on today; resolved through the
// registry is the half that makes it a shape rather than a decoration — this
// machine by name works, and a machine nobody registered is refused by name
// rather than quietly started here.
#[tokio::test]
async fn the_host_field_defaults_to_this_machine_and_is_resolved_rather_than_ignored() {
    let Some(tmux) = common::Private::named("serve-spawn-host") else {
        eprintln!("skipped: no tmux on this machine");
        return;
    };
    let mut settings = settings_on_port(0);
    settings.tmux = tmux.layer();
    let server = Server::start(settings).await;

    // Naming this machine is the same request as naming nothing: both get as
    // far as the agent check rather than being refused for their host.
    for host in [r#""host":"desktop","#, r#""host":"DESKTOP","#, ""] {
        let answer = server
            .post_as_us(
                "/api/spawn",
                &format!(r#"{{{host}"dir":"/tmp","agent":"nonesuch"}}"#),
            )
            .await;
        assert_eq!(
            answer.body_text(),
            r#"{"error":"unknown agent"}"#,
            "host {host:?} is this machine and must not be refused for its host"
        );
    }

    // A machine this deployment has never heard of is refused by name, and
    // nothing is started here on its behalf.
    let elsewhere = server
        .post_as_us(
            "/api/spawn",
            r#"{"host":"gpu-box","dir":"/tmp","agent":"claude"}"#,
        )
        .await;
    assert_eq!(elsewhere.status, 400);
    assert!(
        elsewhere.body_text().contains("gpu-box"),
        "{}",
        elsewhere.body_text()
    );
}

// AYEAYE-51 — a spawn is a write, so it is behind both gates the daemon puts in
// front of one. Worth its own test rather than trusted to the route table: the
// CSRF gate lives above route resolution, so an endpoint added to the router
// instead of to the handler would pass every unit test and sit outside it.
#[tokio::test]
async fn spawning_needs_a_token_and_is_refused_from_another_site() {
    let server = Server::started().await;
    let body = r#"{"dir":"/tmp","agent":"claude"}"#;

    assert_eq!(
        server.post("/api/spawn", body, &[]).await.status,
        401,
        "a spawn with no token must be refused"
    );

    // 403 and *before* the token, so a stolen token buys nothing here.
    for headers in [
        vec![("Sec-Fetch-Site", "cross-site")],
        vec![("Sec-Fetch-Site", "cross-site"), ("X-Voice-Token", TOKEN)],
    ] {
        let answer = server.post("/api/spawn", body, &headers).await;
        assert_eq!(answer.status, 403, "with {headers:?}");
        assert_eq!(answer.body_text(), r#"{"error":"forbidden"}"#);
    }

    // And a GET of it is nothing at all: this endpoint acts, and an action
    // behind a GET would be an action outside the CSRF gate.
    assert_eq!(
        server
            .request("GET", "/api/spawn", &[("X-Voice-Token", TOKEN)])
            .await
            .status,
        404
    );
}

/// The panes `/api/panes` is currently reporting, as `(id, session)`.
///
/// Read through the endpoint rather than off the tmux server, because that is
/// what the panel reads: "the panel reflects it" is a claim about this list.
async fn listed_panes(server: &Server) -> Vec<(String, String)> {
    let body = server
        .request("GET", "/api/panes", &[("X-Voice-Token", TOKEN)])
        .await
        .body_text();
    assert!(
        !body.contains(r#""error""#),
        "the pane list could not be read: {body}"
    );
    body.split(r#"{"id":""#)
        .skip(1)
        .map(|card| {
            let id = card.split('"').next().expect("an id").to_string();
            let session = card
                .split(r#""session":""#)
                .nth(1)
                .and_then(|rest| rest.split('"').next())
                .expect("a session")
                .to_string();
            (id, session)
        })
        .collect()
}

// AYEAYE-51 — a kill, end to end: a window this test made disappears from the
// pane list the panel reads. Asserted by reading that list again rather than by
// trusting the 200 — "the panel reflects it" is a fact about tmux, and only the
// list can say it happened.
#[tokio::test]
async fn killing_a_pane_removes_it_from_the_list_the_panel_reads() {
    let Some(tmux) = common::Private::named("serve-kill") else {
        eprintln!("skipped: no tmux on this machine");
        return;
    };
    tmux.tmux(&[
        "new-window",
        "-t",
        "=work",
        "-n",
        "ayeaye-51-doomed",
        "-d",
        "/bin/sh",
    ]);

    let mut settings = settings_on_port(0);
    settings.tmux = tmux.layer();
    let server = Server::start(settings).await;

    // The safety property of this test, asserted before anything is signalled:
    // the pane about to be killed is one *this test* made, on a tmux server
    // this test started. A test pointed at the wrong tmux fails here, with
    // nothing killed, rather than after.
    let before = listed_panes(&server).await;
    let doomed = before
        .iter()
        .find(|(_, session)| session == "work")
        .map(|(id, _)| id.clone())
        .expect("the private server's own pane");
    assert_eq!(before.len(), 2, "this test made two panes: {before:?}");
    assert!(doomed.starts_with("desktop/%"), "{doomed}");

    let answer = server
        .post_as_us("/api/kill", &format!(r#"{{"pane":"{doomed}"}}"#))
        .await;
    assert_eq!(answer.status, 200, "{}", answer.body_text());
    assert_eq!(answer.body_text(), r#"{"ok":true}"#);

    let after = listed_panes(&server).await;
    assert!(
        !after.iter().any(|(id, _)| *id == doomed),
        "{doomed} is still listed: {after:?}"
    );
    assert_eq!(after.len(), 1, "only the one pane went: {after:?}");
}

// AYEAYE-51 — the membership check, and the reason it is membership rather than
// syntax. A pane in a `_`-prefixed session is a real, live, perfectly killable
// tmux target that `list-panes` deliberately hides — somebody's floating
// scratch pane. It is exactly what a caller would name to reach a pane the
// panel never offered, and it has to be refused *and survive*.
//
// Without this case the whole check could be deleted and every other test here
// would stay green.
#[tokio::test]
async fn a_pane_the_list_hides_is_refused_and_is_still_there_afterwards() {
    let Some(tmux) = common::Private::named("serve-kill-hidden") else {
        eprintln!("skipped: no tmux on this machine");
        return;
    };
    // A floating scratch session, of the kind a tmux configuration makes and
    // the pane list drops. Its pane id is a real target on this server.
    tmux.tmux(&["new-session", "-d", "-s", "_scratch", "/bin/sh"]);

    let mut settings = settings_on_port(0);
    settings.tmux = tmux.layer();
    let server = Server::start(settings).await;

    let listed = listed_panes(&server).await;
    assert_eq!(
        listed.len(),
        1,
        "the scratch session must be hidden from the list: {listed:?}"
    );

    // The hidden pane's real id, asked of the tmux server rather than of the
    // endpoint — the endpoint is precisely the thing that will not reveal it,
    // and it is the thing on trial.
    let hidden = tmux
        .output(&["list-panes", "-t", "=_scratch", "-F", "#{pane_id}"])
        .expect("the scratch session's pane")
        .trim()
        .to_string();
    assert!(hidden.starts_with('%'), "{hidden:?} is not a pane id");

    let answer = server
        .post_as_us("/api/kill", &format!(r#"{{"pane":"desktop/{hidden}"}}"#))
        .await;
    assert_eq!(answer.status, 400);
    assert_eq!(answer.body_text(), r#"{"error":"no such pane"}"#);

    // And it is still running. This is the assertion that makes the test about
    // the check rather than about the status code: a server that answered 400
    // and killed it anyway would pass everything above.
    let survivors = tmux.sessions();
    assert!(
        survivors.iter().any(|name| name == "_scratch"),
        "the hidden session was killed: {survivors:?}"
    );
}

// AYEAYE-51 — the other ways a target can fail to be one of ours. A bare id
// names no machine, a machine nobody registered is not ours, and a pane id that
// is well-formed but was never listed is the ordinary case of a stale panel.
#[tokio::test]
async fn a_target_that_is_not_one_of_ours_is_refused_before_anything_is_signalled() {
    let Some(tmux) = common::Private::named("serve-kill-refusals") else {
        eprintln!("skipped: no tmux on this machine");
        return;
    };
    let mut settings = settings_on_port(0);
    settings.tmux = tmux.layer();
    let server = Server::start(settings).await;

    for body in [
        // Bare: names no machine at all. "Bare means here" is the assumption
        // qualified ids exist to delete.
        r#"{"pane":"%0"}"#,
        // A machine this deployment has never heard of.
        r#"{"pane":"gpu-box/%0"}"#,
        // Well-formed, ours, and never listed.
        r#"{"pane":"desktop/%99"}"#,
        // Shapes tmux itself would take as a target.
        r#"{"pane":"desktop/work"}"#,
        r#"{"pane":"desktop/=work"}"#,
    ] {
        let answer = server.post_as_us("/api/kill", body).await;
        assert_eq!(answer.status, 400, "for {body}");
        assert_eq!(
            answer.body_text(),
            r#"{"error":"no such pane"}"#,
            "for {body}"
        );
    }

    // A request that named no pane, and one that is not a request.
    for (body, expected) in [
        (r#"{}"#, "no pane"),
        (r#"{"pane":""}"#, "no pane"),
        ("[1,2,3]", "bad json"),
    ] {
        let answer = server.post_as_us("/api/kill", body).await;
        assert_eq!(answer.status, 400, "for {body}");
        assert_eq!(
            answer.body_text(),
            format!(r#"{{"error":"{expected}"}}"#),
            "for {body}"
        );
    }

    // Everything this server had is still there.
    assert_eq!(listed_panes(&server).await.len(), 1);
}

// AYEAYE-51 — a kill is a write, so it is behind both gates, and it is a POST
// for the same reason a spawn is: the CSRF gate exempts reads.
#[tokio::test]
async fn killing_needs_a_token_and_is_refused_from_another_site() {
    let server = Server::started().await;
    let body = r#"{"pane":"desktop/%0"}"#;

    assert_eq!(server.post("/api/kill", body, &[]).await.status, 401);
    for headers in [
        vec![("Sec-Fetch-Site", "cross-site")],
        vec![("Sec-Fetch-Site", "cross-site"), ("X-Voice-Token", TOKEN)],
    ] {
        assert_eq!(
            server.post("/api/kill", body, &headers).await.status,
            403,
            "with {headers:?}"
        );
    }
    assert_eq!(
        server
            .request("GET", "/api/kill", &[("X-Voice-Token", TOKEN)])
            .await
            .status,
        404
    );
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
