//! What this machine is, and who to ask to change it.
//!
//! A port of `lib/platform.sh`'s judgement, with its effects left behind. The
//! shell reads files and runs commands; this takes the text they produced and
//! says what it means.
//!
//! Every answer is a word, never a sentence, and `unknown` is a first-class
//! answer everywhere: this module would rather say it cannot identify a machine
//! than guess wrong about one.

/// The kernel this machine runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    Linux,
    Macos,
    Unknown,
}

/// The family whose package manager and conventions this machine follows.
///
/// Deliberately generous: a derivative nobody here has heard of still reaches
/// the right package manager through `ID_LIKE`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    Debian,
    Fedora,
    Arch,
    Suse,
    Macos,
    Unknown,
}

impl Os {
    /// The lower-case word a caller matches on.
    pub fn as_str(self) -> &'static str {
        match self {
            Os::Linux => "linux",
            Os::Macos => "macos",
            Os::Unknown => "unknown",
        }
    }
}

impl Family {
    /// The lower-case word a caller matches on.
    pub fn as_str(self) -> &'static str {
        match self {
            Family::Debian => "debian",
            Family::Fedora => "fedora",
            Family::Arch => "arch",
            Family::Suse => "suse",
            Family::Macos => "macos",
            Family::Unknown => "unknown",
        }
    }
}

/// Everything this layer can say about the machine's identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Platform {
    /// The kernel.
    pub os: Os,
    /// The family, which is what decides who to ask to install something.
    pub family: Family,
    /// The distribution's own id: `ubuntu`, `raspbian`, `macos`, `unknown`.
    pub id: String,
    /// `VERSION_ID`, or the macOS product version. Empty on a rolling release.
    pub version: String,
    /// The human name: `Ubuntu 24.04.1 LTS`, `macOS 15.1`.
    pub pretty: String,
    /// `x86_64`, `arm64`, or the raw `uname -m` when this layer has no opinion.
    pub arch: String,
    /// True on an image-based system, where the package manager is right there
    /// and installing with it does not survive a reboot.
    pub immutable: bool,
}

/// Work out what this machine is from the text its probes produced.
///
/// `os_release` is the contents of the first readable `/etc/os-release`-shaped
/// file, `uname_s` and `uname_m` are what `uname -s` and `uname -m` printed, and
/// `sw_vers` is the whole of `sw_vers` with no arguments. `None` means the probe
/// found nothing to read — which is not the same as an empty answer, and is why
/// these are options rather than empty strings.
pub fn identify(
    os_release: Option<&str>,
    uname_s: Option<&str>,
    uname_m: Option<&str>,
    sw_vers: Option<&str>,
) -> Platform {
    let arch = normalise_arch(uname_m.unwrap_or("").trim());
    let kernel = uname_s.unwrap_or("").trim();

    // The kernel wins over any os-release lying around: a nix or a linuxbrew
    // install can leave one on a Mac.
    if kernel == "Darwin" {
        return macos(arch, sw_vers);
    }
    match os_release {
        Some(text) => linux(arch, text),
        None if kernel == "Linux" => Platform {
            os: Os::Linux,
            family: Family::Unknown,
            id: "unknown".to_string(),
            version: String::new(),
            pretty: "Linux".to_string(),
            arch,
            immutable: false,
        },
        None => Platform {
            os: Os::Unknown,
            family: Family::Unknown,
            id: "unknown".to_string(),
            version: String::new(),
            pretty: "unknown".to_string(),
            arch,
            immutable: false,
        },
    }
}

fn macos(arch: String, sw_vers: Option<&str>) -> Platform {
    // The last assignment wins, as it would in the shell's own read loop.
    let version = sw_vers
        .and_then(|text| {
            text.lines()
                .filter_map(|line| line.strip_prefix("ProductVersion:"))
                .next_back()
        })
        .map(|value| value.trim().to_string())
        .unwrap_or_default();
    let pretty = if version.is_empty() {
        "macOS".to_string()
    } else {
        format!("macOS {version}")
    };
    Platform {
        os: Os::Macos,
        family: Family::Macos,
        id: "macos".to_string(),
        version,
        pretty,
        arch,
        immutable: false,
    }
}

