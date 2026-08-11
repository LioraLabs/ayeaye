#!/usr/bin/env bash
# Is the source tarball on disk something a stranger can actually unpack and
# trust? The installer no longer unpacks it - it fetches a binary (AYEAYE-63)
# - but the tarball is still a published, checksummed release artifact, and
# one that lies about its shape or its source commit is one nobody notices
# until they are debugging something else entirely.
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
# check would have reported - writing it the wrong way here failed with a
# bare 141.
listing="$(tar -tzf "$artifact")"

# has <pattern> - true when the listing contains it, without a pipe that can
# die early.
has() { case "$(printf '%s\n' "$listing" | grep -e "$1" || true)" in "") return 1 ;; *) return 0 ;; esac; }

# 1. Nothing escaping the directory it unpacks into: a tarball that writes
#    outside itself is dead on arrival wherever it lands.
escaping="$(printf '%s\n' "$listing" | grep -e '^/' -e '^\.\./' -e '/\.\./' || true)"
[ -z "$escaping" ]; check "no path escapes the unpack directory" $?

# 2. Exactly one top-level directory, so an unpack puts the tree in one
#    self-describing place rather than spraying it over the cwd.
tops="$(printf '%s\n' "$listing" | awk -F/ 'NF>1 {print $1}' | sort -u | wc -l | tr -d ' ')"
[ "$tops" = 1 ]; check "one top-level directory (found $tops)" $?

# 3. The download front door and the Rust product are in the source archive.
has "/install.sh$";  check "contains install.sh" $?
has "/crates/ayeaye/Cargo.toml$"; check "contains the ayeaye crate" $?

# 4. The installer inside parses. It pins no version of its own any more -
#    it is a downloader (AYEAYE-63) - so parseability is the whole claim.
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
tar -xzf "$artifact" -C "$work"
inner="$(find "$work" -maxdepth 2 -name install.sh -print -quit)"
[ -n "$inner" ]; check "install.sh is where the unpack looks for it" $?
if [ -n "$inner" ]; then
  sh -n "$inner"; check "the installer inside parses" $?
fi

# 5. Reproducible. Build it again and compare: if the same commit does not
#    produce the same bytes, the checksum in install.sh is a number nobody can
#    re-derive, and verification is theatre.
#
#    From the commit the tarball itself names, not from HEAD: git archive
#    writes its source commit into the tar, so the artifact carries the only
#    answer to "the same as what?" that stays right after HEAD moves on -
#    which it does, by design, the moment the stamp is committed.
src="$(gzip -dc "$artifact" | git -C "$root" get-tar-commit-id || true)"
[ -n "$src" ]; check "the tarball names the commit it was built from" $?
if [ -n "$src" ]; then
  again="$work/again.tar.gz"
  bash "$root/scripts/release-archive.sh" "$again" "$src" >/dev/null 2>&1
  [ "$(sha256sum < "$artifact" | cut -d' ' -f1)" = "$(sha256sum < "$again" | cut -d' ' -f1)" ]
  check "byte-identical when rebuilt from $(git -C "$root" rev-parse --short "$src")" $?
fi

[ "$fail" = 0 ] || exit 1
printf 'the artifact is installable: %s\n' "$artifact"
