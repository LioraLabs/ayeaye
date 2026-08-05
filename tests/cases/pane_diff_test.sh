# The /api/pane diff protocol: patches reconstruct the server's capture
# exactly, and an idle pane costs a token match rather than a resend.

setup() {
  require_host_command python3
  stub_real python3
  PANE_TESTS="$REPO_ROOT/tests/python/pane_diff.py"
}

test_pane_diff_protocol_shapes_and_reconstruction() {
  run_script "$PANE_TESTS" ProtocolShapes
  assert_status 0 "$RUN_STATUS" "$RUN_OUTPUT"
}
