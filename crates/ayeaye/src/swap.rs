//! The inference backend, which is now somebody else's process.
//!
//! ayeaye used to hold two models in its own address space. It no longer holds
//! any: `llama-swap` sits in front of `llama-server` and `whisper-server`, and
//! this is the client that talks to it. What that bought is the whole of
//! AYEAYE-101 — no CUDA toolkit in the build, no `metal` feature, no release row
//! that is glibc-dynamic because `candle-kernels` linked `libcudart`, and no
//! residency policy here, because swapping models in and out is the thing
//! llama-swap exists to do.
//!
//! **Through curl, and that is this crate's own rule rather than a shortcut.**
//! `crate::recorder` writes HTTP onto a socket by hand and says why it is
//! allowed to: the recording agent listens on a tailnet with no TLS, so there is
//! nothing to encrypt. A llama-swap deployment is not that. It can be — and the
//! one this was built against is — an `https://` host, and an in-process client
//! there needs a TLS stack; every TLS stack in the ecosystem reaches `ring` or
//! `aws-lc-sys`, and both put `cc` in `Cargo.lock`, which the constitution
//! refuses. So this reaches for the TLS implementation the machine already has,
//! exactly as the model store next door used to and as `crate::health` still
//! does.
//!
//! Bodies go in on **stdin** rather than through a temporary file. The audio is
//! somebody's voice and the transcript is what they said; neither should touch
//! the filesystem on its way to a socket, and `-F file=@…` would put the clip in
//! a file to do it. The multipart body is therefore assembled here and piped.

use std::fmt;
use std::time::Duration;

use ayeaye_core::Pcm16kMono;

use crate::command::{self, Failed};

/// How long a transcription gets.
///
/// Minutes, and deliberately: the first request for a model llama-swap does not
/// currently have loaded pays for loading it, which on a cold page cache is tens
/// of seconds before any audio is looked at. A limit tuned to a warm proxy turns
/// every first dictation after a swap into a failure.
pub const TRANSCRIBE_LIMIT: Duration = Duration::from_secs(300);
/// How long a rewrite gets. Same reasoning, less audio.
pub const COMPLETE_LIMIT: Duration = Duration::from_secs(120);
/// How long the proxy gets to say it is there.
pub const LIST_LIMIT: Duration = Duration::from_secs(10);

/// The multipart boundary.
///
/// Fixed rather than random, and safe to fix because of the part order in
/// [`Swap::transcribe`]: the only attacker-influenced bytes in the body are the
/// audio samples, and they are written **last**. A boundary occurring inside
/// them ends the body early, which truncates the clip. A truncated clip is a
/// worse transcript, not a forged field.
const BOUNDARY: &str = "----ayeaye-clip-boundary";

/// A llama-swap proxy to talk to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Swap {
    base: String,
}

/// Why a base address is not one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BadBase {
    /// A scheme that is not HTTP or HTTPS.
    NotHttp(String),
    /// Empty, or carrying something a URL cannot.
    Unreadable(String),
}

impl fmt::Display for BadBase {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BadBase::NotHttp(scheme) => write!(out, "{scheme:?} is not a scheme this speaks"),
            BadBase::Unreadable(given) => write!(
                out,
                "{given:?} is not an address; write it as https://host or host:port"
            ),
        }
    }
}

impl std::error::Error for BadBase {}

/// Why the backend said nothing usable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unreachable {
    /// curl is not on this machine, or would not start.
    NoClient(String),
    /// Nothing answered, or the connection failed. Carries curl's own words,
    /// which are the useful half — a certificate, a DNS name, a refused port.
    NoAnswer(String),
    /// It answered, with a status that is not success.
    Refused {
        /// The HTTP status.
        status: u16,
        /// What it said, bounded.
        said: String,
    },
    /// It answered with something that is not the reply this asked for.
    Garbled(String),
    /// It did not finish inside the limit.
    TimedOut(Duration),
}

impl fmt::Display for Unreachable {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Unreachable::NoClient(why) => write!(out, "this computer has no working curl: {why}"),
            Unreachable::NoAnswer(why) => write!(out, "{why}"),
            Unreachable::Refused { status, said } if said.is_empty() => {
                write!(out, "it answered HTTP {status}")
            }
            Unreachable::Refused { status, said } => {
                write!(out, "it answered HTTP {status}: {said}")
            }
            Unreachable::Garbled(why) => write!(out, "it answered with {why}"),
            Unreachable::TimedOut(limit) => write!(out, "it did not answer within {limit:?}"),
        }
    }
}

