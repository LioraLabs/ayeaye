# What a generated service definition contains, on both platforms.
#
# The renderers are pure functions - they write to standard output and touch
# nothing - so these tests call them directly rather than driving an install.
# Two kinds of assertion, deliberately:
#
#   A golden file, compared byte for byte, so that a change to what gets
#   installed on somebody's machine cannot happen by accident: it has to be
#   made in the fixture too.
#
#   Named properties - absolute paths, a restart policy, the settings file
#   referenced, no setting duplicated into the definition - so that a golden
#   diff is never the only thing that fails. A fixture updated without reading
#   it would still trip these.
#
# The fixtures carry @REPO@, @ENV@, @CONF@, @STATE@ and @LOGS@ markers because
# every one of those paths is inside the per-test sandbox. The test fills them
# in before comparing; the renderer has no placeholders at all.

# Values chosen to be findable: if any of them turns up inside a unit or a
# plist, a setting has leaked out of the environment file and into a second
# place where settings live.
_env_with_findable_settings() {
  mkdir -p "$XDG_CONFIG_HOME/ayeaye"
  cat > "$XDG_CONFIG_HOME/ayeaye/env" <<'ENV'
AYEAYE_BIND=10.11.12.13
AYEAYE_PORT=54321
AYEAYE_ALLOWED_HOSTS=findable.example.org
VOICE_NTFY_URL=https://ntfy.example.org/findable
VOICE_WHISPER_SERVER=10.11.12.13:59999
VOICE_WHISPER_MODEL=/models/findable-model.bin
VOICE_WHISPER_THREADS=99
ENV
}

# Source the layer the way install.sh does, with the globals it would have set
# by the time lib/steps is loaded. The stages have to exist first: registration
# is checked eagerly, and a step attaching to a stage that was never declared
# is a caller bug rather than a run that limps on.
_load_service_lib() {
  REPO="${1:-$REPO_ROOT}"
  CONF_DIR="$XDG_CONFIG_HOME/ayeaye"
  ENV_FILE="$CONF_DIR/env"
  WIZARD_STATE_DIR="$XDG_STATE_HOME/ayeaye"
  UNIT_DIR="$XDG_CONFIG_HOME/systemd/user"
  NO_SYSTEMD=0
  . "$REPO_ROOT/lib/wizard.sh"
  local stage
  for stage in welcome detect report configure plan install service finish; do
    wizard_stage "$stage" "$stage"
  done
  . "$REPO_ROOT/lib/steps/70-service.sh"
}

# The golden file with its markers filled in.
_golden() {
  local text
  text="$(fixture_cat "units/$1")"
  text="${text//@REPO@/$REPO}"
  text="${text//@ENV@/$ENV_FILE}"
  text="${text//@CONF@/$XDG_CONFIG_HOME}"
  text="${text//@STATE@/$XDG_STATE_HOME}"
  text="${text//@LOGS@/$HOME/Library/Logs/ayeaye}"
  printf '%s\n' "$text"
}

# ------------------------------------------------------------ golden files

test_the_ayeaye_unit_is_what_the_golden_file_says() {
  _load_service_lib
  assert_eq "$(_golden ayeaye-systemd)" "$(service_render_systemd ayeaye)"
}

test_the_ayeaye_agent_is_what_the_golden_file_says() {
  _load_service_lib
  assert_eq "$(_golden ayeaye-launchd)" "$(service_render_launchd ayeaye)"
}

test_the_voice_agent_unit_is_what_the_golden_file_says() {
  _load_service_lib
  assert_eq "$(_golden voice-agent-systemd)" "$(service_render_systemd voice-agent)"
}

test_the_voice_agent_agent_is_what_the_golden_file_says() {
  _load_service_lib
  assert_eq "$(_golden voice-agent-launchd)" "$(service_render_launchd voice-agent)"
}

# ------------------------------------------------------------- properties

test_both_formats_run_an_absolute_path() {
  _load_service_lib
  assert_contains "$(service_render_systemd ayeaye)" "ExecStart=$REPO/bin/ayeaye"
  assert_contains "$(service_render_launchd ayeaye)" "<string>$REPO/bin/ayeaye</string>"
  # The repo path really is absolute in this sandbox, so the assertions above
  # mean what they say rather than passing on a relative one.
  assert_matches "$REPO" "^/"
}

