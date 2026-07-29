# What the platform library says this machine is.
#
# Every answer here comes from a fixture, never from the machine running the
# suite: the os-release path is an overridable variable and uname/sw_vers are
# stubs, so a contributor on Arch and a container on Debian get identical
# results. The one thing that is asserted about the real machine is that
# sourcing the library does not touch it.

# Source the library, or reset it if a previous call in this test already did.
# Sourcing happens inside the test rather than in setup, so that it happens
# under `set -u` - a consumer sourcing this from a `set -euo pipefail` script
# is the normal case, not the exotic one.
_platform_load() {
  if declare -f platform_reset >/dev/null 2>&1; then
    platform_reset
  else
    . "$REPO_ROOT/lib/platform.sh"
  fi
}

# _platform_from <os-release-fixture|none> - point detection at a fixture.
#
# The fixture is asserted to exist before it is used. fixture_file prints
# nothing for a name that is not there, which would leave the candidate list
# empty - and an empty list means "this machine has no os-release", not "read
# the real one", so a typo here would silently pin the developer's own distro.
_platform_from() {
  if [ "$1" = "none" ]; then
    PLATFORM_OS_RELEASE_FILES=""
  else
    assert_fixture_exists "os-release/$1"
    PLATFORM_OS_RELEASE_FILES="$(fixture_file "os-release/$1")"
  fi
  export PLATFORM_OS_RELEASE_FILES
  _platform_load
}

# _stub_uname <sysname> <machine> - uname -s and uname -m, nothing else.
_stub_uname() {
  stub_command uname --exit 1 --stderr "uname: unsupported option"
  stub_when uname '-s' --stdout "$1"
  stub_when uname '-m' --stdout "$2"
}

# _assert_family <fixture> <expected-family> - the whole point of the file.
_assert_family() {
  _platform_from "$1"
  assert_eq "$2" "$(platform_family)" "os-release/$1 belongs to the $2 family"
}

# --------------------------------------------------------------- inertness

test_sourcing_the_library_runs_no_command_at_all() {
  stub_command uname
  stub_command sw_vers
  stub_command systemctl
  stub_command launchctl
  stub_command brew
  stub_command apt-get
  stub_command dnf
  stub_command pacman
  stub_command zypper
  stub_command dirname
  stub_command basename
  stub_command readlink

  local probe="$TEST_TMPDIR/probe.sh"
  printf '%s\n' '#!/usr/bin/env bash' 'set -euo pipefail' \
    ". \"\$REPO_ROOT/lib/platform.sh\"" > "$probe"
  chmod +x "$probe"

  local before after
  before="$(find "$HOME" | sort)"
  run_script "$probe"
  after="$(find "$HOME" | sort)"

  assert_status 0 "$RUN_STATUS" "the library must be safe to source under set -euo pipefail"
  assert_eq "" "$RUN_STDOUT" "sourcing must print nothing"
  assert_eq "" "$RUN_STDERR" "sourcing must print nothing"
  assert_eq "$before" "$after" "sourcing must not create or remove a file"

  local cmd
  # dirname and basename among them: the harness links the real ones in, so a
  # library that shelled out to work out its own directory would go unnoticed
  # by a list of package managers alone.
  for cmd in uname sw_vers systemctl launchctl brew apt-get dnf pacman zypper \
             dirname basename readlink; do
    assert_stub_not_called "$cmd" "sourcing must not probe anything; detection is on demand"
  done
}

test_a_consumer_under_set_u_never_meets_an_unbound_variable() {
  # The wizard runs under `set -euo pipefail`, so every tunable this library
  # reads has to survive the caller never having heard of it. Only the
  # os-release path is set below - and only so the run cannot read the real
  # machine's; PLATFORM_BREW_PREFIXES is deliberately left unset.
  _stub_uname Linux x86_64
  PLATFORM_OS_RELEASE_FILES=""
  export PLATFORM_OS_RELEASE_FILES

  local probe="$TEST_TMPDIR/probe.sh"
  printf '%s\n' '#!/usr/bin/env bash' 'set -euo pipefail' \
    ". \"\$REPO_ROOT/lib/platform.sh\"" 'platform_detect' \
    'printf "%s %s %s\n" "$(platform_family)" "$(platform_arch)" "$(platform_pkg_manager)"' \
    > "$probe"
  chmod +x "$probe"
  run_script "$probe"

  assert_status 0 "$RUN_STATUS"
  assert_eq "" "$RUN_STDERR" "an unbound variable would land here"
  assert_eq "unknown x86_64 none" "$RUN_STDOUT"
}

