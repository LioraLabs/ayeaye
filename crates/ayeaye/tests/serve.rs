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
    // A cliban that is not there by default: no test gets to reach a real
    // board by forgetting to say which one it meant.
    settings_with(port, "/nonexistent/cliban")
}

fn settings_with(port: u16, cliban: &str) -> Settings {
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
        cliban: ayeaye::cliban::Cliban::new(cliban.to_string()),
    }
}

/// A deployment of one machine, under the name a test wants to see.
fn registry(name: &str) -> Registry {
    Registry::new(vec![Peer::here(HostName::new(name).expect("a host name"))])
        .expect("one peer, and it is this machine")
}

/// Where the cliban stand-ins and their argv logs live.
fn scratch() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("serve-stand-ins")
}

/// Where one stand-in records what it was asked.
///
/// One log per stand-in, and one stand-in per test: a single shared log would
/// be appended to by every test running beside this one, and "these are the
/// three questions asked" would be a claim about the whole suite.
fn argv_log(stand_in: &str) -> std::path::PathBuf {
    scratch().join(format!("{stand_in}.argv"))
}

/// The command lines a stand-in was run with, in order.
///
/// Empty when it was never started at all, which is how a test proves that
/// something was refused before any process existed.
fn asked(stand_in: &str) -> Vec<String> {
    std::fs::read_to_string(argv_log(stand_in))
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect()
}

/// Every cliban stand-in, written once before any of them is ever run.
///
/// Never the real cliban: this project is tracked on the board the real one
/// would answer about. Written up front rather than per test because these
/// tests run in parallel threads in one process, and a `fork` in one thread
/// while another still holds the file open for writing is an intermittent
/// `ETXTBSY` on the exec.
///
/// Every one of them records its argv first, so "cliban was never started" is
/// a thing a test can observe rather than infer from a status code.
fn stand_ins() -> &'static std::path::PathBuf {
    static WRITTEN: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
    WRITTEN.get_or_init(|| {
        use std::io::Write;
        use std::os::unix::fs::PermissionsExt;

        let directory = scratch();
        std::fs::create_dir_all(&directory).expect("a writable temporary directory");

        // What a board looks like: one project, one milestone, one issue, plus
        // a blank line and a truncated one, which a real pipe can produce and
        // which must cost their own row and nothing else.
        const ANSWERS: &str = concat!(
            "case \"$1 $2 $3\" in\n",
            "  'project ls --json')   printf '%s\\n' ",
            "'{\"key\":\"AYEAYE\",\"name\":\"AyeAye\"}' '' '{\"key\":\"CLI\"' ",
            "'{\"key\":\"CLI\",\"name\":\"Cliban\"}' ;;\n",
            "  'milestone ls --json') printf '%s\\n' ",
            "'{\"name\":\"One binary\",\"project\":\"AYEAYE\",\"status\":\"open\"}' ;;\n",
            "  'issue ls --json')     printf '%s\\n' ",
            "'{\"key\":\"AYEAYE-53\",\"status\":\"in-progress\"}' ;;\n",
            "  'issue show '*)        printf '%s\\n' ",
            "'{\"key\":\"AYEAYE-53\",\"title\":\"Board integration\"}' ;;\n",
            // Anything else is a question this stand-in was not expecting, and
            // saying so is what makes a dropped `--json` visible rather than
            // silently answered.
            "  *) echo \"unexpected: $*\" >&2; exit 64 ;;\n",
            "esac\n",
        );

        // Answers `project ls` and fails everything after it: the board's
        // "only the first call is a failure" decision has no other witness.
        const HALF: &str = concat!(
            "case \"$1 $2 $3\" in\n",
            "  'project ls --json') printf '%s\\n' ",
            "'{\"key\":\"AYEAYE\",\"name\":\"AyeAye\"}' ;;\n",
            "  *) echo 'cliban: no such view' >&2; exit 1 ;;\n",
            "esac\n",
        );

        for (name, body) in [
            ("board-answers", ANSWERS),
            ("projects-answers", ANSWERS),
            ("issue-answers", ANSWERS),
            ("gated-answers", ANSWERS),
            ("must-not-run", ANSWERS),
            ("half-answers", HALF),
            (
                "fails",
                "echo 'cliban: unable to open database file' >&2\nexit 1",
            ),
            ("garbles", "echo 'not json at all'"),
        ] {
            let path = directory.join(name);
            let log = argv_log(name);
            let _ = std::fs::remove_file(&log);
            let mut file = std::fs::File::create(&path).expect("a written stand-in");
            write!(
                file,
                "#!/bin/sh\necho \"$*\" >> {}\n{body}\n",
                log.to_string_lossy()
            )
            .expect("a written stand-in");
            drop(file);
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
                .expect("a runnable stand-in");
        }
        directory
    })
}