fn linux(arch: String, os_release: &str) -> Platform {
    let id = value(os_release, "ID")
        .map(|v| lower(&v))
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "unknown".to_string());
    let version = value(os_release, "VERSION_ID").unwrap_or_default();
    let pretty = value(os_release, "PRETTY_NAME")
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| id.clone());
    let family = if id == "unknown" {
        Family::Unknown
    } else {
        family_from_os_release(os_release, &id)
    };
    let variant = value(os_release, "VARIANT_ID")
        .map(|v| lower(&v))
        .unwrap_or_default();
    Platform {
        os: Os::Linux,
        family,
        immutable: is_immutable(&id, &variant),
        id,
        version,
        pretty,
        arch,
    }
}

/// `ID` first, then each `ID_LIKE` token, through the one id table.
///
/// The tokens go through the same lookup rather than a table of their own, which
/// is what makes a derivative of a derivative work: Trisquel says
/// `ID_LIKE=ubuntu`, and ubuntu is only debian by the same rule.
fn family_from_os_release(os_release: &str, id: &str) -> Family {
    if let Some(family) = family_of_id(id) {
        return family;
    }
    let like = value(os_release, "ID_LIKE")
        .map(|v| lower(&v))
        .unwrap_or_default();
    like.split_whitespace()
        .find_map(family_of_id)
        .unwrap_or(Family::Unknown)
}

/// The id table. Adding a name here is the cheapest change in this file, and
/// getting one wrong is the most expensive: it sends the shell off to run a
/// package manager the machine does not have.
fn family_of_id(id: &str) -> Option<Family> {
    match id {
        "debian" | "ubuntu" | "raspbian" | "raspberry-pi-os" | "linuxmint" | "mint" | "pop"
        | "elementary" | "neon" | "zorin" | "devuan" | "kali" | "parrot" | "deepin" | "mx"
        | "pureos" | "tuxedo" | "linuxlite" | "peppermint" => Some(Family::Debian),
        "fedora" | "rhel" | "centos" | "rocky" | "almalinux" | "ol" | "oracle" | "amzn"
        | "scientific" | "nobara" | "bazzite" | "silverblue" | "qubes" | "eurolinux" | "circle"
        | "navylinux" => Some(Family::Fedora),
        "arch" | "archarm" | "arch32" | "manjaro" | "manjaro-arm" | "endeavouros" | "garuda"
        | "cachyos" | "artix" | "arcolinux" | "steamos" | "blendos" | "parabola" => {
            Some(Family::Arch)
        }
        "opensuse"
        | "opensuse-leap"
        | "opensuse-tumbleweed"
        | "opensuse-slowroll"
        | "opensuse-microos"
        | "opensuse-aeon"
        | "opensuse-kalpa"
        | "suse"
        | "sles"
        | "sled"
        | "sle-micro"
        | "sle_hpc"
        | "tumbleweed"
        | "leap"
        | "geckolinux" => Some(Family::Suse),
        "macos" | "darwin" => Some(Family::Macos),
        _ => None,
    }
}

/// An image-based system: the package manager is present and installing with it
/// does not survive a reboot. Recognising the distribution and refusing to act
/// on it is the only honest combination — `unknown` would lose the name the
/// manual instructions need to print.
fn is_immutable(id: &str, variant: &str) -> bool {
    matches!(
        id,
        "opensuse-microos"
            | "opensuse-aeon"
            | "opensuse-kalpa"
            | "sle-micro"
            | "steamos"
            | "qubes"
            | "bazzite"
            | "silverblue"
            | "fedora-coreos"
            | "fedora-iot"
    ) || matches!(
        // Silverblue and its siblings arrive as plain ID=fedora; only the
        // variant gives them away.
        variant,
        "silverblue"
            | "kinoite"
            | "sericea"
            | "onyx"
            | "iot"
            | "coreos"
            | "atomic-host"
            | "sway-atomic"
            | "budgie-atomic"
    )
}

/// Normalise what `uname -m` printed.
///
/// Deliberately not `armv8*`: `armv8l` is a 32-bit userland on a 64-bit kernel,
/// and calling it arm64 hands whoever picks a binary by architecture the wrong
/// one. An architecture this layer has no opinion about is passed through rather
/// than flattened to `unknown`, which would lose the only name there is.
fn normalise_arch(machine: &str) -> String {
    match machine {
        "x86_64" | "amd64" | "x64" => "x86_64".to_string(),
        "arm64" | "aarch64" => "arm64".to_string(),
        "" => "unknown".to_string(),
        other => other.to_string(),
    }
}

