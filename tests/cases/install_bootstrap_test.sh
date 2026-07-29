# The front door: what install.sh does before the wizard exists.
#
# Two entry conditions, and they have to stay different. From a clone the
# script has the whole repository next to it and must configure that clone
# without downloading a byte. Piped from the internet it has nothing next to
# it at all - not even a real path in BASH_SOURCE - and has to fetch a pinned
# release, check it, and hand over to the copy it just unpacked.
#
# Everything here is served locally. A test that reaches github.com is a test
# that fails on a train.

setup() {
  stub_real tar gzip sha256sum
  RELEASE_DIR="$TEST_TMPDIR/release"
  mkdir -p "$RELEASE_DIR"
  # The suite is usually started from a terminal, and a bootstrap that found
  # one would stop dead on its own question. Point it at a terminal that is
  # not there; the pty test below points it back at a real one.
  AYEAYE_BOOTSTRAP_TTY="$TEST_TMPDIR/no-terminal"
  export AYEAYE_BOOTSTRAP_TTY
}

# ------------------------------------------------------------------ helpers

# The pinned version install.sh really carries, read out of the script so
# these tests keep working across a release rather than pinning a copy of it.
pinned_version() {
  sed -n 's/^AYEAYE_VERSION="\(.*\)"$/\1/p' "$REPO_ROOT/install.sh"
}

# lone_install -> a directory holding install.sh and nothing else, which is
# the shape of the machine a piped run lands on: the script, no payload.
lone_install() {
  local dir="$TEST_TMPDIR/lone"
  mkdir -p "$dir"
  if [ ! -f "$dir/install.sh" ]; then
    cp "$REPO_ROOT/install.sh" "$dir/install.sh"
    chmod +x "$dir/install.sh"
  fi
  printf '%s' "$dir"
}

# pin_sha256 <hex> - write a checksum into the copy under test, the way the
# release process writes one into the script it publishes. That path cannot be
# reached any other way: in the repository the line is deliberately empty.
pin_sha256() {
  local dir
  dir="$(lone_install)"
  sed "s/^AYEAYE_SHA256=\"\"$/AYEAYE_SHA256=\"$1\"/" "$dir/install.sh" > "$dir/pinned.sh"
  mv "$dir/pinned.sh" "$dir/install.sh"
  chmod +x "$dir/install.sh"
  assert_file_contains "$dir/install.sh" "AYEAYE_SHA256=\"$1\""
}

# digest_of <file> - the sha256 a test compares against.
digest_of() {
  sha256sum "$1" | awk '{print $1}'
}

# fake_payload <dir> - the smallest tree install.sh will accept as ayeaye.
# Its install.sh reports what it was given, which is how the re-exec and the
# arguments that survive it are observed.
fake_payload() {
  local dir="$1"
  mkdir -p "$dir/lib" "$dir/bin"
  : > "$dir/lib/wizard.sh"
  : > "$dir/bin/ayeaye"
  cat > "$dir/install.sh" <<'SH'
#!/usr/bin/env bash
printf 'unpacked copy ran\n'
printf 'args: %s\n' "$*"
printf 'stdin-is-a-terminal: %s\n' "$([ -t 0 ] && echo yes || echo no)"
printf 'unattended: %s\n' "${AYEAYE_BOOTSTRAP_UNATTENDED:-unset}"
exit 0
SH
  chmod +x "$dir/install.sh"
}

# release_tarball [dir] -> builds the artifact the release would publish and
# echoes its path. Shaped like a real one: a single top-level directory.
release_tarball() {
  local src="${1:-}"
  local top="ayeaye-$(pinned_version)"
  local work="$TEST_TMPDIR/build"
  rm -rf "$work"
  mkdir -p "$work/$top"
  if [ -n "$src" ]; then
    cp -R "$src/." "$work/$top/"
  else
    fake_payload "$work/$top"
  fi
  local out="$RELEASE_DIR/$top.tar.gz"
  ( cd "$work" && tar -czf "$out" "$top" )
  printf '%s' "$out"
}

