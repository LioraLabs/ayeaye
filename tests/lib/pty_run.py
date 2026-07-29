#!/usr/bin/env python3
"""Drive an interactive shell script through a real terminal.

    pty_run.py --expect FILE --transcript FILE --status-file FILE
               [--timeout SECONDS] -- command [args...]

Why a pty at all: bash writes a `read -p` prompt only when standard input is a
terminal. Under a pipe the prompt text is never emitted anywhere, so a piped
run can prove what a script *did* but never what it *asked*. Running the script
against a pty makes prompts, defaults and the echoed answers observable as one
ordered transcript.

The expect file is one rule per line, "<substring>\\t<response>". Each rule
waits for its substring to appear in output produced after the previous match,
then types the response and a newline. Waiting on the prompt rather than
sleeping is what keeps the transcript deterministic: no rule can fire early and
scramble the order.

Exit status is 0 when the script was driven to completion, whatever the child
exited with (that goes to --status-file), and 99 for a driver failure: an
expectation that never arrived, or the overall timeout.

Standard library only: python3 is already a hard dependency of this project.
"""

import argparse
import errno
import os
import pty
import select
import struct
import sys
import termios
import time
import fcntl

DRIVER_ERROR = 99


def parse_rules(path):
    rules = []
    if not path:
        return rules
    with open(path, "rb") as handle:
        for raw in handle.read().split(b"\n"):
            if not raw:
                continue
            pattern, _, response = raw.partition(b"\t")
            rules.append((pattern, response))
    return rules


def set_window_size(fd, rows=50, cols=200):
    try:
        fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))
    except OSError:
        pass


def main():
    parser = argparse.ArgumentParser(add_help=False)
    parser.add_argument("--expect")
    parser.add_argument("--transcript", required=True)
    parser.add_argument("--status-file", required=True)
    parser.add_argument("--timeout", type=float, default=20.0)
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()

    command = args.command
    if command and command[0] == "--":
        command = command[1:]
    if not command:
        sys.stderr.write("pty_run: no command given\n")
        return DRIVER_ERROR

    rules = parse_rules(args.expect)

    pid, fd = pty.fork()
    if pid == 0:
        # Child: it is now attached to the slave side of the pty.
        try:
            os.execvp(command[0], command)
        except OSError as exc:
            sys.stderr.write("pty_run: cannot run %s: %s\n" % (command[0], exc))
            os._exit(127)

    set_window_size(fd)

    transcript = bytearray()
    searched = 0
    index = 0
    deadline = time.time() + args.timeout
    timed_out = False

    while True:
        if time.time() > deadline:
            timed_out = True
            break
        try:
            readable, _, _ = select.select([fd], [], [], 0.05)
        except select.error:
            break
        if fd in readable:
            try:
                chunk = os.read(fd, 65536)
            except OSError as exc:
                # A pty master reports the child's exit as EIO, not as EOF.
                if exc.errno in (errno.EIO, errno.EBADF):
                    break
                raise
            if not chunk:
                break
            transcript.extend(chunk)

        if index < len(rules):
            pattern, response = rules[index]
            found = transcript.find(pattern, searched)
            if found >= 0:
                searched = found + len(pattern)
                try:
                    os.write(fd, response + b"\n")
                except OSError:
                    break
                index += 1

    try:
        os.close(fd)
    except OSError:
        pass

    status = None
    if timed_out:
        try:
            os.kill(pid, 9)
        except OSError:
            pass
    try:
        _, wait_status = os.waitpid(pid, 0)
        if os.WIFEXITED(wait_status):
            status = os.WEXITSTATUS(wait_status)
        elif os.WIFSIGNALED(wait_status):
            status = 128 + os.WTERMSIG(wait_status)
    except OSError:
        pass

    text = transcript.decode("utf-8", "replace").replace("\r\n", "\n")
    with open(args.transcript, "w") as handle:
        handle.write(text)
    with open(args.status_file, "w") as handle:
        handle.write("" if status is None else str(status))

    if timed_out:
        sys.stderr.write(
            "pty_run: timed out after %ss with %d of %d expectations satisfied\n"
            % (args.timeout, index, len(rules))
        )
        if index < len(rules):
            sys.stderr.write(
                "pty_run: still waiting for: %r\n" % (rules[index][0].decode("utf-8", "replace"),)
            )
        return DRIVER_ERROR

    if index < len(rules):
        sys.stderr.write(
            "pty_run: the script exited with %d of %d expectations satisfied\n"
            % (index, len(rules))
        )
        sys.stderr.write(
            "pty_run: never saw: %r\n" % (rules[index][0].decode("utf-8", "replace"),)
        )
        return DRIVER_ERROR

    return 0


if __name__ == "__main__":
    sys.exit(main())
