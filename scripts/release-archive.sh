#!/usr/bin/env bash
# Build the release tarball from HEAD, reproducibly.
#
# Reproducible matters here because the checksum baked into install.sh is the
# only thing standing between a stranger and whatever a compromised CDN feels
# like handing them. If the same commit does not produce the same bytes, the
# checksum is a number nobody can re-derive, and "verify the download" becomes
# a ritual rather than a check.
#
# Two sources of noise, both closed:
#   git archive  takes mtimes from the commit, not from the clock
#   gzip -n      omits the filename and timestamp it would otherwise embed
set -euo pipefail

out="${1:?usage: release-archive.sh <out.tar.gz> [commit]}"
commit="${2:-HEAD}"
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

# The version the archived commit claims, not the working tree's: rebuilding
# an old release's artifact after a version bump must reproduce the old
# bytes, prefix included, or the reproducibility check would compare a
# v0.1.0 tarball against a v0.2.0 rebuild and blame the wrong thing.
version="$(git show "$commit:install.sh" \
  | sed -n 's/^AYEAYE_VERSION="\([^"]*\)".*$/\1/p' | head -1)"
[ -n "$version" ] || { echo "no AYEAYE_VERSION in $commit:install.sh" >&2; exit 1; }

# The artifact is built from a commit, so a dirty tree means it does not
# contain what is on screen. Not fatal while developing - `cook publish` is
# where it has to be clean - but it must not pass silently.
#
# One kind of dirt is fatal: an uncommitted version bump. The archive takes
# its content from HEAD and its name from the bumped Cookfile, so building
# through it would mint a tarball whose installer disagrees with its own
# filename - and `cook stamp` would then checksum that mislabeled artifact
# into install.sh as if it were the release. Refused here, where the lie
# would be minted, rather than caught three steps later by a checker.
if [ "$commit" = HEAD ]; then
  tree_version="$(bash scripts/release-version.sh)"
  if [ "$tree_version" != "$version" ]; then
    cat >&2 <<EOF
the version bump is not committed: the working tree says $tree_version, HEAD
says $version, and the artifact is archived from HEAD. Commit the bump first -
it is the commit the release is built from and the one the tag will point at -
then run cook dist again.
EOF
    exit 1
  fi
  if ! git diff-index --quiet HEAD -- 2>/dev/null; then
    echo "note: the working tree is dirty; this artifact is HEAD, not what you see" >&2
  fi
fi

mkdir -p "$(dirname "$out")"
tmp="$out.tmp.$$"
trap 'rm -f "$tmp"' EXIT

# One top-level directory, which is what the bootstrap's unpack expects to
# find. It takes whatever that directory is called, but naming it after the
# release is what makes a downloaded tarball self-describing.
git archive --format=tar --prefix="ayeaye-$version/" "$commit" \
  | gzip -n -9 > "$tmp"

mv "$tmp" "$out"
trap - EXIT

printf 'built %s from %s (%s)\n' "$out" "$(git rev-parse --short "$commit")" "$version"
