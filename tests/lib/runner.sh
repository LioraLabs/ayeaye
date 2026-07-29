#!/usr/bin/env bash
# Run exactly one test function from one case file, in this process.
#
#   bash tests/lib/runner.sh <case-file> <test-name>
#
# One process per test is what makes the isolation guarantees cheap: globals,
# functions, the working directory, PATH and the sandbox all die with it.
# Exit status is the test result: 0 pass, 77 skip, anything else fail.
#
# Not meant to be called by hand; tests/run.sh drives it.

HARNESS_LIB="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TESTS_DIR="$(cd "$HARNESS_LIB/.." && pwd)"
REPO_ROOT="$(cd "$TESTS_DIR/.." && pwd)"
FIXTURES_DIR="$TESTS_DIR/fixtures"

TEST_FILE="$1"
TEST_NAME="$2"

if [ -z "$TEST_FILE" ] || [ -z "$TEST_NAME" ]; then
  echo "usage: runner.sh <case-file> <test-name>" >&2
  exit 2
fi

# shellcheck source=tests/lib/harness.sh
. "$HARNESS_LIB/harness.sh"

harness_begin

_runner_teardown() {
  local status=$?
  if declare -f teardown >/dev/null 2>&1; then
    teardown || true
  fi
  harness_end
  exit "$status"
}
trap _runner_teardown EXIT

# shellcheck source=/dev/null
. "$TEST_FILE" || { echo "runner: could not source $TEST_FILE" >&2; exit 1; }

if ! declare -f "$TEST_NAME" >/dev/null 2>&1; then
  echo "runner: $TEST_FILE defines no function named $TEST_NAME" >&2
  exit 1
fi

if declare -f setup >/dev/null 2>&1; then
  setup || { echo "runner: setup failed" >&2; exit 1; }
fi

cd "$TEST_TMPDIR" || exit 1

# set -u from here on: a misspelled variable in an assertion - assert_eq ""
# "$RUN_STDOTU" - would otherwise expand to the empty string and pass.
set -u
"$TEST_NAME"
status=$?

# An assertion that failed inside a subshell could only exit that subshell, so
# it left a note instead. A test cannot pass over one.
if [ -f "$TEST_TMPDIR/.assertion-failed" ]; then
  echo "runner: an assertion failed inside a subshell and could not end the test:" >&2
  sed 's/^/runner:   /' "$TEST_TMPDIR/.assertion-failed" >&2
  status=1
fi
exit "$status"
