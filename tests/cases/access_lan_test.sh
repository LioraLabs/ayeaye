# The https front for a home network: what is generated, and what is not.
#
# The renderer is a pure function - it writes to standard output and touches
# nothing - so most of this drives it directly rather than installing anything.
# Two properties are worth more than all the others put together and are
# asserted here rather than reviewed:
#
#   the only thing on the network forwards to 127.0.0.1, never to 0.0.0.0
#   the directory handed out over plain http holds the certificate and cannot
#   hold the key beside it

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
  _load_access
}

_caddyfile() { access_render_caddyfile 8911 192.168.1.42 my-box.local; }

# ------------------------------------------------------------- what it says

test_the_only_thing_on_the_network_forwards_to_this_computer() {
  local text
  text="$(_caddyfile)"
  assert_contains "$text" "reverse_proxy 127.0.0.1:8911" \
    "ayeaye stays on loopback and caddy is the only thing reachable"
  assert_not_contains "$text" "0.0.0.0" \
    "nothing generated here may ever name the address that means every address"
  assert_not_contains "$text" "reverse_proxy 192.168.1.42" \
    "forwarding to the network address would put ayeaye on the network twice"
}

test_the_https_site_is_the_lan_address_and_the_mdns_name_on_a_high_port() {
  local text
  text="$(_caddyfile)"
  assert_contains "$text" "https://192.168.1.42:8443, https://my-box.local:8443 {"
  assert_contains "$text" "tls internal" \
    "the certificate comes from the authority made on this machine"
}

test_a_machine_with_no_name_still_gets_a_site() {
  local text
  text="$(access_render_caddyfile 8911 192.168.1.42)"
  assert_contains "$text" "https://192.168.1.42:8443 {"
  assert_not_contains "$text" ", https://:8443" \
    "an absent name must not become an empty one"
}

test_the_admin_endpoint_is_switched_off() {
  # caddy would otherwise listen on this machine for commands and ask nothing
  # of whoever sends them. On a machine running something that approves agent
  # actions that is not a detail.
  assert_contains "$(_caddyfile)" "admin off"
}

test_the_front_end_listens_on_one_interface_rather_than_all_of_them() {
  # A site address in caddy is a name to match, not an address to listen on.
  # Without the bind directive caddy opens both ports on every address this
  # computer has - checked against a real caddy, which reported 0.0.0.0:8443
  # without it and 127.0.0.1:8443 with it. On a machine with a public address
  # as well as a private one, that is the difference between a listener on the
  # home network and a listener on the internet.
  local text
  text="$(_caddyfile)"
  assert_contains "$text" "bind 192.168.1.42"
  assert_eq "2" "$(printf '%s\n' "$text" | grep -c '^	bind 192.168.1.42$')" \
    "both sites, not just the https one"
}

test_nothing_is_asked_to_listen_on_a_port_that_needs_a_password() {
  # caddy adds an http-to-https redirect on port 80 unless told not to, and
  # port 80 needs root. Nothing in this mode runs as root, and caddy that
  # cannot open a listener it was told to open does not start at all - so the
  # whole front end would be lost to a convenience nobody asked for. Checked
  # against a real caddy: without this line the adapted configuration says
  # "enabling automatic HTTP->HTTPS redirects".
  local text
  text="$(_caddyfile)"
  assert_contains "$text" "auto_https disable_redirects"
  assert_not_contains "$text" ":80 " "nothing here may want a privileged port"
  assert_not_contains "$text" ":443" \
    "and not the other one a certificate would normally want"
}

test_the_certificate_is_handed_out_from_a_directory_that_cannot_hold_the_key() {
  local text root pki
  text="$(_caddyfile)"
  root="$(_access_ca_dir)"
  pki="$(_access_caddy_data)"
  assert_contains "$text" "root * \"$root\""
  assert_ne "$root" "$pki" "the served directory must not be caddy's own"
  case "$root" in
    "$pki"*) fail "the served directory is inside the one holding the private key: $root" ;;
  esac
  assert_contains "$text" "@certificate path /ayeaye-ca.crt"
  assert_contains "$text" "respond \"this address only hands out /ayeaye-ca.crt\" 404" \
    "everything except the one file gets nothing"
}

