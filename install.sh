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
  _release_caveat
}

# _release_caveat - said wherever the one-liner is quoted, for as long as it is
# not true.
#
# The command above fetches a pinned release, and no release has been
# published: there is no $AYEAYE_VERSION tag and nothing under
# releases/download/$AYEAYE_VERSION/, so the fetch would 404. Cutting a release
# is a decision for whoever owns the repository and not something setup can do
# on their behalf, so the honest thing available here is to say so out loud
# rather than to print a command that does not work and let somebody find out.
#
# When the release exists, delete this function and its two callers, and fill
# in AYEAYE_SHA256 at the top of this file.
_release_caveat() {
  cat <<CAVEAT

Not yet, though: ayeaye $AYEAYE_VERSION has not been published. Until somebody
tags it and uploads ayeaye-$AYEAYE_VERSION.tar.gz and SHA256SUMS to that
release, the command above has nothing to fetch and will fail. Clone the
repository and run ./install.sh from it instead:

  git clone https://github.com/LioraLabs/ayeaye
  cd ayeaye && ./install.sh
CAVEAT
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
      # And the likeliest reason of all, while that is still true.
      _release_caveat >&2
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

# _needs - the two programs ayeaye cannot run without, named once.
_needs() { printf 'tmux python3'; }

# _needs_missing - which of them this computer does not have, right now, with a
# leading space when there are any. Asked of the machine every time rather than
# read back from the state file: a run picked up tomorrow may well have had one
# of them installed by hand in between, and re-installing something that is
# already there is the second-worst answer available.
_needs_missing() {
  local dep missing=""
  for dep in $(_needs); do
    command -v "$dep" >/dev/null 2>&1 || missing="$missing $dep"
  done
  printf '%s' "$missing"
}

# Stage two looks and says what it found. It does not install: everything setup
# would change is summarized in stage five and confirmed there, and a stage
# that installed two packages three stages earlier would make that promise a
# lie. What it does instead is put them in the plan, so the summary the person
# confirms is the whole of what is about to happen.
step_detect_needs() {
  local dep missing
  for dep in $(_needs); do
    if command -v "$dep" >/dev/null 2>&1; then
      wizard_item ok "$dep"
    else
      wizard_item MISSING "$dep (required)"
    fi
  done
  missing="$(_needs_missing)"
  # Written down as well as planned, so that the log of a run that went wrong
  # says what this stage actually saw rather than only what it decided.
  wizard_remember step.detect.needs.missing "${missing# }"
  [ -n "$missing" ] || return "$WIZARD_STAGE_OK"

  wizard_blank
  wizard_say "ayeaye cannot run without these:$missing"
  wizard_say "tmux is what the coding agents run inside, and python3 is what"
  wizard_say "ayeaye itself is written in."
  wizard_say "Nothing is being installed yet. Setup lists everything it would"
  wizard_say "change in one place further down, and asks before any of it."
  wizard_plan_add package "${missing# }, which ayeaye cannot run without"
  # OK, not FAIL. A FAIL here is what used to stop the run before it reached
  # the plan, which is exactly the thing this arrangement exists to prevent:
  # the missing programs are work to be done, and stage six is where work is
  # done. step_install_needs is what cannot be got past.
  return "$WIZARD_STAGE_OK"
}

# ====================================================== 3. what it can do

# Whether this computer can turn speech into words - and what that is worth -
# is one subject and it belongs to one file. lib/steps/20-hardware.sh measures
# it, names the tier, and says in the report stage what talking out loud would
# be like here; this step says the one thing that file does not, which is that
# there has to be an https address in front of ayeaye before a phone can use a
# microphone at all.
#
# There used to be a voice sweep here as well, and a second framing of the same
# answer, and a third in the closing summary. Three descriptions of one
# capability in one run is not thoroughness, it is a screen nobody reads.
step_report() {
  if ! command -v tailscale >/dev/null 2>&1; then
    wizard_item note "tailscale not found: you will need another way to serve https to the phone"
  fi
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
  # other two. On a first run there is nothing configured and the offer is
  # the built-in default.
  wizard_blank
  wizard_say "Three questions about how to reach ayeaye. Press return to keep"
  wizard_say "what is in the square brackets."
  wizard_blank

  # The address ayeaye answers on is not one of them, and deliberately so.
  # lib/steps/50-access.sh owns how ayeaye is reached: it keeps the program
  # itself on this computer in every one of the four ways in, and puts the
  # thing that is on the network in front of it. Asking here as well was
  # asking the same question twice and getting two answers - and one of the
  # answers this question accepted, 0.0.0.0, is the single setting this
  # project will not write for anybody. What is already in a settings file is
  # still honoured, and still said out loud; see _access_warn_open_bind.
  BIND="$(_current AYEAYE_BIND answer.bind 127.0.0.1)"

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

# The two programs ayeaye cannot run without, installed here rather than in
# stage two - after the plan has been shown and agreed to, and before anything
# else in this stage needs them. step_write_settings runs python3, so the order
# inside the stage is not incidental.
#
# It stays in install.sh rather than moving to lib/steps because a machine with
# no tmux and no python3 cannot run ayeaye at all: that is the lifecycle's
# business, not a capability a step adds. lib/steps/60-packages.sh checks the
# same two and stops when they are absent, which is what keeps a run that lost
# one of them between stages from getting any further.
#
# Nothing here says "installed." for a program that is not there. A zero exit
# from wizard_install_packages means the command it built ran, which is not the
# same fact and never was.
step_install_needs() {
  local dep missing still="" status
  missing="$(_needs_missing)"
  if [ -z "$missing" ]; then
    return "$WIZARD_STAGE_SKIP"
  fi

  wizard_say "ayeaye needs$missing before anything else here can be done."
  # The one door to installing anything, here and in every later ticket.
  # shellcheck disable=SC2086
  wizard_install_packages $missing
  status=$?
  case "$status" in
    0) ;;
    "$WIZARD_REFUSED")
      # Said no to the one thing there is no working setup without. There is
      # nothing to try again and nothing to leave out, so setup stops here
      # rather than asking a second time or half-finishing.
      wizard_blank
      wizard_say "nothing was installed, and this computer is exactly as it was."
      wizard_blank
      wizard_say "Setup stops here.$missing is what ayeaye runs the coding"
      wizard_say "agents inside, so there is no version of this that works"
      wizard_say "without it - which is why setup is not going to ask again"
      wizard_say "or set up half of it."
      wizard_blank
      wizard_say "If you would rather install it yourself:"
      # shellcheck disable=SC2086
      wizard_detail "$(platform_pkg_install_command $missing 2>/dev/null || true)"
      # shellcheck disable=SC2086
      platform_pkg_manual_hint $missing
      wizard_blank
      wizard_say "Then run ./install.sh again. Nothing you answered is lost."
      return "$WIZARD_STAGE_CANCEL"
      ;;
    *)
      wizard_say "that software could not be installed automatically."
      ;;
  esac

  # bash remembers where it found a command, and it also remembers having
  # failed to find one in some shells. Ask the machine again from a clean slate.
  hash -r 2>/dev/null || true
  for dep in $missing; do
    if command -v "$dep" >/dev/null 2>&1; then
      wizard_item ok "$dep"
    else
      wizard_item MISSING "$dep"
      still="$still $dep"
    fi
  done
  if [ -n "$still" ]; then
    wizard_blank
    wizard_say "ayeaye still cannot run here:$still is not on this computer."
    wizard_say "Install it however you normally would, then run ./install.sh"
    wizard_say "again - it carries on from here and asks nothing twice."
    return "$WIZARD_STAGE_FAIL"
  fi
  wizard_say "installed."
  return "$WIZARD_STAGE_OK"
}

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
#
# What is left at the end of a run is a picture of this computer, and the whole
# of this stage's job is that the picture is true. Three things follow:
#
#   Every instruction is built from what actually happened, not from what
#   usually happens. The way to remove ayeaye on a Mac names launchctl and a
#   property list; on Linux it names systemctl and a unit; on a machine with
#   neither it says to stop the program you started by hand. A run that prints
#   systemctl to somebody with no systemd has told them something false about
#   their own computer, which is worse than telling them nothing.
#
#   The address a phone should open is the one the way-in that was set up
#   actually answers on - not the address ayeaye itself listens on, which in
#   every mode is this computer and is not reachable from a phone at all.
#
#   Work that did not finish is listed with how to pick it up, item by item.
#   "Run setup again" is true of most of it and useless for the rest.

