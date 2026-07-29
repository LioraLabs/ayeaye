# Running the code under test, two ways.
#
#   run_script  - a plain pipe run. Keeps stdout and stderr apart, captures the
#                 exit status, never lets a non-zero exit end the test. Use it
#                 for everything that is not about prompts.
#   pty_run     - an interactive run through a real terminal, driven by scripted
#                 expectations. Use it whenever a prompt, a default or the order
#                 of questions is the thing under test, because bash prints a
#                 `read -p` prompt only to a terminal: under a pipe the prompt
#                 text does not exist at all.
#
# bash 3.2: no associative arrays, no ${var,,}, no mapfile.

_RUN_SEQ=0

# stdin_lines <line>... - queue standard input for the next run_script. The
# queue is consumed by that run, so successive runs in one test do not share it.
stdin_lines() {
  RUN_STDIN_FILE="$TEST_TMPDIR/run.stdin"
  : > "$RUN_STDIN_FILE"
  local line
  for line in "$@"; do
    printf '%s\n' "$line" >> "$RUN_STDIN_FILE"
  done
}

# run_script <script> [args...]
#
# Sets:
#   RUN_STATUS       exit status
#   RUN_STDOUT       stdout, trailing newlines trimmed
#   RUN_STDERR       stderr, trailing newlines trimmed
#   RUN_OUTPUT       stdout then stderr, for "it said this somewhere" checks
#   RUN_STDOUT_FILE  / RUN_STDERR_FILE  the raw captures, kept for diagnostics
run_script() {
  local script="$1"
  shift
  _RUN_SEQ=$((_RUN_SEQ + 1))
  RUN_STDOUT_FILE="$TEST_TMPDIR/run.$_RUN_SEQ.out"
  RUN_STDERR_FILE="$TEST_TMPDIR/run.$_RUN_SEQ.err"

  local input="/dev/null"
  if [ -n "${RUN_STDIN_FILE:-}" ] && [ -f "${RUN_STDIN_FILE:-}" ]; then
    input="$RUN_STDIN_FILE"
  fi

  "$script" "$@" >"$RUN_STDOUT_FILE" 2>"$RUN_STDERR_FILE" <"$input"
  RUN_STATUS=$?

  RUN_STDIN_FILE=""
  RUN_STDOUT="$(cat "$RUN_STDOUT_FILE")"
  RUN_STDERR="$(cat "$RUN_STDERR_FILE")"
  RUN_OUTPUT="$RUN_STDOUT
$RUN_STDERR"
  return 0
}

# run_install [args...] - run this repository's installer under the sandbox.
run_install() {
  run_script "$REPO_ROOT/install.sh" "$@"
}

# ------------------------------------------------------- interactive running

# The substring every prompt in this project ends with: `ask` renders
# "bind address [127.0.0.1]: ". Override PTY_PROMPT_PATTERN if that changes.
PTY_PROMPT_PATTERN="]: "

# pty_expect <substring> <response> - wait for the substring, then type the
# response. Rules are consumed in the order they were declared.
pty_expect() {
  [ -n "${PTY_EXPECT_FILE:-}" ] || { PTY_EXPECT_FILE="$TEST_TMPDIR/pty.expect"; : > "$PTY_EXPECT_FILE"; }
  printf '%s\t%s\n' "$1" "$2" >> "$PTY_EXPECT_FILE"
}

# pty_answers <answer>... - the common case: answer each prompt in turn. An
# empty answer types nothing but the newline, which is how the script's own
# default gets exercised.
pty_answers() {
  local answer
  for answer in "$@"; do
    pty_expect "$PTY_PROMPT_PATTERN" "$answer"
  done
}

# pty_run <script> [args...]
#
# Sets:
#   PTY_TRANSCRIPT  everything the terminal saw, prompts and echoed answers
#                   included, with CRLF normalised to LF
#   PTY_STATUS      the script's exit status
#
# An expectation that never arrives, or a script that never exits, fails the
# test with the transcript so far rather than hanging the suite.
pty_run() {
  local script="$1"
  shift
  _RUN_SEQ=$((_RUN_SEQ + 1))
  local transcript="$TEST_TMPDIR/pty.$_RUN_SEQ.transcript"
  local status_file="$TEST_TMPDIR/pty.$_RUN_SEQ.status"
  local driver_err="$TEST_TMPDIR/pty.$_RUN_SEQ.driver-err"
  local expect_file="${PTY_EXPECT_FILE:-}"
  local driver_status

  [ -n "$expect_file" ] || { expect_file="$TEST_TMPDIR/pty.empty.expect"; : > "$expect_file"; }
  [ -n "${HARNESS_PYTHON:-}" ] || fail "pty_run needs python3 on the host"

  "$HARNESS_PYTHON" "$HARNESS_LIB/pty_run.py" \
    --expect "$expect_file" \
    --transcript "$transcript" \
    --status-file "$status_file" \
    --timeout "${PTY_TIMEOUT:-20}" \
    -- "$script" "$@" 2>"$driver_err"
  driver_status=$?

  PTY_EXPECT_FILE=""
  PTY_TRANSCRIPT="$(cat "$transcript" 2>/dev/null)"
  PTY_STATUS="$(cat "$status_file" 2>/dev/null)"

  if [ "$driver_status" != 0 ]; then
    _assert_fail "${BASH_SOURCE[1]}:${BASH_LINENO[0]}" "pty_run" "the interactive run did not complete" \
      "driver said" "$(cat "$driver_err" 2>/dev/null)" \
      "transcript so far" "$PTY_TRANSCRIPT"
  fi
  return 0
}

# pty_install [args...] - the installer, interactively.
pty_install() {
  pty_run "$REPO_ROOT/install.sh" "$@"
}
