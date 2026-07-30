#!/usr/bin/env bash
# Is the thing on disk something the bootstrap can actually install?
#
# Every assertion here mirrors a refusal in install.sh's _bootstrap_unpack. A
# tarball that fails one of these is a tarball that 404s past verification and
# then dies at the unpack, on somebody else's machine, with the network already
# paid for.
set -euo pipefail

artifact="${1:?usage: check-release-artifact.sh <artifact.tar.gz>}"
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
version="$(bash "$root/scripts/release-version.sh")"
fail=0

check() {  # check <what> <status>
  if [ "$2" = 0 ]; then printf '  ok       %s\n' "$1"
  else printf '  FAIL     %s\n' "$1"; fail=1; fi
}

[ -f "$artifact" ] || { echo "no artifact at $artifact" >&2; exit 1; }

# Listed once, into a variable. A `tar | grep -q` stops at the first match and
# kills tar with a broken pipe, which under `set -o pipefail` is the status the
# check would have reported - install.sh's own unpack carries a comment about
# exactly this, and writing it the wrong way here failed with a bare 141.
listing="$(tar -tzf "$artifact")"

# has <pattern> - true when the listing contains it, without a pipe that can
# die early.
has() { case "$(printf '%s\n' "$listing" | grep -e "$1" || true)" in "") return 1 ;; *) return 0 ;; esac; }

# 1. Nothing escaping the directory it unpacks into. install.sh refuses these
#    outright, so a release containing one is dead on arrival.
escaping="$(printf '%s\n' "$listing" | grep -e '^/' -e '^\.\./' -e '/\.\./' || true)"
[ -z "$escaping" ]; check "no path escapes the unpack directory" $?

# 2. Exactly one top-level directory. With more than one the bootstrap treats
#    the scratch directory itself as the payload, and the layout check below
#    is what then fails - confusingly, and one step too late.
tops="$(printf '%s\n' "$listing" | awk -F/ 'NF>1 {print $1}' | sort -u | wc -l | tr -d ' ')"
[ "$tops" = 1 ]; check "one top-level directory (found $tops)" $?

# 3. The two files the bootstrap demands before it will hand over to a payload.
has "/install.sh$";  check "contains install.sh" $?
has "/bin/ayeaye$";  check "contains bin/ayeaye" $?

# 4. The installer inside agrees about which release this is. A tarball whose
#    install.sh disagrees with the tag it hangs under is the sort of thing
#    nobody notices until they are debugging something else entirely.
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
tar -xzf "$artifact" -C "$work"
inner="$(find "$work" -maxdepth 2 -name install.sh -print -quit)"
[ -n "$inner" ]; check "install.sh is where the unpack looks for it" $?
if [ -n "$inner" ]; then
  inner_version="$(sed -n 's/^AYEAYE_VERSION="\([^"]*\)".*$/\1/p' "$inner" | head -1)"
  [ "$inner_version" = "$version" ]
  check "the installer inside says $version (says: ${inner_version:-nothing})" $?
  bash -n "$inner"; check "the installer inside parses" $?
fi

# 5. Reproducible. Build it again and compare: if the same commit does not
#    produce the same bytes, the checksum in install.sh is a number nobody can
#    re-derive, and verification is theatre.
again="$work/again.tar.gz"
bash "$root/scripts/release-archive.sh" "$again" >/dev/null 2>&1
[ "$(sha256sum < "$artifact" | cut -d' ' -f1)" = "$(sha256sum < "$again" | cut -d' ' -f1)" ]
check "byte-identical when built a second time" $?

[ "$fail" = 0 ] || exit 1
printf 'the artifact is installable: %s\n' "$artifact"
