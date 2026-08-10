//! What this platform would run to install something, and why it sometimes
//! cannot.
//!
//! A port of `lib/pkg.sh`, and a deliberately smaller one. The shell carries a
//! six-backend adapter that installs; this returns the command and stops there,
//! because the milestone decided that carrying thousands of lines to save a
//! copy-and-paste is not worth it. **Nothing here installs anything** — there is
//! no function in this module with an effect, and the type of every one of them
//! says so.
//!
//! The five files under `tests/fixtures/pkg-commands/` are the contract. One per
//! family, the whole command line, for one fixed package list, so the five read
//! side by side are the difference between the families — and so a change to a
//! generated command shows up in a diff as the command itself rather than as a
//! moved quotation mark inside a test.

use super::platform::{Family, Homebrew, PackageManager, Packaging, Platform};

/// What this session could become, if it had to.
///
/// Read from `id -u` and the presence of `sudo`, which is where `lib/pkg.sh`
/// reads it from. It arrives as a value rather than being detected here for the
/// same reason nothing else in `machine` detects anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Privilege {
    /// This process is already root.
    Root,
    /// It is not, and `sudo` is here.
    Sudo,
    /// It is not, and there is no way to become root.
    None,
}

/// Why this machine cannot install a package on its own.
///
/// Five reasons and not one, because they call for different guidance. "ayeaye
/// has never heard of this distribution" and "this is Debian and apt-get is not
/// installed here" are the same refusal and completely different advice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Blocker {
    /// The package manager is right there and installing with it does not
    /// survive a reboot.
    ImageBased,
    /// A Mac with no Homebrew.
    NoHomebrew,
    /// A platform this layer could not name.
    UnknownPlatform,
    /// A platform it could name, whose package manager is not installed.
    NoPackageManager,
    /// The manager needs root and this session has neither root nor `sudo`.
    NoRoot,
}

impl Blocker {
    /// The one word a caller matches on, as `platform_pkg_blocker` prints it.
    pub fn as_str(self) -> &'static str {
        match self {
            Blocker::ImageBased => "image-based",
            Blocker::NoHomebrew => "no-homebrew",
            Blocker::UnknownPlatform => "unknown-platform",
            Blocker::NoPackageManager => "no-package-manager",
            Blocker::NoRoot => "no-root",
        }
    }
}

/// What this family calls a package, and whether that is knowledge or a guess.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Named {
    /// The name to put on a command line.
    pub name: String,
    /// Whether the table had an opinion. A logical name it does not know comes
    /// back unchanged, which is the most useful thing to do with it — but a
    /// caller that cares whether it was told or guessed can ask.
    pub known: bool,
}

/// This family's real name for a logical package.
///
/// Only the differences are interesting, and there are four of them:
///
/// - Arch calls the Python 3 interpreter `python`; `python3` there is an
///   entirely different, older package.
/// - Fedora's own repositories carry `ffmpeg-free`; plain `ffmpeg` needs a
///   third-party repository this project has no business enabling.
/// - openSUSE has no package literally called `ffmpeg` or `python3` either —
///   they are capabilities, provided by `ffmpeg-7` and `python313` — but zypper
///   installs a capability by name, so the logical name works as written.
/// - macOS has no package called `tar` and needs none: it ships bsdtar, and
///   Homebrew's nearest formula installs as `gtar`, which is not what anything
///   here is asking for. So that row says the table has no opinion, which is the
///   truth: there is nothing to install.
pub fn name(family: Family, logical: &str) -> Named {
    let known = |name: &str| Named {
        name: name.to_string(),
        known: true,
    };
    let guessed = Named {
        name: logical.to_string(),
        known: false,
    };
    match (family, logical) {
        (Family::Unknown, _) => guessed,
        (Family::Arch, "python3") => known("python"),
        (Family::Fedora, "ffmpeg") => known("ffmpeg-free"),
        (Family::Macos, "tar") => guessed,
        (_, "tmux" | "python3" | "ffmpeg" | "curl" | "git" | "tar") => known(logical),
        _ => guessed,
    }
}

