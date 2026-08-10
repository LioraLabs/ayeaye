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

/// The most recording this will take from an agent.
///
/// The same thirty-two megabytes the browser's dictation endpoint caps its body
/// at, and deliberately the same number: it is the same audio, arriving over the
/// same kind of connection, and a couple of minutes of it is real data. Without
/// a cap the only bound is [`LIMIT`], which on a fast link is gigabytes into
/// this process's memory before the clock runs out.
pub const MAX_CLIP: usize = 32 << 20;

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
    /// The address or the secret carries something a request line cannot.
    Unspeakable(&'static str),
    /// It sent more than a recording could reasonably be.
    TooMuch(usize),
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
            Unreachable::Unspeakable(what) => {
                write!(out, "the {what} carries a character a request cannot")
            }
            Unreachable::TooMuch(cap) => write!(
                out,
                "it sent more than {} MB of audio, which is more than a dictation is",
                cap >> 20
            ),
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
        // Both go into a request line by interpolation, and a newline in either
        // would end the header and begin one this code did not write. Neither
        // can carry one today — the address comes from the process table and the
        // secret from a file — but a hand-rolled client that argues it is safe
        // to hand-roll has to be the thing that checks.
        if speakable(&self.token) {
            return Err(Unreachable::Unspeakable("token"));
        }
        if speakable(&self.host) {
            return Err(Unreachable::Unspeakable("address"));
        }
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

        // Bounded. `read_to_end` on a socket is bounded only by the deadline,
        // and one megabyte per millisecond is a plausible tailnet.
        //
        // One byte past the cap is read on purpose, so that hitting it is
        // *observable*. Stopping exactly at the cap and decoding what arrived
        // would hand the converter a truncated container — which it will
        // usually decode, cheerfully, into a recording that stops mid-sentence.
        // A dictation silently missing its ending is worse than one that failed,
        // because nobody knows to say it again.
        let mut raw = Vec::new();
        (&mut stream)
            .take(MAX_CLIP as u64 + 1)
            .read_to_end(&mut raw)
            .await
            .map_err(|why| Unreachable::NoAnswer(why.to_string()))?;
        if raw.len() > MAX_CLIP {
            return Err(Unreachable::TooMuch(MAX_CLIP));
        }
        parse(&raw)
    }
}

/// Whether text would break out of the request line it is interpolated into.
fn speakable(text: &str) -> bool {
    text.chars().any(|c| c.is_control())
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

#[cfg(test)]
mod tests {
    use super::{Reply, Unreachable, parse};

    fn answer(head: &str, body: &[u8]) -> Vec<u8> {
        let mut raw = head.replace('\n', "\r\n").into_bytes();
        raw.extend_from_slice(b"\r\n\r\n");
        raw.extend_from_slice(body);
        raw
    }

    // AYEAYE-58
    //
    // What `/stop` answers with: the recording, and the container it is in. The
    // header name is matched case-insensitively because HTTP says it is, and
    // because a client that compared spellings would turn a working recorder
    // into an undecodable clip the day the agent's server changed how it writes
    // them.
    #[test]
    fn a_recording_comes_back_with_the_container_the_agent_named() {
        let reply = parse(&answer(
            "HTTP/1.1 200 OK\nContent-Type: application/octet-stream\nX-Audio-Ext: ogg",
            b"the audio itself",
        ))
        .expect("a well-formed answer");

        assert_eq!(
            reply,
            Reply {
                status: 200,
                body: b"the audio itself".to_vec(),
                extension: Some("ogg".to_string()),
            }
        );
        // Any spelling of the same header, because that is what the name means.
        for spelled in ["x-audio-ext", "X-AUDIO-EXT", "X-Audio-Ext"] {
            let reply = parse(&answer(&format!("HTTP/1.1 200 OK\n{spelled}: m4a"), b""))
                .expect("a well-formed answer");
            assert_eq!(reply.extension.as_deref(), Some("m4a"), "{spelled}");
        }
        // And a header that merely ends the same way is not that header.
        let reply = parse(&answer("HTTP/1.1 200 OK\nNot-X-Audio-Ext: nope", b""))
            .expect("a well-formed answer");
        assert_eq!(reply.extension, None);
    }

    // AYEAYE-58
    //
    // `/health` answering 200 is not an answer: the agent runs on machines with
    // no microphone it knows how to use, and says so in the body. Reading the
    // status alone would start a recording on a device that cannot record.
    #[test]
    fn a_health_check_is_the_body_rather_than_the_status() {
        let healthy = parse(&answer(
            "HTTP/1.1 200 OK",
            br#"{"ok":true,"recorder":"ffmpeg"}"#,
        ))
        .expect("a well-formed answer");
        assert!(healthy.healthy());

        for body in [
            &br#"{"ok":false,"recorder":"none"}"#[..],
            &b"{}"[..],
            &b"not json at all"[..],
            &b""[..],
        ] {
            let said = parse(&answer("HTTP/1.1 200 OK", body)).expect("a well-formed answer");
            assert!(!said.healthy(), "{:?} is not a working recorder", body);
        }
        // A refusal is not healthy however it is worded.
        let refused = parse(&answer("HTTP/1.1 401 Unauthorized", br#"{"ok":true}"#))
            .expect("a well-formed answer");
        assert_eq!(refused.status, 401);
        assert!(!refused.healthy(), "a 401 is not a microphone");
    }

    // AYEAYE-58
    //
    // Something that is not an HTTP response is said out loud rather than read
    // as an empty recording. The agent is reached over a tailnet, where the
    // thing on that port may be somebody else's program entirely.
    #[test]
    fn something_that_is_not_an_http_response_is_refused_by_name() {
        for garbled in [
            &b""[..],
            &b"hello?"[..],
            &b"HTTP/1.1 OK\r\n\r\n"[..],
            &b"\x16\x03\x01 a TLS hello\r\n\r\n"[..],
        ] {
            assert!(
                matches!(parse(garbled), Err(Unreachable::Garbled(_))),
                "{garbled:?} is not an answer"
            );
        }
        // A status line and no headers at all is still an answer.
        let bare = parse(b"HTTP/1.1 204 No Content\r\n\r\n").expect("a bare answer");
        assert_eq!(bare.status, 204);
        assert!(bare.body.is_empty());
    }
}