/// One key's value out of an os-release file. The last assignment wins, as it
/// would in the shell.
///
/// Parsed, not sourced. `. /etc/os-release` is the documented way to read the
/// file and it is also arbitrary code execution from a path a caller is
/// explicitly allowed to have pointed somewhere else.
fn value(text: &str, key: &str) -> Option<String> {
    let mut found = None;
    for raw in text.lines() {
        let line = raw.trim_end_matches('\r').trim();
        if line.starts_with('#') {
            continue;
        }
        let Some(rest) = line.strip_prefix(key) else {
            continue;
        };
        let Some(rest) = rest.strip_prefix('=') else {
            continue;
        };
        found = Some(unquote(rest.trim()));
    }
    found
}

/// A value ends where the shell would say it ends: a quoted one at its closing
/// quote, whatever follows it, and an unquoted one at a comment.
///
/// The order is load-bearing. Stripping the comment first would leave the quotes
/// on for `ID="debian" # like this`, which demotes a fully supported machine to
/// unknown.
fn unquote(value: &str) -> String {
    for quote in ['"', '\''] {
        if let Some(rest) = value.strip_prefix(quote) {
            return match rest.find(quote) {
                Some(end) => rest[..end].to_string(),
                None => rest.to_string(),
            };
        }
    }
    match value.find(" #") {
        Some(hash) => value[..hash].trim().to_string(),
        None => value.to_string(),
    }
}

// ------------------------------------------------- who to ask to change it

/// The tool that installs software on this machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackageManager {
    AptGet,
    Dnf,
    Yum,
    Pacman,
    Zypper,
    Brew,
    None,
}

impl PackageManager {
    /// The tool's own name, which is also the word a caller matches on.
    ///
    /// A name, not an invocation. `brew` is the right name and the wrong command
    /// on a machine where Homebrew is in its prefix and not yet on `PATH`; the
    /// command to run is [`Homebrew::invocation`].
    pub fn as_str(self) -> &'static str {
        match self {
            PackageManager::AptGet => "apt-get",
            PackageManager::Dnf => "dnf",
            PackageManager::Yum => "yum",
            PackageManager::Pacman => "pacman",
            PackageManager::Zypper => "zypper",
            PackageManager::Brew => "brew",
            PackageManager::None => "none",
        }
    }
}

/// What may be installed here, and what may only be asked.
///
/// The two are not the same on an image-based system: dnf or zypper is right
/// there, installing with it does not survive a reboot, and asking it what is
/// already present is a read that makes the manual instructions much better.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Packaging {
    /// What the shell may install with. `None` sends it down the manual path.
    pub manager: PackageManager,
    /// The tool that is really there, whether or not installing with it lasts.
    pub tool: PackageManager,
}

/// The service manager this project can install a *user* service into.
///
/// Narrower than it looks, on purpose: a machine plainly running systemd as pid
/// 1 still answers `None` under sudo or before the session is up, because that
/// is the honest answer to "can I install a user service here", which is the
/// only question this project asks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceManager {
    Systemd,
    Launchd,
    None,
}

impl ServiceManager {
    /// The lower-case word a caller matches on.
    pub fn as_str(self) -> &'static str {
        match self {
            ServiceManager::Systemd => "systemd",
            ServiceManager::Launchd => "launchd",
            ServiceManager::None => "none",
        }
    }
}

/// Which package manager this machine has, and which one it may use.
///
/// A family names the candidates and `available` — the commands the shell found
/// on `PATH` — decides which is really there. Claiming apt-get on a debian
/// container that has had it removed would send the shell off to run a command
/// that does not exist. Homebrew is passed separately from `available` because
/// one installed in this same session is in its prefix and not yet on `PATH`.
pub fn packaging(
    family: Family,
    immutable: bool,
    available: &[&str],
    brew: Option<&Homebrew>,
) -> Packaging {
    let candidates: &[PackageManager] = match family {
        Family::Debian => &[PackageManager::AptGet],
        Family::Fedora => &[PackageManager::Dnf, PackageManager::Yum],
        Family::Arch => &[PackageManager::Pacman],
        Family::Suse => &[PackageManager::Zypper],
        Family::Macos => &[PackageManager::Brew],
        Family::Unknown => &[],
    };
    let tool = candidates
        .iter()
        .copied()
        .find(|candidate| match candidate {
            PackageManager::Brew => brew.is_some(),
            other => available.contains(&other.as_str()),
        })
        .unwrap_or(PackageManager::None);
    Packaging {
        manager: if immutable {
            PackageManager::None
        } else {
            tool
        },
        tool,
    }
}

