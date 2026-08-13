#!/bin/sh
set -eu

root="$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
mkdir -p "$work/bin" "$work/release" "$work/home/.local/bin"

cat >"$work/bin/uname" <<'EOF'
#!/bin/sh
[ "$1" = -s ] && echo Linux || echo x86_64
EOF
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

cuda_candidate() {
  result="$1"
  bundle="$work/cuda-bundle"
  rm -rf "$bundle"
  mkdir -p "$bundle/lib"
  cat >"$bundle/ayeaye" <<EOF
#!/bin/sh
[ "\${1:-}" = --version ] && exit $result
echo ayeaye-x86_64-unknown-linux-gnu-cuda >"$work/ran"
EOF
  chmod +x "$bundle/ayeaye"
  printf '%s\n' bundled-runtime >"$bundle/lib/libcudart.so.12"
  tar -czf "$work/release/ayeaye-x86_64-unknown-linux-gnu-cuda" -C "$bundle" .
}

install() {
  rm -f "$work/ran"
  PATH="$work/bin:$PATH" HOME="$work/home" RELEASE_DIR="$work/release" \
    AYEAYE_RELEASE_BASE=https://release.invalid sh "$root/install.sh" --yes </dev/null
}

# AYEAYE-81 — a CUDA artifact that cannot load falls back without replacing
# the working install until the portable candidate has executed successfully.
cuda_candidate 127
candidate ayeaye-x86_64-unknown-linux-musl 0
sha256sum "$work/release"/ayeaye-* | sed "s|$work/release/||" >"$work/release/SHA256SUMS"
printf '%s\n' old >"$work/home/.local/bin/ayeaye"
install_output="$(install)"
# AYEAYE-86 — successful installs steer notification users to HTTPS setup.
printf '%s\n' "$install_output" | grep -q 'Notifications require HTTPS'
printf '%s\n' "$install_output" | grep -q 'README.*HTTPS'
grep -qx ayeaye-x86_64-unknown-linux-musl "$work/ran"
grep -q unknown-linux-musl "$work/home/.local/bin/ayeaye"

# A checksum-valid archive still cannot escape staging through a link member.
bundle="$work/cuda-bundle"
rm -rf "$bundle"
mkdir -p "$bundle/lib"
ln -s "$work/escaped" "$bundle/ayeaye"
tar -czf "$work/release/ayeaye-x86_64-unknown-linux-gnu-cuda" -C "$bundle" .
sha256sum "$work/release"/ayeaye-* | sed "s|$work/release/||" >"$work/release/SHA256SUMS"
install
test ! -e "$work/escaped"
grep -q unknown-linux-musl "$work/home/.local/bin/ayeaye"

cuda_candidate 0
sha256sum "$work/release"/ayeaye-* | sed "s|$work/release/||" >"$work/release/SHA256SUMS"
install
grep -qx ayeaye-x86_64-unknown-linux-gnu-cuda "$work/ran"
test -L "$work/home/.local/bin/ayeaye"
cuda_install="$(readlink -f "$work/home/.local/bin/ayeaye")"
grep -q gnu-cuda "$cuda_install"
grep -qx bundled-runtime "$(dirname "$cuda_install")/lib/libcudart.so.12"

# An interrupted checksum-addressed destination is never trusted or activated.
cuda_candidate 0
sha256sum "$work/release"/ayeaye-* | sed "s|$work/release/||" >"$work/release/SHA256SUMS"
cuda_sum="$(sha256sum "$work/release/ayeaye-x86_64-unknown-linux-gnu-cuda")"
cuda_sum="${cuda_sum%% *}"
rm -rf "$work/home/.local/bin/.ayeaye-cuda-$cuda_sum"
mkdir -p "$work/home/.local/bin/.ayeaye-cuda-$cuda_sum"
printf '%s\n' old >"$work/home/.local/bin/ayeaye"
if install; then
  echo 'installer activated an incomplete prior CUDA bundle' >&2
  exit 1
fi
grep -qx old "$work/home/.local/bin/ayeaye"

cuda_candidate 127
candidate ayeaye-x86_64-unknown-linux-musl 127
sha256sum "$work/release"/ayeaye-* | sed "s|$work/release/||" >"$work/release/SHA256SUMS"
printf '%s\n' old >"$work/home/.local/bin/ayeaye"
if install; then
  echo 'installer accepted two candidates that could not execute' >&2
  exit 1
fi
grep -qx old "$work/home/.local/bin/ayeaye"