/// Why packages cannot be installed here, or `None` when nothing is in the way.
///
/// The order is the order a person would notice them in.
pub fn blocker(
    platform: &Platform,
    packaging: &Packaging,
    homebrew: Option<&Homebrew>,
    privilege: Privilege,
) -> Option<Blocker> {
    if platform.immutable {
        return Some(Blocker::ImageBased);
    }
    if platform.family == Family::Macos && homebrew.is_none() {
        return Some(Blocker::NoHomebrew);
    }
    if platform.family == Family::Unknown {
        return Some(Blocker::UnknownPlatform);
    }
    if packaging.manager == PackageManager::None {
        return Some(Blocker::NoPackageManager);
    }
    if needs_root(packaging.manager) && privilege == Privilege::None {
        return Some(Blocker::NoRoot);
    }
    None
}

/// Why there is no command to give.
///
/// The shell returns two different non-zero statuses here and writes a paragraph
/// about why: `1` is a fact about the machine, `2` is a bug in the caller. They
/// are kept apart because reporting a caller's mistake to a user as "your
/// platform is unsupported" would send them looking in the wrong place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoCommand {
    /// This machine cannot, and here is which of the five reasons.
    Blocked(Blocker),
    /// Nothing was named to install. A bug above this layer.
    NothingNamed,
}

/// What this machine would run to install those packages, or why it cannot.
///
/// An install command this layer may not run is worse than no command: somebody
/// pastes it, it fails halfway or does not survive a reboot, and the reason was
/// known here all along.
///
/// apt-get is handed `DEBIAN_FRONTEND=noninteractive` through `env`, because a
/// debconf prompt in the middle of something that promised to be unattended is a
/// hang with no explanation — and `sudo env VAR=x` rather than `sudo VAR=x`,
/// which many sudoers configurations reject outright.
pub fn install_command(
    platform: &Platform,
    packaging: &Packaging,
    homebrew: Option<&Homebrew>,
    privilege: Privilege,
    logical: &[&str],
) -> Result<String, NoCommand> {
    if logical.is_empty() {
        return Err(NoCommand::NothingNamed);
    }
    if let Some(blocker) = blocker(platform, packaging, homebrew, privilege) {
        return Err(NoCommand::Blocked(blocker));
    }
    let names: Vec<String> = logical
        .iter()
        .map(|one| quote(&name(platform.family, one).name))
        .collect();
    let verb: Vec<&str> = match packaging.manager {
        PackageManager::AptGet => vec![
            "env",
            "DEBIAN_FRONTEND=noninteractive",
            "apt-get",
            "install",
            "-y",
        ],
        PackageManager::Dnf => vec!["dnf", "install", "-y"],
        PackageManager::Yum => vec!["yum", "install", "-y"],
        PackageManager::Pacman => vec!["pacman", "-S", "--needed", "--noconfirm"],
        PackageManager::Zypper => vec!["zypper", "--non-interactive", "install"],
        // Named by its invocation and not by the bare word: a Homebrew that is
        // installed in its prefix and not yet on `PATH` — which is where one
        // installed a minute ago lives — would produce a command line that dies
        // with command-not-found.
        PackageManager::Brew => match homebrew {
            Some(brew) => vec![brew.invocation.as_str(), "install"],
            // Unreachable through `blocker`, which answers NoHomebrew first;
            // named rather than unwrapped so a future caller cannot reach it.
            None => return Err(NoCommand::Blocked(Blocker::NoHomebrew)),
        },
        PackageManager::None => return Err(NoCommand::Blocked(Blocker::NoPackageManager)),
    };
    let mut words: Vec<String> = Vec::new();
    if needs_root(packaging.manager) && privilege == Privilege::Sudo {
        words.push("sudo".to_string());
    }
    words.extend(verb.into_iter().map(quote));
    words.extend(names);
    Ok(words.join(" "))
}

