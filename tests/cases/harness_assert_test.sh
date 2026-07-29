# Self-tests for the assertion helpers themselves. The happy paths are checked
# inline; the failure paths are checked by running an assertion in a subshell,
# because a failing assertion ends the test that called it.

# _assert_fails <assertion> [args...] -> captures stderr in ASSERTION_OUTPUT
# and the exit status in ASSERTION_STATUS.
_assert_fails() {
  ASSERTION_OUTPUT="$( ( "$@" ) 2>&1 )"
  ASSERTION_STATUS=$?
  return 0
}

test_equality_and_negation() {
  assert_eq "same" "same"
  assert_ne "one" "other"
  _assert_fails assert_eq "left" "right"
  assert_status 1 "$ASSERTION_STATUS"
  assert_contains "$ASSERTION_OUTPUT" "left"
  assert_contains "$ASSERTION_OUTPUT" "right"
  _assert_fails assert_ne "identical" "identical"
  assert_status 1 "$ASSERTION_STATUS"
}

test_substring_assertions() {
  assert_contains "the quick brown fox" "quick"
  assert_not_contains "the quick brown fox" "slow"
  _assert_fails assert_contains "haystack" "needle"
  assert_contains "$ASSERTION_OUTPUT" "expected to contain"
  _assert_fails assert_not_contains "haystack" "hay"
  assert_contains "$ASSERTION_OUTPUT" "expected not to contain"
}

test_regex_assertion() {
  assert_matches "port 8911" '^port [0-9]+$'
  _assert_fails assert_matches "port eight" '^port [0-9]+$'
  assert_status 1 "$ASSERTION_STATUS"
}

test_exit_code_assertion() {
  local status
  ( exit 3 ); status=$?
  assert_status 3 "$status"
  _assert_fails assert_status 0 2
  assert_contains "$ASSERTION_OUTPUT" "expected exit status"
}

test_empty_values_are_shown_not_silently_dropped() {
  _assert_fails assert_eq "wanted" ""
  assert_contains "$ASSERTION_OUTPUT" "<empty>"
}

test_file_content_assertions() {
  printf 'AYEAYE_PORT=8911\nAYEAYE_BIND=127.0.0.1\n' > "$TEST_TMPDIR/env"
  assert_file_exists "$TEST_TMPDIR/env"
  assert_file_missing "$TEST_TMPDIR/nothing-here"
  assert_file_contains "$TEST_TMPDIR/env" "AYEAYE_PORT=8911"
  assert_file_not_contains "$TEST_TMPDIR/env" "@PORT@"

  _assert_fails assert_file_contains "$TEST_TMPDIR/env" "@PORT@"
  assert_contains "$ASSERTION_OUTPUT" "AYEAYE_BIND=127.0.0.1" "must show the real contents"

  _assert_fails assert_file_contains "$TEST_TMPDIR/absent" "anything"
  assert_contains "$ASSERTION_OUTPUT" "no such file"

  _assert_fails assert_file_exists "$TEST_TMPDIR/absent"
  assert_status 1 "$ASSERTION_STATUS"
}

test_file_mode_assertion() {
  : > "$TEST_TMPDIR/secret"
  chmod 600 "$TEST_TMPDIR/secret"
  assert_file_mode 600 "$TEST_TMPDIR/secret"
  chmod 755 "$TEST_TMPDIR/secret"
  assert_file_mode 755 "$TEST_TMPDIR/secret"
  chmod 644 "$TEST_TMPDIR/secret"
  _assert_fails assert_file_mode 600 "$TEST_TMPDIR/secret"
  assert_contains "$ASSERTION_OUTPUT" "644"
}

test_failure_names_the_calling_line() {
  _assert_fails fail "deliberate"
  assert_contains "$ASSERTION_OUTPUT" "harness_assert_test.sh"
  assert_contains "$ASSERTION_OUTPUT" "deliberate"
}
