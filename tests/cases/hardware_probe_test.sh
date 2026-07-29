# Reading the machine: every probe, from a fixture, and again with nothing to
# read.
#
# Nothing here asks the machine running the suite anything. Every file-shaped
# input is pointed at a path inside the sandbox before the first test runs, and
# every command-shaped input is a stub - which `PATH` being exactly `$STUB_BIN`
# makes airtight, because an unstubbed command is genuinely absent.
#
# The half of this file that matters is the second one: the probe that finds
# nothing must answer "unknown" and never an error, and never a number it made
# up. An over-promise here becomes a download the machine cannot run.

setup() {
  WIZARD_STATE_DIR="$TEST_TMPDIR/state"
  WIZARD_STATE_FILE="$WIZARD_STATE_DIR/setup-state"
  WIZARD_LOG_FILE="$WIZARD_STATE_DIR/setup.log"

  # Nothing on this machine is readable by the code under test until a test
  # says otherwise. "nowhere" is a directory that is never created.
  local nowhere="$TEST_TMPDIR/nowhere"
  HW_MEMINFO_FILE="$nowhere/meminfo"
  HW_CPUINFO_FILE="$nowhere/cpuinfo"
  HW_ROUTE_FILE="$nowhere/route"
  HW_DOCKERENV_FILE="$nowhere/.dockerenv"
  HW_CONTAINERENV_FILE="$nowhere/.containerenv"
  HW_PROC1_CGROUP_FILE="$nowhere/proc1-cgroup"
  HW_CGROUP_MEM_MAX_FILE="$nowhere/memory.max"
  HW_CGROUP_MEM_LIMIT_FILE="$nowhere/memory.limit_in_bytes"
  HW_CGROUP_CPU_MAX_FILE="$nowhere/cpu.max"
  HW_CGROUP_CPU_QUOTA_FILE="$nowhere/cpu.cfs_quota_us"
  HW_CGROUP_CPU_PERIOD_FILE="$nowhere/cpu.cfs_period_us"
  HW_ROUTE6_FILE="$nowhere/ipv6_route"
  HW_SYSTEMD_CONTAINER_FILE="$nowhere/systemd-container"
  HW_MOUNTINFO_FILE="$nowhere/mountinfo"
  HW_PROC_SELF_CGROUP_FILE="$nowhere/self-cgroup"
  HW_CGROUP_ROOT="$nowhere/cgroup"
  unset container
  HW_MODEL_DIR="$TEST_TMPDIR/models"

  # The platform layer is a dependency, not a thing under test here: pin it to
  # a machine with no os-release and no uname rather than to the host.
  PLATFORM_OS_RELEASE_FILES=""
  export PLATFORM_OS_RELEASE_FILES

  . "$REPO_ROOT/lib/wizard.sh"
  wizard_stage detect "checking this computer"
  wizard_stage report "what ayeaye can do here"
  . "$REPO_ROOT/lib/steps/20-hardware.sh"
  wizard_state_load
  WIZARD_INTERACTIVE=0
}

_stub_uname() {
  stub_command uname --exit 1 --stderr "uname: unsupported option"
  stub_when uname '-s' --stdout "$1"
  stub_when uname '-m' --stdout "$2"
  platform_reset
}

_file_from() {
  assert_fixture_exists "$1"
  fixture_file "$1"
}

# ============================================================== core count

test_the_core_count_comes_from_lscpu_when_there_is_one() {
  stub_command_from_fixture lscpu lscpu/x86_64-8core
  assert_eq 8 "$(hw_probe_cores)"
}

test_lscpu_on_a_four_core_arm_board_reads_four_and_not_the_numa_line() {
  # lscpu prints "CPU(s):" three times: once at the left margin and twice
  # indented, for the on-line list and the NUMA node. Only the first is a count.
  stub_command_from_fixture lscpu lscpu/aarch64-4core
  assert_eq 4 "$(hw_probe_cores)"
}

test_without_lscpu_the_core_count_falls_back_to_nproc() {
  stub_command nproc --stdout "6"
  assert_command_absent lscpu
  assert_eq 6 "$(hw_probe_cores)"
}

test_without_lscpu_or_nproc_the_core_count_is_counted_out_of_cpuinfo() {
  HW_CPUINFO_FILE="$TEST_TMPDIR/cpuinfo"
  {
    printf 'processor\t: 0\nmodel name\t: A\n\n'
    printf 'processor\t: 1\nmodel name\t: A\n\n'
    printf 'processor\t: 2\nmodel name\t: A\n\n'
  } > "$HW_CPUINFO_FILE"
  assert_eq 3 "$(hw_probe_cores)"
}

