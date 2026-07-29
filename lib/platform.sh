# The platform layer: what this machine is, and who to ask to change it.
#
#   . "$REPO/lib/platform.sh"
#
# Sourcing is inert. It defines functions, gives the tunables their defaults
# and does nothing else: no command is run, no file is written, no shell option
# is changed, nothing is sent anywhere. The first question asked triggers
# detection, the answer is cached, and platform_reset throws the cache away.
#
# Every answer is a lower-case word, never a sentence, so callers can `case`
# on it. "unknown" is a first-class answer everywhere: this layer would rather
# say it cannot identify a machine than guess wrong about one.
#
# ---------------------------------------------------------------- detection
#
#   platform_os               linux | macos | unknown
#   platform_family           debian | fedora | arch | suse | macos | unknown
#   platform_id               the distro's own id: ubuntu, raspbian, macos...
#   platform_version          VERSION_ID, or the macOS product version; may be
#                             empty on a rolling release
#   platform_pretty           the human name: "Ubuntu 24.04.1 LTS", "macOS 15.1"
#   platform_arch             x86_64 | arm64 | the raw uname -m | unknown
#   platform_pkg_manager      apt-get | dnf | pacman | zypper | brew | none
#   platform_service_manager  systemd | launchd | none
#   platform_brew_prefix      /opt/homebrew, /usr/local, or empty
#   platform_has_brew         status 0 when Homebrew is installed
#   platform_is_known         status 0 when this layer can act automatically
#   platform_summary          the whole verdict as one line
#   platform_detect           force detection now
#   platform_reset            drop the cache; the next question detects again
#
# Call platform_detect once, early, in your own shell. The accessors are meant
# to be read through `$(...)`, and a command substitution is a subshell, so a
# cache filled inside one dies with it - a consumer that never calls
# platform_detect itself re-probes on every single question. Correct either
# way, but only free the first way.
#
# ----------------------------------------------------------------- tunables
#
#   PLATFORM_OS_RELEASE_FILE   where the distro identity is read from. Unset
#                              means /etc/os-release, falling back to
#                              /usr/lib/os-release. Set it, and that path is
#                              the only one consulted - which is how the tests
#                              feed it a fixture.
#   PLATFORM_BREW_PREFIXES     space-separated prefixes to look for Homebrew
#                              in when it is not on PATH.
#
# bash 3.2: no associative arrays, no ${var,,}, no mapfile, no local -n.

# Plain assignment rather than `: "${var:=}"`: bash 3.2 does not create a
# variable when the := default is the empty string, so under `set -u` the
# first read of it would be a fatal unbound-variable error on macOS and
# nowhere else. Assigning through :- keeps whatever the caller already set.
PLATFORM_OS_RELEASE_FILE="${PLATFORM_OS_RELEASE_FILE:-}"
PLATFORM_BREW_PREFIXES="${PLATFORM_BREW_PREFIXES:-/opt/homebrew /usr/local}"

# The cache. Only platform_detect and platform_reset write these. Sourcing the
# library twice must not throw away an answer already worked out.
_PLATFORM_DETECTED="${_PLATFORM_DETECTED:-0}"
_PLATFORM_OS="${_PLATFORM_OS:-}"
_PLATFORM_FAMILY="${_PLATFORM_FAMILY:-}"
_PLATFORM_ID="${_PLATFORM_ID:-}"
_PLATFORM_VERSION="${_PLATFORM_VERSION:-}"
_PLATFORM_PRETTY="${_PLATFORM_PRETTY:-}"
_PLATFORM_ARCH="${_PLATFORM_ARCH:-}"
_PLATFORM_PKG_MANAGER="${_PLATFORM_PKG_MANAGER:-}"
_PLATFORM_SERVICE_MANAGER="${_PLATFORM_SERVICE_MANAGER:-}"
_PLATFORM_BREW_PREFIX="${_PLATFORM_BREW_PREFIX:-}"

_PLATFORM_LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# --------------------------------------------------------- reading os-release
#
# Parsed, not sourced. `. /etc/os-release` is the documented way to read the
# file and it is also arbitrary code execution from a path this layer is
# explicitly allowed to have pointed somewhere else, which is not a trade this
# library is willing to make.

