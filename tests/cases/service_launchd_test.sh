# The macOS half: what gets written to ~/Library/LaunchAgents, and exactly
# which launchctl commands would run.
#
# There is no Mac to run this on. Everything here is a Linux host wearing a
# Darwin costume - `uname -s` answering Darwin, sw_vers from a fixture, a
# recording launchctl - which is enough to pin every decision this project
# makes and not enough to prove that launchd accepts the result. What a
# property list has to contain is pinned byte for byte in service_units_test.sh
# and checked against plistlib; that launchd loads it is not verified anywhere,
# and that gap is recorded in the ticket rather than papered over here.

_hard_deps_present() {
  require_host_command python3
  stub_command tmux
  stub_real python3
}

# A Mac with launchd, and no service of ours registered yet.
#
# The launchctl stub keeps one bit of state - whether our label is registered -
# because that bit is what every decision on this platform turns on, and a
# fixed answer could not tell a first install from a repair. `bootstrap` sets
# it, `bootout` clears it, `print` reports it. Each call is still recorded the
# ordinary way, so the assertions are on argv as everywhere else.
_mac_with_launchd() {
  stub_command uname --exit 1
  stub_when uname '-s' --stdout "Darwin"
  stub_when uname '-m' --stdout "arm64"
  stub_command_from_fixture sw_vers sw_vers/macos-15.1
  PLATFORM_OS_RELEASE_FILES=""
  export PLATFORM_OS_RELEASE_FILES
  stub_script launchctl <<'SH'
# Whether the property list was already on disk when this call was made. The
# ordering test is the only reader; every other test ignores the file.
if [ -f "$HOME/Library/LaunchAgents/dev.ayeaye.plist" ]; then
  printf 'plist-present %s\n' "$*" >> "$HOME/launchctl-order"
else
  printf 'plist-absent %s\n' "$*" >> "$HOME/launchctl-order"
fi
case "$*" in
  print*)
    [ -f "$HOME/launchctl-registered" ] || exit 1
    printf 'state = running\n'
    ;;
  bootstrap*) : > "$HOME/launchctl-registered" ;;
  bootout*)   rm -f "$HOME/launchctl-registered" ;;
esac
exit 0
SH
}

# The same Mac, with our agent already registered and running.
_already_registered() {
  : > "$HOME/launchctl-registered"
}

_plist() { printf '%s' "$HOME/Library/LaunchAgents/dev.ayeaye.plist"; }
_logs()  { printf '%s' "$HOME/Library/Logs/ayeaye"; }
_uid()   { id -u; }
_state() { printf '%s' "$XDG_STATE_HOME/ayeaye/setup-state"; }

# ------------------------------------------------------------ what is written

test_a_mac_gets_a_launch_agent_and_not_a_systemd_unit() {
  _hard_deps_present
  _mac_with_launchd
  run_install --defaults
  assert_status 0 "$RUN_STATUS"
  assert_file_exists "$(_plist)"
  assert_contains "$RUN_STDOUT" "installed $(_plist)"
  assert_file_missing "$XDG_CONFIG_HOME/systemd/user/ayeaye.service" \
    "a Mac has no systemd to install a unit into"
  assert_command_absent systemctl
}

test_the_agent_names_this_machines_real_paths() {
  _hard_deps_present
  _mac_with_launchd
  run_install --defaults
  local plist
  plist="$(_plist)"
  assert_file_contains "$plist" "<string>$REPO_ROOT/bin/ayeaye</string>"
  assert_file_contains "$plist" "<string>$XDG_CONFIG_HOME</string>"
  assert_file_contains "$plist" "<string>$(_logs)/ayeaye.log</string>"
  assert_file_contains "$plist" "<string>dev.ayeaye</string>"
}

test_no_setting_is_copied_into_the_agent() {
  _hard_deps_present
  _mac_with_launchd
  stdin_lines "0.0.0.0" "9977" "mac.example.org" "https://ntfy.example/mac"
  run_install
  assert_status 0 "$RUN_STATUS"
  assert_file_contains "$XDG_CONFIG_HOME/ayeaye/env" "AYEAYE_PORT=9977" \
    "anchor: those answers really were taken"
  assert_file_not_contains "$(_plist)" "9977"
  assert_file_not_contains "$(_plist)" "0.0.0.0"
  assert_file_not_contains "$(_plist)" "mac.example.org"
  assert_file_not_contains "$(_plist)" "ntfy.example"
}

test_the_log_directory_the_agent_names_is_created() {
  # launchd refuses to start a job whose StandardOutPath cannot be opened, so
  # an agent installed without this is an agent that never runs.
  _hard_deps_present
  _mac_with_launchd
  assert_file_missing "$(_logs)"
  run_install --defaults
  assert_file_exists "$(_logs)"
}

test_the_agent_is_on_disk_before_anything_is_registered() {
  # Bootstrapping a plist that is not there yet fails, and the failure is
  # confusing rather than informative. The stub records, for every call,
  # whether the file existed at the moment it was made.
  _hard_deps_present
  _mac_with_launchd
  pty_answers "" "" "" ""
  pty_expect "enable and start it now?" "y"
  pty_install
  assert_status 0 "$PTY_STATUS"
  assert_file_contains "$HOME/launchctl-order" "plist-present bootstrap"
  assert_file_not_contains "$HOME/launchctl-order" "plist-absent bootstrap"
}

# --------------------------------------------------------------- registering