test_both_formats_carry_a_restart_policy() {
  _load_service_lib
  local unit plist
  unit="$(service_render_systemd ayeaye)"
  plist="$(service_render_launchd ayeaye)"
  assert_contains "$unit" "Restart=on-failure"
  assert_contains "$unit" "RestartSec=5"
  # launchd spells the same intention as "bring it back unless it exited
  # cleanly", plus a floor on how fast it may be restarted.
  assert_contains "$plist" "<key>KeepAlive</key>"
  assert_contains "$plist" "<key>SuccessfulExit</key>"
  assert_contains "$plist" "<false/>"
  assert_contains "$plist" "<key>ThrottleInterval</key>"
}

test_both_formats_start_the_service_at_login() {
  _load_service_lib
  assert_contains "$(service_render_systemd ayeaye)" "WantedBy=default.target"
  assert_contains "$(service_render_launchd ayeaye)" "<key>RunAtLoad</key>"
}

test_the_settings_file_is_referenced_exactly_once_in_the_unit() {
  _load_service_lib
  local count
  count="$(service_render_systemd ayeaye | grep -c "^EnvironmentFile=$ENV_FILE\$")"
  assert_eq "1" "$count" "one reference, and it is the whole of the wiring"
}

test_the_agent_points_at_where_the_settings_are_not_at_what_they_say() {
  _load_service_lib
  local plist
  plist="$(service_render_launchd ayeaye)"
  # launchd has no EnvironmentFile, so the agent is told where the settings
  # live instead - which is the same wiring by another name.
  assert_contains "$plist" "<key>XDG_CONFIG_HOME</key>"
  assert_contains "$plist" "<string>$XDG_CONFIG_HOME</string>"
  assert_contains "$plist" "<key>XDG_STATE_HOME</key>"
}

test_no_setting_from_the_environment_file_is_copied_into_a_definition() {
  # The property the whole design turns on: a person who changes the port
  # edits one file, not two, and a definition installed months ago never
  # disagrees with the settings in force.
  _env_with_findable_settings
  _load_service_lib
  local text name fmt
  for name in ayeaye voice-agent; do
    for fmt in systemd launchd; do
      text="$(service_render_$fmt "$name")"
      assert_not_contains "$text" "10.11.12.13" "$name/$fmt leaked AYEAYE_BIND"
      assert_not_contains "$text" "54321" "$name/$fmt leaked AYEAYE_PORT"
      assert_not_contains "$text" "findable.example.org" "$name/$fmt leaked AYEAYE_ALLOWED_HOSTS"
      assert_not_contains "$text" "ntfy.example.org" "$name/$fmt leaked VOICE_NTFY_URL"
    done
  done
}

test_no_placeholder_survives_either_renderer() {
  _load_service_lib
  local leftovers name fmt
  for name in ayeaye voice-agent; do
    for fmt in systemd launchd; do
      leftovers="$(service_render_$fmt "$name" | grep -o '@[A-Za-z_]*@' | sort -u)"
      assert_eq "" "$leftovers" "$name/$fmt still has an unsubstituted placeholder"
    done
  done
}

test_an_unknown_service_is_a_caller_bug_not_an_empty_file() {
  _load_service_lib
  local out status
  out="$(service_render_systemd no-such-service 2>&1)"
  status=$?
  assert_status 2 "$status"
  assert_eq "" "$out" "nothing is written for a service that does not exist"
  service_render_launchd no-such-service >/dev/null 2>&1
  assert_status 2 "$?"
  service_render_systemd >/dev/null 2>&1
  assert_status 2 "$?" "a missing name is the same kind of mistake"
}

# ------------------------------------------------------------- awkward paths

test_a_repo_path_containing_a_space_survives_both_formats() {
  # macOS paths routinely have spaces in them, and a unit whose ExecStart was
  # split in two would install cleanly and never start.
  local repo="$TEST_TMPDIR/my repo"
  mkdir -p "$repo"
  _load_service_lib "$repo"
  local unit plist
  unit="$(service_render_systemd ayeaye)"
  plist="$(service_render_launchd ayeaye)"
  assert_contains "$unit" "ExecStart=\"$repo/bin/ayeaye\"" \
    "systemd splits an unquoted argument on the space"
  assert_contains "$plist" "<string>$repo/bin/ayeaye</string>" \
    "an XML string element needs no quoting for a space"
}

