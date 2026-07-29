# The wizard as a whole: eight stages, in order, in plain language, and the
# two safety properties that have to hold whatever else changes.
#
# The other install_* files pin what the installer does. This one pins the
# shape of the conversation - which is the part six later tickets are going to
# add themselves to, and the part a person actually experiences.

_hard_deps_present() {
  require_host_command python3
  stub_command tmux
  stub_real python3
}

_state() { printf '%s' "$XDG_STATE_HOME/ayeaye/setup-state"; }
_ledger() { printf '%s' "$XDG_STATE_HOME/ayeaye/setup-consent.log"; }

# The line number a substring first appears on, for asserting order.
_line_of() {
  printf '%s\n' "$1" | grep -n -F "$2" | head -1 | cut -d: -f1
}

# A stage heading, as it really appears: bold, which is what tells it apart
# from the same word inside a sentence. "service" is in the welcome text long
# before the stage called service.
_heading() { printf '\033[1m%s\033[0m' "$1"; }

_assert_stage_order() {
  local haystack="$1" first second a b
  shift
  first="$1"
  shift
  while [ "$#" -gt 0 ]; do
    second="$1"
    shift
    a="$(_line_of "$haystack" "$(_heading "$first")")"
    b="$(_line_of "$haystack" "$(_heading "$second")")"
    [ -n "$a" ] || fail "no stage headed \"$first\":
$haystack"
    [ -n "$b" ] || fail "no stage headed \"$second\":
$haystack"
    [ "$a" -lt "$b" ] || fail "the \"$first\" stage must come before \"$second\":
$haystack"
    first="$second"
  done
}

_assert_before() {
  local haystack="$1" first="$2" second="$3" a b
  a="$(_line_of "$haystack" "$first")"
  b="$(_line_of "$haystack" "$second")"
  [ -n "$a" ] || fail "never said: $first"
  [ -n "$b" ] || fail "never said: $second"
  [ "$a" -lt "$b" ] || fail "\"$first\" must come before \"$second\":
$haystack"
}

# ------------------------------------------------------- the eight stages

test_the_eight_stages_arrive_in_order() {
  _hard_deps_present
  run_install --defaults --no-systemd
  assert_status 0 "$RUN_STATUS"
  _assert_stage_order "$RUN_STDOUT" \
    "ayeaye setup" \
    "checking this computer" \
    "what ayeaye can do here" \
    "configuration" \
    "what is about to happen" \
    "setting it up" \
    "service" \
    "done"
}

test_the_first_stage_says_what_may_change_and_what_never_will() {
  _hard_deps_present
  run_install --defaults --no-systemd
  local out="$RUN_STDOUT"
  assert_contains "$out" "What setup may change on this computer:"
  assert_contains "$out" "a settings file at $XDG_CONFIG_HOME/ayeaye/env"
  assert_contains "$out" "a private key at $XDG_STATE_HOME/ayeaye/token"
  assert_contains "$out" "What it will never do without asking first"
  assert_contains "$out" "listens to this computer only"
}

test_the_second_stage_reports_the_machine_from_the_platform_layer() {
  _hard_deps_present
  run_install --defaults --no-systemd
  assert_matches "$RUN_STDOUT" "computer +.+"
  assert_matches "$RUN_STDOUT" "software +.+"
  assert_matches "$RUN_STDOUT" "startup +.+"
}

test_everything_that_happens_is_listed_before_it_happens() {
  _hard_deps_present
  run_install --defaults --no-systemd
  local out="$RUN_STDOUT"
  assert_contains "$out" "Before anything changes, here is all of it:"
  _assert_before "$out" "Before anything changes" "wrote $XDG_CONFIG_HOME/ayeaye/env"
  assert_contains "$out" "settings to write:"
  assert_contains "$out" "ayeaye will answer on 127.0.0.1:8911, this computer only"
}

test_a_run_with_nothing_to_install_asks_no_extra_question() {
  # Stage five only stops for consequential work. Everything here is the user's
  # own settings and the user's own service, so there is nothing to stop for.
  _hard_deps_present
  pty_answers "" "" "" ""
  pty_install --no-systemd
  assert_status 0 "$PTY_STATUS"
  assert_not_contains "$PTY_TRANSCRIPT" "go ahead with all of that?"
  assert_contains "$PTY_TRANSCRIPT" "Before anything changes, here is all of it:" \
    "anchor: the plan really was shown, it just had nothing to stop for"
}

