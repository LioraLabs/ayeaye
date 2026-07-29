# cliban, the project board, and the promise that ayeaye works without it.
#
# The one component in this milestone that is optional out loud. Everything
# here is about that: it is offered rather than assumed, saying no costs
# nothing, and it failing costs nothing either - the run carries on and the
# board lands on the list of things worth coming back to.
#
# No byte crosses the network. curl is a stub serving an archive laid out the
# way the real release is, which is how the download URL, the verification and
# the deliberate omission of the multi-user server are all pinned here.

setup() {
  # wizard_env_merge does the text work in python3, which this project already
  # requires and the sandbox deliberately withholds.
  require_host_command python3
  stub_real python3
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
  stub_real tar gzip cmp
  wizard_remember step.install.packages.download 1
  wizard_remember step.install.packages.unpack 1
  wizard_remember answer.board 1
  ENV_FILE="$XDG_CONFIG_HOME/ayeaye/env"
  CLIBAN_DB="$TEST_TMPDIR/board/cliban.db"
  export CLIBAN_DB
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

# The release archive, laid out exactly the way upstream's is: a directory
# named after the release, holding the program, the multi-user server, and two
# text files.
_release_archive() {
  local target="$1" name dir out
  name="cliban-v0.1.0-$target"
  dir="$TEST_TMPDIR/build"
  out="$TEST_TMPDIR/$name.tar.gz"
  rm -rf "$dir"
  mkdir -p "$dir/$name"
  cat > "$dir/$name/cliban" <<'SH'
#!/bin/sh
case "$1" in
  --help) echo "AI-agent-first kanban board for the terminal"; exit 0 ;;
  project)
    mkdir -p "${CLIBAN_DB%/*}"
    printf 'a board\n' > "$CLIBAN_DB"
    exit 0
    ;;
esac
exit 1
SH
  printf '#!/bin/sh\necho the multi-user server\n' > "$dir/$name/cliband"
  printf 'MIT\n' > "$dir/$name/LICENSE"
  printf '# cliban\n' > "$dir/$name/README.md"
  chmod 755 "$dir/$name/cliban" "$dir/$name/cliband"
  ( cd "$dir" && tar -czf "$out" "$name" )
  printf '%s' "$out"
}

_an_env_file() {
  mkdir -p "$XDG_CONFIG_HOME/ayeaye"
  printf 'AYEAYE_BIND=127.0.0.1\nAYEAYE_PORT=8911\n' > "$ENV_FILE"
}

_run_step() {
  STEP_OUT="$(_pkgs_board_step 2>&1)"
  STEP_STATUS=$?
  wizard_state_load
  return 0
}

_installed() { printf '%s/.local/bin/cliban' "$HOME"; }

# ------------------------------------------------------------ it is optional

test_saying_no_costs_nothing_and_says_what_still_works() {
  _machine Linux x86_64
  stub_command curl
  wizard_remember answer.board 0
  _run_step
  assert_status 11 "$STEP_STATUS" "declined is finished business, not a failure"
  assert_stub_not_called curl
  assert_contains "$STEP_OUT" "starting agents, reading them, talking to them"
  assert_contains "$STEP_OUT" "Only the board page and the links to"
}

test_what_needs_cliban_and_what_does_not_is_said_in_the_question_too() {
  _machine Linux x86_64
  stub_command tmux
  stub_command python3
  stub_command curl
  stub_command tar
  WIZARD_INTERACTIVE=1
  local out
  out="$(_pkgs_choose_step < /dev/null 2>&1)"
  assert_contains "$out" "Everything else works without it"
  assert_contains "$out" "the board page and"
  assert_contains "$out" "stay empty without it"
}

# ------------------------------------------------- the artifact per platform

test_the_release_for_this_computer_is_the_one_that_is_fetched() {
  local target
  for target in x86_64-unknown-linux-musl aarch64-unknown-linux-musl \
                x86_64-apple-darwin aarch64-apple-darwin; do
    case "$target" in
      x86_64-unknown-linux-musl)  _machine Linux x86_64 ;;
      aarch64-unknown-linux-musl) _machine Linux aarch64 ;;
      x86_64-apple-darwin)        _machine Darwin x86_64 ;;
      aarch64-apple-darwin)       _machine Darwin arm64 ;;
    esac
    _curl_serves "$(_release_archive "$target")"
    _run_step
    assert_stub_called_with curl \
      "https://github.com/LioraLabs/cliban/releases/download/v0.1.0/cliban-v0.1.0-$target.tar.gz" \
      "the release built for $target and no other"
    assert_file_exists "$(_installed)"
    rm -f "$(_installed)"
  done
}

