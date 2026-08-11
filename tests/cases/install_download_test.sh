# The installer is a downloader. It looks at the machine, picks one release
# artifact, verifies it against the release's own SHA256SUMS, puts it on the
# PATH, and hands the rest to `ayeaye setup`. These tests drive the real
# script against a curl that answers from a local release directory, on a
# machine described by a stubbed uname and nvidia-smi.
#
# bash 3.2: no associative arrays, no ${var,,}, no mapfile.

setup() {
  require_host_command sha256sum
  stub_real sha256sum
  INSTALL="$REPO_ROOT/install.sh"
  RELEASE_DIR="$TEST_TMPDIR/release"
  mkdir -p "$RELEASE_DIR"
  AYEAYE_RELEASE_BASE="https://release.test/latest/download"
  AYEAYE_BIN_DIR="$HOME/.local/bin"
  export AYEAYE_RELEASE_BASE AYEAYE_BIN_DIR
  serve_release
}

# machine <kernel> <arch> - what uname answers on this machine.
machine() {
  UNAME_S="$1" UNAME_M="$2"
  export UNAME_S UNAME_M
  stub_script uname <<'SH'
case "${1:-}" in
  -m) printf '%s\n' "$UNAME_M" ;;
  *)  printf '%s\n' "$UNAME_S" ;;
esac
SH
}

# nvidia_answers - an nvidia-smi whose driver is loaded and lists a card.
nvidia_answers() {
  stub_script nvidia-smi <<'SH'
printf 'GPU 0: NVIDIA GeForce RTX 3080 (UUID: GPU-fake)\n'
SH
}

# nvidia_dead - an nvidia-smi that is on the PATH but whose driver is not
# talking: prose on stderr and a non-zero status, which is what a machine
# with the package installed and the module missing really does.
nvidia_dead() {
  stub_script nvidia-smi <<'SH'
echo "NVIDIA-SMI has failed because it couldn't communicate with the NVIDIA driver." >&2
exit 9
SH
}

# make_release <artifact>... - put fake binaries in the release directory and
# publish SHA256SUMS over everything in it. Mode 644 on purpose: release
# assets carry no exec bit, so the installer has to chmod what it fetches.
make_release() {
  local name
  for name in "$@"; do
    cat > "$RELEASE_DIR/$name" <<'SH'
#!/bin/sh
printf 'fake ayeaye ran: %s\n' "$*"
printf 'stdin-is-a-terminal: %s\n' "$([ -t 0 ] && echo yes || echo no)"
SH
    chmod 644 "$RELEASE_DIR/$name"
  done
  ( cd "$RELEASE_DIR" && sha256sum ayeaye-* > SHA256SUMS )
}

# serve_release - a curl that answers from $RELEASE_DIR by basename, so what
# was asked for stays assertable in the stub log.
serve_release() {
  export RELEASE_DIR
  stub_script curl <<'SH'
out="" url=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    -o|--output) out="$2"; shift 2 ;;
    -*)          shift ;;
    *)           url="$1"; shift ;;
  esac
done
file="$RELEASE_DIR/$(basename "$url")"
[ -f "$file" ] || exit 22
if [ -n "$out" ]; then cp "$file" "$out"; else cat "$file"; fi
SH
}

# ------------------------------------------------------------ picking the row

# AYEAYE-63/S1: detects operating system and architecture and fetches the
# matching artifact - plain Linux gets the static musl binary.
test_linux_x86_64_without_nvidia_fetches_the_musl_binary_and_runs_setup() {
  machine Linux x86_64
  make_release ayeaye-x86_64-unknown-linux-musl
  run_script "$INSTALL"
  assert_status 0 "$RUN_STATUS" "$RUN_OUTPUT"
  assert_stub_called_with curl "ayeaye-x86_64-unknown-linux-musl" \
    "the static musl artifact is the row for a Linux machine with no NVIDIA card"
  assert_contains "$RUN_STDOUT" "fake ayeaye ran: setup" \
    "the downloaded binary's own setup command is the handoff"
}

# AYEAYE-63/S1: a usable NVIDIA card on x86_64 Linux selects the CUDA build.
test_linux_x86_64_with_a_live_nvidia_driver_fetches_the_cuda_binary() {
  machine Linux x86_64
  nvidia_answers
  make_release ayeaye-x86_64-unknown-linux-gnu-cuda
  run_script "$INSTALL"
  assert_status 0 "$RUN_STATUS" "$RUN_OUTPUT"
  assert_stub_called_with curl "ayeaye-x86_64-unknown-linux-gnu-cuda"
  assert_contains "$RUN_STDOUT" "fake ayeaye ran: setup"
}