test_detection_happens_on_demand_and_is_cached() {
  _stub_uname Linux x86_64
  _platform_from debian-12
  assert_stub_not_called uname "sourcing and resetting ask nothing"

  platform_detect
  assert_stub_call_count uname 2 "detection probes once: uname -s and uname -m"

  assert_eq "x86_64" "$(platform_arch)"
  assert_eq "linux" "$(platform_os)"
  assert_eq "debian" "$(platform_family)"
  assert_stub_call_count uname 2 "every later question is served from the cache"
}

test_a_cache_filled_only_inside_a_subshell_does_not_survive_it() {
  # Documented, not incidental: `$(platform_family)` runs in a subshell, and a
  # subshell's variables die with it. A consumer that never calls
  # platform_detect in its own shell re-probes on every question - which is
  # correct, just not free, and is why platform_detect exists as a public
  # function at all.
  _stub_uname Linux x86_64
  _platform_from debian-12
  assert_eq "debian" "$(platform_family)"
  assert_eq "debian" "$(platform_family)"
  assert_stub_call_count uname 4 "two subshell questions, two probes"

  platform_detect
  assert_eq "debian" "$(platform_family)"
  assert_stub_call_count uname 6 "one more probe to fill the cache, then no more"
}

test_a_reset_makes_the_library_look_again() {
  _stub_uname Linux x86_64
  _platform_from debian-12
  assert_eq "debian" "$(platform_family)"

  _platform_from fedora-40
  assert_eq "fedora" "$(platform_family)" "a reset must re-read the new os-release"
  assert_eq "fedora" "$(platform_id)"
}

# ----------------------------------------------------------- linux families

test_the_debian_family_covers_its_derivatives() {
  _assert_family debian-12 debian
  _assert_family ubuntu-24.04 debian
  _assert_family raspbian-12 debian
}

test_a_distro_nobody_here_has_heard_of_is_placed_by_id_like() {
  # Trisquel is deliberately absent from the id table. The only thing that can
  # place it is ID_LIKE=ubuntu - and ubuntu is itself only debian by the same
  # rule, so this also pins that the lookup recurses through the one table
  # rather than mapping ID_LIKE separately.
  _assert_family trisquel-11 debian
  assert_eq "trisquel" "$(platform_id)" "it is still named after itself"
}

test_the_id_like_list_is_searched_past_its_first_token() {
  _stub_uname Linux x86_64
  local path="$TEST_TMPDIR/os-release"
  printf '%s\n' 'ID=speculative' 'ID_LIKE="madeup alsomadeup debian"' > "$path"
  PLATFORM_OS_RELEASE_FILES="$path"
  export PLATFORM_OS_RELEASE_FILES
  _platform_load
  assert_eq "debian" "$(platform_family)" \
    "a distro that lists three parents must be placed by whichever one we know"
}

test_a_derivative_named_in_the_table_does_not_need_id_like() {
  _assert_family linuxmint-21 debian
}

test_the_fedora_family_covers_the_rhel_rebuilds() {
  _assert_family fedora-40 fedora
  _assert_family rocky-9 fedora
}

test_the_arch_family_covers_its_derivatives() {
  _assert_family arch arch
  _assert_family manjaro arch
}

test_the_suse_family_covers_leap_and_tumbleweed() {
  _assert_family opensuse-leap-15.6 suse
  _assert_family opensuse-tumbleweed suse
}

test_an_unrecognised_distro_is_unknown_rather_than_guessed_at() {
  _assert_family alpine-3.20 unknown
  _assert_family void unknown
  _assert_family nixos-24.05 unknown
}

