# The lifecycle: eight stages, the steps inside them, and resume.
#
#   . "$REPO/lib/stage.sh"      (needs lib/state.sh and lib/ui.sh first)
#
# Sourcing is inert.
#
# A stage is a chapter of the conversation and has a title a person reads. A
# step is one unit of work inside it, and is the seam every other ticket in
# this milestone attaches to: hardware detection, package installation, service
# setup, voice model selection, secure access and health checking are all steps
# registered onto a stage that already exists, by a file that never edits the
# stage itself.
#
# ---------------------------------------------------------------- registering
#
#   wizard_stage <id> <title>
#         Declare a stage. Order of declaration is order of execution. 2 for a
#         malformed or duplicate id.
#
#   wizard_step <stage> <step> <function> <label> [policy] [resume]
#         Add a unit of work to a stage that has already been declared.
#           policy  required (default) | optional
#                   Optional work that fails does not stop the run.
#           resume  once (default) | always
#                   "once" work is remembered and not repeated by a resumed
#                   run. "always" work - a welcome, a summary - is what the run
#                   looks like rather than something that can be done already,
#                   and its outcome is never recorded.
#         2 for an unknown stage, a duplicate step id, or a function that does
#         not exist. Registering is checked eagerly on purpose: a typo in a
#         step name must fail at the top of the run, not two stages in.
#
# The function is called with no arguments and may read:
#
#   WIZARD_STAGE_ID   the stage it is running in
#   WIZARD_STEP_ID    "<stage>.<step>", which is what it should key any state
#                     of its own on - wizard_state_set "step.$WIZARD_STEP_ID.…"
#
# ------------------------------------------------------------ what it answers
#
# A step function answers with one of these, and nothing else:
#
#   $WIZARD_STAGE_OK       0   the work is finished
#   $WIZARD_STAGE_PENDING  10  it ran, and work remains. This is what a seam
#                              whose implementation has not landed yet returns,
#                              and what deferred optional work returns. A stage
#                              holding a pending step is never recorded as done,
#                              so nothing can claim success it did not have.
#   $WIZARD_STAGE_SKIP     11  it does not apply to this machine, or the user
#                              said no. Finished business; nothing remains.
#   $WIZARD_STAGE_FAIL     12  it tried and could not
#
# Any other non-zero status is read as a failure, so a step that forgets to
# say so is treated as having failed rather than as having succeeded.
#
# ------------------------------------------------------------------- running
#
#   wizard_begin_run       decide whether this invocation resumes an interrupted
#                          run or starts a new pass, and say which. Always 0.
#   wizard_run_stages      walk them. 0 when nothing required was left undone,
#                          non-zero when the run stopped early.
#   wizard_end_run <walk-status>
#                          record how the walk ended: completed when the status
#                          is 0, still open otherwise. Always 0.
#   wizard_stage_status <stage>         done | pending | failed | ""
#   wizard_step_status <stage> <step>   the same, for one step
#   wizard_unfinished      the outstanding work, one line each, in words
#   wizard_unfinished_rows the same, as "<stage>\t<step>\t<status>\t<label>",
#                          for a caller that has to say how to resume each one
#   wizard_reset_progress  forget which stages and steps finished, keep the
#                          answers. This is what a rerun does, and what --fresh
#                          does more of.
#   wizard_remember <key> <value>
#                          persist something without any risk of ending the run.
#                          What a step records its own state through.
#
# A resumed run skips steps recorded done, says so, and re-runs everything
# else. That is the whole of resumability: nothing is redone, and nothing that
# was not finished is forgotten.
#
# bash 3.2: no associative arrays, no ${var,,}, no mapfile, no local -n.

WIZARD_STAGE_OK=0
WIZARD_STAGE_PENDING=10
WIZARD_STAGE_SKIP=11
WIZARD_STAGE_FAIL=12

# How many times the wizard will try again by itself before it stops asking.
# Without it an answer of "again" to something that can never work is a loop
# with no way out but a keyboard interrupt.
WIZARD_MAX_RETRIES="${WIZARD_MAX_RETRIES:-3}"