test_a_mac_answers_the_core_count_through_sysctl() {
  stub_command sysctl --exit 1
  stub_when sysctl '-n hw.ncpu' --stdout "10"
  assert_eq 10 "$(hw_probe_cores)"
}

test_a_mac_with_no_sysctl_answer_reads_the_core_count_out_of_system_profiler() {
  stub_command_from_fixture system_profiler system_profiler/apple-m3-24gb
  assert_eq 8 "$(hw_probe_cores)" \
    "\"Total Number of Cores: 8 (4 performance and 4 efficiency)\" is eight"
}

test_a_machine_that_will_not_say_how_many_processors_it_has_answers_unknown() {
  assert_command_absent lscpu
  assert_command_absent nproc
  assert_eq unknown "$(hw_probe_cores)"
}

test_a_core_count_that_is_not_a_number_is_thrown_away_rather_than_believed() {
  stub_command nproc --stdout "quite a few"
  assert_eq unknown "$(hw_probe_cores)"
}

test_a_core_count_of_zero_is_thrown_away_too() {
  # Seen from a stub, from a broken container runtime, and from nproc under a
  # cpuset with nothing in it. Zero cores is not a machine.
  stub_command nproc --stdout "0"
  assert_eq unknown "$(hw_probe_cores)"
}

# ==================================================================== memory

test_memory_comes_from_proc_meminfo_in_whole_megabytes() {
  HW_MEMINFO_FILE="$(_file_from meminfo/16gb)"
  assert_eq 15846 "$(hw_probe_ram_mb)" "16226356 kB is 15846 whole megabytes"
}

test_a_two_gigabyte_board_reads_as_two_gigabytes() {
  HW_MEMINFO_FILE="$(_file_from meminfo/2gb)"
  assert_eq 1849 "$(hw_probe_ram_mb)"
}

test_a_meminfo_with_no_total_line_in_it_is_not_a_memory_figure() {
  HW_MEMINFO_FILE="$(_file_from meminfo/truncated)"
  assert_eq unknown "$(hw_probe_ram_mb)"
}

test_without_proc_meminfo_memory_falls_back_to_free() {
  stub_command_from_fixture free free/16gb
  assert_eq 15845 "$(hw_probe_ram_mb)" \
    "free -m answers in megabytes already"
}

test_a_mac_answers_its_memory_through_sysctl_in_bytes() {
  stub_command sysctl --exit 1
  stub_when sysctl '-n hw.memsize' --stdout "25769803776"
  assert_eq 24576 "$(hw_probe_ram_mb)"
}

test_a_mac_with_no_sysctl_answer_reads_its_memory_out_of_system_profiler() {
  stub_command_from_fixture system_profiler system_profiler/apple-m3-24gb
  assert_eq 24576 "$(hw_probe_ram_mb)" "\"Memory: 24 GB\""
}

test_a_machine_that_will_not_say_how_much_memory_it_has_answers_unknown() {
  assert_command_absent free
  assert_command_absent sysctl
  assert_eq unknown "$(hw_probe_ram_mb)"
}

# ================================================================ free space

test_free_space_comes_from_df_in_whole_megabytes() {
  mkdir -p "$HW_MODEL_DIR"
  stub_command_from_fixture df df/roomy
  assert_eq 510580 "$(hw_probe_disk_mb)" "522834900 blocks of 1 KB, rounded down"
}

test_a_nearly_full_disk_reads_as_nearly_full() {
  mkdir -p "$HW_MODEL_DIR"
  stub_command_from_fixture df df/tight
  assert_eq 800 "$(hw_probe_disk_mb)"
}

test_df_is_asked_about_a_directory_that_exists_and_not_one_that_does_not() {
  # The models directory has not been created yet on a first run, and df has
  # nothing to say about a path that is not there. The probe walks up.
  stub_command_from_fixture df df/roomy
  assert_file_missing "$HW_MODEL_DIR"
  hw_probe_disk_mb >/dev/null
  assert_stub_called_with df "$TEST_TMPDIR"
  local calls
  calls="$(stub_calls df)"
  assert_not_contains "$calls" "$HW_MODEL_DIR" \
    "asking df about a directory nobody has created answers nothing"
}

test_without_df_the_free_space_is_unknown() {
  assert_command_absent df
  assert_eq unknown "$(hw_probe_disk_mb)"
}

test_a_df_that_prints_nothing_useful_is_unknown_and_not_zero() {
  stub_command df --stdout "df: /nope: No such file or directory" --exit 1
  assert_eq unknown "$(hw_probe_disk_mb)"
}

