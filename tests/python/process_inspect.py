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
import json
import os
import shutil
import sys
import tempfile
import time
import unittest
from importlib.machinery import SourceFileLoader

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
            self.assertAlmostEqual(reference(pid), live.start_time(pid),
                                   delta=0.01, msg="pid %s" % pid)
            self.assertEqual(os.readlink("/proc/%s/cwd" % os.getpid()),
                             live.cwd(os.getpid()))


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
        self.rollout("2026-03-04T09-00-30", "/home/someone/dev/thing",
                     sid="ffffffff-late")
        near = self.rollout("2026-03-04T09-00-01", "/home/someone/dev/thing",
                            sid="aaaaaaaa-near")
        got, _ = self.resolve()
        self.assertEqual(near, got["path"])
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
        return {"ps -axo": fixture("ps/darwin-codex-tree")}

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

    def test_the_whole_walk_costs_one_ps_call(self):
        calls = []

        def run(argv, **kw):
            calls.append(argv)
            return fixture("ps/darwin-codex-tree")

        ayeaye._DarwinProcessInfo(run=run).descendant(801, "codex")
        self.assertEqual(1, len(calls))
        self.assertEqual(["ps", "-axo", "pid=,ppid=,comm="], calls[0])

    def test_a_repeated_walk_takes_a_fresh_snapshot(self):
        calls = []

        def run(argv, **kw):
            calls.append(argv)
            return fixture("ps/darwin-codex-tree")

        info = ayeaye._DarwinProcessInfo(run=run)
        info.descendant(801, "codex")
        info.descendant(801, "codex")
        self.assertEqual(2, len(calls))

    def test_start_time_comes_from_ps_lstart(self):
        info = self.darwin({"ps -p": fixture("ps/darwin-lstart")})
        expected = time.mktime(time.strptime("Wed Mar 4 09:00:02 2026",
                                             "%a %b %d %H:%M:%S %Y"))
        self.assertEqual(expected, info.start_time(702))

    def test_start_time_parses_a_space_padded_day_of_month(self):
        info = self.darwin({"ps -p": fixture("ps/darwin-lstart-padded")})
        expected = time.mktime(time.strptime("Tue Jul 7 04:05:06 2026",
                                             "%a %b %d %H:%M:%S %Y"))
        self.assertEqual(expected, info.start_time(702))

    def test_start_time_is_none_when_ps_says_nothing(self):
        self.assertIsNone(self.darwin({"ps -p": ""}).start_time(702))
        self.assertIsNone(self.darwin({}).start_time(702))
        self.assertIsNone(self.darwin({"ps -p": "not a date\n"}).start_time(702))

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
            "ps -axo": fixture("ps/darwin-codex-tree"),
            "ps -p": fixture("ps/darwin-lstart"),
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

    def test_selecting_a_backend_touches_nothing(self):
        # Whichever host this runs on, building the other platform's backend
        # must not look for tools or files it has not got. Import-time failure
        # on macOS is exactly what this ticket exists to remove.
        for plat in ("linux", "darwin"):
            ayeaye._make_process_info(plat)

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
