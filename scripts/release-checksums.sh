#!/usr/bin/env bash
# SHA256SUMS, in the shape the bootstrap reads it: bare filenames, no paths.
#
# The bootstrap looks the artifact up by basename, because that is the name it
# has on the release page. A sums file listing dist/ayeaye-v0.1.0.tar.gz would
# parse, find nothing, and report "the checksums published with this release do
# not mention this file" - which is true, and useless.
set -euo pipefail

artifact="${1:?usage: release-checksums.sh <artifact> <out>}"
out="${2:?usage: release-checksums.sh <artifact> <out>}"

command -v sha256sum >/dev/null 2>&1 || {
  echo "no sha256sum on this machine, so no checksums can be published" >&2
  exit 1
}

mkdir -p "$(dirname "$out")"
( cd "$(dirname "$artifact")" && sha256sum "$(basename "$artifact")" ) > "$out"

cat "$out"
