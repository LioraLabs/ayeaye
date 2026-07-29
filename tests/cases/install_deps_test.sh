# What install.sh detects today, and what it says about it.
#
# Nothing here is installed, probed for real, or started: the dependency tier
# is whatever the stub directory says it is, which is the whole point of the
# stub PATH. These pin the semantics the platform adapters are about to
# replace.
#
# The report is column-aligned today. The alignment is not pinned - a rewrite is
# free to lay it out differently - so the assertions match the name paired with
# its verdict and let the whitespace vary.

# The minimum for install.sh to get past its hard-dependency gate. python3 is
# the real one because the installer genuinely runs python for templating and
# for the token; tmux is only ever looked up, never executed.
_hard_deps_present() {
  require_host_command python3
  stub_command tmux
  stub_real python3
}

test_both_hard_dependencies_present_are_reported_as_ok() {
  _hard_deps_present
  run_install --defaults --no-systemd
  assert_status 0 "$RUN_STATUS"
  assert_matches "$RUN_STDOUT" "ok[[:space:]]+tmux"
  assert_matches "$RUN_STDOUT" "ok[[:space:]]+python3"
}

test_a_missing_hard_dependency_stops_the_install() {
  stub_real python3          # tmux deliberately absent
  run_install --defaults --no-systemd
  assert_status 1 "$RUN_STATUS"
  assert_matches "$RUN_STDOUT" "MISSING[[:space:]]+tmux \\(required\\)"
  assert_contains "$RUN_STDOUT" "install the missing packages and re-run."
  assert_file_missing "$XDG_CONFIG_HOME/ayeaye/env" "it must stop before writing config"
  assert_file_missing "$XDG_STATE_HOME/ayeaye/token"
}

test_every_missing_hard_dependency_is_named_before_giving_up() {
  # Neither tmux nor python3 exists in this sandbox.
  run_install --defaults --no-systemd
  assert_status 1 "$RUN_STATUS"
  assert_matches "$RUN_STDOUT" "MISSING[[:space:]]+tmux \\(required\\)"
  assert_matches "$RUN_STDOUT" "MISSING[[:space:]]+python3 \\(required\\)"
}

test_the_repo_it_will_install_from_is_announced() {
  _hard_deps_present
  run_install --defaults --no-systemd
  assert_contains "$RUN_STDOUT" "repo: $REPO_ROOT"
}

test_a_missing_tailscale_is_a_note_not_a_failure() {
  _hard_deps_present
  run_install --defaults --no-systemd
  assert_status 0 "$RUN_STATUS"
  assert_matches "$RUN_STDOUT" "note[[:space:]]+tailscale not found"
  assert_contains "$RUN_STDOUT" "another way to serve https to the phone"
}

test_a_present_tailscale_produces_no_note() {
  _hard_deps_present
  stub_command_from_fixture tailscale tailscale/status-online
  run_install --defaults --no-systemd
  # Anchored on something positive: an installer that had done nothing at all
  # would also fail to print the note.
  assert_status 0 "$RUN_STATUS"
  assert_stub_called_with tailscale "tailscale status --json"
  assert_not_contains "$RUN_STDOUT" "tailscale not found"
}

test_the_voice_tier_names_exactly_what_is_missing() {
  _hard_deps_present
  stub_command ffmpeg
  run_install --defaults --no-systemd
  assert_contains "$RUN_STDOUT" "voice tier: missing whisper-server ollama"
  assert_not_contains "$RUN_STDOUT" "missing ffmpeg"
}

test_a_complete_voice_tier_is_reported_as_present() {
  _hard_deps_present
  stub_command ffmpeg
  stub_command whisper-server
  stub_command ollama
  run_install --defaults --no-systemd
  assert_contains "$RUN_STDOUT" "voice tier: all present (ffmpeg, whisper-server, ollama)"
  assert_not_contains "$RUN_STDOUT" "voice tier: missing"
}

test_an_absent_voice_tier_does_not_fail_the_install() {
  _hard_deps_present
  run_install --defaults --no-systemd
  assert_status 0 "$RUN_STATUS"
  assert_contains "$RUN_STDOUT" "voice tier: missing ffmpeg whisper-server ollama"
  assert_contains "$RUN_STDOUT" "text-only mode works without them"
}

