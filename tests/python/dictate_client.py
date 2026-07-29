#!/usr/bin/env python3
"""Which device is bin/voice-dictate going to record from?

Dictation records on the machine the tmux client is sitting at, not on the
machine tmux is running on, so `resolve_client_pid` and `client_peer_ip`
between them decide whose microphone opens. Getting them wrong is quiet: a
wrong client is a client that answers, and what comes back is somebody else's
room. That is the whole reason this file exists rather than a comment.

Two kinds of coverage, and they are not interchangeable:

  * The Linux half runs against real processes. A child is spawned with a
    chosen environment and its own /proc/<pid>/environ is read back, so the
    assertion is against what the kernel actually wrote rather than against
    what somebody remembered about the format. These were written before the
    backend was extracted and are unchanged by it; if they ever move, the
    refactor moved something.
  * The macOS half runs against canned `ps` output, because no machine here
    can run `ps` in its BSD form. Same behaviours, same assertions, different
    backend underneath.

Driven by tests/cases/dictate_client_test.sh. Run directly for the whole file,
or with a test id:

    tests/python/dictate_client.py
    tests/python/dictate_client.py LiveProcTest.test_a_live_pid_is_accepted
"""
import ast
import builtins
import os
import shutil
import subprocess
import sys
import tempfile
import time
import unittest
from importlib.machinery import SourceFileLoader
from importlib.util import module_from_spec, spec_from_file_location

# Importing the code under test would otherwise leave a __pycache__ inside the
# checkout, and a suite that writes into the tree it is testing has already
# broken the promise it makes about where it may write.
sys.dont_write_bytecode = True

