#!/usr/bin/env bash
# Setup for ayeaye, as an eight-stage conversation.
#
# The stages are declared at the bottom of this file and the work inside them
# is above it. Nothing here detects a platform, names a package manager or
# knows what a service unit looks like: that is lib/platform.sh. Nothing here
# asks for permission in its own words either: that is lib/consent.sh. What
# this file owns is the lifecycle - the order of the conversation, what is
# remembered between runs, and what happens when a step does not work.
#
# A ticket adding work to setup does not edit this file. It drops a file into
# lib/steps/, which registers a step onto a stage that already exists; see
# lib/steps/README.md.
#
# bash 3.2: no associative arrays, no ${var,,}, no mapfile, no local -n.
set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

CONF_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/ayeaye"
ENV_FILE="$CONF_DIR/env"
WIZARD_STATE_DIR="${XDG_STATE_HOME:-$HOME/.local/state}/ayeaye"
TOKEN_FILE="$WIZARD_STATE_DIR/token"
UNIT_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"

DEFAULTS=0
NO_SYSTEMD=0
FRESH=0
WIZARD_INTERACTIVE=1
WIZARD_DETAILS=0
WIZARD_AUTO_CONSENT=""

usage() {
  cat <<'USAGE'
One-command setup for ayeaye. Safe to re-run: it keeps the settings and the
key you already have, asks before changing anything, and picks up where an
interrupted run stopped.

  ./install.sh               interactive, sane defaults
  ./install.sh --defaults    accept every default, no prompts
  ./install.sh --no-systemd  skip the unit; prints the manual run command
  ./install.sh --yes         say yes to installing and to replacing settings,
                             but never to a network or a certificate change
  ./install.sh --details     show the raw commands as they run
  ./install.sh --fresh       forget what earlier runs recorded and start over

What it does, in eight steps:

  1. explains what setup may change, and what it will never change
  2. works out what this computer is and what it already has
  3. says in plain words what ayeaye can do here
  4. asks how you want to reach it and what to switch on
  5. lists everything it is about to install or change, and asks
  6. installs and configures what you chose
  7. starts it in the background and checks that it answers
  8. prints the address to open on your phone, and anything left to do

Nothing is installed, downloaded, opened to the network or trusted without a
question first, and answering no to any of them leaves this computer exactly
as it was.
USAGE
}

for arg in "$@"; do
  case "$arg" in
    --defaults)   DEFAULTS=1; WIZARD_INTERACTIVE=0 ;;
    --no-systemd) NO_SYSTEMD=1 ;;
    --yes)        WIZARD_AUTO_CONSENT="privileged download replace" ;;
    --details)    WIZARD_DETAILS=1 ;;
    --fresh)      FRESH=1 ;;
    -h|--help)    usage; exit 0 ;;
    *) echo "unknown option: $arg (try --help)" >&2; exit 2 ;;
  esac
done

# shellcheck source=lib/wizard.sh
. "$REPO/lib/wizard.sh"

# What the conversation decided. These are working copies: the decisions
# themselves live in the state file, because a resumed run skips the step that
# made them and a variable set by a step that did not run this time holds
# nothing but its default. Anything one stage tells a later stage goes through
# wizard_remember, and is read back where it is used.
CHANGE_SETTINGS=0
BIND="127.0.0.1"
PORT="8911"
HOSTS=""
NTFY=""
TS_HOST=""
STARTED=0
SERVICE_KIND="none"

# A run stopped with ctrl-c is an interrupted run, not a broken one: the state
# file already holds everything finished so far, so the only thing left to do
# is say so.
_interrupted() {
  printf '\n'
  wizard_say "stopped. Nothing that already worked has been lost."
  wizard_say "Run ./install.sh again and it carries on from here."
  exit 130
}
trap _interrupted INT TERM

# ============================================================ 1. welcome