impl std::error::Error for Unreachable {}

impl Swap {
    /// Read a base address.
    ///
    /// Accepts `https://host`, `http://host:port`, `host:port`, and a bare host
    /// — the last two taking `http://` and llama-swap's own port, because that
    /// is the form somebody types for one on the loopback. A scheme that is
    /// written is honoured exactly, path and all: a proxy behind a path prefix
    /// is an ordinary way to deploy one.
    pub fn at(base: &str) -> Result<Swap, BadBase> {
        let base = base.trim().trim_end_matches('/');
        if base.is_empty() || base.chars().any(|c| c.is_control() || c.is_whitespace()) {
            return Err(BadBase::Unreadable(base.to_string()));
        }
        let base = match base.split_once("://") {
            Some(("http" | "https", rest)) if !rest.is_empty() => base.to_string(),
            Some((scheme, _)) if scheme == "http" || scheme == "https" => {
                return Err(BadBase::Unreadable(base.to_string()));
            }
            Some((scheme, _)) => return Err(BadBase::NotHttp(scheme.to_string())),
            // No scheme. A port is added only when there is none, and only to
            // the authority — anything after the first `/` is a path.
            None => {
                let (authority, path) = match base.split_once('/') {
                    Some((authority, path)) => (authority, format!("/{path}")),
                    None => (base, String::new()),
                };
                if authority.is_empty() {
                    return Err(BadBase::Unreadable(base.to_string()));
                }
                match authority.rsplit_once(':') {
                    // A port that is not a number is a typo worth naming, not a
                    // hostname with a colon in it.
                    Some((host, port)) => {
                        if host.is_empty() || port.parse::<u16>().is_err() {
                            return Err(BadBase::Unreadable(base.to_string()));
                        }
                        format!("http://{authority}{path}")
                    }
                    None => format!("http://{authority}:8080{path}"),
                }
            }
        };
        Ok(Swap { base })
    }

    /// The address, as it would be written back out.
    pub fn base(&self) -> &str {
        &self.base
    }

    /// Every model the proxy is configured to serve.
    ///
    /// This is also the health check: a proxy that answers `/v1/models` is one
    /// that is up and could read its config. It is not a claim that any of those
    /// models will load — llama-swap starts them on demand, and finding that out
    /// costs a real request.
    pub async fn models(&self) -> Result<Vec<String>, Unreachable> {
        let said = self.call("/v1/models", None, LIST_LIMIT).await?;
        let body: serde_json::Value =
            serde_json::from_str(&said).map_err(|why| Unreachable::Garbled(why.to_string()))?;
        Ok(body["data"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|model| model["id"].as_str().map(str::to_string))
            .collect())
    }

