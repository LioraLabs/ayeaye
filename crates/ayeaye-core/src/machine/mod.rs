//! What this computer is: an operating system, a package manager, a service
//! manager, and a way out to the internet.
//!
//! Every module here is a parser and a judgement: raw probe text in, a word out.
//! Nothing runs a command, opens a file or asks the operating system anything —
//! the shell above captures the text and hands it over, which is what makes the
//! whole answer reproducible from the fixtures under `tests/fixtures`.
//!
//! # What is deliberately not here any more
//!
//! Memory, processors, free disk, the graphics card, the container share, and
//! the tier verdict built out of all five. Every one of them existed to answer
//! one question — *which speech model will run on this machine* — and AYEAYE-101
//! took that question away: the models live in a `llama-swap` that may not even
//! be on this computer, and sizing them is that process's business.
//!
//! This is not a reduction in what ayeaye knows, it is a correction. A tier
//! verdict computed here would have described the wrong machine, and a card
//! detected here is one nothing in this binary can use.
//!
//! It is a port of `lib/steps/20-hardware.sh` and `lib/platform.sh`, minus the
//! hardware half, and is held to the same corpus those two are tested against.

pub mod network;
pub mod packages;
pub mod platform;

// The rule for this list: the *types* a consumer names, and nothing else. The
// parsers keep their module paths — `network::classify` says what it reads only
// because `network` is in front of it. A flat list of verbs here would have to
// rename half of them to stay unambiguous.
pub use network::Network;
pub use packages::Privilege;
pub use platform::{Family, Homebrew, Os, PackageManager, Packaging, Platform, ServiceManager};

/// The captured probe output the shell suite reads, reaching a pure crate the
/// only way it may: at compile time.
///
/// Defined once, here, so every module's corpus is the same corpus. The relative
/// path resolves against the file the macro is expanded in, and every module
/// that uses it sits at this same depth — one directory below `src/`.
#[cfg(test)]
macro_rules! fixture {
    ($path:literal) => {
        include_str!(concat!("../../../../tests/fixtures/", $path))
    };
}
#[cfg(test)]
pub(crate) use fixture;

/// Everything the shell captured about this machine, exactly as it was printed.
///
/// This is the seam. Every field is text some command or file produced, and
/// `None` means the probe found nothing to read — a command that is not
/// installed, a file that is not there, an exit status that said the answer was
/// not to be trusted. `None` is never the same as an empty answer, which is why
/// none of these is a bare `&str`.
///
/// [`Probes::default`] is a machine that would say nothing at all about itself,
/// which is both the honest starting point and what makes a test name only the
/// one probe it is about.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Probes<'a> {
    /// The first readable `/etc/os-release`-shaped file.
    pub os_release: Option<&'a str>,
    /// `uname -s`.
    pub uname_s: Option<&'a str>,
    /// `uname -m`.
    pub uname_m: Option<&'a str>,
    /// `sw_vers`, with no arguments, so one exec answers everything.
    pub sw_vers: Option<&'a str>,
    /// The commands `command -v` found on `PATH`. Only membership matters, and
    /// only for names this crate asks about.
    pub available_commands: &'a [&'a str],
    /// Whether `systemctl --user show-environment` succeeded. A container has
    /// the binary and no user session.
    pub user_bus_responds: bool,
    /// What `command -v brew` printed.
    pub brew_on_path: Option<&'a str>,
    /// The first standard prefix whose `bin/brew` is executable.
    pub brew_in_prefix: Option<&'a str>,
    /// `/proc/net/route`.
    pub route: Option<&'a str>,
    /// `/proc/net/ipv6_route`.
    pub route6: Option<&'a str>,
    /// Whether `route -n get default` resolved a route, on a machine with no
    /// `/proc` to read. `None` when there was no `route` command to ask.
    pub route_default_exists: Option<bool>,
}

/// Everything this crate can say about the machine it was handed.
///
/// One value, read once. The decision behind it is the ticket's: *one capability
/// is described in exactly one place, and everything else reads that answer*. A
/// previous run described voice three times in two framings, and the least
/// accurate description was the one nobody read — so the tier, the cause, the
/// reason, the acceleration, whether it is usable and why not, and the card's
/// own name all come off this value and are computed nowhere else.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Machine {
    /// What this machine is.
    pub platform: Platform,
    /// What may be installed here, and what may only be asked.
    pub packaging: Packaging,
    /// Where a user service can be installed.
    pub services: ServiceManager,
    /// Homebrew, if it is installed, and how to invoke it.
    pub homebrew: Option<Homebrew>,
    /// Whether there is a way out to the internet.
    pub network: Network,
}

impl Machine {
    /// Read the whole machine out of what its probes printed.
    ///
    /// The order is the shell's own: identify the platform, then everything
    /// that hangs off knowing what it is.
    pub fn read(probes: &Probes<'_>) -> Machine {
        let platform = platform::identify(
            probes.os_release,
            probes.uname_s,
            probes.uname_m,
            probes.sw_vers,
        );
        let homebrew = platform::homebrew(probes.brew_on_path, probes.brew_in_prefix);
        let packaging = platform::packaging(
            platform.family,
            platform.immutable,
            probes.available_commands,
            homebrew.as_ref(),
        );
        let services = platform::service_manager(
            platform.os,
            probes.available_commands,
            probes.user_bus_responds,
        );

        Machine {
            platform,
            packaging,
            services,
            homebrew,
            network: network::classify(probes),
        }
    }

    /// Whether this layer recognised the machine well enough to act on its own.
    pub fn is_known(&self) -> bool {
        platform::is_known(&self.platform, &self.packaging)
    }

    /// The whole verdict as one line, for one line of output.
    pub fn summary(&self) -> String {
        platform::summary(&self.platform, &self.packaging, self.services)
    }
}

#[cfg(test)]
mod tests {
    use super::{Machine, PackageManager, Probes};

    // AYEAYE-60, narrowed by AYEAYE-101 — one machine, every question this
    // module still answers. The tier, the card, the memory, the free disk and
    // the container share all left with the models: they existed to size a
    // local speech model, and there is no local speech model.
    #[test]
    fn one_machine_answers_every_question_about_itself() {
        let machine = Machine::read(&Probes {
            os_release: Some(fixture!("os-release/ubuntu-24.04")),
            uname_s: Some("Linux"),
            uname_m: Some("x86_64"),
            available_commands: &["apt-get", "systemctl"],
            user_bus_responds: true,
            route: Some(fixture!("route/default")),
            ..Probes::default()
        });
        assert_eq!(machine.packaging.manager, PackageManager::AptGet);
        assert!(machine.is_known());
        assert_eq!(
            machine.summary(),
            "Ubuntu 24.04.1 LTS (debian) x86_64, packages: apt-get, services: systemd"
        );
    }

    // AYEAYE-60 — nothing readable at all is still an answer, and it must not
    // be a confident one.
    #[test]
    fn a_machine_that_says_nothing_about_itself_is_not_claimed_to_be_known() {
        let unknown = Machine::read(&Probes::default());
        assert!(!unknown.is_known());
    }
}