# _platform_os_release_value <file> <key> - the value, unquoted, last wins.
# Silent and non-zero when the file or the key is missing.
_platform_os_release_value() {
  local file="$1"
  local key="$2"
  local line value found
  value=""
  found=1
  [ -n "$file" ] && [ -r "$file" ] || return 1
  # `|| [ -n "$line" ]` so a final line without a newline is still read.
  while IFS= read -r line || [ -n "$line" ]; do
    line="${line%$'\r'}"
    # Leading whitespace, then comments, then anything that is not this key.
    while :; do
      case "$line" in
        " "*|"	"*) line="${line#?}" ;;
        *) break ;;
      esac
    done
    case "$line" in
      "#"*) continue ;;
      "$key="*) value="${line#*=}" ;;
      *) continue ;;
    esac
    # Strip one layer of matching quotes.
    case "$value" in
      '"'*'"') value="${value#\"}"; value="${value%\"}" ;;
      "'"*"'") value="${value#\'}"; value="${value%\'}" ;;
    esac
    found=0
  done < "$file"
  [ "$found" = 0 ] || return 1
  printf '%s' "$value"
}

# _platform_os_release_path - the file detection should actually read.
_platform_os_release_path() {
  # Read defensively. A caller who writes `PLATFORM_OS_RELEASE_FILE=x . lib`
  # gets a temporary assignment that does not outlive the source, so by the
  # time this runs the name can be unset again - and under `set -u` that is
  # fatal rather than empty.
  if [ -n "${PLATFORM_OS_RELEASE_FILE:-}" ]; then
    printf '%s' "$PLATFORM_OS_RELEASE_FILE"
    return 0
  fi
  if [ -r /etc/os-release ]; then
    printf '%s' /etc/os-release
  elif [ -r /usr/lib/os-release ]; then
    printf '%s' /usr/lib/os-release
  fi
}

# ------------------------------------------------------------------ families
#
# The mapping is by distro id, and it is deliberately generous: a derivative
# that nobody here has heard of still reaches the right package manager
# through ID_LIKE. Adding a name to one of these lists is the cheapest change
# in this file - and getting one wrong is the most expensive, because it makes
# the wizard run a package manager the machine does not have.

# _platform_family_of_id <id> - the family, or non-zero if the id means nothing.
_platform_family_of_id() {
  case "$1" in
    debian|ubuntu|raspbian|raspberry-pi-os|linuxmint|mint|pop|elementary|neon\
      |zorin|devuan|kali|parrot|deepin|mx|pureos|tuxedo|linuxlite|peppermint)
      printf 'debian' ;;
    fedora|rhel|centos|rocky|almalinux|ol|oracle|amzn|scientific|nobara\
      |bazzite|silverblue|qubes|eurolinux|circle|navylinux)
      printf 'fedora' ;;
    arch|archarm|arch32|manjaro|manjaro-arm|endeavouros|garuda|cachyos|artix\
      |arcolinux|steamos|blendos|parabola)
      printf 'arch' ;;
    opensuse|opensuse-leap|opensuse-tumbleweed|opensuse-slowroll\
      |opensuse-microos|opensuse-aeon|opensuse-kalpa|suse|sles|sled|sle-micro\
      |sle_hpc|tumbleweed|leap|geckolinux)
      printf 'suse' ;;
    macos|darwin)
      printf 'macos' ;;
    *)
      return 1 ;;
  esac
}

# _platform_family_from_os_release <file> - ID first, then each ID_LIKE token.
#
# ID_LIKE tokens go through the same id lookup rather than a separate table,
# which is what makes a derivative of a derivative work: Linux Mint says
# ID_LIKE=ubuntu, and ubuntu is only debian by the same rule.
_platform_family_from_os_release() {
  local file="$1" id like token family
  id="$(_platform_os_release_value "$file" ID)" || id=""
  if [ -n "$id" ]; then
    family="$(_platform_family_of_id "$id")" && { printf '%s' "$family"; return 0; }
  fi
  like="$(_platform_os_release_value "$file" ID_LIKE)" || like=""
  for token in $like; do
    family="$(_platform_family_of_id "$token")" && { printf '%s' "$family"; return 0; }
  done
  printf 'unknown'
}

# --------------------------------------------------------------- the detector

