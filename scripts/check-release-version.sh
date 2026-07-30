#!/usr/bin/env bash
# The claim-in-sync gate. install.sh owns the version; the Cookfile names the
# artifact after it. Those two drifting apart would build ayeaye-v0.1.0.tar.gz
# for a v0.2.0 installer, and the bootstrap would fetch a name that is not
# there - a 404 at somebody else's machine rather than a red build at ours.
set -euo pipefail

expected="${1:?usage: check-release-version.sh <expected-version>}"
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
actual="$(bash "$root/scripts/release-version.sh")"
fail=0

if [ "$expected" != "$actual" ]; then
  cat >&2 <<EOF
version drift: install.sh says $actual, the Cookfile says $expected.

Bumping the release means changing both:
  - AYEAYE_VERSION at the top of install.sh
  - the artifact name and this gate's argument in the Cookfile
EOF
  fail=1
fi

# The artifact name itself, so a bump that edits the gate but forgets the
# output path cannot pass.
if ! grep -q "dist/ayeaye-$actual\.tar\.gz" "$root/Cookfile"; then
  echo "the Cookfile does not build dist/ayeaye-$actual.tar.gz" >&2
  fail=1
fi

# A released tarball that says it is a different version than the tag it hangs
# under is the kind of thing nobody notices until they are debugging something
# else entirely.
if [ -f "$root/README.md" ] && ! grep -q 'raw.githubusercontent.com/LioraLabs/ayeaye/main/install.sh' "$root/README.md"; then
  echo "README.md no longer documents the raw install.sh one-liner" >&2
  fail=1
fi

[ "$fail" = 0 ] || exit 1
printf 'version %s, agreed on by install.sh, the Cookfile and the README\n' "$actual"
