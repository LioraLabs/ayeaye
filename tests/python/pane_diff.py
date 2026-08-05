#!/usr/bin/env python3
"""The /api/pane diff protocol: what the server resends, and what it never
needs to.

The server promises one thing: whatever form the response takes -- full,
patch, or same -- a client that applies it holds exactly the history window
and screen the server captured. Every test here is that promise from a
different angle, plus the shapes themselves: an unchanged pane must cost a
token match, a repaint must carry the screen alone, and a scroll must carry
only the lines that crossed into history."""
import os
import sys
import unittest
from importlib.machinery import SourceFileLoader
from importlib.util import module_from_spec, spec_from_file_location

sys.dont_write_bytecode = True
REPO_ROOT = os.environ.get("REPO_ROOT") or os.path.dirname(
    os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
os.environ.setdefault("AYEAYE_TOKEN", "test-token")
spec = spec_from_file_location(
    "ayeaye_pane_diff_test", os.path.join(REPO_ROOT, "bin", "ayeaye"),
    loader=SourceFileLoader("ayeaye_pane_diff_test",
                            os.path.join(REPO_ROOT, "bin", "ayeaye")))
ayeaye = module_from_spec(spec)
sys.modules[spec.name] = ayeaye
spec.loader.exec_module(ayeaye)

ROWS, COLS, WINDOW = 4, 80, 10


class FakePane:
    """A pane as capture-pane sees it: an append-only scrollback whose last
    ROWS lines are the screen and whose WINDOW lines above are the history
    the capture reaches."""

    def __init__(self):
        self.lines = ["$ start"]

    def scroll(self, *fresh):
        self.lines.extend(fresh)

    def repaint(self, *screen):
        """A TUI redraw: the visible rows change, nothing enters history."""
        assert len(self.lines) >= ROWS
        self.lines[-ROWS:] = list(screen)[:ROWS]

    def capture(self):
        return "\n".join(self.lines[-(WINDOW + ROWS):])

    def hist(self):
        return self.lines[-(WINDOW + ROWS):-ROWS]

    def screen(self):
        return "\n".join(self.lines[-ROWS:])


class Client:
    """The app.html side of the protocol, reduced to its state machine."""

    def __init__(self):
        self.hist, self.screen, self.hh, self.sh = [], "", "", ""

    def poll(self, pane_id="%1"):
        d = ayeaye.pane_view_diff(pane_id, WINDOW, self.hh, self.sh)
        if not d.get("same"):
            if "hist" in d:
                self.hist = d["hist"]
            else:
                self.hist = self.hist[d["drop"]:] + d["add"]
            self.screen = d["screen"]
        self.hh, self.sh = d["hh"], d["sh"]
        return d


class ProtocolShapes(unittest.TestCase):
    def setUp(self):
        self.pane = FakePane()
        self.original = ayeaye.tmux
        ayeaye.tmux = self.fake_tmux
        self.addCleanup(setattr, ayeaye, "tmux", self.original)
        ayeaye._PANE_CACHE.clear()
        self.client = Client()

    def fake_tmux(self, *args):
        if args[0] == "capture-pane":
            return self.pane.capture()
        return "%d\t%d" % (COLS, ROWS)

    def fill(self):
        """Scroll until the history window is at capacity."""
        for n in range(WINDOW + ROWS):
            self.pane.scroll("line %d" % n)

    def assertInSync(self):
        self.assertEqual(self.pane.hist(), self.client.hist)
        self.assertEqual(self.pane.screen(), self.client.screen)

    def test_a_client_with_no_tokens_gets_a_full_send(self):
        d = self.client.poll()
        self.assertIn("hist", d)
        self.assertNotIn("same", d)
        self.assertTrue(d["hh"] and d["sh"])
        self.assertInSync()

    def test_an_unchanged_pane_is_a_token_match_and_nothing_else(self):
        self.client.poll()
        d = self.client.poll()
        self.assertEqual(1, d.get("same"))
        for heavy in ("hist", "screen", "add"):
            self.assertNotIn(heavy, d)
        self.assertInSync()

    def test_a_repaint_carries_the_screen_and_no_history(self):
        self.fill()
        self.client.poll()
        self.pane.repaint("spinner |", "thinking...", "esc to stop", "> _")
        d = self.client.poll()
        self.assertEqual((0, []), (d["drop"], d["add"]))
        self.assertNotIn("hist", d)
        self.assertInSync()

    def test_a_scroll_carries_only_the_lines_that_crossed_into_history(self):
        self.fill()
        self.client.poll()
        self.pane.scroll("fresh 1", "fresh 2")
        d = self.client.poll()
        self.assertEqual(2, d["drop"])
        self.assertEqual(2, len(d["add"]))
        self.assertInSync()

    def test_history_still_filling_appends_without_dropping(self):
        self.client.poll()
        self.pane.scroll("second", "third")
        d = self.client.poll()
        self.assertEqual(0, d["drop"])
        self.assertInSync()

    def test_a_rewrap_that_shares_no_lines_still_reconstructs_exactly(self):
        self.fill()
        self.client.poll()
        # A resize rewraps every line: same scrollback, no line in common.
        self.pane.lines = ["rewrapped %d" % n for n in range(WINDOW + ROWS)]
        self.client.poll()
        self.assertInSync()

    def test_a_token_the_server_does_not_remember_gets_a_full_send(self):
        self.client.poll()
        self.pane.scroll("more")
        ayeaye._PANE_CACHE.clear()          # restart, or plain eviction
        d = self.client.poll()
        self.assertIn("hist", d)
        self.assertInSync()

    def test_a_long_burst_of_scrolls_never_desyncs_the_client(self):
        self.fill()
        self.client.poll()
        for n in range(3 * WINDOW):
            self.pane.scroll("burst %d" % n)
            if n % 3 == 0:
                self.pane.repaint("a", "b", "c", "run %d" % n)
            self.client.poll()
            self.assertInSync()

    def test_the_cache_stays_bounded(self):
        for n in range(3 * ayeaye._PANE_CACHE_MAX):
            self.pane.scroll("line %d" % n)
            self.client.poll()
        self.assertLessEqual(len(ayeaye._PANE_CACHE),
                             ayeaye._PANE_CACHE_MAX)

    def test_the_legacy_shape_is_still_whole_text(self):
        d = ayeaye.pane_view("%1", WINDOW)
        self.assertEqual(sorted(d), ["cols", "rows", "text"])
        self.assertEqual((COLS, ROWS), (d["cols"], d["rows"]))
        self.assertIn("$ start", d["text"])


if __name__ == "__main__":
    unittest.main(verbosity=2)
