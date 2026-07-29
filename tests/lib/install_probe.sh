#!/usr/bin/env bash
# Really installing, inside a real distro image.
#
#   bash tests/lib/install_probe.sh
#
# The sibling of tests/lib/platform_probe.sh, and the opposite of it in one
# respect: that one only asks questions, and this one changes the machine. It
# is therefore only ever run inside a container that is about to be thrown
# away - tests/containers.sh is what does that. Running it on a laptop would
# install packages on the laptop.
#
# What it proves that a stub cannot: that the command this project generates
# for a family is a command that family's package manager accepts, that the
# name table names packages that exist, and that the program really is there
# afterwards. A stub can only prove that the string came out the way somebody
# typed it.
#
# It goes through wizard_install_packages, the same door install.sh uses, so
# the consent layer and the platform adapter are both exercised rather than
# bypassed. WIZARD_AUTO_CONSENT is what --yes sets, and there is nobody in a
# container to ask.
#
# Prints one key=value per line and nothing else:
#
#   family, install_command, refresh_command
#   before_<name>   yes | no   was the program there to start with
#   install_status  the status wizard_install_packages returned
#   after_<name>    yes | no   is it there now
#   db_<name>       yes | no   and does the package database agree
#
# bash 3.2: no associative arrays, no ${var,,}, no mapfile.
set -u

PROBE_DIR="${BASH_SOURCE[0]%/*}"
case "$PROBE_DIR" in
  "${BASH_SOURCE[0]}") PROBE_DIR="." ;;
esac
REPO="$PROBE_DIR/../.."

# The repository is mounted read-only, so everything this writes - the state
# file, the consent ledger, the log - goes somewhere inside the container.
WIZARD_STATE_DIR="${WIZARD_STATE_DIR:-/tmp/ayeaye-install-probe}"
export WIZARD_STATE_DIR

# shellcheck source=lib/wizard.sh
. "$REPO/lib/wizard.sh"

platform_detect

WIZARD_INTERACTIVE=0
WIZARD_DETAILS=0
# What ./install.sh --yes sets. A container has nobody to ask, and refusing
# every question would make this probe prove nothing at all.
WIZARD_AUTO_CONSENT="privileged"

# The hard requirements, plus the two tools setup fetches with. Overridable so
# one image can be asked about a narrower set.
WANT="${INSTALL_PROBE_PACKAGES:-tmux python3 curl tar}"

# The program each logical name is supposed to leave behind. Not always the
# package name - Arch installs "python" and you get python3 - which is exactly
# the sort of thing this probe exists to catch.
probe_program() {
  case "$1" in
    python3) printf 'python3' ;;
    *)       printf '%s' "$1" ;;
  esac
}

yesno() { if "$@" >/dev/null 2>&1; then printf 'yes'; else printf 'no'; fi; }

printf 'family=%s\n'   "$(platform_family)"
printf 'manager=%s\n'  "$(platform_pkg_manager)"
# shellcheck disable=SC2086
printf 'install_command=%s\n' "$(platform_pkg_install_command $WANT || printf '(none)')"
printf 'refresh_command=%s\n' "$(platform_pkg_refresh_command || printf '(none)')"

for logical in $WANT; do
  printf 'before_%s=%s\n' "$logical" \
    "$(yesno command -v "$(probe_program "$logical")")"
done

# The one line in this file that changes the machine, and it goes through the
# wrapper rather than around it.
# shellcheck disable=SC2086
if out="$(wizard_install_packages $WANT 2>&1)"; then
  status=0
else
  status=$?
fi
printf 'install_status=%s\n' "$status"

for logical in $WANT; do
  printf 'after_%s=%s\n' "$logical" \
    "$(yesno command -v "$(probe_program "$logical")")"
  printf 'db_%s=%s\n' "$logical" "$(yesno platform_pkg_is_installed "$logical")"
done

# Last, and only when something went wrong, so the useful lines are never
# buried under a page of package-manager chatter.
if [ "$status" != 0 ]; then
  printf 'output<<\n%s\n>>output\n' "$out"
fi