step_welcome() {
  wizard_say "ayeaye puts a small web page on your phone that shows what the"
  wizard_say "coding agents on this computer are doing, and lets you talk to them."
  wizard_blank
  wizard_say "repo: $REPO"
  wizard_blank
  wizard_say "What setup may change on this computer:"
  wizard_say "  - a settings file at $ENV_FILE"
  wizard_say "  - a private key at $TOKEN_FILE, so that only you can open the page"
  wizard_say "  - a background service that starts ayeaye when you log in"
  wizard_say "  - software ayeaye needs, if you say yes when it asks"
  wizard_blank
  wizard_say "What it will never do without asking first: install anything, download"
  wizard_say "anything, change what this computer accepts from the network, trust a"
  wizard_say "new certificate, or replace settings you already have. Answering no to"
  wizard_say "any of those leaves this computer exactly as it is now."
  wizard_blank
  wizard_say "The page is locked to a key kept on this computer, and by default"
  wizard_say "ayeaye listens to this computer only - nothing else on your network"
  wizard_say "can reach it until you ask for that."
  if [ "$WIZARD_RESUMING" = 1 ]; then
    wizard_blank
    wizard_say "The last run stopped before it finished. This one carries on from"
    wizard_say "where it got to, and skips what was already done."
  fi
  return "$WIZARD_STAGE_OK"
}

# ============================================================= 2. detect

step_detect_machine() {
  # One probe for the whole run: the accessors below read a cache, and a cache
  # filled inside a command substitution would die with the subshell.
  platform_detect
  wizard_item "computer" "$(platform_pretty)"
  wizard_item "software" "$(platform_pkg_manager)"
  wizard_item "startup" "$(platform_service_manager)"
  wizard_detail "platform: $(platform_summary)"
  wizard_detail "privilege: $(platform_privilege)"
  return "$WIZARD_STAGE_OK"
}

step_detect_needs() {
  local dep missing=""
  for dep in tmux python3; do
    if command -v "$dep" >/dev/null 2>&1; then
      wizard_item ok "$dep"
    else
      wizard_item MISSING "$dep (required)"
      missing="$missing $dep"
    fi
  done
  [ -n "$missing" ] || return "$WIZARD_STAGE_OK"

  wizard_blank
  wizard_say "ayeaye cannot run without these:$missing"
  wizard_say "tmux is what the coding agents run inside, and python3 is what"
  wizard_say "ayeaye itself is written in."
  wizard_blank
  # The one door to installing anything, here and in every later ticket.
  # shellcheck disable=SC2086
  if wizard_install_packages $missing; then
    wizard_say "installed."
    return "$WIZARD_STAGE_OK"
  fi
  wizard_blank
  wizard_say "install the missing packages and re-run."
  return "$WIZARD_STAGE_FAIL"
}

# ====================================================== 3. what it can do

step_report() {
  local dep voice_missing="" transcriber=0
  if ! command -v tailscale >/dev/null 2>&1; then
    wizard_item note "tailscale not found: you will need another way to serve https to the phone"
  fi
  for dep in ffmpeg whisper-server ollama; do
    command -v "$dep" >/dev/null 2>&1 || voice_missing="$voice_missing $dep"
  done
  if [ -n "$voice_missing" ]; then
    wizard_say "  voice tier: missing$voice_missing (optional; text-only mode works without them)"
  else
    wizard_say "  voice tier: all present (ffmpeg, whisper-server, ollama)"
  fi
  wizard_blank
  wizard_say "What that means for you:"
  wizard_say "  - reading and typing to your agents from the phone: yes, always."
  if command -v ffmpeg >/dev/null 2>&1; then
    if command -v whisper-server >/dev/null 2>&1 \
       || command -v whisper-cpp >/dev/null 2>&1; then
      transcriber=1
    fi
  fi
  if [ "$transcriber" = 1 ]; then
    wizard_say "  - talking to them out loud: yes, this computer can transcribe."
  else
    wizard_say "  - talking to them out loud: not yet. ayeaye works without it and"
    wizard_say "    picks it up by itself once the pieces are installed - no re-run."
  fi
  wizard_blank
  wizard_say "Recommended here: text on the phone now, over an https address only"
  wizard_say "you can reach. Voice is worth adding later and costs nothing to skip."
  return "$WIZARD_STAGE_OK"
}

