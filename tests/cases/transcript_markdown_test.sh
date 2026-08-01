# Markdown in conversational transcript cards is readable and remains safe.

setup() {
  require_host_command node
  stub_real node
}

test_conversation_cards_render_the_markdown_people_actually_read() {
  run_script node "$TESTS_DIR/lib/markdown_probe.js" "$REPO_ROOT/share/app.html"
  assert_status 0 "$RUN_STATUS" "$RUN_OUTPUT"
}
