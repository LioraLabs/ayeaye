#!/usr/bin/env bash
# Setup for ayeaye, as an eight-stage conversation.
#
# The stages are declared at the bottom of this file and the work inside them
# is above it. Nothing here detects a platform, names a package manager or
# knows what a service unit looks like: that is lib/platform.sh. Nothing here
# asks for permission in its own words either: that is lib/consent.sh. What
# this file owns is the lifecycle - the order of the conversation, what is
# remembered between runs, and what happens when a step does not work.
#
# A ticket adding work to setup does not edit this file. It drops a file into
# lib/steps/, which registers a step onto a stage that already exists; see
# lib/steps/README.md.
#
# bash 3.2: no associative arrays, no ${var,,}, no mapfile, no local -n.
set -euo pipefail

# ---------------------------------------------------------------- the release
#
# The version of ayeaye this script installs is named here and nowhere else.
# It is a pinned tag rather than a branch, so that two people running the same
# command on the same afternoon get the same software, and so that the person
# running it can see which ayeaye they are about to get: it is printed by
# --help, and printed again before anything is downloaded.
#
# Cutting a release sets these two lines and nothing else:
#
#   AYEAYE_VERSION  the tag that was published.
#   AYEAYE_SHA256   the sha256 of the artifact uploaded to it, when the
#                   release process knows it at the time it writes this file.
#                   Left empty, the checksum is taken from the SHA256SUMS
#                   published beside the artifact instead. With neither, the
#                   run says out loud that nothing was compared, rather than
#                   letting silence imply that it was.
AYEAYE_VERSION="v0.1.0"
AYEAYE_SHA256=""

# The published one-liner, quoted in --help and in README.md. It fetches this
# script, and this script pins the release above.
AYEAYE_INSTALL_URL="https://raw.githubusercontent.com/LioraLabs/ayeaye/main/install.sh"

# Where releases come from, where an unpacked one lives, and which terminal to
# ask questions on. Overridable so that an internal mirror, a scratch
# directory, or a terminal that is not /dev/tty can be pointed at without
# editing this file - which is also how the tests serve a release without
# touching the internet.
AYEAYE_RELEASE_BASE="${AYEAYE_RELEASE_BASE:-https://github.com/LioraLabs/ayeaye/releases/download}"
AYEAYE_DATA_DIR="${AYEAYE_DATA_DIR:-${XDG_DATA_HOME:-$HOME/.local/share}/ayeaye}"
AYEAYE_BOOTSTRAP_TTY="${AYEAYE_BOOTSTRAP_TTY:-/dev/tty}"

# REPO - the copy of ayeaye this run configures - cannot be worked out here any
# more. Whether there is one is the question the bootstrap below answers, and
# it is assigned at the end of that block, just above the line that sources
# lib/wizard.sh. Nothing above that point may use it.

CONF_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/ayeaye"
ENV_FILE="$CONF_DIR/env"
WIZARD_STATE_DIR="${XDG_STATE_HOME:-$HOME/.local/state}/ayeaye"
TOKEN_FILE="$WIZARD_STATE_DIR/token"
UNIT_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"

DEFAULTS=0
NO_SYSTEMD=0
FRESH=0
WIZARD_INTERACTIVE=1
# Set by the bootstrap below when it hands over to this script on a machine
# that turned out to have no terminal to ask questions on. Its own variable
# rather than WIZARD_INTERACTIVE, and only believed when it arrives together
# with the marker the handover sets, so that neither of them left in somebody's
# shell can quietly turn an attended run into an unattended one.
[ -z "${AYEAYE_BOOTSTRAP_UNATTENDED:-}" ] || [ -z "${AYEAYE_BOOTSTRAPPED:-}" ] \
  || WIZARD_INTERACTIVE=0
WIZARD_DETAILS=0
WIZARD_AUTO_CONSENT=""

usage() {
  cat <<'USAGE'
One-command setup for ayeaye. Safe to re-run: it keeps the settings and the
key you already have, asks before changing anything, and picks up where an
interrupted run stopped.

  ./install.sh               interactive, sane defaults
  ./install.sh --defaults    accept every default, no prompts
  ./install.sh --no-systemd  skip the unit; prints the manual run command
  ./install.sh --yes         say yes to installing and to replacing settings,
                             but never to a network or a certificate change
  ./install.sh --details     show the raw commands as they run
  ./install.sh --fresh       forget what earlier runs recorded and start over

What it does, in eight steps:

  1. explains what setup may change, and what it will never change
  2. works out what this computer is and what it already has
  3. says in plain words what ayeaye can do here
  4. asks how you want to reach it and what to switch on
  5. lists everything it is about to install or change, and asks
  6. installs and configures what you chose
  7. starts it in the background and checks that it answers
  8. prints the address to open on your phone, and anything left to do

Nothing is installed, downloaded, opened to the network or trusted without a
question first, and answering no to any of them leaves this computer exactly
as it was.
USAGE
  cat <<BOOTSTRAP

On a computer that has no copy of ayeaye yet, the same setup can be run
straight from the internet:

  curl -fsSL $AYEAYE_INSTALL_URL | bash

That asks before it downloads anything, fetches ayeaye $AYEAYE_VERSION into
  $AYEAYE_DATA_DIR
and then does exactly what running it from a copy does. Add arguments to it
with -s --, for example: | bash -s -- --yes
BOOTSTRAP
}

for arg in "$@"; do
  case "$arg" in
    --defaults)   DEFAULTS=1; WIZARD_INTERACTIVE=0 ;;
    --no-systemd) NO_SYSTEMD=1 ;;
    --yes)        WIZARD_AUTO_CONSENT="privileged download replace" ;;
    --details)    WIZARD_DETAILS=1 ;;
    --fresh)      FRESH=1 ;;
    -h|--help)    usage; exit 0 ;;
    *) echo "unknown option: $arg (try --help)" >&2; exit 2 ;;
  esac
