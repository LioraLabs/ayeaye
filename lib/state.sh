# What a setup run remembers between invocations.
#
#   . "$REPO/lib/state.sh"
#
# Sourcing is inert: it defines functions, gives the two tunables their
# defaults, and reads nothing. wizard_state_load is what touches the disk.
#
# This is the file that makes an interrupted run resumable. It is written by
# machine and read by machine; it is not configuration and nobody is expected
# to edit it. Configuration lives in the env file, which this store never
# touches.
#
# ------------------------------------------------------------------- the file
#
#   ${XDG_STATE_HOME:-~/.local/state}/ayeaye/setup-state, mode 0600.
#
# One record per line, "key=value". Keys match [A-Za-z0-9._-]+ and carry four
# namespaces, none of which the other three may write into:
#
#   run.<name>            metadata about the run itself: run.version,
#                         run.started, run.count, run.completed, run.repo
#   stage.<stage>         the lifecycle status of one stage
#   step.<stage>.<step>   the lifecycle status of one step inside it
#   answer.<name>         something the user chose, so a later run can offer it
#                         back as the default instead of asking blind
#
# Values are escaped on the way in - a backslash becomes two, a newline becomes
# backslash-n - so one record is always exactly one physical line and a value
# containing a newline still round-trips. Nothing else is transformed: a value
# with an "=" in it is fine, because only the first "=" of a line separates.
#
# ---------------------------------------------------------------- the contract
#
#   wizard_state_path                    where the file is. Always 0.
#   wizard_state_load                    read it into memory. Always 0, whether
#                                        or not the file exists or parses.
#   wizard_state_is_new                  status 0 when nothing has been stored
#   wizard_state_get <key> [default]     the value, or the default. Always 0,
#                                        so it is safe to assign bare under
#                                        `set -e`.
#   wizard_state_has <key>               status only
#   wizard_state_set <key> <value>       store it, now, atomically. 0 on
#                                        success, 1 when the disk refused, 2
#                                        when the key is malformed.
#   wizard_state_unset <key>             forget one key. 0, or 2 for a
#                                        malformed key.
#   wizard_state_unset_prefix <prefix>   forget every key starting with it.
#                                        Always 0. This is how a rerun drops
#                                        the previous run's stage statuses
#                                        while keeping its answers.
#   wizard_state_keys [prefix]           the keys, one per line. Always 0.
#   wizard_state_clear                   remove the file and the memory of it.
#
# Every write goes to a temporary file in the same directory and is renamed
# over the real one, so a run killed mid-write leaves either the old state or
# the new one, never half of either.
#
# ----------------------------------------------------------------- tunables
#
#   WIZARD_STATE_DIR   the directory. Overridable so a test can point the whole
#                      store somewhere it is allowed to write.
#   WIZARD_STATE_FILE  the file itself, if it should not be $WIZARD_STATE_DIR's
#                      default name.
#
# bash 3.2: no associative arrays, no ${var,,}, no mapfile, no local -n.

WIZARD_STATE_DIR="${WIZARD_STATE_DIR:-${XDG_STATE_HOME:-$HOME/.local/state}/ayeaye}"
WIZARD_STATE_FILE="${WIZARD_STATE_FILE:-$WIZARD_STATE_DIR/setup-state}"

# The whole store, in memory, as "key=escaped-value" lines with a trailing
# newline. Empty means "nothing stored", which is also what an absent file
# means, and the two are deliberately indistinguishable to a reader.
_WIZARD_STATE="${_WIZARD_STATE:-}"
_WIZARD_STATE_LOADED="${_WIZARD_STATE_LOADED:-0}"
_WIZARD_STATE_S="${_WIZARD_STATE_S:-}"

_WIZARD_STATE_NL='
'

# --------------------------------------------------------------- escaping
#
# Answered in $_WIZARD_STATE_S rather than on stdout: these run once per key
# per write, and a command substitution per key would be a fork per key.