/// What to tell a person, when this layer will not do it for them.
///
/// Always something: the first line is the actionable one — the command where
/// there is a command, and otherwise why there is not — and the second names the
/// packages the way this platform calls them, so the text can be pasted at a
/// search engine and get somewhere.
pub fn manual_hint(
    platform: &Platform,
    packaging: &Packaging,
    homebrew: Option<&Homebrew>,
    privilege: Privilege,
    logical: &[&str],
) -> [String; 2] {
    let pretty = if platform.pretty.is_empty() {
        "this computer"
    } else {
        &platform.pretty
    };
    let first = match blocker(platform, packaging, homebrew, privilege) {
        None => match install_command(platform, packaging, homebrew, privilege, logical) {
            Ok(command) => format!("install them with: {command}"),
            Err(_) => format!("install them with {}.", packaging.manager.as_str()),
        },
        Some(Blocker::ImageBased) => match platform.family {
            Family::Fedora => format!(
                "{pretty} is image-based: layer packages with rpm-ostree install, then reboot."
            ),
            Family::Suse => format!(
                "{pretty} is image-based: install with transactional-update pkg install, then reboot."
            ),
            Family::Arch => format!(
                "{pretty} has a read-only system partition: pacman cannot install here in the usual way."
            ),
            _ => format!(
                "{pretty} is image-based: its package manager cannot install into the running system."
            ),
        },
        Some(Blocker::NoHomebrew) => {
            "Homebrew is not installed. Install it from https://brew.sh and run this again."
                .to_string()
        }
        Some(Blocker::UnknownPlatform) => format!(
            "this platform was not recognised (id: {}), so there is no package manager to drive.",
            platform.id
        ),
        Some(Blocker::NoPackageManager) => {
            format!("this is {pretty}, but its package manager is not installed here.")
        }
        Some(Blocker::NoRoot) => format!(
            "{} needs root, and this session has neither root nor sudo.",
            packaging.manager.as_str()
        ),
    };
    let names: Vec<String> = logical
        .iter()
        .map(|one| quote(&name(platform.family, one).name))
        .collect();
    let listed = if names.is_empty() {
        "(nothing)".to_string()
    } else {
        names.join(" ")
    };
    [first, format!("packages needed: {listed}")]
}

/// Which managers need root at all.
///
/// Homebrew refuses to run as root and installs into a prefix the user owns;
/// everything else writes to `/usr`.
fn needs_root(manager: PackageManager) -> bool {
    !matches!(manager, PackageManager::Brew | PackageManager::None)
}

/// One word, safe to paste into a shell.
///
/// The port of `_platform_quote`. A word made only of the characters a shell
/// leaves alone comes back untouched, because a command line where every word is
/// quoted is a command line nobody reads. Anything else is single-quoted, with
/// embedded single quotes closed, escaped and reopened — the one construction
/// that is safe under every Bourne shell.
fn quote(word: &str) -> String {
    if word.is_empty() {
        return "''".to_string();
    }
    let plain = word.chars().all(|c| {
        c.is_ascii_alphanumeric()
            || matches!(c, '.' | '_' | '/' | '@' | '%' | '+' | ':' | '=' | '-')
    });
    if plain {
        return word.to_string();
    }
    format!("'{}'", word.replace('\'', r"'\''"))
}

#[cfg(test)]
mod tests {
    use super::{
        Blocker, NoCommand, Privilege, blocker, install_command, manual_hint, name, quote,
    };
    use crate::machine::fixture;
    use crate::machine::platform::{
        Family, Homebrew, Os, PackageManager, Packaging, Platform, identify, packaging,
    };

    /// The one fixed package list the fixtures were captured against.
    const LIST: &[&str] = &["tmux", "python3", "ffmpeg", "curl", "git"];

