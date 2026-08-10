#!/usr/bin/env bash
# Where the released version is written down: AYEAYE_VERSION at the top of
# install.sh, which is also the value the bootstrap fetches by. The Cookfile
# and the README derive from it, and the release-version gate checks that they
# have not drifted.
#
# The Cargo workspace also claims a version, in Cargo.toml, and nothing here
# knows about it: `cook bump` will not move it and this gate will not notice.
# Teaching both is AYEAYE-59's, along with the fact that cargo writes it
# without the `v` this file carries.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
version="$(sed -n 's/^AYEAYE_VERSION="\([^"]*\)".*$/\1/p' "$root/install.sh" | head -1)"

if [ -z "$version" ]; then
  echo "no AYEAYE_VERSION in install.sh" >&2
  exit 1
fi
printf '%s\n' "$version"
