#!/usr/bin/env python3
"""Reading the question a pane is stopped on, off the screen it is drawn on.

This is the one thing the app exists to get right. A pane at a prompt is the
answer to "who needs me", and it is invisible in the transcript -- codex logs
the command it wants to run but never the approval, and claude's question is
a tool call like any other -- so the screen is the only place it can be seen.
Miss it and the card says `working`, which is the worst available lie: the
session is stopped dead and the board says it is busy.

Which is exactly what happened, and is the reason these tests exist. When an
`AskUserQuestion` carries per-option previews, claude draws it in two columns
-- options on the left, a preview box on the right -- and the box is taller
than the option list. The freshness check asked whether anything followed the
last option, meant to catch a prompt already answered and scrolled past; the
bottom of the preview box followed it, so every prompt with previews read as
unanswered work. One had been sitting unnoticed for half an hour.

So the fixtures here are real screens, captured from real sessions rather
than typed out, because every one of these bugs is a detail of the drawing:
box art sharing rows with the labels, labels wrapping onto the row beneath,
a hint that is the last line rather than the only thing after the options.

Driven by tests/cases/pane_prompt_test.sh. Run directly for the whole file,
or with a test id:

    tests/python/pane_prompt.py
    tests/python/pane_prompt.py PreviewColumns
    tests/python/pane_prompt.py Staleness.test_an_answered_prompt_is_gone
"""
import os
import sys
import unittest
from importlib.machinery import SourceFileLoader
from importlib.util import module_from_spec, spec_from_file_location

