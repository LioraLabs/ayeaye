//! Asking this computer about itself.
//!
//! [`ayeaye_core::machine`] is every judgement about hardware and platform and
//! not one of the questions: it takes captured text and returns a verdict, which
//! is what makes the whole verdict reproducible from the fixtures. This is the
//! half that was missing — the commands that produce that text, and the files
//! that hold it.
//!
//! **Nothing here changes anything.** Every command in the list is a read, and
//! the one that addresses a service manager —
//! `systemctl --user show-environment` — is the read the shell has always used
//! to find out whether there is a user session at all. That is not a stylistic
//! preference: a probe with an effect would make `ayeaye setup` alter a machine
//! before it had decided anything about it.
//!
//! Everything goes through [`Sources`] rather than straight to `Command` and
//! `fs`, so the whole capture can be asserted — the argv of every command
//! included — without running any of it.

use std::path::Path;

use ayeaye_core::machine::{Privilege, Probes, ServiceManager, packages, platform, share, size};
use ayeaye_core::service::Session;

use crate::service::{Outcome, Runner, Subprocess};

/// Where `/proc/self/cgroup` names the paths a container's limits live under.
const CGROUP_ROOT: &str = "/sys/fs/cgroup";

/// Every command whose mere presence a decision is made from.
///
/// A fixed list rather than "whatever is on `PATH`" because
/// [`Probes::available_commands`] is asked `contains`, and a decision must not
/// depend on how long somebody's `PATH` is. The first seven are the platform
/// layer's — the five package managers and the two service managers; the rest
/// are what setup verifies and never installs.
pub const LOOKED_FOR: &[&str] = &[
    "apt-get",
    "dnf",
    "yum",
    "pacman",
    "zypper",
    "systemctl",
    "launchctl",
    "brew",
    "sudo",
    "tmux",
    "curl",
    "ffmpeg",
    "claude",
    "codex",
    "cliban",
    "tailscale",
    "route",
];

// Every name above is asked about somewhere: the five package managers and the
// two service managers by `machine::platform`, and `brew`, `sudo`, `tmux`,
// `curl`, `ffmpeg`, `claude`, `codex`, `cliban`, `tailscale` and `route` by the
// capture itself or by `crate::health`. A name nobody asks about is a probe run
// on every machine for nothing.

/// The prefixes a Homebrew is installed under, in the order the shell tries
/// them: Apple silicon, Intel, and Linuxbrew.
const BREW_PREFIXES: &[&str] = &["/opt/homebrew", "/usr/local", "/home/linuxbrew/.linuxbrew"];

/// The three files a container leaves. None is reliable alone, and the fourth
/// mark is the `container` environment variable, which is not a path.
const CONTAINER_MARKERS: &[&str] = &[
    "/.dockerenv",
    "/run/.containerenv",
    "/run/systemd/container",
];

/// Where the world is asked.
///
/// Four operations, and the split between them is what a test needs to say
/// something precise: `run` records an argv, `read` and `is_dir` record a path,
/// and `which` is a `PATH` search rather than another subprocess.
pub trait Sources {
    /// Run a command and hand back what it said.
    fn run(&self, argv: &[String]) -> Outcome;
    /// The contents of a file, or `None` when there is nothing to read. A file
    /// that exists and is empty reads as `Some("")`, which is a different fact
    /// from a file that is not there.
    fn read(&self, path: &str) -> Option<String>;
    /// Whether that path is a directory.
    fn is_dir(&self, path: &str) -> bool;
    /// Whether that path is a file this machine could run.
    fn is_executable(&self, path: &str) -> bool;
    /// One variable out of the process environment.
    fn env(&self, name: &str) -> Option<String>;
    /// Where that command is on `PATH`, if it is anywhere.
    fn which(&self, name: &str) -> Option<String>;
}

/// The real machine.
#[derive(Debug, Clone, Copy, Default)]
pub struct System;

impl Sources for System {
    fn run(&self, argv: &[String]) -> Outcome {
        Subprocess.run(argv)
    }

    fn read(&self, path: &str) -> Option<String> {
        std::fs::read(path)
            .ok()
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
    }

    fn is_dir(&self, path: &str) -> bool {
        Path::new(path).is_dir()
    }

    fn is_executable(&self, path: &str) -> bool {
        is_executable(Path::new(path))
    }

    fn env(&self, name: &str) -> Option<String> {
        std::env::var(name).ok()
    }

