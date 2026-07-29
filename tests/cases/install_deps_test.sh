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
  # Detection no longer stops the run: what is missing is reported, put in the
  # plan, and only then attempted. What must not change is the ending - a
  # machine with no tmux does not come away with a config, a key and a zero
  # exit status that all say it worked.
  stub_real python3          # tmux deliberately absent
  run_install --defaults --no-systemd
  assert_status 1 "$RUN_STATUS"
  assert_matches "$RUN_STDOUT" "MISSING[[:space:]]+tmux \\(required\\)"
  assert_contains "$RUN_STDOUT" "ayeaye still cannot run here: tmux is not on this computer."
  assert_file_missing "$XDG_CONFIG_HOME/ayeaye/env" "it must stop before writing config"
  assert_file_missing "$XDG_STATE_HOME/ayeaye/token"
}

test_what_is_missing_reaches_the_plan_before_anything_is_installed() {
  # The promise the welcome makes is that everything setup would change is
  # listed in one place and asked about before any of it happens. A stage that
  # installed two packages three stages earlier would make that a lie, so the
  # detect stage says what it found and the plan is what carries it.
  stub_real python3          # tmux deliberately absent
  run_install --defaults --no-systemd
  assert_contains "$RUN_STDOUT" "Nothing is being installed yet. Setup lists everything it would"
  assert_contains "$RUN_STDOUT" "tmux, which ayeaye cannot run without" \
    "the missing program is an entry in the plan, not a reason to stop reading"

  local plan_line attempt_line
  plan_line="$(printf '%s\n' "$RUN_STDOUT" \
    | grep -n "tmux, which ayeaye cannot run without" | head -1 | cut -d: -f1)"
  attempt_line="$(printf '%s\n' "$RUN_STDOUT" \
    | grep -n "ayeaye needs tmux before anything else here can be done" \
    | head -1 | cut -d: -f1)"
  [ -n "$attempt_line" ] || fail "nothing ever tried to install the missing program:
$RUN_STDOUT"
  [ "$plan_line" -lt "$attempt_line" ] || fail "the plan must be shown before anything is installed:
$RUN_STDOUT"
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

# --------------------------------------------------- what talking out loud needs
#
# install.sh used to sweep for ffmpeg, a transcriber and ollama itself and
# print "voice tier: missing ..." from what it found. That sweep is gone: the
# capability is described once, by lib/steps/20-hardware.sh, which names the
# job before the command and marks a missing one "not yet" rather than
# "MISSING". The assertions below are the same properties against the one
# place that now answers for them.

test_the_toolbox_names_exactly_what_is_missing() {
  _hard_deps_present
  stub_command ffmpeg
  run_install --defaults --no-systemd
  assert_matches "$RUN_STDOUT" "ok[[:space:]]+recording what you say \\(ffmpeg\\)"
  assert_matches "$RUN_STDOUT" "not yet[[:space:]]+turning what you said into words \\(whisper\\)"
  assert_matches "$RUN_STDOUT" "not yet[[:space:]]+tidying up what it heard \\(ollama\\)"
  assert_not_contains "$RUN_STDOUT" "not yet  recording what you say" \
    "what is installed must not be listed as missing"
}

test_a_complete_toolbox_is_reported_as_present() {
  _hard_deps_present
  stub_command ffmpeg
  stub_command whisper-server
  stub_command ollama
  run_install --defaults --no-systemd
  assert_matches "$RUN_STDOUT" "ok[[:space:]]+recording what you say \\(ffmpeg\\)"
  assert_matches "$RUN_STDOUT" "ok[[:space:]]+turning what you said into words \\(whisper-server\\)"
  assert_matches "$RUN_STDOUT" "ok[[:space:]]+tidying up what it heard \\(ollama\\)"
  assert_not_contains "$RUN_STDOUT" "not yet  turning what you said into words"
}

test_an_absent_toolbox_does_not_fail_the_install() {
  _hard_deps_present
  run_install --defaults --no-systemd
  assert_status 0 "$RUN_STATUS"
  assert_matches "$RUN_STDOUT" "not yet[[:space:]]+recording what you say \\(ffmpeg\\)"
  assert_matches "$RUN_STDOUT" "not yet[[:space:]]+turning what you said into words \\(whisper\\)"
  assert_contains "$RUN_STDOUT" "Nothing above is required." \
    "none of it is a reason for the install to stop"
}

test_the_summary_line_says_what_was_set_up_and_not_what_the_machine_could_do() {
  # Two ways to get this line wrong, and the milestone managed both.
  #
  # It used to be computed from a sweep of its own, so a computer that had just
  # downloaded a listening model was told it was text-only. Reading the
  # hardware tier instead fixes that and breaks it worse in the other
  # direction: the tier is a measurement of the machine, so a run that
  # installed nothing and was asked for nothing would announce that talking
  # works on a computer where the talk button is grey.
  #
  # What is asserted here is the property that survives both: on a run where
  # nothing about voice was chosen, the line must not claim talking - however
  # roomy the machine is, and whatever happens to be on PATH.
  _hard_deps_present
  run_install --defaults --no-systemd
  assert_contains "$RUN_STDOUT" "what works: typing"
  assert_not_contains "$RUN_STDOUT" "what works: typing, and talking out loud" \
    "nothing was set up, so nothing may be claimed"
  assert_contains "$RUN_STDOUT" "talking out loud was not set up"

  # And still not, with a whole transcription toolbox on PATH. The line is
  # about this run, not about this computer.
  stub_command ffmpeg
  stub_command whisper-server
  stub_command ollama
  run_install --defaults --no-systemd
  assert_contains "$RUN_STDOUT" "what works: typing"
  assert_not_contains "$RUN_STDOUT" "what works: typing, and talking out loud" \
    "a program on PATH is not a capability somebody asked for"
}

test_whisper_cpp_counts_as_a_transcriber_too() {
  _hard_deps_present
  stub_command ffmpeg
  stub_command whisper-cpp
  run_install --defaults --no-systemd
  assert_matches "$RUN_STDOUT" "ok[[:space:]]+turning what you said into words \\(whisper-cpp\\)" \
    "whisper-cpp is an accepted alternative to whisper-server"
}

test_whisper_cli_counts_as_a_transcriber_too() {
  # The name whisper.cpp's current builds, Homebrew's formula and Arch's
  # package all install. A run that told this machine it could not transcribe
  # while the app's talk button worked would be the one-name-per-tool mistake
  # in a third place.
  _hard_deps_present
  stub_command ffmpeg
  stub_command whisper-cli
  run_install --defaults --no-systemd
  assert_matches "$RUN_STDOUT" "ok[[:space:]]+turning what you said into words \\(whisper-cli\\)"
}

test_ffmpeg_alone_is_not_a_transcriber() {
  _hard_deps_present
  stub_command ffmpeg
  run_install --defaults --no-systemd
  assert_matches "$RUN_STDOUT" "ok[[:space:]]+recording what you say \\(ffmpeg\\)" \
    "anchor: the checklist was printed"
  assert_matches "$RUN_STDOUT" "not yet[[:space:]]+turning what you said into words \\(whisper\\)" \
    "recording is not transcribing, and ffmpeg alone cannot do the second one"
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
  # And offering it is where it stops. The name is a *default* for a question,
  # so an unattended run puts it in the settings file without anybody having
  # chosen anything and without a front end existing - which is precisely how
  # the closing screen came to hand somebody a phone address on a run that had
  # set nothing up at all.
  assert_not_contains "$RUN_STDOUT" "bookmark: https://my-box.tail1a2b3c.ts.net" \
    "an address in the allow list is not a way in that was set up"
  assert_not_contains "$RUN_STDOUT" "open that one on your phone." \
    "and nobody is told to open an address nothing is serving"
  assert_contains "$RUN_STDOUT" "setup did not put it there and cannot see it"
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
  pty_answers "" "" ""
  # A machine with tailscale is offered the ways in that tailscale makes
  # possible, which is a question after these three. Answered "this computer
  # only" so that this test stays about the wording of the prompt above.
  pty_expect "which one?" "2"
  pty_install --no-systemd
  assert_contains "$PTY_TRANSCRIPT" "allowed hosts (your https front) [my-box.tail1a2b3c.ts.net]:"
}

test_without_tailscale_the_prompt_changes_its_wording() {
  _hard_deps_present
  pty_answers "" "" ""
  pty_install --no-systemd
  assert_contains "$PTY_TRANSCRIPT" \
    "allowed hosts (your https front, comma separated; empty for none) [empty]:"
}
