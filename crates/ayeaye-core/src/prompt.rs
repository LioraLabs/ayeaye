//! The question a pane is stopped on, read off the screen it is drawn on.
//!
//! This is the one thing the app exists to get right, and it is the reason the
//! core is a crate of its own. A pane at a prompt is the answer to "who needs
//! me", and it is invisible everywhere else: codex logs the command it wants to
//! run but never the approval, and claude's question is a tool call like any
//! other. The screen is the only place it can be seen. Miss it and the card says
//! `working`, which is the worst lie available — the session is stopped dead and
//! the board says it is busy. One sat like that for half an hour.
//!
//! Nothing here runs `capture-pane`. The shell above captures the screen and
//! hands the text over, which is what lets this be held to screens really
//! captured from real sessions rather than to screens somebody typed out.

use crate::json;

/// One option the pane is offering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    /// What to press to pick it: `1` through `9`, as the TUI numbers them.
    pub key: String,
    /// What it says, wrapped rows and the description under it folded in.
    pub label: String,
}

/// A question a pane is stopped on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prompt {
    /// The prose above the options, read back until the agent's turn starts.
    pub question: String,
    /// Every option, in the order they are drawn.
    pub options: Vec<Choice>,
}

/// How many rows from the bottom of the screen are considered.
///
/// `bin/ayeaye`'s `pane_prompt(pane_id, window=30)`. A prompt further up than
/// this has something under it, and something under it is what "already
/// answered" looks like.
pub const WINDOW: usize = 30;

/// The longest a label is carried at, in characters.
pub const LABEL_LIMIT: usize = 90;

/// The longest a question is carried at, in characters.
pub const QUESTION_LIMIT: usize = 400;

/// Read the question a pane is stopped on, or `None`.
///
/// Same shape for both agents: numbered options with a selection marker and a
/// confirm/escape hint below them.
///
/// Claude draws that shape two ways, and the difference is not cosmetic.
/// Options carrying previews are laid out in two columns, with a preview box to
/// the right of the list that is taller than the list and keeps drawing below
/// the last option. So neither the labels nor the end of the block can be read
/// off whole screen rows: the labels are the left column, and what ends the
/// block is the hint rather than the last option.
pub fn read(screen: &str) -> Option<Prompt> {
    let mut lines: Vec<&str> = screen.lines().map(str::trim_end).collect();
    // Trailing blank rows hide the block: `capture-pane` pads to the pane's
    // height, so the hint is rarely the last row of the capture.
    while lines.last().is_some_and(|line| line.trim().is_empty()) {
        lines.pop();
    }
    let tail = &lines[lines.len().saturating_sub(WINDOW)..];
    // The left column throughout. With previews on, the box art shares rows
    // with the options and would otherwise be parsed as part of them — an
    // option would reach the phone labelled "Explicit range ┌────────┐".
    let column: Vec<&str> = tail.iter().map(|line| left_column(line)).collect();

    let at: Vec<usize> = (0..column.len())
        .filter(|&row| option(column[row]).is_some())
        .collect();
    let (&first, &last) = match (at.first(), at.last()) {
        (Some(first), Some(last)) if at.len() >= 2 => (first, last),
        _ => return None,
    };
    if !tail[first..].iter().any(|line| confirms(line)) {
        return None;
    }
    // The prompt must still be the last thing on screen. Once the agent moves
    // on, its output lands below the old block — without this, answered prompts
    // keep reporting "blocked" until they scroll away.
    //
    // Two endings, because the layouts differ in what follows the last option.
    // A plain prompt has nothing under it but the hint. One with previews has a
    // box taller than the list, and the notes and "chat about this" rows under
    // that — all of it above the hint, which the TUI draws last. So where the
    // plain shape asks what follows the options, this one asks whether the hint
    // is still the bottom of the screen.
    //
    // Asked of `tail` rather than of `column`: what follows the options is
    // output, and output is a whole row.
    let below = tail[last + 1..]
        .iter()
        .filter(|line| !line.trim().is_empty());
    if !below.clone().all(|line| confirms(line)) && !tail.last().is_some_and(|line| confirms(line))
    {
        return None;
    }

    Some(Prompt {
        question: question(tail, &column, first),
        options: choices(&column, &at),
    })
}

