# The two coding agents: finding them, fetching them, signing in, and saying
# exactly what was proved.
#
# Nothing here reaches the internet. curl is a stub that serves a file this
# test built, so the URL, the archive layout and what is done with what comes
# back are all asserted without a byte crossing the network. What that cannot
# prove - that the real claude.ai installer and the real Codex release behave
# as expected - is recorded on the ticket rather than pretended at.

setup() {
  WIZARD_STATE_DIR="$TEST_TMPDIR/state"
  WIZARD_STATE_FILE="$WIZARD_STATE_DIR/setup-state"
  WIZARD_CONSENT_LEDGER="$WIZARD_STATE_DIR/setup-consent.log"
  WIZARD_LOG_FILE="$WIZARD_STATE_DIR/setup.log"
  WIZARD_BACKUP_DIR="$WIZARD_STATE_DIR/backups"
  PLATFORM_OS_RELEASE_FILES=""
  export PLATFORM_OS_RELEASE_FILES
  . "$REPO_ROOT/lib/wizard.sh"
  _declare_the_stages
  . "$REPO_ROOT/lib/steps/60-packages.sh"
  wizard_state_load
  wizard_consent_reset
  wizard_plan_reset
  WIZARD_INTERACTIVE=0
  WIZARD_AUTO_CONSENT="privileged download replace"
  stub_real tar cmp gzip
  # The step before this one decides whether anything can be fetched at all.
  wizard_remember step.install.packages.download 1
  wizard_remember step.install.packages.unpack 1
}

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

# _machine <kernel> <arch> - what the release artifact is chosen from.
_machine() {
  _stub_uname "$1" "$2"
  stub_command id --stdout "1000"
  if [ "$1" = Darwin ]; then
    stub_command_from_fixture sw_vers sw_vers/macos-15.1
    PLATFORM_OS_RELEASE_FILES=""
  else
    PLATFORM_OS_RELEASE_FILES="$(fixture_file os-release/debian-12)"
  fi
  export PLATFORM_OS_RELEASE_FILES
  platform_reset
}

# A curl that serves one file, and records the URL it was asked for.
_curl_serves() {
  CURL_PAYLOAD="$1"
  export CURL_PAYLOAD
  stub_script curl <<'SH'
dest=""
prev=""
for arg in "$@"; do
  [ "$prev" = "-o" ] && dest="$arg"
  prev="$arg"
done
[ -n "$dest" ] || exit 0
cp "$CURL_PAYLOAD" "$dest"
SH
}

# The Claude Code installer, as far as this file is concerned: a script that
# leaves a claude behind. What Anthropic's real one does is its own business.
_fake_claude_installer() {
  local path="$TEST_TMPDIR/install.sh"
  cat > "$path" <<'SH'
#!/bin/sh
mkdir -p "$HOME/.local/bin"
cat > "$HOME/.local/bin/claude" <<'INNER'
#!/bin/sh
[ "$1" = "--version" ] && { echo "2.1.211 (Claude Code)"; exit 0; }
exit 1
INNER
chmod 755 "$HOME/.local/bin/claude"
SH
  printf '%s' "$path"
}

# A Codex release archive, laid out the way the real one is: one executable,
# named after the platform it was built for.
_fake_codex_archive() {
  local target="$1" dir="$TEST_TMPDIR/build" out="$TEST_TMPDIR/codex.tar.gz"
  rm -rf "$dir"
  mkdir -p "$dir"
  cat > "$dir/codex-$target" <<'SH'
#!/bin/sh
[ "$1" = "--version" ] && { echo "codex-cli 0.146.0"; exit 0; }
exit 1
SH
  chmod 755 "$dir/codex-$target"
  ( cd "$dir" && tar -czf "$out" "codex-$target" )
  printf '%s' "$out"
}

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
  wizard_state_load
  return 0
}

# ---------------------------------------------------------------- finding

test_an_agent_that_is_already_here_is_left_alone() {
  _machine Linux x86_64
  stub_command claude
  stub_command curl
  wizard_remember answer.agent.claude 1
  _run_step _pkgs_agents_step
  assert_stub_not_called curl "nothing is downloaded for a program already here"
  assert_contains "$STEP_OUT" "Claude Code is already here"
}

test_an_agent_nobody_asked_for_is_not_installed() {
  _machine Linux x86_64
  stub_command curl
  stub_command codex
  _run_step _pkgs_agents_step
  assert_stub_not_called curl
  assert_contains "$STEP_OUT" "Claude Code (not chosen)"
}

