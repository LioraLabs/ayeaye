# The two detect steps, driven the way a person drives them: by running the
# whole installer.
#
# A step is registered rather than called, so there is no way to exercise one
# except end to end. What is pinned here is the seam rather than the wording -
# hardware_report_test.sh has the wording - and the seam is the state file:
# every stage after this one, in three other tickets, reads its answer out of
# it rather than out of a variable, because a resumed run skips the step that
# set the variable.

setup() {
  stub_real grep cut
  require_host_command python3
  stub_command tmux
  stub_real python3

  # Every hardware input starts pointing at a path inside the sandbox that
  # will never exist, so a test that does not describe a machine gets a
  # machine that says nothing rather than the one running the suite.
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

_state() { printf '%s' "$XDG_STATE_HOME/ayeaye/setup-state"; }

_file_from() {
  assert_fixture_exists "$1"
  fixture_file "$1"
}

# A desktop with a large NVIDIA card, plenty of memory and plenty of room.
_cuda_desktop() {
  HW_MEMINFO_FILE="$(_file_from meminfo/64gb)"
  HW_ROUTE_FILE="$(_file_from route/default)"
  mkdir -p "$HW_MODEL_DIR"
  stub_command_from_fixture lscpu lscpu/x86_64-8core
  stub_command_from_fixture df df/roomy
  stub_command_from_fixture nvidia-smi nvidia-smi/rtx-4090
}

# A small ARM board: two gigabytes, four slow cores, a nearly full card.
_weak_laptop() {
  HW_MEMINFO_FILE="$(_file_from meminfo/2gb)"
  HW_ROUTE_FILE="$(_file_from route/default)"
  mkdir -p "$HW_MODEL_DIR"
  stub_command_from_fixture lscpu lscpu/aarch64-4core
  stub_command_from_fixture df df/tight
}

# =============================================================== the verdict

test_a_cuda_desktop_publishes_cuda_and_the_top_tier() {
  _cuda_desktop
  run_install --defaults --no-systemd
  assert_status 0 "$RUN_STATUS"
  assert_file_contains "$(_state)" "step.detect.hardware.acceleration=cuda"
  assert_file_contains "$(_state)" "step.detect.hardware.tier=maximum"
}

test_a_small_board_publishes_no_acceleration_and_the_bottom_tier() {
  _weak_laptop
  run_install --defaults --no-systemd
  assert_status 0 "$RUN_STATUS"
  assert_file_contains "$(_state)" "step.detect.hardware.acceleration=cpu"
  assert_file_contains "$(_state)" "step.detect.hardware.tier=text-only"
}

test_a_machine_that_says_nothing_about_itself_publishes_a_cautious_verdict() {
  run_install --defaults --no-systemd
  assert_status 0 "$RUN_STATUS"
  assert_file_contains "$(_state)" "step.detect.hardware.acceleration=cpu"
  assert_file_contains "$(_state)" "step.detect.hardware.tier=lightweight"
  assert_file_contains "$(_state)" "step.detect.hardware.ram_mb=unknown"
}

test_a_machine_that_could_not_be_read_at_all_is_not_recorded_as_checked() {
  # The verdict is real - it is the lowest one there is, which is what knowing
  # nothing earns - but the check did not happen, and a step that reported it
  # as done would be saying this computer was looked at when it was not.
  run_install --defaults --no-systemd
  assert_status 0 "$RUN_STATUS" "not knowing is not a reason to stop"
  assert_file_not_contains "$(_state)" "stage.detect=done"
  assert_contains "$RUN_STDOUT" "What setup could not read: memory processors free-space."
}

test_the_caveat_arrives_before_the_answers_it_applies_to() {
  # A caveat printed after the items is a caveat somebody has already acted on.
  run_install --defaults --no-systemd
  local caveat item
  caveat="$(printf '%s\n' "$RUN_STDOUT" | grep -n -F \
    "Setup could not read everything about this computer" | head -1 | cut -d: -f1)"
  item="$(printf '%s\n' "$RUN_STDOUT" | grep -n -F "hardware " | head -1 | cut -d: -f1)"
  [ -n "$caveat" ] || fail "no caveat printed:
$RUN_STDOUT"
  [ -n "$item" ] || fail "no hardware item printed:
$RUN_STDOUT"
  [ "$caveat" -lt "$item" ] || fail "the caveat came after the answer it applies to"
}

test_one_unreadable_measurement_out_of_three_is_still_a_check_that_did_not_finish() {
  # Two readable figures and one missing is not "checked". Recording it as done
  # would say this computer was looked at, and part of it was not.
  HW_MEMINFO_FILE="$(_file_from meminfo/64gb)"
  HW_ROUTE_FILE="$(_file_from route/default)"
  mkdir -p "$HW_MODEL_DIR"
  stub_command_from_fixture lscpu lscpu/x86_64-8core
  assert_command_absent df
  run_install --defaults --no-systemd
  assert_status 0 "$RUN_STATUS"
  assert_file_not_contains "$(_state)" "stage.detect=done"
  assert_contains "$RUN_STDOUT" "What setup could not read: free-space."
}

test_the_step_is_named_so_that_it_reads_under_the_unfinished_heading() {
  # A step that returns PENDING is listed in the closing summary under "not
  # finished, and worth coming back to", where its label is a sentence
  # fragment somebody has to make sense of on its own.
  run_install --defaults --no-systemd
  assert_contains "$RUN_STDOUT" "not finished, and worth coming back to"
  assert_contains "$RUN_STDOUT" "Reading this computer's size"
}

test_a_machine_that_could_be_read_is_recorded_as_checked() {
  _cuda_desktop
  run_install --defaults --no-systemd
  assert_file_contains "$(_state)" "stage.detect=done"
}

test_the_reason_the_tier_is_not_higher_is_published_too() {
  _cuda_desktop
  stub_command_from_fixture df df/eight-gigabytes
  run_install --defaults --no-systemd
  assert_file_contains "$(_state)" "step.detect.hardware.tier=recommended"
  assert_file_contains "$(_state)" "step.detect.hardware.tier_reason=there is not much free space"
  assert_file_contains "$(_state)" "step.detect.hardware.tier_cause=disk"
}

test_the_raw_measurements_are_published_for_whoever_wants_them() {
  _cuda_desktop
  run_install --defaults --no-systemd
  assert_file_contains "$(_state)" "step.detect.hardware.cores=8"
  assert_file_contains "$(_state)" "step.detect.hardware.ram_mb=64263"
  assert_file_contains "$(_state)" "step.detect.hardware.vram_mb=24564"
  assert_file_contains "$(_state)" "step.detect.hardware.gpu=NVIDIA GeForce RTX 4090"
  assert_file_contains "$(_state)" "step.detect.hardware.network=online"
  assert_file_contains "$(_state)" "step.detect.hardware.container=0"
}

test_a_card_too_small_to_be_worth_using_is_published_as_such() {
  # Still honestly cuda - the card and the driver are both there - and
  # honestly no use for holding a listening model. Publishing both facts is
  # what stops the next ticket building a CUDA whisper for two gigabytes of
  # graphics memory.
  _cuda_desktop
  stub_command_from_fixture nvidia-smi nvidia-smi/gtx-1050
  run_install --defaults --no-systemd
  assert_file_contains "$(_state)" "step.detect.hardware.acceleration=cuda"
  assert_file_contains "$(_state)" "step.detect.hardware.accel_usable=0"
  assert_file_contains "$(_state)" "step.detect.hardware.tier=recommended"
}

test_a_card_worth_using_is_published_as_such() {
  _cuda_desktop
  run_install --defaults --no-systemd
  assert_file_contains "$(_state)" "step.detect.hardware.accel_usable=1"
}

# ------------------------------------------ what a later stage actually reads

test_the_verdict_survives_into_a_shell_that_probed_nothing() {
  # This is the contract the voice ticket is written against: it runs in a
  # later stage, or in a resumed run where the detect step did not run at all,
  # and it must get the same two words without a single probe of its own.
  _cuda_desktop
  run_install --defaults --no-systemd
  assert_file_exists "$(_state)"

  stub_remove lscpu df nvidia-smi
  HW_MEMINFO_FILE="$TEST_TMPDIR/nowhere/meminfo"
  local probe="$TEST_TMPDIR/probe.sh"
  cat > "$probe" <<'SH'
set -eu
. "$REPO_ROOT/lib/wizard.sh"
wizard_stage detect "d"
wizard_stage report "r"
. "$REPO_ROOT/lib/steps/20-hardware.sh"
wizard_state_load
printf 'accel=%s\n' "$(hw_acceleration)"
printf 'tier=%s\n' "$(hw_voice_tier)"
printf 'gpu=%s\n' "$(hw_gpu_name)"
printf 'reason=[%s]\n' "$(hw_tier_reason)"
if hw_tier_at_least recommended; then printf 'at-least-recommended=yes\n'; fi
if hw_accel_usable; then printf 'usable=yes\n'; else printf 'usable=no\n'; fi
SH
  REPO_ROOT="$REPO_ROOT" run_script bash "$probe"
  assert_status 0 "$RUN_STATUS"
  assert_contains "$RUN_STDOUT" "accel=cuda"
  assert_contains "$RUN_STDOUT" "tier=maximum"
  assert_contains "$RUN_STDOUT" "at-least-recommended=yes"
  assert_contains "$RUN_STDOUT" "gpu=NVIDIA GeForce RTX 4090"
  assert_contains "$RUN_STDOUT" "usable=yes"
}

# ================================================================== the tools

test_the_tools_sweep_records_what_is_missing_as_well_as_what_is_there() {
  stub_command ffmpeg
  stub_command ollama
  run_install --defaults --no-systemd
  assert_file_contains "$(_state)" "step.detect.tools.ffmpeg=1"
  assert_file_contains "$(_state)" "step.detect.tools.ollama=1"
  assert_file_contains "$(_state)" "step.detect.tools.whisper=0"
  assert_file_contains "$(_state)" "step.detect.tools.tailscale=0"
}

test_the_optional_toolbox_is_not_shouted_at_the_way_a_missing_requirement_is() {
  # MISSING is what the stage above uses for tmux and python3, which setup
  # cannot continue without. Six more of them for things that are all optional
  # is how somebody on their first run decides the whole thing is broken.
  run_install --defaults --no-systemd
  assert_contains "$RUN_STDOUT" "not yet  recording what you say (ffmpeg)"
  assert_not_contains "$RUN_STDOUT" "MISSING  recording what you say"
  assert_contains "$RUN_STDOUT" "Nothing above is required."
}

test_whichever_whisper_is_installed_is_the_one_that_is_recorded() {
  stub_command whisper-cpp
  run_install --defaults --no-systemd
  assert_file_contains "$(_state)" "step.detect.tools.whisper=1"
  assert_file_contains "$(_state)" "step.detect.tools.whisper_command=whisper-cpp"
}

test_the_agent_commands_that_are_installed_are_listed() {
  stub_command claude
  stub_command codex
  run_install --defaults --no-systemd
  assert_file_contains "$(_state)" "step.detect.tools.agents=claude codex"
}

test_a_machine_with_no_agent_commands_on_it_says_so_rather_than_saying_nothing() {
  run_install --defaults --no-systemd
  assert_status 0 "$RUN_STATUS"
  assert_file_contains "$(_state)" "step.detect.tools.agents="
  assert_contains "$RUN_STDOUT" "no coding agent"
}

test_the_tools_sweep_runs_nothing_it_finds() {
  # `command -v` looks a command up without running it, which is the whole
  # difference between detecting ollama and starting it.
  stub_command ollama
  stub_command ffmpeg
  stub_command tailscale
  run_install --defaults --no-systemd
  assert_stub_not_called ollama
  assert_stub_not_called ffmpeg
}

# ========================================================= nothing is changed

test_detection_asks_no_questions_and_changes_nothing() {
  _cuda_desktop
  stub_command sudo
  stub_command apt-get
  run_install --defaults --no-systemd
  assert_status 0 "$RUN_STATUS"
  assert_stub_not_called sudo
  assert_stub_not_called apt-get
}

test_a_machine_that_cannot_be_read_does_not_stop_the_run() {
  # install.sh runs under `set -euo pipefail`. A probe that came back non-zero
  # from a directory it could not read would take the whole setup with it.
  run_install --defaults --no-systemd
  assert_status 0 "$RUN_STATUS"
  assert_eq "" "$RUN_STDERR"
}
