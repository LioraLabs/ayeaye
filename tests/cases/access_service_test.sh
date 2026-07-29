# The https front, written down as a service.
#
# It is written from the same description as every other service in this
# project rather than from a generator of its own, so there is one answer to
# "what does setup install" and a unit and a property list cannot say two
# different things about the same program.
#
# What must not be in either of them: an address, a port, a hostname or a key.
# All of that is in the file caddy is pointed at.

_load_access() {
  REPO="$REPO_ROOT"
  CONF_DIR="$XDG_CONFIG_HOME/ayeaye"
  ENV_FILE="$CONF_DIR/env"
  WIZARD_STATE_DIR="$XDG_STATE_HOME/ayeaye"
  WIZARD_STATE_FILE="$WIZARD_STATE_DIR/setup-state"
  WIZARD_CONSENT_LEDGER="$WIZARD_STATE_DIR/setup-consent.log"
  WIZARD_LOG_FILE="$WIZARD_STATE_DIR/setup.log"
  WIZARD_BACKUP_DIR="$WIZARD_STATE_DIR/backups"
  UNIT_DIR="$XDG_CONFIG_HOME/systemd/user"
  NO_SYSTEMD=0
  SERVICE_CADDY_BIN="$TEST_TMPDIR/bin/caddy"
  ACCESS_MDNS_NAME="my-box.local"
  PLATFORM_OS_RELEASE_FILES=""
  export PLATFORM_OS_RELEASE_FILES
  . "$REPO_ROOT/lib/wizard.sh"
  local stage
  for stage in welcome detect report configure plan install service finish; do
    wizard_stage "$stage" "$stage"
  done
  . "$REPO_ROOT/lib/steps/50-access.sh"
  . "$REPO_ROOT/lib/steps/70-service.sh"
  wizard_state_load
  WIZARD_INTERACTIVE=0
}

setup() {
  _load_access
}

# ------------------------------------------------------------------ systemd

test_the_unit_runs_caddy_against_the_generated_settings() {
  local unit
  unit="$(service_render_systemd ayeaye-caddy)"
  assert_contains "$unit" "ExecStart=$TEST_TMPDIR/bin/caddy run --config $XDG_CONFIG_HOME/ayeaye/Caddyfile --adapter caddyfile"
  assert_contains "$unit" "Restart=on-failure"
  assert_contains "$unit" "WantedBy=default.target"
}

test_the_unit_says_the_caddyfile_this_project_writes() {
  # One path, named in two files. A definition pointing somewhere setup does
  # not write would start caddy on whatever happened to be there.
  local unit
  unit="$(service_render_systemd ayeaye-caddy)"
  assert_contains "$unit" "$(_access_caddyfile)"
}

test_no_setting_of_any_kind_is_copied_into_the_unit() {
  local unit
  mkdir -p "$CONF_DIR"
  cat > "$ENV_FILE" <<'ENV'
AYEAYE_BIND=127.0.0.1
AYEAYE_PORT=54321
AYEAYE_ALLOWED_HOSTS=findable.example.org
AYEAYE_TOKEN=findable-secret-token
ENV
  unit="$(service_render_systemd ayeaye-caddy)"
  assert_not_contains "$unit" "54321" "the port lives in the settings file, not here"
  assert_not_contains "$unit" "findable.example.org"
  assert_not_contains "$unit" "findable-secret-token" \
    "a key in a unit file is a key in a world-readable file"
  assert_not_contains "$unit" "0.0.0.0"
}

test_a_machine_with_no_caddy_gets_no_definition() {
  # A unit naming nothing installs perfectly and fails at every login, which
  # is a worse answer than refusing to write one.
  SERVICE_CADDY_BIN=""
  stub_remove caddy 2>/dev/null || true
  service_render_systemd ayeaye-caddy >/dev/null 2>&1
  assert_status 2 "$?"
  service_render_launchd ayeaye-caddy >/dev/null 2>&1
  assert_status 2 "$?"
}

test_the_other_services_are_unaffected_by_the_new_one() {
  local unit
  unit="$(service_render_systemd ayeaye)"
  assert_contains "$unit" "ExecStart=$REPO_ROOT/bin/ayeaye"
  assert_not_contains "$unit" "--adapter caddyfile" \
    "adding a service must not change the ones that were already there"
  assert_not_contains "$unit" "$(_access_caddyfile)"
}

# ------------------------------------------------------------------ launchd

test_the_agent_parses_and_runs_the_same_program() {
  require_host_command python3
  stub_real python3
  local plist out
  plist="$TEST_TMPDIR/caddy.plist"
  service_render_launchd ayeaye-caddy > "$plist"
  out="$(python3 - "$plist" <<'PY'
import plistlib, sys
with open(sys.argv[1], "rb") as fh:
    doc = plistlib.load(fh)
print("label=%s" % doc["Label"])
print("argv=%s" % " ".join(doc["ProgramArguments"]))
print("keepalive=%s" % doc["KeepAlive"]["SuccessfulExit"])
PY
)"
  assert_contains "$out" "label=dev.ayeaye-caddy"
  assert_contains "$out" "argv=$TEST_TMPDIR/bin/caddy run --config $XDG_CONFIG_HOME/ayeaye/Caddyfile --adapter caddyfile"
  assert_contains "$out" "keepalive=False"
}

# ------------------------------------------------------------ the registry

test_the_front_end_is_one_of_the_services_this_project_knows() {
  assert_contains "$(service_names)" "ayeaye-caddy"
}

test_the_front_end_is_not_called_after_the_program_it_runs() {
  # Somebody who would choose an https front on their home network is exactly
  # the kind of person who already runs a caddy of their own, and the obvious
  # name for that is caddy.service. Taking that name would replace their
  # definition, and switching modes later would stop and delete it.
  assert_not_contains "$(service_names)" "
caddy
"
  service_render_systemd caddy >/dev/null 2>&1
  assert_status 2 "$?" "setup does not know how to write down a service called caddy"
}

test_regenerating_somebody_elses_definitions_leaves_this_one_alone() {
  # The migration loop rewrites definitions that are already on disk. It knows
  # nothing about whether this machine still has caddy, or whether the person
  # still wants their home network to reach ayeaye, so it must not touch this
  # one - the access step owns it.
  local source
  source="$(cat "$REPO_ROOT/lib/steps/70-service.sh")"
  assert_matches "$source" "ayeaye\\|whisper-server\\|ayeaye-caddy\\) continue"
}
