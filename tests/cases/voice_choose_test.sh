# The question: which experience is offered on which machine, what a machine
# is told about the ones it cannot have, and the whole cost of the one it
# picks - all of it before a single byte moves.
#
# Driven by running the whole installer, because a step is registered rather
# than called. The machine is described with the same fixtures the hardware
# ticket measures, so "which presets does a 4 GB laptop get" is a question this
# file can ask without a 4 GB laptop.

setup() {
  stub_real grep cut
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

_state() { printf '%s' "$XDG_STATE_HOME/ayeaye/setup-state"; }

_file_from() {
  assert_fixture_exists "$1"
  fixture_file "$1"
}

# whisper.cpp is already on this computer, which is what makes any of the
# listening presets offerable at all.
_whisper_present() { stub_command whisper-server; }

# A desktop with a large NVIDIA card, plenty of memory and plenty of room.
_cuda_desktop() {
  HW_MEMINFO_FILE="$(_file_from meminfo/64gb)"
  HW_ROUTE_FILE="$(_file_from route/default)"
  mkdir -p "$HW_MODEL_DIR"
  stub_command_from_fixture lscpu lscpu/x86_64-8core
  stub_command_from_fixture df df/roomy
  stub_command_from_fixture nvidia-smi nvidia-smi/rtx-4090
  _whisper_present
}

# Sixteen gigabytes, eight cores, room to spare, and no graphics card at all -
# the processor-only machine the requirement says must keep its option.
_cpu_only_workstation() {
  HW_MEMINFO_FILE="$(_file_from meminfo/16gb)"
  HW_ROUTE_FILE="$(_file_from route/default)"
  mkdir -p "$HW_MODEL_DIR"
  stub_command_from_fixture lscpu lscpu/x86_64-8core
  stub_command_from_fixture df df/roomy
  _whisper_present
}

# Four gigabytes: enough to listen a little, not enough for the rest.
_small_laptop() {
  HW_MEMINFO_FILE="$(_file_from meminfo/4gb)"
  HW_ROUTE_FILE="$(_file_from route/default)"
  mkdir -p "$HW_MODEL_DIR"
  stub_command_from_fixture lscpu lscpu/x86_64-8core
  stub_command_from_fixture df df/roomy
  _whisper_present
}

# Two gigabytes and a tight disk: measured as text-only by the hardware step.
_tiny_board() {
  HW_MEMINFO_FILE="$(_file_from meminfo/2gb)"
  HW_ROUTE_FILE="$(_file_from route/default)"
  mkdir -p "$HW_MODEL_DIR"
  stub_command_from_fixture lscpu lscpu/aarch64-4core
  stub_command_from_fixture df df/tight
  _whisper_present
}

# The four questions install.sh asks before this file gets a turn.
_answer_config_prompts() {
  pty_expect "bind address" ""
  pty_expect "port [" ""
  pty_expect "allowed hosts" ""
  pty_expect "ntfy topic URL" ""
}

# =========================================================== who gets what

test_a_machine_with_room_for_everything_is_offered_everything() {
  _cuda_desktop
  run_install --defaults --no-systemd
  assert_status 0 "$RUN_STATUS"
  assert_contains "$RUN_STDOUT" "type to your agents, and download nothing"
  assert_contains "$RUN_STDOUT" "talk to your agents, with a small quick listener"
  assert_contains "$RUN_STDOUT" "talk to your agents, and have what you said tidied up"
  assert_contains "$RUN_STDOUT" "the most accurate listening this computer has room for"
  assert_contains "$RUN_STDOUT" "choose the listening model yourself"
  assert_not_contains "$RUN_STDOUT" "not offered here" \
    "nothing is out of reach on a machine like this"
}

test_a_small_laptop_is_told_which_experiences_it_cannot_have_and_why() {
  _small_laptop
  run_install --defaults --no-systemd
  assert_status 0 "$RUN_STATUS"
  assert_file_contains "$(_state)" "step.detect.hardware.tier=lightweight"
  assert_contains "$RUN_STDOUT" "talk to your agents, with a small quick listener (about 78 MB)"
  assert_contains "$RUN_STDOUT" \
    "talk to your agents, and have what you said tidied up - not offered here: there is not much memory in this computer" \
    "an option that is simply absent reads as a limitation of the software"
  assert_contains "$RUN_STDOUT" \
    "the most accurate listening this computer has room for - not offered here:"
}

test_a_machine_measured_as_text_only_is_not_asked_a_question_it_has_no_answers_for() {
  _tiny_board
  run_install --defaults --no-systemd
  assert_status 0 "$RUN_STATUS"
  assert_file_contains "$(_state)" "step.detect.hardware.tier=text-only"
  assert_contains "$RUN_STDOUT" "So this computer will be set up for typing"
  assert_contains "$RUN_STDOUT" "Nothing will be downloaded."
  assert_file_contains "$(_state)" "step.configure.voice=skipped"
  assert_file_contains "$(_state)" "answer.voice.preset=text-only"
}

test_a_computer_with_no_way_to_run_a_model_is_told_so_rather_than_offered_one() {
  # No whisper stub and no package for it on this family: the presets that
  # need one are named, blocked, and explained, and the model is never
  # offered to a machine with nothing to run it.
  _cuda_desktop
  stub_remove whisper-server
  run_install --defaults --no-systemd
  assert_status 0 "$RUN_STATUS"
  assert_contains "$RUN_STDOUT" \
    "not offered here: nothing on this computer can turn speech into words yet"
  assert_file_contains "$(_state)" "step.configure.voice=skipped"
}

# ================================================ choosing to type is fine

test_choosing_to_type_is_finished_business_and_not_unfinished_work() {
  # SKIP, not PENDING. "The user said no" is a completed conversation, and a
  # closing checklist that nags about it would be nagging about a decision.
  _cuda_desktop
  run_install --defaults --no-systemd
  assert_status 0 "$RUN_STATUS"
  assert_file_contains "$(_state)" "step.configure.voice=skipped"
  assert_not_contains "$RUN_STDOUT" "Choosing how you talk to ayeaye (not finished)"
}

test_an_unattended_run_never_downloads_a_gigabyte_on_its_own() {
  # --defaults means take the defaults, and a default that quietly fetches a
  # model is not a default.
  #
  # curl is stubbed here rather than left absent on purpose: a run that could
  # have fetched and did not is the thing being pinned, and a run with no
  # fetcher on the machine proves only that the machine had no fetcher.
  _cuda_desktop
  stub_command curl
  run_install --defaults --no-systemd
  assert_status 0 "$RUN_STATUS"
  assert_stub_not_called curl "nothing was fetched"
  # An exact line, because the state file is matched by substring and
  # "answer.voice.model=" is a prefix of every value this key can hold.
  assert_eq "answer.voice.model=" \
    "$(grep '^answer.voice.model=' "$(_state)")" \
    "no model was chosen at all, rather than one that starts with nothing"
  assert_not_contains "$RUN_STDOUT" "a listening model, about"
  assert_file_missing "$HW_MODEL_DIR/ggml-large-v3-turbo.bin"
}

test_a_remembered_answer_is_only_offered_again_while_it_is_still_on_offer() {
  # The trap this closes: a computer that had room for the largest listener
  # last month, was picked up again after its disk filled, and was handed its
  # own old answer back as the default - printed as unavailable two lines
  # above, and taken without a word if the question went unanswered.
  _small_laptop
  mkdir -p "$XDG_STATE_HOME/ayeaye"
  printf 'answer.voice.preset=maximum\n' >> "$(_state)"
  _answer_config_prompts
  pty_expect "which one?" ""
  pty_expect "go ahead with all of that?" "n"
  pty_install --no-systemd

  assert_status 0 "$PTY_STATUS"
  assert_contains "$PTY_TRANSCRIPT" \
    "the most accurate listening this computer has room for - not offered here:"
  assert_not_contains "$PTY_TRANSCRIPT" "taking the one offered" \
    "the question had a default this machine can actually have"
  assert_ne "maximum" "$(grep '^answer.voice.preset=' "$(_state)" | tail -1 | cut -d= -f2)" \
    "and last month's answer was not quietly acted on"
  assert_not_contains "$PTY_TRANSCRIPT" "1625 MB"
}

test_an_answer_that_is_never_given_falls_back_to_something_on_offer() {
  # Three unparsed answers, and what is taken must be in the list that was
  # printed. Anything else would be selecting a blocked preset in silence.
  _small_laptop
  _answer_config_prompts
  pty_expect "which one?" "9"
  pty_expect "not one of the numbers above" "9"
  pty_expect "not one of the numbers above" "9"
  pty_expect "go ahead with all of that?" "n"
  pty_install --no-systemd

  assert_status 0 "$PTY_STATUS"
  assert_contains "$PTY_TRANSCRIPT" "taking the one offered"
  assert_file_contains "$(_state)" "answer.voice.preset=lightweight" \
    "the default this machine was measured for, and nothing above it"
  assert_not_contains "$PTY_TRANSCRIPT" "1625 MB"
}

# ============================================ the cost, before the asking

test_the_whole_cost_is_shown_before_anything_is_asked_for() {
  _cuda_desktop
  _answer_config_prompts
  pty_expect "which one?" "3"
  pty_expect "go ahead with all of that?" "n"
  pty_install --no-systemd
  assert_status 0 "$PTY_STATUS"
  assert_contains "$PTY_TRANSCRIPT" "Before anything is downloaded, here is the whole of it:"
  assert_contains "$PTY_TRANSCRIPT" "download about 488 MB, once"
  assert_contains "$PTY_TRANSCRIPT" "disk     about 488 MB kept on this computer"
  assert_contains "$PTY_TRANSCRIPT" "memory   about 1000 MB while it is listening"
  assert_contains "$PTY_TRANSCRIPT" "using    your NVIDIA graphics card"
  assert_contains "$PTY_TRANSCRIPT" "from     huggingface.co"
  assert_contains "$PTY_TRANSCRIPT" "remove   delete $HW_MODEL_DIR/ggml-small.en.bin"
  assert_contains "$PTY_TRANSCRIPT" "change   run this setup again and pick a different one"
}

test_the_plan_names_the_download_and_its_size_before_the_install_stage_runs() {
  _cuda_desktop
  _answer_config_prompts
  pty_expect "which one?" "2"
  pty_expect "go ahead with all of that?" "n"
  pty_install --no-systemd
  assert_status 0 "$PTY_STATUS"
  assert_contains "$PTY_TRANSCRIPT" "a listening model, about 78 MB, from huggingface.co"
  assert_contains "$PTY_TRANSCRIPT" "where the listening model is, added to"
}

test_the_raw_size_and_the_model_name_go_to_the_log_and_not_to_the_screen() {
  _cuda_desktop
  _answer_config_prompts
  pty_expect "which one?" "2"
  pty_expect "go ahead with all of that?" "n"
  pty_install --no-systemd
  assert_not_contains "$PTY_TRANSCRIPT" "77704715" "a byte count is not a thing to read"
  assert_not_contains "$PTY_TRANSCRIPT" "921e4cf8" "nor is a checksum"
  assert_file_contains "$XDG_STATE_HOME/ayeaye/setup.log" "voice: model tiny.en, 77704715 bytes"
  assert_file_contains "$XDG_STATE_HOME/ayeaye/setup.log" \
    "ggml-tiny.en.bin -> $HW_MODEL_DIR/ggml-tiny.en.bin"
}

# =========================================== a blocked option is not a number

test_an_option_this_computer_cannot_have_cannot_be_typed_by_accident() {
  # The blocked presets are shown and explained, and they carry no number, so
  # the number that would have selected one selects nothing instead.
  _small_laptop
  _answer_config_prompts
  pty_expect "which one?" "4"
  pty_expect "not one of the numbers above" "2"
  pty_expect "go ahead with all of that?" "n"
  pty_install --no-systemd
  assert_status 0 "$PTY_STATUS"
  assert_contains "$PTY_TRANSCRIPT" "that is not one of the numbers above."
  assert_file_contains "$(_state)" "answer.voice.preset=lightweight"
  assert_file_contains "$(_state)" "answer.voice.model=tiny.en"
}

# ======================================================= choosing your own

test_choosing_a_model_yourself_warns_before_going_above_the_measurement() {
  _small_laptop
  _answer_config_prompts
  pty_expect "which one?" "3"
  pty_expect "the smallest one there is" "5"
  pty_expect "use it anyway?" "y"
  pty_expect "go ahead with all of that?" "n"
  pty_install --no-systemd
  assert_status 0 "$PTY_STATUS"
  assert_contains "$PTY_TRANSCRIPT" "That is larger than this computer was measured for."
  assert_contains "$PTY_TRANSCRIPT" "there is not much memory in this computer"
  assert_contains "$PTY_TRANSCRIPT" "it may run out of memory and stop"
  assert_file_contains "$(_state)" "answer.voice.preset=custom"
  assert_file_contains "$(_state)" "answer.voice.model=large-v3-turbo"
  assert_file_contains "$(_state)" "answer.voice.override=1"
  assert_contains "$PTY_TRANSCRIPT" "Before anything is downloaded, here is the whole of it:" \
    "an override is still shown what it costs"
  assert_contains "$PTY_TRANSCRIPT" "download about 1625 MB, once"
}

test_a_choice_within_the_measurement_is_not_recorded_as_an_override() {
  # The flag belongs to the answer that was overridden. Left behind from a
  # previous run it would say something untrue about this one.
  _small_laptop
  mkdir -p "$XDG_STATE_HOME/ayeaye"
  printf 'answer.voice.override=1\n' >> "$(_state)"
  _answer_config_prompts
  pty_expect "which one?" "2"
  pty_expect "go ahead with all of that?" "n"
  pty_install --no-systemd
  assert_eq "answer.voice.override=0" \
    "$(grep '^answer.voice.override=' "$(_state)" | tail -1)"
}

test_declining_the_warning_leaves_nothing_chosen_and_nothing_planned() {
  _small_laptop
  _answer_config_prompts
  pty_expect "which one?" "3"
  pty_expect "the smallest one there is" "5"
  pty_expect "use it anyway?" "n"
  pty_install --no-systemd
  assert_status 0 "$PTY_STATUS"
  assert_contains "$PTY_TRANSCRIPT" "nothing has been chosen."
  assert_not_contains "$PTY_TRANSCRIPT" "go ahead with all of that?" \
    "and with nothing chosen there is nothing for the plan gate to stop for"
  assert_file_contains "$(_state)" "answer.voice.preset=text-only"
  assert_not_contains "$PTY_TRANSCRIPT" "a listening model, about 1625 MB"
}

test_choosing_your_own_is_not_offered_when_there_is_nothing_to_override() {
  # An override of a measurement is a decision. An override of "there is no
  # program to run it" is a wish, and offering it would be offering nothing.
  # It still says so: an option that simply vanishes reads as a limitation of
  # the software, which is the thing every other line here exists to avoid.
  _cuda_desktop
  stub_remove whisper-server
  run_install --defaults --no-systemd
  assert_contains "$RUN_STDOUT" \
    "choose the listening model yourself - not offered here:"
  assert_not_contains "$RUN_STDOUT" "  4        choose the listening model yourself" \
    "and it carries no number, so no number can select it"
}

# ================================================== the processor-only path

test_a_computer_with_no_graphics_card_still_gets_to_listen() {
  # A slow honest option beats an absent one.
  _cpu_only_workstation
  _answer_config_prompts
  pty_expect "which one?" "3"
  pty_expect "go ahead with all of that?" "n"
  pty_install --no-systemd
  assert_status 0 "$PTY_STATUS"
  assert_contains "$PTY_TRANSCRIPT" "using    this computer's processor"
  assert_contains "$PTY_TRANSCRIPT" \
    "speed    on this processor, expect about as long again as you spoke"
  assert_file_contains "$(_state)" "answer.voice.backend=cpu"
}

test_the_largest_model_on_a_processor_says_what_slow_really_means() {
  _cpu_only_workstation
  _answer_config_prompts
  pty_expect "which one?" "4"
  pty_expect "the smallest one there is" "4"
  pty_expect "use it anyway?" "y"
  pty_expect "go ahead with all of that?" "n"
  pty_install --no-systemd
  assert_contains "$PTY_TRANSCRIPT" \
    "expect several times as long as you spoke - minutes for a long sentence"
}