    fn which(&self, name: &str) -> Option<String> {
        let path = std::env::var_os("PATH")?;
        std::env::split_paths(&path)
            // An empty `PATH` element means the current directory, and a
            // command found there is whatever the caller happened to be
            // standing in. `config::choose_cliban` skips it for the same
            // reason; so does this.
            .filter(|dir| !dir.as_os_str().is_empty())
            .map(|dir| dir.join(name))
            .find(|candidate| is_executable(candidate))
            .map(|found| found.to_string_lossy().into_owned())
    }
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

/// Everything this machine said about itself, held so it can be borrowed.
///
/// [`Probes`] is all `&str`, deliberately: the core may not own a heap of text
/// it did not ask for. So the capture owns the strings and hands out a borrow of
/// them, and one capture answers every question for a whole run — which is also
/// what stops two answers to "what is this machine" from disagreeing inside one
/// command.
#[derive(Debug, Clone, Default)]
pub struct Captured {
    os_release: Option<String>,
    uname_s: Option<String>,
    uname_m: Option<String>,
    sw_vers: Option<String>,
    available: Vec<String>,
    user_bus_responds: bool,
    brew_on_path: Option<String>,
    brew_in_prefix: Option<String>,
    nproc: Option<String>,
    lscpu: Option<String>,
    cpuinfo: Option<String>,
    sysctl_ncpu: Option<String>,
    sysctl_memsize: Option<String>,
    system_profiler: Option<String>,
    meminfo: Option<String>,
    free_m: Option<String>,
    df_pk: Option<String>,
    nvidia_smi: Option<String>,
    rocminfo: Option<String>,
    sysctl_brand_string: Option<String>,
    container_marker: bool,
    proc1_cgroup: Option<String>,
    mountinfo: Option<String>,
    cgroup_memory_max: Option<String>,
    cgroup_memory_limit: Option<String>,
    cgroup_cpu_max: Option<String>,
    cgroup_cpu_quota: Option<String>,
    cgroup_cpu_period: Option<String>,
    route: Option<String>,
    route6: Option<String>,
    route_default_exists: Option<bool>,
    privilege: Privilege,
    launchd_uid: Option<String>,
}

impl Captured {
    /// What the core reads, borrowed from what was captured — **without the
    /// commands on `PATH`**.
    ///
    /// Private, and it has to stay private. `available_commands` is a slice of
    /// `&str` that cannot be built inside this method and outlive it, so it is
    /// left empty here and filled by [`Captured::machine`]. A caller reaching
    /// for this directly would silently be told there is no package manager, no
    /// service manager and no Homebrew — exactly the class of wrong answer
    /// `Probes`' own doc ("`None` is never the same as an empty answer") exists
    /// to prevent. Ask [`Captured::machine`] instead.
    fn probes(&self) -> Probes<'_> {
        Probes {
            os_release: self.os_release.as_deref(),
            uname_s: self.uname_s.as_deref(),
            uname_m: self.uname_m.as_deref(),
            sw_vers: self.sw_vers.as_deref(),
            available_commands: &[],
            user_bus_responds: self.user_bus_responds,
            brew_on_path: self.brew_on_path.as_deref(),
            brew_in_prefix: self.brew_in_prefix.as_deref(),
            nproc: self.nproc.as_deref(),
            lscpu: self.lscpu.as_deref(),
            cpuinfo: self.cpuinfo.as_deref(),
            sysctl_ncpu: self.sysctl_ncpu.as_deref(),
            sysctl_memsize: self.sysctl_memsize.as_deref(),
            system_profiler: self.system_profiler.as_deref(),
            meminfo: self.meminfo.as_deref(),
            free_m: self.free_m.as_deref(),
            df_pk: self.df_pk.as_deref(),
            nvidia_smi: self.nvidia_smi.as_deref(),
            rocminfo: self.rocminfo.as_deref(),
            sysctl_brand_string: self.sysctl_brand_string.as_deref(),
            container_marker: self.container_marker,
            proc1_cgroup: self.proc1_cgroup.as_deref(),
            mountinfo: self.mountinfo.as_deref(),
            cgroup_memory_max: self.cgroup_memory_max.as_deref(),
            cgroup_memory_limit: self.cgroup_memory_limit.as_deref(),
            cgroup_cpu_max: self.cgroup_cpu_max.as_deref(),
            cgroup_cpu_quota: self.cgroup_cpu_quota.as_deref(),
            cgroup_cpu_period: self.cgroup_cpu_period.as_deref(),
            route: self.route.as_deref(),
            route6: self.route6.as_deref(),
            route_default_exists: self.route_default_exists,
        }
    }

    /// The whole machine, judged.
    ///
    /// The borrowed `available_commands` cannot be built inside `probes` — it is
    /// a slice of `&str` and would not outlive the call — so it is assembled
    /// here and the verdict is taken while it is alive. That is the whole reason
    /// this is the only door.
    pub fn machine(&self) -> ayeaye_core::Machine {
        let names: Vec<&str> = self.available.iter().map(String::as_str).collect();
        ayeaye_core::Machine::read(&Probes {
            available_commands: &names,
            ..self.probes()
        })
    }

