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
  count="$(service_render_systemd ayeaye | grep -c "^EnvironmentFile=-$ENV_FILE\$")"
  assert_eq "1" "$count" "one reference, and it is the whole of the wiring"
}

test_a_settings_file_that_is_not_there_yet_does_not_stop_the_service() {
  _load_service_lib
  # EnvironmentFile= without the "-" makes a missing file a fatal start error,
  # and with Restart=on-failure that is a restart loop rather than a program
  # running on its defaults.
  assert_matches "$(service_render_systemd ayeaye)" "^EnvironmentFile=-/"
}

test_a_percent_sign_in_the_settings_path_is_escaped_for_systemd() {
  # systemd runs specifier expansion over EnvironmentFile= as well as over
  # ExecStart=, and an unknown specifier is not an error - it drops the whole
  # directive with a warning nobody reads. The unit would start perfectly and
  # run with none of the user's settings, which is the one outcome this
  # design exists to make impossible.
  _load_service_lib
  ENV_FILE="$TEST_TMPDIR/100%/ayeaye/env"
  local unit
  unit="$(service_render_systemd ayeaye)"
  assert_contains "$unit" "EnvironmentFile=-$TEST_TMPDIR/100%%/ayeaye/env"
  assert_not_contains "$unit" "EnvironmentFile=-$TEST_TMPDIR/100%/ayeaye/env"
}

test_the_golden_comparison_is_byte_for_byte() {
  # The comparisons above go through $( ), which strips trailing newlines from
  # both sides and so cannot see one. This one does not.
  _load_service_lib
  _golden ayeaye-systemd > "$TEST_TMPDIR/golden"
  service_render_systemd ayeaye > "$TEST_TMPDIR/rendered"
  assert_eq "$(cksum < "$TEST_TMPDIR/golden")" "$(cksum < "$TEST_TMPDIR/rendered")" \
    "including the trailing newline"
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

test_the_agent_can_find_tmux_where_a_mac_actually_keeps_it() {
  # launchd hands a GUI agent PATH=/usr/bin:/bin:/usr/sbin:/sbin and nothing
  # else. tmux on a Mac comes from Homebrew, under one prefix on Apple silicon
  # and another on Intel, and neither is on that list. Without this the agent
  # starts, answers, passes its health check, and shows an empty list of
  # sessions forever - bin/ayeaye reads a tmux it cannot run as a tmux with
  # nothing in it.
  _load_service_lib
  local plist
  plist="$(service_render_launchd ayeaye)"
  assert_contains "$plist" "<key>PATH</key>"
  assert_contains "$plist" "/opt/homebrew/bin"
  assert_contains "$plist" "/usr/local/bin"
  assert_not_contains "$plist" "$STUB_BIN" \
    "a copy of whatever PATH happened to be exported is not a description of a Mac"
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

test_a_repo_path_containing_a_dollar_or_a_percent_survives_the_unit() {
  # systemd expands $NAME and its own %h-style specifiers inside ExecStart, so
  # both have to be doubled for the path to arrive at execve intact.
  local repo="$TEST_TMPDIR/100% \$path"
  mkdir -p "$repo"
  _load_service_lib "$repo"
  assert_contains "$(service_render_systemd ayeaye)" \
    'ExecStart="'"$TEST_TMPDIR"'/100%% $$path/bin/ayeaye"'
  assert_contains "$(service_render_launchd ayeaye)" \
    "<string>$repo/bin/ayeaye</string>" "neither is special in XML"
}

test_a_quote_or_a_backslash_in_the_path_is_something_systemd_cannot_express() {
  # There is no escaping for these. systemd unquotes the executable word and
  # then rejects the result if it holds a quote, a backslash or a control
  # character - "Executable path contains special characters", which is a
  # fatal unit error. A unit that installs cleanly and can never start is
  # worse than an install that says what is wrong, so the step refuses.
  _load_service_lib
  _service_systemd_can_express "/home/me/repo"
  assert_status 0 "$?"
  _service_systemd_can_express "/home/o\"d/repo"
  assert_status 1 "$?" "a double quote"
  _service_systemd_can_express "/home/o'd/repo"
  assert_status 1 "$?" "a single quote"
  _service_systemd_can_express "/home/o\\\\d/repo"
  assert_status 1 "$?" "a backslash"
  _service_systemd_can_express "/home/ok" "/home/o
d"
  assert_status 1 "$?" "a newline, in any of the paths given"
}

test_launchd_can_express_what_systemd_cannot() {
  # The Mac side has no such limit: XML says what it means about any of these,
  # and plistlib gives the path back unchanged. Pinned so that the refusal
  # above is understood as systemd's rule and not as a rule of this project.
  require_host_command python3
  stub_real python3
  local repo="$TEST_TMPDIR/o\"d 'and' \\ odder"
  mkdir -p "$repo"
  _load_service_lib "$repo"
  service_render_launchd ayeaye > "$TEST_TMPDIR/agent.plist"
  local out
  out="$(python3 - "$TEST_TMPDIR/agent.plist" <<'PY'
import plistlib, sys
with open(sys.argv[1], "rb") as fh:
    print(plistlib.load(fh)["ProgramArguments"][0])
PY
)"
  assert_status 0 "$?"
  assert_eq "$repo/bin/ayeaye" "$out"
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
  # The work itself stays on the step install.sh already registered, which is
  # what keeps the health check running after it rather than before it. That
  # is pinned end to end in install_systemd_test.sh; what is checked here is
  # that this file provides the function that step hands over to.
  command -v service_step >/dev/null
  assert_status 0 "$?"
}
