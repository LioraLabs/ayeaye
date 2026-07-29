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
os.environ.setdefault("AYEAYE_TOKEN", "test-token")
ayeaye = SourceFileLoader(
    "ayeaye_under_test", os.path.join(REPO_ROOT, "bin", "ayeaye")).load_module()

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
    """A process tree with no processes in it."""

    def __init__(self, pid=None, started=None, cwd=None):
        self._pid, self._started, self._cwd = pid, started, cwd
        self.asked = []

    def descendant(self, pid, name, depth=3):
        self.asked.append(("descendant", str(pid), name))
        return self._pid

    def start_time(self, pid):
        return self._started

    def cwd(self, pid):
        return self._cwd


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
            "lsof": fixture("lsof/darwin-cwd"),
        }))
        got = ayeaye.codex_session_for("801", proc=info)
        self.assertEqual({"kind": "codex", "id": "77778888",
                          "path": self.path}, got)

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
                           "exists", "ssh_peer_ip", "descendant"):
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
    """The claude path must not care what platform it is on."""

    def setUp(self):
        TempTree.setUp(self)
        self.projects = os.path.join(self.tmp, "projects")
        os.makedirs(os.path.join(self.projects, "-home-someone-dev-thing"))
        self.path = os.path.join(self.projects, "-home-someone-dev-thing",
                                 "1a2b3c4d-5555-6666-7777-888899990000.jsonl")
        with open(self.path, "w") as fh:
            fh.write("{}\n")
        saved = ayeaye.CLAUDE_PROJECTS
        ayeaye.CLAUDE_PROJECTS = self.projects
        self.addCleanup(setattr, ayeaye, "CLAUDE_PROJECTS", saved)

    def patch_tmux(self, pane_text, command="bash"):
        def tmux(*args):
            if args[0] == "capture-pane":
                return pane_text
            return command + "\n"
        saved = ayeaye.tmux
        ayeaye.tmux = tmux
        self.addCleanup(setattr, ayeaye, "tmux", saved)

    def explode(self):
        """Make process inspection unusable, to prove it is never reached."""
        class Exploding(object):
            def descendant(self, *a, **kw):
                raise AssertionError("the claude path inspected a process")

        saved = ayeaye.PROCESS_INFO
        ayeaye.PROCESS_INFO = Exploding()
        self.addCleanup(setattr, ayeaye, "PROCESS_INFO", saved)

    def test_a_marker_in_the_pane_resolves_a_transcript_with_no_processes(self):
        self.explode()
        self.patch_tmux("some output\n⟪cc:1a2b3c4d⟫\n$ ")
        got = ayeaye.pane_session("%1")
        self.assertEqual({"kind": "claude", "id": "1a2b3c4d",
                          "path": self.path}, got)

    def test_a_marker_with_no_transcript_behind_it_resolves_to_nothing(self):
        self.explode()
        self.patch_tmux("⟪cc:deadbeef⟫\n")
        self.assertIsNone(ayeaye.pane_session("%1"))

    def test_a_plain_shell_pane_is_not_an_agent(self):
        self.explode()
        self.patch_tmux("$ ls\nfoo bar\n$ ")
        self.assertIsNone(ayeaye.pane_session("%1"))


if __name__ == "__main__":
    unittest.main(verbosity=2)
