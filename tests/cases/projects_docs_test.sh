# The onboarding story has to match the code. A tool the app no longer uses
# must not still be listed as something to install, and a setting the app
# reads must be documented in the one file that documents settings -- those
# are both checkable, so they are checked rather than remembered.

setup() {
  # tests/ names the retired tool on purpose, here and in the pick-store
  # tests, so it is the one place excluded from the sweep below. The rest is
  # everything that is not source: git's own storage (a plain file inside a
  # worktree, so --exclude-dir cannot reach it), build artefacts, and other
  # checkouts hanging below this one.
  SWEEP_EXCLUDES="--exclude-dir=tests --exclude-dir=.git --exclude=.git
--exclude-dir=__pycache__ --exclude=*.pyc --exclude-dir=.worktrees"
}

test_nothing_shipped_still_points_at_the_old_directory_database() {
  local hits
  # Derived, not listed: a hand-maintained file list stops covering the
  # repository the moment someone adds a file to it.
  # shellcheck disable=SC2086
  hits="$(grep -ril zoxide "$REPO_ROOT" $SWEEP_EXCLUDES 2>/dev/null)"
  assert_eq "" "$hits" \
    "the picker no longer uses it, so nothing may still present it as needed"
}

test_the_readme_no_longer_lists_the_picker_as_needing_anything() {
  # The paragraph this pins used to read "Optional: zoxide, for the project
  # picker" -- which is why asserting merely that the README says "project
  # picker" would have passed on exactly the text this forbids.
  assert_file_contains "$REPO_ROOT/README.md" "project picker needs nothing installed"
  assert_file_contains "$REPO_ROOT/README.md" "AYEAYE_PROJECT_" \
    "and it points at where the picker is tuned"
}

test_every_project_search_setting_is_documented() {
  local name found="" missing=""
  for name in $(grep -o '_env("PROJECT_[A-Z_]*"' "$REPO_ROOT/bin/ayeaye" \
                | sed 's/.*("//; s/"//'); do
    found="$found $name"
    grep -q "AYEAYE_$name" "$REPO_ROOT/env.template" || missing="$missing $name"
  done
  assert_ne "" "$found" \
    "the extraction has to find the settings for this test to mean anything"
  assert_eq "" "$missing" \
    "env.template is the one place everything is configured from"
}

test_the_picker_settings_are_documented_together_and_not_among_someone_elses() {
  local section
  # From the picker's own banner to the next one: every AYEAYE_PROJECT_ entry
  # must be inside it, and nothing else may have been swallowed by it.
  section="$(sed -n '/^# -.* the project picker/,/^# -.*optional)/p' \
             "$REPO_ROOT/env.template")"
  assert_contains "$section" "AYEAYE_PROJECT_ROOTS"
  assert_contains "$section" "AYEAYE_PROJECT_SKIP"
  assert_not_contains "$section" "VOICE_TX_ROWS" \
    "a new section must not adopt the settings that used to follow it"
  assert_not_contains "$section" "VOICE_CLIBAN"
}
