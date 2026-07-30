#!/usr/bin/env bash
# Tag the release and upload its artifacts.
#
# This is the only script here that talks to anybody else. It is deliberately
# not a recipe: publishing is not something a build graph should be able to do
# on its own, and `cook test` must never reach it.
set -euo pipefail

version="${1:?usage: release-publish.sh <version>}"
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

expected="$(bash scripts/release-version.sh)"
[ "$version" = "$expected" ] || {
  echo "asked to publish $version, but install.sh says $expected" >&2
  exit 1
}

artifact="dist/ayeaye-$version.tar.gz"
sums="dist/SHA256SUMS"
for f in "$artifact" "$sums"; do
  [ -f "$f" ] || { echo "missing $f - run 'cook dist' first" >&2; exit 1; }
done

command -v gh >/dev/null 2>&1 || { echo "no gh on this machine" >&2; exit 1; }
gh auth status >/dev/null 2>&1 || { echo "gh is not logged in" >&2; exit 1; }

if gh release view "$version" >/dev/null 2>&1; then
  echo "release $version already exists - delete it first, or bump the version" >&2
  exit 1
fi

commit="$(git rev-parse HEAD)"
cat <<EOF
About to publish:

  tag       $version  ->  $commit
  artifact  $artifact
  checksums $sums

The tag must point at the commit the artifact was built from. If install.sh
has been stamped since the build, that commit is the one BEFORE the stamp.
EOF
printf '\ncontinue? [y/N] '
read -r reply </dev/tty || reply=n
case "$reply" in y|Y|yes) ;; *) echo "nothing was published"; exit 0 ;; esac

gh release create "$version" "$artifact" "$sums" \
  --title "ayeaye $version" \
  --notes "One-command setup for ayeaye.

    curl -fsSL https://raw.githubusercontent.com/LioraLabs/ayeaye/main/install.sh | bash

The installer fetches this release, checks it against SHA256SUMS, and runs the
setup wizard from the copy it unpacks. Running it from a clone downloads
nothing.

macOS is implemented but has not been verified on Apple hardware."

printf '\npublished. The one-liner should now work end to end:\n'
printf '  curl -fsSL https://raw.githubusercontent.com/LioraLabs/ayeaye/main/install.sh | bash\n'