# serve <tarball> [sums-file] - a curl that answers from the local release
# directory instead of the internet. Every call is still recorded, so what was
# asked for is assertable.
serve() {
  local tarball="$1" sums="${2:-}"
  SERVE_TARBALL="$tarball"
  SERVE_SUMS="$sums"
  export SERVE_TARBALL SERVE_SUMS
  stub_script curl <<'SH'
out=""
url=""
resume=0
while [ "$#" -gt 0 ]; do
  case "$1" in
    -o|--output)  out="$2"; shift 2 ;;
    -C)           resume=1; shift 2 ;;
    --retry)      shift 2 ;;
    -*)           shift ;;
    *)            url="$1"; shift ;;
  esac
done
case "$url" in
  *SHA256SUMS)
    [ -n "$SERVE_SUMS" ] || exit 22
    cp "$SERVE_SUMS" "$out"
    exit 0 ;;
esac
[ -f "$SERVE_TARBALL" ] || exit 22
if [ "$resume" = 1 ]; then
  # 33 is what curl says when the server will not do ranges.
  [ -z "${SERVE_NO_RESUME:-}" ] || exit 33
  have="$(wc -c < "$out" | tr -d " ")"
  tail -c "+$((have + 1))" "$SERVE_TARBALL" >> "$out"
else
  cp "$SERVE_TARBALL" "$out"
fi
SH
}

# sums_file <tarball> - the SHA256SUMS a release publishes beside it.
sums_file() {
  local out="$RELEASE_DIR/SHA256SUMS"
  ( cd "$(dirname "$1")" && sha256sum "$(basename "$1")" > "$out" )
  printf '%s' "$out"
}

payload_dir() {
  printf '%s/ayeaye/releases/%s' "$XDG_DATA_HOME" "$(pinned_version)"
}

# run_lone [args...] - the script on a machine that has no ayeaye on it.
run_lone() {
  local dir
  dir="$(lone_install)"
  run_script "$dir/install.sh" "$@"
}

# ------------------------------------------------- from a clone, nothing moves

test_a_run_from_the_clone_downloads_nothing() {
  # The whole point of the detection: a contributor's checkout is already the
  # payload, so the bootstrap must not even consider fetching one.
  stub_command curl
  stub_command tmux
  stub_real python3
  run_install --defaults --no-systemd
  # Deliberately about the release and not about curl in general: sibling
  # tickets are adding steps that may legitimately want to fetch something,
  # and this test is not the place that decides whether they may.
  assert_not_contains "$(stub_calls curl 2>/dev/null || true)" "ayeaye-$(pinned_version)" \
    "a clone has the files already"
  assert_file_missing "$XDG_DATA_HOME/ayeaye/releases" \
    "and unpacks no release of itself"
}

test_the_version_it_installs_is_pinned_and_readable_in_the_script() {
  # Not "main". Somebody has to be able to see which ayeaye they are getting,
  # and every other test here reads it back out of the script.
  local version
  version="$(pinned_version)"
  assert_ne "" "$version" "install.sh must carry an explicit AYEAYE_VERSION"
  assert_matches "$version" '^v[0-9]+\.[0-9]+\.[0-9]+$'
}

test_help_from_the_clone_is_still_answered_before_anything_else() {
  stub_command curl
  run_install --help
  assert_status 0 "$RUN_STATUS"
  assert_contains "$RUN_STDOUT" "One-command setup for ayeaye"
  assert_stub_not_called curl
  assert_eq "" "$(ls -A "$XDG_DATA_HOME")" "--help must not create so much as a directory"
}

test_help_is_answered_even_with_no_payload_next_to_the_script() {
  # Arguments are parsed before the payload is looked for, so the one flag a
  # nervous person types first works whichever way the script arrived.
  stub_command curl
  local dir
  dir="$(lone_install)"
  run_script "$dir/install.sh" --help
  assert_status 0 "$RUN_STATUS"
  assert_contains "$RUN_STDOUT" "One-command setup for ayeaye"
  assert_stub_not_called curl
}

