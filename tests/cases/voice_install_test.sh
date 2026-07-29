# Fetching it: the exact address, the one door that asks first, and what
# happens to a file that arrives wrong.
#
# Nothing here downloads anything. curl is a stub that records the request and
# writes a file of the length the real artifact has, so the request, the
# destination and every branch after it can be asserted without moving half a
# gigabyte. The one test that really fetches is opt-in, fetches the smallest
# artifact there is, and says so in its name.

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

# The smallest model in the catalogue, and the size its authors publish for it.
TINY_URL="https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.en.bin"
TINY_BYTES="77704715"
TINY_SUM="921e4cf8686fdd993dcd081a5da5b6c365bfde1162e72b08d75ac75289920b1f"

_state() { printf '%s' "$XDG_STATE_HOME/ayeaye/setup-state"; }
_env()   { printf '%s' "$XDG_CONFIG_HOME/ayeaye/env"; }
_model() { printf '%s' "$HW_MODEL_DIR/ggml-tiny.en.bin"; }

_file_from() {
  assert_fixture_exists "$1"
  fixture_file "$1"
}

# A machine that can be offered every experience, with everything listening
# needs already on it, so that what is under test is the fetch and not the
# package layer.
_ready_machine() {
  HW_MEMINFO_FILE="$(_file_from meminfo/64gb)"
  HW_ROUTE_FILE="$(_file_from route/default)"
  mkdir -p "$HW_MODEL_DIR"
  stub_command_from_fixture lscpu lscpu/x86_64-8core
  stub_command_from_fixture df df/roomy
  stub_command_from_fixture nvidia-smi nvidia-smi/rtx-4090
  stub_command whisper-server
  stub_command ffmpeg
}

# curl, recording the request and writing a file of a length this test picks.
# Sparse, so "the artifact is 78 MB" costs the suite no disk at all.
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

# The prompts that come before and after this file's own, in the order the run
# asks them. curl being present is what makes the agent question appear at all.
_config_prompts() {
  pty_expect "bind address" ""
  pty_expect "port [" ""
  pty_expect "allowed hosts" ""
  pty_expect "ntfy topic URL" ""
}

_decline_the_agents() { pty_expect "install Claude Code?" "n"; }

# A machine with a package manager can also unpack, so two more offers appear.
_decline_everything_else() {
  pty_expect "install Claude Code?" "n"
  pty_expect "install OpenAI Codex?" "n"
  pty_expect "install cliban as well?" "n"
}

# =============================================== the request that is made

test_the_model_is_fetched_from_the_address_its_authors_publish() {
  _ready_machine
  _curl_writes "$TINY_BYTES"
  _config_prompts
  pty_expect "which one?" "2"
  _decline_the_agents
  pty_expect "go ahead with all of that?" "y"
  pty_expect "may I download the listening model?" "y"
  pty_install --no-systemd

  assert_status 0 "$PTY_STATUS"
  assert_stub_called_with curl "$TINY_URL"
  assert_stub_called_with curl "-o $(_model)"
  assert_file_exists "$(_model)"
  assert_file_contains "$(_state)" "step.install.voice=done"
}

test_the_question_says_what_it_will_cost_before_it_is_answered() {
  _ready_machine
  _curl_writes "$TINY_BYTES"
  _config_prompts
  pty_expect "which one?" "2"
  _decline_the_agents
  pty_expect "go ahead with all of that?" "y"
  pty_expect "may I download the listening model?" "y"
  pty_install --no-systemd
  assert_contains "$PTY_TRANSCRIPT" \
    "may I download the listening model? It is about 78 MB."
}

# ================================================= refusing changes nothing

test_refusing_the_download_leaves_the_computer_exactly_as_it_was() {
  _ready_machine
  _curl_writes "$TINY_BYTES"
  _config_prompts
  pty_expect "which one?" "2"
  _decline_the_agents
  pty_expect "go ahead with all of that?" "y"
  pty_expect "may I download the listening model?" "n"
  pty_install --no-systemd

  assert_status 0 "$PTY_STATUS"
  assert_stub_not_called curl
  assert_file_missing "$(_model)"
  assert_file_not_contains "$(_env)" "VOICE_WHISPER_MODEL=$HW_MODEL_DIR"
  assert_file_contains "$(_state)" "step.install.voice=skipped" \
    "a refusal is finished business, not work left half done"
  assert_contains "$PTY_TRANSCRIPT" "ayeaye will run text-only until it is."
}

