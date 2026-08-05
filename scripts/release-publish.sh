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

# A dirty tree cannot be published - main must end up describing what
# shipped. But the flow's own last step dirties the tree: cook stamp writes
# the checksum into install.sh, and committing it is what the person is in
# the middle of doing. Saying "dirty" to that person, in the same words as
# to one with half an edit open, is what made this feel like a chicken and
# egg - so the one kind of dirt the flow itself creates is named as the next
# step instead of an obstacle.
if ! git diff-index --quiet HEAD -- 2>/dev/null; then
  others="$(git diff HEAD --name-only | grep -v '^install\.sh$' || true)"
  stamp_only=""
  if [ -z "$others" ] \
     && ! git diff HEAD -- install.sh | grep '^[+-]' | grep -v '^[+-][+-]' \
          | grep -qv 'AYEAYE_SHA256='; then
    stamp_only=1
  fi
  if [ -n "$stamp_only" ]; then
    cat >&2 <<'EOF'
one thing left: the stamp cook stamp wrote is not committed yet. Commit it,
push, and publish again - the tag lands on the build commit either way, so
the stamp commit is safe to make.
EOF
  else
    echo "the working tree is dirty - commit before publishing" >&2
  fi
  exit 1
fi

artifact="dist/ayeaye-$version.tar.gz"
sums="dist/SHA256SUMS"
for f in "$artifact" "$sums"; do
  [ -f "$f" ] || { echo "missing $f - run 'cook dist' first" >&2; exit 1; }
done

# The same checks the release-installable recipe runs, against the artifact
# actually being uploaded. Publishing cannot depend on that recipe: it
# depends on dist, and dist rebuilds on every commit - including the stamp
# commit - which would replace the artifact the stamp describes.
bash scripts/check-release-artifact.sh "$artifact"

# The stamp in install.sh and the artifact on disk must be the same release.
# This is the check that makes the flow's one trap impossible to fall into:
# rebuilding dist after the stamp commit produces a tarball the stamp does
# not describe, and publishing that pair would make the bootstrap's strongest
# check fail on every stranger's machine.
artifact_sum="$(sha256sum < "$artifact" | cut -d' ' -f1)"
stamped="$(sed -n 's/^AYEAYE_SHA256="\([^"]*\)".*$/\1/p' install.sh | head -1)"
if [ "$stamped" != "$artifact_sum" ]; then
  cat >&2 <<EOF
the artifact on disk is not the one install.sh is stamped with.

  artifact  $artifact_sum
  stamped   ${stamped:-nothing}

This happens when dist was rebuilt after the stamp (each commit changes the
tarball). To converge: run 'cook stamp', commit the new stamp, and publish
again - the tag will point at the commit this artifact was built from.
EOF
  exit 1
fi

# The commit the tag must point at is written inside the tarball by
# git archive. Rebuild from it and compare, so what gets tagged is provably
# the source of the bytes being uploaded - not HEAD, which by design has
# moved past it (the stamp commit lands after the build).
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
EOF
printf '\ncontinue? [y/N] '
read -r reply </dev/tty || reply=n
case "$reply" in y|Y|yes) ;; *) echo "nothing was published"; exit 0 ;; esac

gh release create "$version" "$artifact" "$sums" \
  --target "$src" \
  --title "ayeaye $version" \
  --notes "One-command setup for ayeaye.

    curl -fsSL https://raw.githubusercontent.com/LioraLabs/ayeaye/main/install.sh | bash

The installer fetches this release, checks it against SHA256SUMS, and runs the
setup wizard from the copy it unpacks. Running it from a clone downloads
nothing.

macOS is implemented but has not been verified on Apple hardware."

printf '\npublished. The one-liner should now work end to end:\n'
printf '  curl -fsSL https://raw.githubusercontent.com/LioraLabs/ayeaye/main/install.sh | bash\n'
