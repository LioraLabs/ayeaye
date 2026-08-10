//! Who the machines are, and how a pane id says which one it lives on.

/// The character between a host and a pane in a qualified id.
pub const SEPARATOR: char = '/';

/// A machine's name, in a form a qualified pane id can carry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostName(String);

impl HostName {
    /// Read a name, or say why it is not one.
    ///
    /// Trimmed, because an env file writes a trailing newline and means nothing
    /// by it. Not otherwise normalised: the name is what the panel shows, and
    /// lowercasing somebody's `MacBook-Pro` for them is a change to what they
    /// see. Case is folded where names are *compared* instead — see
    /// [`Registry::new`].
    pub fn new(name: &str) -> Result<HostName, BadName> {
        let name = name.trim();
        if name.is_empty() {
            return Err(BadName::Empty);
        }
        if name.contains(SEPARATOR) {
            return Err(BadName::HasSeparator(name.to_string()));
        }
        if name.chars().any(char::is_control) {
            return Err(BadName::HasControl(name.to_string()));
        }
        Ok(HostName(name.to_string()))
    }

    /// The name as it should be shown.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Why a name cannot be a host name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BadName {
    /// Nothing, or nothing but whitespace.
    Empty,
    /// It carries the separator, so a qualified id built from it could not be
    /// split back apart.
    HasSeparator(String),
    /// It carries a control character, which is not part of any machine's name
    /// and is part of several ways to make one line look like two.
    HasControl(String),
}

/// A pane, and the machine it is on: `desktop/%12`.
///
/// **Local panes are qualified too.** The machine ayeaye runs on is a peer that
/// happens to need no transport, and naming it means one code path instead of
/// two — the alternative, a bare `%12` meaning local, puts an `if` in front of
/// every route for the rest of the project's life.
///
/// The pane half is carried verbatim and is only checked for being a thing that
/// can live inside a qualified id. Whether it is a *usable tmux target* is a
/// different question, and it belongs to whoever first sends one to tmux rather
/// than to the type that federates it — with one warning worth inheriting: the
/// daemon's defence there is **membership, not syntax**. It checks the id
/// against the pane list it just read before using it as a target, and a
/// pattern over the pane half is not a substitute for that.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneId {
    host: HostName,
    pane: String,
}

impl PaneId {
    /// Qualify a pane id tmux gave us with the machine it came from.
    pub fn new(host: HostName, pane: &str) -> Result<PaneId, BadPaneId> {
        let pane = pane.trim();
        if pane.is_empty() {
            return Err(BadPaneId::NoPane);
        }
        if pane.chars().any(char::is_control) {
            return Err(BadPaneId::HasControl(pane.to_string()));
        }
        Ok(PaneId {
            host,
            pane: pane.to_string(),
        })
    }

    /// Read one back, splitting on the **first** separator, which is what the
    /// multi-host design specifies.
    ///
    /// An id with no separator is refused rather than assumed to be local:
    /// "bare means here" is the conditional this type exists to delete.
    pub fn parse(text: &str) -> Result<PaneId, BadPaneId> {
        let (host, pane) = text.trim().split_once(SEPARATOR).ok_or(BadPaneId::NoPane)?;
        let host = HostName::new(host).map_err(BadPaneId::BadHost)?;
        PaneId::new(host, pane)
    }

    /// The machine.
    pub fn host(&self) -> &HostName {
        &self.host
    }

    /// The pane, as tmux spells it.
    pub fn pane(&self) -> &str {
        &self.pane
    }

    /// The whole id, as it goes over the wire.
    pub fn qualified(&self) -> String {
        format!("{}{}{}", self.host.as_str(), SEPARATOR, self.pane)
    }
}

/// Why a qualified pane id is not one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BadPaneId {
    /// No separator, or nothing after it. A bare pane id names no machine.
    NoPane,
    /// The host half is not a host name.
    BadHost(BadName),
    /// The pane half carries a control character.
    HasControl(String),
}

/// How a peer is reached.
///
/// The local machine has none — it is a peer with a **null transport**, which
/// is what lets one code path serve the machine ayeaye runs on and the machines
/// it does not. Nothing in this crate dials anything; a transport is a fact
/// about a peer that the shell above acts on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Transport {
    /// An `ssh` target, verbatim — whatever `ssh` itself understands. Building
    /// the channel is the multi-host milestone's work; the registry has to be
    /// able to *hold* one now, or that work arrives as a change to every caller
    /// instead of a change to this.
    Ssh(String),
}