test_an_unknown_option_is_rejected_before_the_payload_is_looked_for() {
  stub_command curl
  local dir
  dir="$(lone_install)"
  run_script "$dir/install.sh" --frobnicate
  assert_status 2 "$RUN_STATUS"
  assert_contains "$RUN_STDERR" "unknown option: --frobnicate"
  assert_stub_not_called curl
}

test_a_script_with_no_payload_beside_it_takes_the_bootstrap_path() {
  # The detection itself: same file, no repository around it.
  stub_command curl
  local dir
  dir="$(lone_install)"
  run_script "$dir/install.sh" --defaults
  assert_not_contains "$RUN_OUTPUT" "No such file or directory" \
    "it must not try to source a library that is not next to it"
  assert_contains "$RUN_OUTPUT" "ayeaye $(pinned_version)" \
    "it says which version it would fetch"
}

# ------------------------------------------- a pinned release, asked for first

test_the_release_it_asks_for_is_the_pinned_one() {
  local version
  version="$(pinned_version)"
  serve "$(release_tarball)"
  run_lone --yes
  assert_status 0 "$RUN_STATUS"
  assert_stub_called_with curl "releases/download/$version/ayeaye-$version.tar.gz" \
    "the version is in the URL, and it is not a branch"
  assert_not_contains "$(stub_calls curl)" "/main/" "nothing tracks a branch"
}

test_a_run_that_may_not_ask_refuses_the_download_and_fetches_nothing() {
  # --defaults is "take every default without asking me", and downloading is
  # not a thing to do to somebody on a default.
  serve "$(release_tarball)"
  run_lone --defaults
  assert_status 3 "$RUN_STATUS"
  assert_stub_not_called curl
  assert_contains "$RUN_OUTPUT" "nothing was downloaded, and this computer is exactly as it was."
  assert_contains "$RUN_OUTPUT" "--yes" "it says how an unattended run answers this"
  assert_file_missing "$(payload_dir)"
}

test_with_no_terminal_and_no_yes_it_refuses_rather_than_guessing() {
  serve "$(release_tarball)"
  run_lone
  assert_status 3 "$RUN_STATUS"
  assert_stub_not_called curl
  assert_contains "$RUN_OUTPUT" "no way to ask"
}

test_a_computer_with_no_way_to_download_says_so_instead_of_pretending() {
  stub_remove curl
  assert_command_absent curl
  run_lone --yes
  assert_status 1 "$RUN_STATUS"
  assert_contains "$RUN_OUTPUT" "no way to download files (curl is missing)"
  assert_contains "$RUN_OUTPUT" "ayeaye-$(pinned_version).tar.gz" \
    "and names the file, so it can be fetched by hand"
}

test_a_tampered_release_aborts_and_installs_nothing() {
  # The one that matters. The checksum is the one the release published; the
  # bytes that arrive are somebody else's.
  local good sums
  good="$(release_tarball)"
  sums="$(sums_file "$good")"
  printf 'not the release you asked for' > "$good"
  serve "$good" "$sums"
  run_lone --yes
  assert_status 1 "$RUN_STATUS"
  assert_contains "$RUN_OUTPUT" "STOP."
  assert_contains "$RUN_OUTPUT" "is not what ayeaye $(pinned_version) published"
  assert_file_missing "$(payload_dir)" "nothing was unpacked"
  assert_file_missing "$XDG_DATA_HOME/ayeaye/downloads/ayeaye-$(pinned_version).tar.gz" \
    "and what arrived was deleted rather than left lying about"
}

test_a_checksum_that_matches_is_reported_as_checked() {
  local tarball
  tarball="$(release_tarball)"
  serve "$tarball" "$(sums_file "$tarball")"
  run_lone --yes
  assert_status 0 "$RUN_STATUS"
  assert_contains "$RUN_OUTPUT" "checked: what arrived matches the checksum"
  assert_contains "$RUN_OUTPUT" "down the same connection" \
    "and says where that checksum came from, because it makes a difference"
  assert_contains "$RUN_OUTPUT" "unpacked copy ran"
}

