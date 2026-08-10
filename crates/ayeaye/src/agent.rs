//! Starting an agent, and stopping one: the parts that touch the machine.
//!
//! Every decision here comes from `ayeaye_core::agent` — which agents exist,
//! what the session is called, what tmux is asked, what gets typed, and what
//! each refusal says. What is left is the effects: reading a request body,
//! looking at a directory, running tmux, and turning the answer into a status
//! and a body.
//!
//! Both endpoints are POSTs, which matters more than it looks: the CSRF gate in
//! `server.rs` exempts GET and HEAD, so it is only safe while every write is a
//! POST. These two are the first writes this binary has, and they keep that
//! true.

use ayeaye_core::agent::{self, refused};
use ayeaye_core::http::refused as bad_request;
use ayeaye_core::json;
use ayeaye_core::peer::{HostName, Peer};

use crate::config::Settings;
use crate::tmux::Trouble;

/// A status code and the body that goes with it.
///
/// The shape the daemon answers in: `400 {"error": ...}` for anything refused,
/// `200` for anything done. Not an axum `Response`, so the whole of both
/// endpoints can be driven by a test that never opens a socket — and so this
/// file names no HTTP type at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answered {
    /// The HTTP status.
    pub status: u16,
    /// The JSON body.
    pub body: String,
}

impl Answered {
    /// Refused, with the reason the panel will show.
    fn refused(said: impl AsRef<str>) -> Answered {
        Answered {
            status: 400,
            body: json::error(said.as_ref()),
        }
    }

    /// Done.
    fn done(body: impl Into<String>) -> Answered {
        Answered {
            status: 200,
            body: body.into(),
        }
    }
}

/// Start an agent in a new pane, and say which pane it is.
///
/// `{"dir": …, "agent": …, "prompt": …, "host": …}`, the last two optional.
pub async fn spawn(settings: &Settings, body: &[u8]) -> Answered {
    let Some(request) = Request::read(body) else {
        return Answered::refused(bad_request::BAD_JSON);
    };

    let here = match this_machine(settings, request.field("host")) {
        Ok(here) => here,
        Err(refusal) => return refusal,
    };
    let Some(agent) = agent::agent(request.field("agent").unwrap_or_default()) else {
        return Answered::refused(refused::UNKNOWN_AGENT);
    };
    let dir = request.field("dir").unwrap_or_default();
    if dir.is_empty() || !is_dir(dir).await {
        return Answered::refused(refused::NO_SUCH_DIRECTORY);
    }

    // Before tmux is asked for anything. A prompt that cannot be typed is a
    // refusal, and refusing it after making the pane would leave an empty pane
    // behind for a request that failed.
    let prompt = agent::within_limit(request.field("prompt").unwrap_or_default());
    let command_line = match agent::command_line(agent, prompt) {
        Ok(line) => line,
        Err(unquotable) => return Answered::refused(unquotable.to_string()),
    };

    let session = agent::session_name(dir, tmux_yaml(dir).await.as_deref());
    let live = match settings.tmux.sessions().await {
        Ok(live) => live,
        Err(trouble) => return Answered::refused(trouble.to_string()),
    };
    // Matched by name, targeted by id. The match is an exact string comparison
    // made here, where it is a comparison and not a tmux target — tmux never
    // sees this name again, which is what makes a project directory called
    // `$0` or `work:9` unable to reach somebody else's session.
    let existing = live
        .iter()
        .find(|live| live.name == session)
        .map(|live| live.id.as_str());
    let made = agent::create(&session, dir, existing);

    let printed = match settings.tmux.ask(&borrowed(made.argv())).await {
        Ok(printed) => printed,
        Err(trouble) => return Answered::refused(trouble.to_string()),
    };
    // tmux prints the pane it made, and that print is the only way to know
    // which one it was. An empty answer means nothing was made, whatever the
    // exit status said.
    let Ok(pane) = ayeaye_core::peer::PaneId::new(here.clone(), printed.trim()) else {
        return Answered::refused(refused::nothing_was_created(made.created()));
    };

    if let Err(trouble) = type_into(settings, pane.pane(), &command_line).await {
        return Answered::refused(refused::started_but_could_not_type(
            made.created(),
            &pane,
            &trouble.to_string(),
        ));
    }

    // Starting an agent somewhere is the strongest possible signal that this
    // is one of your projects, so it is what teaches the picker — the daemon's
    // own placement, after the typing and before the body. The directory goes
    // exactly as the request spelled it; `store::key_for` is the one place
    // that decides what the row is called.
    note_pick(settings, dir).await;

    Answered::done(agent::spawned_body(&pane, &session, &made, agent))
}

