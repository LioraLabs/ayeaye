# Running setup again on a machine that already has ayeaye.
#
# The single most damaging thing this code could do is regenerate the auth
# token. Somebody's phone has a bookmark with that token in it; a new one
# means a bookmark that silently stops working, and no message anywhere
# explaining why. Every test in the first section is that property looked at
# from a different angle.
#
# The second property is narrower and just as easy to break: repairing a
# service repairs the service. Not the settings, not the key, not somebody
# else's unit that happens to live in the same directory.

_hard_deps_present() {
  require_host_command python3
  stub_command tmux
  stub_real python3
}

_systemd_session() {
  stub_command systemctl --exit 1 --stderr "unknown systemctl invocation"
  stub_when systemctl '--user show-environment' --exit 0
  stub_when systemctl '--user daemon-reload' --exit 0
  stub_when systemctl '--user enable --now *' --exit 0
}

# The same session, with our unit already up. `status` is how this project
# asks, so that is the rule that changes.
_already_running() {
  stub_command systemctl --exit 1 --stderr "unknown systemctl invocation"
  stub_when systemctl '--user show-environment' --exit 0
  stub_when systemctl '--user daemon-reload' --exit 0
  stub_when systemctl '--user status *' --exit 0 --stdout "active (running)"
  stub_when systemctl '--user stop *' --exit 0
  stub_when systemctl '--user start *' --exit 0
  stub_when systemctl '--user enable --now *' --exit 0
}

_unit()  { printf '%s' "$XDG_CONFIG_HOME/systemd/user/ayeaye.service"; }
_env()   { printf '%s' "$XDG_CONFIG_HOME/ayeaye/env"; }
_token() { printf '%s' "$XDG_STATE_HOME/ayeaye/token"; }

# The bytes of a file, as something two assertions apart can compare.
_bytes() { cksum < "$1"; }

# --------------------------------------------------------------- the token

test_a_second_run_does_not_regenerate_the_token() {
  _hard_deps_present
  _systemd_session
  run_install --defaults
  assert_file_exists "$(_token)"
  local before
  before="$(_bytes "$(_token)")"
  run_install --defaults
  assert_status 0 "$RUN_STATUS"
  assert_eq "$before" "$(_bytes "$(_token)")" \
    "a new token silently breaks every bookmark already on a phone"
  assert_file_mode 600 "$(_token)"
}

test_reinstalling_the_service_does_not_touch_the_token() {
  # Narrower than the test above and the one that would actually catch a
  # regression in this ticket: the service really is reinstalled - the unit
  # was deleted, and it comes back - and the token is still the same bytes.
  _hard_deps_present
  _systemd_session
  run_install --defaults
  local before
  before="$(_bytes "$(_token)")"
  rm -f "$(_unit)"
  run_install --defaults
  assert_file_exists "$(_unit)" "anchor: the service really was reinstalled"
  assert_eq "$before" "$(_bytes "$(_token)")"
  assert_file_mode 600 "$(_token)"
  assert_contains "$RUN_STDOUT" "keeping the key you already have"
}

test_repairing_a_running_service_does_not_touch_the_token() {
  _hard_deps_present
  _already_running
  run_install --defaults
  local before
  before="$(_bytes "$(_token)")"
  printf 'hand-edited nonsense\n' > "$(_unit)"
  run_install --defaults
  assert_contains "$RUN_STDOUT" "restarted on the new" "anchor: it really was repaired"
  assert_eq "$before" "$(_bytes "$(_token)")"
  assert_file_mode 600 "$(_token)"
}