/// Every option, with the rows that belong to it folded in.
fn choices(column: &[&str], at: &[usize]) -> Vec<Choice> {
    let mut options = Vec::with_capacity(at.len());
    for (which, &row) in at.iter().enumerate() {
        let found = option(column[row]).expect("every row in `at` matched a moment ago");
        let mut label = found.label.to_string();
        let indent = indent_of(column[row]);
        // A label too wide for the column wraps onto the rows beneath it,
        // indented past its number. Those rows match nothing and used to be
        // dropped, which silently truncated exactly the options worth reading in
        // full — "(Recommended)" is the first thing to fall off the end. An
        // option's own description is indented the same way and reads as one
        // option on the screen, so folding both in is right for both.
        let until = at.get(which + 1).copied().unwrap_or(column.len());
        for line in &column[row + 1..until] {
            let said = line.trim();
            if said.is_empty() || is_rule(said) || confirms(said) {
                break;
            }
            if indent_of(line) <= indent {
                break;
            }
            label.push(' ');
            label.push_str(said);
        }
        options.push(Choice {
            key: found.key.to_string(),
            label: capped(&label, LABEL_LIMIT),
        });
    }
    options
}

/// The prose above the options, walked back from the first of them.
///
/// Blank lines interleave the question block in both agents, so they are skipped
/// rather than stopping it. It ends at the bullet that starts the agent's turn,
/// or at a rule — claude draws one between its prose and the prompt, and without
/// that the walk-back runs on into the startup banner.
fn question(tail: &[&str], column: &[&str], first: usize) -> String {
    let mut said: Vec<&str> = Vec::new();
    for row in (0..first).rev() {
        let line = column[row].trim();
        if line.is_empty() {
            // Cutting the preview column blanks a row that held nothing but box
            // art, which is what should happen — but it would blank an indented
            // rule too, and a rule is the one thing that must stop the walk-back
            // rather than be skipped over.
            if is_rule(tail[row].trim()) {
                break;
            }
            continue;
        }
        if is_rule(line) {
            break;
        }
        said.push(line.trim_start_matches(ORNAMENT));
        let opens_a_turn = line.starts_with(BULLET);
        if opens_a_turn || said.len() >= 8 {
            break;
        }
    }
    said.reverse();
    capped(&said.join(" "), QUESTION_LIMIT)
}

/// One screen row with the preview column, if there is one, cut away.
///
/// A prompt whose options carry previews is drawn in two columns: the list on
/// the left, a box holding the focused option's preview on the right. The box
/// occupies the same rows as the options, so every row of the list ends in a
/// slice of it, and the part of the row that belongs to the prompt is what
/// stands to the left. Anchored on the run of spaces that separates the columns,
/// because a label may legitimately contain a dash.
fn left_column(line: &str) -> &str {
    let mut run: Option<usize> = None;
    let mut wide = 0;
    for (index, character) in line.char_indices() {
        if character.is_whitespace() {
            run.get_or_insert(index);
            wide += 1;
            continue;
        }
        if wide >= 2
            && BOX.contains(&character)
            && let Some(start) = run
        {
            return line[..start].trim_end();
        }
        run = None;
        wide = 0;
    }
    line.trim_end()
}

/// The number and the label of one option row, if it is one.
struct Numbered<'a> {
    key: &'a str,
    label: &'a str,
}

/// `^\s*[›❯>]?\s*(\d+)\.\s+(.*\S)\s*$`, spelled out.
///
/// Only ASCII digits, where python's `\d` would also have taken the digits of
/// other alphabets. That is deliberate: the number is pressed as a key, and a
/// key nothing on a keyboard produces is not an option anyone can pick.
fn option(line: &str) -> Option<Numbered<'_>> {
    let mut rest = line.trim_start();
    if let Some(marker) = rest.chars().next()
        && SELECTED.contains(&marker)
    {
        rest = rest[marker.len_utf8()..].trim_start();
    }
    let digits = rest.len() - rest.trim_start_matches(|c: char| c.is_ascii_digit()).len();
    if digits == 0 {
        return None;
    }
    let (key, rest) = rest.split_at(digits);
    let rest = rest.strip_prefix('.')?;
    // `\s+`: the space after the number is required, so "Step 1.run it" is
    // prose and not an option.
    let label = rest.trim_start();
    if label.len() == rest.len() {
        return None;
    }
    let label = label.trim_end();
    if label.is_empty() {
        return None;
    }
    Some(Numbered { key, label })
}

