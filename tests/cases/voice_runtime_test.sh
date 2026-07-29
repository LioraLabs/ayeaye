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

test_a_machine_whose_whisper_has_the_other_name_can_talk() {
  # The other half of the one-name-per-tool mistake, and the direction this
  # assertion used to run in. bin/ayeaye's probe looked for the literal name
  # whisper-cpp, so a computer carrying whisper-cli - which is what the
  # current builds, Homebrew's formula and Arch's package all install -
  # transcribed perfectly and was refused by the gate anyway. Setup said so
  # out loud and reported itself unfinished, which was the honest thing to do
  # about a file it could not change. The probe now asks for the transcriber
  # this machine really has.
  _ready_machine
  stub_remove whisper-cpp
  stub_command whisper-cli
  _install_choosing "y"
  assert_status 0 "$PTY_STATUS" "voice is optional and does not fail the run"
  # One wizard_say is one line: a needle spanning two of them can never match,
  # whether the branch fired or not.
  assert_not_contains "$PTY_TRANSCRIPT" "button greyed out for now"
  assert_file_contains "$(_state)" "step.install.voice=done"

  probe
  assert_contains "$RUN_STDOUT" "gate_cli=yes" \
    "the gate and the pipeline are one question now, not two"
  assert_contains "$RUN_STDOUT" "voice=available" \
    "which is what a computer with a working transcriber should be told"
}

test_a_machine_with_the_older_spelling_can_still_talk() {
  # Both names are still in the world, and the older one must not be traded
  # away for the newer one - that would be the same bug facing the other way.
  _ready_machine
  _install_choosing "y"
  assert_file_contains "$(_state)" "step.install.voice=done"

  probe
  assert_contains "$RUN_STDOUT" "dictate_cli=$STUB_BIN/whisper-cpp"
  assert_contains "$RUN_STDOUT" "gate_cli=yes"
  assert_contains "$RUN_STDOUT" "voice=available"
}

test_a_transcriber_that_is_not_on_the_path_at_all_satisfies_the_gate() {
  # VOICE_WHISPER_CLI is a path, not a name - it is what `command -v` returned
  # when setup looked - and somebody who builds whisper.cpp themselves points
  # it somewhere that was never on PATH. A gate that only understood names
  # would refuse exactly the machine that was configured most deliberately.
  _ready_machine
  _install_choosing "y"
  mkdir -p "$TEST_TMPDIR/opt"
  printf '#!/bin/sh\nexit 0\n' > "$TEST_TMPDIR/opt/whisper-cli"
  chmod +x "$TEST_TMPDIR/opt/whisper-cli"
  # Rewritten in the settings file rather than exported: that is where the app
  # reads it from, and the file is what setup wrote a moment ago.
  grep -v '^VOICE_WHISPER_CLI=' "$(_env)" > "$TEST_TMPDIR/env.new"
  printf 'VOICE_WHISPER_CLI=%s\n' "$TEST_TMPDIR/opt/whisper-cli" >> "$TEST_TMPDIR/env.new"
  mv "$TEST_TMPDIR/env.new" "$(_env)"
  stub_remove whisper-cpp

  probe
  assert_contains "$RUN_STDOUT" "dictate_cli=$TEST_TMPDIR/opt/whisper-cli"
  assert_contains "$RUN_STDOUT" "gate_cli=yes" \
    "an absolute path is a command shutil.which can answer for"
  assert_contains "$RUN_STDOUT" "voice=available"
}

