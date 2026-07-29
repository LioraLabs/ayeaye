#!/usr/bin/env bash
# Print the test functions a case file really defines, one per line.
#
#   bash tests/lib/list_functions.sh <case-file>
#
# Scanning the text alone is not enough: a case file may legitimately contain
# the text "test_x() {" inside a here-document (the harness self-tests do), and
# that is not a test. Sourcing alone is not enough either, because it loses
# source order. tests/run.sh intersects the two.
#
# A case file must therefore define functions and nothing else: this sources it
# outside any sandbox, so top-level statements would run on the real machine.
set -u

[ "$#" = 1 ] || { echo "usage: list_functions.sh <case-file>" >&2; exit 2; }

# shellcheck source=/dev/null
. "$1" >/dev/null 2>&1 </dev/null

compgen -A function | grep '^test_'
exit 0
