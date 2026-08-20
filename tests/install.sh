#!/bin/sh
# The installer, driven against a release made of files.
#
# Much shorter since AYEAYE-101. Most of what was here was about the CUDA
# artifact: it was a tarball rather than a binary, so it had to be unpacked,
# checked for members that could escape staging, staged under a
# checksum-addressed name, and fallen back from when the machine turned out not
# to be able to run it. There is one static binary per machine now, so all of
# that went with it — what is left is the part that was always the point.
set -eu

root="$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
mkdir -p "$work/bin" "$work/release" "$work/home/.local/bin"

cat >"$work/bin/uname" <<'EOF'
#!/bin/sh
[ "$1" = -s ] && echo Linux || echo x86_64
EOF
# Still on PATH, and deliberately: a machine with an NVIDIA card must now get
# exactly the same artifact as one without. An installer that still asked would
# be choosing a build that is not published any more.
cat >"$work/bin/nvidia-smi" <<'EOF'
#!/bin/sh
echo 'GPU 0: NVIDIA'
EOF
cat >"$work/bin/curl" <<'EOF'
#!/bin/sh
while [ "$1" != -o ]; do shift; done
out="$2"
shift 2
cp "$RELEASE_DIR/${1##*/}" "$out"
EOF
chmod +x "$work/bin/"*

candidate() {
  name="$1"
  result="$2"
  cat >"$work/release/$name" <<EOF
#!/bin/sh
[ "\${1:-}" = --version ] && exit $result
echo "$name" >"$work/ran"
EOF
  chmod +x "$work/release/$name"
}

sums() {
  sha256sum "$work/release"/ayeaye-* | sed "s|$work/release/||" >"$work/release/SHA256SUMS"
}

install() {
  rm -f "$work/ran"
  PATH="$work/bin:$PATH" HOME="$work/home" RELEASE_DIR="$work/release" \
    AYEAYE_RELEASE_BASE=https://release.invalid sh "$root/install.sh" --yes </dev/null
}

# AYEAYE-101 — a machine with an NVIDIA card gets the portable build, because
# there is no other one. This is the assertion the deleted CUDA row is replaced
# by: `nvidia-smi` answers on this fake machine, and it changes nothing.
candidate ayeaye-x86_64-unknown-linux-musl 0
sums
printf '%s\n' old >"$work/home/.local/bin/ayeaye"
install_output="$(install)"
grep -qx ayeaye-x86_64-unknown-linux-musl "$work/ran"
grep -q unknown-linux-musl "$work/home/.local/bin/ayeaye"
# ...and it is a binary, not a symlink into a staged bundle.
test ! -L "$work/home/.local/bin/ayeaye"

# AYEAYE-86 — successful installs steer notification users to HTTPS setup.
printf '%s\n' "$install_output" | grep -q 'Notifications require HTTPS'
printf '%s\n' "$install_output" | grep -q 'README.*HTTPS'

# AYEAYE-81 — a candidate that cannot execute never replaces a working install.
# The fallback row it used to fall back *to* is gone, so this is now the whole
# of that behaviour: refuse, and leave what was there.
candidate ayeaye-x86_64-unknown-linux-musl 127
sums
printf '%s\n' old >"$work/home/.local/bin/ayeaye"
if install; then
  echo 'installer accepted a candidate that could not execute' >&2
  exit 1
fi
grep -qx old "$work/home/.local/bin/ayeaye"

# A download whose bytes do not match the published checksum is refused, and
# the working install survives that too.
candidate ayeaye-x86_64-unknown-linux-musl 0
sums
printf '%s\n' tampered >>"$work/release/ayeaye-x86_64-unknown-linux-musl"
printf '%s\n' old >"$work/home/.local/bin/ayeaye"
if install; then
  echo 'installer accepted bytes the release did not vouch for' >&2
  exit 1
fi
grep -qx old "$work/home/.local/bin/ayeaye"
