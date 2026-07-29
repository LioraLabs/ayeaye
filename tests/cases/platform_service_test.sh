# The service adapter: systemd's user session, launchd, or neither.
#
# Same shape as the package adapter - a string-returning form that runs
# nothing and an executing form that runs exactly that string - because the
# wizard has to be able to show someone the command it is about to run before
# it runs it.

_platform_load() {
  if declare -f platform_reset >/dev/null 2>&1; then
    platform_reset
  else
    . "$REPO_ROOT/lib/platform.sh"
  fi
}

_platform_from() {
  if [ "$1" = "none" ]; then
    PLATFORM_OS_RELEASE_FILES=""
  else
    assert_fixture_exists "os-release/$1"
    PLATFORM_OS_RELEASE_FILES="$(fixture_file "os-release/$1")"
  fi
  export PLATFORM_OS_RELEASE_FILES
  _platform_load
}

_stub_uname() {
  stub_command uname --exit 1
  stub_when uname '-s' --stdout "$1"
  stub_when uname '-m' --stdout "$2"
}

# A linux box with a working systemd user session.
_on_systemd() {
  _stub_uname Linux x86_64
  stub_command id --stdout "1000"
  stub_command systemctl --exit 1
  stub_when systemctl '--user show-environment' --exit 0 --stdout "LANG=C"
  _platform_from debian-12
}

# A Mac with launchd. The uid is what addresses a launchd domain, so it is
# stubbed rather than taken from whoever is running the suite.
_on_launchd() {
  _stub_uname Darwin arm64
  stub_command_from_fixture sw_vers sw_vers/macos-15.1
  stub_command id --stdout "501"
  stub_command launchctl
  _platform_from none
}

# Neither: the documented manual-fallback path.
_on_nothing() {
  _stub_uname Linux x86_64
  stub_command id --stdout "1000"
  assert_command_absent systemctl
  assert_command_absent launchctl
  _platform_from void
}

# ------------------------------------------------------------------ naming

test_systemd_names_a_unit_and_launchd_names_a_label() {
  _on_systemd
  assert_eq "ayeaye.service" "$(platform_service_unit ayeaye)"

  _on_launchd
  assert_eq "dev.ayeaye" "$(platform_service_unit ayeaye)" \
    "the label this project's plist already uses"
}

test_one_name_works_whichever_platform_it_was_written_for() {
  # Shared code holds one name for the service. It must not matter whether it
  # was spelled the systemd way, the launchd way, or neither.
  _on_systemd
  assert_eq "ayeaye.service" "$(platform_service_unit ayeaye.service)"
  assert_eq "ayeaye.service" "$(platform_service_unit dev.ayeaye)" \
    "the launchd spelling must not become dev.ayeaye.service"

  _on_launchd
  assert_eq "dev.ayeaye" "$(platform_service_unit dev.ayeaye)"
  assert_eq "dev.ayeaye" "$(platform_service_unit ayeaye.service)" \
    "and the systemd spelling must not become the label ayeaye.service"
}

test_a_unit_that_is_not_a_service_keeps_its_own_kind() {
  _on_systemd
  assert_eq "ayeaye.socket" "$(platform_service_unit ayeaye.socket)"
  assert_eq "ayeaye.timer" "$(platform_service_unit ayeaye.timer)"
}

test_the_launchd_prefix_is_a_tunable_and_really_is_used() {
  PLATFORM_LAUNCHD_PREFIX="com.example"
  export PLATFORM_LAUNCHD_PREFIX
  _on_launchd
  assert_eq "com.example.ayeaye" "$(platform_service_unit ayeaye)"
  assert_eq "launchctl kickstart gui/501/com.example.ayeaye" \
    "$(platform_service_command ayeaye start)"
  assert_eq "com.example.ayeaye" "$(platform_service_unit com.example.ayeaye)" \
    "and a name already carrying the new prefix is left alone"
}

test_the_detected_uid_is_the_one_the_commands_address() {
  _on_launchd
  assert_eq "501" "$(platform_uid)"
  assert_contains "$(platform_service_command ayeaye status)" "gui/501/"
}