# ======================================================= 4. your choices

step_choose() {
  if command -v tailscale >/dev/null 2>&1; then
    TS_HOST="$(tailscale status --json 2>/dev/null \
      | python3 -c 'import sys,json;print(json.load(sys.stdin)["Self"]["DNSName"].rstrip("."))' \
      2>/dev/null)" || TS_HOST=""
  fi

  if [ -s "$ENV_FILE" ]; then
    wizard_say "config exists at $ENV_FILE"
    wizard_say "Nothing in it is lost either way: settings you are not asked about"
    wizard_say "here stay exactly as they are, and the old file is copied aside"
    wizard_say "before anything changes."
    # Replacing configuration somebody already has is one of the five things
    # that is never done without asking, so it goes through the primitive.
    if wizard_consent replace "rewrite it? (n keeps the current file)" \
         "merging into $ENV_FILE"; then
      CHANGE_SETTINGS=1
    else
      CHANGE_SETTINGS=0
      wizard_say "keeping existing config"
    fi
  else
    CHANGE_SETTINGS=1
  fi

  wizard_remember answer.change_settings "$CHANGE_SETTINGS"
  if [ "$CHANGE_SETTINGS" = 0 ]; then
    return "$WIZARD_STAGE_OK"
  fi

  # Each question offers what is already configured, so that pressing return
  # means "leave this as it is" - which is what pressing return looks like it
  # means, and what somebody rerunning setup to change one thing expects of the
  # other three. On a first run there is nothing configured and the offer is
  # the built-in default.
  wizard_blank
  wizard_say "Four questions about how to reach ayeaye. Press return to keep"
  wizard_say "what is in the square brackets."
  wizard_blank

  wizard_say "Which address should ayeaye answer on? 127.0.0.1 means this"
  wizard_say "computer only, which is the safe answer unless something else on"
  wizard_say "your network has to reach it directly."
  wizard_ask "bind address" "$(_current AYEAYE_BIND answer.bind 127.0.0.1)"
  BIND="$REPLY"

  wizard_say "Which port? Any number will do; change it only if something else"
  wizard_say "on this computer already uses this one."
  wizard_ask "port" "$(_current AYEAYE_PORT answer.port 8911)"
  PORT="$REPLY"

  wizard_say "If you reach ayeaye through an https address - a tailscale name,"
  wizard_say "or your own domain - name it here so the app accepts it."
  if [ -n "$(_current AYEAYE_ALLOWED_HOSTS answer.hosts "")" ]; then
    wizard_ask "allowed hosts (your https front)" \
      "$(_current AYEAYE_ALLOWED_HOSTS answer.hosts "")"
  elif [ -n "$TS_HOST" ]; then
    wizard_ask "allowed hosts (your https front)" "$TS_HOST"
  else
    wizard_ask "allowed hosts (your https front, comma separated; empty for none)" ""
  fi
  HOSTS="$REPLY"

  wizard_say "ayeaye can send your phone a notification when an agent needs you."
  wizard_say "That needs an ntfy topic address; leave it empty to switch it off."
  wizard_ask "ntfy topic URL for push notifications (empty disables)" \
    "$(_current VOICE_NTFY_URL answer.ntfy "")"
  NTFY="$REPLY"

  _decide_exposure
  _remember_choices
  wizard_plan_add config "your settings, at $ENV_FILE"
  return "$WIZARD_STAGE_OK"
}

