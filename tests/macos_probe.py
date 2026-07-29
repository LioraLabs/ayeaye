#!/usr/bin/env python3
"""Print what the macOS process backend actually sees, on a real Mac.

The rest of the suite drives that backend from canned bytes, because no
machine here can run `ps` or `lsof` in their BSD form. This is the other half:
run it on real hardware, in a tmux pane with codex running in it, and it shows
every command line the backend uses, whether each answered, and what was
concluded from it.

    tests/macos_probe.py            # this pane
    tests/macos_probe.py <pane-pid> # a specific one

Two questions, because two commands ask them. Which agent is behind a pane,
which is what bin/ayeaye needs; and which device a tmux client is sitting at,
which is what bin/voice-dictate needs before it opens a microphone.

The second is the newer half, and it carries the one thing no fixture on a
Linux host can settle: whether `ps -E` will show the environment of another
process at all. That is a permission question, macOS has tightened such
things before, and the failure is quiet -- a command line with no variables
after it means ssh_peer_ip finds nothing, every tmux client looks local, and
dictation records in whichever room tmux happened to list first.

Nothing here asserts. It is a thing to read when session matching says a pane
has no agent, or dictation opens the wrong microphone, and you need to know
which probe went quiet.
"""
import os
import subprocess
import sys
from importlib.machinery import SourceFileLoader
from importlib.util import module_from_spec, spec_from_file_location

os.environ.setdefault("AYEAYE_TOKEN", "probe")
ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

# Loaded, not imported: bin/ayeaye has no .py extension because it is a
# command, which is why the loader is named rather than inferred from one.
# exec_module rather than the load_module() that warns today and goes away in
# python 3.15, and registered under its name first because that is what
# load_module() did and what anything looking the module up by name expects.
_NAME = "ayeaye_probe"
_PATH = os.path.join(ROOT, "bin", "ayeaye")
_spec = spec_from_file_location(_NAME, _PATH,
                                loader=SourceFileLoader(_NAME, _PATH))
ayeaye = module_from_spec(_spec)
sys.modules[_NAME] = ayeaye
_spec.loader.exec_module(ayeaye)


def clients(info):
    """The half bin/voice-dictate needs: is a pid still there, and where is
    the person sitting?

    Two things only a real Mac can answer, and both fail silently:

      exists()       `ps -p <pid> -o pid=` has to distinguish a live pid from
                     a dead one. If it answers nothing for a pid that is
                     plainly alive, every client_pid tmux passes in is thrown
                     away and the fallback list is used instead -- which is
                     less wrong than it sounds, and still not what was meant.

      ssh_peer_ip()  `ps -ww -E` has to print the environment of a process
                     this user owns. If macOS declines, the line below shows a
                     command and no variables after it, and the remote client
                     preference is gone: dictation records on the machine
                     rather than on the device you are typing from, with no
                     error anywhere. That is the bug this backend was written
                     to remove, wearing a different hat.

    Run this from a tmux client that is attached over ssh for the second one
    to mean anything.
    """
    print("\ntmux clients:")
    listed = subprocess.run(
        ["tmux", "list-clients", "-F", "#{client_pid} #{client_tty}"],
        capture_output=True, text=True).stdout.split("\n")
    rows = [r.split()[0] for r in listed if r.strip() and r.split()[0].isdigit()]
    if not rows:
        print("  none listed. Run this inside tmux for the rest to mean"
              " anything; falling back to this process.")
        rows = [str(os.getpid())]

    for client in rows:
        print("  pid %s: exists -> %r, ssh_peer_ip -> %r"
              % (client, info.exists(client), info.ssh_peer_ip(client)))

    gone = subprocess.Popen([sys.executable, "-c", ""],
                            stdout=subprocess.DEVNULL,
                            stderr=subprocess.DEVNULL)
    gone.wait()
    print("  a pid that has just exited (%s): exists -> %r  (want False)"
          % (gone.pid, info.exists(gone.pid)))

    raw = info.run(["ps", "-ww", "-E", "-p", rows[0], "-o", "command="]) or ""
    raw = raw.strip()
    print("  raw `ps -E` for pid %s: %d chars, %d tokens looking like"
          " variables" % (rows[0], len(raw),
                          len([t for t in raw.split() if "=" in t])))
    if raw and len([t for t in raw.split() if "=" in t]) < 3:
        print("  NO ENVIRONMENT. ps printed the command and stopped. This is"
              " the failure that looks like 'every client is local'.")
    print("  %s" % (raw[:300] or "no answer at all"))


def main():
    print("platform: %s" % sys.platform)
    if sys.platform != "darwin":
        print("NOTE: not a Mac. The darwin backend is being forced anyway, so"
              " the commands below are almost certainly answering in their GNU"
              " form and prove nothing about BSD.")

    info = ayeaye._make_process_info("darwin")
    seen = []
    real = ayeaye._run_tool

    def watched(argv, **kw):
        out = real(argv, **kw)
        seen.append((argv, out))
        return out

    info.run = watched

    pid = sys.argv[1] if len(sys.argv) > 1 else str(os.getppid())
    print("starting from pid %s\n" % pid)

    kids, comm = info._snapshot()
    print("ps snapshot: %d processes, %d with children"
          % (len(comm), len(kids)))
    if not comm:
        print("  EMPTY. This is the failure that looks like 'no agent here'.")
    named = [c for c in comm.values() if c and not c.startswith("(")]
    print("  %d rows carry a real name, e.g. %s" % (len(named), named[:3]))
    print("  paths over 67 chars (these need -ww): %d"
          % len([a for a, o in seen for line in (o or "").splitlines()
                 if len(line) > 67]))

    for agent in ("codex", "claude", "node"):
        print("\ndescendant(%s, %r) -> %r"
              % (pid, agent, info.descendant(pid, agent)))

    target = info.descendant(pid, "codex") or pid
    print("\nfor pid %s:" % target)
    print("  start_time -> %r" % info.start_time(target))
    print("  cwd        -> %r" % info.cwd(target))
    print("  session    -> %r" % ayeaye.codex_session_for(pid, proc=info))

    clients(info)

    print("\ncommands run:")
    for argv, out in seen:
        head = (out or "").splitlines()[:1]
        print("  %s\n    -> %s" % (" ".join(argv),
                                   "no answer" if out is None
                                   else "%d lines, first: %s"
                                   % (len((out or "").splitlines()),
                                      head[0] if head else "")))

    for tool in ("ps", "lsof", "pgrep"):
        which = subprocess.run(["which", tool], capture_output=True,
                               text=True).stdout.strip()
        print("  %-6s %s" % (tool, which or "NOT ON PATH"))


if __name__ == "__main__":
    main()
