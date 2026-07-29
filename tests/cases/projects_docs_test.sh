# The onboarding story has to match the code. A tool the app no longer uses
# must not still be listed as something to install, and a setting the app
# reads must be documented in the one file that documents settings -- those
# are both checkable, so they are checked rather than remembered.

setup() {
  SHIPPED="$REPO_ROOT/README.md
$REPO_ROOT/env.template
$REPO_ROOT/install.sh
$REPO_ROOT/bin/ayeaye
$REPO_ROOT/share/app.html
$REPO_ROOT/share/board.html"
}

test_no_shipped_file_still_points_at_the_old_directory_database() {
  local hits=""
  local path
  for path in $SHIPPED; do
    if grep -qi "zoxide" "$path"; then
      hits="$hits $path"
    fi
  done
  assert_eq "" "$hits" \
    "the picker no longer uses it, so nothing may still present it as needed"
}

test_every_project_search_setting_is_documented() {
  local name missing=""
  for name in $(grep -o '_env("PROJECT_[A-Z_]*"' "$REPO_ROOT/bin/ayeaye" \
                | sed 's/.*("//; s/"//'); do
    grep -q "AYEAYE_$name" "$REPO_ROOT/env.template" || missing="$missing $name"
  done
  assert_ne "" "$(grep -o '_env("PROJECT_[A-Z_]*"' "$REPO_ROOT/bin/ayeaye")" \
    "the extraction has to find the settings for this test to mean anything"
  assert_eq "" "$missing" \
    "env.template is the one place everything is configured from"
}

test_the_readme_says_the_picker_needs_nothing_installed() {
  assert_file_contains "$REPO_ROOT/README.md" "project picker"
}
