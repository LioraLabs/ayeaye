//! Where a write says it came from.
//!
//! This is the CSRF gate, ported from the daemon's `_origin_ok`. A browser
//! always says who sent a request — with `Sec-Fetch-Site`, or with an `Origin`
//! naming the page it came from — so a request from somebody else's page can
//! be refused before it reaches a handler that would act on a pane.
//!
//! **Defence in depth, not the only defence.** The login cookie is
//! `SameSite=Strict`, so a cross-site request carries no cookie and would fail
//! the token gate behind this one anyway. This gate is what keeps that true if
//! the cookie's attributes ever change, and it is what refuses the request
//! before any body is read.
//!
//! **An `Origin` is allowed exactly when its netloc is a `Host` we would have
//! answered to**, judged by the same [`AllowedHosts`] the Host gate consults.
//! One list, so widening it widens both gates together and there is no second
//! place to forget.
//!
//! Three deliberate departures from CPython, stated rather than reproduced —
//! and a fourth on the netloc parse below:
//!
//! - `urlsplit` deletes ASCII tab and newline from anywhere in the URL before
//!   parsing, so CPython reads `http://loc<TAB>alhost:8911` as `localhost:8911`
//!   and admits it. This does not, and refusing is the direction that matters
//!   for a gate: no browser sends that, and anything that can put a tab in a
//!   header can equally send no `Origin` at all, which is allowed by design.
//! - The scheme is not checked, in either implementation. `http://` and
//!   `https://` on an allowlisted netloc are both allowed, because the netloc
//!   is what identifies the origin and the Host allow-list carries no scheme
//!   to compare it against.
//! - Python's `str.strip()` also strips `\x1c`–`\x1f`, which Rust's `trim` does
//!   not. On an `Origin` that leaves this stricter; on `Sec-Fetch-Site` it
//!   leaves it *weaker* — CPython refuses `\x1ccross-site` and this reads it as
//!   an unrecognised label. Unreachable over the wire, since an HTTP parser
//!   refuses header bytes below `0x20` other than tab and no page can set
//!   `Sec-Fetch-Site` in the first place; recorded because the direction is the
//!   dangerous one and a future caller that passes something other than a
//!   header value would inherit it.

use super::hosts::AllowedHosts;

/// Whether a request may act.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Site {
    /// Nothing says this came from anywhere else. On to the token gate.
    Same,
    /// The browser labelled this as coming from another site. 403.
    Cross,
}

/// The `Origin:` header, as the server found it.
///
/// Three states rather than an `Option<&str>`, because the two ways a server
/// can fail to produce a string mean opposite things. [`Origin::Absent`] is
/// the *allowing* branch — a non-browser client sends no `Origin` and is
/// judged by the token instead — so a header that arrived and could not be
/// read must not arrive here wearing the same face. This type is what makes
/// the caller choose out loud: there is no conversion from `Option<&str>`, so
/// a `to_str().ok()` cannot quietly turn a malformed header into an allowed
/// request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin<'a> {
    /// No `Origin` header at all.
    Absent,
    /// An `Origin` header whose bytes are not text.
    ///
    /// Refused. The daemon reads its headers as latin-1, so bytes that are not
    /// UTF-8 still become a string there — one whose netloc no ASCII
    /// allow-list can hold, which is a refusal by another route. No browser
    /// can produce this: `Origin` is a forbidden header name, so a page cannot
    /// set it, and what a browser sets is a serialised origin.
    Unreadable,
    /// The header's value, as it arrived.
    Value(&'a str),
}

/// The label a browser puts on a request that came from another site.
const CROSS_SITE: &str = "cross-site";

/// The origin a page has when it has no host to name: a sandboxed iframe, a
/// `file://` document, a cross-origin redirect.
const OPAQUE: &str = "null";