test_a_release_with_no_published_checksum_says_what_was_not_checked() {
  # Silence here would read as "verified". It has to say which of the two
  # things actually happened.
  serve "$(release_tarball)"
  run_lone --yes
  assert_status 0 "$RUN_STATUS"
  assert_contains "$RUN_OUTPUT" "not checked: no checksums could be fetched"
  assert_contains "$RUN_OUTPUT" "encrypted connection"
  assert_contains "$RUN_OUTPUT" "unpacked copy ran" "and it still installs"
}

test_a_computer_that_cannot_compute_a_checksum_says_so_too() {
  local tarball
  tarball="$(release_tarball)"
  serve "$tarball" "$(sums_file "$tarball")"
  stub_remove sha256sum
  assert_command_absent shasum
  assert_command_absent openssl
  run_lone --yes
  assert_status 0 "$RUN_STATUS"
  assert_contains "$RUN_OUTPUT" "not checked: this computer has no sha256sum"
}

# -------------------------------------------- unpack, resume, and the handover

test_the_arguments_survive_the_handover() {
  # What the person typed is what the wizard has to see, in the order they
  # typed it. The unpacked copy prints its own argv back.
  serve "$(release_tarball)"
  run_lone --yes --no-systemd --details
  assert_status 0 "$RUN_STATUS"
  assert_contains "$RUN_OUTPUT" "unpacked copy ran"
  assert_contains "$RUN_OUTPUT" "args: --yes --no-systemd --details"
}

test_a_release_already_unpacked_is_used_without_downloading_anything() {
  # Re-runnable and resumable: the second run of an interrupted bootstrap has
  # nothing to fetch, and must not need the network to find that out.
  assert_command_absent curl
  fake_payload "$(payload_dir)"
  run_lone --defaults
  assert_status 0 "$RUN_STATUS"
  assert_contains "$RUN_OUTPUT" "is already on this computer"
  assert_contains "$RUN_OUTPUT" "unpacked copy ran"
  assert_contains "$RUN_OUTPUT" "args: --defaults" "even a run that may not ask carries on"
}

test_an_artifact_an_earlier_run_downloaded_is_checked_rather_than_fetched_again() {
  local tarball
  tarball="$(release_tarball)"
  serve "$tarball" "$(sums_file "$tarball")"
  mkdir -p "$XDG_DATA_HOME/ayeaye/downloads"
  cp "$tarball" "$XDG_DATA_HOME/ayeaye/downloads/ayeaye-$(pinned_version).tar.gz"
  run_lone --yes
  assert_status 0 "$RUN_STATUS"
  assert_contains "$RUN_OUTPUT" "an earlier run already downloaded this release"
  assert_contains "$RUN_OUTPUT" "checked: what arrived matches the checksum"
  assert_stub_call_count curl 1 "only the checksum was fetched, not the release"
  assert_stub_called_with curl "SHA256SUMS"
}

test_a_download_that_stopped_halfway_is_picked_up_where_it_stopped() {
  # A real byte range, not just the flag: the part file holds the first bytes
  # of the release, the stub serves the rest, and what the two add up to has
  # to be the release itself.
  local tarball artifact
  tarball="$(release_tarball)"
  serve "$tarball"
  artifact="$XDG_DATA_HOME/ayeaye/downloads/ayeaye-$(pinned_version).tar.gz"
  mkdir -p "$XDG_DATA_HOME/ayeaye/downloads"
  head -c 64 "$tarball" > "$artifact.part"
  run_lone --yes
  assert_status 0 "$RUN_STATUS"
  assert_stub_called_with curl "-C -" "it asks the server to carry on from the offset"
  assert_eq "$(digest_of "$tarball")" "$(digest_of "$artifact")" \
    "what the resumed download added up to is the release, byte for byte"
  assert_contains "$RUN_OUTPUT" "unpacked copy ran"
  assert_file_missing "$artifact.part" "and the part file became the whole file"
}