REPO_ROOT = os.environ.get("REPO_ROOT") or os.path.dirname(
    os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
FIXTURES_DIR = os.environ.get("FIXTURES_DIR") or os.path.join(
    REPO_ROOT, "tests", "fixtures")


def load(name, filename):
    """Loaded, not imported: the files under bin/ have no .py extension,
    because they are commands. The loader is named rather than inferred for
    that reason, and exec_module is used rather than the load_module() that
    goes away in python 3.15 -- the same shape the commands themselves use."""
    path = os.path.join(REPO_ROOT, "bin", filename)
    spec = spec_from_file_location(name, path,
                                   loader=SourceFileLoader(name, path))
    module = module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


vd = load("voice_dictate_under_test", "voice-dictate")

PEER = "100.101.102.103"
SSH_CONNECTION = "%s 54321 100.64.0.1 22" % PEER


def fixture(name):
    """The bytes of tests/fixtures/<name>, as text."""
    with open(os.path.join(FIXTURES_DIR, name)) as fh:
        return fh.read()


class ClientTest(unittest.TestCase):
    """Shared scaffolding: a fake tmux, and children that really exist."""

    # Says it has exec'd, then stays alive to be looked at. Both halves are
    # load-bearing -- see spawn().
    CHILD = ("import sys, time; sys.stdout.write('ready\\n');"
             " sys.stdout.flush(); time.sleep(60)")

    def spawn(self, **env):
        """A real process with a chosen environment, reaped on the way out.

        os.environ cannot be used for this. /proc/<pid>/environ is the block
        the kernel copied in at exec, and a later setenv does not appear in
        it, so the only way to put a variable where this code reads it is to
        start a process with it.

        Two things this has to get right, and both of them fail as a flake
        rather than as an error:

        The inherited environment is scrubbed of SSH_CONNECTION first. Anyone
        running this suite over ssh has one, and a child that inherited it
        would answer "remote" for every test that wanted a local client.

        The child announces itself before it is read. Popen returns as soon as
        the fork has happened, and until the exec completes /proc/<pid>/environ
        is still the *parent's* environment block -- so a test that read it
        immediately would sometimes see this process's variables instead of the
        ones it asked for.
        """
        base = dict(os.environ)
        base.pop("SSH_CONNECTION", None)
        base.pop("SSH_CLIENT", None)
        child = subprocess.Popen([sys.executable, "-c", self.CHILD],
                                 env=dict(base, **env),
                                 stdout=subprocess.PIPE,
                                 stderr=subprocess.DEVNULL,
                                 text=True)
        self.addCleanup(child.wait)
        self.addCleanup(child.stdout.close)
        self.addCleanup(child.kill)
        child.stdout.readline()
        return str(child.pid)

    def reaped(self):
        """A pid that existed a moment ago and does not now."""
        child = subprocess.Popen([sys.executable, "-c", ""],
                                 stdout=subprocess.DEVNULL,
                                 stderr=subprocess.DEVNULL)
        child.wait()
        return str(child.pid)

    def patch_tmux(self, session="main", clients=()):
        """Stand in for the two tmux questions resolve_client_pid asks."""
        asked = []

        def tmux(*args):
            asked.append(args)
            if args[:1] == ("display-message",):
                return session
            if args[:1] == ("list-clients",):
                return " ".join(clients)
            return ""

        saved = vd.tmux
        vd.tmux = tmux
        self.addCleanup(setattr, vd, "tmux", saved)
        return asked


# ------------------------------------------------------ the /proc oracle

class LiveProcTest(ClientTest):
    """Real processes, real /proc. The regression pin for the extraction.

    Every assertion here was written against the pre-extraction file and is
    run again against the extracted one. Nothing is stubbed except tmux, which
    is not a process question.
    """

    def setUp(self):
        if not sys.platform.startswith("linux") or not os.path.isdir("/proc"):
            self.skipTest("the /proc oracle needs a linux host")

    def test_a_live_pid_is_accepted_as_passed(self):
        pid = self.spawn()
        self.patch_tmux(clients=("999999",))
        self.assertEqual(pid, vd.resolve_client_pid("%1", pid))

    def test_a_reaped_pid_is_not_accepted_and_the_session_is_asked_instead(self):
        gone = self.reaped()
        alive = self.spawn()
        self.patch_tmux(clients=(alive,))
        self.assertEqual(alive, vd.resolve_client_pid("%1", gone))

    def test_something_that_is_not_a_pid_at_all_is_not_accepted(self):
        alive = self.spawn()
        self.patch_tmux(clients=(alive,))
        # "0" is the one of these that .isdigit() lets through, so it is the
        # one the process question itself has to turn away.
        for passed in (None, "", "  ", "0", "-1", "0x10", "1e3", "abc"):
            self.assertEqual(alive, vd.resolve_client_pid("%1", passed),
                             "passed %r" % (passed,))

    def test_the_peer_address_is_the_first_field_of_ssh_connection(self):
        pid = self.spawn(SSH_CONNECTION=SSH_CONNECTION)
        self.assertEqual(PEER, vd.client_peer_ip(pid))

    def test_a_local_client_has_no_peer_address(self):
        pid = self.spawn()
        self.assertIsNone(vd.client_peer_ip(pid))

    def test_an_empty_ssh_connection_is_not_an_address(self):
        pid = self.spawn(SSH_CONNECTION="")
        self.assertIsNone(vd.client_peer_ip(pid))

    def test_a_process_that_has_gone_has_no_peer_address(self):
        self.assertIsNone(vd.client_peer_ip(self.reaped()))

    def test_a_pid_that_never_existed_has_no_peer_address(self):
        self.assertIsNone(vd.client_peer_ip("4294967295"))

    def test_a_variable_that_merely_ends_in_ssh_connection_is_not_it(self):
        # The environ block is NUL-separated and matched on a whole entry, so
        # a variable whose name ends in SSH_CONNECTION must not answer for it.
        pid = self.spawn(MY_SSH_CONNECTION=SSH_CONNECTION)
        self.assertIsNone(vd.client_peer_ip(pid))

    def test_the_remote_client_is_preferred_over_the_local_one(self):
        """The behaviour this whole ticket is about, on the platform where it
        already worked. A local client listed first must not win."""
        local = self.spawn()
        remote = self.spawn(SSH_CONNECTION=SSH_CONNECTION)
        self.patch_tmux(clients=(local, remote))
        self.assertEqual(remote, vd.resolve_client_pid("%1", None))

    def test_the_first_client_is_the_last_resort_when_none_is_remote(self):
        first = self.spawn()
        second = self.spawn()
        self.patch_tmux(clients=(first, second))
        self.assertEqual(first, vd.resolve_client_pid("%1", None))

    def test_a_session_with_no_clients_resolves_to_nothing(self):
        self.patch_tmux(clients=())
        self.assertIsNone(vd.resolve_client_pid("%1", None))

    def test_the_pane_is_asked_for_its_session_before_the_clients(self):
        asked = self.patch_tmux(session="work", clients=())
        vd.resolve_client_pid("%7", None)
        self.assertEqual(
            [("display-message", "-p", "-t", "%7", "#{session_name}"),
             ("list-clients", "-t", "work", "-F", "#{client_pid}")], asked)


# --------------------------------------------------------------- macOS

class FakeBackend(object):
    """A backend that answers from dicts. Not a substitute for the real macOS
    one -- that is covered against canned `ps` bytes in process_inspect.py --
    but the way to drive resolve_client_pid's branches on any host."""

    def __init__(self, alive=(), peers=None):
        self.alive = set(str(p) for p in alive)
        self.peers = dict((str(k), v) for k, v in (peers or {}).items())

    def exists(self, pid):
        return str(pid) in self.alive

    def ssh_peer_ip(self, pid):
        return self.peers.get(str(pid))


class DarwinClientTest(ClientTest):
    """The same behaviours with no /proc anywhere, on the real macOS backend
    fed the bytes `ps` produces.

    "No /proc anywhere" is enforced rather than assumed: every open and every
    readlink in the test is watched, and a path under /proc reaching one of
    them fails the test whichever host it runs on. Swapping the backend and
    hoping is how the Linux reflex survives a rewrite.

    What these do not check is the exact argv - the canned runner matches on
    a prefix. That is pinned next door, in process_inspect.py's
    DarwinProcTest.test_the_client_probes_ask_for_what_they_need.
    """

    def setUp(self):
        self.opened = []
        real_open, real_readlink = builtins.open, os.readlink

        def watch(fn):
            def wrapper(path, *a, **kw):
                self.opened.append(str(path))
                return fn(path, *a, **kw)
            return wrapper

        builtins.open, os.readlink = watch(real_open), watch(real_readlink)
        self.addCleanup(setattr, os, "readlink", real_readlink)
        self.addCleanup(setattr, builtins, "open", real_open)
        self.addCleanup(self.assert_no_proc)

    def assert_no_proc(self):
        self.assertEqual([], [p for p in self.opened if p.startswith("/proc")])

    def use(self, info):
        saved = vd.PROCESS_INFO
        vd.PROCESS_INFO = info
        self.addCleanup(setattr, vd, "PROCESS_INFO", saved)
        return info

    def darwin(self, responses):
        def run(argv, **kw):
            joined = " ".join(argv)
            for prefix in sorted(responses, key=len, reverse=True):
                if joined.startswith(prefix):
                    return responses[prefix]
            return None
        return self.use(vd.process_inspect._DarwinProcessInfo(run=run))

    def test_a_live_pid_is_accepted_as_passed(self):
        self.darwin({"ps -p 802 -o pid=": fixture("ps/darwin-pid-alive")})
        self.patch_tmux(clients=("999",))
        self.assertEqual("802", vd.resolve_client_pid("%1", "802"))

    def test_a_pid_ps_does_not_know_falls_back_to_the_session(self):
        self.darwin({"ps -p 702 -o pid=": "", "ps -p 801 -o pid=": ""})
        self.patch_tmux(clients=("801",))
        self.assertEqual("801", vd.resolve_client_pid("%1", "702"))

    def test_a_client_that_died_between_the_two_questions_is_passed_over(self):
        # exists() and ssh_peer_ip() are two separate calls to ps, and a
        # process is free to leave between them.
        self.darwin({"ps -p": fixture("ps/darwin-pid-alive"),
                     "ps -ww -E -p 801": "",
                     "ps -ww -E -p 802": fixture("ps/darwin-ssh-environ")})
        self.patch_tmux(clients=("801", "802"))
        self.assertEqual("802", vd.resolve_client_pid("%1", None))

    def test_an_empty_ssh_connection_is_not_the_next_variable_along(self):
        # The shape that catches a loosened pattern. `ps -E` is space
        # separated, so an SSH_CONNECTION with no value is followed straight
        # away by SSH_TTY -- and anything hungrier than \S+ hands back
        # "SSH_TTY=/dev/ttys004" as somebody's address.
        self.darwin({"ps -ww -E": fixture("ps/darwin-ssh-empty")})
        self.assertIsNone(vd.client_peer_ip("801"))

    def test_the_peer_address_comes_out_of_the_process_environment(self):
        self.darwin({"ps -ww -E -p 801": fixture("ps/darwin-ssh-environ")})
        self.assertEqual("100.101.102.103", vd.client_peer_ip("801"))

    def test_a_client_sitting_at_the_machine_has_no_peer_address(self):
        self.darwin({"ps -ww -E -p 801": fixture("ps/darwin-local-environ")})
        self.assertIsNone(vd.client_peer_ip("801"))

    def test_the_remote_client_is_preferred_over_the_local_one(self):
        # The bug in one line: on macOS this used to read /proc, find nothing
        # for either client, and hand the first one back. The phone in the
        # other room got the microphone.
        self.darwin({
            "ps -ww -E -p 801": fixture("ps/darwin-local-environ"),
            "ps -ww -E -p 802": fixture("ps/darwin-ssh-environ"),
        })
        self.patch_tmux(clients=("801", "802"))
        self.assertEqual("802", vd.resolve_client_pid("%1", None))

    def test_the_first_client_is_the_last_resort_when_none_is_remote(self):
        self.darwin({"ps -ww -E -p 801": fixture("ps/darwin-local-environ"),
                     "ps -ww -E -p 802": fixture("ps/darwin-local-environ")})
        self.patch_tmux(clients=("801", "802"))
        self.assertEqual("801", vd.resolve_client_pid("%1", None))

    def test_a_mac_with_no_ps_at_all_keeps_the_pid_tmux_handed_over(self):
        # Degrading soft: nothing here may raise into the caller. And with no
        # answer from ps there is nothing to learn, so the one fact the caller
        # already had wins - tmux passed in the client that pressed the key.
        # Throwing it away for clients[0] would be the bug this ticket closed,
        # reappearing on a machine with a stripped-down PATH.
        self.use(vd.process_inspect._DarwinProcessInfo(
            run=lambda argv, **kw: None))
        self.patch_tmux(clients=("801", "802"))
        self.assertEqual("702", vd.resolve_client_pid("%1", "702"))
        self.assertIsNone(vd.client_peer_ip("801"))

    def test_a_mac_whose_ps_reports_nothing_does_fall_back(self):
        # The other half of the same distinction: ps ran, ps found no such
        # process, so the pid really is gone and the session is asked.
        self.darwin({"ps -p": "", "ps -ww -E": ""})
        self.patch_tmux(clients=("801", "802"))
        self.assertEqual("801", vd.resolve_client_pid("%1", "702"))


class BranchTest(ClientTest):
    """resolve_client_pid's decision table, on any host, with the process
    questions answered by a dict rather than by an operating system."""

    def use(self, **kw):
        saved = vd.PROCESS_INFO
        vd.PROCESS_INFO = FakeBackend(**kw)
        self.addCleanup(setattr, vd, "PROCESS_INFO", saved)

    def test_the_passed_pid_wins_without_asking_tmux_anything(self):
        self.use(alive=("77",))
        asked = self.patch_tmux(clients=("88",))
        self.assertEqual("77", vd.resolve_client_pid("%1", "77"))
        self.assertEqual([], asked, "a pid that is alive settles it")

    def test_a_dead_passed_pid_falls_through_to_the_clients(self):
        self.use(alive=("88",), peers={"88": PEER})
        self.patch_tmux(clients=("88",))
        self.assertEqual("88", vd.resolve_client_pid("%1", "77"))

    def test_a_client_with_no_peer_address_is_passed_over(self):
        # Only the pid tmux passed in is checked for liveness; the ones read
        # out of list-clients are chosen on having an address, because tmux
        # has just said they are attached.
        self.use(alive=(), peers={"89": PEER})
        self.patch_tmux(clients=("88", "89"))
        self.assertEqual("89", vd.resolve_client_pid("%1", None))


# ------------------------------------------------ nothing reaches for /proc

class NoProcTest(unittest.TestCase):

    def test_bin_voice_dictate_builds_no_path_under_proc(self):
        """A path spelled /proc is one that does not exist on a Mac, and that
        answers "no" there rather than failing.

        Read as syntax rather than as text: the file is entitled to say the
        word in a comment explaining why it no longer reaches for it, and it
        does. What it may not do is build one, so the check is on string
        literals - which is where such a path would have to appear.
        """
        path = os.path.join(REPO_ROOT, "bin", "voice-dictate")
        with open(path) as fh:
            tree = ast.parse(fh.read(), filename=path)
        hits = ["line %d: %r" % (node.lineno, node.value)
                for node in ast.walk(tree)
                if isinstance(node, ast.Constant)
                and isinstance(node.value, str)
                and (node.value == "/proc" or "/proc/" in node.value)]
        self.assertEqual([], hits, "\n".join(["bin/voice-dictate"] + hits))

    def test_the_client_questions_go_through_the_platform_backend(self):
        """Every process question routed through the backend, and none asked
        any other way. `hasattr` would not show that: a file that still read
        /proc directly would pass it."""
        asked = []

        class Recorder(object):
            def exists(self, pid):
                asked.append(("exists", str(pid)))
                return False

            def ssh_peer_ip(self, pid):
                asked.append(("ssh_peer_ip", str(pid)))
                return None

        saved = vd.PROCESS_INFO
        vd.PROCESS_INFO = Recorder()
        saved_tmux = vd.tmux
        vd.tmux = lambda *a: "88 89" if a[:1] == ("list-clients",) else "main"
        try:
            self.assertEqual("88", vd.resolve_client_pid("%1", "77"))
            vd.client_peer_ip("90")
        finally:
            vd.PROCESS_INFO, vd.tmux = saved, saved_tmux
        self.assertEqual([("exists", "77"), ("ssh_peer_ip", "88"),
                          ("ssh_peer_ip", "89"), ("ssh_peer_ip", "90")], asked)

    def test_the_command_still_starts_when_it_is_reached_by_a_symlink(self):
        """The documented way to run this is a tmux binding that finds it on
        PATH, and the way it gets onto PATH is a symlink into ~/.local/bin.

        Its sibling module is found relative to this file, so the resolution
        has to see through that link. It is asserted by really running the
        command through one, because the failure is an ImportError at startup
        and `run-shell -b` throws the traceback away: the key stops working
        and nothing anywhere says why.
        """
        tmp = tempfile.mkdtemp(prefix="ayeaye-link-")
        self.addCleanup(shutil.rmtree, tmp, True)
        link = os.path.join(tmp, "voice-dictate")
        os.symlink(os.path.join(REPO_ROOT, "bin", "voice-dictate"), link)
        # No arguments: it prints its usage and stops, which is the cheapest
        # thing it does that still proves every import ran.
        # PYTHONDONTWRITEBYTECODE for the same reason the module sets
        # sys.dont_write_bytecode: this one really runs the command, and the
        # command really loads a module out of the checkout.
        done = subprocess.run([sys.executable, link], capture_output=True,
                              text=True,
                              env=dict(os.environ, PYTHONDONTWRITEBYTECODE="1"))
        self.assertEqual(1, done.returncode, done.stderr)
        self.assertIn("usage: voice-dictate", done.stderr)


if __name__ == "__main__":
    unittest.main(verbosity=2)
