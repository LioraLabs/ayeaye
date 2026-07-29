# Running ayeaye by hand, as a supported way to use it.
#
# A machine with no user service manager is not a broken machine and setup
# must not treat it as one. A container, a stripped-down Linux, a Mac without
# launchctl, and anybody who passed --no-systemd all end up here, and what
# they get is the command to run, where its settings come from, where its
# output goes, and a run that finishes successfully.
#
# The distinction being pinned throughout is SKIP against PENDING. SKIP is the
# wizard's word for "does not apply here" - finished business, nothing left
# over. PENDING means work remains and the closing summary says so. Getting
# these the wrong way round would either nag somebody forever about a machine
# that is fine, or quietly promise a service to somebody who has not got one.

_hard_deps_present() {
  require_host_command python3
  stub_command tmux
  stub_real python3
}

_systemd_session() {
  stub_command systemctl --exit 1 --stderr "unknown systemctl invocation"
  stub_when systemctl '--user show-environment' --exit 0
  stub_when systemctl '--user daemon-reload' --exit 0
  stub_when systemctl '--user enable --now *' --exit 0
}

_unit_file() { printf '%s' "$XDG_CONFIG_HOME/systemd/user/ayeaye.service"; }
_state()     { printf '%s' "$XDG_STATE_HOME/ayeaye/setup-state"; }

# ------------------------------------------------- the machine has no manager

test_a_machine_with_no_service_manager_is_told_how_to_run_it() {
  _hard_deps_present
  run_install --defaults
  assert_status 0 "$RUN_STATUS"
  assert_contains "$RUN_STDOUT" "no systemd user session detected"
  assert_contains "$RUN_STDOUT" "run the server with: $REPO_ROOT/bin/ayeaye"
  assert_contains "$RUN_STDOUT" "(it reads $XDG_CONFIG_HOME/ayeaye/env by itself)"
  assert_contains "$RUN_STDOUT" "its log is whatever the terminal you start it in shows."
}

test_the_manual_path_is_a_finished_run_and_not_an_unfinished_one() {
  # The property the whole ticket turns on. A machine that cannot have a
  # service is a machine where setup has nothing left to do, and telling
  # somebody to come back to it later would be telling them to fix something
  # that is not broken.
  _hard_deps_present
  run_install --defaults
  assert_status 0 "$RUN_STATUS"
  # Narrowed at integration: a sibling step legitimately reports an unread
  # measurement as unfinished in the sandbox, where PATH is only the stubs.
  # The property this test owns is that the *service* leaves nothing behind.
  assert_not_contains "$RUN_STDOUT" "Starting ayeaye in the background (not finished)"
  assert_file_not_contains "$(_state)" "stage.service=pending"
  assert_file_not_contains "$(_state)" "stage.service=failed"
}

test_the_manual_path_writes_nothing_and_runs_nothing() {
  _hard_deps_present
  stub_command systemctl --exit 1
  stub_command launchctl
  run_install --defaults
  assert_file_missing "$(_unit_file)"
  assert_file_missing "$HOME/Library/LaunchAgents/dev.ayeaye.plist"
  assert_file_missing "$HOME/Library/Logs/ayeaye"
  assert_stub_not_called launchctl
  assert_not_contains "$(stub_calls systemctl)" "daemon-reload"
  assert_not_contains "$(stub_calls systemctl)" "enable"
}

test_the_manual_path_still_leaves_the_settings_and_the_key_in_place() {
  # Nothing about "there is no service manager here" should reduce what setup
  # did for the person: they still have a configured, keyed install.
  _hard_deps_present
  run_install --defaults
  assert_file_exists "$XDG_CONFIG_HOME/ayeaye/env"
  assert_file_exists "$XDG_STATE_HOME/ayeaye/token"
  assert_file_mode 600 "$XDG_STATE_HOME/ayeaye/token"
  assert_contains "$RUN_STDOUT" "bookmark: http://"
}

