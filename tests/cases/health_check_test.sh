# The closing health check: does what was chosen actually work?
#
# Every check here is driven against a real HTTP server - tests/lib/fake_ayeaye.py
# - rather than against a stubbed curl, because what is under test is a set of
# assertions about http status codes and a stub that answers whatever the test
# tells it to answer would be testing the stub.
#
# The property this file exists to defend, above all the others: a check that
# was **skipped** because nobody asked for that capability must never be
# reported the same way as a check that **passed**. Conflating them is how
# somebody ends up believing their phone can reach their agents when nothing
# was ever set up for it.
#
# The second one: an ayeaye that answers an API request carrying no key at all
# is a computer anybody on that network can run commands on. That is asserted
# positively - the check has to fail when the lock is off - and the proof that
# it can fail is a server deliberately built with the lock off.

_load_health() {
  REPO="$REPO_ROOT"
  CONF_DIR="$XDG_CONFIG_HOME/ayeaye"
  ENV_FILE="$CONF_DIR/env"
  TOKEN_FILE="$XDG_STATE_HOME/ayeaye/token"
  WIZARD_STATE_DIR="$XDG_STATE_HOME/ayeaye"
  WIZARD_STATE_FILE="$WIZARD_STATE_DIR/setup-state"
  WIZARD_CONSENT_LEDGER="$WIZARD_STATE_DIR/setup-consent.log"
  WIZARD_LOG_FILE="$WIZARD_STATE_DIR/setup.log"
  UNIT_DIR="$XDG_CONFIG_HOME/systemd/user"
  NO_SYSTEMD=0
  HEALTH_TIMEOUT=5
  PLATFORM_OS_RELEASE_FILES=""
  export PLATFORM_OS_RELEASE_FILES
  . "$REPO_ROOT/lib/wizard.sh"
  local stage
  for stage in welcome detect report configure plan install service finish; do
    wizard_stage "$stage" "$stage"
  done
  . "$REPO_ROOT/lib/steps/50-access.sh"
  . "$REPO_ROOT/lib/steps/80-health.sh"
  wizard_state_load
  WIZARD_INTERACTIVE=0
}

setup() {
  require_host_command python3
  require_host_command curl
  stub_real curl
  HEALTH_TEST_TOKEN="health-token-Qv93zT"
  FAKE_PID=""
  _load_health
  mkdir -p "$CONF_DIR" "$WIZARD_STATE_DIR"
  printf '%s\n' "$HEALTH_TEST_TOKEN" > "$TOKEN_FILE"
  chmod 600 "$TOKEN_FILE"
  # The service stage's answer, which is what makes the check run at all.
  wizard_remember step.service.unit.started 1
  # Nothing on this machine has a user service manager inside the sandbox, so
  # the service check reports "not asked for" unless a test says otherwise.
  stub_command systemctl --exit 1
}

teardown() {
  [ -n "${FAKE_PID:-}" ] || return 0
  kill "$FAKE_PID" 2>/dev/null
  wait "$FAKE_PID" 2>/dev/null
  return 0
}

# _serve [argument]… - start the stand-in and point the settings file at it.
#
# The port is whatever the operating system handed out, read back from the
# server's own first line: a fixed port would make two tests running at once
# fight over it, and the suite runs tests in parallel with nothing but a
# sandbox between them.
_serve() {
  local waited=0 line
  : > "$TEST_TMPDIR/fake.out"
  "$HARNESS_PYTHON" "$TESTS_DIR/lib/fake_ayeaye.py" \
    --token "$HEALTH_TEST_TOKEN" "$@" \
    >"$TEST_TMPDIR/fake.out" 2>"$TEST_TMPDIR/fake.err" &
  FAKE_PID=$!
  while [ "$waited" -lt 100 ]; do
    line="$(cat "$TEST_TMPDIR/fake.out" 2>/dev/null || true)"
    case "$line" in
      port=*)
        FAKE_PORT="${line#port=}"
        FAKE_PORT="${FAKE_PORT%%
*}"
        return 0
        ;;
    esac
    sleep 0.1
    waited=$((waited + 1))
  done
  fail "the stand-in ayeaye never said which port it got: $(cat "$TEST_TMPDIR/fake.err" 2>/dev/null)"
}

# _config <allowed-hosts> - the settings a run would have left behind.
_config() {
  cat > "$ENV_FILE" <<ENV
AYEAYE_BIND=127.0.0.1
AYEAYE_PORT=${FAKE_PORT:-8911}
AYEAYE_ALLOWED_HOSTS=${1:-}
ENV
}

# _verdict <check> - what the step recorded for one capability.
_verdict() { wizard_state_get "step.service.health.$1" ""; }

# Into a file rather than into a command substitution: a substitution is a
# subshell, and the step records every verdict through wizard_remember - which
# a subshell would write to disk and then throw away the memory of, leaving
# this file asserting on the state the test started with.
_run_health() {
  _health_check_step >"$TEST_TMPDIR/health.out" 2>&1
  HEALTH_STATUS=$?
  HEALTH_OUTPUT="$(cat "$TEST_TMPDIR/health.out" 2>/dev/null || true)"
  return 0
}