    /// What to tell somebody who has to install those packages themselves.
    ///
    /// Two lines, and never an install: the criterion is that setup prints the
    /// exact command for this platform, and the whole of `machine::packages` is
    /// built so that nothing here can run it.
    pub fn install_hint(&self, logical: &[&str]) -> [String; 2] {
        let machine = self.machine();
        packages::manual_hint(
            &machine.platform,
            &machine.packaging,
            machine.homebrew.as_ref(),
            self.privilege,
            logical,
        )
    }

    /// The session this machine's service verbs address, out of the one capture
    /// rather than asked again.
    ///
    /// `None` is the third answer — a machine with neither manager — and it is
    /// the same answer [`session`] gives; this is the door for a caller that has
    /// already asked the machine everything, so `ayeaye setup` does not run the
    /// platform probes twice and cannot end up with two ideas of what it is on.
    pub fn session(&self) -> Option<Session> {
        Session::for_manager(self.machine().services, self.launchd_uid.as_deref())
    }

    /// Whether that command is on this machine's `PATH`.
    ///
    /// Only the names in [`LOOKED_FOR`] can answer: a decision made from a
    /// command nobody captured would be a decision made from `false`.
    pub fn has(&self, name: &str) -> bool {
        debug_assert!(
            LOOKED_FOR.contains(&name),
            "{name} was never looked for; add it to LOOKED_FOR"
        );
        self.available.iter().any(|found| found == name)
    }
}

/// Which service manager this machine's user session has, asking only what that
/// question needs.
///
/// `ayeaye service <verb>` has no use for a graphics card or for free disk
/// space, and [`capture`] starts a dozen processes to learn about both. The
/// *decision* is still AYEAYE-60's and made in exactly one place — this narrows
/// the probes, not the judgement.
pub fn service_manager(sources: &impl Sources) -> ServiceManager {
    let uname_s = said(sources, &["uname", "-s"]);
    let os_release = read_first(sources, &["/etc/os-release", "/usr/lib/os-release"]);
    let available: Vec<&str> = ["systemctl", "launchctl"]
        .into_iter()
        .filter(|name| sources.which(name).is_some())
        .collect();
    let platform = platform::identify(os_release.as_deref(), uname_s.as_deref(), None, None);
    platform::service_manager(
        platform.os,
        &available,
        // Not asked where there is no systemctl to ask, so a Mac does not start
        // a process to be told the command does not exist.
        available.contains(&"systemctl")
            && sources
                .run(&argv(&["systemctl", "--user", "show-environment"]))
                .ok,
    )
}

/// The session this machine's service verbs address, or `None` where there is
/// nothing to address.
///
/// This is where AYEAYE-60's detector and AYEAYE-61's verbs meet, and the whole
/// of what AYEAYE-61 recorded as owed. `None` is the third answer: a container
/// and a stripped-down Linux really have neither manager, and the shell treats
/// running ayeaye by hand as a supported way to use it rather than as a broken
/// machine. Before this, the binary assumed a manager existed and such a machine
/// got a `systemctl` that is not there.
pub fn session(sources: &impl Sources) -> Option<Session> {
    let manager = service_manager(sources);
    Session::for_manager(manager, launchd_uid(sources, manager).as_deref())
}

/// The uid a launchd domain is addressed by, where one is needed.
///
/// `id -u` is where the shell has always read it from. `None` on anything that
/// is not launchd, because systemd's user manager is addressed as whoever is
/// calling and a uid there would describe a thing that does not exist.
pub fn launchd_uid(sources: &impl Sources, manager: ServiceManager) -> Option<String> {
    if manager != ServiceManager::Launchd {
        return None;
    }
    said(sources, &["id", "-u"]).map(|said| said.trim().to_string())
}

