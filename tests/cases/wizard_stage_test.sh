# The eight-stage lifecycle, in the abstract: registration, ordering, resume,
# and what happens when a step does not work.
#
# The wizard's own eight stages are covered by wizard_flow_test.sh. This file
# pins the machinery underneath them, because that machinery is the interface
# every other ticket in this milestone registers into: a step is how hardware
# detection, package installation, service setup, voice model selection, secure
# access and health checking each attach themselves to a stage without editing
# the stage.

setup() {
  WIZARD_STATE_DIR="$TEST_TMPDIR/state"
  WIZARD_STATE_FILE="$WIZARD_STATE_DIR/setup-state"
  WIZARD_CONSENT_LEDGER="$WIZARD_STATE_DIR/setup-consent.log"
  WIZARD_LOG_FILE="$WIZARD_STATE_DIR/setup.log"
  . "$REPO_ROOT/lib/state.sh"
  . "$REPO_ROOT/lib/ui.sh"
  . "$REPO_ROOT/lib/stage.sh"
  wizard_state_load
  WIZARD_INTERACTIVE=0
  TRACE="$TEST_TMPDIR/trace"
  : > "$TRACE"
}

_trace() { printf '%s\n' "$1" >> "$TRACE"; }
_trace_lines() { cat "$TRACE"; }

# Answers for a run driven in this same shell, so `read` has somewhere to read
# from. stdin_lines is for run_script, which starts a process of its own.
_answers() {
  local file="$TEST_TMPDIR/answers" line
  : > "$file"
  for line in "$@"; do
    printf '%s\n' "$line" >> "$file"
  done
  printf '%s' "$file"
}

# Run the stages, keeping the output where a later assertion can read it, and
# without a subshell - a command substitution would take every state change
# with it when it ended.
_run() {
  wizard_run_stages > "$TEST_TMPDIR/out" 2>"$TEST_TMPDIR/err" < "${1:-/dev/null}"
  RUN_STAGES_STATUS=$?
  STAGES_OUT="$(cat "$TEST_TMPDIR/out")"
  return 0
}

_ok_step()      { _trace "$WIZARD_STEP_ID"; return "$WIZARD_STAGE_OK"; }
_pending_step()  { _trace "$WIZARD_STEP_ID"; return "$WIZARD_STAGE_PENDING"; }
_skip_step()     { _trace "$WIZARD_STEP_ID"; return "$WIZARD_STAGE_SKIP"; }
_fail_step()     { _trace "$WIZARD_STEP_ID"; return "$WIZARD_STAGE_FAIL"; }

# Fails the first time it is asked and works the second, so retry can be told
# apart from "it never really failed".
_flaky_step() {
  _trace "$WIZARD_STEP_ID"
  if [ -f "$TEST_TMPDIR/flaked" ]; then
    return "$WIZARD_STAGE_OK"
  fi
  : > "$TEST_TMPDIR/flaked"
  return "$WIZARD_STAGE_FAIL"
}

# ------------------------------------------------------------ registration

test_stages_run_in_the_order_they_were_declared() {
  wizard_stage first "the first thing"
  wizard_stage second "the second thing"
  wizard_step first a _ok_step "a"
  wizard_step second b _ok_step "b"
  _run
  assert_eq "first.a
second.b" "$(_trace_lines)"
}

test_steps_run_in_the_order_they_were_added() {
  wizard_stage only "the only thing"
  wizard_step only one _ok_step "one"
  wizard_step only two _ok_step "two"
  wizard_step only three _ok_step "three"
  _run
  assert_eq "only.one
only.two
only.three" "$(_trace_lines)"
}

test_a_stage_title_is_printed_as_a_heading() {
  wizard_stage welcome "ayeaye setup"
  wizard_step welcome hello _ok_step "hello"
  _run
  assert_contains "$STAGES_OUT" "ayeaye setup"
}

test_a_step_on_a_stage_that_was_never_declared_is_a_caller_bug() {
  wizard_stage only "the only thing"
  wizard_step nowhere a _ok_step "a"
  assert_status 2 "$?"
}

