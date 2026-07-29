# Claude Code's status line, and the settings file it is configured in.
#
# This is somebody else's configuration file. It may have been theirs for
# months, it may hold things this project has never heard of, and setup adds
# exactly one key to it. Everything in this file is about that one promise:
# read it, show the change, ask, take a copy, and only then write.

setup() {
  require_host_command python3
  stub_real python3 cmp
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
  wizard_remember answer.agent.marker 1
}

_declare_the_stages() {
  local stage
  for stage in welcome detect report configure plan install service finish; do
    wizard_stage "$stage" "$stage" || true
  done
}

_settings() { printf '%s/.claude/settings.json' "$HOME"; }
_script()   { printf '%s/ayeaye/statusline-command.sh' "$XDG_DATA_HOME"; }

_write_settings() {
  mkdir -p "$HOME/.claude"
  cat > "$(_settings)"
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
  STEP_OUT="$(_pkgs_marker_step 2>&1)"
  STEP_STATUS=$?
  wizard_state_load
  return 0
}

_run_step_answering() {
  STEP_OUT="$(_pkgs_marker_step < "$(_answers "$@")" 2>&1)"
  STEP_STATUS=$?
  wizard_state_load
  return 0
}

# _key <name> - a top-level value out of the settings file, through python so
# the test reads it the same way anything else would.
_key() {
  python3 -c 'import json,sys
print(json.dumps(json.load(open(sys.argv[1])).get(sys.argv[2])))' "$(_settings)" "$1"
}

_backups() { ls "$WIZARD_BACKUP_DIR" 2>/dev/null; }

# ---------------------------------------------------------------- nothing yet

test_a_computer_with_no_claude_settings_gets_a_file_with_one_thing_in_it() {
  _run_step
  assert_status 0 "$STEP_STATUS"
  assert_file_exists "$(_settings)"
  assert_eq '"command"' "$(_key statusLine | python3 -c 'import json,sys
print(json.dumps(sorted(json.loads(sys.stdin.read()).keys())[0]))')"
  assert_contains "$(cat "$(_settings)")" "statusline-command.sh"
  assert_contains "$STEP_OUT" "no settings of yours to change"
}

test_the_status_line_script_is_written_where_setup_can_own_it() {
  _run_step
  assert_file_exists "$(_script)"
  assert_file_mode 755 "$(_script)"
  local body
  body="$(cat "$(_script)")"
  assert_contains "$body" "session_id" "it reads the conversation out of the JSON"
  assert_contains "$body" "cc:" "and prints the tag ayeaye looks for"
  assert_not_contains "$body" "jq" "no dependency this project does not already have"
}

test_the_marker_the_script_prints_is_the_one_ayeaye_reads() {
  # bin/ayeaye matches this with a regular expression. The two have to agree,
  # and nothing else in the repository would notice if they stopped.
  _run_step
  local rendered
  rendered="$(printf '%s' '{"session_id":"abcdef0123456789","workspace":{"current_dir":"/tmp/x"}}' \
    | sh "$(_script)")"
  assert_contains "$rendered" "cc:abcdef01"
  assert_matches "$(cat "$REPO_ROOT/bin/ayeaye")" 'cc:\(\[0-9a-f\]\{8\}\)' \
    "the pattern bin/ayeaye searches panes for"
}

test_the_script_survives_a_settings_payload_with_no_session_in_it() {
  _run_step
  local rendered status
  rendered="$(printf '%s' '{"workspace":{"current_dir":"/tmp/x"}}' | sh "$(_script)")"
  status=$?
  assert_status 0 "$status" "a status line that fails is a status line that hides"
  assert_not_contains "$rendered" "cc:"
}

# --------------------------------------------------- somebody else's settings

