#!/usr/bin/env bash
# What a real distro image makes of the service definitions this project
# generates. Run inside a container by tests/containers.sh; also the quickest
# way to see what your own machine's systemd thinks of them.
#
#   bash tests/lib/service_probe.sh
#
# It prints key=value lines and changes nothing. The repository is mounted
# read-only, everything written goes under a temporary directory, and no
# service is installed, enabled or started - not because that would be
# awkward, but because a container has no user session bus and any answer it
# gave about starting one would be a lie. What that costs is written down in
# the output rather than left for somebody to assume:
#
#   user_bus=absent          there is no session to install a user service into
#   enable_verified=no       so nothing here proves a unit can be enabled
#
# bash 3.2: no associative arrays, no ${var,,}, no mapfile.
set -u

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
WORK="$(mktemp -d "${TMPDIR:-/tmp}/ayeaye-service-probe.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT INT TERM

# The globals install.sh would have set by the time lib/steps is loaded.
export XDG_CONFIG_HOME="$WORK/config"
export XDG_STATE_HOME="$WORK/state"
# systemd-analyze --user wants somewhere to look for a runtime directory and
# refuses to start its manager without one. A container has no /run/user/<uid>,
# and an empty temporary directory is enough to get past it - which is the
# difference between this image parsing our units and reporting that it could
# not try.
export XDG_RUNTIME_DIR="$WORK/run"
mkdir -p "$XDG_RUNTIME_DIR"
CONF_DIR="$XDG_CONFIG_HOME/ayeaye"
ENV_FILE="$CONF_DIR/env"
UNIT_DIR="$WORK/units"
WIZARD_STATE_DIR="$XDG_STATE_HOME/ayeaye"
NO_SYSTEMD=0
SERVICE_WHISPER_BIN="/usr/bin/whisper-server"
mkdir -p "$CONF_DIR" "$UNIT_DIR"
printf 'AYEAYE_PORT=8911\nVOICE_WHISPER_MODEL=/models/ggml-large-v3.bin\n' > "$ENV_FILE"

# shellcheck source=lib/wizard.sh
. "$REPO/lib/wizard.sh"
for stage in welcome detect report configure plan install service finish; do
  wizard_stage "$stage" "$stage" || true
done
# shellcheck source=lib/steps/70-service.sh
. "$REPO/lib/steps/70-service.sh"

say() { printf '%s=%s\n' "$1" "$2"; }

say service_manager "$(platform_service_manager)"
say blocker "$(platform_service_blocker)"

# The distinction the detection exists to make: three of these images ship
# systemctl and none of them has a session for a user service to live in.
if command -v systemctl >/dev/null 2>&1; then
  say systemctl present
else
  say systemctl absent
fi
if systemctl --user show-environment >/dev/null 2>&1; then
  say user_bus present
else
  say user_bus absent
fi

# ------------------------------------------------------------ what is written

service_render_systemd ayeaye > "$UNIT_DIR/ayeaye.service" 2>/dev/null \
  && say rendered_ayeaye yes || say rendered_ayeaye no
service_render_systemd whisper-server > "$UNIT_DIR/whisper-server.service" 2>/dev/null \
  && say rendered_whisper yes || say rendered_whisper no

leftovers="$(grep -ho '@[A-Za-z_]*@' "$UNIT_DIR"/*.service 2>/dev/null | sort -u | tr '\n' ' ')"
say placeholders "${leftovers:-none}"

if grep -q "^ExecStart=$REPO/bin/ayeaye\$" "$UNIT_DIR/ayeaye.service" 2>/dev/null; then
  say exec_absolute yes
else
  say exec_absolute no
fi
if grep -q "^EnvironmentFile=-$ENV_FILE\$" "$UNIT_DIR/ayeaye.service" 2>/dev/null; then
  say env_referenced yes
else
  say env_referenced no
fi
# No setting may be duplicated into a definition. 8911 is in the settings file
# this probe wrote; if it turns up in a unit, there are two places to change it.
if grep -q '8911\|ggml-large-v3' "$UNIT_DIR"/*.service 2>/dev/null; then
  say settings_leaked yes
else
  say settings_leaked no
fi

# ------------------------------------------------- what this image's systemd says
#
# The real check, and the reason a container is worth the trouble at all:
# `systemd-analyze verify` is this distro's own parser reading what we wrote,
# rather than our idea of what it would accept. It is not in every image, and
# saying so beats reporting a pass nobody earned.
if command -v systemd-analyze >/dev/null 2>&1; then
  out="$(systemd-analyze verify --user "$UNIT_DIR/ayeaye.service" "$UNIT_DIR/whisper-server.service" 2>&1)"
  status=$?
  # An ExecStart naming a binary that is not in the image is a fact about the
  # image, not about the unit, and this probe does not install whisper.
  out="$(printf '%s\n' "$out" | grep -v 'whisper-server is not executable' || true)"
  out="$(printf '%s\n' "$out" | grep -v '^[[:space:]]*$' || true)"
  case "$out" in
    # systemd-analyze itself could not start, which is a fact about the
    # container rather than about anything this project wrote. Reported as its
    # own answer so that it is skipped rather than counted as a unit fault.
    *"Failed to initialize manager"*|*"Failed to lookup RuntimeDirectory"*)
      say analyze unusable
      say analyze_note "$(printf '%s' "$out" | tr '\n' ';' | cut -c1-200)"
      ;;
    *)
      if [ "$status" = 0 ] && [ -z "$out" ]; then
        say analyze ok
      else
        say analyze failed
        say analyze_note "$(printf '%s' "$out" | tr '\n' ';' | cut -c1-200)"
      fi
      ;;
  esac
else
  say analyze unavailable
fi

# ----------------------------------------------------- what was not verified
#
# Stated as data rather than left implicit, so a reader of the output cannot
# come away thinking a container proved something it cannot.
say enable_verified no
say start_verified no
say launchd_verified no
exit 0