# platform_reset - forget everything. Call it after something that changes the
# answers, such as installing Homebrew, or from a test.
platform_reset() {
  _PLATFORM_DETECTED=0
  _PLATFORM_OS=""
  _PLATFORM_FAMILY=""
  _PLATFORM_ID=""
  _PLATFORM_VERSION=""
  _PLATFORM_PRETTY=""
  _PLATFORM_ARCH=""
  _PLATFORM_PKG_MANAGER=""
  _PLATFORM_SERVICE_MANAGER=""
  _PLATFORM_BREW_PREFIX=""
}

# platform_detect - work everything out now. Idempotent; every accessor calls
# it first, so it is rarely needed by hand.
platform_detect() {
  [ "$_PLATFORM_DETECTED" = 1 ] && return 0
  _PLATFORM_DETECTED=1

  local kernel="" machine="" file=""
  if command -v uname >/dev/null 2>&1; then
    kernel="$(uname -s 2>/dev/null)" || kernel=""
    machine="$(uname -m 2>/dev/null)" || machine=""
  fi
  file="$(_platform_os_release_path)"

  case "$machine" in
    x86_64|amd64|x64)      _PLATFORM_ARCH="x86_64" ;;
    arm64|aarch64|armv8*)  _PLATFORM_ARCH="arm64" ;;
    "")                    _PLATFORM_ARCH="unknown" ;;
    *)                     _PLATFORM_ARCH="$machine" ;;
  esac

  if [ "$kernel" = "Darwin" ]; then
    # The kernel wins over any os-release lying around: a nix or a linuxbrew
    # install can leave one on a Mac.
    _platform_detect_macos
  elif [ -n "$file" ] && [ -r "$file" ]; then
    _PLATFORM_OS="linux"
    _platform_detect_linux "$file"
  elif [ "$kernel" = "Linux" ]; then
    _PLATFORM_OS="linux"
    _PLATFORM_FAMILY="unknown"
    _PLATFORM_ID="unknown"
    _PLATFORM_PRETTY="Linux"
  else
    _PLATFORM_OS="unknown"
    _PLATFORM_FAMILY="unknown"
    _PLATFORM_ID="unknown"
    _PLATFORM_PRETTY="unknown"
  fi

  _platform_detect_brew
  _platform_detect_pkg_manager
  _platform_detect_service_manager
  return 0
}

_platform_detect_linux() {
  local file="$1"
  _PLATFORM_ID="$(_platform_os_release_value "$file" ID)" || _PLATFORM_ID=""
  [ -n "$_PLATFORM_ID" ] || _PLATFORM_ID="unknown"
  _PLATFORM_VERSION="$(_platform_os_release_value "$file" VERSION_ID)" \
    || _PLATFORM_VERSION=""
  _PLATFORM_PRETTY="$(_platform_os_release_value "$file" PRETTY_NAME)" \
    || _PLATFORM_PRETTY=""
  [ -n "$_PLATFORM_PRETTY" ] || _PLATFORM_PRETTY="$_PLATFORM_ID"
  if [ "$_PLATFORM_ID" = "unknown" ]; then
    _PLATFORM_FAMILY="unknown"
  else
    _PLATFORM_FAMILY="$(_platform_family_from_os_release "$file")"
  fi
}

_platform_detect_macos() {
  _PLATFORM_OS="macos"
  _PLATFORM_FAMILY="macos"
  _PLATFORM_ID="macos"
  _PLATFORM_VERSION=""
  # sw_vers with no arguments, one exec for both fields. A Mac without it is
  # broken in ways this layer cannot fix, but it is still a Mac.
  if command -v sw_vers >/dev/null 2>&1; then
    _PLATFORM_VERSION="$(sw_vers 2>/dev/null \
      | sed -n 's/^ProductVersion:[[:space:]]*//p' | head -1)" \
      || _PLATFORM_VERSION=""
  fi
  if [ -n "$_PLATFORM_VERSION" ]; then
    _PLATFORM_PRETTY="macOS $_PLATFORM_VERSION"
  else
    _PLATFORM_PRETTY="macOS"
  fi
}