test_no_agent_at_all_and_none_wanted_is_a_choice_not_a_failure() {
  _machine Linux x86_64
  stub_command curl
  _run_step _pkgs_agents_step
  assert_status 11 "$STEP_STATUS" \
    "skip: the user said no, and nothing is outstanding from setup"
  assert_contains "$STEP_OUT" "there is no coding agent on this computer yet"
}

# ------------------------------------------------------ the exact commands

test_claude_code_comes_from_anthropics_own_installer() {
  _machine Linux x86_64
  _curl_serves "$(_fake_claude_installer)"
  wizard_remember answer.agent.claude 1
  _run_step _pkgs_agents_step
  assert_stub_called_with curl "https://claude.ai/install.sh" \
    "the one URL Anthropic documents for this"
  assert_status 10 "$STEP_STATUS" \
    "pending: installed, and an unattended run could not wait for a sign-in"
  assert_file_exists "$HOME/.local/bin/claude"
}

test_the_installer_is_downloaded_and_then_run_and_not_piped_into_a_shell() {
  # Fetching a script from the internet and running it are different things to
  # agree to, and the second is the one worth being able to refuse on its own.
  _machine Linux x86_64
  _curl_serves "$(_fake_claude_installer)"
  wizard_remember answer.agent.claude 1
  _run_step _pkgs_agents_step
  local ledger
  ledger="$(cat "$WIZARD_CONSENT_LEDGER")"
  assert_contains "$ledger" "download	granted"
  assert_contains "$ledger" "privileged	granted"
  assert_contains "$STEP_OUT" "hands over to Anthropic's own installer" \
    "and says so, rather than promising what somebody else's program does"
}

test_the_installer_cannot_eat_the_answer_to_the_next_question() {
  # The hazard the whole file is careful about, at the one place it is easiest
  # to miss: the installer runs through eval and inherits the wizard's own
  # standard input. One that reads a line would swallow the next answer.
  _machine Linux x86_64
  cat > "$TEST_TMPDIR/greedy.sh" <<'SH'
#!/bin/sh
cat >/dev/null
mkdir -p "$HOME/.local/bin"
printf '#!/bin/sh\necho 1.0\n' > "$HOME/.local/bin/claude"
chmod 755 "$HOME/.local/bin/claude"
SH
  _curl_serves "$TEST_TMPDIR/greedy.sh"
  wizard_remember answer.agent.claude 1
  WIZARD_INTERACTIVE=1
  local answers
  # One redirect covering both, or the second call would simply be handed a
  # fresh copy of the file and the assertion would hold whatever happened.
  answers="$(_answers "" "the answer after it")"
  { _pkgs_agents_step >/dev/null 2>&1
    wizard_ask "and the question after that" "not this"; } < "$answers"
  assert_eq "the answer after it" "$REPLY" \
    "the installer read standard input and must not have been given the wizard's"
}

test_a_temporary_folder_with_a_space_in_its_name_still_works() {
  # $TMPDIR belongs to the user, and everything wizard_privileged runs goes
  # through eval.
  _machine Linux x86_64
  TMPDIR="$TEST_TMPDIR/a folder with spaces"
  mkdir -p "$TMPDIR"
  export TMPDIR
  _curl_serves "$(_fake_claude_installer)"
  wizard_remember answer.agent.claude 1
  _run_step _pkgs_agents_step
  assert_file_exists "$HOME/.local/bin/claude"
  assert_not_contains "$STEP_OUT" "could not be installed"
}

test_codex_comes_from_the_release_built_for_this_computer() {
  local target
  for target in x86_64-unknown-linux-musl aarch64-unknown-linux-musl \
                x86_64-apple-darwin aarch64-apple-darwin; do
    case "$target" in
      x86_64-unknown-linux-musl)  _machine Linux x86_64 ;;
      aarch64-unknown-linux-musl) _machine Linux aarch64 ;;
      x86_64-apple-darwin)        _machine Darwin x86_64 ;;
      aarch64-apple-darwin)       _machine Darwin arm64 ;;
    esac
    _curl_serves "$(_fake_codex_archive "$target")"
    wizard_remember answer.agent.codex 1
    _run_step _pkgs_agents_step
    assert_stub_called_with curl \
      "https://github.com/openai/codex/releases/latest/download/codex-$target.tar.gz" \
      "the artifact for $target and no other"
    assert_file_exists "$HOME/.local/bin/codex"
    rm -f "$HOME/.local/bin/codex"
  done
}

