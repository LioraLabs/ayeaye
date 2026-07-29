# Permission, in one place.
#
#   . "$REPO/lib/consent.sh"      (needs lib/state.sh and lib/ui.sh first)
#
# Sourcing is inert.
#
# The rule this file exists to make unbreakable: nothing privileged runs,
# nothing is downloaded, no firewall is opened, no certificate is trusted and
# no existing configuration is replaced until a person has said yes. Every
# ticket in this milestone adds actions of exactly those kinds. If each of them
# asked in its own way, one of them would eventually forget - so none of them
# asks at all. They call a wrapper, and the wrapper asks.
#
# ------------------------------------------------------------------- the kinds
#
#   privileged  a command that needs root, or sudo, or writes outside $HOME
#   download    fetching bytes from the internet
#   firewall    changing what this machine accepts from the network
#   trust       teaching this machine to trust a certificate or a key
#   replace     overwriting a file the user already has
#   expose      making ayeaye reachable from somewhere other than this machine
#
# ---------------------------------------------------------------- the contract
#
#   wizard_consent <kind> <question> [detail]…
#         0 granted, 3 refused, 2 the kind is not one of the six (caller bug).
#         3 rather than 1 so that "refused" is the same number here as it is
#         from every wrapper below, where 1 already means "it ran and failed".
#         The question is plain language and is what the user sees; the details
#         are the raw commands and go to the log, not into the question.
#
#   wizard_consent_policy <kind>   -> ask | grant | refuse. Always 0 for a
#         known kind, 2 otherwise. The single place policy is decided.
#
#   wizard_consent_record <kind> <decision> <text>
#         append to the ledger. For a decision that was settled by an explicit
#         answer earlier in the run rather than by asking now, so the audit
#         trail is complete either way. Always 0.
#
#   wizard_consent_ledger          -> the audit trail, one decision per line,
#         "<kind>\t<decision>\t<question>". Always 0.
#   wizard_consent_reset           start a fresh ledger. Always 0.
#
# ----------------------------------------------------------- doing the thing
#
# These are the only functions in this repository allowed to have an effect of
# their kind. Each asks first, and does nothing whatsoever when refused.
#
#   wizard_privileged <question> <command-string>
#   wizard_firewall   <question> <command-string>
#   wizard_trust      <question> <command-string>
#   wizard_may_expose <question> [detail]…   the kind with nothing to run
#   wizard_download   <question> <url> <destination> [bytes]
#   wizard_replace    <path> [question]     -> 0 when the caller may now write
#   wizard_install_packages <logical>…      -> the package layer, with consent
#   wizard_backup     <path>                -> the backup path on stdout
#
# Statuses, and the difference matters at every call site:
#
#   0   done
#   1   it was allowed to run and it did not work, or this machine cannot do it
#   2   the call was wrong: a missing argument, an unknown kind. Caller bug.
#   3   refused. Nothing happened, and nothing is wrong.
#
# --------------------------------------------------------------- the plan
#
# Stage 5 has to summarise what is about to happen before anything happens, so
# the decisions taken in stages 2 to 4 are recorded rather than acted on:
#
#   wizard_plan_add <category> <text> [bytes]
#         categories: package download privileged network trust config service
#   wizard_plan_show              the summary, including estimated disk use
#   wizard_plan_is_consequential  status 0 when the plan contains something
#         worth stopping for. Writing your own config file and your own user
#         service is not; installing, downloading, privilege and the network
#         are.
#   wizard_plan_reset
#
# ----------------------------------------------------------------- tunables
#
#   WIZARD_AUTO_CONSENT     space-separated kinds granted without asking when
#                           the run may not ask. firewall, trust and expose are
#                           ignored here whatever it says: they are never
#                           granted on someone's behalf.
#   WIZARD_CONSENT_LEDGER   where the audit trail goes. Empty means
#                           $WIZARD_STATE_DIR/setup-consent.log, resolved when
#                           it is used rather than when this file is sourced.
#   WIZARD_BACKUP_DIR       where replaced files are copied to. Empty means
#                           $WIZARD_STATE_DIR/backups, resolved the same way.
#
# platform_pkg_install and platform_service_run change the machine and are not
# wrappers. Nothing in this project may call them directly; use
# wizard_install_packages, and route a service operation through
# platform_service_command plus a wrapper. tests/cases/wizard_contract_test.sh
# is what stops that rule from being quietly forgotten.
#
# bash 3.2: no associative arrays, no ${var,,}, no mapfile, no local -n.

