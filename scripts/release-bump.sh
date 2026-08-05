#!/usr/bin/env bash
# Move the release version everywhere it is written down, in one command.
#
# install.sh owns the version; the Cookfile repeats it because recipe output
# paths are literals - four places, which is three too many to edit by hand
# without one drifting. This rewrites all of them from the one value given,
# and the release-version gate stays the check that nothing was missed.
#
# The stamp is cleared, not kept: between this bump and `cook stamp` the old
# checksum describes an artifact this version will never be, and a stale sum
# fails loudly at a stranger's machine where an absent one falls back to the
# published SHA256SUMS and says so. Honest and weaker beats wrong and scary.
#
# [root] exists for the tests, which run this against a copied tree rather
# than the repository they live in.
set -euo pipefail

version="${1:?usage: release-bump.sh <version> [root]}"
root="${2:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"

case "$version" in
  v[0-9]*.[0-9]*.[0-9]*) ;;
  *) echo "versions look like v0.2.0, not '$version'" >&2; exit 1 ;;
esac

install_sh="$root/install.sh"
cookfile="$root/Cookfile"
for f in "$install_sh" "$cookfile"; do
  [ -f "$f" ] || { echo "no $f to bump" >&2; exit 1; }
done

old="$(sed -n 's/^AYEAYE_VERSION="\([^"]*\)".*$/\1/p' "$install_sh" | head -1)"
[ -n "$old" ] || { echo "no AYEAYE_VERSION in $install_sh" >&2; exit 1; }
if [ "$old" = "$version" ]; then
  echo "already at $version" >&2
  exit 0
fi

# install.sh: the version, and the stamp it no longer has.
tmp="$install_sh.tmp.$$"
trap 'rm -f "$tmp" "$cookfile.tmp.$$"' EXIT
sed -e 's|^AYEAYE_VERSION=".*"$|AYEAYE_VERSION="'"$version"'"|' \
    -e 's|^AYEAYE_SHA256=".*"$|AYEAYE_SHA256=""|' "$install_sh" > "$tmp"
grep -q "^AYEAYE_VERSION=\"$version\"\$" "$tmp" \
  || { echo "could not write the version into $install_sh" >&2; exit 1; }
bash -n "$tmp" || { echo "bumping broke $install_sh, so it was not written" >&2; exit 1; }

# Cookfile: every place the old version appears is a place the new one
# belongs - artifact names, the gate's argument, the publish default.
sed "s|$old|$version|g" "$cookfile" > "$cookfile.tmp.$$"
grep -q "$version" "$cookfile.tmp.$$" \
  || { echo "could not write the version into $cookfile" >&2; exit 1; }
if grep -q "$old" "$cookfile.tmp.$$"; then
  echo "$cookfile still mentions $old after the rewrite" >&2
  exit 1
fi

cat "$tmp" > "$install_sh"
cat "$cookfile.tmp.$$" > "$cookfile"
trap - EXIT
rm -f "$tmp" "$cookfile.tmp.$$"

cat <<EOF
bumped $old -> $version in install.sh and the Cookfile.
The stamp was cleared; the release flow from here:

  commit this, then:
    cook dist         build the artifact from that commit
    cook stamp        write its checksum into install.sh
  commit the stamp, push, then:
    cook publish $version
EOF