/// Ask this machine everything, once.
///
/// `model_dir` is where the weights would land, and it is an argument because
/// free space is measured *there* rather than wherever this process happens to
/// have been started from.
pub fn capture(sources: &impl Sources, model_dir: &str) -> Captured {
    let mut captured = Captured {
        os_release: read_first(sources, &["/etc/os-release", "/usr/lib/os-release"]),
        uname_s: said(sources, &["uname", "-s"]),
        uname_m: said(sources, &["uname", "-m"]),
        available: LOOKED_FOR
            .iter()
            .filter(|name| sources.which(name).is_some())
            .map(|name| (*name).to_string())
            .collect(),
        // `show-environment` is a read, and it is the read the shell has always
        // used: a container has the systemctl binary and no user session, and
        // asking the binary whether it exists answers the wrong question.
        user_bus_responds: sources
            .run(&argv(&["systemctl", "--user", "show-environment"]))
            .ok,
        brew_on_path: sources.which("brew"),
        brew_in_prefix: BREW_PREFIXES
            .iter()
            .find(|prefix| sources.is_executable(&format!("{prefix}/bin/brew")))
            .map(|prefix| (*prefix).to_string()),
        nproc: said(sources, &["nproc"]),
        // Under `LC_ALL=C`, because util-linux is translated and every field
        // this is parsed for is an English label. Through `env` rather than a
        // variable on the runner, so that what changes the locale is visible in
        // the recorded argv.
        lscpu: said(sources, &["env", "LC_ALL=C", "lscpu"]),
        cpuinfo: sources.read("/proc/cpuinfo"),
        sysctl_ncpu: said(sources, &["sysctl", "-n", "hw.ncpu"]),
        sysctl_memsize: said(sources, &["sysctl", "-n", "hw.memsize"]),
        system_profiler: said(sources, &["system_profiler", "SPHardwareDataType"]),
        meminfo: sources.read("/proc/meminfo"),
        free_m: said(sources, &["free", "-m"]),
        nvidia_smi: said(
            sources,
            &[
                "nvidia-smi",
                "--query-gpu=name,memory.total",
                "--format=csv,noheader",
            ],
        ),
        rocminfo: said(sources, &["rocminfo"]),
        sysctl_brand_string: said(sources, &["sysctl", "-n", "machdep.cpu.brand_string"]),
        container_marker: CONTAINER_MARKERS
            .iter()
            .any(|mark| sources.read(mark).is_some())
            || sources.env("container").is_some(),
        proc1_cgroup: sources.read("/proc/1/cgroup"),
        mountinfo: sources.read("/proc/self/mountinfo"),
        route: sources.read("/proc/net/route"),
        route6: sources.read("/proc/net/ipv6_route"),
        ..Captured::default()
    };

    captured.df_pk = size::model_dir_ancestors(model_dir)
        .iter()
        .find(|dir| sources.is_dir(dir))
        .and_then(|dir| said(sources, &["df", "-Pk", dir]));

    let self_cgroup = sources.read("/proc/self/cgroup").unwrap_or_default();
    captured.cgroup_memory_max = cgroup(sources, &self_cgroup, "", "memory.max");
    captured.cgroup_memory_limit = cgroup(sources, &self_cgroup, "memory", "memory.limit_in_bytes");
    captured.cgroup_cpu_max = cgroup(sources, &self_cgroup, "", "cpu.max");
    captured.cgroup_cpu_quota = cgroup(sources, &self_cgroup, "cpu", "cpu.cfs_quota_us");
    captured.cgroup_cpu_period = cgroup(sources, &self_cgroup, "cpu", "cpu.cfs_period_us");

    // Only where there is no `/proc/net/route` to read, which is what makes
    // this a Mac question. Asked otherwise it would start a process on every
    // Linux machine to learn something already known.
    if captured.route.is_none() && captured.available.iter().any(|name| name == "route") {
        captured.route_default_exists =
            Some(sources.run(&argv(&["route", "-n", "get", "default"])).ok);
    }

    // `id -u` is where lib/pkg.sh reads privilege from, and `sudo` on PATH is
    // the rest of it. Homebrew refuses to run as root, so this is not simply
    // "can I install": it is what a generated command line has to say.
    let uid = said(sources, &["id", "-u"]).map(|said| said.trim().to_string());
    captured.privilege = match uid.as_deref() {
        Some("0") => Privilege::Root,
        _ if captured.available.iter().any(|name| name == "sudo") => Privilege::Sudo,
        _ => Privilege::None,
    };
    // Kept only where a domain is addressed by one. systemd's user manager is
    // addressed as whoever is calling, and a uid there would describe a thing
    // that does not exist.
    captured.launchd_uid = uid.filter(|uid| !uid.is_empty());

    // macOS only, and asked last so the two cheap `uname` calls have already
    // said whether this is a Mac. The one gated probe, because it is the one
    // whose *absence* the platform layer reads as a fact: `identify` falls back
    // to `uname` when `sw_vers` says nothing, so running it on a Linux would be
    // a process started to be told it does not exist. The other single-platform
    // probes are ungated because a failure and an absence are the same answer to
    // them — `None`.
    if captured.uname_s.as_deref().map(str::trim) == Some("Darwin") {
        captured.sw_vers = said(sources, &["sw_vers"]);
    }

    captured
}

