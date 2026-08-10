//! The HTTP shell.
//!
//! One handler for every request, in the daemon's own order: the Host gate,
//! then the route, then the token gate, then the answer. axum is here for what
//! comes next — `/api/stream` is server-sent events, and the app is not usable
//! without them — but nothing in this file decides anything. Every verdict
//! comes from `ayeaye_core::http`, which a test can reach without a socket.

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, Method, StatusCode, Uri, header};
use axum::response::Response;

use ayeaye_core::http::origin::{Origin, Site};
use ayeaye_core::http::route::{Asset, Gate, Route};
use ayeaye_core::http::{auth, login, origin, route};
use ayeaye_core::pane;
use ayeaye_core::peer::PaneId;

/// The header a browser labels a request's provenance with. `http` has no
/// constant for it: it is a fetch-metadata header, newer than the crate's
/// table.
const SEC_FETCH_SITE: &str = "sec-fetch-site";

use crate::assets;
use crate::config::Settings;

/// Build the router.
///
/// Every path and method lands on the one handler, because the gates apply to
/// all of them equally: a per-route layer would have to be remembered again
/// for every endpoint the rest of this milestone adds, and the one that was
/// forgotten would be the hole.
pub fn router(settings: Arc<Settings>) -> Router {
    Router::new().fallback(handle).with_state(settings)
}

/// Bind the address the settings resolved to.
///
/// Separate from [`serve`] so a test can ask for the port it actually got —
/// and so the binding a test exercises is this function rather than a line of
/// the test's own, which would prove only that the harness can bind.
pub async fn listen(settings: &Settings) -> std::io::Result<tokio::net::TcpListener> {
    tokio::net::TcpListener::bind(settings.address()).await
}

/// Serve on an already-bound listener until the process ends.
///
/// The listener is an argument rather than something built from [`Settings`]
/// here, so a test can bind port 0 and still know which port it got.
pub async fn serve(
    listener: tokio::net::TcpListener,
    settings: Arc<Settings>,
) -> std::io::Result<()> {
    // Started here rather than in `main`, so a server driven by an integration
    // test has the sweeper a real one has. A lease that only expired in
    // production would be a lease nothing ever proved expires.
    crate::fit::sweeper(Arc::clone(&settings));
    axum::serve(listener, router(settings)).await
}

