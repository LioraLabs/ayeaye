# When a push notification is worth sending.
#
# A notification marks the moment a session became your problem -- arriving at
# a prompt, or handing the turn back -- and never the fact that it still is,
# nor anything it does on its own in between. Getting that wrong in either
# direction ruins it: notify on states and the same waiting session pages you
# every ten seconds; notify on every change and your own reply pages you back.
#
# The assertions live in tests/python/notify_watch.py, driven as sequences of
# sweeps; this file is the bridge that puts them under `tests/run.sh`. Nothing
# here reaches ntfy or runs tmux.

setup() {
  require_host_command python3
  stub_real python3
  NOTIFY_TESTS="$REPO_ROOT/tests/python/notify_watch.py"
}

run_notify_tests() {
  run_script "$NOTIFY_TESTS" "$@"
}

test_an_agent_handing_the_turn_back_notifies_once() {
  run_notify_tests TurnHandedBack
  assert_status 0 "$RUN_STATUS" "$RUN_OUTPUT"
}

test_replying_to_an_agent_never_notifies_you_about_yourself() {
  run_notify_tests YouReplying
  assert_status 0 "$RUN_STATUS" "$RUN_OUTPUT"
}

test_an_agent_getting_on_with_its_work_stays_quiet() {
  run_notify_tests WorkingIsNotNews
  assert_status 0 "$RUN_STATUS" "$RUN_OUTPUT"
}

test_a_pane_stopped_at_a_prompt_notifies_per_question() {
  run_notify_tests StoppedAtAPrompt
  assert_status 0 "$RUN_STATUS" "$RUN_OUTPUT"
}

test_restarts_panes_and_settings_do_not_confuse_the_watcher() {
  run_notify_tests Housekeeping
  assert_status 0 "$RUN_STATUS" "$RUN_OUTPUT"
}
