//! What a request path means, and whether it is gated.
//!
//! One table rather than a chain of `if`s in the server, so "the pages are
//! open and `/api/*` is gated" is a fact a test can read rather than a shape
//! that has to be re-derived from the handler.

/// A file that ships inside the binary, and how to label it on the way out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Asset {
    /// The file's name under `share/`, which is also its key in the binary's
    /// embedded table. The bytes live in the shell; the core only names them.
    pub file: &'static str,
    /// The `Content-Type` this file is served with.
    pub content_type: &'static str,
}

/// What a request path resolves to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    /// The login handshake: a token in the query becomes a cookie.
    Login,
    /// A file compiled into the binary.
    Asset(Asset),
    /// Anything under `/api/`. Gated, whether or not it exists.
    Api,
    /// Nothing here.
    NotFound,
}

/// Whether a route may be reached without a token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gate {
    /// No token needed. The pages carry no data and no secrets; every call
    /// they go on to make is what is gated.
    Open,
    /// A valid token, or 401.
    Token,
}

const HTML: &str = "text/html; charset=utf-8";

/// Every path the daemon answers on, and the file it answers with.
///
/// The aliases are real: `/message` is a deep link into the same single-page
/// app, and `/board.html` is what a bookmark from an older build says.
pub const ASSET_ROUTES: &[(&str, Asset)] = &[
    ("/", asset("app.html", HTML)),
    ("/index.html", asset("app.html", HTML)),
    ("/message", asset("app.html", HTML)),
    ("/board", asset("board.html", HTML)),
    ("/board.html", asset("board.html", HTML)),
    // Static, data-free, and needed before login: the manifest is what makes
    // "add to home screen" install a named, icon'd app.
    (
        "/manifest.webmanifest",
        asset("manifest.webmanifest", "application/manifest+json"),
    ),
    // The favicon and the small header mark use the simplified small-size art;
    // the touch and manifest sizes use the full-detail art.
    ("/favicon.ico", asset("favicon.ico", "image/x-icon")),
    ("/icon-64.png", asset("icon-64.png", PNG)),
    ("/icon-180.png", asset("icon-180.png", PNG)),
    ("/icon-192.png", asset("icon-192.png", PNG)),
    ("/icon-512.png", asset("icon-512.png", PNG)),
    ("/icon-maskable-192.png", asset("icon-maskable-192.png", PNG)),
    ("/icon-maskable-512.png", asset("icon-maskable-512.png", PNG)),
];

const PNG: &str = "image/png";

const fn asset(file: &'static str, content_type: &'static str) -> Asset {
    Asset { file, content_type }
}

/// The paths where a `?token=` is read as a login rather than passed over.
///
/// Only the app's own entry points, so the one link a phone is given is the
/// one it already bookmarks.
const LOGIN_ENTRY_PATHS: &[&str] = &["/", "/index.html"];

/// What a path means.
///
/// `has_token_query` is whether the request carried a `token` parameter at
/// all — not whether it was the right one, which is the caller's business and
/// needs the secret this module does not have.
pub fn resolve(path: &str, has_token_query: bool) -> Route {
    if path == "/login" || (has_token_query && LOGIN_ENTRY_PATHS.contains(&path)) {
        return Route::Login;
    }
    // Before the asset table, and before anything can 404: a path under /api/
    // is gated whether or not an endpoint answers on it, so an unauthenticated
    // caller cannot map the API by watching which paths come back 404.
    if path.starts_with(API_PREFIX) {
        return Route::Api;
    }
    if let Some((_, asset)) = ASSET_ROUTES.iter().find(|(route, _)| *route == path) {
        return Route::Asset(*asset);
    }
    Route::NotFound
}

/// Everything below this is the API. The trailing slash matters: `/apifake` is
/// not the API, and must not be gated as though it were.
const API_PREFIX: &str = "/api/";

impl Route {
    /// Whether this route may be reached without a token.
    pub fn gate(self) -> Gate {
        match self {
            Route::Api => Gate::Token,
            // The pages carry no data and no secrets, and the login handshake
            // presents its own token in the query. `NotFound` is open on
            // purpose: a 401 on an unknown path would tell an unauthenticated
            // caller which paths are real.
            Route::Login | Route::Asset(_) | Route::NotFound => Gate::Open,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Gate, Route, resolve};

    fn asset_file(path: &str) -> &'static str {
        match resolve(path, false) {
            Route::Asset(asset) => asset.file,
            other => panic!("{path} resolved to {other:?}, not an asset"),
        }
    }

    // AYEAYE-42 — the app answers on three paths and the board on two, which
    // is what makes an old bookmark and the `/message` deep link keep working.
    #[test]
    fn the_pages_answer_on_every_path_the_daemon_answers_on() {
        for path in ["/", "/index.html", "/message"] {
            assert_eq!(asset_file(path), "app.html", "for {path}");
        }
        for path in ["/board", "/board.html"] {
            assert_eq!(asset_file(path), "board.html", "for {path}");
        }
        assert_eq!(asset_file("/manifest.webmanifest"), "manifest.webmanifest");
        assert_eq!(asset_file("/icon-192.png"), "icon-192.png");
        assert_eq!(resolve("/nope", false), Route::NotFound);
    }

    // AYEAYE-42 — a token in the query turns the app's own URL into the login
    // handshake, which is what lets a phone be handed one bookmarkable link.
    // Without a token the same path is just the app, or the bookmark would
    // stop working the moment the cookie was set.
    #[test]
    fn a_token_in_the_query_makes_the_entry_paths_a_login() {
        assert_eq!(resolve("/login", false), Route::Login);
        assert_eq!(resolve("/", true), Route::Login);
        assert_eq!(resolve("/index.html", true), Route::Login);
        assert_eq!(asset_file("/"), "app.html");
        // Not every path is an entry point: a token on the board is ignored,
        // exactly as the daemon ignores it today.
        assert_eq!(
            match resolve("/board", true) {
                Route::Asset(asset) => asset.file,
                other => panic!("/board?token= resolved to {other:?}"),
            },
            "board.html"
        );
    }

    // AYEAYE-42 — the gate is the point of the table. Everything under /api/
    // needs a token whether or not it exists; the pages do not, because they
    // carry no data and every call they go on to make is gated.
    #[test]
    fn everything_under_api_is_gated_and_nothing_else_is() {
        assert_eq!(resolve("/api/panes", false), Route::Api);
        assert_eq!(resolve("/api/", false), Route::Api);
        // A token in the query does not make an API path a login.
        assert_eq!(resolve("/api/panes", true), Route::Api);
        assert_eq!(Route::Api.gate(), Gate::Token);
        for path in [
            "/",
            "/index.html",
            "/message",
            "/board",
            "/manifest.webmanifest",
            "/favicon.ico",
            "/icon-maskable-512.png",
            "/login",
        ] {
            assert_eq!(
                resolve(path, false).gate(),
                Gate::Open,
                "{path} must not need a token"
            );
        }
        // An unknown path is a 404, not a 401: refusing it for the wrong
        // reason would tell an unauthenticated caller what exists.
        assert_eq!(Route::NotFound.gate(), Gate::Open);
        // A path that merely starts with the letters is not the API.
        assert_eq!(resolve("/apifake", false), Route::NotFound);
    }

}
