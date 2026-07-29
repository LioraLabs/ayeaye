# Changing your mind about how you reach ayeaye.
#
# The failure this pins is a quiet one: setup run twice leaving two ways in,
# one of them forgotten - a caddy still answering on the wifi after somebody
# moved to tailscale, with a certificate on a phone nobody remembers
# installing. Choosing a way in takes the previous one down first.

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
  _load_access
  mkdir -p "$CONF_DIR"
  cat > "$ENV_FILE" <<'ENV'
AYEAYE_BIND=127.0.0.1
AYEAYE_PORT=8911
AYEAYE_ALLOWED_HOSTS=
ENV
  stub_command systemctl --exit 1
  stub_when systemctl '--user show-environment' --exit 0
  stub_when systemctl '--user daemon-reload' --exit 0
  stub_when systemctl '--user disable --now *' --exit 0
  stub_when systemctl '--user enable --now *' --exit 0
  stub_script tailscale <<SH
case "\$*" in
  "status --json") cat "$FIXTURES_DIR/tailscale/status-online" ;;
  "serve status") echo "https://my-box.tail1a2b3c.ts.net (tailnet only)" ;;
  *) exit 0 ;;
esac
SH
  stub_command curl --stdout "401"
  # A person is present. Setting up a way in is refused outright when nobody
  # is, which is what access_security_test pins - so a test about switching
  # between them has to be a run somebody is watching.
  WIZARD_INTERACTIVE=1
}

# What a finished run of the home-network mode leaves behind.
_pretend_lan_is_installed() {
  mkdir -p "$(_access_ca_dir)" "$UNIT_DIR"
  printf 'lan front\n' > "$(_access_caddyfile)"
  printf 'PUBLIC CERTIFICATE\n' > "$(_access_ca_file)"
  printf 'how to trust it\n' > "$(_access_ca_help)"
  printf 'a unit\n' > "$UNIT_DIR/ayeaye-caddy.service"
  wizard_env_merge "$ENV_FILE" "AYEAYE_ALLOWED_HOSTS=192.168.1.42,my-box.local"
  wizard_remember step.install.access.installed lan
}

_pretend_proxy_is_installed() {
  mkdir -p "$CONF_DIR"
  printf 'caddy snippet\n' > "$(_access_proxy_caddy_file)"
  printf 'nginx snippet\n' > "$(_access_proxy_nginx_file)"
  wizard_env_merge "$ENV_FILE" "AYEAYE_ALLOWED_HOSTS=old.example.com"
  wizard_remember step.install.access.installed proxy
}

# ------------------------------------------------------- home network away

test_moving_from_the_home_network_to_tailscale_takes_the_old_front_end_down() {
  _pretend_lan_is_installed
  wizard_remember answer.access.mode tailscale
  _access_install_step </dev/null >/dev/null
  assert_file_missing "$(_access_caddyfile)"
  assert_file_missing "$(_access_ca_file)"
  assert_file_missing "$UNIT_DIR/ayeaye-caddy.service"
  assert_stub_called_with systemctl "systemctl --user disable --now ayeaye-caddy.service"
  assert_stub_called_with tailscale "tailscale serve --bg http://127.0.0.1:8911"
}

test_the_phone_is_told_about_the_part_only_it_can_undo() {
  _pretend_lan_is_installed
  wizard_remember answer.access.mode tailscale
  local out
  out="$(_access_install_step </dev/null)"
  assert_contains "$out" "Remove it from any"
}

test_the_addresses_allowed_through_are_replaced_and_never_added_to() {
  # A stale name left in the allow list is a name an old front end could still
  # use. The list is what this mode needs and nothing else.
  _pretend_lan_is_installed
  wizard_remember answer.access.mode tailscale
  wizard_remember answer.access.bind "127.0.0.1"
  _access_install_step </dev/null >/dev/null
  assert_file_contains "$ENV_FILE" "AYEAYE_ALLOWED_HOSTS=my-box.tail1a2b3c.ts.net"
  assert_file_not_contains "$ENV_FILE" "192.168.1.42"
  assert_file_not_contains "$ENV_FILE" "my-box.local"
}

test_moving_to_this_computer_only_clears_the_allow_list_as_well() {
  _pretend_lan_is_installed
  wizard_remember answer.access.mode local
  wizard_remember answer.access.bind "127.0.0.1"
  _access_install_step </dev/null >/dev/null
  assert_status 0 "$?"
  assert_file_contains "$ENV_FILE" "AYEAYE_ALLOWED_HOSTS="
  assert_file_not_contains "$ENV_FILE" "my-box.local"
  assert_file_missing "$(_access_caddyfile)"
}

# ---------------------------------------------------------- tailscale away

test_moving_off_tailscale_takes_its_address_away_again() {
  wizard_remember step.install.access.installed tailscale
  wizard_remember answer.access.mode local
  _access_install_step </dev/null >/dev/null
  assert_stub_called_with tailscale "tailscale serve --https=443 off"
}

test_the_way_off_tailscale_is_scoped_rather_than_a_reset() {
  # `serve reset` would throw away anything else the person serves from this
  # machine. Only the mapping setup made is taken away.
  wizard_remember step.install.access.installed tailscale
  wizard_remember answer.access.mode local
  _access_install_step </dev/null >/dev/null
  assert_not_contains "$(stub_calls tailscale)" "serve reset"
}

