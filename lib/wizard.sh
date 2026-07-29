# The whole setup layer, in one line.
#
#   WIZARD_STATE_DIR="…"          (before sourcing, if not the default)
#   . "$REPO/lib/wizard.sh"
#
# Sourcing is inert: every file below defines functions, gives its tunables
# their defaults and does nothing else. Nothing is read, nothing is written,
# nothing is run, no shell option is changed.
#
# What arrives:
#
#   lib/platform.sh   what this machine is, and who to ask to change it.
#                     Sources lib/pkg.sh and lib/service.sh itself.
#   lib/state.sh      what a run remembers between invocations
#   lib/ui.sh         how the wizard talks, and how it listens
#   lib/consent.sh    permission, and the only wrappers allowed to act
#   lib/envfile.sh    the settings file: render once, merge ever after
#   lib/stage.sh      the eight-stage lifecycle and its steps
#
# The order is the dependency order and is not arbitrary: consent asks through
# ui and records through state, and stage does both.
#
# --------------------------------------------------------------- adding a step
#
# A ticket that adds work to the wizard does not edit install.sh. It drops a
# file into lib/steps/, which install.sh sources in filename order after the
# core steps are registered, and that file registers itself:
#
#   wizard_step detect hardware detect_hardware_step "What this computer has"
#
#   detect_hardware_step() {
#     …
#     return "$WIZARD_STAGE_OK"
#   }
#
# See lib/steps/README.md for the whole contract.
#
# bash 3.2: no associative arrays, no ${var,,}, no mapfile, no local -n.

# Worked out by parameter expansion alone: running dirname here would be a
# command executed at source time, and a `cd` would read CDPATH - both of them
# side effects in the one place this layer promises to have none. It outlives
# the source because wizard_load_steps needs it later.
WIZARD_LIB_DIR="${BASH_SOURCE[0]:-}"
case "$WIZARD_LIB_DIR" in
  */*) WIZARD_LIB_DIR="${WIZARD_LIB_DIR%/*}" ;;
  *)   WIZARD_LIB_DIR="." ;;
esac

# shellcheck source=lib/platform.sh
. "$WIZARD_LIB_DIR/platform.sh"
# shellcheck source=lib/state.sh
. "$WIZARD_LIB_DIR/state.sh"
# shellcheck source=lib/ui.sh
. "$WIZARD_LIB_DIR/ui.sh"
# shellcheck source=lib/consent.sh
. "$WIZARD_LIB_DIR/consent.sh"
# shellcheck source=lib/envfile.sh
. "$WIZARD_LIB_DIR/envfile.sh"
# shellcheck source=lib/stage.sh
. "$WIZARD_LIB_DIR/stage.sh"

# wizard_load_steps [dir] - source every step file a sibling ticket has added.
#
# Filename order, so a numeric prefix decides where a step lands inside its
# stage. Called by install.sh after the core steps are registered; a directory
# that is not there is not a problem.
wizard_load_steps() {
  local dir="${1:-$WIZARD_LIB_DIR/steps}" file
  [ -d "$dir" ] || return 0
  for file in "$dir"/*.sh; do
    [ -f "$file" ] || continue
    # shellcheck disable=SC1090
    . "$file"
  done
  return 0
}
