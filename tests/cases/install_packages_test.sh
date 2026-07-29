# The software ayeaye cannot run without, and the questions that decide what
# else a run has to fetch.
#
# Nothing is installed here. Every package manager is a stub, and what is
# asserted is the exact command each family would have been handed - which is
# the only honest way to test "what would this do on Fedora" from a machine
# that is not one. tests/containers.sh is where those same commands are made to
# run for real.

setup() {
  WIZARD_STATE_DIR="$TEST_TMPDIR/state"
  WIZARD_STATE_FILE="$WIZARD_STATE_DIR/setup-state"
  WIZARD_CONSENT_LEDGER="$WIZARD_STATE_DIR/setup-consent.log"
  WIZARD_LOG_FILE="$WIZARD_STATE_DIR/setup.log"
  WIZARD_BACKUP_DIR="$WIZARD_STATE_DIR/backups"
  # No os-release until a test names one, so the platform layer describes a
  # fixture and never the machine running the suite.
  PLATFORM_OS_RELEASE_FILES=""
  export PLATFORM_OS_RELEASE_FILES
  . "$REPO_ROOT/lib/wizard.sh"
  _declare_the_stages
  . "$REPO_ROOT/lib/steps/60-packages.sh"
  wizard_state_load
  wizard_consent_reset
  wizard_plan_reset
  WIZARD_INTERACTIVE=0
  # What --yes sets. These tests are about what would be run, so the asking is
  # answered here and pinned on its own in wizard_consent_test.
  WIZARD_AUTO_CONSENT="privileged download replace"
}

# The eight install.sh declares, so a step file registers onto the same names it
# would in a real run.
_declare_the_stages() {
  local stage
  for stage in welcome detect report configure plan install service finish; do
    wizard_stage "$stage" "$stage" || true
  done
}

_stub_uname() {
  stub_command uname --exit 1
  stub_when uname '-s' --stdout "$1"
  stub_when uname '-m' --stdout "$2"
}

# _on <os-release fixture> <package manager> - a Linux family, as root.
#
# Root rather than a user with sudo, so the recorded call is the package
# manager's own argv rather than the elevation wrapper's. The prefix is a
# separate question and has a test of its own below.
_on() {
  _stub_uname Linux x86_64
  stub_command id --stdout "0"
  stub_command "$2"
  assert_fixture_exists "os-release/$1"
  PLATFORM_OS_RELEASE_FILES="$(fixture_file "os-release/$1")"
  export PLATFORM_OS_RELEASE_FILES
  platform_reset
}

_on_macos_with_brew() {
  _stub_uname Darwin arm64
  stub_command_from_fixture sw_vers sw_vers/macos-15.1
  stub_command id --stdout "501"
  stub_command brew
  PLATFORM_OS_RELEASE_FILES=""
  export PLATFORM_OS_RELEASE_FILES
  platform_reset
}

# Answers for a call made in this shell. stdin_lines drives a separate process
# and cannot reach a function called here.
_answers() {
  local file="$TEST_TMPDIR/answers" line
  : > "$file"
  for line in "$@"; do
    printf '%s\n' "$line" >> "$file"
  done
  printf '%s' "$file"
}

_run_step() {
  local fn="$1"
  shift
  STEP_OUT="$("$fn" "$@" 2>&1)"
  STEP_STATUS=$?
  # The step ran in a command substitution, which is a subshell: anything it
  # remembered reached the file and not this shell's copy of it.
  wizard_state_load
  return 0
}

# _installs_for_real <manager> - a package manager stub that leaves the program
# behind, the way a real one would. Without it every install in this file looks
# like one that silently did nothing, which is a different test.
_installs_for_real() {
  stub_script "$1" <<SH
# "is it already installed?" - always no here, so there is something for the
# install below to do. pacman and brew answer both questions; the others ask
# rpm or dpkg-query, which are not on this PATH at all.
case " \$* " in
  *" -Q "*|*" list "*) exit 1 ;;
esac
case " \$* " in
  *" install "*|*" -S "*) ;;
  *) exit 0 ;;
esac
for _name in \$*; do
  case "\$_name" in
    tmux|curl|tar)  _made="\$_name" ;;
    python|python3) _made=python3 ;;
    *) continue ;;
  esac
  printf '#!/bin/sh\\nexit 0\\n' > "$STUB_BIN/\$_made"
  chmod 755 "$STUB_BIN/\$_made"
