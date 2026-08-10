//! Whether the numbers describe a machine, or a share of one.
//!
//! This is the sharp case in the whole file. The core count and the memory
//! figure a container reports are the host's, so a 512 MB share of a 64-core
//! server reads as a 64-core server unless somebody looks at the cgroup limits.
//! This module looks, so the numbers can be clamped to the share where it can be
//! read, and the tier capped where it cannot.

use super::Probes;
use super::text::{answered, positive};

/// A cgroup limit, which has three states and not two.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Limit {
    /// A real share of the machine: this many whole megabytes, or whole cores.
    At(u64),
    /// Read, and there is no limit — the machine's own figure is the right one.
    None,
    /// Could not be read at all. The dangerous state, and the only one that
    /// costs a tier.
    Unknown,
}

/// Whether this machine's share of itself is known.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Limits {
    /// Not a container. The numbers describe the whole machine.
    None,
    /// A container whose share was read — including reading that there is no
    /// limit.
    Known,
    /// A container whose share could not be read. The numbers describe the host.
    Unknown,
}

impl Limits {
    /// The lower-case word a caller matches on.
    pub fn as_str(self) -> &'static str {
        match self {
            Limits::None => "none",
            Limits::Known => "known",
            Limits::Unknown => "unknown",
        }
    }
}

/// How much of this computer is yours to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Share {
    /// True inside a container, or anywhere a real cgroup limit is in force.
    pub container: bool,
    /// Whether the share could be read.
    pub limits: Limits,
    /// The memory limit, in whole megabytes.
    pub ram_mb: Limit,
    /// The processor limit, in whole cores.
    pub cores: Limit,
}

/// Anything at or above this is a cgroup saying "no limit" in the version-one
/// idiom of a number as large as it can count to.
const NO_LIMIT_FLOOR: u64 = 4_611_686_018_427_387_904;

/// Work out whether this is a share of a machine, and how big a share.
///
/// The limits are read whether or not a marker was found, and for two reasons. A
/// kubernetes pod under containerd has no marker file, an unshared cgroup
/// namespace that makes `/proc/1/cgroup` read `0::/`, and a memory limit — so
/// the limit is the only evidence there is. And on a machine that is not a
/// container at all, a limit that is really there is worth obeying anyway.
pub fn read(probes: &Probes<'_>) -> Share {
    let ram_mb = memory_limit(probes);
    let cores = core_limit(probes);

    // A real limit is a real share of a machine, whatever is or is not written
    // in /.dockerenv.
    let container =
        marked(probes) || matches!(ram_mb, Limit::At(_)) || matches!(cores, Limit::At(_));

    let limits = if !container {
        Limits::None
    } else if ram_mb == Limit::Unknown || cores == Limit::Unknown {
        Limits::Unknown
    } else {
        Limits::Known
    };

    Share {
        container,
        limits,
        ram_mb,
        cores,
    }
}

/// Bring a measurement down to the share, and only ever down.
///
/// An upper bound is not a measurement. A container that was allowed 32 GB and
/// whose `/proc` could not be read has not been measured at 32 GB, and answering
/// 32768 here would turn "we could not tell" — which caps the tier — into a
/// large machine, which does not.
pub fn clamp(measured: Option<u64>, limit: Limit) -> Option<u64> {
    let Limit::At(ceiling) = limit else {
        return measured;
    };
    Some(measured?.min(ceiling))
}

/// Where this process's own limit file lives.
///
/// `/sys/fs/cgroup/memory.max` is the *host's* root under a shared cgroup
/// namespace, and the root has no limit on anything — a true statement about the
/// host and a dangerous one about a container. Composing the path from
/// `/proc/self/cgroup` asks the question about this process instead.
///
/// `controller` is empty for the unified hierarchy and a controller name for the
/// version-one hierarchy that governs it.
pub fn cgroup_path(self_cgroup: &str, controller: &str, leaf: &str, root: &str) -> Option<String> {
    let relative = cgroup_relative(self_cgroup, controller)?;
    let relative = match relative {
        "/" => "",
        other => other.strip_suffix('/').unwrap_or(other),
    };
    let base = if controller.is_empty() {
        format!("{root}{relative}")
    } else {
        format!("{root}/{controller}{relative}")
    };
    Some(format!("{base}/{leaf}"))
}

