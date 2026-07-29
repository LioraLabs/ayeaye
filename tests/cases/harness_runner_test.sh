# Self-tests for the runner: discovery, reporting, exit status, filters,
# isolation. These drive tests/run.sh over a throwaway case directory so the
# runner's own accounting is observable.

# Write a throwaway case file into $1/<name>_test.sh with body $2.
_write_case() {
  mkdir -p "$1"
  cat > "$1/$2" || fail "could not write case file"
}

# Run the real runner over a throwaway case dir. Sets RUNNER_OUT/RUNNER_STATUS.
_run_runner() {
  local dir="$1"; shift
  RUNNER_OUT="$(bash "$TESTS_DIR/run.sh" --cases "$dir" "$@" 2>&1)"
  RUNNER_STATUS=$?
  return 0
}

test_reports_each_test_and_summarises() {
  local d="$TEST_TMPDIR/cases"
  _write_case "$d" "alpha_test.sh" <<'CASE'
test_one() { assert_eq a a; }
test_two() { assert_eq b b; }
CASE
  _run_runner "$d"
  assert_status 0 "$RUNNER_STATUS" "all-passing run must exit 0"
  assert_contains "$RUNNER_OUT" "alpha_test::test_one"
  assert_contains "$RUNNER_OUT" "alpha_test::test_two"
  assert_contains "$RUNNER_OUT" "2 passed"
  assert_contains "$RUNNER_OUT" "0 failed"
}

test_failing_test_fails_the_run_but_others_still_run() {
  local d="$TEST_TMPDIR/cases"
  _write_case "$d" "beta_test.sh" <<'CASE'
test_first_fails() { assert_eq "wanted" "got"; }
test_second_still_runs() { assert_eq ok ok; }
CASE
  _run_runner "$d"
  assert_ne 0 "$RUNNER_STATUS" "a failing test must make the run exit non-zero"
  assert_contains "$RUNNER_OUT" "1 passed"
  assert_contains "$RUNNER_OUT" "1 failed"
  assert_contains "$RUNNER_OUT" "test_second_still_runs"
}

test_failure_output_shows_expected_and_actual() {
  local d="$TEST_TMPDIR/cases"
  _write_case "$d" "gamma_test.sh" <<'CASE'
test_mismatch() { assert_eq "the-expected-value" "the-actual-value"; }
CASE
  _run_runner "$d"
  assert_contains "$RUNNER_OUT" "assert_eq"
  assert_contains "$RUNNER_OUT" "the-expected-value"
  assert_contains "$RUNNER_OUT" "the-actual-value"
  assert_contains "$RUNNER_OUT" "gamma_test.sh:1" "failure must name the source line"
}

test_multiline_actual_is_not_truncated() {
  local d="$TEST_TMPDIR/cases"
  _write_case "$d" "multi_test.sh" <<'CASE'
test_multiline() {
  local actual
  actual="$(printf 'first-line\nsecond-line\nthird-line\n')"
  assert_eq "nope" "$actual"
}
CASE
  _run_runner "$d"
  assert_contains "$RUNNER_OUT" "first-line"
  assert_contains "$RUNNER_OUT" "second-line"
  assert_contains "$RUNNER_OUT" "third-line"
}

test_a_crashing_test_is_a_failure_not_a_runner_crash() {
  local d="$TEST_TMPDIR/cases"
  _write_case "$d" "crash_test.sh" <<'CASE'
test_explodes() { this_command_does_not_exist_anywhere; }
test_after_crash() { assert_eq ok ok; }
CASE
  _run_runner "$d"
  assert_ne 0 "$RUNNER_STATUS"
  assert_contains "$RUNNER_OUT" "1 passed"
  assert_contains "$RUNNER_OUT" "1 failed"
}

test_runs_tests_in_source_order() {
  local d="$TEST_TMPDIR/cases"
  _write_case "$d" "order_test.sh" <<'CASE'
test_zulu() { assert_eq ok ok; }
test_alpha() { assert_eq ok ok; }
CASE
  _run_runner "$d" --list
  local first
  first="$(printf '%s\n' "$RUNNER_OUT" | head -1)"
  assert_contains "$first" "test_zulu" "source order must win over alphabetical"
}

test_filter_selects_a_subset() {
  local d="$TEST_TMPDIR/cases"
  _write_case "$d" "filter_a_test.sh" <<'CASE'
test_keep_me() { assert_eq ok ok; }
test_drop_me() { assert_eq ok ok; }
CASE
  _run_runner "$d" keep_me
  assert_status 0 "$RUNNER_STATUS"
  assert_contains "$RUNNER_OUT" "test_keep_me"
  assert_not_contains "$RUNNER_OUT" "test_drop_me"
  assert_contains "$RUNNER_OUT" "1 passed"
}