done
exit 0
SH
}

# ------------------------------------------------------- how it is wired in

test_the_file_registers_the_steps_it_says_it_does() {
  # A mistyped stage or policy is status 2 from wizard_step at source time,
  # which kills install.sh before it has printed a word. Worth pinning where
  # the failure is one line rather than a whole run.
  local table
  table="$(printf '%s' "$_WIZARD_STEP_TABLE")"
  assert_matches "$table" "detect	software	_pkgs_detect_step	required	always	"
  assert_matches "$table" "configure	software	_pkgs_choose_step	required	once	"
  assert_matches "$table" "install	packages	_pkgs_requirements_step	required	once	"
  assert_matches "$table" "install	agents	_pkgs_agents_step	optional	once	"
  assert_matches "$table" "install	marker	_pkgs_marker_step	optional	once	"
  assert_matches "$table" "install	board	_pkgs_board_step	optional	once	"
}

test_the_optional_work_is_registered_optional() {
  # The rule the run depends on: an agent or a board that could not be
  # installed is reported and the run carries on. Three of the six steps.
  local table optional
  table="$(printf '%s' "$_WIZARD_STEP_TABLE")"
  optional="$(printf '%s\n' "$table" | grep -c "	optional	")"
  assert_eq "3" "$optional"
}

test_the_detect_stage_says_what_is_already_here() {
  _on debian-12 apt-get
  stub_command claude
  _run_step _pkgs_detect_step
  assert_status 0 "$STEP_STATUS"
  assert_contains "$STEP_OUT" "ok       Claude Code"
  assert_contains "$STEP_OUT" "OpenAI Codex (not installed)"
  assert_contains "$STEP_OUT" "cliban (optional; the project board needs it)"
}

# ------------------------------------------------------ what is required

test_ayeayes_own_two_are_checked_here_and_installed_three_stages_earlier() {
  # install.sh asks about tmux and python3 in the detect stage, because a run
  # without them cannot reach this one. Asking again here would be a second
  # password prompt for something already done.
  _on debian-12 apt-get
  stub_command tmux
  stub_command python3
  _run_step _pkgs_requirements_step
  assert_status 0 "$STEP_STATUS"
  assert_contains "$STEP_OUT" "ok       tmux"
  assert_contains "$STEP_OUT" "ok       python3"
  assert_stub_not_called apt-get "nothing was missing, so nothing was installed"
  assert_contains "$STEP_OUT" "everything setup needs is already on this computer."
}

test_a_run_that_somehow_lost_tmux_stops_rather_than_installing_it_twice() {
  _on debian-12 apt-get
  stub_command python3
  _run_step _pkgs_requirements_step
  assert_status 12 "$STEP_STATUS"
  assert_stub_not_called apt-get \
    "the detect stage owns these; this step reports and does not act"
  assert_contains "$STEP_OUT" "ayeaye cannot run without tmux."
}

test_the_download_tools_are_installed_only_when_there_is_a_download() {
  _on debian-12 apt-get
  assert_eq "" "$(_pkgs_fetch_tools)" "nothing was chosen, so nothing is needed"
  wizard_remember answer.agent.claude 1
  assert_eq "curl" "$(_pkgs_fetch_tools)" \
    "the Claude Code installer is a script, not an archive"
  wizard_remember answer.agent.claude 0
  wizard_remember answer.board 1
  assert_eq "curl tar" "$(_pkgs_fetch_tools)" \
    "a prebuilt release is fetched and then unpacked"
  wizard_remember answer.board 0
  wizard_remember answer.agent.codex 1
  assert_eq "curl tar" "$(_pkgs_fetch_tools)"
}

test_a_program_already_here_needs_nothing_fetched_however_it_was_answered() {
  _on debian-12 apt-get
  stub_command cliban
  wizard_remember answer.board 1
  assert_eq "" "$(_pkgs_fetch_tools)" \
    "an answer from a previous run must not order a download of what is here"
}

# --------------------------------------------- the exact command per family

# _wants_an_archive - the answer that makes this step install curl and tar.
_wants_an_archive() {
  stub_command tmux
  stub_command python3
  wizard_remember answer.board 1
}

