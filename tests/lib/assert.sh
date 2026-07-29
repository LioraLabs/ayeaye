# Assertion helpers. Sourced by tests/lib/runner.sh; never run directly.
#
# Every assertion that fails prints the assertion name, the source location of
# the call, and the expected and actual values, then ends the test. Because a
# test function is the only thing running in its process, ending the test is a
# plain `exit`.
#
# bash 3.2: no associative arrays, no ${var,,}, no mapfile.

TEST_EXIT_FAIL=1
TEST_EXIT_SKIP=77

_ASSERT_NL='
'

# Print "label: value" for a one-line value, or an indented block for a value
# that is empty or spans several lines. Nothing is ever truncated.
_assert_show() {
  local label="$1" value="$2"
  case "$value" in
    *"$_ASSERT_NL"*)
      printf '  %s: |\n' "$label"
      printf '%s\n' "$value" | sed 's/^/    /'
      ;;
    "")
      printf '  %s: <empty>\n' "$label"
      ;;
    *)
      printf '  %s: %s\n' "$label" "$value"
      ;;
  esac
}

# _assert_fail <location> <assertion-name> <message> [label value]...
_assert_fail() {
  local loc="$1" name="$2" msg="$3"
  shift 3
  {
    printf '%s: %s failed' "$loc" "$name"
    [ -n "$msg" ] && printf ': %s' "$msg"
    printf '\n'
    while [ "$#" -ge 2 ]; do
      _assert_show "$1" "$2"
      shift 2
    done
  } >&2
  exit "$TEST_EXIT_FAIL"
}

# fail "message" - end the current test as a failure.
fail() {
  _assert_fail "${BASH_SOURCE[1]}:${BASH_LINENO[0]}" "fail" "$*"
}

# skip "reason" - end the current test as skipped, not failed. Use for
# coverage that only makes sense on another platform or inside a container.
skip() {
  printf 'skip: %s\n' "$*" >&2
  exit "$TEST_EXIT_SKIP"
}

# assert_eq <expected> <actual> [message]
assert_eq() {
  [ "$1" = "$2" ] && return 0
  _assert_fail "${BASH_SOURCE[1]}:${BASH_LINENO[0]}" "assert_eq" "${3:-}" \
    "expected" "$1" "actual" "$2"
}

# assert_ne <not-expected> <actual> [message]
assert_ne() {
  [ "$1" != "$2" ] && return 0
  _assert_fail "${BASH_SOURCE[1]}:${BASH_LINENO[0]}" "assert_ne" "${3:-}" \
    "expected anything but" "$1" "actual" "$2"
}

# assert_contains <haystack> <needle> [message]
assert_contains() {
  case "$1" in
    *"$2"*) return 0 ;;
  esac
  _assert_fail "${BASH_SOURCE[1]}:${BASH_LINENO[0]}" "assert_contains" "${3:-}" \
    "expected to contain" "$2" "actual" "$1"
}

# assert_not_contains <haystack> <needle> [message]
assert_not_contains() {
  case "$1" in
    *"$2"*)
      _assert_fail "${BASH_SOURCE[1]}:${BASH_LINENO[0]}" "assert_not_contains" "${3:-}" \
        "expected not to contain" "$2" "actual" "$1"
      ;;
  esac
  return 0
}

# assert_matches <string> <extended-regex> [message]
assert_matches() {
  printf '%s' "$1" | grep -Eq -- "$2" && return 0
  _assert_fail "${BASH_SOURCE[1]}:${BASH_LINENO[0]}" "assert_matches" "${3:-}" \
    "expected to match" "$2" "actual" "$1"
}

# assert_status <expected-code> <actual-code> [message]
assert_status() {
  [ "$1" = "$2" ] && return 0
  _assert_fail "${BASH_SOURCE[1]}:${BASH_LINENO[0]}" "assert_status" "${3:-}" \
    "expected exit status" "$1" "actual exit status" "$2"
}

# assert_file_exists <path> [message]
assert_file_exists() {
  [ -e "$1" ] && return 0
  _assert_fail "${BASH_SOURCE[1]}:${BASH_LINENO[0]}" "assert_file_exists" "${2:-}" \
    "expected to exist" "$1" "parent listing" "$(ls -A "$(dirname "$1")" 2>&1)"
}

# assert_file_missing <path> [message]
assert_file_missing() {
  [ -e "$1" ] || return 0
  _assert_fail "${BASH_SOURCE[1]}:${BASH_LINENO[0]}" "assert_file_missing" "${2:-}" \
    "expected not to exist" "$1" "contents" "$(cat "$1" 2>&1)"
}

# assert_file_contains <path> <needle> [message]
assert_file_contains() {
  local body
  if [ ! -f "$1" ]; then
    _assert_fail "${BASH_SOURCE[1]}:${BASH_LINENO[0]}" "assert_file_contains" "${3:-}" \
      "expected file" "$1" "actual" "no such file"
  fi
  body="$(cat "$1")"
  case "$body" in
    *"$2"*) return 0 ;;
  esac
  _assert_fail "${BASH_SOURCE[1]}:${BASH_LINENO[0]}" "assert_file_contains" "${3:-}" \
    "expected $1 to contain" "$2" "actual" "$body"
}

# assert_file_not_contains <path> <needle> [message]
assert_file_not_contains() {
  local body
  [ -f "$1" ] || return 0
  body="$(cat "$1")"
  case "$body" in
    *"$2"*)
      _assert_fail "${BASH_SOURCE[1]}:${BASH_LINENO[0]}" "assert_file_not_contains" "${3:-}" \
        "expected $1 not to contain" "$2" "actual" "$body"
      ;;
  esac
  return 0
}

# assert_file_mode <expected-octal> <path> [message]
# Uses the portable `ls -l` symbolic mode: BSD stat and GNU stat disagree on
# flags, ls does not.
assert_file_mode() {
  local want="$1" path="$2" sym got
  if [ ! -e "$path" ]; then
    _assert_fail "${BASH_SOURCE[1]}:${BASH_LINENO[0]}" "assert_file_mode" "${3:-}" \
      "expected file" "$path" "actual" "no such file"
  fi
  sym="$(ls -ld "$path" | cut -c2-10)"
  got="$(_assert_symbolic_to_octal "$sym")"
  [ "$want" = "$got" ] && return 0
  _assert_fail "${BASH_SOURCE[1]}:${BASH_LINENO[0]}" "assert_file_mode" "${3:-}" \
    "expected mode of $path" "$want" "actual" "$got ($sym)"
}

_assert_symbolic_to_octal() {
  local sym="$1" out="" i part n c
  i=1
  while [ "$i" -le 7 ]; do
    part="$(printf '%s' "$sym" | cut -c"$i"-"$((i + 2))")"
    n=0
    c="$(printf '%s' "$part" | cut -c1)"; [ "$c" = "r" ] && n=$((n + 4))
    c="$(printf '%s' "$part" | cut -c2)"; [ "$c" = "w" ] && n=$((n + 2))
    c="$(printf '%s' "$part" | cut -c3)"; case "$c" in x|s|t) n=$((n + 1)) ;; esac
    out="$out$n"
    i=$((i + 3))
  done
  printf '%s' "$out"
}
