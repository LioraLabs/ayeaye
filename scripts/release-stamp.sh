#!/usr/bin/env bash
# Write the artifact's checksum into install.sh.
#
# The ordering here is worth understanding before running it, because it looks
# circular and is not:
#
#   install.sh is inside the tarball, so baking a sum for the tarball into
#   install.sh would change the tarball and invalidate the sum.
#
# It resolves because the two copies play different parts. The one a stranger
# runs comes from raw.githubusercontent.com on main - stamped, and doing the
# verifying. The one inside the tarball never bootstraps: it has a payload
# already, so it goes straight to setup and its own AYEAYE_SHA256 is never
# read. So the tag points at the commit the artifact was built from, and the
# stamped commit lands on main after it.
set -euo pipefail

sums="${1:?usage: release-stamp.sh <SHA256SUMS> <install.sh>}"
target="${2:?usage: release-stamp.sh <SHA256SUMS> <install.sh>}"

sum="$(awk 'NR==1 {print $1}' "$sums")"
case "$sum" in
  [0-9a-f]*) [ "${#sum}" = 64 ] || { echo "not a sha256: $sum" >&2; exit 1; } ;;
  *) echo "no checksum in $sums" >&2; exit 1 ;;
esac

current="$(sed -n 's/^AYEAYE_SHA256="\([^"]*\)".*$/\1/p' "$target" | head -1)"
if [ "$current" = "$sum" ]; then
  printf 'already stamped: %s\n' "$sum"
  exit 0
fi

tmp="$target.tmp.$$"
trap 'rm -f "$tmp"' EXIT
sed 's|^AYEAYE_SHA256=".*"$|AYEAYE_SHA256="'"$sum"'"|' "$target" > "$tmp"
grep -q "^AYEAYE_SHA256=\"$sum\"$" "$tmp" || {
  echo "could not write the checksum into $target" >&2
  exit 1
}
bash -n "$tmp" || { echo "stamping broke $target, so it was not written" >&2; exit 1; }
cat "$tmp" > "$target"
trap - EXIT
rm -f "$tmp"

printf 'stamped %s with %s\n' "$target" "$sum"
printf 'commit this, then tag the commit the artifact was built from.\n'