/// This process's own cgroup path for one controller.
///
/// An empty controller asks for the unified hierarchy, whose line is
/// `0::/some/path`. A named one asks for the version-one hierarchy that controls
/// it, whose line is `7:memory,cpuset:/some/path` — the controller field is a
/// comma-separated list and the name has to be a whole element of it, or `cpu`
/// would match `cpuacct`.
fn cgroup_relative<'a>(self_cgroup: &'a str, controller: &str) -> Option<&'a str> {
    for line in self_cgroup.lines() {
        let mut parts = line.splitn(3, ':');
        let Some(hierarchy) = parts.next() else {
            continue;
        };
        let Some(controllers) = parts.next() else {
            continue;
        };
        let Some(path) = parts.next() else {
            continue;
        };
        if path.is_empty() {
            continue;
        }
        if controller.is_empty() {
            if hierarchy == "0" && controllers.is_empty() {
                return Some(path);
            }
            continue;
        }
        if controllers.split(',').any(|name| name == controller) {
            return Some(path);
        }
    }
    None
}

// -------------------------------------------------------------- the limits

fn memory_limit(probes: &Probes<'_>) -> Limit {
    // Version two first, version one second; both are read, neither is required.
    // Only a failed read falls through — a version-two file that says `max` has
    // answered, and its answer is "no limit".
    if let Some(text) = answered(probes.cgroup_memory_max) {
        let value = text.trim();
        if value == "max" {
            return Limit::None;
        }
        return bytes_to_limit(value);
    }
    match answered(probes.cgroup_memory_limit) {
        Some(text) => bytes_to_limit(text.trim()),
        None => Limit::Unknown,
    }
}

fn bytes_to_limit(value: &str) -> Limit {
    let Some(bytes) = positive(value) else {
        return Limit::Unknown;
    };
    if bytes >= NO_LIMIT_FLOOR {
        return Limit::None;
    }
    Limit::At(bytes / 1_048_576)
}

/// A share of a core and a half is one core you can count on.
///
/// Rounding it up to two would be counting on a core that is not there, which is
/// the direction this file never rounds in; a share of less than a whole core
/// still counts as one, because there is no such thing as most of a processor to
/// schedule on.
fn core_limit(probes: &Probes<'_>) -> Limit {
    let (quota, period) = if let Some(text) = answered(probes.cgroup_cpu_max) {
        let mut fields = text.split_whitespace();
        let quota = fields.next().unwrap_or("");
        if quota == "max" {
            return Limit::None;
        }
        (quota, fields.next().unwrap_or(""))
    } else if let Some(text) = answered(probes.cgroup_cpu_quota) {
        let quota = text.trim();
        if quota == "-1" {
            return Limit::None;
        }
        (quota, probes.cgroup_cpu_period.unwrap_or("").trim())
    } else {
        return Limit::Unknown;
    };

    match (positive(quota), positive(period)) {
        (Some(quota), Some(period)) => Limit::At((quota / period).max(1)),
        _ => Limit::Unknown,
    }
}

// --------------------------------------------------------------- the marks

/// Something on this machine says outright that it is a container.
///
/// Several marks, because no one of them is reliable. Docker writes
/// `/.dockerenv`; podman and lxc write `/run/.containerenv` and the `container`
/// environment variable; systemd-nspawn writes `/run/systemd/container`. A
/// containerd or CRI pod writes none of them, which is what the limits are for.
fn marked(probes: &Probes<'_>) -> bool {
    if probes.container_marker {
        return true;
    }
    let in_cgroup = probes.proc1_cgroup.is_some_and(|text| {
        text.lines().any(|line| {
            ["docker", "lxc", "kubepods", "libpod", "containerd"]
                .iter()
                .any(|mark| line.contains(mark))
                || line.contains("/machine.slice/")
        })
    });
    if in_cgroup {
        return true;
    }
    probes.mountinfo.is_some_and(|text| {
        text.lines().any(|line| {
            line.contains("/docker/containers/")
                || line.contains("/kubepods/")
                || line.contains("/containers/storage/")
        })
    })
}

#[cfg(test)]
mod tests {
    use super::{Limit, Limits, cgroup_path, clamp, read};
    use crate::machine::Probes;
    use crate::machine::fixture;

    /// A container that says it is one, so only the limits vary.
    fn in_a_container(probes: Probes<'_>) -> super::Share {
        read(&Probes {
            container_marker: true,
            ..probes
        })
    }

