# Does what was set up actually work?
#
# The last thing setup does before it tells somebody they are finished. It
# checks the capabilities that were *chosen*, one at a time, and says which of
# four things happened to each:
#
#   ok        it was checked and it works
#   FAILED    it was checked and it does not
#   skipped   it is not part of what you asked for, so there was nothing to check
#   unknown   it is part of what you asked for and setup could not tell
#
# The four are four marks and not three, and never collapsed into two. A check
# that was skipped because nobody asked for it and a check that passed are
# different facts, and a closing screen that renders them the same way is the
# failure this whole stage exists to prevent - it is how somebody ends up
# believing their phone can reach their agents when nothing was ever set up.
# "unknown" is the fourth because "setup has no curl" is not a passing grade
# either, and calling it one would be the worst answer available.
#
# One of the checks is a positive security assertion rather than a liveness
# one: an API request carrying no key must be *refused*. A 200 there means the
# lock is off, and that is the one verdict here that fails the step outright
# rather than leaving it unfinished.
#
# It runs last in the service stage, after whatever installed and started
# things, which is what the 80 in the filename is for.
#
# Note what it does *not* do: read the shell variables the earlier stages set.
# A resumed run skips those steps, so those variables would hold their defaults
# and this check would quietly decide there was nothing to check. Everything
# one stage tells another goes through the state file and the settings file.
#
# bash 3.2: no associative arrays, no ${var,,}, no mapfile, no local -n.

# How long any one request is given. Short: everything here is on this
# computer or on this computer's own network, and a health check that hangs
# for a minute per capability is one nobody will ever wait for.
HEALTH_TIMEOUT="${HEALTH_TIMEOUT:-5}"

_health_conf_dir()  { printf '%s' "${CONF_DIR:-${XDG_CONFIG_HOME:-$HOME/.config}/ayeaye}"; }
_health_state_dir() { printf '%s' "${WIZARD_STATE_DIR:-${XDG_STATE_HOME:-$HOME/.local/state}/ayeaye}"; }
_health_env_file()  { printf '%s' "${ENV_FILE:-$(_health_conf_dir)/env}"; }
_health_token_file() { printf '%s' "${TOKEN_FILE:-$(_health_state_dir)/token}"; }

# The one file this step writes, and it holds the key. Named here so that the
# tripwire in tests/run.sh can watch it: it is written, used and removed inside
# a single check, and a run that leaves it behind is a run that leaked a
# credential into a file somebody else may be able to read.
_health_request_file() { printf '%s' "$(_health_state_dir)/health-request"; }

_health_have() { command -v "${1:-}" >/dev/null 2>&1; }

# ------------------------------------------------------------------ verdicts
#
# Counted as they are printed, so the step's own status is a consequence of
# what it found rather than a second opinion about it.

_HEALTH_PASSED=0
_HEALTH_FAILED=0
_HEALTH_SKIPPED=0
_HEALTH_UNKNOWN=0
_HEALTH_INSECURE=0

# _health_report <check> <verdict> <text> - one line, one mark, one memory.
#
# The verdict is written to the state file as well as to the screen, because
# the closing checklist in install.sh has to be able to say what worked without
# re-running any of this, and because a resumed run must not be able to report
# last week's answer as this week's.
_health_report() {
  local check="${1:-}" verdict="${2:-}" text="${3:-}"
  case "$verdict" in
    pass)
      _HEALTH_PASSED=$((_HEALTH_PASSED + 1))
      wizard_item ok "$text"
      ;;
    fail)
      _HEALTH_FAILED=$((_HEALTH_FAILED + 1))
      wizard_item FAILED "$text"
      ;;
    skip)
      _HEALTH_SKIPPED=$((_HEALTH_SKIPPED + 1))
      wizard_item skipped "$text - you did not ask for this"
      ;;
    *)
      verdict=unknown
      _HEALTH_UNKNOWN=$((_HEALTH_UNKNOWN + 1))
      wizard_item unknown "$text - setup could not check this"
      ;;
  esac
  wizard_remember "step.service.health.$check" "$verdict"
  return 0
}

# ------------------------------------------------------------------- asking
#
# Every request below is made without the key unless it says otherwise, and the
# ones that do need it get it from a file rather than from a command line. A
# token in an argument is a token in the process list, in this shell's history
# and in the log this step writes - and the log is exactly where somebody
# debugging a failed check will paste from.

