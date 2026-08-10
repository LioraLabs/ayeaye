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
pub mod platform;
pub mod size;

pub use graphics::{Acceleration, Graphics};
pub use platform::{
    Family, Os, PackageManager, Packaging, Platform, ServiceManager, brew_prefix_of, identify,
    packaging, service_manager,
};

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
}