test_an_unattended_run_registers_nothing_and_says_how() {
  _hard_deps_present
  _mac_with_launchd
  run_install --defaults
  assert_status 0 "$RUN_STATUS"
  assert_not_contains "$(stub_calls launchctl)" "bootstrap" \
    "an unattended run must not start a service behind the user's back"
  assert_contains "$RUN_STDOUT" "start it with: launchctl bootstrap gui/$(_uid) $(_plist)"
  assert_file_contains "$(_state)" "step.service.unit.started=0"
}

test_confirming_the_prompt_bootstraps_the_agent() {
  _hard_deps_present
  _mac_with_launchd
  pty_answers "" "" "" ""
  pty_expect "enable and start it now?" "y"
  pty_install
  assert_status 0 "$PTY_STATUS"
  assert_stub_called_with launchctl "launchctl bootstrap gui/$(_uid) $(_plist)"
  assert_contains "$PTY_TRANSCRIPT" "started. status: launchctl print gui/$(_uid)/dev.ayeaye"
  assert_file_contains "$(_state)" "step.service.unit.started=1"
}

test_declining_the_prompt_leaves_the_agent_registered_as_it_was() {
  _hard_deps_present
  _mac_with_launchd
  pty_answers "" "" "" ""
  pty_expect "enable and start it now?" "n"
  pty_install
  assert_status 0 "$PTY_STATUS"
  assert_not_contains "$(stub_calls launchctl)" "bootstrap"
  assert_not_contains "$(stub_calls launchctl)" "bootout"
  assert_file_exists "$(_plist)" "the agent is still installed, just not started"
}

test_no_daemon_reload_is_invented_for_a_platform_that_has_none() {
  # launchd re-reads a property list by being handed it again. A command that
  # looks like systemd's daemon-reload and does nothing would be worse than
  # the honest absence of one.
  _hard_deps_present
  _mac_with_launchd
  run_install --defaults
  assert_not_contains "$(stub_calls launchctl)" "daemon-reload"
  assert_not_contains "$(stub_calls launchctl)" "load"
}

# ------------------------------------------------------------------- repair

test_a_changed_agent_is_booted_out_before_it_is_bootstrapped_again() {
  # Bootstrapping a label launchd already knows fails outright. Without the
  # bootout, every second run on a Mac would be an error and a broken agent
  # could never be repaired.
  _hard_deps_present
  _mac_with_launchd
  _already_registered
  run_install --defaults
  assert_status 0 "$RUN_STATUS"
  local calls
  calls="$(stub_calls launchctl)"
  assert_contains "$calls" "bootout gui/$(_uid)/dev.ayeaye"
  assert_contains "$calls" "bootstrap gui/$(_uid) $(_plist)"
  assert_contains "$RUN_STDOUT" "ayeaye was already running, so it has been restarted"
  assert_file_contains "$(_state)" "step.service.unit.started=1"
}

test_an_unchanged_agent_that_is_already_running_is_left_entirely_alone() {
  # The repair path must not restart a service for no reason: somebody who
  # re-runs setup to change one setting should not have their session
  # interrupted by an agent that did not need touching.
  _hard_deps_present
  _mac_with_launchd
  run_install --defaults
  _already_registered
  run_install --defaults
  assert_status 0 "$RUN_STATUS"
  assert_not_contains "$(stub_calls launchctl)" "bootout"
  assert_not_contains "$(stub_calls launchctl)" "bootstrap"
  assert_contains "$RUN_STDOUT" "ayeaye is already set up to start when you log in, and running."
}

test_a_hand_edited_agent_is_kept_before_it_is_replaced() {
  _hard_deps_present
  _mac_with_launchd
  run_install --defaults
  printf 'not a property list at all\n' > "$(_plist)"
  run_install --defaults
  assert_status 0 "$RUN_STATUS"
  assert_file_not_contains "$(_plist)" "not a property list" \
    "the agent is a generated file, not a config file"
  assert_contains "$RUN_STDOUT" "saved a copy of the previous ayeaye definition at"
  local backup
  backup="$(ls "$XDG_STATE_HOME/ayeaye/backups")"
  assert_contains "$backup" "dev.ayeaye.plist"
}

# --------------------------------------------------------------- what to do next

test_a_mac_is_told_where_its_log_is_and_how_to_remove_the_agent() {
  _hard_deps_present
  _mac_with_launchd
  run_install --defaults
  assert_contains "$RUN_STDOUT" "to read its log: tail -f $(_logs)/ayeaye.log"
  assert_contains "$RUN_STDOUT" "to remove it later: launchctl bootout gui/$(_uid)/dev.ayeaye"
  assert_contains "$RUN_STDOUT" "then delete $(_plist)"
  assert_not_contains "$RUN_STDOUT" "journalctl" "there is no journal on a Mac"
  assert_not_contains "$RUN_STDOUT" "enable-linger" "and no linger either"
}

test_no_systemd_on_a_mac_installs_nothing_and_still_succeeds() {
  _hard_deps_present
  _mac_with_launchd
  run_install --defaults --no-systemd
  assert_status 0 "$RUN_STATUS"
  assert_file_missing "$(_plist)"
  assert_not_contains "$(stub_calls launchctl)" "bootstrap"
  assert_contains "$RUN_STDOUT" "run the server with: $REPO_ROOT/bin/ayeaye"
  assert_file_contains "$(_state)" "step.service.unit.started=0"
}

test_a_mac_without_launchctl_takes_the_manual_path() {
  _hard_deps_present
  _mac_with_launchd
  stub_remove launchctl
  run_install --defaults
  assert_status 0 "$RUN_STATUS"
  assert_contains "$RUN_STDOUT" "no launchd session detected"
  assert_contains "$RUN_STDOUT" "run the server with: $REPO_ROOT/bin/ayeaye"
  assert_file_missing "$(_plist)"
}