# _summary_service_kind - systemd, launchd, or none. Asked of the platform
# layer rather than read from SERVICE_KIND: the service stage is where that
# variable is set and a resumed run skips it, so by the time this runs it may
# well hold nothing but its default.
_summary_service_kind() {
  [ "$NO_SYSTEMD" = 0 ] || { printf 'none'; return 0; }
  platform_service_manager
}

# _summary_voice - the listening tier, in the words the hardware step measured
# it in, plus notifications when they are configured.
#
# What was **set up**, not what this computer could have managed. There are two
# ways to get this wrong and the milestone managed both of them. The first was
# a probe of its own that printed "text-only" whatever had happened, so a
# machine that had just downloaded a listening model was told it could not
# listen. The second, which is worse, is reading the hardware tier: that is a
# measurement of the machine and has nothing to do with whether anybody chose
# to set voice up, so a run that installed nothing and asked for nothing would
# announce "talking and typing" on a computer where the talk button is grey.
#
# So the sources are, in order: what the health check found, which is the only
# one that watched the app answer; then whether a listening model was chosen at
# all; and the hardware tier last and only to explain why it was never offered.
_summary_voice() {
  local health model tier=""
  health="$(wizard_state_get step.service.health.voice "")"
  model="$(wizard_state_get answer.voice.model "")"
  command -v hw_voice_tier >/dev/null 2>&1 && tier="$(hw_voice_tier)"

  if [ "$health" = pass ]; then
    printf 'typing, and talking out loud'
  elif [ -n "$model" ]; then
    # Set up, and not seen working. The health check either could not reach the
    # app or found the talk button still grey, and both of those are on the
    # unfinished list with their own line further down.
    printf 'typing; talking out loud is set up but was not confirmed'
  elif [ "$tier" = text-only ]; then
    printf 'typing (this computer has no room to listen)'
  else
    printf 'typing (talking out loud was not set up; ./install.sh again offers it)'
  fi
  [ -n "$NTFY" ] && printf ', with notifications to your phone'
  return 0
}