test_a_server_that_will_not_resume_is_asked_for_the_whole_thing_once() {
  local tarball artifact
  tarball="$(release_tarball)"
  serve "$tarball"
  SERVE_NO_RESUME=1
  export SERVE_NO_RESUME
  artifact="$XDG_DATA_HOME/ayeaye/downloads/ayeaye-$(pinned_version).tar.gz"
  mkdir -p "$XDG_DATA_HOME/ayeaye/downloads"
  printf 'left over by something else entirely' > "$artifact.part"
  run_lone --yes
  assert_status 0 "$RUN_STATUS"
  assert_stub_call_count curl 3 "the refused resume, the whole file, the checksums"
  assert_eq "$(digest_of "$tarball")" "$(digest_of "$artifact")" \
    "the useless part file was thrown away rather than built on"
  assert_contains "$RUN_OUTPUT" "unpacked copy ran"
}

test_a_copy_that_cannot_find_itself_stops_instead_of_downloading_for_ever() {
  # The loop guard. Without it a release that unpacked into something
  # incomplete would fetch itself again on every handover, for ever.
  serve "$(release_tarball)"
  AYEAYE_BOOTSTRAPPED=1
  export AYEAYE_BOOTSTRAPPED
  run_lone --yes
  assert_status 1 "$RUN_STATUS"
  assert_stub_not_called curl
  assert_contains "$RUN_STDERR" "not complete"
  assert_contains "$RUN_STDERR" "run it again"
}

# ------------------------------------------------------ the piped-stdin trap

test_a_piped_run_still_reaches_the_person_at_the_keyboard() {
  # The whole trap in one test. Under `curl … | bash` standard input is the
  # script itself: a wizard that read it would ask nobody anything, take every
  # default, install nothing and look exactly like a success. So this runs the
  # real thing - the script piped into bash, with no repository anywhere near
  # it - through a terminal, answers the question by typing at it, and checks
  # that the copy handed to afterwards has a terminal of its own.
  require_host_command python3
  serve "$(release_tarball)"
  local dir wrapper
  dir="$(lone_install)"
  wrapper="$TEST_TMPDIR/piped-run"
  cat > "$wrapper" <<SH
#!/usr/bin/env bash
cat "$dir/install.sh" | bash -s -- --no-systemd
SH
  chmod +x "$wrapper"
  AYEAYE_BOOTSTRAP_TTY="/dev/tty"
  export AYEAYE_BOOTSTRAP_TTY

  pty_expect "may I download" "y"
  pty_run "$wrapper"

  assert_contains "$PTY_TRANSCRIPT" "This computer does not have ayeaye on it yet"
  assert_contains "$PTY_TRANSCRIPT" "may I download ayeaye $(pinned_version)?" \
    "the question has to arrive on the terminal, not into the pipe"
  assert_contains "$PTY_TRANSCRIPT" "unpacked copy ran"
  assert_contains "$PTY_TRANSCRIPT" "args: --no-systemd" "arguments survive a real pipe too"
  assert_contains "$PTY_TRANSCRIPT" "stdin-is-a-terminal: yes" \
    "the wizard it hands over to can ask its own questions"
  assert_status 0 "$PTY_STATUS"
}

test_a_piped_run_that_is_answered_no_leaves_the_computer_alone() {
  require_host_command python3
  serve "$(release_tarball)"
  local dir wrapper
  dir="$(lone_install)"
  wrapper="$TEST_TMPDIR/piped-refusal"
  cat > "$wrapper" <<SH
#!/usr/bin/env bash
cat "$dir/install.sh" | bash
SH
  chmod +x "$wrapper"
  AYEAYE_BOOTSTRAP_TTY="/dev/tty"
  export AYEAYE_BOOTSTRAP_TTY

  pty_expect "may I download" "n"
  pty_run "$wrapper"

  assert_contains "$PTY_TRANSCRIPT" "nothing was downloaded"
  assert_stub_not_called curl
  assert_file_missing "$(payload_dir)"
  assert_status 3 "$PTY_STATUS"
}