test_an_unrecognised_distro_still_reports_its_own_identity() {
  _platform_from alpine-3.20
  assert_eq "unknown" "$(platform_family)"
  assert_eq "alpine" "$(platform_id)" "the wizard has to be able to name it in the fallback text"
  assert_eq "3.20.3" "$(platform_version)"
  assert_eq "Alpine Linux v3.20" "$(platform_pretty)"
  assert_eq "linux" "$(platform_os)" "it is still linux, just not a family we can act on"
}

# ------------------------------------------------------- identity and shape

test_the_distro_identity_comes_out_of_os_release() {
  _platform_from ubuntu-24.04
  assert_eq "ubuntu" "$(platform_id)"
  assert_eq "24.04" "$(platform_version)" "quoted values arrive unquoted"
  assert_eq "Ubuntu 24.04.1 LTS" "$(platform_pretty)"
}

test_an_unversioned_rolling_release_reports_no_version_rather_than_junk() {
  _platform_from arch
  assert_eq "arch" "$(platform_id)"
  assert_eq "" "$(platform_version)" "Arch has no VERSION_ID"
  assert_eq "Arch Linux" "$(platform_pretty)"
}

test_commented_out_lines_in_os_release_are_not_values() {
  # The real tumbleweed file carries a commented VERSION and two commented
  # CPE_NAMEs above the live one.
  _platform_from opensuse-tumbleweed
  assert_eq "opensuse-tumbleweed" "$(platform_id)"
  assert_eq "20260724" "$(platform_version)"
  assert_eq "openSUSE Tumbleweed" "$(platform_pretty)"
}

test_a_missing_os_release_is_unknown_and_not_an_error() {
  _stub_uname Linux x86_64
  _platform_from none
  assert_eq "unknown" "$(platform_family)"
  assert_eq "unknown" "$(platform_id)"
  assert_eq "" "$(platform_version)"
  assert_eq "linux" "$(platform_os)" "uname still knows what kernel this is"
}

test_an_os_release_without_an_id_is_unknown_too() {
  _stub_uname Linux x86_64
  local path="$TEST_TMPDIR/os-release"
  printf '%s\n' 'PRETTY_NAME="Something Else"' 'HOME_URL="https://example.invalid/"' > "$path"
  PLATFORM_OS_RELEASE_FILES="$path"
  export PLATFORM_OS_RELEASE_FILES
  _platform_load
  assert_eq "unknown" "$(platform_id)"
  assert_eq "unknown" "$(platform_family)"
}

test_a_windows_line_ending_does_not_become_part_of_the_value() {
  _stub_uname Linux x86_64
  local path="$TEST_TMPDIR/os-release"
  printf 'ID=debian\r\nVERSION_ID="12"\r\n' > "$path"
  PLATFORM_OS_RELEASE_FILES="$path"
  export PLATFORM_OS_RELEASE_FILES
  _platform_load
  assert_eq "debian" "$(platform_id)"
  assert_eq "12" "$(platform_version)"
}

test_neither_kernel_nor_os_release_is_unknown_everything() {
  # uname absent entirely: the bare-PATH case, and the honest answer is that
  # this machine cannot be identified.
  assert_command_absent uname
  _platform_from none
  assert_eq "unknown" "$(platform_os)"
  assert_eq "unknown" "$(platform_family)"
  assert_eq "unknown" "$(platform_arch)"
}

test_os_release_alone_identifies_linux_without_uname() {
  assert_command_absent uname
  _platform_from fedora-40
  assert_eq "linux" "$(platform_os)" "a readable os-release is proof enough of linux"
  assert_eq "fedora" "$(platform_family)"
}

# ------------------------------------------------------------------ macos

test_macos_is_recognised_from_uname_and_sw_vers() {
  _stub_uname Darwin arm64
  stub_command_from_fixture sw_vers sw_vers/macos-15.1
  _platform_from none
  assert_eq "macos" "$(platform_os)"
  assert_eq "macos" "$(platform_family)"
  assert_eq "macos" "$(platform_id)"
  assert_eq "15.1" "$(platform_version)"
  assert_eq "macOS 15.1" "$(platform_pretty)"
}

test_an_older_macos_reports_its_own_version() {
  _stub_uname Darwin x86_64
  stub_command_from_fixture sw_vers sw_vers/macos-13.6
  _platform_from none
  assert_eq "macos" "$(platform_family)"
  assert_eq "13.6" "$(platform_version)"
  assert_eq "x86_64" "$(platform_arch)"
}

