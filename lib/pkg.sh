# The package adapter: one interface, five package managers behind it.
#
# Sourced by lib/platform.sh; never on its own. Like the rest of the layer it
# is inert on load and every answer comes from platform_detect's cache.
#
#   platform_pkg_can_act              status 0 when packages can be installed
#                                     here without asking a human anything
#   platform_pkg_blocker              why not, as one word, or empty:
#                                     unknown-platform | image-based |
#                                     no-homebrew | no-root
#   platform_pkg_name <logical>       this family's real name for a package.
#                                     Status 1 means "no mapping, here is your
#                                     own string back" - a guess, and the
#                                     caller is told it is one.
#   platform_pkg_is_installed <l>     status 0 when it is already there. Asks;
#                                     never installs.
#   platform_pkg_query_command <l>    the command that asking would run
#   platform_pkg_refresh_command      "update the package lists", as a string
#   platform_pkg_install_command <l>… "install these", as a string. Empty and
#                                     status 1 when this layer cannot act.
#   platform_pkg_install <l>…         the same thing, actually run. The only
#                                     function in this file with an effect.
#   platform_pkg_manual_hint <l>…     what to tell a human, several lines
#   platform_privilege                root | sudo | none
#   platform_sudo_prefix              "sudo" or ""; empty for both root and
#                                     for a manager that never needed it
#
# The logical names are the ones this project installs: tmux, python3, ffmpeg,
# curl, git. Anything else is passed through unchanged, with a non-zero status
# to say the table had no opinion. Extending the table is the intended way to
# add a dependency - see _platform_pkg_name_for.
#
# bash 3.2: no associative arrays, so the name table is a case statement. It
# reads better than a parallel-array lookup would anyway.

_PLATFORM_PRIVILEGE="${_PLATFORM_PRIVILEGE:-}"

# ------------------------------------------------------------------ privilege

# Filled by platform_detect. `id -u` rather than $EUID because a test has to
# be able to answer it, and because $EUID is bash-only.
_platform_detect_privilege() {
  local uid=""
  _PLATFORM_PRIVILEGE="none"
  if command -v id >/dev/null 2>&1; then
    uid="$(id -u 2>/dev/null)" || uid=""
  fi
  if [ "$uid" = "0" ]; then
    _PLATFORM_PRIVILEGE="root"
  elif command -v sudo >/dev/null 2>&1; then
    _PLATFORM_PRIVILEGE="sudo"
  fi
  return 0
}

platform_privilege() {
  platform_detect
  printf '%s' "$_PLATFORM_PRIVILEGE"
}

# Which managers need root at all. Homebrew refuses to run as root and
# installs into a prefix the user owns; everything else writes to /usr.
_platform_pkg_needs_root() {
  case "$_PLATFORM_PKG_MANAGER" in
    brew|none) return 1 ;;
    *) return 0 ;;
  esac
}

# platform_sudo_prefix - "sudo" or "", never a trailing space.
platform_sudo_prefix() {
  platform_detect
  _platform_pkg_needs_root || return 0
  [ "$_PLATFORM_PRIVILEGE" = "sudo" ] && printf 'sudo'
  return 0
}

# _platform_pkg_cmd <words...> - join with the sudo prefix, if there is one.
_platform_pkg_cmd() {
  local prefix
  prefix="$(platform_sudo_prefix)"
  if [ -n "$prefix" ]; then
    printf '%s %s' "$prefix" "$*"
  else
    printf '%s' "$*"
  fi
}

# ---------------------------------------------------------------- can we act

