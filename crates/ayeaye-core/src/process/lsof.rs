//! What `lsof -Fn` prints, and what can be read out of it.
//!
//! Field output puts each value on its own line behind a one-letter tag, and it
//! is the only form that survives a path with a space in it — which on a Mac is
//! an ordinary path, not an exotic one. `n` is a name; everything else is a
//! process or a descriptor number this does not need.
//!
//! The command lines live here for the same reason the parsing does: an argv is
//! data, and the two below differ by exactly the flag that decides whether the
//! answer is one path or all of them.

/// Ask for one process's working directory.
///
/// `-a` is what ANDs `-d` with `-p`. Without it `lsof` ORs them, and the first
/// name that comes back can be some other process's working directory entirely.
pub fn cwd_argv(pid: u32) -> Vec<String> {
    argv(&["lsof", "-a", "-d", "cwd", "-p", &pid.to_string(), "-Fn"])
}

/// Ask for everything one process has open.
///
/// Deliberately without the `-d cwd` above: with it the only file ever reported
/// is the working directory, and the path a resumed session is found by is a
/// descriptor.
pub fn names_argv(pid: u32) -> Vec<String> {
    argv(&["lsof", "-p", &pid.to_string(), "-Fn"])
}

fn argv(words: &[&str]) -> Vec<String> {
    words.iter().map(|word| (*word).to_string()).collect()
}

/// Every name `lsof` reported, in the order it reported them.
///
/// A name is not necessarily a file: the working directory, the executable and
/// any socket come back through the same record, and `lsof` does not sort them
/// out. Neither does this. The caller is looking for one known path among them,
/// not taking an inventory of the process.
pub fn names(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|line| line.strip_prefix('n'))
        .map(str::to_string)
        .collect()
}

/// The first name reported, which is the working directory when the caller
/// narrowed the query to it with `-d cwd`.
pub fn cwd(text: &str) -> Option<String> {
    text.lines()
        .find_map(|line| line.strip_prefix('n'))
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::{cwd, cwd_argv, names, names_argv};
    use crate::machine::fixture;

    // AYEAYE-44 — `-a` is what ANDs the two filters; without it the first name
    // back can belong to another process entirely.
    #[test]
    fn the_working_directory_query_ands_its_two_filters() {
        assert_eq!(
            cwd_argv(702),
            ["lsof", "-a", "-d", "cwd", "-p", "702", "-Fn"]
        );
    }

    // AYEAYE-44 — `-d cwd` is what the query above needs and what this must not
    // inherit: with it, the only file ever reported is the working directory.
    #[test]
    fn the_open_files_query_is_not_narrowed_to_one_descriptor() {
        let asked = names_argv(702);
        assert_eq!(asked, ["lsof", "-p", "702", "-Fn"]);
        assert!(!asked.iter().any(|word| word == "-d"));
    }

    // AYEAYE-44
    #[test]
    fn a_working_directory_comes_out_of_the_field_output() {
        assert_eq!(
            cwd(fixture!("lsof/darwin-cwd")).as_deref(),
            Some("/Users/someone/dev/thing")
        );
    }

    // AYEAYE-44 — ~/My Projects/... is entirely ordinary on a Mac, and the
    // field output is exactly what makes it safe to read.
    #[test]
    fn a_working_directory_containing_a_space_survives() {
        assert_eq!(
            cwd(fixture!("lsof/darwin-cwd-spaces")).as_deref(),
            Some("/Users/someone/My Projects/thing")
        );
    }

    // AYEAYE-44 — a process this user may not look into still gets a `p`
    // record and nothing else.
    #[test]
    fn output_with_no_name_in_it_is_no_working_directory() {
        assert_eq!(cwd(fixture!("lsof/darwin-denied")), None);
        assert_eq!(cwd(""), None);
    }

    // AYEAYE-44 — the paths, not a count: a resumed session is only
    // resolvable because the file it holds open can be found by name.
    #[test]
    fn every_name_record_is_an_open_path() {
        let open = names(fixture!("lsof/darwin-open-files"));
        assert!(
            open.contains(
                &"/Users/someone/.codex/sessions/2026/03/04/\
              rollout-2026-03-04T09-00-02-0123abcd-dead-beef.jsonl"
                    .to_string()
            )
        );
        assert!(open.contains(&"/Users/someone/dev/thing".to_string()));
        assert!(open.contains(&"/opt/homebrew/bin/codex".to_string()));
        assert!(open.contains(&"/Users/someone/My Sessions/notes.jsonl".to_string()));
    }

    // AYEAYE-44
    #[test]
    fn output_with_no_names_in_it_holds_nothing_open() {
        assert_eq!(names(fixture!("lsof/darwin-denied")), Vec::<String>::new());
        assert_eq!(names(""), Vec::<String>::new());
    }
}
