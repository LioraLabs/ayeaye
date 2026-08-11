#!/usr/bin/env bash
# The claim-in-sync gate. Cargo.toml's [workspace.package] owns the version;
# the Cookfile names the artifact after it and the lockfile repeats it once
# per workspace member. Those drifting apart would build ayeaye-v0.1.0.tar.gz
# for a v0.2.0 manifest, or fail `--locked` on tag day. The release workflow
# runs this gate against the pushed tag, so drift in any claim is a red
# verify job rather than a release whose binaries report the wrong version.
set -euo pipefail

expected="${1:?usage: check-release-version.sh <expected-version>}"
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
actual="$(bash "$root/scripts/release-version.sh")"
fail=0

if [ "$expected" != "$actual" ]; then
  cat >&2 <<EOF
version drift: Cargo.toml says $actual, the Cookfile's gate says $expected.

Bumping the release means moving every claim - the [workspace.package]
version in Cargo.toml and its lockfile, and the artifact name and this
gate's argument in the Cookfile - which is what 'cook bump' is for.
EOF
  fail=1
fi

# The artifact name itself, so a bump that edits the gate but forgets the
# output path cannot pass.
if ! grep -q "dist/ayeaye-$actual\.tar\.gz" "$root/Cookfile"; then
  echo "the Cookfile does not build dist/ayeaye-$actual.tar.gz" >&2
  fail=1
fi

# The lockfile's copy of the manifest's claim, once per workspace member (the
# blocks with no source line). cargo test --locked refuses a lagging lock, so
# catching it here turns a red tag day into a red minute at home.
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

# The one-liner is the product's front door; a README that stops naming it is
# the kind of thing nobody notices until they are debugging something else
# entirely.
if [ -f "$root/README.md" ] && ! grep -q 'raw.githubusercontent.com/LioraLabs/ayeaye/main/install.sh' "$root/README.md"; then
  echo "README.md no longer documents the raw install.sh one-liner" >&2
  fail=1
fi

[ "$fail" = 0 ] || exit 1
printf 'version %s, agreed on by Cargo.toml, its lockfile, the Cookfile and the README\n' "$actual"
