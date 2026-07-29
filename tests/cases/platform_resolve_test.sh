# Who this machine's package manager and service manager actually are.
#
# A family names a package manager; PATH decides whether it is really there.
# Both halves matter, and the second is the one that bites: a container image
# with systemctl and no user session, or a Mac with no Homebrew, are ordinary
# machines the wizard has to cope with rather than errors.

_platform_load() {
  if declare -f platform_reset >/dev/null 2>&1; then
    platform_reset
  else
    . "$REPO_ROOT/lib/platform.sh"
  fi
}

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

_stub_uname() {
  stub_command uname --exit 1 --stderr "uname: unsupported option"
  stub_when uname '-s' --stdout "$1"
  stub_when uname '-m' --stdout "$2"
}

# A macOS with nothing else on it. Homebrew and launchctl are added per test.
_stub_macos() {
  _stub_uname Darwin arm64
  stub_command_from_fixture sw_vers sw_vers/macos-15.1
}

# ------------------------------------------------------- package managers

test_each_family_reaches_for_its_own_package_manager() {
  _stub_uname Linux x86_64

  stub_command apt-get
  _platform_from debian-12
  assert_eq "apt-get" "$(platform_pkg_manager)"
  stub_remove apt-get

  stub_command dnf
  _platform_from fedora-40
  assert_eq "dnf" "$(platform_pkg_manager)"
  stub_remove dnf

  stub_command pacman
  _platform_from arch
  assert_eq "pacman" "$(platform_pkg_manager)"
  stub_remove pacman

  stub_command zypper
  _platform_from opensuse-tumbleweed
  assert_eq "zypper" "$(platform_pkg_manager)"
}

test_a_derivative_inherits_its_parents_package_manager() {
  _stub_uname Linux aarch64
  stub_command apt-get
  _platform_from raspbian-12
  assert_eq "apt-get" "$(platform_pkg_manager)"

  stub_remove apt-get
  stub_command pacman
  _platform_from manjaro
  assert_eq "pacman" "$(platform_pkg_manager)"
}

test_an_older_redhat_without_dnf_still_has_yum() {
  _stub_uname Linux x86_64
  stub_command yum
  _platform_from rocky-9
  assert_eq "yum" "$(platform_pkg_manager)"

  # dnf wins wherever both are installed, which on a modern Fedora is always.
  stub_command dnf
  _platform_from rocky-9
  assert_eq "dnf" "$(platform_pkg_manager)"
}

test_a_family_whose_package_manager_is_missing_reports_none() {
  # Debian with apt-get removed is rare and real - a stripped container image.
  # Naming a manager that is not there sends the wizard off to run nothing.
  _stub_uname Linux x86_64
  assert_command_absent apt-get
  _platform_from debian-12
  assert_eq "debian" "$(platform_family)" "the family is still known"
  assert_eq "none" "$(platform_pkg_manager)"
  platform_detect
  if platform_is_known; then
    fail "a family with no reachable package manager cannot be acted on"
  fi
}

test_an_unrecognised_distro_has_no_package_manager_even_if_one_is_installed() {
  _stub_uname Linux x86_64
  stub_command apt-get
  stub_command dnf
  _platform_from alpine-3.20
  assert_eq "unknown" "$(platform_family)"
  assert_eq "none" "$(platform_pkg_manager)" \
    "an unknown family must not borrow a manager it merely found lying about"
}

test_resolving_a_package_manager_does_not_run_it() {
  _stub_uname Linux x86_64
  stub_command apt-get
  _platform_from debian-12
  platform_detect
  assert_eq "apt-get" "$(platform_pkg_manager)"
  assert_stub_not_called apt-get "detection looks a command up; it never runs it"
}

# -------------------------------------------------------------- homebrew

test_macos_with_homebrew_on_path_resolves_to_brew() {
  _stub_macos
  stub_command brew
  _platform_from none
  assert_eq "macos" "$(platform_family)"
  assert_eq "brew" "$(platform_pkg_manager)"
  platform_detect
  platform_has_brew || fail "brew is on PATH"
  # The prefix is derived from where brew was found, by path arithmetic - the
  # stub lives in $STUB_BIN, so that is the prefix here. On a real Mac the
  # same rule turns /opt/homebrew/bin/brew into /opt/homebrew.
  assert_eq "$STUB_BIN" "$(platform_brew_prefix)"
  assert_stub_not_called brew "finding brew must not run brew"
}