test_a_handover_with_no_terminal_anywhere_is_unattended_out_loud() {
  # The failure this whole file exists to prevent, in its quietest form: no
  # terminal, so nothing can be asked, so every answer is the default. That is
  # an acceptable way to run and an unacceptable thing to do silently.
  serve "$(release_tarball)"
  run_lone --yes
  assert_status 0 "$RUN_STATUS"
  assert_contains "$RUN_OUTPUT" "there is no terminal here to ask questions on"
  assert_contains "$RUN_OUTPUT" "unattended: 1" \
    "and the copy it hands over to is told, so it cannot pretend it asked"
}

# -------------------------------------------- the checksum a release bakes in

test_a_checksum_written_into_the_script_needs_no_network_at_all() {
  # The strongest of the three verification paths, and the one a release
  # actually ships: the expected checksum arrives inside the script, over the
  # connection the person already chose to trust. With the release already in
  # the cache there is then nothing left to fetch, and nothing to ask about.
  local tarball artifact
  tarball="$(release_tarball)"
  pin_sha256 "$(digest_of "$tarball")"
  artifact="$XDG_DATA_HOME/ayeaye/downloads/ayeaye-$(pinned_version).tar.gz"
  mkdir -p "$XDG_DATA_HOME/ayeaye/downloads"
  cp "$tarball" "$artifact"
  assert_command_absent curl
  run_lone
  assert_status 0 "$RUN_STATUS"
  assert_contains "$RUN_OUTPUT" "checksum written into setup itself" \
    "the strongest of the three, and it says which one it used"
  assert_contains "$RUN_OUTPUT" "unpacked copy ran"
  assert_not_contains "$RUN_OUTPUT" "may I download" "there was nothing to ask about"
}

test_a_cached_artifact_is_checked_against_the_baked_in_sum_rather_than_trusted() {
  # A poisoned cache is the other way in, and it does not depend on being
  # anywhere near the network at the time.
  local tarball artifact
  tarball="$(release_tarball)"
  pin_sha256 "$(digest_of "$tarball")"
  artifact="$XDG_DATA_HOME/ayeaye/downloads/ayeaye-$(pinned_version).tar.gz"
  mkdir -p "$XDG_DATA_HOME/ayeaye/downloads"
  printf 'somebody else got here first' > "$artifact"
  run_lone
  assert_status 1 "$RUN_STATUS"
  assert_contains "$RUN_STDERR" "STOP."
  assert_file_missing "$artifact" "the cache is emptied, not left to be found again"
  assert_file_missing "$(payload_dir)"
}

# ------------------------------------------------ what an archive may contain

test_an_archive_that_would_write_outside_the_place_it_unpacks_is_refused() {
  # There are releases with no checksum to check, so what an archive is allowed
  # to contain cannot rest on having had one. Built with python3 rather than
  # tar, because every tar worth using refuses to create this. Both escapes
  # point inside the sandbox, so a regression here is caught rather than
  # scattered across the machine running the suite.
  require_host_command python3
  stub_real python3
  local evil
  evil="$RELEASE_DIR/ayeaye-$(pinned_version).tar.gz"
  python3 -c 'import io,sys,tarfile
out, top, absolute = sys.argv[1], sys.argv[2], sys.argv[3]
tf = tarfile.open(out, "w:gz")
for name in (top + "/install.sh", top + "/lib/wizard.sh", top + "/bin/ayeaye",
             "../escaped", absolute):
    data = b"escaped\n"
    info = tarfile.TarInfo(name)
    info.size = len(data)
    tf.addfile(info, io.BytesIO(data))
tf.close()' "$evil" "ayeaye-$(pinned_version)" "$TEST_TMPDIR/escaped"
  serve "$evil"
  run_lone --yes
  assert_status 1 "$RUN_STATUS"
  assert_contains "$RUN_STDERR" "would be written outside"
  assert_file_missing "$(payload_dir)"
  assert_file_missing "$TEST_TMPDIR/escaped"
  assert_file_missing "$XDG_DATA_HOME/ayeaye/releases/escaped"
}

