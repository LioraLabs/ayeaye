# The consent primitive, and the wrappers that are the only sanctioned way to
# have an effect on the machine.
#
# One rule, enforced in one place: nothing privileged, nothing downloaded, no
# firewall change, no certificate trusted and no existing configuration
# replaced without a question first. Every later ticket in this milestone adds
# actions of exactly those kinds, so the rule has to live somewhere they cannot
# reach around - which is what this file pins.

setup() {
  WIZARD_STATE_DIR="$TEST_TMPDIR/state"
  WIZARD_STATE_FILE="$WIZARD_STATE_DIR/setup-state"
  WIZARD_CONSENT_LEDGER="$WIZARD_STATE_DIR/setup-consent.log"
  WIZARD_LOG_FILE="$WIZARD_STATE_DIR/setup.log"
  WIZARD_BACKUP_DIR="$WIZARD_STATE_DIR/backups"
  # No os-release unless a test says otherwise, so the platform layer answers
  # "unknown" here rather than describing whatever machine is running the suite.
  PLATFORM_OS_RELEASE_FILES=""
  export PLATFORM_OS_RELEASE_FILES
  . "$REPO_ROOT/lib/state.sh"
  . "$REPO_ROOT/lib/ui.sh"
  . "$REPO_ROOT/lib/platform.sh"
  . "$REPO_ROOT/lib/consent.sh"
  wizard_state_load
  wizard_consent_reset
  wizard_plan_reset
  WIZARD_INTERACTIVE=0
  WIZARD_AUTO_CONSENT=""
}

# A marker only a command that really ran can leave behind.
_marker() { printf '%s' "$TEST_TMPDIR/ran"; }
_touch_marker_command() { printf '%s' "printf ran > $(_marker)"; }

# Answers for a call made in this same shell. stdin_lines is for run_script,
# which starts a process of its own and cannot help here.
_answers() {
  local file="$TEST_TMPDIR/answers" line
  : > "$file"
  for line in "$@"; do
    printf '%s\n' "$line" >> "$file"
  done
  printf '%s' "$file"
}

# ------------------------------------------------------------------- policy

test_the_six_kinds_are_the_whole_vocabulary() {
  local kind
  for kind in privileged download firewall trust replace expose; do
    wizard_consent_policy "$kind" >/dev/null
    assert_status 0 "$?" "$kind must be a known kind"
  done
  wizard_consent_policy invented >/dev/null
  assert_status 2 "$?" "an invented kind is a caller bug, not a refusal"
}

test_an_interactive_run_asks_about_every_kind() {
  WIZARD_INTERACTIVE=1
  local kind
  for kind in privileged download firewall trust replace expose; do
    assert_eq "ask" "$(wizard_consent_policy "$kind")"
  done
}

test_an_unattended_run_refuses_every_kind() {
  # This is what --defaults means: never ask, and therefore never assume yes.
  local kind
  for kind in privileged download firewall trust replace expose; do
    assert_eq "refuse" "$(wizard_consent_policy "$kind")"
  done
}

test_blanket_yes_still_cannot_reach_the_network_or_the_trust_store() {
  # --yes exists so automation can install packages. It must not be a way to
  # open a firewall, trust a certificate or put ayeaye on a network, because
  # those are the decisions nobody should be able to make on someone's behalf.
  WIZARD_AUTO_CONSENT="privileged download firewall trust expose replace"
  assert_eq "grant" "$(wizard_consent_policy privileged)"
  assert_eq "grant" "$(wizard_consent_policy download)"
  assert_eq "grant" "$(wizard_consent_policy replace)"
  assert_eq "refuse" "$(wizard_consent_policy firewall)" \
    "a firewall change is never granted without a human"
  assert_eq "refuse" "$(wizard_consent_policy trust)"
  assert_eq "refuse" "$(wizard_consent_policy expose)"
}

# ------------------------------------------------------------------ asking

test_a_refused_consent_returns_three_and_records_the_refusal() {
  # 3, the same number every wrapper uses, so that "refused" never has to be
  # told apart from "it ran and failed" by which function was called.
  wizard_consent privileged "install two packages"
  assert_status 3 "$?"
  assert_contains "$(wizard_consent_ledger)" "privileged	refused	install two packages"
}

test_a_granted_consent_returns_zero_and_records_the_grant() {
  WIZARD_AUTO_CONSENT="privileged"
  wizard_consent privileged "install two packages"
  assert_status 0 "$?"
  assert_contains "$(wizard_consent_ledger)" "privileged	granted	install two packages"
}

test_the_question_is_asked_in_words_and_answered_no_by_default() {
  WIZARD_INTERACTIVE=1
  wizard_consent privileged "may I install ffmpeg for you?" < "$(_answers "")"
  assert_status 3 "$?" "an empty answer must not be read as permission"
}

test_a_typed_yes_grants_it() {
  WIZARD_INTERACTIVE=1
  wizard_consent privileged "may I install ffmpeg for you?" < "$(_answers "y")"
  assert_status 0 "$?"
}

