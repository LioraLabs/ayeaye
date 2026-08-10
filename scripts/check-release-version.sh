#!/usr/bin/env bash
# The claim-in-sync gate. install.sh owns the version; the Cookfile names the
# artifact after it. Those two drifting apart would build ayeaye-v0.1.0.tar.gz
# for a v0.2.0 installer, and the bootstrap would fetch a name that is not
# there - a 404 at somebody else's machine rather than a red build at ours.
#
# The Cargo workspace claims the version too - Cargo.toml's
# [workspace.package], repeated in Cargo.lock once per member, both without
# the v everything else carries. The release workflow runs this gate against
# the pushed tag, so drift in any claim is a red verify job rather than a
# release whose binaries report the wrong version.
set -euo pipefail

expected="${1:?usage: check-release-version.sh <expected-version>}"
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
actual="$(bash "$root/scripts/release-version.sh")"
fail=0

if [ "$expected" != "$actual" ]; then
  cat >&2 <<EOF
version drift: install.sh says $actual, the Cookfile says $expected.

Bumping the release means moving every claim - AYEAYE_VERSION in install.sh,
the artifact name and this gate's argument in the Cookfile, and the Cargo
workspace version - which is what 'cook bump' is for.
EOF
  fail=1
fi

# The artifact name itself, so a bump that edits the gate but forgets the
# output path cannot pass.
if ! grep -q "dist/ayeaye-$actual\.tar\.gz" "$root/Cookfile"; then
  echo "the Cookfile does not build dist/ayeaye-$actual.tar.gz" >&2
  fail=1
fi

# The crate manifest, without the v: a binary built from a drifted workspace
# would answer --version with a number the release page does not show.
cargo_claim="$(awk '
  /^\[/ { wp = ($0 == "[workspace.package]") }
  wp && /^version = / { gsub(/^version = "|"$/, ""); print; exit }
' "$root/Cargo.toml" 2>/dev/null || true)"
if [ "v$cargo_claim" != "$actual" ]; then
  cat >&2 <<EOF
version drift: install.sh says $actual, Cargo.toml says ${cargo_claim:-nothing}.
cook bump moves both; a hand edit has left one behind.
EOF
  fail=1
fi

# And the lockfile's copy of it, once per workspace member (the blocks with no
# source line). cargo test --locked refuses a lagging lock, so catching it
# here turns a red tag day into a red minute at home.
lock_drift="$(awk -v want="${actual#v}" '
  function flush(   i, sourced, name, ver) {
    if (!n) return
    sourced = 0; name = ""; ver = ""
    for (i = 1; i <= n; i++) {
      if (buf[i] ~ /^source = /) sourced = 1
      if (buf[i] ~ /^name = /) { name = buf[i]; gsub(/^name = "|"$/, "", name) }
      if (buf[i] ~ /^version = /) { ver = buf[i]; gsub(/^version = "|"$/, "", ver) }
    }
    if (!sourced && ver != want) print "  " name " claims " ver
    n = 0
  }
  /^\[/ { flush(); inblock = ($0 == "[[package]]") }
  { if (inblock) buf[++n] = $0 }
  END { flush() }
' "$root/Cargo.lock" 2>/dev/null || echo "  no readable Cargo.lock")"
if [ -n "$lock_drift" ]; then
  printf 'version drift: Cargo.lock disagrees with %s:\n%s\nrun cook bump, or cargo update --workspace, to move it.\n' \
    "$actual" "$lock_drift" >&2
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
printf 'version %s, agreed on by install.sh, the Cookfile, the Cargo workspace and the README\n' "$actual"