done

# ================================================ is ayeaye here, or not yet?
#
# There are two ways this file gets run and they need different things. From a
# copy of ayeaye - a clone, or a release already unpacked - everything it needs
# is next to it, and it configures that copy without fetching a byte. Piped
# from the internet there is nothing next to it at all, and the first job is to
# fetch the pinned release and hand over to the copy that comes out of it.
#
# Which of the two is decided by looking rather than by guessing. Under
# `curl … | bash` the script has no directory: "$(dirname "${BASH_SOURCE[0]}")"
# resolves to wherever the person happened to be standing, which is a real
# directory that will happily be treated as a broken checkout. So the question
# asked is not "where did this file come from" but "is ayeaye actually there".
#
# Everything here runs before lib/ exists, which is why it says things in its
# own words instead of through lib/ui.sh and asks in its own words instead of
# through lib/consent.sh. It is the one place in this repository allowed to
# do either, it is confined to what a bootstrap cannot avoid, and
# tests/cases/wizard_contract_test.sh is what keeps it confined: the fetch
# below is exempted from the tree-wide download lint by name, and the lint
# fails if a second one appears.

_boot_say()   { printf '%s\n' "$*"; }
_boot_blank() { printf '\n'; }
# Something went wrong, as opposed to somebody said no. On stderr, so that it
# is still seen by a person who sent the rest of this to a file.
_boot_err()   { printf '%s\n' "$*" >&2; }

# _bootstrap_payload <dir> - can setup run from this directory? Exactly the
# two files it cannot start without, so an empty directory - which is what the
# working directory is, to a script arriving down a pipe - is never mistaken
# for a copy of ayeaye. A freshly unpacked release is held to more than this;
# see _bootstrap_unpack.
_bootstrap_payload() {
  [ -n "${1:-}" ] || return 1
  [ -f "$1/install.sh" ] || return 1
  [ -f "$1/lib/wizard.sh" ] || return 1
  return 0
}

# _bootstrap_here - the directory this script was read from, when it really was
# read from one. Empty under a pipe, where BASH_SOURCE holds the name of the
# shell rather than a path. Always 0: "no directory" is an answer, not a fault.
_bootstrap_here() {
  local self="${BASH_SOURCE[0]:-}" dir
  case "$self" in
    ""|bash|-bash|sh|-sh|dash|zsh|main|/dev/stdin|/dev/fd/*|/proc/self/fd/*) return 0 ;;
  esac
  [ -f "$self" ] || return 0
  case "$self" in
    */*) dir="${self%/*}" ;;
    *)   dir="." ;;
  esac
  # CDPATH would make cd print somewhere else entirely, and this runs on
  # whatever shell environment the machine happens to have.
  dir="$(CDPATH='' cd -- "$dir" 2>/dev/null && pwd)" || return 0
  printf '%s' "$dir"
  return 0
}

# _bootstrap_tty - is there a terminal to ask questions on? Opening it is the
# only honest test: /dev/tty exists on a machine with no controlling terminal
# and fails at the moment it is read.
_bootstrap_tty() {
  [ -n "$AYEAYE_BOOTSTRAP_TTY" ] || return 1
  # In a subshell: a redirection that fails is reported by the shell that
  # performs it, before the 2>/dev/null on the same command can apply, and a
  # headless machine is not a thing to print an error about.
  ( : <"$AYEAYE_BOOTSTRAP_TTY" ) 2>/dev/null || return 1
  return 0
}

# _bootstrap_may_fetch - permission to use the network, in the same shape
# lib/consent.sh gives it once lib/ is here: --yes is a yes given in advance,
# a run that may not ask is a no, and anything that is not a yes is a no.
_bootstrap_may_fetch() {
  local auto reply
  for auto in ${WIZARD_AUTO_CONSENT:-}; do
    if [ "$auto" = download ]; then
      _boot_say "fetching it, because --yes already said yes to downloading."
      return 0
    fi
  done
  if [ "$WIZARD_INTERACTIVE" = 0 ] || ! _bootstrap_tty; then
    _boot_say "this run has no way to ask, and downloading is not something to"
    _boot_say "decide on somebody else's behalf."
    _boot_say "run it again with --yes to answer this one in advance."
    return 1
  fi
  reply=""
  read -r -p "may I download ayeaye $AYEAYE_VERSION? [n]: " reply \
    <"$AYEAYE_BOOTSTRAP_TTY" || reply=""
  case "$reply" in
    y|Y|yes|YES|Yes) return 0 ;;
  esac
  return 1
}