test_every_other_setting_comes_through_untouched() {
  _write_settings <<'JSON'
{
  "model": "opus",
  "permissions": {"allow": ["Bash(git status)"], "deny": []},
  "env": {"FOO": "bar"},
  "autoMemoryEnabled": true
}
JSON
  _run_step
  assert_status 0 "$STEP_STATUS"
  assert_eq '"opus"' "$(_key model)"
  assert_eq '{"allow": ["Bash(git status)"], "deny": []}' "$(_key permissions)"
  assert_eq '{"FOO": "bar"}' "$(_key env)"
  assert_eq 'true' "$(_key autoMemoryEnabled)"
  assert_contains "$(cat "$(_settings)")" "statusline-command.sh"
}

test_a_copy_of_the_old_file_is_taken_before_it_is_changed() {
  _write_settings <<'JSON'
{"model": "opus"}
JSON
  _run_step
  local saved
  saved="$(_backups)"
  assert_ne "" "$saved" "wizard_backup keeps the version they had"
  assert_contains "$(cat "$WIZARD_BACKUP_DIR/$saved")" '"model": "opus"'
  assert_not_contains "$(cat "$WIZARD_BACKUP_DIR/$saved")" "statusLine" \
    "the copy is of the file as it was, not as it became"
}

test_the_change_is_shown_before_the_question_is_asked() {
  _write_settings <<'JSON'
{"model": "opus", "permissions": {}}
JSON
  WIZARD_INTERACTIVE=1
  WIZARD_AUTO_CONSENT=""
  _run_step_answering n
  assert_contains "$STEP_OUT" "already has: model,permissions"
  assert_contains "$STEP_OUT" "All of that stays exactly as it is."
  assert_contains "$STEP_OUT" "The one change:"
}

test_saying_no_leaves_the_file_exactly_as_it_was() {
  _write_settings <<'JSON'
{"model": "opus"}
JSON
  local before
  before="$(cat "$(_settings)")"
  WIZARD_INTERACTIVE=1
  WIZARD_AUTO_CONSENT=""
  _run_step_answering n
  assert_status 11 "$STEP_STATUS" "the user said no; that is finished business"
  assert_eq "$before" "$(cat "$(_settings)")"
  assert_contains "$STEP_OUT" "Nothing about Claude Code changed."
}

test_an_unattended_run_never_changes_settings_it_was_not_told_to() {
  _write_settings <<'JSON'
{"model": "opus"}
JSON
  local before
  before="$(cat "$(_settings)")"
  WIZARD_AUTO_CONSENT=""
  _run_step
  assert_eq "$before" "$(cat "$(_settings)")"
  assert_status 11 "$STEP_STATUS"
}

# ------------------------------------------------ a status line already there

test_a_status_line_the_user_already_has_is_never_taken_away_silently() {
  _write_settings <<'JSON'
{"statusLine": {"type": "command", "command": "/home/me/my-own-status.sh"}}
JSON
  local before
  before="$(cat "$(_settings)")"
  WIZARD_INTERACTIVE=1
  WIZARD_AUTO_CONSENT=""
  _run_step_answering n
  assert_status 10 "$STEP_STATUS" \
    "pending: the marker is still not configured, and that is worth coming back to"
  assert_eq "$before" "$(cat "$(_settings)")"
  assert_contains "$STEP_OUT" "/home/me/my-own-status.sh"
  assert_contains "$STEP_OUT" "replacing yours would lose whatever else yours shows"
  assert_contains "$STEP_OUT" "copy the last few lines of"
}

test_a_status_line_the_user_already_has_can_be_replaced_when_they_say_so() {
  _write_settings <<'JSON'
{"model": "opus", "statusLine": {"type": "command", "command": "/home/me/mine.sh", "padding": 0}}
JSON
  WIZARD_INTERACTIVE=1
  WIZARD_AUTO_CONSENT=""
  # yes to replacing the status line, yes to changing the file
  _run_step_answering y y
  assert_status 0 "$STEP_STATUS"
  assert_eq '"opus"' "$(_key model)" "everything else still survives"
  assert_contains "$(cat "$(_settings)")" "statusline-command.sh"
  assert_contains "$(cat "$(_settings)")" '"padding": 0' \
    "and the parts of their status line setting that are not the command"
  assert_ne "" "$(_backups)"
}

