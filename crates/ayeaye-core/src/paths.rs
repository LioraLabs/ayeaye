//! Reading a path as text.
//!
//! Not `std::path`, and deliberately. Every path this app handles arrived as
//! text — out of a transcript somebody's agent wrote, out of a `git ls-files`,
//! out of a `/proc` symlink — and is compared against other text and handed
//! back over HTTP as text. Turning it into a `Path` to ask what its last
//! component is would buy a platform's separator rules on the way in and cost a
//! lossy decode on the way out.
//!
//! Separators are `/` and only `/`, which is what all three of those sources
//! print and what a JSON body carries.

/// The last component of a path.
///
/// Empty for a path that ends in a separator, which is a real answer: `/a/b/`
/// names a directory and has no file at the end of it. Callers that mean "the
/// directory's own name" trim the separator first — see
/// [`crate::tmux::session_name`], where trimming it is the difference between
/// naming a session after the project and naming it after nothing.
pub fn file_name(path: &str) -> &str {
    match path.rsplit_once('/') {
        Some((_, name)) => name,
        None => path,
    }
}

/// How many steps apart two paths are, through the deepest directory they
/// share.
///
/// Up from one to the common ancestor and back down to the other, so `a/b/c`
/// and `a/b/d` are two steps apart and `a/b/c` is nought from itself. Empty
/// components are ignored, which makes a leading separator, a trailing one, and
/// a doubled one all cost nothing — none of the three changes which directory
/// is being named.
///
/// This is what ranks a file reference against the pane that mentioned it: two
/// files with the same name in one repository are told apart by which of them
/// is nearer to where the agent is working.
///
/// Not a general path metric. `..` is a component like any other here, and that
/// is right for the input: these are repository-relative paths out of
/// `git ls-files`, which never contains one.
pub fn distance(left: &str, right: &str) -> usize {
    let mut left = left.split('/').filter(|part| !part.is_empty()).peekable();
    let mut right = right.split('/').filter(|part| !part.is_empty()).peekable();
    let mut apart = 0;
    // Walk down the shared prefix. Everything below it on either side is a step.
    while let (Some(here), Some(there)) = (left.peek(), right.peek()) {
        if here != there {
            break;
        }
        left.next();
        right.next();
    }
    apart += left.count();
    apart += right.count();
    apart
}

#[cfg(test)]
mod tests {
    use super::{distance, file_name};

    // AYEAYE-45
    #[test]
    fn a_path_ends_in_its_name() {
        assert_eq!(file_name("/a/b/c.jsonl"), "c.jsonl");
        assert_eq!(file_name("c.jsonl"), "c.jsonl");
        assert_eq!(file_name("/a/b/"), "");
        assert_eq!(file_name(""), "");
    }

    // AYEAYE-45 — up to the common ancestor and back down. This is what tells
    // two files with the same name in one repository apart: the one nearer to
    // where the agent is working is the one it meant.
    #[test]
    fn two_paths_are_the_steps_between_them() {
        assert_eq!(distance("a/b/c", "a/b/c"), 0);
        assert_eq!(distance("a/b/c", "a/b/d"), 2);
        assert_eq!(distance("a/b", "a/b/c/d"), 2);
        assert_eq!(distance("a/b/c", "x/y/z"), 6);
        assert_eq!(distance("", ""), 0);
        assert_eq!(distance("", "a/b"), 2, "the root is two steps above a/b");
    }

    // AYEAYE-45 — it is a distance, so it does not depend on which way round it
    // is asked. The scoring sorts on it, and a comparison that answered
    // differently in the two directions would order by whichever argument
    // happened to be first.
    #[test]
    fn a_distance_is_the_same_in_both_directions() {
        for (left, right) in [
            ("a/b/c", "a/b/d"),
            ("", "a/b"),
            ("a", "a/b/c/d/e"),
            ("crates/ayeaye/src", "crates/ayeaye-core/src/http"),
        ] {
            assert_eq!(
                distance(left, right),
                distance(right, left),
                "{left} to {right}"
            );
        }
    }

    // AYEAYE-45 — a leading separator, a trailing one and a doubled one all
    // name the same directory, and all three arrive: these paths come out of a
    // transcript an agent wrote, a `git ls-files`, and a `/proc` symlink.
    #[test]
    fn separators_that_name_nothing_cost_nothing() {
        assert_eq!(distance("/a/b", "a/b"), 0);
        assert_eq!(distance("a/b/", "a/b"), 0);
        assert_eq!(distance("a//b", "a/b"), 0);
        assert_eq!(distance("/", ""), 0);
        assert_eq!(distance("a/b/c/", "/a/b/d"), 2);
    }

    // AYEAYE-45 — a prefix in *characters* is not a prefix in components, and
    // reading it as one would score a file in a neighbouring directory as
    // though it sat beside the agent.
    #[test]
    fn a_shared_prefix_is_whole_components() {
        assert_eq!(distance("src/app", "src/apple"), 2);
        assert_eq!(distance("ayeaye", "ayeaye-core"), 2);
    }
}
