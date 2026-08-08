# Long transcript cards link to a complete, rendered message page.

setup() {
  require_host_command python3
  stub_real python3
}

test_clipped_transcript_rows_can_resolve_the_complete_original_message() {
  run_script "$REPO_ROOT/tests/python/transcript_message.py"
  assert_status 0 "$RUN_STATUS" "$RUN_OUTPUT"
}