# ============================================================== acceleration

test_the_biggest_card_in_a_machine_is_the_one_that_counts() {
  # "Can a card hold the model" is a question about the largest single card,
  # and nvidia-smi lists them in bus order, which puts a display adapter ahead
  # of the one that does the work as often as not.
  stub_command_from_fixture nvidia-smi nvidia-smi/two-cards
  hw_probe_acceleration
  assert_eq cuda "$_HW_ACCEL"
  assert_eq 24564 "$_HW_VRAM_MB"
  assert_eq "NVIDIA GeForce RTX 4090" "$_HW_GPU_NAME"
}

test_a_card_that_will_not_say_its_size_is_still_a_card() {
  # "[N/A]" is what a MIG slice, a vGPU and WSL answer. The verdict is a
  # statement of fact and the fact is that CUDA is there; it is the size that
  # is unknown, and unknown is what caps the tier.
  stub_command_from_fixture nvidia-smi nvidia-smi/no-size
  hw_probe_acceleration
  assert_eq cuda "$_HW_ACCEL"
  assert_eq unknown "$_HW_VRAM_MB"
  assert_ne "" "$_HW_GPU_NAME"
}

test_graphics_built_into_the_processor_do_not_count_their_pool_twice() {
  # An integrated Radeon is an HSA agent like any other and the pool it reports
  # is the machine's own memory - the same gigabytes the memory line has
  # already counted. Reporting them again as graphics memory would let one pool
  # satisfy two independent checks.
  stub_script rocminfo <<'SH'
cat <<'OUT'
==========
HSA Agents
==========
*******
Agent 1
*******
  Name:                    AMD Ryzen 7 7840U
  Device Type:             CPU
  Pool Info:
    Pool 1
      Segment:                 GLOBAL; FLAGS: FINE GRAINED
      Size:                    32505856(0x1f00000) KB
*******
Agent 2
*******
  Name:                    gfx1103
  Marketing Name:          AMD Radeon 780M Graphics
  Device Type:             GPU
  Pool Info:
    Pool 1
      Segment:                 GLOBAL; FLAGS: COARSE GRAINED
      Size:                    32505856(0x1f00000) KB
OUT
SH
  hw_probe_acceleration
  assert_eq rocm "$_HW_ACCEL" "the card is real"
  assert_eq unknown "$_HW_VRAM_MB" "and it has no memory of its own to offer"
}

test_an_nvidia_card_is_cuda_and_its_size_is_read() {
  stub_command_from_fixture nvidia-smi nvidia-smi/rtx-4090
  hw_probe_acceleration
  assert_eq cuda "$_HW_ACCEL"
  assert_eq 24564 "$_HW_VRAM_MB"
  assert_eq "NVIDIA GeForce RTX 4090" "$_HW_GPU_NAME"
}

test_nvidia_smi_that_cannot_find_a_driver_is_not_cuda() {
  stub_command_from_fixture nvidia-smi nvidia-smi/no-devices --exit 9
  hw_probe_acceleration
  assert_eq cpu "$_HW_ACCEL" \
    "the tool being installed is not the same as a card being present"
  assert_eq unknown "$_HW_VRAM_MB"
}

test_nvidia_smi_that_answers_zero_but_names_no_card_is_not_cuda() {
  stub_command nvidia-smi --stdout ""
  hw_probe_acceleration
  assert_eq cpu "$_HW_ACCEL"
}

test_an_amd_card_is_rocm_and_its_size_is_read() {
  stub_command_from_fixture rocminfo rocminfo/gfx1100
  hw_probe_acceleration
  assert_eq rocm "$_HW_ACCEL"
  assert_eq 24560 "$_HW_VRAM_MB" "25149440 KB"
  assert_eq "AMD Radeon RX 7900 XTX" "$_HW_GPU_NAME"
}

test_rocminfo_with_no_graphics_agent_in_it_is_not_rocm() {
  stub_command_from_fixture rocminfo rocminfo/no-gpu
  hw_probe_acceleration
  assert_eq cpu "$_HW_ACCEL"
}

test_an_amd_card_whose_size_cannot_be_found_is_rocm_with_an_unknown_size() {
  # Which is the whole point of separating the verdict from the size: the card
  # is real, and the recommendation stays conservative because the size is not
  # known rather than because the card is not there.
  stub_command_from_fixture rocminfo rocminfo/gfx-without-a-size
  hw_probe_acceleration
  assert_eq rocm "$_HW_ACCEL"
  assert_eq unknown "$_HW_VRAM_MB"
}

