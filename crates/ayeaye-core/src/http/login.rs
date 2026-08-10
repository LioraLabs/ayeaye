//! The one-time handshake that turns a token in a URL into a cookie.
//!
//! A phone types the token once, from a bookmarked `/?token=…`, and every
//! later visit is clean. That makes the redirect target attacker-supplied, so
//! it is checked here rather than trusted.

use super::auth::COOKIE_NAME;

/// How long the login cookie lives, in seconds. A year: the phone should not
/// have to be told the token twice.
pub const COOKIE_MAX_AGE: u32 = 31_536_000;

/// Where to send the browser after a successful login.
///
/// Only a path on this origin is honoured. Everything else becomes `/`, so a
/// crafted `?next=` cannot turn the login URL into an open redirect that
/// launders somebody else's link through a host the user trusts.
///
/// Control characters are refused as well, and that is not the same concern.
/// This value goes into a `Location:` header, so a CR or LF in it is response
/// splitting — the daemon writes headers by hand and would splice the injected
/// line straight through; a server that validates its headers instead refuses
/// to build the response at all, which is a panic on an attacker-supplied
/// string. Neither is acceptable, and the check belongs here rather than at
/// whichever of the two is writing the header this week.
pub fn safe_next(next: &str) -> &str {
    let local = next.starts_with('/') && !next.starts_with("//") && !next.contains('\\');
    if local && !next.chars().any(is_forbidden) {
        next
    } else {
        "/"
    }
}

/// Characters no URL path may carry into a header.
///
/// C0, DEL and space. A space is legal in neither a request target nor a
/// header value, and percent-decoding a `next` can produce one.
fn is_forbidden(character: char) -> bool {
    character.is_control() || character == ' ' || character == '\u{7f}'
}

/// The `Set-Cookie` value the handshake returns.
///
/// `HttpOnly` keeps the token out of any script on the page, and
/// `SameSite=Strict` means a cross-site request never carries it — which is
/// the reason a stolen link cannot act on the user's behalf even before the
/// Host allow-list is consulted.
pub fn set_cookie(token: &str) -> String {
    format!("{COOKIE_NAME}={token}; Path=/; Max-Age={COOKIE_MAX_AGE}; HttpOnly; SameSite=Strict")
}

#[cfg(test)]
mod tests {
    use super::{safe_next, set_cookie};

    // AYEAYE-42 — `next` arrives in a URL anyone can craft, so a path on this
    // origin is honoured and every other shape collapses to `/`.
    #[test]
    fn only_a_local_path_survives_as_the_redirect_target() {
        assert_eq!(safe_next("/board"), "/board");
        assert_eq!(safe_next("/"), "/");
        // Protocol-relative: the browser reads `//evil.example` as a host.
        assert_eq!(safe_next("//evil.example"), "/");
        // Backslashes are normalised to slashes by some browsers.
        assert_eq!(safe_next("/\\evil.example"), "/");
        // A CR or LF would be spliced into the `Location:` header by anything
        // that writes headers by hand, which is a response-splitting hole and,
        // in a server that validates its headers, a panic instead.
        assert_eq!(safe_next("/a\r\nX-Injected: yes"), "/");
        assert_eq!(safe_next("/a\nX-Injected: yes"), "/");
        assert_eq!(safe_next("/a\0b"), "/");
        assert_eq!(safe_next("/a\x7fb"), "/");
        assert_eq!(safe_next("https://evil.example"), "/");
        assert_eq!(safe_next("board"), "/");
        assert_eq!(safe_next(""), "/");
    }

    // AYEAYE-42 — the cookie is the daemon's existing one, attribute for
    // attribute: same name, so a browser already logged in to the Python
    // daemon is logged in here too.
    #[test]
    fn the_cookie_is_the_one_the_daemon_already_sets() {
        assert_eq!(
            set_cookie("s3cret"),
            "voice_token=s3cret; Path=/; Max-Age=31536000; HttpOnly; SameSite=Strict"
        );
    }
}
