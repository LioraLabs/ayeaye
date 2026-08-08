#!/usr/bin/env python3
"""What `bin/ayeaye` may assume about the processes behind a tmux pane.

Session matching decides which transcript is shown for which pane, so the
things pinned here are the load-bearing ones: the arithmetic that turns
/proc/<pid>/stat field 22 into a wall-clock start time, the ancestry walk that
finds the agent below the pane's shell, and the window in which a codex rollout
is accepted as belonging to a process. Getting any of them wrong shows someone
else's conversation, quietly.

Nothing here needs tmux, a codex process or even the platform it is testing:
the Linux backend is pointed at a fake /proc tree on disk, and the macOS one is
fed canned `ps` and `lsof` bytes. Both therefore run everywhere, which is the
only way macOS coverage exists at all on a Linux host.

Driven by tests/cases/process_inspect_test.sh, one bash test per test below.
Run directly for the whole file, or with a test id:

    tests/python/process_inspect.py
    tests/python/process_inspect.py LinuxProcTest.test_cwd_reads_the_symlink
"""
import builtins
import json
import os
import shutil
import sys
import tempfile
import time
import unittest
from importlib.machinery import SourceFileLoader
from importlib.util import module_from_spec, spec_from_file_location

# Importing the code under test would otherwise leave a __pycache__ inside
# the checkout, and a suite that writes into the tree it is testing has
# already broken the promise it makes about where it may write.
sys.dont_write_bytecode = True

REPO_ROOT = os.environ.get("REPO_ROOT") or os.path.dirname(
    os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
FIXTURES_DIR = os.environ.get("FIXTURES_DIR") or os.path.join(
    REPO_ROOT, "tests", "fixtures")

# Loaded, not imported: bin/ayeaye has no .py extension because it is a
# command. A token is already in the environment so the load does not generate
# one and write it to disk.
#
# exec_module rather than the load_module() that goes away in python 3.15, and
# a named loader because a command has no .py extension for the machinery to
# recognise from. Registered in sys.modules before it is executed, which is
# what load_module() did and what SharedModuleTest below depends on.
os.environ.setdefault("AYEAYE_TOKEN", "test-token")
_AYEAYE_NAME = "ayeaye_under_test"
_AYEAYE_PATH = os.path.join(REPO_ROOT, "bin", "ayeaye")
_ayeaye_spec = spec_from_file_location(
    _AYEAYE_NAME, _AYEAYE_PATH,
    loader=SourceFileLoader(_AYEAYE_NAME, _AYEAYE_PATH))
ayeaye = module_from_spec(_ayeaye_spec)
sys.modules[_AYEAYE_NAME] = ayeaye
_ayeaye_spec.loader.exec_module(ayeaye)

CLK_TCK = os.sysconf("SC_CLK_TCK")


def fixture(name):
    """The bytes of tests/fixtures/<name>, as text."""
    with open(os.path.join(FIXTURES_DIR, name)) as fh:
        return fh.read()


class TempTree(unittest.TestCase):
    """A scratch directory that goes away with the test."""

    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="ayeaye-proc-")
        self.addCleanup(shutil.rmtree, self.tmp, True)


# --------------------------------------------------------------- fake /proc

def write_proc(root, pid, name="bash", ppid=1, start_ticks=0, cwd=None):
    """One process in a fake /proc tree, shaped like the real thing.

    The stat line matters more than it looks: the comm sits in parentheses and
    may itself contain one, everything before the last `)` is discarded, and
    starttime is the twentieth field of what is left. Padding after it is
    deliberate -- a parser that took the last field instead of the twentieth
    would pass without it.
    """
    d = os.path.join(root, str(pid))
    os.makedirs(d, exist_ok=True)
    after = ["S", str(ppid)] + ["0"] * 17 + [str(start_ticks)] + ["7"] * 30
    with open(os.path.join(d, "stat"), "w") as fh:
        fh.write("%s (%s) %s\n" % (pid, name, " ".join(after)))
    with open(os.path.join(d, "comm"), "w") as fh:
        fh.write(name + "\n")
    if cwd is not None:
        link = os.path.join(d, "cwd")
        if os.path.islink(link):
            os.unlink(link)
        os.symlink(cwd, link)
    return d


def write_uptime(root, seconds):
    with open(os.path.join(root, "uptime"), "w") as fh:
        fh.write("%s 9876.54\n" % seconds)


def write_fds(root, pid, *targets):
    """/proc/<pid>/fd: one numbered symlink per open file.

    Real ones point at anything -- sockets, pipes, deleted files -- and are
    named by descriptor number, which is why the numbering starts where a
    process's first open file really would rather than at zero.
    """
    d = os.path.join(write_proc(root, pid), "fd")
    os.makedirs(d, exist_ok=True)
    for n, target in enumerate(targets, start=3):
        link = os.path.join(d, str(n))
        if os.path.islink(link):
            os.unlink(link)
        os.symlink(target, link)
    return d


def write_environ(root, pid, **env):
    """/proc/<pid>/environ: NUL-separated, NUL-terminated, no newline.

    The separator is the whole point. A value may contain spaces -- an
    SSH_CONNECTION always does -- so anything that split this on whitespace
    would read the port as a variable.
    """
    d = write_proc(root, pid)
    blob = b"".join(("%s=%s\0" % (k, v)).encode() for k, v in env.items())
    with open(os.path.join(d, "environ"), "wb") as fh:
        fh.write(blob)
    return d


def pgrep_runner(tree):
    """Stand in for `pgrep -P`, answering from a {ppid: [pid]} dict."""
    def run(argv, **kw):
        if argv[:2] == ["pgrep", "-P"]:
            return "".join("%s\n" % c for c in tree.get(str(argv[2]), []))
        return None
    return run


def canned(responses):
    """A tool runner answering from {argv-joined-prefix: stdout}."""
    def run(argv, **kw):
        joined = " ".join(argv)
        for prefix in sorted(responses, key=len, reverse=True):
            if joined.startswith(prefix):
                return responses[prefix]
        return None
    return run


