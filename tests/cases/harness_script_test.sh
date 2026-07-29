# Self-tests for run_script and the interactive pty driver. The scripts under
# test are written into the sandbox so the harness is exercised against known
# behaviour rather than against install.sh.

# _fake_script <name> - write an executable script from stdin and put its path
# in $SCRIPT. It sets a variable instead of echoing one because
# `x="$(_fake_script f <<'SH')"` leaves the here-document outside the command
# substitution, which bash accepts only with a warning.
_fake_script() {
  SCRIPT="$TEST_TMPDIR/$1"
  cat > "$SCRIPT"
  chmod +x "$SCRIPT"
}

test_run_script_keeps_the_streams_apart() {
  _fake_script talker.sh <<'SH'
#!/usr/bin/env bash
printf 'to stdout\n'
printf 'to stderr\n' >&2
SH
  run_script "$SCRIPT"
  assert_eq "to stdout" "$RUN_STDOUT"
  assert_eq "to stderr" "$RUN_STDERR"
  assert_not_contains "$RUN_STDOUT" "to stderr" "a diagnostic must not be mistaken for output"
  assert_contains "$RUN_OUTPUT" "to stdout"
  assert_contains "$RUN_OUTPUT" "to stderr"
}

test_run_script_captures_a_non_zero_exit_without_ending_the_test() {
  _fake_script failer.sh <<'SH'
#!/usr/bin/env bash
echo "boom" >&2
exit 2
SH
  run_script "$SCRIPT"
  assert_status 2 "$RUN_STATUS"
  assert_eq "boom" "$RUN_STDERR"
  # Reaching this line at all is the point: a failing script is data, not a
  # test failure.
  assert_eq "still here" "still here"
}

test_run_script_passes_arguments_through() {
  _fake_script arger.sh <<'SH'
#!/usr/bin/env bash
printf 'got:%s\n' "$*"
printf 'count:%s\n' "$#"
SH
  run_script "$SCRIPT" --defaults --no-systemd
  assert_contains "$RUN_STDOUT" "got:--defaults --no-systemd"
  assert_contains "$RUN_STDOUT" "count:2"
}

test_run_script_gets_empty_stdin_by_default() {
  _fake_script reader.sh <<'SH'
#!/usr/bin/env bash
if read -r line; then printf 'read:%s\n' "$line"; else printf 'eof\n'; fi
SH
  run_script "$SCRIPT"
  assert_eq "eof" "$RUN_STDOUT" "an unprepared run must see EOF, never the terminal"
}

test_stdin_lines_feeds_a_run_and_is_then_spent() {
  _fake_script echoer.sh <<'SH'
#!/usr/bin/env bash
while read -r line; do printf 'line:%s\n' "$line"; done
SH
  stdin_lines "first" "second"
  run_script "$SCRIPT"
  assert_contains "$RUN_STDOUT" "line:first"
  assert_contains "$RUN_STDOUT" "line:second"

  run_script "$SCRIPT"
  assert_eq "" "$RUN_STDOUT" "the queue must not carry over into the next run"
}

test_run_script_runs_under_the_sandbox_and_the_stub_path() {
  stub_command tmux
  _fake_script prober.sh <<'SH'
#!/usr/bin/env bash
printf 'home:%s\n' "$HOME"
command -v tmux >/dev/null 2>&1 && printf 'tmux:present\n' || printf 'tmux:absent\n'
command -v tailscale >/dev/null 2>&1 && printf 'ts:present\n' || printf 'ts:absent\n'
tmux -V
SH
  run_script "$SCRIPT"
  assert_contains "$RUN_STDOUT" "home:$TEST_TMPDIR"
  assert_contains "$RUN_STDOUT" "tmux:present"
  assert_contains "$RUN_STDOUT" "ts:absent"
  assert_stub_called_with tmux "tmux -V" \
    "a stub records what was run, not what was merely looked up"
}

test_the_raw_capture_files_are_kept_for_diagnostics() {
  _fake_script noisy.sh <<'SH'
#!/usr/bin/env bash
printf 'one\ntwo\n'
SH
  run_script "$SCRIPT"
  assert_file_contains "$RUN_STDOUT_FILE" "two"
  assert_file_exists "$RUN_STDERR_FILE"
}

# ------------------------------------------------------------- prompt flow

test_a_piped_run_cannot_see_prompts_but_a_pty_run_can() {
  _fake_script asker.sh <<'SH'
#!/usr/bin/env bash
read -r -p "port [8911]: " REPLY || REPLY=""
[ -n "$REPLY" ] || REPLY=8911
printf 'chose:%s\n' "$REPLY"
SH
  stdin_lines "9000"
  run_script "$SCRIPT"
  assert_contains "$RUN_STDOUT" "chose:9000"
  assert_not_contains "$RUN_OUTPUT" "port [8911]" \
    "documents why the pty driver exists: a pipe never sees the prompt"

  pty_answers "9000"
  pty_run "$SCRIPT"
  assert_contains "$PTY_TRANSCRIPT" "port [8911]:" "a terminal does see the prompt"
  assert_contains "$PTY_TRANSCRIPT" "chose:9000"
  assert_status 0 "$PTY_STATUS"
}

