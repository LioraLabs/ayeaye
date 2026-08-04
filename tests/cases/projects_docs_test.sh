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

# ------------------------------------------------- the onboarding the milestone built

test_the_readme_quotes_the_one_liner_without_hedging() {
  # The release exists now, so the caveat that used to sit beside the
  # one-liner would itself be the false claim. The command is quoted, and
  # nothing beside it says it does not work.
  local readme
  readme="$(cat "$REPO_ROOT/README.md")"
  assert_contains "$readme" "curl -fsSL https://raw.githubusercontent.com/LioraLabs/ayeaye/main/install.sh"
  assert_not_contains "$readme" "does not work yet"
}

test_the_readme_describes_the_four_ways_in_and_the_rule_under_them() {
  local readme
  readme="$(cat "$REPO_ROOT/README.md")"
  assert_contains "$readme" "Tailscale"
  assert_contains "$readme" "this computer only"
  assert_contains "$readme" "your home network"
  assert_contains "$readme" "an HTTPS address you already have"
  assert_contains "$readme" "ayeaye itself never leaves this computer" \
    "the one rule all four keep is the thing worth documenting about them"
}

test_the_readme_lists_the_flags_the_installer_really_takes() {
  # Derived from the argument parser rather than from a memory of it: the flag
  # set changed during this milestone, and a documented flag that no longer
  # exists is worse than an undocumented one that does.
  local flag readme missing=""
  readme="$(cat "$REPO_ROOT/README.md")"
  for flag in $(grep -o -- '--[a-z-]*)' "$REPO_ROOT/install.sh" \
                | sed 's/)$//' | sort -u); do
    case "$flag" in
      --*) ;;
      *) continue ;;
    esac
    case "$readme" in
      *"$flag"*) ;;
      *) missing="$missing $flag" ;;
    esac
  done
  assert_eq "" "$missing" "install.sh accepts these and the README does not name them"
}

test_the_readme_says_how_to_remove_it_on_both_platforms() {
  local readme
  readme="$(cat "$REPO_ROOT/README.md")"
  assert_contains "$readme" "systemctl --user disable --now ayeaye.service"
  assert_contains "$readme" "launchctl bootout gui/"
  assert_contains "$readme" "remove the certificate from every phone" \
    "the one part of removal no computer can do for you"
}

test_nothing_shipped_still_points_at_a_directory_of_unit_templates() {
  # The milestone deleted systemd/user/: a unit and a property list are two
  # spellings of one description now, generated from lib/steps/70-service.sh.
  # Anything still telling somebody to copy a template is pointing at a
  # directory that is not there.
  local hits
  # shellcheck disable=SC2086
  hits="$(grep -rl "systemd/user/@" "$REPO_ROOT" $SWEEP_EXCLUDES 2>/dev/null)"
  assert_eq "" "$hits"
  assert_file_missing "$REPO_ROOT/systemd/user" \
    "the directory is gone, and the renderers replaced it"
}

test_the_readme_says_what_the_closing_check_does_not_prove() {
  local readme
  readme="$(cat "$REPO_ROOT/README.md")"
  assert_contains "$readme" "What it cannot tell you"
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
