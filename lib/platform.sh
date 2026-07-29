# The platform layer: what this machine is, and who to ask to change it.
#
#   . "$REPO/lib/platform.sh"
#
# Sourcing is inert. It defines functions, gives the tunables their defaults
# and does nothing else: no command is run, no file is read or written, no
# shell option is changed, nothing is sent anywhere. The first question asked
# triggers detection, the answer is cached, and platform_reset throws the
# cache away.
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
#   platform_pkg_manager      apt-get | dnf | yum | pacman | zypper | brew | none
#   platform_service_manager  systemd | launchd | none
#   platform_brew_prefix      /opt/homebrew, /usr/local, or empty
#   platform_has_brew         status 0 when Homebrew is installed
#   platform_is_immutable     status 0 on an image-based system whose package
#                             manager cannot install into the running system
#   platform_is_known         status 0 when this layer can act automatically
#   platform_summary          the whole verdict as one line
#   platform_detect           detect now if it has not happened yet
#   platform_reset            drop the cache; the next question detects again
#
# Two of those names are narrower than they look, on purpose:
#
#   platform_service_manager reports the *user-session* service manager, which
#   is what this project installs into. It answers systemd only when a user
#   bus really responds, so a machine plainly running systemd as pid 1 can
#   still answer "none" - under sudo, or before the session is up. That is the
#   honest answer to "can I install a user service here", which is the only
#   question this project asks.
#
#   platform_pkg_manager answers "none" on an image-based system - Silverblue,
#   Aeon, MicroOS, SteamOS - even though dnf or zypper is right there, because
#   installing with it does not survive a reboot. platform_is_immutable says
#   why, so the manual path can explain itself rather than shrug.
#
# Call platform_detect once, early, in your own shell. The accessors are meant
# to be read through $(...), and a command substitution is a subshell, so a
# cache filled inside one dies with it - a consumer that never calls
# platform_detect itself re-probes on every single question. Correct either
# way, but only free the first way.
#
# ----------------------------------------------------------------- tunables
#
#   PLATFORM_OS_RELEASE_FILES  space-separated candidate paths for the distro
#                              identity; the first readable one wins. Defaults
#                              to "/etc/os-release /usr/lib/os-release". Set it
#                              to one path to pin it, or to the empty string to
#                              say there is none - which is how the tests feed
#                              it a fixture without the real machine ever
#                              leaking in.
#   PLATFORM_BREW_PREFIXES     space-separated prefixes to look for Homebrew in
#                              when it is not on PATH.
#
# Neither list can contain a path with a space in it. Both are configuration,
# not user input, and treating them as lists is worth more than that case.
#
# bash 3.2: no associative arrays, no ${var,,}, no mapfile, no local -n.

# Plain assignment rather than `: "${var:=}"`: bash 3.2 does not create a
# variable when the := default is the empty string, so under `set -u` the
# first read of it would be a fatal unbound-variable error on macOS and
# nowhere else. Assigning through :- keeps whatever the caller already set.
PLATFORM_OS_RELEASE_FILES="${PLATFORM_OS_RELEASE_FILES-/etc/os-release /usr/lib/os-release}"
PLATFORM_BREW_PREFIXES="${PLATFORM_BREW_PREFIXES-/opt/homebrew /usr/local /home/linuxbrew/.linuxbrew}"

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
_PLATFORM_IMMUTABLE="${_PLATFORM_IMMUTABLE:-0}"

# Scratch for the two string helpers below, which return through it rather
# than through stdout. Initialised so that `set -u` has nothing to complain
# about before the first call.
_PLATFORM_S="${_PLATFORM_S:-}"

# ------------------------------------------------------------------- strings
#
# Small text helpers, all pure parameter expansion. They exist so that nothing
# in this file has to reach for sed, cut or tr - a library that shells out to
# trim a string is a library that stops working on a bare PATH.
#
# Both answer in $_PLATFORM_S rather than on stdout. Reading them through
# $(...) would be nicer to look at and would fork a subshell per line of every
# file this library reads, which is a fork per line too many for something the
# wizard calls before it has printed anything at all.

# _platform_trim <string> -> $_PLATFORM_S, without leading or trailing blanks.
_platform_trim() {
  _PLATFORM_S="$1"
  while :; do
    case "$_PLATFORM_S" in
      " "*|"	"*) _PLATFORM_S="${_PLATFORM_S#?}" ;;
      *) break ;;
    esac
  done
  while :; do
    case "$_PLATFORM_S" in
      *" "|*"	") _PLATFORM_S="${_PLATFORM_S%?}" ;;
      *) break ;;
    esac
  done
}

