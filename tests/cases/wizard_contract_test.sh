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

# _judge_downloads - offender lines in, the ones that really keep bytes out.
#
# A GET is not a download; keeping what comes back is. The harmless redirects
# are cut out of each line before it is judged rather than excused after it, so
# that "curl -o file url >/dev/null" is still the download it obviously is.
_judge_downloads() {
  sed -e 's/2>&1//g' \
      -e 's/[0-9]\{0,1\}>[[:space:]]*\/dev\/null//g' \
      -e 's/-o[[:space:]][[:space:]]*\/dev\/null//g' \
      -e 's/--output[[:space:]][[:space:]]*\/dev\/null//g' \
    | grep -E 'curl[^|]*(-o|-O|--output|--remote-name|>)' \
    | grep -v -E '#[[:space:]]*bootstrap-fetch$' || true
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
  # draws: curl with an output flag or with a redirect that keeps the body,
  # while "-o /dev/null" and "> /dev/null" - which is how a probe asks for a
  # status code and throws the body away - are on the safe side of it.
  #
  # The harmless redirects are cut out of each line before it is judged rather
  # than excused after it, so that "curl -o file url >/dev/null" is still the
  # download it obviously is.
  local offenders
  offenders="$(_offenders 'curl' consent.sh | _judge_downloads)"
  assert_eq "" "$offenders" \
    "fetching bytes onto this machine goes through wizard_download, which asks"
}

test_the_download_lint_catches_a_redirect_and_not_a_probe() {
  # The rule above is only worth what it can see, and until this ticket it
  # could not see `curl … > file` at all. Fed lines that are not in the tree,
  # so that the judgement is pinned rather than the tree's current innocence.
  local judged
  judged="$(printf '%s\n' \
    'made-up.sh:1:  curl -fsSL "$url" > "$dest"' \
    'made-up.sh:2:  curl -fsSL --retry 2 -o "$dest" "$url"' \
    'made-up.sh:3:  curl -sS -o /dev/null -w "%{http_code}" "$url" 2>&1' \
    'made-up.sh:4:  curl -fsSL "$url" > /dev/null' \
    'made-up.sh:5:  command -v curl >/dev/null 2>&1' \
    'made-up.sh:6:  curl -fsSL "$url" -o "$dest"  # bootstrap-fetch' \
    'made-up.sh:7:  command -v curl >/dev/null 2>&1 && curl -fsSL "$url" -o "$dest"' \
    | _judge_downloads)"
  assert_contains "$judged" "made-up.sh:1" "a redirect that keeps the body is a download"
  assert_contains "$judged" "made-up.sh:2" "so is an output flag"
  assert_not_contains "$judged" "made-up.sh:3" "throwing the body away is a probe"
  assert_not_contains "$judged" "made-up.sh:4" "so is a redirect to /dev/null"
  assert_not_contains "$judged" "made-up.sh:5" "and looking curl up is not using it"
  assert_not_contains "$judged" "made-up.sh:6" "the bootstrap carries its exemption"
  assert_contains "$judged" "made-up.sh:7" \
    "looking curl up on the same line as using it excuses nothing - a rule that
judged whole lines by their least incriminating clause would be no rule at all"
}

test_the_one_download_outside_the_wrapper_is_the_bootstrap_and_there_is_one_of_it() {
  # install.sh downloads once before lib/consent.sh exists to ask on its
  # behalf: the release it is about to hand over to. That fetch is exempt from
  # the rule above by a marker on its own line, which makes the exemption
  # something a reviewer can see rather than something a pattern happens to
  # miss. These two assertions are what stop the marker spreading.
  local marked count fetch_line source_line
  marked="$(_offenders '#[[:space:]]*bootstrap-fetch$')"
  count="$(printf '%s\n' "$marked" | grep -c 'bootstrap-fetch' || true)"
  assert_eq "1" "$count" "exactly one line in this repository may carry the marker"
  assert_matches "$marked" '^install\.sh:' "and it is the bootstrap in install.sh"

  # And it is in the half of install.sh that runs before the wizard layer is
  # loaded. Below that line there is a wrapper to use, and no excuse.
  fetch_line="$(grep -n -E '#[[:space:]]*bootstrap-fetch$' "$REPO_ROOT/install.sh" \
    | head -1 | cut -d: -f1)"
  source_line="$(grep -n 'lib/wizard\.sh"$' "$REPO_ROOT/install.sh" | head -1 | cut -d: -f1)"
  assert_ne "" "$source_line" "install.sh must still source the wizard layer"
  if [ "$fetch_line" -ge "$source_line" ]; then
    fail "the bootstrap fetch (line $fetch_line) is below the point where
lib/consent.sh becomes available (line $source_line), and must use wizard_download"
  fi
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
  found="$(_offenders 'wizard_install_packages' consent.sh)"
  assert_contains "$found" "install.sh:" \
    "the scan really does read install.sh"
}

test_the_only_callers_of_the_install_wrapper_are_where_work_is_added() {
  # install.sh owns the lifecycle and lib/steps/ is where a ticket adds work to
  # it. Anywhere else calling the wrapper - a library, an adapter - would be a
  # layer installing something on a caller's behalf, which is how "we ask
  # first" quietly stops being true.
  local allowed="" file found
  for file in "$REPO_ROOT"/lib/steps/*.sh; do
    [ -f "$file" ] || continue
    allowed="$allowed ${file##*/}"
  done
  # shellcheck disable=SC2086
  found="$(_offenders 'wizard_install_packages' install.sh consent.sh $allowed)"
  assert_eq "" "$found" \
    "installing is asked for by install.sh or a file in lib/steps, and nowhere else"
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
              "share}/ayeaye" "whisper-models" \
              "ayeaye/Caddyfile" "ayeaye/ca/ayeaye-ca.crt" \
              "ayeaye/reverse-proxy.caddy" "ayeaye/reverse-proxy.nginx" \
              "ayeaye/phone-certificate.txt" \
              "systemd/user/ayeaye-caddy.service" \
              "systemd/user/ayeaye.service"; do
    assert_contains "$guard" "$path" \
      "$path is written by setup and must be on GUARDED_PATHS"
  done
}