/// Whether a row is the confirm/escape hint the TUI draws under a prompt.
fn confirms(line: &str) -> bool {
    let said = line.to_lowercase();
    HINTS.iter().any(|hint| said.contains(hint))
}

/// Whether a row is one of the rules an agent draws to separate its blocks.
fn is_rule(line: &str) -> bool {
    line.chars().count() >= 8 && line.chars().all(|character| RULE.contains(&character))
}

/// How many whitespace characters a row starts with.
fn indent_of(line: &str) -> usize {
    line.chars().take_while(|c| c.is_whitespace()).count()
}

/// The first `limit` **characters**, so a cut never lands inside one.
fn capped(text: &str, limit: usize) -> String {
    text.chars().take(limit).collect()
}

/// The body `/api/prompt` answers with.
///
/// `{"prompt":null}` when the pane is not stopped on anything, which is not the
/// same as an error and must not be sent as one: `share/app.html`'s `pollPrompt`
/// reads `d && d.prompt` and clears the card on a null, while a failed request
/// leaves the last card standing for the next poll to correct.
pub fn body(prompt: Option<&Prompt>) -> String {
    let Some(prompt) = prompt else {
        return r#"{"prompt":null}"#.to_string();
    };
    let mut out = format!(
        r#"{{"prompt":{{"question":{},"options":["#,
        json::string(&prompt.question)
    );
    for (index, choice) in prompt.options.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push_str(&format!(
            r#"{{"key":{},"label":{}}}"#,
            json::string(&choice.key),
            json::string(&choice.label)
        ));
    }
    out.push_str("]}}");
    out
}

/// One keypress the remote is allowed to make.
///
/// There is no way to build one but through [`press`], which is the point: this
/// endpoint is reachable over the network and must not become a general
/// `send-keys` hole. What crosses the boundary is a value from a table, never a
/// string somebody sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Press {
    /// The argument `send-keys` is given.
    key: &'static str,
    /// Whether it goes with `-l`, meaning "this is text, not a key name".
    literal: bool,
}

impl Press {
    /// What `send-keys` is given.
    pub fn key(&self) -> &'static str {
        self.key
    }

    /// Whether `send-keys` is told to read it literally.
    pub fn literal(&self) -> bool {
        self.literal
    }
}

/// The key this name means, or `None` if it means nothing.
///
/// **A digit is a press on its own and is never paired with Enter.** Claude's
/// multi-select uses digits to toggle checkboxes and Enter to submit, so pairing
/// them toggled and submitted in a single tap. Codex's approve prompt needs the
/// same two steps. Confirming is always explicit — which is also what you want
/// from a remote that can approve `git restore`.
///
/// Stricter than the daemon this replaces, deliberately. That one asked
/// `key.isdigit() and 1 <= int(key) <= 9`, which says yes to `"05"` — and then
/// sends `"05"`, two keystrokes, one of them the `0` the range was written to
/// exclude — and yes to the digits of other alphabets, which `int()` accepts and
/// no terminal produces. A table of the nine keys says neither.
pub fn press(key: &str) -> Option<Press> {
    if let Some((_, sent)) = NAV_KEYS.iter().find(|(name, _)| *name == key) {
        return Some(Press {
            key: sent,
            literal: false,
        });
    }
    DIGITS
        .iter()
        .find(|digit| **digit == key)
        .map(|digit| Press {
            key: digit,
            literal: true,
        })
}

/// The named keys the remote may press, and what tmux calls each.
///
/// An allow-list, transcribed from `bin/ayeaye`'s `NAV_KEYS`. Between these and
/// the numbers, most of working with an agent is confirming, backing out, or
/// moving the cursor.
pub const NAV_KEYS: &[(&str, &str)] = &[
    ("enter", "Enter"),
    ("esc", "Escape"),
    ("up", "Up"),
    ("down", "Down"),
    ("left", "Left"),
    ("right", "Right"),
    ("tab", "Tab"),
    ("space", "Space"),
];

/// The option numbers a prompt can be answered with, spelled out so that what
/// reaches `send-keys` is a `&'static str` from this file.
const DIGITS: &[&str] = &["1", "2", "3", "4", "5", "6", "7", "8", "9"];