test_debian_is_given_apt_get_with_the_prompts_switched_off() {
  _on debian-12 apt-get
  _wants_an_archive
  _installs_for_real apt-get
  _run_step _pkgs_requirements_step
  assert_status 0 "$STEP_STATUS"
  assert_stub_called_with apt-get "apt-get update"
  assert_stub_called_with apt-get "apt-get install -y curl tar"
  assert_contains "$STEP_OUT" "installed."
}

test_a_package_that_did_not_arrive_is_never_reported_as_installed() {
  # The failure mode the whole lifecycle exists to prevent. apt-get here says
  # yes and does nothing, which is exactly what a repository outage looks like.
  _on debian-12 apt-get
  _wants_an_archive
  _run_step _pkgs_requirements_step
  assert_stub_called_with apt-get "apt-get install -y curl tar"
  assert_status 10 "$STEP_STATUS" \
    "the command ran and curl is still not here, so this is not finished"
  assert_contains "$STEP_OUT" "MISSING  curl"
  assert_eq "0" "$(wizard_state_get step.install.packages.download)" \
    "the steps after this one have to be able to find out"
}

test_fedora_is_given_dnf() {
  _on fedora-40 dnf
  _wants_an_archive
  _installs_for_real dnf
  _run_step _pkgs_requirements_step
  assert_status 0 "$STEP_STATUS"
  assert_stub_called_with dnf "dnf makecache"
  assert_stub_called_with dnf "dnf install -y curl tar"
}

test_arch_is_given_pacman() {
  _on arch pacman
  _wants_an_archive
  _installs_for_real pacman
  _run_step _pkgs_requirements_step
  assert_status 0 "$STEP_STATUS"
  assert_stub_called_with pacman "pacman -Sy"
  assert_stub_called_with pacman "pacman -S --needed --noconfirm curl tar"
}

test_opensuse_is_given_zypper() {
  _on opensuse-tumbleweed zypper
  _wants_an_archive
  _installs_for_real zypper
  _run_step _pkgs_requirements_step
  assert_status 0 "$STEP_STATUS"
  assert_stub_called_with zypper "zypper --non-interactive refresh"
  assert_stub_called_with zypper "zypper --non-interactive install curl tar"
}

test_a_mac_is_given_homebrew_and_never_an_elevated_command() {
  # Homebrew refuses to run as root and installs into a prefix the user owns.
  # tar is not asked for here at all: see the name-table test below.
  _on_macos_with_brew
  _wants_an_archive
  _installs_for_real brew
  stub_command sudo
  _run_step _pkgs_requirements_step
  assert_stub_called_with brew "brew install curl"
  assert_stub_not_called sudo "Homebrew is never run with elevated privileges"
  assert_status 10 "$STEP_STATUS" \
    "curl arrived and tar has no formula, so this is not finished"
}

test_an_unprivileged_user_gets_the_command_with_its_elevation_prefix() {
  _stub_uname Linux x86_64
  stub_command id --stdout "1000"
  stub_command sudo
  stub_command apt-get
  _wants_an_archive
  PLATFORM_OS_RELEASE_FILES="$(fixture_file os-release/debian-12)"
  export PLATFORM_OS_RELEASE_FILES
  platform_reset
  _run_step _pkgs_requirements_step
  assert_stub_called_with sudo "env DEBIAN_FRONTEND=noninteractive apt-get install -y curl tar" \
    "the elevation is part of the generated command, not something typed here"
}

test_a_package_manager_that_already_has_it_is_believed() {
  # The other half of "is this installed": a package that is there under a name
  # the shell does not know about yet. tmux is not on this PATH and dpkg-query
  # says it is installed, and that is enough.
  _on debian-12 apt-get
  stub_command python3
  stub_script dpkg-query <<'SH'
printf 'installed'
SH
  _run_step _pkgs_requirements_step
  assert_status 0 "$STEP_STATUS"
  assert_contains "$STEP_OUT" "ok       tmux"
  assert_stub_called_with dpkg-query "dpkg-query -W" \
    "the package database was really asked"
}

