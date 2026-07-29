# Command stubs: make an external command resolve to a recording shim.
#
# Two guarantees make detection testable:
#
#   1. PATH is exactly $STUB_BIN. A command that was not stubbed, and was not
#      explicitly linked in with stub_real, is genuinely absent - so a test
#      that asserts "tailscale is not installed" gives the same answer on a
#      laptop that has it and in a bare container.
#   2. Every stub invocation appends one line to $STUB_LOG_DIR/<name>.log:
#      the command name followed by its argv. That recording is how the
#      commands an installer *would* run get asserted on without running them.
#
# bash 3.2: no associative arrays, no ${var,,}, no mapfile.

# Linked in by default because the harness, and any shell script under test,
# would not work without them. Deliberately absent: tmux, python3-adjacent
# tooling, uname, systemctl, tailscale, package managers - anything a
# detection step is supposed to probe must be stubbed or linked on purpose.
STUB_CORE_COMMANDS="bash sh env sed awk grep cat cut tr head tail sort uniq wc
find ls mkdir rmdir rm mv cp ln chmod touch dirname basename mktemp date sleep
cksum id whoami printf test true false expr"

# Called by harness_begin once the sandbox exists.
stub_begin() {
  HARNESS_HOST_PATH="$PATH"
  export HARNESS_HOST_PATH
  stub_real $STUB_CORE_COMMANDS
  PATH="$STUB_BIN"
  export PATH
}

# stub_real <command>... - link the host's real binary into $STUB_BIN, for
# tools the code under test legitimately needs (python3 in this repo) or that
# a test wants to behave for real. Silently skips what the host does not have.
stub_real() {
  local name real
  for name in "$@"; do
    real="$(PATH="$HARNESS_HOST_PATH" command -v "$name" 2>/dev/null)" || continue
    [ -n "$real" ] || continue
    [ -e "$STUB_BIN/$name" ] && continue
    ln -s "$real" "$STUB_BIN/$name" 2>/dev/null || true
  done
}

# stub_remove <command>... - make a command absent again, whether it was a
# stub or a real one linked in by default.
stub_remove() {
  local name
  for name in "$@"; do
    rm -f "$STUB_BIN/$name"
  done
}

_stub_tab="$(printf '\t')"

# _stub_payload <name> <slot> <text> - park text in a file and echo its path,
# or echo "-" for no output at all.
_stub_payload() {
  local name="$1" slot="$2" text="$3" file
  [ -n "$text" ] || { printf '%s' "-"; return 0; }
  file="$STUB_LOG_DIR/$name.$slot.$$.$RANDOM"
  printf '%s' "$text" > "$file"
  printf '%s' "$file"
}

# Write the shim itself. Rules are consulted at call time, so stub_when can
# add to a stub that already exists.
_stub_write() {
  # Separate declarations on purpose: `local a="$1" b="$a"` expands $a before
  # the assignment to a has happened, and would silently pick up whatever a
  # meant in the calling function.
  local name="$1"
  local path="$STUB_BIN/$name"
  cat > "$path" <<EOF
#!/usr/bin/env bash
# Generated command stub for "$name". Recording shim, not a real command.
_name='$name'
_log='$STUB_LOG_DIR/$name.log'
_rules='$STUB_LOG_DIR/$name.rules'

_line="\$_name"
for _arg in "\$@"; do
  case "\$_arg" in
    ''|*[![:print:]]*|*[[:space:]]*) _line="\$_line '\$_arg'" ;;
    *)                               _line="\$_line \$_arg" ;;
  esac
done
printf '%s\n' "\$_line" >> "\$_log"

_status=0
_out='-'
_err='-'
if [ -f "\$_rules" ]; then
  while IFS='$_stub_tab' read -r _pat _rstatus _rout _rerr; do
    [ -n "\$_pat" ] || continue
    case "\$*" in
      \$_pat)
        _status="\$_rstatus"
        _out="\$_rout"
        _err="\$_rerr"
        break
        ;;
    esac
  done < "\$_rules"