/// Kill one pane, if it is a pane this server just listed.
///
/// `{"pane": "desktop/%7"}`.
pub async fn kill(settings: &Settings, body: &[u8]) -> Answered {
    let Some(request) = Request::read(body) else {
        return Answered::refused(bad_request::BAD_JSON);
    };
    let Some(named) = request.field("pane").filter(|pane| !pane.is_empty()) else {
        return Answered::refused(bad_request::NO_PANE);
    };

    // The machine is read out of the id the client handed back, rather than out
    // of a second field beside it that could disagree with it.
    let Ok((peer, id)) = settings.peers.route(named) else {
        return Answered::refused(refused::NO_SUCH_PANE);
    };
    if !peer.is_here() {
        return Answered::refused(refused::not_reachable_yet(peer.name().as_str()));
    }

    // **Membership, not syntax.** `%1` is a well-formed pane id on every
    // machine there is, so a check that the target merely *looks* like one
    // leaves this endpoint a remote off-switch for every tmux the user runs —
    // including the floating `_`-prefixed scratch panes the pane list
    // deliberately hides, which are exactly what a caller would name to reach
    // something the panel never offered. The defence is that the pane has to be
    // one this server just listed, and it is the same defence `bin/ayeaye`'s
    // `kill_pane` makes at `:1933`.
    //
    // A pane list that could not be read is **not** an empty pane list. The
    // daemon cannot tell those apart and answers "no such pane" to both; here a
    // tmux that could not be asked says so, because "there is no such pane" and
    // "I could not look" are opposite answers — and only one of them means the
    // pane is safe.
    let listed = match settings.tmux.panes(peer.name()).await {
        Ok(listed) => listed,
        Err(trouble) => return Answered::refused(trouble.to_string()),
    };
    // Compared on the pane half. The host half has already been answered, and
    // answered better: `route` folds case the way DNS does, so `DESKTOP/%1` and
    // `desktop/%1` reach the same peer — while these listed ids all carry that
    // peer's name as the registry spells it. Comparing whole ids would refuse
    // the shouted one as "no such pane" and make this layer disagree with the
    // routing layer about how many machines there are.
    if !listed.iter().any(|pane| pane.id.pane() == id.pane()) {
        return Answered::refused(refused::NO_SUCH_PANE);
    }

    // The bare id, because that is what tmux understands — and it is the pane
    // half of an id that was just found in tmux's own output, not a string the
    // request supplied.
    //
    // tmux handles the cascade from here: the window dies with its last pane,
    // and the session with its last window.
    match settings.tmux.ask(&["kill-pane", "-t", id.pane()]).await {
        Ok(_) => Answered::done(agent::killed_body()),
        Err(Trouble::Refused(said)) => Answered::refused(refused::could_not_kill(&said)),
        Err(trouble) => Answered::refused(refused::could_not_kill(&trouble.to_string())),
    }
}

/// The machine a request names, which must be this one for now.
///
/// The field is optional and defaults to this machine, so the body the panel
/// sends today needs nothing added to it. A body that *does* name a machine is
/// resolved through the registry rather than compared against a string: that is
/// the whole point of the field being here before federation is, and it means
/// the day a peer is reachable this reads the peer instead of refusing it.
fn this_machine<'s>(settings: &'s Settings, named: Option<&str>) -> Result<&'s HostName, Answered> {
    let here: &Peer = settings.peers.here();
    let Some(named) = named.filter(|name| !name.trim().is_empty()) else {
        return Ok(here.name());
    };
    match settings.peers.get(named) {
        Some(peer) if peer.is_here() => Ok(peer.name()),
        Some(peer) => Err(Answered::refused(refused::not_reachable_yet(
            peer.name().as_str(),
        ))),
        None => Err(Answered::refused(refused::no_such_machine(named))),
    }
}