# AYEAYE-63/S1: nvidia-smi on the PATH with a dead driver is not a usable
# card - the static build is the honest choice.
test_a_dead_nvidia_driver_falls_back_to_the_musl_binary() {
  machine Linux x86_64
  nvidia_dead
  make_release ayeaye-x86_64-unknown-linux-musl
  run_script "$INSTALL"
  assert_status 0 "$RUN_STATUS" "$RUN_OUTPUT"
  assert_stub_called_with curl "ayeaye-x86_64-unknown-linux-musl"
}

# AYEAYE-63/S1: there is no aarch64 CUDA row, so an ARM machine with an
# NVIDIA card still gets the static build.
test_aarch64_linux_with_nvidia_still_fetches_the_musl_binary() {
  machine Linux aarch64
  nvidia_answers
  make_release ayeaye-aarch64-unknown-linux-musl
  run_script "$INSTALL"
  assert_status 0 "$RUN_STATUS" "$RUN_OUTPUT"
  assert_stub_called_with curl "ayeaye-aarch64-unknown-linux-musl"
  assert_not_contains "$(stub_calls curl)" "cuda" \
    "no CUDA row exists for aarch64, so none may be asked for"
}

# AYEAYE-63/S1: Apple hardware, both architectures - and uname says arm64
# where the release rows say aarch64.
test_apple_silicon_fetches_the_aarch64_darwin_binary() {
  machine Darwin arm64
  make_release ayeaye-aarch64-apple-darwin
  run_script "$INSTALL"
  assert_status 0 "$RUN_STATUS" "$RUN_OUTPUT"
  assert_stub_called_with curl "ayeaye-aarch64-apple-darwin"
}

# AYEAYE-63/S1
test_intel_mac_fetches_the_x86_64_darwin_binary() {
  machine Darwin x86_64
  make_release ayeaye-x86_64-apple-darwin
  run_script "$INSTALL"
  assert_status 0 "$RUN_STATUS" "$RUN_OUTPUT"
  assert_stub_called_with curl "ayeaye-x86_64-apple-darwin"
}

# AYEAYE-63/S1: a machine no release row covers is refused by name, before
# anything is downloaded.
test_an_unsupported_machine_is_refused_before_any_download() {
  machine FreeBSD riscv64
  make_release ayeaye-x86_64-unknown-linux-musl
  run_script "$INSTALL"
  assert_status 1 "$RUN_STATUS"
  assert_contains "$RUN_STDERR" "no ayeaye build for FreeBSD/riscv64"
  assert_stub_not_called curl "refusal must come before the network"
}

# ------------------------------------------------------------- verification

# AYEAYE-63/S2: the checksum is verified before anything runs - a tampered
# artifact is refused, not installed, not executed.
test_a_tampered_artifact_is_refused_before_install_or_handoff() {
  machine Linux x86_64
  make_release ayeaye-x86_64-unknown-linux-musl
  printf '#!/bin/sh\necho compromised\n' > "$RELEASE_DIR/ayeaye-x86_64-unknown-linux-musl"
  run_script "$INSTALL"
  assert_status 1 "$RUN_STATUS"
  assert_contains "$RUN_STDERR" "checksum mismatch"
  assert_file_missing "$AYEAYE_BIN_DIR/ayeaye" \
    "a binary that failed verification must not land on the PATH"
  assert_not_contains "$RUN_STDOUT" "compromised" \
    "and it must never have run"
}

# AYEAYE-63/S2: a SHA256SUMS that does not mention the artifact means nothing
# was compared - which is a refusal, not a shrug.
test_a_sums_file_that_omits_the_artifact_is_a_refusal() {
  machine Darwin arm64
  # Two artifacts published, then the sums line for the one this machine
  # needs is dropped - so the file still parses and still finds nothing.
  make_release ayeaye-aarch64-apple-darwin ayeaye-x86_64-apple-darwin
  ( cd "$RELEASE_DIR" && grep -v aarch64-apple-darwin SHA256SUMS > s && mv s SHA256SUMS )
  run_script "$INSTALL"
  assert_status 1 "$RUN_STATUS"
  assert_contains "$RUN_STDERR" "does not mention ayeaye-aarch64-apple-darwin"
  assert_file_missing "$AYEAYE_BIN_DIR/ayeaye"
}

# AYEAYE-63/S2: no sha256sum on the machine (macOS) - shasum answers instead.
test_verification_falls_back_to_shasum_where_sha256sum_is_absent() {
  machine Darwin x86_64
  make_release ayeaye-x86_64-apple-darwin
  # shasum first, while the real sha256sum can still be resolved to back it.
  REAL_SHA256SUM="$(PATH="$HARNESS_HOST_PATH" command -v sha256sum)"
  export REAL_SHA256SUM
  stub_script shasum <<'SH'
# shasum -a 256 <file>, answered by the host's sha256sum.
exec "$REAL_SHA256SUM" "$3"
SH
  stub_remove sha256sum
  run_script "$INSTALL"
  assert_status 0 "$RUN_STATUS" "$RUN_OUTPUT"
  assert_contains "$RUN_STDOUT" "fake ayeaye ran: setup"
  assert_stub_called_with shasum "-a 256" "the fallback names its algorithm"
}