# ============================================== a file that arrives wrong

test_a_download_that_stops_halfway_is_deleted_rather_than_believed() {
  # The failure this really has to catch: the right name, the wrong length.
  # Kept, it would be a model every later run believed in and nothing could
  # load.
  _ready_machine
  _curl_writes "12345"
  _config_prompts
  pty_expect "which one?" "2"
  _decline_the_agents
  pty_expect "go ahead with all of that?" "y"
  pty_expect "may I download the listening model?" "y"
  pty_install --no-systemd

  assert_status 0 "$PTY_STATUS"
  assert_stub_called curl
  assert_file_missing "$(_model)" "the file that arrived wrong was not kept"
  assert_contains "$PTY_TRANSCRIPT" "it is not the file it should"
  assert_file_contains "$(_state)" "step.install.voice=pending" \
    "it tried, it did not finish, and the closing checklist should say so"
  assert_contains "$PTY_TRANSCRIPT" "Setting up talking out loud (not finished)"
}

test_a_computer_with_no_way_to_take_a_fingerprint_says_so_and_checks_the_size() {
  # No sha256sum and no shasum on this machine. The size check is not sold as
  # a checksum; it is named for what it is.
  _ready_machine
  _curl_writes "$TINY_BYTES"
  assert_command_absent sha256sum
  assert_command_absent shasum
  _config_prompts
  pty_expect "which one?" "2"
  _decline_the_agents
  pty_expect "go ahead with all of that?" "y"
  pty_expect "may I download the listening model?" "y"
  pty_install --no-systemd
  assert_contains "$PTY_TRANSCRIPT" "only"
  assert_contains "$PTY_TRANSCRIPT" "its size was checked"
  assert_file_contains "$XDG_STATE_HOME/ayeaye/setup.log" \
    "no sha256sum or shasum here, checking the size only"
}

# ==================================================== the published checksum

test_the_checksum_its_authors_published_is_what_is_checked_when_it_can_be() {
  _ready_machine
  _curl_writes "$TINY_BYTES"
  stub_command sha256sum --stdout "$TINY_SUM  ignored"
  _config_prompts
  pty_expect "which one?" "2"
  _decline_the_agents
  pty_expect "go ahead with all of that?" "y"
  pty_expect "may I download the listening model?" "y"
  pty_install --no-systemd

  assert_status 0 "$PTY_STATUS"
  assert_stub_called_with sha256sum "$(_model)"
  assert_contains "$PTY_TRANSCRIPT" \
    "the listening model, checked against the checksum its authors published"
  assert_file_contains "$(_state)" "step.install.voice=done"
}

test_a_checksum_that_does_not_match_throws_the_download_away() {
  _ready_machine
  _curl_writes "$TINY_BYTES"
  stub_command sha256sum --stdout "0000000000000000000000000000000000000000000000000000000000000000  x"
  _config_prompts
  pty_expect "which one?" "2"
  _decline_the_agents
  pty_expect "go ahead with all of that?" "y"
  pty_expect "may I download the listening model?" "y"
  pty_install --no-systemd

  assert_file_missing "$(_model)"
  assert_file_contains "$(_state)" "step.install.voice=pending"
  assert_file_contains "$XDG_STATE_HOME/ayeaye/setup.log" "expected $TINY_SUM"
}

# ================================================ picking up where it stopped

test_a_model_already_on_this_computer_is_not_fetched_a_second_time() {
  # This is what makes an interrupted fetch resumable: the run after it finds
  # the file, checks it, asks nothing and moves on.
  _ready_machine
  _curl_writes "$TINY_BYTES"
  mkdir -p "$HW_MODEL_DIR"
  python3 -c 'import sys; open(sys.argv[1], "wb").truncate(int(sys.argv[2]))' \
    "$(_model)" "$TINY_BYTES"
  _config_prompts
  pty_expect "which one?" "2"
  _decline_the_agents
  pty_expect "go ahead with all of that?" "y"
  pty_install --no-systemd

  assert_status 0 "$PTY_STATUS"
  assert_stub_not_called curl "there was nothing left to fetch"
  assert_contains "$PTY_TRANSCRIPT" "the listening model is already on this computer"
  assert_not_contains "$PTY_TRANSCRIPT" "may I download the listening model?" \
    "nor anything left to ask about"
  assert_file_contains "$(_state)" "step.install.voice=done"
}

