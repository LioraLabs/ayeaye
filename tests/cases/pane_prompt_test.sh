# Seeing that a pane has stopped to ask you something.
#
# A pending question is invisible in the transcript, so the screen is the only
# place it can be read, and getting it wrong costs the app its whole point: a
# session stopped dead reads as `working` and drops down the board. That is
# what an AskUserQuestion with per-option previews did -- drawn in two columns,
# with the preview box running on below the last option.
#
# The assertions live in tests/python/pane_prompt.py, against real screens
# captured under tests/fixtures/pane-prompt; this file is the bridge that puts
# them under `tests/run.sh`. Nothing here runs tmux or starts an agent.

setup() {
  require_host_command python3
  stub_real python3
  PROMPT_TESTS="$REPO_ROOT/tests/python/pane_prompt.py"
}

run_prompt_tests() {
  run_script "$PROMPT_TESTS" "$@"
}

test_a_prompt_drawn_in_two_columns_is_read_past_its_preview_box() {
  run_prompt_tests PreviewColumns
  assert_status 0 "$RUN_STATUS" "$RUN_OUTPUT"
}

test_a_single_column_prompt_reads_the_same_as_it_always_did() {
  run_prompt_tests PlainPrompts
  assert_status 0 "$RUN_STATUS" "$RUN_OUTPUT"
}

test_a_prompt_counts_only_while_it_is_still_the_bottom_of_the_screen() {
  run_prompt_tests Staleness
  assert_status 0 "$RUN_STATUS" "$RUN_OUTPUT"
}

test_the_screen_overrides_what_the_transcript_concluded() {
  run_prompt_tests TheBlockedOverride
  assert_status 0 "$RUN_STATUS" "$RUN_OUTPUT"
}