/// One cgroup file, at the path this process's own cgroup names, falling back to
/// the fixed one.
///
/// The fallback is the shell's, and it is not redundant: `/sys/fs/cgroup/memory.max`
/// under a shared cgroup namespace is the *host's* root and says `max`, while
/// the path built from `/proc/self/cgroup` is the one that carries the limit
/// this process is really under. Either can be the readable one.
fn cgroup(
    sources: &impl Sources,
    self_cgroup: &str,
    controller: &str,
    leaf: &str,
) -> Option<String> {
    let derived = share::cgroup_path(self_cgroup, controller, leaf, CGROUP_ROOT)
        .and_then(|path| sources.read(&path));
    derived.or_else(|| {
        let fallback = if controller.is_empty() {
            format!("{CGROUP_ROOT}/{leaf}")
        } else {
            format!("{CGROUP_ROOT}/{controller}/{leaf}")
        };
        sources.read(&fallback)
    })
}

/// The first of those files that can be read at all.
fn read_first(sources: &impl Sources, paths: &[&str]) -> Option<String> {
    paths.iter().find_map(|path| sources.read(path))
}

/// What that command printed, or `None` when it could not be run or said
/// nothing.
///
/// A command that failed said nothing worth parsing: `nvidia-smi` on a machine
/// with no driver exits non-zero and prints an explanation, and treating that
/// explanation as a list of graphics cards is how a machine acquires an
/// imaginary one. Whitespace alone is not an answer either — `Probes` is built
/// on the distinction between "nothing to read" and "an empty answer", and a
/// command that printed a newline has told us nothing.
fn said(sources: &impl Sources, words: &[&str]) -> Option<String> {
    let outcome = sources.run(&argv(words));
    if !outcome.ok || outcome.output.trim().is_empty() {
        return None;
    }
    Some(outcome.output)
}

fn argv(words: &[&str]) -> Vec<String> {
    words.iter().map(|word| (*word).to_string()).collect()
}

#[cfg(test)]
mod tests {
    use super::{Captured, LOOKED_FOR, Sources, capture, session};
    use crate::service::Outcome;
    use ayeaye_core::machine::{Acceleration, PackageManager, ServiceManager, Tier};
    use ayeaye_core::service::Manager;
    use std::cell::RefCell;
    use std::collections::HashMap;

    /// A machine made of answers, which records everything it was asked.
    #[derive(Default)]
    struct Fake {
        /// Keyed on the argv joined with spaces.
        commands: HashMap<String, Outcome>,
        files: HashMap<String, String>,
        dirs: Vec<String>,
        executable: Vec<String>,
        environment: HashMap<String, String>,
        path: Vec<String>,
        ran: RefCell<Vec<Vec<String>>>,
        read: RefCell<Vec<String>>,
    }

    impl Fake {
        fn says(mut self, argv: &str, output: &str) -> Self {
            self.commands.insert(
                argv.to_string(),
                Outcome {
                    ok: true,
                    output: output.to_string(),
                },
            );
            self
        }

        fn holds(mut self, path: &str, text: &str) -> Self {
            self.files.insert(path.to_string(), text.to_string());
            self
        }

        fn directory(mut self, path: &str) -> Self {
            self.dirs.push(path.to_string());
            self
        }

        fn on_path(mut self, name: &str) -> Self {
            self.path.push(name.to_string());
            self
        }
    }

    impl Sources for Fake {
        fn run(&self, argv: &[String]) -> Outcome {
            self.ran.borrow_mut().push(argv.to_vec());
            self.commands
                .get(&argv.join(" "))
                .cloned()
                .unwrap_or(Outcome {
                    ok: false,
                    output: "command not found".to_string(),
                })
        }

        fn read(&self, path: &str) -> Option<String> {
            self.read.borrow_mut().push(path.to_string());
            self.files.get(path).cloned()
        }

        fn is_dir(&self, path: &str) -> bool {
            self.dirs.iter().any(|dir| dir == path)
        }

        fn is_executable(&self, path: &str) -> bool {
            self.executable.iter().any(|found| found == path)
        }

        fn env(&self, name: &str) -> Option<String> {
            self.environment.get(name).cloned()
        }

        fn which(&self, name: &str) -> Option<String> {
            self.path
                .iter()
                .any(|on| on == name)
                .then(|| format!("/usr/bin/{name}"))
        }
    }