test_a_model_already_here_is_checked_against_its_checksum_before_being_believed() {
  # The path above proves only that a file of the right length is kept. This
  # one proves the checksum is what decides it, which is the difference
  # between "there is a file here" and "the right file is here".
  _ready_machine
  _curl_writes "$TINY_BYTES"
  stub_command sha256sum --stdout "$TINY_SUM  ignored"
  mkdir -p "$HW_MODEL_DIR"
  python3 -c 'import sys; open(sys.argv[1], "wb").truncate(int(sys.argv[2]))' \
    "$(_model)" "$TINY_BYTES"
  _config_prompts
  pty_expect "which one?" "2"
  _decline_the_agents
  pty_expect "go ahead with all of that?" "y"
  pty_install --no-systemd

  assert_status 0 "$PTY_STATUS"
  assert_stub_called_with sha256sum "$(_model)"
  assert_stub_not_called curl "the checksum matched, so there was nothing to fetch"
  assert_contains "$PTY_TRANSCRIPT" "the listening model is already on this computer"
}

test_a_file_of_the_right_length_and_the_wrong_content_is_not_believed() {
  # A length check cannot tell these apart and a checksum can. Setup did not
  # put this file here, so it is not setup's to delete without asking.
  _ready_machine
  _curl_writes "$TINY_BYTES"
  stub_command sha256sum --stdout "00000000000000000000000000000000000000000000000000000000deadbeef  x"
  mkdir -p "$HW_MODEL_DIR"
  python3 -c 'import sys; open(sys.argv[1], "wb").truncate(int(sys.argv[2]))' \
    "$(_model)" "$TINY_BYTES"
  _config_prompts
  pty_expect "which one?" "2"
  _decline_the_agents
  pty_expect "go ahead with all of that?" "y"
  pty_expect "May I replace it?" "n"
  pty_install --no-systemd

  assert_status 0 "$PTY_STATUS"
  assert_stub_not_called curl
  assert_file_exists "$(_model)" "a file setup did not write is not setup's to delete"
  assert_contains "$PTY_TRANSCRIPT" "left alone, so nothing has been downloaded"
}

test_a_file_somebody_else_put_there_is_backed_up_before_it_is_replaced() {
  _ready_machine
  _curl_writes "$TINY_BYTES"
  mkdir -p "$HW_MODEL_DIR"
  printf 'my own model\n' > "$(_model)"
  _config_prompts
  pty_expect "which one?" "2"
  _decline_the_agents
  pty_expect "go ahead with all of that?" "y"
  pty_expect "May I replace it?" "y"
  pty_expect "may I download the listening model?" "y"
  pty_install --no-systemd

  assert_status 0 "$PTY_STATUS"
  assert_contains "$PTY_TRANSCRIPT" "saved a copy of the old version at"
  assert_stub_called_with curl "$TINY_URL"
}

test_a_fetch_setup_started_itself_is_finished_without_asking_again() {
  # Setup wrote the half file, setup knows it is half a file, and asking
  # somebody for permission to delete the wreckage of setup's own interrupted
  # download would be asking a question with only one sensible answer.
  _ready_machine
  _curl_writes "$TINY_BYTES"
  mkdir -p "$XDG_STATE_HOME/ayeaye" "$HW_MODEL_DIR"
  printf 'half a file\n' > "$(_model)"
  printf 'step.install.voice.fetching=%s\n' "$(_model)" \
    >> "$XDG_STATE_HOME/ayeaye/setup-state"
  _config_prompts
  pty_expect "which one?" "2"
  _decline_the_agents
  pty_expect "go ahead with all of that?" "y"
  pty_expect "may I download the listening model?" "y"
  pty_install --no-systemd

  assert_status 0 "$PTY_STATUS"
  assert_not_contains "$PTY_TRANSCRIPT" "May I replace it?"
  assert_contains "$PTY_TRANSCRIPT" "it is not complete"
  assert_stub_called_with curl "$TINY_URL"
  assert_file_contains "$(_state)" "step.install.voice=done"
}

