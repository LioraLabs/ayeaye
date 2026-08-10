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
use std::time::Duration;

use ayeaye::config::Settings;
use ayeaye::fit::Fits;
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

    /// A GET carrying the token, which is what every `/api/` call needs.
    async fn api(&self, path: &str) -> Answer {
        self.request("GET", path, &[("X-Voice-Token", TOKEN)]).await
    }

    /// A clip of audio, posted the way the page posts one.
    ///
    /// Not JSON: `/api/dictate` is the one endpoint whose body is the data
    /// itself, with the pane and the container in the query.
    async fn clip(&self, path: &str, body: &str) -> Answer {
        let origin = format!("http://127.0.0.1:{}", self.port);
        let length = body.len().to_string();
        self.request_with_body(
            "POST",
            path,
            &[
                ("Origin", origin.as_bytes()),
                ("X-Voice-Token", TOKEN.as_bytes()),
                ("Content-Type", b"application/octet-stream"),
                ("Content-Length", length.as_bytes()),
            ],
            body.as_bytes(),
        )
        .await
    }

    /// A write, presented the way the panel presents one.
    ///
    /// The `Origin` is this server's own and the token is the test token,
    /// because AYEAYE-69's CSRF gate judges every non-GET before anything reads
    /// a body: a write test that sent neither would be refused at that gate and
    /// would look exactly like the endpoint being broken.
    async fn write(&self, path: &str, body: &str) -> Answer {
        self.post_as_us(path, body).await
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
        // Pointed at a home nobody has, for the same reason the tmux above
        // points at a socket nobody is on: a test that does not care about
        // sessions must not read the ones belonging to whoever is running the
        // suite.
        agents: ayeaye::session::Agents::under("/nonexistent/home"),
        cliban: ayeaye::cliban::Cliban::new(cliban.to_string()),
        pane_cache: Arc::new(Mutex::new(ayeaye_core::pane::Cache::default())),
        // No path: a test must never write into the state directory of whoever
        // is running the suite. Recovery across a restart is proved in
        // `tests/fit.rs`, where the file is the test's own.
        fits: Arc::new(Fits::new(ayeaye_core::fit::DEFAULT_TTL_MS, None)),
        // No models and a converter that is not there, so nothing here can load
        // a model or start a process by accident. The cases that care about
        // voice say so.
        voice: Arc::new(ayeaye::dictate::Voice::new(
            std::path::PathBuf::from("/nonexistent/store"),
            ayeaye_core::model::settings::ModelSettings::resolve(|_| None, "")
                .expect("the defaults resolve"),
            ayeaye_core::cleanup::Policy::default(),
            "ayeaye-58-no-such-converter".to_string(),
        )),
        // No path, for the reason `fits` has none: a test must never write
        // into the pick history of whoever is running the suite. The case
        // that proves spawn teaches the picker points this at its own file.
        store: None,
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
fn stand_in_root() -> &'static std::path::Path {
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
            // Where it records is `$AYEAYE_51_ARGV`, set per test through the
            // pane's own environment. One shared log would be appended to by
            // whichever spawn test ran beside this one -- and the tests run as
            // threads in one process, so "what the agent was given" would
            // become a claim about the whole file rather than about this test.
            "#!/bin/sh\nprintf '%s\\n' \"$#\" \"$@\" > \"$AYEAYE_51_ARGV\"\n",
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
async fn recorded_argv(log: &std::path::Path) -> Vec<String> {
    for _ in 0..200 {
        if let Ok(text) = std::fs::read_to_string(log) {
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
    let log = stand_in_root().join("argv-spawn");
    tmux.tmux(&[
        "set-option",
        "-g",
        "default-command",
        &format!(
            "PATH={} AYEAYE_51_ARGV={} /bin/sh",
            stand_in_root().join("bin").display(),
            log.display()
        ),
    ]);

    let project = stand_in_root().join("ayeaye-51-project");
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
    let argv = recorded_argv(&log).await;
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

// AYEAYE-51 — the second agent for a project joins the first one's session
// rather than starting a second, and it does so **by that session's id**.
//
// Found at the final gate. A tmux target is a grammar — `session:window.pane`,
// with a leading `$` meaning a session id — so a session *name* used as a
// target is a directory's name being read as syntax. This is the case that
// proves it: the machine already has a session whose tmux id is `$0`, and the
// project is a directory literally called `$0`. Targeting by name sends the
// second window into `important-work`; targeting by id sends it where it
// belongs. Verified against a real tmux before it was written.
#[tokio::test]
async fn a_second_agent_joins_its_own_session_even_when_the_project_is_named_like_a_target() {
    let Some(tmux) = common::Private::named("serve-spawn-twice") else {
        eprintln!("skipped: no tmux on this machine");
        return;
    };
    tmux.tmux(&[
        "set-option",
        "-g",
        "default-command",
        &format!(
            "PATH={} AYEAYE_51_ARGV={} /bin/sh",
            stand_in_root().join("bin").display(),
            stand_in_root().join("argv-twice").display()
        ),
    ]);
    // The session that must not gain a window. It is the first session on this
    // server, so tmux calls it `$0` — which is also what the project below is
    // called.
    let hijackable = tmux
        .output(&["list-sessions", "-F", "#{session_name}\t#{session_id}"])
        .expect("the private server's own session");
    assert!(
        hijackable.contains("work\t$0"),
        "this test needs `work` to be session $0: {hijackable}"
    );

    let project = stand_in_root().join("$0");
    std::fs::create_dir_all(&project).expect("a project directory");
    let body = format!(
        r#"{{"dir":{},"agent":"claude"}}"#,
        serde_json::to_string(&project.to_string_lossy().into_owned()).unwrap()
    );

    let mut settings = settings_on_port(0);
    settings.tmux = tmux.layer();
    let server = Server::start(settings).await;

    let first = server.post_as_us("/api/spawn", &body).await;
    assert_eq!(first.status, 200, "{}", first.body_text());
    assert!(
        first.body_text().contains(r#""created":"session""#),
        "{}",
        first.body_text()
    );

    let second = server.post_as_us("/api/spawn", &body).await;
    assert_eq!(second.status, 200, "{}", second.body_text());
    assert!(
        second.body_text().contains(r#""created":"window""#),
        "the second agent should join the first one's session: {}",
        second.body_text()
    );

    // Where the windows actually are. Asked of tmux by listing everything, not
    // by naming a session — naming one is the very thing under test.
    let windows = tmux
        .output(&["list-windows", "-a", "-F", "#{session_name}"])
        .expect("the window list");
    let in_session = |name: &str| windows.lines().filter(|line| *line == name).count();
    assert_eq!(
        in_session("work"),
        1,
        "session `work` gained a window that belongs to the project: {windows}"
    );
    assert_eq!(
        in_session("$0"),
        2,
        "both agents should be in the project's own session: {windows}"
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
    let project = stand_in_root().join("ayeaye-51-unquotable");
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

    // And the host half is matched the way the registry matches it. `route`
    // folds case because DNS does — AYEAYE-43 has a test insisting on it — so a
    // membership check that compared whole ids would route `DESKTOP/%1` to this
    // machine and then answer "no such pane" about it, with the two layers
    // disagreeing about how many machines there are.
    let (survivor, _) = after.first().expect("one pane left").clone();
    let shouted = survivor.replace("desktop/", "DESKTOP/");
    assert_ne!(
        shouted, survivor,
        "this asserts nothing unless the case moved"
    );
    let answer = server
        .post_as_us("/api/kill", &format!(r#"{{"pane":"{shouted}"}}"#))
        .await;
    assert_eq!(answer.status, 200, "{}", answer.body_text());
    assert!(listed_panes(&server).await.is_empty());
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
        .split_once([',', '}'])
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

    // Echo the tokens back, as the poll does. An **idle** pane costs a header.
    //
    // Polled until it settles rather than exactly twice, because the shell in
    // this pane is still drawing its prompt when the first request lands: a
    // single second poll asserts "same" against a pane that genuinely changed
    // between the two, and fails for the one reason that is not the claim. The
    // property under test is about a pane that is not changing, so waiting for
    // one is part of stating it — and a server that never answered `same` still
    // fails, because the loop runs out.
    let (mut hh, mut sh) = (hh, sh);
    let mut last = String::new();
    for _ in 0..40 {
        last = server
            .api(&format!("{asked}&df=1&hh={hh}&sh={sh}"))
            .await
            .body_text();
        if last.contains(r#""same":1"#) {
            break;
        }
        hh = field(&last, "hh");
        sh = field(&last, "sh");
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    assert!(
        last.contains(r#""same":1"#),
        "an unchanged pane should cost nothing: {last}"
    );
    // And `same` really is all of it: no lines, no screen, nothing to render.
    assert!(
        !last.contains(r#""screen":"#) && !last.contains(r#""hist":"#),
        "a matching poll must carry no text at all: {last}"
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
        assert_eq!(
            answer.status,
            404,
            "{asked} was not refused: {}",
            answer.body_text()
        );
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
    assert_eq!(server.post_as_us("/api/pane", "{}").await.status, 404);
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
        .post_as_us(
            "/api/resize",
            &format!(r#"{{"pane":"{id}","auto":true,"cols":40,"rows":10}}"#),
        )
        .await;
    assert_eq!(fitted.status, 200);
    assert_eq!(fitted.body_text(), r#"{"ok":true,"cols":40,"rows":10}"#);
    assert_eq!(window_size(&layer, &pane).await, (40, 10));

    let released = server
        .post_as_us(
            "/api/resize",
            &format!(r#"{{"pane":"{id}","release":true}}"#),
        )
        .await;
    assert_eq!(released.body_text(), r#"{"ok":true,"restored":true}"#);
    assert_eq!(
        window_size(&layer, &pane).await,
        (100, 40),
        "the desktop should have its own window back"
    );
    // Releasing again restores nothing: there is no lease left to end.
    let twice = server
        .post_as_us(
            "/api/resize",
            &format!(r#"{{"pane":"{id}","release":true}}"#),
        )
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
        .post_as_us(
            "/api/resize",
            &format!(r#"{{"pane":"{id}","cols":1,"rows":1}}"#),
        )
        .await;
    assert_eq!(tiny.body_text(), r#"{"ok":true,"cols":20,"rows":5}"#);

    let huge = server
        .post_as_us(
            "/api/resize",
            &format!(r#"{{"pane":"{id}","cols":99999,"rows":99999}}"#),
        )
        .await;
    assert_eq!(huge.body_text(), r#"{"ok":true,"cols":400,"rows":200}"#);

    for (body, refused) in [
        ("not json at all", r#"{"error":"bad json"}"#),
        ("{}", r#"{"error":"no pane"}"#),
    ] {
        let answer = server.post_as_us("/api/resize", body).await;
        assert_eq!(answer.status, 400, "{body}");
        assert_eq!(answer.body_text(), refused);
    }
    let bad_size = server
        .post_as_us(
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

// AYEAYE-47 — "the pane poll renews the lease as a side effect of watching",
// and the sweeper is what ends it when the watching stops. Both halves at once,
// through the running server: the lease outlives several TTLs while the terminal
// is being polled, and dies on its own once the polling stops. Nothing here
// calls `Fits::sweep` — the sweeper `server::serve` spawned is what has to
// notice, or the claim is about a function nobody runs in production.
#[tokio::test]
async fn watching_the_terminal_holds_the_fit_and_stopping_lets_it_go() {
    let Some(tmux) = common::Private::named("serve-lease") else {
        eprintln!("skipped: no tmux on this machine");
        return;
    };
    let layer = tmux.layer();
    let mut settings = settings_on_port(0);
    settings.tmux = tmux.layer();
    // A lease that runs out in a third of a second, so this proves expiry in
    // real time without waiting out the production twelve.
    settings.fits = Arc::new(Fits::new(300, None));
    let fits = Arc::clone(&settings.fits);
    let server = Server::start(settings).await;

    let id = first_pane_id(&server.api("/api/panes").await.body_text());
    let pane = id.split_once('/').expect("a qualified id").1.to_string();
    let asked = format!("/api/pane?id={}", id.replace('/', "%2F"));
    ayeaye::fit::resize(&layer, &pane, 100, 40).await;

    server
        .post_as_us(
            "/api/resize",
            &format!(r#"{{"pane":"{id}","auto":true,"cols":40,"rows":10}}"#),
        )
        .await;
    let held = ayeaye_core::peer::PaneId::parse(&id).expect("a qualified id");
    assert!(fits.holds(&held), "the resize should have taken a lease");

    // Watched for well over three lease lifetimes. The sweeper is running the
    // whole time and must not take a window somebody is looking at.
    for _ in 0..10 {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert_eq!(server.api(&asked).await.status, 200);
    }
    assert!(
        fits.holds(&held),
        "watching the pane is what holds the fit, and it did not"
    );
    assert_eq!(window_size(&layer, &pane).await, (40, 10));

    // Then the phone goes into a pocket. Nobody releases anything.
    tokio::time::sleep(std::time::Duration::from_millis(900)).await;
    assert!(
        !fits.holds(&held),
        "an unwatched lease has to expire on its own"
    );
    assert_eq!(
        window_size(&layer, &pane).await,
        (100, 40),
        "the sweeper should have given the desktop its window back"
    );
}

// AYEAYE-47 — the resize acts on a pane, so membership matters there at least as
// much as on the read. Every id here is well-formed and none of them is a pane
// this machine listed.
#[tokio::test]
async fn only_a_pane_this_machine_lists_can_be_resized() {
    let Some(tmux) = common::Private::named("serve-resize-forged") else {
        eprintln!("skipped: no tmux on this machine");
        return;
    };
    let layer = tmux.layer();
    let mut settings = settings_on_port(0);
    settings.tmux = tmux.layer();
    let server = Server::start(settings).await;
    let id = first_pane_id(&server.api("/api/panes").await.body_text());
    let pane = id.split_once('/').expect("a qualified id").1.to_string();
    ayeaye::fit::resize(&layer, &pane, 100, 40).await;

    for forged in ["%0", "gpu-box/%0", "desktop/%99", "desktop/work:0.0"] {
        for shape in [
            format!(r#"{{"pane":"{forged}","auto":true,"cols":40,"rows":10}}"#),
            format!(r#"{{"pane":"{forged}","release":true}}"#),
            format!(r#"{{"pane":"{forged}","restore":true}}"#),
        ] {
            let answer = server.post_as_us("/api/resize", &shape).await;
            assert_eq!(answer.status, 404, "{shape} was not refused");
            assert_eq!(answer.body_text(), r#"{"error":"no such pane"}"#);
        }
    }
    // And nothing reached tmux: the one real window is the size it was.
    assert_eq!(window_size(&layer, &pane).await, (100, 40));
}

// AYEAYE-47 — the manual escape hatch. `restore` hands sizing back to tmux
// outright, whatever the window was before, and drops the lease — because a
// lease still holding the old state would undo the user's choice the moment it
// expired. It is the one branch that ignores the recorded state, so it is the
// one most able to lose somebody's manual sizing by accident.
#[tokio::test]
async fn restore_hands_sizing_back_to_tmux_and_leaves_no_lease() {
    let Some(tmux) = common::Private::named("serve-restore") else {
        eprintln!("skipped: no tmux on this machine");
        return;
    };
    let layer = tmux.layer();
    let mut settings = settings_on_port(0);
    settings.tmux = tmux.layer();
    settings.fits = Arc::new(Fits::new(300, None));
    let fits = Arc::clone(&settings.fits);
    let server = Server::start(settings).await;

    let id = first_pane_id(&server.api("/api/panes").await.body_text());
    let pane = id.split_once('/').expect("a qualified id").1.to_string();
    let held = ayeaye_core::peer::PaneId::parse(&id).expect("a qualified id");

    ayeaye::fit::resize(&layer, &pane, 100, 40).await;
    server
        .post_as_us(
            "/api/resize",
            &format!(r#"{{"pane":"{id}","auto":true,"cols":40,"rows":10}}"#),
        )
        .await;
    assert!(fits.holds(&held));

    let restored = server
        .post_as_us(
            "/api/resize",
            &format!(r#"{{"pane":"{id}","restore":true}}"#),
        )
        .await;
    assert_eq!(restored.status, 200);
    assert_eq!(restored.body_text(), r#"{"ok":true,"restored":true}"#);
    assert!(!fits.holds(&held), "the lease has to go with the sizing");
    assert!(
        !window_is_manual(&layer, &pane).await,
        "sizing should be tmux's again, not pinned to the recorded size"
    );

    // And it stays that way: an expired lease that had survived would put the
    // window back to 100x40 a third of a second from now.
    tokio::time::sleep(std::time::Duration::from_millis(900)).await;
    assert!(!window_is_manual(&layer, &pane).await);
}

/// Whether tmux says this window is sized by hand.
async fn window_is_manual(tmux: &ayeaye::tmux::Tmux, pane: &str) -> bool {
    let shown = tmux
        .ask(&["show-options", "-w", "-t", pane, "window-size"])
        .await
        .unwrap_or_default();
    ayeaye_core::fit::manual(&shown)
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

// AYEAYE-45 — a request naming no pane at all. `kind` is always there and is
// null, so the page can tell "not an agent" from "something went wrong"
// without reading a status code.
#[tokio::test]
async fn the_session_endpoint_answers_a_request_that_names_no_pane() {
    let server = Server::started().await;
    let answer = server
        .request("GET", "/api/session", &[("X-Voice-Token", TOKEN)])
        .await;
    assert_eq!(answer.status, 200);
    assert_eq!(answer.body_text(), r#"{"kind":null}"#);
}

// AYEAYE-45 — an id shaped like a tmux target, or like nothing at all, is
// *answered* rather than refused: the panel asks about whatever is selected,
// and a pane that has just closed is a race rather than a caller doing
// something wrong. This says nothing about membership on its own — the server
// here has no panes at all — and the test that does is
// `a_pane_the_list_excludes_is_never_a_target` in `tests/session.rs`, which
// needs a real tmux to have a real pane to exclude.
#[tokio::test]
async fn an_odd_pane_id_is_answered_rather_than_refused() {
    let server = Server::started().await;
    for pane in [
        "desktop/%99",
        "desktop/%0",
        // Shaped like a target and not one. The list is what refuses these,
        // and a pattern over the id would not be a substitute.
        "desktop/work:0.0",
        "desktop/-X",
        "%0",
        "../../etc/passwd",
    ] {
        let answer = server
            .request(
                "GET",
                &format!("/api/session?pane={}", urlencode(pane)),
                &[("X-Voice-Token", TOKEN)],
            )
            .await;
        assert_eq!(answer.status, 200, "{pane}");
        assert_eq!(answer.body_text(), r#"{"kind":null}"#, "{pane}");
    }
}

// AYEAYE-45 — it is an `/api/` path, so it is gated like every other one. This
// is what an endpoint mounted on the router with `.route(…)` would skip, and
// nothing else in the suite would notice.
#[tokio::test]
async fn the_session_endpoint_is_gated_like_the_rest_of_the_api() {
    let server = Server::started().await;
    let anonymous = server.get("/api/session?pane=desktop/%250").await;
    assert_eq!(anonymous.status, 401);
    assert_eq!(anonymous.body_text(), r#"{"error":"unauthorized"}"#);
}

/// Percent-encode the characters a pane id can carry that a query cannot.
fn urlencode(text: &str) -> String {
    form_urlencoded::byte_serialize(text.as_bytes()).collect()
}

/// The same settings, with a voice of the caller's own.
///
/// Voice is the one thing on `Settings` that can start a process and load a
/// model, so it is spelled out per test rather than shared: a case that does not
/// mention it gets one that can do neither.
fn settings_with_voice(port: u16, voice: ayeaye::dictate::Voice) -> Settings {
    Settings {
        voice: Arc::new(voice),
        ..settings_on_port(port)
    }
}

/// A model store of this test's own, removed when it goes out of scope.
struct Store(std::path::PathBuf);

impl Store {
    fn named(what: &str) -> Store {
        let path = scratch().join(format!("store-{what}"));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("a store");
        Store(path)
    }

    /// Put a model in it, as a pull would have.
    fn holding(self, id: &str) -> Store {
        let id = ayeaye_core::model::ModelId::parse(id).expect("a well-formed id");
        std::fs::create_dir_all(self.0.join(id.relative_dir())).expect("a model directory");
        self
    }
}

impl Drop for Store {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A voice pointed at a store, with a converter that is really there or a name
/// that is not.
fn voice(store: &Store, speech: Option<&str>, converter: &str) -> ayeaye::dictate::Voice {
    let file = speech
        .map(|id| format!("AYEAYE_SPEECH_MODEL={id}\n"))
        .unwrap_or_default();
    ayeaye::dictate::Voice::new(
        store.0.clone(),
        ayeaye_core::model::settings::ModelSettings::resolve(|_| None, &file)
            .expect("a readable configuration"),
        ayeaye_core::cleanup::Policy::default(),
        converter.to_string(),
    )
}

// AYEAYE-58
//
// The capability probe, over a socket. It is what the page reads to decide
// whether the talk button is live, so it has to be honest in both directions: a
// machine with nothing says so and names the next thing to do, and a machine
// with a model and a converter says it can dictate.
#[tokio::test]
async fn the_capability_probe_says_what_is_missing_and_what_is_not() {
    let bare = Store::named("probe-bare");
    let server = Server::start(settings_with_voice(
        0,
        voice(&bare, None, "ayeaye-58-no-such-converter"),
    ))
    .await;

    let answer = server
        .request("GET", "/api/voice", &[("X-Voice-Token", TOKEN)])
        .await;
    assert_eq!(answer.status, 200);
    assert!(
        answer.body_text().contains(r#""voice":false"#),
        "{}",
        answer.body_text()
    );
    assert!(
        answer.body_text().contains("converter"),
        "{}",
        answer.body_text()
    );

    let ready = Store::named("probe-ready").holding("openai/whisper-small.en");
    let server = Server::start(settings_with_voice(
        0,
        voice(&ready, Some("openai/whisper-small.en"), "sh"),
    ))
    .await;
    let answer = server
        .request("GET", "/api/voice", &[("X-Voice-Token", TOKEN)])
        .await;
    assert!(
        answer.body_text().contains(r#""voice":true"#),
        "{}",
        answer.body_text()
    );
    assert!(
        answer.body_text().contains(r#""why":null"#),
        "{}",
        answer.body_text()
    );
}

// AYEAYE-58
//
// Both voice paths are gated. A POST here starts a model and reads a pane, and
// the probe reports what somebody has installed on their machine.
#[tokio::test]
async fn neither_voice_endpoint_answers_without_a_token() {
    let server = Server::started().await;

    for (method, path) in [("GET", "/api/voice"), ("POST", "/api/dictate")] {
        let answer = server.request(method, path, &[]).await;
        assert_eq!(answer.status, 401, "{method} {path}");
        assert!(answer.body_text().contains("unauthorized"));
    }
}

// AYEAYE-58
//
// A server with no voice refuses up front rather than failing partway down the
// pipeline. The page shows the reason, so it has to be the next thing to do
// rather than "voice not configured".
#[tokio::test]
async fn a_server_with_no_voice_refuses_a_clip_and_says_what_to_do_about_it() {
    let bare = Store::named("dictate-bare");
    let server = Server::start(settings_with_voice(
        0,
        voice(&bare, None, "ayeaye-58-no-such-converter"),
    ))
    .await;

    let answer = server.clip("/api/dictate", "not really audio").await;

    assert_eq!(answer.status, 503);
    let body = answer.body_text();
    assert!(body.contains(r#""error""#), "{body}");
    assert!(body.contains("converter"), "{body}");
}

// AYEAYE-58
//
// A model chosen and never pulled is the state a machine is in between two
// commands. It is a refusal that names the model, not a stack trace and not a
// silent empty transcription.
#[tokio::test]
async fn a_model_that_was_chosen_and_never_pulled_is_named_in_the_refusal() {
    let store = Store::named("dictate-unpulled");
    let server = Server::start(settings_with_voice(
        0,
        voice(&store, Some("openai/whisper-small.en"), "sh"),
    ))
    .await;

    let answer = server.clip("/api/dictate", "not really audio").await;

    assert_eq!(answer.status, 503);
    assert!(
        answer.body_text().contains("whisper-small.en"),
        "{}",
        answer.body_text()
    );
}

// AYEAYE-58
//
// A container this build does not read is refused, and the answer says which
// one it was — the client chose it, and it is the thing to change.
#[tokio::test]
async fn a_container_this_build_does_not_read_is_refused_over_the_socket() {
    let store = Store::named("dictate-ext").holding("openai/whisper-small.en");
    let server = Server::start(settings_with_voice(
        0,
        voice(&store, Some("openai/whisper-small.en"), "sh"),
    ))
    .await;

    let answer = server
        .clip("/api/dictate?ext=exe", "not really audio")
        .await;

    assert_eq!(answer.status, 400);
    assert!(answer.body_text().contains("exe"), "{}", answer.body_text());
}

// AYEAYE-58
//
// A cross-site POST is refused before a byte of the clip is read, which is the
// property the handler's ordering exists for: an attacker's thirty megabytes
// must not be buffered on the way to a 403.
#[tokio::test]
async fn a_cross_site_clip_is_refused_before_it_is_read() {
    let server = Server::started().await;

    let answer = server
        .request_with_body(
            "POST",
            "/api/dictate",
            &[
                ("X-Voice-Token", TOKEN.as_bytes()),
                ("Origin", b"https://evil.example"),
                ("Sec-Fetch-Site", b"cross-site"),
                ("Content-Length", b"16"),
            ],
            b"not really audio",
        )
        .await;

    assert_eq!(answer.status, 403);
    assert!(answer.body_text().contains("forbidden"));
}

// AYEAYE-58
//
// The names come off the pane the request named, through the same membership
// check every other pane-shaped endpoint makes. A pane this machine does not
// list contributes nothing rather than refusing the dictation: the hint is an
// improvement to the spelling, and losing it must not cost somebody their words.
#[tokio::test]
async fn the_names_are_read_off_the_pane_the_request_named() {
    if !common::have_tmux() {
        return;
    }
    let Some(server) = Private::named("serve-vocabulary") else {
        return;
    };
    server
        .tmux(&["send-keys", "-t", "work", "-l", "--", "ls parse_config.py"])
        .expect("the test's own tmux should take the text");
    server
        .tmux(&["send-keys", "-t", "work", "Enter"])
        .expect("and submit it");

    let settings = Settings {
        tmux: server.layer(),
        ..settings_on_port(0)
    };
    let running = Server::start(settings.clone()).await;
    let panes = running.api("/api/panes").await.body_text();
    let pane = first_pane_id(&panes);

    let names = ayeaye::server::pane_vocabulary(&settings, &pane).await;
    assert!(
        names.contains("parse_config.py"),
        "the identifier on the screen is missing from {names:?}"
    );

    // A pane on another machine, and a pane that is not there at all: no names,
    // and no refusal.
    assert_eq!(
        ayeaye::server::pane_vocabulary(&settings, "elsewhere/%0").await,
        ""
    );
    assert_eq!(
        ayeaye::server::pane_vocabulary(&settings, "desktop/%99").await,
        ""
    );
    assert_eq!(ayeaye::server::pane_vocabulary(&settings, "").await, "");
}
