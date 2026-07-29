# The settings file: writing it once, and changing it every time after that.
#
#   . "$REPO/lib/envfile.sh"      (needs lib/ui.sh; python3 must be present)
#
# Sourcing is inert.
#
# ---------------------------------------------------------------- the contract
#
#   wizard_env_render <template> <dest> <KEY=VALUE>…
#         Write <dest> from <template>, substituting @PLACEHOLDER@ for each key
#         given. One pass over the text, so a value that happens to look like a
#         placeholder is written literally rather than expanded again. The file
#         is created 0600 whatever the ambient umask is, because the template
#         documents AYEAYE_TOKEN as a supported key and that is a credential.
#         0, or 1 with the destination untouched and no temporary file left.
#
#   wizard_env_merge <dest> <KEY=VALUE>…
#         Change only those keys, in place. Every other line of the file - the
#         other settings, the comments, a key this project has never heard of,
#         the order they are all in - comes through byte for byte. A key the
#         file does not have is appended; a key it has commented out is
#         uncommented rather than duplicated; a key it has twice is left with
#         one live value. 0, or 1 when there is no such file.
#
#   wizard_env_get <file> <key> [default]
#         The effective value: last assignment wins, surrounding quotes
#         stripped, a commented line is not an assignment. Always 0, so it is
#         safe to assign bare under `set -e`.
#
# Merging rather than rewriting is the whole point. A rerun of setup is not a
# reinstall: the person running it has had the file for months, and anything in
# it they were not asked about this time is theirs.
#
# ---------------------------------------------------------------------- how
#
# python3 does the text work. It is already a hard dependency of this project,
# it is the only thing here that can substitute a value containing an ampersand
# or a slash without an escaping dance, and a `sed -i` would not be portable
# between the two userlands this project supports anyway.
#
# Every write goes to a temporary file beside the destination and is renamed
# over it, so a failure leaves either the old file or the new one.
#
# bash 3.2: no associative arrays, no ${var,,}, no mapfile, no local -n.

# _wizard_env_python - the interpreter, or nothing and non-zero.
_wizard_env_python() {
  command -v python3 >/dev/null 2>&1 || return 1
  printf 'python3'
}

# wizard_env_render <template> <dest> <KEY=VALUE>…
wizard_env_render() {
  local template="${1:-}" dest="${2:-}" dir tmp status
  [ -n "$template" ] && [ -n "$dest" ] || return 2
  shift 2
  [ -f "$template" ] && [ -r "$template" ] || {
    wizard_say "the settings template is missing: $template"
    return 1
  }
  _wizard_env_python >/dev/null || {
    wizard_say "python3 is needed to write the settings file and is not here."
    return 1
  }

  dir="${dest%/*}"
  [ "$dir" = "$dest" ] || mkdir -p "$dir" 2>/dev/null || return 1
  tmp="$dest.tmp.$$"

  # The umask covers the moment of creation; the chmod covers a file the
  # rename replaces rather than creates.
  ( umask 077
    python3 -c '
import os, re, sys

template, dest = sys.argv[1], sys.argv[2]
# Each key is offered under two placeholder names, so the template may name a
# placeholder either after the variable it fills - @AYEAYE_PORT@ - or after the
# thing it is, @PORT@, which is what env.template does today.
values = {}
for pair in sys.argv[3:]:
    key, _, value = pair.partition("=")
    values["@" + key + "@"] = value
    for prefix in ("AYEAYE_", "VOICE_"):
        if key.startswith(prefix):
            values["@" + key[len(prefix):] + "@"] = value

text = open(template).read()

# One pass. Substituting the placeholders one after another over the whole
# text would make a value substituted early eligible for the substitutions
# that follow it, which is how an answer becomes an instruction.
def replace(match):
    return values.get(match.group(0), match.group(0))

sys.stdout.write(re.sub(r"@[A-Za-z0-9_]+@", replace, text))
' "$template" "$dest" "$@" > "$tmp"
  )
  status=$?
  if [ "$status" != 0 ]; then
    rm -f "$tmp" 2>/dev/null
    return 1
  fi
  chmod 600 "$tmp" 2>/dev/null
  mv -f "$tmp" "$dest" 2>/dev/null || {
    rm -f "$tmp" 2>/dev/null
    return 1
  }
  return 0
}