# ============================================ what listening needs, installed

test_what_listening_needs_is_installed_through_the_one_door_that_asks_first() {
  _ready_machine
  stub_remove ffmpeg
  # Root, so the generated command is apt-get itself rather than sudo, and the
  # stub standing in for the package manager is the thing that really runs -
  # which is what lets it put ffmpeg on the path the way a real install would.
  stub_command id --stdout "0"
  stub_script apt-get <<'SH'
for arg in "$@"; do
  if [ "$arg" = ffmpeg ]; then
    printf '#!/bin/sh\nexit 0\n' > "$STUB_BIN/ffmpeg"
    chmod +x "$STUB_BIN/ffmpeg"
  fi
done
exit 0
SH
  export PLATFORM_OS_RELEASE_FILES="$(_file_from os-release/debian-12)"
  _curl_writes "$TINY_BYTES"
  _config_prompts
  pty_expect "which one?" "2"
  _decline_everything_else
  pty_expect "go ahead with all of that?" "y"
  pty_expect "may I install ffmpeg on this computer?" "y"
  pty_expect "may I download the listening model?" "y"
  pty_install --no-systemd

  assert_status 0 "$PTY_STATUS"
  assert_stub_called_with apt-get "apt-get install -y ffmpeg"
  assert_contains "$PTY_TRANSCRIPT" "recording what you say"
}

test_nothing_is_downloaded_onto_a_computer_that_still_cannot_hear() {
  # Programs first and weights second: 78 MB fetched onto a machine with no
  # way to record audio is 78 MB wasted, and finding that out afterwards is
  # the afternoon this ticket exists to prevent.
  _ready_machine
  stub_remove ffmpeg
  stub_command id --stdout "1000"
  stub_command sudo
  stub_command apt-get
  export PLATFORM_OS_RELEASE_FILES="$(_file_from os-release/debian-12)"
  _curl_writes "$TINY_BYTES"
  _config_prompts
  pty_expect "which one?" "2"
  _decline_everything_else
  pty_expect "go ahead with all of that?" "y"
  pty_expect "may I install ffmpeg on this computer?" "n"
  pty_install --no-systemd

  assert_status 0 "$PTY_STATUS"
  assert_stub_not_called curl
  assert_file_missing "$(_model)"
  assert_contains "$PTY_TRANSCRIPT" "so nothing has been downloaded"
  assert_file_contains "$(_state)" "step.install.voice=skipped" \
    "the answer was no, which is a finished conversation and not an errand"
  assert_not_contains "$PTY_TRANSCRIPT" "Setting up talking out loud (not finished)"
}

# ==================================================== the real thing, opt-in

test_the_smallest_model_really_downloads_and_really_verifies() {
  # The one test in this suite that touches the network, and the only way to
  # find out that the address, the size and the checksum in the table are the
  # ones the world has rather than the ones this file believes. 78 MB, the
  # smallest artifact there is, and off unless it is asked for:
  #
  #   REAL_DOWNLOAD=1 tests/run.sh voice_install
  #
  # Not named AYEAYE_ or VOICE_ on purpose: the harness scrubs both prefixes
  # out of the environment before a test runs, so a flag spelled either way
  # would be a flag that could never be set.
  [ "${REAL_DOWNLOAD:-0}" = 1 ] \
    || skip "opt-in: set REAL_DOWNLOAD=1 to fetch 78 MB for real"
  require_host_command curl
  _ready_machine
  stub_real curl
  stub_real sha256sum shasum
  _config_prompts
  pty_expect "which one?" "2"
  _decline_the_agents
  pty_expect "go ahead with all of that?" "y"
  pty_expect "may I download the listening model?" "y"
  PTY_TIMEOUT=300 pty_install --no-systemd

  assert_status 0 "$PTY_STATUS"
  assert_contains "$PTY_TRANSCRIPT" \
    "the listening model, checked against the checksum its authors published"
  assert_file_contains "$(_state)" "step.install.voice=done"
}
