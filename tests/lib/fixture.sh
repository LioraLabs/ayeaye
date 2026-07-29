# Fixtures: canned bytes that stand in for what a real command or system file
# would produce.
#
# Layout is tests/fixtures/<category>/<name>, where the category is the command
# or file being canned and the name describes the scenario:
#
#   tests/fixtures/os-release/ubuntu-24.04
#   tests/fixtures/uname/darwin-arm64
#   tests/fixtures/tailscale/status-online
#
# Names carry no extension on purpose - the repository's .gitignore drops
# *.log and a file called token, and extensionless names sidestep all of it.
#
# bash 3.2: no associative arrays, no ${var,,}, no mapfile.

# fixture_path <category/name> - absolute path of a fixture.
#
# A missing fixture is loud in two ways: it complains on stderr, which lands in
# the failing test's output, and it still prints a path, one that cannot exist,
# so whatever consumes it fails with the name in the message. It cannot end a
# test on its own, because it is normally called inside a command substitution
# and an exit there would only leave the subshell.
fixture_path() {
  local rel="$1" path="$FIXTURES_DIR/$1"
  if [ ! -f "$path" ]; then
    {
      printf 'fixture: no such fixture: %s\n' "$rel"
      printf 'fixture: looked for %s\n' "$path"
      printf 'fixture: available in that category:\n'
      ls "$FIXTURES_DIR/${rel%%/*}" 2>/dev/null | sed 's/^/  /'
    } >&2
    printf '%s' "$FIXTURES_DIR/MISSING-FIXTURE/$rel"
    return 1
  fi
  printf '%s' "$path"
}

# fixture_cat <category/name> - the fixture's bytes on stdout.
fixture_cat() {
  local path
  path="$(fixture_path "$1")" || return 1
  cat "$path"
}

# fixture_file <category/name> [destination] - copy a fixture into the sandbox
# and print where it landed. This is how file-shaped input such as
# /etc/os-release reaches code that takes its input path from an overridable
# variable:
#
#   OS_RELEASE_FILE="$(fixture_file os-release/ubuntu-24.04)" run_script ...
fixture_file() {
  local rel="$1" dest="${2:-}" src
  src="$(fixture_path "$rel")" || return 1
  if [ -z "$dest" ]; then
    mkdir -p "$TEST_TMPDIR/fixtures/${rel%/*}"
    dest="$TEST_TMPDIR/fixtures/$rel"
  else
    mkdir -p "$(dirname "$dest")"
  fi
  cp "$src" "$dest" || return 1
  printf '%s' "$dest"
}

# fixture_list <category> - the scenarios available in a category, one per
# line. Useful for a test that should cover every distro fixture there is.
fixture_list() {
  ls "$FIXTURES_DIR/$1" 2>/dev/null
}

# assert_fixture_exists <category/name> [message]
assert_fixture_exists() {
  [ -f "$FIXTURES_DIR/$1" ] && return 0
  _assert_fail "${BASH_SOURCE[1]}:${BASH_LINENO[0]}" "assert_fixture_exists" "${2:-}" \
    "expected fixture" "$1" "looked in" "$FIXTURES_DIR"
}