/// A machine in this deployment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Peer {
    name: HostName,
    transport: Option<Transport>,
}

impl Peer {
    /// This machine: a peer with no transport at all.
    pub fn here(name: HostName) -> Peer {
        Peer {
            name,
            transport: None,
        }
    }

    /// Another machine, and how to reach it.
    pub fn over(name: HostName, transport: Transport) -> Peer {
        Peer {
            name,
            transport: Some(transport),
        }
    }

    /// What this machine is called.
    pub fn name(&self) -> &HostName {
        &self.name
    }

    /// How it is reached, or `None` for the machine this process is on.
    pub fn transport(&self) -> Option<&Transport> {
        self.transport.as_ref()
    }

    /// Whether this is the machine this process is on.
    pub fn is_here(&self) -> bool {
        self.transport.is_none()
    }
}

/// Every machine this deployment can see, the local one included.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Registry {
    peers: Vec<Peer>,
    /// Which of them is this machine. Held rather than searched for, because
    /// [`Registry::new`] has already proved there is exactly one and `here()`
    /// should not be able to fail afterwards.
    here: usize,
}

impl Registry {
    /// Read a peer list, or refuse it.
    ///
    /// The whole list is the argument rather than "the local one, plus some":
    /// the failure this exists to prevent is two machines being served as one,
    /// and a constructor that can only be handed one peer could never be shown
    /// that failure. The same reasoning the constitution's `STRATA` table is
    /// passed in by its rule.
    ///
    /// Names are compared with case folded, because `gpu-box` and `GPU-Box` are
    /// one machine to DNS and would be two rows in a panel. They are *stored*
    /// as written: the name is what the panel shows. The fold is ASCII-only,
    /// which is what DNS means by case — so two names differing only in the
    /// case of a non-ASCII letter are two peers here. That is a gap between the
    /// justification and the rule, and it is left open rather than papered
    /// over, because these are display names and case-folding somebody's
    /// alphabet for them is a bigger decision than this.
    pub fn new(peers: Vec<Peer>) -> Result<Registry, BadRegistry> {
        let mut here: Option<usize> = None;
        for (index, peer) in peers.iter().enumerate() {
            let already_named = peers[..index]
                .iter()
                .any(|seen| seen.name.as_str().eq_ignore_ascii_case(peer.name.as_str()));
            if already_named {
                return Err(BadRegistry::Collision(peer.name.as_str().to_string()));
            }
            if peer.is_here() {
                if let Some(first) = here {
                    return Err(BadRegistry::TwoLocalPeers(
                        peers[first].name.as_str().to_string(),
                        peer.name.as_str().to_string(),
                    ));
                }
                here = Some(index);
            }
        }
        match here {
            Some(here) => Ok(Registry { peers, here }),
            None => Err(BadRegistry::NoLocalPeer),
        }
    }

    /// The machine this process is on.
    pub fn here(&self) -> &Peer {
        &self.peers[self.here]
    }

    /// Every machine, in the order the list gave them.
    pub fn peers(&self) -> &[Peer] {
        &self.peers
    }

    /// The peer with this name, if there is one.
    pub fn get(&self, name: &str) -> Option<&Peer> {
        self.peers
            .iter()
            .find(|peer| peer.name.as_str().eq_ignore_ascii_case(name.trim()))
    }

    /// Split a qualified id into the machine it names and the id itself.
    ///
    /// This is the "split on the way in" half of qualified ids: a route reads
    /// the destination out of the id the client handed back, rather than out of
    /// a second parameter that could disagree with it.
    pub fn route(&self, qualified: &str) -> Result<(&Peer, PaneId), Unroutable> {
        let id = PaneId::parse(qualified).map_err(Unroutable::NotAnId)?;
        let peer = self
            .get(id.host().as_str())
            .ok_or_else(|| Unroutable::NoSuchHost(id.host().as_str().to_string()))?;
        Ok((peer, id))
    }
}