test_macos_without_sw_vers_is_still_macos() {
  _stub_uname Darwin arm64
  assert_command_absent sw_vers
  _platform_from none
  assert_eq "macos" "$(platform_family)" "the kernel name alone is enough to know the family"
  assert_eq "" "$(platform_version)"
  assert_eq "macOS" "$(platform_pretty)"
}

test_an_os_release_on_a_darwin_kernel_does_not_win_over_uname() {
  # Nix on macOS can leave a linux-looking os-release around. The kernel is
  # the stronger signal.
  _stub_uname Darwin arm64
  stub_command_from_fixture sw_vers sw_vers/macos-15.1
  _platform_from nixos-24.05
  assert_eq "macos" "$(platform_os)"
  assert_eq "macos" "$(platform_family)"
}

# ------------------------------------------------------------ architecture

test_the_two_intel_spellings_normalise_to_one() {
  _stub_uname Linux x86_64
  _platform_from debian-12
  assert_eq "x86_64" "$(platform_arch)"

  _stub_uname Linux amd64
  _platform_from debian-12
  assert_eq "x86_64" "$(platform_arch)" "amd64 is the same machine by another name"
}

test_the_two_arm_spellings_normalise_to_one() {
  _stub_uname Linux aarch64
  _platform_from raspbian-12
  assert_eq "arm64" "$(platform_arch)" "linux says aarch64"

  _stub_uname Darwin arm64
  stub_command_from_fixture sw_vers sw_vers/macos-15.1
  _platform_from none
  assert_eq "arm64" "$(platform_arch)" "apple silicon says arm64"
}

test_an_unfamiliar_architecture_is_passed_through_not_flattened() {
  _stub_uname Linux riscv64
  _platform_from debian-12
  assert_eq "riscv64" "$(platform_arch)" \
    "reporting an architecture we have no opinion about beats calling it unknown"
}

test_a_32_bit_userland_on_a_64_bit_kernel_is_not_called_arm64() {
  # armv8l is 32-bit Raspberry Pi OS on an arm64 kernel. Calling it arm64
  # hands whoever picks a binary by architecture the wrong one.
  _stub_uname Linux armv8l
  _platform_from raspbian-12
  assert_eq "armv8l" "$(platform_arch)"
  assert_ne "arm64" "$(platform_arch)"
}

# ---------------------------------------------------------------- summary

test_the_summary_names_everything_the_wizard_reports() {
  _stub_uname Linux x86_64
  _platform_from ubuntu-24.04
  local summary
  summary="$(platform_summary)"
  assert_contains "$summary" "Ubuntu 24.04.1 LTS"
  assert_contains "$summary" "debian"
  assert_contains "$summary" "x86_64"
  assert_eq "$summary" "$(printf '%s\n' "$summary" | head -1)" \
    "the summary is one line, because it is one line of wizard output"
}

test_the_summary_of_an_unknown_platform_says_so_plainly() {
  _stub_uname Linux x86_64
  _platform_from void
  local summary
  summary="$(platform_summary)"
  assert_contains "$summary" "void" "it still names what it found"
  assert_contains "$summary" "(unknown)" "and says the family is not one it knows"
  assert_contains "$summary" "packages: none"
  assert_contains "$summary" "services: none"
}

# --------------------------------------------------- os-release, harder cases

test_the_candidate_paths_are_tried_in_order() {
  # The real default is "/etc/os-release /usr/lib/os-release", and which one
  # wins matters: a distro that ships both expects the first to shadow the
  # second.
  _stub_uname Linux x86_64
  local first="$TEST_TMPDIR/etc-os-release" second="$TEST_TMPDIR/usrlib-os-release"
  printf 'ID=debian\n' > "$first"
  printf 'ID=fedora\n' > "$second"

  PLATFORM_OS_RELEASE_FILES="$first $second"
  export PLATFORM_OS_RELEASE_FILES
  _platform_load
  assert_eq "debian" "$(platform_id)"

  rm -f "$first"
  _platform_load
  assert_eq "fedora" "$(platform_id)" "the fallback is used only once the first is gone"
}

