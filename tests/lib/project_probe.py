#!/usr/bin/env python3
"""Drive bin/ayeaye's project discovery from the shell test suite.

The suite is bash and the code under test is Python, so something has to sit
between them. It is this: one process per invocation, `key=value` lines on
stdout for bash to assert on, and no state carried between runs beyond what
the code under test itself writes to disk.

It exists because the properties worth pinning are not observable from
outside an HTTP request. "Which bound stopped this walk", "how many
directories did it actually list", "did the superseded search deliver
anything" are all invisible to a client that only sees a JSON array, and a
test that cannot see them can only assert that something plausible came back.

Sub-commands:

  maketree ROOT --width W --depth D     build a synthetic tree, print its size
  walk [--query Q] [--limit N] ...      one walk, its bound and its cost
  projects [--query Q] [--limit N]      the picker's answer, in order
  cached [--query Q]                    the same query twice, and what it cost
  shape                                 the keys of one result row
  recents note PATH...                  record picks
  recents dump                          the store, as bash can read it
  supersede --query-a A --query-b B     two searches racing, and who wins
  join --query Q                        the same query twice at once
  spawn DIR                             start an agent, as /api/spawn does

Every sub-command takes --slow to give each directory listing a fixed cost.
Real filesystems have latency and tmpfs in a container does not, so a bound
that ends a walk on a laptop can be missed entirely in CI; a modelled cost
makes the bound, rather than the host's disk, the thing under test.
"""
import argparse
import json
import os
import sys
import threading
import time
from importlib.machinery import SourceFileLoader

# Importing the code under test would otherwise leave a __pycache__ inside
# the checkout, and a suite that writes into the tree it is testing has
# already broken the promise it makes about where it may write.
sys.dont_write_bytecode = True

REPO_ROOT = os.path.dirname(os.path.dirname(os.path.dirname(
    os.path.abspath(__file__))))


def load():
    return SourceFileLoader(
        "ayeaye_under_test", os.path.join(REPO_ROOT, "bin", "ayeaye")
    ).load_module()


# Per-query listing counts. The walk runs on its own thread, so attributing a
# listing to the search that asked for it is the only way to tell a cancelled
# walk (its count stops growing) from one that was merely overtaken.
COUNTS = {}
WALKS = [0]
COUNTS_LOCK = threading.Lock()
TAG = threading.local()


def instrument(m, delay):
    """Wrap the module's listing and walk so cost and attribution are visible."""
    real_list = m._list_dir
    real_walk = m.walk_projects

    def walk(query="", *args, **kwargs):
        TAG.query = query
        with COUNTS_LOCK:
            WALKS[0] += 1
        return real_walk(query, *args, **kwargs)

    def listing(path):
        # Charge only for listings that really touch the filesystem. Charging
        # for a memoised one would hide the memoisation from every timing
        # assertion, which is exactly the sort of blind spot that lets a
        # caching bug ship.
        if delay and path not in m._list_cache:
            time.sleep(delay)
        tag = getattr(TAG, "query", "")
        with COUNTS_LOCK:
            COUNTS[tag] = COUNTS.get(tag, 0) + 1
            COUNTS["*"] = COUNTS.get("*", 0) + 1
        return real_list(path)

    m.walk_projects = walk
    m._list_dir = listing


def count(tag="*"):
    with COUNTS_LOCK:
        return COUNTS.get(tag, 0)


def emit(**kw):
    for key in sorted(kw):
        print("%s=%s" % (key, kw[key]))


# ------------------------------------------------------------ sub-commands

def cmd_maketree(m, args):
    """A balanced tree, built breadth first so a partial run is still a tree.

    Names are distinct per level so a query can select a level rather than a
    branch, which is what the depth and limit assertions need.
    """
    made = 0
    frontier = [args.root]
    os.makedirs(args.root, exist_ok=True)
    for level in range(args.depth):
        nxt = []
        for parent in frontier:
            for i in range(args.width):
                child = os.path.join(parent, "L%dn%d" % (level + 1, i))
                os.mkdir(child)
                made += 1
                nxt.append(child)
        frontier = nxt
    emit(made=made)


def cmd_walk(m, args):
    cancel = threading.Event()
    if args.cancel_after is not None:
        threading.Timer(args.cancel_after, cancel.set).start()
    deadline = None
    if args.budget is not None:
        deadline = time.monotonic() + args.budget
    started = time.monotonic()
    rows, stopped = m.walk_projects(
        args.query, args.limit,
        roots=args.root or None,
        depth=args.depth,
        deadline=deadline,
        cancel=cancel)
    elapsed = time.monotonic() - started
    emit(stopped=stopped or "exhausted", count=len(rows),
         listings=count(), elapsed="%.3f" % elapsed)
    for path, is_git, level in rows:
        print("hit\t%s\t%d\t%s" % ("git" if is_git else "dir", level, path))


def cmd_projects(m, args):
    started = time.monotonic()
    rows = m.projects(args.query, args.limit)
    elapsed = time.monotonic() - started
    emit(count=len(rows), elapsed="%.3f" % elapsed)
    for row in rows:
        if args.field:
            print(row[args.field])
        else:
            print(json.dumps(row, sort_keys=True))


