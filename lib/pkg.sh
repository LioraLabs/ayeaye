# The package adapter: one interface, five package managers behind it.
#
# Sourced by lib/platform.sh; never on its own. Like the rest of the layer it
# is inert on load and every answer comes from platform_detect's cache.
#
# ------------------------------------------------------------------ questions
#
#   platform_pkg_can_act              0 when packages can be installed here
#                                     without asking a human anything
#   platform_pkg_blocker              why not, as one word, or empty. Always 0.
#                                     unknown-platform | no-package-manager |
#                                     image-based | no-homebrew | no-root
#   platform_pkg_has_mapping <l>      0 when the name table has an opinion
#                                     about that logical name
#   platform_pkg_is_installed <l>     0 when it is already there. Asks; never
#                                     installs. 1 for "no", and also for "no
#                                     way to ask".
#   platform_privilege                root | sudo | none. Always 0.
#
# --------------------------------------------------------------- what to run
#
#   platform_pkg_name <logical>       this family's real name for a package,
#                                     or the logical name back unchanged when
#                                     the table has no opinion. Always 0 - ask
#                                     platform_pkg_has_mapping if the
#                                     difference matters.
#   platform_pkg_query_command <l>    what "is this installed?" would run
#   platform_pkg_refresh_command      "update the package lists"
#   platform_pkg_install_command <l>… "install these". Homebrew is named by
#                                     platform_brew_bin, so a brew that is
#                                     installed but not yet on PATH still
#                                     produces a command that runs.
#   platform_pkg_manual_hint <l>…     what to tell a human, several lines.
#                                     Always 0.
#   platform_sudo_prefix              "sudo" or "". Always 0.
#
# ------------------------------------------------------------------- do it
#
#   platform_pkg_install <logical>…   runs exactly what
#                                     platform_pkg_install_command returned.
#                                     The only function here with an effect.
#
# ------------------------------------------------------------------ statuses
#
# The command-returning functions answer with a string and 0, or with nothing
# and non-zero. Two non-zero values, and the difference matters:
#
#   1   this machine cannot do that - no package manager, no root, an
#       image-based system. A fact about the machine, and the wizard's cue to
#       print platform_pkg_manual_hint instead.
#   2   the call was wrong - no packages named. A bug in the caller, which
#       should not be reported to a user as "your platform is unsupported".
#
# **A consumer running under `set -e` must not write `cmd="$(platform_pkg_…)"`
# bare.** A command substitution that returns non-zero is fatal there. Write
# `if cmd="$(platform_pkg_install_command tmux)"; then … else … fi`, or add
# `|| true` and test for the empty string. Everything documented "Always 0"
# above is safe to assign bare.
#
# ---------------------------------------------------------------------- names
#
# The logical names are the ones this project installs: tmux, python3, ffmpeg,
# curl, git. Anything else is passed through unchanged. Extending the table is
# the intended way to add a dependency - see _platform_pkg_name_for.
#
# bash 3.2: no associative arrays, so the name table is a case statement. It
# reads better than a parallel-array lookup would anyway.

_PLATFORM_PRIVILEGE="${_PLATFORM_PRIVILEGE:-}"

# ------------------------------------------------------------------ privilege