test_an_apple_silicon_mac_is_metal() {
  _stub_uname Darwin arm64
  hw_probe_acceleration
  assert_eq metal "$_HW_ACCEL"
}

test_an_intel_mac_is_not_metal_even_with_the_rest_of_a_mac_around_it() {
  _stub_uname Darwin x86_64
  stub_command_from_fixture system_profiler system_profiler/intel-i9-32gb
  stub_command_from_fixture sw_vers sw_vers/macos-13.6
  hw_probe_acceleration
  assert_eq cpu "$_HW_ACCEL"
  assert_eq unknown "$_HW_VRAM_MB"
}

test_an_intel_mac_is_not_metal() {
  # Conservative on purpose. Intel Macs have a Metal device, but the
  # accelerated transcription builds this project points people at are built
  # for Apple Silicon, and promising acceleration that is not there costs more
  # than declining to promise it.
  _stub_uname Darwin x86_64
  hw_probe_acceleration
  assert_eq cpu "$_HW_ACCEL"
}

test_metal_wins_over_a_card_when_both_could_be_claimed() {
  _stub_uname Darwin arm64
  stub_command_from_fixture nvidia-smi nvidia-smi/rtx-4090
  hw_probe_acceleration
  assert_eq metal "$_HW_ACCEL"
}

test_cuda_wins_over_rocm_when_both_are_present() {
  stub_command_from_fixture nvidia-smi nvidia-smi/rtx-4090
  stub_command_from_fixture rocminfo rocminfo/gfx1100
  hw_probe_acceleration
  assert_eq cuda "$_HW_ACCEL"
}

test_a_machine_with_neither_tool_installed_is_cpu() {
  assert_command_absent nvidia-smi
  assert_command_absent rocminfo
  hw_probe_acceleration
  assert_eq cpu "$_HW_ACCEL"
  assert_eq unknown "$_HW_VRAM_MB"
  assert_eq "" "$_HW_GPU_NAME"
}

# ================================================================= containers

test_a_machine_with_none_of_the_container_marks_is_not_one() {
  hw_probe_container
  assert_eq 0 "$_HW_CONTAINER"
  assert_eq none "$_HW_LIMITS"
}

test_the_docker_marker_file_is_enough_to_call_it_a_container() {
  HW_DOCKERENV_FILE="$TEST_TMPDIR/.dockerenv"
  : > "$HW_DOCKERENV_FILE"
  hw_probe_container
  assert_eq 1 "$_HW_CONTAINER"
}

test_the_podman_marker_file_is_enough_too() {
  HW_CONTAINERENV_FILE="$TEST_TMPDIR/.containerenv"
  : > "$HW_CONTAINERENV_FILE"
  hw_probe_container
  assert_eq 1 "$_HW_CONTAINER"
}

test_a_docker_path_in_the_first_processs_cgroup_is_enough_too() {
  HW_PROC1_CGROUP_FILE="$(_file_from cgroup/proc1-docker-v1)"
  hw_probe_container
  assert_eq 1 "$_HW_CONTAINER"
}

test_the_first_processs_cgroup_on_a_real_machine_is_not_a_container() {
  HW_PROC1_CGROUP_FILE="$(_file_from cgroup/proc1-host)"
  hw_probe_container
  assert_eq 0 "$_HW_CONTAINER"
}

test_a_container_with_a_memory_limit_reports_the_limit_and_not_the_host() {
  HW_DOCKERENV_FILE="$TEST_TMPDIR/.dockerenv"; : > "$HW_DOCKERENV_FILE"
  HW_CGROUP_MEM_MAX_FILE="$(_file_from cgroup/memory-max-1g)"
  HW_CGROUP_CPU_MAX_FILE="$(_file_from cgroup/cpu-max-unlimited)"
  hw_probe_container
  assert_eq 1024 "$_HW_LIMIT_RAM_MB"
  assert_eq known "$_HW_LIMITS"
}

test_a_container_with_no_memory_limit_says_so_rather_than_guessing() {
  HW_DOCKERENV_FILE="$TEST_TMPDIR/.dockerenv"; : > "$HW_DOCKERENV_FILE"
  HW_CGROUP_MEM_MAX_FILE="$(_file_from cgroup/memory-max-unlimited)"
  HW_CGROUP_CPU_MAX_FILE="$(_file_from cgroup/cpu-max-unlimited)"
  hw_probe_container
  assert_eq none "$_HW_LIMIT_RAM_MB" "\"max\" means the host's figure is right"
  assert_eq known "$_HW_LIMITS"
}