# -------------------------------------------------------------- proxy away

test_moving_off_an_existing_proxy_removes_what_setup_wrote_and_nothing_else() {
  _pretend_proxy_is_installed
  wizard_remember answer.access.mode local
  wizard_remember answer.access.bind "127.0.0.1"
  local out
  out="$(_access_install_step </dev/null)"
  assert_file_missing "$(_access_proxy_caddy_file)"
  assert_file_missing "$(_access_proxy_nginx_file)"
  assert_contains "$out" "Your own"
  assert_file_contains "$ENV_FILE" "AYEAYE_ALLOWED_HOSTS="
}

# ------------------------------------------------------------ no change

test_staying_where_you_are_takes_nothing_down() {
  _pretend_lan_is_installed
  wizard_remember answer.access.mode lan
  wizard_remember answer.change_settings 1
  # The mode is unchanged, so nothing is removed before it is set up again.
  # It is re-run from scratch, which is what makes setup repairable.
  stub_command caddy
  stub_script ip <<'SH'
cat <<'OUT'
2: eth0    inet 192.168.1.42/24 brd 192.168.1.255 scope global eth0\       valid_lft forever
OUT
SH
  mkdir -p "$(_access_caddy_data)/pki/authorities/local"
  printf 'PUBLIC CERTIFICATE\n' > "$(_access_caddy_data)/pki/authorities/local/root.crt"
  WIZARD_INTERACTIVE=1
  printf 'y\ny\nn\n' > "$TEST_TMPDIR/answers"
  _access_install_step < "$TEST_TMPDIR/answers" >/dev/null
  assert_not_contains "$(stub_calls systemctl)" "disable" \
    "the front end that is already there is repaired, not taken down"
  assert_file_exists "$(_access_caddyfile)"
}

test_a_run_that_kept_its_settings_changes_nothing_about_the_way_in() {
  _pretend_lan_is_installed
  wizard_remember answer.change_settings 0
  wizard_remember answer.access.mode tailscale
  _access_install_step </dev/null >/dev/null
  assert_status 11 "$?"
  assert_file_exists "$(_access_caddyfile)" "nothing is taken down behind the person's back"
  assert_not_contains "$(stub_calls tailscale)" "serve --bg"
}

# ------------------------------------------- what is on the disk, not just what
#                                             setup remembers
#
# `--fresh` is a supported thing to want, and it throws away everything earlier
# runs wrote down - including which way in is currently set up. A run that then
# believed nothing was there would leave a caddy answering on the wifi while
# telling the person their phone cannot reach ayeaye.

test_a_front_end_on_the_disk_is_found_after_setup_forgot_about_it() {
  _pretend_lan_is_installed
  wizard_state_unset step.install.access.installed
  assert_eq "lan" "$(_access_previous_mode)"

  wizard_remember answer.access.mode tailscale
  _access_install_step </dev/null >/dev/null
  assert_file_missing "$(_access_caddyfile)" \
    "a forgotten front end is still a front end"
  assert_stub_called_with systemctl "systemctl --user disable --now ayeaye-caddy.service"
}

test_a_definition_on_the_disk_counts_even_with_no_settings_beside_it() {
  mkdir -p "$UNIT_DIR"
  printf 'a unit\n' > "$UNIT_DIR/ayeaye-caddy.service"
  assert_eq "lan" "$(_access_previous_mode)"
}

test_generated_proxy_configuration_on_the_disk_is_found_too() {
  _pretend_proxy_is_installed
  wizard_state_unset step.install.access.installed
  assert_eq "proxy" "$(_access_previous_mode)"
}

test_a_tailscale_serving_this_computers_ayeaye_is_found() {
  stub_script tailscale <<SH
case "\$*" in
  "serve status") cat "$FIXTURES_DIR/tailscale/serve-status" ;;
  *) exit 0 ;;
esac
SH
  assert_eq "tailscale" "$(_access_previous_mode)"
}

test_a_tailscale_serving_something_else_is_left_alone() {
  # Somebody serving their own program on loopback has not set ayeaye up on
  # tailscale, and reading it as though they had would end with setup turning
  # their serve off to make room.
  stub_script tailscale <<'SH'
case "$*" in
  "serve status") printf 'https://my-box.tail1a2b3c.ts.net (tailnet only)\n|-- / proxy http://127.0.0.1:3000\n' ;;
  *) exit 0 ;;
esac
SH
  assert_eq "" "$(_access_previous_mode)"
  wizard_remember answer.access.mode local
  _access_install_step </dev/null >/dev/null
  assert_not_contains "$(stub_calls tailscale)" "serve --https=443 off" \
    "somebody else's serve is not setup's to take away"
}

test_a_mode_that_was_chosen_and_never_installed_leaves_nothing_to_undo() {
  # A run can record the choice and stop before anything reaches the disk.
  # Saying a certificate has been removed then would send somebody looking for
  # a profile on their phone that was never installed.
  wizard_remember step.install.access.installed lan
  wizard_remember answer.access.mode local
  local out
  out="$(_access_install_step </dev/null)"
  assert_not_contains "$out" "Remove it from any" \
    "nothing was made, so there is nothing to say about a phone"
  assert_not_contains "$(wizard_consent_ledger)" "replace	granted	removed"
}
