#!/usr/bin/env bash
# What the platform layer says about the machine it is actually running on.
#
#   bash tests/lib/platform_probe.sh
#
# Prints one key=value per line and nothing else. It is what tests/containers.sh
# runs inside a real distro image, and it is also the quickest way to see what
# this layer thinks of your own machine.
#
# Nothing here installs, writes or starts anything. The two package questions
# are queries, and the two command lines are the string-returning forms, which
# by contract execute nothing.
#
# bash 3.2: no associative arrays, no ${var,,}, no mapfile.
set -u

PROBE_DIR="${BASH_SOURCE[0]%/*}"
case "$PROBE_DIR" in
  "${BASH_SOURCE[0]}") PROBE_DIR="." ;;
esac
REPO="$PROBE_DIR/../.."

# shellcheck source=lib/platform.sh
. "$REPO/lib/platform.sh"

platform_detect

# A package every one of these images has, and one that none of them has.
# Neither is in the name table, so this exercises the pass-through as well as
# the query.
PRESENT="${PROBE_PRESENT_PACKAGE:-bash}"
ABSENT="${PROBE_ABSENT_PACKAGE:-tmux}"

yesno() { if "$@"; then printf 'yes'; else printf 'no'; fi; }

printf 'os=%s\n'               "$(platform_os)"
printf 'family=%s\n'           "$(platform_family)"
printf 'id=%s\n'               "$(platform_id)"
printf 'version=%s\n'          "$(platform_version)"
printf 'pretty=%s\n'           "$(platform_pretty)"
printf 'arch=%s\n'             "$(platform_arch)"
printf 'pkg_manager=%s\n'      "$(platform_pkg_manager)"
printf 'service_manager=%s\n'  "$(platform_service_manager)"
printf 'privilege=%s\n'        "$(platform_privilege)"
printf 'uid=%s\n'              "$(platform_uid)"
printf 'immutable=%s\n'        "$(yesno platform_is_immutable)"
printf 'known=%s\n'            "$(yesno platform_is_known)"
printf 'can_act=%s\n'          "$(yesno platform_pkg_can_act)"
printf 'blocker=%s\n'          "$(platform_pkg_blocker)"
printf 'present_pkg=%s\n'      "$PRESENT"
printf 'present_installed=%s\n' "$(yesno platform_pkg_is_installed "$PRESENT")"
printf 'absent_pkg=%s\n'       "$ABSENT"
printf 'absent_installed=%s\n' "$(yesno platform_pkg_is_installed "$ABSENT")"
printf 'query_command=%s\n'    "$(platform_pkg_query_command "$PRESENT" || printf '(none)')"
printf 'refresh_command=%s\n'  "$(platform_pkg_refresh_command || printf '(none)')"
printf 'install_command=%s\n'  "$(platform_pkg_install_command tmux python3 ffmpeg || printf '(none)')"
printf 'summary=%s\n'          "$(platform_summary)"