_WIZARD_STAGE_IDS="${_WIZARD_STAGE_IDS:-}"
_WIZARD_STAGE_TITLES="${_WIZARD_STAGE_TITLES:-}"
_WIZARD_STEP_TABLE="${_WIZARD_STEP_TABLE:-}"
_WIZARD_RUN_FAILED="${_WIZARD_RUN_FAILED:-0}"

WIZARD_STAGE_ID="${WIZARD_STAGE_ID:-}"
WIZARD_STEP_ID="${WIZARD_STEP_ID:-}"
WIZARD_RESUMING="${WIZARD_RESUMING:-0}"

_WIZARD_STAGE_TAB='	'
_WIZARD_RECOVER="${_WIZARD_RECOVER:-}"
_WIZARD_STORE_WARNED=0

# wizard_remember <key> <value> - persist, and never let the store take the run
# down with it. Always 0.
#
# This is what a step should record its own state through, rather than
# wizard_state_set: a store that cannot be written costs resumability, which is
# worth one warning and nothing more, and every call site is bare under
# `set -e` where wizard_state_set's status 1 would be fatal.
wizard_remember() {
  wizard_state_set "$1" "$2" && return 0
  if [ "$_WIZARD_STORE_WARNED" = 0 ]; then
    _WIZARD_STORE_WARNED=1
    wizard_say "note: setup cannot save its progress to $(wizard_state_path),"
    wizard_say "so an interrupted run will start over rather than pick up."
  fi
  return 0
}

# ---------------------------------------------------------------- registering

_wizard_valid_id() {
  case "${1:-}" in
    "") return 1 ;;
    *[!A-Za-z0-9_-]*) return 1 ;;
    *) return 0 ;;
  esac
}

_wizard_stage_known() {
  case "
$_WIZARD_STAGE_IDS" in
    *"
$1
"*) return 0 ;;
  esac
  return 1
}

wizard_stage() {
  local id="${1:-}" title="${2:-}"
  _wizard_valid_id "$id" || return 2
  [ -n "$title" ] || return 2
  _wizard_stage_known "$id" && return 2
  _WIZARD_STAGE_IDS="$_WIZARD_STAGE_IDS$id
"
  _WIZARD_STAGE_TITLES="$_WIZARD_STAGE_TITLES$id$_WIZARD_STAGE_TAB$title
"
  return 0
}

wizard_stage_title() {
  local want="${1:-}" line=""
  while IFS= read -r line || [ -n "$line" ]; do
    case "$line" in
      "$want$_WIZARD_STAGE_TAB"*) printf '%s' "${line#*"$_WIZARD_STAGE_TAB"}"; return 0 ;;
    esac
  done <<EOF
$_WIZARD_STAGE_TITLES
EOF
  return 1
}

wizard_stages() { printf '%s' "$_WIZARD_STAGE_IDS"; }

# _wizard_step_known <stage> <step>
_wizard_step_known() {
  local line=""
  while IFS= read -r line || [ -n "$line" ]; do
    case "$line" in
      "$1$_WIZARD_STAGE_TAB$2$_WIZARD_STAGE_TAB"*) return 0 ;;
    esac
  done <<EOF
$_WIZARD_STEP_TABLE
EOF
  return 1
}

wizard_step() {
  local stage="${1:-}" step="${2:-}" fn="${3:-}" label="${4:-}"
  local policy="${5:-required}" resume="${6:-once}"
  _wizard_valid_id "$stage" || return 2
  _wizard_valid_id "$step" || return 2
  [ -n "$fn" ] && [ -n "$label" ] || return 2
  _wizard_stage_known "$stage" || return 2
  _wizard_step_known "$stage" "$step" && return 2
  case "$policy" in required|optional) ;; *) return 2 ;; esac
  # The registry is a tab-delimited table and the label is its last column, so
  # a label carrying either delimiter would be read back as something else.
  case "$label" in
    *"$_WIZARD_STAGE_TAB"*|*"
"*) return 2 ;;
  esac
  case "$resume" in once|always) ;; *) return 2 ;; esac
  # Eagerly, so a mistyped step name is a complaint at registration rather
  # than a stage that quietly does nothing halfway through a run.
  command -v "$fn" >/dev/null 2>&1 || return 2
  _WIZARD_STEP_TABLE="$_WIZARD_STEP_TABLE$stage$_WIZARD_STAGE_TAB$step$_WIZARD_STAGE_TAB$fn$_WIZARD_STAGE_TAB$policy$_WIZARD_STAGE_TAB$resume$_WIZARD_STAGE_TAB$label