test_the_old_cgroup_layout_is_read_when_the_new_one_is_absent() {
  HW_DOCKERENV_FILE="$TEST_TMPDIR/.dockerenv"; : > "$HW_DOCKERENV_FILE"
  HW_CGROUP_MEM_LIMIT_FILE="$(_file_from cgroup/memory-limit-v1-2g)"
  HW_CGROUP_CPU_QUOTA_FILE="$(_file_from cgroup/cpu-quota-v1-four-cores)"
  HW_CGROUP_CPU_PERIOD_FILE="$(_file_from cgroup/cpu-period-v1)"
  hw_probe_container
  assert_eq 2048 "$_HW_LIMIT_RAM_MB"
  assert_eq 4 "$_HW_LIMIT_CORES"
  assert_eq known "$_HW_LIMITS"
}

test_the_old_layouts_enormous_number_means_no_limit_at_all() {
  HW_DOCKERENV_FILE="$TEST_TMPDIR/.dockerenv"; : > "$HW_DOCKERENV_FILE"
  HW_CGROUP_MEM_LIMIT_FILE="$(_file_from cgroup/memory-limit-v1-unlimited)"
  HW_CGROUP_CPU_QUOTA_FILE="$(_file_from cgroup/cpu-quota-v1-unlimited)"
  HW_CGROUP_CPU_PERIOD_FILE="$(_file_from cgroup/cpu-period-v1)"
  hw_probe_container
  assert_eq none "$_HW_LIMIT_RAM_MB"
  assert_eq none "$_HW_LIMIT_CORES"
}

test_a_cpu_share_of_two_cores_reads_as_two_cores() {
  HW_DOCKERENV_FILE="$TEST_TMPDIR/.dockerenv"; : > "$HW_DOCKERENV_FILE"
  HW_CGROUP_MEM_MAX_FILE="$(_file_from cgroup/memory-max-unlimited)"
  HW_CGROUP_CPU_MAX_FILE="$(_file_from cgroup/cpu-max-two-cores)"
  hw_probe_container
  assert_eq 2 "$_HW_LIMIT_CORES"
}

test_a_fractional_cpu_share_rounds_down_and_never_below_one() {
  HW_DOCKERENV_FILE="$TEST_TMPDIR/.dockerenv"; : > "$HW_DOCKERENV_FILE"
  HW_CGROUP_MEM_MAX_FILE="$(_file_from cgroup/memory-max-unlimited)"
  HW_CGROUP_CPU_MAX_FILE="$(_file_from cgroup/cpu-max-one-and-a-half-cores)"
  hw_probe_container
  assert_eq 1 "$_HW_LIMIT_CORES" \
    "a share and a half is one whole core you can count on"
}

test_a_pod_with_no_marker_file_at_all_is_still_found_by_its_limits() {
  # A kubernetes pod under containerd has no /.dockerenv, no /run/.containerenv
  # and, with its own cgroup namespace, a /proc/1/cgroup that reads exactly
  # "0::/". The limits are the only evidence there is that this is a share of a
  # machine rather than a machine, and a run that only looked for marker files
  # would tell a 512 MB pod on a 64-core node that it can hold the largest
  # model there is.
  HW_PROC1_CGROUP_FILE="$(_file_from cgroup/proc1-docker)"
  HW_CGROUP_MEM_MAX_FILE="$(_file_from cgroup/memory-max-1g)"
  HW_CGROUP_CPU_MAX_FILE="$(_file_from cgroup/cpu-max-two-cores)"
  hw_probe_container
  assert_eq 1 "$_HW_CONTAINER" "a real limit is a real share of a machine"
  assert_eq known "$_HW_LIMITS"
  assert_eq 1024 "$_HW_LIMIT_RAM_MB"
  assert_eq 2 "$_HW_LIMIT_CORES"
}

test_a_systemd_nspawn_container_is_recognised() {
  HW_PROC1_CGROUP_FILE="$(_file_from cgroup/proc1-nspawn)"
  hw_probe_container
  assert_eq 1 "$_HW_CONTAINER"
}

test_the_container_environment_variable_is_a_mark_of_its_own() {
  container=podman
  export container
  hw_probe_container
  assert_eq 1 "$_HW_CONTAINER"
}