test_a_stage_declared_twice_is_a_caller_bug() {
  wizard_stage only "the only thing"
  wizard_stage only "again"
  assert_status 2 "$?"
}

test_a_step_declared_twice_in_one_stage_is_a_caller_bug() {
  wizard_stage only "the only thing"
  wizard_step only a _ok_step "a"
  wizard_step only a _ok_step "a again"
  assert_status 2 "$?"
}

test_a_step_naming_a_function_that_does_not_exist_is_a_caller_bug() {
  wizard_stage only "the only thing"
  wizard_step only a _no_such_function "a"
  assert_status 2 "$?"
}

test_a_step_knows_which_stage_and_step_it_is() {
  # The identity is what a step records its own sub-state under, so a sibling
  # ticket's step can remember more than done-or-not.
  wizard_stage detect "detect"
  wizard_step detect hardware _ok_step "hardware"
  _run
  assert_eq "detect.hardware" "$(_trace_lines)"
}

# ------------------------------------------------------------- the statuses

test_a_finished_step_is_recorded_as_done() {
  wizard_stage only "only"
  wizard_step only a _ok_step "a"
  _run
  assert_eq "done" "$(wizard_step_status only a)"
  assert_eq "done" "$(wizard_stage_status only)"
}

test_a_pending_step_keeps_its_stage_out_of_done() {
  # This is the property that stops a not-yet-implemented seam from claiming
  # success: a stage holding unfinished work is never "done".
  wizard_stage only "only"
  wizard_step only a _ok_step "a"
  wizard_step only b _pending_step "b"
  _run
  assert_eq "done" "$(wizard_step_status only a)"
  assert_eq "pending" "$(wizard_step_status only b)"
  assert_eq "pending" "$(wizard_stage_status only)" \
    "one unfinished step makes the whole stage unfinished"
}

test_a_skipped_step_does_not_hold_its_stage_back() {
  wizard_stage only "only"
  wizard_step only a _ok_step "a"
  wizard_step only b _skip_step "b"
  _run
  assert_eq "skipped" "$(wizard_step_status only b)"
  assert_eq "done" "$(wizard_stage_status only)" \
    "a step that does not apply here is finished business"
}

test_unfinished_work_can_be_listed_at_the_end() {
  wizard_stage only "only"
  wizard_step only a _ok_step "the finished one"
  wizard_step only b _pending_step "the unfinished one"
  _run
  local left
  left="$(wizard_unfinished)"
  assert_contains "$left" "the unfinished one"
  assert_not_contains "$left" "the finished one"
}

test_a_run_with_nothing_outstanding_lists_nothing() {
  wizard_stage only "only"
  wizard_step only a _ok_step "a"
  _run
  assert_eq "" "$(wizard_unfinished)"
}

# ------------------------------------------------------------------ resume

test_a_completed_step_is_not_run_again_by_a_resumed_run() {
  wizard_stage only "only"
  wizard_step only a _ok_step "a"
  wizard_step only b _fail_step "b"
  _run

  : > "$TRACE"
  _run
  assert_not_contains "$(_trace_lines)" "only.a" \
    "work that is already done must not be redone"
  assert_contains "$(_trace_lines)" "only.b" "and the work that is not, must be"
}

test_a_resumed_run_says_what_it_is_skipping() {
  wizard_stage only "only"
  wizard_step only a _ok_step "already finished"
  wizard_step only b _fail_step "b"
  _run
  _run
  assert_contains "$STAGES_OUT" "already finished" \
    "a person watching must see that the earlier work was kept, not silently dropped"
}

test_a_pending_step_is_run_again() {
  wizard_stage only "only"
  wizard_step only a _pending_step "a"
  _run
  : > "$TRACE"
  _run
  assert_contains "$(_trace_lines)" "only.a" "unfinished work is picked up again"
}

