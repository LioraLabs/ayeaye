#!/usr/bin/env bash
# The whole wizard, end to end, inside a real distro image.
#
#   bash tests/lib/wizard_probe.sh
#
# The third probe, and the one that runs `./install.sh` itself rather than a
# piece of the layer underneath it. tests/lib/platform_probe.sh asks the
# platform layer questions; tests/lib/install_probe.sh installs four packages
# through the consent wrapper; this one does what a person does, on a machine
# with no tmux and no python3, and then asks the only question that matters
# about the closing checklist: **does what it claims match what is actually on
# this machine.**
#
# It changes the machine it runs on, so it is only ever run inside a container
# that is about to be thrown away. tests/containers.sh is what does that.
# Running it on a laptop would install packages on the laptop.
#
# Four passes, and each one answers something the others cannot:
#
#   1  a first install on a bare machine
#   2  a rerun, which must change nothing and must keep a setting nobody asked
#      about
#   3  a run in which an optional component cannot be finished, which must end
#      with that item on the unfinished list and must not claim it works
#   4  a resumed run, which must skip what the first one finished
#
# Prints one key=value per line and nothing else. A value that is a sentence is
# reduced to yes/no here rather than left for the caller to grep, so that the
# assertions in tests/containers.sh stay one line each.
#
# bash 3.2: no associative arrays, no ${var,,}, no mapfile.
set -u

PROBE_DIR="${BASH_SOURCE[0]%/*}"
case "$PROBE_DIR" in
  "${BASH_SOURCE[0]}") PROBE_DIR="." ;;
esac
REPO="$(cd "$PROBE_DIR/../.." && pwd)"

# A home of its own inside the container. The repository is mounted read-only,
# and this has to be somewhere the wizard may write.
WORK="${WIZARD_PROBE_HOME:-/tmp/ayeaye-wizard-probe}"
rm -rf "$WORK"
mkdir -p "$WORK"
HOME="$WORK"
XDG_CONFIG_HOME="$WORK/.config"
XDG_STATE_HOME="$WORK/.local/state"
XDG_DATA_HOME="$WORK/.local/share"
export HOME XDG_CONFIG_HOME XDG_STATE_HOME XDG_DATA_HOME

ENV_FILE="$XDG_CONFIG_HOME/ayeaye/env"
TOKEN_FILE="$XDG_STATE_HOME/ayeaye/token"
STATE_FILE="$XDG_STATE_HOME/ayeaye/setup-state"

say()  { printf '%s=%s\n' "$1" "$2"; }
yesno() { if [ "$1" = 0 ]; then printf 'yes'; else printf 'no'; fi; }

# has <file> <substring> - yes when the file contains it.
has() {
  [ -f "$1" ] || { printf 'no'; return 0; }
  case "$(cat "$1")" in
    *"$2"*) printf 'yes' ;;
    *)      printf 'no' ;;
  esac
}

# sum <file> - something to compare a file against itself with later. cksum is
# in every one of these images; sha256sum is not.
sum() { [ -f "$1" ] && cksum < "$1" || printf 'absent'; }

run_wizard() {
  local out="$1"
  shift
  ( cd "$REPO" && bash ./install.sh "$@" ) >"$out" 2>&1
  return $?
}

# ================================================================ pass 1: fresh

FIRST="$WORK/first.out"
run_wizard "$FIRST" --defaults --yes
say first_status "$?"

say tmux_here "$(command -v tmux >/dev/null 2>&1 && printf yes || printf no)"
say python3_here "$(command -v python3 >/dev/null 2>&1 && printf yes || printf no)"
say env_written "$([ -s "$ENV_FILE" ] && printf yes || printf no)"
say token_written "$([ -s "$TOKEN_FILE" ] && printf yes || printf no)"

# Nothing was installed before the plan was shown. The plan heading has to come
# out before the install stage's heading, and the packages have to be named in
# it - which is the whole of the promise "everything is summarized before you
# are asked".
plan_line="$(grep -n 'what is about to happen' "$FIRST" | head -1 | cut -d: -f1)"
install_line="$(grep -n 'setting it up' "$FIRST" | head -1 | cut -d: -f1)"
if [ -n "$plan_line" ] && [ -n "$install_line" ] && [ "$plan_line" -lt "$install_line" ]; then
  say plan_before_install yes
else
  say plan_before_install no
fi
say plan_named_packages "$(has "$FIRST" 'which ayeaye cannot run without')"

