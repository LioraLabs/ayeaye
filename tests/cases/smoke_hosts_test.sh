# The six real machines one-command onboarding is supposed to work on, and the
# fact that not one of them has ever been tried.
#
# This file does two jobs and they pull in opposite directions on purpose.
#
# The first is ordinary coverage: tests/smoke.sh is real code that would really
# run, and the things that can be checked without a spare Ubuntu box - that it
# refuses to change a machine without being told to, that it names all six
# profiles, that it and the manifest cannot drift apart - are checked here like
# anything else.
#
# The second is to make the gap **loud**. Six of the tests below do nothing but
# skip, by name, one per machine. A skip is not a pass: tests/run.sh counts them
# separately and prints the reason under each one, so a run of the suite says
# out loud, six times, that this coverage does not exist. Deleting those six
# would make the suite greener and the project less honest.

_smoke()  { printf '%s' "$REPO_ROOT/tests/smoke.sh"; }
_manifest() { printf '%s' "$REPO_ROOT/tests/smoke-hosts"; }

_profiles() { printf 'ubuntu fedora arch opensuse macos-intel macos-arm'; }

setup() {
  require_host_command python3
  stub_real python3
}

# ============================================================== the two files

test_the_smoke_runner_and_its_record_both_exist() {
  assert_file_exists "$(_smoke)"
  assert_file_exists "$(_manifest)"
}

test_every_machine_the_milestone_names_is_in_the_record() {
  local id manifest
  manifest="$(cat "$(_manifest)")"
  for id in $(_profiles); do
    assert_matches "$manifest" "^$id\|" "$id must have a line of its own"
  done
}

test_the_record_says_none_of_them_has_been_done() {
  # The day one of these is verified, this test is the one that has to be
  # changed - deliberately, by whoever verified it.
  local id verified manifest
  manifest="$(cat "$(_manifest)")"
  for id in $(_profiles); do
    verified="$(printf '%s\n' "$manifest" | sed -n "s/^$id|.*|//p" | head -1)"
    assert_eq "no" "$verified" \
      "$id has never been run on a real machine; if that has changed, change it here"
  done
}

test_the_runner_names_every_profile_the_record_holds() {
  # Two files, one list. The profile matching in tests/smoke.sh has a case
  # branch per id, and an id in the record with no branch would silently never
  # match anything.
  local id runner
  runner="$(cat "$(_smoke)")"
  for id in $(_profiles); do
    assert_contains "$runner" "$id)" "tests/smoke.sh must know how to recognise $id"
  done
}

test_the_record_holds_nothing_the_runner_does_not_know() {
  local line id runner
  runner="$(cat "$(_smoke)")"
  while IFS= read -r line; do
    case "$line" in
      ""|"#"*) continue ;;
    esac
    id="${line%%|*}"
    assert_contains "$(_profiles)" "$id" \
      "the record names $id and this test does not know about it"
  done < "$(_manifest)"
}

# ============================================================== the runner runs

test_listing_the_machines_needs_no_permission_and_changes_nothing() {
  run_script "$(_smoke)" --list
  assert_status 0 "$RUN_STATUS"
  local id
  for id in $(_profiles); do
    assert_contains "$RUN_STDOUT" "$id"
  done
  assert_contains "$RUN_STDOUT" "has never been run"
}

test_the_list_says_plainly_that_nothing_is_covered_by_the_other_suites() {
  run_script "$(_smoke)" --list
  assert_contains "$RUN_STDOUT" "Nothing in tests/run.sh or tests/containers.sh covers it"
}

test_it_refuses_to_change_a_machine_without_being_told_to() {
  # The gate, and the reason for it: this is the one thing in the repository
  # that installs software on the machine it is run from.
  run_script "$(_smoke)" --run --profile ubuntu
  assert_status 2 "$RUN_STATUS"
  assert_contains "$RUN_OUTPUT" "this changes the computer it runs on"
  assert_contains "$RUN_OUTPUT" "Nothing has been done."
  assert_file_missing "$XDG_CONFIG_HOME/ayeaye/env" \
    "the gate must hold before anything is written"
  assert_file_missing "$XDG_STATE_HOME/ayeaye/token"
}

test_the_confirmation_is_not_something_anybody_types_by_accident() {
  local runner
  runner="$(cat "$(_smoke)")"
  assert_contains "$runner" "AYEAYE_SMOKE_CONFIRM"
  assert_contains "$runner" "yes-change-this-computer"
}

test_a_machine_that_is_none_of_the_six_is_told_so_rather_than_guessed_at() {
  # The sandbox has no uname, so the platform layer cannot place this machine
  # at all - which is exactly the case a smoke runner must refuse rather than
  # assume its way through.
  run_script "$(_smoke)" --plan
  assert_status 2 "$RUN_STATUS"
  assert_contains "$RUN_OUTPUT" "not one of the six"
}

test_planning_says_what_it_would_do_and_does_none_of_it() {
  stub_command uname --exit 1
  stub_when uname '-s' --stdout "Linux"
  stub_when uname '-m' --stdout "x86_64"
  run_script "$(_smoke)" --plan --profile ubuntu
  assert_status 0 "$RUN_STATUS"
  assert_contains "$RUN_STDOUT" "Nothing above has happened."
  assert_contains "$RUN_STDOUT" "$XDG_CONFIG_HOME/ayeaye/env"
  assert_file_missing "$XDG_CONFIG_HOME/ayeaye/env"
}

test_a_verification_is_never_written_by_the_thing_being_verified() {
  # The record is edited by a person. A runner that marked its own homework
  # would turn "verified" into "ran once, somewhere, somehow".
  local runner
  runner="$(cat "$(_smoke)")"
  assert_contains "$runner" "Nothing here writes that line for you."
  assert_not_contains "$runner" '> "$HOSTS_FILE"'
  assert_not_contains "$runner" '>> "$HOSTS_FILE"'
}

# ========================================== the coverage that does not exist yet
#
# One per machine, and each one says which. They cost a line each in the suite's
# output and they are the only place a reader is told, by the tests themselves,
# that six of the platforms this milestone promises have never executed a line
# of this code.

test_ubuntu_on_a_real_host_has_never_been_run() {
  skip "Ubuntu on a real host: UNVERIFIED. tests/smoke.sh --run does it; no container and no fixture covers a real login session, a real systemd user bus or a real apt."
}

test_fedora_on_a_real_host_has_never_been_run() {
  skip "Fedora on a real host: UNVERIFIED. The container proves dnf and the name table; it proves nothing about a user service that has to start at login."
}

test_arch_on_a_real_host_has_never_been_run() {
  skip "Arch Linux on a real host: UNVERIFIED. The container proves pacman; the whisper.cpp package names this project chooses for cuda and rocm have never been installed anywhere."
}

test_opensuse_on_a_real_host_has_never_been_run() {
  skip "openSUSE Tumbleweed on a real host: UNVERIFIED. The container has neither find nor python3 to begin with, so even the unit suite has never run there."
}

test_intel_macos_has_never_been_run() {
  skip "Intel macOS: UNVERIFIED, and uncoverable here. There is no Mac: launchd, Homebrew under /usr/local, and the ps/lsof session matching have never executed."
}

test_apple_silicon_macos_has_never_been_run() {
  skip "Apple Silicon macOS: UNVERIFIED, and uncoverable here. There is no Mac: launchd, Homebrew under /opt/homebrew, and the Metal acceleration path have never executed."
}