"
  return 0
}

# ------------------------------------------------------------------- statuses

wizard_step_status() { wizard_state_get "step.${1:-}.${2:-}"; }
wizard_stage_status() { wizard_state_get "stage.${1:-}"; }

# _wizard_worse <a> <b> - the more unfinished of two statuses.
_wizard_worse() {
  case "$1:$2" in
    failed:*|*:failed)   printf 'failed' ;;
    pending:*|*:pending) printf 'pending' ;;
    *)                   printf 'done' ;;
  esac
}

_wizard_status_word() {
  case "$1" in
    0)  printf 'done' ;;
    10) printf 'pending' ;;
    11) printf 'skipped' ;;
    *)  printf 'failed' ;;
  esac
}

# wizard_unfinished - what is still outstanding, in the words it was announced
# in. Empty when there is nothing, so a caller can test it as a string.
wizard_unfinished() {
  local line="" stage step label status
  while IFS= read -r line || [ -n "$line" ]; do
    [ -n "$line" ] || continue
    stage="${line%%"$_WIZARD_STAGE_TAB"*}"
    line="${line#*"$_WIZARD_STAGE_TAB"}"
    step="${line%%"$_WIZARD_STAGE_TAB"*}"
    label="${line##*"$_WIZARD_STAGE_TAB"}"
    status="$(wizard_step_status "$stage" "$step")"
    case "$status" in
      pending) printf '%s (not finished)\n' "$label" ;;
      failed)  printf '%s (did not work)\n' "$label" ;;
    esac
  done <<EOF
$_WIZARD_STEP_TABLE
EOF
  return 0
}

# wizard_unfinished_rows - the same outstanding work, with the stage and the
# step it belongs to still attached: "<stage>\t<step>\t<status>\t<label>", one
# row each, in registration order. Empty when there is nothing.
#
# wizard_unfinished answers "what is left" in words a person reads; this
# answers "which piece of setup was that" so that a caller can say how to pick
# each one up again. Two functions rather than one because the first is printed
# straight to a screen and adding two columns to it would put step ids in front
# of somebody who has never used a terminal.
wizard_unfinished_rows() {
  local line="" stage step label status
  while IFS= read -r line || [ -n "$line" ]; do
    [ -n "$line" ] || continue
    stage="${line%%"$_WIZARD_STAGE_TAB"*}"
    line="${line#*"$_WIZARD_STAGE_TAB"}"
    step="${line%%"$_WIZARD_STAGE_TAB"*}"
    label="${line##*"$_WIZARD_STAGE_TAB"}"
    status="$(wizard_step_status "$stage" "$step")"
    case "$status" in
      pending|failed)
        printf '%s%s%s%s%s%s%s\n' \
          "$stage" "$_WIZARD_STAGE_TAB" "$step" "$_WIZARD_STAGE_TAB" \
          "$status" "$_WIZARD_STAGE_TAB" "$label"
        ;;
    esac
  done <<EOF
$_WIZARD_STEP_TABLE
EOF
  return 0
}

wizard_reset_progress() {
  wizard_state_unset_prefix "stage."
  wizard_state_unset_prefix "step."
  return 0
}

# --------------------------------------------------------------- the run

# wizard_begin_run - is this a resume, or a new pass?
#
# A run that walked all its stages records run.completed. The next invocation
# is therefore a deliberate rerun - reconfiguration or repair - and starts from
# the top with the previous answers offered back as defaults. A run that did
# not get that far was interrupted, and is picked up where it stopped.
wizard_begin_run() {
  local count
  wizard_state_load
  WIZARD_RESUMING=0
  if wizard_state_is_new; then
    wizard_remember run.version 1
    wizard_remember run.count 1
  elif [ "$(wizard_state_get run.completed)" = 1 ]; then
    wizard_reset_progress
    count="$(wizard_state_get run.count 0)"
    # The store tolerates a file somebody edited, so the count might not be a
    # number. Under `set -u` feeding that to $(( )) is fatal, not zero.
    case "$count" in
      ""|*[!0-9]*) count=0 ;;
    esac
    wizard_remember run.count "$((count + 1))"
  else
    WIZARD_RESUMING=1
  fi
  wizard_remember run.completed 0
  wizard_remember run.started "$(date +%s 2>/dev/null || printf 0)"
  return 0
}