fi

[ "\$_out" = '-' ] || cat "\$_out"
[ "\$_err" = '-' ] || cat "\$_err" >&2
exit "\$_status"
EOF
  chmod +x "$path"
  : > "$STUB_LOG_DIR/$name.log"
  [ -f "$STUB_LOG_DIR/$name.rules" ] || : > "$STUB_LOG_DIR/$name.rules"
}

# _stub_rule <name> <pattern> <args...> - append a match rule.
# Options: --exit N --stdout TEXT --stderr TEXT --stdout-fixture REL
_stub_rule() {
  local name="$1" pattern="$2"
  shift 2
  local status=0 out="" err="" outfile="-" errfile="-"
  while [ "$#" -gt 0 ]; do
    case "$1" in
      --exit)           status="$2"; shift 2 ;;
      --stdout)         out="$2"; shift 2 ;;
      --stderr)         err="$2"; shift 2 ;;
      --stdout-fixture) outfile="$(fixture_path "$2")"; shift 2 ;;
      --stderr-fixture) errfile="$(fixture_path "$2")"; shift 2 ;;
      *) fail "stub: unknown option $1" ;;
    esac
  done
  [ "$outfile" = "-" ] && outfile="$(_stub_payload "$name" out "$out")"
  [ "$errfile" = "-" ] && errfile="$(_stub_payload "$name" err "$err")"
  printf '%s\t%s\t%s\t%s\n' "$pattern" "$status" "$outfile" "$errfile" \
    >> "$STUB_LOG_DIR/$name.rules"
}

# stub_command <name> [--exit N] [--stdout TEXT] [--stderr TEXT]
#                     [--stdout-fixture REL] [--stderr-fixture REL]
#
# Create (or reset) a stub and set its default response. Any earlier rules for
# the name are dropped, so a test can always start from a clean slate.
stub_command() {
  local name="$1"
  shift
  rm -f "$STUB_LOG_DIR/$name.rules"
  _stub_write "$name"
  _stub_rule "$name" '*' "$@"
}

# stub_command_from_fixture <name> <fixture> [--exit N]
# Convenience for the common case: the command prints canned output.
stub_command_from_fixture() {
  local name="$1" fixture="$2"
  shift 2
  stub_command "$name" --stdout-fixture "$fixture" "$@"
}

# stub_when <name> <argv-glob> [--exit N] [--stdout TEXT] ...
#
# Add a rule matched against the joined argv ("$*") before the default. Rules
# are tried in the order they were added, so
#
#   stub_command systemctl --exit 1
#   stub_when systemctl '--user show-environment' --exit 0
#
# gives a systemctl where only that one subcommand succeeds. The glob is a
# shell pattern: 'enable --now *' matches a trailing unit name.
stub_when() {
  local name="$1" pattern="$2"
  shift 2
  [ -x "$STUB_BIN/$name" ] || _stub_write "$name"
  # New rules go before the catch-all default so that ordering reads the way
  # it was written.
  local rules="$STUB_LOG_DIR/$name.rules" tmp="$STUB_LOG_DIR/$name.rules.tmp"
  local default_line=""
  if [ -s "$rules" ]; then
    default_line="$(grep "^\\*$_stub_tab" "$rules" | tail -1)"
    grep -v "^\\*$_stub_tab" "$rules" > "$tmp" 2>/dev/null || : > "$tmp"
    mv "$tmp" "$rules"
  fi
  _stub_rule "$name" "$pattern" "$@"
  [ -n "$default_line" ] && printf '%s\n' "$default_line" >> "$rules"
  return 0
}

