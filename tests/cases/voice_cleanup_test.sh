# Tidying up what was heard: optional inside an option, and offered only where
# it would not make talking slower than typing.
#
# Three rules, and each has a test: setup does not install ollama, only
# instruct models are named, and a weak processor-only machine is not offered a
# rewrite model at all. Nothing here pulls anything - ollama is a stub, and
# what is asserted is the command it was asked to run.

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

_file_from() {
  assert_fixture_exists "$1"
  fixture_file "$1"
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

_cuda_desktop() {
  HW_MEMINFO_FILE="$(_file_from meminfo/64gb)"
  HW_ROUTE_FILE="$(_file_from route/default)"
  mkdir -p "$HW_MODEL_DIR"
  stub_command_from_fixture lscpu lscpu/x86_64-8core
  stub_command_from_fixture df df/roomy
  stub_command_from_fixture nvidia-smi nvidia-smi/rtx-4090
  stub_command whisper-cpp
  stub_command ffmpeg
  _curl_writes "$TINY_BYTES"
}

# Four gigabytes and no graphics card: the machine the rule about weak
# processor-only computers is written for.
_weak_laptop() {
  HW_MEMINFO_FILE="$(_file_from meminfo/4gb)"
  HW_ROUTE_FILE="$(_file_from route/default)"
  mkdir -p "$HW_MODEL_DIR"
  stub_command_from_fixture lscpu lscpu/x86_64-8core
  stub_command_from_fixture df df/roomy
  stub_command whisper-cpp
  stub_command ffmpeg
  _curl_writes "$TINY_BYTES"
}

_config_prompts() {
  pty_expect "port [" ""
  pty_expect "allowed hosts" ""
  pty_expect "ntfy topic URL" ""
}

# ==================================================== when it is offered

test_the_cleanup_model_is_offered_where_ollama_is_already_installed() {
  _cuda_desktop
  stub_command ollama
  _config_prompts
  pty_expect "which one?" "3"
  pty_expect "tidy up what you say?" "y"
  pty_expect "install Claude Code?" "n"
  pty_expect "go ahead with all of that?" "n"
  pty_install --no-systemd

  assert_status 0 "$PTY_STATUS"
  assert_contains "$PTY_TRANSCRIPT" "ayeaye can also tidy up what it heard"
  assert_contains "$PTY_TRANSCRIPT" "a tidying-up model, about 4700 MB, fetched by ollama"
  assert_file_contains "$(_state)" "answer.voice.cleanup=1"
  assert_file_contains "$(_state)" "answer.voice.cleanup_model=qwen2.5:7b-instruct"
}

test_setup_will_not_install_ollama_and_says_where_to_find_it_instead() {
  # There is exactly one exemption in this repository from "ask before you
  # fetch", and it is install.sh's bootstrap. Piping somebody else's installer
  # into a shell would be a second one.
  _cuda_desktop
  assert_command_absent ollama
  _config_prompts
  pty_expect "which one?" "3"
  pty_expect "install Claude Code?" "n"
  pty_expect "go ahead with all of that?" "n"
  pty_install --no-systemd

  assert_contains "$PTY_TRANSCRIPT" \
    "not offered here: that needs a separate program called ollama"
  assert_contains "$PTY_TRANSCRIPT" "https://ollama.com"
  assert_contains "$PTY_TRANSCRIPT" "setup will notice it next time"
  assert_file_contains "$(_state)" "answer.voice.cleanup=0"
  assert_not_contains "$PTY_TRANSCRIPT" "a tidying-up model, about"
}

test_a_weak_processor_only_computer_is_not_offered_it_and_is_told_why() {
  _weak_laptop
  stub_command ollama
  _config_prompts
  pty_expect "which one?" "2"
  pty_expect "install Claude Code?" "n"
  pty_expect "go ahead with all of that?" "n"
  pty_install --no-systemd

  assert_contains "$PTY_TRANSCRIPT" \
    "not offered here: it would run after everything you say"
  assert_contains "$PTY_TRANSCRIPT" "slower than typing"
  assert_file_contains "$(_state)" "answer.voice.cleanup=0"
  assert_not_contains "$PTY_TRANSCRIPT" "tidy up what you say?"
}

# ===================================================== what is really run

test_the_pull_that_is_asked_about_is_the_pull_that_is_run() {
  _cuda_desktop
  stub_command ollama
  _config_prompts
  pty_expect "which one?" "2"
  pty_expect "tidy up what you say?" "y"
  pty_expect "install Claude Code?" "n"
  pty_expect "go ahead with all of that?" "y"
  pty_expect "may I download the listening model?" "y"
  pty_expect "may I download the tidying-up model?" "y"
  pty_install --no-systemd

  assert_status 0 "$PTY_STATUS"
  assert_stub_called_with ollama "ollama pull qwen2.5:7b-instruct"
  assert_eq "VOICE_OLLAMA_MODEL=qwen2.5:7b-instruct" \
    "$(grep '^VOICE_OLLAMA_MODEL=' "$(_env)")"
  assert_file_contains "$(_state)" "step.install.voice=done"
}

test_refusing_the_cleanup_model_fetches_nothing_and_fails_nothing() {
  _cuda_desktop
  stub_command ollama
  _config_prompts
  pty_expect "which one?" "2"
  pty_expect "tidy up what you say?" "y"
  pty_expect "install Claude Code?" "n"
  pty_expect "go ahead with all of that?" "y"
  pty_expect "may I download the listening model?" "y"
  pty_expect "may I download the tidying-up model?" "n"
  pty_install --no-systemd

  assert_status 0 "$PTY_STATUS"
  assert_stub_not_called ollama
  assert_eq "" "$(grep '^VOICE_OLLAMA_MODEL=' "$(_env)")" \
    "the template's commented-out default is not an assignment"
  assert_file_contains "$(_state)" "step.install.voice=done" \
    "the listening model arrived, and a refusal is not a failure"
  assert_contains "$PTY_TRANSCRIPT" "was heard, which is what ayeaye does without it anyway."
}

test_a_pull_that_does_not_work_lands_on_the_closing_checklist() {
  _cuda_desktop
  stub_command ollama --exit 1 --stderr "pull model manifest: connection refused"
  _config_prompts
  pty_expect "which one?" "2"
  pty_expect "tidy up what you say?" "y"
  pty_expect "install Claude Code?" "n"
  pty_expect "go ahead with all of that?" "y"
  pty_expect "may I download the listening model?" "y"
  pty_expect "may I download the tidying-up model?" "y"
  pty_install --no-systemd

  assert_status 0 "$PTY_STATUS" "voice is optional, and it does not fail the run"
  assert_contains "$PTY_TRANSCRIPT" "the tidying-up model would not download"
  assert_file_contains "$(_state)" "step.install.voice=pending"
  assert_contains "$PTY_TRANSCRIPT" "Setting up talking out loud (not finished)"
}

test_declining_at_the_question_never_reaches_the_download() {
  _cuda_desktop
  stub_command ollama
  _config_prompts
  pty_expect "which one?" "2"
  pty_expect "tidy up what you say?" "n"
  pty_expect "install Claude Code?" "n"
  pty_expect "go ahead with all of that?" "y"
  pty_expect "may I download the listening model?" "y"
  pty_install --no-systemd

  assert_status 0 "$PTY_STATUS"
  assert_stub_not_called ollama
  assert_not_contains "$PTY_TRANSCRIPT" "may I download the tidying-up model?"
  assert_file_contains "$(_state)" "step.install.voice=done"
}