test_the_limits_are_read_from_this_processs_own_cgroup_and_not_the_machines() {
  # Under a shared cgroup namespace /sys/fs/cgroup/memory.max is the *host's*
  # root, which has no limit on anything - a true statement about the host and
  # a dangerous one about a container. The path has to be composed from
  # /proc/self/cgroup or the answer is always "no limit".
  HW_PROC_SELF_CGROUP_FILE="$(_file_from cgroup/self-cgroup-v2-docker)"
  HW_CGROUP_ROOT="$TEST_TMPDIR/cgroup"
  mkdir -p "$HW_CGROUP_ROOT/docker/abc123"
  printf 'max\n' > "$HW_CGROUP_ROOT/memory.max"
  printf 'max 100000\n' > "$HW_CGROUP_ROOT/cpu.max"
  printf '2147483648\n' > "$HW_CGROUP_ROOT/docker/abc123/memory.max"
  printf '400000 100000\n' > "$HW_CGROUP_ROOT/docker/abc123/cpu.max"
  hw_probe_container
  assert_eq 2048 "$_HW_LIMIT_RAM_MB" "the container's own limit, not the root's"
  assert_eq 4 "$_HW_LIMIT_CORES"
}

test_the_old_cgroup_layout_is_composed_the_same_way() {
  HW_PROC_SELF_CGROUP_FILE="$(_file_from cgroup/self-cgroup-v1-docker)"
  HW_CGROUP_ROOT="$TEST_TMPDIR/cgroup"
  mkdir -p "$HW_CGROUP_ROOT/memory/docker/abc123" "$HW_CGROUP_ROOT/cpu/docker/abc123"
  printf '1073741824\n' > "$HW_CGROUP_ROOT/memory/docker/abc123/memory.limit_in_bytes"
  printf '200000\n' > "$HW_CGROUP_ROOT/cpu/docker/abc123/cpu.cfs_quota_us"
  printf '100000\n' > "$HW_CGROUP_ROOT/cpu/docker/abc123/cpu.cfs_period_us"
  hw_probe_container
  assert_eq 1024 "$_HW_LIMIT_RAM_MB"
  assert_eq 2 "$_HW_LIMIT_CORES"
}

test_a_controller_name_is_matched_whole_and_not_as_a_substring() {
  # "cpu,cpuacct" contains "cpuacct", and a cgroup line for cpuacct alone is
  # not a cgroup line for cpu.
  HW_PROC_SELF_CGROUP_FILE="$TEST_TMPDIR/self-cgroup"
  printf '9:cpuacct:/somewhere-else\n5:cpu,cpuset:/right-here\n' \
    > "$HW_PROC_SELF_CGROUP_FILE"
  HW_CGROUP_ROOT="$TEST_TMPDIR/cgroup"
  mkdir -p "$HW_CGROUP_ROOT/cpu/right-here"
  printf '300000\n' > "$HW_CGROUP_ROOT/cpu/right-here/cpu.cfs_quota_us"
  printf '100000\n' > "$HW_CGROUP_ROOT/cpu/right-here/cpu.cfs_period_us"
  HW_DOCKERENV_FILE="$TEST_TMPDIR/.dockerenv"; : > "$HW_DOCKERENV_FILE"
  hw_probe_container
  assert_eq 3 "$_HW_LIMIT_CORES"
}

test_a_container_whose_limits_cannot_be_read_says_unknown_and_not_none() {
  HW_DOCKERENV_FILE="$TEST_TMPDIR/.dockerenv"; : > "$HW_DOCKERENV_FILE"
  hw_probe_container
  assert_eq 1 "$_HW_CONTAINER"
  assert_eq unknown "$_HW_LIMITS" \
    "not a container without limits: a container whose limits are a mystery"
}

# ================================================================== network

test_a_default_route_means_the_internet_is_probably_reachable() {
  HW_ROUTE_FILE="$(_file_from route/default)"
  assert_eq online "$(hw_probe_network)"
}

test_a_routing_table_with_only_loopback_in_it_means_offline() {
  HW_ROUTE_FILE="$(_file_from route/no-default)"
  assert_eq offline "$(hw_probe_network)"
}

test_a_machine_whose_routing_table_cannot_be_read_answers_unknown() {
  assert_command_absent route
  assert_eq unknown "$(hw_probe_network)"
}

test_a_machine_with_only_an_ipv6_address_is_not_called_offline() {
  # It has no entry in the v4 table at all, and calling it offline would be a
  # positive claim made out of having looked in one place.
  HW_ROUTE_FILE="$(_file_from route/no-default)"
  HW_ROUTE6_FILE="$(_file_from route/ipv6-default)"
  assert_eq online "$(hw_probe_network)"
}

test_a_container_with_no_network_at_all_is_offline_despite_its_loopback_routes() {
  # Docker's --network=none leaves two ::/0 entries pointing at loopback. They
  # are the unreachable routes that make a send fail rather than hang, and
  # reading them as a way out would suppress the one warning somebody about to
  # download a model needs. Taken from a real `docker run --network=none`.
  HW_ROUTE_FILE="$TEST_TMPDIR/route"
  printf 'Iface\tDestination\tGateway\n' > "$HW_ROUTE_FILE"
  HW_ROUTE6_FILE="$(_file_from route/ipv6-loopback-only)"
  assert_eq offline "$(hw_probe_network)"
}

