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

/// The header a browser labels a request's provenance with. `http` has no
/// constant for it: it is a fetch-metadata header, newer than the crate's
/// table.
const SEC_FETCH_SITE: &str = "sec-fetch-site";

use crate::assets;
use crate::board;
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
    axum::serve(listener, router(settings)).await
}

async fn handle(
    State(settings): State<Arc<Settings>>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
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
        // Authenticated by the gate above, whatever it turns out to name. The
        // endpoints live in one module rather than on the router, so they
        // inherit every gate in this handler instead of each having to
        // remember them.
        Route::Api if method == Method::GET => {
            match board::answer(&settings, uri.path(), uri.query()).await {
                Some((status, body)) => json_owned(status, body),
                None => json(StatusCode::NOT_FOUND, r#"{"error":"not found"}"#),
            }
        }
        // An `/api/` path that got this far is authenticated and simply does
        // not exist yet; an unknown path never needed a token to be told so;
        // and a method with no route here is the same answer the daemon gives.
        Route::Panes | Route::Api | Route::NotFound | Route::Login | Route::Asset(_) => {
            json(StatusCode::NOT_FOUND, r#"{"error":"not found"}"#)
        }
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

/// The same, for a body that was assembled rather than written out.
fn json_owned(status: StatusCode, body: String) -> Response {
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
}

impl Query {
    fn of(raw: Option<&str>) -> Query {
        let mut token = None;
        let mut next = None;
        for (key, value) in form_urlencoded::parse(raw.unwrap_or("").as_bytes()) {
            if value.is_empty() {
                continue;
            }
            // First occurrence wins, as `parse_qs(...)[0]` does.
            match key.as_ref() {
                "token" if token.is_none() => token = Some(value.into_owned()),
                "next" if next.is_none() => next = Some(value.into_owned()),
                _ => {}
            }
        }
        Query { token, next }
    }
}