# wizard_env_merge <dest> <KEY=VALUE>…
wizard_env_merge() {
  local dest="${1:-}" tmp status
  [ -n "$dest" ] || return 2
  shift
  [ -f "$dest" ] || return 1
  [ "$#" -gt 0 ] || return 0
  _wizard_env_python >/dev/null || return 1

  tmp="$dest.tmp.$$"
  ( umask 077
    python3 -c '
import re, sys

dest = sys.argv[1]
wanted = []
seen = set()
for pair in sys.argv[2:]:
    key, _, value = pair.partition("=")
    if key and key not in seen:
        seen.add(key)
        wanted.append((key, value))

lines = open(dest).read().split("\n")
trailing_newline = lines and lines[-1] == ""
if trailing_newline:
    lines = lines[:-1]

written = set()
out = []
for line in lines:
    stripped = line.lstrip()
    # A commented-out assignment is the template documenting a default. Setting
    # it for the first time replaces that line, so the file never ends up with
    # a comment and a value that disagree.
    bare = stripped[1:].lstrip() if stripped.startswith("#") else stripped
    matched = None
    for key, value in wanted:
        if re.match(r"^" + re.escape(key) + r"\s*=", bare):
            matched = (key, value)
            break
    if matched is None:
        out.append(line)
        continue
    key, value = matched
    if key in written:
        # A key assigned twice keeps one live value; a second one would be
        # dead text that looks like it is doing something.
        continue
    written.add(key)
    out.append("%s=%s" % (key, value))

missing = [(k, v) for k, v in wanted if k not in written]
if missing:
    if out and out[-1].strip():
        out.append("")
    for key, value in missing:
        out.append("%s=%s" % (key, value))

text = "\n".join(out)
if trailing_newline or text:
    text += "\n"
sys.stdout.write(text)
' "$dest" "$@" > "$tmp"
  )
  status=$?
  if [ "$status" != 0 ]; then
    rm -f "$tmp" 2>/dev/null
    return 1
  fi
  chmod 600 "$tmp" 2>/dev/null
  mv -f "$tmp" "$dest" 2>/dev/null || {
    rm -f "$tmp" 2>/dev/null
    return 1
  }
  return 0
}

# wizard_env_get <file> <key> [default] - always 0.
#
# Written in bash rather than handed to python: it is called several times per
# run to build the summary and the service unit, and an interpreter start per
# value is a quarter of a second of somebody's life each time.
wizard_env_get() {
  local file="${1:-}" key="${2:-}" value="${3:-}" line="" candidate rest
  [ -n "$key" ] || { printf '%s' "$value"; return 0; }
  [ -f "$file" ] && [ -r "$file" ] || { printf '%s' "$value"; return 0; }
  while IFS= read -r line || [ -n "$line" ]; do
    line="${line%$'\r'}"
    # Leading blanks only; a leading # makes it documentation, not a setting.
    while :; do
      case "$line" in
        " "*|"	"*) line="${line#?}" ;;
        *) break ;;
      esac
    done
    case "$line" in
      "$key="*) candidate="${line#*=}" ;;
      *) continue ;;
    esac
    # Trailing blanks, then one layer of surrounding quotes.
    while :; do
      case "$candidate" in
        *" "|*"	") candidate="${candidate%?}" ;;
        *) break ;;
      esac
    done
    case "$candidate" in
      '"'*'"') rest="${candidate#\"}"; candidate="${rest%\"}" ;;
      "'"*"'") rest="${candidate#\'}"; candidate="${rest%\'}" ;;
    esac
    value="$candidate"
  done < "$file"
  printf '%s' "$value"
  return 0
}
