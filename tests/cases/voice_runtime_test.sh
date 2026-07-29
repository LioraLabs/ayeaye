# Do the installer and the running app agree?
#
# They did not. bin/ayeaye's voice_available() looks for VOICE_WHISPER_MODEL
# and nothing in setup had ever written it, with two consequences: the
# transcription service in lib/steps/70-service.sh refused to install itself
# on every machine there is, because it will not write a definition without a
# model named; and the app's own fallback route could only ever look at a
# default path setup had never put a file at.
#
# So this file drives a real install and then asks the app's own probe what it
# makes of the machine that install left behind. It is the only assertion in
# the suite that both halves of the handshake are in the same room.

setup() {
  stub_real grep cut wc
  require_host_command python3
  stub_command tmux
  stub_real python3

  local nowhere="$TEST_TMPDIR/nowhere"
  export HW_MEMINFO_FILE="$nowhere/meminfo"
  export HW_CPUINFO_FILE="$nowhere/cpuinfo"
  export HW_ROUTE_FILE="$nowhere/route"
  export HW_DOCKERENV_FILE="$nowhere/.dockerenv"
  export HW_CONTAINERENV_FILE="$nowhere/.containerenv"
  export HW_PROC1_CGROUP_FILE="$nowhere/proc1-cgroup"
  export HW_CGROUP_MEM_MAX_FILE="$nowhere/memory.max"
  export HW_CGROUP_MEM_LIMIT_FILE="$nowhere/memory.limit_in_bytes"
  export HW_CGROUP_CPU_MAX_FILE="$nowhere/cpu.max"
  export HW_CGROUP_CPU_QUOTA_FILE="$nowhere/cpu.cfs_quota_us"
  export HW_CGROUP_CPU_PERIOD_FILE="$nowhere/cpu.cfs_period_us"
  export HW_ROUTE6_FILE="$nowhere/ipv6_route"
  export HW_SYSTEMD_CONTAINER_FILE="$nowhere/systemd-container"
  export HW_MOUNTINFO_FILE="$nowhere/mountinfo"
  export HW_PROC_SELF_CGROUP_FILE="$nowhere/self-cgroup"
  export HW_CGROUP_ROOT="$nowhere/cgroup"
  unset container
  export HW_MODEL_DIR="$TEST_TMPDIR/models"
  export PLATFORM_OS_RELEASE_FILES=""
}

TINY_BYTES="77704715"

_state() { printf '%s' "$XDG_STATE_HOME/ayeaye/setup-state"; }
_env()   { printf '%s' "$XDG_CONFIG_HOME/ayeaye/env"; }
_model() { printf '%s' "$HW_MODEL_DIR/ggml-tiny.en.bin"; }

_file_from() {
  assert_fixture_exists "$1"
  fixture_file "$1"
}

# The whisper this machine has is the command-line one, which is the case the
# app's fallback route is for: no server is listening in a sandbox, so this is
# the route the probe has to take.
_ready_machine() {
  HW_MEMINFO_FILE="$(_file_from meminfo/64gb)"
  HW_ROUTE_FILE="$(_file_from route/default)"
  mkdir -p "$HW_MODEL_DIR"
  stub_command_from_fixture lscpu lscpu/x86_64-8core
  stub_command_from_fixture df df/roomy
  stub_command_from_fixture nvidia-smi nvidia-smi/rtx-4090
  stub_command whisper-cpp
  stub_command ffmpeg
}

_curl_writes() {
  printf '%s' "$1" > "$TEST_TMPDIR/model-size"
  stub_script curl <<'SH'
dest=""
prev=""
for arg in "$@"; do
  [ "$prev" = "-o" ] && dest="$arg"
  prev="$arg"
done
[ -n "$dest" ] || exit 1
size="$(cat "$TEST_TMPDIR/model-size")"
python3 -c 'import sys; open(sys.argv[1], "wb").truncate(int(sys.argv[2]))' \
  "$dest" "$size"
SH
}

_config_prompts() {
  pty_expect "bind address" ""
  pty_expect "port [" ""
  pty_expect "allowed hosts" ""
  pty_expect "ntfy topic URL" ""
}