test_naming_a_unit_where_there_is_no_service_manager_says_nothing() {
  _on_nothing
  local out status
  out="$(platform_service_unit ayeaye)"
  status=$?
  assert_eq "" "$out"
  assert_status 1 "$status"
}

test_naming_nothing_at_all_is_a_caller_bug_not_a_platform_fact() {
  _on_systemd
  local status
  platform_service_unit >/dev/null
  status=$?
  assert_status 2 "$status"

  platform_service_command "" start >/dev/null
  status=$?
  assert_status 2 "$status" "and set -u must not turn a missing argument into a crash"
}

# ---------------------------------------------------------- systemd commands

test_systemd_generates_its_user_scoped_commands() {
  _on_systemd
  assert_eq "systemctl --user daemon-reload" \
    "$(platform_service_command ayeaye reload)"
  assert_eq "systemctl --user enable --now ayeaye.service" \
    "$(platform_service_command ayeaye enable)"
  assert_eq "systemctl --user disable --now ayeaye.service" \
    "$(platform_service_command ayeaye disable)"
  assert_eq "systemctl --user start ayeaye.service" \
    "$(platform_service_command ayeaye start)"
  assert_eq "systemctl --user stop ayeaye.service" \
    "$(platform_service_command ayeaye stop)"
  assert_eq "systemctl --user status ayeaye.service" \
    "$(platform_service_command ayeaye status)"
}

test_every_systemd_command_is_user_scoped_and_none_asks_for_root() {
  _on_systemd
  local op cmd
  for op in reload enable disable start stop status; do
    cmd="$(platform_service_command ayeaye "$op")"
    assert_contains "$cmd" "--user" "$op must not reach for the system manager"
    assert_not_contains "$cmd" "sudo" "$op is a per-user service; root is never right"
  done
}

# ---------------------------------------------------------- launchd commands

test_launchd_generates_domain_addressed_commands() {
  _on_launchd
  assert_eq "launchctl bootstrap gui/501 /tmp/dev.ayeaye.plist" \
    "$(platform_service_command ayeaye enable /tmp/dev.ayeaye.plist)"
  assert_eq "launchctl bootout gui/501/dev.ayeaye" \
    "$(platform_service_command ayeaye disable)"
  assert_eq "launchctl kickstart gui/501/dev.ayeaye" \
    "$(platform_service_command ayeaye start)"
  assert_eq "launchctl print gui/501/dev.ayeaye" \
    "$(platform_service_command ayeaye status)"
}

test_stopping_leaves_the_service_registered_on_both_platforms() {
  # bootout is the inverse of bootstrap, so mapping stop to it would make
  # stop-then-start work on Linux and fail on a Mac with "Could not find
  # service". A signal is the analogue of systemctl --user stop.
  _on_launchd
  assert_eq "launchctl kill SIGTERM gui/501/dev.ayeaye" \
    "$(platform_service_command ayeaye stop)"
  assert_ne "$(platform_service_command ayeaye stop)" \
    "$(platform_service_command ayeaye disable)" \
    "stop and disable are different questions"
}

test_a_plist_path_with_a_space_in_it_stays_one_argument() {
  # macOS home directories with a space in them are ordinary, and this string
  # is about to be handed to eval.
  _on_launchd
  local cmd
  cmd="$(platform_service_command ayeaye enable "/Users/John Smith/Library/LaunchAgents/dev.ayeaye.plist")"
  assert_contains "$cmd" "'/Users/John Smith/Library/LaunchAgents/dev.ayeaye.plist'"

  platform_service_run ayeaye enable "/Users/John Smith/Library/LaunchAgents/dev.ayeaye.plist"
  assert_stub_called_with launchctl \
    "launchctl bootstrap gui/501 '/Users/John Smith/Library/LaunchAgents/dev.ayeaye.plist'"
}