test_homebrew_is_found_in_its_standard_prefix_before_it_reaches_path() {
  # Immediately after the Homebrew installer runs, brew exists but the shell
  # that started the wizard has not been told about it.
  _stub_macos
  assert_command_absent brew
  mkdir -p "$TEST_TMPDIR/opt/homebrew/bin"
  printf '#!/bin/sh\nexit 0\n' > "$TEST_TMPDIR/opt/homebrew/bin/brew"
  chmod +x "$TEST_TMPDIR/opt/homebrew/bin/brew"
  PLATFORM_BREW_PREFIXES="$TEST_TMPDIR/opt/homebrew $TEST_TMPDIR/usr/local"
  export PLATFORM_BREW_PREFIXES

  _platform_from none
  platform_detect
  platform_has_brew || fail "brew is installed, just not on PATH yet"
  assert_eq "$TEST_TMPDIR/opt/homebrew" "$(platform_brew_prefix)"
  assert_eq "brew" "$(platform_pkg_manager)"

  # And every generated command has to name it by that path. Saying "brew"
  # here would produce a command that dies with command-not-found - in exactly
  # the situation this whole search exists to serve.
  assert_eq "$TEST_TMPDIR/opt/homebrew/bin/brew" "$(platform_brew_bin)"
  assert_eq "$TEST_TMPDIR/opt/homebrew/bin/brew install tmux" \
    "$(platform_pkg_install_command tmux)"
  assert_eq "$TEST_TMPDIR/opt/homebrew/bin/brew update" "$(platform_pkg_refresh_command)"
  assert_contains "$(platform_pkg_query_command tmux)" \
    "$TEST_TMPDIR/opt/homebrew/bin/brew list --versions"
}

test_a_brew_already_on_path_is_named_plainly() {
  _stub_macos
  stub_command brew
  _platform_from none
  assert_eq "brew" "$(platform_brew_bin)" \
    "a path would still work, but the command a person is shown should read well"
  assert_eq "brew install tmux" "$(platform_pkg_install_command tmux)"
}

test_the_intel_and_apple_silicon_prefixes_are_both_searched() {
  _stub_macos
  mkdir -p "$TEST_TMPDIR/usr/local/bin"
  printf '#!/bin/sh\nexit 0\n' > "$TEST_TMPDIR/usr/local/bin/brew"
  chmod +x "$TEST_TMPDIR/usr/local/bin/brew"
  PLATFORM_BREW_PREFIXES="$TEST_TMPDIR/opt/homebrew $TEST_TMPDIR/usr/local"
  export PLATFORM_BREW_PREFIXES

  _platform_from none
  platform_detect
  assert_eq "$TEST_TMPDIR/usr/local" "$(platform_brew_prefix)" \
    "an Intel Mac keeps Homebrew in /usr/local"
}

test_macos_without_homebrew_is_a_handled_state_and_not_an_error() {
  _stub_macos
  assert_command_absent brew
  PLATFORM_BREW_PREFIXES="$TEST_TMPDIR/opt/homebrew $TEST_TMPDIR/usr/local"
  export PLATFORM_BREW_PREFIXES

  _platform_from none
  assert_eq "macos" "$(platform_os)" "it is still a Mac"
  assert_eq "macos" "$(platform_family)"
  assert_eq "" "$(platform_brew_prefix)"
  assert_eq "none" "$(platform_pkg_manager)"
  platform_detect
  if platform_has_brew; then fail "there is no Homebrew here"; fi
  if platform_is_known; then
    fail "without a package manager this layer cannot act on its own"
  fi
}

test_homebrew_is_detected_independently_of_the_operating_system() {
  # Homebrew on Linux exists. The detection reports it either way; only the
  # package-manager choice is a macOS thing.
  _stub_uname Linux x86_64
  stub_command brew
  stub_command apt-get
  _platform_from debian-12
  platform_detect
  platform_has_brew || fail "brew is installed on this linux box"
  assert_eq "apt-get" "$(platform_pkg_manager)" "a debian box still uses apt-get"
}

# -------------------------------------------------------- service managers

