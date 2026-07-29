#!/usr/bin/env bash
# One-command onboarding, on a real machine, for real.
#
#   tests/smoke.sh --list          the six machines, and whether any has been done
#   tests/smoke.sh --plan          what this would do to THIS machine, without doing it
#   tests/smoke.sh --run           do it. Needs the confirmation below.
#
# ---------------------------------------------------------------- read this
#
# **This script changes the computer it is run on.** It installs packages, it
# writes ~/.config/ayeaye/env, it creates an auth token, it installs and starts
# a background service, and it does not undo any of it. It is not a unit test
# and it does not belong on a laptop you care about: run it on a fresh install,
# a virtual machine, or a machine you are willing to reinstall.
#
# That is why running it needs a confirmation nobody types by accident:
#
#   AYEAYE_SMOKE_CONFIRM=yes-change-this-computer tests/smoke.sh --run
#
# Without it, --run prints this paragraph and exits 2, having done nothing.
#
# ------------------------------------------------------- why this exists at all
#
# tests/run.sh proves the wizard's decisions against fixtures and stubs.
# tests/containers.sh proves the Linux package adapters against four real
# distributions. Neither can prove the thing this proves: that on a whole
# machine, with a real login session, a real service manager and a real network,
# `./install.sh` gets somebody from nothing to a working install.
#
# **None of the six has ever been run.** tests/smoke-hosts is the record and
# every line of it says no. tests/cases/smoke_hosts_test.sh makes the ordinary
# suite say so out loud - six named skips, one per machine - so that the gap is
# counted and printed rather than quietly absent. A skip is not a pass and this
# suite never pretends otherwise.
#
# ----------------------------------------------------------------- what it does
#
# Four passes, in order, on the machine it finds itself on:
#
#   1  a first install: ./install.sh --defaults --yes, then check that the plan
#      came before the installing, that the settings file, the token and the
#      service definition are really there, and that every claim the closing
#      checklist makes is true of this machine
#   2  the health check, against the service this run really started
#   3  a rerun: nothing installed again, the key kept, a setting the wizard has
#      never heard of kept byte for byte
#   4  a resumed run: what the first one finished is not done twice
#
# It prints one line per assertion and a summary, and exits non-zero if any of
# them failed. When it passes it prints the line to paste into tests/smoke-hosts.
#
# bash 3.2: no associative arrays, no ${var,,}, no mapfile.
set -u

TESTS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$TESTS_DIR/.." && pwd)"
HOSTS_FILE="$TESTS_DIR/smoke-hosts"

MODE=""
WANT_PROFILE=""

usage() { sed -n '2,52p' "$0" | sed 's/^# \{0,1\}//'; }

while [ "$#" -gt 0 ]; do
  case "$1" in
    -h|--help) usage; exit 0 ;;
    -l|--list) MODE=list ;;
    --plan)    MODE=plan ;;
    --run)     MODE=run ;;
    --profile)
      [ "$#" -gt 1 ] || { echo "--profile needs a name (see --list)" >&2; exit 2; }
      WANT_PROFILE="$2"; shift ;;
    -*)        echo "unknown option: $1 (try --help)" >&2; exit 2 ;;
    *)         echo "unexpected argument: $1 (try --help)" >&2; exit 2 ;;
  esac
  shift
done
[ -n "$MODE" ] || MODE=list

if [ ! -f "$HOSTS_FILE" ]; then
  echo "the record of which machines have been done is missing: $HOSTS_FILE" >&2
  exit 2
fi

# ------------------------------------------------------------------ the record

# hosts - the manifest, comments and blank lines dropped.
hosts() {
  while IFS= read -r line || [ -n "$line" ]; do
    case "$line" in
      ""|"#"*) continue ;;
    esac
    printf '%s\n' "$line"
  done < "$HOSTS_FILE"
}

host_field() {
  local id="$1" want="$2" line rest
  while IFS= read -r line; do
    case "$line" in
      "$id|"*) ;;
      *) continue ;;
    esac
    rest="${line#*|}"
    case "$want" in
      os)   printf '%s' "${rest%%|*}" ;;
      arch) rest="${rest#*|}"; printf '%s' "${rest%%|*}" ;;
      what) rest="${rest#*|}"; rest="${rest#*|}"; printf '%s' "${rest%%|*}" ;;
      verified) printf '%s' "${line##*|}" ;;
    esac
    return 0
  done <<EOF
