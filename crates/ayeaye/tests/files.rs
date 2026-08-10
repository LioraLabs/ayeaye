//! File references and previews, observed the way a phone observes them: over
//! a socket, against a real tmux whose pane sits in a real git repository.
//!
//! Every test here skips cleanly on a machine with no tmux or no git, in one
//! place, so a missing program is one message rather than a scatter of
//! failures. The repository is the test's own, under `CARGO_TARGET_TMPDIR`,
//! and the tmux is a private server on its own socket — nothing reads the
//! panes or repositories of whoever is running the suite.

mod common;

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, Mutex};

use ayeaye::config::Settings;
use ayeaye::fit::Fits;
use ayeaye_core::http::hosts::AllowedHosts;
use ayeaye_core::peer::{HostName, Peer, Registry};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// The token every test logs in with.
const TOKEN: &str = "test-token-not-a-real-secret";

/// A response, split into what a test asserts on.
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
    async fn start(settings: Settings) -> Server {
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

    /// A gated GET, carrying the token unless the test says otherwise.
    async fn get(&self, path: &str, token: Option<&str>) -> Answer {
        let mut headers: Vec<(&str, &[u8])> = Vec::new();
        if let Some(token) = token {
            headers.push(("X-Voice-Token", token.as_bytes()));
        }
        self.exchange("GET", path, &headers, b"").await
    }

    /// A JSON write, presented the way the panel presents one: same-origin,
    /// with the token. The Origin matters — the CSRF gate judges every POST
    /// before anything reads a body.
    async fn post(&self, path: &str, body: &str, token: Option<&str>) -> Answer {
        let origin = format!("http://127.0.0.1:{}", self.port);
        let length = body.len().to_string();
        let mut headers: Vec<(&str, &[u8])> = vec![
            ("Origin", origin.as_bytes()),
            ("Content-Type", b"application/json"),
            ("Content-Length", length.as_bytes()),
        ];
        if let Some(token) = token {
            headers.push(("X-Voice-Token", token.as_bytes()));
        }
        self.exchange("POST", path, &headers, body.as_bytes()).await
    }

    async fn exchange(
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
        request.extend_from_slice(format!("Host: 127.0.0.1:{}\r\n", self.port).as_bytes());
        for (name, value) in headers {
            request.extend_from_slice(name.as_bytes());
            request.extend_from_slice(b": ");
            request.extend_from_slice(value);
            request.extend_from_slice(b"\r\n");
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
}

/// Split a raw HTTP/1.1 response into its parts.
fn parse(raw: &[u8]) -> Answer {
    let split = raw
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .expect("a header/body boundary");
    let head = String::from_utf8_lossy(&raw[..split]);
    let mut lines = head.lines();
    let status = lines
        .next()
        .expect("a status line")
        .split_whitespace()
        .nth(1)
        .expect("a status code")
        .parse()
        .expect("a numeric status");
    let mut headers = HashMap::new();
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            headers.insert(name.trim().to_lowercase(), value.trim().to_string());
        }
    }
    Answer {
        status,
        headers,
        body: raw[split + 4..].to_vec(),
    }
}

/// Whether this machine has a git for the repository half of the harness.
fn have_git() -> bool {
    Command::new("git").arg("--version").output().is_ok()
}

/// Run git in a directory, asserting it worked: the harness's own git, not
/// the code under test's.
fn git(repo: &PathBuf, args: &[&str]) {
    let ran = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .expect("the harness git should run");
    assert!(
        ran.status.success(),
        "git {args:?} refused: {}",
        String::from_utf8_lossy(&ran.stderr)
    );
}

/// The eight PNG signature bytes and a little padding: enough to be a file
/// whose bytes a test can compare, no image library required.
const PNG_BYTES: &[u8] = &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 1, 2, 3, 4];

