# What a card says a session is doing.
#
# The behaviour lives in python, so the assertions do too: this file is the
# bridge that puts tests/python/session_state.py under `tests/run.sh`, one
# bash test per group of behaviours. That python file names each behaviour
# individually and its output is printed in full when anything here fails.
#
# Nothing here runs tmux or an agent. Both transcript formats are written to a
# temporary file exactly as the agents write them, which is the only way to
# test a state that depends on the difference between an acknowledgement and
# an answer.

setup() {
  require_host_command python3
  stub_real python3
  STATE_TESTS="$REPO_ROOT/tests/python/session_state.py"
}

run_state_tests() {
  run_script "$STATE_TESTS" "$@"
}

test_whose_turn_it_is_comes_from_the_last_thing_in_the_transcript() {
  run_state_tests ClaudeStates CodexStates
  assert_status 0 "$RUN_STATUS" "$RUN_OUTPUT"
}

test_an_agent_waiting_on_its_own_agents_is_delegating_not_idle() {
  run_state_tests ClaudeDelegation
  assert_status 0 "$RUN_STATUS" "$RUN_OUTPUT"
}

test_the_codex_wait_loop_reads_as_one_delegation_rather_than_flicker() {
  run_state_tests CodexDelegation
  assert_status 0 "$RUN_STATUS" "$RUN_OUTPUT"
}

test_how_long_ago_comes_from_the_conversation_and_not_the_file() {
  run_state_tests Age
  assert_status 0 "$RUN_STATUS" "$RUN_OUTPUT"
}

test_the_server_and_the_page_agree_on_the_vocabulary() {
  run_state_tests TheStatesThemselves
  assert_status 0 "$RUN_STATUS" "$RUN_OUTPUT"
}