test_a_repo_path_containing_a_quote_and_a_dollar_survives_both_formats() {
  require_host_command python3
  stub_real python3
  local repo="$TEST_TMPDIR/od\"d \$path"
  mkdir -p "$repo"
  _load_service_lib "$repo"
  local unit
  unit="$(service_render_systemd ayeaye)"
  # systemd expands $NAME inside ExecStart and reads \" as a literal quote, so
  # both have to be escaped for the path to arrive at execve intact.
  assert_contains "$unit" 'ExecStart="'"$TEST_TMPDIR"'/od\"d $$path/bin/ayeaye"'
  # The plist is XML, and what it needs escaping for is different again.
  assert_contains "$(service_render_launchd ayeaye)" \
    "<string>$repo/bin/ayeaye</string>"
}

test_a_repo_path_containing_an_ampersand_is_escaped_for_xml() {
  local repo="$TEST_TMPDIR/rock & roll"
  mkdir -p "$repo"
  _load_service_lib "$repo"
  local plist
  plist="$(service_render_launchd ayeaye)"
  assert_contains "$plist" "rock &amp; roll"
  assert_not_contains "$plist" "rock & roll" "a bare ampersand is not XML"
}

test_the_generated_plist_really_is_a_property_list() {
  require_host_command python3
  stub_real python3
  _load_service_lib
  service_render_launchd ayeaye > "$TEST_TMPDIR/agent.plist"
  local out
  out="$(python3 - "$TEST_TMPDIR/agent.plist" <<'PY'
import plistlib, sys
with open(sys.argv[1], "rb") as fh:
    d = plistlib.load(fh)
print("label=%s" % d["Label"])
print("argv0=%s" % d["ProgramArguments"][0])
print("argc=%d" % len(d["ProgramArguments"]))
print("runatload=%s" % d["RunAtLoad"])
print("keepalive=%s" % d["KeepAlive"]["SuccessfulExit"])
print("throttle=%s" % d["ThrottleInterval"])
print("err=%s" % d["StandardErrorPath"])
PY
)"
  assert_status 0 "$?" "plistlib refused to parse the agent we would install"
  assert_contains "$out" "label=dev.ayeaye"
  assert_contains "$out" "argv0=$REPO/bin/ayeaye"
  assert_contains "$out" "argc=1"
  assert_contains "$out" "runatload=True"
  assert_contains "$out" "keepalive=False"
  assert_contains "$out" "throttle=5"
  assert_contains "$out" "err=$HOME/Library/Logs/ayeaye/ayeaye.log"
}

test_an_awkward_path_still_parses_as_a_property_list() {
  require_host_command python3
  stub_real python3
  local repo="$TEST_TMPDIR/a & b <c> \"d\""
  mkdir -p "$repo"
  _load_service_lib "$repo"
  service_render_launchd ayeaye > "$TEST_TMPDIR/agent.plist"
  local out
  out="$(python3 - "$TEST_TMPDIR/agent.plist" <<'PY'
import plistlib, sys
with open(sys.argv[1], "rb") as fh:
    d = plistlib.load(fh)
print(d["ProgramArguments"][0])
PY
)"
  assert_status 0 "$?"
  assert_eq "$repo/bin/ayeaye" "$out" \
    "the path has to come back out of the XML exactly as it went in"
}

# ------------------------------------------------------------------- inert

test_sourcing_the_step_file_runs_nothing_and_touches_nothing() {
  stub_command systemctl
  stub_command launchctl
  stub_command loginctl
  local out
  out="$(_load_service_lib 2>&1)"
  assert_eq "" "$out" "sourcing a step file must be silent"
  assert_stub_not_called systemctl
  assert_stub_not_called launchctl
  assert_stub_not_called loginctl
  assert_file_missing "$XDG_CONFIG_HOME/systemd/user/ayeaye.service"
  assert_file_missing "$HOME/Library/LaunchAgents/dev.ayeaye.plist"
}

test_the_step_file_registers_where_it_says_it_does() {
  _load_service_lib
  # The plan entry belongs to stage four so that stage five can say a service
  # is about to be installed before it is.
  _wizard_step_known configure service
  assert_status 0 "$?" "the plan entry has to be registered on the configure stage"
  # And the work itself stays where install.sh already registered it, so the
  # health check that runs after it keeps running after it.
  _wizard_step_known service unit
  assert_status 1 "$?" "the install step belongs to install.sh, not to this file"
}
