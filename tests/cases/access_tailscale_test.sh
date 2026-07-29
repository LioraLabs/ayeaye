# Tailscale: the recommended way in, and what it does when it cannot be.
#
# Nothing here runs a real `tailscale serve`. The stub records what would have
# been run, which is the whole point: the command a machine would be given is
# asserted as a string rather than executed on the machine running the tests.

_load_access() {
  REPO="$REPO_ROOT"
  CONF_DIR="$XDG_CONFIG_HOME/ayeaye"
  ENV_FILE="$CONF_DIR/env"
  WIZARD_STATE_DIR="$XDG_STATE_HOME/ayeaye"
  WIZARD_STATE_FILE="$WIZARD_STATE_DIR/setup-state"
  WIZARD_CONSENT_LEDGER="$WIZARD_STATE_DIR/setup-consent.log"
  WIZARD_LOG_FILE="$WIZARD_STATE_DIR/setup.log"
  WIZARD_BACKUP_DIR="$WIZARD_STATE_DIR/backups"
  UNIT_DIR="$XDG_CONFIG_HOME/systemd/user"
  NO_SYSTEMD=0
  PLATFORM_OS_RELEASE_FILES=""
  export PLATFORM_OS_RELEASE_FILES
  . "$REPO_ROOT/lib/wizard.sh"
  local stage
  for stage in welcome detect report configure plan install service finish; do
    wizard_stage "$stage" "$stage"
  done
  . "$REPO_ROOT/lib/steps/50-access.sh"
  . "$REPO_ROOT/lib/steps/70-service.sh"
  wizard_state_load
  wizard_consent_reset
  wizard_plan_reset
  WIZARD_INTERACTIVE=0
  WIZARD_AUTO_CONSENT=""
}

_settings() {
  mkdir -p "$CONF_DIR"
  cat > "$ENV_FILE" <<'ENV'
# a comment somebody wrote
AYEAYE_BIND=127.0.0.1
AYEAYE_PORT=8911
AYEAYE_ALLOWED_HOSTS=
VOICE_NTFY_URL=https://ntfy.example/kept
ENV
}

setup() {
  require_host_command python3
  stub_real python3
  _load_access
  _settings
}

_tailscale_online() {
  stub_script tailscale <<SH
case "\$*" in
  "status --json") cat "$FIXTURES_DIR/tailscale/status-online" ;;
  "serve status") cat "$FIXTURES_DIR/tailscale/serve-status" ;;
  *) exit 0 ;;
esac
SH
}

# --------------------------------------------------------------- the command

test_the_serve_command_points_at_loopback_and_nothing_else() {
  assert_eq "tailscale serve --bg http://127.0.0.1:8911" \
    "$(_access_tailscale_serve_command 8911)"
  assert_eq "tailscale serve --bg http://127.0.0.1:9000" \
    "$(_access_tailscale_serve_command 9000)"
  assert_not_contains "$(_access_tailscale_serve_command 8911)" "0.0.0.0"
}

test_the_name_comes_from_the_daemon_rather_than_from_a_guess() {
  _tailscale_online
  assert_eq "my-box.tail1a2b3c.ts.net" "$(_access_tailscale_hostname)"
  assert_eq "Running" "$(_access_tailscale_state)"
}

test_a_daemon_that_is_not_signed_in_has_no_name() {
  stub_script tailscale <<SH
case "\$*" in
  "status --json") cat "$FIXTURES_DIR/tailscale/status-logged-out" ;;
  *) exit 0 ;;
esac
SH
  assert_eq "" "$(_access_tailscale_hostname)"
  assert_eq "NeedsLogin" "$(_access_tailscale_state)"
}

test_nonsense_from_the_daemon_is_not_read_as_a_name() {
  stub_script tailscale <<SH
case "\$*" in
  "status --json") cat "$FIXTURES_DIR/tailscale/status-malformed" ;;
  *) exit 0 ;;
esac
SH
  assert_eq "" "$(_access_tailscale_hostname)"
}

# ------------------------------------------------------------ setting it up