test_an_archive_that_cannot_be_unpacked_is_not_kept_to_fail_again() {
  # Keeping it would make the next run fail in exactly the same way without
  # ever fetching a good one: the installer would be wedged for good.
  local artifact
  printf 'this is not a tar file at all' > "$RELEASE_DIR/ayeaye-$(pinned_version).tar.gz"
  serve "$RELEASE_DIR/ayeaye-$(pinned_version).tar.gz"
  artifact="$XDG_DATA_HOME/ayeaye/downloads/ayeaye-$(pinned_version).tar.gz"
  run_lone --yes
  assert_status 1 "$RUN_STATUS"
  assert_contains "$RUN_STDERR" "could not be unpacked"
  assert_file_missing "$artifact"
  assert_file_missing "$(payload_dir)"
}

# --------------------------------------- what the handover tells the wizard

test_the_unattended_marker_really_makes_the_wizard_stop_asking() {
  # The other half of "unattended out loud": the marker the handover exports
  # has to be something this script acts on, or the message was a lie.
  require_host_command python3
  stub_command tmux
  stub_real python3
  AYEAYE_BOOTSTRAP_UNATTENDED=1
  AYEAYE_BOOTSTRAPPED=1
  export AYEAYE_BOOTSTRAP_UNATTENDED AYEAYE_BOOTSTRAPPED
  stdin_lines "9999" "somewhere.example" "https://ntfy.example/t"
  run_install --no-systemd
  assert_status 0 "$RUN_STATUS"
  assert_file_contains "$XDG_CONFIG_HOME/ayeaye/env" "AYEAYE_PORT=8911" \
    "a run that may not ask cannot be steered by what is on standard input"
  assert_file_contains "$XDG_CONFIG_HOME/ayeaye/env" "AYEAYE_BIND=127.0.0.1"
}

# ------------------------------------------------------- the published command

test_the_one_liner_in_the_readme_is_the_one_the_script_is_written_for() {
  # Two places, one command. A README that drifts from the script sends people
  # to a URL that answers with a 404 page and a shell that runs it.
  local url readme
  url="$(sed -n 's/^AYEAYE_INSTALL_URL="\(.*\)"$/\1/p' "$REPO_ROOT/install.sh")"
  assert_ne "" "$url" "install.sh must name the command it is published as"
  readme="$(cat "$REPO_ROOT/README.md")"
  assert_contains "$readme" "curl -fsSL $url | bash"
  assert_contains "$readme" "$url | bash -s -- --yes" \
    "and how to pass it an argument, which is not obvious"
  run_install --help
  assert_contains "$RUN_STDOUT" "curl -fsSL $url | bash" "--help says the same thing"
}

test_the_readme_says_which_version_is_installed_and_that_it_is_checked() {
  local readme
  readme="$(cat "$REPO_ROOT/README.md")"
  assert_contains "$readme" "pinned release"
  assert_contains "$readme" "checksum"
}

test_a_release_that_is_replaced_is_only_let_go_of_once_the_new_one_is_in_place() {
  # An unpack that lands on top of a copy already there must not be able to
  # leave the machine with neither.
  local tarball
  tarball="$(release_tarball)"
  serve "$tarball"
  mkdir -p "$(payload_dir)/lib"
  printf 'the copy that was already here' > "$(payload_dir)/marker"
  : > "$(payload_dir)/lib/wizard.sh"
  # No install.sh, so this is not yet a copy setup would use, and the bootstrap
  # unpacks over it.
  run_lone --yes
  assert_status 0 "$RUN_STATUS"
  assert_contains "$RUN_OUTPUT" "unpacked copy ran"
  assert_file_missing "$(payload_dir)/marker" "the old tree is gone"
  assert_file_missing "$(payload_dir).previous" "and so is the copy it was moved to"
}