test_the_last_stage_says_how_to_change_it_and_how_to_remove_it() {
  _hard_deps_present
  run_install --defaults --no-systemd
  local out="$RUN_STDOUT"
  assert_contains "$out" "to change any of this later: run ./install.sh again."
  assert_contains "$out" "to remove it: systemctl --user disable --now ayeaye"
  assert_contains "$out" "$XDG_CONFIG_HOME/systemd/user/ayeaye.service"
  assert_contains "$out" "open the bookmark URL once on the phone"
}

test_no_stage_asks_the_user_about_a_unit_file_or_a_umask() {
  # Jargon in a prompt is a defect. This is a blunt instrument and deliberately
  # so: it is the assertion that fails when somebody adds a question worded for
  # the person who wrote it.
  _hard_deps_present
  local word
  pty_answers "" "" "" ""
  pty_install --no-systemd
  for word in "umask" "chmod" "stdin" "XDG_" "systemd unit" "daemon-reload"; do
    assert_not_contains "$PTY_TRANSCRIPT" "$word" \
      "\"$word\" is not language anyone should have to answer a question in"
  done
}

# ------------------------------------------------- what --defaults may not do

test_defaults_never_opens_ayeaye_to_the_network() {
  _hard_deps_present
  run_install --defaults --no-systemd
  assert_file_contains "$XDG_CONFIG_HOME/ayeaye/env" "AYEAYE_BIND=127.0.0.1"
  assert_contains "$(cat "$(_ledger)")" "expose	refused"
  assert_not_contains "$(cat "$(_ledger)")" "expose	granted"
}

test_defaults_grants_no_consent_of_any_kind() {
  # The whole point of the ledger: one place to check that an unattended run
  # decided nothing on anybody's behalf.
  _hard_deps_present
  stub_command_from_fixture tailscale tailscale/status-online
  run_install --defaults --no-systemd
  assert_file_exists "$(_ledger)"
  assert_not_contains "$(cat "$(_ledger)")" "granted" \
    "an unattended run must never grant itself permission"
}

test_defaults_installs_nothing_even_where_it_could() {
  # A machine with a package manager, no tmux, and nobody watching. It must
  # explain rather than act.
  require_host_command python3
  stub_real python3
  stub_command apt-get
  stub_command sudo
  stub_command_from_fixture uname uname/linux-x86_64
  PLATFORM_OS_RELEASE_FILES="$(fixture_file os-release/debian-12)"
  export PLATFORM_OS_RELEASE_FILES
  run_install --defaults --no-systemd
  assert_ne 0 "$RUN_STATUS" "it cannot claim success without the thing it needs"
  assert_stub_not_called apt-get "nothing may be installed unattended"
  assert_stub_not_called sudo
  assert_contains "$RUN_STDOUT" "install the missing packages and re-run."
  assert_contains "$(cat "$(_ledger)")" "privileged	refused"
}

test_a_typed_address_that_is_not_this_computer_is_recorded_as_such() {
  _hard_deps_present
  stdin_lines "0.0.0.0" "8911" "" ""
  run_install --no-systemd
  assert_status 0 "$RUN_STATUS"
  assert_file_contains "$XDG_CONFIG_HOME/ayeaye/env" "AYEAYE_BIND=0.0.0.0"
  assert_contains "$(cat "$(_ledger)")" "expose	granted" \
    "the answer is the consent, and it still belongs in the audit trail"
  assert_contains "$RUN_STDOUT" "which other computers can reach" \
    "and the plan says so in words before it is written"
}

# ------------------------------------------------------------- consent

test_refusing_to_replace_settings_leaves_them_alone() {
  _hard_deps_present
  mkdir -p "$XDG_CONFIG_HOME/ayeaye"
  printf 'AYEAYE_PORT=7777\n# mine\n' > "$XDG_CONFIG_HOME/ayeaye/env"
  local before
  before="$(cat "$XDG_CONFIG_HOME/ayeaye/env")"
  stdin_lines "n"
  run_install --no-systemd
  assert_status 0 "$RUN_STATUS"
  assert_eq "$before" "$(cat "$XDG_CONFIG_HOME/ayeaye/env")" \
    "a refusal must not change a byte"
  assert_contains "$(cat "$(_ledger)")" "replace	refused"
  assert_eq "" "$(find "$XDG_STATE_HOME/ayeaye/backups" -type f 2>/dev/null)" \
    "and nothing was replaced, so there was nothing to back up"
}