test_a_name_this_family_has_no_package_for_is_not_asked_of_it() {
  # macOS ships bsdtar and Homebrew's nearest formula is gnu-tar, which is a
  # different program under a different name. "brew install tar" is a command
  # for a thing that does not exist, so it is never generated.
  _on arch pacman
  assert_eq "tar" "$(platform_pkg_name tar)"
  platform_pkg_has_mapping tar || fail "the table has an opinion about tar on Linux"

  _on_macos_with_brew
  if platform_pkg_has_mapping tar; then
    fail "there is nothing called tar to install on a Mac"
  fi
  _wants_an_archive
  stub_when brew 'list *' --exit 1
  stub_command curl
  _run_step _pkgs_requirements_step
  assert_status 10 "$STEP_STATUS"
  assert_contains "$STEP_OUT" "no package for tar"
  assert_not_contains "$(stub_calls brew)" "brew install" \
    "asking whether it is there is a read; asking for it is not done at all"
}

# ------------------------------------------------- refusing, and not lying

test_refusing_the_install_changes_nothing_and_says_so() {
  _on debian-12 apt-get
  _wants_an_archive
  WIZARD_AUTO_CONSENT=""
  _run_step _pkgs_requirements_step
  assert_status 10 "$STEP_STATUS" \
    "pending: the run carries on and the stage is not recorded as done"
  assert_stub_not_called apt-get "refused means nothing ran"
  assert_contains "$STEP_OUT" "exactly as it was"
  assert_contains "$STEP_OUT" "cannot be fetched, so that part is left"
  assert_eq "0" "$(wizard_state_get step.install.packages.download)"
}

test_a_machine_that_cannot_install_is_told_what_to_do_by_hand() {
  # Debian with no apt-get: recognised, and unable to act. The distinction
  # matters because the advice is different.
  _stub_uname Linux x86_64
  stub_command id --stdout "0"
  _wants_an_archive
  PLATFORM_OS_RELEASE_FILES="$(fixture_file os-release/debian-12)"
  export PLATFORM_OS_RELEASE_FILES
  platform_reset
  _run_step _pkgs_requirements_step
  assert_status 10 "$STEP_STATUS"
  assert_contains "$STEP_OUT" "package manager is not installed here"
  assert_contains "$STEP_OUT" "packages needed: curl tar"
}

# ------------------------------------------------------------ the questions

test_the_answers_are_remembered_where_the_install_steps_read_them() {
  _on debian-12 apt-get
  stub_command tmux
  stub_command python3
  stub_command curl
  stub_command tar
  WIZARD_INTERACTIVE=1
  _pkgs_choose_step < "$(_answers y n y y)" >/dev/null
  assert_eq "1" "$(wizard_state_get answer.agent.claude)"
  assert_eq "0" "$(wizard_state_get answer.agent.codex)"
  assert_eq "1" "$(wizard_state_get answer.agent.marker)"
  assert_eq "1" "$(wizard_state_get answer.board)"
  assert_file_contains "$WIZARD_STATE_FILE" "answer.board=1" \
    "on disk, because the stage that asked is one a resumed run skips"
}

test_the_marker_is_not_asked_about_when_there_will_be_no_claude_code() {
  _on debian-12 apt-get
  stub_command tmux
  stub_command python3
  stub_command curl
  stub_command tar
  WIZARD_INTERACTIVE=1
  # claude: no, codex: no, board: no. Three answers, not four.
  _pkgs_choose_step < "$(_answers n n n)" >/dev/null
  assert_eq "0" "$(wizard_state_get answer.agent.marker)"
  assert_eq "0" "$(wizard_state_get answer.board)"
}

test_a_rerun_offers_back_what_was_answered_last_time() {
  _on debian-12 apt-get
  stub_command tmux
  stub_command python3
  stub_command curl
  stub_command tar
  wizard_remember answer.board 1
  WIZARD_INTERACTIVE=1
  # Two answers, then the file ends: the board question falls back to its
  # default, which is what was chosen before.
  _pkgs_choose_step < "$(_answers n n)" >/dev/null
  assert_eq "1" "$(wizard_state_get answer.board)" \
    "a rerun keeps the choices somebody already made"
}

