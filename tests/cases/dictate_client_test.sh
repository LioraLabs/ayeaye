# Which device bin/voice-dictate opens a microphone on.
#
# Dictation records where the tmux client is sitting, so the two functions
# that pick a client decide whose room is recorded. They used to answer out of
# /proc, which a Mac does not have: every client looked local, no client was
# ever preferred, and the first one in the list got the microphone with no
# error anywhere. That is the bug this file guards.
#
# The behaviour lives in python, so the assertions do too: this file is the
# bridge that puts tests/python/dictate_client.py under `tests/run.sh`, one
# bash test per group of behaviours. That python file names each behaviour
# individually and its output is printed in full when anything here fails.

setup() {
  require_host_command python3
  stub_real python3
  CLIENT_TESTS="$REPO_ROOT/tests/python/dictate_client.py"
}

# run_client_tests <test-id>... - run those python tests and remember the result.
run_client_tests() {
  run_script "$CLIENT_TESTS" "$@"
}

test_the_client_questions_answer_the_same_as_they_did_against_proc() {
  # The regression pin, and the reason it is worth having: these assertions
  # were written against the pre-extraction file, run green there, and are
  # unchanged. Real child processes with real environments, read back out of
  # this host's own /proc.
  case "$OSTYPE" in
    linux*) ;;
    *) skip "the /proc oracle needs a linux host" ;;
  esac
  run_client_tests LiveProcTest
  assert_status 0 "$RUN_STATUS" "$RUN_OUTPUT"
}

test_a_macos_client_is_picked_with_no_proc_anywhere() {
  # "no proc anywhere" is enforced inside: every open and readlink the test
  # makes is watched, and one under /proc fails it.
  run_client_tests DarwinClientTest
  assert_status 0 "$RUN_STATUS" "$RUN_OUTPUT"
}

test_the_choice_between_clients_is_made_the_same_way_on_both_platforms() {
  run_client_tests BranchTest
  assert_status 0 "$RUN_STATUS" "$RUN_OUTPUT"
}

test_the_dictation_command_reaches_for_proc_nowhere() {
  run_client_tests NoProcTest
  assert_status 0 "$RUN_STATUS" "$RUN_OUTPUT"
}

test_the_whole_python_suite_passes() {
  run_client_tests
  assert_status 0 "$RUN_STATUS" "$RUN_OUTPUT"
  assert_not_contains "$RUN_OUTPUT" "FAILED"
}