test_the_plain_http_site_cannot_reach_ayeaye() {
  # It exists for one public file. A phone with no certificate yet cannot
  # check one, which is the only reason there is an unencrypted listener at
  # all - and it must never become a second way in.
  local text after
  text="$(_caddyfile)"
  after="${text#*http://192.168.1.42:8480}"
  assert_not_contains "$after" "reverse_proxy" \
    "the certificate endpoint must not forward anything anywhere"
}

test_the_renderer_refuses_without_an_address() {
  access_render_caddyfile 8911 >/dev/null 2>&1
  assert_status 2 "$?" "no address is a caller bug, not an empty file"
}

# --------------------------------------------------------- the allowed hosts

test_the_allowed_hosts_are_the_two_names_the_front_end_answers_for() {
  # With the port. bin/ayeaye matches an entry exactly before it tries it with
  # the port stripped, so a bare address here would also accept requests
  # claiming to come from the plain-http certificate endpoint on 8480.
  assert_eq "192.168.1.42:8443,my-box.local:8443" \
    "$(_access_lan_hosts 192.168.1.42 my-box.local)"
  assert_eq "192.168.1.42:8443" "$(_access_lan_hosts 192.168.1.42 "")"
}

# ------------------------------------------------------- the phone walkthrough

test_both_phones_get_real_steps() {
  local help
  help="$(_access_mobile_ca_help "http://192.168.1.42:8480/ayeaye-ca.crt" \
    "https://my-box.local:8443/")"
  assert_contains "$help" "ON AN IPHONE OR IPAD"
  assert_contains "$help" "ON AN ANDROID PHONE"
  assert_contains "$help" "http://192.168.1.42:8480/ayeaye-ca.crt" \
    "the address the phone opens is the certificate, never the app"
  assert_contains "$help" "https://my-box.local:8443/"
}

test_the_ios_step_everybody_misses_is_in_there() {
  # A certificate installed on an iPhone and not switched on under Certificate
  # Trust Settings looks exactly like one that does not work. A walkthrough
  # without this step sends people back to the beginning.
  local help
  help="$(_access_mobile_ca_help "http://x/ayeaye-ca.crt" "https://y/")"
  assert_contains "$help" "Certificate Trust Settings"
  assert_contains "$help" "Enable full trust for root certificates"
  assert_contains "$help" "Profile Downloaded"
}

test_the_android_steps_name_the_screen_and_the_warning() {
  local help
  help="$(_access_mobile_ca_help "http://x/ayeaye-ca.crt" "https://y/")"
  assert_contains "$help" "Encryption & credentials"
  assert_contains "$help" "CA certificate"
  assert_contains "$help" "Install anyway" \
    "android's own wording, so the button is findable"
  assert_contains "$help" "ayeaye-ca.crt"
}

test_both_phones_are_told_how_to_undo_it() {
  local help
  help="$(_access_mobile_ca_help "http://x/ayeaye-ca.crt" "https://y/")"
  assert_contains "$help" "Remove Profile"
  assert_contains "$help" "Trusted credentials"
}

# --------------------------------------------------------- exporting the CA

_pretend_caddy_made_a_certificate() {
  local dir
  dir="$(_access_caddy_data)/pki/authorities/local"
  mkdir -p "$dir"
  printf 'PUBLIC CERTIFICATE\n' > "$dir/root.crt"
  printf 'PRIVATE KEY, NEVER COPIED\n' > "$dir/root.key"
}

test_the_certificate_is_copied_out_and_the_key_is_not() {
  ACCESS_CA_WAIT=1
  _pretend_caddy_made_a_certificate
  _access_export_ca
  assert_status 0 "$?"
  assert_file_exists "$(_access_ca_file)"
  assert_file_contains "$(_access_ca_file)" "PUBLIC CERTIFICATE"
  assert_file_missing "$(_access_ca_dir)/root.key" \
    "the authority's private key must never leave the directory caddy keeps it in"
  assert_eq "" "$(ls "$(_access_ca_dir)" | grep -v '^ayeaye-ca.crt$')" \
    "the directory caddy hands out over http holds exactly one file"
}