test_a_machine_with_no_default_route_in_either_table_is_offline() {
  HW_ROUTE_FILE="$(_file_from route/no-default)"
  HW_ROUTE6_FILE="$(_file_from route/ipv6-no-default)"
  assert_eq offline "$(hw_probe_network)"
}

test_nothing_is_dialled_to_find_out_whether_the_machine_is_online() {
  HW_ROUTE_FILE="$(_file_from route/default)"
  stub_command curl
  stub_command ping
  hw_probe_network >/dev/null
  assert_stub_not_called curl
  assert_stub_not_called ping
}

# ========================================================= the whole picture

test_detection_clamps_the_core_count_to_a_containers_share() {
  HW_MEMINFO_FILE="$(_file_from meminfo/64gb)"
  stub_command_from_fixture lscpu lscpu/x86_64-8core
  stub_command_from_fixture df df/roomy
  mkdir -p "$HW_MODEL_DIR"
  HW_DOCKERENV_FILE="$TEST_TMPDIR/.dockerenv"; : > "$HW_DOCKERENV_FILE"
  HW_CGROUP_MEM_MAX_FILE="$(_file_from cgroup/memory-max-1g)"
  HW_CGROUP_CPU_MAX_FILE="$(_file_from cgroup/cpu-max-two-cores)"

  hw_detect
  assert_eq 2 "$_HW_CORES" "lscpu sees the host's eight; the share is two"
  assert_eq 1024 "$_HW_RAM_MB" "meminfo sees the host's 64 GB; the share is one"
  assert_eq text-only "$_HW_TIER" "a gigabyte of memory is a gigabyte of memory"
}

test_detection_never_raises_a_number_to_a_containers_limit() {
  # A limit larger than the machine is not more machine.
  HW_MEMINFO_FILE="$(_file_from meminfo/2gb)"
  HW_DOCKERENV_FILE="$TEST_TMPDIR/.dockerenv"; : > "$HW_DOCKERENV_FILE"
  HW_CGROUP_MEM_MAX_FILE="$(_file_from cgroup/memory-max-unlimited)"
  HW_CGROUP_CPU_MAX_FILE="$(_file_from cgroup/cpu-max-unlimited)"
  hw_detect
  assert_eq 1849 "$_HW_RAM_MB"
}

test_a_bare_machine_with_nothing_readable_detects_as_the_lowest_tier() {
  hw_detect
  assert_eq unknown "$_HW_RAM_MB"
  assert_eq unknown "$_HW_CORES"
  assert_eq unknown "$_HW_DISK_MB"
  assert_eq cpu "$_HW_ACCEL"
  assert_eq lightweight "$_HW_TIER" \
    "capped by not knowing, not by knowing something bad"
  assert_contains "$_HW_TIER_REASON" "could not"
}

test_detection_runs_once_and_remembers_its_answer() {
  stub_command nproc --stdout "6"
  hw_detect
  hw_detect
  assert_stub_call_count nproc 1
}

test_every_probe_answers_zero_whether_or_not_it_found_anything() {
  # The one contract the whole design rests on: nothing here is worth ending a
  # setup run over, and install.sh runs under `set -euo pipefail`.
  local status
  hw_probe_cores >/dev/null; status=$?; assert_status 0 "$status" "cores, absent"
  hw_probe_ram_mb >/dev/null; status=$?; assert_status 0 "$status" "memory, absent"
  hw_probe_disk_mb >/dev/null; status=$?; assert_status 0 "$status" "disk, absent"
  hw_probe_network >/dev/null; status=$?; assert_status 0 "$status" "network, absent"
  hw_probe_acceleration; status=$?; assert_status 0 "$status" "graphics, absent"
  hw_probe_container; status=$?; assert_status 0 "$status" "container, absent"

  HW_MEMINFO_FILE="$(_file_from meminfo/16gb)"
  HW_ROUTE_FILE="$(_file_from route/default)"
  stub_command_from_fixture lscpu lscpu/x86_64-8core
  stub_command_from_fixture df df/roomy
  stub_command_from_fixture nvidia-smi nvidia-smi/rtx-4090
  hw_probe_cores >/dev/null; status=$?; assert_status 0 "$status" "cores, present"
  hw_probe_ram_mb >/dev/null; status=$?; assert_status 0 "$status" "memory, present"
  hw_probe_disk_mb >/dev/null; status=$?; assert_status 0 "$status" "disk, present"
  hw_probe_network >/dev/null; status=$?; assert_status 0 "$status" "network, present"
  hw_probe_acceleration; status=$?; assert_status 0 "$status" "graphics, present"
  hw_detect; status=$?; assert_status 0 "$status" "the whole thing"
}

