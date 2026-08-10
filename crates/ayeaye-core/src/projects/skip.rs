//! Which directories the walk must neither return nor enter.

/// Names that are never a project and never contain one.
///
/// Transcribed from `PROJECT_SKIP` in `bin/ayeaye`, and explicit rather than
/// clever on purpose: every name here is a decision. Dependency trees and
/// build output, caches and trash, virtualenvs, and the platform's own
/// folders.
pub const DEFAULT: &[&str] = &[
    "node_modules",
    "bower_components",
    "jspm_packages",
    "vendor",
    "Godeps",
    "target",
    "build",
    "dist",
    "out",
    "obj",
    "Debug",
    "Release",
    "site-packages",
    "dist-packages",
    "__pycache__",
    "eggs",
    "venv",
    "virtualenv",
    "env",
    "Library",
    "Applications",
    "System",
    "Volumes",
    "AppData",
    "Trash",
    "trash",
    "tmp",
    "temp",
    "cache",
    "Caches",
    "snap",
    "flatpak",
];

/// The suffix of a generated directory whose exact name differs per project.
const GENERATED_SUFFIX: &str = ".egg-info";

/// Whether the walk must skip a directory of this name.
///
/// A leading dot covers the whole class the list cannot enumerate: tool and
/// application internals, `.venv` and `.tox` and `.cache` and `.git` and every
/// dotfile directory a package manager will invent next year.
///
/// `extra` is a name list, matched exactly, never a path — so a machine can
/// hide `scratch` without hiding `scratchpad`, and a root named in
/// `PROJECT_ROOTS` is scanned whatever it is called, which is the escape hatch
/// for a directory these exclusions would otherwise hide.
pub fn is_skipped(name: &str, extra: &[String]) -> bool {
    name.starts_with('.')
        || DEFAULT.contains(&name)
        || name.ends_with(GENERATED_SUFFIX)
        || extra.iter().any(|other| other == name)
}

/// The per-machine additions, as `AYEAYE_PROJECT_SKIP` spells them.
///
/// Comma separated and trimmed, with blanks dropped, so a trailing comma or a
/// line continuation in an env file adds nothing rather than hiding the
/// directory named the empty string.
pub fn extra_names(setting: &str) -> Vec<String> {
    setting
        .split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{extra_names, is_skipped};

    // AYEAYE-50 — the exclusion rule the walk is bounded by. A leading dot
    // covers the class the list cannot enumerate — tool and application
    // internals, `.venv`, `.tox`, `.git` itself, and whatever a package
    // manager invents next year — and the suffix covers the one generated
    // name that is different on every machine.
    #[test]
    fn a_dot_directory_a_listed_name_and_an_egg_info_are_skipped() {
        let none: &[String] = &[];
        assert!(is_skipped(".venv", none));
        assert!(is_skipped(".git", none));
        assert!(is_skipped("node_modules", none));
        assert!(is_skipped("target", none));
        assert!(is_skipped("ayeaye.egg-info", none));
        assert!(!is_skipped("ayeaye", none));
        assert!(!is_skipped("src", none));
        // A name, matched exactly, never a path: `my-target` is not `target`.
        assert!(!is_skipped("my-target", none));
        assert!(!is_skipped("targets", none));
    }

    // AYEAYE-50 — one machine extends the list with
    // `AYEAYE_PROJECT_SKIP=name,name`: names, trimmed, blanks dropped.
    #[test]
    fn a_machine_can_add_names_of_its_own() {
        let extra = extra_names(" scratch , archive ,, ");
        assert_eq!(extra, vec!["scratch".to_string(), "archive".to_string()]);
        assert!(is_skipped("scratch", &extra));
        assert!(is_skipped("archive", &extra));
        assert!(!is_skipped("scratchpad", &extra));
        assert_eq!(extra_names(""), Vec::<String>::new());
    }

    // AYEAYE-50 — the list is the Python daemon's, transcribed. Spot-checking
    // a table this mechanical leaves a dropped name invisible, so this is the
    // set written out again from `bin/ayeaye` rather than read from the code
    // it is checking.
    #[test]
    fn the_list_is_the_one_the_daemon_carries() {
        const FROM_THE_DAEMON: &str = "
            node_modules bower_components jspm_packages vendor Godeps
            target build dist out obj Debug Release
            site-packages dist-packages __pycache__ eggs venv virtualenv env
            Library Applications System Volumes AppData
            Trash trash tmp temp cache Caches
            snap flatpak
        ";
        let mut expected: Vec<&str> = FROM_THE_DAEMON.split_whitespace().collect();
        let mut ours = super::DEFAULT.to_vec();
        expected.sort_unstable();
        ours.sort_unstable();
        assert_eq!(ours, expected);
    }
}