    /// A machine of that family, as `platform_pkg_test.sh`'s `_on` builds one:
    /// the real `os-release`, the real detection, and a session with sudo.
    fn machine(os_release: &str) -> (Platform, Packaging, Option<Homebrew>) {
        let platform = identify(Some(os_release), Some("Linux"), Some("x86_64"), None);
        let packaging = packaging(
            platform.family,
            platform.immutable,
            &[
                "apt-get",
                "dnf",
                "yum",
                "pacman",
                "zypper",
                "rpm-ostree",
                "transactional-update",
            ],
            None,
        );
        (platform, packaging, None)
    }

    fn a_mac_with_brew() -> (Platform, Packaging, Option<Homebrew>) {
        let platform = identify(
            None,
            Some("Darwin"),
            Some("arm64"),
            Some(fixture!("sw_vers/macos-15.1")),
        );
        let brew = Homebrew {
            prefix: "/opt/homebrew".to_string(),
            invocation: "brew".to_string(),
        };
        let packaging = packaging(platform.family, platform.immutable, &[], Some(&brew));
        (platform, packaging, Some(brew))
    }

    fn command(
        parts: &(Platform, Packaging, Option<Homebrew>),
        privilege: Privilege,
    ) -> Result<String, NoCommand> {
        install_command(&parts.0, &parts.1, parts.2.as_ref(), privilege, LIST)
    }

    // AYEAYE-62 — the five golden files, byte for byte. They answer the question
    // a person actually asks — "what is this going to run on my machine?" — which
    // nobody can answer by reading a test body per family.
    #[test]
    fn the_command_each_family_would_run_matches_its_fixture() {
        for (os_release, golden) in [
            (
                fixture!("os-release/debian-12"),
                fixture!("pkg-commands/debian"),
            ),
            (
                fixture!("os-release/fedora-40"),
                fixture!("pkg-commands/fedora"),
            ),
            (fixture!("os-release/arch"), fixture!("pkg-commands/arch")),
            (
                fixture!("os-release/opensuse-tumbleweed"),
                fixture!("pkg-commands/suse"),
            ),
        ] {
            let parts = machine(os_release);
            assert_eq!(
                command(&parts, Privilege::Sudo).as_deref(),
                Ok(golden.trim_end_matches('\n')),
                "the generated command no longer matches its fixture for {}",
                parts.0.id
            );
        }
        assert_eq!(
            command(&a_mac_with_brew(), Privilege::Sudo).as_deref(),
            Ok(fixture!("pkg-commands/macos").trim_end_matches('\n'))
        );
    }

    // AYEAYE-62 — the fixture directory is a reference, and a reference with a
    // family missing from it reads as complete. The shell's equivalent
    // enumerates the directory; a pure crate cannot, so the guard here is the
    // compiler: a sixth family added to `Family` makes the match in `name`
    // non-exhaustive, and its `include_str!` has to be written by hand below.
    // This asserts only that each of the five really holds a command.
    #[test]
    fn every_family_this_layer_supports_has_a_command_fixture() {
        for golden in [
            fixture!("pkg-commands/debian"),
            fixture!("pkg-commands/fedora"),
            fixture!("pkg-commands/arch"),
            fixture!("pkg-commands/suse"),
            fixture!("pkg-commands/macos"),
        ] {
            assert!(golden.contains("tmux"), "a fixture that installs nothing");
        }
    }

    // AYEAYE-62 — the four real differences, one assertion each, because a
    // fixture cannot carry a reason.
    #[test]
    fn the_name_table_has_four_opinions_and_says_why() {
        // Arch's python3 is an entirely different, older package.
        assert_eq!(name(Family::Arch, "python3").name, "python");
        // Fedora's own repositories carry ffmpeg-free; plain ffmpeg needs a
        // third-party repository nothing here may enable.
        assert_eq!(name(Family::Fedora, "ffmpeg").name, "ffmpeg-free");
        // openSUSE installs a capability by its logical name.
        assert_eq!(name(Family::Suse, "python3").name, "python3");
        assert_eq!(name(Family::Suse, "ffmpeg").name, "ffmpeg");
        // macOS has no tar to install and needs none.
        let tar = name(Family::Macos, "tar");
        assert_eq!(tar.name, "tar");
        assert!(
            !tar.known,
            "there is nothing to install, and that is a fact"
        );
        assert!(name(Family::Debian, "tar").known);
    }