def cmd_cached(m, args):
    """The same query twice, so the second one's cost is visible.

    --settle pauses between them, which is how a walk that outlived the
    waiter that started it gets a chance to finish before anyone asks again.
    """
    before = count()
    first = m.projects(args.query, args.limit)
    middle = count()
    if args.settle:
        time.sleep(args.settle)
    # Sampled after the settle, not before it: a walk still finishing in the
    # background is the first call's cost, and charging it to the second
    # would hide whether the second walked at all.
    before_second = count()
    second = m.projects(args.query, args.limit)
    emit(first_count=len(first), second_count=len(second),
         first_listings=middle - before,
         settle_listings=before_second - middle,
         second_listings=count() - before_second)


def cmd_shape(m, args):
    rows = m.projects(args.query, args.limit)
    if not rows:
        emit(keys="")
        return
    emit(keys=",".join(sorted(rows[0])))


def cmd_recents(m, args):
    if args.action == "note":
        for path in args.paths:
            m.note_project_pick(path)
        return
    if args.action == "score":
        now = time.time()
        record = (args.count, now - args.age_days * 86400.0)
        emit(score="%.6f" % m._recent_score(record, now))
        return
    store = m._recents_load()
    emit(entries=len(store), file=m._RECENTS_FILE)
    for path in sorted(store):
        n, _t = store[path]
        print("pick\t%s\t%d" % (path, n))


def cmd_supersede(m, args):
    """Two searches, the second issued while the first is still walking."""
    first = {}

    def run_first():
        try:
            first["rows"] = m.projects(args.query_a, 10)
        except m.ProjectSearchSuperseded:
            first["rows"] = []

    thread = threading.Thread(target=run_first)
    thread.start()
    time.sleep(args.gap)
    second = m.projects(args.query_b, 10)
    thread.join(20)
    # Sampled after both calls have returned, so no assertion depends on
    # catching the moment the cancel lands -- only on the walk having stopped
    # by the time anyone could look.
    settled = count(args.query_a)
    time.sleep(0.4)      # long enough that a walk still going would show it
    after = count(args.query_a)
    emit(first_rows=len(first.get("rows", ["unset"])),
         second_rows=len(second),
         first_listings_settled=settled,
         first_listings_after=after,
         # One listing may already be in flight when the cancel lands.
         stopped_walking="yes" if after - settled <= 2 else "no")


def cmd_join(m, args):
    """The same query twice at once: one walk should serve both callers."""
    first = {}

    def run_first():
        first["rows"] = m.projects(args.query, 10)

    thread = threading.Thread(target=run_first)
    thread.start()
    time.sleep(args.gap)
    second = m.projects(args.query, 10)
    thread.join(20)
    with COUNTS_LOCK:
        walks = WALKS[0]
    emit(first_rows=len(first.get("rows", [])), second_rows=len(second),
         walks=walks)


def cmd_spawn(m, args):
    """Start an agent the way the API does, so the pick history is exercised
    through the wiring that actually feeds it rather than around it."""
    emit(**m.spawn_agent(args.dir, args.agent))


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--slow", type=float, default=0.0,
                    help="seconds to charge for each directory listing")
    sub = ap.add_subparsers(dest="cmd")

    p = sub.add_parser("maketree")
    p.add_argument("root")
    p.add_argument("--width", type=int, default=3)
    p.add_argument("--depth", type=int, default=3)
    p.set_defaults(fn=cmd_maketree)

    p = sub.add_parser("walk")
    p.add_argument("--query", default="")
    p.add_argument("--limit", type=int, default=0)
    p.add_argument("--root", action="append")
    p.add_argument("--depth", type=int, default=None)
    p.add_argument("--budget", type=float, default=None)
    p.add_argument("--cancel-after", type=float, default=None)
    p.set_defaults(fn=cmd_walk)

    p = sub.add_parser("projects")
    p.add_argument("--query", default="")
    p.add_argument("--limit", type=int, default=40)
    p.add_argument("--field", default=None)
    p.set_defaults(fn=cmd_projects)

    p = sub.add_parser("cached")
    p.add_argument("--query", default="")
    p.add_argument("--limit", type=int, default=40)
    p.add_argument("--settle", type=float, default=0.0)
    p.set_defaults(fn=cmd_cached)

    p = sub.add_parser("shape")
    p.add_argument("--query", default="")
    p.add_argument("--limit", type=int, default=40)
    p.set_defaults(fn=cmd_shape)

    p = sub.add_parser("recents")
    p.add_argument("action", choices=("note", "dump", "score"))
    p.add_argument("paths", nargs="*")
    p.add_argument("--count", type=int, default=1)
    p.add_argument("--age-days", type=float, default=0.0)
    p.set_defaults(fn=cmd_recents)

    p = sub.add_parser("supersede")
    p.add_argument("--query-a", default="alpha")
    p.add_argument("--query-b", default="beta")
    p.add_argument("--gap", type=float, default=0.3)
    p.set_defaults(fn=cmd_supersede)

    p = sub.add_parser("join")
    p.add_argument("--query", default="")
    p.add_argument("--gap", type=float, default=0.3)
    p.set_defaults(fn=cmd_join)

    p = sub.add_parser("spawn")
    p.add_argument("dir")
    p.add_argument("--agent", default="claude")
    p.set_defaults(fn=cmd_spawn)

    args = ap.parse_args()
    if not getattr(args, "fn", None):
        ap.print_usage(sys.stderr)
        return 2

    m = load()
    if args.fn is not cmd_maketree:
        instrument(m, args.slow)
    args.fn(m, args)
    return 0


if __name__ == "__main__":
    sys.exit(main())
