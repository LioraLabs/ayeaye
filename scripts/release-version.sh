#!/usr/bin/env bash
# The one place the released version is written down: AYEAYE_VERSION at the top
# of install.sh, which is also the value the bootstrap fetches by. Everything
# else derives from this, so there is nothing to keep in sync by hand.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
version="$(sed -n 's/^AYEAYE_VERSION="\([^"]*\)".*$/\1/p' "$root/install.sh" | head -1)"

if [ -z "$version" ]; then
  echo "no AYEAYE_VERSION in install.sh" >&2
  exit 1
fi
printf '%s\n' "$version"