# _current <env-key> <state-key> <built-in> - what to offer as the default.
#
# What is in the settings file wins, because that is what is true right now.
# Then what was chosen last time, which is what an interrupted run left behind
# before it got as far as writing anything. Then the built-in.
_current() {
  local value
  value="$(wizard_env_get "$ENV_FILE" "$1" "")"
  [ -n "$value" ] || value="$(wizard_state_get "$2" "")"
  [ -n "$value" ] || value="$3"
  printf '%s' "$value"
}

# Binding anywhere but this computer is the one choice that puts ayeaye within
# reach of something else, so it is a consent decision - recorded either way,
# and never one an unattended run is allowed to take.
_decide_exposure() {
  case "$BIND" in
    127.0.0.1|localhost|::1|"")
      wizard_consent_record expose refused "listening on this computer only"
      wizard_plan_add network "ayeaye will answer on $BIND:$PORT, this computer only"
      return 0
      ;;
  esac
  if [ "$WIZARD_INTERACTIVE" = 0 ]; then
    # Belt and braces. An unattended run never reads an answer, so it cannot
    # get here today - and the day a default changes, this is what stops that
    # change from putting somebody's machine on a network without them.
    wizard_consent_record expose refused "an unattended run never opens ayeaye to a network"
    BIND="127.0.0.1"
    wizard_plan_add network "ayeaye will answer on $BIND:$PORT, this computer only"
    return 0
  fi
  # The answer to the address question is the consent to exposure - it is an
  # explicit choice, typed a moment ago, and asking a second question about it
  # would be asking the same thing twice. What it does need is for the
  # consequence to be said out loud in words rather than left implied by an IP
  # address, and for the decision to be in the ledger like every other one.
  wizard_blank
  wizard_say "note: $BIND is not just this computer. Anything that can reach"
  wizard_say "this machine on your network will be able to reach ayeaye, and"
  wizard_say "the key in the bookmark is the only thing keeping them out."
  wizard_say "Answer 127.0.0.1 instead if you did not mean that."
  wizard_consent_record expose granted "you chose to listen on $BIND"
  wizard_plan_add network "ayeaye will answer on $BIND:$PORT, which other computers can reach"
  return 0
}

_remember_choices() {
  wizard_remember answer.bind "$BIND"
  wizard_remember answer.port "$PORT"
  wizard_remember answer.hosts "$HOSTS"
  wizard_remember answer.ntfy "$NTFY"
}

# ========================================================== 5. the plan

step_plan() {
  wizard_say "Before anything changes, here is all of it:"
  wizard_blank
  wizard_plan_show
  wizard_blank
  wizard_detail_hint

  # Only consequential work is worth stopping for. Writing your own settings
  # file and your own background service is not; installing, downloading,
  # privilege, trust and the network are.
  if [ "$WIZARD_INTERACTIVE" = 0 ]; then
    return "$WIZARD_STAGE_OK"
  fi
  if ! wizard_plan_is_consequential; then
    return "$WIZARD_STAGE_OK"
  fi
  if wizard_confirm "go ahead with all of that?" "y"; then
    return "$WIZARD_STAGE_OK"
  fi
  wizard_say "stopping here. Nothing has been changed."
  wizard_say "Run ./install.sh again whenever you want to pick this up."
  exit 0
}

# ======================================================== 6. do the work