test_the_token_survives_a_move_to_a_platform_it_was_not_made_on() {
  # An install made on Linux, opened on a Mac: same home directory over a
  # network mount, or a restored backup. The service definition is rewritten
  # in the other format and the key is still the key.
  _hard_deps_present
  _systemd_session
  run_install --defaults
  local before
  before="$(_bytes "$(_token)")"
  stub_command uname --exit 1
  stub_when uname '-s' --stdout "Darwin"
  stub_when uname '-m' --stdout "arm64"
  stub_command_from_fixture sw_vers sw_vers/macos-15.1
  stub_command launchctl --exit 1
  PLATFORM_OS_RELEASE_FILES=""
  export PLATFORM_OS_RELEASE_FILES
  run_install --defaults
  assert_status 0 "$RUN_STATUS"
  assert_file_exists "$HOME/Library/LaunchAgents/dev.ayeaye.plist"
  assert_eq "$before" "$(_bytes "$(_token)")"
  assert_file_mode 600 "$(_token)"
}

# ------------------------------------------------------------- the settings

test_a_rerun_that_keeps_the_settings_leaves_the_file_byte_for_byte() {
  _hard_deps_present
  _systemd_session
  stdin_lines "10.9.8.7" "9099" "kept.example.org" "https://ntfy.example/kept"
  run_install
  local before
  before="$(_bytes "$(_env)")"
  # "no" to the one question a rerun asks: do not change what is there.
  stdin_lines "n"
  run_install
  assert_status 0 "$RUN_STATUS"
  assert_contains "$RUN_STDOUT" "keeping the settings that are already there."
  assert_eq "$before" "$(_bytes "$(_env)")"
  assert_file_contains "$(_env)" "AYEAYE_PORT=9099" "anchor: it is still the file we wrote"
}

test_a_comment_somebody_added_to_the_settings_survives_a_service_reinstall() {
  _hard_deps_present
  _systemd_session
  run_install --defaults
  printf '\n# my own note, do not lose this\nMY_OWN_KEY=mine\n' >> "$(_env)"
  local before
  before="$(_bytes "$(_env)")"
  rm -f "$(_unit)"
  run_install --defaults
  assert_file_exists "$(_unit)" "anchor: the service really was reinstalled"
  assert_eq "$before" "$(_bytes "$(_env)")" \
    "reinstalling a service is not a reason to rewrite anybody's settings"
  assert_file_contains "$(_env)" "# my own note, do not lose this"
}

test_nothing_in_the_service_step_writes_the_settings_or_the_key() {
  # A lint rather than a run: the tests above can only see the paths they
  # happen to exercise, and this can see the whole file.
  stub_real grep
  local offenders
  offenders="$(grep -n -E '(ENV_FILE|TOKEN_FILE|wizard_env_render|wizard_env_merge)' \
    "$REPO_ROOT/lib/steps/70-service.sh" \
    | grep -v -E '^[0-9]+:[[:space:]]*#' \
    | grep -E '>|wizard_env_render|wizard_env_merge|chmod|rm ' || true)"
  assert_eq "" "$offenders" \
    "the service step reads the settings file and never writes it, and has no
business with the token at all"
}

# ------------------------------------------------- somebody else's units

test_an_unrelated_unit_in_the_same_directory_is_never_touched() {
  _hard_deps_present
  _systemd_session
  mkdir -p "$XDG_CONFIG_HOME/systemd/user"
  printf '[Unit]\nDescription=not ours\n[Service]\nExecStart=/bin/true\n' \
    > "$XDG_CONFIG_HOME/systemd/user/something-else.service"
  local before
  before="$(_bytes "$XDG_CONFIG_HOME/systemd/user/something-else.service")"
  run_install --defaults
  assert_file_exists "$(_unit)"
  assert_eq "$before" "$(_bytes "$XDG_CONFIG_HOME/systemd/user/something-else.service")"
}

test_a_voice_agent_unit_somebody_already_has_is_brought_up_to_date() {
  # Its unit used to be a template a person copied by hand, and that template
  # is gone. Somebody who copied it should end up with a correct file rather
  # than a stale one naming a path this repository has since moved from.
  _hard_deps_present
  _systemd_session
  mkdir -p "$XDG_CONFIG_HOME/systemd/user"
  printf '[Service]\nExecStart=/where/it/used/to/be/bin/voice-agent\n' \
    > "$XDG_CONFIG_HOME/systemd/user/voice-agent.service"
  run_install --defaults
  assert_file_contains "$XDG_CONFIG_HOME/systemd/user/voice-agent.service" \
    "ExecStart=$REPO_ROOT/bin/voice-agent"
  assert_contains "$RUN_STDOUT" "brought your voice-agent definition up to date as well"
}

