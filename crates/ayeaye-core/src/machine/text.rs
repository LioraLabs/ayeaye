//! Reading numbers and fields out of what a command printed.
//!
//! One copy, because the shell has one copy: `_hw_is_number` and `_hw_positive`
//! are single functions across 1700 lines of `lib/steps/20-hardware.sh`, and the
//! rule they encode — anything that is not a run of digits is not a number, and
//! zero is a probe that answered something it did not mean — has to say the same
//! thing everywhere or the port has three subtly different ideas of "unknown".

/// A run of digits and nothing else.
///
/// `unknown`, `-1`, `[N/A]`, `quite a few` and `8 cores` all arrive here at some
/// point from a command that answered something unexpected, and none of them is
/// a number. Surrounding blanks are allowed and stripped, because a value read
/// off a line or out of a file carries them.
pub fn whole(text: &str) -> Option<u64> {
    let text = text.trim();
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

/// A whole number greater than zero.
///
/// Zero cores and zero bytes of memory are not machines. Both have been seen —
/// from a broken container runtime, and from `nproc` under a cpuset with nothing
/// in it — and believing either is how a machine gets offered something it
/// cannot run.
pub fn positive(text: &str) -> Option<u64> {
    whole(text).filter(|n| *n > 0)
}

/// The first whitespace-separated word, or the empty string when there is none.
pub fn first_word(text: &str) -> &str {
    text.split_whitespace().next().unwrap_or("")
}

/// The nth whitespace-separated field of a line, counting from one.
pub fn field(line: &str, n: usize) -> Option<&str> {
    line.split_whitespace().nth(n - 1)
}

/// The first line a probe really produced, or `None` when it produced nothing.
///
/// A file that exists and is blank has not answered. `_hw_read_line` reads the
/// *first* line and treats it being empty as a failed read, so the caller falls
/// through to the next source — otherwise a zero-length `memory.max` would count
/// as "the version-two layout answered" and suppress the version-one fallback.
/// The first line and not the whole text, because a second line saying something
/// is a file this parser does not understand, and the shell would not read it
/// either.
pub fn answered(text: Option<&str>) -> Option<&str> {
    let first = text?.lines().next()?.trim();
    (!first.is_empty()).then_some(first)
}

/// One `Label: value` line, wherever it is indented to.
///
/// `system_profiler` is the only source shaped this way, and the shell reads it
/// with one function; so does this. The **first** matching line wins, whatever it
/// holds — an empty value is an answer, and looking past it for a better one is a
/// second opinion the shell never offers.
pub fn labelled_line<'a>(text: &'a str, label: &str) -> Option<&'a str> {
    text.lines()
        .find_map(|line| Some(line.trim().strip_prefix(label)?.strip_prefix(':')?.trim()))
}

#[cfg(test)]
mod tests {
    use super::{answered, field, first_word, labelled_line, positive, whole};

    // AYEAYE-60
    #[test]
    fn only_a_run_of_digits_is_a_number() {
        assert_eq!(whole(" 15846 \n"), Some(15846));
        assert_eq!(whole("0"), Some(0));
        assert_eq!(positive("0"), None, "zero cores is not a machine");
        for not_a_number in [
            "",
            "  ",
            "unknown",
            "-1",
            "[N/A]",
            "quite a few",
            "8 cores",
            "1.5",
        ] {
            assert_eq!(whole(not_a_number), None, "{not_a_number:?}");
        }
    }

    // AYEAYE-60
    #[test]
    fn fields_are_counted_from_one_and_split_on_any_blank() {
        assert_eq!(first_word("8 (4 performance and 4 efficiency)"), "8");
        assert_eq!(first_word("   "), "");
        assert_eq!(
            field("/dev/sda1\t104857600 96468992 8388608", 4),
            Some("8388608")
        );
        assert_eq!(field("a b", 3), None);
    }

    // AYEAYE-60 — a file that exists and is blank has not answered, and must
    // not suppress the next source.
    #[test]
    fn a_blank_answer_is_not_an_answer() {
        assert_eq!(answered(Some("")), None);
        assert_eq!(answered(Some(" \n")), None);
        assert_eq!(answered(Some("max\n")), Some("max"));
        assert_eq!(answered(None), None);
        assert_eq!(
            answered(Some("\n1073741824\n")),
            None,
            "the shell reads the first line and gives up on it being blank"
        );
    }
    // AYEAYE-60 — the first matching line wins, whatever it holds.
    #[test]
    fn a_labelled_line_is_read_wherever_it_is_indented_to() {
        let text =
            "Hardware:\n\n    Hardware Overview:\n\n      Chip: Apple M3\n      Memory: 24 GB\n";
        assert_eq!(labelled_line(text, "Chip"), Some("Apple M3"));
        assert_eq!(labelled_line(text, "Memory"), Some("24 GB"));
        assert_eq!(labelled_line(text, "Serial Number"), None);
        assert_eq!(
            labelled_line("  Chip:\n  Chip: Apple M3\n", "Chip"),
            Some(""),
            "an empty value is an answer, and the shell does not look past it"
        );
    }
}