test_the_summary_tier_follows_what_was_probed() {
  _hard_deps_present
  run_install --defaults --no-systemd
  assert_matches "$RUN_STDOUT" "tier[[:space:]]*: text-only \\(probed live"

  # ffmpeg plus a transcriber is what earns the voice suffix.
  stub_command ffmpeg
  stub_command whisper-server
  run_install --defaults --no-systemd
  assert_matches "$RUN_STDOUT" "tier[[:space:]]*: text-only \\+voice"
}

test_whisper_cpp_counts_as_a_transcriber_too() {
  _hard_deps_present
  stub_command ffmpeg
  stub_command whisper-cpp
  run_install --defaults --no-systemd
  assert_contains "$RUN_STDOUT" "+voice" \
    "whisper-cpp is an accepted alternative to whisper-server in the tier line"
  assert_contains "$RUN_STDOUT" "voice tier: missing whisper-server ollama" \
    "but the tier *checklist* only ever looks for whisper-server"
}

test_ffmpeg_alone_is_not_a_voice_tier() {
  _hard_deps_present
  stub_command ffmpeg
  run_install --defaults --no-systemd
  assert_matches "$RUN_STDOUT" "tier[[:space:]]*: text-only" "anchor: the tier line was printed"
  assert_not_contains "$RUN_STDOUT" "+voice"
}

# ------------------------------------------------------- tailscale discovery

test_a_running_tailscale_supplies_the_allowed_host_default() {
  _hard_deps_present
  stub_command_from_fixture tailscale tailscale/status-online
  run_install --defaults --no-systemd
  assert_status 0 "$RUN_STATUS"
  assert_stub_called_with tailscale "tailscale status --json"
  assert_file_contains "$XDG_CONFIG_HOME/ayeaye/env" \
    "AYEAYE_ALLOWED_HOSTS=my-box.tail1a2b3c.ts.net" \
    "the node name becomes the allowed host, with its trailing dot stripped"
  assert_not_contains "$RUN_STDOUT" "my-box.tail1a2b3c.ts.net./" "the trailing dot must be gone"
  assert_contains "$RUN_STDOUT" "https://my-box.tail1a2b3c.ts.net/?token="
}

test_a_logged_out_tailscale_falls_back_to_no_allowed_hosts() {
  _hard_deps_present
  stub_command_from_fixture tailscale tailscale/status-logged-out
  run_install --defaults --no-systemd
  assert_status 0 "$RUN_STATUS"
  assert_file_contains "$XDG_CONFIG_HOME/ayeaye/env" "AYEAYE_ALLOWED_HOSTS="
  assert_not_contains "$RUN_STDOUT" "behind tailscale serve"
}

test_unparseable_tailscale_output_is_survived_not_propagated() {
  _hard_deps_present
  stub_command_from_fixture tailscale tailscale/status-malformed
  run_install --defaults --no-systemd
  assert_status 0 "$RUN_STATUS" "a broken probe must not abort the install"
  assert_not_contains "$RUN_STDERR" "Traceback" "the python failure must stay swallowed"
  assert_file_contains "$XDG_CONFIG_HOME/ayeaye/env" "AYEAYE_ALLOWED_HOSTS="
}

test_a_failing_tailscale_command_is_survived_too() {
  _hard_deps_present
  stub_command tailscale --exit 1 --stderr "failed to connect to local tailscaled"
  run_install --defaults --no-systemd
  assert_status 0 "$RUN_STATUS"
  assert_stub_called_with tailscale "tailscale status --json" "anchor: it really did probe"
  assert_file_contains "$XDG_CONFIG_HOME/ayeaye/env" "AYEAYE_ALLOWED_HOSTS=" \
    "anchor: it really did carry on and write the config"
  assert_not_contains "$RUN_STDERR" "failed to connect" "the probe's noise must not reach the user"
}

test_the_allowed_hosts_prompt_offers_the_tailscale_name() {
  _hard_deps_present
  stub_command_from_fixture tailscale tailscale/status-online
  pty_answers "" "" "" ""
  # A machine with tailscale is offered the ways in that tailscale makes
  # possible, which is a question after these four. Answered "this computer
  # only" so that this test stays about the wording of the prompt above.
  pty_expect "which one?" "2"
  pty_install --no-systemd
  assert_contains "$PTY_TRANSCRIPT" "allowed hosts (your https front) [my-box.tail1a2b3c.ts.net]:"
}

test_without_tailscale_the_prompt_changes_its_wording() {
  _hard_deps_present
  pty_answers "" "" "" ""
  pty_install --no-systemd
  assert_contains "$PTY_TRANSCRIPT" \
    "allowed hosts (your https front, comma separated; empty for none) [empty]:"
}