test_a_directory_where_a_file_was_expected_is_quiet_about_it() {
  # `[ -r ]` is true of a directory, and reading one prints a raw diagnostic on
  # stderr in the middle of a wizard that has promised to be quiet.
  HW_MEMINFO_FILE="$TEST_TMPDIR/meminfo-dir"
  HW_ROUTE_FILE="$TEST_TMPDIR/route-dir"
  mkdir -p "$HW_MEMINFO_FILE" "$HW_ROUTE_FILE"
  local err="$TEST_TMPDIR/err"
  hw_probe_ram_mb >/dev/null 2>"$err"
  assert_eq unknown "$(hw_probe_ram_mb 2>/dev/null)"
  assert_eq unknown "$(hw_probe_network 2>/dev/null)"
  assert_eq "" "$(cat "$err")" "nothing raw reaches the screen"
}

test_the_module_survives_being_sourced_by_a_script_with_no_home_directory() {
  # install.sh sources every step file under `set -euo pipefail`, before it has
  # printed anything. An unbound variable here is a wizard that dies with a
  # bash diagnostic and no explanation.
  local script="$TEST_TMPDIR/strict.sh"
  cat > "$script" <<'SH'
set -euo pipefail
unset HOME
. "$REPO_ROOT/lib/wizard.sh"
wizard_stage detect d
wizard_stage report r
. "$REPO_ROOT/lib/steps/20-hardware.sh"
hw_detect
printf 'survived tier=%s\n' "$_HW_TIER"
SH
  REPO_ROOT="$REPO_ROOT" run_script bash "$script"
  assert_status 0 "$RUN_STATUS"
  assert_contains "$RUN_STDOUT" "survived tier="
}

test_the_answers_are_read_back_out_of_the_state_file_by_a_shell_that_probed_nothing() {
  # The published contract, at its own level: no installer, no cache, nothing
  # readable on the machine, and every accessor still answers what an earlier
  # run worked out rather than its own default.
  wizard_remember step.detect.hardware.acceleration cuda
  wizard_remember step.detect.hardware.tier maximum
  wizard_remember step.detect.hardware.tier_reason ""
  wizard_remember step.detect.hardware.tier_cause ""
  wizard_remember step.detect.hardware.vram_mb 24564
  wizard_remember step.detect.hardware.gpu "NVIDIA GeForce RTX 4090"
  _HW_DETECTED=0
  assert_eq cuda "$(hw_acceleration)"
  assert_eq maximum "$(hw_voice_tier)"
  assert_eq "NVIDIA GeForce RTX 4090" "$(hw_gpu_name)"
  assert_eq "" "$(hw_tier_reason)"
  hw_accel_usable || fail "a 24 GB card is worth using"
  hw_tier_at_least recommended || fail "maximum is at least recommended"
  assert_eq 0 "$_HW_DETECTED" "and none of that probed anything"
}

test_an_accessor_asked_before_anything_has_happened_measures_rather_than_guesses() {
  # The dangerous case: no cache and no state file. Every default here is a
  # meaningful, wrong claim - an empty reason means "nothing held this machine
  # back", an empty card name means "there is no card" - so the answer has to
  # be worked out rather than defaulted to.
  stub_command_from_fixture nvidia-smi nvidia-smi/rtx-4090
  HW_MEMINFO_FILE="$(_file_from meminfo/2gb)"
  _HW_DETECTED=0
  assert_eq "" "$(wizard_state_get step.detect.hardware.tier "")" "nothing stored"
  assert_eq "NVIDIA GeForce RTX 4090" "$(hw_gpu_name)"
  _HW_DETECTED=0
  assert_ne "" "$(hw_tier_reason)" "this machine is capped, and it must say so"
  _HW_DETECTED=0
  hw_accel_usable || fail "the card is right there"
}

test_detection_writes_nothing_and_runs_nothing_that_changes_the_machine() {
  HW_MEMINFO_FILE="$(_file_from meminfo/16gb)"
  stub_command_from_fixture lscpu lscpu/x86_64-8core
  stub_command sudo
  stub_command apt-get
  stub_command curl
  hw_detect
  assert_stub_not_called sudo
  assert_stub_not_called apt-get
  assert_stub_not_called curl
}
