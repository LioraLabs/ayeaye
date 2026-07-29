# The four promises this whole subject exists to keep, as assertions.
#
# Every one of them is the kind of thing that is easy to write in a review note
# and easy to break six months later, so none of them is a review note:
#
#   ayeaye is never told to answer on every address this computer has
#   the key never appears in anything setup generates, or in its log
#   the address check is on in all four ways in, and is never a wildcard
#   nothing that touches a network can be reached without a person
#
# They are checked against the files a real run leaves behind rather than
# against the source, because what is on disk is what the machine obeys.

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
  ACCESS_MDNS_NAME="my-box.local"
  ACCESS_CA_WAIT=1
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

setup() {
  require_host_command python3
  stub_real python3
  # A key that could not be anything else, so finding it anywhere is proof.
  # Assigned here rather than at the top of the file: a case file defines
  # functions and nothing else, and a statement outside one runs at discovery,
  # before there is a sandbox to run it in.
  ACCESS_TEST_TOKEN="findable-token-Nx7Qb2fL"
  _load_access
  mkdir -p "$CONF_DIR" "$WIZARD_STATE_DIR" "$UNIT_DIR"
  cat > "$ENV_FILE" <<'ENV'
AYEAYE_BIND=127.0.0.1
AYEAYE_PORT=8911
AYEAYE_ALLOWED_HOSTS=
ENV
  printf '%s\n' "$ACCESS_TEST_TOKEN" > "$WIZARD_STATE_DIR/token"
  chmod 600 "$WIZARD_STATE_DIR/token"
  stub_command systemctl --exit 1
  stub_when systemctl '--user show-environment' --exit 0
  stub_when systemctl '--user daemon-reload' --exit 0
  stub_when systemctl '--user enable --now *' --exit 0
  stub_command curl --stdout "401"
  stub_command caddy
  stub_script tailscale <<SH
case "\$*" in
  "status --json") cat "$FIXTURES_DIR/tailscale/status-online" ;;
  "serve status") echo "https://my-box.tail1a2b3c.ts.net (tailnet only)" ;;
  *) exit 0 ;;
esac
SH
  stub_script ip <<'SH'
cat <<'OUT'
2: eth0    inet 192.168.1.42/24 brd 192.168.1.255 scope global eth0\       valid_lft forever
OUT
SH
}

# _set_up <mode> - a finished run of one way in, with every question answered
# yes so that as much as possible really gets written.
_set_up() {
  local mode="$1"
  WIZARD_INTERACTIVE=1
  printf 'y\ny\ny\ny\n' > "$TEST_TMPDIR/answers"
  case "$mode" in
    lan) mkdir -p "$(_access_caddy_data)/pki/authorities/local"
         printf 'PUBLIC CERTIFICATE\n' > "$(_access_caddy_data)/pki/authorities/local/root.crt" ;;
    proxy) wizard_remember answer.hosts "ayeaye.example.com" ;;
  esac
  wizard_remember answer.access.mode "$mode"
  wizard_remember answer.access.bind "127.0.0.1"
  wizard_remember answer.change_settings 1
  _access_install_step < "$TEST_TMPDIR/answers" >/dev/null 2>&1
}

# Everything a run of setup could have written, as one blob: the settings, the
# front end's own configuration, the service definitions, the audit trail and
# the log.
_everything_generated() {
  local file
  for file in "$ENV_FILE" "$(_access_caddyfile)" "$(_access_ca_file)" \
              "$(_access_ca_help)" "$(_access_proxy_caddy_file)" \
              "$(_access_proxy_nginx_file)" \
              "$UNIT_DIR/ayeaye-caddy.service" "$UNIT_DIR/ayeaye.service" \
              "$WIZARD_LOG_FILE" "$WIZARD_CONSENT_LEDGER" "$WIZARD_STATE_FILE"; do
    [ -f "$file" ] || continue
    printf '=== %s\n' "$file"
    cat "$file"
  done
  # And the definitions that would be installed on the other platform, which no
  # test on this one would otherwise ever read.
  service_render_systemd ayeaye-caddy 2>/dev/null || true
  service_render_launchd ayeaye-caddy 2>/dev/null || true
  service_render_systemd ayeaye 2>/dev/null || true
  service_render_launchd ayeaye 2>/dev/null || true
}

# --------------------------------------------- ayeaye never answers everywhere

test_no_way_in_ever_tells_ayeaye_to_answer_on_every_address() {
  local mode generated
  for mode in local tailscale lan proxy; do
    _set_up "$mode"
    generated="$(_everything_generated)"
    assert_not_contains "$generated" "0.0.0.0" \
      "$mode: nothing setup writes may name the address that means every address"
    assert_not_contains "$generated" "AYEAYE_BIND=::" \
      "$mode: nor the one that means every address there is as well"
    assert_file_contains "$ENV_FILE" "AYEAYE_BIND=127.0.0.1" \
      "$mode: ayeaye itself stays on this computer"
  done
}