# _health_code <url> [curl argument]… - the http status, or nothing when the
# request could not be made at all.
_health_code() {
  local url="${1:-}" out
  shift
  _health_have curl || return 1
  wizard_detail "checking: $url"
  out="$(curl -sS -o /dev/null -w '%{http_code}' --max-time "$HEALTH_TIMEOUT" \
    "$@" "$url" 2>&1 </dev/null)" || out=""
  [ -n "$out" ] || return 1
  case "$out" in
    *[!0-9]*) wizard_detail "$out"; return 1 ;;
  esac
  printf '%s' "$out"
  return 0
}

# _health_authed_body <url> - the body of a request that carried the key.
#
# The key goes into a curl configuration file created with a private umask and
# deleted the moment the request is done. Status 1 when there is no key, no
# curl, or no answer.
_health_authed_body() {
  local url="${1:-}" token file out status
  _health_have curl || return 1
  token="$(cat "$(_health_token_file)" 2>/dev/null || true)"
  [ -n "$token" ] || return 1
  file="$(_health_request_file)"
  mkdir -p "$(_health_state_dir)" 2>/dev/null || return 1
  rm -f "$file" 2>/dev/null || true
  ( umask 077; printf 'header = "X-Voice-Token: %s"\n' "$token" > "$file" ) \
    2>/dev/null || return 1
  wizard_detail "checking (with your key): $url"
  out="$(curl -sS --max-time "$HEALTH_TIMEOUT" -K "$file" "$url" 2>&1 </dev/null)"
  status=$?
  rm -f "$file" 2>/dev/null || true
  [ "$status" = 0 ] || { wizard_detail "$out"; return 1; }
  printf '%s' "$out"
  return 0
}

# _health_is_answer <status> - did something on the other end answer as an
# http server at all? A refusal counts: without the key ayeaye is supposed to
# refuse, and a refusal proves the address reaches it.
_health_is_answer() {
  case "${1:-}" in
    2*|3*|401|403) return 0 ;;
    *) return 1 ;;
  esac
}

# --------------------------------------------------------------- what was chosen

_health_mode()  { wizard_state_get answer.access.mode ""; }
_health_bind()  { wizard_env_get "$(_health_env_file)" AYEAYE_BIND 127.0.0.1; }
_health_port()  { wizard_env_get "$(_health_env_file)" AYEAYE_PORT 8911; }
_health_hosts() { wizard_env_get "$(_health_env_file)" AYEAYE_ALLOWED_HOSTS ""; }
_health_url()   { printf 'http://%s:%s' "$(_health_bind)" "$(_health_port)"; }

# ------------------------------------------------------------------ the checks

# 1. Is there a service, and is it up? Skipped rather than failed on a machine
# that has no user service manager, or a run that was told not to install one:
# ayeaye started by hand is a supported way to use it.
_health_check_service() {
  local kind cmd
  if [ "${NO_SYSTEMD:-0}" = 1 ]; then
    _health_report service skip "ayeaye starting when you log in"
    return 0
  fi
  kind="$(platform_service_manager)"
  case "$kind" in
    systemd|launchd) ;;
    *)
      _health_report service skip "ayeaye starting when you log in"
      return 0
      ;;
  esac
  if ! cmd="$(platform_service_command ayeaye status)"; then
    _health_report service unknown "ayeaye starting when you log in"
    return 0
  fi
  wizard_detail "running: $cmd"
  if eval "$cmd" >/dev/null 2>&1; then
    _health_report service pass "ayeaye starts when you log in, and is running now"
  else
    _health_report service fail "ayeaye starts when you log in, and is running now"
  fi
  return 0
}

# 2. Does the program answer on this computer at all?
_health_check_local() {
  local code
  if ! code="$(_health_code "$(_health_url)/")"; then
    _health_report local unknown "ayeaye answering on $(_health_url)"
    return 0
  fi
  case "$code" in
    2*|3*) _health_report local pass "ayeaye answers on $(_health_url)" ;;
    *)
      wizard_detail "health: $(_health_url)/ answered $code"
      _health_report local fail "ayeaye answering on $(_health_url)"
      ;;
  esac
  return 0
}