# Both default to a name under $WIZARD_STATE_DIR, resolved at the moment they
# are used rather than at the moment this file is sourced: an argument parser
# can only choose a state directory after the libraries it parses with are
# already in memory.
WIZARD_CONSENT_LEDGER="${WIZARD_CONSENT_LEDGER:-}"
WIZARD_BACKUP_DIR="${WIZARD_BACKUP_DIR:-}"
WIZARD_AUTO_CONSENT="${WIZARD_AUTO_CONSENT:-}"

wizard_ledger_path() {
  if [ -n "$WIZARD_CONSENT_LEDGER" ]; then
    printf '%s' "$WIZARD_CONSENT_LEDGER"
  else
    printf '%s/setup-consent.log' "$WIZARD_STATE_DIR"
  fi
}

wizard_backup_dir() {
  if [ -n "$WIZARD_BACKUP_DIR" ]; then
    printf '%s' "$WIZARD_BACKUP_DIR"
  else
    printf '%s/backups' "$WIZARD_STATE_DIR"
  fi
}

# Refused. Not a failure: the user said no and the machine is as they left it.
WIZARD_REFUSED=3

# The three kinds that are never granted without a person, whatever
# WIZARD_AUTO_CONSENT says. A blanket yes for automation is a reasonable thing
# to want; a blanket yes to opening a firewall is not.
_WIZARD_NEVER_AUTO="firewall trust expose"

_WIZARD_PLAN="${_WIZARD_PLAN:-}"
_WIZARD_PLAN_BYTES="${_WIZARD_PLAN_BYTES:-0}"

_WIZARD_TAB='	'

# ------------------------------------------------------------------- policy

_wizard_consent_known_kind() {
  case "${1:-}" in
    privileged|download|firewall|trust|replace|expose) return 0 ;;
    *) return 1 ;;
  esac
}

# wizard_consent_policy <kind> -> ask | grant | refuse
wizard_consent_policy() {
  local kind="${1:-}" auto
  _wizard_consent_known_kind "$kind" || return 2
  if [ "${WIZARD_INTERACTIVE:-1}" != 0 ]; then
    printf 'ask'
    return 0
  fi
  case " $_WIZARD_NEVER_AUTO " in
    *" $kind "*) printf 'refuse'; return 0 ;;
  esac
  for auto in ${WIZARD_AUTO_CONSENT:-}; do
    if [ "$auto" = "$kind" ]; then
      printf 'grant'
      return 0
    fi
  done
  printf 'refuse'
  return 0
}

# ------------------------------------------------------------------ ledger

wizard_consent_reset() {
  local path dir
  path="$(wizard_ledger_path)"
  dir="${path%/*}"
  [ "$dir" = "$path" ] || mkdir -p "$dir" 2>/dev/null || return 0
  : > "$path" 2>/dev/null || true
  return 0
}

# wizard_consent_record <kind> <decision> <text> - always 0.
#
# Public, and meant to be called directly for a decision that was settled by an
# explicit answer earlier in the run rather than by asking now - choosing an
# address to listen on is a consent to exposure, and the audit trail should say
# so without asking the same question twice.
wizard_consent_record() {
  local path dir
  path="$(wizard_ledger_path)"
  dir="${path%/*}"
  [ "$dir" = "$path" ] || mkdir -p "$dir" 2>/dev/null || return 0
  printf '%s%s%s%s%s\n' "${1:-}" "$_WIZARD_TAB" "${2:-}" "$_WIZARD_TAB" "${3:-}" \
    >> "$path" 2>/dev/null || true
  return 0
}

