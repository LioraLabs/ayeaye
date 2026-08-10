//! Quoting a string that is going to be typed into somebody's shell.
//!
//! An opening prompt is handed to an agent by *typing* a command line into the
//! pane's shell — `send-keys -l` — rather than by handing a program an argv.
//! That is the daemon's design and it is kept: both agents take the prompt as
//! an argument, which sidesteps guessing how long a TUI takes to boot before
//! anything can be pasted into it. It also makes the prompt the single most
//! attacker-influenceable string in the app, because it is read by a shell.
//!
//! **Quoting here can fail, and that is the point.** The usual API — Python's
//! `shlex.quote`, and every Rust equivalent — is total: every string has a
//! POSIX quoting. But the shell on the other end is whichever one the user
//! runs, and fish is not POSIX. Inside single quotes fish honours `\\` and
//! `\'` as escapes and POSIX honours neither, so one backslash away the two
//! read the same bytes differently or refuse them outright. A quoting function
//! that cannot say no would have to pick a shell and be wrong on the other.

/// One word, quoted so a shell reads it back as exactly this text.
///
/// Single quotes, because inside them every shell in scope suspends expansion
/// entirely: `$`, a backtick, `*`, `"` and `#` in a prompt stay text. What
/// cannot be carried through them is refused rather than transformed — see
/// [`Unquotable`] for which, and why each.
pub fn quote(text: &str) -> Result<String, Unquotable> {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('\'');
    for character in text.chars() {
        match character {
            // Close, escape, reopen. The only way to get an apostrophe past
            // single quotes, and the only spelling of it both shells read the
            // same way.
            '\'' => out.push_str("'\\''"),
            '\\' => return Err(Unquotable::Backslash),
            control if control.is_control() => return Err(Unquotable::Control(control)),
            other => out.push(other),
        }
    }
    out.push('\'');
    Ok(out)
}

/// Why a string cannot be quoted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unquotable {
    /// It carries a character that is a keystroke rather than text.
    ///
    /// This string is typed, not written. An escape byte begins a terminal
    /// sequence, a newline submits the line before the rest of it arrives, and
    /// there is no quoting that turns either back into text.
    Control(char),
    /// It carries a backslash, which POSIX and fish disagree about.
    ///
    /// Inside single quotes fish reads `\\` as one backslash and `\'` as an
    /// apostrophe, where POSIX reads both literally. So `a\` quotes to `'a\'`,
    /// which POSIX reads as `a\` and fish reads as a string nobody closed —
    /// leaving the shell on a continuation prompt with the agent never started.
    /// Refused whole rather than per-backslash: the rule that admits `C:\path`
    /// has to reason about what follows each one, and this is the last string
    /// in the app worth being clever with.
    Backslash,
}

#[cfg(test)]
mod tests {
    use super::{Unquotable, quote};

    // AYEAYE-51 — the prompt is typed into the user's shell, so it arrives as
    // one word only if it is quoted. Single quotes, because inside them every
    // shell in scope suspends expansion entirely: a `$`, a backtick, a `*` and
    // a `"` in a prompt are text and must stay text.
    #[test]
    fn a_prompt_comes_back_as_one_word_that_expands_to_nothing() {
        assert_eq!(quote("fix the tests").unwrap(), "'fix the tests'");
        assert_eq!(
            quote("run $(rm -rf ~) and `id` on *.rs \"now\"").unwrap(),
            "'run $(rm -rf ~) and `id` on *.rs \"now\"'"
        );
        assert_eq!(quote("").unwrap(), "''");
    }

    // AYEAYE-51 — the one character single quotes cannot hold is the single
    // quote. `'\''` is the spelling that closes, escapes and reopens, and it is
    // the one spelling POSIX and fish agree on: fish reads the `\'` outside the
    // quotes as an escaped quote, POSIX reads it as a backslash-escaped quote,
    // and both end up with one apostrophe.
    #[test]
    fn an_apostrophe_is_closed_escaped_and_reopened() {
        assert_eq!(quote("it's").unwrap(), r"'it'\''s'");
        assert_eq!(quote("''").unwrap(), r"''\'''\'''");
    }

    // AYEAYE-51 — this text is not written to a file, it is *typed*. A control
    // character is a keystroke: an escape byte begins a terminal sequence, a
    // newline submits the line early, and no amount of quoting makes either of
    // them text again. Refused with the character named, because the caller has
    // to be able to say which one.
    #[test]
    fn a_control_character_is_refused_rather_than_typed() {
        assert_eq!(
            quote("fix\nthe tests").unwrap_err(),
            Unquotable::Control('\n')
        );
        assert_eq!(quote("\u{1b}[31m").unwrap_err(), Unquotable::Control('\u{1b}'));
        assert_eq!(quote("a\u{0}b").unwrap_err(), Unquotable::Control('\u{0}'));
        assert_eq!(quote("a\tb").unwrap_err(), Unquotable::Control('\t'));
        // Ordinary spaces are not control characters and are the reason the
        // text needed quoting in the first place.
        assert!(quote("a b  c").is_ok());
    }

    // AYEAYE-51 — the reason this API is fallible at all. `bin/ayeaye` quotes
    // the prompt with `shlex.quote` and says in a comment that fish reads the
    // result the same way POSIX does. It does not, and the difference is a
    // backslash. Run against a real `sh` and a real `fish`:
    //
    // | prompt   | `shlex.quote` | sh       | fish                        |
    // |----------|---------------|----------|-----------------------------|
    // | `a\`     | `'a\'`        | `a\`     | *Unexpected end of string*  |
    // | `a\\b`   | `'a\\b'`      | `a\\b`   | `a\b`                       |
    // | `C:\p`   | `'C:\p'`      | `C:\p`   | `C:\p`                      |
    //
    // The first row is the one that matters: the pane is created, the command
    // is typed, and the shell sits on a continuation prompt waiting for a quote
    // that never comes — an agent that looks started and is not. A rule that
    // admitted the third row would have to reason about what follows each
    // backslash, and this is the last string in the app to be clever with.
    #[test]
    fn a_backslash_is_refused_because_the_shells_disagree_about_it() {
        for hostile in [r"a\", r"a\\b", r"C:\path", r"\", r"a\'b"] {
            assert_eq!(
                quote(hostile).unwrap_err(),
                Unquotable::Backslash,
                "{hostile:?} means different things in sh and in fish"
            );
        }
    }
}
