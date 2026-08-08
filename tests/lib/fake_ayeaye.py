#!/usr/bin/env python3
"""A stand-in for a running ayeaye, for tests of the health check.

    fake_ayeaye.py --port 0 --token abc [--behaviour name]…

Prints one line - "port=<n>" - as soon as it is listening, then serves until
it is killed. A test starts it in the background, reads that line to learn
which port it got, and points the health check at it.

Why a real server rather than a stubbed curl: the thing under test is a set of
HTTP assertions, and the interesting ones are about status codes an ayeaye
really returns - 401 for an unauthenticated /api/ call, 403 for a Host it was
not told about. A stub that answers whatever the test tells it to answer would
be testing the stub. This copies bin/ayeaye's three gates and nothing else,
and every way of getting them wrong is available as a named behaviour so that
a test can prove the check catches it.

The gates, as bin/ayeaye has them:

    Host        checked on every request, against a list. Not on it: 403.
    auth        checked on /api/* only. Missing or wrong: 401. The credential
                is the X-Voice-Token header or the voice_token cookie.
    the pages   /, /board, the icons and the manifest are served to anybody.
                They contain no data and never carry the key.

    behaviours (each one a way for a real deployment to be broken):
      open-api        /api/* answers 200 with no key at all. This is the
                      failure the unauthenticated-rejection check exists to
                      find, and the reason that check is worth having.
      no-host-check   any Host is accepted.
      voice-off       /api/overview reports "voice": false.
      no-board        /api/cliban/projects answers 500.
      down            listen, then refuse to answer anything: every request
                      gets a closed connection.
      slow            take longer than any health check will wait.
"""

import argparse
import http.server
import json
import socket
import sys
import threading
import time

ARGS = None


def _allowed(host):
    if "no-host-check" in ARGS.behaviour:
        return True
    host = (host or "").strip().lower()
    if not host:
        return False
    if host in ARGS.allow:
        return True
    # bin/ayeaye retries once with a trailing :<port> removed, and only when
    # there was one to remove.
    bare = host.rsplit(":", 1)[0] if ":" in host and host.rsplit(":", 1)[1].isdigit() else host
    return bare != host and bare in ARGS.allow


def _credential(handler):
    tok = handler.headers.get("X-Voice-Token", "")
    if tok:
        return tok
    for part in (handler.headers.get("Cookie") or "").split(";"):
        name, _, value = part.strip().partition("=")
        if name == "voice_token":
            return value
    return ""


class Handler(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *_a):
        pass

    def _send(self, code, body, ctype="application/json"):
        raw = body.encode()
        self.send_response(code)
        self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(raw)))
        self.send_header("Cache-Control", "no-store")
        self.end_headers()
        self.wfile.write(raw)

    def do_GET(self):
        if "down" in ARGS.behaviour:
            self.close_connection = True
            return
        if "slow" in ARGS.behaviour:
            time.sleep(ARGS.slow_seconds)
        if not _allowed(self.headers.get("Host", "")):
            return self._send(403, json.dumps({"error": "forbidden"}))

        path = self.path.split("?", 1)[0]
        if path in ("/", "/index.html"):
            return self._send(200, "<!doctype html><title>ayeaye</title>",
                              "text/html; charset=utf-8")
        if path in ("/board", "/board.html"):
            return self._send(200, "<!doctype html><title>board</title>",
                              "text/html; charset=utf-8")
        if path == "/favicon.ico":
            return self._send(200, "\x00", "image/x-icon")
        if path in ("/icon-64.png", "/icon-180.png",
                    "/icon-192.png", "/icon-512.png",
                    "/icon-maskable-192.png", "/icon-maskable-512.png"):
            return self._send(200, "\x89PNG", "image/png")

        if path.startswith("/api/"):
            if "open-api" not in ARGS.behaviour:
                if _credential(self) != ARGS.token:
                    return self._send(401, json.dumps({"error": "unauthorized"}))
            if path == "/api/overview":
                voice = "voice-off" not in ARGS.behaviour
                return self._send(200, json.dumps(
                    {"host": "fake", "panes": [], "voice": voice}))
            if path == "/api/cliban/projects":
                if "no-board" in ARGS.behaviour:
                    return self._send(500, json.dumps({"error": "no cliban"}))
                return self._send(200, json.dumps({"keys": ["DEMO"]}))
            return self._send(200, json.dumps({"ok": True}))

        return self._send(404, json.dumps({"error": "not found"}))


class Server(http.server.ThreadingHTTPServer):
    daemon_threads = True
    allow_reuse_address = True


def main():
    global ARGS
    p = argparse.ArgumentParser()
    p.add_argument("--port", type=int, default=0)
    p.add_argument("--bind", default="127.0.0.1")
    p.add_argument("--token", default="")
    p.add_argument("--allow", action="append", default=[],
                   help="an extra Host value to accept, repeatable")
    p.add_argument("--behaviour", action="append", default=[])
    p.add_argument("--slow-seconds", type=float, default=30.0)
    ARGS = p.parse_args()

    srv = Server((ARGS.bind, ARGS.port), Handler)
    port = srv.server_address[1]
    # Whatever a real one would accept, plus whatever the test named.
    ARGS.allow = set(h.strip().lower() for h in ARGS.allow if h.strip())
    for host in (ARGS.bind, "localhost", "127.0.0.1"):
        ARGS.allow.add(host.lower())
        ARGS.allow.add("%s:%d" % (host.lower(), port))

    print("port=%d" % port, flush=True)
    t = threading.Thread(target=srv.serve_forever, daemon=True)
    t.start()
    try:
        while True:
            time.sleep(3600)
    except KeyboardInterrupt:
        pass
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (OSError, socket.error) as exc:
        print("fake_ayeaye: %s" % exc, file=sys.stderr)
        sys.exit(1)