    // AYEAYE-60 — the captured cgroup files, at the figures
    // tests/cases/hardware_probe_test.sh pins for them.
    #[test]
    fn a_share_of_the_machine_is_read_from_either_cgroup_layout() {
        let v2 = in_a_container(Probes {
            cgroup_memory_max: Some(fixture!("cgroup/memory-max-1g")),
            cgroup_cpu_max: Some(fixture!("cgroup/cpu-max-two-cores")),
            ..Probes::default()
        });
        assert_eq!(v2.ram_mb, Limit::At(1024));
        assert_eq!(v2.cores, Limit::At(2));
        assert_eq!(v2.limits, Limits::Known);

        let v1 = in_a_container(Probes {
            cgroup_memory_limit: Some(fixture!("cgroup/memory-limit-v1-2g")),
            cgroup_cpu_quota: Some(fixture!("cgroup/cpu-quota-v1-four-cores")),
            cgroup_cpu_period: Some(fixture!("cgroup/cpu-period-v1")),
            ..Probes::default()
        });
        assert_eq!(v1.ram_mb, Limit::At(2048));
        assert_eq!(v1.cores, Limit::At(4));
        assert_eq!(v1.limits, Limits::Known);
    }

    // AYEAYE-60 — "no limit" is an answer, not a failure to answer: the
    // machine's own figure is the right one.
    #[test]
    fn no_limit_is_read_as_an_answer_and_not_as_a_mystery() {
        let v2 = in_a_container(Probes {
            cgroup_memory_max: Some(fixture!("cgroup/memory-max-unlimited")),
            cgroup_cpu_max: Some(fixture!("cgroup/cpu-max-unlimited")),
            ..Probes::default()
        });
        assert_eq!(
            v2.ram_mb,
            Limit::None,
            "\"max\" means the host's figure is right"
        );
        assert_eq!(v2.cores, Limit::None);
        assert_eq!(v2.limits, Limits::Known);

        let v1 = in_a_container(Probes {
            cgroup_memory_limit: Some(fixture!("cgroup/memory-limit-v1-unlimited")),
            cgroup_cpu_quota: Some(fixture!("cgroup/cpu-quota-v1-unlimited")),
            cgroup_cpu_period: Some(fixture!("cgroup/cpu-period-v1")),
            ..Probes::default()
        });
        assert_eq!(
            v1.ram_mb,
            Limit::None,
            "the old layout says no limit with a number as large as it can count to"
        );
        assert_eq!(v1.cores, Limit::None);
    }

    // AYEAYE-60
    #[test]
    fn a_fractional_processor_share_rounds_down_and_never_below_one() {
        let half = in_a_container(Probes {
            cgroup_memory_max: Some(fixture!("cgroup/memory-max-unlimited")),
            cgroup_cpu_max: Some(fixture!("cgroup/cpu-max-one-and-a-half-cores")),
            ..Probes::default()
        });
        assert_eq!(
            half.cores,
            Limit::At(1),
            "a share and a half is one whole core you can count on"
        );
        let sliver = in_a_container(Probes {
            cgroup_memory_max: Some(fixture!("cgroup/memory-max-unlimited")),
            cgroup_cpu_max: Some("50000 100000\n"),
            ..Probes::default()
        });
        assert_eq!(
            sliver.cores,
            Limit::At(1),
            "there is no such thing as most of a processor to schedule on"
        );
    }

    // AYEAYE-60 — "cpu,cpuacct" contains "cpuacct", and a cgroup line for
    // cpuacct alone is not a cgroup line for cpu.
    #[test]
    fn a_controller_name_is_matched_whole_and_not_as_a_substring() {
        let mixed = "9:cpuacct:/somewhere-else\n5:cpu,cpuset:/right-here\n";
        assert_eq!(
            cgroup_path(mixed, "cpu", "cpu.cfs_quota_us", "/sys/fs/cgroup").as_deref(),
            Some("/sys/fs/cgroup/cpu/right-here/cpu.cfs_quota_us")
        );
    }

