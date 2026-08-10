//! The picker, over a socket.
//!
//! Its own test binary, so the roots it walks are set once for this process
//! and cannot leak into another test's environment — and so the tree it walks
//! is one this test made, never somebody's home directory.

mod common;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use ayeaye::config::Settings;
use ayeaye::fit::Fits;
use ayeaye_core::http::hosts::AllowedHosts;
use ayeaye_core::peer::{HostName, Peer, Registry};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

const TOKEN: &str = "test-token-not-a-real-secret";

/// A tree of this test's own, removed when it is done.
struct TempTree {
    path: std::path::PathBuf,
}

impl TempTree {
    fn new() -> TempTree {
        let path = std::env::temp_dir().join(format!("ayeaye-50-picker-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("a temporary directory");
        TempTree { path }
    }

    fn dir(&self, relative: &str) {
        std::fs::create_dir_all(self.path.join(relative)).expect("a directory");
    }

    fn file(&self, relative: &str, contents: &str) {
        std::fs::write(self.path.join(relative), contents).expect("a file");
    }
}

impl Drop for TempTree {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

struct Answer {
    status: u16,
    body: String,
}

async fn request(port: u16, path: &str, headers: &[(&str, &str)]) -> Answer {
    let mut stream = TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("the server should be listening");
    let mut request = format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n");
    for (name, value) in headers {
        request.push_str(&format!("{name}: {value}\r\n"));
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

    let split = raw
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .expect("a response should have a header block");
    let head = String::from_utf8_lossy(&raw[..split]).into_owned();
    let status = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse().ok())
        .expect("a status code");
    Answer {
        status,
        body: String::from_utf8_lossy(&raw[split + 4..]).into_owned(),
    }
}

/// The one row the page would draw, read out of the body.
fn rows(body: &str) -> Vec<HashMap<String, String>> {
    let read = ayeaye_core::projects::json::parse(body).expect("the page parses this");
    let ayeaye_core::projects::json::Value::Array(projects) =
        read.get("projects").expect("a projects array").clone()
    else {
        panic!("projects is not an array: {body}");
    };
    projects
        .iter()
        .map(|project| {
            let ayeaye_core::projects::json::Value::Object(fields) = project else {
                panic!("a row is not an object: {body}");
            };
            fields
                .iter()
                .map(|(name, value)| {
                    let rendered = match value {
                        ayeaye_core::projects::json::Value::String(text) => text.clone(),
                        other => other.render(),
                    };
                    (name.clone(), rendered)
                })
                .collect()
        })
        .collect()
}

// AYEAYE-50 — "the project picker, served by the Rust binary": the whole of it
// over a real socket, from the gate a browser meets to the fields the page
// reads. A picker that ranks perfectly and is not reachable is not a picker.
#[tokio::test(flavor = "multi_thread")]
async fn the_picker_answers_over_a_socket() {
    let tree = TempTree::new();
    tree.dir("ayeaye/.git");
    tree.dir("ayeaye/deep");
    tree.dir("plain-folder");
    tree.dir("node_modules");
    tree.dir("named");
    tree.file("named/.tmux.yaml", "session_name: chosen\n");

    // Only inside the tree this test made, and never somebody's home
    // directory: the walk is the one thing here that could wander.
    let state = tree.path.join("state");
    // SAFETY: this test binary sets these before it starts a server and never
    // changes them again, and no other test in this file reads them.
    unsafe {
        std::env::set_var("AYEAYE_PROJECT_ROOTS", &tree.path);
        std::env::set_var("AYEAYE_PROJECT_DEPTH", "4");
        std::env::set_var("AYEAYE_PROJECT_WAIT", "5");
        std::env::set_var("XDG_STATE_HOME", &state);
        std::env::set_var("HOME", &tree.path);
    }

    let settings = Settings {
        bind: "127.0.0.1".to_string(),
        port: 0,
        allowed_hosts: AllowedHosts::new("127.0.0.1", 0, ""),
        token: TOKEN.to_string(),
        // Everything below belongs to sibling endpoints this test never calls.
        // Pointed at nothing on purpose: the picker is what is under test, and
        // a tmux or a cliban reachable from here would be somebody's real one.
        peers: Registry::new(vec![Peer::here(
            HostName::new("desktop").expect("a host name"),
        )])
        .expect("one peer, and it is this machine"),
        tmux: common::nowhere("projects-nobody"),
        cliban: ayeaye::cliban::Cliban::new("/nonexistent/cliban".to_string()),
        pane_cache: Arc::new(Mutex::new(ayeaye_core::pane::Cache::default())),
        fits: Arc::new(Fits::new(ayeaye_core::fit::DEFAULT_TTL_MS, None)),
    };
    let listener = ayeaye::server::listen(&settings)
        .await
        .expect("a bindable address");
    let port = listener.local_addr().expect("a bound address").port();
    tokio::spawn(async move {
        let _ = ayeaye::server::serve(listener, Arc::new(settings)).await;
    });

    // Gated, like every other `/api/` path: the picker names every directory
    // you have, which is not something to hand out unauthenticated.
    let refused = request(port, "/api/projects", &[]).await;
    assert_eq!(refused.status, 401);

    let token = [("X-Voice-Token", TOKEN)];
    let answer = request(port, "/api/projects", &token).await;
    assert_eq!(answer.status, 200);
    let found = rows(&answer.body);
    let named: Vec<&str> = found
        .iter()
        .map(|row| row["dir"].as_str())
        .map(|dir| dir.rsplit('/').next().unwrap_or(dir))
        .collect();
    assert!(named.contains(&"ayeaye"), "{named:?}");
    assert!(named.contains(&"plain-folder"), "{named:?}");
    assert!(
        !named.contains(&"node_modules"),
        "the skip list reaches the wire: {named:?}"
    );

    let repo = found
        .iter()
        .find(|row| row["dir"].ends_with("/ayeaye"))
        .expect("the repository is in the answer");
    assert_eq!(repo["git"], "true", "it holds a .git");
    assert_eq!(repo["session"], "ayeaye");
    // Not which one: whether a session is running is what tmux says on the
    // machine the test is on, and this repository is called `ayeaye`. That
    // wiring is pinned in `projects::tests` where the live list is an
    // argument; what belongs here is that a boolean reached the wire at all.
    assert!(
        repo["exists"] == "true" || repo["exists"] == "false",
        "{:?}",
        repo["exists"]
    );
    assert_eq!(repo["short"], "~/ayeaye", "shortened against HOME");

    // A project that names its own session is shown under that name, or
    // choosing it from a phone would start a second session beside the one
    // already running.
    let named_row = found
        .iter()
        .find(|row| row["dir"].ends_with("/named"))
        .expect("the named project is in the answer");
    assert_eq!(named_row["session"], "chosen");

    // A query narrows it, and the answer is still the same shape.
    let searched = request(port, "/api/projects?q=plain", &token).await;
    assert_eq!(searched.status, 200);
    let narrowed = rows(&searched.body);
    assert_eq!(narrowed.len(), 1, "{}", searched.body);
    assert!(narrowed[0]["dir"].ends_with("/plain-folder"));

    // And a query nothing answers is an honest empty list — which the page
    // draws as "nothing matching", and which is *not* the superseded answer.
    let empty = request(port, "/api/projects?q=no-such-project", &token).await;
    assert_eq!(empty.body, r#"{"projects":[]}"#);
}
