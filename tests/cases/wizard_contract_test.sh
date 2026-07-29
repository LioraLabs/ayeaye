# The rules the wizard layer exists to enforce, checked against the source
# rather than against a run.
#
# Six tickets are going to add steps to this wizard. Every one of them will
# want to run something privileged, download something, or replace a file, and
# every one of them could do it directly - the functions are right there in
# lib/pkg.sh and lib/service.sh, and sudo is a word anyone can type. A test
# that only drives the wizard cannot see that happen: the run would work
# perfectly, and the promise on the first screen would quietly be false.
#
# So this file reads the tree. It is a lint, and it is the only thing standing
# between "we ask before we act" and "we asked, once, in the ticket that wrote
# the sentence".

setup() {
  stub_real grep find sort
}

# Every shell file that could plausibly do something to this machine.
_wizard_sources() {
  printf '%s\n' "$REPO_ROOT/install.sh"
  find "$REPO_ROOT/lib" -name '*.sh' -type f | LC_ALL=C sort
}

# _offenders <regex> <allowed-basename>... - matching lines outside the files
# that are allowed to contain them, as "<file>:<line>: <text>".
_offenders() {
  local pattern="$1" file base allowed found
  shift
  while IFS= read -r file; do
    [ -n "$file" ] || continue
    base="${file##*/}"
    allowed=0
    for name in "$@"; do
      [ "$base" = "$name" ] && allowed=1
    done
    [ "$allowed" = 1 ] && continue
    # Comments are prose, not code: the wrappers are named in the header of
    # nearly every file here, and a rule that fired on its own documentation
    # would be turned off within a week.
    found="$(grep -n -E "$pattern" "$file" 2>/dev/null | grep -v -E '^[0-9]+:[[:space:]]*#' || true)"
    [ -n "$found" ] || continue
    printf '%s:%s\n' "$base" "$found"
  done <<EOF
$(_wizard_sources)
EOF
}

# --------------------------------------------------- nothing acts on its own

test_nothing_installs_a_package_except_the_wrapper() {
  # platform_pkg_install is public, documented and effectful, and the header of
  # lib/pkg.sh calls it "the executing form" - which is exactly what makes it
  # the thing somebody reaches for.
  local offenders
  offenders="$(_offenders 'platform_pkg_install[^_]' consent.sh pkg.sh)"
  assert_eq "" "$offenders" \
    "installing goes through wizard_install_packages, which asks first"
}

test_nothing_operates_a_service_except_through_a_wrapper() {
  local offenders
  offenders="$(_offenders 'platform_service_run' consent.sh service.sh)"
  assert_eq "" "$offenders" \
    "build the command with platform_service_command and run it where the
outcome can be reported, not through the executing form"
}

test_nothing_reaches_for_sudo_by_hand() {
  local offenders
  offenders="$(_offenders '(^|[^A-Za-z_-])sudo[[:space:]]' consent.sh pkg.sh)"
  assert_eq "" "$offenders" \
    "anything needing a password goes through wizard_privileged"
}

test_nothing_downloads_a_file_except_the_wrapper() {
  # A GET is not a download; keeping what comes back is. That is the line this
  # draws: curl with an output flag, and "-o /dev/null" - which is how a probe
  # asks for a status code and throws the body away - is on the safe side of it.
  local offenders
  offenders="$(_offenders 'curl[^|]*(-o|-O|--output|--remote-name)' consent.sh \
    | grep -v -E '(-o|--output)[[:space:]]+/dev/null' || true)"
  assert_eq "" "$offenders" \
    "fetching bytes onto this machine goes through wizard_download, which asks"
}

test_nothing_changes_a_firewall_or_a_trust_store_by_hand() {
  local offenders
  offenders="$(_offenders '(ufw|firewall-cmd|iptables|nft|security add-trusted-cert|update-ca-certificates|trust anchor)' consent.sh)"
  assert_eq "" "$offenders" \
    "these go through wizard_firewall and wizard_trust, which are never granted
without a person"
}

test_the_lint_can_actually_fail() {
  # A lint that cannot fire is decoration. This proves the scan reads the files
  # it says it reads by looking for something that really is in one of them.
  local found
  found="$(_offenders 'wizard_install_packages' install.sh consent.sh)"
  assert_eq "" "$found" "nothing else calls it today"
  found="$(_offenders 'wizard_install_packages' consent.sh)"
  assert_contains "$found" "install.sh:" \
    "the scan really does read install.sh"
}

# ------------------------------------------------------- the step seam

test_a_step_file_dropped_into_the_steps_directory_is_picked_up() {
  # The whole extension mechanism, end to end: lib/steps/80-health.sh is not
  # mentioned anywhere in install.sh, and its step still runs.
  require_host_command python3
  stub_command tmux
  stub_real python3
  assert_file_exists "$REPO_ROOT/lib/steps/80-health.sh"
  assert_eq "" "$(grep -c '80-health' "$REPO_ROOT/install.sh" | grep -v '^0$')" \
    "install.sh must not know the name of any file in lib/steps"
  run_install --defaults --no-systemd
  assert_status 0 "$RUN_STATUS"
  assert_contains "$RUN_STDOUT" "ayeaye is not running yet, so there is nothing to check." \
    "the registered step ran, and said what it could see"
}

test_the_step_seam_is_documented_where_a_sibling_will_look() {
  assert_file_exists "$REPO_ROOT/lib/steps/README.md"
  local doc
  doc="$(cat "$REPO_ROOT/lib/steps/README.md")"
  assert_contains "$doc" "wizard_step <stage> <step> <function> <label>"
  assert_contains "$doc" "WIZARD_STAGE_PENDING"
  assert_contains "$doc" "GUARDED_PATHS"
  assert_contains "$doc" "wizard_privileged"
  assert_contains "$doc" "wizard_remember"
}

test_every_stage_the_documentation_names_really_exists() {
  # A sibling registering onto a stage that was renamed gets status 2 and a run
  # that stops before it prints anything, which is a bad way to find out.
  local stage
  for stage in welcome detect report configure plan install service finish; do
    assert_matches "$(cat "$REPO_ROOT/install.sh")" "^wizard_stage +$stage " \
      "lib/steps/README.md names \"$stage\" as a stage a step may attach to"
    assert_contains "$(cat "$REPO_ROOT/lib/steps/README.md")" "$stage"
  done
}

# ---------------------------------------------------- guarded paths

test_every_path_the_wizard_writes_is_guarded_by_the_suite() {
  # The tripwire is what stops a test escaping into a real home directory, and
  # it only covers the paths it has been told about.
  local guard path
  guard="$(cat "$REPO_ROOT/tests/run.sh")"
  for path in "ayeaye/setup-state" "ayeaye/setup-consent.log" "ayeaye/setup.log" \
              "ayeaye/backups" "ayeaye/env" "ayeaye/token" \
              "systemd/user/ayeaye.service"; do
    assert_contains "$guard" "$path" \
      "$path is written by setup and must be on GUARDED_PATHS"
  done
}
