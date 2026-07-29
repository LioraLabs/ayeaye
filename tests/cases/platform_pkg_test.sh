# The package adapter: one interface, five package managers behind it.
#
# Nothing here installs anything. The command-returning forms are asserted as
# strings, and the executing forms are asserted through the stub recordings -
# which is the only honest way to test "what would this run on Fedora" from a
# machine that is not one.

_platform_load() {
  if declare -f platform_reset >/dev/null 2>&1; then
    platform_reset
  else
    . "$REPO_ROOT/lib/platform.sh"
  fi
}

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
  stub_command uname --exit 1
  stub_when uname '-s' --stdout "$1"
  stub_when uname '-m' --stdout "$2"
}

# An ordinary unprivileged user with sudo available - the common case, and the
# one the generated commands are written for. `id` is stubbed rather than
# trusted: the suite runs as root inside a container and as a user on a
# laptop, and the answer must not depend on which.
_as_user_with_sudo() {
  stub_command id --stdout "1000"
  stub_command sudo
}

# _on <fixture> <package-manager> - a Linux family with its manager present.
_on() {
  _stub_uname Linux x86_64
  _as_user_with_sudo
  stub_command "$2"
  _platform_from "$1"
}

_on_macos_with_brew() {
  _stub_uname Darwin arm64
  stub_command_from_fixture sw_vers sw_vers/macos-15.1
  stub_command id --stdout "501"
  stub_command sudo
  stub_command brew
  _platform_from none
}

# ------------------------------------------------------------ package names

test_a_logical_name_becomes_this_familys_real_package_name() {
  _on arch pacman
  assert_eq "python" "$(platform_pkg_name python3)" \
    "arch calls the python 3 interpreter python, and python3 is a different thing"
  assert_eq "tmux" "$(platform_pkg_name tmux)"

  _on fedora-40 dnf
  assert_eq "ffmpeg-free" "$(platform_pkg_name ffmpeg)" \
    "fedora's own repositories have no package called ffmpeg"
  assert_eq "python3" "$(platform_pkg_name python3)"

  _on debian-12 apt-get
  assert_eq "python3" "$(platform_pkg_name python3)"
  assert_eq "ffmpeg" "$(platform_pkg_name ffmpeg)"

  _on opensuse-tumbleweed zypper
  assert_eq "ffmpeg" "$(platform_pkg_name ffmpeg)" \
    "openSUSE has no package called ffmpeg either, but it does have the capability"

  _on_macos_with_brew
  assert_eq "python3" "$(platform_pkg_name python3)"
}

test_a_name_this_layer_has_no_opinion_about_is_passed_through_and_says_so() {
  _on debian-12 apt-get
  local name status
  name="$(platform_pkg_name ripgrep)"
  status=$?
  assert_eq "ripgrep" "$name" "passing it through is more useful than refusing"
  assert_status 1 "$status" "but the caller has to be able to tell it was a guess"
}

test_an_unknown_platform_maps_nothing_and_admits_it() {
  _stub_uname Linux x86_64
  _as_user_with_sudo
  _platform_from void
  local name status
  name="$(platform_pkg_name python3)"
  status=$?
  assert_eq "python3" "$name"
  assert_status 1 "$status"
}

# -------------------------------------------------------- generated commands

test_each_family_generates_its_own_install_command() {
  _on debian-12 apt-get
  assert_eq "sudo apt-get install -y tmux ffmpeg" \
    "$(platform_pkg_install_command tmux ffmpeg)"

  _on fedora-40 dnf
  assert_eq "sudo dnf install -y tmux ffmpeg-free" \
    "$(platform_pkg_install_command tmux ffmpeg)"

  _on arch pacman
  assert_eq "sudo pacman -S --needed --noconfirm tmux python" \
    "$(platform_pkg_install_command tmux python3)"

  _on opensuse-tumbleweed zypper
  assert_eq "sudo zypper --non-interactive install tmux ffmpeg" \
    "$(platform_pkg_install_command tmux ffmpeg)"

  _on_macos_with_brew
  assert_eq "brew install tmux ffmpeg" \
    "$(platform_pkg_install_command tmux ffmpeg)"
}

test_homebrew_is_never_run_with_sudo() {
  _on_macos_with_brew
  local cmd
  cmd="$(platform_pkg_install_command tmux)"
  assert_eq "brew install tmux" "$cmd"
  assert_not_contains "$cmd" "sudo" "brew refuses to run as root, and says so rudely"
  assert_not_contains "$(platform_pkg_refresh_command)" "sudo"
}

test_root_needs_no_sudo() {
  _stub_uname Linux x86_64
  stub_command id --stdout "0"
  stub_command apt-get
  _platform_from debian-12
  assert_eq "root" "$(platform_privilege)"
  assert_eq "apt-get install -y tmux" "$(platform_pkg_install_command tmux)"
}