test_a_computer_no_release_is_built_for_is_told_so_rather_than_guessed_at() {
  _machine Linux armv7l
  stub_command curl
  wizard_remember answer.agent.codex 1
  _run_step _pkgs_agents_step
  assert_stub_not_called curl "there is nothing safe to download"
  assert_contains "$STEP_OUT" "does not publish a ready-made Codex"
  assert_contains "$STEP_OUT" "npm install -g @openai/codex"
  assert_status 10 "$STEP_STATUS" \
    "unfinished rather than failed: trying it again cannot help"
}

test_an_archive_that_is_not_the_release_is_never_unpacked() {
  _machine Linux x86_64
  printf 'this is not a tar file at all\n' > "$TEST_TMPDIR/junk.tar.gz"
  _curl_serves "$TEST_TMPDIR/junk.tar.gz"
  wizard_remember answer.agent.codex 1
  _run_step _pkgs_agents_step
  assert_file_missing "$HOME/.local/bin/codex"
  assert_contains "$STEP_OUT" "is not the Codex release it should be"
  assert_status 12 "$STEP_STATUS"
}

# -------------------------------------------------- signing in, and probing

test_the_pause_for_signing_in_does_not_eat_the_answer_after_it() {
  # The hazard this whole file is careful about: a step that reads standard
  # input in its own way swallows the answers to the questions after it.
  WIZARD_INTERACTIVE=1
  local answers
  answers="$(_answers "" "the next answer")"
  { _pkgs_sign_in "Claude Code" "claude" >/dev/null
    wizard_ask "the question after it" "not this"; } < "$answers"
  assert_eq "the next answer" "$REPLY"
}

test_signing_in_can_be_left_for_later() {
  WIZARD_INTERACTIVE=1
  local status
  _pkgs_sign_in "OpenAI Codex" "codex login" < "$(_answers skip)" >/dev/null
  status=$?
  assert_status 1 "$status" "and the caller reports it as unfinished"
}

test_an_unattended_run_does_not_pretend_the_sign_in_happened() {
  WIZARD_INTERACTIVE=0
  local out status
  out="$(_pkgs_sign_in "Claude Code" "claude")"
  status=$?
  assert_status 1 "$status"
  assert_contains "$out" "this run cannot wait for that, so it has not happened yet."
}

test_the_agent_is_checked_with_something_that_costs_nothing() {
  # A version flag, never a prompt. Asking an agent to answer something real
  # would prove more and would spend the user's money to do it.
  _machine Linux x86_64
  stub_command claude --stdout "2.1.99 (Claude Code)"
  wizard_remember answer.agent.claude 1
  _run_step _pkgs_agents_step
  assert_stub_not_called claude \
    "an agent already here is not probed at all, let alone prompted"

  stub_remove claude
  _curl_serves "$(_fake_claude_installer)"
  _run_step _pkgs_agents_step
  assert_contains "$STEP_OUT" "Claude Code runs: 2.1.211 (Claude Code)" \
    "one version flag, and nothing that would cost the user anything"
}

test_the_report_says_what_the_check_proved_and_what_it_did_not() {
  _machine Linux x86_64
  _curl_serves "$(_fake_claude_installer)"
  wizard_remember answer.agent.claude 1
  WIZARD_INTERACTIVE=1
  STEP_OUT="$(_pkgs_agents_step < "$(_answers "" "")" 2>&1)"
  STEP_STATUS=$?
  assert_status 0 "$STEP_STATUS" "installed, and the user said they signed in"
  assert_contains "$STEP_OUT" "setup checked that it starts; whether the account works is"
  assert_contains "$STEP_OUT" "asking it would"
}

test_an_agent_that_installed_but_will_not_start_is_reported_as_broken() {
  _machine Linux x86_64
  cat > "$TEST_TMPDIR/broken-installer.sh" <<'SH'
#!/bin/sh
mkdir -p "$HOME/.local/bin"
printf '#!/bin/sh\nexit 3\n' > "$HOME/.local/bin/claude"
chmod 755 "$HOME/.local/bin/claude"
SH
  _curl_serves "$TEST_TMPDIR/broken-installer.sh"
  wizard_remember answer.agent.claude 1
  _run_step _pkgs_agents_step
  assert_contains "$STEP_OUT" "will not start"
  assert_status 12 "$STEP_STATUS"
}

# ------------------------------------------------- refusing, and not fetching

test_refusing_the_download_installs_nothing_and_is_not_a_failure() {
  _machine Linux x86_64
  stub_command curl
  WIZARD_AUTO_CONSENT=""
  wizard_remember answer.agent.claude 1
  _run_step _pkgs_agents_step
  assert_stub_not_called curl "refused means nothing happened"
  assert_contains "$STEP_OUT" "Claude Code (you said no)"
  assert_status 11 "$STEP_STATUS"
}