/// Which service manager a user service can be installed into.
///
/// `user_bus_responds` is what `systemctl --user show-environment` answered: a
/// container has the binary and no user session, and enabling a unit there fails
/// after setup has promised it would work.
pub fn service_manager(os: Os, available: &[&str], user_bus_responds: bool) -> ServiceManager {
    if os == Os::Macos {
        return if available.contains(&"launchctl") {
            ServiceManager::Launchd
        } else {
            ServiceManager::None
        };
    }
    if available.contains(&"systemctl") && user_bus_responds {
        ServiceManager::Systemd
    } else {
        ServiceManager::None
    }
}

/// A Homebrew that is really installed, and how to invoke it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Homebrew {
    /// The prefix it is installed under: `/opt/homebrew`, `/usr/local`, …
    pub prefix: String,
    /// What a generated command line must actually say to run it.
    pub invocation: String,
}

/// Find Homebrew, and work out what a generated command line has to say.
///
/// Homebrew is detected on its own and not as a property of macOS: a Mac without
/// it is an ordinary machine, and Homebrew on Linux exists too.
///
/// `on_path` is what `command -v brew` printed, and `in_prefix` is the first
/// standard prefix whose `bin/brew` is executable. The distinction is the whole
/// point of this function. On `PATH`, a generated command may plainly say
/// `brew`. Found only in its prefix — which is where a Homebrew installed in
/// this same session lives — a command saying `brew` would die with
/// command-not-found, so it has to say where the binary is.
///
/// The prefix is path arithmetic and never `brew --prefix`, because asking would
/// mean running it. A `command -v brew` that answered with something which is
/// not a path at all — a shell function or an alias by that name — is not a
/// Homebrew.
pub fn homebrew(on_path: Option<&str>, in_prefix: Option<&str>) -> Option<Homebrew> {
    if let Some(prefix) = on_path.and_then(brew_prefix_of) {
        return Some(Homebrew {
            prefix,
            invocation: "brew".to_string(),
        });
    }
    let prefix = in_prefix?;
    Some(Homebrew {
        prefix: prefix.to_string(),
        invocation: format!("{prefix}/bin/brew"),
    })
}

/// The prefix a `brew` binary at this path is installed under.
fn brew_prefix_of(brew_path: &str) -> Option<String> {
    if !brew_path.starts_with('/') {
        return None;
    }
    let directory = brew_path.rsplit_once('/').map(|(head, _)| head)?;
    let prefix = directory.strip_suffix("/bin").unwrap_or(directory);
    Some(if prefix.is_empty() {
        "/".to_string()
    } else {
        prefix.to_string()
    })
}

/// Whether this layer recognised the machine well enough to act on its own.
///
/// The one question asked before anything is installed without being told how.
/// A family it cannot name, or a package manager it has nothing to say to, both
/// mean the manual path — which is the only one that works.
pub fn is_known(platform: &Platform, packaging: &Packaging) -> bool {
    platform.family != Family::Unknown && packaging.manager != PackageManager::None
}

/// The whole verdict as one line, because it is one line of output.
pub fn summary(platform: &Platform, packaging: &Packaging, services: ServiceManager) -> String {
    let name = if platform.pretty.is_empty() {
        "unknown"
    } else {
        &platform.pretty
    };
    let note = if platform.immutable {
        ", image-based"
    } else {
        ""
    };
    format!(
        "{name} ({}) {}, packages: {}, services: {}{note}",
        platform.family.as_str(),
        platform.arch,
        packaging.manager.as_str(),
        services.as_str(),
    )
}

