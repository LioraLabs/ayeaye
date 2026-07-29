# An https address the person already has: what ayeaye is told to accept, and
# what their proxy has to be told.
#
# The two snippets are generated for somebody else's program to read, so what
# they contain is asserted line by line rather than eyeballed. The one that
# matters most is the header that keeps the address check working: a proxy
# that rewrites the Host header turns a 403 into a puzzle nobody can solve
# from either end.

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
  wizard_consent_reset
  wizard_plan_reset
  WIZARD_INTERACTIVE=0
  WIZARD_AUTO_CONSENT=""
}

_settings() {
  mkdir -p "$CONF_DIR"
  cat > "$ENV_FILE" <<'ENV'
AYEAYE_BIND=127.0.0.1
AYEAYE_PORT=8911
AYEAYE_ALLOWED_HOSTS=
ENV
}

setup() {
  require_host_command python3
  stub_real python3
  _load_access
  _settings
}

# --------------------------------------------------------------- the caddy one

test_the_caddy_snippet_forwards_to_this_computer() {
  local text
  text="$(access_render_proxy_caddy ayeaye.example.com 8911)"
  assert_contains "$text" "reverse_proxy 127.0.0.1:8911"
  assert_contains "$text" "ayeaye.example.com"
  assert_not_contains "$text" "0.0.0.0"
}

test_the_caddy_snippet_is_a_line_and_not_a_second_site_block() {
  # Mode 4 exists because caddy already answers https for that address, so
  # the person already has a block for it. A second one is something caddy
  # reads as a mistake: it refuses the whole file, and the site they already
  # had goes down with it.
  local text last
  text="$(access_render_proxy_caddy ayeaye.example.com 8911)"
  last="$(printf '%s\n' "$text" | grep -v '^#' | grep -v '^$' | tail -1)"
  assert_eq "reverse_proxy 127.0.0.1:8911" "$last" \
    "everything but the one line is a comment"
  assert_not_contains "$text" "
ayeaye.example.com {" "a block of its own would take their site down"
}

# --------------------------------------------------------------- the nginx one

test_the_nginx_snippet_forwards_to_this_computer() {
  local text
  text="$(access_render_proxy_nginx ayeaye.example.com 8911)"
  assert_contains "$text" "proxy_pass http://127.0.0.1:8911;"
  assert_not_contains "$text" "0.0.0.0"
}

test_the_nginx_snippet_keeps_the_address_check_working() {
  # ayeaye answers 403 to a Host it was not told about, which is what stops
  # another site pointing a browser at this one. A proxy handing it a fixed
  # Host would defeat that quietly.
  local text
  text="$(access_render_proxy_nginx ayeaye.example.com 8911)"
  # $http_host, not $host: nginx strips the port off $host, so an https
  # address on any port but 443 would be handed to ayeaye without it and
  # refused by the very check this line exists to satisfy.
  assert_contains "$text" 'proxy_set_header Host $http_host;'
  assert_not_contains "$text" 'proxy_set_header Host $host;'
  assert_not_contains "$text" 'proxy_set_header Host 127.0.0.1'
  assert_contains "$text" 'proxy_set_header X-Forwarded-Proto $scheme;'
}

test_the_nginx_snippet_lets_the_stream_through() {
  # ayeaye sends what the agents are doing as it happens. nginx buffers that
  # by default and the page just sits there, with nothing in any log to say
  # why.
  local text
  text="$(access_render_proxy_nginx ayeaye.example.com 8911)"
  assert_contains "$text" "proxy_buffering off;"
  assert_contains "$text" "proxy_read_timeout 3600s;"
}

test_neither_renderer_invents_a_hostname() {
  access_render_proxy_caddy "" 8911 >/dev/null 2>&1
  assert_status 2 "$?"
  access_render_proxy_nginx "" 8911 >/dev/null 2>&1
  assert_status 2 "$?"
}

# ------------------------------------------------------------- setting it up

test_the_address_is_written_into_the_settings_and_both_snippets_are_left_ready() {
  wizard_remember answer.hosts "ayeaye.example.com"
  local out
  out="$(_access_setup_proxy)"
  assert_file_contains "$ENV_FILE" "AYEAYE_ALLOWED_HOSTS=ayeaye.example.com"
  assert_file_contains "$ENV_FILE" "AYEAYE_BIND=127.0.0.1"
  assert_file_exists "$CONF_DIR/reverse-proxy.caddy"
  assert_file_exists "$CONF_DIR/reverse-proxy.nginx"
  assert_file_contains "$CONF_DIR/reverse-proxy.caddy" "reverse_proxy 127.0.0.1:8911"
  assert_file_contains "$CONF_DIR/reverse-proxy.nginx" "proxy_pass http://127.0.0.1:8911;"
  assert_contains "$out" "$CONF_DIR/reverse-proxy.nginx"
}

test_a_proxy_that_does_not_answer_yet_is_unfinished_rather_than_done() {
  # Setup has just handed the person the configuration their proxy needs. It
  # cannot also have been added to it already, so "done" would be a lie on
  # every first run.
  wizard_remember answer.hosts "ayeaye.example.com"
  stub_command curl --exit 7 --stderr "could not connect"
  local out
  out="$(_access_setup_proxy 2>/dev/null)"
  assert_status 10 "$?"
  assert_contains "$out" "Add it, reload it"
}

test_a_proxy_that_answers_as_something_else_is_not_taken_for_ayeaye() {
  wizard_remember answer.hosts "ayeaye.example.com"
  stub_command curl --stdout "404"
  local out
  out="$(_access_setup_proxy)"
  assert_status 10 "$?"
  assert_contains "$out" "not as ayeaye"
}

test_a_proxy_that_answers_is_done() {
  wizard_remember answer.hosts "ayeaye.example.com"
  stub_command curl --stdout "401"
  local out
  out="$(_access_setup_proxy)"
  assert_status 0 "$?"
  assert_contains "$out" "https://ayeaye.example.com/ answers"
}

test_the_first_of_several_addresses_is_the_one_the_snippets_are_written_for() {
  wizard_remember answer.hosts "one.example.com,two.example.com"
  stub_command curl --stdout "401"
  _access_setup_proxy >/dev/null
  assert_file_contains "$ENV_FILE" "AYEAYE_ALLOWED_HOSTS=one.example.com,two.example.com" \
    "both are allowed through"
  assert_file_contains "$CONF_DIR/reverse-proxy.caddy" "one.example.com"
}

test_without_an_address_nothing_is_changed() {
  local out
  out="$(_access_setup_proxy)"
  assert_status 10 "$?"
  assert_file_contains "$ENV_FILE" "AYEAYE_ALLOWED_HOSTS="
  assert_file_missing "$CONF_DIR/reverse-proxy.caddy"
}