# stub_script <name> - create a stub whose behaviour is arbitrary bash read
# from stdin. The recording line is still written before the body runs, and
# the body's own exit status is the stub's.
#
#   stub_script tailscale <<'SH'
#   [ "$1" = status ] && cat "$FIXTURES_DIR/tailscale/status-online"
#   SH
stub_script() {
  local name="$1"
  local path="$STUB_BIN/$name"
  local body="$STUB_LOG_DIR/$name.body"
  cat > "$body"
  cat > "$path" <<EOF
#!/usr/bin/env bash
# Generated command stub for "$name". Recording shim, not a real command.
_name='$name'
_line="\$_name"
for _arg in "\$@"; do
  case "\$_arg" in
    ''|*[![:print:]]*|*[[:space:]]*) _line="\$_line '\$_arg'" ;;
    *)                               _line="\$_line \$_arg" ;;
  esac
done
printf '%s\n' "\$_line" >> '$STUB_LOG_DIR/$name.log'
. '$body'
EOF
  chmod +x "$path"
  : > "$STUB_LOG_DIR/$name.log"
}

# ------------------------------------------------------------------ queries

# stub_calls <name> - every recorded invocation, one per line, in order.
stub_calls() {
  [ -f "$STUB_LOG_DIR/$1.log" ] || return 0
  cat "$STUB_LOG_DIR/$1.log"
}

# stub_call_count <name>
stub_call_count() {
  local count
  if [ -f "$STUB_LOG_DIR/$1.log" ]; then
    # grep -c prints 0 and exits 1 on no match, so take the value, not the
    # status, or an empty log counts as "0\n0".
    count="$(grep -c . "$STUB_LOG_DIR/$1.log" 2>/dev/null)"
    printf '%s' "${count:-0}"
  else
    printf '0'
  fi
}

# stub_call <name> <n> - the nth recorded invocation, 1-based.
stub_call() {
  [ -f "$STUB_LOG_DIR/$1.log" ] || return 0
  sed -n "$2p" "$STUB_LOG_DIR/$1.log"
}

# ---------------------------------------------------------------- assertions

assert_stub_called() {
  local count
  count="$(stub_call_count "$1")"
  [ "${count:-0}" -gt 0 ] && return 0
  _assert_fail "${BASH_SOURCE[1]}:${BASH_LINENO[0]}" "assert_stub_called" "${2:-}" \
    "expected at least one call to" "$1" "recorded calls" "$(stub_calls "$1")"
}

assert_stub_not_called() {
  local count
  count="$(stub_call_count "$1")"
  [ "${count:-0}" = 0 ] && return 0
  _assert_fail "${BASH_SOURCE[1]}:${BASH_LINENO[0]}" "assert_stub_not_called" "${2:-}" \
    "expected no calls to" "$1" "recorded calls" "$(stub_calls "$1")"
}

# assert_stub_called_with <name> <argv-substring> [message]
assert_stub_called_with() {
  local calls
  calls="$(stub_calls "$1")"
  case "$calls" in
    *"$2"*) return 0 ;;
  esac
  _assert_fail "${BASH_SOURCE[1]}:${BASH_LINENO[0]}" "assert_stub_called_with" "${3:-}" \
    "expected a call to $1 containing" "$2" "recorded calls" "$calls"
}

# assert_stub_call_count <name> <expected> [message]
assert_stub_call_count() {
  local count
  count="$(stub_call_count "$1")"
  [ "$count" = "$2" ] && return 0
  _assert_fail "${BASH_SOURCE[1]}:${BASH_LINENO[0]}" "assert_stub_call_count" "${3:-}" \
    "expected calls to $1" "$2" "actual" "$count ($(stub_calls "$1"))"
}

# assert_command_absent <name> [message] - the guarantee detection tests lean
# on: this command cannot be found no matter what the host has installed.
assert_command_absent() {
  command -v "$1" >/dev/null 2>&1 || return 0
  _assert_fail "${BASH_SOURCE[1]}:${BASH_LINENO[0]}" "assert_command_absent" "${2:-}" \
    "expected not to be on PATH" "$1" "resolved to" "$(command -v "$1")"
}