# ------------------------------------------- the checklist against the machine
#
# Every claim the closing screen makes is checked against the container it was
# made on. None of these images has a user service session, none of them has a
# coding agent, and none of them has cliban - so a checklist that says
# otherwise is the exact failure this whole ticket exists to prevent.
say claims_service "$(has "$FIRST" 'ayeaye starts when you log in, and is running now')"
say claims_systemctl_removal "$(has "$FIRST" 'systemctl --user disable')"
say claims_phone_address "$(has "$FIRST" 'open that one on your phone.')"
say says_no_way_in_yet "$(has "$FIRST" 'cannot reach ayeaye yet')"
say says_manual_start "$(has "$FIRST" 'run the server with:')"
say health_had_nothing_to_check "$(has "$FIRST" 'ayeaye is not running yet, so there is nothing to check.')"
say voice_described_once "$(test "$(grep -c 'Talking to your agents out loud' "$FIRST")" = 1 && printf yes || printf no)"
say old_voice_sweep_gone "$(has "$FIRST" 'voice tier:')"

FIRST_ENV_SUM="$(sum "$ENV_FILE")"
FIRST_TOKEN="$(cat "$TOKEN_FILE" 2>/dev/null || printf '')"

# ============================================================== pass 2: a rerun
#
# A second pass must install nothing again, must keep the key - a bookmark
# already on somebody's phone depends on it - and must keep a setting the
# wizard has never heard of.
printf 'MY_OWN_SETTING=keep-me\n' >> "$ENV_FILE"
KEPT_SUM="$(sum "$ENV_FILE")"

SECOND="$WORK/second.out"
run_wizard "$SECOND" --defaults --yes
say second_status "$?"
say second_installed_nothing "$(has "$SECOND" 'everything setup needs is already on this computer')"
say second_kept_the_key \
  "$([ "$(cat "$TOKEN_FILE" 2>/dev/null || printf '')" = "$FIRST_TOKEN" ] && printf yes || printf no)"
say second_kept_unknown_setting "$(has "$ENV_FILE" 'MY_OWN_SETTING=keep-me')"
say second_left_settings_alone \
  "$([ "$(sum "$ENV_FILE")" = "$KEPT_SUM" ] && printf yes || printf no)"

# ================================== pass 3: an optional component that cannot be
#
# The board was chosen, permission to download was given, and the download
# cannot work. That is a component that was tried and failed, which is a
# different thing from one that was refused - a refusal is finished business
# and belongs on no list at all. The run must still end successfully, must put
# the board on the unfinished list, and must never say the board works.

# set_answer <key> <value> - the state store returns the *first* line matching a
# key, so appending a second one changes nothing at all.
set_answer() {
  [ -f "$STATE_FILE" ] || return 0
  grep -v "^$1=" "$STATE_FILE" > "$STATE_FILE.tmp" 2>/dev/null
  printf '%s=%s\n' "$1" "$2" >> "$STATE_FILE.tmp"
  mv "$STATE_FILE.tmp" "$STATE_FILE"
}

if command -v cliban >/dev/null 2>&1; then
  say optional_case skipped-cliban-present
else
  say optional_case tried
  set_answer answer.board 1
  THIRD="$WORK/third.out"
  # Port 9 is discard: nothing is listening, and nothing ever will be.
  AYEAYE_CLIBAN_RELEASE_URL="http://127.0.0.1:9/not-a-release" \
    run_wizard "$THIRD" --defaults --yes
  say third_status "$?"
  say third_reports_unfinished "$(has "$THIRD" 'not finished, and worth coming back to:')"
  say third_names_the_board "$(has "$THIRD" 'cliban, for the project board (did not work)')"
  say third_says_how_to_resume "$(has "$THIRD" 'install it yourself and setup will find it')"
  say third_claims_board_works "$(has "$THIRD" 'your ticket board answers on the phone')"
  # And the rest of the run is unaffected: an optional component that could not
  # be installed must not take the settings or the key down with it.
  say third_kept_the_key \
    "$([ "$(cat "$TOKEN_FILE" 2>/dev/null || printf '')" = "$FIRST_TOKEN" ] && printf yes || printf no)"
fi

# ======================================================== pass 4: picking it up
#
# Resumability, made deterministic. A real keyboard interrupt lands wherever
# the machine happens to be, which is not something to build an assertion on;
# what is worth pinning is the property underneath it - a run that did not
# reach the end is picked up rather than started again - and that is a fact
# about the state file, so the state file is what is arranged.
if [ -f "$STATE_FILE" ]; then
  # What an interrupted run leaves: everything finished so far, and no record
  # of having completed.
  sed 's/^run\.completed=.*/run.completed=0/' "$STATE_FILE" > "$STATE_FILE.tmp" \
    && mv "$STATE_FILE.tmp" "$STATE_FILE"
  FOURTH="$WORK/fourth.out"
  run_wizard "$FOURTH" --defaults
  say fourth_status "$?"
  say fourth_resumed "$(has "$FOURTH" 'The last run stopped before it finished.')"
  say fourth_skipped_finished_work "$(has "$FOURTH" '(already done)')"
  say fourth_kept_the_key \
    "$([ "$(cat "$TOKEN_FILE" 2>/dev/null || printf '')" = "$FIRST_TOKEN" ] && printf yes || printf no)"
else
  say fourth_status no-state-file
fi

exit 0