test_a_transcriber_installed_after_the_app_started_is_noticed_without_a_restart() {
  # Setup tells people ayeaye notices each of these the moment it is installed,
  # and for the whisper *server* that has always been true - the probe opens a
  # socket every VOICE_PROBE_TTL seconds and does not care what the program
  # behind it is called. For the command-line route it was not: the name was
  # resolved once, when bin/voice-dictate was imported, which for a server that
  # has been up for a fortnight means the answer it got at boot. A machine with
  # no transcriber at boot would keep its talk button greyed out until somebody
  # thought to restart it, on a computer that could transcribe perfectly.
  #
  # live_cli is the whole assertion: what the gate resolves *now*, as against
  # the imported_cli the module settled on when it was loaded.
  _ready_machine
  _install_choosing "y"
  assert_file_contains "$(_state)" "step.install.voice=done"

  # As it was at boot: nothing on PATH, and the settings file naming nothing.
  stub_remove whisper-cpp
  grep -v '^VOICE_WHISPER_CLI=' "$(_env)" > "$TEST_TMPDIR/env.new"
  mv "$TEST_TMPDIR/env.new" "$(_env)"
  probe
  assert_contains "$RUN_STDOUT" "gate_cli=no" \
    "with no transcriber anywhere, the gate is shut"

  # And now one arrives, without anything being restarted.
  stub_command whisper-cli
  probe
  assert_contains "$RUN_STDOUT" "live_cli=whisper-cli" \
    "the gate asks the question again rather than remembering the answer"
  assert_contains "$RUN_STDOUT" "gate_cli=yes"
  assert_contains "$RUN_STDOUT" "voice=available" \
    "which is what makes 'ayeaye notices it the moment it is installed' true"
}

# _voice_layer - the voice step's own functions, without an installer around
# them. Same shape as _service_layer below and for the same reason: the case
# under test is one the pty cannot set up.
_voice_layer() {
  WIZARD_STATE_DIR="$XDG_STATE_HOME/ayeaye"
  WIZARD_STATE_FILE="$WIZARD_STATE_DIR/setup-state"
  WIZARD_LOG_FILE="$WIZARD_STATE_DIR/setup.log"
  ENV_FILE="$(_env)"
  mkdir -p "$HW_MODEL_DIR" "$(dirname "$(_env)")"
  . "$REPO_ROOT/lib/wizard.sh"
  wizard_stage detect    "detect"
  wizard_stage configure "configuration"
  wizard_stage install   "install"
  # 40-voice asks the hardware step what this computer is, so the hardware
  # step has to be in the room. Same order install.sh sources them in.
  . "$REPO_ROOT/lib/steps/20-hardware.sh"
  . "$REPO_ROOT/lib/steps/40-voice.sh"
}

# ============================== the branch that says the work is not finished
#
# The mismatch _voice_talk_button_missing was written for is gone, so the
# question worth answering is whether the branch it guards can still be
# reached. It can, and these say by what. A step that can never report itself
# unfinished is as much a defect as one that always reports itself done: the
# closing checklist would go quiet on a machine that cannot talk.
#
# Driven by sourcing the layer rather than through a run, for the same reason
# the service tests below are: the reachable case needs the machine to change
# between two points of one installer run, which no pty can arrange.

test_a_machine_with_no_transcriber_left_is_told_the_button_stays_grey() {
  # How it is reached: _voice_programs is satisfied by the whisper the detect
  # stage recorded, and on a resumed run that is a fact about the machine as
  # it was. Remove the program in between and the installer has to notice.
  _voice_layer
  : > "$HW_MODEL_DIR/ggml-tiny.en.bin"
  stub_command ffmpeg
  stub_command whisper-cli
  assert_eq "" "$(_voice_talk_button_missing tiny.en)" \
    "with a transcriber and a model here there is nothing missing"

  stub_remove whisper-cli
  assert_eq "transcriber" "$(_voice_talk_button_missing tiny.en)"
}

test_a_settings_file_pointing_at_no_model_is_not_a_working_button() {
  # The other half of the app's fallback route, and why the check reads the
  # file rather than trusting that a download happened.
  _voice_layer
  stub_command ffmpeg
  stub_command whisper-cli
  assert_eq "model" "$(_voice_talk_button_missing tiny.en)"
}

test_a_whisper_server_satisfies_the_gate_on_its_own() {
  # The probe's first route is a socket: it does not care what the program is
  # called and it does not read a model file. A model is deliberately absent
  # here, to pin that the server route does not ask for one.
  #
  # What this counts is the program being installed, not a port answering -
  # the service stage that starts it has not run yet. That is the same signal
  # install.sh's tier line and _voice_programs both use, and the honest name
  # for it is a forecast.
  _voice_layer
  stub_command ffmpeg
  stub_command whisper-server
  assert_eq "" "$(_voice_talk_button_missing tiny.en)"
}