/// The box-drawing characters a preview column is recognised by.
const BOX: &[char] = &[
    '─', '━', '│', '┃', '┌', '┐', '└', '┘', '├', '┤', '┬', '┴', '┼', '╌', '┈', '✂',
];

/// What a rule is drawn out of.
const RULE: &[char] = &['─', '━', '═', '-', '_', '╌', '┈'];

/// The bullet an agent opens a turn with. Reaching one ends the walk-back.
const BULLET: &[char] = &['•', '·', '●', '○', '◆', '▪'];

/// What is trimmed off the front of a line of the question: the turn bullets,
/// and the checkbox claude draws its question's header in.
const ORNAMENT: &[char] = &['•', '·', '●', '○', '◆', '▪', '☐', '☑', ' '];

/// The cursor an agent marks the focused option with.
const SELECTED: &[char] = &['›', '❯', '>'];

/// The hints that mean these numbered lines are a prompt and not a list.
const HINTS: &[&str] = &[
    "press enter to confirm",
    "esc to cancel",
    "(esc)",
    "enter to select",
    "arrow keys",
];

#[cfg(test)]
mod tests {
    use super::{Choice, NAV_KEYS, Prompt, body, press, read};

    /// An `AskUserQuestion` whose options carry previews.
    ///
    /// Captured from a claude session that had been stopped on this question for
    /// half an hour while its card read `working`. The pane is narrow, which is
    /// what forces the labels to wrap and makes the preview box run on well past
    /// the last option — both of the things that used to defeat the reader.
    const COLUMNS: &str =
        include_str!("../../../tests/fixtures/pane-prompt/claude-preview-columns");
    /// The same prompt without previews: single column, and the shape that
    /// already worked. Kept so the fix for the two-column case cannot quietly
    /// cost the common one.
    const DESCRIPTIONS: &str =
        include_str!("../../../tests/fixtures/pane-prompt/claude-plain-descriptions");
    /// The trust prompt at startup, which is not an `AskUserQuestion` at all.
    const TRUST: &str =
        include_str!("../../../tests/fixtures/pane-prompt/claude-plain-two-options");
    /// The real screen after answering: claude replaces the whole block with a
    /// summary, so there is nothing left to match.
    const ANSWERED: &str = include_str!("../../../tests/fixtures/pane-prompt/claude-answered");

    fn labels(screen: &str) -> Vec<String> {
        read(screen)
            .expect("no prompt found on this screen")
            .options
            .into_iter()
            .map(|choice| choice.label)
            .collect()
    }

    fn keys(screen: &str) -> Vec<String> {
        read(screen)
            .expect("no prompt found on this screen")
            .options
            .into_iter()
            .map(|choice| choice.key)
            .collect()
    }

    // AYEAYE-48 — the whole bug in one assertion: this returned nothing at all,
    // and the pane's card said `working` for half an hour while it sat here.
    #[test]
    fn a_prompt_drawn_in_two_columns_is_seen_at_all() {
        assert!(read(COLUMNS).is_some());
        assert_eq!(keys(COLUMNS), ["1", "2", "3"]);
    }

    // AYEAYE-48 — the box shares rows with the options, so a reader that takes
    // the whole row sends "Explicit per-poke  ┌─────────────┐" to the phone.
    #[test]
    fn no_label_carries_a_piece_of_the_preview_box() {
        for label in labels(COLUMNS) {
            assert!(
                !label.contains(['─', '│', '┌', '┐', '└', '┘', '├', '┤', '✂']),
                "a label carries the preview box: {label:?}"
            );
        }
    }

    // AYEAYE-48 — "(Recommended)" wraps onto the row beneath, and the row
    // beneath matches nothing — so the marker worth reading was the one dropped.
    #[test]
    fn a_label_too_wide_for_its_column_keeps_its_tail() {
        assert_eq!(labels(COLUMNS)[0], "Explicit per-poke range (Recommended)");
    }

    // AYEAYE-48 — the question is the prose above the options, and it stops at
    // the rule claude draws under it rather than running on into the turn above.
    #[test]
    fn the_question_is_read_from_above_the_options() {
        let asked = read(COLUMNS).expect("a prompt").question;
        assert!(
            asked.contains("How should a scanline poke's hook range be decided?"),
            "{asked:?}"
        );
        assert!(!asked.contains("Fixing it is straightforward"), "{asked:?}");
    }

