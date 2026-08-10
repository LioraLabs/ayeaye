//! Whether there is a way out to the internet, read and never dialled.
//!
//! Setup asks this before it offers to download anything, and a program that
//! quietly contacts a server while working out whether it can contact a server
//! has already done the thing it was checking permission for. A default route is
//! the strongest claim that can honestly be made without sending a packet, and
//! it is a claim about routing rather than about the internet — which is why the
//! sentence a person reads never says "you are online", only that there is
//! nothing obviously in the way.

use super::Probes;

/// What can be said about this machine's way out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Network {
    /// There is a default route that does not point at loopback.
    Online,
    /// The routing tables were read and there is no way out in them.
    Offline,
    /// Nothing could be read. Not the same as offline, and never rendered as it.
    Unknown,
}

impl Network {
    /// The lower-case word a caller matches on.
    pub fn as_str(self) -> &'static str {
        match self {
            Network::Online => "online",
            Network::Offline => "offline",
            Network::Unknown => "unknown",
        }
    }
}

/// The all-zero IPv6 destination, as `/proc/net/ipv6_route` spells it.
const V6_ANY: &str = "00000000000000000000000000000000";

/// Read the routing tables and say what they allow.
pub fn classify(probes: &Probes<'_>) -> Network {
    let mut seen = false;

    if let Some(text) = probes.route {
        seen = true;
        for line in text.lines() {
            let mut fields = line.split_whitespace();
            let (Some(interface), Some(destination)) = (fields.next(), fields.next()) else {
                continue;
            };
            if interface == "Iface" {
                continue;
            }
            if destination == "00000000" && interface != "lo" {
                return Network::Online;
            }
        }
    }

    // A machine with only an IPv6 address has no entry in the table above at
    // all, and calling it offline would be a positive claim made out of having
    // looked in one place.
    //
    // The device is why this is not two lines shorter. A container started with
    // no network at all still has two ::/0 entries pointing at loopback — the
    // unreachable routes that make a send fail immediately rather than hang —
    // and reading those as a way out would suppress the one warning somebody
    // about to download a model needs.
    if let Some(text) = probes.route6 {
        seen = true;
        for line in text.lines() {
            let fields: Vec<&str> = line.split_whitespace().collect();
            let (Some(destination), Some(prefix), Some(interface)) =
                (fields.first(), fields.get(1), fields.last())
            else {
                continue;
            };
            if *interface == "lo" {
                continue;
            }
            if *destination == V6_ANY && *prefix == "00" {
                return Network::Online;
            }
        }
    }

    if seen {
        return Network::Offline;
    }

    // No /proc to read: a Mac, where `route -n get default` is the same question
    // asked of the kernel directly. Still a read — it resolves a route, it does
    // not send anything down one.
    match probes.route_default_exists {
        Some(true) => Network::Online,
        Some(false) => Network::Offline,
        None => Network::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::{Network, classify};
    use crate::machine::Probes;

    macro_rules! fixture {
        ($path:literal) => {
            include_str!(concat!("../../../../tests/fixtures/", $path))
        };
    }

    // AYEAYE-60
    #[test]
    fn a_default_route_means_there_is_nothing_obviously_in_the_way() {
        assert_eq!(
            classify(&Probes {
                route: Some(fixture!("route/default")),
                ..Probes::default()
            }),
            Network::Online
        );
        assert_eq!(
            classify(&Probes {
                route: Some(fixture!("route/no-default")),
                ..Probes::default()
            }),
            Network::Offline,
            "a routing table with only loopback in it means offline"
        );
        assert_eq!(
            classify(&Probes::default()),
            Network::Unknown,
            "a table that could not be read is not a table that said no"
        );
    }

    // AYEAYE-60 — a machine with only an IPv6 address has no entry in the v4
    // table at all, and calling it offline would be a positive claim made out
    // of having looked in one place.
    #[test]
    fn the_second_table_is_read_before_the_answer_is_no() {
        assert_eq!(
            classify(&Probes {
                route: Some(fixture!("route/no-default")),
                route6: Some(fixture!("route/ipv6-default")),
                ..Probes::default()
            }),
            Network::Online
        );
        assert_eq!(
            classify(&Probes {
                route: Some(fixture!("route/no-default")),
                route6: Some(fixture!("route/ipv6-no-default")),
                ..Probes::default()
            }),
            Network::Offline
        );
    }

    // AYEAYE-60 — docker's --network=none leaves two ::/0 entries pointing at
    // loopback. They are the unreachable routes that make a send fail rather
    // than hang, and reading them as a way out would suppress the one warning
    // somebody about to download a model needs.
    #[test]
    fn a_container_with_no_network_is_offline_despite_its_loopback_routes() {
        assert_eq!(
            classify(&Probes {
                route: Some("Iface\tDestination\tGateway\n"),
                route6: Some(fixture!("route/ipv6-loopback-only")),
                ..Probes::default()
            }),
            Network::Offline
        );
    }

    // AYEAYE-60 — a Mac has no /proc, and `route -n get default` resolves a
    // route rather than sending anything down one.
    #[test]
    fn a_machine_with_no_proc_falls_back_to_asking_the_kernel_for_a_route() {
        assert_eq!(
            classify(&Probes {
                route_default_exists: Some(true),
                ..Probes::default()
            }),
            Network::Online
        );
        assert_eq!(
            classify(&Probes {
                route_default_exists: Some(false),
                ..Probes::default()
            }),
            Network::Offline
        );
        assert_eq!(
            classify(&Probes {
                route: Some(fixture!("route/no-default")),
                route_default_exists: Some(true),
                ..Probes::default()
            }),
            Network::Offline,
            "a table that was read is the answer; the fallback is for having none"
        );
    }
}