test_a_computer_no_release_is_built_for_is_told_so_rather_than_guessed_at() {
  _machine Linux armv7l
  stub_command curl
  _run_step
  assert_stub_not_called curl "there is nothing safe to download"
  assert_contains "$STEP_OUT" "does not publish a ready-made program"
  assert_status 10 "$STEP_STATUS" "unfinished, and worth coming back to"
}

# ------------------------------------------------------------- verification

test_an_archive_that_is_not_the_release_is_never_unpacked() {
  _machine Linux x86_64
  printf 'not an archive\n' > "$TEST_TMPDIR/junk.tar.gz"
  _curl_serves "$TEST_TMPDIR/junk.tar.gz"
  _run_step
  assert_file_missing "$(_installed)"
  assert_contains "$STEP_OUT" "not the cliban release it should be"
  assert_status 12 "$STEP_STATUS"
}

test_a_real_archive_holding_the_wrong_thing_is_refused_too() {
  # A tar that opens cleanly and does not contain the program. Checking the
  # bytes are a valid archive is not the same as checking they are the release.
  _machine Linux x86_64
  local dir out
  dir="$TEST_TMPDIR/wrong"
  out="$TEST_TMPDIR/wrong.tar.gz"
  mkdir -p "$dir/cliban-v0.1.0-x86_64-unknown-linux-musl"
  printf 'nothing here\n' > "$dir/cliban-v0.1.0-x86_64-unknown-linux-musl/README.md"
  ( cd "$dir" && tar -czf "$out" "cliban-v0.1.0-x86_64-unknown-linux-musl" )
  _curl_serves "$out"
  _run_step
  assert_file_missing "$(_installed)"
  assert_contains "$STEP_OUT" "not the cliban release it should be"
  assert_status 12 "$STEP_STATUS"
}

test_the_multi_user_server_is_never_put_on_this_computer() {
  # An explicit non-goal. The archive carries cliband and setup takes one file
  # out of it by name, so the server is never even written to disk.
  _machine Linux x86_64
  _curl_serves "$(_release_archive x86_64-unknown-linux-musl)"
  _an_env_file
  _run_step
  assert_status 0 "$STEP_STATUS"
  assert_file_exists "$(_installed)"
  assert_file_missing "$HOME/.local/bin/cliband"
  assert_not_contains "$(stub_calls tar)" "cliband"
}

# ------------------------------------------------ it runs, and ayeaye finds it

test_the_program_is_started_once_with_something_harmless_before_it_is_trusted() {
  _machine Linux x86_64
  _curl_serves "$(_release_archive x86_64-unknown-linux-musl)"
  _an_env_file
  _run_step
  assert_status 0 "$STEP_STATUS"
  assert_contains "$STEP_OUT" "cliban runs"
}

test_its_board_is_started_when_there_is_not_one() {
  _machine Linux x86_64
  _curl_serves "$(_release_archive x86_64-unknown-linux-musl)"
  _an_env_file
  assert_file_missing "$CLIBAN_DB"
  _run_step
  assert_file_exists "$CLIBAN_DB" "cliban makes it the first time it is asked"
  assert_contains "$STEP_OUT" "started an empty board at"
}

test_a_board_that_is_already_there_is_left_alone() {
  _machine Linux x86_64
  _curl_serves "$(_release_archive x86_64-unknown-linux-musl)"
  _an_env_file
  mkdir -p "${CLIBAN_DB%/*}"
  printf 'somebody else already had one\n' > "$CLIBAN_DB"
  _run_step
  assert_eq "somebody else already had one" "$(cat "$CLIBAN_DB")"
  assert_contains "$STEP_OUT" "its board is at"
}

test_ayeaye_is_told_where_the_program_is() {
  # bin/ayeaye shells out to cliban and looks for it on PATH. The background
  # service does not have the PATH a terminal window has, so it is told rather
  # than left to find it.
  _machine Linux x86_64
  _curl_serves "$(_release_archive x86_64-unknown-linux-musl)"
  _an_env_file
  _run_step
  assert_status 0 "$STEP_STATUS"
  assert_file_contains "$ENV_FILE" "VOICE_CLIBAN=$(_installed)"
  assert_file_contains "$ENV_FILE" "AYEAYE_PORT=8911" "and nothing else changed"
  assert_contains "$STEP_OUT" "told ayeaye where cliban is"
}

test_the_settings_file_is_copied_aside_before_that_line_is_added() {
  _machine Linux x86_64
  _curl_serves "$(_release_archive x86_64-unknown-linux-musl)"
  _an_env_file
  _run_step
  local saved
  saved="$(ls "$WIZARD_BACKUP_DIR" 2>/dev/null)"
  assert_ne "" "$saved"
  assert_not_contains "$(cat "$WIZARD_BACKUP_DIR/$saved")" "VOICE_CLIBAN"
}