# _wizard_state_escape <value> -> one line, safe to store.
_wizard_state_escape() {
  local s="$1" out=""
  while :; do
    case "$s" in
      *\\*) out="$out${s%%\\*}\\\\"; s="${s#*\\}" ;;
      *)    out="$out$s"; break ;;
    esac
  done
  s="$out"; out=""
  while :; do
    case "$s" in
      *"$_WIZARD_STATE_NL"*)
        out="$out${s%%"$_WIZARD_STATE_NL"*}\\n"
        s="${s#*"$_WIZARD_STATE_NL"}"
        ;;
      *) out="$out$s"; break ;;
    esac
  done
  _WIZARD_STATE_S="$out"
}

# _wizard_state_unescape <stored> -> the original value.
#
# One left-to-right pass, so an escaped backslash cannot be re-read as the
# start of the escape that follows it: "\\n" is a backslash then an n, and
# not a newline.
_wizard_state_unescape() {
  local s="$1" out="" head
  while :; do
    case "$s" in
      *\\*)
        head="${s%%\\*}"
        s="${s#*\\}"
        out="$out$head"
        case "$s" in
          n*)  out="$out$_WIZARD_STATE_NL"; s="${s#?}" ;;
          \\*) out="$out\\";               s="${s#?}" ;;
          # A lone trailing backslash was not written by this library, but
          # keeping it beats losing it.
          *)   out="$out\\" ;;
        esac
        ;;
      *) out="$out$s"; break ;;
    esac
  done
  _WIZARD_STATE_S="$out"
}

# _wizard_state_valid_key <key> - status only.
_wizard_state_valid_key() {
  case "${1:-}" in
    "") return 1 ;;
    *[!A-Za-z0-9._-]*) return 1 ;;
    *) return 0 ;;
  esac
}

# -------------------------------------------------------------------- reading

wizard_state_path() { printf '%s' "$WIZARD_STATE_FILE"; }

# wizard_state_load - read the file, keeping only the lines that are records.
# A file that is missing, unreadable or full of rubbish all mean the same
# thing to a caller: whatever could be read, was.
wizard_state_load() {
  local line=""
  _WIZARD_STATE=""
  _WIZARD_STATE_LOADED=1
  [ -f "$WIZARD_STATE_FILE" ] && [ -r "$WIZARD_STATE_FILE" ] || return 0
  while IFS= read -r line || [ -n "$line" ]; do
    case "$line" in
      *=*) ;;
      *) continue ;;
    esac
    _wizard_state_valid_key "${line%%=*}" || continue
    _WIZARD_STATE="$_WIZARD_STATE$line$_WIZARD_STATE_NL"
  done < "$WIZARD_STATE_FILE"
  return 0
}

_wizard_state_ensure_loaded() {
  [ "$_WIZARD_STATE_LOADED" = 1 ] || wizard_state_load
}

# wizard_state_is_new - status 0 when nothing has been stored yet.
wizard_state_is_new() {
  _wizard_state_ensure_loaded
  [ -z "$_WIZARD_STATE" ]
}

# _wizard_state_raw <key> -> the stored, still-escaped value in
# $_WIZARD_STATE_S. Status 1 when the key is absent.
_wizard_state_raw() {
  local key="$1" line=""
  _wizard_state_ensure_loaded
  _WIZARD_STATE_S=""
  while IFS= read -r line || [ -n "$line" ]; do
    case "$line" in
      "$key="*) _WIZARD_STATE_S="${line#*=}"; return 0 ;;
    esac
  done <<EOF
$_WIZARD_STATE
EOF
  return 1
}

# wizard_state_has <key> - status only.
wizard_state_has() {
  _wizard_state_valid_key "${1:-}" || return 1
  _wizard_state_raw "$1"
}