# _summary_phone_url <token> - the address to open on a phone.
#
#   0  here it is
#   1  there is no way in to a phone, which is what "this computer only" means
#   2  a way in was chosen and is not finished, so the address is not one to
#      hand somebody yet
#
# The list of addresses ayeaye accepts is **not** enough to answer this on its
# own, and reading it as though it were is how this screen came to invent a
# phone address on a run that had set nothing up: install.sh's own allowed-hosts
# question offers a tailscale name it noticed as its default, and pressing
# return puts that name in the list without a front end existing anywhere. The
# fact that means what this needs is what the access step actually installed,
# and whether that step finished.
#
# For the home-network mode the allow-list entry carries the port as well, which
# is why the port is never added here.
_summary_phone_url() {
  local token="${1:-}" host mode
  host="${HOSTS%%,*}"
  mode="$(wizard_state_get step.install.access.installed "")"
  case "$mode" in
    tailscale|lan|proxy)
      [ -n "$host" ] || return 1
      printf 'https://%s/?token=%s' "$host" "$token"
      # The address is right either way - it is the one the front end will
      # answer on - but a front end that is not finished is not one to send
      # somebody to from the sofa, so the caller says which of the two it has.
      [ "$(wizard_state_get step.install.access.ready 0)" = 1 ] || return 2
      return 0
      ;;
  esac
  # Nothing was set up, and there is still a name in the allow list. It got
  # there one of two ways and this screen cannot tell them apart: somebody
  # typed it, or install.sh offered a tailscale name it had noticed as the
  # default and they pressed return. So the address is printed - losing it
  # would be unhelpful to the first of those - and it is not led with, and it
  # is not called theirs to open, because for the second it is a front end that
  # does not exist.
  [ -n "$host" ] || return 1
  printf 'https://%s/?token=%s' "$host" "$token"
  return 3
}