test_filter_matches_file_name_too() {
  local d="$TEST_TMPDIR/cases"
  _write_case "$d" "wanted_test.sh" <<'CASE'
test_in_wanted() { assert_eq ok ok; }
CASE
  _write_case "$d" "ignored_test.sh" <<'CASE'
test_in_ignored() { assert_eq ok ok; }
CASE
  _run_runner "$d" wanted_test
  assert_contains "$RUNNER_OUT" "test_in_wanted"
  assert_not_contains "$RUNNER_OUT" "test_in_ignored"
}

test_filter_accepts_a_path() {
  local d="$TEST_TMPDIR/cases"
  _write_case "$d" "pathy_test.sh" <<'CASE'
test_by_path() { assert_eq ok ok; }
CASE
  _write_case "$d" "other_test.sh" <<'CASE'
test_not_by_path() { assert_eq ok ok; }
CASE
  _run_runner "$d" "$d/pathy_test.sh"
  assert_contains "$RUNNER_OUT" "test_by_path"
  assert_not_contains "$RUNNER_OUT" "test_not_by_path"
}

test_list_does_not_execute_tests() {
  local d="$TEST_TMPDIR/cases"
  _write_case "$d" "listing_test.sh" <<'CASE'
test_would_touch_disk() { : > "$TEST_TMPDIR/../marker-should-not-exist"; }
CASE
  _run_runner "$d" --list
  assert_status 0 "$RUNNER_STATUS"
  assert_contains "$RUNNER_OUT" "listing_test::test_would_touch_disk"
  assert_not_contains "$RUNNER_OUT" "passed"
}

test_a_test_the_scanner_cannot_place_still_runs() {
  local d="$TEST_TMPDIR/cases"
  _write_case "$d" "generated_test.sh" <<'CASE'
test_written_out() { assert_eq ok ok; }
eval 'test_generated_by_eval() { assert_eq ok ok; }'
CASE
  _run_runner "$d"
  assert_status 0 "$RUNNER_STATUS"
  assert_contains "$RUNNER_OUT" "test_generated_by_eval" \
    "a test the text scan cannot place must still run, never be skipped in silence"
  assert_contains "$RUNNER_OUT" "2 passed"
}

test_the_function_keyword_and_spaced_parentheses_are_understood() {
  local d="$TEST_TMPDIR/cases"
  _write_case "$d" "spelling_test.sh" <<'CASE'
test_plain() { assert_eq ok ok; }
test_spaced ()  { assert_eq ok ok; }
function test_keyword() { assert_eq ok ok; }
CASE
  _run_runner "$d"
  assert_status 0 "$RUNNER_STATUS"
  assert_contains "$RUNNER_OUT" "3 passed"
}

test_no_matching_filter_is_an_error() {
  local d="$TEST_TMPDIR/cases"
  _write_case "$d" "nomatch_test.sh" <<'CASE'
test_something() { assert_eq ok ok; }
CASE
  _run_runner "$d" definitely-not-a-test-name
  assert_ne 0 "$RUNNER_STATUS" "a filter matching nothing must not silently pass"
}

test_skip_is_reported_separately() {
  local d="$TEST_TMPDIR/cases"
  _write_case "$d" "skipper_test.sh" <<'CASE'
test_skipped() { skip "not applicable here"; fail "skip must stop the test"; }
test_ran() { assert_eq ok ok; }
CASE
  _run_runner "$d"
  assert_status 0 "$RUNNER_STATUS" "a skip is not a failure"
  assert_contains "$RUNNER_OUT" "skip"
  assert_contains "$RUNNER_OUT" "not applicable here"
  assert_contains "$RUNNER_OUT" "1 skipped"
}

test_setup_and_teardown_run_around_each_test() {
  local d="$TEST_TMPDIR/cases"
  _write_case "$d" "fixture_hooks_test.sh" <<'CASE'
setup() { echo "SETUP-RAN" >&2; }
teardown() { echo "TEARDOWN-RAN" >&2; }
test_needs_setup() { fail "boom"; }
CASE
  _run_runner "$d"
  assert_contains "$RUNNER_OUT" "SETUP-RAN"
  assert_contains "$RUNNER_OUT" "TEARDOWN-RAN" "teardown must run even when the test fails"
}

