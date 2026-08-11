#!/usr/bin/env bash
# Move the release version everywhere it is written down, in one command.
#
# Cargo.toml's [workspace.package] owns the version; the Cookfile repeats it
# because recipe output paths are literals, and the lockfile repeats it once
# per workspace member - which the release workflow's `--locked` verify turns
# into a red tag if it lags. This rewrites all of them from the one value
# given, and the release-version gate stays the check that nothing was
# missed. The installer is not touched: it is a downloader with nothing
# pinned in it (AYEAYE-63).
#
# The lockfile edit touches only sourceless blocks - packages with no
# `source =` line, which is how Cargo marks workspace-local crates - because
# a dependency is allowed to share our version number and must not move.
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

cookfile="$root/Cookfile"
cargo_toml="$root/Cargo.toml"
cargo_lock="$root/Cargo.lock"
for f in "$cookfile" "$cargo_toml" "$cargo_lock"; do
  [ -f "$f" ] || { echo "no $f to bump" >&2; exit 1; }
done

old_plain="$(awk '
  /^\[/ { wp = ($0 == "[workspace.package]") }
  wp && /^version = / { gsub(/^version = "|"$/, ""); print; exit }
' "$cargo_toml")"
[ -n "$old_plain" ] || { echo "no [workspace.package] version in $cargo_toml" >&2; exit 1; }
old="v$old_plain"
if [ "$old" = "$version" ]; then
  echo "already at $version" >&2
  exit 0
fi
new_plain="${version#v}"

trap 'rm -f "$cookfile.tmp.$$" "$cargo_toml.tmp.$$" "$cargo_lock.tmp.$$"' EXIT

# Cookfile: every place the old version appears is a place the new one
# belongs - artifact names, the gate's argument, the publish default.
sed "s|$old|$version|g" "$cookfile" > "$cookfile.tmp.$$"
grep -q "$version" "$cookfile.tmp.$$" \
  || { echo "could not write the version into $cookfile" >&2; exit 1; }
if grep -q "$old" "$cookfile.tmp.$$"; then
  echo "$cookfile still mentions $old after the rewrite" >&2
  exit 1
fi

# Cargo.toml: the one version line inside [workspace.package], and nothing
# else - the bare number can legitimately appear elsewhere in the manifest as
# a dependency requirement.
awk -v old="$old_plain" -v new="$new_plain" '
  /^\[/ { wp = ($0 == "[workspace.package]") }
  wp && $0 == "version = \"" old "\"" { print "version = \"" new "\""; next }
  { print }
' "$cargo_toml" > "$cargo_toml.tmp.$$"
grep -q "^version = \"$new_plain\"\$" "$cargo_toml.tmp.$$" \
  || { echo "could not write the version into $cargo_toml" >&2; exit 1; }

# Cargo.lock: the workspace members repeat the manifest's claim, and cargo
# refuses --locked over a lock that lags it. Only sourceless blocks move.
awk -v old="$old_plain" -v new="$new_plain" '
  function flush(   i, sourced, line) {
    if (!n) return
    sourced = 0
    for (i = 1; i <= n; i++) if (buf[i] ~ /^source = /) sourced = 1
    for (i = 1; i <= n; i++) {
      line = buf[i]
      if (!sourced && line == "version = \"" old "\"") line = "version = \"" new "\""
      print line
    }
    n = 0
  }
  /^\[/ { flush(); inblock = ($0 == "[[package]]") }
  { if (inblock) buf[++n] = $0; else print }
  END { flush() }
' "$cargo_lock" > "$cargo_lock.tmp.$$"
grep -q "^version = \"$new_plain\"\$" "$cargo_lock.tmp.$$" \
  || { echo "could not write the version into $cargo_lock" >&2; exit 1; }

cat "$cookfile.tmp.$$" > "$cookfile"
cat "$cargo_toml.tmp.$$" > "$cargo_toml"
cat "$cargo_lock.tmp.$$" > "$cargo_lock"
trap - EXIT
rm -f "$cookfile.tmp.$$" "$cargo_toml.tmp.$$" "$cargo_lock.tmp.$$"

cat <<EOF
bumped $old -> $version in Cargo.toml, Cargo.lock and the Cookfile.
The release flow from here:

  commit this, then:
    cook dist            build the artifact from that commit
    cook publish $version
EOF