# 3. Is the lock on?
#
# The one check here that asserts something rather than measuring it: a
# request with no key at all must be refused. 200 means anybody who can reach
# this address can drive the coding agents on this computer, which is the worst
# thing this project can do to somebody, and it is worth saying at length.
_health_check_unauthenticated() {
  local code
  if ! code="$(_health_code "$(_health_url)/api/overview")"; then
    _health_report auth unknown "ayeaye refusing anyone without your key"
    return 0
  fi
  case "$code" in
    401)
      _health_report auth pass "ayeaye refuses anyone without your key"
      ;;
    403)
      # Refused, but for the other reason. The lock may well be on; this
      # request never got far enough to find out.
      wizard_detail "health: unauthenticated /api/overview answered 403"
      _health_report auth unknown "ayeaye refusing anyone without your key"
      ;;
    2*)
      _HEALTH_INSECURE=1
      wizard_detail "health: unauthenticated /api/overview answered $code"
      _health_report auth fail "ayeaye refusing anyone without your key"
      wizard_blank
      wizard_say "STOP. Something answered a request that carried no key, and"
      wizard_say "answered it in full. Whatever is on $(_health_url) can be"
      wizard_say "driven by anybody who can reach that address, and driving"
      wizard_say "ayeaye means running commands on this computer."
      wizard_say "Either something else is listening on that port, or the key"
      wizard_say "has been switched off. Do not open this from your phone until"
      wizard_say "you know which."
      wizard_blank
      ;;
    *)
      wizard_detail "health: unauthenticated /api/overview answered $code"
      _health_report auth fail "ayeaye refusing anyone without your key"
      ;;
  esac
  return 0
}

# 4. The agents. Only the ones that were chosen; the others are not missing,
# they were never asked for.
_health_check_agents() {
  local name key chosen found=0 wanted=0 missing=""
  for name in claude codex; do
    key="answer.agent.$name"
    chosen="$(wizard_state_get "$key" 0)"
    [ "$chosen" = 1 ] || continue
    wanted=$((wanted + 1))
    if _health_have "$name"; then
      found=$((found + 1))
    else
      missing="$missing $name"
    fi
  done
  if [ "$wanted" = 0 ]; then
    _health_report agents skip "a coding agent for ayeaye to show you"
    return 0
  fi
  if [ "$found" = "$wanted" ]; then
    _health_report agents pass "the coding agents you chose are installed"
    return 0
  fi
  wizard_detail "health: not on PATH:$missing"
  _health_report agents fail "the coding agents you chose are installed"
  return 0
}

# 5. Host validation, asserted in both directions.
#
# One request per configured address, claiming to be that address, which must
# not be refused - and one claiming an address nobody configured, which must
# be. Only the second half proves anything: a server that accepts everything
# passes the first half perfectly.
_health_check_hosts() {
  local hosts host rest code bad="" ok=1
  hosts="$(_health_hosts)"
  if [ -z "$hosts" ]; then
    _health_report hosts skip "ayeaye accepting the https address you named"
    return 0
  fi
  rest="$hosts"
  while [ -n "$rest" ]; do
    host="${rest%%,*}"
    case "$rest" in
      *,*) rest="${rest#*,}" ;;
      *)   rest="" ;;
    esac
    [ -n "$host" ] || continue
    if ! code="$(_health_code "$(_health_url)/" -H "Host: $host")"; then
      _health_report hosts unknown "ayeaye accepting the https address you named"
      return 0
    fi
    if [ "$code" = 403 ]; then
      bad="$bad $host"
      ok=0
    fi
  done
  if ! code="$(_health_code "$(_health_url)/" -H "Host: not-configured.invalid")"; then
    _health_report hosts unknown "ayeaye accepting the https address you named"
    return 0
  fi
  if [ "$code" != 403 ]; then
    wizard_detail "health: an address nobody configured answered $code"
    ok=0
  fi
  if [ "$ok" = 1 ]; then
    _health_report hosts pass "ayeaye accepts $hosts and refuses anything else"
    return 0
  fi
  [ -n "$bad" ] && wizard_detail "health: refused its own configured address:$bad"
  _health_report hosts fail "ayeaye accepting the https address you named"
  return 0
}

# 6. The https front end, whichever one was set up. The mode that keeps ayeaye
# on this computer has no front end by design, and is reported as not asked
# for rather than as working.
_health_check_https() {
  local mode host code ca
  mode="$(_health_mode)"
  case "$mode" in
    tailscale|lan|proxy) ;;
    *)
      _health_report https skip "an https address your phone can open"
      return 0
      ;;
  esac
  host="$(_health_hosts)"
  host="${host%%,*}"
  if [ -z "$host" ]; then
    _health_report https unknown "an https address your phone can open"
    return 0
  fi
  if [ "$mode" = lan ] && command -v _access_ca_file >/dev/null 2>&1; then
    ca="$(_access_ca_file)"
    if [ -s "$ca" ]; then
      code="$(_health_code "https://$host/" --cacert "$ca")" || code=""
    else
      code="$(_health_code "https://$host/")" || code=""
    fi
  else
    code="$(_health_code "https://$host/")" || code=""
  fi
  if [ -z "$code" ]; then
    _health_report https unknown "https://$host/ answering for your phone"
    return 0
  fi
  if _health_is_answer "$code"; then
    _health_report https pass "https://$host/ answers, which is what your phone opens"
  else
    wizard_detail "health: https://$host/ answered $code"
    _health_report https fail "https://$host/ answering for your phone"
  fi
  return 0
}