test_an_unattended_run_never_decides_to_install_an_agent() {
  _on debian-12 apt-get
  stub_command tmux
  stub_command python3
  stub_command curl
  stub_command tar
  WIZARD_INTERACTIVE=0
  _pkgs_choose_step >/dev/null
  assert_eq "0" "$(wizard_state_get answer.agent.claude)" \
    "installing Claude Code is a good suggestion to someone who is there to
hear it, and a decision taken for them when they are not"
  assert_eq "0" "$(wizard_state_get answer.board)"
}

test_nothing_is_offered_on_a_computer_that_could_not_fetch_it() {
  # No curl and no package manager to get one with. Four questions whose
  # answers could not be acted on are worse than one sentence.
  _stub_uname Linux x86_64
  stub_command id --stdout "0"
  stub_command tmux
  stub_command python3
  PLATFORM_OS_RELEASE_FILES="$(fixture_file os-release/debian-12)"
  export PLATFORM_OS_RELEASE_FILES
  platform_reset
  WIZARD_INTERACTIVE=1
  _run_step _pkgs_choose_step
  assert_status 11 "$STEP_STATUS" "it does not apply here"
  assert_contains "$STEP_OUT" "no way to fetch a program from the internet"
  assert_not_contains "$STEP_OUT" "curl" "a program name is not language to ask in"
  assert_eq "0" "$(wizard_state_get answer.agent.claude 0)"
  assert_eq "0" "$(wizard_state_get answer.board 0)"
}

test_a_computer_that_can_download_but_not_unpack_is_offered_what_it_can_have() {
  # curl and no tar and no package manager: the Claude Code installer is a
  # script and works, and the two that arrive in an archive do not.
  _stub_uname Linux x86_64
  stub_command id --stdout "0"
  stub_command curl
  PLATFORM_OS_RELEASE_FILES="$(fixture_file os-release/debian-12)"
  export PLATFORM_OS_RELEASE_FILES
  platform_reset
  WIZARD_INTERACTIVE=1
  _pkgs_choose_step < "$(_answers y y)" >/dev/null
  wizard_state_load
  assert_eq "1" "$(wizard_state_get answer.agent.claude)" \
    "the first answer went to Claude Code"
  assert_eq "1" "$(wizard_state_get answer.agent.marker)" \
    "and the second to the marker, because Codex was never asked about"
  assert_eq "0" "$(wizard_state_get answer.board 0)"
}

test_a_program_already_installed_is_never_asked_about_or_planned() {
  _on debian-12 apt-get
  stub_command tmux
  stub_command python3
  stub_command curl
  stub_command tar
  stub_command claude
  stub_command cliban
  WIZARD_INTERACTIVE=1
  # One question only: Codex. The marker still is asked, because there is a
  # Claude Code to mark.
  _pkgs_choose_step < "$(_answers n n)" >/dev/null
  wizard_state_load
  assert_eq "" "$(wizard_state_get answer.agent.claude)" \
    "an answer key is what the user chose, never a fact about the machine"
  assert_eq "" "$(wizard_state_get answer.board)"
  assert_not_contains "$(wizard_plan_show)" "cliban" "there is nothing to download"
}

# ---------------------------------------------------------------- the plan

test_the_plan_names_everything_before_any_of_it_happens() {
  _on debian-12 apt-get
  stub_command tmux
  stub_command python3
  WIZARD_INTERACTIVE=1
  _pkgs_choose_step < "$(_answers y n y y)" >/dev/null
  local plan
  plan="$(wizard_plan_show)"
  assert_contains "$plan" "what setup needs to fetch things: curl tar"
  assert_contains "$plan" "Claude Code, from claude.ai"
  assert_contains "$plan" "cliban v0.1.0, for the project board"
  assert_contains "$plan" "a status line for Claude Code"
  assert_not_contains "$plan" "OpenAI Codex" "it was not chosen"
  wizard_plan_is_consequential \
    || fail "downloading and installing is what stage five stops for"
}

test_the_plan_says_how_much_of_the_disk_this_will_cost() {
  _on debian-12 apt-get
  stub_command tmux
  stub_command python3
  stub_command curl
  stub_command tar
  WIZARD_INTERACTIVE=1
  _pkgs_choose_step < "$(_answers n y n)" >/dev/null
  assert_contains "$(wizard_plan_show)" "MB of downloads and disk space" \
    "Codex is 110 MB and somebody on a phone tether should be told"
}