$(hosts)
EOF
  return 1
}

if [ "$MODE" = list ]; then
  printf '%-14s %-8s %-8s %-9s %s\n' id os arch verified what
  while IFS='|' read -r id os arch what verified; do
    [ -n "$id" ] || continue
    printf '%-14s %-8s %-8s %-9s %s\n' "$id" "$os" "$arch" "$verified" "$what"
  done <<EOF
$(hosts)
EOF
  printf '\n'
  printf 'verified=no means the code has never been run on that kind of machine.\n'
  printf 'Nothing in tests/run.sh or tests/containers.sh covers it; both say so.\n'
  exit 0
fi

# --------------------------------------------------------- which machine is this

# shellcheck source=lib/platform.sh
. "$REPO_ROOT/lib/platform.sh"
platform_detect

# _profile_for - the manifest id this machine matches, or nothing.
#
# By what the platform layer says rather than by asking the machine again here:
# if the layer is wrong about this computer then everything downstream of it is
# wrong too, and a smoke test that worked that out for itself would hide the
# only bug worth finding.
_profile_for() {
  local id os arch what verified this_os this_arch
  this_os="$(platform_os)"
  this_arch="$(platform_arch)"
  while IFS='|' read -r id os arch what verified; do
    [ -n "$id" ] || continue
    [ "$os" = "$this_os" ] || continue
    [ "$arch" = any ] || [ "$arch" = "$this_arch" ] || continue
    case "$id" in
      ubuntu)      [ "$(platform_id)" = ubuntu ] || continue ;;
      fedora)      [ "$(platform_id)" = fedora ] || continue ;;
      arch)        [ "$(platform_id)" = arch ] || continue ;;
      opensuse)    case "$(platform_id)" in opensuse*) ;; *) continue ;; esac ;;
      macos-intel) [ "$this_arch" = x86_64 ] || continue ;;
      macos-arm)   [ "$this_arch" = arm64 ] || continue ;;
    esac
    printf '%s' "$id"
    return 0
  done <<EOF
$(hosts)
EOF
  return 1
}

PROFILE="$WANT_PROFILE"
if [ -z "$PROFILE" ]; then
  PROFILE="$(_profile_for || true)"
fi

if [ -z "$PROFILE" ]; then
  echo "this machine is not one of the six the milestone names:" >&2
  echo "  $(platform_summary)" >&2
  echo "run tests/smoke.sh --list to see them, or pass --profile <id> to say" >&2
  echo "which one you mean it to stand in for." >&2
  exit 2
fi

if [ "$MODE" = plan ]; then
  cat <<PLAN
this machine matches the "$PROFILE" profile:
  $(platform_summary)
  package manager : $(platform_pkg_manager)
  service manager : $(platform_service_manager)
  verified so far : $(host_field "$PROFILE" verified)

Running it would, on THIS computer:

  install tmux and python3 if they are missing, through $(platform_pkg_manager)
  write   ${XDG_CONFIG_HOME:-$HOME/.config}/ayeaye/env
  write   ${XDG_STATE_HOME:-$HOME/.local/state}/ayeaye/token
  install a background service and start it
  leave   all of it in place afterwards

Nothing above has happened. To do it:

  AYEAYE_SMOKE_CONFIRM=yes-change-this-computer tests/smoke.sh --run
PLAN
  exit 0
fi

# ------------------------------------------------------------------- the gate

if [ "${AYEAYE_SMOKE_CONFIRM:-}" != "yes-change-this-computer" ]; then
  cat >&2 <<'GATE'
this changes the computer it runs on, and does not undo any of it: it
installs packages, writes settings, creates a key, and installs and starts a
background service. It is meant for a fresh install or a virtual machine.

Nothing has been done. If you meant it:

  AYEAYE_SMOKE_CONFIRM=yes-change-this-computer tests/smoke.sh --run
GATE
  exit 2
