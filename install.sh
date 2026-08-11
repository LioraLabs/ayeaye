#!/bin/sh
# One-command install for ayeaye: a downloader, nothing more.
#
#   curl -fsSL https://raw.githubusercontent.com/LioraLabs/ayeaye/main/install.sh | bash
#
# It looks at this machine just enough to name one release artifact - an
# operating system, an architecture, and whether an NVIDIA card answers -
# fetches the newest release's copy, refuses it unless the SHA256SUMS
# published beside it agrees with the bytes, puts it on the PATH, and hands
# everything else to the binary's own `ayeaye setup`. Arguments pass through
# to setup, so `... | bash -s -- --yes` still means what it always meant.
#
# There is deliberately no more to it. Hardware classification, questions,
# consent, service units, health checks: all of that lives in the binary,
# where it can be tested. Re-running this script is the upgrade path - it
# fetches the latest release and replaces the binary in place.
set -eu

base="${AYEAYE_RELEASE_BASE:-https://github.com/LioraLabs/ayeaye/releases/latest/download}"
bin_dir="${AYEAYE_BIN_DIR:-$HOME/.local/bin}"

os="$(uname -s)"
arch="$(uname -m)"
case "$arch" in arm64) arch="aarch64" ;; esac
case "$os/$arch" in
  Linux/x86_64|Linux/aarch64)
    artifact="ayeaye-$arch-unknown-linux-musl"
    # The one question asked of the hardware: does an NVIDIA driver answer?
    # The release has a single CUDA row, x86_64; every verdict beyond this -
    # tier, model, whether the card is worth using - belongs to the binary.
    if [ "$arch" = x86_64 ] && command -v nvidia-smi >/dev/null 2>&1 \
        && nvidia-smi -L 2>/dev/null | grep -q '^GPU '; then
      artifact="ayeaye-x86_64-unknown-linux-gnu-cuda"
    fi ;;
  Darwin/x86_64|Darwin/aarch64)
    artifact="ayeaye-$arch-apple-darwin" ;;
  *)
    echo "no ayeaye build for $os/$arch - releases cover Linux and macOS on x86_64 and aarch64" >&2
    exit 1 ;;
esac

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
curl -fsSL -o "$work/SHA256SUMS" "$base/SHA256SUMS"

fetch() {
  echo "fetching $artifact"
  curl -fsSL -o "$work/ayeaye" "$base/$artifact"
  want="$(awk -v name="$artifact" '$2 == name { print $1; exit }' "$work/SHA256SUMS")"
  [ -n "$want" ] || { echo "the release's SHA256SUMS does not mention $artifact" >&2; return 1; }
  if command -v sha256sum >/dev/null 2>&1; then
    got="$(sha256sum "$work/ayeaye")"
  elif command -v shasum >/dev/null 2>&1; then
    got="$(shasum -a 256 "$work/ayeaye")"
  else
    echo "neither sha256sum nor shasum is on this machine - nothing can verify the download" >&2
    return 1
  fi
  got="${got%% *}"
  if [ "$got" != "$want" ]; then
    echo "checksum mismatch for $artifact - refusing to run it" >&2
    echo "  published $want" >&2
    echo "  fetched   $got" >&2
    return 1
  fi
  chmod +x "$work/ayeaye"
  "$work/ayeaye" --version >/dev/null 2>&1
}

if ! fetch; then
  if [ "$artifact" != ayeaye-x86_64-unknown-linux-gnu-cuda ]; then
    echo "$artifact cannot run on this machine - keeping the existing install" >&2
    exit 1
  fi
  echo "the CUDA build cannot run on this machine; falling back to the portable CPU build" >&2
  artifact=ayeaye-x86_64-unknown-linux-musl
  fetch || { echo "the portable CPU build cannot run either - keeping the existing install" >&2; exit 1; }
fi

# Release assets are bare bytes; the exec bit is this script's to grant.
mkdir -p "$bin_dir"
mv "$work/ayeaye" "$bin_dir/ayeaye"
echo "installed $bin_dir/ayeaye"
case ":$PATH:" in
  *:"$bin_dir":*) ;;
  *) echo "note: $bin_dir is not on your PATH" ;;
esac

rm -rf "$work"
trap - EXIT
# Piped through bash, stdin is this script and already spent; hand setup the
# terminal instead, so it can still ask its two questions.
if [ ! -t 0 ] && (exec </dev/tty) 2>/dev/null; then
  exec "$bin_dir/ayeaye" setup "$@" </dev/tty
fi
exec "$bin_dir/ayeaye" setup "$@"
