"""Who is running under a tmux pane, and where -- on Linux and on macOS.

Everything below the pane is platform-specific: Linux answers out of /proc,
macOS has no /proc at all and answers out of ps and lsof. This module is the
whole of that difference, and it is a module rather than a paragraph in a
command because two commands need it:

    bin/ayeaye         which agent is behind a pane, and which transcript
    bin/voice-dictate  which tmux client is at a microphone, and where it is

The second one used to answer both of its questions by reading /proc itself.
On a Mac that read simply fails, every client looks local, none is preferred,
and the first one in the list gets the microphone -- with no error anywhere.
Two implementations of the same platform question is how that happens, so
there is one.

Neither command can import the other: bin/ayeaye already loads bin/voice-dictate
to reuse its transcription pipeline, and the reverse would close the circle.
So this is a third file that neither owns, loaded by the same `_load_sibling`
both of them carry. It has a .py extension precisely
because the commands beside it do not: in bin/, an extension means "import
me", not "run me". There is no package here and nothing to install.

Six questions are ever asked -- who are this process's children, what is each
one called, when did it start, where is it running, is it still there, and
what address was it reached from -- so that is the whole interface, and the
walk that uses them is shared.

None means "could not find out", never an exception: this runs inside a
request handler, and a pane whose agent cannot be identified is an ordinary
state of the world, not an error.

Both commands load this at startup rather than lazily, so a copy of bin/ that
is missing it fails to start rather than degrading. That is the right shape:
the alternative is a server that runs and quietly cannot tell one pane's
agent from another's.
"""
import os
import re
import subprocess
import sys
import time


def _run_tool(argv, timeout=5):
    """Standard output of an inspection tool, or None if it could not run.

    A non-zero exit is not a failure here. pgrep exits 1 when nothing matched
    and lsof exits 1 when it had a complaint about a file it could still
    report on; in both cases the output is the answer.

    errors="replace" is load-bearing. `ps` prints the path of every process on
    the machine, no filesystem this runs on enforces valid UTF-8 in a name,
    and a strict decode would turn one unrelated process into an empty tree
    for every pane at once.
    """
    try:
        return subprocess.run(argv, capture_output=True, text=True,
                              errors="replace", timeout=timeout,
                              env=dict(os.environ, LC_ALL="C")).stdout
    except Exception:
        return None


class _ProcessInfo(object):
    """The shared half: finding a named process below a pane's shell."""

    def _snapshot(self):
        """Whatever a backend wants to gather once per walk, or None."""
        return None

    def descendant(self, pid, name, depth=3):
        """Find a named descendant. tmux's pane_pid is the shell; the agent is
        a child of it (and sometimes a grandchild via a wrapper)."""
        snap = self._snapshot()
        frontier = [str(pid)]
        for _ in range(depth):
            nxt = []
            for p in frontier:
                for c in self.children(p, snap):
                    comm = self.comm(c, snap)
                    if comm is None:        # gone, or not ours to look at
                        continue
                    if comm == name:
                        return c
                    nxt.append(c)
            frontier = nxt
        return None


class _LinuxProcessInfo(_ProcessInfo):
    """/proc. `proc`, `run` and `clk_tck` are injectable for tests only."""

    def __init__(self, proc="/proc", run=None, clk_tck=None):
        self.proc = proc
        self.run = run or _run_tool
        self.clk_tck = clk_tck or os.sysconf("SC_CLK_TCK")

    def children(self, pid, snap=None):
        return (self.run(["pgrep", "-P", str(pid)]) or "").split()

    def comm(self, pid, snap=None):
        # errors="replace": comm is whatever bytes the exec'd path ended in,
        # and an undecodable one is a process with an odd name, not a reason
        # to fail a request.
        try:
            with open("%s/%s/comm" % (self.proc, pid), errors="replace") as fh:
                return fh.read().strip()
        except (OSError, ValueError):
            return None

    def start_time(self, pid):
        """Wall-clock time the process began, from /proc/<pid>/stat field 22.

        The comm sits in parentheses and may contain one itself, so the split
        is from the right; what is left starts at field 3, putting starttime
        at index 19. It counts clock ticks since boot, and boot is now minus
        uptime.
        """
        try:
            with open("%s/%s/stat" % (self.proc, pid)) as fh:
                fields = fh.read().rsplit(")", 1)[1].split()
            with open("%s/uptime" % self.proc) as fh:
                uptime = float(fh.read().split()[0])
            return (time.time() - uptime) + float(fields[19]) / self.clk_tck
        except (OSError, ValueError, IndexError):
            return None

    def cwd(self, pid):
        try:
            return os.readlink("%s/%s/cwd" % (self.proc, pid))
        except OSError:
            return None

    def exists(self, pid):
        return os.path.exists("%s/%s" % (self.proc, pid))

    def ssh_peer_ip(self, pid):
        """The address this process was reached from, or None if it is local.

        environ is the block the kernel copied in at exec: NUL-separated,
        which is what makes it safe to read a value that contains spaces, and
        SSH_CONNECTION always contains three of them. Entries are matched
        whole so OLD_SSH_CONNECTION does not answer for it.
        """
        try:
            with open("%s/%s/environ" % (self.proc, pid), "rb") as fh:
                entries = fh.read().split(b"\0")
        except (OSError, ValueError):
            return None
        for entry in entries:
            if entry.startswith(b"SSH_CONNECTION="):
                parts = entry.decode("utf-8", "replace").split("=", 1)[1].split()
                if parts:
                    return parts[0]
        return None