wizard_consent_ledger() {
  local path
  path="$(wizard_ledger_path)"
  [ -f "$path" ] && cat "$path" 2>/dev/null
  return 0
}

# ------------------------------------------------------------------ asking

# wizard_consent <kind> <question> [detail]…
wizard_consent() {
  local kind="${1:-}" question="${2:-}" policy detail
  _wizard_consent_known_kind "$kind" || return 2
  [ -n "$question" ] || return 2
  shift 2

  # Details are the raw material, and the raw material is never part of the
  # question. It goes to the log first so that it is already there for anyone
  # who answers "what is this actually going to run".
  for detail in "$@"; do
    wizard_detail "$kind: $detail"
  done

  policy="$(wizard_consent_policy "$kind")"
  case "$policy" in
    grant)
      wizard_consent_record "$kind" granted "$question"
      return 0
      ;;
    refuse)
      wizard_consent_record "$kind" refused "$question"
      return "$WIZARD_REFUSED"
      ;;
  esac

  if wizard_confirm "$question" "n"; then
    wizard_consent_record "$kind" granted "$question"
    return 0
  fi
  wizard_consent_record "$kind" refused "$question"
  return "$WIZARD_REFUSED"
}

# ------------------------------------------------------------- doing things

# _wizard_do <kind> <question> <command-string> - ask, then run, then report.
#
# The command's own output is technical detail: it goes to the log, and is
# repeated on stderr only when the command failed, which is the one time the
# person in front of the wizard has any use for it.
_wizard_do() {
  local kind="${1:-}" question="${2:-}" cmd="${3:-}" out status
  [ -n "$cmd" ] || return 2
  wizard_consent "$kind" "$question" "$cmd"
  status=$?
  [ "$status" = 2 ] && return 2
  [ "$status" = 0 ] || return "$WIZARD_REFUSED"


  wizard_detail "running: $cmd"
  # Assigned through an `if`, never bare: a command substitution that returns
  # non-zero is fatal under `set -e`, and "it ran and did not work" is an
  # outcome this function is supposed to be able to report.
  if out="$(eval "$cmd" 2>&1)"; then
    status=0
  else
    status=$?
  fi
  if [ -n "$out" ]; then
    wizard_detail "$out"
  fi
  if [ "$status" != 0 ]; then
    # The tool's own words, once, on stderr: the one moment they are worth more
    # to the reader than the plain-language explanation above them.
    if [ -n "$out" ]; then
      printf '%s\n' "$out" >&2
    fi
    return 1
  fi
  return 0
}

# _wizard_exec <command-string> - run something a grant already covers.
#
# The one door past wizard_consent, and it is private for that reason: it may
# only be called inside a function that has just been granted consent covering
# exactly this command. wizard_install_packages is its only caller.
_wizard_exec() {
  local cmd="${1:-}" out status
  [ -n "$cmd" ] || return 2
  wizard_detail "running: $cmd"
  if out="$(eval "$cmd" 2>&1)"; then
    status=0
  else
    status=$?
  fi
  if [ -n "$out" ]; then
    wizard_detail "$out"
  fi
  if [ "$status" != 0 ]; then
    if [ -n "$out" ]; then
      printf '%s\n' "$out" >&2
    fi
    return 1
  fi
  return 0
}

wizard_privileged() { _wizard_do privileged "${1:-}" "${2:-}"; }
wizard_firewall()   { _wizard_do firewall   "${1:-}" "${2:-}"; }

# wizard_may_expose <question> [detail]… - may ayeaye be reachable from
# somewhere other than this computer?
#
# The one kind with no command behind it: exposure is a decision, not a thing
# to run. It still goes through the same primitive and the same ledger, so
# "did anything put this machine on a network" has one place to look and one
# answer, and an unattended run can never answer it yes.
wizard_may_expose() {
  local question="${1:-}"
  [ -n "$question" ] || return 2
  shift
  wizard_consent expose "$question" "$@"
}
wizard_trust()      { _wizard_do trust      "${1:-}" "${2:-}"; }