test_agreeing_to_replace_settings_backs_the_old_ones_up_first() {
  _hard_deps_present
  mkdir -p "$XDG_CONFIG_HOME/ayeaye"
  printf 'AYEAYE_PORT=7777\n' > "$XDG_CONFIG_HOME/ayeaye/env"
  stdin_lines "y" "" "9000" "" ""
  run_install --no-systemd
  assert_status 0 "$RUN_STATUS"
  local backups
  backups="$(find "$XDG_STATE_HOME/ayeaye/backups" -type f -name 'env.*')"
  assert_ne "" "$backups" "the old settings must be recoverable"
  assert_file_contains "$backups" "AYEAYE_PORT=7777"
  assert_contains "$RUN_STDOUT" "saved a copy of your old settings at" \
    "a backup nobody is told about is a backup nobody can use"
  assert_file_contains "$XDG_CONFIG_HOME/ayeaye/env" "AYEAYE_PORT=9000"
}

test_settings_the_wizard_never_asked_about_survive_a_rewrite() {
  _hard_deps_present
  mkdir -p "$XDG_CONFIG_HOME/ayeaye"
  cat > "$XDG_CONFIG_HOME/ayeaye/env" <<'ENV'
# the note I wrote to myself
AYEAYE_PORT=7777
AYEAYE_NAME=the-workshop
AYEAYE_LINES=48
SOMETHING_NOBODY_HAS_HEARD_OF=1
ENV
  stdin_lines "y" "" "9000" "" ""
  run_install --no-systemd
  assert_status 0 "$RUN_STATUS"
  local env_file="$XDG_CONFIG_HOME/ayeaye/env"
  assert_file_contains "$env_file" "AYEAYE_PORT=9000" "what was asked about changed"
  assert_file_contains "$env_file" "AYEAYE_NAME=the-workshop"
  assert_file_contains "$env_file" "AYEAYE_LINES=48"
  assert_file_contains "$env_file" "SOMETHING_NOBODY_HAS_HEARD_OF=1"
  assert_file_contains "$env_file" "# the note I wrote to myself"
}

test_the_settings_file_is_not_readable_by_anybody_else() {
  # It is documented as a place to put AYEAYE_TOKEN.
  _hard_deps_present
  run_install --defaults --no-systemd
  assert_file_mode 600 "$XDG_CONFIG_HOME/ayeaye/env"
}

# -------------------------------------------------------- the state file

test_a_run_records_the_stages_it_finished() {
  _hard_deps_present
  run_install --defaults --no-systemd
  assert_file_exists "$(_state)"
  assert_file_mode 600 "$(_state)"
  assert_file_contains "$(_state)" "stage.welcome=done"
  assert_file_contains "$(_state)" "stage.configure=done"
  assert_file_contains "$(_state)" "stage.install=done"
  assert_file_contains "$(_state)" "run.completed=1"
}

test_a_run_records_what_the_user_chose() {
  _hard_deps_present
  stdin_lines "10.1.2.3" "9911" "one.example" "https://ntfy.example/t"
  run_install --no-systemd
  assert_file_contains "$(_state)" "answer.bind=10.1.2.3"
  assert_file_contains "$(_state)" "answer.port=9911"
  assert_file_contains "$(_state)" "answer.hosts=one.example"
  assert_file_contains "$(_state)" "answer.ntfy=https://ntfy.example/t"
}

test_forgetting_everything_is_a_supported_thing_to_want() {
  _hard_deps_present
  run_install --defaults --no-systemd
  assert_file_contains "$(_state)" "answer.port=8911"
  stdin_lines "n"
  run_install --no-systemd --fresh
  assert_status 0 "$RUN_STATUS"
  assert_file_contains "$(_state)" "run.count=1" \
    "--fresh starts the count over, because it started the run over"
}

# ------------------------------------------------- honest about seams

test_work_this_version_cannot_do_is_reported_as_not_done() {
  # macOS is another ticket's. What matters here is the shape of the answer:
  # the stage says what it could not do, the stage is not recorded as done, and
  # the closing summary lists it.
  require_host_command python3
  stub_real python3
  stub_command tmux
  stub_command launchctl
  stub_command uname --exit 1
  stub_when uname '-s' --stdout "Darwin"
  stub_when uname '-m' --stdout "arm64"
  stub_command_from_fixture sw_vers sw_vers/macos-15.1
  PLATFORM_OS_RELEASE_FILES=""
  export PLATFORM_OS_RELEASE_FILES
  run_install --defaults
  assert_status 0 "$RUN_STATUS" "unfinished optional work is not a failed install"
  assert_contains "$RUN_STDOUT" "not set up for"
  assert_contains "$RUN_STDOUT" "not finished, and worth coming back to:"
  assert_file_contains "$(_state)" "stage.service=pending" \
    "a stage holding work it could not do is never recorded as done"
  assert_file_not_contains "$(_state)" "stage.service=done"
}