# probe - what bin/ayeaye's own voice_available() says, reading the settings
# file this run wrote, the way the service reads it at boot.
# Port 9 is discard, and nothing in a sandbox listens on it. Pinning it is
# what stops a transcription server running on the developer's own machine
# from answering the probe on the fallback route's behalf.
probe() {
  run_script "$TESTS_DIR/lib/voice_probe.py" voice \
    --env "$(_env)" --server "127.0.0.1:9"
}

_install_choosing() {
  _curl_writes "$TINY_BYTES"
  _config_prompts
  pty_expect "which one?" "2"
  pty_expect "install Claude Code?" "n"
  pty_expect "go ahead with all of that?" "y"
  pty_expect "may I download the listening model?" "$1"
  pty_install --no-systemd
}

# ===================================================== the two directions

test_the_app_reports_voice_available_after_an_install_that_worked() {
  _ready_machine
  _install_choosing "y"
  assert_status 0 "$PTY_STATUS"
  assert_file_exists "$(_model)"

  probe
  assert_status 0 "$RUN_STATUS"
  assert_contains "$RUN_STDOUT" "voice=available" \
    "the installer put a model on this computer and the app can see it"
  assert_contains "$RUN_STDOUT" "model=$(_model)" \
    "and it is the file that was really fetched, not a default nobody wrote to"
  assert_contains "$RUN_STDOUT" "model_present=yes"
}

test_the_app_reports_text_only_after_an_install_that_was_declined() {
  _ready_machine
  _install_choosing "n"
  assert_status 0 "$PTY_STATUS"
  assert_file_missing "$(_model)"

  probe
  assert_status 0 "$RUN_STATUS"
  assert_contains "$RUN_STDOUT" "voice=text-only" \
    "nothing was installed, and the app says so rather than greying out later"
  assert_contains "$RUN_STDOUT" "model_present=no"
}

test_the_app_reports_text_only_when_it_cannot_hear_at_all() {
  # ffmpeg is the other half of the probe, and it is not optional to it.
  _ready_machine
  _install_choosing "y"
  stub_remove ffmpeg
  probe
  assert_contains "$RUN_STDOUT" "ffmpeg=no"
  assert_contains "$RUN_STDOUT" "voice=text-only"
}

# ================================================ the setting that was missing

test_setup_writes_the_setting_the_app_and_the_service_both_read() {
  _ready_machine
  _install_choosing "y"
  assert_file_contains "$(_env)" "VOICE_WHISPER_MODEL=$(_model)"
  assert_file_contains "$(_env)" "VOICE_WHISPER_THREADS=8" \
    "from the cores the hardware step measured, not from a guess"
}

test_setup_records_which_whisper_program_this_computer_actually_has() {
  # whisper.cpp renamed its binaries and both names are still out there.
  # Recording the one that is here is what stops the app running a command
  # that setup never found.
  # Anchored, because env.template ships this key commented out with an
  # example beside it: a whole-file substring match would pass on a tree where
  # nothing was ever written.
  _ready_machine
  _install_choosing "y"
  assert_eq "VOICE_WHISPER_CLI=$STUB_BIN/whisper-cpp" \
    "$(grep '^VOICE_WHISPER_CLI=' "$(_env)")" \
    "the transcriber setup found is the one the app will run"
}

test_the_transcriber_recorded_is_the_one_this_machine_really_has() {
  # The other spelling, and the one the modern builds use. Setup must record
  # what is here rather than what was here when the line was written.
  _ready_machine
  stub_remove whisper-cpp
  stub_command whisper-cli
  _install_choosing "y"
  assert_eq "VOICE_WHISPER_CLI=$STUB_BIN/whisper-cli" \
    "$(grep '^VOICE_WHISPER_CLI=' "$(_env)")"

  probe
  assert_contains "$RUN_STDOUT" "dictate_cli=$STUB_BIN/whisper-cli" \
    "and bin/voice-dictate will run that one"
  assert_contains "$RUN_STDOUT" "dictate_cli_present=yes"
}