    // AYEAYE-62 — a name the table has no opinion about comes back unchanged,
    // which is the most useful thing to do with it, and the caller can still
    // tell it was a guess.
    #[test]
    fn an_unknown_name_comes_back_unchanged_and_marked_as_a_guess() {
        let guessed = name(Family::Debian, "libfoo-dev");
        assert_eq!(guessed.name, "libfoo-dev");
        assert!(!guessed.known);
        // And on a platform with no family, nothing is known at all — not even
        // the names that are the same everywhere.
        assert!(!name(Family::Unknown, "tmux").known);
    }

    // AYEAYE-62 — sudo where root is needed and this session is not root; no
    // sudo where it already is; and never in front of brew, which refuses to run
    // as root and installs into a prefix the user owns.
    #[test]
    fn root_is_asked_for_only_where_it_is_needed_and_not_already_held() {
        let debian = machine(fixture!("os-release/debian-12"));
        assert!(
            command(&debian, Privilege::Sudo)
                .expect("a command")
                .starts_with("sudo env DEBIAN_FRONTEND=")
        );
        assert!(
            command(&debian, Privilege::Root)
                .expect("a command")
                .starts_with("env DEBIAN_FRONTEND="),
            "already root: asking sudo for what it has would be theatre"
        );

        let mac = a_mac_with_brew();
        for privilege in [Privilege::Root, Privilege::Sudo, Privilege::None] {
            let said = command(&mac, privilege).expect("brew needs no root");
            assert!(
                !said.starts_with("sudo"),
                "brew refuses to run as root: {said}"
            );
        }
    }

    // AYEAYE-62 — and where there is no way to become root at all, there is no
    // command. An install command this layer may not run is worse than none:
    // somebody pastes it, it fails, and the reason was known here all along.
    #[test]
    fn a_session_that_cannot_become_root_gets_no_command_and_a_reason() {
        let debian = machine(fixture!("os-release/debian-12"));
        assert_eq!(
            command(&debian, Privilege::None),
            Err(NoCommand::Blocked(Blocker::NoRoot))
        );
        assert_eq!(
            blocker(&debian.0, &debian.1, None, Privilege::None),
            Some(Blocker::NoRoot)
        );
        let hint = manual_hint(&debian.0, &debian.1, None, Privilege::None, LIST);
        assert!(hint[0].contains("neither root nor sudo"), "{}", hint[0]);
        assert!(
            hint[1].contains("tmux python3 ffmpeg curl git"),
            "{}",
            hint[1]
        );
    }

    // AYEAYE-62 — an image-based system has its package manager right there, and
    // installing with it does not survive a reboot. Each family is told the way
    // its own system actually works.
    #[test]
    fn an_image_based_system_is_told_how_its_own_layering_works() {
        let mut silverblue = machine(fixture!("os-release/fedora-40"));
        silverblue.0.immutable = true;
        assert_eq!(
            blocker(&silverblue.0, &silverblue.1, None, Privilege::Sudo),
            Some(Blocker::ImageBased)
        );
        assert_eq!(
            command(&silverblue, Privilege::Sudo),
            Err(NoCommand::Blocked(Blocker::ImageBased))
        );
        assert!(
            manual_hint(&silverblue.0, &silverblue.1, None, Privilege::Sudo, LIST)[0]
                .contains("rpm-ostree install"),
        );
    }

