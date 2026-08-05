# The version bump: one command moves the version everywhere it is written
# down, clears the stamp, and refuses input that would leave the tree half-
# bumped. Run against a copied tree - the bump script takes a root argument
# for exactly this reason.

setup() {
  require_host_command git
  require_host_command gzip
  stub_real git gzip
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

# _release_repo - a git repository holding the release scripts and a committed
# install.sh, for the checks that compare the working tree against HEAD. The
# scripts resolve their root from their own location, which is why they are
# copied in rather than run from the repository they normally live in.
_release_repo() {
  RELEASE_REPO="$TEST_TMPDIR/repo"
  mkdir -p "$RELEASE_REPO/scripts"
  cp "$REPO_ROOT"/scripts/release-*.sh "$RELEASE_REPO/scripts/"
  chmod +x "$RELEASE_REPO"/scripts/*.sh
  cp "$FAKE_ROOT/install.sh" "$RELEASE_REPO/install.sh"
  git -C "$RELEASE_REPO" init -q
  git -C "$RELEASE_REPO" -c user.email=t@test -c user.name=t add -A
  git -C "$RELEASE_REPO" -c user.email=t@test -c user.name=t commit -qm base
}

test_the_archive_refuses_a_bump_that_is_not_committed() {
  _release_repo
  sed -i 's/^AYEAYE_VERSION="v0.1.0"/AYEAYE_VERSION="v0.2.0"/' "$RELEASE_REPO/install.sh"
  run_script "$RELEASE_REPO/scripts/release-archive.sh" "$RELEASE_REPO/dist/out.tar.gz"
  assert_status 1 "$RUN_STATUS" "$RUN_OUTPUT"
  assert_contains "$RUN_STDERR" "not committed" \
    "an uncommitted bump must be named, not archived around"
  assert_contains "$RUN_STDERR" "Commit the bump first"
  [ ! -e "$RELEASE_REPO/dist/out.tar.gz" ]
  assert_status 0 $? "a refused archive must not leave an artifact behind"
}

test_the_archive_builds_when_the_tree_agrees_with_head() {
  _release_repo
  run_script "$RELEASE_REPO/scripts/release-archive.sh" "$RELEASE_REPO/dist/out.tar.gz"
  assert_status 0 "$RUN_STATUS" "$RUN_OUTPUT"
  [ -s "$RELEASE_REPO/dist/out.tar.gz" ]
  assert_status 0 $? "the artifact must exist"
}

test_publish_names_an_uncommitted_stamp_as_the_next_step() {
  _release_repo
  sed -i 's/^AYEAYE_SHA256=".*"/AYEAYE_SHA256="1111111111111111111111111111111111111111111111111111111111111111"/' \
    "$RELEASE_REPO/install.sh"
  run_script "$RELEASE_REPO/scripts/release-publish.sh" v0.1.0
  assert_status 1 "$RUN_STATUS" "$RUN_OUTPUT"
  assert_contains "$RUN_STDERR" "stamp" \
    "the flow's own dirt deserves better words than 'dirty'"
  assert_contains "$RUN_STDERR" "Commit it"
}

test_publish_still_refuses_ordinary_dirt_plainly() {
  _release_repo
  echo "work in progress" > "$RELEASE_REPO/notes.txt"
  git -C "$RELEASE_REPO" add notes.txt
  run_script "$RELEASE_REPO/scripts/release-publish.sh" v0.1.0
  assert_status 1 "$RUN_STATUS" "$RUN_OUTPUT"
  assert_contains "$RUN_STDERR" "dirty - commit before publishing"
}

test_the_real_repository_is_in_sync_right_now() {
  # The drift gate, run as a test: whatever version install.sh names, the
  # real Cookfile builds and publishes that artifact and no other.
  version="$(bash "$REPO_ROOT/scripts/release-version.sh")"
  run_script "$REPO_ROOT/scripts/check-release-version.sh" "$version"
  assert_status 0 "$RUN_STATUS" "$RUN_OUTPUT"
}