test_details_are_logged_rather_than_put_in_the_question() {
  WIZARD_INTERACTIVE=1
  wizard_consent privileged "may I install ffmpeg for you?" \
    "sudo apt-get install -y ffmpeg" < "$(_answers "n")"
  assert_file_contains "$WIZARD_LOG_FILE" "sudo apt-get install -y ffmpeg" \
    "the raw command belongs in the technical detail"
}

test_an_unknown_kind_is_a_caller_bug_and_grants_nothing() {
  wizard_consent invented "do a thing"
  assert_status 2 "$?"
  assert_eq "" "$(wizard_consent_ledger)" "a rejected call must not be recorded as a decision"
}

# --------------------------------------------------------- effect wrappers

test_a_refused_privileged_command_never_runs() {
  wizard_privileged "may I change a system file?" "$(_touch_marker_command)"
  assert_status 3 "$?" "3 is refused: nothing happened, which is not a failure"
  assert_file_missing "$(_marker)" "refusal must leave the machine exactly as it was"
}

test_a_granted_privileged_command_runs_exactly_what_it_was_given() {
  WIZARD_AUTO_CONSENT="privileged"
  wizard_privileged "may I change a system file?" "$(_touch_marker_command)"
  assert_status 0 "$?"
  assert_file_contains "$(_marker)" "ran"
}

test_a_granted_command_that_fails_reports_its_own_failure() {
  WIZARD_AUTO_CONSENT="privileged"
  wizard_privileged "may I?" "exit 7"
  assert_status 1 "$?" "1 is: it was allowed to run, and it did not work"
}

test_a_refused_download_fetches_nothing() {
  stub_command curl
  wizard_download "may I fetch the voice model?" \
    "https://example.invalid/model.bin" "$TEST_TMPDIR/model.bin"
  assert_status 3 "$?"
  assert_stub_not_called curl
  assert_file_missing "$TEST_TMPDIR/model.bin"
}

test_a_granted_download_asks_curl_for_exactly_that_url() {
  WIZARD_AUTO_CONSENT="download"
  stub_command curl
  wizard_download "may I fetch the voice model?" \
    "https://example.invalid/model.bin" "$TEST_TMPDIR/model.bin"
  assert_status 0 "$?"
  assert_stub_called_with curl "https://example.invalid/model.bin"
  assert_stub_called_with curl "$TEST_TMPDIR/model.bin"
}

test_a_download_with_no_fetcher_says_so_rather_than_pretending() {
  WIZARD_AUTO_CONSENT="download"
  assert_command_absent curl
  wizard_download "may I fetch it?" "https://example.invalid/x" "$TEST_TMPDIR/x"
  assert_ne 0 "$?" "a machine with nothing to download with must not report success"
  assert_file_missing "$TEST_TMPDIR/x"
}

test_a_refused_firewall_change_changes_no_firewall() {
  wizard_firewall "may I open a port on this computer?" "$(_touch_marker_command)"
  assert_status 3 "$?"
  assert_file_missing "$(_marker)"
}

test_a_refused_certificate_trust_trusts_nothing() {
  wizard_trust "may I tell this computer to trust a new certificate?" \
    "$(_touch_marker_command)"
  assert_status 3 "$?"
  assert_file_missing "$(_marker)"
}

test_every_wrapper_leaves_a_line_in_the_ledger() {
  # The ledger is the audit trail a later ticket's coverage asserts against.
  stub_command curl
  wizard_privileged "p" "true" || true
  wizard_download "d" "https://example.invalid/x" "$TEST_TMPDIR/x" || true
  wizard_firewall "f" "true" || true
  wizard_trust "t" "true" || true
  local ledger
  ledger="$(wizard_consent_ledger)"
  assert_contains "$ledger" "privileged	refused	p"
  assert_contains "$ledger" "download	refused	d"
  assert_contains "$ledger" "firewall	refused	f"
  assert_contains "$ledger" "trust	refused	t"
}

# ------------------------------------------------- replacing configuration

test_replacing_a_file_that_does_not_exist_needs_no_permission() {
  wizard_replace "$TEST_TMPDIR/absent.conf"
  assert_status 0 "$?" "there is nothing of the user's to lose"
  assert_eq "" "$(wizard_consent_ledger)"
}

test_replacing_an_existing_file_is_refused_by_default() {
  printf 'mine\n' > "$TEST_TMPDIR/existing.conf"
  wizard_replace "$TEST_TMPDIR/existing.conf"
  assert_status 3 "$?"
  assert_file_contains "$TEST_TMPDIR/existing.conf" "mine" "untouched"
}

test_an_agreed_replacement_backs_the_old_file_up_first() {
  WIZARD_AUTO_CONSENT="replace"
  printf 'the old settings\n' > "$TEST_TMPDIR/existing.conf"
  wizard_replace "$TEST_TMPDIR/existing.conf"
  assert_status 0 "$?"
  local backups
  backups="$(find "$WIZARD_BACKUP_DIR" -type f -name 'existing.conf.*')"
  assert_ne "" "$backups" "the previous version must be recoverable"
  assert_file_contains "$backups" "the old settings"
}

