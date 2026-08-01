# Markdown in conversational transcript cards is readable and remains safe.

setup() {
  require_host_command node
  stub_real node
}

test_conversation_cards_render_the_markdown_people_actually_read() {
  run_script node "$TESTS_DIR/lib/markdown_probe.js" "$REPO_ROOT/share/app.html"
  assert_status 0 "$RUN_STATUS" "$RUN_OUTPUT"
}

test_conversation_cards_link_safe_file_references_and_define_preview_ui() {
  run_script node "$TESTS_DIR/lib/markdown_probe.js" "$REPO_ROOT/share/app.html"
  assert_status 0 "$RUN_STATUS" "$RUN_OUTPUT"
  local html
  html="$(<"$REPO_ROOT/share/app.html")"
  assert_contains "$html" 'filePreviewState'
  assert_contains "$html" '/api/files/resolve'
  assert_contains "$html" '/api/files/preview?'
}