# _summary_health_trouble - the capabilities the health step checked and could
# not report as working, in the words a person would use for them, with a
# leading space. Empty when there are none.
#
# Read out of the state file by key, and not by knowing which file wrote them:
# install.sh owns the lifecycle and never names a file in lib/steps, which is
# what makes the seam a seam. It is read rather than remembered for the same
# reason every other cross-stage fact is - the step that wrote it is one a
# resumed run skips.
_summary_health_trouble() {
  local check verdict out=""
  for check in service local auth agents hosts https voice board; do
    verdict="$(wizard_state_get "step.service.health.$check" "")"
    case "$verdict" in
      fail|unknown) ;;
      *) continue ;;
    esac
    case "$check" in
      service) out="$out ayeaye starting when you log in" ;;
      local)   out="$out ayeaye answering on this computer" ;;
      auth)    out="$out ayeaye refusing anyone without your key" ;;
      agents)  out="$out the coding agents you chose" ;;
      hosts)   out="$out the https address you named" ;;
      https)   out="$out the https address your phone opens" ;;
      voice)   out="$out talking to your agents out loud" ;;
      board)   out="$out your ticket board" ;;
    esac
    out="$out;"
  done
  # Trim the semicolon the last one left.
  printf '%s' "${out%;}"
  return 0
}

# _summary_resume <stage> <step> - how to pick that piece up again, in one
# line. A step this does not know gets the general answer, which is true of
# almost all of them; the ones named here are the ones for which it is not
# enough.
_summary_resume() {
  local trouble
  case "${1:-}.${2:-}" in
    install.voice)
      printf 'run ./install.sh again and choose a listening option; what was already downloaded is kept' ;;
    install.access)
      # Deliberately vague, and it is the one place in this table where vague is
      # the honest answer. There are six ways this step can end unfinished -
      # tailscale absent, tailscale not signed in, serve refused, no address on
      # the network yet, a proxy waiting for its configuration, nobody watching
      # - and it says which, in words, at the moment it happens. Naming one of
      # the six here would contradict that paragraph five times out of six.
      printf 'the way in step said above what it is waiting for; do that, then run ./install.sh again' ;;
    install.agents|install.board)
      printf 'run ./install.sh again, or install it yourself and setup will find it' ;;
    install.marker)
      printf 'run ./install.sh again and say yes to the status line question' ;;
    service.unit)
      printf 'run ./install.sh again; until then start it by hand with %s/bin/ayeaye' "$REPO" ;;
    service.health)
      # Named, not left as "the check did not pass". The check knows which
      # capability it was, it wrote that down, and "your ticket board on the
      # phone did not answer" is a thing somebody can act on in a way that
      # "the health check reported a problem" is not.
      trouble="$(_summary_health_trouble)"
      if [ -n "$trouble" ]; then
        printf 'still to sort out:%s. Fix what you can and run ./install.sh again to re-check' "$trouble"
      else
        printf 'start ayeaye, open the address above, and run ./install.sh again to re-check'
      fi
      ;;
    *)
      printf 'run ./install.sh again - it carries on from here and asks nothing twice' ;;
  esac
  return 0
}

# _summary_removal <kind> - how to take it off this computer, correct for the
# service manager that was actually used.
_summary_removal() {
  local kind="${1:-none}" cmd path mode files
  files="$ENV_FILE
$WIZARD_STATE_DIR"
  mode="$(wizard_state_get answer.access.mode "")"

  wizard_say "to remove it:"
  case "$kind" in
    systemd|launchd)
      if cmd="$(platform_service_command ayeaye disable)"; then
        wizard_say "  $cmd"
      fi
      if command -v service_definition_path >/dev/null 2>&1 \
         && path="$(service_definition_path ayeaye "$kind")"; then
        files="$path
$files"
      fi
      # The https front is a second service, and a run that set one up and did
      # not mention it here would leave something answering on the network
      # after somebody believed they had removed ayeaye.
      if [ "$mode" = lan ]; then
        if cmd="$(platform_service_command ayeaye-caddy disable)"; then
          wizard_say "  $cmd"
        fi
        if command -v service_definition_path >/dev/null 2>&1 \
           && path="$(service_definition_path ayeaye-caddy "$kind")"; then
          files="$path
$files"
        fi
      fi
      ;;
    *)
      wizard_say "  stop the ayeaye you started by hand - nothing on this"
      wizard_say "  computer starts it for you"
      ;;
  esac

  # Whatever the way in left behind. The certificate authority's directory is
  # the one that has to be named out loud rather than left inside "and the rest
  # of the settings": it holds the private key this computer signs with, and a
  # removal that left it there would leave a machine able to mint certificates
  # for a network it is no longer on.
  case "$mode" in
    lan)
      if command -v _access_caddyfile >/dev/null 2>&1; then
        files="$files
$(_access_caddyfile)
$(_access_ca_dir)
$(_access_caddy_data)"
      fi
      ;;
    proxy)
      if command -v _access_proxy_caddy_file >/dev/null 2>&1; then
        files="$files