# platform_pkg_blocker - one word, or empty when nothing is in the way. The
# order is the order a human would notice them in.
platform_pkg_blocker() {
  platform_detect
  if [ "$_PLATFORM_IMMUTABLE" = 1 ]; then
    printf 'image-based'
    return 0
  fi
  if [ "$_PLATFORM_FAMILY" = "macos" ] && [ -z "$_PLATFORM_BREW_PREFIX" ]; then
    printf 'no-homebrew'
    return 0
  fi
  if [ "$_PLATFORM_PKG_MANAGER" = "none" ]; then
    printf 'unknown-platform'
    return 0
  fi
  if _platform_pkg_needs_root && [ "$_PLATFORM_PRIVILEGE" = "none" ]; then
    printf 'no-root'
    return 0
  fi
  return 0
}

# platform_pkg_can_act - status only.
platform_pkg_can_act() {
  [ -z "$(platform_pkg_blocker)" ]
}

# ------------------------------------------------------------- package names

# _platform_pkg_name_for <family> <logical> - the table. Non-zero means the
# table has no opinion, and the logical name is the best guess available.
#
# Only the differences are interesting, and there are four of them:
#   arch calls the python 3 interpreter "python"; "python3" there is an
#     entirely different, older package.
#   fedora's own repositories carry "ffmpeg-free"; plain "ffmpeg" needs a
#     third-party repository this wizard has no business enabling.
#   openSUSE has no package literally called ffmpeg or python3 either - they
#     are capabilities, provided by ffmpeg-7 and python313 - but zypper
#     installs a capability by name, so the logical name works as written.
_platform_pkg_name_for() {
  case "$1:$2" in
    arch:python3)     printf 'python' ;;
    fedora:ffmpeg)    printf 'ffmpeg-free' ;;
    *:tmux)           printf 'tmux' ;;
    *:python3)        printf 'python3' ;;
    *:ffmpeg)         printf 'ffmpeg' ;;
    *:curl)           printf 'curl' ;;
    *:git)            printf 'git' ;;
    *)                printf '%s' "$2"; return 1 ;;
  esac
}

# platform_pkg_name <logical>
platform_pkg_name() {
  platform_detect
  if [ "$_PLATFORM_FAMILY" = "unknown" ]; then
    printf '%s' "$1"
    return 1
  fi
  _platform_pkg_name_for "$_PLATFORM_FAMILY" "$1"
}

# _platform_pkg_names <logical>... - the mapped names, space separated. An
# unmapped name is still included; a caller that cares asked platform_pkg_name
# about it directly.
_platform_pkg_names() {
  local out="" one name
  for one in "$@"; do
    name="$(_platform_pkg_name_for "$_PLATFORM_FAMILY" "$one")" || true
    if [ -n "$out" ]; then
      out="$out $name"
    else
      out="$name"
    fi
  done
  printf '%s' "$out"
}

# ------------------------------------------------------------------ querying

# platform_pkg_query_command <logical> - what "is this installed?" runs.
#
# The redirections are part of the string on purpose: this is the single
# source of truth for the question, and platform_pkg_is_installed answers by
# running exactly it. Two of them need more than an exit status - dpkg-query
# exits 0 for a package whose configuration files survive its removal - so the
# test is in the command rather than around it.
platform_pkg_query_command() {
  platform_detect
  local name
  name="$(platform_pkg_name "$1")" || true
  case "$_PLATFORM_PKG_MANAGER" in
    apt-get)
      printf "dpkg-query -W -f='\${db:Status-Status}' %s 2>/dev/null | grep -qx installed" "$name" ;;
    dnf|yum|zypper)
      # --whatprovides so that a capability answers too, which on openSUSE is
      # the only way ffmpeg or python3 can answer at all.
      printf 'rpm -q --whatprovides %s' "$name" ;;
    pacman)
      printf 'pacman -Q %s' "$name" ;;
    brew)
      printf 'brew list --versions %s' "$name" ;;
    *)
      return 1 ;;
  esac
}

# platform_pkg_is_installed <logical> - status only. Asks; never installs.
platform_pkg_is_installed() {
  local cmd
  cmd="$(platform_pkg_query_command "$1")" || return 1
  [ -n "$cmd" ] || return 1
  eval "$cmd" >/dev/null 2>&1
}

