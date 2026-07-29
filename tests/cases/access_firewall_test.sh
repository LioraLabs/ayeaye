# Firewall rules: scoped to one subnet, asked about every time, or not
# generated at all.
#
# The builder lives in lib/consent.sh because the tree-wide lint makes that the
# only file allowed to name a firewall program - see wizard_contract_test.sh.
# It has no effect of its own; wizard_firewall is what asks and runs.
#
# Nothing here runs a real firewall command. PATH inside a test is exactly the
# stub directory, so the programs named below are recordings and the host's
# own firewall is untouched whatever these tests do.

_load_consent() {
  WIZARD_STATE_DIR="$TEST_TMPDIR/state"
  WIZARD_STATE_FILE="$WIZARD_STATE_DIR/setup-state"
  WIZARD_CONSENT_LEDGER="$WIZARD_STATE_DIR/setup-consent.log"
  WIZARD_LOG_FILE="$WIZARD_STATE_DIR/setup.log"
  WIZARD_BACKUP_DIR="$WIZARD_STATE_DIR/backups"
  PLATFORM_OS_RELEASE_FILES=""
  export PLATFORM_OS_RELEASE_FILES
  . "$REPO_ROOT/lib/state.sh"
  . "$REPO_ROOT/lib/ui.sh"
  . "$REPO_ROOT/lib/platform.sh"
  . "$REPO_ROOT/lib/consent.sh"
  wizard_state_load
  wizard_consent_reset
  WIZARD_INTERACTIVE=0
  WIZARD_AUTO_CONSENT=""
}

setup() {
  _load_consent
}

# Root, so that the generated rule is the command itself rather than the
# command behind a sudo this sandbox does not have. What runs as what is
# lib/pkg.sh's business and is tested there.
_as_root() {
  stub_script id <<'SH'
case "$*" in
  "-u") echo 0 ;;
  *) echo root ;;
esac
SH
  platform_reset
}

_answers() {
  local file="$TEST_TMPDIR/answers" line
  : > "$file"
  for line in "$@"; do
    printf '%s\n' "$line" >> "$file"
  done
  printf '%s' "$file"
}

# ------------------------------------------------------------- what is built

test_a_rule_names_the_subnet_it_is_for() {
  _as_root
  stub_command ufw
  local rule
  rule="$(wizard_firewall_rule 192.168.1.0/24 8443)"
  assert_status 0 "$?"
  assert_contains "$rule" "192.168.1.0/24"
  assert_contains "$rule" "8443"
}

test_no_rule_this_can_build_is_open_to_everything() {
  # The property, not the spelling. There is deliberately no way to ask this
  # for an unscoped rule, because a rule allowing the whole internet is not a
  # smaller version of what the caller wanted.
  local rule tool
  _as_root
  for tool in ufw firewall-cmd; do
    stub_remove ufw 2>/dev/null || true
    stub_remove firewall-cmd 2>/dev/null || true
    stub_command "$tool"
    rule="$(wizard_firewall_rule 10.0.0.0/8 8443)" || fail "no rule for $tool"
    assert_contains "$rule" "10.0.0.0/8" "$tool: the rule must name the subnet"
    assert_not_contains "$rule" "0.0.0.0/0" "$tool"
    assert_not_contains "$rule" "from any" "$tool"
    assert_not_contains "$rule" "--add-port" "$tool: a bare port is open to everything"
  done
}

test_a_machine_with_no_firewall_this_knows_gets_no_rule() {
  local rule
  _as_root
  rule="$(wizard_firewall_rule 192.168.1.0/24 8443)"
  assert_status 1 "$?" "not knowing how is a fact about the machine, not a bug"
  assert_eq "" "$rule"
}

test_a_subnet_that_is_not_one_is_a_caller_bug() {
  _as_root
  stub_command ufw
  wizard_firewall_rule "192.168.1.42" 8443 >/dev/null 2>&1
  assert_status 2 "$?" "an address is not a subnet"
  wizard_firewall_rule "; rm -rf /" 8443 >/dev/null 2>&1
  assert_status 2 "$?" "a value that reaches a command run as root is checked here"
  wizard_firewall_rule "192.168.1.0/24" "8443; echo" >/dev/null 2>&1
  assert_status 2 "$?"
  wizard_firewall_rule >/dev/null 2>&1
  assert_status 2 "$?"
}

test_a_machine_that_cannot_become_root_is_told_so_rather_than_handed_a_rule() {
  stub_command ufw
  stub_remove sudo 2>/dev/null || true
  # A non-root user with no way to become one cannot change a firewall, and a
  # command that would fail is worse than an honest "this machine cannot".
  stub_script id <<'SH'
case "$*" in
  "-u") echo 1000 ;;
  *) echo tester ;;
esac
SH
  platform_reset
  wizard_firewall_rule 192.168.1.0/24 8443 >/dev/null 2>&1
  assert_status 1 "$?"
}

# ------------------------------------------------------------- who may run it

test_nothing_happens_to_the_firewall_when_the_answer_is_no() {
  _as_root
  stub_command ufw
  WIZARD_INTERACTIVE=1
  local rule
  rule="$(wizard_firewall_rule 192.168.1.0/24 8443)"
  wizard_firewall "may I let devices on 192.168.1.0/24 reach port 8443?" "$rule" \
    < "$(_answers "n")"
  assert_status 3 "$?" "refused is 3, and is not a failure"
  assert_stub_not_called ufw "a refused rule must not have been run"
  assert_contains "$(wizard_consent_ledger)" "firewall	refused	"
}

test_a_typed_yes_is_what_runs_it_and_the_ledger_says_so() {
  _as_root
  stub_command ufw
  WIZARD_INTERACTIVE=1
  local rule
  rule="$(wizard_firewall_rule 192.168.1.0/24 8443)"
  wizard_firewall "may I let devices on 192.168.1.0/24 reach port 8443?" "$rule" \
    < "$(_answers "y")"
  assert_status 0 "$?"
  assert_stub_called_with ufw "from 192.168.1.0/24"
  assert_contains "$(wizard_consent_ledger)" "firewall	granted	"
}

test_no_flag_can_open_a_firewall() {
  # --yes exists so automation can install packages. A blanket yes to opening
  # a machine to a network is not a thing this project offers.
  stub_command ufw
  WIZARD_AUTO_CONSENT="privileged download replace firewall trust expose"
  WIZARD_INTERACTIVE=0
  wizard_firewall "may I?" "ufw allow proto tcp from 192.168.1.0/24 to any port 8443"
  assert_status 3 "$?"
  assert_stub_not_called ufw
}

test_an_unattended_run_never_opens_one_either() {
  stub_command ufw
  WIZARD_INTERACTIVE=0
  WIZARD_AUTO_CONSENT=""
  wizard_firewall "may I?" "ufw allow proto tcp from 192.168.1.0/24 to any port 8443"
  assert_status 3 "$?"
  assert_stub_not_called ufw
  assert_contains "$(wizard_consent_ledger)" "firewall	refused	"
}
