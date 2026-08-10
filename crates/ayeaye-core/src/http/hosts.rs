//! Which `Host:` values this daemon will answer to.
//!
//! This is the DNS-rebinding gate. A page on an attacker's origin can resolve
//! their hostname to this address, and the browser will still send *their*
//! `Host` header — which is not in this set, so the request never reaches a
//! route.

use std::collections::BTreeSet;

/// The `Host` header values a request may carry.
///
/// The defaults cover direct access to the bind address. Anything fronting the
/// daemon — `tailscale serve`, a reverse proxy — presents its own `Host` and
/// has to be named in the override list.
#[derive(Debug, Clone)]
pub struct AllowedHosts {
    hosts: BTreeSet<String>,
}

impl AllowedHosts {
    /// Build the set from the bind address, the port, and an override list.
    ///
    /// `extra` is the comma-separated `AYEAYE_ALLOWED_HOSTS` value verbatim:
    /// entries are trimmed, lowercased, and stripped of a trailing slash, so
    /// spaces after the commas and a stray slash on the end do not silently
    /// lock somebody out. Note that these are `Host` values, not origins: a
    /// pasted `https://phone.local/` normalises to `https://phone.local`,
    /// which no `Host` header will ever equal. The daemon has the same gap
    /// today; it is stated here rather than fixed, because widening it is a
    /// change to what the allow-list admits.
    pub fn new(bind: &str, port: u16, extra: &str) -> Self {
        let mut hosts = BTreeSet::new();
        for host in [bind, "localhost", "127.0.0.1"] {
            let host = host.to_lowercase();
            hosts.insert(format!("{host}:{port}"));
            hosts.insert(host);
        }
        for entry in extra.split(',') {
            let entry = entry.trim().to_lowercase();
            let entry = entry.trim_end_matches('/');
            if !entry.is_empty() {
                hosts.insert(entry.to_string());
            }
        }
        Self { hosts }
    }

    /// Whether a `Host` header value is one we answer to.
    ///
    /// An exact match first, then the same value with a trailing `:<digits>`
    /// removed — so allowlisting a bare hostname covers it on any port. The
    /// port is only stripped when there was one, which is what leaves an
    /// IPv6 literal like `[::1]` with its brackets rather than eating into it.
    pub fn allows(&self, host: &str) -> bool {
        let host = host.trim().to_lowercase();
        if self.hosts.contains(&host) {
            return true;
        }
        match without_port(&host) {
            Some(bare) => self.hosts.contains(bare),
            None => false,
        }
    }
}

/// The host with a trailing `:<digits>` removed, or `None` if it carried no
/// port at all.
///
/// Returning `None` rather than the input is what keeps the fallback from
/// being a second exact match: only a value that really had a port gets a
/// second look.
fn without_port(host: &str) -> Option<&str> {
    let (bare, port) = host.rsplit_once(':')?;
    if port.is_empty() || !port.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some(bare)
}

#[cfg(test)]
mod tests {
    use super::AllowedHosts;

    // AYEAYE-42 — the defaults cover direct access to the bind address, bare
    // and with the port, exactly as the daemon's `_allowed_hosts` does today.
    #[test]
    fn the_defaults_cover_the_bind_address_localhost_and_loopback() {
        let allowed = AllowedHosts::new("127.0.0.1", 8912, "");
        for host in ["127.0.0.1", "127.0.0.1:8912", "localhost", "localhost:8912"] {
            assert!(allowed.allows(host), "{host} should be allowed");
        }
        assert!(!allowed.allows("evil.example"));
    }

    // AYEAYE-42 — allowlisting a bare hostname has to cover it on whatever
    // port it is reached on, which is the fallback the daemon's `_in_allowlist`
    // does with a regex today.
    #[test]
    fn a_bare_allowlisted_host_is_matched_on_any_port() {
        let allowed = AllowedHosts::new("127.0.0.1", 8912, "ayeaye.tail-scale.ts.net");
        assert!(allowed.allows("ayeaye.tail-scale.ts.net"));
        assert!(allowed.allows("ayeaye.tail-scale.ts.net:443"));
        assert!(!allowed.allows("evil.example:443"));
    }

    // AYEAYE-42 — stripping "the port" must not eat into an IPv6 literal: the
    // brackets are part of the name, and only a trailing `:<digits>` is a port.
    #[test]
    fn an_ipv6_literal_keeps_its_brackets() {
        let allowed = AllowedHosts::new("127.0.0.1", 8912, "[::1]");
        assert!(allowed.allows("[::1]"));
        assert!(allowed.allows("[::1]:8912"));
        // `::1` without brackets is a different name and was never listed; if
        // the port strip had chewed through the colons it would match here.
        assert!(!allowed.allows("::1"));
    }

    // AYEAYE-42 — the override list is a human-typed environment variable, so
    // spaces after the commas, a pasted trailing slash, and shouted case all
    // have to name the same host.
    #[test]
    fn the_override_list_is_normalised_before_it_is_believed() {
        let allowed = AllowedHosts::new("127.0.0.1", 8912, " Phone.Local/ , , box.lan ");
        assert!(allowed.allows("phone.local"));
        assert!(allowed.allows("box.lan"));
        // An empty entry between the commas must not admit an empty Host.
        assert!(!allowed.allows(""));
    }
}