test_the_exported_certificate_is_readable_because_it_is_public() {
  ACCESS_CA_WAIT=1
  _pretend_caddy_made_a_certificate
  _access_export_ca
  assert_file_mode 644 "$(_access_ca_file)"
}

test_no_certificate_yet_is_reported_rather_than_assumed() {
  ACCESS_CA_WAIT=0
  _access_export_ca
  assert_status 1 "$?" "a certificate that is not there is not a certificate"
  assert_file_missing "$(_access_ca_file)"
}

# ------------------------------------------------------------ finding the lan

test_a_private_address_is_found_and_a_public_one_is_not() {
  stub_script ip <<'SH'
cat <<'OUT'
1: lo    inet 127.0.0.1/8 scope host lo\       valid_lft forever
2: eth0    inet 192.168.1.42/24 brd 192.168.1.255 scope global eth0\       valid_lft forever
3: wan0    inet 81.2.3.4/24 brd 81.2.3.255 scope global wan0\       valid_lft forever
OUT
SH
  assert_eq "192.168.1.42" "$(_access_lan_ip)"
  assert_eq "192.168.1.0/24" "$(_access_lan_subnet)"
}

test_a_machine_on_no_private_network_offers_nothing() {
  stub_script ip <<'SH'
cat <<'OUT'
1: lo    inet 127.0.0.1/8 scope host lo\       valid_lft forever
2: wan0    inet 81.2.3.4/24 brd 81.2.3.255 scope global wan0\       valid_lft forever
OUT
SH
  assert_eq "" "$(_access_lan_ip)"
  assert_eq "" "$(_access_lan_subnet)"
  _access_lan_possible
  assert_status 1 "$?" "with no home network there is nothing to put a certificate on"
}

test_a_subnet_that_cannot_be_written_down_safely_is_not_written_down() {
  # A prefix that stops in the middle of an octet would need arithmetic to
  # turn into a network address, and a firewall rule that is subtly wrong is
  # worse than no rule at all.
  stub_script ip <<'SH'
cat <<'OUT'
2: eth0    inet 192.168.1.42/26 brd 192.168.1.63 scope global eth0\       valid_lft forever
OUT
SH
  assert_eq "192.168.1.42" "$(_access_lan_ip)"
  assert_eq "" "$(_access_lan_subnet)"
}

# ------------------------------------------------------ setting the whole thing up
#
# The step is driven directly rather than through a whole install, and every
# program it would talk to is a recording. Nothing here starts a real server,
# opens a real firewall or trusts a real certificate.

_lan_machine() {
  require_host_command python3
  stub_real python3
  stub_command caddy
  stub_command systemctl --exit 1
  stub_when systemctl '--user show-environment' --exit 0
  stub_when systemctl '--user daemon-reload' --exit 0
  stub_when systemctl '--user enable --now *' --exit 0
  stub_script ip <<'SH'
cat <<'OUT'
2: eth0    inet 192.168.1.42/24 brd 192.168.1.255 scope global eth0\       valid_lft forever
OUT
SH
  mkdir -p "$CONF_DIR"
  cat > "$ENV_FILE" <<'ENV'
AYEAYE_BIND=127.0.0.1
AYEAYE_PORT=8911
AYEAYE_ALLOWED_HOSTS=
ENV
  _pretend_caddy_made_a_certificate
  ACCESS_CA_WAIT=1
  WIZARD_INTERACTIVE=1
}

# The answers a person would type, in order: the certificate, then the
# firewall. Anything this run does not ask for is simply not read.
_lan_answers() {
  local file="$TEST_TMPDIR/answers" line
  : > "$file"
  for line in "$@"; do
    printf '%s\n' "$line" >> "$file"
  done
  printf '%s' "$file"
}

