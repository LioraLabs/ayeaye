//! A pane's capture, read as the two things it actually is.
//!
//! Nothing here runs `capture-pane`. The shell above captures the text and the
//! grid size and hands them over; this decides what they mean, which is what
//! lets the split and the diff be held to output really captured from a tmux
//! server rather than to output somebody imagined.
//!
//! The capture is two things with very different lives glued together: the
//! **scrollback** above the screen, which tmux never rewrites and only appends
//! to, and the **screen**, which a TUI repaints constantly but which is only
//! ever `rows` lines. Splitting there is what makes most polls cost almost
//! nothing — see [`crate::pane::Cache::diff`].

use crate::json;
use crate::sha1;

/// How much of the digest names a window or a screen.
///
/// Twelve hex characters, as `bin/ayeaye`'s `_pane_hash` takes. The token is
/// opaque to the client and lands in a query string, so its only jobs are to be
/// short and to change when the text does.
const TOKEN_LEN: usize = 12;

/// The token naming this text.
pub fn token(text: &str) -> String {
    let mut digest = sha1::hex(text.as_bytes());
    digest.truncate(TOKEN_LEN);
    digest
}

/// The lines of a capture, as they are held and sent.
///
/// A line carrying escapes keeps its trailing spaces — they may be painted
/// background, and trimming them would strip the colour off the right-hand end
/// of a TUI's panel. A line with no escapes in it is right-trimmed, because the
/// spaces tmux pads it to the grid width with are nothing but bytes on the
/// radio.
pub fn lines(raw: &str) -> Vec<String> {
    raw.lines()
        .map(|line| {
            if line.contains(ESC) {
                line.to_string()
            } else {
                line.trim_end().to_string()
            }
        })
        .collect()
}

/// The escape byte `capture-pane -e` writes colour with.
const ESC: char = '\u{1b}';

/// Whether a line would look empty on screen.
///
/// `capture-pane -e` emits SGR sequences and nothing cursor-y, so a line that is
/// blank apart from its colouring is blank. Written as a scan rather than a
/// pattern because there is no regex crate below the shell and there must not
/// be one: the core declares no dependencies at all.
pub fn blank(line: &str) -> bool {
    let mut rest = line;
    while let Some(start) = rest.find(ESC) {
        if !rest[..start].trim().is_empty() {
            return false;
        }
        // `\x1b[` then the parameter bytes then `m`. Anything else is not an SGR
        // sequence, and is left alone rather than guessed at — a line that is
        // not blank must never be mistaken for one.
        let Some(after) = rest[start..].strip_prefix("\u{1b}[") else {
            return false;
        };
        let end = after.find(|c: char| !matches!(c, '0'..='9' | ';' | ':'));
        match end {
            Some(index) if after[index..].starts_with('m') => rest = &after[index + 1..],
            _ => return false,
        }
    }
    rest.trim().is_empty()
}

/// A capture, split where its two halves live.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct View {
    /// The scrollback above the screen. Append-only, as far as tmux is
    /// concerned, which is the property the patch below relies on.
    pub hist: Vec<String>,
    /// The visible screen, with its trailing blank lines dropped.
    pub screen: Vec<String>,
}

impl View {
    /// Split a capture at the bottom `rows` lines.
    ///
    /// `rows` of zero means the grid size could not be read, and then the whole
    /// capture is screen: pretending the top of it is settled history would let
    /// a repaint be sent as an append, and the client would grow a transcript of
    /// every frame a TUI ever drew.
    pub fn split(mut lines: Vec<String>, rows: usize) -> View {
        let hist = if rows > 0 && lines.len() > rows {
            lines.drain(..lines.len() - rows).collect()
        } else {
            Vec::new()
        };
        let mut screen = lines;
        // Only the screen is trimmed. tmux pads the grid to `rows` whatever the
        // program drew, so the blank lines at the bottom are the terminal's, not
        // the program's — while a blank line in the scrollback is one something
        // really printed.
        while screen.last().is_some_and(|line| blank(line)) {
            screen.pop();
        }
        View { hist, screen }
    }

    /// The screen as one block of text, which is what a token names and what
    /// goes over the wire.
    pub fn screen_text(&self) -> String {
        self.screen.join("\n")
    }
}

/// The whole capture as one block, for a client that does not speak the diff
/// protocol.
///
/// The whole thing rather than the last `rows` of it: trimming decapitates a
/// full-screen TUI, whose top box is as important as its bottom.
pub fn whole(lines: &[String]) -> String {
    let mut kept = lines;
    while kept.last().is_some_and(|line| blank(line)) {
        kept = &kept[..kept.len() - 1];
    }
    kept.join("\n")
}

/// The body `/api/pane` answers with when the client did not ask for a diff.
///
/// The grid comes back with the text so the client can render at the width tmux
/// actually wrapped for.
pub fn whole_body(text: &str, cols: u16, rows: u16) -> String {
    format!(
        "{{\"text\":{},\"cols\":{cols},\"rows\":{rows}}}",
        json::string(text)
    )
}

#[cfg(test)]
mod tests {
    use super::{View, blank, lines, token, whole, whole_body};

    fn owned(list: &[&str]) -> Vec<String> {
        list.iter().map(|line| line.to_string()).collect()
    }

    // AYEAYE-47 — a line that is blank apart from its colouring is blank. tmux
    // pads a repainted screen with them, and a screen whose trailing lines were
    // kept would change its token every time a TUI repainted the same frame.
    #[test]
    fn a_line_is_blank_when_nothing_but_colour_is_left_of_it() {
        assert!(blank(""));
        assert!(blank("   "));
        assert!(blank("\u{1b}[0m"));
        assert!(blank("\u{1b}[38;5;244m   \u{1b}[0m"));
        // The colon form is real: `38:2:255:0:0` is the direct-colour spelling.
        assert!(blank("\u{1b}[38:2:255:0:0m \u{1b}[m"));
        assert!(!blank("x"));
        assert!(!blank("\u{1b}[31mx\u{1b}[0m"));
        assert!(!blank("  text  "));
    }