# wizard_end_run <walk-status> - record how the walk ended. Always 0.
#
# Completion means "the walk reached the last stage", not "everything got
# done": a stage holding an unfinished seam is still a stage the wizard walked
# past, and the next invocation is a deliberate rerun rather than a resume.
# Anything short of that - a failure, a keyboard interrupt - leaves the run
# open, which is what makes the next invocation pick up where this one stopped.
wizard_end_run() {
  if [ "${1:-1}" = 0 ]; then
    wizard_remember run.completed 1
  else
    wizard_remember run.completed 0
  fi
  return 0
}

# _wizard_step_recover <policy> <may-retry> -> $_WIZARD_RECOVER
#      again | skip | stop | auto
#
# Plain language, and it never offers to leave out work the install cannot do
# without: an offer that would produce a broken install is worse than no offer.
#
# An unattended run has nobody to ask, so it answers "auto" and leaves the
# decision to policy: the caller carries on past optional work and gives up on
# required work. An optional failure stays recorded as a failure rather than
# being dressed up as a choice nobody made.
#
# It answers through a variable rather than stdout because it also *talks*:
# "please answer with one of the words in brackets" is stdout too, and a
# command substitution would hand that text back as the user's answer.
_wizard_step_recover() {
  local policy="$1" may_retry="$2" tries=0
  _WIZARD_RECOVER=""
  if [ "${WIZARD_INTERACTIVE:-1}" = 0 ]; then
    _WIZARD_RECOVER="auto"
    return 0
  fi
  while [ "$tries" -lt 5 ]; do
    tries=$((tries + 1))
    if [ "$policy" = optional ] && [ "$may_retry" = 1 ]; then
      wizard_ask "try again, leave it out, or stop here? (again/skip/stop)" "again"
    elif [ "$policy" = optional ]; then
      wizard_ask "leave it out, or stop here? (skip/stop)" "skip"
    elif [ "$may_retry" = 1 ]; then
      wizard_ask "try again, or stop here? (again/stop)" "again"
    else
      wizard_ask "stop here and pick this up later? (stop)" "stop"
    fi
    case "$REPLY" in
      a|again|retry|y|yes)
        if [ "$may_retry" = 1 ]; then
          _WIZARD_RECOVER="again"
          return 0
        fi
        wizard_say "it has been tried enough times; something else has to change first."
        ;;
      s|stop|q|quit|exit|n|no) _WIZARD_RECOVER="stop"; return 0 ;;
      skip|leave|omit)
        if [ "$policy" = optional ]; then
          _WIZARD_RECOVER="skip"
          return 0
        fi
        wizard_say "this part is needed, so it cannot be left out."
        ;;
      *) wizard_say "please answer with one of the words in brackets." ;;
    esac
  done
  _WIZARD_RECOVER="stop"
  return 0
}

