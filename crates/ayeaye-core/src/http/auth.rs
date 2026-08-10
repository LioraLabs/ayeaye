//! Who is allowed to ask.
//!
//! Network placement — a tailnet, or loopback — is the front door, but it is
//! not sufficient on its own: every peer on the tailnet would otherwise hold
//! what amounts to shell access. So every gated request presents a shared
//! secret, as an `X-Voice-Token` header or as the `voice_token` cookie the
//! login handshake sets so a phone only ever types it once.

/// Compare two secrets without letting the time taken describe them.
///
/// Every byte of the shorter run is compared whatever the earlier bytes said:
/// there is no early return, so a caller cannot learn how long a shared prefix
/// was by measuring. The lengths themselves are compared up front and leak, as
/// they do in Python's `hmac.compare_digest` over bytes — a token's length is
/// not the secret.
///
/// This is hand-rolled rather than taken from `constant_time_eq` or `subtle`
/// because the core's dependency allowlist is empty by design; see
/// `crates/constitution/README.md`.
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut difference = 0u8;
    for (left, right) in a.iter().zip(b) {
        difference |= left ^ right;
    }
    difference == 0
}

/// The name of the cookie the login handshake sets.
pub const COOKIE_NAME: &str = "voice_token";

/// The name of the header a non-browser client presents its token in.
pub const TOKEN_HEADER: &str = "X-Voice-Token";

/// The token a request is presenting, if it presents one at all.
///
/// The header wins; the cookie is the fallback, so a browser that logged in
/// once keeps working while a script can override it per request. An empty
/// header is the same as no header, which is what makes the fallback reachable
/// from a browser that sends both.
pub fn presented_token<'a>(
    token_header: Option<&'a str>,
    cookie_header: Option<&'a str>,
) -> Option<&'a str> {
    token_header
        .filter(|token| !token.is_empty())
        .or_else(|| cookie_header.and_then(|jar| cookie_value(jar, COOKIE_NAME)))
        .filter(|token| !token.is_empty())
}

/// One cookie's value out of a `Cookie:` header, hand-parsed.
///
/// Hand-rolled because the `cookie` crate carries a non-optional dependency on
/// a clock, which the pure core may not have. The need is small: split on `;`,
/// match the name, unquote the value.
pub fn cookie_value<'a>(cookie_header: &'a str, name: &str) -> Option<&'a str> {
    cookie_header.split(';').find_map(|pair| {
        let (found, value) = pair.split_once('=')?;
        if found.trim() != name {
            return None;
        }
        let value = value.trim();
        Some(
            value
                .strip_prefix('"')
                .and_then(|inner| inner.strip_suffix('"'))
                .unwrap_or(value),
        )
    })
}

/// Whether a presented token is the one this daemon was configured with.
///
/// Absent and empty are both refusals, and they are refused *before* the
/// comparison: an empty configured token would otherwise make every request
/// authorised, which is the failure mode a misread config file produces.
pub fn authorized(presented: Option<&str>, expected: &str) -> bool {
    if expected.is_empty() {
        return false;
    }
    match presented {
        Some(token) if !token.is_empty() => constant_time_eq(token.as_bytes(), expected.as_bytes()),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::{authorized, constant_time_eq, cookie_value, presented_token};

    // AYEAYE-42 — the comparison still has to be a comparison. A difference in
    // the first byte and a difference in the last byte are both refusals; the
    // last-byte case is the one an early return would get wrong.
    #[test]
    fn secrets_match_only_when_every_byte_does() {
        assert!(constant_time_eq(b"s3cret", b"s3cret"));
        assert!(!constant_time_eq(b"s3cret", b"X3cret"));
        assert!(!constant_time_eq(b"s3cret", b"s3creX"));
    }

    // AYEAYE-42 — a shorter guess is not a match, and comparing a prefix
    // against the whole must not panic on the zip.
    #[test]
    fn a_length_mismatch_is_never_a_match() {
        assert!(!constant_time_eq(b"s3cre", b"s3cret"));
        assert!(!constant_time_eq(b"s3cret", b"s3cre"));
        assert!(!constant_time_eq(b"", b"s3cret"));
        assert!(constant_time_eq(b"", b""));
    }

    // AYEAYE-42 — the header wins over the cookie, and an empty header is the
    // same as no header, so a browser sending both still logs in.
    #[test]
    fn the_header_wins_and_the_cookie_is_the_fallback() {
        assert_eq!(
            presented_token(Some("from-header"), Some("voice_token=from-cookie")),
            Some("from-header")
        );
        assert_eq!(
            presented_token(None, Some("voice_token=from-cookie")),
            Some("from-cookie")
        );
        assert_eq!(
            presented_token(Some(""), Some("voice_token=from-cookie")),
            Some("from-cookie")
        );
        assert_eq!(presented_token(None, None), None);
    }

    // AYEAYE-42 — a `Cookie:` header is whatever the browser has accumulated,
    // so the parse has to survive other cookies, spaces, quotes and junk
    // without finding a token that is not there.
    #[test]
    fn the_cookie_is_found_among_the_others_and_never_invented() {
        assert_eq!(
            cookie_value("theme=dark; voice_token=abc123; tz=UTC", "voice_token"),
            Some("abc123")
        );
        assert_eq!(cookie_value("voice_token=\"abc123\"", "voice_token"), Some("abc123"));
        assert_eq!(cookie_value("theme=dark", "voice_token"), None);
        assert_eq!(cookie_value("", "voice_token"), None);
        // A name that merely ends with ours is a different cookie.
        assert_eq!(cookie_value("not_voice_token=abc123", "voice_token"), None);
        // A flag with no `=` must not be read as a nameless value.
        assert_eq!(cookie_value("voice_token; theme=dark", "voice_token"), None);
    }

    // AYEAYE-42 — nothing presented is a refusal, and so is an empty
    // configured token: a config that failed to load must close the door, not
    // open it to every request that also presents nothing.
    #[test]
    fn nothing_presented_and_nothing_configured_are_both_refusals() {
        assert!(authorized(Some("s3cret"), "s3cret"));
        assert!(!authorized(Some("wrong"), "s3cret"));
        assert!(!authorized(None, "s3cret"));
        assert!(!authorized(Some(""), "s3cret"));
        assert!(!authorized(Some(""), ""));
        assert!(!authorized(None, ""));
    }
}
