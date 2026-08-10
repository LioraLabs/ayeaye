//! The microphone on the machine the person is actually sitting at.
//!
//! `bin/voice-agent` runs on the client device — the phone, the laptop you are
//! SSH'd in from — and it stays Python: it is installed separately, on hardware
//! this binary is not shipped to, and moving it would gain nothing. So this is
//! only the client half: three requests over plain HTTP.
//!
//! **Plain HTTP, and therefore no dependency.** The agent listens on a tailnet
//! address with no TLS, so this is a socket and a few hundred bytes of protocol.
//! That is worth saying out loud because the model store next door reaches for
//! `curl` instead: an in-process client *there* needs TLS, every TLS stack in
//! the ecosystem reaches `ring` or `aws-lc-sys`, and both put `cc` in
//! `Cargo.lock`, which the constitution refuses. The difference is not taste, it
//! is whether the connection is encrypted.
//!
//! `Connection: close` and read to EOF, so there is no `Content-Length` to trust
//! and no chunked decoding to write. The agent speaks HTTP/1.1 and honours it.

use std::fmt;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// The port the recording agent listens on.
///
/// `bin/voice-agent`'s `VOICE_PORT`, and the same default.
pub const DEFAULT_PORT: u16 = 8787;

/// The header the shared secret is presented in.
pub const TOKEN_HEADER: &str = "X-Voice-Token";

/// The header the agent names the container in.
pub const EXTENSION_HEADER: &str = "x-audio-ext";

/// How long the agent gets to answer.
///
/// Longer for `/stop` than for the rest: it is handing back a recording, which
/// is megabytes over somebody's phone connection, where `/health` is a probe
/// that should fail fast enough to say "there is nothing there".
pub const HEALTH_LIMIT: Duration = Duration::from_secs(5);
/// How long the agent gets to answer anything else.
pub const LIMIT: Duration = Duration::from_secs(30);

/// What the agent said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reply {
    /// The status line's code.
    pub status: u16,
    /// The body, which for `/stop` is the recording itself.
    pub body: Vec<u8>,
    /// The container the recording is in, where the agent named one.
    pub extension: Option<String>,
}

impl Reply {
    /// Whether the agent said yes.
    ///
    /// `/health` answers `{"ok": true}` only when it found a recording backend,
    /// so a 200 on its own is not an answer: it means the daemon is running on a
    /// machine with no microphone it knows how to use.
    pub fn healthy(&self) -> bool {
        self.status == 200
            && ayeaye_core::json::parse(&String::from_utf8_lossy(&self.body))
                .ok()
                .and_then(|value| value.get("ok").map(ayeaye_core::json::Value::truthy))
                .unwrap_or(false)
    }
}

/// Why the agent said nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unreachable {
    /// Nothing answered on that address.
    NoAnswer(String),
    /// It answered with something that is not an HTTP response.
    Garbled(String),
    /// It did not finish inside the limit.
    TimedOut(Duration),
}

impl fmt::Display for Unreachable {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Unreachable::NoAnswer(why) => write!(out, "{why}"),
            Unreachable::Garbled(why) => write!(out, "it answered with {why}"),
            Unreachable::TimedOut(limit) => write!(out, "it did not answer within {limit:?}"),
        }
    }
}

/// A recording agent to talk to.
#[derive(Debug, Clone)]
pub struct Recorder {
    host: String,
    port: u16,
    token: String,
}

impl Recorder {
    /// The agent on one machine.
    pub fn at(host: &str, port: u16, token: &str) -> Recorder {
        Recorder {
            host: host.to_string(),
            port,
            token: token.to_string(),
        }
    }

    /// Is there a microphone over there.
    pub async fn health(&self) -> Result<Reply, Unreachable> {
        self.call("GET", "/health", HEALTH_LIMIT).await
    }

    /// Turn it on.
    pub async fn start(&self) -> Result<Reply, Unreachable> {
        self.call("POST", "/start", LIMIT).await
    }

    /// Turn it off, and hand back what it heard.
    pub async fn stop(&self) -> Result<Reply, Unreachable> {
        self.call("POST", "/stop", LIMIT).await
    }

    async fn call(&self, method: &str, path: &str, limit: Duration) -> Result<Reply, Unreachable> {
        match tokio::time::timeout(limit, self.exchange(method, path)).await {
            Ok(answer) => answer,
            Err(_) => Err(Unreachable::TimedOut(limit)),
        }
    }

    async fn exchange(&self, method: &str, path: &str) -> Result<Reply, Unreachable> {
        let mut stream = TcpStream::connect((self.host.as_str(), self.port))
            .await
            .map_err(|why| Unreachable::NoAnswer(why.to_string()))?;

        // The token goes in a header rather than the path: a path is written to
        // the agent's own log, and this one turns a microphone on.
        let request = format!(
            "{method} {path} HTTP/1.1\r\nHost: {}:{}\r\n{TOKEN_HEADER}: {}\r\n\
             Content-Length: 0\r\nConnection: close\r\n\r\n",
            self.host, self.port, self.token
        );
        stream
            .write_all(request.as_bytes())
            .await
            .map_err(|why| Unreachable::NoAnswer(why.to_string()))?;

        let mut raw = Vec::new();
        stream
            .read_to_end(&mut raw)
            .await
            .map_err(|why| Unreachable::NoAnswer(why.to_string()))?;
        parse(&raw)
    }
}

/// Read one HTTP response.
///
/// Its own function, and public to the crate, because the parsing is the part
/// worth holding to a shape without a socket in the way.
pub(crate) fn parse(raw: &[u8]) -> Result<Reply, Unreachable> {
    let split = raw
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| Unreachable::Garbled("no end to its headers".to_string()))?;
    let head = String::from_utf8_lossy(&raw[..split]);
    let body = raw[split + 4..].to_vec();

    let mut lines = head.lines();
    let status = lines
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse().ok())
        .ok_or_else(|| Unreachable::Garbled(format!("{:?}", head.lines().next().unwrap_or(""))))?;

    // Header names are case-insensitive, and the agent spells this one
    // `X-Audio-Ext`. Matching the spelling rather than the name is how a working
    // recorder comes back as an undecodable clip.
    let extension = lines.find_map(|line| {
        let (name, value) = line.split_once(':')?;
        (name.trim().eq_ignore_ascii_case(EXTENSION_HEADER)).then(|| value.trim().to_string())
    });

    Ok(Reply {
        status,
        body,
        extension,
    })
}