    fn linux_with_nvidia() -> Fake {
        Fake::default()
            .holds(
                "/etc/os-release",
                include_str!("../../../tests/fixtures/os-release/ubuntu-24.04"),
            )
            .holds(
                "/proc/meminfo",
                include_str!("../../../tests/fixtures/meminfo/64gb"),
            )
            .holds(
                "/proc/net/route",
                include_str!("../../../tests/fixtures/route/default"),
            )
            .says("uname -s", "Linux\n")
            .says("uname -m", "x86_64\n")
            .says(
                "env LC_ALL=C lscpu",
                include_str!("../../../tests/fixtures/lscpu/x86_64-8core"),
            )
            .says(
                "df -Pk /home/tester/.local/state/ayeaye",
                include_str!("../../../tests/fixtures/df/roomy"),
            )
            .says(
                "nvidia-smi --query-gpu=name,memory.total --format=csv,noheader",
                include_str!("../../../tests/fixtures/nvidia-smi/rtx-4090"),
            )
            .says("systemctl --user show-environment", "PATH=/usr/bin\n")
            .directory("/home/tester/.local/state/ayeaye")
            .on_path("apt-get")
            .on_path("systemctl")
            .on_path("tmux")
    }

    // AYEAYE-62 — the whole point of the capture: what the real machine says,
    // judged by AYEAYE-60's parsers, comes out as one verdict. The figures here
    // are the ones machine/mod.rs pins for these same fixtures, reached through
    // the commands and files instead of through a literal.
    #[test]
    fn what_this_machine_says_becomes_one_verdict_about_it() {
        let captured = capture(&linux_with_nvidia(), "/home/tester/.local/state/ayeaye");
        let machine = captured.machine();
        assert_eq!(machine.tier(), Tier::Maximum);
        assert_eq!(machine.acceleration(), Acceleration::Cuda);
        assert_eq!(machine.gpu_name(), Some("NVIDIA GeForce RTX 4090"));
        assert_eq!(machine.packaging.manager, PackageManager::AptGet);
        assert_eq!(machine.services, ServiceManager::Systemd);
        assert_eq!(machine.ram_mb, Some(64263));
        assert_eq!(machine.cores, Some(8));
        assert_eq!(machine.disk_mb, Some(510580));
        assert!(captured.has("tmux"));
        assert!(!captured.has("ffmpeg"));
    }

    // AYEAYE-62 — a container is the case AYEAYE-61 left open, and the one this
    // ticket owes: systemctl is installed and there is no user session behind
    // it, so there is no service manager here and the answer must say so rather
    // than reporting systemd because the binary exists.
    #[test]
    fn a_systemctl_with_no_user_session_is_not_a_service_manager() {
        let container = Fake::default()
            .holds(
                "/etc/os-release",
                include_str!("../../../tests/fixtures/os-release/debian-12"),
            )
            .says("uname -s", "Linux\n")
            .says("uname -m", "x86_64\n")
            .on_path("systemctl");
        // `show-environment` is deliberately not answered, so it fails.
        let machine = capture(&container, "/tmp/models").machine();
        assert_eq!(machine.services, ServiceManager::None);
    }

    // AYEAYE-62 — **no probe may change anything**. Asserted on the argv that
    // actually reached the runner rather than by reading the source, because
    // that is the only evidence that survives an edit. AYEAYE-61 disabled this
    // machine's real service by running one service-manager verb by hand; the
    // capture runs before setup has decided anything at all, and a single
    // mutating verb in here would do it to every machine ayeaye is installed on.
    #[test]
    fn nothing_the_capture_runs_changes_anything() {
        let fake = linux_with_nvidia();
        capture(&fake, "/home/tester/.local/state/ayeaye");
        let ran = fake.ran.borrow();
        assert!(!ran.is_empty(), "it should have asked something");
        for argv in ran.iter() {
            let said = argv.join(" ");
            assert!(
                !argv.iter().any(|word| matches!(
                    word.as_str(),
                    "enable"
                        | "disable"
                        | "start"
                        | "stop"
                        | "restart"
                        | "install"
                        | "bootstrap"
                        | "bootout"
                        | "kickstart"
                        | "daemon-reload"
                        | "remove"
                )),
                "a probe with an effect: {said}"
            );
            assert!(
                !said.starts_with("systemctl") || said == "systemctl --user show-environment",
                "the only systemctl a probe may run is show-environment: {said}"
            );
            assert!(
                !said.starts_with("launchctl"),
                "launchctl has no read a probe needs: {said}"
            );
        }
    }

    // AYEAYE-62 — a command that fails said nothing worth parsing. nvidia-smi
    // on a machine with no driver exits non-zero and prints an explanation, and
    // reading that explanation as a list of cards is how a machine acquires an
    // imaginary one.
    #[test]
    fn a_command_that_failed_is_not_an_answer() {
        let mut fake = Fake::default().says("uname -s", "Linux\n");
        fake.commands.insert(
            "nvidia-smi --query-gpu=name,memory.total --format=csv,noheader".to_string(),
            Outcome {
                ok: false,
                output: "NVIDIA-SMI has failed because it couldn't communicate with the driver\n"
                    .to_string(),
            },
        );
        let machine = capture(&fake, "/tmp/models").machine();
        assert_eq!(machine.acceleration(), Acceleration::Cpu);
        assert_eq!(machine.gpu_name(), None);
    }