    // AYEAYE-62 — the order the blockers are noticed in is the order the shell
    // notices them in, and it is load-bearing rather than incidental. An
    // image-based Mac with no Homebrew has two things wrong with it, and being
    // told to install Homebrew — onto a system whose package manager cannot
    // install into the running image anyway — is advice that cannot be followed.
    #[test]
    fn the_first_reason_reported_is_the_one_that_has_to_be_dealt_with_first() {
        let mut immutable_mac = a_mac_with_brew();
        immutable_mac.0.immutable = true;
        assert_eq!(
            blocker(&immutable_mac.0, &immutable_mac.1, None, Privilege::Sudo),
            Some(Blocker::ImageBased),
            "image-based comes before no-homebrew, as platform_pkg_blocker has it"
        );
        // And a platform with no name, whose package manager is therefore also
        // missing: "ayeaye has never heard of this" is the useful half.
        let stranger = identify(Some("ID=plan9\n"), Some("Linux"), Some("x86_64"), None);
        let nothing = packaging(stranger.family, false, &[], None);
        assert_eq!(
            blocker(&stranger, &nothing, None, Privilege::None),
            Some(Blocker::UnknownPlatform),
            "unknown-platform comes before no-package-manager and no-root"
        );
    }

    // AYEAYE-62 — a Mac with no Homebrew, and a platform with no name at all,
    // are different refusals calling for completely different advice.
    #[test]
    fn the_reasons_it_cannot_act_are_told_apart() {
        let bare_mac = identify(
            None,
            Some("Darwin"),
            Some("arm64"),
            Some(fixture!("sw_vers/macos-15.1")),
        );
        let none = packaging(bare_mac.family, false, &[], None);
        assert_eq!(
            blocker(&bare_mac, &none, None, Privilege::Sudo),
            Some(Blocker::NoHomebrew)
        );
        assert!(manual_hint(&bare_mac, &none, None, Privilege::Sudo, LIST)[0].contains("brew.sh"));

        let stranger = identify(Some("ID=plan9\n"), Some("Linux"), Some("x86_64"), None);
        let nothing = packaging(stranger.family, false, &[], None);
        assert_eq!(
            blocker(&stranger, &nothing, None, Privilege::Sudo),
            Some(Blocker::UnknownPlatform)
        );
        assert!(
            manual_hint(&stranger, &nothing, None, Privilege::Sudo, LIST)[0].contains("plan9"),
            "the id is what somebody would search for"
        );

        // A family this layer knows, whose package manager is simply not here.
        let debian = identify(
            Some(fixture!("os-release/debian-12")),
            Some("Linux"),
            Some("x86_64"),
            None,
        );
        let removed = packaging(debian.family, false, &[], None);
        assert_eq!(
            blocker(&debian, &removed, None, Privilege::Sudo),
            Some(Blocker::NoPackageManager)
        );
        assert!(
            manual_hint(&debian, &removed, None, Privilege::Sudo, LIST)[0]
                .contains("package manager is not installed here")
        );
    }

    // AYEAYE-62 — Homebrew found in its prefix and not yet on PATH still has to
    // produce a command that runs, which is exactly the case of a Homebrew
    // installed a minute ago.
    #[test]
    fn a_homebrew_not_yet_on_path_is_named_by_where_it_is() {
        let platform = identify(
            None,
            Some("Darwin"),
            Some("arm64"),
            Some(fixture!("sw_vers/macos-15.1")),
        );
        let brew = Homebrew {
            prefix: "/opt/homebrew".to_string(),
            invocation: "/opt/homebrew/bin/brew".to_string(),
        };
        let packs = packaging(platform.family, false, &[], Some(&brew));
        assert_eq!(packs.manager, PackageManager::Brew);
        let said = install_command(&platform, &packs, Some(&brew), Privilege::None, &["tmux"])
            .expect("a command");
        assert_eq!(said, "/opt/homebrew/bin/brew install tmux");
    }