sys.dont_write_bytecode = True
REPO_ROOT = os.environ.get("REPO_ROOT") or os.path.dirname(
    os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
FIXTURES_DIR = os.environ.get("FIXTURES_DIR") or os.path.join(
    REPO_ROOT, "tests", "fixtures")
os.environ.setdefault("AYEAYE_TOKEN", "test-token")
_PATH = os.path.join(REPO_ROOT, "bin", "ayeaye")
_NAME = "ayeaye_pane_prompt_test"
spec = spec_from_file_location(_NAME, _PATH, loader=SourceFileLoader(_NAME, _PATH))
ayeaye = module_from_spec(spec)
sys.modules[_NAME] = ayeaye
spec.loader.exec_module(ayeaye)


def screen(name):
    """A captured pane, as `tmux capture-pane -p` printed it."""
    with open(os.path.join(FIXTURES_DIR, "pane-prompt", name)) as fh:
        return fh.read()


class Screen(unittest.TestCase):
    """A test whose pane shows whatever screen it is handed.

    capture-pane is the only tmux call under test, so it is the only one
    stubbed: nothing here starts an agent or reads a live pane.
    """

    def prompt(self, text):
        self.addCleanup(setattr, ayeaye, "tmux", ayeaye.tmux)
        ayeaye.tmux = lambda *a, **k: text
        return ayeaye.pane_prompt("%1")

    def labels(self, text):
        p = self.prompt(text)
        self.assertIsNotNone(p, "no prompt found on this screen")
        return [o["label"] for o in p["options"]]


# ------------------------------------------------------------ two columns

class PreviewColumns(Screen):
    """An AskUserQuestion whose options carry previews.

    Captured from a claude session that had been stopped on this question for
    half an hour while its card read `working`. The pane is narrow, which is
    what forces the labels to wrap and makes the preview box run on well past
    the last option -- both of the things that used to defeat the reader.
    """

    name = "claude-preview-columns"

    def test_a_prompt_drawn_in_two_columns_is_seen_at_all(self):
        # The whole bug in one assertion: this returned None.
        self.assertIsNotNone(self.prompt(screen(self.name)))

    def test_every_option_is_found(self):
        self.assertEqual(["1", "2", "3"],
                         [o["key"] for o in self.prompt(screen(self.name))["options"]])

    def test_no_label_carries_a_piece_of_the_preview_box(self):
        # The box shares rows with the options, so a reader that takes the
        # whole row sends "Explicit per-poke  ┌─────────────┐" to the phone.
        for label in self.labels(screen(self.name)):
            self.assertNotRegex(label, r"[─│┌┐└┘├┤✂]")

    def test_a_label_too_wide_for_its_column_keeps_its_tail(self):
        # "(Recommended)" wraps onto the row beneath, and the row beneath
        # matches nothing -- so the marker worth reading was the one dropped.
        self.assertEqual("Explicit per-poke range (Recommended)",
                         self.labels(screen(self.name))[0])

    def test_the_question_is_read_from_above_the_options(self):
        q = self.prompt(screen(self.name))["question"]
        self.assertIn("How should a scanline poke's hook range be decided?", q)
        # Stops at the rule claude draws under its prose, rather than running
        # on into the turn above it.
        self.assertNotIn("Fixing it is straightforward", q)

    def test_an_indented_rule_still_ends_the_question(self):
        # The rule is the only thing keeping the walk-back out of the turn
        # above. Cutting the preview column blanks a row of box art, and an
        # indented rule is a row it could blank by the same token -- at which
        # point the question swallows whatever the agent last said.
        p = self.prompt("● An earlier turn nobody asked about.\n"
                        "   ──────────────────────────────\n"
                        " ☐ Header\n\nPick one?\n\n"
                        "❯ 1. First      ┌──────┐\n"
                        "  2. Second     │ note │\n"
                        "                └──────┘\n\n"
                        "Enter to select · Esc to cancel\n")
        self.assertEqual("Header Pick one?", p["question"])
        self.assertEqual(["First", "Second"],
                         [o["label"] for o in p["options"]])


# ----------------------------------------------------------- one column

class PlainPrompts(Screen):
    """The same prompt without previews, and the trust prompt at startup.

    Single column, and the shape that already worked -- kept so the fix for
    the two-column case cannot quietly cost the common one.
    """

    def test_a_plain_question_is_read(self):
        p = self.prompt(screen("claude-plain-descriptions"))
        self.assertIn("Do you prefer tabs or spaces for indentation?",
                      p["question"])
        self.assertEqual(["1", "2", "3", "4", "5"],
                         [o["key"] for o in p["options"]])

    def test_an_options_own_description_is_part_of_it(self):
        # Claude prints a description under each label, indented past the
        # number. It reads as one option on the screen and must read as one
        # on the card: the pane cannot tell a description from a wrapped
        # label, and folding both in is right for both.
        self.assertEqual("Tabs Indent with tab characters.",
                         self.labels(screen("claude-plain-descriptions"))[0])

    def test_the_last_option_stops_at_the_hint(self):
        # "Chat about this" is the last option, and the hint is right under
        # it -- nothing from the hint may be swept into the label.
        self.assertEqual("Chat about this",
                         self.labels(screen("claude-plain-descriptions"))[-1])

    def test_the_startup_trust_prompt_is_a_prompt_like_any_other(self):
        # Not an AskUserQuestion at all, and worth answering from the phone:
        # a session waiting here has not begun and writes no transcript.
        p = self.prompt(screen("claude-plain-two-options"))
        self.assertEqual(["Yes, I trust this folder", "No, exit"],
                         [o["label"] for o in p["options"]])
        self.assertIn("Is this a project you created or one you trust?",
                      p["question"])


# ------------------------------------------------------------- staleness

class Staleness(Screen):
    """A prompt counts only while it is still the bottom of the screen.

    The reason the check exists: an answered prompt whose options have not
    yet scrolled away would otherwise keep the card pinned to `blocked`, and
    a card that cries for attention it does not need is worth no more than
    one that stays quiet when it does.
    """

    def test_an_answered_prompt_is_gone(self):
        # Claude replaces the whole block with a summary, so there is nothing
        # left to match -- this is the real screen after answering.
        self.assertIsNone(self.prompt(screen("claude-answered")))

    def test_output_below_a_plain_prompt_ends_it(self):
        stale = screen("claude-plain-descriptions") + \
            "\n● Noted -- tabs.\n\n✻ Churned for 4s\n"
        self.assertIsNone(self.prompt(stale))

    def test_output_below_a_two_column_prompt_ends_it(self):
        # The case that must keep working once the box below the options is
        # no longer treated as output.
        stale = screen("claude-preview-columns") + \
            "\n● Scoping the poke to lines 96-223.\n"
        self.assertIsNone(self.prompt(stale))

    def test_options_with_no_hint_under_them_are_not_a_prompt(self):
        self.assertIsNone(self.prompt(
            "Ranked them:\n  1. first\n  2. second\n  3. third\n"))

    def test_one_numbered_line_is_not_a_prompt(self):
        self.assertIsNone(self.prompt(
            "Step 1. run it\n\nEnter to select · Esc to cancel\n"))


# -------------------------------------------------------------- the card

class TheBlockedOverride(unittest.TestCase):
    """What the screen says beats what the transcript says.

    The transcript cannot see a pending question -- it reads `working`, which
    is the state this whole file exists to correct -- so `overview` asks the
    pane for every card and lets the answer win.
    """

    def setUp(self):
        for name in ("list_panes", "pane_session", "session_status",
                     "pane_prompt"):
            self.addCleanup(setattr, ayeaye, name, getattr(ayeaye, name))
        ayeaye.list_panes = lambda: [
            {"id": "%1", "session": "ppu-toys", "window": "1",
             "name": "ppu-toys", "cmd": "claude", "active": False}]
        ayeaye.pane_session = lambda pid: {
            "kind": "claude", "id": "abcd1234", "path": "/nonexistent"}
        # What the transcript concluded on its own, and why it is not enough.
        ayeaye.session_status = lambda sess, **kw: {
            "state": "working", "age": 1994, "last": "My typo -- m7.cx is 6"}

    def test_a_pane_at_a_prompt_is_blocked_however_busy_it_looks(self):
        ayeaye.pane_prompt = lambda pid: {"question": "Hook range?",
                                          "options": [{"key": "1",
                                                       "label": "Explicit"}]}
        row = ayeaye.overview()[0]
        self.assertEqual("blocked", row["state"])
        self.assertEqual("Hook range?", row["prompt"]["question"])
        # `age` survives the override: how long it has been stopped is the
        # thing worth knowing once you know it is stopped.
        self.assertEqual(1994, row["age"])

    def test_a_pane_with_no_prompt_keeps_the_transcripts_answer(self):
        ayeaye.pane_prompt = lambda pid: None
        row = ayeaye.overview()[0]
        self.assertEqual("working", row["state"])
        self.assertIsNone(row["prompt"])


if __name__ == "__main__":
    unittest.main(verbosity=2)