# 7. Talking out loud, when a listening model was chosen. The app is the thing
# asked, not the file system: bin/ayeaye decides whether the talk button is
# live by probing for itself, and its answer is the only one that matters.
_health_check_voice() {
  local body
  if [ -z "$(wizard_state_get answer.voice.model "")" ]; then
    _health_report voice skip "talking to your agents out loud"
    return 0
  fi
  if ! body="$(_health_authed_body "$(_health_url)/api/overview")"; then
    _health_report voice unknown "talking to your agents out loud"
    return 0
  fi
  case "$body" in
    *'"voice": true'*|*'"voice":true'*)
      _health_report voice pass "the talk button is live"
      ;;
    *'"voice": false'*|*'"voice":false'*)
      wizard_detail "health: the app reports voice as not configured"
      _health_report voice fail "the talk button is live"
      ;;
    *)
      wizard_detail "health: could not read the app's answer: $body"
      _health_report voice unknown "talking to your agents out loud"
      ;;
  esac
  return 0
}

# 8. The board, when cliban was chosen. Two things have to be true and they
# fail differently: the program has to be here, and the app has to be able to
# get an answer out of it.
_health_check_board() {
  local body
  if [ "$(wizard_state_get answer.board 0)" != 1 ]; then
    _health_report board skip "your ticket board on the phone"
    return 0
  fi
  if ! _health_have cliban; then
    wizard_detail "health: cliban is not on PATH"
    _health_report board fail "your ticket board on the phone"
    return 0
  fi
  if ! body="$(_health_authed_body "$(_health_url)/api/cliban/projects")"; then
    _health_report board unknown "your ticket board on the phone"
    return 0
  fi
  case "$body" in
    *'"keys"'*) _health_report board pass "your ticket board answers on the phone" ;;
    *)
      wizard_detail "health: /api/cliban/projects said: $body"
      _health_report board fail "your ticket board on the phone"
      ;;
  esac
  return 0
}

# ---------------------------------------------------------------- the step

_health_check_step() {
  local started

  # Whatever an earlier run recorded is not this run's answer.
  wizard_state_unset_prefix "step.service.health." || true
  rm -f "$(_health_request_file)" 2>/dev/null || true
  _HEALTH_PASSED=0
  _HEALTH_FAILED=0
  _HEALTH_SKIPPED=0
  _HEALTH_UNKNOWN=0
  _HEALTH_INSECURE=0

  started="$(wizard_state_get step.service.unit.started 0)"
  if [ "$started" != 1 ]; then
    wizard_say "ayeaye is not running yet, so there is nothing to check."
    wizard_say "start it, then open the address below on your phone."
    return "$WIZARD_STAGE_SKIP"
  fi

  if ! _health_have curl; then
    # Said once, at the top, rather than eight times underneath. Everything
    # that needs the network will report "unknown", which is the truth.
    wizard_say "this computer has no curl, so most of these could not be checked."
  fi

  wizard_say "Checking what you asked for. Each line is one of:"
  wizard_say "  ok       it works             FAILED   it does not"
  wizard_say "  skipped  you did not ask      unknown  setup could not tell"
  wizard_blank

  _health_check_service
  _health_check_local
  _health_check_unauthenticated
  _health_check_agents
  _health_check_hosts
  _health_check_https
  _health_check_voice
  _health_check_board

  wizard_detail "health: $_HEALTH_PASSED ok, $_HEALTH_FAILED failed, \
$_HEALTH_SKIPPED skipped, $_HEALTH_UNKNOWN unknown"

  # A key left behind in a file is worth being sure about twice.
  rm -f "$(_health_request_file)" 2>/dev/null || true

  if [ "$_HEALTH_INSECURE" = 1 ]; then
    return "$WIZARD_STAGE_FAIL"
  fi
  if [ "$_HEALTH_FAILED" -gt 0 ] || [ "$_HEALTH_UNKNOWN" -gt 0 ]; then
    return "$WIZARD_STAGE_PENDING"
  fi
  return "$WIZARD_STAGE_OK"
}

# required, and not optional, and the distinction is one status code wide.
#
# Everything this step can be unhappy about answers PENDING - a capability that
# did not work, a check that could not be made - and a pending step never stops
# a run whatever its policy is. The single path that answers FAIL is the lock
# being off, and that is not a thing to offer somebody the choice of leaving
# out. `optional` would print "try again, leave it out, or stop here?" over a
# paragraph saying anybody who can reach this address can run commands on this
# computer.
wizard_step service health _health_check_step "Checking that what you chose works"
