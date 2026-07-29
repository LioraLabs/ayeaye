#!/usr/bin/env bash
# The whole test suite, with no dependencies beyond bash and coreutils.
#
#   tests/run.sh                     run everything
#   tests/run.sh install_env         run tests whose id contains "install_env"
#   tests/run.sh tests/cases/x_test.sh
#   tests/run.sh --list              print test ids, run nothing
#   tests/run.sh -v                  show output of passing tests too
#   tests/run.sh --timeout 30        per-test watchdog, seconds
#
# A test id is "<case-file-without-.sh>::<test-function>". Filters are
# substrings matched against that id, so a container job can run a named
# subset with the same command a contributor uses locally.
#
# Exit status is 0 only when every selected test passed or skipped.
#
# bash 3.2: no associative arrays, no ${var,,}, no mapfile.

set -u

TESTS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LIB_DIR="$TESTS_DIR/lib"
CASES_DIR="$TESTS_DIR/cases"

VERBOSE=0
LIST_ONLY=0
TIMEOUT="${TEST_TIMEOUT:-120}"
FILTERS=""
FILTER_COUNT=0

usage() {
  sed -n '2,18p' "$0" | sed 's/^# \{0,1\}//'
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    -h|--help)    usage; exit 0 ;;
    -v|--verbose) VERBOSE=1 ;;
    -l|--list)    LIST_ONLY=1 ;;
    --cases)      CASES_DIR="$2"; shift ;;
    --timeout)    TIMEOUT="$2"; shift ;;
    -*)           echo "unknown option: $1 (try --help)" >&2; exit 2 ;;
    *)
      # A path filter is reduced to its bare case-file name, so both
      # "tests/cases/install_env_test.sh" and "install_env" select the file.
      filter="$1"
      case "$filter" in */*) filter="${filter##*/}" ;; esac
      filter="${filter%.sh}"
      FILTERS="$FILTERS$filter
"
      FILTER_COUNT=$((FILTER_COUNT + 1))
      ;;
  esac
  shift
done

if [ ! -d "$CASES_DIR" ]; then
  echo "no such case directory: $CASES_DIR" >&2
  exit 2
fi

# ------------------------------------------------------------------ tripwire
# The suite must not touch the real machine. These are exactly the paths
# install.sh writes; if any of their signatures changes across a run then a
# test escaped its sandbox, and the run is void whatever the assertions said.
GUARDED_PATHS="${XDG_CONFIG_HOME:-$HOME/.config}/ayeaye/env
${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user/ayeaye.service
${XDG_STATE_HOME:-$HOME/.local/state}/ayeaye/token"

guard_signature() {
  printf '%s\n' "$GUARDED_PATHS" | while IFS= read -r guarded; do
    [ -n "$guarded" ] || continue
    if [ -e "$guarded" ]; then
      printf '%s %s\n' "$(ls -ld "$guarded" 2>&1)" "$(cksum <"$guarded" 2>/dev/null)"
    else
      printf '%s absent\n' "$guarded"
    fi
  done
}

# ----------------------------------------------------------------- discovery
# Test functions run in source order, not alphabetical order: a case file
# reads top to bottom and its output should too. Source order comes from the
# text, but the text alone over-reports (a here-document can contain something
# that looks like a definition), so the scan is intersected with the functions
# the file actually defines when sourced.
test_names_in() {
  local defined seen name
  defined="$(bash "$LIB_DIR/list_functions.sh" "$1" 2>/dev/null)"
  seen=""
  for name in $(sed -n 's/^\(test_[A-Za-z0-9_]*\)[[:space:]]*()[[:space:]]*{.*/\1/p' "$1"); do
    printf '%s\n' "$defined" | grep -qx "$name" || continue
    case "$seen" in
      *" $name "*) continue ;;
    esac
    seen="$seen $name "
    printf '%s\n' "$name"
  done
}

selected() {
  local id="$1" filter
  [ "$FILTER_COUNT" = 0 ] && return 0
  while IFS= read -r filter; do
    [ -n "$filter" ] || continue
    case "$id" in
      *"$filter"*) return 0 ;;
    esac
  done <<EOF
$FILTERS
EOF
  return 1
}

CASE_FILES=""
while IFS= read -r case_file; do
  [ -n "$case_file" ] || continue
  CASE_FILES="$CASE_FILES$case_file
"
done <<EOF
$(find "$CASES_DIR" -type f -name '*_test.sh' | LC_ALL=C sort)
EOF