test_the_manual_path_records_that_nothing_was_started() {
  _hard_deps_present
  run_install --defaults
  assert_file_contains "$(_state)" "step.service.unit.started=0"
  assert_contains "$RUN_STDOUT" "ayeaye is not running yet, so there is nothing to check."
}

# ------------------------------------------------------- the flag says no

test_the_flag_is_the_same_supported_mode() {
  _hard_deps_present
  _systemd_session
  run_install --defaults --no-systemd
  assert_status 0 "$RUN_STATUS"
  assert_contains "$RUN_STDOUT" "skipping systemd (--no-systemd)"
  assert_contains "$RUN_STDOUT" "run the server with: $REPO_ROOT/bin/ayeaye"
  # Narrowed at integration: a sibling step legitimately reports an unread
  # measurement as unfinished in the sandbox, where PATH is only the stubs.
  # The property this test owns is that the *service* leaves nothing behind.
  assert_not_contains "$RUN_STDOUT" "Starting ayeaye in the background (not finished)"
  assert_file_missing "$(_unit_file)"
  assert_file_contains "$(_state)" "step.service.unit.started=0"
}

test_the_flag_promises_no_service_in_the_plan_either() {
  # Stage five says what is about to happen. Promising a service and then not
  # installing one is the kind of small lie that makes a person stop reading
  # the rest.
  _hard_deps_present
  _systemd_session
  run_install --defaults --no-systemd
  assert_not_contains "$RUN_STDOUT" "ayeaye will start when you log in"
}

# ------------------------------------------------------------- the plan entry

test_a_machine_that_can_have_a_service_is_told_so_before_it_gets_one() {
  _hard_deps_present
  _systemd_session
  run_install --defaults
  assert_contains "$RUN_STDOUT" "ayeaye will start when you log in, and restart if it stops"
}

test_promising_a_service_is_not_something_the_run_stops_to_ask_about() {
  # A user service of one's own is not in the same class as installing a
  # package or opening a firewall: it is shown and it does not gate. If it
  # did, an unattended run would refuse to proceed.
  _hard_deps_present
  _systemd_session
  run_install --defaults
  assert_status 0 "$RUN_STATUS"
  assert_file_exists "$(_unit_file)" "the run got all the way through"
  assert_not_contains "$RUN_OUTPUT" "may I" "nothing was asked"
}

test_a_machine_with_no_service_manager_promises_nothing() {
  _hard_deps_present
  run_install --defaults
  assert_not_contains "$RUN_STDOUT" "ayeaye will start when you log in"
}

# ------------------------------------- a folder systemd cannot put in a unit

test_a_path_systemd_cannot_express_falls_back_and_says_it_is_unfinished() {
  # The other side of the SKIP/PENDING line. This machine *can* run a service;
  # the folder ayeaye happens to be in is what stops it, and that is fixed by
  # moving the folder rather than by setup trying again. So: the manual
  # instructions, a successful run, and the work listed as outstanding.
  _hard_deps_present
  _systemd_session
  local repo="$TEST_TMPDIR/od\"d"
  mkdir -p "$repo"
  cp "$REPO_ROOT/install.sh" "$REPO_ROOT/env.template" "$repo/"
  cp -R "$REPO_ROOT/lib" "$repo/lib"

  run_script "$repo/install.sh" --defaults
  assert_status 0 "$RUN_STATUS" "everything else about the install worked"
  assert_contains "$RUN_STDOUT" "systemd cannot name such a folder in a"
  assert_contains "$RUN_STDOUT" "run the server with: $repo/bin/ayeaye"
  assert_file_missing "$(_unit_file)" "a unit that could never load is not written"
  assert_not_contains "$(stub_calls systemctl)" "daemon-reload"
  assert_contains "$RUN_STDOUT" "not finished, and worth coming back to:"
  assert_file_contains "$(_state)" "stage.service=pending"
}
