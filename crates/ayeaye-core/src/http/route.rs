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
const PNG: &str = "image/png";

/// Everything below this is the API. The trailing slash matters: `/apifake` is
/// not the API, and must not be gated as though it were.
const API_PREFIX: &str = "/api/";

const fn asset(file: &'static str, content_type: &'static str) -> Asset {
    Asset { file, content_type }
}

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
    (
        "/icon-maskable-192.png",
        asset("icon-maskable-192.png", PNG),
    ),
    (
        "/icon-maskable-512.png",
        asset("icon-maskable-512.png", PNG),
    ),
];

/// The paths where a `?token=` is read as a login rather than passed over.
///
/// Only the app's own entry points, so the one link a phone is given is the
/// one it already bookmarks.
const LOGIN_ENTRY_PATHS: &[&str] = &["/", "/index.html"];

/// What a path means.
///
/// `has_token_query` is whether the request carried a **non-empty** `token`
/// parameter — not whether it was the right one, which is the caller's
/// business and needs the secret this module does not have. Non-empty matters:
/// the daemon reads its query with `parse_qs`, which drops blank values, so
/// `/?token=` is the app rather than a login that is certain to fail.
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

/// Whether a request may be served without a token.
///
/// The method is half of the answer, not a detail. The daemon serves pages to
/// anyone who can reach it, but `do_POST` gates *every* POST before it looks
/// at the path at all — a POST acts on a pane, and no path is exempt from
/// that. So anything that could write is gated whatever it names, and the
/// route table only decides the reading case.
///
/// HEAD reads what GET reads and returns no body, so it is gated exactly where
/// GET is. That is a decision rather than an oversight: the server answers
/// HEAD out of the GET handler, and gating it would 401 the link previewers
/// and PWA install checks that HEAD an icon nobody needs a token to fetch.
/// The daemon refuses HEAD outright today (501, no `do_HEAD`), so this is a
/// deliberate widening, and it exposes nothing a GET does not already.
pub fn gate(method: &str, route: Route) -> Gate {
    if !matches!(method, "GET" | "HEAD") {
        return Gate::Token;
    }
    match route {
        Route::Api => Gate::Token,
        // The pages carry no data and no secrets, and the login handshake
        // presents its own token in the query. `NotFound` is open on purpose:
        // a 401 on an unknown path would tell an unauthenticated caller which
        // paths are real.
        Route::Login | Route::Asset(_) | Route::NotFound => Gate::Open,
    }
}

#[cfg(test)]
mod tests {
    use super::{Gate, Route, gate, resolve};

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

    // AYEAYE-42 — the whole table, path by path, against the daemon's own
    // `ICON_FILES` plus its APP_HTML/BOARD_HTML/MANIFEST routes. Spot-checking
    // a table this mechanical leaves a deleted row and a mistyped content type
    // both invisible; this is the list transcribed from `bin/ayeaye`, so it
    // disagrees with the code rather than agreeing with it by construction.
    #[test]
    fn every_served_path_carries_the_file_and_type_the_daemon_sends() {
        const EXPECTED: &[(&str, &str, &str)] = &[
            ("/", "app.html", "text/html; charset=utf-8"),
            ("/index.html", "app.html", "text/html; charset=utf-8"),
            ("/message", "app.html", "text/html; charset=utf-8"),
            ("/board", "board.html", "text/html; charset=utf-8"),
            ("/board.html", "board.html", "text/html; charset=utf-8"),
            (
                "/manifest.webmanifest",
                "manifest.webmanifest",
                "application/manifest+json",
            ),
            ("/favicon.ico", "favicon.ico", "image/x-icon"),
            ("/icon-64.png", "icon-64.png", "image/png"),
            ("/icon-180.png", "icon-180.png", "image/png"),
            ("/icon-192.png", "icon-192.png", "image/png"),
            ("/icon-512.png", "icon-512.png", "image/png"),
            (
                "/icon-maskable-192.png",
                "icon-maskable-192.png",
                "image/png",
            ),
            (
                "/icon-maskable-512.png",
                "icon-maskable-512.png",
                "image/png",
            ),
        ];

        for (path, file, content_type) in EXPECTED {
            match resolve(path, false) {
                Route::Asset(asset) => {
                    assert_eq!(asset.file, *file, "file for {path}");
                    assert_eq!(asset.content_type, *content_type, "type for {path}");
                }
                other => panic!("{path} resolved to {other:?}, not an asset"),
            }
        }
        // And nothing beyond them: a row added here without a reason is a path
        // the daemon does not answer on.
        assert_eq!(
            super::ASSET_ROUTES.len(),
            EXPECTED.len(),
            "the asset table and the daemon's routes have drifted apart"
        );
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

    // AYEAYE-42 — the gate is the point of the table. On a GET, everything
    // under /api/ needs a token whether or not it exists; the pages do not,
    // because they carry no data and every call they go on to make is gated.
    #[test]
    fn a_get_is_gated_under_api_and_open_on_the_pages() {
        assert_eq!(resolve("/api/panes", false), Route::Api);
        assert_eq!(resolve("/api/", false), Route::Api);
        // A token in the query does not make an API path a login.
        assert_eq!(resolve("/api/panes", true), Route::Api);
        assert_eq!(gate("GET", Route::Api), Gate::Token);
        for path in [
            "/",
            "/index.html",
            "/message",
            "/board",
            "/board.html",
            "/manifest.webmanifest",
            "/favicon.ico",
            "/icon-maskable-512.png",
            "/login",
        ] {
            assert_eq!(
                gate("GET", resolve(path, false)),
                Gate::Open,
                "{path} must not need a token"
            );
        }
        // An unknown path is a 404, not a 401: refusing it for the wrong
        // reason would tell an unauthenticated caller what exists.
        assert_eq!(gate("GET", Route::NotFound), Gate::Open);
        // A path that merely starts with the letters is not the API.
        assert_eq!(resolve("/apifake", false), Route::NotFound);
    }

    // AYEAYE-42 — the daemon's `do_POST` gates every POST before it looks at
    // the path: a POST acts on a pane, and no path is exempt. Anything that
    // could write is therefore gated whatever it names, or serving the pages
    // openly would open a hole the moment the first endpoint lands.
    #[test]
    fn nothing_that_could_write_is_ever_open() {
        for method in ["POST", "PUT", "PATCH", "DELETE", "OPTIONS", "get"] {
            for path in ["/", "/board", "/login", "/nope", "/api/send"] {
                assert_eq!(
                    gate(method, resolve(path, false)),
                    Gate::Token,
                    "{method} {path} must need a token"
                );
            }
        }
    }

    // AYEAYE-42 — HEAD reads what GET reads and returns no body, so it is
    // gated exactly where GET is. Deliberate: the server answers HEAD from the
    // GET handler, and refusing it would 401 the link previewers and PWA
    // install checks that HEAD an icon nobody needs a token to fetch.
    #[test]
    fn head_is_gated_exactly_where_get_is() {
        for path in ["/", "/board", "/favicon.ico", "/manifest.webmanifest"] {
            assert_eq!(
                gate("HEAD", resolve(path, false)),
                Gate::Open,
                "HEAD {path} must be as open as GET"
            );
        }
        assert_eq!(gate("HEAD", resolve("/api/panes", false)), Gate::Token);
    }
}