    /// Turn audio into words, through whichever speech model `model` names.
    ///
    /// The clip is re-encoded as a WAVE because the endpoint takes a container
    /// rather than samples. That is a header and a memcpy over audio this
    /// process already decoded, and it is what lets the energy gate in
    /// `crate::dictate` still run before anything is sent: silence costs a
    /// buffer walk here rather than a round trip and a model load.
    pub async fn transcribe(&self, model: &str, audio: &Pcm16kMono) -> Result<String, Unreachable> {
        let wav = ayeaye_core::audio::to_wav(audio);
        let mut body = Vec::with_capacity(wav.len() + 512);
        for (name, value) in [("model", model), ("response_format", "json")] {
            body.extend_from_slice(
                format!(
                    "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n\
                     {value}\r\n"
                )
                .as_bytes(),
            );
        }
        // Last, on purpose. See BOUNDARY.
        body.extend_from_slice(
            format!(
                "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"file\"; \
                 filename=\"clip.wav\"\r\nContent-Type: audio/wav\r\n\r\n"
            )
            .as_bytes(),
        );
        body.extend_from_slice(&wav);
        body.extend_from_slice(format!("\r\n--{BOUNDARY}--\r\n").as_bytes());

        let said = self
            .call(
                "/v1/audio/transcriptions",
                Some((format!("multipart/form-data; boundary={BOUNDARY}"), body)),
                TRANSCRIBE_LIMIT,
            )
            .await?;
        let body: serde_json::Value =
            serde_json::from_str(&said).map_err(|why| Unreachable::Garbled(why.to_string()))?;
        body["text"]
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| Unreachable::Garbled("an answer with no transcript in it".to_string()))
    }

    /// Rewrite one turn, through whichever language model `model` names.
    ///
    /// A chat request rather than a completion, and that is the change of
    /// address the whole ticket turns on: the chat template belongs to the
    /// server that loaded the weights and knows which one they were trained
    /// with. ayeaye used to carry a `Template` and pick between ChatML and
    /// Llama-3 from an environment variable, which is a thing to get wrong on
    /// behalf of a model it cannot see.
    pub async fn complete(
        &self,
        model: &str,
        system: &str,
        user: &str,
        max_tokens: usize,
    ) -> Result<String, Unreachable> {
        // Neutralised at the wire, both turns. This used to happen inside
        // `chat::Template::render`, and losing it with the template would have
        // been the quiet half of this change: llama-server renders these
        // messages into the model's own template, so a transcription carrying
        // `<|im_end|>` — whatever was said in the room, or read off a screen —
        // would end the user's turn early and leave the rest of it addressed to
        // the model as instructions.
        let request = serde_json::json!({
            "model": model,
            "messages": [
                {"role": "system", "content": ayeaye_core::chat::neutralise(system)},
                {"role": "user", "content": ayeaye_core::chat::neutralise(user)},
            ],
            "max_tokens": max_tokens,
            // A rewrite wants the same answer twice. A dictation is not a place
            // for variety, and this is the same greedy decode the in-process
            // path used.
            "temperature": 0.0,
            "stream": false,
        });
        let body = serde_json::to_vec(&request).map_err(|why| {
            Unreachable::Garbled(format!("a request that will not serialise: {why}"))
        })?;
        let said = self
            .call(
                "/v1/chat/completions",
                Some(("application/json".to_string(), body)),
                COMPLETE_LIMIT,
            )
            .await?;
        let body: serde_json::Value =
            serde_json::from_str(&said).map_err(|why| Unreachable::Garbled(why.to_string()))?;
        body["choices"][0]["message"]["content"]
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| Unreachable::Garbled("an answer with no message in it".to_string()))
    }

    /// One request, and the body it answered with.
    async fn call(
        &self,
        path: &str,
        body: Option<(String, Vec<u8>)>,
        limit: Duration,
    ) -> Result<String, Unreachable> {
        let url = format!("{}{path}", self.base);
        let mut argv: Vec<String> = [
            "curl",
            "--silent",
            "--show-error",
            // curl's own deadline, inside `command::run`'s. Two limits rather
            // than one because they fail differently: this one lets curl say
            // *what* timed out, and the outer one is what stops a curl that
            // ignored its own.
            "--max-time",
            &limit.as_secs().to_string(),
            // The status, on its own last line. `--fail` would throw the body
            // away, and the body of a refusal is where llama-swap names the
            // model it has never heard of.
            "--write-out",
            "\n%{http_code}",
        ]
        .iter()
        .map(|word| (*word).to_string())
        .collect();
        if let Some((kind, _)) = &body {
            argv.push("--request".to_string());
            argv.push("POST".to_string());
            argv.push("--header".to_string());
            argv.push(format!("Content-Type: {kind}"));
            // From stdin. The clip is somebody's voice; it does not go to disk
            // on its way to a socket.
            argv.push("--data-binary".to_string());
            argv.push("@-".to_string());
        }
        // `--` before the URL: an address is configuration, and configuration
        // that begins with a dash must not become a flag.
        argv.push("--".to_string());
        argv.push(url);

        let ran = match &body {
            Some((_, payload)) => command::run_with_input(&argv, payload, limit).await,
            None => command::run(&argv, limit).await,
        };
        let ran = match ran {
            Ok(ran) => ran,
            Err(Failed::TimedOut(limit)) => return Err(Unreachable::TimedOut(limit)),
            Err(Failed::NotStarted(why)) => return Err(Unreachable::NoClient(why)),
        };
        if !ran.ok {
            // curl's exit status covers everything before a status line: DNS,
            // the connection, the certificate. Its stderr is the sentence
            // somebody can act on, so it is passed through rather than replaced.
            return Err(Unreachable::NoAnswer(bound(
                if ran.stderr.trim().is_empty() {
                    "it could not be reached"
                } else {
                    ran.stderr.trim()
                },
            )));
        }
        // The status is the last line, because `--write-out` wrote it there.
        let (said, status) = ran
            .stdout
            .rsplit_once('\n')
            .ok_or_else(|| Unreachable::Garbled("no status at all".to_string()))?;
        let status: u16 = status
            .trim()
            .parse()
            .map_err(|_| Unreachable::Garbled(format!("{status:?} where a status goes")))?;
        if !(200..300).contains(&status) {
            return Err(Unreachable::Refused {
                status,
                said: bound(said.trim()),
            });
        }
        Ok(said.to_string())
    }
}

