#!/usr/bin/env python3
"""What a card says a session is doing, and why.

Every state here answers one question -- who is this session waiting on --
and the reason that is worth pinning is that the answer used to be wrong in
the case the app exists for. An orchestrator that hands its work to other
agents writes nothing to its own transcript while they run, so the pane
making the most progress was the one that went quiet and read as idle.

The same split runs one level down: a session mid tool call is `running` and
not `working`, because a call that is out is the one place a session hangs for
an hour, and `working` reads as a model being thorough.

There is no idle state to test. How long a session has been quiet is `age`,
which is reported beside the state and never instead of it: a session that
handed you the turn three hours ago is still waiting on you.

Both transcript formats are written here as the agents really write them,
because the pairing this rests on is a detail of each. Claude gets an
acknowledgement back the instant a background agent is launched and the real
answer much later, in a turn naming the tool call it answers. Codex blocks in
a tool on a short timeout, in a loop.

Driven by tests/cases/session_state_test.sh. Run directly for the whole file,
or with a test id:

    tests/python/session_state.py
    tests/python/session_state.py ClaudeDelegation.test_a_launched_agent_is_outstanding
"""
import json
import os
import shutil
import sys
import tempfile
import time
import unittest
from importlib.machinery import SourceFileLoader
from importlib.util import module_from_spec, spec_from_file_location

sys.dont_write_bytecode = True
REPO_ROOT = os.environ.get("REPO_ROOT") or os.path.dirname(
    os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
os.environ.setdefault("AYEAYE_TOKEN", "test-token")
_PATH = os.path.join(REPO_ROOT, "bin", "ayeaye")
_NAME = "ayeaye_session_state_test"
spec = spec_from_file_location(_NAME, _PATH, loader=SourceFileLoader(_NAME, _PATH))
ayeaye = module_from_spec(spec)
sys.modules[_NAME] = ayeaye
spec.loader.exec_module(ayeaye)


# ------------------------------------------------------- transcript writing

class Transcript(unittest.TestCase):
    """A transcript on disk, written a line at a time, then asked its state."""

    kind = "claude"

    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="ayeaye-state-")
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.path = os.path.join(self.tmp, "transcript.jsonl")
        self.lines = []

    def write(self, d):
        self.lines.append(d)
        with open(self.path, "w") as fh:
            for line in self.lines:
                fh.write(json.dumps(line) + "\n")

    def state(self, **kw):
        return ayeaye.session_status(
            {"kind": self.kind, "path": self.path, "id": "abcd1234"}, **kw)["state"]

    def status(self, **kw):
        return ayeaye.session_status(
            {"kind": self.kind, "path": self.path, "id": "abcd1234"}, **kw)


# ------------------------------------------------------------------ claude

def stamp(offset=0.0):
    """An ISO-8601 UTC timestamp, as both agents write them, `offset` seconds
    from now. Z, and gmtime: a transcript is not written in the reader's
    timezone and must not be read as though it were."""
    return time.strftime("%Y-%m-%dT%H:%M:%S.000Z", time.gmtime(time.time() + offset))


def claude(type_, *blocks, **kw):
    """One claude transcript line carrying content blocks."""
    ts = kw.pop("ts", "2026-03-04T09:00:00.000Z")
    d = {"type": type_, "message": {"role": type_, "content": list(blocks)}}
    if ts is not None:
        d["timestamp"] = ts
    d.update(kw)
    return d


def said(type_, text, **kw):
    return claude(type_, {"type": "text", "text": text}, **kw)


def launched(tool_id, name="Agent", background=True):
    return claude("assistant", {
        "type": "tool_use", "id": tool_id, "name": name,
        "input": {"description": "go and look", "run_in_background": background}})


def acknowledged(tool_id):
    """What comes straight back from a background launch: not the work."""
    return claude("user", {
        "type": "tool_result", "tool_use_id": tool_id,
        "content": [{"type": "text", "text":
                     "Async agent launched successfully. (This tool result is "
                     "internal metadata — never quote or paste any part of it)"}]})


def returned(tool_id, text="here is what I found"):
    """What a synchronous agent gives back: the work itself."""
    return claude("user", {"type": "tool_result", "tool_use_id": tool_id,
                           "content": [{"type": "text", "text": text}]})


def notified(tool_id, task="a711c9edb2fa4fd7c"):
    """The completion turn, hours later, naming the call it answers."""
    return {"type": "user", "timestamp": "2026-03-04T11:00:00.000Z",
            "message": {"role": "user", "content":
                        "<task-notification> <task-id>%s</task-id> "
                        "<tool-use-id>%s</tool-use-id> "
                        "<output-file>/tmp/x/%s.output</output-file>"
                        % (task, tool_id, task)}}