# Filled by platform_detect, from the uid it has already worked out. `id -u`
# rather than $EUID because a test has to be able to answer it, and because
# $EUID is bash-only.
_platform_detect_privilege() {
  _PLATFORM_PRIVILEGE="none"
  if [ "$_PLATFORM_UID" = "0" ]; then
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

# _platform_pkg_cmd <word>... - the words, quoted where a word needs it, with
# the sudo prefix in front if one is called for. Reads the cache directly
# rather than calling platform_sudo_prefix: this runs for every generated
# command, and a subshell to learn one word is a subshell too many.
_platform_pkg_cmd() {
  local out="" word
  if _platform_pkg_needs_root && [ "$_PLATFORM_PRIVILEGE" = "sudo" ]; then
    out="sudo"
  fi
  for word in "$@"; do
    _platform_quote "$word"
    if [ -n "$out" ]; then
      out="$out $_PLATFORM_S"
    else
      out="$_PLATFORM_S"
    fi
  done
  printf '%s' "$out"
}

# ---------------------------------------------------------------- can we act

# platform_pkg_blocker - one word, or empty when nothing is in the way. The
# order is the order a human would notice them in.
#
# "unknown platform" and "recognised platform, no package manager" are
# different words because they call for different guidance: one is "ayeaye has
# never heard of this distro", the other is "this is Debian and apt-get is not
# installed".
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
  if [ "$_PLATFORM_FAMILY" = "unknown" ]; then
    printf 'unknown-platform'
    return 0
  fi
  if [ "$_PLATFORM_PKG_MANAGER" = "none" ]; then
    printf 'no-package-manager'
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
# Only the differences are interesting, and there are three of them:
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

# platform_pkg_name <logical> - always succeeds. A name the table has no
# opinion about comes back unchanged, which is the most useful thing to do
# with it; platform_pkg_has_mapping is how a caller learns it was a guess.
platform_pkg_name() {
  platform_detect
  _platform_pkg_name_for "$_PLATFORM_FAMILY" "${1:-}" || true
}

# platform_pkg_has_mapping <logical> - status only.
platform_pkg_has_mapping() {
  platform_detect
  [ "$_PLATFORM_FAMILY" != "unknown" ] || return 1
  _platform_pkg_name_for "$_PLATFORM_FAMILY" "${1:-}" >/dev/null
}

# _platform_pkg_names <logical>... - the mapped names, space separated and
# quoted where a name needs it.
_platform_pkg_names() {
  local out="" one name
  for one in "$@"; do
    name="$(_platform_pkg_name_for "$_PLATFORM_FAMILY" "$one")" || true
    _platform_quote "$name"
    if [ -n "$out" ]; then
      out="$out $_PLATFORM_S"
    else
      out="$_PLATFORM_S"
    fi
  done
  printf '%s' "$out"
}

# ------------------------------------------------------------------ querying

# platform_pkg_query_command <logical> - what "is this installed?" would run.
#
# It uses the package manager the family has, even on an image-based system
# where installing with it is not allowed: asking is a read, it needs no root,
# and manual instructions are far better for knowing what is already there.
#
# The redirections are part of the string on purpose: this is the single
# source of truth for the question, and platform_pkg_is_installed answers by
# running exactly it. One of them needs more than an exit status - dpkg-query
# exits 0 for a package whose configuration files survive its removal - so the
# test is in the command rather than around it.
platform_pkg_query_command() {
  platform_detect
  local name
  [ "$#" -gt 0 ] || return 2
  name="$(platform_pkg_name "$1")"
  _platform_quote "$name"
  name="$_PLATFORM_S"
  case "$_PLATFORM_PKG_TOOL" in
    apt-get)
      printf "dpkg-query -W -f='\${db:Status-Status}' %s 2>/dev/null | grep -qx installed" "$name" ;;
    dnf|yum|zypper)
      # --whatprovides so that a capability answers too, which on openSUSE is
      # the only way ffmpeg or python3 can answer at all.
      printf 'rpm -q --whatprovides %s' "$name" ;;
    pacman)
      printf 'pacman -Q %s' "$name" ;;
    brew)
      _platform_quote "$_PLATFORM_BREW_BIN"
      printf '%s list --versions %s' "$_PLATFORM_S" "$name" ;;
    *)
      return 1 ;;
  esac
}

# platform_pkg_is_installed <logical> - status only. Asks; never installs.
platform_pkg_is_installed() {
  local cmd
  cmd="$(platform_pkg_query_command "$@")" || return 1
  [ -n "$cmd" ] || return 1
  eval "$cmd" >/dev/null 2>&1
}

# --------------------------------------------------------- generated commands