/// A repository of the test's own, with a spread of tracked files.
///
/// - `src/main.rs`: 300 numbered lines, for the window assertions.
/// - `notes/notes.md` and `deep/x/y/notes.md`: one name, two places, so the
///   ranking has something to rank.
/// - `shot.png`: bytes to get back.
/// - `linked.md`: a *tracked symlink* — git tracks those happily, which is
///   exactly why the preview cannot lean on the tracked list alone.
/// - `untracked.txt`: on disk and not in the index.
fn repository(what: &str) -> PathBuf {
    let repo = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("files-{what}"));
    let _ = std::fs::remove_dir_all(&repo);
    std::fs::create_dir_all(repo.join("src")).expect("directories");
    std::fs::create_dir_all(repo.join("notes")).expect("directories");
    std::fs::create_dir_all(repo.join("deep/x/y")).expect("directories");
    let main: String = (1..=300).map(|n| format!("line {n}\n")).collect();
    std::fs::write(repo.join("src/main.rs"), main).expect("a text file");
    std::fs::write(repo.join("notes/notes.md"), "near\n").expect("a near file");
    std::fs::write(repo.join("deep/x/y/notes.md"), "far away\n").expect("a far file");
    std::fs::write(repo.join("shot.png"), PNG_BYTES).expect("an image");
    std::fs::write(repo.join("untracked.txt"), "not in the index\n").expect("a stray");
    std::os::unix::fs::symlink("notes/notes.md", repo.join("linked.md")).expect("a symlink");
    git(&repo, &["init", "-q"]);
    // `add` is enough: `ls-files` reads the index, and nothing here commits.
    git(
        &repo,
        &[
            "add",
            "src/main.rs",
            "notes/notes.md",
            "deep/x/y/notes.md",
            "shot.png",
            "linked.md",
        ],
    );
    repo
}

/// A server whose tmux holds one pane working inside the repository, and that
/// pane's id as the server hands it out.
///
/// `None` is a skip: no tmux or no git on this machine.
/// `None` is a skip, and the message is printed here so a missing program is
/// named once, correctly, rather than guessed at by every caller.
async fn harness(what: &str) -> Option<(common::Private, Server, String)> {
    if !have_git() {
        eprintln!("skipped: no git on this machine");
        return None;
    }
    let Some(tmux) = common::Private::named(&format!("files-{what}")) else {
        eprintln!("skipped: no tmux on this machine");
        return None;
    };
    let repo = repository(what);
    // A second window working in the repository. `-P -F` prints the new
    // pane's id, so the test does not have to guess which pane is where.
    let pane = tmux
        .output(&[
            "new-window",
            "-t",
            "work",
            "-c",
            repo.to_str().expect("a UTF-8 repo path"),
            "-P",
            "-F",
            "#{pane_id}",
            "/bin/sh",
        ])?
        .trim()
        .to_string();
    let mut settings = settings_on_port(0);
    settings.tmux = tmux.layer();
    let server = Server::start(settings).await;
    // Qualified as the pane list qualifies it, which is what every request
    // carries back.
    Some((tmux, server, format!("desktop/{pane}")))
}

fn settings_on_port(port: u16) -> Settings {
    Settings {
        bind: "127.0.0.1".to_string(),
        port,
        allowed_hosts: AllowedHosts::new("127.0.0.1", port, ""),
        token: TOKEN.to_string(),
        peers: Registry::new(vec![Peer::here(
            HostName::new("desktop").expect("a host name"),
        )])
        .expect("one peer, and it is this machine"),
        tmux: common::nowhere("files-nobody"),
        agents: ayeaye::session::Agents::under("/nonexistent/home"),
        cliban: ayeaye::cliban::Cliban::new("/nonexistent/cliban".to_string()),
        pane_cache: Arc::new(Mutex::new(ayeaye_core::pane::Cache::default())),
        fits: Arc::new(Fits::new(ayeaye_core::fit::DEFAULT_TTL_MS, None)),
        voice: Arc::new(ayeaye::dictate::Voice::new(
            std::path::PathBuf::from("/nonexistent/store"),
            ayeaye_core::model::settings::ModelSettings::resolve(|_| None, "")
                .expect("the defaults resolve"),
            ayeaye_core::cleanup::Policy::default(),
            "ayeaye-52-no-such-converter".to_string(),
        )),
    }
}

/// One value, percent-encoded the way `encodeURIComponent` encodes it: pane
/// ids carry `%` and `/`, and a server comparing raw text would be looking at
/// different text than the panel sent.
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