test_a_hanging_test_is_killed_by_the_timeout() {
  local d="$TEST_TMPDIR/cases"
  _write_case "$d" "hang_test.sh" <<'CASE'
test_hangs() { sleep 30; }
CASE
  local started ended
  started=$(date +%s)
  TEST_TIMEOUT=2 _run_runner "$d"
  ended=$(date +%s)
  assert_ne 0 "$RUNNER_STATUS"
  assert_contains "$RUNNER_OUT" "timeout"
  [ "$((ended - started))" -lt 20 ] || fail "timeout did not cut the run short (took $((ended - started))s)"
}

test_each_test_gets_a_fresh_sandbox() {
  local d="$TEST_TMPDIR/cases"
  _write_case "$d" "sandbox_test.sh" <<'CASE'
test_first() {
  printf '%s\n' "$TEST_TMPDIR" >> "$SELFTEST_RECORD"
  : > "$TEST_TMPDIR/leftover"
}
test_second() {
  printf '%s\n' "$TEST_TMPDIR" >> "$SELFTEST_RECORD"
  assert_file_missing "$TEST_TMPDIR/leftover" "sandbox leaked between tests"
}
CASE
  SELFTEST_RECORD="$TEST_TMPDIR/record" _run_runner "$d"
  assert_status 0 "$RUNNER_STATUS"
  local first second
  first="$(sed -n 1p "$TEST_TMPDIR/record")"
  second="$(sed -n 2p "$TEST_TMPDIR/record")"
  assert_ne "$first" "$second" "each test must get its own sandbox"
  assert_file_missing "$first" "the sandbox must be removed once the test ends"
}

test_home_and_xdg_point_inside_the_tmpdir() {
  local d="$TEST_TMPDIR/cases"
  _write_case "$d" "isolation_test.sh" <<'CASE'
test_env_is_redirected() {
  case "$HOME" in
    "$TEST_TMPDIR"/*) : ;;
    *) fail "HOME must live under TEST_TMPDIR, got: $HOME" ;;
  esac
  case "$XDG_CONFIG_HOME" in "$TEST_TMPDIR"/*) : ;; *) fail "XDG_CONFIG_HOME leaked: $XDG_CONFIG_HOME" ;; esac
  case "$XDG_STATE_HOME"  in "$TEST_TMPDIR"/*) : ;; *) fail "XDG_STATE_HOME leaked: $XDG_STATE_HOME" ;; esac
  case "$XDG_DATA_HOME"   in "$TEST_TMPDIR"/*) : ;; *) fail "XDG_DATA_HOME leaked: $XDG_DATA_HOME" ;; esac
  case "$XDG_CACHE_HOME"  in "$TEST_TMPDIR"/*) : ;; *) fail "XDG_CACHE_HOME leaked: $XDG_CACHE_HOME" ;; esac
  case "$TMPDIR"          in "$TEST_TMPDIR"/*) : ;; *) fail "TMPDIR leaked: $TMPDIR" ;; esac
  [ -d "$HOME" ] || fail "HOME must be a real directory"
}
CASE
  _run_runner "$d"
  assert_status 0 "$RUNNER_STATUS"
}

test_ambient_app_env_is_scrubbed() {
  local d="$TEST_TMPDIR/cases"
  _write_case "$d" "scrub_test.sh" <<'CASE'
test_no_ayeaye_env() {
  [ -z "${AYEAYE_PORT:-}" ] || fail "AYEAYE_PORT leaked in from the caller: ${AYEAYE_PORT:-}"
  [ -z "${AYEAYE_BIND:-}" ] || fail "AYEAYE_BIND leaked in from the caller: ${AYEAYE_BIND:-}"
}
CASE
  AYEAYE_PORT=9999 AYEAYE_BIND=0.0.0.0 _run_runner "$d"
  assert_status 0 "$RUNNER_STATUS"
}

test_repo_root_and_tests_dir_are_exported() {
  local d="$TEST_TMPDIR/cases"
  _write_case "$d" "paths_test.sh" <<'CASE'
test_paths() {
  [ -x "$REPO_ROOT/install.sh" ] || fail "REPO_ROOT must point at the repo, got: $REPO_ROOT"
  [ -f "$TESTS_DIR/run.sh" ] || fail "TESTS_DIR must point at tests/, got: $TESTS_DIR"
  [ -d "$FIXTURES_DIR" ] || fail "FIXTURES_DIR must exist, got: $FIXTURES_DIR"
}
CASE
  _run_runner "$d"
  assert_status 0 "$RUNNER_STATUS"
}
