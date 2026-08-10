//! What `lsof -Fn` prints, and what can be read out of it.
//!
//! Field output puts each value on its own line behind a one-letter tag, and it
//! is the only form that survives a path with a space in it — which on a Mac is
//! an ordinary path, not an exotic one. `n` is a name; everything else is a
//! process or a descriptor number this does not need.

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
    use super::{cwd, names};
    use crate::machine::fixture;

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
