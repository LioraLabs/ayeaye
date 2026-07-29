# How the wizard talks, and how it listens.
#
#   . "$REPO/lib/ui.sh"
#
# Sourcing is inert. Nothing is printed, nothing is read, no file is opened.
#
# The audience has never used a terminal. Two rules follow from that and this
# file is where they are enforced rather than remembered:
#
#   Jargon in a prompt is a defect. A question names the thing in the words
#   the person already has - "your phone", "this computer", "the internet" -
#   and never a unit name, a flag or a path they did not choose.
#
#   Raw commands and raw output are technical details. They go to the log
#   through wizard_detail, where they are available to anyone who wants them
#   and in the way of nobody who does not.
#
# ---------------------------------------------------------------- the contract
#
#   wizard_say <text>…          one line of plain language on stdout
#   wizard_blank                one empty line
#   wizard_head <title>         a stage heading
#   wizard_item <mark> <text>   a checklist line: "  ok       tmux"
#   wizard_detail <text>…       a technical detail: always logged, echoed only
#                               when the run asked for details
#   wizard_detail_hint          tell the reader where the log is, once
#   wizard_log_path             where that log is. Always 0.
#   wizard_ask <prompt> <default>       -> $REPLY. Always 0.
#   wizard_confirm <prompt> <default>   status 0 for yes
#
# wizard_ask renders "<prompt> [<default>]: " and treats an empty answer as the
# default, which is what makes a scripted answer of "" mean "take what you
# offered me". An empty default renders as "[empty]". When the run may not ask
# - WIZARD_INTERACTIVE=0, which is what --defaults sets - it takes the default
# without reading, so an unattended run can never be steered by whatever
# happens to be on standard input.
#
# ----------------------------------------------------------------- tunables
#
#   WIZARD_INTERACTIVE  1 when questions may be asked, 0 when they may not
#   WIZARD_DETAILS      1 to echo technical details as well as logging them
#   WIZARD_LOG_FILE     where the technical detail goes
#
# bash 3.2: no associative arrays, no ${var,,}, no mapfile, no local -n.

WIZARD_INTERACTIVE="${WIZARD_INTERACTIVE:-1}"
WIZARD_DETAILS="${WIZARD_DETAILS:-0}"
WIZARD_LOG_FILE="${WIZARD_LOG_FILE:-${WIZARD_STATE_DIR:-${XDG_STATE_HOME:-$HOME/.local/state}/ayeaye}/setup.log}"

_WIZARD_DETAIL_HINTED="${_WIZARD_DETAIL_HINTED:-0}"

wizard_say()   { printf '%s\n' "$*"; }
wizard_blank() { printf '\n'; }

# wizard_head <title> - a stage heading. Bold when the terminal will have it,
# and harmless bytes when it will not.
wizard_head() { printf '\n\033[1m%s\033[0m\n' "$*"; }

# wizard_item <mark> <text> - "  ok       tmux", "  MISSING  tmux (required)".
# The mark is padded to a fixed width so the verdicts line up in a column and
# the eye can run down them without reading a word.
wizard_item() {
  local mark="${1:-}"
  [ "$#" -eq 0 ] || shift
  printf '  %-9s%s\n' "$mark" "$*"
}

wizard_log_path() { printf '%s' "$WIZARD_LOG_FILE"; }

# wizard_detail <text>… - the raw material: a command about to run, the output
# of one that failed, a path nobody asked about. Always appended to the log,
# echoed to the screen only when the run was started with --details.
#
# A log that cannot be written is not worth failing a run over, so the write
# is best-effort and its failure is silent.
wizard_detail() {
  local dir
  if [ "$WIZARD_DETAILS" = 1 ]; then
    printf '  | %s\n' "$*"
  fi
  dir="${WIZARD_LOG_FILE%/*}"
  [ "$dir" = "$WIZARD_LOG_FILE" ] || mkdir -p "$dir" 2>/dev/null || return 0
  printf '%s\n' "$*" >> "$WIZARD_LOG_FILE" 2>/dev/null || true
  return 0
}

# wizard_detail_hint - name the log, at most once per run, and only when there
# is one to name. Nobody needs to be told about a log that is empty.
wizard_detail_hint() {
  [ "$_WIZARD_DETAIL_HINTED" = 0 ] || return 0
  [ -s "$WIZARD_LOG_FILE" ] || return 0
  _WIZARD_DETAIL_HINTED=1
  wizard_say "technical details: $WIZARD_LOG_FILE"
  return 0
}

# wizard_ask <prompt> <default> -> $REPLY. Always 0.
wizard_ask() {
  local prompt="${1:-}" def="${2:-}"
  REPLY=""
  if [ "$WIZARD_INTERACTIVE" != 0 ]; then
    read -r -p "$prompt [${def:-empty}]: " REPLY || REPLY=""
  fi
  [ -n "$REPLY" ] || REPLY="$def"
  return 0
}

# wizard_confirm <prompt> <default> - status 0 for yes.
#
# Anything that is not a yes is a no, deliberately: this is asked before things
# that change the machine, and a typo must never be read as permission.
wizard_confirm() {
  wizard_ask "${1:-}" "${2:-n}"
  case "$REPLY" in
    y|Y|yes|YES|Yes) return 0 ;;
    *) return 1 ;;
  esac
}