# wizard_state_get <key> [default] - always succeeds.
wizard_state_get() {
  local key="${1:-}" fallback="${2:-}"
  if ! _wizard_state_valid_key "$key" || ! _wizard_state_raw "$key"; then
    printf '%s' "$fallback"
    return 0
  fi
  _wizard_state_unescape "$_WIZARD_STATE_S"
  printf '%s' "$_WIZARD_STATE_S"
  return 0
}

# wizard_state_keys [prefix] - always succeeds.
wizard_state_keys() {
  local prefix="${1:-}" line="" key
  _wizard_state_ensure_loaded
  while IFS= read -r line || [ -n "$line" ]; do
    [ -n "$line" ] || continue
    key="${line%%=*}"
    case "$key" in
      "$prefix"*) printf '%s\n' "$key" ;;
    esac
  done <<EOF
$_WIZARD_STATE
EOF
  return 0
}

# -------------------------------------------------------------------- writing

# _wizard_state_flush - put $_WIZARD_STATE on disk, atomically and privately.
_wizard_state_flush() {
  local tmp
  mkdir -p "$WIZARD_STATE_DIR" 2>/dev/null || return 1
  tmp="$WIZARD_STATE_FILE.tmp.$$"
  # The umask covers the window between creation and chmod, and the chmod
  # covers a file the rename replaced rather than created.
  ( umask 077; printf '%s' "$_WIZARD_STATE" > "$tmp" ) 2>/dev/null || {
    rm -f "$tmp" 2>/dev/null
    return 1
  }
  chmod 600 "$tmp" 2>/dev/null
  mv -f "$tmp" "$WIZARD_STATE_FILE" 2>/dev/null || {
    rm -f "$tmp" 2>/dev/null
    return 1
  }
  return 0
}

# _wizard_state_drop <key> - remove it from memory. No disk write.
_wizard_state_drop() {
  local key="$1" line="" out=""
  while IFS= read -r line || [ -n "$line" ]; do
    [ -n "$line" ] || continue
    case "$line" in
      "$key="*) continue ;;
    esac
    out="$out$line$_WIZARD_STATE_NL"
  done <<EOF
$_WIZARD_STATE
EOF
  _WIZARD_STATE="$out"
}

# wizard_state_set <key> <value>
wizard_state_set() {
  local key="${1:-}" value="${2:-}"
  _wizard_state_valid_key "$key" || return 2
  _wizard_state_ensure_loaded
  _wizard_state_drop "$key"
  _wizard_state_escape "$value"
  _WIZARD_STATE="$_WIZARD_STATE$key=$_WIZARD_STATE_S$_WIZARD_STATE_NL"
  _wizard_state_flush
}

# wizard_state_unset <key>
wizard_state_unset() {
  local key="${1:-}"
  _wizard_state_valid_key "$key" || return 2
  _wizard_state_ensure_loaded
  wizard_state_has "$key" || return 0
  _wizard_state_drop "$key"
  _wizard_state_flush
}

# wizard_state_unset_prefix <prefix> - always 0. An empty prefix would match
# every key, which is what wizard_state_clear is for, so it is refused here to
# make the difference deliberate.
wizard_state_unset_prefix() {
  local prefix="${1:-}" line="" out="" changed=0
  [ -n "$prefix" ] || return 0
  _wizard_state_ensure_loaded
  while IFS= read -r line || [ -n "$line" ]; do
    [ -n "$line" ] || continue
    case "${line%%=*}" in
      "$prefix"*) changed=1; continue ;;
    esac
    out="$out$line$_WIZARD_STATE_NL"
  done <<EOF
$_WIZARD_STATE
EOF
  [ "$changed" = 1 ] || return 0
  _WIZARD_STATE="$out"
  _wizard_state_flush || return 0
  return 0
}

# wizard_state_clear - always 0. Nothing to remove is not a failure.
wizard_state_clear() {
  _WIZARD_STATE=""
  _WIZARD_STATE_LOADED=1
  rm -f "$WIZARD_STATE_FILE" 2>/dev/null
  return 0
}