class ClaudeStates(Transcript):
    """Whose turn it is, from the last thing in the transcript."""

    def test_an_agent_that_spoke_last_is_waiting_on_you(self):
        self.write(said("user", "have a look at the parser"))
        self.write(said("assistant", "done — two bugs, both in the lexer"))
        self.assertEqual("waiting", self.state())

    def test_an_agent_mid_tool_call_is_running(self):
        # The call is out and nothing has come back: the holdup is the tool,
        # not the model, which is the difference `age` then makes readable.
        self.write(said("user", "have a look at the parser"))
        self.write(claude("assistant", {"type": "tool_use", "id": "t1",
                                        "name": "Bash", "input": {}}))
        self.assertEqual("running", self.state())

    def test_a_tool_result_hands_the_work_back_to_the_agent(self):
        # The agent has the ball back and has not spoken yet.
        self.write(claude("assistant", {"type": "tool_use", "id": "t1",
                                        "name": "Bash", "input": {}}))
        self.write(returned("t1", "ok"))
        self.assertEqual("working", self.state())

    def test_the_last_call_of_a_batch_decides_it(self):
        # Parallel calls: one message, several tool_use blocks, and the
        # results back together. Anything outstanding means a tool is running.
        self.write(claude("assistant",
                          {"type": "tool_use", "id": "t1", "name": "Read",
                           "input": {}},
                          {"type": "tool_use", "id": "t2", "name": "Grep",
                           "input": {}}))
        self.assertEqual("running", self.state())
        self.write(claude("user",
                          {"type": "tool_result", "tool_use_id": "t1",
                           "content": "ok"},
                          {"type": "tool_result", "tool_use_id": "t2",
                           "content": "ok"}))
        self.assertEqual("working", self.state())

    def test_what_you_just_sent_is_the_agents_turn(self):
        self.write(said("assistant", "anything else?"))
        self.write(said("user", "yes — ship it"))
        self.assertEqual("working", self.state())

    def test_a_transcript_with_nothing_in_it_waits_on_you(self):
        # Started, never spoken to. The turn has to be someone's, and it is
        # not the agent's -- it has not been asked anything.
        self.write({"type": "mode", "mode": "normal"})
        self.assertEqual("waiting", self.state())

    def test_a_transcript_that_is_gone_is_gone(self):
        self.write(said("assistant", "hello"))
        os.remove(self.path)
        got = self.status()
        self.assertEqual("gone", got["state"])
        self.assertIsNone(got["age"])

    def test_the_age_is_reported_however_old_and_never_becomes_a_state(self):
        # The whole point of dropping idle: a turn handed back a week ago is
        # still a turn handed back.
        self.write(said("assistant", "done", ts=stamp(-7 * 86400)))
        got = self.status()
        self.assertEqual("waiting", got["state"])
        self.assertAlmostEqual(7 * 86400, got["age"], delta=90)


class Age(Transcript):
    """How long ago something happened here.

    It used to be the transcript's mtime, and mtime is not that: a live claude
    touches the file long after it stops writing to it -- four days after, on
    the machine this was found on -- so every quiet session dated itself to a
    few minutes ago and sorted to the top of the board above the ones that
    genuinely wanted something.
    """

    def test_the_conversation_dates_the_session_not_the_file(self):
        self.write(said("assistant", "done", ts=stamp(-7200)))
        os.utime(self.path, None)                  # touched, nothing written
        self.assertAlmostEqual(7200, self.status()["age"], delta=90)

    def test_bookkeeping_cannot_make_an_old_session_look_fresh(self):
        # Modes, titles, hook summaries: written whether or not anything
        # happened, and none of them is anything happening.
        self.write(said("assistant", "done", ts=stamp(-7200)))
        for kind in ("mode", "ai-title", "system", "file-history-snapshot"):
            self.write({"type": kind, "timestamp": stamp()})
        self.assertAlmostEqual(7200, self.status()["age"], delta=90)

    def test_the_newest_turn_dates_it_rather_than_the_last_line(self):
        # Claude interleaves lines timestamped before the turn they belong
        # to; reading in file order would let one of those wind the clock back.
        self.write(said("assistant", "done", ts=stamp(-60)))
        self.write(said("user", "an earlier line, written later", ts=stamp(-3600)))
        self.assertAlmostEqual(60, self.status()["age"], delta=30)

    def test_a_tail_carrying_no_timestamp_falls_back_to_the_file(self):
        # The only answer left, and better than none.
        self.write(said("assistant", "done", ts=None))
        old = time.time() - 1800
        os.utime(self.path, (old, old))
        self.assertAlmostEqual(1800, self.status()["age"], delta=90)

    def test_a_clock_behind_the_transcript_does_not_read_as_the_future(self):
        self.write(said("assistant", "done", ts=stamp(+600)))
        self.assertEqual(0, self.status()["age"])

    def test_a_codex_rollout_is_dated_the_same_way(self):
        self.kind = "codex"
        self.write({"timestamp": stamp(-4000),
                    "payload": {"type": "agent_message", "message": "shipped"}})
        os.utime(self.path, None)
        self.assertAlmostEqual(4000, self.status()["age"], delta=90)