test_nothing_is_fetched_when_the_step_before_said_it_could_not_be() {
  # Two different explanations for one fact is one too many: the packages step
  # already said this was left for another run.
  _machine Linux x86_64
  stub_command curl
  wizard_remember step.install.packages.download 0
  wizard_remember step.install.packages.unpack 0
  wizard_remember answer.agent.claude 1
  _run_step _pkgs_agents_step
  assert_stub_not_called curl
  assert_contains "$STEP_OUT" "nothing here can fetch it yet"
  assert_status 10 "$STEP_STATUS"
}

test_a_failure_is_still_the_answer_when_the_other_agent_only_went_pending() {
  # The worse of the two, in the order that is easy to get wrong: Claude Code
  # fails, and Codex then installs but is not signed in. Pending must not
  # overwrite failed.
  _machine Linux x86_64
  _curl_serves "$(_fake_codex_archive x86_64-unknown-linux-musl)"
  wizard_remember answer.agent.claude 1
  wizard_remember answer.agent.codex 1
  _run_step _pkgs_agents_step
  assert_contains "$STEP_OUT" "Claude Code could not be installed"
  assert_file_exists "$HOME/.local/bin/codex" "and Codex arrived"
  assert_status 12 "$STEP_STATUS" "the worse of the two is what the run is told"
}

test_a_download_that_simply_does_not_work_is_reported_as_such() {
  _machine Linux x86_64
  stub_command curl --exit 1
  wizard_remember answer.agent.claude 1
  _run_step _pkgs_agents_step
  assert_stub_called_with curl "https://claude.ai/install.sh"
  assert_file_missing "$HOME/.local/bin/claude"
  assert_contains "$STEP_OUT" "Claude Code could not be installed"
  assert_status 12 "$STEP_STATUS"
}

test_a_computer_with_no_way_to_download_says_so_and_installs_nothing() {
  _machine Linux x86_64
  stub_remove curl
  wizard_remember answer.agent.claude 1
  _run_step _pkgs_agents_step
  assert_contains "$STEP_OUT" "no way to download files"
  assert_file_missing "$HOME/.local/bin/claude"
  assert_status 12 "$STEP_STATUS"
}

test_nothing_is_left_behind_in_the_temporary_folder() {
  _machine Linux x86_64
  TMPDIR="$TEST_TMPDIR/scratch"
  mkdir -p "$TMPDIR"
  export TMPDIR
  _curl_serves "$(_fake_claude_installer)"
  wizard_remember answer.agent.claude 1
  _run_step _pkgs_agents_step
  assert_eq "" "$(ls "$TMPDIR" 2>/dev/null)" \
    "the archive and the installer are gone, whatever happened to them"
}

test_one_agent_failing_does_not_stop_the_other() {
  _machine Linux x86_64
  _curl_serves "$(_fake_claude_installer)"
  wizard_remember answer.agent.claude 1
  wizard_remember answer.agent.codex 1
  _run_step _pkgs_agents_step
  assert_file_exists "$HOME/.local/bin/claude" \
    "Claude Code arrived even though the same curl served Codex a script"
  assert_status 12 "$STEP_STATUS" "and the run is told that something failed"
}

# ------------------------------------------------- at an actual terminal

test_the_questions_read_the_way_they_look_when_somebody_is_watching() {
  # Bash writes a prompt only to a terminal, so this is the only kind of test
  # that can see the words a person is actually asked, and the defaults they
  # are offered. Everything above it can prove what was done and never what
  # was said.
  require_host_command python3
  stub_real python3
  _stub_uname Linux x86_64
  stub_command tmux
  stub_command curl
  stub_command tar
  pty_answers "" "" ""
  pty_expect "install Claude Code?" "n"
  pty_expect "install OpenAI Codex?" "n"
  pty_expect "install cliban as well?" "n"
  pty_install --no-systemd
  assert_status 0 "$PTY_STATUS"
  assert_contains "$PTY_TRANSCRIPT" "install Claude Code? [y]: n" \
    "offered as the sensible answer, and refusable"
  assert_contains "$PTY_TRANSCRIPT" "install OpenAI Codex? [n]: n"
  assert_contains "$PTY_TRANSCRIPT" "install cliban as well? [n]: n"
  assert_contains "$PTY_TRANSCRIPT" "You need at least one."
}

