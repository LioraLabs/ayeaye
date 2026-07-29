# The service adapter: systemd's user session, launchd, or neither.
#
# Sourced by lib/platform.sh; never on its own. Inert on load, like the rest
# of the layer.
#
#   platform_service_can_act              0 when there is a service manager to
#                                         talk to
#   platform_service_blocker              why not, as one word, or empty.
#                                         Always 0. Today the only word is
#                                         no-service-manager.
#   platform_service_unit <name>          this platform's identifier for a
#                                         logical service: ayeaye.service
#                                         under systemd, dev.ayeaye under
#                                         launchd
#   platform_service_command <name> <op> [unit-path]
#                                         the command for op, as a string,
#                                         executing nothing
#   platform_service_run <name> <op> [unit-path]
#                                         the same thing, actually run
#
# ---------------------------------------------------------------- operations
#
#   reload    re-read unit files. systemd only; launchd re-reads a plist by
#             being bootstrapped again, which is enable.
#   enable    install and start, and keep it that way across logins. Under
#             launchd this is bootstrap, which needs the path of the plist as
#             the third argument.
#   disable   the inverse of enable: stop it and do not bring it back.
#   start     start it now, leaving enablement alone.
#   stop      stop it now, leaving enablement alone.
#   status    what is it doing.
#
# start and stop are deliberately the "leave enablement alone" pair on both
# platforms, so that stop-then-start behaves the same either side. Under
# launchd that makes stop a signal rather than a bootout - bootout is the
# unregistering one, and it is what disable does.
#
# ------------------------------------------------------------------ statuses
#
#   0   here is the command
#   1   this machine cannot do that - no service manager at all, an operation
#       that does not exist here (launchd has no reload), or a required
#       argument that was not given (launchd's enable needs the path of the
#       plist, and guessing one would be worse than refusing)
#   2   the call was wrong - an operation this interface does not have, or a
#       missing service name. A bug in the caller rather than a fact about the
#       machine, and reporting it to a user as "your platform is unsupported"
#       would send them looking in the wrong place.
#
# **A consumer running under `set -e` must not assign any `platform_service_…`
# result bare** - `unit="$(platform_service_unit ayeaye)"` and
# `cmd="$(platform_service_command …)"` are both fatal there when they return
# non-zero, which they do on any machine with no service manager. Put them in
# an `if`, or add `|| true` and test for the empty string.
# platform_service_blocker is the one function here that always succeeds, and
# is safe to assign bare.
#
# Everything generated here is user-scoped and none of it takes sudo. This
# project installs a per-user service; a root-owned one would keep running
# after the person who wanted it logged out, which is not what was asked for.
#
# bash 3.2: no associative arrays, no ${var,,}, no mapfile.

# ----------------------------------------------------------------- tunables
#
#   PLATFORM_LAUNCHD_PREFIX  the reverse-DNS prefix for a launchd label,
#                            "dev" by default. It matches the plist name this
#                            project already uses, which the test suite's
#                            guarded-path list knows as
#                            ~/Library/LaunchAgents/dev.ayeaye.plist. Changing
#                            it changes every label this adapter generates.
PLATFORM_LAUNCHD_PREFIX="${PLATFORM_LAUNCHD_PREFIX:-dev}"

platform_service_blocker() {
  platform_detect
  [ "$_PLATFORM_SERVICE_MANAGER" = "none" ] && printf 'no-service-manager'
  return 0
}

platform_service_can_act() {
  [ -z "$(platform_service_blocker)" ]
}

# platform_service_unit <name> - what this platform calls that service.
#
# A name is reduced to the bare logical one first and then dressed in the
# local convention, so that one name works everywhere: ayeaye, ayeaye.service
# and dev.ayeaye all come out as ayeaye.service under systemd and dev.ayeaye
# under launchd. Shared code holds one name; it must not matter which one.
platform_service_unit() {
  platform_detect
  local name="${1:-}"
  [ -n "$name" ] || return 2

  # Strip whichever platform's clothes it arrived in.
  case "$name" in
    "$PLATFORM_LAUNCHD_PREFIX".*) name="${name#"$PLATFORM_LAUNCHD_PREFIX".}" ;;
  esac
  case "$name" in
    *.service) name="${name%.service}" ;;
  esac

  case "$_PLATFORM_SERVICE_MANAGER" in
    systemd)
      case "$name" in
        *.socket|*.timer|*.target) printf '%s' "$name" ;;
        *) printf '%s.service' "$name" ;;
      esac
      ;;
    launchd)
      printf '%s.%s' "$PLATFORM_LAUNCHD_PREFIX" "$name"
      ;;
    *)
      return 1 ;;
  esac
}

# platform_service_command <name> <op> [unit-path]
platform_service_command() {
  platform_detect
  local name="${1:-}" op="${2:-}" unit_path="${3:-}" unit

  case "$op" in
    reload|enable|disable|start|stop|status) ;;
    *) return 2 ;;
  esac
  [ -n "$name" ] || return 2

  [ "$_PLATFORM_SERVICE_MANAGER" = "none" ] && return 1
  unit="$(platform_service_unit "$name")" || return 1
  _platform_quote "$unit"
  unit="$_PLATFORM_S"

  case "$_PLATFORM_SERVICE_MANAGER" in
    systemd)
      case "$op" in
        reload)  printf 'systemctl --user daemon-reload' ;;
        enable)  printf 'systemctl --user enable --now %s' "$unit" ;;
        disable) printf 'systemctl --user disable --now %s' "$unit" ;;
        start)   printf 'systemctl --user start %s' "$unit" ;;
        stop)    printf 'systemctl --user stop %s' "$unit" ;;
        status)  printf 'systemctl --user status %s' "$unit" ;;
      esac
      ;;
    launchd)
      # Every launchd command addresses a domain by uid. Without one the
      # target would be the malformed gui//label, which is worse than saying
      # this cannot be done.
      [ -n "$_PLATFORM_UID" ] || return 1
      case "$op" in
        # launchd has no daemon-reload: a changed plist is re-read by being
        # bootstrapped again, which is the enable step. Saying so beats
        # inventing a command that does nothing.
        reload) return 1 ;;
        enable)
          [ -n "$unit_path" ] || return 1
          _platform_quote "$unit_path"
          printf 'launchctl bootstrap gui/%s %s' "$_PLATFORM_UID" "$_PLATFORM_S" ;;
        disable) printf 'launchctl bootout gui/%s/%s' "$_PLATFORM_UID" "$unit" ;;
        start)   printf 'launchctl kickstart gui/%s/%s' "$_PLATFORM_UID" "$unit" ;;
        # A signal, not a bootout: stop must leave the service registered, the
        # way systemctl --user stop does, or stop-then-start works on Linux
        # and fails on a Mac with "Could not find service".
        stop)    printf 'launchctl kill SIGTERM gui/%s/%s' "$_PLATFORM_UID" "$unit" ;;
        status)  printf 'launchctl print gui/%s/%s' "$_PLATFORM_UID" "$unit" ;;
      esac
      ;;
    *)
      return 1 ;;
  esac
}

# platform_service_run <name> <op> [unit-path] - the executing form. It
# forwards the command's own status, so a caller sees the same 1 and 2 it
# would have seen from the string form.
platform_service_run() {
  local cmd status
  cmd="$(platform_service_command "$@")"
  status=$?
  [ "$status" = 0 ] || return "$status"
  [ -n "$cmd" ] || return 1
  eval "$cmd"
}