# _platform_lower <string> -> $_PLATFORM_S, in ASCII lower case. Deepin has
# shipped ID=Deepin, and os-release ids are compared, not displayed.
_platform_lower() {
  local out="" c rest
  rest="$1"
  while [ -n "$rest" ]; do
    c="${rest%"${rest#?}"}"
    rest="${rest#?}"
    case "$c" in
      A) c=a ;; B) c=b ;; C) c=c ;; D) c=d ;; E) c=e ;; F) c=f ;; G) c=g ;;
      H) c=h ;; I) c=i ;; J) c=j ;; K) c=k ;; L) c=l ;; M) c=m ;; N) c=n ;;
      O) c=o ;; P) c=p ;; Q) c=q ;; R) c=r ;; S) c=s ;; T) c=t ;; U) c=u ;;
      V) c=v ;; W) c=w ;; X) c=x ;; Y) c=y ;; Z) c=z ;;
    esac
    out="$out$c"
  done
  _PLATFORM_S="$out"
}

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
  # Initialised, not merely declared: a read error - $file is a directory, say
  # - leaves $line unset, and reading an unset name under `set -u` is fatal.
  local line=""
  local value=""
  local found=1
  [ -n "$file" ] && [ -f "$file" ] && [ -r "$file" ] || return 1
  # `|| [ -n "$line" ]` so a final line without a newline is still read.
  while IFS= read -r line || [ -n "$line" ]; do
    line="${line%$'\r'}"
    _platform_trim "$line"; line="$_PLATFORM_S"
    case "$line" in
      "#"*) continue ;;
      "$key="*) value="${line#*=}" ;;
      *) continue ;;
    esac
    _platform_trim "$value"; value="$_PLATFORM_S"
    case "$value" in
      '"'*'"') value="${value#\"}"; value="${value%\"}" ;;
      "'"*"'") value="${value#\'}"; value="${value%\'}" ;;
      # An unquoted value ends at a comment, the way the shell would read it.
      *" #"*) _platform_trim "${value%% #*}"; value="$_PLATFORM_S" ;;
    esac
    found=0
  done < "$file"
  [ "$found" = 0 ] || return 1
  printf '%s' "$value"
}

# _platform_os_release_path - the first candidate that exists, or nothing.
_platform_os_release_path() {
  local candidate
  # Read defensively. A caller who writes `PLATFORM_OS_RELEASE_FILES=x . lib`
  # gets a temporary assignment that does not outlive the source, so by the
  # time this runs the name can be unset again - and under `set -u` that is
  # fatal rather than empty.
  for candidate in ${PLATFORM_OS_RELEASE_FILES-}; do
    if [ -f "$candidate" ] && [ -r "$candidate" ]; then
      printf '%s' "$candidate"
      return 0
    fi
  done
  return 0
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

# An image-based system: the package manager is present and installing with it
# does not survive a reboot. Recognising the distro and refusing to act on it
# is the only honest combination - "unknown" would lose the name the manual
# instructions need to print.
_platform_is_immutable_id() {
  case "$1" in
    opensuse-microos|opensuse-aeon|opensuse-kalpa|sle-micro|steamos|qubes\
      |bazzite|silverblue|fedora-coreos|fedora-iot)
      return 0 ;;
  esac
  # Silverblue and its siblings arrive as plain ID=fedora; only the variant
  # gives them away.
  case "$2" in
    silverblue|kinoite|sericea|onyx|iot|coreos|atomic-host|sway-atomic\
      |budgie-atomic)
      return 0 ;;
  esac
  return 1
}

# _platform_family_from_os_release <file> - ID first, then each ID_LIKE token.
#
# ID_LIKE tokens go through the same id lookup rather than a separate table,
# which is what makes a derivative of a derivative work: Trisquel says
# ID_LIKE=ubuntu, and ubuntu is only debian by the same rule.
#
# The token loop is written with parameter expansion rather than `for t in
# $like`, because an unquoted expansion is subject to pathname expansion too -
# and every byte here comes out of a file this library is explicitly allowed
# to have pointed somewhere else.
_platform_family_from_os_release() {
  local file="$1" id like rest token family
  id="$(_platform_os_release_value "$file" ID)" || id=""
  _platform_lower "$id"; id="$_PLATFORM_S"
  if [ -n "$id" ]; then
    family="$(_platform_family_of_id "$id")" && { printf '%s' "$family"; return 0; }
  fi
  like="$(_platform_os_release_value "$file" ID_LIKE)" || like=""
  _platform_lower "$like"; rest="$_PLATFORM_S"
  while :; do
    _platform_trim "$rest"; rest="$_PLATFORM_S"
    [ -n "$rest" ] || break
    token="${rest%%[ 	]*}"
    rest="${rest#"$token"}"
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
  _PLATFORM_IMMUTABLE=0
}