/// Whether a request may act on what it says it came from.
///
/// The method is half of the answer, as it is for [`super::route::gate`]: a
/// read is exempt and everything that could write is judged. Both header
/// arguments are the values as they arrived, untrimmed and in whatever case
/// they were sent.
///
/// `sec_fetch_site` is an `Option<&str>` where [`Origin`] is a three-state
/// type, and the asymmetry is deliberate: a value that is not UTF-8 can never
/// be the ASCII label this looks for, in this implementation or in CPython's,
/// so dropping an unreadable one loses nothing. An unreadable `Origin` loses a
/// refusal, which is why it has a name of its own.
pub fn gate(
    method: &str,
    sec_fetch_site: Option<&str>,
    origin: Origin<'_>,
    allowed: &AllowedHosts,
) -> Site {
    // A read is exempt, exactly as it is in the daemon, whose `do_GET` never
    // consults `_origin_ok`. HEAD rides with GET for the reason `route::gate`
    // gives: it is the read a link preview does, and it is labelled
    // `cross-site` while doing it.
    if matches!(method, "GET" | "HEAD") {
        return Site::Same;
    }
    if let Some(label) = sec_fetch_site
        && label.trim().eq_ignore_ascii_case(CROSS_SITE)
    {
        return Site::Cross;
    }
    let origin = match origin {
        // Nothing to judge, and nothing that could have been sent by a page.
        Origin::Absent => return Site::Same,
        // Something was sent and this server cannot say what: refused, because
        // the alternative is to treat it as though nothing had been sent.
        Origin::Unreadable => return Site::Cross,
        Origin::Value(value) => value.trim(),
    };
    // A header that is present but blank is the same as no header: the daemon
    // reads it with `if not origin`, and a browser that sends one sends a host
    // with it.
    if origin.is_empty() {
        return Site::Same;
    }
    if origin.eq_ignore_ascii_case(OPAQUE) {
        return Site::Cross;
    }
    match netloc(origin) {
        Some(netloc) if allowed.allows(netloc) => Site::Same,
        _ => Site::Cross,
    }
}

/// The `netloc` of a URL, as `urllib.parse.urlsplit` reports it.
///
/// Everything after the `//` and before the first `/`, `?` or `#`. `None` is
/// the two cases the daemon refuses without consulting the allow-list at all:
/// a URL with no `//`, which `urlsplit` gives an empty netloc and which
/// therefore names no host however hostlike its text; and unbalanced IPv6
/// brackets, which is where `urlsplit` raises `ValueError` and `_origin_ok`
/// answers False from its `except`.
///
/// `urlsplit` has one other way to raise, and this does not reproduce it: a
/// non-ASCII netloc that NFKC-normalises to contain `/?#@:` is refused there
/// and allowed here. It is unreachable without a non-ASCII entry in
/// `AYEAYE_ALLOWED_HOSTS`, because an ASCII entry can never equal a netloc
/// that trips the check, and carrying a Unicode normalisation table into the
/// pure core to close it would cost more than the case is worth.
fn netloc(origin: &str) -> Option<&str> {
    let after_slashes = strip_scheme(origin).strip_prefix("//")?;
    let end = after_slashes
        .find(['/', '?', '#'])
        .unwrap_or(after_slashes.len());
    let netloc = &after_slashes[..end];
    if netloc.contains('[') != netloc.contains(']') {
        return None;
    }
    Some(netloc)
}

/// A URL with its `scheme:` removed, if it carries one.
///
/// A scheme is a letter followed by letters, digits, `+`, `-` and `.` — RFC
/// 3986's production, and the one `urlsplit` enforces. Anything else is not a
/// scheme, so the text before the colon is part of what follows: `1https://x`
/// has no scheme and, having no leading `//` either, no netloc.
fn strip_scheme(url: &str) -> &str {
    let Some((scheme, rest)) = url.split_once(':') else {
        return url;
    };
    let mut characters = scheme.chars();
    let looks_like_a_scheme = characters
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic())
        && characters.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
    if looks_like_a_scheme { rest } else { url }
}

#[cfg(test)]
mod tests {
    use super::{Origin, Site, gate};
    use crate::http::hosts::AllowedHosts;

    fn allowed() -> AllowedHosts {
        AllowedHosts::new("127.0.0.1", 8912, "phone.local")
    }

    // AYEAYE-69 — a browser labels every request it makes, and `cross-site` is
    // the label on the one that came from somebody else's page. It is the only
    // value that is a refusal: `same-site` is a sibling subdomain and `none` is
    // a typed URL, and both still have to face the `Origin` clause below.
    #[test]
    fn a_cross_site_label_is_the_one_refusal() {
        assert_eq!(
            gate("POST", Some("cross-site"), Origin::Absent, &allowed()),
            Site::Cross
        );
        for label in ["same-origin", "same-site", "none"] {
            assert_eq!(
                gate("POST", Some(label), Origin::Absent, &allowed()),
                Site::Same,
                "{label} is not a cross-site request"
            );
        }
    }