test_a_transcriber_named_in_the_settings_counts_even_if_it_is_off_the_path() {
  # The app reads VOICE_WHISPER_CLI first, so this has to as well. Somebody
  # who built whisper.cpp themselves points that at a path that was never on
  # PATH, and telling them they have no transcriber would be the same false
  # refusal this ticket removed.
  _voice_layer
  : > "$HW_MODEL_DIR/ggml-tiny.en.bin"
  stub_command ffmpeg
  mkdir -p "$TEST_TMPDIR/opt"
  printf '#!/bin/sh\nexit 0\n' > "$TEST_TMPDIR/opt/whisper-cli"
  chmod +x "$TEST_TMPDIR/opt/whisper-cli"
  mkdir -p "$(dirname "$(_env)")"
  printf 'VOICE_WHISPER_CLI=%s\n' "$TEST_TMPDIR/opt/whisper-cli" > "$(_env)"
  assert_eq "" "$(_voice_talk_button_missing tiny.en)"

  # And a setting pointing at something that is not there falls through to
  # PATH rather than being taken on trust.
  printf 'VOICE_WHISPER_CLI=%s\n' "$TEST_TMPDIR/opt/gone" > "$(_env)"
  assert_eq "transcriber" "$(_voice_talk_button_missing tiny.en)"
}

test_the_step_itself_reports_pending_and_says_which_thing_is_missing() {
  # The branch end to end, one level down from a pty run - because the state
  # that reaches it cannot be built inside a single installer run. A previous
  # run recorded the whisper this machine had; the machine no longer has it;
  # the model is already fetched and verified, so everything ahead of the
  # check passes and the check is what decides.
  #
  # Asserted on this step's own return value and its own words. The run's
  # closing "worth coming back to" block is not evidence of anything: the
  # hardware step reports an unread measurement as unfinished in a sandbox
  # whatever this step does.
  _voice_layer
  stub_command ffmpeg
  stub_command curl
  # A model that verifies. Nothing here can hash it - PATH is stubs only - so
  # _voice_verify falls back to the published size, which is what this is.
  python3 -c 'import sys; open(sys.argv[1], "wb").truncate(int(sys.argv[2]))' \
    "$(_model)" "$TINY_BYTES"
  : > "$(_env)"
  wizard_remember answer.voice.model tiny.en
  # What the detect stage wrote when this computer still had a transcriber.
  wizard_remember step.detect.tools.whisper_command whisper-cli

  local out status
  out="$(_voice_install_step 2>&1)"
  status=$?
  assert_eq "$WIZARD_STAGE_PENDING" "$status" \
    "the work did not happen, so the step must not say it did: $out"
  assert_contains "$out" "button greyed out for now"
  assert_contains "$out" "a program that turns speech into words"
  assert_file_contains "$WIZARD_LOG_FILE" \
    "voice: bin/ayeaye's probe will not find: transcriber"
}

test_the_step_reports_ok_when_the_app_will_find_everything() {
  # The same run with the transcriber still here, so that the test above is
  # pinning the check rather than some other thing going wrong on the way.
  _voice_layer
  stub_command ffmpeg
  stub_command whisper-cli
  stub_command curl
  python3 -c 'import sys; open(sys.argv[1], "wb").truncate(int(sys.argv[2]))' \
    "$(_model)" "$TINY_BYTES"
  : > "$(_env)"
  wizard_remember answer.voice.model tiny.en
  wizard_remember step.detect.tools.whisper_command whisper-cli

  local out status
  out="$(_voice_install_step 2>&1)"
  status=$?
  assert_eq "$WIZARD_STAGE_OK" "$status" "$out"
  assert_not_contains "$out" "button greyed out for now"
}

test_without_ffmpeg_there_is_nothing_else_worth_saying() {
  _voice_layer
  stub_command whisper-cli
  assert_eq "ffmpeg" "$(_voice_talk_button_missing tiny.en)"
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
  assert_not_contains "$PTY_TRANSCRIPT" "button greyed out for now"
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