class ClaudeDelegation(Transcript):
    """Waiting on agents of its own, which outranks waiting on you."""

    def test_a_launched_agent_is_outstanding(self):
        self.write(said("user", "explore both of these"))
        self.write(launched("toolu_A"))
        self.assertEqual("delegating", self.state())

    def test_the_launch_acknowledgement_is_not_the_answer(self):
        # It comes back in milliseconds and says only that the agent started.
        # Treating it as the result is how the orchestrator went quiet.
        self.write(launched("toolu_A"))
        self.write(acknowledged("toolu_A"))
        self.assertEqual("delegating", self.state())

    def test_handing_the_turn_back_does_not_end_the_delegation(self):
        # "I've launched two explorers, I'll report back" -- the agent spoke
        # last, but you are not what it is waiting on.
        self.write(launched("toolu_A"))
        self.write(acknowledged("toolu_A"))
        self.write(said("assistant", "two explorers are running; standing by"))
        self.assertEqual("delegating", self.state())

    def test_the_notification_ends_it(self):
        # Into `working`, not `waiting`: the answer has just landed on the
        # agent's side of the table and it is about to do something with it.
        self.write(launched("toolu_A"))
        self.write(acknowledged("toolu_A"))
        self.write(said("assistant", "two explorers are running; standing by"))
        self.write(notified("toolu_A"))
        self.assertEqual("working", self.state())
        self.write(said("assistant", "both reported — here is the plan"))
        self.assertEqual("waiting", self.state())

    def test_a_notification_is_not_something_you_said(self):
        # It is addressed to the agent. Left classed as your turn, the card's
        # "last thing either of you said" is a lump of markup.
        self.write(said("user", "explore both of these"))
        self.write(launched("toolu_A"))
        self.write(acknowledged("toolu_A"))
        self.write(notified("toolu_A"))
        self.assertEqual("explore both of these", self.status()["last"])

    def test_agents_are_paired_by_id_so_one_answer_is_not_all_of_them(self):
        self.write(launched("toolu_A"))
        self.write(launched("toolu_B"))
        self.write(acknowledged("toolu_A"))
        self.write(acknowledged("toolu_B"))
        self.write(notified("toolu_A"))
        self.assertEqual("delegating", self.state())
        self.write(notified("toolu_B"))
        self.assertEqual("working", self.state())

    def test_a_synchronous_agent_is_done_when_its_result_lands(self):
        # No notification is coming for this one: the result IS the work.
        self.write(launched("toolu_A", background=False))
        self.assertEqual("delegating", self.state())
        self.write(returned("toolu_A", "found it in the lexer"))
        self.assertEqual("working", self.state())

    def test_the_older_name_for_the_tool_counts_too(self):
        self.write(launched("toolu_A", name="Task", background=False))
        self.assertEqual("delegating", self.state())

    def test_an_ordinary_tool_is_not_a_delegation(self):
        # It is `running` -- a tool it is waiting on -- and not `delegating`,
        # which is agents of its own.
        self.write(claude("assistant", {"type": "tool_use", "id": "t1",
                                        "name": "Bash", "input": {}}))
        self.assertEqual("running", self.state())

    def test_a_launched_agent_outranks_the_call_that_launched_it(self):
        # The launch is a tool call like any other, and reporting it as one
        # would be the smaller truth: what the session is waiting on is the
        # agent, which is the answer that survives the acknowledgement.
        self.write(launched("toolu_A"))
        self.assertEqual("delegating", self.state())

    def test_a_transcript_that_merely_quotes_a_notification_ends_nothing(self):
        # Tool output is blocks, never a plain turn -- which is what keeps a
        # session reading its own logs (this suite does) from cancelling a
        # live delegation.
        self.write(launched("toolu_A"))
        self.write(acknowledged("toolu_A"))
        self.write(returned("t9", "grep found: <tool-use-id>toolu_A</tool-use-id>"))
        self.assertEqual("delegating", self.state())

    def test_a_launch_that_scrolled_out_of_the_tail_is_forgotten(self):
        # The tail is the horizon, deliberately: a session busy enough to push
        # the launch out of it is not one sitting still waiting for an answer.
        self.write(launched("toolu_A"))
        self.write(acknowledged("toolu_A"))
        for i in range(200):
            self.write(said("assistant", "line %d %s" % (i, "x" * 200)))
        self.assertEqual("waiting", self.state(tail_bytes=2000))