    // AYEAYE-48 — the rule is the only thing keeping the walk-back out of the
    // turn above. Cutting the preview column blanks a row of box art, and an
    // indented rule is a row it could blank by the same token — at which point
    // the question swallows whatever the agent last said.
    #[test]
    fn an_indented_rule_still_ends_the_question() {
        let prompt = read(concat!(
            "● An earlier turn nobody asked about.\n",
            "   ──────────────────────────────\n",
            " ☐ Header\n\nPick one?\n\n",
            "❯ 1. First      ┌──────┐\n",
            "  2. Second     │ note │\n",
            "                └──────┘\n\n",
            "Enter to select · Esc to cancel\n",
        ))
        .expect("a prompt");
        assert_eq!(prompt.question, "Header Pick one?");
        assert_eq!(
            prompt
                .options
                .iter()
                .map(|o| o.label.as_str())
                .collect::<Vec<_>>(),
            ["First", "Second"]
        );
    }

    // AYEAYE-48 — the plain shape, which must keep working: the same prompt with
    // no previews on it, and every option found.
    #[test]
    fn a_plain_question_is_read() {
        let asked = read(DESCRIPTIONS).expect("a prompt").question;
        assert!(
            asked.contains("Do you prefer tabs or spaces for indentation?"),
            "{asked:?}"
        );
        assert_eq!(keys(DESCRIPTIONS), ["1", "2", "3", "4", "5"]);
    }

    // AYEAYE-48 — claude prints a description under each label, indented past
    // the number. It reads as one option on the screen and must read as one on
    // the card: the pane cannot tell a description from a wrapped label, and
    // folding both in is right for both.
    #[test]
    fn an_options_own_description_is_part_of_it() {
        assert_eq!(labels(DESCRIPTIONS)[0], "Tabs Indent with tab characters.");
    }

    // AYEAYE-48 — "Chat about this" is the last option and the hint is right
    // under it; nothing from the hint may be swept into the label.
    #[test]
    fn the_last_option_stops_at_the_hint() {
        assert_eq!(
            labels(DESCRIPTIONS).last().map(String::as_str),
            Some("Chat about this")
        );
    }

    // AYEAYE-48 — not an AskUserQuestion at all, and worth answering from the
    // phone: a session waiting here has not begun and writes no transcript.
    #[test]
    fn the_startup_trust_prompt_is_a_prompt_like_any_other() {
        assert_eq!(labels(TRUST), ["Yes, I trust this folder", "No, exit"]);
        let asked = read(TRUST).expect("a prompt").question;
        assert!(
            asked.contains("Is this a project you created or one you trust?"),
            "{asked:?}"
        );
    }

    // AYEAYE-48 — a prompt counts only while it is still the bottom of the
    // screen. A card that cries for attention it does not need is worth no more
    // than one that stays quiet when it does.
    #[test]
    fn an_answered_prompt_is_gone() {
        assert_eq!(read(ANSWERED), None);
    }

    // AYEAYE-48 — output below the options ends the prompt, in both layouts. The
    // two-column case is the one that must keep working once the box below the
    // options is no longer treated as output.
    #[test]
    fn output_below_a_prompt_ends_it() {
        let plain = format!("{DESCRIPTIONS}\n● Noted — tabs.\n\n✻ Churned for 4s\n");
        assert_eq!(read(&plain), None);
        let columns = format!("{COLUMNS}\n● Scoping the poke to lines 96-223.\n");
        assert_eq!(read(&columns), None);
    }

    // AYEAYE-48 — numbered lines are not a prompt on their own. Without the
    // hint, every ranked list an agent writes would stop its own pane.
    #[test]
    fn numbered_lines_with_no_hint_under_them_are_not_a_prompt() {
        assert_eq!(
            read("Ranked them:\n  1. first\n  2. second\n  3. third\n"),
            None
        );
        assert_eq!(
            read("Step 1. run it\n\nEnter to select · Esc to cancel\n"),
            None
        );
        assert_eq!(read(""), None);
    }