test_no_settings_file_yet_means_the_line_is_offered_rather_than_written() {
  _machine Linux x86_64
  _curl_serves "$(_release_archive x86_64-unknown-linux-musl)"
  _run_step
  assert_status 10 "$STEP_STATUS" "the program is here; ayeaye cannot be told yet"
  assert_contains "$STEP_OUT" "VOICE_CLIBAN=$(_installed)"
}

test_a_cliban_already_on_this_computer_is_used_rather_than_downloaded() {
  _machine Linux x86_64
  stub_command curl
  stub_script cliban <<'SH'
case "$1" in
  --help) echo "AI-agent-first kanban board for the terminal"; exit 0 ;;
  project) mkdir -p "${CLIBAN_DB%/*}"; printf 'a board\n' > "$CLIBAN_DB"; exit 0 ;;
esac
exit 1
SH
  _an_env_file
  _run_step
  assert_status 0 "$STEP_STATUS"
  assert_stub_not_called curl "nothing is downloaded for a program already here"
  assert_contains "$STEP_OUT" "cliban is already here"
}

# ------------------------------------------------------------ failing softly

test_a_download_that_does_not_work_does_not_stop_the_run() {
  _machine Linux x86_64
  stub_command curl --exit 1
  _run_step
  assert_status 12 "$STEP_STATUS" \
    "failed, and the step is registered optional so the walk carries on"
  assert_file_missing "$(_installed)"
  assert_contains "$STEP_OUT" "Everything else ayeaye does is unaffected."
}

test_refusing_the_download_is_not_a_failure_at_all() {
  _machine Linux x86_64
  stub_command curl
  WIZARD_AUTO_CONSENT=""
  _run_step
  assert_status 11 "$STEP_STATUS"
  assert_stub_not_called curl
  assert_contains "$STEP_OUT" "The board page will stay empty, and"
}

test_nothing_is_fetched_when_the_step_before_said_it_could_not_be() {
  _machine Linux x86_64
  stub_command curl
  wizard_remember step.install.packages.unpack 0
  _run_step
  assert_stub_not_called curl
  assert_status 10 "$STEP_STATUS"
  assert_contains "$STEP_OUT" "left for another run"
}

# ---------------------------------------------------- building it, the long way

test_building_from_source_is_offered_and_never_taken_by_itself() {
  _machine Linux armv7l
  stub_command curl
  stub_command cargo
  WIZARD_INTERACTIVE=0
  _run_step
  assert_stub_not_called cargo \
    "compiling for several minutes is not something to start on somebody's behalf"
  assert_status 10 "$STEP_STATUS"
}

test_building_from_source_is_described_as_the_long_way_round() {
  _machine Linux armv7l
  stub_command curl
  stub_command cargo
  WIZARD_INTERACTIVE=1
  STEP_OUT="$(_pkgs_board_step < /dev/null 2>&1)"
  assert_contains "$STEP_OUT" "the long way round"
  assert_contains "$STEP_OUT" "takes several"
}

test_without_a_rust_toolchain_the_long_way_is_only_described() {
  _machine Linux armv7l
  stub_command curl
  _run_step
  assert_contains "$STEP_OUT" "cargo install --git https://github.com/LioraLabs/cliban cliban"
  assert_contains "$STEP_OUT" "https://rustup.rs"
}

test_a_chosen_build_runs_exactly_the_command_upstream_documents() {
  _machine Linux armv7l
  stub_command curl
  stub_command cargo
  WIZARD_INTERACTIVE=1
  # yes to building it, and yes to the consent question that shows the command
  printf 'y\ny\n' > "$TEST_TMPDIR/yes"
  _pkgs_board_step < "$TEST_TMPDIR/yes" >/dev/null 2>&1
  assert_stub_called_with cargo \
    "cargo install --git https://github.com/LioraLabs/cliban cliban"
}

# ------------------------------------------- and now the whole run, for real

# _seed_answer <key> <value> - what a previous run left behind. The stage that
# asks is one a resumed run skips, so this is also how a run started with
# --defaults acts on a choice somebody made earlier.
_seed_answer() {
  mkdir -p "$XDG_STATE_HOME/ayeaye"
  printf '%s=%s\n' "$1" "$2" >> "$XDG_STATE_HOME/ayeaye/setup-state"
}

_line_of() { printf '%s\n' "$1" | grep -n -F "$2" | head -1 | cut -d: -f1; }

