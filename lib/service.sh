# The service adapter: systemd's user session, launchd, or neither.
#
# Sourced by lib/platform.sh; never on its own. Inert on load, like the rest
# of the layer.
#
#   platform_service_can_act              status 0 when there is a service
#                                         manager to talk to
#   platform_service_blocker              why not, as one word, or empty:
#                                         no-service-manager
#   platform_service_unit <name>          this platform's identifier for a
#                                         logical service: ayeaye.service
#                                         under systemd, dev.ayeaye under
#                                         launchd. Empty and status 1 when
#                                         there is no service manager.
#   platform_service_command <name> <op> [unit-path]
#                                         the command for op, as a string,
#                                         executing nothing
#   platform_service_run <name> <op> [unit-path]
#                                         the same thing, actually run
#
# The operations are reload, enable, start, stop and status. The three exit
# statuses are load-bearing and different on purpose:
#
#   0   here is the command
#   1   this platform cannot do that - no service manager at all, an
#       operation that does not exist here (launchd has no daemon-reload), or
#       a required argument that was not given (launchd's enable needs the
#       path of the plist, and guessing one would be worse than refusing)
#   2   that is not an operation this interface has, which is a bug in the
#       caller rather than a fact about the machine
#
# Everything generated here is user-scoped and none of it takes sudo. This
# project installs a per-user service; a root-owned one would keep running
# after the person who wanted it logged out, which is not what was asked for.
#
# bash 3.2: no associative arrays, no ${var,,}, no mapfile.

# The reverse-DNS prefix for a launchd label. It matches the plist name this
# project already uses, which the test suite's guarded-path list knows as
# ~/Library/LaunchAgents/dev.ayeaye.plist.
_PLATFORM_LAUNCHD_PREFIX="dev"

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
# A name that already carries the platform's own shape is left alone, so a
# caller may pass either the logical name or the real one.
platform_service_unit() {
  platform_detect
  case "$_PLATFORM_SERVICE_MANAGER" in
    systemd)
      case "$1" in
        *.service|*.socket|*.timer|*.target) printf '%s' "$1" ;;
        *) printf '%s.service' "$1" ;;
      esac
      ;;
    launchd)
      case "$1" in
        *.*) printf '%s' "$1" ;;
        *) printf '%s.%s' "$_PLATFORM_LAUNCHD_PREFIX" "$1" ;;
      esac
      ;;
    *)
      return 1 ;;
  esac
}

# platform_service_command <name> <op> [unit-path]
platform_service_command() {
  platform_detect
  local name="$1" op="$2" unit_path="${3:-}" unit

  case "$op" in
    reload|enable|start|stop|status) ;;
    *) return 2 ;;
  esac

  [ "$_PLATFORM_SERVICE_MANAGER" = "none" ] && return 1
  unit="$(platform_service_unit "$name")" || return 1

  case "$_PLATFORM_SERVICE_MANAGER" in
    systemd)
      case "$op" in
        reload) printf 'systemctl --user daemon-reload' ;;
        enable) printf 'systemctl --user enable --now %s' "$unit" ;;
        start)  printf 'systemctl --user start %s' "$unit" ;;
        stop)   printf 'systemctl --user stop %s' "$unit" ;;
        status) printf 'systemctl --user status %s' "$unit" ;;
      esac
      ;;
    launchd)
      case "$op" in
        # launchd has no daemon-reload: a changed plist is re-read by being
        # bootstrapped again, which is the enable step. Saying so beats
        # inventing a command that does nothing.
        reload) return 1 ;;
        enable)
          [ -n "$unit_path" ] || return 1
          printf 'launchctl bootstrap gui/%s %s' "$_PLATFORM_UID" "$unit_path" ;;
        start)  printf 'launchctl kickstart gui/%s/%s' "$_PLATFORM_UID" "$unit" ;;
        stop)   printf 'launchctl bootout gui/%s/%s' "$_PLATFORM_UID" "$unit" ;;
        status) printf 'launchctl print gui/%s/%s' "$_PLATFORM_UID" "$unit" ;;
      esac
      ;;
    *)
      return 1 ;;
  esac
}

# platform_service_run <name> <op> [unit-path] - the executing form.
platform_service_run() {
  local cmd status
  cmd="$(platform_service_command "$@")"
  status=$?
  [ "$status" = 0 ] || return "$status"
  [ -n "$cmd" ] || return 1
  eval "$cmd"
}