# _bootstrap_fetch <url> <destination> - the only download in this file, and
# the reason it is the only one: a second would need its own exemption from
# the lint, and the lint is what keeps this honest.
#
# Resumable, because a bootstrap interrupted halfway down a release should
# carry on rather than start again: what arrived is left in <destination>.part
# and the next run asks the server to continue from there. A partial file the
# server will not resume is worth nothing, so it is thrown away and the whole
# thing is asked for once. Nothing here trusts those bytes - what they add up
# to is checked, below, before anything is unpacked or run.
_bootstrap_fetch() {
  local url="${1:-}" dest="${2:-}" dir resume status attempt opts
  [ -n "$url" ] && [ -n "$dest" ] || return 2
  dir="${dest%/*}"
  [ "$dir" = "$dest" ] || mkdir -p "$dir" 2>/dev/null || return 1
  if ! command -v curl >/dev/null 2>&1; then
    _boot_err "this computer has no way to download files (curl is missing)."
    _boot_err "install curl and run setup again, or fetch it yourself:"
    _boot_err "  $url"
    return 1
  fi
  # -L follows redirects, and a redirect to plain http would quietly undo the
  # only protection an unchecksummed release has. Said out loud to curl for an
  # https URL; a mirror somebody deliberately pointed at over http is their
  # decision and is left alone. The timeouts are what turn a black-holed
  # connection into the resumable failure the rest of this is built around,
  # rather than an installer that never comes back.
  opts="--connect-timeout 20 --max-time 1800"
  case "$url" in
    https://*) opts="$opts --proto =https --proto-redir =https" ;;
  esac
  attempt=1
  while [ "$attempt" -le 2 ]; do
    resume=""
    [ -s "$dest.part" ] && resume="-C -"
    status=0
    # shellcheck disable=SC2086
    curl -fsSL --retry 2 $opts $resume -o "$dest.part" "$url" || status=$?  # bootstrap-fetch
    if [ "$status" = 0 ]; then
      mv -f "$dest.part" "$dest" 2>/dev/null || return 1
      return 0
    fi
    # A whole download that failed leaves its bytes where the next run can
    # carry on from them. A resume that failed means those bytes are the
    # problem: throw them away and ask once for the whole thing.
    [ -n "$resume" ] || break
    rm -f "$dest.part" 2>/dev/null || true
    attempt=$((attempt + 1))
  done
  return 1
}

# _bootstrap_digest <file> - the sha256 of a file, using whatever this machine
# has. 1 when it has none of them, which is a thing to say out loud rather
# than a thing to fail over.
_bootstrap_digest() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | awk '{print $1}'
  elif command -v openssl >/dev/null 2>&1; then
    openssl dgst -sha256 "$1" | awk '{print $NF}'
  else
    return 1
  fi
}

# _bootstrap_verify <artifact> <sums-url> <name> - 0 to carry on, 1 to stop.
#
# Says which of the three things happened, every time, because silence here
# reads as "verified" and is the one thing this must never mean:
#
#   the checksum matched
#   there was no checksum, or no way to work one out, and here is which
#   the checksum did not match - which stops the run
#
# The strongest of the three is AYEAYE_SHA256, written into this file by the
# release process: it arrives with the script over the same connection the
# person already chose to trust, and cannot be swapped out by whoever is in a
# position to swap out the artifact.
_bootstrap_verify() {
  local artifact="$1" sums_url="$2" name="$3" expected="" actual sums
  if [ -n "$AYEAYE_SHA256" ]; then
    expected="$AYEAYE_SHA256"
  else
    sums="$AYEAYE_DATA_DIR/downloads/SHA256SUMS-$AYEAYE_VERSION"
    # Never resumed: a checksum file half of one run and half of another reads
    # as a release that says nothing about this artifact, which is the one
    # answer here that must never be arrived at by accident.
    rm -f "$sums.part" "$sums" 2>/dev/null || true
    if _bootstrap_fetch "$sums_url" "$sums" >/dev/null 2>&1 && [ -f "$sums" ]; then
      expected="$(awk -v want="$name" \
        '{ n = $2; sub(/^\*/, "", n); if (n == want) { print $1; exit } }' "$sums")"
      if [ -z "$expected" ]; then
        _boot_say "not checked: the checksums published with ayeaye $AYEAYE_VERSION say"
        _boot_say "nothing about $name, so there was nothing to compare this against."
      fi
    else
      _boot_say "not checked: no checksums could be fetched for ayeaye $AYEAYE_VERSION -"
      _boot_say "either the release published none, or they did not arrive. The only"
      _boot_say "thing protecting this download was the encrypted connection to the"
      _boot_say "server."
    fi
  fi
  [ -n "$expected" ] || return 0

  actual="$(_bootstrap_digest "$artifact")" || actual=""
  if [ -z "$actual" ]; then
    _boot_say "not checked: this computer has no sha256sum, shasum or openssl, so the"
    _boot_say "checksum published with the release could not be compared."
    return 0
  fi
  if [ "$actual" != "$expected" ]; then
    rm -f "$artifact" 2>/dev/null || true
    _boot_err ""
    _boot_err "STOP. What was downloaded is not what ayeaye $AYEAYE_VERSION published."
    _boot_err "  expected $expected"
    _boot_err "  received $actual"
    _boot_err ""
    _boot_err "Nothing has been installed, and the file has been deleted. This can be"
    _boot_err "an interrupted download, and it can be somebody in the way of this one."
    _boot_err "Try again, and if it happens twice do not run it."
    return 1
  fi
  if [ -n "$AYEAYE_SHA256" ]; then
    _boot_say "checked: what arrived matches the checksum written into setup itself,"
    _boot_say "which came from somewhere else than the file it just checked."
  else
    # Worth having and worth being exact about: it catches a download that
    # broke in transit, and it does not catch somebody who is able to replace
    # the artifact, because they could replace this alongside it.
    _boot_say "checked: what arrived matches the checksum ayeaye $AYEAYE_VERSION publishes"
    _boot_say "beside it, which came down the same connection."
  fi
  return 0
}