test_the_whole_home_network_mode_writes_starts_and_checks_itself() {
  _lan_machine
  stub_command curl --stdout "401"
  local out
  out="$(_access_setup_lan < "$(_lan_answers y n)")"
  assert_status 0 "$?"
  assert_file_exists "$(_access_caddyfile)"
  assert_file_contains "$(_access_caddyfile)" "reverse_proxy 127.0.0.1:8911"
  assert_file_exists "$(_access_ca_file)"
  assert_file_exists "$XDG_CONFIG_HOME/systemd/user/ayeaye-caddy.service"
  assert_stub_called_with caddy "caddy validate --adapter caddyfile"
  assert_stub_called_with systemctl "systemctl --user enable --now ayeaye-caddy.service"
  assert_file_contains "$ENV_FILE" "AYEAYE_ALLOWED_HOSTS=192.168.1.42:8443,"
  assert_file_contains "$ENV_FILE" "AYEAYE_BIND=127.0.0.1"
  assert_contains "$out" "ON AN IPHONE OR IPAD"
}

test_saying_no_to_the_certificate_writes_nothing_at_all() {
  # The certificate is what this mode is, and making one is a decision about
  # what somebody's phones will believe. Refusing it has to leave the machine
  # exactly as it was.
  _lan_machine
  local out
  out="$(_access_setup_lan < "$(_lan_answers n)")"
  assert_status 11 "$?" "refused is finished business, not a failure"
  assert_file_missing "$(_access_caddyfile)"
  assert_file_missing "$(_access_ca_file)"
  assert_file_missing "$XDG_CONFIG_HOME/systemd/user/ayeaye-caddy.service"
  assert_stub_not_called systemctl
  assert_contains "$out" "nothing has been made"
  assert_contains "$(wizard_consent_ledger)" "trust	refused	"
}

test_no_flag_can_make_that_certificate() {
  # trust is one of the three kinds --yes is not allowed to answer.
  _lan_machine
  WIZARD_INTERACTIVE=0
  WIZARD_AUTO_CONSENT="privileged download replace trust expose firewall"
  _access_setup_lan >/dev/null
  assert_status 11 "$?"
  assert_file_missing "$(_access_caddyfile)"
  assert_contains "$(wizard_consent_ledger)" "trust	refused	"
}

test_a_caddy_that_rejects_the_settings_starts_nothing() {
  _lan_machine
  stub_command caddy --exit 1 --stderr "adapting config: unrecognized directive"
  local out
  out="$(_access_setup_lan < "$(_lan_answers y n)" 2>/dev/null)"
  assert_status 10 "$?" "written but not accepted is not done"
  assert_not_contains "$(stub_calls systemctl)" "enable" "nothing is started on a bad config"
}

# Root, so the rule is the command itself rather than one behind a sudo this
# sandbox does not have. Who runs what is lib/pkg.sh's business.
_as_root() {
  stub_script id <<'SH'
case "$*" in
  "-u") echo 0 ;;
  *) echo root ;;
esac
SH
  platform_reset
}

test_a_machine_whose_firewall_setup_cannot_write_for_gets_words_instead_of_a_rule() {
  # Generating a rule for a firewall this does not understand would be
  # guessing at the one thing worth being sure about.
  _lan_machine
  stub_command curl --stdout "401"
  local out
  out="$(_access_setup_lan < "$(_lan_answers y)")"
  assert_contains "$out" "would rather say so than write the wrong one"
  assert_contains "$out" "192.168.1.0/24"
}

test_the_firewall_question_is_scoped_to_this_network_and_may_be_refused() {
  _lan_machine
  _as_root
  stub_command ufw
  stub_command curl --stdout "401"
  local out
  out="$(_access_setup_lan < "$(_lan_answers y n)")"
  # The question itself is only written to a terminal, so what it asked is
  # read back out of the ledger - which is where it has to be anyway.
  assert_contains "$(wizard_consent_ledger)" \
    "firewall	refused	may I let devices on 192.168.1.0/24 reach this computer on port 8443?"
  assert_stub_not_called ufw "a refused rule leaves the firewall alone"
  assert_contains "$out" "nothing about your firewall has changed"
}