test_a_machine_whose_whisper_has_the_other_name_is_told_the_button_stays_grey() {
  # bin/ayeaye's probe looks for the literal name whisper-cpp. A computer with
  # whisper-cli and no server transcribes perfectly and is still refused by
  # it. That file belongs to another ticket; reporting this run finished would
  # be reporting work that did not happen.
  _ready_machine
  stub_remove whisper-cpp
  stub_command whisper-cli
  _install_choosing "y"
  assert_status 0 "$PTY_STATUS" "voice is optional and does not fail the run"
  assert_contains "$PTY_TRANSCRIPT" "will still show the talk button greyed out"
  assert_file_contains "$(_state)" "step.install.voice=pending"
  assert_contains "$PTY_TRANSCRIPT" "Setting up talking out loud (not finished)"

  probe
  assert_contains "$RUN_STDOUT" "gate_cli=no"
  assert_contains "$RUN_STDOUT" "voice=text-only" \
    "which is exactly what setup said would happen"
}

test_a_machine_with_the_server_is_not_warned_about_the_button() {
  # The probe's first route is a socket and does not care what the program is
  # called, and the service stage installs that server now that the settings
  # name a model.
  _ready_machine
  stub_remove whisper-cpp
  stub_command whisper-cli
  stub_command whisper-server
  _install_choosing "y"
  assert_not_contains "$PTY_TRANSCRIPT" "will still show the talk button greyed out"
  assert_file_contains "$(_state)" "step.install.voice=done"
}

test_the_settings_the_user_already_had_come_through_untouched() {
  _ready_machine
  _install_choosing "y"
  assert_file_contains "$(_env)" "AYEAYE_PORT=8911"
  assert_file_contains "$(_env)" "AYEAYE_BIND=127.0.0.1"
}

# =================================== the service that could never install

# The service step is driven directly here rather than through a run: a
# sandbox has no user session bus, so `--no-systemd` is the only way the
# installer finishes, and that is exactly the flag that skips the stage under
# test. Sourcing it and calling the one function keeps the seam honest without
# pretending there is a systemd here.
_service_layer() {
  WIZARD_STATE_DIR="$XDG_STATE_HOME/ayeaye"
  WIZARD_STATE_FILE="$WIZARD_STATE_DIR/setup-state"
  WIZARD_LOG_FILE="$WIZARD_STATE_DIR/setup.log"
  ENV_FILE="$(_env)"
  SERVICE_WHISPER_BIN="$STUB_BIN/whisper-server"
  . "$REPO_ROOT/lib/wizard.sh"
  wizard_stage configure "configuration"
  wizard_stage service   "service"
  . "$REPO_ROOT/lib/steps/70-service.sh"
}

test_the_transcription_service_can_now_be_written_because_a_model_is_named() {
  # lib/steps/70-service.sh refuses to write a whisper definition unless the
  # settings name a model. Before this ticket nothing wrote that setting, so
  # it refused on every machine there is, and the message it printed instead
  # is the proof it never got past that check.
  _ready_machine
  stub_command whisper-server
  _install_choosing "y"
  assert_file_contains "$(_env)" "VOICE_WHISPER_MODEL=$(_model)"

  _service_layer
  local out
  out="$(_service_install_whisper systemd 2>&1)"
  assert_not_contains "$out" "your settings do not say where the"
  assert_file_exists "$XDG_CONFIG_HOME/systemd/user/whisper-server.service"
  assert_file_contains "$XDG_CONFIG_HOME/systemd/user/whisper-server.service" \
    "VOICE_WHISPER_MODEL"
}

test_without_a_model_the_service_still_says_what_it_is_waiting_for() {
  _ready_machine
  stub_command whisper-server
  _install_choosing "n"

  _service_layer
  local out
  out="$(_service_install_whisper systemd 2>&1)"
  assert_contains "$out" "your settings do not say where the"
  assert_file_missing "$XDG_CONFIG_HOME/systemd/user/whisper-server.service"
}

# ============================================ what the client device is told

test_the_client_instructions_name_nothing_setup_does_not_create() {
  # voice-dictate-setup used to tell people to copy a voice-agent unit off the
  # server. Setup does not write one - voice-agent belongs on the device with
  # the microphone - so the instruction sent them looking for a file that was
  # never there.
  local text
  text="$(cat "$REPO_ROOT/bin/voice-dictate-setup")"
  assert_not_contains "$text" "copy the systemd unit from"
  assert_contains "$text" "There is no unit to copy from"
  assert_contains "$text" "voice-agent belongs"
}
