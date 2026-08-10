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

pub use graphics::{Acceleration, Graphics};
pub use network::Network;
pub use platform::{
    Family, Os, PackageManager, Packaging, Platform, ServiceManager, brew_prefix_of, identify,
    packaging, service_manager,
};
pub use share::{Limit, Limits, Share};

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
