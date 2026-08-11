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

install() {
  rm -f "$work/ran"
  PATH="$work/bin:$PATH" HOME="$work/home" RELEASE_DIR="$work/release" \
    AYEAYE_RELEASE_BASE=https://release.invalid sh "$root/install.sh" --yes </dev/null
}

# AYEAYE-81 — a CUDA artifact that cannot load falls back without replacing
# the working install until the portable candidate has executed successfully.
candidate ayeaye-x86_64-unknown-linux-gnu-cuda 127
candidate ayeaye-x86_64-unknown-linux-musl 0
sha256sum "$work/release"/ayeaye-* | sed "s|$work/release/||" >"$work/release/SHA256SUMS"
printf '%s\n' old >"$work/home/.local/bin/ayeaye"
install_output="$(install)"
# AYEAYE-86 — successful installs steer notification users to HTTPS setup.
printf '%s\n' "$install_output" | grep -q 'Notifications require HTTPS'
printf '%s\n' "$install_output" | grep -q 'README.*HTTPS'
grep -qx ayeaye-x86_64-unknown-linux-musl "$work/ran"
grep -q unknown-linux-musl "$work/home/.local/bin/ayeaye"

candidate ayeaye-x86_64-unknown-linux-gnu-cuda 0
sha256sum "$work/release"/ayeaye-* | sed "s|$work/release/||" >"$work/release/SHA256SUMS"
install
grep -qx ayeaye-x86_64-unknown-linux-gnu-cuda "$work/ran"
grep -q gnu-cuda "$work/home/.local/bin/ayeaye"

candidate ayeaye-x86_64-unknown-linux-gnu-cuda 127
candidate ayeaye-x86_64-unknown-linux-musl 127
sha256sum "$work/release"/ayeaye-* | sed "s|$work/release/||" >"$work/release/SHA256SUMS"
printf '%s\n' old >"$work/home/.local/bin/ayeaye"
if install; then
  echo 'installer accepted two candidates that could not execute' >&2
  exit 1
fi
grep -qx old "$work/home/.local/bin/ayeaye"