/// Why a peer list is not a deployment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BadRegistry {
    /// No peer without a transport. Something has to be this machine.
    NoLocalPeer,
    /// More than one peer without a transport.
    TwoLocalPeers(String, String),
    /// Two peers under one name, which is the failure that would serve two
    /// machines as one and never say so.
    Collision(String),
}

/// Why a qualified id does not name a pane on a machine we know.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unroutable {
    /// It is not a qualified pane id at all.
    NotAnId(BadPaneId),
    /// It names a machine that is not in the registry.
    NoSuchHost(String),
}

#[cfg(test)]
mod tests {
    use super::{
        BadName, BadPaneId, BadRegistry, HostName, PaneId, Peer, Registry, Transport, Unroutable,
    };

    fn host(name: &str) -> HostName {
        HostName::new(name).expect("a test host name should be a host name")
    }

    // AYEAYE-43 — a host name carrying the separator would build a qualified id
    // that splits back into a different host and a different pane, so it is
    // refused rather than escaped.
    #[test]
    fn a_name_carrying_the_separator_is_refused() {
        assert_eq!(
            HostName::new("gpu/box"),
            Err(BadName::HasSeparator("gpu/box".to_string()))
        );
        assert_eq!(HostName::new("").unwrap_err(), BadName::Empty);
        assert_eq!(HostName::new("   ").unwrap_err(), BadName::Empty);
        // A name is trimmed rather than refused for its edges: an env file
        // writes a trailing newline and means nothing by it.
        assert_eq!(HostName::new("  gpu-box\n").unwrap().as_str(), "gpu-box");
        // And a name with a space in it is a name. `AYEAYE_NAME="Alex's Mac"`
        // is a display name, not a mistake.
        assert_eq!(HostName::new("Alex's Mac").unwrap().as_str(), "Alex's Mac");
    }

    // AYEAYE-43 — a name is written into a JSON body, a log line and a
    // qualified id. An embedded newline or escape byte is not a machine name;
    // it is something that ends up meaning a second line somewhere.
    #[test]
    fn a_name_carrying_a_control_character_is_refused() {
        assert_eq!(
            HostName::new("gpu\nbox").unwrap_err(),
            BadName::HasControl("gpu\nbox".to_string())
        );
        assert_eq!(
            HostName::new("gpu\u{1b}[31mbox").unwrap_err(),
            BadName::HasControl("gpu\u{1b}[31mbox".to_string())
        );
        // Inner whitespace that is not a control character is still fine; only
        // the edges are trimmed.
        assert_eq!(HostName::new(" gpu box ").unwrap().as_str(), "gpu box");
    }

    // AYEAYE-43 — every pane id is `host/pane`, local ones included, and it
    // reads back as the same two halves. A machine name may carry spaces, so
    // the split is on the separator and nothing else.
    #[test]
    fn a_pane_id_is_qualified_and_reads_back_as_its_two_halves() {
        let id = PaneId::new(host("desktop"), "%12").expect("a pane id");
        assert_eq!(id.qualified(), "desktop/%12");
        assert_eq!(PaneId::parse("desktop/%12"), Ok(id));

        let spaced = PaneId::new(host("Alex's Mac"), "%3").expect("a pane id");
        assert_eq!(spaced.qualified(), "Alex's Mac/%3");
        assert_eq!(PaneId::parse("Alex's Mac/%3"), Ok(spaced));
    }

    // AYEAYE-43 — the split is on the *first* separator, which is what the
    // multi-host design specifies: the host is everything before it and the
    // rest is carried whole. Splitting on the last would read the host of `a/b/%12` as `a/b`,
    // which is not a name any peer can be registered under.
    #[test]
    fn a_qualified_id_splits_on_the_first_separator() {
        let id = PaneId::parse("a/b/%12").expect("the first separator ends the host");
        assert_eq!(id.host().as_str(), "a");
        assert_eq!(id.pane(), "b/%12");
        assert_eq!(id.qualified(), "a/b/%12");
    }