test_a_step_marked_always_runs_on_every_pass() {
  # Printing a welcome or a closing summary is not work that can be "already
  # done"; it is what the run looks like.
  wizard_stage only "only"
  wizard_step only greet _ok_step "greet" required always
  _run
  : > "$TRACE"
  _run
  assert_contains "$(_trace_lines)" "only.greet"
  assert_eq "" "$(wizard_step_status only greet)" \
    "a step that always runs has no completion worth recording"
}

test_forgetting_the_run_makes_everything_run_again() {
  wizard_stage only "only"
  wizard_step only a _ok_step "a"
  _run
  wizard_reset_progress
  : > "$TRACE"
  _run
  assert_contains "$(_trace_lines)" "only.a"
}

test_forgetting_the_run_keeps_the_answers() {
  wizard_state_set answer.port 9000
  wizard_reset_progress
  assert_eq "9000" "$(wizard_state_get answer.port)" \
    "a reconfiguration run offers back what was chosen last time"
}

# ------------------------------------------------------------- failing

test_a_failed_required_step_stops_the_run() {
  wizard_stage first "first"
  wizard_stage second "second"
  wizard_step first a _fail_step "a"
  wizard_step second b _ok_step "b"
  _run
  assert_ne 0 "$RUN_STAGES_STATUS" "a run that could not do what it was asked is not a success"
  assert_not_contains "$(_trace_lines)" "second.b" \
    "it must not carry on past work it could not finish"
  assert_eq "failed" "$(wizard_step_status first a)"
}

test_a_failed_optional_step_does_not_stop_the_run() {
  wizard_stage first "first"
  wizard_stage second "second"
  wizard_step first a _fail_step "a" optional
  wizard_step second b _ok_step "b"
  _run
  assert_status 0 "$RUN_STAGES_STATUS"
  assert_contains "$(_trace_lines)" "second.b"
  assert_eq "failed" "$(wizard_step_status first a)"
}

test_a_failure_is_explained_before_the_raw_output() {
  wizard_stage only "only"
  wizard_step only a _fail_step "fetching the thing"
  _run
  assert_contains "$STAGES_OUT" "fetching the thing" "it names what did not work, in the words used to announce it"
  assert_contains "$STAGES_OUT" "install.sh" "and how to pick up where it stopped"
}

test_completed_work_survives_a_failure() {
  wizard_stage only "only"
  wizard_step only a _ok_step "a"
  wizard_step only b _fail_step "b"
  _run
  assert_eq "done" "$(wizard_step_status only a)" \
    "the point of the state file: a failure never costs the work that already succeeded"
}

test_a_person_can_ask_for_another_try() {
  WIZARD_INTERACTIVE=1
  wizard_stage only "only"
  wizard_step only a _flaky_step "a"
  _run "$(_answers again)"
  assert_contains "$STAGES_OUT" "again" "the choices are offered in words"
  assert_eq "done" "$(wizard_step_status only a)" "the second attempt worked"
}

test_a_person_can_stop_and_come_back() {
  WIZARD_INTERACTIVE=1
  wizard_stage first "first"
  wizard_stage second "second"
  wizard_step first a _fail_step "a"
  wizard_step second b _ok_step "b"
  _run "$(_answers stop)"
  assert_ne 0 "$RUN_STAGES_STATUS"
  assert_not_contains "$(_trace_lines)" "second.b"
}

test_a_person_can_skip_optional_work() {
  WIZARD_INTERACTIVE=1
  wizard_stage first "first"
  wizard_stage second "second"
  wizard_step first a _fail_step "a" optional
  wizard_step second b _ok_step "b"
  _run "$(_answers skip)"
  assert_status 0 "$RUN_STAGES_STATUS"
  assert_contains "$(_trace_lines)" "second.b"
  assert_eq "skipped" "$(wizard_step_status first a)"
}

test_required_work_is_not_offered_as_skippable() {
  WIZARD_INTERACTIVE=1
  wizard_stage only "only"
  wizard_step only a _fail_step "a"
  _run "$(_answers stop)"
  assert_not_contains "$STAGES_OUT" "skip" \
    "offering to skip something the install cannot do without would be a lie"
}