# wizard_download <question> <url> <destination> [bytes]
#
# curl is the one fetcher this project will use: it is already a logical
# package name the platform layer knows, and every supported platform either
# ships it or can install it. A machine with none says so rather than reporting
# a download that never happened - the seam the bootstrap ticket builds on.
wizard_download() {
  local question="${1:-}" url="${2:-}" dest="${3:-}" bytes="${4:-}" dir status
  [ -n "$url" ] && [ -n "$dest" ] || return 2
  wizard_consent download "$question" "GET $url -> $dest"
  status=$?
  [ "$status" = 2 ] && return 2
  [ "$status" = 0 ] || return "$WIZARD_REFUSED"

  if ! command -v curl >/dev/null 2>&1; then
    wizard_say "this computer has no way to download files (curl is missing)."
    wizard_say "install curl and run setup again, or fetch it yourself:"
    wizard_say "  $url"
    wizard_say "  and save it as $dest"
    return 1
  fi
  dir="${dest%/*}"
  [ "$dir" = "$dest" ] || mkdir -p "$dir" 2>/dev/null || return 1
  wizard_detail "downloading $url ($bytes bytes estimated) to $dest"
  curl -fsSL --retry 2 -o "$dest" "$url" || {
    rm -f "$dest" 2>/dev/null
    return 1
  }
  return 0
}

# wizard_backup <path> -> the backup path on stdout. 1 when it could not be
# made, which a caller must treat as a reason not to overwrite anything.
wizard_backup() {
  local path="${1:-}" name stamp target dir
  [ -n "$path" ] || return 2
  [ -e "$path" ] || return 1
  name="${path##*/}"
  stamp="$(date +%Y%m%d-%H%M%S 2>/dev/null)" || stamp=""
  [ -n "$stamp" ] || stamp="backup"
  dir="$(wizard_backup_dir)"
  mkdir -p "$dir" 2>/dev/null || return 1
  target="$dir/$name.$stamp"
  # A second backup inside the same second must not silently replace the
  # first: the two versions are exactly what somebody comparing them needs.
  if [ -e "$target" ]; then
    local n=2
    while [ -e "$target-$n" ]; do
      n=$((n + 1))
    done
    target="$target-$n"
  fi
  cp -p "$path" "$target" 2>/dev/null || cp "$path" "$target" 2>/dev/null || return 1
  printf '%s' "$target"
  return 0
}

# wizard_replace <path> [question] - may the caller overwrite this file?
#
# 0 means yes and the old version is already safe. A file that does not exist
# needs no permission: there is nothing of the user's to lose.
wizard_replace() {
  local path="${1:-}" question="${2:-}" backup
  [ -n "$path" ] || return 2
  [ -e "$path" ] || return 0
  [ -n "$question" ] || question="you already have settings in $path. May I change them?"

  wizard_consent replace "$question" "replacing $path"
  case "$?" in
    2) return 2 ;;
    0) ;;
    *) return "$WIZARD_REFUSED" ;;
  esac

  if backup="$(wizard_backup "$path")"; then
    wizard_say "saved a copy of the old version at $backup"
    return 0
  fi
  wizard_say "could not save a copy of $path, so it has been left alone."
  return 1
}