fi

# ------------------------------------------------------------------ assertions

PASSED=0
FAILED=0
FAILURES=""

ok()   { printf '  ok     %s\n' "$*"; PASSED=$((PASSED + 1)); }
bad()  { printf '  FAIL   %s\n' "$*"; FAILED=$((FAILED + 1)); FAILURES="$FAILURES
  $*"; }
note() { printf '  --     %s\n' "$*"; }

want_status() {
  if [ "$1" = "$2" ]; then ok "$3"; else bad "$3 (status $1, wanted $2)"; fi
}
want_file() {
  if [ -s "$1" ]; then ok "$2"; else bad "$2 ($1 is missing or empty)"; fi
}
want_in() {
  case "$(cat "$1" 2>/dev/null)" in
    *"$2"*) ok "$3" ;;
    *)      bad "$3 (not in the output: $2)" ;;
  esac
}
want_not_in() {
  case "$(cat "$1" 2>/dev/null)" in
    *"$2"*) bad "$3 (the output said: $2)" ;;
    *)      ok "$3" ;;
  esac
}

ENV_FILE="${XDG_CONFIG_HOME:-$HOME/.config}/ayeaye/env"
STATE_DIR="${XDG_STATE_HOME:-$HOME/.local/state}/ayeaye"
TOKEN_FILE="$STATE_DIR/token"
STATE_FILE="$STATE_DIR/setup-state"
OUT_DIR="${TMPDIR:-/tmp}/ayeaye-smoke.$$"
mkdir -p "$OUT_DIR"

printf 'ayeaye smoke test: %s\n' "$PROFILE"
printf '  %s\n\n' "$(platform_summary)"

# ------------------------------------------------------------- 1. a first run
#
# Answers fed down a pipe rather than --defaults, and the difference is the
# whole reason this pass is worth running on a real machine. --defaults never
# reads an answer, and "enable and start it now?" is an answer - so an
# unattended run never starts the service, the health check finds nothing
# running, and the eight checks this ticket exists for are never executed
# anywhere. Empty lines take every offered default, which on a real host means
# the local port, no https front (--yes cannot grant exposure, and nothing here
# would want it to) and yes to starting the service.
printf 'first install\n'
( cd "$REPO_ROOT" && bash ./install.sh --yes <<'ANSWERS'








ANSWERS
) >"$OUT_DIR/first" 2>&1
want_status "$?" 0 "the run finished"
want_file "$ENV_FILE" "the settings file was written"
want_file "$TOKEN_FILE" "the key was made"
want_in "$OUT_DIR/first" "bookmark:" "the closing screen gave an address to open"
want_in "$OUT_DIR/first" "signing in, once per device:" "and said how to sign in"

# The plan came first. This is the promise the whole conversation is built on
# and the one that is easiest to break by moving one line.
plan_at="$(grep -n 'what is about to happen' "$OUT_DIR/first" | head -1 | cut -d: -f1)"
work_at="$(grep -n 'setting it up' "$OUT_DIR/first" | head -1 | cut -d: -f1)"
if [ -n "$plan_at" ] && [ -n "$work_at" ] && [ "$plan_at" -lt "$work_at" ]; then
  ok "everything was listed before any of it happened"
else
  bad "the plan did not come before the work"
fi

# Every claim the closing screen made, against this machine.
case "$(platform_service_manager)" in
  systemd)
    want_file "${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user/ayeaye.service" \
      "a systemd user unit is really on disk"
    want_in "$OUT_DIR/first" "systemctl --user disable --now" \
      "and removal is described in systemd's own words"
    want_not_in "$OUT_DIR/first" "launchctl" "with no mention of launchd"
    ;;
  launchd)
    want_file "$HOME/Library/LaunchAgents/dev.ayeaye.plist" \
      "a launch agent is really on disk"
    want_in "$OUT_DIR/first" "launchctl bootout" \
      "and removal is described in launchd's own words"
    want_not_in "$OUT_DIR/first" "systemctl" "with no mention of systemd"
    want_not_in "$OUT_DIR/first" "journalctl" "and no mention of a journal"
    ;;
  *)
    want_in "$OUT_DIR/first" "stop the ayeaye you started by hand" \
      "a machine that starts nothing for you is told so"
    ;;