test_each_family_generates_its_own_refresh_command() {
  _on debian-12 apt-get
  assert_eq "sudo apt-get update" "$(platform_pkg_refresh_command)"

  _on fedora-40 dnf
  assert_eq "sudo dnf makecache" "$(platform_pkg_refresh_command)"

  _on arch pacman
  assert_eq "sudo pacman -Sy" "$(platform_pkg_refresh_command)"

  _on opensuse-tumbleweed zypper
  assert_eq "sudo zypper --non-interactive refresh" "$(platform_pkg_refresh_command)"

  _on_macos_with_brew
  assert_eq "brew update" "$(platform_pkg_refresh_command)"
}

test_asking_what_would_be_run_runs_nothing() {
  _on debian-12 apt-get
  assert_ne "" "$(platform_pkg_install_command tmux ffmpeg)" "anchor: it did generate one"
  assert_ne "" "$(platform_pkg_refresh_command)"
  assert_stub_not_called apt-get "the whole point of the form is that it does not act"
  assert_stub_not_called sudo
}

test_the_executing_form_runs_exactly_what_the_string_said() {
  _on debian-12 apt-get
  platform_pkg_install tmux ffmpeg
  assert_stub_called_with sudo "sudo apt-get install -y tmux ffmpeg"
}

test_the_executing_form_refuses_where_the_string_form_would_be_empty() {
  _stub_uname Linux x86_64
  _as_user_with_sudo
  _platform_from void
  local status
  platform_pkg_install tmux
  status=$?
  assert_status 1 "$status" "it must not fall back to guessing a command"
  assert_stub_not_called sudo
}

# ------------------------------------------------------------ is it installed

test_asking_whether_a_package_is_installed_uses_this_familys_query() {
  _on debian-12 apt-get
  stub_command dpkg-query --stdout "installed"
  platform_pkg_is_installed tmux || fail "dpkg-query said installed"
  assert_stub_called_with dpkg-query "dpkg-query -W"
  assert_stub_called_with dpkg-query "tmux"
  assert_stub_not_called apt-get "asking is not installing"
}

test_a_package_that_is_not_installed_is_reported_as_such() {
  _on debian-12 apt-get
  stub_command dpkg-query --exit 1 --stderr "no packages found matching tmux"
  if platform_pkg_is_installed tmux; then fail "dpkg-query said it is not there"; fi
}

test_a_removed_but_not_purged_package_does_not_count_as_installed() {
  # dpkg-query exits 0 for a package whose configuration files are still on
  # disk. Only the status word tells the two apart.
  _on debian-12 apt-get
  stub_command dpkg-query --exit 0 --stdout "config-files"
  if platform_pkg_is_installed tmux; then
    fail "config-files means the package itself is gone"
  fi
}

test_the_rpm_families_ask_rpm_and_the_others_ask_their_own_tool() {
  _on fedora-40 dnf
  stub_command rpm
  platform_pkg_is_installed ffmpeg || fail "rpm said yes"
  assert_stub_called_with rpm "rpm -q --whatprovides ffmpeg-free" \
    "the query has to use the mapped name, not the logical one"

  _on opensuse-tumbleweed zypper
  stub_command rpm
  platform_pkg_is_installed ffmpeg || fail "rpm said yes"
  assert_stub_called_with rpm "--whatprovides ffmpeg" \
    "openSUSE only provides ffmpeg as a capability, so the query must accept one"

  _on arch pacman
  platform_pkg_is_installed python3 || fail "pacman said yes"
  assert_stub_called_with pacman "pacman -Q python" "arch's name for it, not ours"

  _on_macos_with_brew
  platform_pkg_is_installed tmux || fail "brew said yes"
  assert_stub_called_with brew "brew list --versions tmux"
}

test_a_missing_query_tool_is_not_the_same_as_an_installed_package() {
  _on debian-12 apt-get
  assert_command_absent dpkg-query
  if platform_pkg_is_installed tmux; then
    fail "with no way to ask, the honest answer is no"
  fi
}

test_asking_on_an_unknown_platform_is_answered_not_attempted() {
  _stub_uname Linux x86_64
  _as_user_with_sudo
  stub_command dpkg-query --stdout "installed"
  _platform_from nixos-24.05
  if platform_pkg_is_installed tmux; then
    fail "there is no query for a platform this layer does not know"
  fi
  assert_stub_not_called dpkg-query
}

# ------------------------------------------------- cannot act, and saying why

test_a_recognised_platform_with_a_manager_can_act() {
  _on ubuntu-24.04 apt-get
  platform_pkg_can_act || fail "ubuntu with apt-get and sudo is the ordinary case"
  assert_eq "" "$(platform_pkg_blocker)"
}