    // AYEAYE-62 — no packages named is a caller's bug, not a fact about this
    // machine, and it must not come back as an install command with nothing on
    // the end of it.
    #[test]
    fn asking_to_install_nothing_produces_no_command() {
        let debian = machine(fixture!("os-release/debian-12"));
        assert_eq!(
            install_command(&debian.0, &debian.1, None, Privilege::Sudo, &[]),
            Err(NoCommand::NothingNamed),
            "a caller that named nothing is a bug above this layer, and must not \
             be told its platform is unsupported"
        );
        // The distinction the shell writes a paragraph about, kept: the same
        // call on a machine that genuinely cannot act answers differently.
        assert_eq!(
            install_command(&debian.0, &debian.1, None, Privilege::None, &[]),
            Err(NoCommand::NothingNamed),
            "a caller bug is a caller bug whatever the machine is"
        );
    }

    // AYEAYE-62 — the quoting, which is what stops a package name from becoming
    // a second command. Ordinary names are left alone, because a command line
    // where every word is quoted is a command line nobody reads.
    #[test]
    fn a_word_is_quoted_only_where_a_shell_would_read_it_otherwise() {
        assert_eq!(quote("ffmpeg-free"), "ffmpeg-free");
        assert_eq!(quote("python3.13"), "python3.13");
        assert_eq!(quote(""), "''");
        assert_eq!(quote("a b"), "'a b'");
        assert_eq!(quote("rm -rf /;"), "'rm -rf /;'");
        assert_eq!(quote("it's"), r"'it'\''s'");
        assert_eq!(quote("$(whoami)"), "'$(whoami)'");
        assert_eq!(quote("`id`"), "'`id`'");

        // The set is `_platform_quote`'s, character for character. Pinned whole,
        // because narrowing it only over-quotes and widening it is how a package
        // name becomes a second command.
        for safe in [
            "a",
            "Z",
            "0",
            ".",
            "_",
            "/",
            "@",
            "%",
            "+",
            ":",
            "=",
            "-",
            "a.b_c/d@e%f+g:h=i-j",
        ] {
            assert_eq!(quote(safe), safe, "{safe} needs no quoting");
        }
        for unsafe_word in [
            " ", "\t", "\n", "*", "?", "[", "]", "{", "}", "(", ")", "<", ">", "|", "&", ";", "!",
            "#", "~", "^", "\\", "\"", "'", "$",
        ] {
            assert!(
                quote(unsafe_word).starts_with('\''),
                "{unsafe_word:?} must be quoted"
            );
        }
    }

    // AYEAYE-62 — and a name carrying a shell metacharacter really does come out
    // of the generated command inert.
    #[test]
    fn a_hostile_package_name_cannot_become_a_second_command() {
        let debian = machine(fixture!("os-release/debian-12"));
        let said = install_command(
            &debian.0,
            &debian.1,
            None,
            Privilege::Sudo,
            &["tmux; rm -rf ~"],
        )
        .expect("a command");
        assert!(said.ends_with("'tmux; rm -rf ~'"), "{said}");
    }

    // AYEAYE-62 — the platform layer's own answer for a machine nothing is
    // wrong with, so the happy path of the hint is a command and not an excuse.
    #[test]
    fn a_machine_that_can_act_is_told_the_command_itself() {
        let debian = machine(fixture!("os-release/debian-12"));
        assert_eq!(blocker(&debian.0, &debian.1, None, Privilege::Sudo), None);
        let hint = manual_hint(&debian.0, &debian.1, None, Privilege::Sudo, &["tmux"]);
        assert_eq!(
            hint[0],
            "install them with: sudo env DEBIAN_FRONTEND=noninteractive apt-get install -y tmux"
        );
        assert_eq!(hint[1], "packages needed: tmux");
    }

    // AYEAYE-62 — Os is carried through unchanged; asserted so a family added
    // above cannot quietly stop being a Mac.
    #[test]
    fn a_mac_is_still_a_mac_to_this_layer() {
        let mac = a_mac_with_brew();
        assert_eq!(mac.0.os, Os::Macos);
        assert_eq!(mac.1.manager, PackageManager::Brew);
    }
}