# platform_pkg_refresh_command - "update the package lists".
#
# Callers should run this before platform_pkg_install_command. On Arch it is
# not optional: pacman -S against a database that was never synced fails
# outright. -Sy without a full -Syu is a partial upgrade, which Arch documents
# as unsupported; it is also what every install script does, and a wizard that
# ran -Syu would be rebooting people's machines. -Sy it is, and a caller is
# free to offer -Syu instead.
platform_pkg_refresh_command() {
  platform_detect
  platform_pkg_can_act || return 1
  case "$_PLATFORM_PKG_MANAGER" in
    apt-get) _platform_pkg_cmd apt-get update ;;
    dnf)     _platform_pkg_cmd dnf makecache ;;
    yum)     _platform_pkg_cmd yum makecache ;;
    pacman)  _platform_pkg_cmd pacman -Sy ;;
    zypper)  _platform_pkg_cmd zypper --non-interactive refresh ;;
    brew)    _platform_quote "$_PLATFORM_BREW_BIN"; printf '%s update' "$_PLATFORM_S" ;;
    *)       return 1 ;;
  esac
}

# platform_pkg_install_command <logical>... - "install these", as a string.
# Nothing and non-zero whenever this layer cannot act, because an install
# command it is not allowed to run is worse than no command at all.
#
# apt-get is handed DEBIAN_FRONTEND=noninteractive through env, because a
# debconf prompt in the middle of a wizard that already promised to be
# unattended is a hang with no explanation. `sudo env VAR=x` rather than
# `sudo VAR=x`, which many sudoers configurations reject outright.
platform_pkg_install_command() {
  platform_detect
  [ "$#" -gt 0 ] || return 2
  platform_pkg_can_act || return 1
  local names
  names="$(_platform_pkg_names "$@")"
  case "$_PLATFORM_PKG_MANAGER" in
    apt-get) printf '%s %s' \
               "$(_platform_pkg_cmd env DEBIAN_FRONTEND=noninteractive apt-get install -y)" \
               "$names" ;;
    dnf)     printf '%s %s' "$(_platform_pkg_cmd dnf install -y)" "$names" ;;
    yum)     printf '%s %s' "$(_platform_pkg_cmd yum install -y)" "$names" ;;
    pacman)  printf '%s %s' "$(_platform_pkg_cmd pacman -S --needed --noconfirm)" "$names" ;;
    zypper)  printf '%s %s' "$(_platform_pkg_cmd zypper --non-interactive install)" "$names" ;;
    brew)    _platform_quote "$_PLATFORM_BREW_BIN"
             printf '%s install %s' "$_PLATFORM_S" "$names" ;;
    *)       return 1 ;;
  esac
}

# platform_pkg_install <logical>... - the executing form, and the only
# function in this file that changes the machine.
platform_pkg_install() {
  local cmd status
  cmd="$(platform_pkg_install_command "$@")"
  status=$?
  [ "$status" = 0 ] || return "$status"
  [ -n "$cmd" ] || return 1
  eval "$cmd"
}

# ------------------------------------------------------------ manual guidance

# platform_pkg_manual_hint <logical>... - several lines, for a human. Always
# succeeds: there is always something worth saying.
#
# The first line is the actionable one: the command, when there is a command,
# and otherwise why there is not. Then the packages, by the name this platform
# would call them, so the text can be pasted at a search engine and get
# somewhere.
platform_pkg_manual_hint() {
  platform_detect
  local blocker names cmd
  blocker="$(platform_pkg_blocker)"
  names="$(_platform_pkg_names "$@")"
  [ -n "$names" ] || names="(nothing)"

  case "$blocker" in
    "")
      cmd="$(platform_pkg_install_command "$@")" || cmd=""
      if [ -n "$cmd" ]; then
        printf '%s\n' "install them with: $cmd"
      else
        printf '%s\n' "install them with $_PLATFORM_PKG_MANAGER."
      fi
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
    no-package-manager)
      printf '%s\n' "this is $_PLATFORM_PRETTY, but its package manager is not installed here."
      ;;
    no-root)
      printf '%s\n' "$_PLATFORM_PKG_MANAGER needs root, and this session has neither root nor sudo."
      ;;
    *)
      printf '%s\n' "packages cannot be installed automatically here ($blocker)."
      ;;
  esac
  printf '%s\n' "packages needed: $names"
}