    // AYEAYE-48 — the allow-list, key by key, transcribed from `bin/ayeaye`'s
    // NAV_KEYS. This endpoint is reachable over the network, and anything that
    // is not on the list is a general send-keys hole.
    #[test]
    fn only_the_named_keys_and_the_nine_numbers_can_be_pressed() {
        for (name, sent) in [
            ("enter", "Enter"),
            ("esc", "Escape"),
            ("up", "Up"),
            ("down", "Down"),
            ("left", "Left"),
            ("right", "Right"),
            ("tab", "Tab"),
            ("space", "Space"),
        ] {
            let pressed = press(name).unwrap_or_else(|| panic!("{name} is a key"));
            assert_eq!(pressed.key(), sent);
            assert!(!pressed.literal(), "{name} is a key name, not text");
        }
        assert_eq!(NAV_KEYS.len(), 8, "a key arrived or left without a reason");

        for number in ["1", "5", "9"] {
            let pressed = press(number).unwrap_or_else(|| panic!("{number} is an option"));
            assert_eq!(pressed.key(), number);
            assert!(pressed.literal(), "a number is typed, not named");
        }
    }

    // AYEAYE-48 — everything else. `Enter` and `C-c` are what a caller reaches
    // for first when probing this for a send-keys hole; `0`, `10` and `05` are
    // what the daemon's `key.isdigit() and 1 <= int(key) <= 9` lets through or
    // nearly does — `"05"` passes that test and then sends two keystrokes, one
    // of them the `0` the range was written to exclude.
    #[test]
    fn nothing_else_is_a_key_at_all() {
        for hostile in [
            "",
            "0",
            "10",
            "05",
            "1 ",
            " 1",
            "Enter",
            "ENTER",
            "C-c",
            "C-d",
            "kill-server",
            ";",
            "Escape",
            "\u{661}",
            "\u{665}",
            "1;2",
        ] {
            assert_eq!(press(hostile), None, "{hostile:?} must not be a key");
        }
    }

    // AYEAYE-48 — the body `share/app.html`'s pollPrompt reads. A pane that is
    // not stopped on anything answers a null prompt rather than an error: the
    // page clears its card on a null and keeps the last one on a failure, and
    // those are opposite instructions.
    #[test]
    fn the_body_carries_the_question_and_a_card_per_option() {
        assert_eq!(body(None), r#"{"prompt":null}"#);
        let prompt = Prompt {
            question: "Hook range?".to_string(),
            options: vec![
                Choice {
                    key: "1".to_string(),
                    label: "Explicit".to_string(),
                },
                Choice {
                    key: "2".to_string(),
                    label: "Span it".to_string(),
                },
            ],
        };
        assert_eq!(
            body(Some(&prompt)),
            concat!(
                r#"{"prompt":{"question":"Hook range?","options":["#,
                r#"{"key":"1","label":"Explicit"},"#,
                r#"{"key":"2","label":"Span it"}"#,
                r#"]}}"#,
            )
        );
    }

    // AYEAYE-48 — a label is whatever an agent drew on a screen, escape bytes
    // and quotes included. One badly-drawn option must not end the string that
    // carries it, or the panel gets no prompt at all for the pane that has one.
    #[test]
    fn a_label_off_a_real_screen_cannot_break_the_body() {
        let prompt = Prompt {
            question: "say \"hi\"".to_string(),
            options: vec![Choice {
                key: "1".to_string(),
                label: "a\u{1b}[31mb".to_string(),
            }],
        };
        assert_eq!(
            body(Some(&prompt)),
            concat!(
                r#"{"prompt":{"question":"say \"hi\"","options":["#,
                r#"{"key":"1","label":"a\u001b[31mb"}]}}"#,
            )
        );
    }

    // AYEAYE-48 — and the two halves meet: a real captured screen goes in and
    // the body the phone reads comes out, which is the claim neither the parser
    // test nor the writer test makes on its own.
    #[test]
    fn a_captured_screen_becomes_the_body_the_phone_reads() {
        let answered = body(read(COLUMNS).as_ref());
        assert!(
            answered.starts_with(r#"{"prompt":{"question":"Hook range How should"#),
            "{answered}"
        );
        assert!(
            answered.contains(r#"{"key":"1","label":"Explicit per-poke range (Recommended)"}"#),
            "{answered}"
        );
        assert_eq!(body(read(ANSWERED).as_ref()), r#"{"prompt":null}"#);
    }
}