esac

for want in tmux python3; do
  if command -v "$want" >/dev/null 2>&1; then
    ok "$want is on this machine"
  else
    bad "$want is still not on this machine, and the run said it finished"
  fi
done

# ------------------------------------------------------------- 2. health
printf '\nthe health check\n'
if grep -q 'ayeaye is not running yet' "$OUT_DIR/first"; then
  # Not a note any more: this pass answers the start question, so nothing
  # having been started means something went wrong with the service, and the
  # eight checks below it did not happen.
  bad "nothing was started, so none of the health checks ran"
else
  want_in "$OUT_DIR/first" "Checking what you asked for." "the health check ran"
  want_in "$OUT_DIR/first" "ok       ayeaye answers on http://" \
    "ayeaye really answered on this machine"
  want_in "$OUT_DIR/first" "ok       ayeaye refuses anyone without your key" \
    "and really refused a request that carried no key"
  want_not_in "$OUT_DIR/first" "STOP." "with the lock on"
  # The four verdicts, and the one that matters most: a capability nobody
  # asked for must be reported as such and not as working.
  want_in "$OUT_DIR/first" "skipped  " "a capability nobody chose is marked skipped"
  want_not_in "$OUT_DIR/first" "ok       an https address your phone can open" \
    "a way in that was never set up is never reported as working"
fi

# ------------------------------------------------------------- 3. a rerun
printf '\na second run\n'
printf 'SMOKE_OWN_SETTING=keep-me\n' >> "$ENV_FILE"
before="$(cksum < "$ENV_FILE")"
token_before="$(cat "$TOKEN_FILE")"
( cd "$REPO_ROOT" && bash ./install.sh --defaults --yes ) >"$OUT_DIR/second" 2>&1
want_status "$?" 0 "the second run finished"
want_in "$OUT_DIR/second" "everything setup needs is already on this computer" \
  "and installed nothing a second time"
if [ "$(cat "$TOKEN_FILE")" = "$token_before" ]; then
  ok "the key is the one the phone already has"
else
  bad "the key was replaced, so a bookmark on a phone has stopped working"
fi
if [ "$(cksum < "$ENV_FILE")" = "$before" ]; then
  ok "a setting the wizard never asked about survived byte for byte"
else
  bad "the settings file changed under a rerun"
fi

# ------------------------------------------------------------- 4. picking up
printf '\na run that is picked up\n'
if [ -f "$STATE_FILE" ]; then
  sed 's/^run\.completed=.*/run.completed=0/' "$STATE_FILE" > "$STATE_FILE.smoke" \
    && mv "$STATE_FILE.smoke" "$STATE_FILE"
  ( cd "$REPO_ROOT" && bash ./install.sh --defaults ) >"$OUT_DIR/third" 2>&1
  want_status "$?" 0 "the resumed run finished"
  want_in "$OUT_DIR/third" "The last run stopped before it finished." \
    "and knew it was picking one up"
  want_in "$OUT_DIR/third" "(already done)" "and did not redo what was done"
else
  bad "no state file, so nothing can be resumed"
fi

# ------------------------------------------------------------------- the report
printf '\n%s passed, %s failed\n' "$PASSED" "$FAILED"
printf 'the whole output is under %s\n' "$OUT_DIR"
if [ "$FAILED" -gt 0 ]; then
  printf 'failed:%s\n' "$FAILURES"
  exit 1
fi

cat <<DONE

This machine passed. That is a fact about one machine on one day, and the
record is kept by hand: edit $HOSTS_FILE and change the
"$PROFILE" line's last column from no to today's date and the version you ran,
for example:

  $PROFILE|$(host_field "$PROFILE" os)|$(host_field "$PROFILE" arch)|$(host_field "$PROFILE" what)|$(date +%Y-%m-%d) $(cd "$REPO_ROOT" && git rev-parse --short HEAD 2>/dev/null || printf unknown)

Nothing here writes that line for you. A verification is somebody saying they
watched it, and a script that recorded its own success would be worth nothing.
DONE
exit 0