test_a_voice_agent_unit_nobody_has_is_not_installed_by_surprise() {
  _hard_deps_present
  _systemd_session
  run_install --defaults
  assert_file_exists "$(_unit)"
  assert_file_missing "$XDG_CONFIG_HOME/systemd/user/voice-agent.service" \
    "setup installs what was asked for, and voice is a separate decision"
  assert_not_contains "$RUN_STDOUT" "brought your voice-agent"
}

# ------------------------------------------------------------------ repair

test_a_hand_edited_unit_is_kept_before_it_is_replaced() {
  _hard_deps_present
  _systemd_session
  run_install --defaults
  printf '[Service]\nExecStart=/my/own/thing\n' > "$(_unit)"
  run_install --defaults
  assert_file_contains "$(_unit)" "ExecStart=$REPO_ROOT/bin/ayeaye"
  assert_contains "$RUN_STDOUT" "saved a copy of the previous ayeaye definition at"
  local backup
  backup="$(ls "$XDG_STATE_HOME/ayeaye/backups"/ayeaye.service.* | head -1)"
  assert_file_contains "$backup" "ExecStart=/my/own/thing" \
    "the copy has to be the old bytes, not the new ones"
}

test_an_unchanged_unit_is_not_rewritten_and_not_backed_up() {
  # Re-running setup on a machine where nothing changed should leave the disk
  # exactly as it found it. A backup on every run is noise that makes the one
  # backup somebody actually needs impossible to find.
  _hard_deps_present
  _systemd_session
  run_install --defaults
  run_install --defaults
  assert_status 0 "$RUN_STATUS"
  assert_not_contains "$RUN_STDOUT" "saved a copy of the previous"
  assert_file_missing "$XDG_STATE_HOME/ayeaye/backups/ayeaye.service" \
    "nothing needed keeping"
}

test_a_running_service_is_restarted_only_when_its_definition_changed() {
  _hard_deps_present
  _already_running
  run_install --defaults
  # Re-declaring the stub empties its recording, so what is asserted below is
  # what the *second* run did and not what both runs did between them. The
  # first run legitimately restarts: it wrote the unit for the first time.
  _already_running
  # Second run, nothing changed: no stop, no start, and it says why.
  run_install --defaults
  assert_not_contains "$(stub_calls systemctl)" "--user stop"
  assert_not_contains "$(stub_calls systemctl)" "--user start"
  assert_contains "$RUN_STDOUT" "ayeaye is already set up to start when you log in, and running."
}

test_a_running_service_picks_up_a_definition_that_did_change() {
  # `enable --now` does nothing to an already-active unit, so without an
  # explicit restart, moving the repository and re-running setup would report
  # success while the old process carried on from the old path.
  _hard_deps_present
  _already_running
  run_install --defaults
  printf 'hand-edited nonsense\n' > "$(_unit)"
  run_install --defaults
  assert_status 0 "$RUN_STATUS"
  assert_stub_called_with systemctl "systemctl --user stop ayeaye.service"
  assert_stub_called_with systemctl "systemctl --user start ayeaye.service"
  assert_contains "$RUN_STDOUT" "ayeaye was already running, so it has been restarted on the new"
  assert_file_contains "$XDG_STATE_HOME/ayeaye/setup-state" "step.service.unit.started=1"
}

test_a_repair_asks_nothing_about_starting_something_already_started() {
  _hard_deps_present
  _already_running
  run_install --defaults
  rm -f "$(_unit)"
  run_install --defaults
  assert_not_contains "$RUN_OUTPUT" "enable and start it now?" \
    "it is already running; asking would be asking about nothing"
}