SELECTION=""
SELECTION_COUNT=0
while IFS= read -r case_file; do
  [ -n "$case_file" ] || continue
  base="${case_file##*/}"
  base="${base%.sh}"
  for name in $(test_names_in "$case_file"); do
    id="$base::$name"
    if selected "$id"; then
      SELECTION="$SELECTION$case_file|$id|$name
"
      SELECTION_COUNT=$((SELECTION_COUNT + 1))
    fi
  done
done <<EOF
$CASE_FILES
EOF

if [ "$SELECTION_COUNT" = 0 ]; then
  if [ "$FILTER_COUNT" -gt 0 ]; then
    echo "no test matched the filter" >&2
  else
    echo "no tests found under $CASES_DIR" >&2
  fi
  exit 2
fi

if [ "$LIST_ONLY" = 1 ]; then
  while IFS='|' read -r case_file id name; do
    [ -n "$id" ] || continue
    printf '%s\n' "$id"
  done <<EOF
$SELECTION
EOF
  exit 0
fi

# ------------------------------------------------------------------- running
RUN_TMP="$(mktemp -d "${TMPDIR:-/tmp}/ayeaye-run.XXXXXX")"
trap 'rm -rf "$RUN_TMP"' EXIT INT TERM

BEFORE="$(guard_signature)"

PASSED=0
FAILED=0
SKIPPED=0
FAILED_IDS=""
STARTED="$(date +%s)"

# Run one test under a watchdog. Sets RESULT_STATUS; 124 means it was killed.
run_one() {
  local case_file="$1" name="$2" out="$3"
  local marker="$RUN_TMP/timeout.marker"
  local pid watchdog
  rm -f "$marker"

  bash "$LIB_DIR/runner.sh" "$case_file" "$name" >"$out" 2>&1 &
  pid=$!
  (
    waited=0
    while [ "$waited" -lt "$TIMEOUT" ]; do
      kill -0 "$pid" 2>/dev/null || exit 0
      sleep 1
      waited=$((waited + 1))
    done
    if kill -0 "$pid" 2>/dev/null; then
      : >"$marker"
      kill -9 "$pid" 2>/dev/null
    fi
  ) 2>/dev/null &
  watchdog=$!

  wait "$pid" 2>/dev/null
  RESULT_STATUS=$?
  kill "$watchdog" 2>/dev/null
  wait "$watchdog" 2>/dev/null

  if [ -f "$marker" ]; then
    rm -f "$marker"
    printf 'timeout: killed after %ss\n' "$TIMEOUT" >>"$out"
    RESULT_STATUS=124
  fi
}

show_output() {
  [ -s "$1" ] || return 0
  sed 's/^/    /' "$1"
}

printf '%s' "$SELECTION" > "$RUN_TMP/selection"
while IFS='|' read -r case_file id name; do
  [ -n "$id" ] || continue
  out="$RUN_TMP/out"
  run_one "$case_file" "$name" "$out"
  case "$RESULT_STATUS" in
    0)
      PASSED=$((PASSED + 1))
      printf 'ok       %s\n' "$id"
      [ "$VERBOSE" = 1 ] && show_output "$out"
      ;;
    77)
      SKIPPED=$((SKIPPED + 1))
      printf 'skip     %s\n' "$id"
      show_output "$out"
      ;;
    124)
      FAILED=$((FAILED + 1))
      FAILED_IDS="$FAILED_IDS $id"
      printf 'FAIL     %s (timeout)\n' "$id"
      show_output "$out"
      ;;
    *)
      FAILED=$((FAILED + 1))
      FAILED_IDS="$FAILED_IDS $id"
      printf 'FAIL     %s (exit %s)\n' "$id" "$RESULT_STATUS"
      show_output "$out"
      ;;
  esac
done < "$RUN_TMP/selection"

ELAPSED=$(( $(date +%s) - STARTED ))

AFTER="$(guard_signature)"
if [ "$BEFORE" != "$AFTER" ]; then
  {
    echo ""
    echo "SANDBOX ESCAPE: a test modified the real machine."
    echo "before:"; printf '%s\n' "$BEFORE"
    echo "after:";  printf '%s\n' "$AFTER"
  } >&2
  exit 1
fi

echo ""
printf '%s passed, %s failed, %s skipped in %ss\n' \
  "$PASSED" "$FAILED" "$SKIPPED" "$ELAPSED"
if [ "$FAILED" -gt 0 ]; then
  printf 'failed:%s\n' "$FAILED_IDS"
  exit 1
fi
exit 0