    // AYEAYE-47 — only SGR. `capture-pane -e` emits colour and nothing
    // cursor-y, so anything else in a line is content: a sequence this does not
    // understand must leave the line looking non-blank rather than being
    // skipped over as decoration.
    #[test]
    fn an_escape_that_is_not_colour_is_not_treated_as_decoration() {
        assert!(!blank("\u{1b}[2J"));
        assert!(!blank("\u{1b}]0;title\u{7}"));
        assert!(!blank("\u{1b}"));
        assert!(!blank("\u{1b}[31"));
    }

    // AYEAYE-47 — a line with escapes keeps its trailing spaces, because they
    // may be painted background; a line without them is right-trimmed, because
    // tmux padded it to the grid and those bytes are nothing.
    #[test]
    fn a_coloured_line_keeps_its_padding_and_a_plain_one_does_not() {
        let raw = "plain   \n\u{1b}[44mpainted   \u{1b}[0m   \nlast";
        assert_eq!(
            lines(raw),
            owned(&[
                "plain",
                "\u{1b}[44mpainted   \u{1b}[0m   ",
                "last"
            ])
        );
        assert!(lines("").is_empty());
        // A trailing newline is a line ending, not an extra line — `str::lines`
        // and Python's `splitlines` agree, and the daemon relies on it.
        assert_eq!(lines("one\n"), owned(&["one"]));
    }

    // AYEAYE-47 — the split is the whole point: the bottom `rows` lines are the
    // screen a TUI repaints, everything above is scrollback tmux only appends
    // to. Getting it wrong turns every repaint into an append and grows the
    // client a transcript of every frame ever drawn.
    #[test]
    fn the_bottom_rows_are_the_screen_and_the_rest_is_scrollback() {
        let view = View::split(owned(&["h1", "h2", "s1", "s2", "s3"]), 3);
        assert_eq!(view.hist, owned(&["h1", "h2"]));
        assert_eq!(view.screen, owned(&["s1", "s2", "s3"]));
        assert_eq!(view.screen_text(), "s1\ns2\ns3");

        // A capture no longer than the grid is all screen: there is nothing
        // above it yet.
        let short = View::split(owned(&["s1", "s2"]), 3);
        assert!(short.hist.is_empty());
        assert_eq!(short.screen, owned(&["s1", "s2"]));
        assert_eq!(View::split(owned(&["a", "b", "c"]), 3).hist, Vec::<String>::new());
    }

    // AYEAYE-47 — a grid size that could not be read means the whole capture is
    // screen. The alternative — guessing a split — would send a repaint as an
    // append, and the client trusts appends blindly.
    #[test]
    fn a_grid_of_no_rows_is_all_screen() {
        let view = View::split(owned(&["a", "b", "c"]), 0);
        assert!(view.hist.is_empty());
        assert_eq!(view.screen, owned(&["a", "b", "c"]));
    }

    // AYEAYE-47 — the screen's trailing blanks come off and the scrollback's do
    // not. A blank line in the scrollback is one something really printed; a
    // blank line at the bottom of the screen is tmux padding the grid.
    #[test]
    fn trailing_blanks_come_off_the_screen_only() {
        let view = View::split(owned(&["h1", "", "h2", "s1", "", "\u{1b}[0m  "]), 3);
        assert_eq!(view.hist, owned(&["h1", "", "h2"]));
        assert_eq!(view.screen, owned(&["s1"]));

        // A screen that is entirely blank empties, and the history stays where
        // it is: the daemon pops out of the screen and no further.
        let empty = View::split(owned(&["h1", "", ""]), 2);
        assert_eq!(empty.hist, owned(&["h1"]));
        assert!(empty.screen.is_empty());
        assert_eq!(empty.screen_text(), "");
    }

    // AYEAYE-47 — the whole-text shape is the whole capture, not the last
    // screenful. Trimming to the screen decapitates a full-screen TUI, whose top
    // box is as important as its bottom.
    #[test]
    fn the_whole_shape_is_the_whole_capture_without_its_trailing_blanks() {
        assert_eq!(whole(&owned(&["a", "", "b", "", "  "])), "a\n\nb");
        assert_eq!(whole(&owned(&["", " "])), "");
        assert!(whole(&[]).is_empty());
    }

    // AYEAYE-47 — the body the panel reads when it did not ask for a diff. The
    // grid comes with it, or the page cannot render at the width tmux wrapped
    // for; and the text is a JSON string, so an escape byte in a coloured line
    // cannot end it.
    #[test]
    fn the_whole_body_carries_the_text_and_the_grid() {
        assert_eq!(
            whole_body("hi\nthere", 80, 24),
            r#"{"text":"hi\nthere","cols":80,"rows":24}"#
        );
        assert_eq!(
            whole_body("\u{1b}[31mred\"", 0, 0),
            r#"{"text":"\u001b[31mred\"","cols":0,"rows":0}"#
        );
    }

    // AYEAYE-47 — the token is `sha1(text)[:12]`, which is what `bin/ayeaye`
    // mints. Held to the digest rather than to itself, so the two daemons can be
    // compared: a phone mid-session moved between them keeps its window.
    #[test]
    fn a_token_is_the_first_twelve_hex_of_the_daemons_own_hash() {
        assert_eq!(token(""), "da39a3ee5e6b");
        assert_eq!(token("abc"), "a9993e364706");
        assert_eq!(token("abc").len(), 12);
        assert_ne!(token("a\nb"), token("a\nc"));
    }
}