test_a_usable_systemd_user_session_is_systemd() {
  _stub_uname Linux x86_64
  stub_command systemctl --exit 1
  stub_when systemctl '--user show-environment' --exit 0 --stdout "LANG=C"
  _platform_from debian-12
  assert_eq "systemd" "$(platform_service_manager)"
  assert_stub_called_with systemctl "systemctl --user show-environment"
}

test_a_systemctl_without_a_user_session_is_not_systemd() {
  # Every container image has the binary. Almost none has the session, and
  # `systemctl --user enable` there fails after the wizard promised it would
  # work.
  _stub_uname Linux x86_64
  stub_command systemctl --exit 1 --stderr "Failed to connect to bus"
  _platform_from debian-12
  assert_eq "none" "$(platform_service_manager)"
  assert_stub_called_with systemctl "systemctl --user show-environment" \
    "anchor: it really did ask"
}

test_no_systemctl_at_all_is_none() {
  _stub_uname Linux x86_64
  assert_command_absent systemctl
  _platform_from debian-12
  assert_eq "none" "$(platform_service_manager)"
}

test_macos_uses_launchd() {
  _stub_macos
  stub_command launchctl
  _platform_from none
  assert_eq "launchd" "$(platform_service_manager)"
  assert_stub_not_called launchctl "finding launchctl must not run it"
}

test_macos_never_claims_systemd_however_much_systemctl_is_installed() {
  _stub_macos
  stub_command systemctl --exit 0 --stdout "LANG=C"
  assert_command_absent launchctl
  _platform_from none
  assert_eq "none" "$(platform_service_manager)"
  assert_stub_not_called systemctl "a Mac is never asked about systemd"
}

test_a_machine_with_no_service_manager_says_none() {
  _stub_uname Linux x86_64
  assert_command_absent systemctl
  assert_command_absent launchctl
  _platform_from void
  assert_eq "linux" "$(platform_os)" "anchor: it did detect something"
  assert_eq "none" "$(platform_service_manager)" \
    "none is the documented manual-fallback path, not a failure"
}

# ------------------------------------------------------- image-based systems

test_an_image_based_system_has_its_package_manager_and_may_not_use_it() {
  # Silverblue arrives as plain ID=fedora with dnf right there, and installing
  # with it does not survive a reboot. Only VARIANT_ID gives it away.
  _stub_uname Linux x86_64
  stub_command dnf
  _platform_from fedora-silverblue-40
  assert_eq "fedora" "$(platform_family)" "it is still recognisably Fedora"
  assert_eq "none" "$(platform_pkg_manager)"
  platform_detect
  platform_is_immutable || fail "silverblue is image-based"
  if platform_is_known; then
    fail "promising to install here and then not surviving a reboot is worse than the manual path"
  fi
  assert_stub_not_called dnf
}

test_an_image_based_system_is_still_named_in_the_summary() {
  _stub_uname Linux x86_64
  stub_command pacman
  _platform_from steamos-3.5
  local summary
  summary="$(platform_summary)"
  assert_contains "$summary" "SteamOS" "the manual instructions have to be able to name it"
  assert_contains "$summary" "packages: none"
  assert_contains "$summary" "image-based" "and say why there is no package manager"
}

test_an_ordinary_fedora_is_not_mistaken_for_an_image_based_one() {
  _stub_uname Linux x86_64
  stub_command dnf
  _platform_from fedora-40
  platform_detect
  if platform_is_immutable; then fail "Workstation is not image-based"; fi
  assert_eq "dnf" "$(platform_pkg_manager)"
  platform_is_known || fail "an ordinary Fedora is exactly what this layer acts on"
}

# ----------------------------------------------------------------- verdict

test_a_fully_recognised_machine_is_known() {
  _stub_uname Linux x86_64
  stub_command apt-get
  stub_command systemctl --exit 0 --stdout "LANG=C"
  _platform_from ubuntu-24.04
  platform_detect
  platform_is_known || fail "ubuntu with apt-get is exactly what this layer is for"
  assert_contains "$(platform_summary)" "apt-get"
  assert_contains "$(platform_summary)" "systemd"
}

test_an_unknown_machine_is_not_known() {
  _stub_uname Linux x86_64
  _platform_from nixos-24.05
  platform_detect
  if platform_is_known; then fail "nixos is not a family this layer can act on"; fi
  assert_contains "$(platform_summary)" "packages: none"
}