/// ASCII lower case. Deepin has shipped `ID=Deepin`, and os-release ids are
/// compared, not displayed.
fn lower(text: &str) -> String {
    text.to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::{
        Family, Homebrew, Os, PackageManager, ServiceManager, homebrew, identify, is_known,
        packaging, service_manager, summary,
    };
    use crate::machine::fixture;

    /// A Linux machine described by one os-release fixture and nothing else.
    fn distro(os_release: &str) -> super::Platform {
        identify(Some(os_release), Some("Linux"), Some("x86_64"), None)
    }

    // AYEAYE-60 — the sixteen captured os-release files, each landing in the
    // family `tests/cases/platform_detect_test.sh` pins for it. The port is only
    // a port if it agrees with the shell over the same corpus.
    #[test]
    fn every_captured_distribution_lands_in_the_family_the_shell_puts_it_in() {
        let corpus: &[(&str, Family)] = &[
            (fixture!("os-release/debian-12"), Family::Debian),
            (fixture!("os-release/ubuntu-24.04"), Family::Debian),
            (fixture!("os-release/raspbian-12"), Family::Debian),
            (fixture!("os-release/linuxmint-21"), Family::Debian),
            // Deliberately absent from the id table: only ID_LIKE=ubuntu can
            // place it, and ubuntu is itself only debian by the same rule.
            (fixture!("os-release/trisquel-11"), Family::Debian),
            (fixture!("os-release/fedora-40"), Family::Fedora),
            (fixture!("os-release/fedora-silverblue-40"), Family::Fedora),
            (fixture!("os-release/rocky-9"), Family::Fedora),
            (fixture!("os-release/arch"), Family::Arch),
            (fixture!("os-release/manjaro"), Family::Arch),
            (fixture!("os-release/steamos-3.5"), Family::Arch),
            (fixture!("os-release/opensuse-leap-15.6"), Family::Suse),
            (fixture!("os-release/opensuse-tumbleweed"), Family::Suse),
            (fixture!("os-release/alpine-3.20"), Family::Unknown),
            (fixture!("os-release/void"), Family::Unknown),
            (fixture!("os-release/nixos-24.05"), Family::Unknown),
        ];
        let got: Vec<Family> = corpus.iter().map(|(text, _)| distro(text).family).collect();
        let want: Vec<Family> = corpus.iter().map(|(_, family)| *family).collect();
        assert_eq!(got, want);
    }

    // AYEAYE-60
    #[test]
    fn a_distribution_nobody_knows_still_reports_its_own_identity() {
        let alpine = distro(fixture!("os-release/alpine-3.20"));
        assert_eq!(alpine.family, Family::Unknown);
        assert_eq!(alpine.id, "alpine");
        assert_eq!(alpine.version, "3.20.3");
        assert_eq!(alpine.pretty, "Alpine Linux v3.20");
        assert_eq!(alpine.os, Os::Linux);
    }

    // AYEAYE-60
    #[test]
    fn an_id_like_list_is_searched_past_its_first_token() {
        let made_up = distro("ID=speculative\nID_LIKE=\"madeup alsomadeup debian\"\n");
        assert_eq!(made_up.family, Family::Debian);
    }

    // AYEAYE-60 — a quoted value ends at its closing quote and an unquoted one
    // at a comment, in that order. Doing the comment first leaves the quotes on
    // for `ID="debian" # like this`, which demotes a supported machine.
    #[test]
    fn a_value_ends_where_the_shell_would_say_it_ends() {
        let commented = distro(concat!(
            "ID=\"debian\" # the only one that matters\n",
            "VERSION_ID=\"12\" # stable\n",
            "PRETTY_NAME=\"Debian GNU/Linux 12\" # codename bookworm\n",
        ));
        assert_eq!(commented.id, "debian");
        assert_eq!(commented.family, Family::Debian);
        assert_eq!(commented.version, "12");
        assert_eq!(commented.pretty, "Debian GNU/Linux 12");

        assert_eq!(distro("ID=arch # rolling\n").id, "arch");
        assert_eq!(
            distro("ID=debian\nPRETTY_NAME=\"Debian #1\"\n").pretty,
            "Debian #1",
            "inside the quotes a hash is just a hash"
        );
        assert_eq!(
            distro("ID=\"fedora\"   \nVERSION_ID=40  \n").version,
            "40",
            "trailing blanks must not defeat the quote stripping"
        );
    }

    // AYEAYE-60
    #[test]
    fn the_shapes_that_are_not_values_are_not_read_as_values() {
        // The real tumbleweed file carries a commented VERSION and two commented
        // CPE_NAMEs above the live one.
        let tumbleweed = distro(fixture!("os-release/opensuse-tumbleweed"));
        assert_eq!(tumbleweed.id, "opensuse-tumbleweed");
        assert_eq!(tumbleweed.version, "20260724");
        assert_eq!(tumbleweed.pretty, "openSUSE Tumbleweed");

        // Deepin has shipped ID=Deepin. An id is compared, not displayed.
        let deepin = distro("ID=Deepin\nPRETTY_NAME=\"deepin 23\"\n");
        assert_eq!(deepin.id, "deepin");
        assert_eq!(deepin.family, Family::Debian);

        let crlf = distro("ID=debian\r\nVERSION_ID=\"12\"\r\n");
        assert_eq!((crlf.id.as_str(), crlf.version.as_str()), ("debian", "12"));

        let anonymous = distro("PRETTY_NAME=\"Something Else\"\nHOME_URL=\"https://x.invalid/\"\n");
        assert_eq!(anonymous.id, "unknown");
        assert_eq!(anonymous.family, Family::Unknown);

        assert_eq!(
            distro("ID=debian\n").pretty,
            "debian",
            "there has to be something to print"
        );
    }

    // AYEAYE-60 — the kernel is the stronger signal: nix on macOS leaves a
    // linux-looking os-release lying around, and a Mac is still a Mac.
    #[test]
    fn a_darwin_kernel_beats_any_os_release_lying_around() {
        // The uname/* fixtures are whole `uname -a` lines; this seam takes
        // `uname -s` and `uname -m`, which the shell's own tests stub with
        // literals for the same reason.
        let mac = identify(
            Some(fixture!("os-release/nixos-24.05")),
            Some("Darwin"),
            Some("arm64"),
            Some(fixture!("sw_vers/macos-15.1")),
        );
        assert_eq!(mac.os, Os::Macos);
        assert_eq!(mac.family, Family::Macos);
        assert_eq!(mac.id, "macos");
        assert_eq!(mac.version, "15.1");
        assert_eq!(mac.pretty, "macOS 15.1");

        let older = identify(
            None,
            Some("Darwin"),
            Some("x86_64"),
            Some(fixture!("sw_vers/macos-13.6")),
        );
        assert_eq!(older.version, "13.6");
        assert_eq!(older.arch, "x86_64");

        let bare = identify(None, Some("Darwin"), Some("arm64"), None);
        assert_eq!(
            bare.family,
            Family::Macos,
            "the kernel name alone is enough"
        );
        assert_eq!(bare.version, "");
        assert_eq!(bare.pretty, "macOS");
    }

    // AYEAYE-60
    #[test]
    fn a_machine_with_neither_kernel_nor_os_release_is_unknown_everything() {
        let nothing = identify(None, None, None, None);
        assert_eq!(nothing.os, Os::Unknown);
        assert_eq!(nothing.family, Family::Unknown);
        assert_eq!(nothing.arch, "unknown");

        let kernel_only = identify(None, Some("Linux"), Some("x86_64"), None);
        assert_eq!(kernel_only.os, Os::Linux);
        assert_eq!(kernel_only.family, Family::Unknown);

        let os_release_only = identify(Some(fixture!("os-release/fedora-40")), None, None, None);
        assert_eq!(
            os_release_only.os,
            Os::Linux,
            "a readable os-release is proof enough of linux"
        );
        assert_eq!(os_release_only.family, Family::Fedora);
    }

    // AYEAYE-60
    #[test]
    fn architecture_is_normalised_without_being_flattened() {
        let arch_of = |machine| identify(None, Some("Linux"), Some(machine), None).arch;
        assert_eq!(arch_of("x86_64"), "x86_64");
        assert_eq!(
            arch_of("amd64"),
            "x86_64",
            "the same machine by another name"
        );
        assert_eq!(arch_of("x64"), "x86_64");
        assert_eq!(arch_of("aarch64"), "arm64", "linux says aarch64");
        assert_eq!(arch_of("arm64"), "arm64", "apple silicon says arm64");
        assert_eq!(
            arch_of("riscv64"),
            "riscv64",
            "an architecture we have no opinion about beats calling it unknown"
        );
        assert_eq!(
            arch_of("armv8l"),
            "armv8l",
            "a 32-bit userland on a 64-bit kernel is not arm64"
        );
    }

    // AYEAYE-60 — an image-based system is recognised and refused, not made
    // unknown: the manual instructions still need its name.
    #[test]
    fn an_image_based_system_is_named_and_marked() {
        let silverblue = distro(fixture!("os-release/fedora-silverblue-40"));
        assert!(
            silverblue.immutable,
            "only VARIANT_ID gives Silverblue away"
        );
        assert_eq!(silverblue.id, "fedora");

        let steamos = distro(fixture!("os-release/steamos-3.5"));
        assert!(steamos.immutable);
        assert_eq!(steamos.family, Family::Arch);

        assert!(!distro(fixture!("os-release/fedora-40")).immutable);
        assert!(!distro(fixture!("os-release/debian-12")).immutable);
    }

    // AYEAYE-60 — a family names the candidates and PATH decides. The
    // fixtures under tests/fixtures/pkg-commands pin which tool each family's
    // generated command line is built around.
    #[test]
    fn each_family_reaches_for_the_tool_its_pkg_command_fixture_names() {
        let corpus: &[(Family, &[&str], &str)] = &[
            (Family::Debian, &["apt-get"], "apt-get"),
            (Family::Fedora, &["dnf", "yum"], "dnf"),
            (Family::Fedora, &["yum"], "yum"),
            (Family::Arch, &["pacman"], "pacman"),
            (Family::Suse, &["zypper"], "zypper"),
            (Family::Unknown, &["apt-get", "dnf", "pacman"], "none"),
        ];
        for (family, available, want) in corpus {
            let got = packaging(*family, false, available, None);
            assert_eq!(got.manager.as_str(), *want, "{family:?} with {available:?}");
        }
    }

    // AYEAYE-60
    #[test]
    fn a_family_whose_tool_is_not_installed_says_none_rather_than_naming_it() {
        // A debian container that has had apt-get removed is a real machine,
        // and running a command that is not there is not a plan.
        let stripped = packaging(Family::Debian, false, &["dpkg"], None);
        assert_eq!(stripped.manager, PackageManager::None);
        assert_eq!(stripped.tool, PackageManager::None);
    }

    // AYEAYE-60 — Homebrew is not a property of macOS: it is in its prefix
    // before it is on PATH, and it exists on Linux too.
    #[test]
    fn homebrew_counts_when_it_is_installed_rather_than_when_it_is_on_the_path() {
        let on_path = homebrew(Some("/opt/homebrew/bin/brew"), None).unwrap();
        assert_eq!(on_path.prefix, "/opt/homebrew");
        assert_eq!(
            on_path.invocation, "brew",
            "on PATH, a generated command may plainly say brew"
        );

        // The case the distinction exists for: a Homebrew installed in this
        // same session is in its prefix and not yet on PATH, and a command
        // saying "brew" would die with command-not-found.
        let fresh = homebrew(None, Some("/home/linuxbrew/.linuxbrew")).unwrap();
        assert_eq!(fresh.prefix, "/home/linuxbrew/.linuxbrew");
        assert_eq!(fresh.invocation, "/home/linuxbrew/.linuxbrew/bin/brew");

        assert_eq!(homebrew(Some("/bin/brew"), None).unwrap().prefix, "/");
        assert_eq!(
            homebrew(Some("brew is a function"), None),
            None,
            "a shell function named brew is not a Homebrew"
        );
        assert_eq!(homebrew(None, None), None);

        assert_eq!(
            packaging(Family::Macos, false, &[], Some(&on_path)).manager,
            PackageManager::Brew
        );
        assert_eq!(
            packaging(Family::Macos, false, &["brew"], None).manager,
            PackageManager::None,
            "brew on PATH without a Homebrew behind it is not a package manager"
        );
    }

    // AYEAYE-60 — the tool is remembered even where installing with it does
    // not last, because asking it what is already present is a read.
    #[test]
    fn an_image_based_system_may_not_install_and_is_still_asked() {
        let silverblue = packaging(Family::Fedora, true, &["dnf"], None);
        assert_eq!(
            silverblue.manager,
            PackageManager::None,
            "installing does not last"
        );
        assert_eq!(
            silverblue.tool,
            PackageManager::Dnf,
            "asking it is still a read"
        );
    }

    // AYEAYE-60 — systemd has to be present *and* usable as a user session:
    // a container has the binary and no user bus, and enabling a unit there
    // fails after setup has promised it would work.
    #[test]
    fn a_user_service_is_only_promised_where_one_can_really_be_installed() {
        let systemd = |available: &[&str], bus| service_manager(Os::Linux, available, bus);
        assert_eq!(systemd(&["systemctl"], true), ServiceManager::Systemd);
        assert_eq!(
            systemd(&["systemctl"], false),
            ServiceManager::None,
            "the binary without a user bus is not a place to install a service"
        );
        assert_eq!(systemd(&[], true), ServiceManager::None);

        assert_eq!(
            service_manager(Os::Macos, &["launchctl"], false),
            ServiceManager::Launchd,
            "launchd asks no user-bus question"
        );
        assert_eq!(service_manager(Os::Macos, &[], false), ServiceManager::None);
        assert_eq!(
            service_manager(Os::Unknown, &["systemctl"], true),
            ServiceManager::Systemd,
            "a machine we cannot name can still have a user bus that answers"
        );
    }
    // AYEAYE-60 — the one question asked before anything is installed without
    // being told how.
    #[test]
    fn a_machine_is_only_known_when_it_has_both_a_family_and_a_tool() {
        let ubuntu = distro(fixture!("os-release/ubuntu-24.04"));
        assert!(is_known(
            &ubuntu,
            &packaging(ubuntu.family, false, &["apt-get"], None)
        ));
        assert!(
            !is_known(&ubuntu, &packaging(ubuntu.family, false, &[], None)),
            "a family with nothing to install with is the manual path"
        );

        let silverblue = distro(fixture!("os-release/fedora-silverblue-40"));
        assert!(
            !is_known(
                &silverblue,
                &packaging(silverblue.family, true, &["dnf"], None)
            ),
            "installing does not last here, so setup may not act on its own"
        );

        let void = distro(fixture!("os-release/void"));
        assert!(!is_known(
            &void,
            &packaging(void.family, false, &["xbps-install"], None)
        ));
    }

    // AYEAYE-60 — one line, because it is one line of output.
    #[test]
    fn the_summary_names_everything_and_stays_on_one_line() {
        let ubuntu = distro(fixture!("os-release/ubuntu-24.04"));
        let line = summary(
            &ubuntu,
            &packaging(ubuntu.family, false, &["apt-get"], None),
            ServiceManager::Systemd,
        );
        assert_eq!(
            line,
            "Ubuntu 24.04.1 LTS (debian) x86_64, packages: apt-get, services: systemd"
        );
        assert!(!line.contains('\n'));

        let void = distro(fixture!("os-release/void"));
        let unknown = summary(
            &void,
            &packaging(void.family, false, &[], None),
            ServiceManager::None,
        );
        assert!(unknown.contains("void"), "it still names what it found");
        assert!(unknown.contains("(unknown)"));
        assert!(unknown.contains("packages: none"));
        assert!(unknown.contains("services: none"));

        let steamos = distro(fixture!("os-release/steamos-3.5"));
        assert!(
            summary(
                &steamos,
                &packaging(steamos.family, true, &["pacman"], None),
                ServiceManager::Systemd,
            )
            .ends_with(", image-based"),
            "an image-based system says so, or its packages line reads as a fault"
        );
    }

    // AYEAYE-60 — these words are the state-file contract: the shell writes
    // them into step.detect.* and later stages branch on them. A typo here is
    // a stage that silently stops matching.
    #[test]
    fn the_words_a_caller_matches_on_are_the_words_the_shell_writes() {
        assert_eq!(
            [Os::Linux, Os::Macos, Os::Unknown].map(Os::as_str),
            ["linux", "macos", "unknown"]
        );
        assert_eq!(
            [
                Family::Debian,
                Family::Fedora,
                Family::Arch,
                Family::Suse,
                Family::Macos,
                Family::Unknown,
            ]
            .map(Family::as_str),
            ["debian", "fedora", "arch", "suse", "macos", "unknown"]
        );
        assert_eq!(
            [
                PackageManager::AptGet,
                PackageManager::Dnf,
                PackageManager::Yum,
                PackageManager::Pacman,
                PackageManager::Zypper,
                PackageManager::Brew,
                PackageManager::None,
            ]
            .map(PackageManager::as_str),
            ["apt-get", "dnf", "yum", "pacman", "zypper", "brew", "none"]
        );
        assert_eq!(
            [
                ServiceManager::Systemd,
                ServiceManager::Launchd,
                ServiceManager::None,
            ]
            .map(ServiceManager::as_str),
            ["systemd", "launchd", "none"]
        );
    }

    // AYEAYE-60 — keeps the type in the test's own use list honest.
    #[test]
    fn a_homebrew_can_be_written_down() {
        let brew = Homebrew {
            prefix: "/usr/local".to_string(),
            invocation: "brew".to_string(),
        };
        assert_eq!(homebrew(Some("/usr/local/bin/brew"), None), Some(brew));
    }
}