/// Type the command line into the pane, then press Enter.
///
/// Two calls, as the daemon makes them. `-l` sends the line literally, so
/// nothing in it is read as a key name — and it is only ever the agent's own
/// command followed by an already-quoted argument, so it can never begin with
/// the `-` that would make tmux read it as an option.
async fn type_into(settings: &Settings, pane: &str, command_line: &str) -> Result<(), Trouble> {
    settings
        .tmux
        .ask(&["send-keys", "-t", pane, "-l", command_line])
        .await?;
    settings
        .tmux
        .ask(&["send-keys", "-t", pane, "Enter"])
        .await?;
    Ok(())
}

/// Record the pick in the store the picker ranks by.
///
/// Best effort in every direction, as `store::note_pick` itself is: a settings
/// with no store path, a write that fails, even a panic on the blocking thread
/// all cost ranking quality and nothing else — no failure here may reach the
/// spawn response. On a blocking thread because it is file I/O on the request
/// path, and **awaited** because the daemon writes before it answers: the row
/// is on disk by the time the caller sees the pane, which is also the only
/// ordering a test can assert against.
async fn note_pick(settings: &Settings, dir: &str) {
    let Some(store) = settings.store.clone() else {
        return;
    };
    let dir = dir.to_string();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_secs_f64())
        .unwrap_or(0.0);
    let _ = tokio::task::spawn_blocking(move || {
        crate::projects::store::note_pick(&store, &dir, now);
    })
    .await;
}

/// Whether this is a directory an agent could be started in.
async fn is_dir(path: &str) -> bool {
    tokio::fs::metadata(path)
        .await
        .map(|found| found.is_dir())
        .unwrap_or(false)
}

/// A project's `.tmux.yaml`, if it has one and we may read it.
///
/// Anything wrong with the file is no file: a project that does not name itself
/// is named after its directory, which is also what happens when the name
/// cannot be read. There is nothing here worth failing a spawn over.
async fn tmux_yaml(dir: &str) -> Option<String> {
    let path = std::path::Path::new(dir).join(".tmux.yaml");
    tokio::fs::read_to_string(path).await.ok()
}

fn borrowed(argv: &[String]) -> Vec<&str> {
    argv.iter().map(String::as_str).collect()
}

/// One request body, read as an object of strings.
///
/// Only the fields these endpoints ask for, and only as text. A number or a
/// list where a string was expected reads as absent rather than as its own
/// spelling: `{"agent": 7}` is a request that named no agent, which is exactly
/// what it is.
struct Request(serde_json::Value);

impl Request {
    /// Read a body, or `None` if it is not a JSON object.
    ///
    /// An empty body is `{}` — the daemon's `json.loads(self._body() or b"{}")`
    /// — so a POST with nothing in it is a request that supplied no fields
    /// rather than a parse failure.
    fn read(body: &[u8]) -> Option<Request> {
        if body.is_empty() {
            return Some(Request(serde_json::Value::Object(Default::default())));
        }
        let value: serde_json::Value = serde_json::from_slice(body).ok()?;
        value.is_object().then_some(Request(value))
    }

    fn field(&self, name: &str) -> Option<&str> {
        self.0.get(name)?.as_str()
    }
}

#[cfg(test)]
mod tests {
    use super::Request;

    // AYEAYE-51 — a request body arrives from the network. The fields these
    // endpoints read are text, and anything else is a field that was not sent:
    // `{"agent": 7}` named no agent, and reading it as `"7"` would be inventing
    // a request nobody made.
    #[test]
    fn a_body_reads_only_the_strings_it_was_sent() {
        let request = Request::read(br#"{"dir":"/tmp","agent":7,"prompt":null}"#)
            .expect("an object is a request");
        assert_eq!(request.field("dir"), Some("/tmp"));
        assert_eq!(request.field("agent"), None);
        assert_eq!(request.field("prompt"), None);
        assert_eq!(request.field("host"), None);
    }

    // AYEAYE-51 — an empty body is a request with no fields, which is what the
    // daemon's `json.loads(self._body() or b"{}")` makes it. Anything that is
    // not an object is not a request at all — including valid JSON that is a
    // list or a string, which the daemon reaches an exception on.
    #[test]
    fn an_empty_body_is_a_request_and_a_list_is_not() {
        let empty = Request::read(b"").expect("an empty body is an empty request");
        assert_eq!(empty.field("dir"), None);
        assert!(Request::read(b"{}").is_some());

        assert!(Request::read(b"[1,2,3]").is_none());
        assert!(Request::read(br#""hello""#).is_none());
        assert!(Request::read(b"{not json").is_none());
        assert!(Request::read(b"null").is_none());
    }
}