# _bootstrap_unpack <artifact> <destination> - through a scratch directory and
# a rename, so that an unpacking interrupted halfway never leaves something
# behind that looks like a working copy.
#
# The archive is read before it is written out. An entry naming an absolute
# path or climbing out with ".." would land wherever it liked and is refused:
# the checksum is what makes a release trustworthy and there are releases that
# publish none, so this does not depend on having had one. It reads entry names
# and not symlink targets - a link pointing out of the tree is left to tar,
# which refuses to follow one on the way in.
_bootstrap_unpack() {
  local artifact="$1" dest="$2" work top entry count escaping
  command -v tar >/dev/null 2>&1 || {
    _boot_err "this computer has no tar, so the release cannot be unpacked."
    return 1
  }
  # Read into a variable rather than tested with grep -q: a grep that stops at
  # the first match kills tar with a broken pipe, and under `set -o pipefail`
  # that is the status the test would have seen.
  escaping="$(tar -tzf "$artifact" 2>/dev/null | grep -e '^/' -e '^\.\./' -e '/\.\./' || true)"
  if [ -n "$escaping" ]; then
    _boot_err "this release contains files that would be written outside the place"
    _boot_err "setup unpacks into, which a release does not do. Refusing it."
    return 1
  fi
  # Anything left behind by a run that was interrupted mid-unpack. Its own
  # scratch directory is removed on every path out of here, but a ctrl-c is
  # not a path out of here.
  rm -rf "$AYEAYE_DATA_DIR"/releases/.unpacking.* 2>/dev/null || true
  work="$AYEAYE_DATA_DIR/releases/.unpacking.$$"
  mkdir -p "$work" 2>/dev/null || return 1
  tar -xzf "$artifact" -C "$work" 2>/dev/null || { rm -rf "$work"; return 1; }
  # A release archive holds one directory. Anything else is unpacked as it is,
  # and judged by the same question everything else is judged by.
  count=0
  top="$work"
  for entry in "$work"/*; do
    [ -e "$entry" ] || continue
    top="$entry"
    count=$((count + 1))
  done
  [ "$count" = 1 ] && [ -d "$top" ] || top="$work"
  # More is asked of a fresh release than of a directory somebody is already
  # standing in: this one has to be whole, not merely startable.
  if ! _bootstrap_payload "$top" || [ ! -f "$top/bin/ayeaye" ]; then
    rm -rf "$work"
    return 1
  fi
  # The copy that is already there is moved aside rather than deleted, and only
  # let go of once its replacement is in place: a rename that fails must not be
  # how somebody ends up with no ayeaye at all.
  if [ -e "$dest" ]; then
    rm -rf "$dest.previous" 2>/dev/null || true
    mv "$dest" "$dest.previous" 2>/dev/null || true
  fi
  if ! mv "$top" "$dest" 2>/dev/null; then
    if [ -e "$dest.previous" ]; then
      mv "$dest.previous" "$dest" 2>/dev/null || true
    fi
    rm -rf "$work"
    return 1
  fi
  rm -rf "$dest.previous" "$work" 2>/dev/null || true
  return 0
}

# _bootstrap_handover <payload> <argument>… - run the copy that is now on this
# computer, with the arguments the person actually typed.
#
# Standard input is the whole trap. Under `curl … | bash` it is the script
# itself, and a wizard that read it would swallow its own source and then take
# every default without anybody having been asked anything - a run that
# installs nothing, exposes nothing and looks exactly like success. So the
# terminal is reopened for the copy being handed to, and when there is no
# terminal at all the run is made explicitly unattended and says so.
_bootstrap_handover() {
  local payload="$1"
  shift
  AYEAYE_BOOTSTRAPPED=1
  export AYEAYE_BOOTSTRAPPED
  if _bootstrap_tty; then
    exec "${BASH:-bash}" "$payload/install.sh" "$@" <"$AYEAYE_BOOTSTRAP_TTY"
  fi
  _boot_say "there is no terminal here to ask questions on, so the rest of setup"
  _boot_say "takes the default answer to every one of them."
  _boot_blank
  # /dev/null, not the pipe this script arrived down: whatever of it is still
  # unread is the text of this file, and the first thing downstream to read
  # standard input would get a mouthful of shell script.
  AYEAYE_BOOTSTRAP_UNATTENDED=1
  export AYEAYE_BOOTSTRAP_UNATTENDED
  exec "${BASH:-bash}" "$payload/install.sh" "$@" </dev/null
}

# _bootstrap <argument>… - fetch the pinned release and hand over to it. Never
# returns: it execs the unpacked copy, or it exits.
_bootstrap() {
  local payload artifact name url sums_url

  # A copy that unpacked itself and still cannot find its own files would
  # otherwise download it all again, for ever.
  if [ -n "${AYEAYE_BOOTSTRAPPED:-}" ]; then
    _boot_err "setup unpacked ayeaye $AYEAYE_VERSION but the copy is not complete."
    _boot_err "delete $AYEAYE_DATA_DIR and run it again."
    exit 1
  fi

  name="ayeaye-$AYEAYE_VERSION.tar.gz"
  payload="$AYEAYE_DATA_DIR/releases/$AYEAYE_VERSION"
  artifact="$AYEAYE_DATA_DIR/downloads/$name"
  url="$AYEAYE_RELEASE_BASE/$AYEAYE_VERSION/$name"
  sums_url="$AYEAYE_RELEASE_BASE/$AYEAYE_VERSION/SHA256SUMS"

  # Already unpacked by an earlier run: nothing to fetch, and so nothing to
  # ask about either.
  if _bootstrap_payload "$payload"; then
    _boot_say "ayeaye $AYEAYE_VERSION is already on this computer, at"
    _boot_say "  $payload"
    _boot_blank
    _bootstrap_handover "$payload" "$@"
  fi

  _boot_say "ayeaye setup"
  _boot_blank
  _boot_say "This computer does not have ayeaye on it yet, so the first thing"
  _boot_say "setup has to do is fetch it."
  _boot_blank
  _boot_say "  version: ayeaye $AYEAYE_VERSION"
  _boot_say "  from:    $url"
  _boot_say "  into:    $payload"
  _boot_blank
  _boot_say "Nothing else is installed or changed by this step, and the questions"
  _boot_say "about the rest of setup come after it."
  _boot_blank

  # An artifact an earlier run already fetched is not fetched again, but it is
  # not trusted either: it is checked below exactly as a fresh one is.
  if [ -f "$artifact" ]; then
    _boot_say "an earlier run already downloaded this release, so setup checks what"
    _boot_say "it has instead of fetching it again."
    _boot_blank
  fi

  # The one question this file asks. It is asked whenever anything at all will
  # come off the network - the release, or the checksums for one that is
  # already here - and everything past it is fetching, checking and unpacking.
  # Every other question in setup belongs to the wizard.
  if [ ! -f "$artifact" ] || [ -z "$AYEAYE_SHA256" ]; then
    if ! _bootstrap_may_fetch; then
      _boot_blank
      _boot_say "nothing was downloaded, and this computer is exactly as it was."
      _boot_say "to do it by hand instead:"
      _boot_say "  git clone https://github.com/LioraLabs/ayeaye"
      _boot_say "  cd ayeaye && ./install.sh"
      exit 3
    fi
  fi

  if [ ! -f "$artifact" ]; then
    if ! _bootstrap_fetch "$url" "$artifact"; then
      _boot_err ""
      _boot_err "the download did not work, so nothing has been changed here."
      _boot_err "check the connection and run it again - what did arrive is kept,"
      _boot_err "and the next run carries on from where this one stopped."
      exit 1
    fi
  fi

  _bootstrap_verify "$artifact" "$sums_url" "$name" || exit 1

  if ! _bootstrap_unpack "$artifact" "$payload"; then
    # Deleted rather than kept: it is the reason this failed, and keeping it
    # would make every later run fail in exactly the same way without ever
    # fetching a good one.
    rm -f "$artifact" 2>/dev/null || true
    _boot_err ""
    _boot_err "the release could not be unpacked, so nothing has been changed here."
    _boot_err "what was downloaded has been deleted; running setup again fetches it"
    _boot_err "afresh."
    exit 1
  fi
  _boot_say "unpacked into $payload"
  _boot_blank
  _bootstrap_handover "$payload" "$@"
}

REPO="$(_bootstrap_here)"
if ! _bootstrap_payload "$REPO"; then
  _bootstrap "$@"
fi

# shellcheck source=lib/wizard.sh
. "$REPO/lib/wizard.sh"

# What the conversation decided. These are working copies: the decisions
# themselves live in the state file, because a resumed run skips the step that
# made them and a variable set by a step that did not run this time holds
# nothing but its default. Anything one stage tells a later stage goes through
# wizard_remember, and is read back where it is used.
CHANGE_SETTINGS=0
BIND="127.0.0.1"
PORT="8911"
HOSTS=""
NTFY=""
TS_HOST=""
STARTED=0
SERVICE_KIND="none"

# A run stopped with ctrl-c is an interrupted run, not a broken one: the state
# file already holds everything finished so far, so the only thing left to do
# is say so.
_interrupted() {
  printf '\n'
  wizard_say "stopped. Nothing that already worked has been lost."
  wizard_say "Run ./install.sh again and it carries on from here."
  exit 130
}
trap _interrupted INT TERM

# ============================================================ 1. welcome

step_welcome() {
  wizard_say "ayeaye puts a small web page on your phone that shows what the"
  wizard_say "coding agents on this computer are doing, and lets you talk to them."
  wizard_blank
  wizard_say "repo: $REPO"
  wizard_blank
  wizard_say "What setup may change on this computer:"
  wizard_say "  - a settings file at $ENV_FILE"
  wizard_say "  - a private key at $TOKEN_FILE, so that only you can open the page"
  wizard_say "  - a background service that starts ayeaye when you log in"
  wizard_say "  - software ayeaye needs, if you say yes when it asks"
  wizard_blank
  wizard_say "What it will never do without asking first: install anything, download"
  wizard_say "anything, change what this computer accepts from the network, trust a"
  wizard_say "new certificate, or replace settings you already have. Answering no to"
  wizard_say "any of those leaves this computer exactly as it is now."
  wizard_blank
  wizard_say "The page is locked to a key kept on this computer, and by default"
  wizard_say "ayeaye listens to this computer only - nothing else on your network"
  wizard_say "can reach it until you ask for that."
  if [ "$WIZARD_RESUMING" = 1 ]; then
    wizard_blank
    wizard_say "The last run stopped before it finished. This one carries on from"
    wizard_say "where it got to, and skips what was already done."
  fi
  return "$WIZARD_STAGE_OK"
}

# ============================================================= 2. detect

step_detect_machine() {
  # One probe for the whole run: the accessors below read a cache, and a cache
  # filled inside a command substitution would die with the subshell.
  platform_detect
  wizard_item "computer" "$(platform_pretty)"
  wizard_item "software" "$(platform_pkg_manager)"
  wizard_item "startup" "$(platform_service_manager)"
  wizard_detail "platform: $(platform_summary)"
  wizard_detail "privilege: $(platform_privilege)"
  return "$WIZARD_STAGE_OK"
}

step_detect_needs() {
  local dep missing=""
  for dep in tmux python3; do
    if command -v "$dep" >/dev/null 2>&1; then
      wizard_item ok "$dep"
    else
      wizard_item MISSING "$dep (required)"
      missing="$missing $dep"
    fi
  done
  [ -n "$missing" ] || return "$WIZARD_STAGE_OK"

  wizard_blank
  wizard_say "ayeaye cannot run without these:$missing"
  wizard_say "tmux is what the coding agents run inside, and python3 is what"
  wizard_say "ayeaye itself is written in."
  wizard_blank
  # The one door to installing anything, here and in every later ticket.
  # shellcheck disable=SC2086
  if wizard_install_packages $missing; then
    wizard_say "installed."
    return "$WIZARD_STAGE_OK"
  fi
  wizard_blank
  wizard_say "install the missing packages and re-run."
  return "$WIZARD_STAGE_FAIL"
}

# ====================================================== 3. what it can do

# _have_transcriber - is there anything on this computer that turns speech
# into words?
#
# One question, asked from one list. whisper.cpp renamed its binaries and both
# names are still in the world, so a check that spells one of them refuses a
# machine that transcribes perfectly - which is exactly how bin/ayeaye's talk
# button and bin/voice-dictate's pipeline came to disagree about the same
# program. The list lives in lib/steps/20-hardware.sh and is borrowed here
# rather than repeated, with a fallback for a file sourced on its own.
_have_transcriber() {
  local name
  for name in ${_HW_WHISPER_COMMANDS:-whisper-server whisper-cli whisper-cpp whisper}; do
    command -v "$name" >/dev/null 2>&1 && return 0
  done
  return 1
}

step_report() {
  local dep voice_missing="" transcriber=0
  if ! command -v tailscale >/dev/null 2>&1; then
    wizard_item note "tailscale not found: you will need another way to serve https to the phone"
  fi
  for dep in ffmpeg whisper-server ollama; do
    command -v "$dep" >/dev/null 2>&1 || voice_missing="$voice_missing $dep"
  done
  if [ -n "$voice_missing" ]; then
    wizard_say "  voice tier: missing$voice_missing (optional; text-only mode works without them)"
  else
    wizard_say "  voice tier: all present (ffmpeg, whisper-server, ollama)"
  fi
  wizard_blank
  wizard_say "What that means for you:"
  wizard_say "  - reading and typing to your agents from the phone: yes, always."
  if command -v ffmpeg >/dev/null 2>&1 && _have_transcriber; then
    transcriber=1
  fi
  if [ "$transcriber" = 1 ]; then
    wizard_say "  - talking to them out loud: yes, this computer can transcribe."
  else
    wizard_say "  - talking to them out loud: not yet. ayeaye works without it, and"
    wizard_say "    finds a listening server the moment one starts answering. A"
    wizard_say "    command-line one it reads when it starts, so restart it after."
  fi
  wizard_blank
  wizard_say "Recommended here: text on the phone now, over an https address only"
  wizard_say "you can reach. Voice is worth adding later and costs nothing to skip."
  return "$WIZARD_STAGE_OK"
}

# ======================================================= 4. your choices

step_choose() {
  if command -v tailscale >/dev/null 2>&1; then
    TS_HOST="$(tailscale status --json 2>/dev/null \
      | python3 -c 'import sys,json;print(json.load(sys.stdin)["Self"]["DNSName"].rstrip("."))' \
      2>/dev/null)" || TS_HOST=""
  fi

  if [ -s "$ENV_FILE" ]; then
    wizard_say "config exists at $ENV_FILE"
    wizard_say "Nothing in it is lost either way: settings you are not asked about"
    wizard_say "here stay exactly as they are, and the old file is copied aside"
    wizard_say "before anything changes."
    # Replacing configuration somebody already has is one of the five things
    # that is never done without asking, so it goes through the primitive.
    if wizard_consent replace "rewrite it? (n keeps the current file)" \
         "merging into $ENV_FILE"; then
      CHANGE_SETTINGS=1
    else
      CHANGE_SETTINGS=0
      wizard_say "keeping existing config"
    fi
  else
    CHANGE_SETTINGS=1
  fi

  wizard_remember answer.change_settings "$CHANGE_SETTINGS"
  if [ "$CHANGE_SETTINGS" = 0 ]; then
    return "$WIZARD_STAGE_OK"
  fi

  # Each question offers what is already configured, so that pressing return
  # means "leave this as it is" - which is what pressing return looks like it
  # means, and what somebody rerunning setup to change one thing expects of the
  # other three. On a first run there is nothing configured and the offer is
  # the built-in default.
  wizard_blank
  wizard_say "Four questions about how to reach ayeaye. Press return to keep"
  wizard_say "what is in the square brackets."
  wizard_blank

  wizard_say "Which address should ayeaye answer on? 127.0.0.1 means this"
  wizard_say "computer only, which is the safe answer unless something else on"
  wizard_say "your network has to reach it directly."
  wizard_ask "bind address" "$(_current AYEAYE_BIND answer.bind 127.0.0.1)"
  BIND="$REPLY"

  wizard_say "Which port? Any number will do; change it only if something else"
  wizard_say "on this computer already uses this one."
  wizard_ask "port" "$(_current AYEAYE_PORT answer.port 8911)"
  PORT="$REPLY"

  wizard_say "If you reach ayeaye through an https address - a tailscale name,"
  wizard_say "or your own domain - name it here so the app accepts it."
  if [ -n "$(_current AYEAYE_ALLOWED_HOSTS answer.hosts "")" ]; then
    wizard_ask "allowed hosts (your https front)" \
      "$(_current AYEAYE_ALLOWED_HOSTS answer.hosts "")"
  elif [ -n "$TS_HOST" ]; then
    wizard_ask "allowed hosts (your https front)" "$TS_HOST"
  else
    wizard_ask "allowed hosts (your https front, comma separated; empty for none)" ""
  fi
  HOSTS="$REPLY"

  wizard_say "ayeaye can send your phone a notification when an agent needs you."
  wizard_say "That needs an ntfy topic address; leave it empty to switch it off."
  wizard_ask "ntfy topic URL for push notifications (empty disables)" \
    "$(_current VOICE_NTFY_URL answer.ntfy "")"
  NTFY="$REPLY"

  _decide_exposure
  _remember_choices
  wizard_plan_add config "your settings, at $ENV_FILE"
  return "$WIZARD_STAGE_OK"
}

# _current <env-key> <state-key> <built-in> - what to offer as the default.
#
# What is in the settings file wins, because that is what is true right now.
# Then what was chosen last time, which is what an interrupted run left behind
# before it got as far as writing anything. Then the built-in.
_current() {
  local value
  value="$(wizard_env_get "$ENV_FILE" "$1" "")"
  [ -n "$value" ] || value="$(wizard_state_get "$2" "")"
  [ -n "$value" ] || value="$3"
  printf '%s' "$value"
}

# Binding anywhere but this computer is the one choice that puts ayeaye within
# reach of something else, so it is a consent decision - recorded either way,
# and never one an unattended run is allowed to take.
_decide_exposure() {
  case "$BIND" in
    127.0.0.1|localhost|::1|"")
      wizard_consent_record expose refused "listening on this computer only"
      wizard_plan_add network "ayeaye will answer on $BIND:$PORT, this computer only"
      return 0
      ;;
  esac
  if [ "$WIZARD_INTERACTIVE" = 0 ]; then
    # Belt and braces. An unattended run never reads an answer, so it cannot
    # get here today - and the day a default changes, this is what stops that
    # change from putting somebody's machine on a network without them.
    wizard_consent_record expose refused "an unattended run never opens ayeaye to a network"
    BIND="127.0.0.1"
    wizard_plan_add network "ayeaye will answer on $BIND:$PORT, this computer only"
    return 0
  fi
  # The answer to the address question is the consent to exposure - it is an
  # explicit choice, typed a moment ago, and asking a second question about it
  # would be asking the same thing twice. What it does need is for the
  # consequence to be said out loud in words rather than left implied by an IP
  # address, and for the decision to be in the ledger like every other one.
  wizard_blank
  wizard_say "note: $BIND is not just this computer. Anything that can reach"
  wizard_say "this machine on your network will be able to reach ayeaye, and"
  wizard_say "the key in the bookmark is the only thing keeping them out."
  wizard_say "Answer 127.0.0.1 instead if you did not mean that."
  wizard_consent_record expose granted "you chose to listen on $BIND"
  wizard_plan_add network "ayeaye will answer on $BIND:$PORT, which other computers can reach"
  return 0
}

_remember_choices() {
  wizard_remember answer.bind "$BIND"
  wizard_remember answer.port "$PORT"
  wizard_remember answer.hosts "$HOSTS"
  wizard_remember answer.ntfy "$NTFY"
}

# ========================================================== 5. the plan

step_plan() {
  wizard_say "Before anything changes, here is all of it:"
  wizard_blank
  wizard_plan_show
  wizard_blank
  wizard_detail_hint

  # Only consequential work is worth stopping for. Writing your own settings
  # file and your own background service is not; installing, downloading,
  # privilege, trust and the network are.
  if [ "$WIZARD_INTERACTIVE" = 0 ]; then
    return "$WIZARD_STAGE_OK"
  fi
  if ! wizard_plan_is_consequential; then
    return "$WIZARD_STAGE_OK"
  fi
  if wizard_confirm "go ahead with all of that?" "y"; then
    return "$WIZARD_STAGE_OK"
  fi
  wizard_say "stopping here. Nothing has been changed."
  wizard_say "Run ./install.sh again whenever you want to pick this up."
  exit 0
}

# ======================================================== 6. do the work

step_write_settings() {
  local backup
  # From the state file, not from the variables: the stage that made these
  # decisions may have been finished by an earlier run that was interrupted
  # before it got here.
  CHANGE_SETTINGS="$(wizard_state_get answer.change_settings 0)"
  BIND="$(wizard_state_get answer.bind 127.0.0.1)"
  PORT="$(wizard_state_get answer.port 8911)"
  HOSTS="$(wizard_state_get answer.hosts "")"
  NTFY="$(wizard_state_get answer.ntfy "")"

  if [ "$CHANGE_SETTINGS" = 0 ]; then
    wizard_say "keeping the settings that are already there."
    return "$WIZARD_STAGE_SKIP"
  fi

  if [ -s "$ENV_FILE" ]; then
    if backup="$(wizard_backup "$ENV_FILE")"; then
      wizard_say "saved a copy of your old settings at $backup"
    else
      wizard_say "could not save a copy of $ENV_FILE, so it has been left alone."
      return "$WIZARD_STAGE_FAIL"
    fi
    # A merge, not a rewrite: anything in the file this run did not ask about
    # belongs to whoever put it there.
    wizard_env_merge "$ENV_FILE" \
      "AYEAYE_BIND=$BIND" "AYEAYE_PORT=$PORT" \
      "AYEAYE_ALLOWED_HOSTS=$HOSTS" "VOICE_NTFY_URL=$NTFY" \
      || return "$WIZARD_STAGE_FAIL"
  else
    wizard_env_render "$REPO/env.template" "$ENV_FILE" \
      "AYEAYE_BIND=$BIND" "AYEAYE_PORT=$PORT" \
      "AYEAYE_ALLOWED_HOSTS=$HOSTS" "VOICE_NTFY_URL=$NTFY" \
      || return "$WIZARD_STAGE_FAIL"
  fi
  wizard_say "wrote $ENV_FILE"
  return "$WIZARD_STAGE_OK"
}

step_make_key() {
  if [ ! -s "$TOKEN_FILE" ]; then
    mkdir -p "$WIZARD_STATE_DIR"
    # Same convention as the server: it would make this file itself on first
    # run. Making it here just means the address is printable now.
    ( umask 077
      python3 -c 'import secrets;print(secrets.token_urlsafe(32))' > "$TOKEN_FILE" ) \
      || return "$WIZARD_STAGE_FAIL"
    wizard_say "generated auth token at $TOKEN_FILE"
  else
    wizard_say "keeping the key you already have, so a bookmark already on your"
    wizard_say "phone still works."
  fi
  chmod 600 "$TOKEN_FILE"
  return "$WIZARD_STAGE_OK"
}

# Whatever is in the settings file now is what the last two stages report, so a
# kept config gets a correct summary and a correct service unit. Read at the
# start of every stage that needs it rather than once in a step that a resumed
# run may well skip.
_load_effective_values() {
  BIND="$(wizard_env_get "$ENV_FILE" AYEAYE_BIND 127.0.0.1)"
  PORT="$(wizard_env_get "$ENV_FILE" AYEAYE_PORT 8911)"
  HOSTS="$(wizard_env_get "$ENV_FILE" AYEAYE_ALLOWED_HOSTS "")"
  NTFY="$(wizard_env_get "$ENV_FILE" VOICE_NTFY_URL "")"
  return 0
}

# ====================================================== 7. start it up

_manual_instructions() {
  wizard_say "run the server with: $REPO/bin/ayeaye"
  wizard_say "(it reads $ENV_FILE by itself)"
}

step_service() {
  # The lifecycle's hook, and nothing more. What a service definition
  # contains, how it is installed on each platform, how an existing one is
  # migrated or repaired, and what to say on a machine that has no service
  # manager at all are one subject and they live together, in
  # lib/steps/70-service.sh. This file owns when that happens, not what it is.
  #
  # A missing implementation is reported as unfinished rather than as done:
  # the manual instructions are still true, and the run picks it up next time.
  if ! command -v service_step >/dev/null 2>&1; then
    wizard_say "the part of setup that starts ayeaye for you is not installed."
    _manual_instructions
    return "$WIZARD_STAGE_PENDING"
  fi
  service_step
}

# ============================================================= 8. done

step_summary() {
  local tier="text-only" token url_host first_host left
  _load_effective_values
  # The service stage is where SERVICE_KIND is worked out, and a resumed run
  # skips it once it is done - so ask the platform layer again rather than
  # trusting a variable that may never have been set this time.
  if [ "$NO_SYSTEMD" = 0 ]; then
    SERVICE_KIND="$(platform_service_manager)"
  fi
  [ -n "$NTFY" ] && tier="$tier +notifications"
  if command -v ffmpeg >/dev/null 2>&1 && _have_transcriber; then
    tier="$tier +voice"
  fi
  token="$(cat "$TOKEN_FILE" 2>/dev/null || true)"
  url_host="$BIND"
  first_host="${HOSTS%%,*}"

  wizard_say "tier    : $tier (probed live; the app adapts at runtime)"
  wizard_say "config  : $ENV_FILE"
  wizard_say "bookmark: http://$url_host:$PORT/?token=$token"
  if [ -n "$first_host" ]; then
    wizard_say "          behind tailscale serve or a proxy: https://$first_host/?token=$token"
  fi
  wizard_say "open the bookmark URL once on the phone; it sets a cookie and"
  wizard_say "redirects to /. Bookmark it and you never type the token again."
  if [ "$SERVICE_KIND" = "systemd" ]; then
    wizard_say "logs    : journalctl --user -u ayeaye -f"
  else
    wizard_say "logs    : stderr of $REPO/bin/ayeaye"
  fi
  wizard_say "https for the phone (mic needs a secure origin):"
  wizard_say "  tailscale serve --bg http://$url_host:$PORT"

  wizard_blank
  wizard_say "to change any of this later: run ./install.sh again. It keeps your"
  wizard_say "key and your settings, and asks before it changes either."
  wizard_say "to remove it: systemctl --user disable --now ayeaye, then delete"
  wizard_say "  $UNIT_DIR/ayeaye.service"
  wizard_say "  $ENV_FILE"
  wizard_say "  $WIZARD_STATE_DIR"

  left="$(wizard_unfinished)"
  if [ -n "$left" ]; then
    wizard_blank
    wizard_say "not finished, and worth coming back to:"
    printf '%s\n' "$left" | sed 's/^/  - /'
    wizard_say "running ./install.sh again picks these up."
  fi
  wizard_detail_hint
  return "$WIZARD_STAGE_OK"
}

# ========================================================== the lifecycle

wizard_stage welcome   "ayeaye setup"
wizard_stage detect    "checking this computer"
wizard_stage report    "what ayeaye can do here"
wizard_stage configure "configuration"
wizard_stage plan      "what is about to happen"
wizard_stage install   "setting it up"
wizard_stage service   "service"
wizard_stage finish    "done"

# The welcome, the report, the plan and the summary are what the run looks
# like rather than work that can be already done, so they run every time.
wizard_step welcome   greeting step_welcome        "Welcome"                 required always
wizard_step detect    machine  step_detect_machine "What this computer is"
wizard_step detect    needs    step_detect_needs   "What ayeaye needs"
wizard_step report    capable  step_report         "What ayeaye can do here" required always
wizard_step configure choices  step_choose         "Your choices"
wizard_step plan      confirm  step_plan           "The plan"                required always
wizard_step install   settings step_write_settings "Your settings file"
wizard_step install   key      step_make_key       "Your private key"
wizard_step service   unit     step_service        "Starting ayeaye in the background"
wizard_step finish    summary  step_summary        "What to do next"         required always

# Anything a sibling ticket owns registers itself here, after the core steps
# and in filename order.
wizard_load_steps

if [ "$FRESH" = 1 ]; then
  wizard_state_clear
fi
wizard_begin_run
# A new pass starts a fresh plan and a fresh audit trail. A resumed run does
# not: the stages that filled them in are the stages it is about to skip, and
# emptying them would let stage five say "nothing needs to be installed" and
# then install something.
if [ "$WIZARD_RESUMING" = 0 ]; then
  wizard_consent_reset
  wizard_plan_reset
fi

STATUS=0
wizard_run_stages || STATUS=$?
wizard_end_run "$STATUS"
exit "$STATUS"