# Homebrew is detected on its own, not as a property of macOS: a Mac without
# Homebrew is an ordinary machine the wizard has to cope with, and Homebrew on
# Linux exists too. PATH first, because that is where an installed brew is;
# then the standard prefixes, because a brew installed in this same session is
# not on PATH yet.
_platform_detect_brew() {
  local brew_bin prefix
  _PLATFORM_BREW_PREFIX=""
  if brew_bin="$(command -v brew 2>/dev/null)" && [ -n "$brew_bin" ]; then
    # .../<prefix>/bin/brew
    prefix="${brew_bin%/*}"
    prefix="${prefix%/bin}"
    [ -n "$prefix" ] || prefix="/"
    _PLATFORM_BREW_PREFIX="$prefix"
    return 0
  fi
  for prefix in ${PLATFORM_BREW_PREFIXES:-/opt/homebrew /usr/local}; do
    if [ -x "$prefix/bin/brew" ]; then
      _PLATFORM_BREW_PREFIX="$prefix"
      return 0
    fi
  done
  return 0
}

# A family names a package manager; PATH decides whether it is really there.
# Claiming apt-get on a debian container that has had it removed would send
# the wizard off to run a command that does not exist.
_platform_detect_pkg_manager() {
  local candidates="" candidate
  case "$_PLATFORM_FAMILY" in
    debian) candidates="apt-get" ;;
    fedora) candidates="dnf yum" ;;
    arch)   candidates="pacman" ;;
    suse)   candidates="zypper" ;;
    macos)  candidates="brew" ;;
    *)      candidates="" ;;
  esac
  _PLATFORM_PKG_MANAGER="none"
  for candidate in $candidates; do
    if [ "$candidate" = "brew" ]; then
      [ -n "$_PLATFORM_BREW_PREFIX" ] || continue
      _PLATFORM_PKG_MANAGER="brew"
      return 0
    fi
    if command -v "$candidate" >/dev/null 2>&1; then
      _PLATFORM_PKG_MANAGER="$candidate"
      return 0
    fi
  done
  return 0
}

# systemd has to be present *and* usable. `systemctl --user show-environment`
# is the same question install.sh already asks: a container has the binary and
# no user session, and enabling a unit there fails after the wizard has
# promised it would work.
_platform_detect_service_manager() {
  _PLATFORM_SERVICE_MANAGER="none"
  if [ "$_PLATFORM_OS" = "macos" ]; then
    command -v launchctl >/dev/null 2>&1 && _PLATFORM_SERVICE_MANAGER="launchd"
    return 0
  fi
  if command -v systemctl >/dev/null 2>&1 \
     && systemctl --user show-environment >/dev/null 2>&1; then
    _PLATFORM_SERVICE_MANAGER="systemd"
  fi
  return 0
}

# ------------------------------------------------------------------ accessors

platform_os()              { platform_detect; printf '%s' "$_PLATFORM_OS"; }
platform_family()          { platform_detect; printf '%s' "$_PLATFORM_FAMILY"; }
platform_id()              { platform_detect; printf '%s' "$_PLATFORM_ID"; }
platform_version()         { platform_detect; printf '%s' "$_PLATFORM_VERSION"; }
platform_pretty()          { platform_detect; printf '%s' "$_PLATFORM_PRETTY"; }
platform_arch()            { platform_detect; printf '%s' "$_PLATFORM_ARCH"; }
platform_pkg_manager()     { platform_detect; printf '%s' "$_PLATFORM_PKG_MANAGER"; }
platform_service_manager() { platform_detect; printf '%s' "$_PLATFORM_SERVICE_MANAGER"; }
platform_brew_prefix()     { platform_detect; printf '%s' "$_PLATFORM_BREW_PREFIX"; }

# platform_has_brew - status only, for `if platform_has_brew; then`.
platform_has_brew() {
  platform_detect
  [ -n "$_PLATFORM_BREW_PREFIX" ]
}

# platform_is_known - the one question the wizard asks before deciding whether
# to act on its own. True means this layer recognised the platform and found a
# package manager to talk to.
platform_is_known() {
  platform_detect
  [ "$_PLATFORM_FAMILY" != "unknown" ] && [ "$_PLATFORM_PKG_MANAGER" != "none" ]
}

# platform_summary - one line, for one line of wizard output.
platform_summary() {
  platform_detect
  local name="$_PLATFORM_PRETTY"
  [ -n "$name" ] || name="unknown"
  printf '%s (%s) %s, packages: %s, services: %s' \
    "$name" "$_PLATFORM_FAMILY" "$_PLATFORM_ARCH" \
    "$_PLATFORM_PKG_MANAGER" "$_PLATFORM_SERVICE_MANAGER"
}