_assert_before() {
  local haystack="$1" first="$2" second="$3" a b
  a="$(_line_of "$haystack" "$first")"
  b="$(_line_of "$haystack" "$second")"
  [ -n "$a" ] || fail "never said: $first"
  [ -n "$b" ] || fail "never said: $second"
  [ "$a" -lt "$b" ] || fail "\"$first\" must come before \"$second\""
}

test_the_whole_run_says_what_it_will_fetch_before_it_fetches_it() {
  # The rule stage five exists for, end to end rather than by unit: the plan
  # names the download, and the plan is printed before the stage that does it.
  _stub_uname Linux x86_64
  stub_command tmux
  stub_command tar
  stub_command curl --exit 1
  _seed_answer answer.board 1
  run_install --defaults --yes --no-systemd
  _assert_before "$RUN_STDOUT" \
    "cliban v0.1.0, for the project board" \
    "cliban could not be downloaded"
  _assert_before "$RUN_STDOUT" \
    "Before anything changes, here is all of it:" \
    "cliban v0.1.0, for the project board"
}

test_a_board_that_could_not_be_installed_leaves_the_run_finished_anyway() {
  # The whole promise of an optional component: it failed, everything else is
  # still good, the run exits successfully, and the board is on the closing
  # list rather than quietly forgotten.
  _stub_uname Linux x86_64
  stub_command tmux
  stub_command tar
  stub_command curl --exit 1
  _seed_answer answer.board 1
  run_install --defaults --yes --no-systemd
  assert_status 0 "$RUN_STATUS" \
    "an optional component that did not work must not fail the run"
  assert_file_exists "$XDG_CONFIG_HOME/ayeaye/env" "the rest of setup still happened"
  assert_contains "$RUN_STDOUT" "not finished, and worth coming back to"
  assert_contains "$RUN_STDOUT" "cliban, for the project board (did not work)"
  assert_file_missing "$(_installed)"
}

test_the_run_that_was_never_offered_a_board_says_nothing_about_one_at_the_end() {
  _stub_uname Linux x86_64
  stub_command tmux
  stub_command tar
  stub_command curl
  run_install --defaults --no-systemd
  assert_status 0 "$RUN_STATUS"
  assert_not_contains "$RUN_STDOUT" "cliban, for the project board (did not work)"
}

test_settings_that_could_not_be_copied_are_not_changed_either() {
  # The one write in this ticket to a file the user already had that does not
  # go through wizard_replace. It has to keep the same promise by hand: no
  # copy, no change.
  _machine Linux x86_64
  _curl_serves "$(_release_archive x86_64-unknown-linux-musl)"
  _an_env_file
  local before
  before="$(cat "$ENV_FILE")"
  WIZARD_BACKUP_DIR="$TEST_TMPDIR/nowhere/backups"
  printf 'not a directory\n' > "$TEST_TMPDIR/nowhere"
  _run_step
  assert_status 10 "$STEP_STATUS" "the program is here and ayeaye cannot be told"
  assert_eq "$before" "$(cat "$ENV_FILE")" "not one byte"
  assert_contains "$STEP_OUT" "could not save a copy of your settings"
  assert_contains "$STEP_OUT" "VOICE_CLIBAN=$(_installed)"
}

test_a_program_that_will_not_start_is_not_called_installed() {
  _machine Linux x86_64
  local dir out name
  name="cliban-v0.1.0-x86_64-unknown-linux-musl"
  dir="$TEST_TMPDIR/broken"
  out="$TEST_TMPDIR/broken.tar.gz"
  mkdir -p "$dir/$name"
  printf '#!/bin/sh\nexit 7\n' > "$dir/$name/cliban"
  chmod 755 "$dir/$name/cliban"
  ( cd "$dir" && tar -czf "$out" "$name" )
  _curl_serves "$out"
  _an_env_file
  _run_step
  assert_status 10 "$STEP_STATUS"
  assert_contains "$STEP_OUT" "will not start"
  assert_not_contains "$STEP_OUT" "cliban runs"
}

test_an_archive_naming_something_that_only_starts_the_same_way_is_refused() {
  # The member is matched as a whole line: an archive containing
  # "<release>/cliban-notes" is not an archive containing "<release>/cliban".
  _machine Linux x86_64
  local dir out name
  name="cliban-v0.1.0-x86_64-unknown-linux-musl"
  dir="$TEST_TMPDIR/nearly"
  out="$TEST_TMPDIR/nearly.tar.gz"
  mkdir -p "$dir/$name"
  printf 'not the program\n' > "$dir/$name/cliban-notes"
  ( cd "$dir" && tar -czf "$out" "$name" )
  _curl_serves "$out"
  _run_step
  assert_file_missing "$(_installed)"
  assert_contains "$STEP_OUT" "not the cliban release it should be"
}