async fn handle(
    State(settings): State<Arc<Settings>>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    // Last, because it consumes the request. Taken here rather than in a route
    // of its own so that every gate below still runs before anything reads it.
    body: Body,
) -> Response {
    // The Host gate comes first and applies to everything, pages included:
    // it is what stops a page on an attacker's origin, resolving their name to
    // this address, from talking to this server at all.
    let host = header_str(&headers, header::HOST.as_str()).unwrap_or("");
    if !settings.allowed_hosts.allows(host) {
        return json(StatusCode::FORBIDDEN, r#"{"error":"forbidden"}"#);
    }

    // Then the CSRF gate, and before the token: the daemon's `do_POST` refuses
    // a bad Host and a bad Origin together, in one `_forbidden`, so a write
    // from another site is refused whatever token it carries and cannot be
    // told from a refused Host by its answer.
    if origin::gate(
        method.as_str(),
        header_str(&headers, SEC_FETCH_SITE),
        presented_origin(&headers),
        &settings.allowed_hosts,
    ) == Site::Cross
    {
        return json(StatusCode::FORBIDDEN, r#"{"error":"forbidden"}"#);
    }

    let query = Query::of(uri.query());
    let route = route::resolve(uri.path(), query.token.is_some());

    if route::gate(method.as_str(), route) == Gate::Token {
        let presented = auth::presented_token(
            header_str(&headers, auth::TOKEN_HEADER),
            header_str(&headers, header::COOKIE.as_str()),
        );
        if !auth::authorized(presented, &settings.token) {
            return json(StatusCode::UNAUTHORIZED, r#"{"error":"unauthorized"}"#);
        }
    }

    match route {
        // The handshake is a GET. The daemon has no `do_POST` route for
        // /login, so a POST there falls through to 404 — answering one with a
        // Set-Cookie would be a divergence nobody asked for.
        Route::Login if method == Method::GET => log_in(&settings, &query),
        Route::Asset(asset) if method == Method::GET || method == Method::HEAD => {
            serve_asset(asset)
        }
        Route::Panes if method == Method::GET || method == Method::HEAD => panes(&settings).await,
        Route::Pane if method == Method::GET || method == Method::HEAD => {
            terminal(&settings, &query).await
        }
        Route::Resize if method == Method::POST => resize(&settings, body).await,
        // An `/api/` path that got this far is authenticated and simply does
        // not exist yet; an unknown path never needed a token to be told so;
        // and a method with no route here is the same answer the daemon gives.
        Route::Panes
        | Route::Pane
        | Route::Resize
        | Route::Api
        | Route::NotFound
        | Route::Login
        | Route::Asset(_) => json(StatusCode::NOT_FOUND, r#"{"error":"not found"}"#),
    }
}

/// The one-time handshake: a token in the query becomes a cookie.
fn log_in(settings: &Settings, query: &Query) -> Response {
    if !auth::authorized(query.token.as_deref(), &settings.token) {
        return json(StatusCode::UNAUTHORIZED, r#"{"error":"unauthorized"}"#);
    }
    let next = login::safe_next(query.next.as_deref().unwrap_or("/"));
    // The cookie carries the *configured* token, not the presented one. They
    // are equal by the line above, and taking the configured one means a
    // future change to how tokens are compared cannot echo attacker input.
    build(
        StatusCode::SEE_OTHER,
        "text/plain",
        Body::empty(),
        |response| {
            response
                .header(header::LOCATION, next)
                .header(header::SET_COOKIE, login::set_cookie(&settings.token))
        },
    )
}

/// Every live pane on this machine, with every id already qualified.
///
/// A tmux that could not be asked is answered as an empty list *carrying the
/// reason*, at 200 rather than 500. The panel polls this every couple of
/// seconds and has to keep rendering; what it must never do is show an empty
/// list that means "nothing needs you" when the truth is "I could not look".
///
/// **Nothing reads that field yet.** `share/app.html` checks only that `panes`
/// is an array, so today the reason reaches the journal and no further; the
/// panel learning to render it is a change to a file the Python daemon is still
/// serving, and belongs to the ticket that retires that daemon. The field is
/// here now because the alternative — adding it later — means every endpoint
/// written in between decides for itself what a failure looks like.
async fn panes(settings: &Settings) -> Response {
    let here = settings.peers.here().name();
    let body = match settings.tmux.panes(here).await {
        Ok(panes) => ayeaye_core::tmux::panes_body(here, &panes, None),
        Err(trouble) => {
            // On stderr as well, because that is where it is read today. The
            // panel polls, so a machine with no tmux at all writes this line
            // every couple of seconds per client: worth a rate limit once
            // something else reads the field, not worth state on `Settings`
            // while the journal is the only reader there is.
            eprintln!("ayeaye: {trouble}");
            ayeaye_core::tmux::panes_body(here, &[], Some(&trouble.to_string()))
        }
    };
    build(
        StatusCode::OK,
        "application/json",
        Body::from(body),
        |response| response,
    )
}

/// One pane's terminal view.
///
/// `df=1` opts into the diff protocol; its absence keeps the whole-text shape,
/// so a page cached from before the protocol existed still renders rather than
/// showing nothing.
///
/// The lease is renewed here, as a side effect of the pane being watched. That
/// is the whole shape of auto-fit: viewing the terminal *is* the request, so a
/// client that stops watching for any reason — including having ceased to exist
/// — stops renewing, and the sweeper gives the desktop its window back.
async fn terminal(settings: &Settings, query: &Query) -> Response {
    let Some(asked) = query.id.as_deref() else {
        return json(StatusCode::BAD_REQUEST, r#"{"error":"no pane"}"#);
    };
    let Some(id) = local_pane(settings, asked).await else {
        return json(StatusCode::NOT_FOUND, r#"{"error":"no such pane"}"#);
    };
    settings.fits.touch(&id);

    let captured = settings
        .tmux
        .ask(&[
            "capture-pane",
            "-p",
            // The escapes stay: the browser turns them into styled spans, and a
            // terminal view without colour is a different program's output.
            "-e",
            "-t",
            id.pane(),
            "-S",
            &format!("-{}", pane::LINES),
        ])
        .await;
    let raw = match captured {
        Ok(raw) => raw,
        // Said out loud rather than served as an empty screen. The page leaves
        // what it is showing alone when neither shape arrives, which is the
        // right thing to do about a pane nobody could look at.
        Err(trouble) => {
            eprintln!("ayeaye: {trouble}");
            return json_body(json_object(&[("error", &trouble.to_string())]));
        }
    };
    let (cols, rows) = grid(settings, &id).await;

    // One capture for history and screen together, not one each: the pane
    // scrolls between two calls, and a line that moved in the gap would arrive
    // twice or not at all.
    let lines = pane::lines(&raw);
    if !query.diff {
        return json_body(pane::whole_body(&pane::whole(&lines), cols, rows));
    }
    let view = pane::View::split(lines, rows);
    let diff = settings.pane_cache.lock().unwrap_or_else(|held| held.into_inner()).diff(
        &id.qualified(),
        &view,
        cols,
        rows,
        query.hh.as_deref().unwrap_or(""),
        query.sh.as_deref().unwrap_or(""),
    );
    json_body(diff.body())
}

/// Resize a window, and take or drop the lease that holds it there.
///
/// tmux keeps one grid per pane, so the phone and the desktop cannot each have
/// their own width — resizing here reflows the window for every attached
/// client. The agent TUIs reflow cleanly, so `auto` holds the fit as a lease and
/// the desktop gets its window back the moment the phone stops watching.
async fn resize(settings: &Settings, body: Body) -> Response {
    let Ok(bytes) = axum::body::to_bytes(body, MAX_BODY).await else {
        return json(StatusCode::BAD_REQUEST, r#"{"error":"bad json"}"#);
    };
    // The one place this daemon is handed JSON. It is read in the shell rather
    // than in the core, which declares no dependencies and has no reader.
    let Ok(asked) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return json(StatusCode::BAD_REQUEST, r#"{"error":"bad json"}"#);
    };
    let Some(pane_id) = asked.get("pane").and_then(serde_json::Value::as_str) else {
        return json(StatusCode::BAD_REQUEST, r#"{"error":"no pane"}"#);
    };
    let Some(id) = local_pane(settings, pane_id).await else {
        return json(StatusCode::NOT_FOUND, r#"{"error":"no such pane"}"#);
    };

    if truthy(&asked, "release") {
        // End an auto-fit lease: back to the recorded pre-fit state.
        let restored = settings.fits.release(&settings.tmux, &id).await;
        return json_body(format!("{{\"ok\":true,\"restored\":{restored}}}"));
    }
    if truthy(&asked, "restore") {
        // The manual escape hatch: drop any lease and hand sizing back to tmux
        // outright, whatever the window was before.
        settings.fits.forget(&settings.tmux, &id).await;
        return json(StatusCode::OK, r#"{"ok":true,"restored":true}"#);
    }

    let (Some(cols), Some(rows)) = (number(&asked, "cols"), number(&asked, "rows")) else {
        return json(StatusCode::BAD_REQUEST, r#"{"error":"bad size"}"#);
    };
    let (cols, rows) = ayeaye_core::fit::clamp(cols, rows);
    if truthy(&asked, "auto") {
        // Before the resize, or the state recorded as "what this window was" is
        // the size the resize is about to give it.
        settings.fits.acquire(&settings.tmux, &id).await;
    }
    crate::fit::resize(&settings.tmux, id.pane(), cols, rows).await;
    json_body(format!("{{\"ok\":true,\"cols\":{cols},\"rows\":{rows}}}"))
}

/// The largest resize body this server will read.
///
/// A resize is five short fields. The limit is here because `to_bytes` needs
/// one, and because a request that is not a resize should be refused by its size
/// before it is parsed.
const MAX_BODY: usize = 64 * 1024;

/// A pane on *this* machine that is really in the pane list.
///
/// Three refusals in one, and all three matter:
///
/// - the id is un-qualified through the registry, because `share/app.html` holds
///   `host/pane` ids and hands them straight back;
/// - a peer's pane is not this machine's to capture — the transport for that is
///   the multi-host milestone's;
/// - and **membership, not syntax**, is the defence against a forged target.
///   `PaneId` only says an id could live in a qualified id; the daemon checks
///   the id against the pane list it just read, and so does this. It costs one
///   `list-panes` per poll, which is the price of not re-deriving the "not a
///   scratch session, not dead" rule a second time somewhere it could drift.
async fn local_pane(settings: &Settings, asked: &str) -> Option<PaneId> {
    let (peer, id) = settings.peers.route(asked).ok()?;
    if !peer.is_here() {
        return None;
    }
    let panes = settings
        .tmux
        .panes(settings.peers.here().name())
        .await
        .ok()?;
    panes.iter().any(|pane| pane.id == id).then_some(id)
}

/// The grid this pane is drawn at, so the client can render at the width tmux
/// actually wrapped for.
///
/// A size tmux would not give is zero, which the split reads as "all screen" —
/// the answer that cannot turn a repaint into an append.
async fn grid(settings: &Settings, id: &PaneId) -> (u16, u16) {
    let said = settings
        .tmux
        .ask(&[
            "display-message",
            "-p",
            "-t",
            id.pane(),
            "#{pane_width}\t#{pane_height}",
        ])
        .await
        .unwrap_or_default();
    ayeaye_core::fit::size(&said).unwrap_or((0, 0))
}

/// Whether a field in the request means yes.
///
/// `parse_qs`-free, but the same reading the daemon's `req.get(...)` gets: the
/// page sends `true`, and anything else that is plainly not empty is taken as
/// meaning it too.
fn truthy(asked: &serde_json::Value, field: &str) -> bool {
    match asked.get(field) {
        Some(serde_json::Value::Bool(yes)) => *yes,
        Some(serde_json::Value::Number(number)) => number.as_f64().is_some_and(|n| n != 0.0),
        Some(serde_json::Value::String(text)) => !text.is_empty(),
        _ => false,
    }
}

/// A number in the request, or `None` if there is something there that is not
/// one.
///
/// A field that is missing is zero, which clamps to the smallest allowed size —
/// that is `req.get("cols", 0)`, and it means a client that forgot a field gets
/// a usable window rather than an error. A field carrying something that is not
/// a number is a different thing entirely and is refused.
fn number(asked: &serde_json::Value, field: &str) -> Option<i64> {
    match asked.get(field) {
        None | Some(serde_json::Value::Null) => Some(0),
        Some(serde_json::Value::Number(number)) => number
            .as_i64()
            .or_else(|| number.as_f64().map(|value| value as i64)),
        Some(serde_json::Value::String(text)) => text.trim().parse().ok(),
        _ => None,
    }
}

/// A JSON object of string fields, escaped.
fn json_object(fields: &[(&str, &str)]) -> String {
    let mut out = String::from("{");
    for (index, (name, value)) in fields.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push_str(&format!(
            "{}:{}",
            ayeaye_core::json::string(name),
            ayeaye_core::json::string(value)
        ));
    }
    out.push('}');
    out
}

/// A JSON body this server built at runtime.
fn json_body(text: String) -> Response {
    build(
        StatusCode::OK,
        "application/json",
        Body::from(text),
        |response| response,
    )
}

/// A file compiled into the binary.
fn serve_asset(asset: Asset) -> Response {
    match assets::bytes(asset.file) {
        Some(bytes) => build(
            StatusCode::OK,
            asset.content_type,
            Body::from(bytes),
            |response| response,
        ),
        // Unreachable while `tests/assets.rs` passes — it asserts every file
        // the route table names is embedded. Answered rather than panicked
        // because a 500 on one icon should not take the process down.
        None => json(
            StatusCode::INTERNAL_SERVER_ERROR,
            r#"{"error":"missing asset"}"#,
        ),
    }
}

fn json(status: StatusCode, body: &'static str) -> Response {
    build(status, "application/json", Body::from(body), |r| r)
}

/// Every response goes through here, so every response carries `no-store`.
///
/// The pages and the API both describe live state; a cached one is a lie about
/// a pane, which is exactly the daemon's reasoning for setting it on all of
/// them rather than picking.
fn build(
    status: StatusCode,
    content_type: &str,
    body: Body,
    extra: impl FnOnce(axum::http::response::Builder) -> axum::http::response::Builder,
) -> Response {
    let response = Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, content_type)
        .header(header::CACHE_CONTROL, "no-store");
    extra(response)
        .body(body)
        .expect("the headers this server sets are always valid")
}

fn header_str<'h>(headers: &'h HeaderMap, name: &str) -> Option<&'h str> {
    headers.get(name).and_then(|value| value.to_str().ok())
}

/// The `Origin:` header, told apart from a request that carries none.
///
/// Not `header_str`: for `Origin`, "not there" is the branch that *allows* —
/// a non-browser client sends no Origin and is judged by its token instead —
/// so a header that arrived with bytes `to_str` cannot render must not come
/// back as `None`. `Origin::Unreadable` is what carries that difference into
/// the gate, which refuses it.
fn presented_origin(headers: &HeaderMap) -> Origin<'_> {
    match headers.get(header::ORIGIN) {
        None => Origin::Absent,
        Some(value) => value.to_str().map_or(Origin::Unreadable, Origin::Value),
    }
}

/// The two query parameters this server reads.
///
/// Percent-decoded, and `+` read as a space, because that is what the daemon's
/// `parse_qs` does — and a `next` compared before decoding is a check looking
/// at different text than the browser will follow. Blank values are dropped,
/// also matching `parse_qs`, so `/?token=` is the app rather than a login that
/// could only fail.
struct Query {
    token: Option<String>,
    next: Option<String>,
    /// The qualified pane id the terminal view is about.
    id: Option<String>,
    /// `df=1`: whether the client speaks the diff protocol.
    diff: bool,
    /// The token naming the history window the client says it holds.
    hh: Option<String>,
    /// The token naming the screen it says it holds.
    sh: Option<String>,
}

impl Query {
    fn of(raw: Option<&str>) -> Query {
        let mut query = Query {
            token: None,
            next: None,
            id: None,
            diff: false,
            hh: None,
            sh: None,
        };
        for (key, value) in form_urlencoded::parse(raw.unwrap_or("").as_bytes()) {
            if value.is_empty() {
                continue;
            }
            // First occurrence wins, as `parse_qs(...)[0]` does.
            match key.as_ref() {
                "token" if query.token.is_none() => query.token = Some(value.into_owned()),
                "next" if query.next.is_none() => query.next = Some(value.into_owned()),
                "id" if query.id.is_none() => query.id = Some(value.into_owned()),
                // Presence, not value: the daemon reads `q.get("df")` for truth
                // and the page only ever sends `df=1`.
                "df" => query.diff = true,
                "hh" if query.hh.is_none() => query.hh = Some(value.into_owned()),
                "sh" if query.sh.is_none() => query.sh = Some(value.into_owned()),
                _ => {}
            }
        }
        query
    }
}
