#!/usr/bin/env bash
# Where the released version is written down: Cargo.toml's [workspace.package],
# which the binary itself answers `--version` from. The Cookfile repeats it
# (with the v) in its artifact names, and Cargo.lock repeats it once per
# workspace member; `cook bump` moves every claim and the release-version gate
# checks that none has drifted. The installer claims nothing: it is a
# downloader that fetches whatever release is newest (AYEAYE-63).
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
version="$(awk '
  /^\[/ { wp = ($0 == "[workspace.package]") }
  wp && /^version = / { gsub(/^version = "|"$/, ""); print; exit }
' "$root/Cargo.toml" 2>/dev/null || true)"

if [ -z "$version" ]; then
  echo "no [workspace.package] version in Cargo.toml" >&2
  exit 1
fi
printf 'v%s\n' "$version"