    // AYEAYE-69 — the daemon reads this header as `.strip().lower()`, and this
    // is the clause where dropping either half is invisible: the value arrives
    // as raw header bytes, browsers send it lowercase today, and a comparison
    // that only handles what they send now would refuse nothing the day one
    // sends `Cross-Site`.
    #[test]
    fn the_label_is_read_the_way_the_daemon_reads_it() {
        for label in ["Cross-Site", "CROSS-SITE", " cross-site ", "\tcross-site"] {
            assert_eq!(
                gate("POST", Some(label), Origin::Absent, &allowed()),
                Site::Cross,
                "{label:?} is the cross-site label, however it is spelled"
            );
        }
        // And the trim does not turn a different label into this one.
        assert_eq!(
            gate("POST", Some(" cross-site-ish "), Origin::Absent, &allowed()),
            Site::Same
        );
    }

    // AYEAYE-69 — `null` is the origin of a sandboxed iframe, a `file://` page
    // and a redirected cross-origin POST. It names no host, so no allow-list
    // can admit it, and the daemon refuses it by name before it parses.
    //
    // The named clause is belt and braces, here and in the daemon: `null` has
    // no `//`, so the netloc parse below would refuse it anyway. Deleting
    // either the clause or this test would leave the suite green, which is
    // worth saying out loud rather than leaving for the next reader to
    // rediscover — the clause stays because `_origin_ok` has it and this is a
    // port, and because it says in one line what the parse says in ten.
    #[test]
    fn an_opaque_origin_is_refused() {
        for origin in ["null", "NULL", " null "] {
            assert_eq!(
                gate("POST", None, Origin::Value(origin), &allowed()),
                Site::Cross,
                "{origin:?} names no host this daemon answers to"
            );
        }
    }

    // AYEAYE-69 — a request that carries no `Origin` at all is allowed. That
    // is `curl`, the phone's shortcut, anything that is not a browser: they
    // send no `Origin` and they still have to present the token. Refusing them
    // would lock out every non-browser client to stop an attack no browser can
    // mount, since a browser always labels what it sends.
    #[test]
    fn a_request_that_names_no_origin_is_allowed() {
        assert_eq!(gate("POST", None, Origin::Absent, &allowed()), Site::Same);
        // Present but empty is the same thing: the daemon reads a blank header
        // as no origin, and a browser that sends one sends a host with it.
        assert_eq!(
            gate("POST", None, Origin::Value(""), &allowed()),
            Site::Same
        );
        assert_eq!(
            gate("POST", None, Origin::Value("   "), &allowed()),
            Site::Same
        );
        // But it buys nothing on a request the browser already labelled.
        assert_eq!(
            gate("POST", Some("cross-site"), Origin::Absent, &allowed()),
            Site::Cross
        );
    }

    // AYEAYE-69 — the ticket's decision: an `Origin` is allowed exactly when
    // its netloc is a `Host` we would have answered to, judged by the same
    // `AllowedHosts` the Host gate consults. So the allow-list is written down
    // once, and widening it widens both gates together.
    #[test]
    fn an_origin_is_allowed_when_its_netloc_is_a_host_we_answer_to() {
        for origin in [
            "https://phone.local",
            // A bare allowlisted name covers it on any port, as it does for Host.
            "https://phone.local:8443",
            // The scheme is lowercased by every browser; the host is not always.
            "HTTPS://Phone.Local",
            // Browsers send no path, but a trailing slash must not become part
            // of the name if one ever arrives.
            "https://phone.local/",
            "http://127.0.0.1:8912",
            "http://localhost:8912",
        ] {
            assert_eq!(
                gate("POST", None, Origin::Value(origin), &allowed()),
                Site::Same,
                "{origin} is this daemon's own origin"
            );
        }
        for origin in [
            "https://evil.example",
            "https://phone.local.evil.example",
            // Userinfo is part of the netloc and is not stripped: the name
            // after the `@` is the host, and it is not ours.
            "https://phone.local:8443@evil.example",
        ] {
            assert_eq!(
                gate("POST", None, Origin::Value(origin), &allowed()),
                Site::Cross,
                "{origin} is somebody else's origin"
            );
        }
    }

