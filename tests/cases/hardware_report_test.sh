# What the third stage says, word for word, on three machines.
#
# The wording is the deliverable here, not a decoration on one: the person
# reading it has never used a terminal, and "16 GB, no CUDA device" is not an
# answer to the question they are actually asking, which is whether this thing
# will work for them. So the sentences are asserted rather than described, and
# a wording regression shows up as a failing test instead of as a support
# thread six months later.
#
# The rule the assertions at the bottom enforce is narrower than it first
# looks, and deliberately so: **no raw measurement reaches the screen**. Not a
# unit, not a byte count, not a figure any probe read. Command names do appear,
# in brackets after a plain-language description of what each one is for, and
# that is the toolbox step's job rather than this one's.
#
# The other thing every assertion here is written against: every sentence in
# this report is in the conditional. None of it is installed, the stage above
# has just finished listing what is missing, and "this computer would be able
# to" is a description where "this computer can" would be a promise.

setup() {
  stub_real grep sed
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

_file_from() {
  assert_fixture_exists "$1"
  fixture_file "$1"
}

# A thin laptop: two gigabytes, four small cores, a nearly full disk.
_weak_laptop() {
  HW_MEMINFO_FILE="$(_file_from meminfo/2gb)"
  HW_ROUTE_FILE="$(_file_from route/default)"
  mkdir -p "$HW_MODEL_DIR"
  stub_command_from_fixture lscpu lscpu/aarch64-4core
  stub_command_from_fixture df df/tight
}

# A desktop with a large NVIDIA card in it.
_cuda_desktop() {
  HW_MEMINFO_FILE="$(_file_from meminfo/64gb)"
  HW_ROUTE_FILE="$(_file_from route/default)"
  mkdir -p "$HW_MODEL_DIR"
  stub_command_from_fixture lscpu lscpu/x86_64-8core
  stub_command_from_fixture df df/roomy
  stub_command_from_fixture nvidia-smi nvidia-smi/rtx-4090
}

# A desktop with plenty of everything and no graphics card at all.
_cpu_only_desktop() {
  HW_MEMINFO_FILE="$(_file_from meminfo/64gb)"
  HW_ROUTE_FILE="$(_file_from route/default)"
  mkdir -p "$HW_MODEL_DIR"
  stub_command_from_fixture lscpu lscpu/x86_64-8core
  stub_command_from_fixture df df/roomy
}

# The same desktop with a large AMD card in it.
_rocm_desktop() {
  _cpu_only_desktop
  stub_command_from_fixture rocminfo rocminfo/gfx1100
}

# An Apple Silicon laptop. Fixtures only: there is no Mac in this suite and
# nothing here has ever run on one.
_apple_silicon() {
  stub_command uname --exit 1 --stderr "uname: unsupported option"
  stub_when uname '-s' --stdout "Darwin"
  stub_when uname '-m' --stdout "arm64"
  stub_command_from_fixture sw_vers sw_vers/macos-15.1
  stub_command_from_fixture system_profiler system_profiler/apple-m3-24gb
  stub_command_from_fixture df df/roomy
  HW_ROUTE_FILE="$(_file_from route/default)"
  mkdir -p "$HW_MODEL_DIR"
}

# ==================================================== a weak CPU-only laptop

test_a_weak_laptop_is_told_plainly_that_voice_is_not_for_it() {
  _weak_laptop
  run_install --defaults --no-systemd
  assert_status 0 "$RUN_STATUS"
  # The whole heading, not a fragment of it: the toolbox step in the stage
  # above says "For talking to your agents out loud:", and an assertion on the
  # short form would pass with this entire step deleted.
  assert_contains "$RUN_STDOUT" "Talking to your agents out loud, once it is set up:"
  assert_contains "$RUN_STDOUT" "not as things stand."
  assert_contains "$RUN_STDOUT" \
    "This computer can still show you what your agents are doing and"
}

test_a_weak_laptop_is_told_why_and_not_only_that() {
  _weak_laptop
  run_install --defaults --no-systemd
  assert_contains "$RUN_STDOUT" "Why not more than that:"
  assert_contains "$RUN_STDOUT" "there is not much memory in this computer"
}

test_a_weak_laptop_is_not_told_about_a_graphics_card_it_will_never_use() {
  _weak_laptop
  run_install --defaults --no-systemd
  assert_not_contains "$RUN_STDOUT" "would do the listening work" \
    "there is no listening to do on this machine, so nothing does it"
}

# ========================================== a middling machine, one tier up

test_a_four_gigabyte_laptop_is_offered_the_small_model_and_told_to_expect_a_wait() {
  HW_MEMINFO_FILE="$(_file_from meminfo/4gb)"
  HW_ROUTE_FILE="$(_file_from route/default)"
  mkdir -p "$HW_MODEL_DIR"
  stub_command_from_fixture lscpu lscpu/aarch64-4core
  stub_command_from_fixture df df/roomy
  run_install --defaults --no-systemd
  assert_status 0 "$RUN_STATUS"
  assert_contains "$RUN_STDOUT" "yes, a sentence at a time."
  assert_contains "$RUN_STDOUT" \
    "This computer has room for the smallest of the listening"
  assert_contains "$RUN_STDOUT" "Expect to wait a moment after each sentence."
}

# ============================================================= a CUDA desktop

test_a_cuda_desktop_is_told_it_can_do_everything() {
  _cuda_desktop
  run_install --defaults --no-systemd
  assert_status 0 "$RUN_STATUS"
  assert_contains "$RUN_STDOUT" "yes, and with room for the best of it."
  assert_contains "$RUN_STDOUT" \
    "This computer has room to keep the biggest and most accurate"
}

test_a_cuda_desktop_is_told_which_card_would_be_doing_the_work() {
  _cuda_desktop
  run_install --defaults --no-systemd
  assert_contains "$RUN_STDOUT" \
    "The graphics card in this computer - NVIDIA GeForce RTX 4090 - would do the"
}

test_nothing_in_the_report_claims_any_of_it_is_working_yet() {
  # The stage above has just listed what is missing. This one describes what
  # would work; the difference between the two is the whole point of it being
  # a separate paragraph, and one indicative verb collapses it.
  _cuda_desktop
  run_install --defaults --no-systemd
  local report
  report="$(printf '%s\n' "$RUN_STDOUT" | sed -n '/once it is set up:/,/None of this is being installed/p')"
  assert_not_contains "$report" "This computer can keep"
  assert_not_contains "$report" "does the listening work"
  assert_contains "$report" "would do the"
  assert_contains "$report" "None of this is being installed now."
}

test_a_machine_at_its_ceiling_is_not_given_an_excuse_it_does_not_need() {
  _cuda_desktop
  run_install --defaults --no-systemd
  assert_not_contains "$RUN_STDOUT" "Why not more than that:" \
    "there is nothing to explain when the answer is the top one"
}

test_a_cpu_only_desktop_is_told_the_processor_would_do_it_and_be_slower() {
  _cpu_only_desktop
  run_install --defaults --no-systemd
  assert_contains "$RUN_STDOUT" "yes, comfortably."
  assert_contains "$RUN_STDOUT" \
    "There is no graphics card here that ayeaye can use for the"
  assert_contains "$RUN_STDOUT" "That is slower, and it works."
}

test_the_reason_is_not_repeated_when_the_paragraph_above_it_already_said_it() {
  # "There is no graphics card here..." followed by "Why not more than that:
  # there is no graphics card ayeaye can use here." is how a screen stops
  # being read.
  _cpu_only_desktop
  run_install --defaults --no-systemd
  assert_not_contains "$RUN_STDOUT" "Why not more than that: there is no graphics card"
}

test_a_card_too_small_to_use_is_named_rather_than_denied() {
  # Telling somebody who paid for a graphics card that there is no graphics
  # card is how a tool stops being believed about anything else.
  _cuda_desktop
  stub_command_from_fixture nvidia-smi nvidia-smi/gtx-1050
  run_install --defaults --no-systemd
  assert_contains "$RUN_STDOUT" \
    "There is a graphics card in this computer - NVIDIA GeForce GTX 1050 - but it"
  assert_contains "$RUN_STDOUT" \
    "is too small for the listening, so the processor would do it."
  assert_not_contains "$RUN_STDOUT" "There is no graphics card here"
}

test_a_card_whose_size_will_not_read_is_not_called_small() {
  # A card that would not answer is not a small card, and saying it is would be
  # a claim about their hardware rather than about ours.
  _cuda_desktop
  stub_command_from_fixture nvidia-smi nvidia-smi/no-size
  run_install --defaults --no-systemd
  assert_contains "$RUN_STDOUT" "will not say how big it is, so the processor would do the"
  assert_not_contains "$RUN_STDOUT" "but it is too small for the listening"
}

# ========================================================== an AMD desktop

test_an_amd_desktop_is_told_its_card_would_do_the_work() {
  _rocm_desktop
  run_install --defaults --no-systemd
  assert_status 0 "$RUN_STATUS"
  assert_contains "$RUN_STDOUT" \
    "The graphics card in this computer - AMD Radeon RX 7900 XTX - would do the"
  assert_file_contains "$XDG_STATE_HOME/ayeaye/setup-state" \
    "step.detect.hardware.acceleration=rocm"
}

test_an_amd_card_that_would_not_say_its_size_is_not_denied_either() {
  _rocm_desktop
  stub_command_from_fixture rocminfo rocminfo/gfx-without-a-size
  run_install --defaults --no-systemd
  assert_contains "$RUN_STDOUT" \
    "There is a graphics card in this computer - AMD Radeon VII - but it"
  assert_contains "$RUN_STDOUT" "will not say how big it is"
}

# ========================================================= an Apple Silicon Mac

test_an_apple_silicon_mac_is_told_its_own_graphics_would_do_the_work() {
  _apple_silicon
  run_install --defaults --no-systemd
  assert_status 0 "$RUN_STATUS"
  assert_contains "$RUN_STDOUT" "The graphics built into this Mac would do the listening work."
}

test_an_apple_silicon_mac_reaches_the_top_answer() {
  # Metal is the one acceleration branch the tier does not cap, and that is
  # deliberate rather than an oversight: an Apple machine has one pool of
  # memory shared between processor and graphics, so the memory line has
  # already asked the only size question there is to ask. An 8 GB Mac is
  # capped by that line like anything else - the test below proves it.
  _apple_silicon
  run_install --defaults --no-systemd
  assert_contains "$RUN_STDOUT" "yes, and with room for the best of it."
  assert_file_contains "$XDG_STATE_HOME/ayeaye/setup-state" \
    "step.detect.hardware.acceleration=metal"
}

test_a_small_apple_silicon_mac_is_capped_by_its_memory_like_anything_else() {
  _apple_silicon
  stub_script system_profiler <<'SH'
cat <<'OUT'
Hardware:

    Hardware Overview:

      Model Name: MacBook Air
      Chip: Apple M1
      Total Number of Cores: 8 (4 performance and 4 efficiency)
      Memory: 8 GB
OUT
SH
  run_install --defaults --no-systemd
  assert_contains "$RUN_STDOUT" "yes, comfortably."
  assert_not_contains "$RUN_STDOUT" "yes, and with room for the best of it."
  assert_contains "$RUN_STDOUT" "there is not much memory in this computer"
}

# ============================================== the machine that will not say

test_a_machine_that_says_nothing_is_told_the_answer_is_a_guess() {
  run_install --defaults --no-systemd
  assert_status 0 "$RUN_STATUS"
  assert_contains "$RUN_STDOUT" "Why not more than that:"
  assert_contains "$RUN_STDOUT" "setup could not read this computer's memory"
  assert_contains "$RUN_STDOUT" \
    "That is the least this computer could be, not what it measured." \
    "the floor is not a measurement, and must not read as one"
}

test_a_machine_sharing_itself_with_others_is_told_so_in_those_words() {
  HW_MEMINFO_FILE="$(_file_from meminfo/64gb)"
  HW_ROUTE_FILE="$(_file_from route/default)"
  mkdir -p "$HW_MODEL_DIR"
  stub_command_from_fixture lscpu lscpu/x86_64-8core
  stub_command_from_fixture df df/roomy
  HW_DOCKERENV_FILE="$TEST_TMPDIR/.dockerenv"; : > "$HW_DOCKERENV_FILE"
  run_install --defaults --no-systemd
  assert_contains "$RUN_STDOUT" "only part of this computer is yours to use"
  assert_not_contains "$RUN_STDOUT" "cgroup"
  assert_not_contains "$RUN_STDOUT" "container"
}

test_a_machine_with_no_way_out_is_warned_before_anything_is_downloaded() {
  _cpu_only_desktop
  HW_ROUTE_FILE="$(_file_from route/no-default)"
  run_install --defaults --no-systemd
  assert_contains "$RUN_STDOUT" \
    "The part that does the listening has to be downloaded first, and"
  assert_contains "$RUN_STDOUT" \
    "this computer does not seem to have a way out to the internet"
}

# ================================================== no jargon reaches the screen

# The report, on its own, as the block of text a person reads.
_report_block() {
  printf '%s\n' "$RUN_STDOUT" \
    | sed -n '/once it is set up:/,/None of this is being installed/p'
}

test_the_report_never_puts_a_raw_measurement_on_the_screen() {
  # Scoped to the block this ticket writes. install.sh's own step_report and
  # step_summary print "voice tier:" and "tier :" into the same stage and are
  # not this ticket's to change; that overlap is recorded in the Activity Log
  # as a hand-off rather than papered over with a test that cannot pass.
  _cuda_desktop
  run_install --defaults --no-systemd
  local block word
  block="$(_report_block)"
  assert_ne "" "$block" "the report block must be findable at all"
  for word in "MiB" "GiB" " GB" " kB" "VRAM" "vram" "CUDA" "cuda" "ROCm" "rocm" \
              "Metal" "metal" "cgroup" "/proc" "megabyte" "gigabyte" "tier" \
              "acceleration" "x86_64" "arm64"; do
    assert_not_contains "$block" "$word" \
      "raw specifications belong in the log, not on the screen"
  done
}

test_no_figure_this_ticket_measured_reaches_the_screen_anywhere() {
  # Not scoped: these numbers exist nowhere but in this ticket's probes, so
  # anything that leaked one leaked it from here.
  _cuda_desktop
  run_install --defaults --no-systemd
  local word
  for word in "24564" "64263" "510580" "15360" "7168"; do
    assert_not_contains "$RUN_STDOUT" "$word"
  done
}

test_the_same_raw_numbers_are_in_the_log_for_whoever_wants_them() {
  _cuda_desktop
  run_install --defaults --no-systemd
  local log="$XDG_STATE_HOME/ayeaye/setup.log"
  assert_file_contains "$log" "accel=cuda"
  assert_file_contains "$log" "vram=24564MB"
  assert_file_contains "$log" "ram=64263MB"
  assert_file_contains "$log" "tier=maximum"
}

test_asking_for_details_puts_them_on_the_screen_as_well() {
  _cuda_desktop
  run_install --defaults --no-systemd --details
  assert_contains "$RUN_STDOUT" "accel=cuda"
  assert_contains "$RUN_STDOUT" "tier=maximum"
}