test_a_yes_to_the_firewall_runs_a_rule_naming_this_network_only() {
  _lan_machine
  _as_root
  stub_command ufw
  stub_command curl --stdout "401"
  _access_setup_lan < "$(_lan_answers y y)" >/dev/null
  assert_stub_called_with ufw "from 192.168.1.0/24"
  assert_not_contains "$(stub_calls ufw)" "0.0.0.0/0"
  assert_contains "$(wizard_consent_ledger)" "firewall	granted	"
}

test_the_walkthrough_is_written_down_as_well_as_printed() {
  _lan_machine
  stub_command curl --stdout "401"
  _access_setup_lan < "$(_lan_answers y n)" >/dev/null
  assert_file_exists "$(_access_ca_help)"
  assert_file_contains "$(_access_ca_help)" "Certificate Trust Settings"
  assert_file_contains "$(_access_ca_help)" "http://192.168.1.42:8480/ayeaye-ca.crt"
}

test_a_front_end_that_does_not_answer_is_unfinished_rather_than_done() {
  _lan_machine
  stub_command curl --exit 7 --stderr "connection refused"
  local out
  out="$(_access_setup_lan < "$(_lan_answers y n)" 2>/dev/null)"
  assert_status 10 "$?"
  assert_file_exists "$(_access_caddyfile)" "everything that did work is still on disk"
}

# ------------------------------------------------- which address is the network
#
# Two probes with one job between them: work out which of this computer's
# addresses is the home network. Both were tightened because getting them wrong
# is not cosmetic - the certificate is issued for whatever they answer, the
# phone is told to open it, and the firewall question offers to open a port to
# whatever subnet they name.

test_a_container_bridge_is_not_a_home_network() {
  # A machine with docker on it has a pile of private addresses that no phone
  # can reach. Taking the first one would issue the certificate for 172.17.0.1
  # and offer to open port 8443 to every container on the machine.
  stub_script ip <<'SH'
cat <<'OUT'
1: lo    inet 127.0.0.1/8 scope host lo\       valid_lft forever
2: docker0    inet 172.17.0.1/16 brd 172.17.255.255 scope global docker0\       valid_lft forever
3: br-d671725e59c7    inet 172.20.0.1/16 brd 172.20.255.255 scope global br-d671725e59c7\       valid_lft forever
4: tailscale0    inet 100.107.204.86/32 scope global tailscale0\       valid_lft forever
5: virbr0    inet 192.168.122.1/24 brd 192.168.122.255 scope global virbr0\       valid_lft forever
6: enp6s0    inet 192.168.1.201/24 brd 192.168.1.255 scope global dynamic enp6s0\       valid_lft 60380sec
OUT
SH
  assert_eq "192.168.1.201" "$(_access_lan_ip)"
  assert_eq "192.168.1.0/24" "$(_access_lan_subnet)"
}

test_the_interfaces_that_are_not_a_home_network_are_named() {
  local name
  for name in docker0 br-d671725e59c7 virbr0 vmnet1 veth1234 tailscale0 tun0 wg0 lo; do
    _access_real_interface "$name" \
      && fail "$name is not an interface a phone is on the other end of"
  done
  for name in enp6s0 eth0 wlan0 wlp3s0 en0; do
    _access_real_interface "$name" \
      || fail "$name is an ordinary interface and must be usable"
  done
}

test_only_a_whole_address_in_a_private_range_is_taken() {
  local value
  # Everything this says yes to is written into a generated configuration file
  # and into the list of addresses ayeaye accepts.
  for value in 10.0.0.1 192.168.1.42 172.16.0.1 172.31.255.254; do
    _access_private_v4 "$value" || fail "$value is a private address"
  done
  for value in 8.8.8.8 172.15.0.1 172.32.0.1 127.0.0.1 "" \
               "10.1.2.3.4" "10.1.2" "10.x\$(id)" "192.168.1.1\"" "10.0.0.1 x" "1000.1.1.1"; do
    _access_private_v4 "$value" \
      && fail "[$value] is not an address on a home network"
  done
  # The loop's last statement is a check that is supposed to fail, and a test
  # function's status is its last statement's.
  return 0
}