    // AYEAYE-69 — an `Origin` that is not a URL has no netloc, and a name with
    // no netloc is not a host we answer to however familiar it looks. This is
    // the clause a hand-rolled parser gets wrong in the dangerous direction:
    // reading the bare text as a hostname would admit `phone.local` from
    // anything that can type a header, where `urlsplit` gives it no netloc at
    // all. The scheme-relative form is the one shape that does have one.
    #[test]
    fn an_origin_that_is_not_a_url_names_no_host() {
        for origin in [
            // No `//`, so no netloc — however much it looks like our own host.
            "phone.local",
            "https:phone.local",
            "phone.local:8912",
            // A scheme has to start with a letter, or there is no scheme and
            // no netloc after it either.
            "1https://phone.local",
            "://phone.local",
            // One slash is a path, and three make the first segment empty.
            "http:/phone.local",
            "http:///phone.local",
            "http://",
        ] {
            assert_eq!(
                gate("POST", None, Origin::Value(origin), &allowed()),
                Site::Cross,
                "{origin} has no netloc, so it names no host we answer to"
            );
        }
        // Scheme-relative is a real netloc, which is what `urlsplit` says and
        // what the daemon therefore admits.
        assert_eq!(
            gate("POST", None, Origin::Value("//phone.local"), &allowed()),
            Site::Same
        );
    }

    // AYEAYE-69 — an `Origin` the server could not read is refused, and this
    // is the one place where getting it wrong is a bypass rather than a
    // curiosity: "absent" allows, so an unreadable header that collapsed into
    // absent would hand every write to anything that could put one byte of
    // rubbish in the header it is judged by. The daemon has no such hole — it
    // reads headers as latin-1, so those bytes become a netloc no ASCII
    // allow-list holds — and neither does this.
    #[test]
    fn an_origin_the_server_could_not_read_is_refused_rather_than_ignored() {
        assert_eq!(
            gate("POST", None, Origin::Unreadable, &allowed()),
            Site::Cross
        );
        // Not by being confused with a foreign origin: absent still allows, so
        // the two states really are being told apart.
        assert_eq!(gate("POST", None, Origin::Absent, &allowed()), Site::Same);
        // And a read is still a read: nothing about unreadable bytes makes
        // fetching an icon a write.
        assert_eq!(
            gate("GET", None, Origin::Unreadable, &allowed()),
            Site::Same
        );
    }

    // AYEAYE-69 — the method is half the answer, as it is for `route::gate`.
    // The daemon calls `_origin_ok` from `do_POST` and never from `do_GET`: a
    // read changes nothing, and a page linking to this one is allowed to. HEAD
    // rides with GET for the same reason it does in `route::gate` — it is the
    // read a link preview does, and it carries a cross-site label while doing
    // it.
    #[test]
    fn a_read_is_exempt_and_everything_else_is_not() {
        for method in ["GET", "HEAD"] {
            assert_eq!(
                gate(
                    method,
                    Some("cross-site"),
                    Origin::Value("https://evil.example"),
                    &allowed()
                ),
                Site::Same,
                "{method} changes nothing, so another site may ask for it"
            );
        }
        for method in ["POST", "PUT", "PATCH", "DELETE", "OPTIONS", "get"] {
            assert_eq!(
                gate(
                    method,
                    Some("cross-site"),
                    Origin::Value("https://evil.example"),
                    &allowed()
                ),
                Site::Cross,
                "{method} could write, so a cross-site one is refused"
            );
        }
    }

    // AYEAYE-69 — `urlsplit` raises `ValueError` on a netloc whose IPv6
    // brackets do not match, and the daemon's `except ValueError` turns that
    // into a refusal. It is only observable through an allow-list that holds
    // the same malformed name — a plausible typo in `AYEAYE_ALLOWED_HOSTS` —
    // and it must not become the one entry that admits a stranger.
    #[test]
    fn an_unbalanced_ipv6_bracket_is_refused_even_if_the_allow_list_holds_it() {
        let typo = AllowedHosts::new("127.0.0.1", 8912, "[::1, ::1]");
        assert_eq!(
            gate("POST", None, Origin::Value("http://[::1"), &typo),
            Site::Cross,
            "an unbalanced bracket is where urlsplit raises"
        );
        assert_eq!(
            gate("POST", None, Origin::Value("http://::1]"), &typo),
            Site::Cross
        );
        // The balanced name in the same list is still admitted, or the test
        // above would pass on a gate that refused everything.
        let ok = AllowedHosts::new("127.0.0.1", 8912, "[::1]");
        assert_eq!(
            gate("POST", None, Origin::Value("http://[::1]"), &ok),
            Site::Same
        );
        assert_eq!(
            gate("POST", None, Origin::Value("http://[::1]:8912"), &ok),
            Site::Same
        );
    }
}