# wizard_install_packages <logical>… - the sanctioned way to install anything.
#
# A machine that cannot install packages gets the manual instructions instead,
# which is the honest outcome and the one the platform layer is built to
# produce.
wizard_install_packages() {
  local cmd refresh status
  [ "$#" -gt 0 ] || return 2
  if ! cmd="$(platform_pkg_install_command "$@")" || [ -z "$cmd" ]; then
    platform_pkg_manual_hint "$@"
    return 1
  fi
  refresh="$(platform_pkg_refresh_command)" || refresh=""

  # One question covering both commands, not two: refreshing the package list
  # is part of installing, and asking about it separately would be asking a
  # person to make a decision they have no way to make.
  wizard_consent privileged "may I install $* on this computer?" "$refresh" "$cmd"
  status=$?
  [ "$status" = 2 ] && return 2
  [ "$status" = 0 ] || return "$WIZARD_REFUSED"

  # The refresh is best effort everywhere but Arch, where pacman -S against a
  # database that was never synced fails outright - so its failure is left for
  # the install below to report in terms of the packages the user asked for.
  if [ -n "$refresh" ]; then
    _wizard_exec "$refresh" || true
  fi
  _wizard_exec "$cmd"
}

# --------------------------------------------------------------- the plan

wizard_plan_reset() {
  _WIZARD_PLAN=""
  _WIZARD_PLAN_BYTES=0
  return 0
}

# wizard_plan_add <category> <text> [bytes]
wizard_plan_add() {
  local category="${1:-}" text="${2:-}" bytes="${3:-0}"
  [ -n "$category" ] && [ -n "$text" ] || return 2
  case "$category" in
    package|download|privileged|network|trust|config|service) ;;
    *) return 2 ;;
  esac
  case "$bytes" in
    ""|*[!0-9]*) bytes=0 ;;
  esac
  _WIZARD_PLAN="$_WIZARD_PLAN$category$_WIZARD_TAB$text
"
  _WIZARD_PLAN_BYTES=$((_WIZARD_PLAN_BYTES + bytes))
  return 0
}

# wizard_plan_is_consequential - status 0 when something in the plan deserves
# to be stopped for.
wizard_plan_is_consequential() {
  local line=""
  while IFS= read -r line || [ -n "$line" ]; do
    case "$line" in
      package"$_WIZARD_TAB"*|download"$_WIZARD_TAB"*|privileged"$_WIZARD_TAB"*\
        |network"$_WIZARD_TAB"*|trust"$_WIZARD_TAB"*) return 0 ;;
    esac
  done <<EOF
$_WIZARD_PLAN
EOF
  return 1
}

# _wizard_plan_heading <category> - what to call that group, in words.
_wizard_plan_heading() {
  case "$1" in
    package)    printf 'software to install' ;;
    download)   printf 'things to download' ;;
    privileged) printf 'steps that need your computer password' ;;
    network)    printf 'network' ;;
    trust)      printf 'certificates this computer would start trusting' ;;
    config)     printf 'settings to write' ;;
    service)    printf 'what will run in the background' ;;
    *)          printf '%s' "$1" ;;
  esac
}

# wizard_plan_show - the whole plan, grouped, in the order a person reads it.
wizard_plan_show() {
  local category line="" any=0 shown mb
  if [ -z "$_WIZARD_PLAN" ]; then
    wizard_say "nothing needs to be installed or changed."
    return 0
  fi
  for category in package download privileged trust network config service; do
    shown=0
    while IFS= read -r line || [ -n "$line" ]; do
      case "$line" in
        "$category$_WIZARD_TAB"*) ;;
        *) continue ;;
      esac
      if [ "$shown" = 0 ]; then
        shown=1
        any=1
        wizard_say "$(_wizard_plan_heading "$category"):"
      fi
      wizard_say "  - ${line#*"$_WIZARD_TAB"}"
    done <<EOF
$_WIZARD_PLAN
EOF
  done
  [ "$any" = 1 ] || wizard_say "nothing needs to be installed or changed."
  if [ "$_WIZARD_PLAN_BYTES" -gt 0 ]; then
    if [ "$_WIZARD_PLAN_BYTES" -lt 1000000 ]; then
      wizard_say "less than 1 MB of downloads and disk space."
    else
      mb=$(( (_WIZARD_PLAN_BYTES + 999999) / 1000000 ))
      wizard_say "about $mb MB of downloads and disk space."
    fi
  fi
  return 0
}
