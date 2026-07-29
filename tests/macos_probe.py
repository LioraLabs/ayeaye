#!/usr/bin/env python3
"""Print what the macOS process backend actually sees, on a real Mac.

The rest of the suite drives that backend from canned bytes, because no
machine here can run `ps` or `lsof` in their BSD form. This is the other half:
run it on real hardware, in a tmux pane with codex running in it, and it shows
the three command lines, whether each answered, and what the walk concluded.

    tests/macos_probe.py            # this pane
    tests/macos_probe.py <pane-pid> # a specific one

Nothing here asserts. It is a thing to read when session matching says a pane
has no agent and you need to know which of the three probes went quiet.
"""
import os
import subprocess
import sys
from importlib.machinery import SourceFileLoader

os.environ.setdefault("AYEAYE_TOKEN", "probe")
ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ayeaye = SourceFileLoader("ayeaye_probe",
                          os.path.join(ROOT, "bin", "ayeaye")).load_module()


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