test_launchd_without_a_uid_refuses_rather_than_addressing_nothing() {
  _stub_uname Darwin arm64
  stub_command_from_fixture sw_vers sw_vers/macos-15.1
  stub_command launchctl
  stub_remove id
  assert_command_absent id
  _platform_from none
  assert_eq "launchd" "$(platform_service_manager)" "anchor: launchd is still there"
  assert_eq "" "$(platform_uid)"
  local out status
  out="$(platform_service_command ayeaye start)"
  status=$?
  assert_eq "" "$out" "gui//dev.ayeaye addresses nothing at all"
  assert_status 1 "$status"
}

test_launchd_has_no_reload_and_does_not_invent_one() {
  _on_launchd
  local out status
  out="$(platform_service_command ayeaye reload)"
  status=$?
  assert_eq "" "$out" "launchd re-reads a plist by being bootstrapped again"
  assert_status 1 "$status"
}

test_launchd_cannot_enable_without_being_told_where_the_plist_is() {
  _on_launchd
  local out status
  out="$(platform_service_command ayeaye enable)"
  status=$?
  assert_eq "" "$out"
  assert_status 1 "$status" "bootstrap takes a path, and guessing one would be worse"
}

# ------------------------------------------------------- nothing to talk to

test_where_there_is_no_service_manager_every_command_is_empty() {
  _on_nothing
  if platform_service_can_act; then fail "there is nothing here to talk to"; fi
  assert_eq "no-service-manager" "$(platform_service_blocker)"

  local op out status
  for op in reload enable disable start stop status; do
    out="$(platform_service_command ayeaye "$op")"
    status=$?
    assert_eq "" "$out" "$op has no meaning here"
    assert_status 1 "$status"
  done
}

test_a_machine_with_a_service_manager_can_act() {
  _on_systemd
  platform_service_can_act || fail "a live systemd user session is exactly the case"
  assert_eq "" "$(platform_service_blocker)"

  _on_launchd
  platform_service_can_act || fail "launchd too"
  assert_eq "" "$(platform_service_blocker)"
}

# ------------------------------------------------------------ act, or do not

test_an_operation_this_interface_does_not_know_is_an_error_not_an_empty_string() {
  _on_systemd
  local out status
  out="$(platform_service_command ayeaye restart-please)"
  status=$?
  assert_eq "" "$out"
  assert_status 2 "$status" \
    "a typo in a caller has to be distinguishable from a platform that cannot do it"
}

test_asking_what_would_be_run_runs_nothing() {
  _on_systemd
  # Detection itself asks systemctl one question - whether a user session
  # answers - so it is got out of the way first. Anything past that would be
  # this adapter acting when it was only asked to describe.
  local before after op
  platform_detect
  before="$(stub_call_count systemctl)"
  assert_ne "0" "$before" "anchor: detection really did probe"
  for op in reload enable disable start stop status; do
    platform_service_command ayeaye "$op" >/dev/null
  done
  after="$(stub_call_count systemctl)"
  assert_eq "$before" "$after" "generating a command must not run it"
}

test_the_executing_form_runs_exactly_what_the_string_said() {
  _on_systemd
  platform_service_run ayeaye enable
  assert_stub_called_with systemctl "systemctl --user enable --now ayeaye.service"

  _on_launchd
  platform_service_run ayeaye start
  assert_stub_called_with launchctl "launchctl kickstart gui/501/dev.ayeaye"
}

test_the_executing_form_refuses_where_there_is_nothing_to_run() {
  _on_nothing
  local status
  platform_service_run ayeaye start
  status=$?
  assert_status 1 "$status"
}

test_the_executing_form_reports_the_same_status_the_string_form_would_have() {
  # Without this the "the statuses are load-bearing" promise holds for the
  # string form only, and a caller acting on platform_service_run cannot tell
  # its own typo from a platform that cannot help.
  _on_systemd
  local status
  platform_service_run ayeaye restart-please
  status=$?
  assert_status 2 "$status" "an operation this interface does not have"
  assert_stub_not_called sudo

  _on_launchd
  platform_service_run ayeaye reload
  status=$?
  assert_status 1 "$status" "an operation launchd does not have"

  platform_service_run ayeaye enable
  status=$?
  assert_status 1 "$status" "bootstrap with no plist path given"
  assert_stub_not_called launchctl "none of those may have run anything"
}
