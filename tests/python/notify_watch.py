#!/usr/bin/env python3
"""When a push notification is worth sending.

One rule, and it is a rule about transitions rather than states: a
notification marks the moment a session became your problem. Not the fact
that it is your problem, which stays true for as long as you leave it, and
not anything it does on its own afterwards.

That distinction is the whole feature. A board that notified on states would
tell you about the same waiting session every ten seconds; one that notified
on every change would tell you that you had just replied, which you know,
and would bury the two messages worth having under the working/running
flicker of an agent doing its job.

So what fires is arrival into `waiting` -- the turn handed back -- and into
`blocked` -- stopped at a prompt, unable to proceed at all. Everything else
is a session working, which is not news.

Driven by tests/cases/notify_watch_test.sh. Run directly for the whole file,
or with a test id:

    tests/python/notify_watch.py
    tests/python/notify_watch.py TurnHandedBack
"""
import os
import sys
import unittest
from importlib.machinery import SourceFileLoader
from importlib.util import module_from_spec, spec_from_file_location

sys.dont_write_bytecode = True
REPO_ROOT = os.environ.get("REPO_ROOT") or os.path.dirname(
    os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
os.environ.setdefault("AYEAYE_TOKEN", "test-token")
_PATH = os.path.join(REPO_ROOT, "bin", "ayeaye")
_NAME = "ayeaye_notify_watch_test"
spec = spec_from_file_location(_NAME, _PATH, loader=SourceFileLoader(_NAME, _PATH))
ayeaye = module_from_spec(spec)
sys.modules[_NAME] = ayeaye
spec.loader.exec_module(ayeaye)


def pane(state, pane_id="%1", question=None, **kw):
    """One row of the board, as `overview` builds it."""
    row = dict(id=pane_id, session="ppu-toys", window="1", name="ppu-toys",
               cmd="claude", active=False, kind="claude", agent_id="abcd1234",
               state=state, age=12, last="", prompt=None)
    if question is not None:
        row["prompt"] = {"question": question,
                         "options": [{"key": "1", "label": "Yes"}]}
    row.update(kw)
    return row


class Watch(unittest.TestCase):
    """A sequence of sweeps, and which of them notified."""

    def setUp(self):
        # The default is what ships; a test that cares sets its own.
        self.addCleanup(setattr, ayeaye, "NOTIFY_STATES", ayeaye.NOTIFY_STATES)
        ayeaye.NOTIFY_STATES = {"blocked", "waiting"}

    def sweep(self, *states):
        """Poll once per argument, returning the states that notified.

        A plain string is a pane in that state; a tuple is (state, question)
        for a pane sitting at a prompt.
        """
        seen, fired = {}, []
        for step in states:
            state, question = step if isinstance(step, tuple) else (step, None)
            rows = [] if state is None else [pane(state, question=question)]
            fresh, seen = ayeaye.notify_changes(rows, seen)
            fired.append([r["state"] for r in fresh])
        return fired


# ------------------------------------------------------- the turn comes back

class TurnHandedBack(Watch):
    def test_working_to_waiting_notifies(self):
        self.assertEqual([[], ["waiting"]], self.sweep("working", "waiting"))

    def test_delegating_to_waiting_notifies(self):
        self.assertEqual([[], ["waiting"]], self.sweep("delegating", "waiting"))

    def test_running_to_waiting_notifies(self):
        # The same event as working -> waiting: a tool came back and the agent
        # spoke inside one poll. Watching only the two states named would drop
        # a real hand-back on nothing but the timing of the sweep.
        self.assertEqual([[], ["waiting"]], self.sweep("running", "waiting"))

    def test_a_session_left_waiting_notifies_once_and_then_shuts_up(self):
        self.assertEqual([[], ["waiting"], [], [], []],
                         self.sweep("working", "waiting", "waiting",
                                    "waiting", "waiting"))


# --------------------------------------------------------- you, replying

class YouReplying(Watch):
    def test_replying_to_a_waiting_agent_does_not_notify(self):
        # The case that prompted all of this: your own message moves the pane
        # to `working`, and being told about your own typing is noise.
        self.assertEqual([[], ["waiting"], []],
                         self.sweep("working", "waiting", "working"))

    def test_a_full_round_trip_notifies_only_on_the_way_back(self):
        self.assertEqual([[], ["waiting"], [], [], ["waiting"]],
                         self.sweep("working", "waiting", "working",
                                    "running", "waiting"))


# ------------------------------------------------------------ working alone

class WorkingIsNotNews(Watch):
    def test_the_working_running_flicker_is_silent(self):
        self.assertEqual([[], [], [], [], []],
                         self.sweep("working", "running", "working",
                                    "running", "delegating"))

    def test_no_state_at_all_is_never_a_notification(self):
        # A plain shell. It has no state, so it can never change into one.
        self.assertEqual([[], []], self.sweep(None, None))


# --------------------------------------------------------------- at a prompt

class StoppedAtAPrompt(Watch):
    def test_arriving_at_a_prompt_notifies(self):
        self.assertEqual([[], ["blocked"]],
                         self.sweep("working", ("blocked", "Hook range?")))

    def test_the_same_prompt_left_unanswered_notifies_once(self):
        self.assertEqual([[], ["blocked"], [], []],
                         self.sweep("working", ("blocked", "Hook range?"),
                                    ("blocked", "Hook range?"),
                                    ("blocked", "Hook range?")))

    def test_a_second_question_in_the_same_pane_is_fresh_news(self):
        # The state never leaves `blocked`, so only the question can say that
        # the first one was answered and another has taken its place.
        self.assertEqual([[], ["blocked"], ["blocked"]],
                         self.sweep("working", ("blocked", "Hook range?"),
                                    ("blocked", "Which scanline?")))

    def test_answering_a_prompt_does_not_notify(self):
        self.assertEqual([[], ["blocked"], []],
                         self.sweep("working", ("blocked", "Hook range?"),
                                    "working"))


# ------------------------------------------------------------- housekeeping

class Housekeeping(Watch):
    def test_the_first_sweep_only_seeds(self):
        # Every pane looks new after a restart, and a restart is not news --
        # otherwise restarting the service pages you about the whole board.
        rows = [pane("waiting", "%1"), pane("blocked", "%2", question="Q?")]
        fresh, seen = ayeaye.notify_changes(rows, {})
        self.assertEqual([], fresh)
        self.assertEqual(2, len(seen))

    def test_a_pane_that_goes_away_is_forgotten(self):
        rows = [pane("working", "%1")]
        _, seen = ayeaye.notify_changes(rows, {})
        _, seen = ayeaye.notify_changes([], seen)
        self.assertEqual({}, seen)

    def test_panes_are_tracked_apart(self):
        first = [pane("working", "%1"), pane("working", "%2")]
        _, seen = ayeaye.notify_changes(first, {})
        second = [pane("waiting", "%1"), pane("working", "%2")]
        fresh, _ = ayeaye.notify_changes(second, seen)
        self.assertEqual(["%1"], [r["id"] for r in fresh])

    def test_narrowing_the_setting_silences_the_rest(self):
        ayeaye.NOTIFY_STATES = {"blocked"}
        self.assertEqual([[], []], self.sweep("working", "waiting"))
        self.assertEqual([[], ["blocked"]],
                         self.sweep("working", ("blocked", "Q?")))

    def test_every_notified_state_has_wording_of_its_own(self):
        # A state that fires with no entry here would be announced by the
        # fallback, which says "needs you" whatever actually happened.
        for state in ayeaye.NOTIFY_STATES:
            self.assertIn(state, ayeaye._STATE_NOTE)


if __name__ == "__main__":
    unittest.main(verbosity=2)
