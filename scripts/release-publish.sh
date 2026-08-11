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
  echo "asked to publish $version, but the tree says $expected" >&2
  exit 1
}

# A dirty tree cannot be published - main must end up describing what
# shipped. The tag lands on the commit the artifact was built from, so the
# release is one commit: bump, commit, dist, publish.
if ! git diff-index --quiet HEAD -- 2>/dev/null; then
  echo "the working tree is dirty - commit before publishing" >&2
  exit 1
fi

artifact="dist/ayeaye-$version.tar.gz"
sums="dist/SHA256SUMS"
for f in "$artifact" "$sums"; do
  [ -f "$f" ] || { echo "missing $f - run 'cook dist' first" >&2; exit 1; }
done

# The same checks the release-installable recipe runs, against the artifact
# actually being uploaded.
bash scripts/check-release-artifact.sh "$artifact"

# The commit the tag must point at is written inside the tarball by
# git archive. Rebuild from it and compare, so what gets tagged is provably
# the source of the bytes being uploaded.
artifact_sum="$(sha256sum < "$artifact" | cut -d' ' -f1)"
src="$(gzip -dc "$artifact" | git get-tar-commit-id || true)"
[ -n "$src" ] || { echo "the artifact does not name its source commit" >&2; exit 1; }
again="$(mktemp -u)"
trap 'rm -f "$again"' EXIT
bash scripts/release-archive.sh "$again" "$src" >/dev/null
[ "$(sha256sum < "$again" | cut -d' ' -f1)" = "$artifact_sum" ] || {
  echo "rebuilding from $src does not reproduce $artifact - refusing to tag it" >&2
  exit 1
}
git merge-base --is-ancestor "$src" HEAD || {
  echo "the artifact was built from $src, which is not an ancestor of HEAD" >&2
  exit 1
}

command -v gh >/dev/null 2>&1 || { echo "no gh on this machine" >&2; exit 1; }
gh auth status >/dev/null 2>&1 || { echo "gh is not logged in" >&2; exit 1; }

if gh release view "$version" >/dev/null 2>&1; then
  echo "release $version already exists - delete it first, or bump the version" >&2
  exit 1
fi

cat <<EOF
About to publish:

  tag       $version  ->  $src  (the commit the artifact was built from)
  artifact  $artifact
  checksums $sums

That commit must be on the remote already - push before publishing.
The binaries for each platform are built and uploaded by the release
workflow when this tag lands; the installer fetches those and verifies
them against the SHA256SUMS the workflow publishes over every name.
EOF
printf '\ncontinue? [y/N] '
read -r reply </dev/tty || reply=n
case "$reply" in y|Y|yes) ;; *) echo "nothing was published"; exit 0 ;; esac

gh release create "$version" "$artifact" "$sums" \
  --target "$src" \
  --title "ayeaye $version" \
  --notes "One-command install for ayeaye.

    curl -fsSL https://raw.githubusercontent.com/LioraLabs/ayeaye/main/install.sh | bash

The installer picks the binary for this machine, fetches it from the newest
release, verifies it against SHA256SUMS, and hands off to 'ayeaye setup'.

macOS is implemented but has not been verified on Apple hardware."

printf '\npublished. The one-liner works end to end once the release\n'
printf 'workflow has uploaded the binaries for this tag:\n'
printf '  curl -fsSL https://raw.githubusercontent.com/LioraLabs/ayeaye/main/install.sh | bash\n'
