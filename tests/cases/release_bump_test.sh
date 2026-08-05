# The version bump: one command moves the version everywhere it is written
# down, clears the stamp, and refuses input that would leave the tree half-
# bumped. Run against a copied tree - the bump script takes a root argument
# for exactly this reason.

setup() {
  BUMP="$REPO_ROOT/scripts/release-bump.sh"
  FAKE_ROOT="$TEST_TMPDIR/tree"
  mkdir -p "$FAKE_ROOT"
  cat > "$FAKE_ROOT/install.sh" <<'SH'
#!/usr/bin/env bash
AYEAYE_VERSION="v0.1.0"
AYEAYE_SHA256="81e00b4b05ba7ff2e1a1903e5a408584bccf7625f4b4daf5b2e75dbe8384eadb"
echo "$AYEAYE_VERSION"
SH
  cat > "$FAKE_ROOT/Cookfile" <<'COOK'
recipe release-version
    test { bash scripts/check-release-version.sh v0.1.0 }
recipe dist
    cook "dist/ayeaye-v0.1.0.tar.gz" { bash scripts/release-archive.sh $<out> }
recipe release-installable: dist
    test { bash scripts/check-release-artifact.sh dist/ayeaye-v0.1.0.tar.gz }
chore publish version="v0.1.0": suite release-version
    bash scripts/release-publish.sh $<version>
COOK
}

test_a_bump_moves_the_version_in_both_files_and_clears_the_stamp() {
  run_script "$BUMP" v0.2.0 "$FAKE_ROOT"
  assert_status 0 "$RUN_STATUS" "$RUN_OUTPUT"
  assert_contains "$(cat "$FAKE_ROOT/install.sh")" 'AYEAYE_VERSION="v0.2.0"'
  assert_contains "$(cat "$FAKE_ROOT/install.sh")" 'AYEAYE_SHA256=""'
  assert_not_contains "$(cat "$FAKE_ROOT/Cookfile")" "v0.1.0" \
    "every copy of the old version must move, or the drift gate fires later"
  assert_contains "$(cat "$FAKE_ROOT/Cookfile")" "dist/ayeaye-v0.2.0.tar.gz"
  assert_contains "$(cat "$FAKE_ROOT/Cookfile")" 'publish version="v0.2.0"'
}

test_bumping_to_the_version_already_there_changes_nothing() {
  before_install="$(cat "$FAKE_ROOT/install.sh")"
  before_cook="$(cat "$FAKE_ROOT/Cookfile")"
  run_script "$BUMP" v0.1.0 "$FAKE_ROOT"
  assert_status 0 "$RUN_STATUS" "$RUN_OUTPUT"
  assert_eq "$before_install" "$(cat "$FAKE_ROOT/install.sh")" \
    "an idempotent bump must not clear a stamp it did not replace"
  assert_eq "$before_cook" "$(cat "$FAKE_ROOT/Cookfile")"
}

test_a_version_that_does_not_look_like_one_is_refused() {
  run_script "$BUMP" 0.2.0 "$FAKE_ROOT"
  assert_status 1 "$RUN_STATUS"
  assert_contains "$RUN_STDERR" "look like"
  assert_contains "$(cat "$FAKE_ROOT/install.sh")" 'AYEAYE_VERSION="v0.1.0"' \
    "a refused bump must leave the tree exactly as it was"
}

test_the_real_repository_is_in_sync_right_now() {
  # The drift gate, run as a test: whatever version install.sh names, the
  # real Cookfile builds and publishes that artifact and no other.
  version="$(bash "$REPO_ROOT/scripts/release-version.sh")"
  run_script "$REPO_ROOT/scripts/check-release-version.sh" "$version"
  assert_status 0 "$RUN_STATUS" "$RUN_OUTPUT"
}
