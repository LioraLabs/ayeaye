# The release pipeline: the pieces of "pushing a tag produces every artifact
# and its checksums" that can be held to account without pushing a tag. The
# checksum script is exercised for real; the workflow file is pinned by shape,
# which is weak but is what keeps a row from being deleted without this suite
# noticing.

setup() {
  require_host_command sha256sum
  stub_real sha256sum
  CHECKSUMS="$REPO_ROOT/scripts/release-checksums.sh"
  WORKFLOW="$REPO_ROOT/.github/workflows/release.yml"
  DIST="$TEST_TMPDIR/dist"
  mkdir -p "$DIST"
}

# AYEAYE-59
test_checksums_cover_every_published_name() {
  printf 'one' > "$DIST/ayeaye-v9.9.9-x86_64-unknown-linux-musl"
  printf 'two' > "$DIST/ayeaye-x86_64-unknown-linux-musl"
  printf 'three' > "$DIST/ayeaye-v9.9.9.tar.gz"
  run_script "$CHECKSUMS" \
    "$DIST/ayeaye-v9.9.9-x86_64-unknown-linux-musl" \
    "$DIST/ayeaye-x86_64-unknown-linux-musl" \
    "$DIST/ayeaye-v9.9.9.tar.gz" \
    "$DIST/SHA256SUMS"
  assert_status 0 "$RUN_STATUS" "$RUN_OUTPUT"
  sums="$(cat "$DIST/SHA256SUMS")"
  assert_contains "$sums" "ayeaye-v9.9.9-x86_64-unknown-linux-musl" \
    "the versioned name must be verifiable"
  assert_contains "$sums" "ayeaye-x86_64-unknown-linux-musl" \
    "the versionless alias is a published name too, not a footnote"
  assert_contains "$sums" "ayeaye-v9.9.9.tar.gz"
  assert_not_contains "$sums" "$TEST_TMPDIR" \
    "bare basenames, no paths - the bootstrap looks names up as the release page shows them"
  # sha256 of the literal bytes 'one', computed independently of the script.
  assert_contains "$sums" "7692c3ad3540bb803c020b3aee66cd8887123234ea0c6e7143c0add73ff431ed" \
    "the sums must be of the artifacts, not of anything else"
  assert_eq 3 "$(printf '%s\n' "$sums" | wc -l | tr -d ' ')" \
    "one line per published name, nothing extra"
}

# AYEAYE-59
test_checksums_still_take_a_single_artifact() {
  # The Cookfile's dist recipe calls this with one artifact and one output;
  # going variadic must not break that call.
  printf 'one' > "$DIST/ayeaye-v9.9.9.tar.gz"
  run_script "$CHECKSUMS" "$DIST/ayeaye-v9.9.9.tar.gz" "$DIST/SHA256SUMS"
  assert_status 0 "$RUN_STATUS" "$RUN_OUTPUT"
  assert_eq "7692c3ad3540bb803c020b3aee66cd8887123234ea0c6e7143c0add73ff431ed  ayeaye-v9.9.9.tar.gz" \
    "$(cat "$DIST/SHA256SUMS")"
}