# --------------------------------------------------------- install & handoff

# AYEAYE-63/S3: release assets carry no exec bit; the installed binary must
# be executable, live under AYEAYE_BIN_DIR, and receive the forwarded args.
test_the_binary_lands_executable_and_setup_gets_the_forwarded_args() {
  machine Linux x86_64
  make_release ayeaye-x86_64-unknown-linux-musl
  run_script "$INSTALL" --yes --no-model
  assert_status 0 "$RUN_STATUS" "$RUN_OUTPUT"
  assert_file_exists "$AYEAYE_BIN_DIR/ayeaye"
  [ -x "$AYEAYE_BIN_DIR/ayeaye" ] || fail "the installed binary is not executable"
  assert_contains "$RUN_STDOUT" "fake ayeaye ran: setup --yes --no-model" \
    "arguments pass through to setup, so 'bash -s -- --yes' still works"
}

# AYEAYE-63: re-running the installer is the upgrade path - a binary already
# in place is replaced, not defended.
test_rerunning_replaces_the_installed_binary() {
  machine Linux x86_64
  make_release ayeaye-x86_64-unknown-linux-musl
  mkdir -p "$AYEAYE_BIN_DIR"
  printf '#!/bin/sh\necho the old release\n' > "$AYEAYE_BIN_DIR/ayeaye"
  chmod +x "$AYEAYE_BIN_DIR/ayeaye"
  run_script "$INSTALL"
  assert_status 0 "$RUN_STATUS" "$RUN_OUTPUT"
  assert_contains "$RUN_STDOUT" "fake ayeaye ran: setup"
  assert_file_not_contains "$AYEAYE_BIN_DIR/ayeaye" "the old release"
}

# ------------------------------------------------------------- the contract

# AYEAYE-63/S4: no interpreter beyond the shell itself - and the script stays
# the size the spec promised: a downloader, not a wizard.
test_the_script_is_plain_shell_and_stays_small() {
  assert_eq "#!/bin/sh" "$(head -1 "$INSTALL")" \
    "POSIX sh, not bash: the shell itself is the only interpreter"
  for tool in python python3 perl ruby node; do
    assert_not_contains "$(cat "$INSTALL")" "$tool" \
      "the installer must not reach for $tool"
  done
  lines="$(wc -l < "$INSTALL" | tr -d ' ')"
  [ "$lines" -le 100 ] \
    || fail "install.sh is $lines lines; the spec promised roughly fifty, and 100 is the leash"
}

# AYEAYE-63/S2: a download that never arrives is a failure, not an install -
# the stub curl answers a missing name with 22, as curl -f does a 404.
test_a_missing_release_artifact_is_a_failure_and_installs_nothing() {
  machine Linux x86_64
  # No release published at all: the artifact fetch 404s.
  run_script "$INSTALL"
  assert_ne 0 "$RUN_STATUS" "a 404 must not look like an install"
  assert_file_missing "$AYEAYE_BIN_DIR/ayeaye"
}

# AYEAYE-63/S2: with no checksum tool at all, the download is refused by
# name rather than half-verified or run unchecked.
test_no_checksum_tool_at_all_is_a_named_refusal() {
  machine Linux x86_64
  make_release ayeaye-x86_64-unknown-linux-musl
  stub_remove sha256sum shasum
  run_script "$INSTALL"
  assert_status 1 "$RUN_STATUS"
  assert_contains "$RUN_STDERR" "nothing can verify the download"
  assert_file_missing "$AYEAYE_BIN_DIR/ayeaye"
}

# AYEAYE-63/S3: piped through bash, stdin is the script and already spent -
# the handoff must hand setup the terminal instead, or its two questions
# would be answered by an empty pipe. Driven under a real pty with stdin
# pointed away from it, which is exactly the curl-pipe shape.
test_handoff_reattaches_the_terminal_when_a_pipe_spent_stdin() {
  machine Linux x86_64
  make_release ayeaye-x86_64-unknown-linux-musl
  cat > "$TEST_TMPDIR/piped-install" <<SH
#!/bin/sh
exec "$INSTALL" < /dev/null
SH
  chmod +x "$TEST_TMPDIR/piped-install"
  pty_run "$TEST_TMPDIR/piped-install"
  assert_eq 0 "$PTY_STATUS" "$PTY_TRANSCRIPT"
  assert_contains "$PTY_TRANSCRIPT" "stdin-is-a-terminal: yes" \
    "setup must get /dev/tty back, or it would take the spent pipe for a person saying no"
}