# ------------------------------------------------------------------- codex

def codex(type_, **payload):
    p = {"type": type_}
    p.update(payload)
    return {"timestamp": "2026-03-04T09:00:00.000Z", "payload": p}


def call(name, call_id):
    return codex("function_call", name=name, call_id=call_id, arguments="{}")


def output(call_id, out="{}"):
    return codex("function_call_output", call_id=call_id, output=out)


class CodexStates(Transcript):
    kind = "codex"

    def test_an_agent_that_spoke_last_is_waiting_on_you(self):
        self.write(codex("agent_message", message="that's landed on main"))
        self.assertEqual("waiting", self.state())

    def test_an_agent_mid_tool_call_is_running(self):
        self.write(call("exec", "call_1"))
        self.assertEqual("running", self.state())

    def test_the_output_hands_the_work_back_to_the_agent(self):
        self.write(call("exec", "call_1"))
        self.write(output("call_1", "ok"))
        self.assertEqual("working", self.state())


class CodexDelegation(Transcript):
    """Codex blocks in a tool, on a timeout, in a loop.

    Whether a wait call is outstanding at this instant flickers every few
    seconds, which is no use to a card someone is looking at -- so what is
    asked is which tool it keeps reaching for.
    """

    kind = "codex"

    def test_waiting_on_a_spawned_thread_is_delegating(self):
        self.write(call("spawn_agent", "call_1"))
        self.write(output("call_1", '{"task_name":"/root/review"}'))
        self.write(call("wait_agent", "call_2"))
        self.assertEqual("delegating", self.state())

    def test_the_wait_loop_does_not_flicker_between_polls(self):
        # wait_agent returns on a 1s timeout and is called straight back. The
        # gap between the two is not the agent going back to work.
        self.write(call("wait_agent", "call_1"))
        self.write(output("call_1", '{"timed_out":true}'))
        self.assertEqual("delegating", self.state())

    def test_going_back_to_work_ends_it(self):
        self.write(call("wait_agent", "call_1"))
        self.write(output("call_1", '{"task_name":"/root/review"}'))
        self.write(call("exec", "call_2"))
        self.assertEqual("running", self.state())

    def test_speaking_after_a_wait_still_reads_as_delegating(self):
        # Same rule as claude: an orchestrator narrating between waits is not
        # waiting on you.
        self.write(call("wait_agent", "call_1"))
        self.write(output("call_1", '{"timed_out":true}'))
        self.write(codex("agent_message", message="both reviewers still going"))
        self.assertEqual("delegating", self.state())

    def test_spawning_alone_is_not_waiting(self):
        # Between spawning and waiting, codex is doing its own work.
        self.write(call("spawn_agent", "call_1"))
        self.write(output("call_1", '{"task_name":"/root/review"}'))
        self.assertEqual("working", self.state())


class TheStatesThemselves(unittest.TestCase):
    """The vocabulary, as a whole.

    A state the client has no label for renders as a bare word, so the two
    lists have to agree -- and `idle` has to be gone from both.
    """

    def setUp(self):
        with open(os.path.join(REPO_ROOT, "share", "app.html")) as fh:
            self.app = fh.read()

    def test_every_state_the_server_can_send_has_a_label(self):
        labels = self.app.split("const label = {", 1)[1].split("};", 1)[0]
        for state in ("working", "running", "waiting", "delegating",
                      "blocked", "gone"):
            self.assertIn(state + ":", labels, state)

    def test_the_idle_state_is_gone_from_the_server(self):
        with open(os.path.join(REPO_ROOT, "bin", "ayeaye")) as fh:
            body = fh.read()
        self.assertNotIn('"idle"', body)
        self.assertNotIn("IDLE_AFTER", body)

    def test_the_idle_state_is_gone_from_the_client(self):
        labels = self.app.split("const label = {", 1)[1].split("};", 1)[0]
        self.assertNotIn("idle", labels)

    def test_delegating_is_styled_rather_than_falling_back_to_bare_text(self):
        self.assertIn(".st.delegating", self.app)

    def test_running_is_styled_too(self):
        self.assertIn(".st.running", self.app)


if __name__ == "__main__":
    unittest.main(verbosity=2)