test_a_front_end_always_forwards_to_loopback() {
  _set_up lan
  assert_file_contains "$(_access_caddyfile)" "reverse_proxy 127.0.0.1:8911"
  _set_up proxy
  assert_file_contains "$(_access_proxy_caddy_file)" "reverse_proxy 127.0.0.1:8911"
  assert_file_contains "$(_access_proxy_nginx_file)" "proxy_pass http://127.0.0.1:8911;"
}

# ------------------------------------------------------------- the key stays put

test_the_key_never_appears_in_anything_setup_writes() {
  local mode generated
  for mode in local tailscale lan proxy; do
    _set_up "$mode"
    generated="$(_everything_generated)"
    assert_not_contains "$generated" "$ACCESS_TEST_TOKEN" \
      "$mode: the key belongs in one file, mode 0600, and nowhere else"
  done
}

test_the_key_never_appears_in_the_log_even_with_details_on() {
  # The log is where every command that ran is written down. A check made with
  # the key on its command line would put it there, in the process list, and
  # in somebody's shell history.
  WIZARD_DETAILS=1
  local mode
  for mode in tailscale lan proxy; do
    _set_up "$mode"
  done
  assert_file_exists "$WIZARD_LOG_FILE"
  assert_file_not_contains "$WIZARD_LOG_FILE" "$ACCESS_TEST_TOKEN"
  assert_file_not_contains "$WIZARD_LOG_FILE" "token="
}

test_the_key_file_is_not_touched_by_any_of_this() {
  local mode
  for mode in local tailscale lan proxy; do
    _set_up "$mode"
  done
  assert_file_mode 600 "$WIZARD_STATE_DIR/token"
  assert_file_contains "$WIZARD_STATE_DIR/token" "$ACCESS_TEST_TOKEN"
}

# ------------------------------------------------- the address check stays on

test_every_way_in_leaves_the_address_check_switched_on() {
  # bin/ayeaye answers 403 to a Host or an Origin it was not told about, and
  # the list of what it was told is the only thing this file writes. A wildcard
  # in it, or a way to turn the check off, would undo the lot.
  local mode hosts
  for mode in local tailscale lan proxy; do
    _set_up "$mode"
    hosts="$(wizard_env_get "$ENV_FILE" AYEAYE_ALLOWED_HOSTS "")"
    case "$hosts" in
      *"*"*) fail "$mode: a wildcard in the allow list is no allow list" ;;
    esac
    case "$mode" in
      local) assert_eq "" "$hosts" "local: there is no front end to allow" ;;
      *) [ -n "$hosts" ] || fail "$mode: a way in with nothing allowed through cannot work" ;;
    esac
  done
}

test_each_way_in_allows_its_own_front_end_and_nothing_more() {
  _set_up tailscale
  assert_eq "my-box.tail1a2b3c.ts.net" "$(wizard_env_get "$ENV_FILE" AYEAYE_ALLOWED_HOSTS "")"
  _set_up lan
  assert_eq "192.168.1.42:8443,my-box.local:8443" "$(wizard_env_get "$ENV_FILE" AYEAYE_ALLOWED_HOSTS "")"
  _set_up proxy
  assert_eq "ayeaye.example.com" "$(wizard_env_get "$ENV_FILE" AYEAYE_ALLOWED_HOSTS "")"
}

test_nothing_here_can_switch_the_checks_off() {
  # There is no setting for it in bin/ayeaye and there must never be one here
  # either: the guarantee is that this file has no way to weaken them.
  local source
  source="$(cat "$REPO_ROOT/lib/steps/50-access.sh")"
  assert_not_contains "$source" "AYEAYE_TOKEN" \
    "setup does not manage the key and must not start"
  assert_not_contains "$source" "AYEAYE_ALLOWED_HOSTS=*" \
    "the allow list is a list of names, never a wildcard"
  assert_not_contains "$source" "insecure" \
    "nothing here may have a way to relax a check"
  assert_not_contains "$source" "--disable" \
    "nor a flag that turns one off"
}

test_the_generated_proxy_configuration_passes_the_address_through() {
  # A proxy that rewrote the address before handing the request over would
  # turn the check into a formality that always passes.
  _set_up proxy
  assert_file_contains "$(_access_proxy_nginx_file)" 'proxy_set_header Host $http_host;'
  assert_file_not_contains "$(_access_proxy_caddy_file)" "header_up Host"
}

# --------------------------------------------------- nothing opens by itself

