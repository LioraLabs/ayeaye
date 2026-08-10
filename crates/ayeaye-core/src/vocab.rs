//! The identifiers on somebody's screen, as a hint for the cleanup model.
//!
//! A dictation is speech, so an identifier in it arrives spelled the way it
//! sounds: "parse config" for `parse_config`, "check why fetch user throws" for
//! `checkWhyFetchUser`. The names are *on the screen* the speaker is looking at,
//! so handing that list to the cleanup model is what lets it spell them back.
//!
//! Two things this deliberately is not. It is not the screen text — given prose
//! the model plagiarises it, and a dictation came back as the full pytest
//! command that happened to be on screen, plus an invented line number. And it
//! is not given to the *speech* model as an initial prompt: whisper continues in
//! the style of its prompt, so a list of identifiers turned "check why fetch
//! user throws" into "check_wifetch_user_throwser". Spelling is fixed here,
//! after the words exist, where it cannot corrupt them.
//!
//! It never leaves the machine the pane is on.

/// How far back a capture is worth reading.
///
/// `bin/voice-dictate`'s `VOICE_CONTEXT_LINES`. The bound is here as well as in
/// the capture the shell asks tmux for, so that a caller which handed over a
/// whole scrollback still gets the recent screen rather than the oldest 400
/// characters of it.
pub const LINES: usize = 40;

/// The most vocabulary a prompt will carry.
///
/// `bin/voice-dictate`'s `VOCAB_CHARS`. A hint that grows without bound stops
/// being a hint: it costs prompt tokens, and past a point the model starts
/// reaching into it for words nobody said.
pub const CHARS: usize = 400;

/// The punctuation a name may be wrapped in on a screen, and is not part of.
const WRAPPING: &[char] = &['.', ',', ':', ';', '(', ')', '[', ']', '{', '}', '\'', '"'];

/// The shortest and longest a name is allowed to be.
///
/// Under three characters it is as likely to be an abbreviation as an
/// identifier; over sixty it is a line of base64 or a hash, and a model asked to
/// spell it back will do something worse than ignore it.
const SHORTEST: usize = 3;
const LONGEST: usize = 60;

/// The identifiers visible on a captured screen, newest first.
///
/// Newest first because the bound bites: the line somebody is looking at is more
/// likely to hold the name they just said than the line that scrolled past a
/// minute ago, so the truncation should fall on the old end.
///
/// Deduplicated case-insensitively, and the *first* spelling seen wins. Two
/// spellings of one name are a worse hint than one, and the newest is the one
/// on screen now.
pub fn on_screen(capture: &str) -> String {
    let lines: Vec<&str> = capture.lines().collect();
    let recent = &lines[lines.len().saturating_sub(LINES)..];

    let mut seen: Vec<String> = Vec::new();
    let mut names: Vec<&str> = Vec::new();
    for line in recent.iter().rev() {
        for word in candidates(line) {
            let word = word.trim_matches(|c| WRAPPING.contains(&c));
            if !looks_like_code(word) {
                continue;
            }
            let key = word.to_lowercase();
            if seen.contains(&key) {
                continue;
            }
            seen.push(key);
            names.push(word);
        }
    }

    let joined = names.join(" ");
    // Characters rather than bytes: a screen holding a name with an accent in it
    // must not truncate into invalid text, and `CHARS` is a bound on how much a
    // person can usefully hint with rather than on memory.
    match joined.char_indices().nth(CHARS) {
        Some((at, _)) => joined[..at].to_string(),
        None => joined,
    }
}

/// Every run of screen text that could be a name.
///
/// `[A-Za-z_~./][A-Za-z0-9_./~-]{2,}` from `bin/voice-dictate`, scanned rather
/// than compiled: the core has no dependencies, and a regex engine is a large
/// thing to admit for one pattern that is two character classes and a length.
///
/// Runs are maximal and non-overlapping, which is what the regex does too — it
/// is greedy and has nothing after it to backtrack for.
fn candidates(line: &str) -> Vec<&str> {
    let mut found = Vec::new();
    let mut rest = line.char_indices().peekable();
    while let Some((start, first)) = rest.next() {
        if !starts_a_name(first) {
            continue;
        }
        let mut end = start + first.len_utf8();
        let mut length = 1;
        while let Some(&(at, next)) = rest.peek() {
            if !continues_a_name(next) {
                break;
            }
            end = at + next.len_utf8();
            length += 1;
            rest.next();
        }
        if length >= SHORTEST {
            found.push(&line[start..end]);
        }
    }
    found
}

/// Whether a character can begin a name. `-` is deliberately absent: a leading
/// dash is a flag, and `--json` is not a word anybody dictates.
fn starts_a_name(c: char) -> bool {
    c.is_ascii_alphabetic() || matches!(c, '_' | '~' | '.' | '/')
}