test_the_transcript_interleaves_prompts_and_answers_in_order() {
  _fake_script wizard.sh <<'SH'
#!/usr/bin/env bash
read -r -p "bind address [127.0.0.1]: " bind
read -r -p "port [8911]: " port
read -r -p "hosts [empty]: " hosts
printf 'bind=%s port=%s hosts=%s\n' "$bind" "$port" "$hosts"
SH
  pty_answers "0.0.0.0" "9000" "box.ts.net"
  pty_run "$SCRIPT"
  assert_contains "$PTY_TRANSCRIPT" "bind address [127.0.0.1]: 0.0.0.0"
  assert_contains "$PTY_TRANSCRIPT" "port [8911]: 9000"
  assert_contains "$PTY_TRANSCRIPT" "hosts [empty]: box.ts.net"
  assert_contains "$PTY_TRANSCRIPT" "bind=0.0.0.0 port=9000 hosts=box.ts.net"

  # Order matters as much as content: an answer typed into the wrong prompt
  # would still satisfy a bare "contains" check.
  local first_prompt second_prompt
  first_prompt="$(printf '%s\n' "$PTY_TRANSCRIPT" | grep -n "bind address" | head -1 | cut -d: -f1)"
  second_prompt="$(printf '%s\n' "$PTY_TRANSCRIPT" | grep -n "port \[8911\]" | head -1 | cut -d: -f1)"
  [ "$first_prompt" -lt "$second_prompt" ] || fail "prompts appeared out of order:
$PTY_TRANSCRIPT"
}

test_an_empty_answer_takes_the_scripts_own_default() {
  _fake_script defaulter.sh <<'SH'
#!/usr/bin/env bash
read -r -p "port [8911]: " REPLY || REPLY=""
[ -n "$REPLY" ] || REPLY=8911
printf 'chose:%s\n' "$REPLY"
SH
  pty_answers ""
  pty_run "$SCRIPT"
  assert_contains "$PTY_TRANSCRIPT" "chose:8911"
}

test_pty_expect_can_wait_for_a_specific_question() {
  _fake_script confirmer.sh <<'SH'
#!/usr/bin/env bash
read -r -p "rewrite it? [n]: " a
read -r -p "enable and start it now? [y]: " b
printf 'a=%s b=%s\n' "$a" "$b"
SH
  pty_expect "rewrite it?" "y"
  pty_expect "enable and start it now?" "n"
  pty_run "$SCRIPT"
  assert_contains "$PTY_TRANSCRIPT" "a=y b=n"
}

test_the_pty_run_reports_the_scripts_exit_status() {
  _fake_script exiter.sh <<'SH'
#!/usr/bin/env bash
printf 'about to fail\n'
exit 3
SH
  pty_run "$SCRIPT"
  assert_status 3 "$PTY_STATUS"
  assert_contains "$PTY_TRANSCRIPT" "about to fail"
}

test_an_expectation_that_never_arrives_fails_fast_with_the_transcript() {
  local output status
  _fake_script quiet.sh <<'SH'
#!/usr/bin/env bash
printf 'nothing to ask\n'
SH
  pty_expect "a question that is never asked" "answer"
  output="$( (PTY_TIMEOUT=5 pty_run "$SCRIPT") 2>&1 )"
  status=$?
  assert_status 1 "$status" "a broken expectation must end the test"
  assert_contains "$output" "a question that is never asked" \
    "the diagnostic must name the expectation that was still pending"
  assert_contains "$output" "nothing to ask" "and show the transcript so far"
}

test_a_script_that_never_exits_is_cut_off_by_the_timeout() {
  local output started elapsed
  _fake_script hanger.sh <<'SH'
#!/usr/bin/env bash
printf 'hanging now\n'
sleep 60
SH
  started="$(date +%s)"
  output="$( (PTY_TIMEOUT=3 pty_run "$SCRIPT") 2>&1 )"
  elapsed=$(( $(date +%s) - started ))
  assert_contains "$output" "timed out"
  assert_contains "$output" "hanging now"
  [ "$elapsed" -lt 30 ] || fail "the pty timeout did not fire (took ${elapsed}s)"
}

test_the_interactive_run_uses_the_sandbox_and_the_stub_path_too() {
  stub_command systemctl --exit 0
  _fake_script env_prober.sh <<'SH'
#!/usr/bin/env bash
read -r -p "ready? [y]: " _answer
printf 'home:%s\n' "$HOME"
printf 'path:%s\n' "$PATH"
systemctl --user daemon-reload
command -v tailscale >/dev/null 2>&1 && printf 'ts:present\n' || printf 'ts:absent\n'
SH
  pty_answers "y"
  pty_run "$SCRIPT"
  assert_contains "$PTY_TRANSCRIPT" "home:$TEST_TMPDIR"
  assert_contains "$PTY_TRANSCRIPT" "path:$STUB_BIN"
  assert_contains "$PTY_TRANSCRIPT" "ts:absent"
  assert_stub_called_with systemctl "systemctl --user daemon-reload"
}
