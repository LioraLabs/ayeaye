#!/usr/bin/env python3
"""Ask bin/ayeaye's own voice probe what it makes of this machine.

The suite is bash and the probe under test is Python, so something has to sit
between them. It is this: one process per invocation, `key=value` lines on
stdout for bash to assert on, and nothing carried between runs.

It exists because "did setup leave this computer able to listen" is a question
only the app can answer, and the app answers it in a function whose result
never leaves the process except as one boolean inside a JSON overview. A test
that could only see that boolean could not say *why* it was false, which is
the half worth knowing when the installer and the runtime disagree.

  voice_probe.py voice [--env FILE] [--server HOST:PORT]

--server pins VOICE_WHISPER_SERVER. The probe's first route is a socket to
whichever address that names, and a sandbox does not isolate the network: a
transcription server running on the machine the suite happens to be on would
otherwise answer for it, and the test would pass for a reason that has nothing
to do with what setup did. Pointing it at a port nothing can listen on is what
makes the fallback route - the one setup actually configures - the route under
test.

--env loads a settings file into the environment before the app is imported,
which is what the systemd unit's EnvironmentFile= does at boot and what a
person running the app from a shell does with their profile. It has to happen
first: bin/voice-dictate reads its configuration at import time, and
bin/ayeaye imports it.
"""
import argparse
import os
import shutil
import sys
from importlib.machinery import SourceFileLoader

# Importing the code under test would otherwise leave a __pycache__ inside the
# checkout, and a suite that writes into the tree it is testing has already
# broken the promise it makes about where it may write.
sys.dont_write_bytecode = True

REPO_ROOT = os.path.dirname(os.path.dirname(os.path.dirname(
    os.path.abspath(__file__))))


def load_env(path):
    """The settings file, the way everything else in this project reads it:
    last assignment wins, one layer of quotes stripped, a commented line is
    not an assignment. A value already in the real environment wins, because
    that is what systemd's EnvironmentFile= and a shell profile both do."""
    if not path or not os.path.exists(path):
        return
    with open(path) as handle:
        for line in handle:
            line = line.strip()
            if not line or line.startswith("#") or "=" not in line:
                continue
            key, _, value = line.partition("=")
            key = key.strip()
            value = value.strip()
            if len(value) >= 2 and value[0] == value[-1] and value[0] in "\"'":
                value = value[1:-1]
            os.environ[key] = value


def load_app():
    return SourceFileLoader(
        "ayeaye_under_test", os.path.join(REPO_ROOT, "bin", "ayeaye")
    ).load_module()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("command", choices=["voice"])
    parser.add_argument("--env", default="")
    parser.add_argument("--server", default="")
    args = parser.parse_args()

    load_env(args.env)
    if args.server:
        os.environ["VOICE_WHISPER_SERVER"] = args.server
    app = load_app()

    # Every input the probe consults, printed alongside its verdict, so a
    # failing assertion says which half of the handshake broke rather than
    # only that it did.
    # Every input the verdict is built from, read out of the app itself
    # wherever the app has it. `gate_cli` is the one exception and it is
    # spelled out rather than borrowed: bin/ayeaye's probe looks for the
    # literal name `whisper-cpp`, while the transcriber the app would really
    # run is `dictate_cli`. Those two disagreeing is a real configuration -
    # whisper.cpp renamed its binaries - and a probe that reported one number
    # for both could not show it.
    out = {
        "voice": "available" if app.voice_available() else "text-only",
        "ffmpeg": "yes" if shutil.which("ffmpeg") else "no",
        "server": app.vd.WHISPER_SERVER,
        "gate_cli": "yes" if shutil.which("whisper-cpp") else "no",
        "dictate_cli": app.vd.WHISPER_CLI,
        "dictate_cli_present": "yes" if shutil.which(app.vd.WHISPER_CLI) else "no",
        "model": app.vd.WHISPER_MODEL,
        "model_present": "yes" if os.path.exists(app.vd.WHISPER_MODEL) else "no",
    }
    for key in ("voice", "ffmpeg", "server", "gate_cli", "dictate_cli",
                "dictate_cli_present", "model", "model_present"):
        print("%s=%s" % (key, out[key]))
    return 0


if __name__ == "__main__":
    sys.exit(main())