/// Whether a character can continue one.
fn continues_a_name(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '/' | '~' | '-')
}

/// Whether a word is code rather than English.
///
/// The tell is punctuation a word does not have — an underscore, a slash, a
/// dash, a dot with letters both sides — or an internal capital. Plain prose
/// fails all of them, which is the point: a hint diluted with the words of the
/// sentence is a hint the model reads past.
pub fn looks_like_code(word: &str) -> bool {
    let length = word.chars().count();
    if !(SHORTEST..=LONGEST).contains(&length) {
        return false;
    }
    if word.contains(['_', '/', '-']) {
        return true;
    }
    let characters: Vec<char> = word.chars().collect();
    characters.windows(3).any(|window| {
        window[1] == '.' && is_word_character(window[0]) && is_word_character(window[2])
    }) || characters
        .windows(2)
        .any(|window| window[0].is_lowercase() && window[1].is_uppercase())
}

/// `\w`, as the pattern this was ported from means it.
fn is_word_character(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

#[cfg(test)]
mod tests {
    use super::{CHARS, LINES, on_screen};

    /// A real capture, taken from a private `tmux -L` server running `ls` and
    /// `grep` over this crate's own source.
    const LISTING: &str = include_str!("../../../tests/fixtures/pane-vocab/rust-source-listing");

    // AYEAYE-58
    #[test]
    fn the_identifiers_on_the_screen_come_back_and_the_prose_does_not() {
        let names = on_screen(LISTING);
        let words: Vec<&str> = names.split(' ').collect();

        // Things that look like code, in each of the ways they can: a file
        // name, a path, a snake_case identifier, and an internal capital.
        for name in [
            "cleanup.rs",
            // A colon is not part of a name, so grep's `path:line:` prefix
            // contributes the path and stops there.
            "crates/ayeaye-core/src/audio.rs",
            "max_new_tokens",
            "Pcm16kMono",
        ] {
            assert!(words.contains(&name), "{name:?} is missing from {names:?}");
        }
        // And the sentence somebody typed contributes nothing, so the hint
        // stays dense enough to be worth giving a model.
        for prose in ["the", "tests", "are", "failing", "again", "why"] {
            assert!(!words.contains(&prose), "{prose:?} is prose, not a name");
        }
    }

    // AYEAYE-58
    //
    // Newest first, and the *first* spelling of a repeated name wins. The
    // ordering is not cosmetic: `CHARS` truncates the tail, so a list built
    // oldest-first spends the budget on what has scrolled away and drops the
    // line the speaker is looking at.
    #[test]
    fn the_newest_line_is_hinted_first_and_a_repeat_is_only_hinted_once() {
        let screen = "\
old_name here
middle_name and old_name again
new_name last
";
        assert_eq!(on_screen(screen), "new_name middle_name old_name");
        // Case is not a second name. OLD_NAME two lines up would otherwise
        // spend budget saying the same thing in a spelling nobody can see.
        assert_eq!(on_screen("OLD_NAME\nold_name\n"), "old_name");
    }

    // AYEAYE-58
    //
    // The bound is on characters, and it falls on the oldest end because of the
    // ordering above. Asserting the length alone would pass on an
    // implementation that truncated mid-name and produced a spelling that is on
    // nobody's screen, so what survives is asserted too.
    #[test]
    fn the_hint_is_bounded_and_the_bound_falls_on_the_oldest_names() {
        let mut screen = String::new();
        for index in 0..200 {
            screen.push_str(&format!("name_{index:03}\n"));
        }
        let names = on_screen(&screen);

        assert!(
            names.chars().count() <= CHARS,
            "{} chars",
            names.chars().count()
        );
        assert!(names.starts_with("name_199 name_198"), "{names}");
        assert!(
            !names.contains("name_000"),
            "the oldest names are the ones cut"
        );
    }

    // AYEAYE-58
    //
    // Only the recent screen is read. A caller that hands over a whole
    // scrollback must not be answered with the names that were on it an hour
    // ago — which, given the newest-first ordering, is exactly what would
    // happen if the tail were taken from the wrong end.
    #[test]
    fn only_the_recent_lines_are_read() {
        // Short names on purpose: forty long ones would spend the whole of
        // `CHARS` on their own, and the old name would then be absent because
        // it was truncated rather than because it was out of the window.
        let mut screen = String::from("ancient_name\n");
        for index in 0..LINES {
            screen.push_str(&format!("n_{index:02}\n"));
        }
        let names = on_screen(&screen);

        assert!(names.contains("n_00"), "{names}");
        assert!(
            !names.contains("ancient_name"),
            "a line past the window is not on the screen: {names}"
        );
        // A capture shorter than the window is read whole rather than refused.
        assert_eq!(on_screen("only_one\n"), "only_one");
        assert_eq!(on_screen(""), "");
    }
}
