# Self-tests for the command-stub mechanism. Later tickets assert on generated
# installer commands purely through these recordings, so the recording format
# and the PATH guarantee are the contract.

test_path_is_exactly_the_stub_directory() {
  assert_eq "$STUB_BIN" "$PATH" "PATH must contain nothing but the stub directory"
}

test_an_unstubbed_command_is_absent_whatever_the_host_has() {
  # git is certainly installed on any machine checking this repository out,
  # and must still be invisible to the code under test.
  assert_command_absent git
  assert_command_absent tailscale
  assert_command_absent tmux
}

test_stub_real_brings_a_host_command_back() {
  assert_command_absent git
  stub_real git
  command -v git >/dev/null 2>&1 || fail "stub_real must put git back on PATH"
}

test_stub_remove_takes_a_command_away_again() {
  stub_command tmux
  command -v tmux >/dev/null 2>&1 || fail "stub_command must make tmux resolvable"
  stub_remove tmux
  assert_command_absent tmux
}

test_a_stub_records_every_invocation_in_order() {
  stub_command systemctl
  systemctl --user daemon-reload
  systemctl --user enable --now ayeaye.service
  assert_stub_call_count systemctl 2
  assert_eq "systemctl --user daemon-reload" "$(stub_call systemctl 1)"
  assert_eq "systemctl --user enable --now ayeaye.service" "$(stub_call systemctl 2)"
}

test_a_stub_that_is_never_called_records_nothing() {
  stub_command loginctl
  assert_stub_not_called loginctl
  assert_eq "0" "$(stub_call_count loginctl)"
}

test_arguments_with_spaces_stay_unambiguous_in_the_recording() {
  stub_command sed_like
  sed_like -e "s|@REPO@|/home/a b/ayeaye|g" plain ""
  assert_stub_called_with sed_like "'s|@REPO@|/home/a b/ayeaye|g'"
  assert_stub_called_with sed_like "plain"
  assert_eq "sed_like -e 's|@REPO@|/home/a b/ayeaye|g' plain ''" "$(stub_call sed_like 1)"
}

test_a_stub_returns_its_scripted_exit_code() {
  stub_command failing --exit 3
  local status
  failing; status=$?
  assert_status 3 "$status"

  stub_command succeeding
  succeeding; status=$?
  assert_status 0 "$status"
}

test_a_stub_writes_scripted_stdout_and_stderr() {
  stub_command noisy --stdout "on stdout" --stderr "on stderr"
  local out err
  out="$(noisy 2>/dev/null)"
  err="$(noisy 2>&1 >/dev/null)"
  assert_eq "on stdout" "$out"
  assert_eq "on stderr" "$err"
}

test_when_rules_match_in_declaration_order_with_a_default() {
  stub_command systemctl --exit 1 --stderr "no session"
  stub_when systemctl '--user show-environment' --exit 0
  stub_when systemctl 'enable --now *' --exit 0 --stdout "enabled"

  local status out
  systemctl --user show-environment; status=$?
  assert_status 0 "$status" "the specific rule must win over the default"

  out="$(systemctl enable --now ayeaye.service)"; status=$?
  assert_status 0 "$status"
  assert_eq "enabled" "$out"

  systemctl --user status ayeaye 2>/dev/null; status=$?
  assert_status 1 "$status" "anything unmatched must fall through to the default"
}

test_a_later_rule_does_not_lose_the_recording() {
  stub_command apt-get --exit 0
  stub_when apt-get 'install *' --exit 100
  apt-get update
  apt-get install tmux
  assert_stub_call_count apt-get 2
  assert_stub_called_with apt-get "apt-get install tmux"
}

test_restubbing_resets_rules_and_recording() {
  stub_command probe --exit 7
  probe
  assert_stub_call_count probe 1
  stub_command probe --exit 0
  assert_stub_call_count probe 0 "re-stubbing must start from a clean recording"
  local status
  probe; status=$?
  assert_status 0 "$status" "re-stubbing must drop the old rules"
}

test_stub_script_gives_full_control_and_still_records() {
  stub_script fancy <<'SH'
case "$1" in
  hello) printf 'world\n'; exit 0 ;;
  *)     printf 'unknown\n' >&2; exit 4 ;;
esac
SH
  local out status
  out="$(fancy hello)"
  assert_eq "world" "$out"
  fancy other 2>/dev/null; status=$?
  assert_status 4 "$status"
  assert_stub_call_count fancy 2
  assert_stub_called_with fancy "fancy hello"
}

test_stub_output_can_come_from_a_fixture() {
  stub_command_from_fixture uname uname/darwin-arm64
  local out
  out="$(uname -a)"
  assert_contains "$out" "Darwin"
  assert_eq "$(fixture_cat uname/darwin-arm64)" "$out" \
    "a fixture-backed stub must reproduce the fixture exactly"
  assert_stub_called_with uname "uname -a"
}

test_failed_stub_assertions_show_what_was_recorded() {
  stub_command curl
  curl -fsSL https://example.invalid
  local output status
  output="$( ( assert_stub_called_with curl "wget" ) 2>&1 )"; status=$?
  assert_status 1 "$status"
  assert_contains "$output" "https://example.invalid" "the diagnostic must list the real calls"

  output="$( ( assert_stub_not_called curl ) 2>&1 )"
  assert_contains "$output" "expected no calls to"

  stub_command wget
  output="$( ( assert_stub_called wget ) 2>&1 )"; status=$?
  assert_status 1 "$status"
  assert_contains "$output" "expected at least one call to"
}

test_stubs_do_not_leak_between_tests() {
  # The counterpart of this test ran systemctl twice; a fresh sandbox means a
  # fresh stub directory and a fresh log.
  assert_command_absent systemctl
  assert_eq "0" "$(stub_call_count systemctl)"
}
