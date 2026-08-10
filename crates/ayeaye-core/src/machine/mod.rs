//! What this computer is, and what it can actually do.
//!
//! Every module here is a parser and a judgement: raw probe text in, a word or a
//! number out. Nothing runs a command, opens a file or asks the operating system
//! anything — the shell above captures the text and hands it over, which is what
//! makes the whole verdict reproducible from the fixtures under `tests/fixtures`.
//!
//! It is a port of `lib/steps/20-hardware.sh` and `lib/platform.sh`, and it is
//! held to the same corpus those two are tested against.

pub mod graphics;
pub mod network;
pub mod platform;
pub mod share;
pub mod size;
mod text;
pub mod tier;

// The rule for this list: the *types* a consumer names, and nothing else. The
// parsers keep their module paths — `graphics::classify` and `network::classify`
// are two different questions with one name, and `size::cores` says what it
// reads only because `size` is in front of it. A flat list of verbs here would
// have to rename half of them to stay unambiguous.
pub use graphics::{Acceleration, Graphics};
pub use network::Network;
pub use platform::{Family, Homebrew, Os, PackageManager, Packaging, Platform, ServiceManager};
pub use share::{Limit, Limits, Share};
pub use tier::{Cause, Tier, Usability, Verdict};

/// The captured probe output the shell suite reads, reaching a pure crate the
/// only way it may: at compile time.
///
/// Defined once, here, so every module's corpus is the same corpus. The relative
/// path resolves against this file, and every module that uses it sits in this
/// same directory.
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
    /// `nproc`.
    pub nproc: Option<&'a str>,
    /// `lscpu`, under `LC_ALL=C` — util-linux is translated.
    pub lscpu: Option<&'a str>,
    /// `/proc/cpuinfo`.
    pub cpuinfo: Option<&'a str>,
    /// `sysctl -n hw.ncpu`.
    pub sysctl_ncpu: Option<&'a str>,
    /// `sysctl -n hw.memsize`, in bytes.
    pub sysctl_memsize: Option<&'a str>,
    /// `system_profiler SPHardwareDataType`.
    pub system_profiler: Option<&'a str>,
    /// `/proc/meminfo`.
    pub meminfo: Option<&'a str>,
    /// `free -m`.
    pub free_m: Option<&'a str>,
    /// `df -Pk` over the nearest existing ancestor of the model directory.
    pub df_pk: Option<&'a str>,
    /// `nvidia-smi --query-gpu=name,memory.total --format=csv,noheader`.
    pub nvidia_smi: Option<&'a str>,
    /// `rocminfo`.
    pub rocminfo: Option<&'a str>,
    /// `sysctl -n machdep.cpu.brand_string`.
    pub sysctl_brand_string: Option<&'a str>,
    /// True when any of `/.dockerenv`, `/run/.containerenv` or
    /// `/run/systemd/container` exists, or the `container` environment variable
    /// is set. Four marks, none reliable alone, and only their disjunction is a
    /// judgement worth making.
    pub container_marker: bool,
    /// `/proc/1/cgroup`.
    pub proc1_cgroup: Option<&'a str>,
    /// `/proc/self/mountinfo`.
    pub mountinfo: Option<&'a str>,
    /// The version-two memory limit, read through [`share::cgroup_path`].
    pub cgroup_memory_max: Option<&'a str>,
    /// The version-one memory limit.
    pub cgroup_memory_limit: Option<&'a str>,
    /// The version-two processor limit: `<quota> <period>`.
    pub cgroup_cpu_max: Option<&'a str>,
    /// The version-one processor quota.
    pub cgroup_cpu_quota: Option<&'a str>,
    /// The version-one processor period.
    pub cgroup_cpu_period: Option<&'a str>,
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
    /// What would do the listening work.
    pub graphics: Graphics,
    /// How much of this computer is yours.
    pub share: Share,
    /// Whether there is a way out to the internet.
    pub network: Network,
    /// Memory in whole megabytes, already brought down to the share.
    pub ram_mb: Option<u64>,
    /// Processors, already brought down to the share.
    pub cores: Option<u64>,
    /// Free space where the models would land, in whole megabytes.
    pub disk_mb: Option<u64>,
    /// What this machine can do, and why it is not more.
    pub verdict: Verdict,
}

impl Machine {
    /// Read the whole machine out of what its probes printed.
    ///
    /// The order is the shell's own: identify the platform, work out the share
    /// before anything is measured, measure, and judge last.
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

        let share = share::read(probes);
        // The container correction, applied to the two figures a container lies
        // about. Free space is not one of them: df inside a container reports
        // the filesystem the container can actually write to.
        let cores = share::clamp(size::cores(probes), share.cores);
        let ram_mb = share::clamp(size::ram_mb(probes), share.ram_mb);
        let disk_mb = size::disk_mb(probes);