test_the_mapping_is_confirmed_with_the_daemon_rather_than_assumed() {
  # `serve --bg` exiting 0 says the command was accepted, not that anything is
  # being served. The daemon is asked what it is serving now, and its answer
  # is what the log carries.
  assert_fixture_exists tailscale/serve-status
  _tailscale_online
  stub_command curl --stdout "401"
  _access_setup_tailscale >/dev/null
  assert_stub_called_with tailscale "tailscale serve status"
  assert_contains "$(fixture_cat tailscale/serve-status)" "proxy http://127.0.0.1:8911" \
    "the fixture is what a served loopback port really looks like"
}

test_setting_it_up_runs_serve_and_writes_the_name_into_the_settings() {
  _tailscale_online
  stub_command curl --stdout "401"
  _access_setup_tailscale >/dev/null
  assert_status 0 "$?"
  assert_stub_called_with tailscale "tailscale serve --bg http://127.0.0.1:8911"
  assert_file_contains "$ENV_FILE" "AYEAYE_ALLOWED_HOSTS=my-box.tail1a2b3c.ts.net"
  assert_file_contains "$ENV_FILE" "AYEAYE_BIND=127.0.0.1"
}

test_the_settings_around_it_are_left_exactly_as_they_were() {
  _tailscale_online
  stub_command curl --stdout "401"
  _access_setup_tailscale >/dev/null
  assert_file_contains "$ENV_FILE" "# a comment somebody wrote"
  assert_file_contains "$ENV_FILE" "VOICE_NTFY_URL=https://ntfy.example/kept"
  assert_file_contains "$ENV_FILE" "AYEAYE_PORT=8911"
}

test_the_check_is_made_without_the_key() {
  # A token on a command line is a token in the log, in the process list and
  # in somebody's shell history. A 401 proves everything worth proving: the
  # address reaches ayeaye and ayeaye still wants the key.
  _tailscale_online
  stub_command curl --stdout "401"
  printf 'super-secret-token-value\n' > "$XDG_STATE_HOME/ayeaye/token"
  _access_setup_tailscale >/dev/null
  assert_stub_called_with curl "https://my-box.tail1a2b3c.ts.net/"
  assert_not_contains "$(stub_calls curl)" "super-secret-token-value"
  assert_not_contains "$(stub_calls curl)" "token="
}

test_a_daemon_nobody_has_signed_in_to_says_so_and_finishes_nothing() {
  stub_script tailscale <<SH
case "\$*" in
  "status --json") cat "$FIXTURES_DIR/tailscale/status-logged-out" ;;
  *) exit 0 ;;
esac
SH
  local out
  out="$(_access_setup_tailscale)"
  assert_status 10 "$?" "not signed in is unfinished work, not success"
  assert_contains "$out" "tailscale up"
  assert_stub_not_called curl
  assert_file_contains "$ENV_FILE" "AYEAYE_ALLOWED_HOSTS=" \
    "nothing is allowed through until there is a name to allow"
}

test_a_serve_that_will_not_take_is_reported_and_the_command_is_offered() {
  stub_script tailscale <<SH
case "\$*" in
  "status --json") cat "$FIXTURES_DIR/tailscale/status-online" ;;
  "serve --bg"*) echo "access denied" >&2; exit 1 ;;
  *) exit 0 ;;
esac
SH
  local out
  out="$(_access_setup_tailscale 2>/dev/null)"
  assert_status 10 "$?"
  assert_contains "$out" "tailscale serve --bg http://127.0.0.1:8911"
}

test_a_machine_with_no_tailscale_and_no_way_to_get_it_says_so() {
  local out
  out="$(_access_setup_tailscale)"
  assert_status 10 "$?"
  assert_contains "$out" "tailscale.com/download"
}

test_an_address_that_answers_is_the_only_thing_that_counts_as_done() {
  _tailscale_online
  stub_command curl --stdout "502"
  local out
  out="$(_access_setup_tailscale)"
  assert_status 10 "$?" "a front end with nothing behind it is not finished"
  stub_command curl --stdout "200"
  out="$(_access_setup_tailscale)"
  assert_status 0 "$?"
  assert_contains "$out" "https://my-box.tail1a2b3c.ts.net/ answers"
}
