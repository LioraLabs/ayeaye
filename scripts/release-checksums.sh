#!/usr/bin/env bash
# SHA256SUMS, in the shape the bootstrap reads it: bare filenames, no paths.
#
# The bootstrap looks the artifact up by basename, because that is the name it
# has on the release page. A sums file listing dist/ayeaye-v0.1.0.tar.gz would
# parse, find nothing, and report "the checksums published with this release do
# not mention this file" - which is true, and useless.
#
# Takes any number of artifacts, output last: the release workflow publishes
# every binary under a versioned name and a versionless alias plus the source
# tarball, and one sums file must cover every one of those names.
set -euo pipefail

[ "$#" -ge 2 ] || {
  echo "usage: release-checksums.sh <artifact>... <out>" >&2
  exit 1
}

# Everything but the last argument is an artifact; the last is the output.
artifacts=("$@")
out="${artifacts[$((${#artifacts[@]} - 1))]}"
unset 'artifacts[$((${#artifacts[@]} - 1))]'

command -v sha256sum >/dev/null 2>&1 || {
  echo "no sha256sum on this machine, so no checksums can be published" >&2
  exit 1
}

mkdir -p "$(dirname "$out")"
tmp="$out.tmp.$$"
trap 'rm -f "$tmp"' EXIT
: > "$tmp"
for artifact in "${artifacts[@]}"; do
  [ -f "$artifact" ] || { echo "no artifact at $artifact" >&2; exit 1; }
  ( cd "$(dirname "$artifact")" && sha256sum "$(basename "$artifact")" ) >> "$tmp"
done
mv "$tmp" "$out"
trap - EXIT

cat "$out"