test_no_way_onto_a_network_can_be_reached_without_a_person() {
  # The structural guarantee, rather than the wording of a question: exposure,
  # a firewall change and trusting a certificate are refused outright when
  # there is nobody to ask, whatever any flag says.
  WIZARD_INTERACTIVE=0
  WIZARD_AUTO_CONSENT="privileged download replace expose firewall trust"
  assert_eq "refuse" "$(wizard_consent_policy expose)"
  assert_eq "refuse" "$(wizard_consent_policy firewall)"
  assert_eq "refuse" "$(wizard_consent_policy trust)"
  wizard_may_expose "may anything else reach ayeaye?"
  assert_status 3 "$?"
}

test_choosing_a_way_in_moves_ayeaye_back_off_the_network() {
  # A machine whose settings already had ayeaye answering on the network, on
  # which somebody then chooses one of these ways in, has two things on the
  # network: the front end, and ayeaye behind it. Choosing is what makes this
  # step the owner of that address, and it puts the app back where it belongs.
  wizard_env_merge "$ENV_FILE" "AYEAYE_BIND=0.0.0.0"
  assert_file_contains "$ENV_FILE" "AYEAYE_BIND=0.0.0.0"
  _set_up tailscale
  assert_file_contains "$ENV_FILE" "AYEAYE_BIND=127.0.0.1"
  assert_file_not_contains "$ENV_FILE" "0.0.0.0"
}

test_an_address_this_step_did_not_choose_is_left_alone_and_said_out_loud() {
  # And the other side of it. Where the way in was deduced rather than chosen,
  # the address is somebody else's - answer.bind carries whatever the settings
  # file already said, since nothing asks for one - so it is not quietly
  # overwritten, and it is not quietly accepted either.
  local out
  WIZARD_INTERACTIVE=1
  wizard_remember answer.change_settings 1
  wizard_remember answer.bind "0.0.0.0"
  wizard_remember answer.hosts "ayeaye.example.com"
  stub_remove caddy 2>/dev/null || true
  stub_remove tailscale 2>/dev/null || true
  # Not in a command substitution: what this pins is what the step remembers,
  # and a subshell would remember it somewhere this test cannot see.
  printf 'y\n' > "$TEST_TMPDIR/answers"
  _access_choose_step < "$TEST_TMPDIR/answers" >"$TEST_TMPDIR/said"
  out="$(cat "$TEST_TMPDIR/said")"
  assert_eq "proxy" "$(wizard_state_get answer.access.mode "")"
  assert_eq "0.0.0.0" "$(wizard_state_get answer.bind "")" \
    "the address that was already in their settings is still theirs"
  assert_contains "$out" "answer on every address this computer"
}

test_an_unattended_choice_is_always_this_computer_only() {
  WIZARD_INTERACTIVE=0
  wizard_remember answer.change_settings 1
  _access_choose_step >/dev/null
  assert_status 0 "$?"
  assert_eq "local" "$(wizard_state_get answer.access.mode "")"
  assert_eq "" "$(wizard_state_get answer.access.bind "")" \
    "a run that chose nothing moves nothing"
  assert_contains "$(wizard_consent_ledger)" "expose	refused"
}

test_a_way_in_chosen_by_an_earlier_run_is_not_set_up_with_nobody_there() {
  # A person can choose tailscale, stop at the summary before anything
  # happens, and the next run picks up where that one stopped - skipping the
  # stage that asked. If that next run is `--defaults` in somebody's script,
  # the answer is in the state file and nobody is there to be asked again.
  wizard_remember answer.access.mode tailscale
  wizard_remember answer.change_settings 1
  WIZARD_INTERACTIVE=0
  _access_install_step >/dev/null
  assert_status 10 "$?"
  assert_not_contains "$(stub_calls tailscale)" "serve --bg"
  assert_contains "$(wizard_consent_ledger)" \
    "expose	refused	a run with nobody watching will not set up a way in chosen by an earlier one"
}

test_the_home_network_mode_never_hands_out_the_certificate_authoritys_key() {
  # The one file caddy is asked to serve over plain http is the certificate.
  # The key that signs with it lives beside the original and is never copied
  # into the directory that is handed out.
  mkdir -p "$(_access_caddy_data)/pki/authorities/local"
  printf 'PUBLIC CERTIFICATE\n' > "$(_access_caddy_data)/pki/authorities/local/root.crt"
  printf 'PRIVATE KEY\n' > "$(_access_caddy_data)/pki/authorities/local/root.key"
  _set_up lan
  assert_file_exists "$(_access_ca_file)"
  assert_eq "" "$(ls "$(_access_ca_dir)" | grep -v '^ayeaye-ca.crt$')"
  assert_file_not_contains "$(_access_ca_file)" "PRIVATE KEY"
  local served
  served="$(_access_ca_dir)"
  case "$served" in
    "$(_access_caddy_data)"*) fail "the served directory is inside the one holding the key" ;;
  esac
}