# ================================================== the whole point of the file

test_a_capability_nobody_asked_for_is_not_reported_as_working() {
  _serve
  _config
  _run_health
  # Four capabilities were never chosen in this run. Not one of them may be
  # recorded, or printed, as having passed.
  assert_eq skip "$(_verdict https)"
  assert_eq skip "$(_verdict voice)"
  assert_eq skip "$(_verdict board)"
  assert_eq skip "$(_verdict agents)"
  assert_contains "$HEALTH_OUTPUT" "an https address your phone can open - you did not ask for this"
  assert_contains "$HEALTH_OUTPUT" "talking to your agents out loud - you did not ask for this"
}

test_skipped_and_passed_are_two_different_marks_on_the_screen() {
  _serve
  _config
  _run_health
  # The eye runs down the left-hand column; that column has to carry the
  # difference on its own, without the sentence beside it being read.
  assert_matches "$HEALTH_OUTPUT" "^  ok +ayeaye answers on"
  assert_matches "$HEALTH_OUTPUT" "^  skipped +an https address"
  assert_not_contains "$HEALTH_OUTPUT" "  ok       an https address"
}

test_the_marks_are_explained_before_any_of_them_is_used() {
  _serve
  _config
  _run_health
  assert_contains "$HEALTH_OUTPUT" "ok       it works"
  assert_contains "$HEALTH_OUTPUT" "skipped  you did not ask"
  assert_contains "$HEALTH_OUTPUT" "unknown  setup could not tell"
}

# ============================================ the unauthenticated-rejection assertion

test_an_api_that_refuses_a_request_with_no_key_passes() {
  _serve
  _config
  _run_health
  assert_eq pass "$(_verdict auth)"
  assert_contains "$HEALTH_OUTPUT" "ayeaye refuses anyone without your key"
}

test_an_api_that_answers_a_request_with_no_key_fails_the_step() {
  # The whole reason this check exists. A 200 here means whoever can reach the
  # address can run commands on this computer.
  _serve --behaviour open-api
  _config
  _run_health
  assert_eq fail "$(_verdict auth)"
  assert_status 12 "$HEALTH_STATUS" \
    "a lock that is off is a failure, not merely unfinished work"
}

test_an_open_api_is_said_out_loud_and_not_only_marked() {
  _serve --behaviour open-api
  _config
  _run_health
  assert_contains "$HEALTH_OUTPUT" "STOP."
  assert_contains "$HEALTH_OUTPUT" "can be"
  assert_contains "$HEALTH_OUTPUT" "Do not open this from your phone until"
}

test_nothing_listening_leaves_the_lock_unproven_rather_than_proven() {
  # No server at all. "Could not check" is the only honest answer, and it is
  # emphatically not "passed".
  cat > "$ENV_FILE" <<ENV
AYEAYE_BIND=127.0.0.1
AYEAYE_PORT=1
AYEAYE_ALLOWED_HOSTS=
ENV
  _run_health
  assert_eq unknown "$(_verdict auth)"
  assert_ne pass "$(_verdict auth)"
  assert_status 10 "$HEALTH_STATUS"
}

# ================================================================ local answer

test_a_running_app_is_reported_as_answering() {
  _serve
  _config
  _run_health
  assert_eq pass "$(_verdict local)"
  assert_contains "$HEALTH_OUTPUT" "ayeaye answers on http://127.0.0.1:$FAKE_PORT"
}

test_an_app_that_is_not_there_is_reported_as_unchecked_not_as_broken() {
  cat > "$ENV_FILE" <<ENV
AYEAYE_BIND=127.0.0.1
AYEAYE_PORT=1
AYEAYE_ALLOWED_HOSTS=
ENV
  _run_health
  assert_eq unknown "$(_verdict local)"
}

test_a_run_that_never_started_anything_checks_nothing_and_says_so() {
  wizard_remember step.service.unit.started 0
  _run_health
  assert_status 11 "$HEALTH_STATUS"
  assert_contains "$HEALTH_OUTPUT" "ayeaye is not running yet, so there is nothing to check."
  assert_eq "" "$(_verdict local)" "nothing was checked, so nothing is recorded"
}

# ============================================================ host validation

test_a_configured_host_is_accepted_and_an_unconfigured_one_is_refused() {
  _serve --allow "ayeaye.example.com"
  _config "ayeaye.example.com"
  _run_health
  assert_eq pass "$(_verdict hosts)"
  assert_contains "$HEALTH_OUTPUT" "ayeaye accepts ayeaye.example.com and refuses anything else"
}

test_a_server_that_accepts_every_host_fails_the_host_check() {
  # The half of the assertion that actually proves something. A front end that
  # answers to any name at all is a DNS rebinding away from being driven by a
  # web page somebody else wrote.
  _serve --allow "ayeaye.example.com" --behaviour no-host-check
  _config "ayeaye.example.com"
  _run_health
  assert_eq fail "$(_verdict hosts)"
}