    // AYEAYE-62 — whitespace is not an answer either. `Probes` is built on the
    // distinction between nothing to read and an empty answer, and a command
    // that printed one newline has told us nothing.
    #[test]
    fn a_command_that_printed_only_whitespace_is_not_an_answer() {
        let fake = Fake::default().says("uname -s", " \n").says("uname -m", "");
        let captured = capture(&fake, "/tmp/models");
        assert_eq!(captured.probes().uname_s, None);
        assert_eq!(captured.probes().uname_m, None);
    }

    // AYEAYE-62 — free space is measured where the weights would land, and on a
    // first run that directory does not exist yet, so the nearest ancestor that
    // does is what df is asked about. Measuring the current directory instead
    // would answer about a different filesystem on any machine with a separate
    // home.
    #[test]
    fn free_space_is_measured_at_the_nearest_directory_that_exists() {
        let fake = Fake::default().directory("/home/tester").says(
            "df -Pk /home/tester",
            include_str!("../../../tests/fixtures/df/roomy"),
        );
        let captured = capture(&fake, "/home/tester/.local/state/ayeaye/models/openai");
        assert_eq!(
            ayeaye_core::machine::size::disk_mb(&captured.probes()),
            Some(510580)
        );
        assert!(
            fake.ran
                .borrow()
                .iter()
                .any(|argv| argv == &["df", "-Pk", "/home/tester"]),
            "df should have been asked about the nearest existing ancestor"
        );
    }

    // AYEAYE-62 — the seam, on a machine that has one. A Linux with a user bus
    // is systemd, addressed as whoever is calling: no uid is asked for, because
    // systemd's user manager has no domain to name.
    #[test]
    fn a_linux_with_a_user_bus_is_a_systemd_session() {
        let fake = Fake::default()
            .says("uname -s", "Linux\n")
            .says("systemctl --user show-environment", "PATH=/usr/bin\n")
            .on_path("systemctl");
        let addressed = session(&fake).expect("systemd is here");
        assert_eq!(addressed.manager, Manager::Systemd);
        assert_eq!(addressed.uid, None);
        assert!(
            !fake.ran.borrow().iter().any(|argv| argv[0] == "id"),
            "systemd has no domain to address, so nothing should ask for a uid"
        );
    }

    // AYEAYE-62 — and a Mac, whose every command addresses `gui/<uid>`. `id -u`
    // is where the shell has always read that from.
    #[test]
    fn a_mac_is_a_launchd_session_addressed_by_its_uid() {
        let fake = Fake::default()
            .says("uname -s", "Darwin\n")
            .says("id -u", "501\n")
            .on_path("launchctl");
        let addressed = session(&fake).expect("launchd is here");
        assert_eq!(addressed.manager, Manager::Launchd);
        assert_eq!(addressed.uid.as_deref(), Some("501"));
        assert!(
            !fake.ran.borrow().iter().any(|argv| argv[0] == "systemctl"),
            "a Mac has no systemctl, so nothing should start one to be told so"
        );
    }

    // AYEAYE-61, kept — a Mac whose `id` will not answer. launchctl is still
    // here, so there *is* a session; what is missing is the uid, and without one
    // the target would be the malformed `gui//label`. No uid is better than a
    // wrong one, and the core refuses on it rather than addressing nothing.
    #[test]
    fn a_mac_with_no_answer_from_id_carries_no_uid() {
        let silent = Fake::default()
            .says("uname -s", "Darwin\n")
            .on_path("launchctl");
        let unaddressable = session(&silent).expect("launchctl is still here");
        assert_eq!(unaddressable.uid, None);

        let blank = Fake::default()
            .says("uname -s", "Darwin\n")
            .says("id -u", " \n")
            .on_path("launchctl");
        assert_eq!(session(&blank).expect("still here").uid, None);
    }

    // AYEAYE-62 — the answer AYEAYE-61 could not give, reached end to end. This
    // machine has the systemctl binary and no user session behind it, so there
    // is no session to address, and the verb above is expected to say so rather
    // than to run a command that cannot work.
    #[test]
    fn a_container_has_no_session_to_address_at_all() {
        let container = Fake::default()
            .says("uname -s", "Linux\n")
            .on_path("systemctl");
        assert_eq!(session(&container), None);

        let stripped = Fake::default().says("uname -s", "Linux\n");
        assert_eq!(session(&stripped), None);

        let mac_without_launchctl = Fake::default().says("uname -s", "Darwin\n");
        assert_eq!(session(&mac_without_launchctl), None);
    }

