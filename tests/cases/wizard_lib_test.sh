# The parts of the wizard layer that are contracts rather than behaviour: how
# it asks, how it decides a run is a resume rather than a rerun, and the
# promise that none of it takes down a script running under `set -euo pipefail`.
#
# That last one is not a nicety. install.sh runs under errexit, and a library
# whose "always succeeds" functions can return non-zero kills the wizard
# somewhere in the middle with no message at all - which is the worst failure
# mode this project has.

setup() {
  WIZARD_STATE_DIR="$TEST_TMPDIR/state"
  WIZARD_STATE_FILE="$WIZARD_STATE_DIR/setup-state"
  WIZARD_CONSENT_LEDGER="$WIZARD_STATE_DIR/setup-consent.log"
  WIZARD_LOG_FILE="$WIZARD_STATE_DIR/setup.log"
  WIZARD_BACKUP_DIR="$WIZARD_STATE_DIR/backups"
  PLATFORM_OS_RELEASE_FILES=""
  export PLATFORM_OS_RELEASE_FILES
  . "$REPO_ROOT/lib/state.sh"
  . "$REPO_ROOT/lib/ui.sh"
  . "$REPO_ROOT/lib/stage.sh"
  wizard_state_load
  WIZARD_INTERACTIVE=1
  WIZARD_DETAILS=0
}

_answers() {
  local file="$TEST_TMPDIR/answers" line
  : > "$file"
  for line in "$@"; do
    printf '%s\n' "$line" >> "$file"
  done
  printf '%s' "$file"
}

# ------------------------------------------------------------------ asking

test_a_question_shows_its_default_in_brackets() {
  wizard_ask "which port" "8911" < "$(_answers "")" >/dev/null
  assert_eq "8911" "$REPLY"
}

test_an_empty_default_reads_as_the_word_empty() {
  # "[]" would look like a bug. The wizard has to be able to say "nothing" out
  # loud, because "nothing" is the right answer to several of its questions.
  wizard_ask "which hosts" "" < "$(_answers "")" > "$TEST_TMPDIR/out" 2>&1
  assert_status 0 "$?" "an empty default is a normal default, not an error"
  assert_eq "" "$REPLY"
  # The prompt text itself only ever reaches a terminal, which is what the pty
  # driver is for; wizard_flow_test.sh is where "[empty]" is read back.
  assert_eq "" "$(cat "$TEST_TMPDIR/out")"
}

test_a_typed_answer_wins_over_the_default() {
  wizard_ask "which port" "8911" < "$(_answers "9000")" >/dev/null
  assert_eq "9000" "$REPLY"
}

test_an_unattended_run_never_reads_standard_input() {
  WIZARD_INTERACTIVE=0
  wizard_ask "which port" "8911" < "$(_answers "9000")" >/dev/null
  assert_eq "8911" "$REPLY" \
    "whatever is on standard input must not be able to steer an unattended run"
}

test_only_a_yes_is_a_yes() {
  local answer
  for answer in y Y yes YES Yes; do
    wizard_confirm "shall I" "n" < "$(_answers "$answer")"
    assert_status 0 "$?" "\"$answer\" is a yes"
  done
  for answer in n N no maybe "" yeah "y."; do
    wizard_confirm "shall I" "n" < "$(_answers "$answer")"
    assert_status 1 "$?" "\"$answer\" is not a yes"
  done
}

# --------------------------------------------------------- technical detail

test_a_detail_is_logged_and_not_shown() {
  local out
  out="$(wizard_detail "sudo apt-get install -y tmux")"
  assert_eq "" "$out" "the raw command is not what the reader is here for"
  assert_file_contains "$WIZARD_LOG_FILE" "sudo apt-get install -y tmux"
}

test_asking_for_details_shows_them_too() {
  WIZARD_DETAILS=1
  local out
  out="$(wizard_detail "sudo apt-get install -y tmux")"
  assert_contains "$out" "sudo apt-get install -y tmux"
}