/// As much of an error as belongs in a log line.
///
/// A proxy in front of something unexpected answers with an HTML page, and the
/// whole of one in a log line is how the useful part scrolls away.
fn bound(said: &str) -> String {
    said.chars().take(300).collect()
}

#[cfg(test)]
mod tests {
    use super::{BadBase, Swap, Unreachable, bound};

    // AYEAYE-101 — the forms somebody actually types, including the one this
    // was built against: a real llama-swap behind TLS on a real hostname.
    #[test]
    fn a_base_address_is_read_in_the_forms_people_write_it() {
        for (given, want) in [
            // A scheme that is written is honoured exactly.
            ("https://llama.example.test", "https://llama.example.test"),
            ("https://llama.example.test/", "https://llama.example.test"),
            ("http://127.0.0.1:8080", "http://127.0.0.1:8080"),
            // A proxy behind a path prefix is an ordinary deployment.
            ("https://example.test/llama", "https://example.test/llama"),
            // No scheme: plain HTTP and llama-swap's own port, which is the
            // form somebody types for one on the loopback.
            ("127.0.0.1:8080", "http://127.0.0.1:8080"),
            ("box.tailnet.ts.net", "http://box.tailnet.ts.net:8080"),
            ("  box:9292  ", "http://box:9292"),
        ] {
            assert_eq!(Swap::at(given).expect(given).base(), want, "{given}");
        }
    }

    // AYEAYE-101 — https is spoken rather than refused, which is the whole
    // reason this client is curl and not a socket. The check that would have
    // been wrong is asserted directly, because "it was refused by name" was the
    // first implementation of this file.
    #[test]
    fn an_encrypted_base_is_spoken_rather_than_refused() {
        let swap = Swap::at("https://llama.example.test").expect("https is an address");
        assert!(swap.base().starts_with("https://"), "{}", swap.base());
    }

    // AYEAYE-101 — a scheme this cannot speak is named, and nonsense is refused
    // rather than turned into a hostname.
    #[test]
    fn an_address_that_is_not_one_is_refused_by_name() {
        assert!(matches!(
            Swap::at("ftp://box"),
            Err(BadBase::NotHttp(scheme)) if scheme == "ftp"
        ));
        for nonsense in ["", "   ", "https://", "http://", ":8080", "box:not-a-port"] {
            assert!(
                matches!(Swap::at(nonsense), Err(BadBase::Unreadable(_))),
                "{nonsense:?} is not an address"
            );
        }
        // Whitespace inside would split into two curl arguments.
        assert!(Swap::at("http://box:8080 --output /etc/passwd").is_err());
    }

    // AYEAYE-101 — a refusal carries the proxy's own words, bounded. This is
    // the difference between "no such model" reaching somebody as a stated
    // reason and reaching them as silence.
    #[test]
    fn a_refusal_says_what_the_proxy_said_and_does_not_run_on() {
        let refused = Unreachable::Refused {
            status: 400,
            said: bound("{\"error\":\"model whisper not found\"}"),
        };
        let words = refused.to_string();
        assert!(words.contains("400"), "{words}");
        assert!(words.contains("whisper not found"), "{words}");

        let flood = bound(&"x".repeat(10_000));
        assert_eq!(flood.chars().count(), 300);
    }
}