test_saying_yes_at_a_terminal_reaches_the_step_that_acts_on_it() {
  require_host_command python3
  stub_real python3
  _stub_uname Linux x86_64
  stub_command tmux
  stub_command tar
  _curl_serves "$(_fake_claude_installer)"
  pty_answers "" "" ""
  pty_expect "install Claude Code?" "y"
  pty_expect "install OpenAI Codex?" "n"
  pty_expect "set that up?" "n"
  pty_expect "install cliban as well?" "n"
  pty_expect "go ahead with all of that?" "y"
  pty_expect "may I download the Claude Code installer" "y"
  pty_expect "run the Claude Code installer now?" "y"
  pty_expect "press return once you have signed in" ""
  pty_install --no-systemd
  assert_status 0 "$PTY_STATUS"
  assert_contains "$PTY_TRANSCRIPT" "Claude Code, from claude.ai" \
    "stage five named it before stage six did it"
  assert_contains "$PTY_TRANSCRIPT" "Claude Code runs:"
  assert_file_exists "$HOME/.local/bin/claude"
}

# ------------------------------- a program somebody already put there

# A file of the user's sitting at the name setup wants, which is not something
# setup can run: what an interrupted earlier install leaves behind, and what a
# stray note called "codex" looks like. It is not "already installed", so the
# install goes ahead - and the file is still theirs.
_a_file_of_theirs_at_that_name() {
  mkdir -p "$XDG_BIN_HOME"
  printf 'this was here first\n' > "$XDG_BIN_HOME/codex"
  chmod 644 "$XDG_BIN_HOME/codex"
}

test_a_file_of_the_users_at_that_name_is_never_replaced_without_asking() {
  _machine Linux x86_64
  _a_file_of_theirs_at_that_name
  _curl_serves "$(_fake_codex_archive x86_64-unknown-linux-musl)"
  wizard_remember answer.agent.codex 1
  WIZARD_AUTO_CONSENT=""
  WIZARD_INTERACTIVE=0
  _run_step _pkgs_agents_step
  assert_eq "this was here first" "$(cat "$XDG_BIN_HOME/codex")" \
    "an unattended run refuses, and refusing means nothing happened"
  assert_contains "$STEP_OUT" "OpenAI Codex (you said no)"
  assert_status 11 "$STEP_STATUS"
}

test_replacing_one_takes_a_copy_of_it_first() {
  _machine Linux x86_64
  _a_file_of_theirs_at_that_name
  _curl_serves "$(_fake_codex_archive x86_64-unknown-linux-musl)"
  wizard_remember answer.agent.codex 1
  _run_step _pkgs_agents_step
  local saved
  saved="$(ls "$WIZARD_BACKUP_DIR" 2>/dev/null)"
  assert_ne "" "$saved" "whatever was there is kept"
  assert_file_contains "$WIZARD_BACKUP_DIR/$saved" "this was here first"
  assert_contains "$STEP_OUT" "OpenAI Codex runs: codex-cli 0.146.0"
}

test_an_agent_installed_where_its_own_installer_puts_it_is_found() {
  # claude.ai's installer always writes ~/.local/bin, whatever XDG_BIN_HOME
  # says. A run that installed it and then could not see it would report a
  # working agent as broken, and would then say there is no agent at all.
  _machine Linux x86_64
  XDG_BIN_HOME="$HOME/somewhere/else"
  export XDG_BIN_HOME
  _curl_serves "$(_fake_claude_installer)"
  wizard_remember answer.agent.claude 1
  _run_step _pkgs_agents_step
  assert_file_exists "$HOME/.local/bin/claude"
  assert_contains "$STEP_OUT" "Claude Code runs:"
  assert_not_contains "$STEP_OUT" "cannot be found"
  assert_not_contains "$STEP_OUT" "there is no coding agent on this computer yet" \
    "the step must not contradict itself one line later"
  assert_status 10 "$STEP_STATUS" "installed; an unattended run cannot sign in"
}

test_a_computer_that_can_download_but_not_unpack_still_gets_claude_code() {
  # curl and no tar: the Claude Code installer is a script and works. Telling
  # somebody it does not, because Codex would not, is a sentence that is false.
  _machine Linux x86_64
  _curl_serves "$(_fake_claude_installer)"
  wizard_remember step.install.packages.download 1
  wizard_remember step.install.packages.unpack 0
  wizard_remember answer.agent.claude 1
  wizard_remember answer.agent.codex 1
  _run_step _pkgs_agents_step
  assert_file_exists "$HOME/.local/bin/claude"
  assert_contains "$STEP_OUT" "OpenAI Codex (nothing here can fetch it yet)"
}
