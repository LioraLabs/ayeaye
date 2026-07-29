# When a whisper service is installed, and when it deliberately is not.
#
# whisper.cpp's server used to be an example unit under systemd/user/ that a
# person copied and edited by hand, filling in the path to their binary and
# their model. It is generated now, from the same registry and the same two
# renderers as everything else - which is why that example is gone rather than
# sitting alongside a second description of the same service.
#
# The rule it is installed under: only when this machine really has the binary
# and the settings really name a model. A definition pointing at a binary that
# is not there fails at every login and leaves a red line in somebody's status
# output forever, which is worse than not writing one.

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

_whisper_installed() { stub_command whisper-server; }

_a_model_is_configured() {
  mkdir -p "$XDG_CONFIG_HOME/ayeaye"
  printf 'VOICE_WHISPER_MODEL=/models/ggml-large-v3.bin\n' \
    >> "$XDG_CONFIG_HOME/ayeaye/env"
}

_whisper_unit() { printf '%s' "$XDG_CONFIG_HOME/systemd/user/whisper-server.service"; }

# ------------------------------------------------------------- when it is not

test_no_whisper_binary_means_no_whisper_service_and_nothing_said() {
  _hard_deps_present
  _systemd_session
  assert_command_absent whisper-server
  run_install --defaults
  assert_status 0 "$RUN_STATUS"
  assert_file_missing "$(_whisper_unit)"
  # The dependency report a stage earlier does say the word - it is listing
  # what the voice tier is missing, which is its job. What must not appear is
  # anything about a whisper *service*, on a machine that has no whisper.
  assert_not_contains "$RUN_STDOUT" "installed $(_whisper_unit)" \
    "a machine without it does not need telling about it"
  assert_not_contains "$RUN_STDOUT" "transcription model"
  assert_not_contains "$RUN_STDOUT" "VOICE_WHISPER_MODEL"
}

test_whisper_without_a_model_is_explained_and_not_installed() {
  _hard_deps_present
  _systemd_session
  _whisper_installed
  run_install --defaults
  assert_status 0 "$RUN_STATUS"
  assert_file_missing "$(_whisper_unit)" \
    "a service that would fail at every login is worse than none"
  assert_contains "$RUN_STDOUT" "your settings do not say where the"
  assert_contains "$RUN_STDOUT" "VOICE_WHISPER_MODEL"
}

test_a_missing_whisper_never_stops_the_rest_of_the_install() {
  _hard_deps_present
  _systemd_session
  _whisper_installed
  run_install --defaults
  assert_status 0 "$RUN_STATUS"
  assert_file_exists "$XDG_CONFIG_HOME/systemd/user/ayeaye.service" \
    "ayeaye is what setup was asked for; whisper is an extra"
  assert_contains "$RUN_STDOUT" "bookmark: http://"
}

# --------------------------------------------------------------- when it is

test_a_machine_with_whisper_and_a_model_gets_a_service() {
  _hard_deps_present
  _systemd_session
  _whisper_installed
  run_install --defaults
  _a_model_is_configured
  run_install --defaults
  assert_status 0 "$RUN_STATUS"
  assert_file_exists "$(_whisper_unit)"
  assert_contains "$RUN_STDOUT" "installed $(_whisper_unit)"
  assert_contains "$RUN_STDOUT" "it reads the model, the address and the thread count"
}

test_the_whisper_unit_names_the_binary_this_machine_has() {
  _hard_deps_present
  _systemd_session
  _whisper_installed
  run_install --defaults
  _a_model_is_configured
  run_install --defaults
  assert_file_contains "$(_whisper_unit)" "$STUB_BIN/whisper-server"
  assert_file_contains "$(_whisper_unit)" "EnvironmentFile=-$XDG_CONFIG_HOME/ayeaye/env"
}