test_an_unknown_platform_cannot_act_and_names_the_reason() {
  _stub_uname Linux x86_64
  _as_user_with_sudo
  _platform_from void
  if platform_pkg_can_act; then fail "void is not a family this layer knows"; fi
  assert_eq "unknown-platform" "$(platform_pkg_blocker)"
  assert_eq "" "$(platform_pkg_install_command tmux)" \
    "there is no command to offer, and inventing one would be worse than none"
}

test_macos_without_homebrew_cannot_act_and_names_the_reason() {
  _stub_uname Darwin arm64
  stub_command_from_fixture sw_vers sw_vers/macos-15.1
  stub_command id --stdout "501"
  assert_command_absent brew
  PLATFORM_BREW_PREFIXES="$TEST_TMPDIR/opt/homebrew"
  export PLATFORM_BREW_PREFIXES
  _platform_from none
  if platform_pkg_can_act; then fail "there is nothing here to install with"; fi
  assert_eq "no-homebrew" "$(platform_pkg_blocker)"
}

test_an_image_based_system_cannot_act_and_names_the_reason() {
  _stub_uname Linux x86_64
  _as_user_with_sudo
  stub_command dnf
  _platform_from fedora-silverblue-40
  if platform_pkg_can_act; then fail "installing with dnf here does not survive a reboot"; fi
  assert_eq "image-based" "$(platform_pkg_blocker)"
}

test_no_way_to_reach_root_cannot_act_and_names_the_reason() {
  _stub_uname Linux x86_64
  stub_command id --stdout "1000"
  assert_command_absent sudo
  stub_command apt-get
  _platform_from debian-12
  assert_eq "none" "$(platform_privilege)"
  if platform_pkg_can_act; then fail "apt-get needs root and there is no way to it"; fi
  assert_eq "no-root" "$(platform_pkg_blocker)"
  assert_eq "" "$(platform_pkg_install_command tmux)"
}

test_homebrew_needs_no_root_so_a_rootless_mac_can_still_act() {
  _stub_uname Darwin arm64
  stub_command_from_fixture sw_vers sw_vers/macos-15.1
  stub_command id --stdout "501"
  assert_command_absent sudo
  stub_command brew
  _platform_from none
  platform_pkg_can_act || fail "brew installs into its own prefix and never wanted root"
  assert_eq "" "$(platform_pkg_blocker)"
  assert_eq "brew install tmux" "$(platform_pkg_install_command tmux)"
}

# ------------------------------------------------------------- manual guidance

test_the_manual_hint_names_the_packages_and_why_it_cannot_do_it_itself() {
  _stub_uname Linux x86_64
  _as_user_with_sudo
  _platform_from void
  local hint
  hint="$(platform_pkg_manual_hint tmux ffmpeg)"
  assert_contains "$hint" "tmux"
  assert_contains "$hint" "ffmpeg"
  assert_contains "$hint" "void" "it has to name the platform it did not recognise"
}

test_the_manual_hint_for_a_mac_without_homebrew_points_at_homebrew() {
  _stub_uname Darwin arm64
  stub_command_from_fixture sw_vers sw_vers/macos-15.1
  stub_command id --stdout "501"
  PLATFORM_BREW_PREFIXES="$TEST_TMPDIR/opt/homebrew"
  export PLATFORM_BREW_PREFIXES
  _platform_from none
  local hint
  hint="$(platform_pkg_manual_hint tmux)"
  assert_contains "$hint" "brew.sh" "there is exactly one thing to tell them to do"
  assert_contains "$hint" "tmux"
}

test_the_manual_hint_for_an_image_based_system_names_the_right_tool() {
  _stub_uname Linux x86_64
  _as_user_with_sudo
  stub_command dnf
  _platform_from fedora-silverblue-40
  local hint
  hint="$(platform_pkg_manual_hint tmux)"
  assert_contains "$hint" "rpm-ostree" "dnf is right there and is the wrong answer"

  _platform_from steamos-3.5
  assert_contains "$(platform_pkg_manual_hint tmux)" "read-only"
}

test_the_manual_hint_on_a_platform_it_can_act_on_shows_the_command() {
  _on debian-12 apt-get
  local hint
  hint="$(platform_pkg_manual_hint tmux ffmpeg)"
  assert_contains "$hint" "sudo apt-get install -y tmux ffmpeg" \
    "when there is a command, the useful hint is that command"
}

test_producing_a_manual_hint_runs_nothing() {
  _on debian-12 apt-get
  assert_ne "" "$(platform_pkg_manual_hint tmux)" "anchor"
  assert_stub_not_called apt-get
  assert_stub_not_called sudo
}