test_a_configured_host_the_app_refuses_fails_the_host_check() {
  # Configured in the settings file, not taught to the server: exactly what a
  # run that wrote the allow list and never restarted the app looks like.
  _serve
  _config "forgotten.example.com"
  _run_health
  assert_eq fail "$(_verdict hosts)"
}

test_no_configured_host_is_not_something_to_check() {
  _serve
  _config
  _run_health
  assert_eq skip "$(_verdict hosts)"
}

# ================================================================== the agents

test_only_the_agents_that_were_chosen_are_checked() {
  _serve
  _config
  wizard_remember answer.agent.claude 1
  stub_command claude
  _run_health
  assert_eq pass "$(_verdict agents)"
}

test_an_agent_that_was_chosen_and_is_not_here_fails() {
  _serve
  _config
  wizard_remember answer.agent.codex 1
  _run_health
  assert_eq fail "$(_verdict agents)"
  assert_status 10 "$HEALTH_STATUS"
}

test_choosing_no_agent_is_not_a_failure() {
  _serve
  _config
  _run_health
  assert_eq skip "$(_verdict agents)"
}

# =================================================================== the voice

test_voice_is_checked_by_asking_the_app_not_the_disk() {
  _serve
  _config
  wizard_remember answer.voice.model small.en
  _run_health
  assert_eq pass "$(_verdict voice)"
  assert_contains "$HEALTH_OUTPUT" "the talk button is live"
}

test_a_chosen_voice_the_app_cannot_offer_is_a_failure() {
  _serve --behaviour voice-off
  _config
  wizard_remember answer.voice.model small.en
  _run_health
  assert_eq fail "$(_verdict voice)"
}

test_the_key_is_never_left_behind_after_an_authenticated_check() {
  _serve
  _config
  wizard_remember answer.voice.model small.en
  _run_health
  assert_file_missing "$WIZARD_STATE_DIR/health-request" \
    "the file the key was written into is removed the moment it has been used"
}

test_the_key_never_reaches_the_log() {
  _serve
  _config
  wizard_remember answer.voice.model small.en
  _run_health
  assert_file_not_contains "$WIZARD_LOG_FILE" "$HEALTH_TEST_TOKEN"
  assert_not_contains "$HEALTH_OUTPUT" "$HEALTH_TEST_TOKEN"
}

# =================================================================== the board

test_the_board_is_checked_only_when_it_was_chosen() {
  _serve
  _config
  _run_health
  assert_eq skip "$(_verdict board)"
}

test_a_chosen_board_with_no_cliban_on_the_machine_fails() {
  _serve
  _config
  wizard_remember answer.board 1
  _run_health
  assert_eq fail "$(_verdict board)"
}

test_a_chosen_board_the_app_can_answer_for_passes() {
  _serve
  _config
  wizard_remember answer.board 1
  stub_command cliban
  _run_health
  assert_eq pass "$(_verdict board)"
}

test_a_chosen_board_the_app_cannot_answer_for_fails() {
  _serve --behaviour no-board
  _config
  wizard_remember answer.board 1
  stub_command cliban
  _run_health
  assert_eq fail "$(_verdict board)"
}

# ===================================================================== https

test_the_mode_that_keeps_ayeaye_here_has_no_front_end_to_check() {
  _serve
  _config
  wizard_remember answer.access.mode local
  _run_health
  assert_eq skip "$(_verdict https)"
  assert_not_contains "$HEALTH_OUTPUT" "  ok       https://"
}

test_a_chosen_front_end_that_cannot_be_reached_is_not_reported_as_working() {
  _serve
  _config "nothing-here.invalid"
  wizard_remember answer.access.mode tailscale
  _run_health
  assert_ne pass "$(_verdict https)"
}

# ================================================================ the service

test_a_machine_with_no_service_manager_is_not_failed_for_it() {
  _serve
  _config
  _run_health
  assert_eq skip "$(_verdict service)"
}

test_a_run_told_to_skip_the_service_does_not_check_for_one() {
  _serve
  _config
  NO_SYSTEMD=1
  _run_health
  assert_eq skip "$(_verdict service)"
}

# ================================================== what the step itself answers

test_everything_applicable_passing_is_the_only_way_to_report_finished() {
  _serve
  _config
  _run_health
  assert_status 0 "$HEALTH_STATUS"
  assert_eq 0 "$_HEALTH_FAILED"
  assert_eq 0 "$_HEALTH_UNKNOWN"
}

test_a_previous_runs_verdicts_are_never_reported_as_this_runs() {
  # A resumed run that finds last week's answers in the state file and prints
  # them as today's would be the worst kind of health check.
  wizard_remember step.service.health.voice pass
  wizard_remember step.service.health.board pass
  _serve
  _config
  _run_health
  assert_eq skip "$(_verdict voice)"
  assert_eq skip "$(_verdict board)"
}