# --------------------------------------------------------- generated commands

# platform_pkg_refresh_command - "update the package lists".
#
# Arch is the awkward one: -Sy without a full -Syu is a partial upgrade, which
# Arch documents as unsupported. It is also what every install script does,
# and a wizard that ran -Syu would be rebooting people's machines. -Sy it is,
# and the caller is free to offer -Syu instead.
platform_pkg_refresh_command() {
  platform_detect
  case "$_PLATFORM_PKG_MANAGER" in
    apt-get) _platform_pkg_cmd apt-get update ;;
    dnf)     _platform_pkg_cmd dnf makecache ;;
    yum)     _platform_pkg_cmd yum makecache ;;
    pacman)  _platform_pkg_cmd pacman -Sy ;;
    zypper)  _platform_pkg_cmd zypper --non-interactive refresh ;;
    brew)    printf 'brew update' ;;
    *)       return 1 ;;
  esac
}

# platform_pkg_install_command <logical>... - "install these", as a string.
# Empty and non-zero whenever this layer cannot act, because an install
# command it is not allowed to run is worse than no command at all.
platform_pkg_install_command() {
  platform_detect
  [ "$#" -gt 0 ] || return 1
  platform_pkg_can_act || return 1
  local names
  names="$(_platform_pkg_names "$@")"
  case "$_PLATFORM_PKG_MANAGER" in
    apt-get) _platform_pkg_cmd apt-get install -y "$names" ;;
    dnf)     _platform_pkg_cmd dnf install -y "$names" ;;
    yum)     _platform_pkg_cmd yum install -y "$names" ;;
    pacman)  _platform_pkg_cmd pacman -S --needed --noconfirm "$names" ;;
    zypper)  _platform_pkg_cmd zypper --non-interactive install "$names" ;;
    brew)    printf 'brew install %s' "$names" ;;
    *)       return 1 ;;
  esac
}

# platform_pkg_install <logical>... - the executing form, and the only
# function in this file that changes the machine.
platform_pkg_install() {
  local cmd
  cmd="$(platform_pkg_install_command "$@")" || return 1
  [ -n "$cmd" ] || return 1
  eval "$cmd"
}

# ------------------------------------------------------------ manual guidance

# platform_pkg_manual_hint <logical>... - several lines, for a human.
#
# The first line is the actionable one: the command, when there is a command,
# and otherwise why there is not. Then the packages, by the name this platform
# would call them, so the text can be pasted at a search engine and get
# somewhere.
platform_pkg_manual_hint() {
  platform_detect
  local blocker names
  blocker="$(platform_pkg_blocker)"
  names="$(_platform_pkg_names "$@")"
  [ -n "$names" ] || names="(nothing)"

  case "$blocker" in
    "")
      printf '%s\n' "install them with: $(platform_pkg_install_command "$@")"
      ;;
    image-based)
      case "$_PLATFORM_FAMILY" in
        fedora) printf '%s\n' "$_PLATFORM_PRETTY is image-based: layer packages with rpm-ostree install, then reboot." ;;
        suse)   printf '%s\n' "$_PLATFORM_PRETTY is image-based: install with transactional-update pkg install, then reboot." ;;
        arch)   printf '%s\n' "$_PLATFORM_PRETTY has a read-only system partition: pacman cannot install here in the usual way." ;;
        *)      printf '%s\n' "$_PLATFORM_PRETTY is image-based: its package manager cannot install into the running system." ;;
      esac
      ;;
    no-homebrew)
      printf '%s\n' "Homebrew is not installed. Install it from https://brew.sh and run this again."
      ;;
    unknown-platform)
      printf '%s\n' "this platform was not recognised (id: $_PLATFORM_ID), so there is no package manager to drive."
      ;;
    no-root)
      printf '%s\n' "$_PLATFORM_PKG_MANAGER needs root, and this session has neither root nor sudo."
      ;;
  esac
  printf '%s\n' "packages needed: $names"
}
