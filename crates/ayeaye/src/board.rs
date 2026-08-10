//! The board tab's endpoints.
//!
//! Three questions, each one a run of cliban and a shaping of what it said.
//! Nothing here decides anything: which lines are rows, what the payload looks
//! like and whether a key is a key all come from `ayeaye_core::board`, which a
//! test can reach without starting a process.
//!
//! There is no route table here either. Everything under `/api/` reaches this
//! through the server's one handler, which has already applied the Host gate
//! and the token gate; a path that does not match below is `None` and becomes
//! the server's 404. That is deliberate — an endpoint mounted on the router
//! with `.route(…)` would skip both, and nothing would say so. Every endpoint
//! here is a GET and none of them writes, so the gates that judge a write do
//! not come into it.

use axum::http::StatusCode;
use ayeaye_core::board as shape;
use ayeaye_core::json;

use crate::cliban::Cliban;
use crate::config::Settings;

/// Everything the board renders, in one payload.
const BOARD: &str = "/api/cliban/board";
/// Just the project keys, for the app page's ticket links.
const PROJECTS: &str = "/api/cliban/projects";
/// One issue, by key.
const ISSUE: &str = "/api/cliban/issue";

/// Answer a board endpoint, or `None` if this is not one.
pub async fn answer(
    settings: &Settings,
    path: &str,
    query: Option<&str>,
) -> Option<(StatusCode, String)> {
    let cliban = &settings.cliban;
    match path {
        BOARD => Some(board(cliban).await),
        PROJECTS => Some(projects(cliban).await),
        ISSUE => Some(issue(cliban, query).await),
        _ => None,
    }
}

/// The whole board: projects, milestones and issues in one fetch.
///
/// Only the first failure is a failure. The daemon ignores the errors from the
/// other two calls, and that is worth keeping: a board with no milestones is
/// still a board, where a board with no projects has nothing to render at all.
async fn board(cliban: &Cliban) -> (StatusCode, String) {
    let projects = match cliban.run(&["project", "ls", "--json"]).await {
        Ok(said) => said,
        Err(why) => return (StatusCode::INTERNAL_SERVER_ERROR, shape::failure(&why)),
    };
    let milestones = cliban
        .run(&["milestone", "ls", "--json", "--sort", "activity"])
        .await
        .unwrap_or_default();
    let issues = cliban
        .run(&["issue", "ls", "--json", "--sort", "position"])
        .await
        .unwrap_or_default();

    (
        StatusCode::OK,
        shape::payload(
            &shape::rows(&projects),
            &shape::rows(&milestones),
            &shape::rows(&issues),
        ),
    )
}

/// The project keys, and never an error.
///
/// The app page fetches this to linkify ticket references and swallows a
/// failure into a silent `catch`, so an error here is indistinguishable from
/// no links at all — except that it would also be a 500 in the log for
/// something nobody asked to be told about. No cliban, no links.
async fn projects(cliban: &Cliban) -> (StatusCode, String) {
    let said = cliban
        .run(&["project", "ls", "--json"])
        .await
        .unwrap_or_default();
    (StatusCode::OK, shape::project_keys(&shape::rows(&said)))
}

/// One issue, passed through as cliban rendered it.
async fn issue(cliban: &Cliban, query: Option<&str>) -> (StatusCode, String) {
    let key = first(query, "key").unwrap_or_default();
    if !shape::is_issue_key(&key) {
        // Before anything is started: the key is the one thing a caller
        // chooses that lands in an argv.
        return (StatusCode::BAD_REQUEST, shape::failure("bad key"));
    }
    match cliban.run(&["issue", "show", &key, "--json"]).await {
        // Handed back as cliban wrote it, not re-rendered. It has been checked
        // to parse, which is the only claim this endpoint makes about it.
        Ok(said) if json::is_value(&said) => (StatusCode::OK, said),
        Ok(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            shape::failure("unreadable cliban output"),
        ),
        Err(why) => (StatusCode::INTERNAL_SERVER_ERROR, shape::failure(&why)),
    }
}

/// The first non-empty value of a query parameter.
///
/// Percent-decoded, `+` read as a space, and blanks dropped — `parse_qs`, which
/// is what the daemon reads its query with. Decoding first matters: a check
/// that looked at the raw text would be judging something other than the string
/// that reaches the argv.
fn first(query: Option<&str>, name: &str) -> Option<String> {
    form_urlencoded::parse(query.unwrap_or("").as_bytes())
        .find(|(key, value)| key == name && !value.is_empty())
        .map(|(_, value)| value.into_owned())
}