# platform_detect - work everything out, once. A no-op when the cache is
# already filled; platform_reset is how you make it look again.
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
    x86_64|amd64|x64)  _PLATFORM_ARCH="x86_64" ;;
    # Deliberately not armv8*: armv8l is a 32-bit userland on a 64-bit
    # kernel, and calling it arm64 hands a consumer the wrong binary.
    arm64|aarch64)     _PLATFORM_ARCH="arm64" ;;
    "")                _PLATFORM_ARCH="unknown" ;;
    *)                 _PLATFORM_ARCH="$machine" ;;
  esac

  if [ "$kernel" = "Darwin" ]; then
    # The kernel wins over any os-release lying around: a nix or a linuxbrew
    # install can leave one on a Mac.
    _platform_detect_macos
  elif [ -n "$file" ]; then
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
  local file="$1" variant=""
  _PLATFORM_ID="$(_platform_os_release_value "$file" ID)" || _PLATFORM_ID=""
  _platform_lower "$_PLATFORM_ID"; _PLATFORM_ID="$_PLATFORM_S"
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
  variant="$(_platform_os_release_value "$file" VARIANT_ID)" || variant=""
  _platform_lower "$variant"; variant="$_PLATFORM_S"
  if _platform_is_immutable_id "$_PLATFORM_ID" "$variant"; then
    _PLATFORM_IMMUTABLE=1
  fi
}

_platform_detect_macos() {
  local out="" line=""
  _PLATFORM_OS="macos"
  _PLATFORM_FAMILY="macos"
  _PLATFORM_ID="macos"
  _PLATFORM_VERSION=""
  # sw_vers with no arguments, so one exec answers everything a caller might
  # later want out of it. Parsed without a pipeline: under `pipefail` a reader
  # that stops early turns a correct answer into a failed command substitution.
  if command -v sw_vers >/dev/null 2>&1; then
    out="$(sw_vers 2>/dev/null)" || out=""
  fi
  while IFS= read -r line || [ -n "$line" ]; do
    case "$line" in
      ProductVersion:*)
        _platform_trim "${line#ProductVersion:}"
        _PLATFORM_VERSION="$_PLATFORM_S"
        ;;
    esac
  done <<EOF
$out
EOF
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
#
# The prefix is derived from where brew was found, by path arithmetic - it is
# never `brew --prefix`, because asking would mean running it.
_platform_detect_brew() {
  local brew_bin prefix
  _PLATFORM_BREW_PREFIX=""
  if brew_bin="$(command -v brew 2>/dev/null)" && [ -n "$brew_bin" ]; then
    # A shell function or alias named brew answers `command -v` with something
    # that is not a path at all.
    case "$brew_bin" in
      /*)
        prefix="${brew_bin%/*}"
        prefix="${prefix%/bin}"
        [ -n "$prefix" ] || prefix="/"
        _PLATFORM_BREW_PREFIX="$prefix"
        return 0
        ;;
    esac
  fi
  for prefix in ${PLATFORM_BREW_PREFIXES-}; do
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
  _PLATFORM_PKG_MANAGER="none"
  # An image-based system has its package manager right there and cannot use
  # it for this. Saying none sends the wizard down the manual path, which is
  # the only one that works.
  [ "$_PLATFORM_IMMUTABLE" = 1 ] && return 0
  case "$_PLATFORM_FAMILY" in
    debian) candidates="apt-get" ;;
    fedora) candidates="dnf yum" ;;
    arch)   candidates="pacman" ;;
    suse)   candidates="zypper" ;;
    macos)  candidates="brew" ;;
    *)      candidates="" ;;
  esac
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

# systemd has to be present *and* usable as a user session. `systemctl --user
# show-environment` is the same question install.sh already asks: a container
# has the binary and no user session, and enabling a unit there fails after
# the wizard has promised it would work.
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

# platform_is_immutable - status only. True on an image-based system, where
# the package manager exists and installing with it does not last.
platform_is_immutable() {
  platform_detect
  [ "$_PLATFORM_IMMUTABLE" = 1 ]
}

# platform_is_known - the one question the wizard asks before deciding whether
# to act on its own. True means this layer recognised the platform and found a
# package manager it is willing to talk to.
platform_is_known() {
  platform_detect
  [ "$_PLATFORM_FAMILY" != "unknown" ] && [ "$_PLATFORM_PKG_MANAGER" != "none" ]
}

# platform_summary - one line, for one line of wizard output.
platform_summary() {
  platform_detect
  local name="$_PLATFORM_PRETTY" note=""
  [ -n "$name" ] || name="unknown"
  [ "$_PLATFORM_IMMUTABLE" = 1 ] && note=", image-based"
  printf '%s (%s) %s, packages: %s, services: %s%s' \
    "$name" "$_PLATFORM_FAMILY" "$_PLATFORM_ARCH" \
    "$_PLATFORM_PKG_MANAGER" "$_PLATFORM_SERVICE_MANAGER" "$note"
}