step_write_settings() {
  local backup
  # From the state file, not from the variables: the stage that made these
  # decisions may have been finished by an earlier run that was interrupted
  # before it got here.
  CHANGE_SETTINGS="$(wizard_state_get answer.change_settings 0)"
  BIND="$(wizard_state_get answer.bind 127.0.0.1)"
  PORT="$(wizard_state_get answer.port 8911)"
  HOSTS="$(wizard_state_get answer.hosts "")"
  NTFY="$(wizard_state_get answer.ntfy "")"

  if [ "$CHANGE_SETTINGS" = 0 ]; then
    wizard_say "keeping the settings that are already there."
    return "$WIZARD_STAGE_SKIP"
  fi

  if [ -s "$ENV_FILE" ]; then
    if backup="$(wizard_backup "$ENV_FILE")"; then
      wizard_say "saved a copy of your old settings at $backup"
    else
      wizard_say "could not save a copy of $ENV_FILE, so it has been left alone."
      return "$WIZARD_STAGE_FAIL"
    fi
    # A merge, not a rewrite: anything in the file this run did not ask about
    # belongs to whoever put it there.
    wizard_env_merge "$ENV_FILE" \
      "AYEAYE_BIND=$BIND" "AYEAYE_PORT=$PORT" \
      "AYEAYE_ALLOWED_HOSTS=$HOSTS" "VOICE_NTFY_URL=$NTFY" \
      || return "$WIZARD_STAGE_FAIL"
  else
    wizard_env_render "$REPO/env.template" "$ENV_FILE" \
      "AYEAYE_BIND=$BIND" "AYEAYE_PORT=$PORT" \
      "AYEAYE_ALLOWED_HOSTS=$HOSTS" "VOICE_NTFY_URL=$NTFY" \
      || return "$WIZARD_STAGE_FAIL"
  fi
  wizard_say "wrote $ENV_FILE"
  return "$WIZARD_STAGE_OK"
}

step_make_key() {
  if [ ! -s "$TOKEN_FILE" ]; then
    mkdir -p "$WIZARD_STATE_DIR"
    # Same convention as the server: it would make this file itself on first
    # run. Making it here just means the address is printable now.
    ( umask 077
      python3 -c 'import secrets;print(secrets.token_urlsafe(32))' > "$TOKEN_FILE" ) \
      || return "$WIZARD_STAGE_FAIL"
    wizard_say "generated auth token at $TOKEN_FILE"
  else
    wizard_say "keeping the key you already have, so a bookmark already on your"
    wizard_say "phone still works."
  fi
  chmod 600 "$TOKEN_FILE"
  return "$WIZARD_STAGE_OK"
}

# Whatever is in the settings file now is what the last two stages report, so a
# kept config gets a correct summary and a correct service unit. Read at the
# start of every stage that needs it rather than once in a step that a resumed
# run may well skip.
_load_effective_values() {
  BIND="$(wizard_env_get "$ENV_FILE" AYEAYE_BIND 127.0.0.1)"
  PORT="$(wizard_env_get "$ENV_FILE" AYEAYE_PORT 8911)"
  HOSTS="$(wizard_env_get "$ENV_FILE" AYEAYE_ALLOWED_HOSTS "")"
  NTFY="$(wizard_env_get "$ENV_FILE" VOICE_NTFY_URL "")"
  return 0
}

# ====================================================== 7. start it up

_manual_instructions() {
  wizard_say "run the server with: $REPO/bin/ayeaye"
  wizard_say "(it reads $ENV_FILE by itself)"
}

step_service() {
  # The lifecycle's hook, and nothing more. What a service definition
  # contains, how it is installed on each platform, how an existing one is
  # migrated or repaired, and what to say on a machine that has no service
  # manager at all are one subject and they live together, in
  # lib/steps/70-service.sh. This file owns when that happens, not what it is.
  #
  # A missing implementation is reported as unfinished rather than as done:
  # the manual instructions are still true, and the run picks it up next time.
  if ! command -v service_step >/dev/null 2>&1; then
    wizard_say "the part of setup that starts ayeaye for you is not installed."
    _manual_instructions
    return "$WIZARD_STAGE_PENDING"
  fi
  service_step
}

# ============================================================= 8. done

