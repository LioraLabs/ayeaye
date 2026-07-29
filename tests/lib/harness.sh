# Per-test sandbox and the variables every test can rely on. Sourced by
# tests/lib/runner.sh before the case file; never run directly.
#
# The contract, in one sentence: a test may write anywhere under $TEST_TMPDIR
# and nowhere else. Everything that would normally resolve to the real home
# directory is redirected inside it before the test's first line runs.
#
# Exported for tests:
#   TEST_TMPDIR    per-test sandbox root, removed after the test
#   TEST_NAME      the running test function's name
#   TEST_FILE      absolute path of the case file
#   TESTS_DIR      absolute path of tests/
#   REPO_ROOT      absolute path of the repository
#   FIXTURES_DIR   absolute path of tests/fixtures
#   STUB_BIN       directory of command stubs, see lib/stub.sh
#   STUB_LOG_DIR   where stubs record their argv, see lib/stub.sh
#
# bash 3.2: no associative arrays, no ${var,,}, no mapfile.

# shellcheck source=tests/lib/assert.sh
. "$HARNESS_LIB/assert.sh"
# shellcheck source=tests/lib/fixture.sh
. "$HARNESS_LIB/fixture.sh"
# shellcheck source=tests/lib/stub.sh
. "$HARNESS_LIB/stub.sh"

# Build the sandbox and redirect the environment into it. Called by the runner.
harness_begin() {
  TEST_TMPDIR="$(mktemp -d "${TMPDIR:-/tmp}/ayeaye-test.XXXXXX")" \
    || { echo "harness: cannot create sandbox" >&2; exit 1; }

  HOME="$TEST_TMPDIR/home"
  XDG_CONFIG_HOME="$HOME/.config"
  XDG_STATE_HOME="$HOME/.local/state"
  XDG_DATA_HOME="$HOME/.local/share"
  XDG_CACHE_HOME="$HOME/.cache"
  XDG_RUNTIME_DIR="$TEST_TMPDIR/run"
  TMPDIR="$TEST_TMPDIR/tmp"
  STUB_BIN="$TEST_TMPDIR/stub-bin"
  STUB_LOG_DIR="$TEST_TMPDIR/stub-log"

  mkdir -p "$HOME" "$XDG_CONFIG_HOME" "$XDG_STATE_HOME" "$XDG_DATA_HOME" \
           "$XDG_CACHE_HOME" "$XDG_RUNTIME_DIR" "$TMPDIR" "$STUB_BIN" "$STUB_LOG_DIR"

  export HOME XDG_CONFIG_HOME XDG_STATE_HOME XDG_DATA_HOME XDG_CACHE_HOME \
         XDG_RUNTIME_DIR TMPDIR TEST_TMPDIR STUB_BIN STUB_LOG_DIR \
         TESTS_DIR REPO_ROOT FIXTURES_DIR HARNESS_LIB

  # Ambient application configuration must not reach the code under test: a
  # contributor with AYEAYE_PORT exported would otherwise get different
  # results from a clean machine.
  harness_scrub_env AYEAYE_ VOICE_

  # Keep locale and terminal behaviour identical everywhere.
  LC_ALL=C
  LANG=C
  TERM=dumb
  export LC_ALL LANG TERM
  unset CDPATH

  # Harness infrastructure that happens to need an interpreter resolves it
  # from the host before PATH is narrowed, so that the code under test can
  # still be told python3 does not exist.
  HARNESS_PYTHON="$(command -v python3 2>/dev/null || true)"
  export HARNESS_PYTHON

  # PATH becomes exactly $STUB_BIN from here on; see lib/stub.sh.
  stub_begin
}

# harness_scrub_env <prefix>... - unset every environment variable whose name
# starts with one of the given prefixes.
harness_scrub_env() {
  local prefix name
  for prefix in "$@"; do
    for name in $(env | sed -n "s/^\\($prefix[A-Za-z0-9_]*\\)=.*/\\1/p"); do
      unset "$name"
    done
  done
}

# Remove the sandbox. KEEP_TMPDIR=1 leaves it in place and prints the path,
# which is how you inspect what a failing test actually wrote.
harness_end() {
  [ -n "${TEST_TMPDIR:-}" ] || return 0
  if [ "${KEEP_TMPDIR:-0}" = 1 ]; then
    printf 'sandbox kept: %s\n' "$TEST_TMPDIR" >&2
    return 0
  fi
  rm -rf "$TEST_TMPDIR"
}