/// The preview URL the page builds.
fn preview_url(pane: &str, path: &str, line: Option<usize>) -> String {
    let mut url = format!(
        "/api/files/preview?pane={}&path={}",
        encoded(pane),
        encoded(path)
    );
    if let Some(line) = line {
        url.push_str(&format!("&line={line}"));
    }
    url
}

// AYEAYE-52 — the spec's first criterion, at the seam a phone sees: a
// transcript reference resolves against the pane's repository, ranked with the
// file nearest the pane first, carrying the line for the preview to use.
#[tokio::test]
async fn a_reference_resolves_ranked_against_the_panes_repository() {
    let Some((_tmux, server, pane)) = harness("resolve").await else {
        eprintln!("skipped: no tmux on this machine");
        return;
    };

    let body = format!(
        r#"{{"pane":{},"reference":"`notes.md:2`"}}"#,
        ayeaye_core::json::string(&pane)
    );
    let answer = server.post("/api/files/resolve", &body, Some(TOKEN)).await;
    assert_eq!(answer.status, 200, "{}", answer.body_text());
    let text = answer.body_text();
    // The pane works at the repository root, so `notes/notes.md` is one step
    // away and `deep/x/y/notes.md` three: the near one must come first.
    let near = text.find(r#""path":"notes/notes.md""#).expect("the near file");
    let far = text
        .find(r#""path":"deep/x/y/notes.md""#)
        .expect("the far file");
    assert!(near < far, "the near file must rank first: {text}");
    assert!(text.contains(r#""kind":"text""#), "{text}");
    assert!(text.ends_with(r#""line":2}"#), "the line rides along: {text}");

    // A name the repository does not track resolves to nothing, which the
    // page renders as "no tracked file found" rather than an error.
    let body = format!(
        r#"{{"pane":{},"reference":"nowhere.xyz"}}"#,
        ayeaye_core::json::string(&pane)
    );
    let empty = server.post("/api/files/resolve", &body, Some(TOKEN)).await;
    assert_eq!(empty.status, 200);
    assert_eq!(empty.body_text(), r#"{"candidates":[],"line":null}"#);
}

// AYEAYE-52 — the second criterion: text with bounded context around the
// requested line. 300 lines, line 250 asked for — the window runs to half the
// line budget past the request, hits the end of the file at 300, and keeps
// the last two hundred lines: the region shown is 101..=300 with the
// requested line inside it.
#[tokio::test]
async fn a_text_preview_carries_the_region_around_the_line() {
    let Some((_tmux, server, pane)) = harness("text").await else {
        eprintln!("skipped: no tmux on this machine");
        return;
    };

    let answer = server
        .get(&preview_url(&pane, "src/main.rs", Some(250)), Some(TOKEN))
        .await;
    assert_eq!(answer.status, 200, "{}", answer.body_text());
    assert_eq!(answer.header("content-type"), Some("application/json"));
    let text = answer.body_text();
    assert!(text.contains(r#""kind":"text""#), "{text}");
    assert!(text.contains(r#""start":101"#), "the region starts at 101: {text}");
    assert!(
        text.contains(r#"{"number":250,"text":"line 250\n"}"#),
        "the requested line is in the region: {text}"
    );
    assert!(text.contains(r#"{"number":300,"text":"line 300\n"}"#), "{text}");
    assert!(!text.contains(r#""number":100,"#), "bounded above: {text}");
    assert!(text.ends_with(r#""line":250}"#), "{text}");

    // Without a line: the file from its start.
    let answer = server
        .get(&preview_url(&pane, "src/main.rs", None), Some(TOKEN))
        .await;
    let text = answer.body_text();
    assert!(text.contains(r#""start":1"#), "{text}");
    assert!(text.contains(r#"{"number":1,"text":"line 1\n"}"#), "{text}");
    assert!(
        !text.contains(r#""number":201,"#),
        "two hundred lines and no more: {text}"
    );
}

// AYEAYE-52 — the spec's "including images": the bytes come back as
// themselves, labelled by extension, inline and nosniffed, never cached.
#[tokio::test]
async fn an_image_preview_is_the_bytes_with_the_right_type() {
    let Some((_tmux, server, pane)) = harness("image").await else {
        eprintln!("skipped: no tmux on this machine");
        return;
    };

    let answer = server
        .get(&preview_url(&pane, "shot.png", None), Some(TOKEN))
        .await;
    assert_eq!(answer.status, 200);
    assert_eq!(answer.header("content-type"), Some("image/png"));
    assert_eq!(answer.header("content-disposition"), Some("inline"));
    assert_eq!(answer.header("x-content-type-options"), Some("nosniff"));
    assert_eq!(answer.header("cache-control"), Some("no-store"));
    assert_eq!(answer.body, PNG_BYTES, "the bytes are the file's own");
}

// AYEAYE-52 — the decisions: preview is limited to tracked files, and the
// walk refuses links even when git tracks them. A file on disk beside the
// tracked ones is not previewable, and neither is a tracked symlink — the
// two refusals have different mechanisms and the same 404.
#[tokio::test]
async fn only_plain_tracked_files_preview() {
    let Some((_tmux, server, pane)) = harness("tracked").await else {
        eprintln!("skipped: no tmux on this machine");
        return;
    };

    // On disk, not in the index.
    let stray = server
        .get(&preview_url(&pane, "untracked.txt", None), Some(TOKEN))
        .await;
    assert_eq!(stray.status, 404, "{}", stray.body_text());

    // In the index, and a link: the descriptor walk is what refuses it.
    let linked = server
        .get(&preview_url(&pane, "linked.md", None), Some(TOKEN))
        .await;
    assert_eq!(linked.status, 404, "{}", linked.body_text());

    // And the file the link points at previews fine by its own name.
    let real = server
        .get(&preview_url(&pane, "notes/notes.md", None), Some(TOKEN))
        .await;
    assert_eq!(real.status, 200, "{}", real.body_text());
}

// AYEAYE-52 — a path that could never name a tracked file is refused before
// anything looks for it, and a nonsense line is a bad request rather than a
// scan. Both are 400: the caller did something wrong, not the repository.
#[tokio::test]
async fn escapes_and_nonsense_are_bad_requests() {
    let Some((_tmux, server, pane)) = harness("refusals").await else {
        eprintln!("skipped: no tmux on this machine");
        return;
    };

    for path in ["../outside.txt", "/etc/passwd", "a/./b", "src//main.rs"] {
        let answer = server
            .get(&preview_url(&pane, path, None), Some(TOKEN))
            .await;
        assert_eq!(answer.status, 400, "{path} must be refused as a bad path");
        assert_eq!(answer.body_text(), r#"{"error":"bad path"}"#);
    }
    for line in ["0", "007", "abc", "12345678901"] {
        let url = format!(
            "/api/files/preview?pane={}&path=src%2Fmain.rs&line={line}",
            encoded(&pane)
        );
        let answer = server.get(&url, Some(TOKEN)).await;
        assert_eq!(answer.status, 400, "line={line} must be refused");
        assert_eq!(answer.body_text(), r#"{"error":"line must be positive"}"#);
    }

    // A pane nobody offered answers as a search that found nothing on the
    // resolve, and a 404 on the preview — the daemon's shapes, and neither
    // says anything about what exists.
    let forged = server
        .get(&preview_url("desktop/%9999", "src/main.rs", None), Some(TOKEN))
        .await;
    assert_eq!(forged.status, 404);
    let body = r#"{"pane":"desktop/%9999","reference":"notes.md"}"#;
    let empty = server.post("/api/files/resolve", body, Some(TOKEN)).await;
    assert_eq!(empty.status, 200);
    assert_eq!(empty.body_text(), r#"{"candidates":[],"line":null}"#);
}

// AYEAYE-52 — both endpoints are gated: no token, no answer, whatever the
// request names. The route table promises this; a socket proves it.
#[tokio::test]
async fn without_a_token_both_endpoints_refuse() {
    // No tmux and no repository needed: the gate sits above everything.
    let server = Server::start(settings_on_port(0)).await;

    let preview = server
        .get("/api/files/preview?pane=x&path=y.txt", None)
        .await;
    assert_eq!(preview.status, 401);

    let resolve = server
        .post(
            "/api/files/resolve",
            r#"{"pane":"x","reference":"y.txt"}"#,
            None,
        )
        .await;
    assert_eq!(resolve.status, 401);
}