test_the_log_is_named_once_and_only_when_it_has_something_in_it() {
  # Run in this shell, not a subshell: "at most once" is remembered in a
  # variable, and a command substitution would forget it every time.
  wizard_detail_hint > "$TEST_TMPDIR/hint1"
  assert_eq "" "$(cat "$TEST_TMPDIR/hint1")" "an empty log is not worth mentioning"
  wizard_detail "something happened"
  wizard_detail_hint > "$TEST_TMPDIR/hint2"
  assert_contains "$(cat "$TEST_TMPDIR/hint2")" "$WIZARD_LOG_FILE"
  wizard_detail_hint > "$TEST_TMPDIR/hint3"
  assert_eq "" "$(cat "$TEST_TMPDIR/hint3")" "and it is not repeated"
}

test_a_checklist_line_lines_its_verdicts_up() {
  local out
  out="$(wizard_item ok tmux; wizard_item MISSING "python3 (required)")"
  assert_contains "$out" "  ok       tmux"
  assert_contains "$out" "  MISSING  python3 (required)"
}

# --------------------------------------------------------- run or resume

test_the_first_ever_run_is_neither_a_resume_nor_a_rerun() {
  wizard_begin_run
  assert_eq "0" "$WIZARD_RESUMING"
  assert_eq "1" "$(wizard_state_get run.count)"
  assert_eq "0" "$(wizard_state_get run.completed)"
}

test_a_run_that_did_not_finish_is_resumed() {
  wizard_begin_run
  wizard_state_set stage.detect done
  wizard_begin_run
  assert_eq "1" "$WIZARD_RESUMING"
  assert_eq "done" "$(wizard_state_get stage.detect)" \
    "the whole point: what was finished stays finished"
}

test_a_run_that_finished_starts_over_next_time() {
  wizard_begin_run
  wizard_state_set stage.detect done
  wizard_state_set answer.port 9000
  wizard_end_run 0
  wizard_begin_run
  assert_eq "0" "$WIZARD_RESUMING"
  assert_eq "" "$(wizard_state_get stage.detect)" \
    "a deliberate rerun walks every stage again, so settings can be changed"
  assert_eq "9000" "$(wizard_state_get answer.port)" \
    "and offers back what was chosen last time"
  assert_eq "2" "$(wizard_state_get run.count)"
}

test_a_walk_that_stopped_early_leaves_the_run_open() {
  wizard_begin_run
  wizard_state_set stage.detect done
  wizard_end_run 1
  wizard_begin_run
  assert_eq "1" "$WIZARD_RESUMING"
  assert_eq "done" "$(wizard_state_get stage.detect)"
}

test_a_count_somebody_edited_by_hand_does_not_kill_the_run() {
  wizard_begin_run
  wizard_state_set run.count "not a number"
  wizard_state_set run.completed 1
  wizard_begin_run
  assert_status 0 "$?"
  assert_eq "1" "$(wizard_state_get run.count)"
}

# ---------------------------------------------- surviving set -euo pipefail

# A script that does what install.sh does: source the layer under errexit and
# call the things documented as always succeeding. Any of them returning
# non-zero kills it, and the exit status says which line.
_errexit_script() {
  local script="$TEST_TMPDIR/errexit.sh"
  cat > "$script" <<SCRIPT
#!/usr/bin/env bash
set -euo pipefail
WIZARD_STATE_DIR="\$1"
WIZARD_STATE_FILE="\$WIZARD_STATE_DIR/setup-state"
WIZARD_CONSENT_LEDGER="\$WIZARD_STATE_DIR/setup-consent.log"
WIZARD_LOG_FILE="\$WIZARD_STATE_DIR/setup.log"
WIZARD_BACKUP_DIR="\$WIZARD_STATE_DIR/backups"
WIZARD_INTERACTIVE=0
PLATFORM_OS_RELEASE_FILES=""
. "$REPO_ROOT/lib/state.sh"
. "$REPO_ROOT/lib/ui.sh"
. "$REPO_ROOT/lib/platform.sh"
. "$REPO_ROOT/lib/consent.sh"
. "$REPO_ROOT/lib/stage.sh"

$2

echo "reached the end"
SCRIPT
  chmod +x "$script"
  printf '%s' "$script"
}