test_running_setup_twice_changes_nothing_the_second_time() {
  _run_step
  assert_status 0 "$STEP_STATUS"
  local after_first
  after_first="$(cat "$(_settings)")"
  _run_step
  assert_status 0 "$STEP_STATUS"
  assert_eq "$after_first" "$(cat "$(_settings)")"
  assert_contains "$STEP_OUT" "already set up to print the marker"
  assert_eq "" "$(_backups)" "nothing was replaced, so nothing was backed up"
}

# ------------------------------------------------------- a file it cannot read

test_a_settings_file_that_is_not_readable_json_is_never_guessed_at() {
  _write_settings <<'JSON'
{"model": "opus",   <- half a thought, saved by accident
JSON
  local before
  before="$(cat "$(_settings)")"
  WIZARD_INTERACTIVE=1
  WIZARD_AUTO_CONSENT=""
  _run_step_answering n
  assert_status 11 "$STEP_STATUS"
  assert_eq "$before" "$(cat "$(_settings)")" "not one byte"
  assert_contains "$STEP_OUT" "is not something this can read"
  assert_contains "$STEP_OUT" "save a copy of it"
}

test_a_settings_file_that_is_not_readable_json_is_kept_when_it_is_replaced() {
  _write_settings <<'JSON'
{"model": "opus",   <- half a thought, saved by accident
JSON
  local before saved
  before="$(cat "$(_settings)")"
  _run_step
  assert_status 0 "$STEP_STATUS"
  saved="$(_backups)"
  assert_ne "" "$saved" "whatever it meant is in the copy"
  assert_eq "$before" "$(cat "$WIZARD_BACKUP_DIR/$saved")"
  assert_contains "$(cat "$(_settings)")" "statusline-command.sh"
}

test_an_empty_settings_file_is_treated_as_no_settings() {
  mkdir -p "$HOME/.claude"
  : > "$(_settings)"
  _run_step
  assert_status 0 "$STEP_STATUS"
  assert_contains "$(cat "$(_settings)")" "statusline-command.sh"
}

# ------------------------------------------------------------ not wanted

test_nothing_happens_when_the_marker_was_not_asked_for() {
  wizard_remember answer.agent.marker 0
  _run_step
  assert_status 11 "$STEP_STATUS"
  assert_file_missing "$(_settings)"
  assert_file_missing "$(_script)"
  assert_contains "$STEP_OUT" "Codex needs nothing set up"
}

test_codex_is_said_to_need_nothing_wherever_this_step_ends_up() {
  _run_step
  assert_contains "$STEP_OUT" "Codex needs nothing set up" \
    "somebody who only uses Codex should not go looking for the same setting"
}

test_without_python3_the_settings_file_is_left_alone_rather_than_rewritten() {
  stub_remove python3
  _run_step
  assert_status 10 "$STEP_STATUS" "pending, not done and not failed"
  assert_file_missing "$(_settings)"
  assert_contains "$STEP_OUT" "left alone"
}

# ------------------------------------------------- a script they have edited

test_a_status_line_script_the_user_has_edited_is_not_overwritten_silently() {
  mkdir -p "$XDG_DATA_HOME/ayeaye"
  printf '#!/bin/sh\n# mine now\n' > "$(_script)"
  WIZARD_INTERACTIVE=1
  WIZARD_AUTO_CONSENT=""
  # no to rewriting the script, no to touching the settings file
  _run_step_answering n n
  assert_contains "$(cat "$(_script)")" "# mine now"
  assert_contains "$STEP_OUT" "keeping your version of"
}

test_a_settings_file_kept_private_stays_private() {
  _write_settings <<'JSON'
{"model": "opus"}
JSON
  chmod 600 "$(_settings)"
  _run_step
  assert_status 0 "$STEP_STATUS"
  assert_file_mode 600 "$(_settings)" \
    "adding a line must not be how somebody's settings become readable"
}

test_nothing_is_left_beside_the_settings_file() {
  _write_settings <<'JSON'
{"model": "opus"}
JSON
  _run_step
  assert_eq "settings.json" "$(ls "$HOME/.claude")" \
    "one file, and no half-written one beside it"
}

test_the_order_the_settings_were_written_in_survives() {
  # Not just "every key is still there": a rewrite that reordered somebody's
  # file would pass a key-by-key check and would still be a diff they did not
  # ask for, in a file they may keep under version control.
  _write_settings <<'JSON'
{"zzz": 1, "aaa": 2, "mmm": {"nested": true}}
JSON
  _run_step
  assert_status 0 "$STEP_STATUS"
  assert_eq "zzz,aaa,mmm,statusLine" \
    "$(python3 -c 'import json,sys
print(",".join(json.load(open(sys.argv[1])).keys()))' "$(_settings)")" \
    "in the order they were in, with the one addition at the end"
}

test_a_status_line_that_is_not_a_command_is_never_half_converted() {
  # A status line configured as something other than a command has no command
  # to change, and merging into it would leave half of each. The user is told
  # it exists, asked, and it is replaced whole.
  _write_settings <<'JSON'
{"statusLine": {"type": "static", "text": "my own thing"}}
JSON
  WIZARD_INTERACTIVE=1
  WIZARD_AUTO_CONSENT=""
  _run_step_answering n
  assert_status 10 "$STEP_STATUS"
  assert_contains "$STEP_OUT" "already has a status line of its own"
  assert_contains "$STEP_OUT" "something other than the output of a command"
  assert_contains "$(cat "$(_settings)")" '"my own thing"' "and it is still theirs"

  _run_step_answering y y
  assert_status 0 "$STEP_STATUS"
  assert_eq '{"type": "command", "command": "'"$(_script)"'"}' "$(_key statusLine)" \
    "replaced whole, not merged into"
}

test_the_settings_it_promises_to_keep_never_include_the_one_it_changes() {
  _write_settings <<'JSON'
{"model": "opus", "statusLine": {"type": "command", "command": "/home/me/mine.sh"}}
JSON
  WIZARD_INTERACTIVE=1
  WIZARD_AUTO_CONSENT=""
  _run_step_answering n
  assert_contains "$STEP_OUT" "already has: model"
  assert_not_contains "$STEP_OUT" "already has: model,statusLine" \
    "\"all of that stays exactly as it is\" must not be said about the one thing
that would not have"
}

test_a_settings_file_that_is_a_list_rather_than_an_object_is_kept() {
  _write_settings <<'JSON'
["this was never a settings file"]
JSON
  local before saved
  before="$(cat "$(_settings)")"
  _run_step
  assert_status 0 "$STEP_STATUS"
  saved="$(_backups)"
  assert_ne "" "$saved"
  assert_eq "$before" "$(cat "$WIZARD_BACKUP_DIR/$saved")"
  assert_contains "$(cat "$(_settings)")" "statusline-command.sh"
}

test_a_settings_file_setup_may_not_read_is_never_called_an_absent_one() {
  _write_settings <<'JSON'
{"model": "opus"}
JSON
  chmod 000 "$(_settings)"
  _run_step
  chmod 644 "$(_settings)"
  assert_status 10 "$STEP_STATUS"
  assert_contains "$STEP_OUT" "not allowed to read it"
  assert_not_contains "$STEP_OUT" "no settings of yours to change" \
    "saying somebody has no settings, about a file full of theirs, is the one
answer that must never be given"
  assert_eq '{"model": "opus"}' "$(cat "$(_settings)")"
}

test_a_settings_file_that_is_a_symlink_stays_one() {
  # Every dotfiles manager makes this file a symlink. Replacing it with a
  # regular file makes the file they edit and the file Claude Code reads two
  # different files, silently.
  mkdir -p "$HOME/.claude" "$HOME/dotfiles"
  printf '{"model": "opus"}\n' > "$HOME/dotfiles/settings.json"
  ln -s "$HOME/dotfiles/settings.json" "$(_settings)"
  _run_step
  assert_status 0 "$STEP_STATUS"
  [ -L "$(_settings)" ] || fail "it is not a symlink any more"
  assert_file_contains "$HOME/dotfiles/settings.json" "statusline-command.sh" \
    "and the change reached the file the symlink points at"
}