class LinuxProcTest(TempTree):
    """The /proc backend, against a fake tree. This is the regression pin:
    every number here is what the pre-refactor code produced."""

    def linux(self, run=None, tree=None):
        return ayeaye._LinuxProcessInfo(
            proc=self.tmp, run=run or pgrep_runner(tree or {}), clk_tck=CLK_TCK)

    def test_start_time_is_boot_time_plus_field_22(self):
        # Up for 1000s, started 400s after boot -> began 600s ago.
        write_proc(self.tmp, 42, start_ticks=400 * CLK_TCK)
        write_uptime(self.tmp, 1000.0)
        got = self.linux().start_time(42)
        self.assertAlmostEqual(time.time() - 600.0, got, delta=0.05)

    def test_start_time_keeps_sub_second_resolution(self):
        write_proc(self.tmp, 42, start_ticks=400 * CLK_TCK + CLK_TCK // 4)
        write_uptime(self.tmp, 1000.0)
        got = self.linux().start_time(42)
        self.assertAlmostEqual(time.time() - 599.75, got, delta=0.05)

    def test_start_time_survives_a_parenthesis_in_the_process_name(self):
        write_proc(self.tmp, 42, name="co)de(x", start_ticks=100 * CLK_TCK)
        write_uptime(self.tmp, 1000.0)
        got = self.linux().start_time(42)
        self.assertAlmostEqual(time.time() - 900.0, got, delta=0.05)

    def test_start_time_is_none_when_the_process_is_gone(self):
        write_uptime(self.tmp, 1000.0)
        self.assertIsNone(self.linux().start_time(4242))

    def test_start_time_is_none_when_stat_is_truncated(self):
        os.makedirs(os.path.join(self.tmp, "42"))
        with open(os.path.join(self.tmp, "42", "stat"), "w") as fh:
            fh.write("42 (bash) S 1\n")
        write_uptime(self.tmp, 1000.0)
        self.assertIsNone(self.linux().start_time(42))

    def test_cwd_reads_the_symlink(self):
        write_proc(self.tmp, 42, cwd="/home/someone/dev/thing")
        self.assertEqual("/home/someone/dev/thing", self.linux().cwd(42))

    def test_cwd_is_none_when_it_cannot_be_read(self):
        write_proc(self.tmp, 42)
        self.assertIsNone(self.linux().cwd(42))

    def test_open_files_reads_every_descriptor_symlink(self):
        write_fds(self.tmp, 42, "/home/someone/one.jsonl",
                  "/home/someone/two.jsonl")
        self.assertEqual({"/home/someone/one.jsonl", "/home/someone/two.jsonl"},
                         set(self.linux().open_files(42)))

    def test_open_files_keeps_a_path_with_a_space_in_it_whole(self):
        write_fds(self.tmp, 42, "/home/someone/My Sessions/one.jsonl")
        self.assertEqual(["/home/someone/My Sessions/one.jsonl"],
                         self.linux().open_files(42))

    def test_open_files_is_empty_for_a_process_that_is_gone(self):
        self.assertEqual([], self.linux().open_files(4242))

    def test_a_descriptor_closing_mid_walk_does_not_empty_the_answer(self):
        # The process under inspection is running, so files close while the
        # directory is being read. Answering "holds nothing open" there would
        # send the pane to the fallback for no reason.
        write_fds(self.tmp, 42, "/home/someone/one.jsonl")
        os.symlink("/home/someone/two.jsonl",
                   os.path.join(self.tmp, "42", "fd", "9"))
        real = os.readlink

        def vanishing(path, *a, **kw):
            if path.endswith("/fd/9"):
                raise OSError("closed while we looked")
            return real(path, *a, **kw)

        os.readlink = vanishing
        self.addCleanup(setattr, os, "readlink", real)
        self.assertEqual(["/home/someone/one.jsonl"],
                         self.linux().open_files(42))

    def test_comm_is_the_stripped_contents(self):
        write_proc(self.tmp, 42, name="codex")
        self.assertEqual("codex", self.linux().comm(42))

    def test_comm_is_none_for_a_process_that_vanished(self):
        self.assertIsNone(self.linux().comm(4242))

    def test_a_comm_that_is_not_utf8_does_not_end_the_request(self):
        # comm is the tail of whatever path was exec'd, bytes and all. The
        # blast radius is not one pane: overview() resolves every pane in a
        # single request, so one odd process anywhere turns the board into a
        # 500.
        write_proc(self.tmp, 42, name="bash")
        with open(os.path.join(self.tmp, "42", "comm"), "wb") as fh:
            fh.write(b"co\xffdex\n")
        got = self.linux().comm(42)
        self.assertIsNotNone(got)
        self.assertNotEqual("codex", got)

    def test_an_undecodable_sibling_does_not_hide_the_agent(self):
        write_proc(self.tmp, 100, name="bash")
        write_proc(self.tmp, 101, name="odd", ppid=100)
        write_proc(self.tmp, 102, name="codex", ppid=100)
        with open(os.path.join(self.tmp, "101", "comm"), "wb") as fh:
            fh.write(b"\xff\xfe\n")
        got = self.linux(tree={"100": [101, 102]}).descendant(100, "codex")
        self.assertEqual("102", got)

    def test_descendant_finds_a_direct_child(self):
        write_proc(self.tmp, 100, name="bash")
        write_proc(self.tmp, 101, name="codex", ppid=100)
        got = self.linux(tree={"100": [101]}).descendant(100, "codex")
        self.assertEqual("101", got)

    def test_descendant_finds_a_grandchild_through_a_wrapper(self):
        write_proc(self.tmp, 100, name="bash")
        write_proc(self.tmp, 101, name="npx", ppid=100)
        write_proc(self.tmp, 102, name="codex", ppid=101)
        got = self.linux(tree={"100": [101], "101": [102]}).descendant(100, "codex")
        self.assertEqual("102", got)

    def test_descendant_stops_at_the_depth_limit(self):
        tree = {"100": [101], "101": [102], "102": [103], "103": [104]}
        for pid, name in ((100, "bash"), (101, "a"), (102, "b"),
                          (103, "c"), (104, "codex")):
            write_proc(self.tmp, pid, name=name)
        insp = self.linux(tree=tree)
        self.assertIsNone(insp.descendant(100, "codex"))
        self.assertEqual("104", insp.descendant(100, "codex", depth=4))

    def test_a_child_with_no_readable_comm_is_skipped_and_not_expanded(self):
        # 101 exists as a pid but its comm cannot be read. The pre-refactor
        # walk dropped such a child entirely rather than descending into it,
        # so the codex below it stays invisible.
        write_proc(self.tmp, 100, name="bash")
        write_proc(self.tmp, 102, name="codex", ppid=101)
        tree = {"100": [101], "101": [102]}
        self.assertIsNone(self.linux(tree=tree).descendant(100, "codex"))

    def test_descendant_is_none_when_pgrep_cannot_run(self):
        write_proc(self.tmp, 100, name="bash")
        self.assertIsNone(
            self.linux(run=lambda argv, **kw: None).descendant(100, "codex"))

    # ------------------------------------------- the two client questions

    def test_exists_is_true_for_a_pid_with_a_directory_under_proc(self):
        write_proc(self.tmp, 42)
        self.assertTrue(self.linux().exists(42))
        self.assertTrue(self.linux().exists("42"))

    def test_exists_is_false_for_a_pid_that_is_not_there(self):
        write_proc(self.tmp, 42)
        self.assertFalse(self.linux().exists(4242))

    def test_the_ssh_peer_is_the_first_field_of_ssh_connection(self):
        write_environ(self.tmp, 42, SSH_CONNECTION="10.1.2.3 54321 10.0.0.1 22")
        self.assertEqual("10.1.2.3", self.linux().ssh_peer_ip(42))

    def test_a_process_with_no_ssh_connection_has_no_peer(self):
        write_environ(self.tmp, 42, TERM="xterm-256color")
        self.assertIsNone(self.linux().ssh_peer_ip(42))

    def test_an_ssh_connection_with_no_fields_has_no_peer(self):
        write_environ(self.tmp, 42, SSH_CONNECTION="")
        self.assertIsNone(self.linux().ssh_peer_ip(42))

    def test_a_variable_whose_name_merely_ends_in_it_is_not_it(self):
        # environ is NUL-separated and every entry is matched whole, so
        # OLD_SSH_CONNECTION must not answer for SSH_CONNECTION.
        write_environ(self.tmp, 42, OLD_SSH_CONNECTION="10.9.9.9 1 2 3")
        self.assertIsNone(self.linux().ssh_peer_ip(42))

    def test_a_peer_lookup_on_a_process_that_is_gone_is_not_an_error(self):
        self.assertIsNone(self.linux().ssh_peer_ip(4242))

    def test_a_peer_lookup_that_is_refused_is_not_an_error(self):
        # /proc/<pid>/environ belongs to its owner and nobody else. A tmux
        # client is the same user, but a pid tmux handed over can have been
        # reused by anything at all by the time it is looked at.
        if os.geteuid() == 0:
            self.skipTest("root reads it regardless")
        d = write_environ(self.tmp, 42, SSH_CONNECTION="10.1.2.3 1 2 3")
        path = os.path.join(d, "environ")
        os.chmod(path, 0o000)
        self.addCleanup(os.chmod, path, 0o600)
        self.assertIsNone(self.linux().ssh_peer_ip(42))

    def test_an_undecodable_environment_does_not_cost_the_peer(self):
        # environ is bytes and nothing promises they are UTF-8. One odd
        # variable elsewhere must not lose the address.
        d = write_proc(self.tmp, 42)
        with open(os.path.join(d, "environ"), "wb") as fh:
            fh.write(b"ODD=\xff\xfe\x00SSH_CONNECTION=10.1.2.3 1 2 3\x00")
        self.assertEqual("10.1.2.3", self.linux().ssh_peer_ip(42))

    def test_the_live_proc_parsing_matches_the_reference_implementation(self):
        """The pin that a fake tree cannot give: real /proc, real numbers.

        The reference below is the pre-refactor body, copied verbatim. If the
        backend and it ever disagree the refactor moved something.
        """
        if not sys.platform.startswith("linux") or not os.path.isdir("/proc"):
            self.skipTest("no /proc on this host")

        def reference(pid):
            with open("/proc/%s/stat" % pid) as fh:
                fields = fh.read().rsplit(")", 1)[1].split()
            with open("/proc/uptime") as fh:
                uptime = float(fh.read().split()[0])
            return ((time.time() - uptime)
                    + float(fields[19]) / os.sysconf("SC_CLK_TCK"))

        live = ayeaye._LinuxProcessInfo()
        for pid in (os.getpid(), os.getppid(), 1):
            # /proc/uptime has 10ms resolution, and the two readings of it are
            # not the same reading, so the budget is that plus scheduling --
            # still four orders of magnitude tighter than any change of field.
            self.assertAlmostEqual(reference(pid), live.start_time(pid),
                                   delta=0.05, msg="pid %s" % pid)
        self.assertEqual(os.readlink("/proc/%s/cwd" % os.getpid()),
                         live.cwd(os.getpid()))
        with open("/proc/%s/comm" % os.getpid()) as fh:
            self.assertEqual(fh.read().strip(), live.comm(os.getpid()))


# ------------------------------------------------------------------- codex

class FakeProcessInfo(object):
    """A process tree with no processes in it.

    `holds` defaults to nothing, which is what a codex too old to keep its
    rollout open looks like -- so every test that does not set it is testing
    the fallback, deliberately.
    """

    def __init__(self, pid=None, started=None, cwd=None, holds=()):
        self._pid, self._started, self._cwd = pid, started, cwd
        self._holds = list(holds)
        self.asked = []

    def descendant(self, pid, name, depth=3):
        self.asked.append(("descendant", str(pid), name))
        return self._pid

    def start_time(self, pid):
        return self._started

    def cwd(self, pid):
        return self._cwd

    def open_files(self, pid):
        return list(self._holds)


class CodexSessionTest(TempTree):
    """Which rollout belongs to which codex process."""

    def setUp(self):
        TempTree.setUp(self)
        self.sessions = os.path.join(self.tmp, "sessions")
        self._saved = ayeaye.CODEX_SESSIONS
        ayeaye.CODEX_SESSIONS = self.sessions
        self.addCleanup(setattr, ayeaye, "CODEX_SESSIONS", self._saved)
        self.started = time.mktime(time.strptime("2026-03-04T09-00-00",
                                                 "%Y-%m-%dT%H-%M-%S"))

    def rollout(self, stamp, cwd, sid="0123abcd-dead-beef", first=None):
        d = os.path.join(self.sessions, "2026", "03", "04")
        os.makedirs(d, exist_ok=True)
        path = os.path.join(d, "rollout-%s-%s.jsonl" % (stamp, sid))
        line = first if first is not None else json.dumps(
            {"timestamp": stamp, "type": "session_meta",
             "payload": {"id": sid, "cwd": cwd, "cli_version": "0.145.0"}})
        with open(path, "w") as fh:
            fh.write(line + "\n")
        return path

    def resolve(self, **kw):
        info = FakeProcessInfo(pid="991", started=self.started,
                               cwd="/home/someone/dev/thing", **kw)
        return ayeaye.codex_session_for("900", proc=info), info

    def test_a_rollout_written_just_after_the_process_started_wins(self):
        path = self.rollout("2026-03-04T09-00-02", "/home/someone/dev/thing")
        got, _ = self.resolve()
        self.assertEqual({"kind": "codex", "id": "0123abcd", "path": path}, got)

    def test_the_agent_is_looked_for_below_the_pane_shell(self):
        self.rollout("2026-03-04T09-00-02", "/home/someone/dev/thing")
        _, info = self.resolve()
        self.assertEqual([("descendant", "900", "codex")], info.asked)

    def test_a_rollout_from_another_directory_is_not_this_session(self):
        self.rollout("2026-03-04T09-00-02", "/home/someone/dev/other")
        got, _ = self.resolve()
        self.assertIsNone(got)

    def test_a_rollout_written_long_before_the_process_is_ignored(self):
        self.rollout("2026-03-04T08-59-50", "/home/someone/dev/thing")
        got, _ = self.resolve()
        self.assertIsNone(got)

    def test_a_rollout_written_long_after_the_process_is_ignored(self):
        self.rollout("2026-03-04T09-02-01", "/home/someone/dev/thing")
        got, _ = self.resolve()
        self.assertIsNone(got)

    def test_the_window_edges_are_inclusive(self):
        self.rollout("2026-03-04T08-59-55", "/home/someone/dev/thing")
        self.assertIsNotNone(self.resolve()[0])
        shutil.rmtree(self.sessions)
        self.rollout("2026-03-04T09-02-00", "/home/someone/dev/thing")
        self.assertIsNotNone(self.resolve()[0])

    def test_the_nearest_rollout_wins_when_two_could_match(self):
        late = self.rollout("2026-03-04T09-00-30", "/home/someone/dev/thing",
                            sid="ffffffff-late")
        near = self.rollout("2026-03-04T09-00-01", "/home/someone/dev/thing",
                            sid="aaaaaaaa-near")
        # Both orders, because glob returns directory order and directory
        # order is the filesystem's business: a "first hit wins" bug would
        # otherwise pass or fail depending on the machine.
        for order in ([near, late], [late, near]):
            saved = ayeaye.glob.glob
            ayeaye.glob.glob = lambda pattern, _o=order: list(_o)
            try:
                got, _ = self.resolve()
            finally:
                ayeaye.glob.glob = saved
            self.assertEqual(near, got["path"], "glob order %s" % (order,))
            self.assertEqual("aaaaaaaa", got["id"])

    def test_no_codex_below_the_pane_means_no_session(self):
        self.rollout("2026-03-04T09-00-02", "/home/someone/dev/thing")
        info = FakeProcessInfo(pid=None)
        self.assertIsNone(ayeaye.codex_session_for("900", proc=info))

    def test_a_start_time_that_cannot_be_read_means_no_session(self):
        self.rollout("2026-03-04T09-00-02", "/home/someone/dev/thing")
        info = FakeProcessInfo(pid="991", started=None,
                               cwd="/home/someone/dev/thing")
        self.assertIsNone(ayeaye.codex_session_for("900", proc=info))

    def test_a_cwd_that_cannot_be_read_means_no_session(self):
        self.rollout("2026-03-04T09-00-02", "/home/someone/dev/thing")
        info = FakeProcessInfo(pid="991", started=self.started, cwd=None)
        self.assertIsNone(ayeaye.codex_session_for("900", proc=info))

    def test_an_unreadable_cwd_does_not_match_a_rollout_that_recorded_none(self):
        # Both sides absent is not both sides equal. Without the explicit
        # guard, "we could not read the cwd" matches "the rollout did not
        # record one" and the pane is handed a conversation at random.
        self.rollout("2026-03-04T09-00-02", None, first=json.dumps(
            {"payload": {"id": "0123abcd-dead-beef"}}))
        info = FakeProcessInfo(pid="991", started=self.started, cwd=None)
        self.assertIsNone(ayeaye.codex_session_for("900", proc=info))

    def test_a_rollout_whose_first_line_is_not_json_is_skipped(self):
        self.rollout("2026-03-04T09-00-01", "/home/someone/dev/thing",
                     sid="bbbbbbbb-bad", first="not json at all")
        good = self.rollout("2026-03-04T09-00-05", "/home/someone/dev/thing",
                            sid="cccccccc-good")
        got, _ = self.resolve()
        self.assertEqual(good, got["path"])

    def test_a_rollout_with_an_unparsable_timestamp_is_skipped(self):
        self.rollout("2026-03-04TZZ-00-01", "/home/someone/dev/thing")
        got, _ = self.resolve()
        self.assertIsNone(got)

    def test_a_started_process_with_nothing_written_yet_has_no_session(self):
        got, _ = self.resolve()
        self.assertIsNone(got)


class CodexHeldRolloutTest(CodexSessionTest):
    """The rollout codex is holding open, which beats guessing from a clock.

    Everything the timestamp match infers, an open descriptor states: the
    kernel is asked which file this process has. The case that makes it worth
    having is a resumed session, whose rollout was written days before the
    process that reopened it and therefore sits outside any window around a
    start time.
    """

    def held(self, *paths, **kw):
        """Resolve with those paths held open, and a start time far from all
        of them so nothing can pass by the timestamp route instead."""
        info = FakeProcessInfo(pid="991", started=self.started + 86400,
                               cwd="/home/someone/dev/thing", holds=paths, **kw)
        return ayeaye.codex_session_for("900", proc=info), info

    def subagent(self, stamp, sid, parent="0123abcd-dead-beef"):
        """A rollout codex is keeping open on a spawned thread's behalf."""
        return self.rollout(stamp, "/home/someone/dev/thing", sid=sid,
                            first=json.dumps({
                                "type": "session_meta",
                                "payload": {"id": sid, "session_id": parent,
                                            "parent_thread_id": parent,
                                            "thread_source": "subagent",
                                            "cwd": "/home/someone/dev/thing"}}))

    def test_a_held_rollout_resolves_a_session_older_than_its_process(self):
        # `codex resume`: the file is a week old and the process is minutes
        # old. No window around a start time will ever contain it.
        path = self.rollout("2026-02-25T11-30-00", "/home/someone/dev/thing")
        got, _ = self.held(path)
        self.assertEqual({"kind": "codex", "id": "0123abcd", "path": path}, got)

    def test_a_rollout_held_for_a_subagent_is_not_this_pane_session(self):
        # Codex keeps the rollouts of the threads it spawned open alongside
        # its own. The session in the pane is the one with no parent.
        child = self.subagent("2026-03-04T14-19-02", "ffffffff-sub")
        mine = self.rollout("2026-03-04T09-00-02", "/home/someone/dev/thing")
        got, _ = self.held(child, mine)
        self.assertEqual(mine, got["path"])
        self.assertEqual("0123abcd", got["id"])

    def test_holding_nothing_but_subagents_is_not_a_session(self):
        child = self.subagent("2026-03-04T14-19-02", "ffffffff-sub")
        got, _ = self.held(child)
        self.assertIsNone(got)

    def test_a_file_held_outside_the_sessions_tree_is_not_a_rollout(self):
        # A codex agent has files of its own open all day. Only what is under
        # the sessions directory can be a transcript.
        outside = os.path.join(self.tmp, "rollout-2026-03-04T09-00-02-ffff.jsonl")
        with open(outside, "w") as fh:
            fh.write(json.dumps({"payload": {"id": "ffffffff-nope"}}) + "\n")
        got, _ = self.held(outside)
        self.assertIsNone(got)

    def test_an_ordinary_file_in_the_sessions_tree_is_not_a_rollout(self):
        stray = os.path.join(self.sessions, "notes.jsonl")
        os.makedirs(self.sessions, exist_ok=True)
        with open(stray, "w") as fh:
            fh.write(json.dumps({"payload": {"id": "ffffffff-nope"}}) + "\n")
        got, _ = self.held(stray)
        self.assertIsNone(got)

    def test_the_last_written_wins_when_two_could_be_this_session(self):
        old = self.rollout("2026-03-04T09-00-02", "/home/someone/dev/thing",
                           sid="aaaaaaaa-old")
        new = self.rollout("2026-03-04T09-00-03", "/home/someone/dev/thing",
                           sid="bbbbbbbb-new")
        os.utime(old, (1_700_000_000, 1_700_000_000))
        os.utime(new, (1_700_000_900, 1_700_000_900))
        # Both orders: which descriptor comes back first is the kernel's
        # business, and an answer that depends on it is not an answer.
        for order in ((old, new), (new, old)):
            got, _ = self.held(*order)
            self.assertEqual(new, got["path"], "order %s" % (order,))

    def test_the_timestamp_match_still_answers_when_nothing_is_held(self):
        # An older codex did not reliably hold the rollout open, and the
        # fallback is the whole reason it is still there.
        path = self.rollout("2026-03-04T09-00-02", "/home/someone/dev/thing")
        got, _ = self.resolve()
        self.assertEqual(path, got["path"])

    def test_a_held_rollout_that_cannot_be_read_falls_back(self):
        held = self.rollout("2026-02-25T11-30-00", "/home/someone/dev/thing",
                            sid="dddddddd-unreadable", first="{ not json")
        near = self.rollout("2026-03-04T09-00-02", "/home/someone/dev/thing",
                            sid="eeeeeeee-near")
        info = FakeProcessInfo(pid="991", started=self.started,
                               cwd="/home/someone/dev/thing", holds=[held])
        got = ayeaye.codex_session_for("900", proc=info)
        self.assertEqual(near, got["path"])


# ------------------------------------------------------------------ darwin

class DarwinProcTest(TempTree):
    """The macOS backend, fed the bytes `ps` and `lsof` really produce.

    None of this can be executed on the host that runs it, so the fixtures are
    the contract: they are shaped from the real output formats and are the only
    thing standing between this code and a machine nobody here can boot.
    """

    def darwin(self, responses):
        return ayeaye._DarwinProcessInfo(run=canned(responses))

    def ps_tree(self):
        return {"ps -axww": fixture("ps/darwin-codex-tree")}

    def record(self):
        """A runner that answers with the tree and remembers being asked."""
        class Recorder(object):
            def __init__(self):
                self.argv = []

            def run(self, argv, **kw):
                self.argv.append(argv)
                return fixture("ps/darwin-codex-tree")

        return Recorder()

    def test_a_direct_child_is_found_in_the_ps_snapshot(self):
        got = self.darwin(self.ps_tree()).descendant(701, "codex")
        self.assertEqual("702", got)

    def test_a_grandchild_is_found_through_a_wrapper(self):
        got = self.darwin(self.ps_tree()).descendant(801, "codex")
        self.assertEqual("803", got)

    def test_a_full_path_in_comm_matches_a_bare_agent_name(self):
        # macOS ps reports /opt/homebrew/bin/codex where Linux reports codex.
        info = self.darwin(self.ps_tree())
        self.assertEqual("codex", info.comm(702))

    def test_a_process_whose_path_contains_a_space_is_read_whole(self):
        # /Applications/My Editor.app/... is not exotic on a Mac, and a
        # snapshot that splits on every space mangles the tree from there on.
        info = self.darwin(self.ps_tree())
        self.assertEqual("My Editor", info.comm(951))
        self.assertEqual("952", info.descendant(950, "codex"))

    def test_a_pane_with_no_agent_below_it_resolves_to_nothing(self):
        self.assertIsNone(self.darwin(self.ps_tree()).descendant(901, "codex"))

    def test_an_agent_below_a_long_interpreter_path_is_still_found(self):
        # An nvm install puts codex 95 columns deep. BSD ps truncates to the
        # terminal width, and to 79 columns when there is no terminal -- which
        # is always, under capture_output -- so without -ww the basename here
        # is a fragment of the path and matches nothing.
        info = self.darwin(self.ps_tree())
        self.assertEqual("codex", info.comm(961))
        self.assertEqual("961", info.descendant(960, "codex"))

    def test_the_snapshot_asks_ps_in_the_form_bsd_understands(self):
        calls = self.record()
        ayeaye._DarwinProcessInfo(run=calls.run).descendant(801, "codex")
        self.assertEqual(1, len(calls.argv), "the whole walk costs one ps")
        argv = calls.argv[0]
        # An `=` makes the rest of a BSD -o argument the column header, so
        # "pid=,ppid=,comm=" asks for one column with a silly name and yields
        # a tree with no parents in it. One keyword per -o, and unlimited
        # width, are what both implementations agree on.
        self.assertEqual(["ps", "-axww", "-o", "pid=", "-o", "ppid=",
                          "-o", "comm="], argv)
        self.assertEqual([], [a for a in argv if "," in a],
                         "no -o argument may carry a comma-joined list")

    def test_a_repeated_walk_takes_a_fresh_snapshot(self):
        calls = self.record()
        info = ayeaye._DarwinProcessInfo(run=calls.run)
        info.descendant(801, "codex")
        info.descendant(801, "codex")
        self.assertEqual(2, len(calls.argv))

    def test_a_walk_leaves_no_process_tree_behind_on_the_backend(self):
        # One backend is shared by every thread of a threading server. A tree
        # cached on the instance is a tree another request can be halfway
        # through replacing, and the pane that lands on it resolves to the
        # wrong agent or to none at all.
        calls = self.record()
        info = ayeaye._DarwinProcessInfo(run=calls.run)
        info.descendant(801, "codex")
        self.assertEqual(["run"], sorted(vars(info)))

    def test_a_row_ps_gave_no_name_for_is_kept_with_an_empty_one(self):
        # A row with no third column. It still has a pid, and a pid can have
        # children worth walking, so it is not dropped.
        self.assertEqual("", self.darwin(self.ps_tree()).comm(111))

    def test_the_accounting_names_of_other_users_processes_parse(self):
        # Most rows on a real Mac look like this: ps cannot read the argument
        # buffer of a process it does not own and falls back to the
        # parenthesised 16-character accounting name.
        info = self.darwin(self.ps_tree())
        self.assertEqual("(logd)", info.comm(222))
        self.assertEqual("(mdworker_shared)", info.comm(333))

    def test_a_header_line_is_not_mistaken_for_a_process(self):
        info = self.darwin({"ps -axww": fixture("ps/darwin-with-header")})
        self.assertEqual("702", info.descendant(701, "codex"))
        self.assertIsNone(info.comm("PID"))

    def test_an_undecodable_path_elsewhere_does_not_empty_the_tree(self):
        raw = fixture("ps/darwin-codex-tree").replace(
            "/usr/bin/vim", "/Users/someone/b�d/vim")
        info = self.darwin({"ps -axww": raw})
        self.assertEqual("702", info.descendant(701, "codex"))

    def test_start_time_comes_from_ps_lstart(self):
        info = self.darwin({"ps -ww": fixture("ps/darwin-lstart")})
        expected = time.mktime(time.strptime("Wed Mar 4 09:00:02 2026",
                                             "%a %b %d %H:%M:%S %Y"))
        self.assertEqual(expected, info.start_time(702))

    def test_start_time_parses_a_space_padded_day_of_month(self):
        info = self.darwin({"ps -ww": fixture("ps/darwin-lstart-padded")})
        expected = time.mktime(time.strptime("Tue Jul 7 04:05:06 2026",
                                             "%a %b %d %H:%M:%S %Y"))
        self.assertEqual(expected, info.start_time(702))

    def test_the_two_per_process_probes_ask_for_what_they_need(self):
        # The canned runner matches on an argv prefix, so without this the
        # flags and the pid could both be wrong and nothing would notice --
        # on the platform that cannot be run here.
        seen = []

        def run(argv, **kw):
            seen.append(argv)
            return ""

        info = ayeaye._DarwinProcessInfo(run=run)
        info.start_time(702)
        info.cwd(702)
        self.assertEqual(["ps", "-ww", "-p", "702", "-o", "lstart="], seen[0])
        # -a is what ANDs -d with -p. Without it lsof ORs them and the first
        # n line can be some other process's working directory entirely.
        self.assertEqual(["lsof", "-a", "-d", "cwd", "-p", "702", "-Fn"],
                         seen[1])

    def test_start_time_is_none_when_ps_says_nothing(self):
        self.assertIsNone(self.darwin({"ps -ww": ""}).start_time(702))
        self.assertIsNone(self.darwin({}).start_time(702))
        self.assertIsNone(self.darwin({"ps -ww": "not a date\n"}).start_time(702))

    def test_cwd_comes_from_the_lsof_field_output(self):
        info = self.darwin({"lsof": fixture("lsof/darwin-cwd")})
        self.assertEqual("/Users/someone/dev/thing", info.cwd(702))

    def test_cwd_is_none_when_lsof_is_missing_or_says_nothing(self):
        self.assertIsNone(self.darwin({}).cwd(702))
        self.assertIsNone(self.darwin({"lsof": ""}).cwd(702))
        self.assertIsNone(
            self.darwin({"lsof": fixture("lsof/darwin-denied")}).cwd(702))

    def test_a_cwd_containing_a_space_survives(self):
        # ~/My Projects/... is entirely ordinary on a Mac, and the field
        # output is exactly what makes it safe to read.
        info = self.darwin({"lsof": fixture("lsof/darwin-cwd-spaces")})
        self.assertEqual("/Users/someone/My Projects/thing", info.cwd(702))

    def test_open_files_comes_from_every_name_record(self):
        info = self.darwin({"lsof -p": fixture("lsof/darwin-open-files")})
        got = info.open_files(702)
        self.assertIn("/Users/someone/.codex/sessions/2026/03/04/"
                      "rollout-2026-03-04T09-00-02-0123abcd-dead-beef.jsonl", got)
        # The cwd and the executable are names too. lsof does not sort them
        # out and neither does this -- the caller is looking for one path.
        self.assertIn("/Users/someone/dev/thing", got)
        self.assertIn("/opt/homebrew/bin/codex", got)

    def test_an_open_path_containing_a_space_survives(self):
        info = self.darwin({"lsof -p": fixture("lsof/darwin-open-files")})
        self.assertIn("/Users/someone/My Sessions/notes.jsonl",
                      info.open_files(702))

    def test_open_files_is_empty_when_lsof_is_missing_or_says_nothing(self):
        self.assertEqual([], self.darwin({}).open_files(702))
        self.assertEqual([], self.darwin({"lsof -p": ""}).open_files(702))
        self.assertEqual([], self.darwin(
            {"lsof -p": fixture("lsof/darwin-denied")}).open_files(702))

    def test_the_open_files_query_is_not_narrowed_to_one_descriptor(self):
        # `-d cwd` is what cwd() above needs and what this must not inherit:
        # with it, the only file ever reported is the working directory.
        seen = []

        def run(argv, **kw):
            seen.append(argv)
            return ""

        ayeaye._DarwinProcessInfo(run=run).open_files(702)
        self.assertEqual(["lsof", "-p", "702", "-Fn"], seen[0])

    # ------------------------------------------- the two client questions

    def test_a_pid_ps_reports_back_exists(self):
        info = self.darwin({"ps -p 802 -o pid=": fixture("ps/darwin-pid-alive")})
        self.assertTrue(info.exists(802))
        self.assertTrue(info.exists("802"))

    def test_a_pid_ps_says_nothing_about_does_not_exist(self):
        self.assertFalse(self.darwin({"ps -p": ""}).exists(802))
        self.assertFalse(self.darwin({"ps -p": "\n \n"}).exists(802))

    def test_a_pid_is_believed_when_ps_could_not_be_asked_at_all(self):
        # Not "gone": "could not tell", and the two get opposite answers. The
        # only caller is checking a pid tmux handed it, and its alternative is
        # the first client in a list -- the guess this module exists to stop.
        # Nothing learned means the fact already in hand stands.
        self.assertTrue(self.darwin({}).exists(802))

    def test_the_ssh_peer_comes_out_of_the_process_environment(self):
        info = self.darwin({"ps -ww -E": fixture("ps/darwin-ssh-environ")})
        self.assertEqual("100.101.102.103", info.ssh_peer_ip(802))

    def test_a_client_sitting_at_the_machine_has_no_peer(self):
        info = self.darwin({"ps -ww -E": fixture("ps/darwin-local-environ")})
        self.assertIsNone(info.ssh_peer_ip(801))

    def test_a_variable_whose_name_merely_ends_in_it_is_not_it(self):
        # macOS has no NUL-separated environ to match entries against: `ps -E`
        # appends the block to the command line, space separated. The fixture
        # carries an OLD_SSH_CONNECTION with a different address ahead of the
        # real one, and the wrong one is the one a loose pattern returns.
        info = self.darwin({"ps -ww -E": fixture("ps/darwin-ssh-environ")})
        self.assertEqual("100.101.102.103", info.ssh_peer_ip(802))

    def test_an_ssh_connection_with_no_value_is_not_the_next_variable(self):
        # `ps -E` is space separated, so an SSH_CONNECTION with nothing in it
        # is followed immediately by the next variable. \S+ cannot cross the
        # space; anything hungrier hands back "SSH_TTY=/dev/ttys004" as an
        # address, and the caller would dial it.
        info = self.darwin({"ps -ww -E": fixture("ps/darwin-ssh-empty")})
        self.assertIsNone(info.ssh_peer_ip(801))

    def test_a_peer_lookup_with_no_ps_at_all_is_not_an_error(self):
        self.assertIsNone(self.darwin({}).ssh_peer_ip(802))
        self.assertIsNone(self.darwin({"ps -ww -E": ""}).ssh_peer_ip(802))

    def test_the_client_probes_ask_for_what_they_need(self):
        # Same reason as the other two per-process probes: the canned runner
        # matches on a prefix, so nothing here would notice a wrong flag.
        seen = []

        def run(argv, **kw):
            seen.append(argv)
            return ""

        info = ayeaye._DarwinProcessInfo(run=run)
        info.exists(802)
        info.ssh_peer_ip(802)
        self.assertEqual(["ps", "-p", "802", "-o", "pid="], seen[0])
        # -E is what appends the environment; without it this reads a command
        # line and finds nothing, on every Mac, silently. -ww because the
        # environment is far past any terminal width.
        self.assertEqual(["ps", "-ww", "-E", "-p", "802", "-o", "command="],
                         seen[1])

    def test_neither_client_probe_can_be_answered_from_a_walk_snapshot(self):
        # Both are per-process and neither may quietly reuse the ps -axww
        # snapshot: that one carries no environment and no proof of liveness.
        seen = []
        info = ayeaye._DarwinProcessInfo(
            run=lambda argv, **kw: seen.append(argv))
        info.exists(802)
        info.ssh_peer_ip(802)
        self.assertEqual([], [a for a in seen if "-axww" in a])


class BothBackendsTest(unittest.TestCase):
    """The one cross-platform check this host can actually execute.

    GNU ps is not BSD ps, so this says nothing about `-ww` or how `-o` parses
    its keywords. What it does check is the half that is pure arithmetic and
    would otherwise only ever be compared against bytes written by hand: that
    an `lstart` string parsed by the macOS backend lands on the same instant
    as /proc/<pid>/stat field 22 parsed by the Linux one. A wrong format
    string, a UTC-for-local mix-up or a mktime/timegm slip all miss by hours.

    The tolerance is two seconds and the test asserts no direction, because
    GNU ps does not derive lstart the way Darwin does: it adds field 22 to
    /proc/stat's `btime`, a whole-second reconstruction of the boot instant,
    where Darwin reads the kernel's recorded start timeval directly. Only the
    latter is a pure truncation of the true value.
    """

    def test_lstart_and_proc_agree_on_when_this_process_started(self):
        if not sys.platform.startswith("linux") or not os.path.isdir("/proc"):
            self.skipTest("needs /proc to compare against")
        pid = os.getpid()
        by_lstart = ayeaye._DarwinProcessInfo().start_time(pid)
        if by_lstart is None:
            self.skipTest("this ps cannot report lstart")
        by_proc = ayeaye._LinuxProcessInfo().start_time(pid)
        self.assertLess(abs(by_proc - by_lstart), 2.0,
                        "lstart %r vs /proc %r" % (by_lstart, by_proc))


class DarwinEndToEndTest(TempTree):
    """A pane on macOS, all the way to a rollout, with no /proc anywhere."""

    def setUp(self):
        TempTree.setUp(self)
        self.sessions = os.path.join(self.tmp, "sessions")
        saved = ayeaye.CODEX_SESSIONS
        ayeaye.CODEX_SESSIONS = self.sessions
        self.addCleanup(setattr, ayeaye, "CODEX_SESSIONS", saved)
        d = os.path.join(self.sessions, "2026", "03", "04")
        os.makedirs(d)
        self.path = os.path.join(
            d, "rollout-2026-03-04T09-00-04-77778888-aaaa-bbbb.jsonl")
        with open(self.path, "w") as fh:
            fh.write(json.dumps({"payload": {
                "id": "77778888-aaaa-bbbb",
                "cwd": "/Users/someone/dev/thing"}}) + "\n")

    def test_a_macos_pane_resolves_its_codex_rollout(self):
        info = ayeaye._DarwinProcessInfo(run=canned({
            "ps -axww": fixture("ps/darwin-codex-tree"),
            "ps -ww": fixture("ps/darwin-lstart"),
            "lsof -a -d cwd": fixture("lsof/darwin-cwd"),
        }))
        got = ayeaye.codex_session_for("801", proc=info)
        self.assertEqual({"kind": "codex", "id": "77778888",
                          "path": self.path}, got)

    def test_a_macos_pane_resolves_the_rollout_its_codex_holds_open(self):
        """The same walk again, arriving by the descriptor instead.

        The rollout is a fortnight older than the process, so the timestamp
        match cannot reach it and only lsof can -- which is the resumed
        session, on the platform that has no /proc to ask instead.

        The output is built rather than read from a fixture only because the
        path is this run's temporary directory; it is shaped exactly as
        tests/fixtures/lsof/darwin-open-files is.
        """
        held = os.path.join(self.sessions, "2026", "02", "18",
                            "rollout-2026-02-18T22-40-00-99990000-cccc-dddd.jsonl")
        os.makedirs(os.path.dirname(held))
        with open(held, "w") as fh:
            fh.write(json.dumps({"payload": {
                "id": "99990000-cccc-dddd",
                "cwd": "/Users/someone/dev/thing"}}) + "\n")
        info = ayeaye._DarwinProcessInfo(run=canned({
            "ps -axww": fixture("ps/darwin-codex-tree"),
            "ps -ww": fixture("ps/darwin-lstart"),
            "lsof -a -d cwd": fixture("lsof/darwin-cwd"),
            "lsof -p": "p803\nfcwd\nn/Users/someone/dev/thing\nf3\nn%s\n" % held,
        }))
        got = ayeaye.codex_session_for("801", proc=info)
        self.assertEqual({"kind": "codex", "id": "99990000", "path": held}, got)

    def test_a_macos_pane_with_no_tools_at_all_resolves_to_nothing(self):
        info = ayeaye._DarwinProcessInfo(run=lambda argv, **kw: None)
        self.assertIsNone(ayeaye.codex_session_for("801", proc=info))


# ------------------------------------------------------- platform selection

class SelectionTest(unittest.TestCase):

    def test_the_backend_follows_the_platform(self):
        self.assertIsInstance(ayeaye._make_process_info("linux"),
                              ayeaye._LinuxProcessInfo)
        self.assertIsInstance(ayeaye._make_process_info("darwin"),
                              ayeaye._DarwinProcessInfo)

    def test_an_unknown_platform_falls_back_to_proc(self):
        self.assertIsInstance(ayeaye._make_process_info("freebsd14"),
                              ayeaye._LinuxProcessInfo)

    def test_the_macos_backend_never_reaches_for_proc(self):
        # The whole point: on a Mac there is nothing at /proc to reach for.
        # Building the backend and running a walk through it must not open a
        # file at all, on whichever host this happens to run.
        opened = []
        real_open, real_readlink = builtins.open, os.readlink

        def watch_open(path, *a, **kw):
            opened.append(str(path))
            return real_open(path, *a, **kw)

        def watch_readlink(path, *a, **kw):
            opened.append(str(path))
            return real_readlink(path, *a, **kw)

        builtins.open, os.readlink = watch_open, watch_readlink
        self.addCleanup(setattr, os, "readlink", real_readlink)
        self.addCleanup(setattr, builtins, "open", real_open)
        try:
            info = ayeaye._make_process_info("darwin")
            info.run = lambda argv, **kw: ""
            info.descendant(801, "codex")
            info.start_time(801)
            info.cwd(801)
            info.exists(801)
            info.ssh_peer_ip(801)
        finally:
            builtins.open, os.readlink = real_open, real_readlink
        self.assertEqual([], [p for p in opened if p.startswith("/proc")])

    def test_the_module_selected_one_for_this_host(self):
        self.assertTrue(hasattr(ayeaye.PROCESS_INFO, "descendant"))
        want = (ayeaye._DarwinProcessInfo if sys.platform == "darwin"
                else ayeaye._LinuxProcessInfo)
        self.assertIsInstance(ayeaye.PROCESS_INFO, want)

    def test_a_tool_that_is_not_installed_is_not_an_exception(self):
        self.assertIsNone(ayeaye._run_tool(
            ["ayeaye-no-such-command-exists", "-x"]))

    def test_a_tool_that_fails_yields_its_output_not_an_error(self):
        # pgrep exits 1 when nothing matched, and always has: an empty answer
        # is an answer, not a failure.
        out = ayeaye._run_tool(["sh", "-c", "printf hi; exit 3"])
        self.assertEqual("hi", out)

    def test_a_tool_that_emits_undecodable_bytes_still_answers(self):
        # ps prints the path of every process on the machine, and nothing
        # promises those are UTF-8. A strict decode raises inside
        # subprocess.run, which reads as "no processes at all" -- one stray
        # filename anywhere and every pane loses its agent.
        out = ayeaye._run_tool(["sh", "-c", r"printf 'be\377fore\n'"])
        self.assertIsNotNone(out, "one bad byte must not lose the output")
        self.assertTrue(out.startswith("be"), out)
        self.assertTrue(out.rstrip().endswith("fore"), out)


class SharedModuleTest(unittest.TestCase):
    """One implementation, not two that look alike.

    A second copy of a platform backend does not announce itself. It works on
    the day it is written and then one of the two gets a fix, and the failure
    that follows is a wrong answer rather than an error. The only assertion
    that can see that coming is object identity: the classes the two commands
    hold have to be the same objects, from the same file, loaded once.
    """

    def setUp(self):
        # exec_module rather than the load_module() that goes away in python
        # 3.15, and a named loader because a command has no .py extension.
        path = os.path.join(REPO_ROOT, "bin", "voice-dictate")
        name = "voice_dictate_shared_check"
        spec = spec_from_file_location(name, path,
                                       loader=SourceFileLoader(name, path))
        self.vd = module_from_spec(spec)
        sys.modules[name] = self.vd
        self.addCleanup(sys.modules.pop, name, None)
        spec.loader.exec_module(self.vd)

    def test_both_commands_hold_the_same_backend_classes(self):
        for name in ("_run_tool", "_ProcessInfo", "_LinuxProcessInfo",
                     "_DarwinProcessInfo", "_make_process_info", "PS_SNAPSHOT"):
            self.assertIs(getattr(ayeaye.process_inspect, name),
                          getattr(self.vd.process_inspect, name), name)

    def test_the_module_is_loaded_once_per_process(self):
        self.assertIs(ayeaye.process_inspect, self.vd.process_inspect)
        self.assertIs(ayeaye.PROCESS_INFO, self.vd.PROCESS_INFO)

    def test_the_interface_is_the_whole_of_what_either_command_asks(self):
        for backend in (ayeaye._LinuxProcessInfo, ayeaye._DarwinProcessInfo):
            for method in ("children", "comm", "start_time", "cwd",
                           "open_files", "exists", "ssh_peer_ip", "descendant"):
                self.assertTrue(callable(getattr(backend, method, None)),
                                "%s.%s" % (backend.__name__, method))

    def test_the_backend_lives_in_one_file_and_neither_command_repeats_it(self):
        """The extraction, read off disk rather than out of memory.

        Identity above proves the two agree today. This proves there is
        nowhere for a second one to come back: neither command may define a
        backend class of its own.
        """
        for command in ("ayeaye", "voice-dictate"):
            with open(os.path.join(REPO_ROOT, "bin", command)) as fh:
                body = fh.read()
            for cls in ("class _LinuxProcessInfo", "class _DarwinProcessInfo"):
                self.assertNotIn(cls, body, "bin/%s" % command)

    def test_the_module_is_beside_the_commands_that_load_it(self):
        self.assertTrue(os.path.exists(
            os.path.join(REPO_ROOT, "bin", "process_inspect.py")))


class ClaudeMarkerTest(TempTree):
    """The statusline marker, which is the fallback rather than the answer.

    What it still does: resolve a claude old enough to keep no session file.
    What it cannot do alone, and never could, is say whether the session it
    names is still there -- the text stays in the scrollback after the agent
    exits, and after /clear it names a conversation that has been replaced. So
    it is read for a pane that is running claude now, and only once the
    process has failed to answer.
    """

    def setUp(self):
        TempTree.setUp(self)
        self.projects = os.path.join(self.tmp, "projects")
        os.makedirs(os.path.join(self.projects, "-home-someone-dev-thing"))
        self.path = os.path.join(self.projects, "-home-someone-dev-thing",
                                 "1a2b3c4d-5555-6666-7777-888899990000.jsonl")
        with open(self.path, "w") as fh:
            fh.write("{}\n")
        # No session file anywhere: this is the older claude the marker is
        # here for.
        for name, value in (("CLAUDE_PROJECTS", self.projects),
                            ("CLAUDE_SESSIONS", os.path.join(self.tmp, "none"))):
            self.addCleanup(setattr, ayeaye, name, getattr(ayeaye, name))
            setattr(ayeaye, name, value)

    def patch_tmux(self, pane_text, command="claude"):
        def tmux(*args):
            if args[0] == "capture-pane":
                return pane_text
            return "%s\t900\n" % command
        saved = ayeaye.tmux
        ayeaye.tmux = tmux
        self.addCleanup(setattr, ayeaye, "tmux", saved)

    def patch_processes(self, pid="991"):
        """A claude under the pane, or None for a pane that has none."""
        saved = ayeaye.PROCESS_INFO
        ayeaye.PROCESS_INFO = FakeProcessInfo(pid=pid)
        self.addCleanup(setattr, ayeaye, "PROCESS_INFO", saved)

    def test_a_marker_resolves_a_pane_whose_claude_keeps_no_session_file(self):
        self.patch_tmux("some output\n⟪cc:1a2b3c4d⟫\n$ ")
        self.patch_processes()
        got = ayeaye.pane_session("%1")
        self.assertEqual({"kind": "claude", "id": "1a2b3c4d",
                          "path": self.path}, got)

    def test_a_marker_with_no_transcript_behind_it_resolves_to_nothing(self):
        self.patch_tmux("⟪cc:deadbeef⟫\n")
        self.patch_processes()
        self.assertIsNone(ayeaye.pane_session("%1"))

    def test_a_plain_shell_pane_is_not_an_agent(self):
        self.patch_tmux("$ ls\nfoo bar\n$ ", command="fish")
        self.patch_processes(pid=None)
        self.assertIsNone(ayeaye.pane_session("%1"))

    def test_a_marker_left_behind_by_an_agent_that_exited_is_not_a_session(self):
        # The pane is back at a shell and the marker is still in its
        # scrollback. Reading the text first, this pane reported a live agent
        # and streamed a conversation nobody was having.
        self.patch_tmux("⟪cc:1a2b3c4d⟫\nBye!\n$ ", command="fish")
        self.patch_processes(pid=None)
        self.assertIsNone(ayeaye.pane_session("%1"))


class ClaudeSessionTest(TempTree):
    """Which transcript belongs to a claude pane with no marker in it.

    A marker is not something a pane always has. Claude paints the statusline
    into the live screen and it reaches the scrollback only as later output
    pushes it off, and none is painted at all while a question dialog is up --
    so a young session that stops to ask has the marker nowhere in the pane,
    and that is precisely the pane this app has to show. The session file
    claude keeps per live process is what answers instead.
    """

    def setUp(self):
        TempTree.setUp(self)
        self.projects = os.path.join(self.tmp, "projects")
        self.sessions = os.path.join(self.tmp, "sessions")
        os.makedirs(os.path.join(self.projects, "-home-someone-dev-thing"))
        os.makedirs(self.sessions)
        for name, value in (("CLAUDE_PROJECTS", self.projects),
                            ("CLAUDE_SESSIONS", self.sessions)):
            self.addCleanup(setattr, ayeaye, name, getattr(ayeaye, name))
            setattr(ayeaye, name, value)
        self.sid = "1a2b3c4d-5555-6666-7777-888899990000"
        self.path = os.path.join(self.projects, "-home-someone-dev-thing",
                                 self.sid + ".jsonl")
        with open(self.path, "w") as fh:
            fh.write("{}\n")
        self.started = time.mktime(time.strptime("2026-03-04T09-00-00",
                                                 "%Y-%m-%dT%H-%M-%S"))

    def session_file(self, pid="991", body=None, **over):
        """The file claude writes for a live session, or any body at all."""
        if body is None:
            entry = {"pid": int(pid), "sessionId": self.sid,
                     "cwd": "/home/someone/dev/thing", "kind": "interactive",
                     "startedAt": int(self.started * 1000)}
            entry.update(over)
            body = json.dumps(entry)
        with open(os.path.join(self.sessions, "%s.json" % pid), "w") as fh:
            fh.write(body)

    def resolve(self, pid="991", started=-1):
        info = FakeProcessInfo(
            pid=pid, started=self.started if started == -1 else started)
        return ayeaye.claude_session_for("900", proc=info), info

    def test_a_live_claude_resolves_the_transcript_its_file_names(self):
        self.session_file()
        got, _ = self.resolve()
        self.assertEqual({"kind": "claude", "id": "1a2b3c4d",
                          "path": self.path}, got)

    def test_the_agent_is_looked_for_below_the_pane_shell(self):
        self.session_file()
        _, info = self.resolve()
        self.assertEqual([("descendant", "900", "claude")], info.asked)

    def test_no_claude_below_the_pane_means_no_session(self):
        self.session_file()
        got, _ = self.resolve(pid=None)
        self.assertIsNone(got)

    def test_a_claude_that_wrote_no_session_file_resolves_to_nothing(self):
        # An older claude than the one that keeps these. The marker path is
        # still there for it; nothing here may raise on its absence.
        got, _ = self.resolve()
        self.assertIsNone(got)

    def test_a_session_file_that_is_not_json_resolves_to_nothing(self):
        self.session_file(body="{half a line")
        got, _ = self.resolve()
        self.assertIsNone(got)

    def test_a_recycled_pid_does_not_inherit_the_previous_conversation(self):
        # The file outlived a crash and the system handed the pid out again.
        # The start times are what tell the two processes apart.
        self.session_file()
        got, _ = self.resolve(started=self.started + 3600)
        self.assertIsNone(got)

    def test_the_start_time_window_edges_are_inclusive(self):
        self.session_file()
        skew = ayeaye.CLAUDE_START_SKEW
        for delta in (-skew, skew):
            self.assertIsNotNone(self.resolve(started=self.started + delta)[0],
                                 "delta %s" % delta)

    def test_a_start_time_that_cannot_be_read_means_no_session(self):
        self.session_file()
        got, _ = self.resolve(started=None)
        self.assertIsNone(got)

    def test_a_file_with_no_start_time_in_it_means_no_session(self):
        self.session_file(startedAt=None)
        got, _ = self.resolve()
        self.assertIsNone(got)

    def test_a_session_that_has_written_nothing_yet_resolves_to_nothing(self):
        # Claude writes the transcript when the first message is sent, so a
        # session file can name one that does not exist for minutes.
        os.remove(self.path)
        self.session_file()
        got, _ = self.resolve()
        self.assertIsNone(got)

    def test_a_session_id_that_is_not_a_uuid_is_refused(self):
        # The id is pasted into a glob. A file claiming this one must not be
        # able to aim that glob out of the projects directory.
        self.session_file(sessionId="../../../../etc/*")
        got, _ = self.resolve()
        self.assertIsNone(got)

    def patch_tmux(self, command, on_capture=None):
        """A pane running `command`, whose text is nobody's business here.

        Both fields come back from the one query pane_session makes, tab
        separated, which is what pins the pane's pid reaching the walk.
        """
        def tmux(*args):
            if args[0] == "capture-pane":
                return (on_capture or (lambda: "no statusline down here\n"))()
            return "%s\t900\n" % command
        self.addCleanup(setattr, ayeaye, "tmux", ayeaye.tmux)
        ayeaye.tmux = tmux

    def patch_processes(self, pid="991"):
        self.addCleanup(setattr, ayeaye, "PROCESS_INFO", ayeaye.PROCESS_INFO)
        ayeaye.PROCESS_INFO = FakeProcessInfo(pid=pid, started=self.started)

    def test_a_markerless_claude_pane_resolves_through_pane_session(self):
        self.session_file()
        self.patch_tmux("claude")
        self.patch_processes()
        self.assertEqual({"kind": "claude", "id": "1a2b3c4d",
                          "path": self.path}, ayeaye.pane_session("%1"))
        self.assertEqual([("descendant", "900", "claude")],
                         ayeaye.PROCESS_INFO.asked)

    def test_the_pane_text_is_never_read_once_the_session_file_answered(self):
        # Reading it is not merely wasted work: what it says can disagree,
        # and the file is the one that knows which conversation is current.
        def explode():
            raise AssertionError("the pane text was read anyway")

        self.session_file()
        self.patch_tmux("claude", on_capture=explode)
        self.patch_processes()
        self.assertEqual(self.path, ayeaye.pane_session("%1")["path"])

    def test_a_pane_running_neither_agent_is_never_walked(self):
        # The walk is the expensive half and most panes are not agents, so the
        # pane's own command has to gate it.
        self.session_file()
        self.patch_tmux("fish")
        self.patch_processes()
        self.assertIsNone(ayeaye.pane_session("%1"))
        self.assertEqual([], ayeaye.PROCESS_INFO.asked)

    def test_a_codex_pane_goes_to_the_codex_resolver(self):
        # Same dispatch, both agents: the pane's command picks the resolver
        # and the pane's pid is what it is given.
        self.patch_tmux("codex")
        self.patch_processes(pid=None)
        self.assertIsNone(ayeaye.pane_session("%1"))
        self.assertEqual([("descendant", "900", "codex")],
                         ayeaye.PROCESS_INFO.asked)


if __name__ == "__main__":
    unittest.main(verbosity=2)