/// A server whose cliban is the named stand-in.
async fn served_by(stand_in: &str) -> Server {
    let program = stand_ins().join(stand_in);
    Server::start(settings_with(0, &program.to_string_lossy())).await
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

// AYEAYE-53 — "board rows are fetched and rendered". The page's `load()` reads
// `d.projects`, `d.milestones` and `d.issues`, so this asserts all three
// arrive; and it asserts the three questions asked of cliban are the daemon's
// own argv, because the sort orders are what put the board in the order the
// page renders and nothing downstream would notice them missing.
#[tokio::test]
async fn the_board_is_fetched_with_the_questions_the_daemon_asks() {
    let server = served_by("board-answers").await;

    let answer = server
        .request("GET", "/api/cliban/board", &[("X-Voice-Token", TOKEN)])
        .await;

    assert_eq!(answer.status, 200);
    assert_eq!(answer.header("content-type"), Some("application/json"));
    assert_eq!(answer.header("cache-control"), Some("no-store"));
    assert_eq!(
        answer.body_text(),
        concat!(
            r#"{"projects":[{"key":"AYEAYE","name":"AyeAye"},"#,
            r#"{"key":"CLI","name":"Cliban"}],"#,
            r#""milestones":[{"name":"One binary","project":"AYEAYE","status":"open"}],"#,
            r#""issues":[{"key":"AYEAYE-53","status":"in-progress"}]}"#,
        ),
        "a blank line and a truncated one should cost their own rows and no more"
    );
    assert_eq!(
        asked("board-answers"),
        vec![
            "project ls --json",
            "milestone ls --json --sort activity",
            "issue ls --json --sort position",
        ]
    );
}

// AYEAYE-53 — "a missing or failing board tool degrades to an empty board with
// a stated reason, never to a broken panel". Both halves: a cliban that fails
// and a cliban that is not there. `load()` in board.html reads `d.error` and
// draws it with a retry, so the body has to parse and the reason has to be in
// `error` — a 500 with an unreadable body is the broken panel.
#[tokio::test]
async fn a_failing_or_missing_cliban_is_a_stated_reason_the_page_can_draw() {
    for (stand_in, expected) in [
        ("fails", "cliban: unable to open database file"),
        ("/nonexistent/cliban", "/nonexistent/cliban"),
    ] {
        let server = if stand_in.starts_with('/') {
            Server::start(settings_with(0, stand_in)).await
        } else {
            served_by(stand_in).await
        };

        let answer = server
            .request("GET", "/api/cliban/board", &[("X-Voice-Token", TOKEN)])
            .await;

        assert_eq!(answer.status, 500, "for {stand_in}");
        let body = answer.body_text();
        assert!(
            ayeaye_core::json::is_value(&body),
            "the degraded body must still parse: {body}"
        );
        let reason = ayeaye_core::json::string_member(&body, "error")
            .unwrap_or_else(|| panic!("no stated reason in {body}"));
        assert!(reason.contains(expected), "{reason}");
    }
}

// AYEAYE-53 — only the *first* of the board's three calls is a failure, which
// is the daemon's `cliban_board` and a decision rather than an oversight: a
// board with no milestones is still a board, where a board with no projects
// has nothing to render at all. Nothing else in the suite can see this — a
// cliban that fails everything only ever exercises the first call.
#[tokio::test]
async fn a_board_whose_later_queries_fail_is_still_a_board() {
    let server = served_by("half-answers").await;

    let answer = server
        .request("GET", "/api/cliban/board", &[("X-Voice-Token", TOKEN)])
        .await;

    assert_eq!(answer.status, 200, "{}", answer.body_text());
    assert_eq!(
        answer.body_text(),
        concat!(
            r#"{"projects":[{"key":"AYEAYE","name":"AyeAye"}],"#,
            r#""milestones":[],"issues":[]}"#,
        )
    );
    // And it did ask, rather than skipping them once the first had answered.
    assert_eq!(asked("half-answers").len(), 3);
}

// AYEAYE-53 — the app page linkifies ticket references from this list, and its
// own fetch swallows a failure into a silent catch. So the contract is that it
// degrades to *no links*: an empty list and a 200, never an error.
#[tokio::test]
async fn the_project_keys_degrade_to_no_links_rather_than_to_an_error() {
    let answered = served_by("projects-answers").await;
    let answer = answered
        .request("GET", "/api/cliban/projects", &[("X-Voice-Token", TOKEN)])
        .await;
    assert_eq!(answer.status, 200);
    assert_eq!(answer.body_text(), r#"{"keys":["AYEAYE","CLI"]}"#);
    // The one question, with the flag that makes the answer parseable. Without
    // `--json` cliban prints a table, every line of which is dropped as a row,
    // and this endpoint degrades to no links with nothing saying why.
    assert_eq!(asked("projects-answers"), vec!["project ls --json"]);

    let absent = Server::started().await;
    let answer = absent
        .request("GET", "/api/cliban/projects", &[("X-Voice-Token", TOKEN)])
        .await;
    assert_eq!(
        answer.status, 200,
        "a missing cliban is fewer links, not 500"
    );
    assert_eq!(answer.body_text(), r#"{"keys":[]}"#);
}

// AYEAYE-53 — the key a request names lands in a subprocess argv, so a key
// that is not shaped like one is refused before anything is started. The 400
// and the untouched argv log are the two halves of that claim.
#[tokio::test]
async fn a_key_that_is_not_a_key_is_refused_before_cliban_is_started() {
    // A stand-in that answers perfectly well and records every call: the 400
    // alone would not say whether the process was started and its answer
    // thrown away.
    let server = served_by("must-not-run").await;

    for key in ["", "--help", "AYEAYE", "AYE%20AYE-1", "AYEAYE-53%0Als"] {
        let answer = server
            .request(
                "GET",
                &format!("/api/cliban/issue?key={key}"),
                &[("X-Voice-Token", TOKEN)],
            )
            .await;
        assert_eq!(answer.status, 400, "for key {key:?}");
        assert_eq!(answer.body_text(), r#"{"error":"bad key"}"#);
    }

    // A key with no `key` parameter at all is the same refusal.
    let answer = server
        .request("GET", "/api/cliban/issue", &[("X-Voice-Token", TOKEN)])
        .await;
    assert_eq!(answer.status, 400);

    assert!(
        asked("must-not-run").is_empty(),
        "cliban was started for a key that is not a key: {:?}",
        asked("must-not-run")
    );
}

// AYEAYE-53 — a real key gets cliban's own object back, and output that is not
// one gets the daemon's "unreadable cliban output" rather than a body the
// page's `.json()` throws on.
#[tokio::test]
async fn a_real_key_gets_the_issue_and_garbled_output_gets_a_reason() {
    let server = served_by("issue-answers").await;
    let answer = server
        .request(
            "GET",
            "/api/cliban/issue?key=AYEAYE-53",
            &[("X-Voice-Token", TOKEN)],
        )
        .await;
    assert_eq!(answer.status, 200);
    // The key reaches cliban as its own argument, and `--json` with it.
    assert_eq!(asked("issue-answers"), vec!["issue show AYEAYE-53 --json"]);
    assert_eq!(
        ayeaye_core::json::string_member(answer.body_text().trim(), "title").as_deref(),
        Some("Board integration")
    );

    let garbled = served_by("garbles").await;
    let answer = garbled
        .request(
            "GET",
            "/api/cliban/issue?key=AYEAYE-53",
            &[("X-Voice-Token", TOKEN)],
        )
        .await;
    assert_eq!(answer.status, 500);
    assert_eq!(
        answer.body_text(),
        r#"{"error":"unreadable cliban output"}"#
    );
}

// AYEAYE-53 — every one of these is under /api/, which AYEAYE-42 already gates
// as one table rather than per route. This is the assertion that says so out
// loud: the endpoints added here needed no gate of their own, and would notice
// if they had been mounted somewhere that bypassed it.
#[tokio::test]
async fn none_of_the_board_endpoints_answer_without_a_token() {
    let server = served_by("gated-answers").await;
    for path in [
        "/api/cliban/board",
        "/api/cliban/projects",
        "/api/cliban/issue?key=AYEAYE-53",
    ] {
        let answer = server.get(path).await;
        assert_eq!(answer.status, 401, "for {path}");
        assert_eq!(answer.body_text(), r#"{"error":"unauthorized"}"#);
    }
    // And nothing was asked of cliban on the way to refusing: a gate that let
    // the work happen and threw the answer away would still be a gate that ran
    // the board's three queries for an unauthenticated caller.
    assert!(
        asked("gated-answers").is_empty(),
        "cliban was run for an unauthenticated request: {:?}",
        asked("gated-answers")
    );
}