test_the_state_store_survives_errexit() {
  local script
  script="$(_errexit_script "$TEST_TMPDIR/state" '
wizard_state_load
wizard_state_set answer.port 8911
wizard_state_get answer.port >/dev/null
wizard_state_get answer.nothing >/dev/null
wizard_state_keys >/dev/null
wizard_state_unset_prefix "stage."
wizard_state_is_new || true
')"
  run_script "$script" "$TEST_TMPDIR/state"
  assert_status 0 "$RUN_STATUS"
  assert_contains "$RUN_STDOUT" "reached the end"
}

test_the_consent_layer_survives_errexit() {
  # Every one of these refuses, and a refusal is a normal outcome rather than
  # a reason for the wizard to disappear.
  local script
  script="$(_errexit_script "$TEST_TMPDIR/state" '
wizard_consent_reset
wizard_consent privileged "may I?" || true
wizard_privileged "may I?" "true" || true
wizard_download "may I?" "https://example.invalid/x" "$WIZARD_STATE_DIR/x" || true
wizard_firewall "may I?" "true" || true
wizard_trust "may I?" "true" || true
wizard_may_expose "may I?" || true
wizard_replace "$WIZARD_STATE_DIR/absent"
wizard_consent_ledger >/dev/null
wizard_consent_policy privileged >/dev/null
wizard_plan_reset
wizard_plan_add package "tmux" 100
wizard_plan_show >/dev/null
wizard_plan_is_consequential || true
')"
  run_script "$script" "$TEST_TMPDIR/state"
  assert_status 0 "$RUN_STATUS"
  assert_contains "$RUN_STDOUT" "reached the end"
}

test_a_command_that_fails_does_not_take_the_wizard_with_it() {
  # The documented status 1 - "it was allowed to run and it did not work" - is
  # unreachable if the wrapper itself dies under errexit before it can return.
  local script
  script="$(_errexit_script "$TEST_TMPDIR/state" '
WIZARD_AUTO_CONSENT="privileged"
wizard_privileged "may I?" "exit 7" && echo "unexpected success"
status=$?
echo "wrapper said $status"
')"
  run_script "$script" "$TEST_TMPDIR/state"
  assert_status 0 "$RUN_STATUS"
  assert_contains "$RUN_STDOUT" "wrapper said 1"
  assert_contains "$RUN_STDOUT" "reached the end"
}

test_the_lifecycle_survives_errexit() {
  local script
  script="$(_errexit_script "$TEST_TMPDIR/state" '
_a_step() { return "$WIZARD_STAGE_OK"; }
_a_pending_step() { return "$WIZARD_STAGE_PENDING"; }
wizard_begin_run
wizard_stage one "one"
wizard_step one a _a_step "a"
wizard_step one b _a_pending_step "b"
wizard_run_stages >/dev/null
wizard_unfinished >/dev/null
wizard_stage_status one >/dev/null
wizard_end_run 0
')"
  run_script "$script" "$TEST_TMPDIR/state"
  assert_status 0 "$RUN_STATUS"
  assert_contains "$RUN_STDOUT" "reached the end"
}

test_a_state_directory_it_cannot_write_costs_progress_and_nothing_else() {
  # A read-only state directory is a real thing on a locked-down machine. It
  # must cost resumability, not the install.
  [ "$(id -u)" = 0 ] && skip "root can write into a directory it has no write bit for"
  local locked="$TEST_TMPDIR/locked"
  mkdir -p "$locked"
  chmod 500 "$locked"
  local script
  script="$(_errexit_script "$locked/state" '
_a_step() { return "$WIZARD_STAGE_OK"; }
wizard_begin_run
wizard_stage one "one"
wizard_step one a _a_step "a"
wizard_run_stages >/dev/null
wizard_end_run 0
')"
  run_script "$script" "$locked/state"
  chmod 700 "$locked"
  assert_status 0 "$RUN_STATUS" "the install must still finish"
  assert_contains "$RUN_STDOUT" "reached the end"
}
