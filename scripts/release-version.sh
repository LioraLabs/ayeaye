#!/usr/bin/env bash
# Where the released version is written down: AYEAYE_VERSION at the top of
# install.sh, which is also the value the bootstrap fetches by. The Cookfile,
# the README and the Cargo workspace (Cargo.toml and its lockfile, both
# without the `v` this file carries) all repeat it; `cook bump` moves every
# claim and the release-version gate checks that none has drifted.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
version="$(sed -n 's/^AYEAYE_VERSION="\([^"]*\)".*$/\1/p' "$root/install.sh" | head -1)"

if [ -z "$version" ]; then
  echo "no AYEAYE_VERSION in install.sh" >&2
  exit 1
fi
printf '%s\n' "$version"