# _wizard_run_step <stage> <step> <fn> <policy> <resume> <label>
#
# Sets _WIZARD_STEP_OUTCOME to the status word. Returns 0 when the walk may
# carry on and 1 when the person asked to stop.
#
# Standard input is fd 8 throughout: the walk reads its own tables from
# heredocs, and a step that asked a question would otherwise be answered by the
# table it is being read out of.
_wizard_run_step() {
  local stage="$1" step="$2" fn="$3" policy="$4" resume="$5" label="$6"
  local recorded status word choice attempts=0 may_retry

  if [ "$resume" = once ]; then
    recorded="$(wizard_step_status "$stage" "$step")"
    if [ "$recorded" = done ] || [ "$recorded" = skipped ]; then
      wizard_item "kept" "$label (already done)"
      _WIZARD_STEP_OUTCOME="$recorded"
      return 0
    fi
  fi

  WIZARD_STAGE_ID="$stage"
  WIZARD_STEP_ID="$stage.$step"

  while :; do
    attempts=$((attempts + 1))
    "$fn" <&8
    status=$?
    word="$(_wizard_status_word "$status")"
    if [ "$resume" = always ]; then
      # Nothing to remember about work that runs every time - except a failure,
      # which the closing summary has to be able to report.
      if [ "$word" = failed ]; then
        wizard_remember "step.$stage.$step" failed
      else
        wizard_state_unset "step.$stage.$step" || true
      fi
    else
      wizard_remember "step.$stage.$step" "$word"
    fi
    if [ "$word" != failed ]; then
      _WIZARD_STEP_OUTCOME="$word"
      return 0
    fi

    # Plain language first, the raw material behind the affordance, and the way
    # back always: whatever happens next, the finished work is on disk.
    wizard_blank
    wizard_say "this part did not work: $label"
    wizard_detail_hint
    wizard_say "nothing that already worked has been lost. Run ./install.sh"
    wizard_say "again at any time and it carries on from here."

    may_retry=1
    [ "$attempts" -lt "$WIZARD_MAX_RETRIES" ] || may_retry=0
    _wizard_step_recover "$policy" "$may_retry" <&8
    choice="$_WIZARD_RECOVER"
    case "$choice" in
      again) continue ;;
      skip)
        wizard_remember "step.$stage.$step" skipped
        _WIZARD_STEP_OUTCOME="skipped"
        wizard_say "leaving that out. Setup can add it later."
        return 0
        ;;
      auto)
        _WIZARD_STEP_OUTCOME="failed"
        [ "$policy" = optional ] && return 0
        return 1
        ;;
      *)
        _WIZARD_STEP_OUTCOME="failed"
        return 1
        ;;
    esac
  done
}

# wizard_run_stages - walk them all. 0 when nothing required was left undone.
wizard_run_stages() {
  local stage="" line="" title
  local s_stage s_step s_fn s_policy s_resume s_label rest
  local stage_word stopped=0

  _WIZARD_RUN_FAILED=0
  # fd 8 is the real standard input, kept out of reach of the heredocs below.
  exec 8<&0
  while IFS= read -r stage <&3 || [ -n "$stage" ]; do
    [ -n "$stage" ] || continue
    title="$(wizard_stage_title "$stage")" || title="$stage"
    wizard_head "$title"
    stage_word="done"

    while IFS= read -r line <&4 || [ -n "$line" ]; do
      [ -n "$line" ] || continue
      s_stage="${line%%"$_WIZARD_STAGE_TAB"*}"
      [ "$s_stage" = "$stage" ] || continue
      rest="${line#*"$_WIZARD_STAGE_TAB"}"
      s_step="${rest%%"$_WIZARD_STAGE_TAB"*}";   rest="${rest#*"$_WIZARD_STAGE_TAB"}"
      s_fn="${rest%%"$_WIZARD_STAGE_TAB"*}";     rest="${rest#*"$_WIZARD_STAGE_TAB"}"
      s_policy="${rest%%"$_WIZARD_STAGE_TAB"*}"; rest="${rest#*"$_WIZARD_STAGE_TAB"}"
      s_resume="${rest%%"$_WIZARD_STAGE_TAB"*}"; s_label="${rest#*"$_WIZARD_STAGE_TAB"}"

      _WIZARD_STEP_OUTCOME=""
      _wizard_run_step "$s_stage" "$s_step" "$s_fn" "$s_policy" "$s_resume" "$s_label" \
        || stopped=1
      case "$_WIZARD_STEP_OUTCOME" in
        pending) stage_word="$(_wizard_worse "$stage_word" pending)" ;;
        failed)
          # Optional work that could not be done is reported, not fatal, and
          # stays recorded as failed so the closing summary can say so.
          stage_word="failed"
          [ "$stopped" = 1 ] && _WIZARD_RUN_FAILED=1
          ;;
      esac
      [ "$stopped" = 1 ] && break
    done 4<<EOF
$_WIZARD_STEP_TABLE
EOF

    WIZARD_STAGE_ID=""
    WIZARD_STEP_ID=""
    wizard_remember "stage.$stage" "$stage_word"
    [ "$stopped" = 1 ] && break
  done 3<<EOF
$_WIZARD_STAGE_IDS
EOF
  exec 8<&-

  [ "$_WIZARD_RUN_FAILED" = 0 ] || return 1
  return 0
}