$(_access_proxy_caddy_file)
$(_access_proxy_nginx_file)"
      fi
      ;;
  esac

  wizard_say "  then delete"
  printf '%s\n' "$files" | sed 's/^/    /'
  if [ "$mode" = lan ]; then
    wizard_say "  The last of those holds the key this computer signs certificates"
    wizard_say "  with, so it is the one that matters most."
    wizard_say "  and remove the certificate from every phone you put it on."
    wizard_say "  Only you can do that; nothing here can reach your phone."
  fi
  return 0
}

step_summary() {
  local token url rows line stage step status label
  _load_effective_values
  SERVICE_KIND="$(_summary_service_kind)"
  token="$(cat "$TOKEN_FILE" 2>/dev/null || true)"

  wizard_say "what works: $(_summary_voice)"
  wizard_say "config  : $ENV_FILE"

  # ------------------------------------------------------ opening it on a phone
  url="$(_summary_phone_url "$token")"
  case "$?" in
    0)
      wizard_say "bookmark: $url"
      wizard_say "          open that one on your phone."
      wizard_say "          in a browser on this computer: http://$BIND:$PORT/?token=$token"
      ;;
    2)
      wizard_say "bookmark: $url"
      wizard_say "          that is the address, once the way in you chose is"
      wizard_say "          finished. It is not finished yet, and what it is waiting"
      wizard_say "          for is below."
      wizard_say "          in a browser on this computer: http://$BIND:$PORT/?token=$token"
      ;;
    3)
      wizard_say "bookmark: http://$BIND:$PORT/?token=$token"
      wizard_say "          a browser on this computer. Your settings name"
      wizard_say "          ${HOSTS%%,*} as an https address for this machine, and"
      wizard_say "          setup did not put it there and cannot see it from here."
      wizard_say "          If you have it answering, what to open on your phone is"
      wizard_say "          $url"
      ;;
    *)
      wizard_say "bookmark: http://$BIND:$PORT/?token=$token"
      wizard_say "          a browser on this computer, and nothing else - your phone"
      wizard_say "          cannot reach ayeaye yet. Run ./install.sh again and pick a"
      wizard_say "          way in when you want it to."
      ;;
  esac
  wizard_blank
  wizard_say "signing in, once per device:"
  wizard_say "  open that address once. It puts a key in the browser and sends you"
  wizard_say "  to the page; bookmark what you land on and you never type the key"
  wizard_say "  again."
  wizard_say "  Anything that is not a browser sends the key as a header called"
  wizard_say "  X-Voice-Token instead; the key itself is in $TOKEN_FILE."
  wizard_blank

  case "$SERVICE_KIND" in
    systemd) wizard_say "logs    : journalctl --user -u ayeaye -f" ;;
    launchd) wizard_say "logs    : tail -f $HOME/Library/Logs/ayeaye/ayeaye.log" ;;
    *)       wizard_say "logs    : whatever the terminal you start ayeaye in shows" ;;
  esac

  wizard_blank
  wizard_say "to change any of this later: run ./install.sh again. It keeps your"
  wizard_say "key and your settings, asks before it changes either, and is how"
  wizard_say "you switch between the four ways of reaching ayeaye."
  _summary_removal "$SERVICE_KIND"

  # ------------------------------------------- what did not get done, and how to
  rows="$(wizard_unfinished_rows)"
  if [ -n "$rows" ]; then
    wizard_blank
    wizard_say "not finished, and worth coming back to:"
    while IFS= read -r line || [ -n "$line" ]; do
      [ -n "$line" ] || continue
      stage="${line%%"$_WIZARD_STAGE_TAB"*}"; line="${line#*"$_WIZARD_STAGE_TAB"}"
      step="${line%%"$_WIZARD_STAGE_TAB"*}";  line="${line#*"$_WIZARD_STAGE_TAB"}"
      status="${line%%"$_WIZARD_STAGE_TAB"*}"; label="${line#*"$_WIZARD_STAGE_TAB"}"
      if [ "$status" = failed ]; then
        wizard_say "  - $label (did not work)"
      else
        wizard_say "  - $label (not finished)"
      fi
      wizard_say "    $(_summary_resume "$stage" "$step")"
    done <<EOF
$rows
EOF
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
wizard_step install   needs    step_install_needs  "What ayeaye cannot run without"
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