step_summary() {
  local tier="text-only" token url_host first_host left
  _load_effective_values
  # The service stage is where SERVICE_KIND is worked out, and a resumed run
  # skips it once it is done - so ask the platform layer again rather than
  # trusting a variable that may never have been set this time.
  if [ "$NO_SYSTEMD" = 0 ]; then
    SERVICE_KIND="$(platform_service_manager)"
  fi
  [ -n "$NTFY" ] && tier="$tier +notifications"
  if command -v ffmpeg >/dev/null 2>&1 \
     && { command -v whisper-server >/dev/null 2>&1 \
          || command -v whisper-cpp >/dev/null 2>&1; }; then
    tier="$tier +voice"
  fi
  token="$(cat "$TOKEN_FILE" 2>/dev/null || true)"
  url_host="$BIND"
  first_host="${HOSTS%%,*}"

  wizard_say "tier    : $tier (probed live; the app adapts at runtime)"
  wizard_say "config  : $ENV_FILE"
  wizard_say "bookmark: http://$url_host:$PORT/?token=$token"
  if [ -n "$first_host" ]; then
    wizard_say "          behind tailscale serve or a proxy: https://$first_host/?token=$token"
  fi
  wizard_say "open the bookmark URL once on the phone; it sets a cookie and"
  wizard_say "redirects to /. Bookmark it and you never type the token again."
  if [ "$SERVICE_KIND" = "systemd" ]; then
    wizard_say "logs    : journalctl --user -u ayeaye -f"
  else
    wizard_say "logs    : stderr of $REPO/bin/ayeaye"
  fi
  wizard_say "https for the phone (mic needs a secure origin):"
  wizard_say "  tailscale serve --bg http://$url_host:$PORT"

  wizard_blank
  wizard_say "to change any of this later: run ./install.sh again. It keeps your"
  wizard_say "key and your settings, and asks before it changes either."
  wizard_say "to remove it: systemctl --user disable --now ayeaye, then delete"
  wizard_say "  $UNIT_DIR/ayeaye.service"
  wizard_say "  $ENV_FILE"
  wizard_say "  $WIZARD_STATE_DIR"

  left="$(wizard_unfinished)"
  if [ -n "$left" ]; then
    wizard_blank
    wizard_say "not finished, and worth coming back to:"
    printf '%s\n' "$left" | sed 's/^/  - /'
    wizard_say "running ./install.sh again picks these up."
  fi
  wizard_detail_hint
  return "$WIZARD_STAGE_OK"
}

# ========================================================== the lifecycle

wizard_stage welcome   "ayeaye setup"
wizard_stage detect    "checking this computer"
wizard_stage report    "what ayeaye can do here"
wizard_stage configure "configuration"
wizard_stage plan      "what is about to happen"
wizard_stage install   "setting it up"
wizard_stage service   "service"
wizard_stage finish    "done"

# The welcome, the report, the plan and the summary are what the run looks
# like rather than work that can be already done, so they run every time.
wizard_step welcome   greeting step_welcome        "Welcome"                 required always
wizard_step detect    machine  step_detect_machine "What this computer is"
wizard_step detect    needs    step_detect_needs   "What ayeaye needs"
wizard_step report    capable  step_report         "What ayeaye can do here" required always
wizard_step configure choices  step_choose         "Your choices"
wizard_step plan      confirm  step_plan           "The plan"                required always
wizard_step install   settings step_write_settings "Your settings file"
wizard_step install   key      step_make_key       "Your private key"
wizard_step service   unit     step_service        "Starting ayeaye in the background"
wizard_step finish    summary  step_summary        "What to do next"         required always

# Anything a sibling ticket owns registers itself here, after the core steps
# and in filename order.
wizard_load_steps

if [ "$FRESH" = 1 ]; then
  wizard_state_clear
fi
wizard_begin_run
# A new pass starts a fresh plan and a fresh audit trail. A resumed run does
# not: the stages that filled them in are the stages it is about to skip, and
# emptying them would let stage five say "nothing needs to be installed" and
# then install something.
if [ "$WIZARD_RESUMING" = 0 ]; then
  wizard_consent_reset
  wizard_plan_reset
fi

STATUS=0
wizard_run_stages || STATUS=$?
wizard_end_run "$STATUS"
exit "$STATUS"