test_no_whisper_setting_is_copied_into_the_whisper_unit() {
  # The whole reason the service runs a short shell instead of the binary
  # directly. Baking the model path and the port in would put those settings
  # in two places, and a unit written today would disagree with a settings
  # file edited tomorrow.
  _hard_deps_present
  _systemd_session
  _whisper_installed
  run_install --defaults
  mkdir -p "$XDG_CONFIG_HOME/ayeaye"
  printf 'VOICE_WHISPER_MODEL=/models/findable-model.bin\n' >> "$XDG_CONFIG_HOME/ayeaye/env"
  printf 'VOICE_WHISPER_SERVER=10.11.12.13:59999\n' >> "$XDG_CONFIG_HOME/ayeaye/env"
  printf 'VOICE_WHISPER_THREADS=99\n' >> "$XDG_CONFIG_HOME/ayeaye/env"
  run_install --defaults
  assert_file_exists "$(_whisper_unit)" "anchor: there is a unit to inspect"
  assert_file_not_contains "$(_whisper_unit)" "findable-model"
  assert_file_not_contains "$(_whisper_unit)" "10.11.12.13"
  assert_file_not_contains "$(_whisper_unit)" "59999"
  assert_file_not_contains "$(_whisper_unit)" "99"
}

test_an_unattended_run_does_not_start_whisper() {
  # Keeping a transcription model resident on a GPU is a decision somebody
  # makes, not a default setup gets to take on their behalf.
  _hard_deps_present
  _systemd_session
  _whisper_installed
  run_install --defaults
  _a_model_is_configured
  run_install --defaults
  assert_not_contains "$(stub_calls systemctl)" "enable --now whisper-server" \
    "installed, and left for the person to turn on"
}

test_being_asked_about_whisper_is_what_starts_it() {
  _hard_deps_present
  _systemd_session
  _whisper_installed
  run_install --defaults
  _a_model_is_configured
  pty_answers ""                            # keep the settings that are there
  # This machine has whisper, so setup offers to set listening up as well.
  # Not what this file is about: 1 is "type to your agents, download nothing".
  pty_expect "which one?" "1"
  pty_expect "enable and start it now?" "y"
  pty_expect "keep the transcription model loaded and ready?" "y"
  pty_install
  assert_status 0 "$PTY_STATUS"
  assert_stub_called_with systemctl "systemctl --user enable --now whisper-server.service"
}

test_declining_leaves_whisper_installed_and_stopped() {
  _hard_deps_present
  _systemd_session
  _whisper_installed
  run_install --defaults
  _a_model_is_configured
  pty_answers ""
  # This machine has whisper, so setup offers to set listening up as well.
  # Not what this file is about: 1 is "type to your agents, download nothing".
  pty_expect "which one?" "1"
  pty_expect "enable and start it now?" "y"
  pty_expect "keep the transcription model loaded and ready?" "n"
  pty_install
  assert_status 0 "$PTY_STATUS"
  assert_file_exists "$(_whisper_unit)"
  assert_not_contains "$(stub_calls systemctl)" "enable --now whisper-server"
}

# ------------------------------------------------------------------- macOS

test_a_mac_gets_a_whisper_agent_in_the_same_conditions() {
  _hard_deps_present
  stub_command uname --exit 1
  stub_when uname '-s' --stdout "Darwin"
  stub_when uname '-m' --stdout "arm64"
  stub_command_from_fixture sw_vers sw_vers/macos-15.1
  stub_command launchctl --exit 1
  PLATFORM_OS_RELEASE_FILES=""
  export PLATFORM_OS_RELEASE_FILES
  _whisper_installed
  run_install --defaults
  _a_model_is_configured
  run_install --defaults
  assert_status 0 "$RUN_STATUS"
  local plist
  plist="$HOME/Library/LaunchAgents/dev.whisper-server.plist"
  assert_file_exists "$plist"
  assert_file_contains "$plist" "<string>dev.whisper-server</string>"
  assert_file_contains "$plist" "$STUB_BIN/whisper-server"
  assert_file_not_contains "$plist" "ggml-large-v3" \
    "the same rule on both platforms: the settings stay in the settings file"
}

# ---------------------------------------------------- the example it replaces

test_the_hand_edited_example_unit_is_gone() {
  # One description of a service, not two. Leaving the example beside a
  # generator that writes the same file is exactly the drift this ticket
  # exists to remove.
  assert_file_missing "$REPO_ROOT/systemd/user/whisper-server.service.example"
  assert_file_missing "$REPO_ROOT/systemd/user/ayeaye.service.template"
  assert_file_missing "$REPO_ROOT/systemd/user/voice-agent.service.template"
}