test_an_empty_candidate_list_means_no_os_release_not_the_real_one() {
  # This is the property the whole suite's isolation rests on. If an empty
  # list fell back to /etc/os-release, every fixture-driven test here would
  # quietly start reporting the machine it happens to run on.
  _stub_uname Linux x86_64
  PLATFORM_OS_RELEASE_FILES=""
  export PLATFORM_OS_RELEASE_FILES
  _platform_load
  assert_eq "unknown" "$(platform_id)"
  assert_eq "unknown" "$(platform_family)"
}

test_a_value_with_trailing_blanks_is_still_the_value() {
  _stub_uname Linux x86_64
  local path="$TEST_TMPDIR/os-release"
  printf '%s\n' 'ID="fedora"   ' 'VERSION_ID=40  ' > "$path"
  PLATFORM_OS_RELEASE_FILES="$path"
  export PLATFORM_OS_RELEASE_FILES
  _platform_load
  assert_eq "fedora" "$(platform_id)" "trailing blanks must not defeat the quote stripping"
  assert_eq "fedora" "$(platform_family)"
  assert_eq "40" "$(platform_version)"
}

test_a_trailing_comment_is_not_part_of_an_unquoted_value() {
  _stub_uname Linux x86_64
  local path="$TEST_TMPDIR/os-release"
  printf '%s\n' 'ID=arch # rolling' > "$path"
  PLATFORM_OS_RELEASE_FILES="$path"
  export PLATFORM_OS_RELEASE_FILES
  _platform_load
  assert_eq "arch" "$(platform_id)" "the shell would read it this way too"
  assert_eq "arch" "$(platform_family)"
}

test_an_id_in_capitals_is_matched_all_the_same() {
  # Deepin has shipped ID=Deepin. An id is compared, not displayed.
  _stub_uname Linux x86_64
  local path="$TEST_TMPDIR/os-release"
  printf '%s\n' 'ID=Deepin' 'PRETTY_NAME="deepin 23"' > "$path"
  PLATFORM_OS_RELEASE_FILES="$path"
  export PLATFORM_OS_RELEASE_FILES
  _platform_load
  assert_eq "debian" "$(platform_family)"
  assert_eq "deepin" "$(platform_id)"
}

test_an_os_release_that_is_not_a_file_is_quiet_about_it() {
  # A path pointing at a directory passes a naive readability check and then
  # fails inside the read loop, on the consumer's stderr, from a library that
  # promised to be quiet.
  _stub_uname Linux x86_64
  mkdir -p "$TEST_TMPDIR/a-directory"
  PLATFORM_OS_RELEASE_FILES="$TEST_TMPDIR/a-directory"
  export PLATFORM_OS_RELEASE_FILES

  local probe="$TEST_TMPDIR/probe.sh"
  printf '%s\n' '#!/usr/bin/env bash' 'set -euo pipefail' \
    ". \"\$REPO_ROOT/lib/platform.sh\"" 'platform_detect' \
    'printf "%s\n" "$(platform_id)"' > "$probe"
  chmod +x "$probe"
  run_script "$probe"

  assert_status 0 "$RUN_STATUS"
  assert_eq "" "$RUN_STDERR" "no read error, no unbound variable"
  assert_eq "unknown" "$RUN_STDOUT"
}

test_a_pretty_name_that_is_missing_falls_back_to_the_id() {
  _stub_uname Linux x86_64
  local path="$TEST_TMPDIR/os-release"
  printf '%s\n' 'ID=debian' > "$path"
  PLATFORM_OS_RELEASE_FILES="$path"
  export PLATFORM_OS_RELEASE_FILES
  _platform_load
  assert_eq "debian" "$(platform_pretty)" "there has to be something to print"
}

test_sourcing_the_library_twice_keeps_an_answer_already_worked_out() {
  _stub_uname Linux x86_64
  _platform_from debian-12
  platform_detect
  assert_stub_call_count uname 2

  . "$REPO_ROOT/lib/platform.sh"
  assert_eq "debian" "$(platform_family)"
  assert_stub_call_count uname 2 "a second source must not throw the cache away"
}