test_the_backup_path_is_told_to_the_user() {
  WIZARD_AUTO_CONSENT="replace"
  printf 'the old settings\n' > "$TEST_TMPDIR/existing.conf"
  local out
  out="$(wizard_replace "$TEST_TMPDIR/existing.conf")"
  assert_contains "$out" "$WIZARD_BACKUP_DIR/existing.conf." \
    "a backup nobody is told about is a backup nobody can use"
}

test_two_backups_of_one_file_do_not_collide() {
  WIZARD_AUTO_CONSENT="replace"
  printf 'first\n' > "$TEST_TMPDIR/existing.conf"
  wizard_replace "$TEST_TMPDIR/existing.conf" >/dev/null
  printf 'second\n' > "$TEST_TMPDIR/existing.conf"
  wizard_replace "$TEST_TMPDIR/existing.conf" >/dev/null
  local count
  count="$(find "$WIZARD_BACKUP_DIR" -type f -name 'existing.conf.*' | wc -l | tr -d ' ')"
  assert_eq "2" "$count" "the first backup must survive the second"
}

# -------------------------------------------------------------- packages

test_installing_packages_where_they_cannot_be_installed_explains_instead() {
  # No package manager on this PATH, so the layer cannot act. The wrapper must
  # print what a human would have to run and report that it did nothing.
  local out
  out="$(wizard_install_packages tmux ffmpeg 2>&1)"
  assert_ne 0 "$?" "it must not report success for work it did not do"
  assert_contains "$out" "packages needed:" "the manual instructions are printed instead"
}

test_a_blanket_yes_applies_whether_or_not_the_run_could_have_asked() {
  # --yes is itself an answer, typed by a person. A flag that only worked when
  # nobody was watching would be a flag that does nothing in the case its own
  # help text describes.
  WIZARD_INTERACTIVE=1
  WIZARD_AUTO_CONSENT="privileged download replace"
  assert_eq "grant" "$(wizard_consent_policy privileged)"
  assert_eq "grant" "$(wizard_consent_policy replace)"
  assert_eq "ask" "$(wizard_consent_policy trust)" \
    "and the three it may never grant are still asked about"
  assert_eq "ask" "$(wizard_consent_policy firewall)"
  assert_eq "ask" "$(wizard_consent_policy expose)"
}

test_exposure_is_asked_about_through_the_same_primitive() {
  wizard_may_expose "may other computers on your network reach ayeaye?"
  assert_status 3 "$?" "an unattended run never puts ayeaye on a network"
  assert_contains "$(wizard_consent_ledger)" "expose	refused"
}

test_a_decision_taken_elsewhere_still_reaches_the_ledger() {
  wizard_consent_record expose granted "you chose to reach ayeaye over tailscale"
  assert_contains "$(wizard_consent_ledger)" "expose	granted	you chose to reach ayeaye over tailscale"
}

test_installing_packages_goes_through_consent() {
  stub_command apt-get
  stub_command dpkg-query
  stub_command sudo
  stub_command_from_fixture uname uname/linux-x86_64
  PLATFORM_OS_RELEASE_FILES="$(fixture_file os-release/debian-12)"
  export PLATFORM_OS_RELEASE_FILES
  . "$REPO_ROOT/lib/platform.sh"
  platform_reset
  platform_detect
  wizard_install_packages tmux >/dev/null 2>&1
  assert_status 3 "$?" "unattended means refused, and refused means nothing ran"
  assert_stub_not_called apt-get
  assert_contains "$(wizard_consent_ledger)" "privileged	refused"
}

# ------------------------------------------------------------- the plan

test_the_plan_collects_what_stage_five_has_to_summarise() {
  wizard_plan_add package "tmux, ffmpeg" 42000000
  wizard_plan_add network "listen on this computer only"
  local shown
  shown="$(wizard_plan_show)"
  assert_contains "$shown" "tmux, ffmpeg"
  assert_contains "$shown" "listen on this computer only"
}

test_the_plan_totals_the_disk_it_will_use() {
  wizard_plan_add package "tmux" 20000000
  wizard_plan_add download "a voice model" 80000000
  assert_contains "$(wizard_plan_show)" "100 MB"
}

test_a_small_plan_does_not_round_itself_down_to_nothing() {
  wizard_plan_add package "a config snippet" 120000
  assert_contains "$(wizard_plan_show)" "less than 1 MB" \
    "\"about 0 MB\" reads as \"this is free\", which is not what it means"
}

test_an_empty_plan_says_there_is_nothing_to_do() {
  assert_contains "$(wizard_plan_show)" "nothing"
  wizard_plan_is_consequential
  assert_status 1 "$?"
}

test_only_consequential_work_needs_confirming() {
  # Writing your own config file and your own user service is not something to
  # be asked about; installing packages, downloading and touching the network
  # are.
  wizard_plan_add config "your settings file"
  wizard_plan_add service "a service that starts ayeaye when you log in"
  wizard_plan_add network "reachable from this computer only"
  wizard_plan_is_consequential
  assert_status 1 "$?" \
    "how ayeaye is reached was settled by the step that asked about it; asking again is asking twice"
  wizard_plan_add package "ffmpeg"
  wizard_plan_is_consequential
  assert_status 0 "$?"
}