    // AYEAYE-62 — a container is judged on its share and not on the host it is
    // running on, and every one of the four marks it leaves is read. None is
    // reliable alone, which is why only their disjunction is a judgement worth
    // making — so each is checked on its own here.
    #[test]
    fn a_container_is_detected_by_any_of_its_marks_and_judged_on_its_share() {
        let host = |mark: &str| {
            Fake::default()
                .holds(
                    "/etc/os-release",
                    include_str!("../../../tests/fixtures/os-release/debian-12"),
                )
                .holds(
                    "/proc/meminfo",
                    include_str!("../../../tests/fixtures/meminfo/64gb"),
                )
                .holds(mark, "")
                .holds(
                    "/sys/fs/cgroup/memory.max",
                    include_str!("../../../tests/fixtures/cgroup/memory-max-1g"),
                )
                .holds(
                    "/sys/fs/cgroup/cpu.max",
                    include_str!("../../../tests/fixtures/cgroup/cpu-max-two-cores"),
                )
                .says("uname -s", "Linux\n")
                .says(
                    "env LC_ALL=C lscpu",
                    include_str!("../../../tests/fixtures/lscpu/x86_64-8core"),
                )
        };
        for mark in [
            "/.dockerenv",
            "/run/.containerenv",
            "/run/systemd/container",
        ] {
            let machine = capture(&host(mark), "/tmp/models").machine();
            assert_eq!(machine.cores, Some(2), "{mark}: lscpu sees eight");
            assert_eq!(machine.ram_mb, Some(1024), "{mark}: meminfo sees 64 GB");
        }

        // The fourth mark is an environment variable and nothing on disk, which
        // is how podman and systemd-nspawn say it.
        let mut by_variable = host("/nowhere");
        by_variable
            .environment
            .insert("container".to_string(), "podman".to_string());
        let machine = capture(&by_variable, "/tmp/models").machine();
        assert_eq!(machine.ram_mb, Some(1024));

        // A machine under a limit is judged on that limit whether or not it
        // left a container mark: a systemd slice bounds an ordinary process just
        // as a container runtime bounds one, and the marks say what this is, not
        // what it may use.
        let unmarked = capture(&host("/nowhere"), "/tmp/models").machine();
        assert_eq!(unmarked.cores, Some(2));

        // And with no limit to read anywhere, the whole machine is the answer.
        let unlimited = Fake::default()
            .holds(
                "/proc/meminfo",
                include_str!("../../../tests/fixtures/meminfo/64gb"),
            )
            .says("uname -s", "Linux\n")
            .says(
                "env LC_ALL=C lscpu",
                include_str!("../../../tests/fixtures/lscpu/x86_64-8core"),
            );
        let whole = capture(&unlimited, "/tmp/models").machine();
        assert_eq!(whole.cores, Some(8));
        assert_eq!(whole.ram_mb, Some(64263));
    }

    // AYEAYE-62 — the limit that is really in force is the one at the path
    // `/proc/self/cgroup` names, and the fixed path is a fallback rather than
    // the answer: under a shared cgroup namespace `/sys/fs/cgroup/memory.max` is
    // the *host's* root and says `max`, while the derived path carries the limit
    // this process is actually under.
    #[test]
    fn a_containers_limit_is_read_where_its_own_cgroup_says_it_is() {
        let nested = Fake::default()
            .holds("/proc/self/cgroup", "0::/payload/leaf\n")
            .holds("/sys/fs/cgroup/memory.max", "max\n")
            .holds(
                "/sys/fs/cgroup/payload/leaf/memory.max",
                include_str!("../../../tests/fixtures/cgroup/memory-max-1g"),
            )
            .holds("/.dockerenv", "")
            .holds(
                "/proc/meminfo",
                include_str!("../../../tests/fixtures/meminfo/64gb"),
            )
            .says("uname -s", "Linux\n");
        assert_eq!(capture(&nested, "/tmp/models").machine().ram_mb, Some(1024));
    }

    // AYEAYE-62 — a name nobody captured would answer `false` for ever, which
    // is a silent wrong answer rather than a loud one.
    #[test]
    #[should_panic(expected = "add it to LOOKED_FOR")]
    fn asking_about_a_command_nobody_looked_for_is_a_bug() {
        Captured::default().has("kubectl");
    }

    // AYEAYE-62 — the list is what the platform layer and the health checks
    // read, and a package manager missing from it would be a machine that
    // cannot install anything for a reason nothing states.
    #[test]
    fn every_command_a_decision_is_made_from_is_looked_for() {
        for name in [
            "apt-get",
            "dnf",
            "yum",
            "pacman",
            "zypper",
            "systemctl",
            "launchctl",
            "tmux",
        ] {
            assert!(LOOKED_FOR.contains(&name), "{name} is not looked for");
        }
    }
}