    // AYEAYE-60 — under a shared cgroup namespace the tree root is the host's,
    // which reports no limit at all; believing that is how a 512 MB share gets
    // told it can hold a three-gigabyte model.
    #[test]
    fn the_limit_path_is_composed_from_this_processs_own_cgroup() {
        let root = "/sys/fs/cgroup";
        assert_eq!(
            cgroup_path(
                fixture!("cgroup/self-cgroup-v2-docker"),
                "",
                "memory.max",
                root
            )
            .as_deref(),
            Some("/sys/fs/cgroup/docker/abc123/memory.max")
        );
        assert_eq!(
            cgroup_path(
                fixture!("cgroup/self-cgroup-v1-docker"),
                "memory",
                "memory.limit_in_bytes",
                root
            )
            .as_deref(),
            Some("/sys/fs/cgroup/memory/docker/abc123/memory.limit_in_bytes")
        );
        assert_eq!(
            cgroup_path(fixture!("cgroup/self-cgroup-v2-root"), "", "cpu.max", root).as_deref(),
            Some("/sys/fs/cgroup/cpu.max"),
            "the root's own path adds nothing"
        );
        assert_eq!(
            cgroup_path(
                fixture!("cgroup/self-cgroup-v2-docker"),
                "memory",
                "memory.max",
                root
            ),
            None,
            "a unified-only listing has no version-one hierarchy to ask"
        );
    }

    // AYEAYE-60 — a kubernetes pod under containerd has no marker file and a
    // /proc/1/cgroup that reads exactly "0::/". The limits are the only
    // evidence there is that this is a share of a machine rather than a machine.
    #[test]
    fn a_pod_with_no_marker_at_all_is_still_found_by_its_limits() {
        let pod = read(&Probes {
            proc1_cgroup: Some(fixture!("cgroup/proc1-docker")),
            cgroup_memory_max: Some(fixture!("cgroup/memory-max-1g")),
            cgroup_cpu_max: Some(fixture!("cgroup/cpu-max-two-cores")),
            ..Probes::default()
        });
        assert!(pod.container, "a real limit is a real share of a machine");
        assert_eq!(pod.limits, Limits::Known);
        assert_eq!(pod.ram_mb, Limit::At(1024));
        assert_eq!(pod.cores, Limit::At(2));
    }

    // AYEAYE-60
    #[test]
    fn the_marks_that_say_container_and_the_ones_that_do_not() {
        let by_cgroup = |text| {
            read(&Probes {
                proc1_cgroup: Some(text),
                ..Probes::default()
            })
        };
        assert!(by_cgroup(fixture!("cgroup/proc1-docker-v1")).container);
        assert!(by_cgroup(fixture!("cgroup/proc1-nspawn")).container);
        assert!(!by_cgroup(fixture!("cgroup/proc1-host")).container);

        let bare = read(&Probes::default());
        assert!(!bare.container);
        assert_eq!(
            bare.limits,
            Limits::None,
            "not a container, so nothing to share"
        );

        assert!(
            read(&Probes {
                mountinfo: Some(
                    "42 41 0:38 / /etc/hosts rw - ext4 /dev/sda1 rw,/docker/containers/abc\n"
                ),
                ..Probes::default()
            })
            .container
        );

        // Not a container without limits: a container whose limits are a
        // mystery, which is the one case that costs a tier.
        let mysterious = read(&Probes {
            container_marker: true,
            ..Probes::default()
        });
        assert!(mysterious.container);
        assert_eq!(mysterious.limits, Limits::Unknown);
    }

    // AYEAYE-60
    #[test]
    fn a_measurement_is_only_ever_brought_down_to_the_share() {
        assert_eq!(clamp(Some(64263), Limit::At(1024)), Some(1024));
        assert_eq!(clamp(Some(8), Limit::At(2)), Some(2));
        assert_eq!(
            clamp(Some(1849), Limit::At(32768)),
            Some(1849),
            "a limit larger than the machine is not more machine"
        );
        assert_eq!(clamp(Some(1849), Limit::None), Some(1849));
        assert_eq!(clamp(Some(1849), Limit::Unknown), Some(1849));
        assert_eq!(
            clamp(None, Limit::At(32768)),
            None,
            "an upper bound is not a measurement"
        );
    }
    // AYEAYE-60 — step.detect.hardware.limits is one of these three words.
    #[test]
    fn the_limits_verdict_is_spelled_the_way_the_shell_writes_it() {
        assert_eq!(
            [Limits::None, Limits::Known, Limits::Unknown].map(Limits::as_str),
            ["none", "known", "unknown"]
        );
    }
}