# Three things about BSD ps that a Linux reflex gets wrong, all of which fail
# silently -- an empty process tree looks exactly like a pane with no agent
# under it:
#
#   -o pid=,ppid=,comm=   an `=` makes the REST of the argument the column
#                         header, so this asks for one column named
#                         ",ppid=,comm=". One keyword per -o is the form that
#                         means the same thing to both ps implementations.
#   -ww                   the last column is clipped to the output width,
#                         which ps takes from $COLUMNS or from whichever of
#                         its three streams is still a terminal. Under
#                         capture_output that is stdin, so the answer depends
#                         on how the server was started -- a clipped
#                         interpreter path is exactly the part being matched,
#                         and -ww is the only way to stop asking.
#   comm                  the full executable path, where Linux gives a bare
#                         name truncated to 15 characters; it may contain
#                         spaces, and an agent name longer than 15 characters
#                         would match here and not there.
PS_SNAPSHOT = ["ps", "-axww", "-o", "pid=", "-o", "ppid=", "-o", "comm="]


class _DarwinProcessInfo(_ProcessInfo):
    """ps and lsof. `run` is injectable for tests only.

    One `ps` snapshot serves a whole walk: three levels of ancestry would
    otherwise cost a process per node, on the platform where spawning them is
    slowest. It is taken per walk and carried on the stack, never on the
    instance -- one of these is shared by every thread of a threading server,
    and a half-replaced tree resolves a pane to the wrong agent or to none.
    """

    def __init__(self, run=None):
        self.run = run or _run_tool

    def _snapshot(self):
        kids, comm = {}, {}
        for line in (self.run(PS_SNAPSHOT) or "").splitlines():
            parts = line.split(None, 2)
            if len(parts) < 2 or not parts[0].isdigit() or not parts[1].isdigit():
                continue
            pid, ppid = parts[0], parts[1]
            comm[pid] = os.path.basename(parts[2].strip()) if len(parts) > 2 else ""
            kids.setdefault(ppid, []).append(pid)
        return kids, comm

    def children(self, pid, snap=None):
        kids, _ = snap or self._snapshot()
        return list(kids.get(str(pid), ()))

    def comm(self, pid, snap=None):
        # "" for a row ps gave no name, which still has children worth
        # walking. The Linux side returns None there and prunes the subtree,
        # because that is what it did before this was an interface.
        _, comm = snap or self._snapshot()
        return comm.get(str(pid))

    def start_time(self, pid):
        """`ps -o lstart=` gives "Wed Mar  4 09:00:02 2026" -- local time
        truncated to the second, so it can only ever be earlier than the true
        start, which moves a rollout away from the window's lower edge rather
        than off it. The strptime format is fixed: lstart is ctime(3) output,
        which is C-locale regardless."""
        out = self.run(["ps", "-ww", "-p", str(pid), "-o", "lstart="]) or ""
        try:
            return time.mktime(time.strptime(out.strip(),
                                             "%a %b %d %H:%M:%S %Y"))
        except (ValueError, OverflowError):
            return None

    def cwd(self, pid):
        """`lsof -Fn` puts each field on its own line behind a one-letter tag,
        which is the only form that survives a path with a space in it."""
        out = self.run(["lsof", "-a", "-d", "cwd", "-p", str(pid), "-Fn"]) or ""
        for line in out.splitlines():
            if line.startswith("n"):
                return line[1:]
        return None

    def exists(self, pid):
        """Whether the process is still there.

        Asked of ps rather than of the snapshot: the snapshot is taken per
        walk and this is asked outside one, and a pid that has been gone for
        a whole snapshot is exactly the pid worth catching.

        "ps said nothing" and "ps could not be asked" are answered
        differently, and the difference matters. An empty answer is ps having
        looked, so the pid is gone. No answer at all is no information, and
        the caller's alternative to a pid it was handed is a guess at the
        first client in a list -- which on a Mac is the very failure this
        module exists to remove. When nothing can be learned, the fact the
        caller already had is the better of the two.
        """
        out = self.run(["ps", "-p", str(pid), "-o", "pid="])
        return True if out is None else bool(out.strip())

    def ssh_peer_ip(self, pid):
        """The address this process was reached from, or None if it is local.

        `ps -E` appends the environment to the command column, space
        separated -- so unlike Linux's NUL-separated environ, a value with a
        space in it cannot be recovered whole here. That is the reason this
        interface answers one narrow question instead of offering a general
        environ(): the field wanted is an address, an address has no spaces,
        and both platforms can therefore return exactly the same answer. A
        general reader would have had to truncate a value on one platform and
        not the other, quietly, which is the bug this module was extracted to
        stop repeating.

        The name is anchored to a token boundary so OLD_SSH_CONNECTION and
        SSH_CLIENT do not answer for it. It is still possible in principle for
        a process to carry the literal text in an argument rather than in its
        environment; that costs a preference between two of the user's own
        tmux clients, and nothing else.
        """
        out = self.run(["ps", "-ww", "-E", "-p", str(pid), "-o", "command="])
        found = _SSH_CONNECTION.search(out or "")
        return found.group(1) if found else None


# The first field of SSH_CONNECTION: "<client ip> <client port> <server ip>
# <server port>". \S+ rather than a greedier pattern precisely because only
# the first field is wanted -- the rest are unrecoverable on macOS anyway.
_SSH_CONNECTION = re.compile(r"(?:^|\s)SSH_CONNECTION=(\S+)")


def _make_process_info(plat=None):
    """Pick a backend. By platform at runtime, never by import-time failure:
    the module has to load on a machine with no /proc and on one with no ps."""
    plat = sys.platform if plat is None else plat
    return _DarwinProcessInfo() if plat == "darwin" else _LinuxProcessInfo()


PROCESS_INFO = _make_process_info()
