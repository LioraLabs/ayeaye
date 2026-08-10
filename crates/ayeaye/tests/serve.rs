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
use std::sync::{Arc, Mutex};

use ayeaye::config::Settings;
use ayeaye::fit::Fits;
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

    /// A GET carrying the token, which is what every `/api/` call needs.
    async fn api(&self, path: &str) -> Answer {
        self.request("GET", path, &[("X-Voice-Token", TOKEN)]).await
    }

    /// A POST carrying the token and a JSON body, as the page sends.
    async fn post(&self, path: &str, body: &str) -> Answer {
        let mut stream = TcpStream::connect(("127.0.0.1", self.port))
            .await
            .expect("the server should be listening");
        let request = format!(
            "POST {path} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nX-Voice-Token: {TOKEN}\r\n\
             Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            self.port,
            body.len()
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
        parse(&raw)
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
        pane_cache: Arc::new(Mutex::new(ayeaye_core::pane::Cache::default())),
        // No path: a test must never write into the state directory of whoever
        // is running the suite. Recovery across a restart is proved in
        // `tests/fit.rs`, where the file is the test's own.
        fits: Arc::new(Fits::new(ayeaye_core::fit::DEFAULT_TTL_MS, None)),
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

/// The first pane id out of an `/api/panes` body, qualified as the server sent
/// it.
///
/// Read back off the wire rather than built by the test, because the id the
/// client hands to `/api/pane` is exactly the id `/api/panes` gave it — and an
/// endpoint that answered about *bare* ids would make `share/app.html`'s
/// `reconcileOv` wipe the overview every two seconds. A test that assembled the
/// id itself could not notice that.
fn first_pane_id(panes_body: &str) -> String {
    let after = panes_body
        .split_once(r#""id":""#)
        .unwrap_or_else(|| panic!("no pane in {panes_body}"))
        .1;
    after
        .split_once('"')
        .expect("an id ends its string")
        .0
        .to_string()
}

/// A field out of a small flat JSON body, without a parser.
fn field(body: &str, name: &str) -> String {
    let after = body
        .split_once(&format!("\"{name}\":"))
        .unwrap_or_else(|| panic!("no {name} in {body}"))
        .1;
    let value = after
        .split_once(|c| c == ',' || c == '}')
        .map_or(after, |(value, _)| value);
    value.trim_matches('"').to_string()
}

// AYEAYE-47 — the terminal view, from the seam a phone reads: a real socket, a
// real tmux, and the id the pane list just handed out. Both shapes, because a
// client that does not send `df=1` is a page cached from before the protocol
// existed and still has to render.
#[tokio::test]
async fn the_terminal_view_answers_about_the_id_the_pane_list_gave() {
    let Some(tmux) = common::Private::named("serve-pane") else {
        eprintln!("skipped: no tmux on this machine");
        return;
    };
    let mut settings = settings_on_port(0);
    settings.tmux = tmux.layer();
    let server = Server::start(settings).await;

    let id = first_pane_id(&server.api("/api/panes").await.body_text());
    assert!(id.starts_with("desktop/%"), "{id} is not qualified");
    let asked = format!("/api/pane?id={}", id.replace('/', "%2F"));

    // No `df`: the whole-text shape, with the grid beside it.
    let whole = server.api(&asked).await;
    assert_eq!(whole.status, 200);
    assert_eq!(whole.header("content-type"), Some("application/json"));
    assert_eq!(whole.header("cache-control"), Some("no-store"));
    let body = whole.body_text();
    assert!(body.starts_with(r#"{"text":"#), "{body}");
    assert!(
        field(&body, "cols").parse::<u16>().expect("a width") > 0,
        "the client renders at the width tmux wrapped for: {body}"
    );

    // `df=1`: the diff shape, with the two tokens.
    let first = server.api(&format!("{asked}&df=1")).await;
    assert_eq!(first.status, 200);
    let first_body = first.body_text();
    assert!(first_body.contains(r#""hh":"#), "{first_body}");
    let (hh, sh) = (field(&first_body, "hh"), field(&first_body, "sh"));
    assert_eq!(hh.len(), 12, "{first_body}");

    // Echo the tokens back, as the poll does. An idle pane costs a header.
    let again = server
        .api(&format!("{asked}&df=1&hh={hh}&sh={sh}"))
        .await
        .body_text();
    assert!(
        again.contains(r#""same":1"#),
        "an unchanged pane should cost nothing: {again}"
    );
}

// AYEAYE-47 — the defence against a forged target is **membership, not
// syntax**. `%0` is a perfectly well-formed tmux pane id and `desktop/%99` is a
// perfectly well-formed qualified one; neither is in the list this machine just
// reported, and neither may reach `capture-pane`.
#[tokio::test]
async fn only_a_pane_this_machine_actually_lists_can_be_read() {
    let Some(tmux) = common::Private::named("serve-forged") else {
        eprintln!("skipped: no tmux on this machine");
        return;
    };
    let mut settings = settings_on_port(0);
    settings.tmux = tmux.layer();
    let server = Server::start(settings).await;

    for asked in [
        // Bare: names no machine at all, and "bare means here" is the
        // conditional qualified ids exist to delete.
        "/api/pane?id=%250",
        // A machine this deployment has never heard of.
        "/api/pane?id=gpu-box%2F%250",
        // This machine, and a pane it does not have.
        "/api/pane?id=desktop%2F%2599",
        // A tmux target that is not a pane id: `work:0.0` resolves to a real
        // pane for tmux and to nothing for the pane list.
        "/api/pane?id=desktop%2Fwork:0.0",
    ] {
        let answer = server.api(asked).await;
        assert_eq!(answer.status, 404, "{asked} was not refused: {}", answer.body_text());
        assert_eq!(answer.body_text(), r#"{"error":"no such pane"}"#);
    }
    // And no id at all is a different mistake, told apart from a wrong one.
    let empty = server.api("/api/pane").await;
    assert_eq!(empty.status, 400);
    assert_eq!(empty.body_text(), r#"{"error":"no pane"}"#);
}

// AYEAYE-47 — both endpoints are gated, and the resize is gated as a POST: it
// resizes somebody's actual terminal, so it is refused without a token whatever
// it names.
#[tokio::test]
async fn the_terminal_view_and_the_resize_need_a_token() {
    let server = Server::started().await;
    assert_eq!(server.get("/api/pane?id=desktop%2F%250").await.status, 401);
    assert_eq!(
        server
            .request("POST", "/api/resize", &[("Content-Length", "0")])
            .await
            .status,
        401
    );
    // A GET of the resize is not the resize, and a POST of the view is not the
    // view: the method is half of what a route means.
    assert_eq!(server.api("/api/resize").await.status, 404);
    assert_eq!(server.post("/api/pane", "{}").await.status, 404);
}

// AYEAYE-47 — the whole lease, over the wire: the phone asks for its width, the
// window takes it, and releasing gives the desktop its window back. The size is
// read from tmux rather than from the answer, because an endpoint that reported
// a resize it never performed would pass a test that only read its own reply.
#[tokio::test]
async fn a_resize_takes_the_lease_and_a_release_gives_the_window_back() {
    let Some(tmux) = common::Private::named("serve-resize") else {
        eprintln!("skipped: no tmux on this machine");
        return;
    };
    let layer = tmux.layer();
    let mut settings = settings_on_port(0);
    settings.tmux = tmux.layer();
    let server = Server::start(settings).await;

    let id = first_pane_id(&server.api("/api/panes").await.body_text());
    let pane = id.split_once('/').expect("a qualified id").1.to_string();

    // Somebody had already sized this window by hand.
    ayeaye::fit::resize(&layer, &pane, 100, 40).await;

    let fitted = server
        .post(
            "/api/resize",
            &format!(r#"{{"pane":"{id}","auto":true,"cols":40,"rows":10}}"#),
        )
        .await;
    assert_eq!(fitted.status, 200);
    assert_eq!(fitted.body_text(), r#"{"ok":true,"cols":40,"rows":10}"#);
    assert_eq!(window_size(&layer, &pane).await, (40, 10));

    let released = server
        .post("/api/resize", &format!(r#"{{"pane":"{id}","release":true}}"#))
        .await;
    assert_eq!(released.body_text(), r#"{"ok":true,"restored":true}"#);
    assert_eq!(
        window_size(&layer, &pane).await,
        (100, 40),
        "the desktop should have its own window back"
    );
    // Releasing again restores nothing: there is no lease left to end.
    let twice = server
        .post("/api/resize", &format!(r#"{{"pane":"{id}","release":true}}"#))
        .await;
    assert_eq!(twice.body_text(), r#"{"ok":true,"restored":false}"#);
}

// AYEAYE-47 — a size nobody typed cannot make a window nobody can use, and a
// body that is not a resize is refused before it acts on anything.
#[tokio::test]
async fn a_preposterous_size_is_clamped_and_a_bad_body_is_refused() {
    let Some(tmux) = common::Private::named("serve-clamp") else {
        eprintln!("skipped: no tmux on this machine");
        return;
    };
    let mut settings = settings_on_port(0);
    settings.tmux = tmux.layer();
    let server = Server::start(settings).await;
    let id = first_pane_id(&server.api("/api/panes").await.body_text());

    let tiny = server
        .post(
            "/api/resize",
            &format!(r#"{{"pane":"{id}","cols":1,"rows":1}}"#),
        )
        .await;
    assert_eq!(tiny.body_text(), r#"{"ok":true,"cols":20,"rows":5}"#);

    let huge = server
        .post(
            "/api/resize",
            &format!(r#"{{"pane":"{id}","cols":99999,"rows":99999}}"#),
        )
        .await;
    assert_eq!(huge.body_text(), r#"{"ok":true,"cols":400,"rows":200}"#);

    for (body, refused) in [
        ("not json at all", r#"{"error":"bad json"}"#),
        ("{}", r#"{"error":"no pane"}"#),
    ] {
        let answer = server.post("/api/resize", body).await;
        assert_eq!(answer.status, 400, "{body}");
        assert_eq!(answer.body_text(), refused);
    }
    let bad_size = server
        .post(
            "/api/resize",
            &format!(r#"{{"pane":"{id}","cols":"wide","rows":10}}"#),
        )
        .await;
    assert_eq!(bad_size.status, 400);
    assert_eq!(bad_size.body_text(), r#"{"error":"bad size"}"#);
}

/// The window this pane is in, as tmux reports it.
async fn window_size(tmux: &ayeaye::tmux::Tmux, pane: &str) -> (u16, u16) {
    let said = tmux
        .ask(&[
            "display-message",
            "-p",
            "-t",
            pane,
            "#{window_width}\t#{window_height}",
        ])
        .await
        .expect("tmux should describe a window it has");
    ayeaye_core::fit::size(&said).unwrap_or_else(|| panic!("not a size: {said:?}"))
}