        let graphics = graphics::classify(&platform, probes);
        let verdict = tier::judge(ram_mb, cores, disk_mb, &graphics, share.limits);

        Machine {
            platform,
            packaging,
            services,
            homebrew,
            graphics,
            share,
            network: network::classify(probes),
            ram_mb,
            cores,
            disk_mb,
            verdict,
        }
    }

    /// What this machine can do. The one answer model selection consumes.
    pub fn tier(&self) -> Tier {
        self.verdict.tier
    }

    /// Whether this machine reached that tier or better.
    pub fn tier_at_least(&self, tier: Tier) -> bool {
        self.verdict.tier >= tier
    }

    /// What would do the listening work here.
    pub fn acceleration(&self) -> Acceleration {
        self.graphics.acceleration
    }

    /// Whether that acceleration is worth acting on, and why not.
    pub fn usability(&self) -> Usability {
        tier::usability(&self.graphics)
    }

    /// The card's own name, for a sentence that has to name it.
    pub fn gpu_name(&self) -> Option<&str> {
        self.graphics.name.as_deref()
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
    use super::{Acceleration, Cause, Limits, Machine, PackageManager, Probes, Tier, Usability};

    // AYEAYE-60 — the end-to-end answer tests/cases/hardware_probe_test.sh pins
    // for hw_detect: lscpu sees the host's eight processors and meminfo its
    // 64 GB, and the share is one gigabyte and two cores.
    #[test]
    fn a_container_is_judged_on_its_share_and_not_on_the_host() {
        let machine = Machine::read(&Probes {
            os_release: Some(fixture!("os-release/debian-12")),
            uname_s: Some("Linux"),
            uname_m: Some("x86_64"),
            meminfo: Some(fixture!("meminfo/64gb")),
            lscpu: Some(fixture!("lscpu/x86_64-8core")),
            df_pk: Some(fixture!("df/roomy")),
            container_marker: true,
            cgroup_memory_max: Some(fixture!("cgroup/memory-max-1g")),
            cgroup_cpu_max: Some(fixture!("cgroup/cpu-max-two-cores")),
            ..Probes::default()
        });
        assert_eq!(machine.cores, Some(2), "lscpu sees eight; the share is two");
        assert_eq!(
            machine.ram_mb,
            Some(1024),
            "meminfo sees 64 GB; the share is one"
        );
        assert_eq!(
            machine.tier(),
            Tier::TextOnly,
            "a gigabyte of memory is a gigabyte of memory"
        );
        assert_eq!(machine.share.limits, Limits::Known);
        assert_eq!(
            machine.disk_mb,
            Some(510580),
            "free space is not one of the figures a container lies about"
        );
    }

    // AYEAYE-60 — nothing readable at all. Capped by not knowing, not by
    // knowing something bad, and it must say which.
    #[test]
    fn a_machine_that_says_nothing_about_itself_is_capped_for_not_knowing() {
        let machine = Machine::read(&Probes::default());
        assert_eq!(machine.ram_mb, None);
        assert_eq!(machine.cores, None);
        assert_eq!(machine.disk_mb, None);
        assert_eq!(machine.acceleration(), Acceleration::Cpu);
        assert_eq!(machine.tier(), Tier::Lightweight);
        assert_eq!(machine.verdict.cause, Some(Cause::RamUnknown));
        assert!(
            machine
                .verdict
                .reason
                .is_some_and(|reason| reason.contains("could not"))
        );
        assert!(machine.verdict.cause.is_some_and(Cause::is_ignorance));
        assert!(!machine.is_known());
    }

    // AYEAYE-60 — every answer about one capability comes off the one value.
    #[test]
    fn one_machine_answers_every_question_about_itself() {
        let machine = Machine::read(&Probes {
            os_release: Some(fixture!("os-release/ubuntu-24.04")),
            uname_s: Some("Linux"),
            uname_m: Some("x86_64"),
            available_commands: &["apt-get", "systemctl"],
            user_bus_responds: true,
            meminfo: Some(fixture!("meminfo/64gb")),
            lscpu: Some(fixture!("lscpu/x86_64-8core")),
            df_pk: Some(fixture!("df/roomy")),
            nvidia_smi: Some(fixture!("nvidia-smi/rtx-4090")),
            route: Some(fixture!("route/default")),
            ..Probes::default()
        });
        assert_eq!(machine.tier(), Tier::Maximum);
        assert!(machine.tier_at_least(Tier::Recommended));
        assert!(machine.tier_at_least(Tier::Maximum));
        assert_eq!(machine.acceleration(), Acceleration::Cuda);
        assert_eq!(machine.usability(), Usability::Usable);
        assert_eq!(machine.gpu_name(), Some("NVIDIA GeForce RTX 4090"));
        assert_eq!(machine.verdict.reason, None, "nothing held it back");
        assert_eq!(machine.packaging.manager, PackageManager::AptGet);
        assert!(machine.is_known());
        assert_eq!(
            machine.summary(),
            "Ubuntu 24.04.1 LTS (debian) x86_64, packages: apt-get, services: systemd"
        );
    }

    // AYEAYE-60 — the AMD decision, end to end: the card is detected, named,
    // and declined with the reason stated.
    #[test]
    fn an_amd_machine_names_its_card_and_says_ayeaye_cannot_use_it() {
        let machine = Machine::read(&Probes {
            os_release: Some(fixture!("os-release/arch")),
            uname_s: Some("Linux"),
            uname_m: Some("x86_64"),
            meminfo: Some(fixture!("meminfo/64gb")),
            lscpu: Some(fixture!("lscpu/x86_64-8core")),
            df_pk: Some(fixture!("df/roomy")),
            rocminfo: Some(fixture!("rocminfo/gfx1100")),
            ..Probes::default()
        });
        assert_eq!(
            machine.acceleration(),
            Acceleration::Rocm,
            "the card is real"
        );
        assert_eq!(machine.gpu_name(), Some("AMD Radeon RX 7900 XTX"));
        assert_eq!(
            machine.usability(),
            Usability::Unsupported("ayeaye cannot use an AMD graphics card")
        );
        assert_eq!(machine.tier(), Tier::Recommended);
        assert_eq!(machine.verdict.cause, Some(Cause::GraphicsUnusable));
    }

    // AYEAYE-60 — an Apple Silicon Mac, whose memory is its graphics memory.
    #[test]
    fn a_mac_reaches_the_top_by_having_the_memory_for_it() {
        let machine = Machine::read(&Probes {
            uname_s: Some("Darwin"),
            uname_m: Some("arm64"),
            sw_vers: Some(fixture!("sw_vers/macos-15.1")),
            available_commands: &["launchctl"],
            brew_in_prefix: Some("/opt/homebrew"),
            system_profiler: Some(fixture!("system_profiler/apple-m3-24gb")),
            df_pk: Some(fixture!("df/roomy")),
            route_default_exists: Some(true),
            ..Probes::default()
        });
        assert_eq!(machine.acceleration(), Acceleration::Metal);
        assert_eq!(machine.gpu_name(), Some("Apple M3"));
        assert_eq!(machine.ram_mb, Some(24576));
        assert_eq!(machine.cores, Some(8));
        assert_eq!(machine.tier(), Tier::Maximum);
        assert_eq!(machine.packaging.manager, PackageManager::Brew);
        assert_eq!(
            machine
                .homebrew
                .as_ref()
                .map(|brew| brew.invocation.as_str()),
            Some("/opt/homebrew/bin/brew"),
            "found in its prefix and not on PATH, so a command line has to say where"
        );
        assert_eq!(machine.services.as_str(), "launchd");
    }
    // AYEAYE-60 — the captured small card, end to end. It is real, it is cuda,
    // and it is too small for the listening work, so the machine is a
    // processor-only machine that happens to have a card in it. Saying "there
    // is no graphics card here" to somebody who paid for one is how a tool stops
    // being believed about anything else.
    #[test]
    fn a_captured_small_card_is_named_and_declined_for_being_small() {
        let machine = Machine::read(&Probes {
            os_release: Some(fixture!("os-release/debian-12")),
            uname_s: Some("Linux"),
            uname_m: Some("x86_64"),
            meminfo: Some(fixture!("meminfo/64gb")),
            lscpu: Some(fixture!("lscpu/x86_64-8core")),
            df_pk: Some(fixture!("df/roomy")),
            nvidia_smi: Some(fixture!("nvidia-smi/gtx-1050")),
            ..Probes::default()
        });
        assert_eq!(
            machine.acceleration(),
            Acceleration::Cuda,
            "the card is real"
        );
        assert_eq!(machine.gpu_name(), Some("NVIDIA GeForce GTX 1050"));
        assert_eq!(machine.usability(), Usability::TooSmall);
        assert_eq!(machine.tier(), Tier::Recommended);
        assert_eq!(machine.verdict.cause, Some(Cause::GraphicsSmall));
        assert_eq!(
            machine.verdict.reason,
            Some("the graphics card is too small to be any use")
        );
    }
}