    // AYEAYE-43 — a bare pane id names no machine. Refused rather than assumed
    // local: "bare means here" is the conditional qualifying every id exists to
    // delete, and it would come back the first time a spoke's id was mishandled.
    #[test]
    fn a_bare_pane_id_is_refused_rather_than_assumed_local() {
        assert_eq!(PaneId::parse("%12").unwrap_err(), BadPaneId::NoPane);
        assert_eq!(PaneId::parse("desktop/").unwrap_err(), BadPaneId::NoPane);
        assert_eq!(PaneId::parse("").unwrap_err(), BadPaneId::NoPane);
        assert_eq!(
            PaneId::parse("/%12").unwrap_err(),
            BadPaneId::BadHost(BadName::Empty)
        );
        assert_eq!(
            PaneId::new(host("desktop"), "%1\n2").unwrap_err(),
            BadPaneId::HasControl("%1\n2".to_string())
        );
    }

    // AYEAYE-43 — the local machine is a peer like any other, distinguished
    // only by having no transport to reach it over. That is what makes one code
    // path serve the machine ayeaye runs on and the machines it does not.
    #[test]
    fn the_local_machine_is_a_peer_with_a_null_transport() {
        let registry = Registry::new(vec![
            Peer::here(host("desktop")),
            Peer::over(
                host("gpu-box"),
                Transport::Ssh("alex@gpu-box.lan".to_string()),
            ),
        ])
        .expect("two differently-named peers, one of them here");

        assert_eq!(registry.here().name().as_str(), "desktop");
        assert_eq!(registry.here().transport(), None);
        assert!(registry.here().is_here());

        let gpu = registry
            .get("gpu-box")
            .expect("the peer that was registered");
        assert!(!gpu.is_here());
        assert_eq!(
            gpu.transport(),
            Some(&Transport::Ssh("alex@gpu-box.lan".to_string()))
        );
        assert_eq!(registry.peers().len(), 2);
        assert_eq!(registry.get("laptop"), None);
    }

    // AYEAYE-43 — a deployment that names two machines the same would serve
    // them as one and never say so, which is why the list is refused at startup
    // rather than resolved arbitrarily. DNS does not distinguish case, so
    // neither may this.
    #[test]
    fn a_colliding_peer_list_is_refused() {
        let collides = Registry::new(vec![
            Peer::here(host("desktop")),
            Peer::over(host("DeskTop"), Transport::Ssh("alex@other".to_string())),
        ]);
        assert_eq!(
            collides.unwrap_err(),
            BadRegistry::Collision("DeskTop".to_string())
        );

        // And a list with nobody here, or with two heres, is not a deployment
        // either: `here()` has to be able to answer.
        assert_eq!(
            Registry::new(vec![Peer::over(
                host("gpu-box"),
                Transport::Ssh("alex@gpu-box".to_string())
            )])
            .unwrap_err(),
            BadRegistry::NoLocalPeer
        );
        assert_eq!(
            Registry::new(vec![Peer::here(host("a")), Peer::here(host("b"))]).unwrap_err(),
            BadRegistry::TwoLocalPeers("a".to_string(), "b".to_string())
        );
    }

    // AYEAYE-43 — a qualified id carries its own destination, which is what a
    // route reads instead of trusting a second parameter beside it.
    #[test]
    fn a_qualified_id_routes_to_the_peer_it_names() {
        let registry = Registry::new(vec![
            Peer::here(host("desktop")),
            Peer::over(
                host("gpu-box"),
                Transport::Ssh("alex@gpu-box.lan".to_string()),
            ),
        ])
        .expect("a deployment");

        let (peer, id) = registry.route("gpu-box/%3").expect("a registered machine");
        assert_eq!(peer.name().as_str(), "gpu-box");
        assert_eq!(id.pane(), "%3");
        assert!(registry.route("desktop/%12").expect("here too").0.is_here());

        assert_eq!(
            registry.route("laptop/%1").unwrap_err(),
            Unroutable::NoSuchHost("laptop".to_string())
        );
        assert_eq!(
            registry.route("%12").unwrap_err(),
            Unroutable::NotAnId(BadPaneId::NoPane)
        );

        // The registry refuses `gpu-box` and `GPU-Box` as one machine under two
        // names, so an id shouted back at it has to reach that machine. If the
        // lookup were exact, the collision rule would be refusing lists it then
        // could not route anyway.
        let (shouted, _) = registry
            .route("GPU-Box/%3")
            .expect("one machine, either case");
        assert_eq!(shouted.name().as_str(), "gpu-box");
        assert_eq!(
            registry.get("DESKTOP").map(|peer| peer.is_here()),
            Some(true)
        );
    }
}
