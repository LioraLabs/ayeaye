# Tracked-file reference resolution and the authenticated HTTP boundary.

setup() {
  require_host_command python3
  require_host_command git
  stub_real python3
  stub_real git
  FILE_TESTS="$REPO_ROOT/tests/python/file_preview.py"
}

run_file_tests() {
  run_script "$FILE_TESTS" "$@"
}

test_tracked_file_matching_and_ranking() {
  run_file_tests ResolverMatches
  assert_status 0 "$RUN_STATUS" "$RUN_OUTPUT"
}

test_tracked_file_metadata_and_fail_closed_boundaries() {
  run_file_tests ResolverMetadata
  assert_status 0 "$RUN_STATUS" "$RUN_OUTPUT"
}

test_tracked_file_resolver_http_boundary() {
  run_file_tests ResolverEndpoint
  assert_status 0 "$RUN_STATUS" "$RUN_OUTPUT"
}

test_safe_bounded_tracked_file_previews() {
  run_file_tests PreviewEndpoint
  assert_status 0 "$RUN_STATUS" "$RUN_OUTPUT"
}
