#!/usr/bin/env bash
# Does the platform layer say the right thing about a real Debian, Fedora,
# Arch and openSUSE, and does the command it generates for each of them really
# install what it says? Fixtures can only prove that a file was parsed the way
# somebody typed it, and a stub can only prove that a string came out the way
# somebody typed it. This proves both against a machine.
#
#   tests/containers.sh                 every image: what it says, and what it does
#   tests/containers.sh arch fedora     images whose name contains one of these
#   tests/containers.sh --list          what would be run
#   tests/containers.sh --quick         only the questions; install nothing
#   tests/containers.sh --suite         also run the unit tests inside each image
#   tests/containers.sh --engine podman
#   tests/containers.sh -v              print every probed value, not just the
#                                       ones being asserted
#
# It is deliberately not part of tests/run.sh: the fast suite stays runnable
# with nothing but bash, coreutils and python3, and this needs a container
# engine and a few hundred megabytes of images. With no engine it says so and
# exits 0, because "could not check" is not the same as "found a problem".
#
# Two probes, and they differ in exactly one respect.
#
#   tests/lib/platform_probe.sh only asks questions. The repository is mounted
#   read-only and nothing on the machine changes; the commands the layer would
#   run are asserted as strings and never executed.
#
#   tests/lib/install_probe.sh really installs, through the same
#   wizard_install_packages install.sh goes through - so the consent layer, the
#   name table and the generated command are all exercised rather than
#   imitated. It runs inside a container that is thrown away when it exits, and
#   the repository is still mounted read-only. --quick leaves it out.
#
# bash 3.2: no associative arrays, no ${var,,}, no mapfile.
set -u

TESTS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$TESTS_DIR/.." && pwd)"

ENGINE="${CONTAINER_ENGINE:-}"
VERBOSE=0
LIST_ONLY=0
RUN_SUITE=0
RUN_INSTALL=1
FILTERS=""

usage() { sed -n '2,34p' "$0" | sed 's/^# \{0,1\}//'; }

while [ "$#" -gt 0 ]; do
  case "$1" in
    -h|--help)    usage; exit 0 ;;
    -v|--verbose) VERBOSE=1 ;;
    -l|--list)    LIST_ONLY=1 ;;
    --suite)      RUN_SUITE=1 ;;
    --quick)      RUN_INSTALL=0 ;;
    --engine)
      [ "$#" -gt 1 ] || { echo "--engine needs the name of a container engine" >&2; exit 2; }
      ENGINE="$2"; shift ;;
    -*)           echo "unknown option: $1 (try --help)" >&2; exit 2 ;;
    *)            FILTERS="$FILTERS $1" ;;
  esac
  shift
done

# A filter that matches nothing is an error, not an empty pass - the same rule
# tests/run.sh keeps, and for the same reason: a typo must not read as green.
if [ -n "$FILTERS" ]; then
  matched=0
  for filter in $FILTERS; do
    case "
debian:12
fedora:latest
archlinux:latest
opensuse/tumbleweed:latest" in
      *"$filter"*) matched=$((matched + 1)) ;;
      *) echo "no image matched the filter: $filter" >&2; exit 2 ;;
    esac
  done
fi

# ------------------------------------------------------------------- the plan
#
# One line per image: image, family, package manager, distro id. The id is
# asserted too, because a family that is right for the wrong reason - matched
# through ID_LIKE when ID should have done it - is a bug waiting for the next
# release of that distro.
#
# service_manager is expected to be none in all four. Three of these images
# have systemctl and none of them has a user session, which is exactly the
# distinction the detection is supposed to make and exactly the one that a
# check of "is systemctl installed" would get wrong.
IMAGES="debian:12|debian|apt-get|debian
fedora:latest|fedora|dnf|fedora
archlinux:latest|arch|pacman|arch
opensuse/tumbleweed:latest|suse|zypper|opensuse-tumbleweed"

selected() {
  [ -z "$FILTERS" ] && return 0
  local filter
  for filter in $FILTERS; do
    case "$1" in *"$filter"*) return 0 ;; esac
  done
  return 1
}

# --------------------------------------------------------------- the engine

if [ -z "$ENGINE" ]; then
  for candidate in docker podman; do
    if command -v "$candidate" >/dev/null 2>&1; then
      ENGINE="$candidate"
      break
    fi
  done
fi

if [ "$LIST_ONLY" = 1 ]; then
  while IFS='|' read -r image family manager id; do
    [ -n "$image" ] || continue
    selected "$image" || continue
    printf '%-28s %s / %s / %s\n' "$image" "$id" "$family" "$manager"
  done <<EOF
$IMAGES
EOF
  exit 0
fi

if [ -z "$ENGINE" ] || ! command -v "$ENGINE" >/dev/null 2>&1; then
  echo "skipped: no container engine found (looked for docker and podman)."
  echo "         these checks need one to run the platform layer inside real"
  echo "         distro images; the rest of the suite does not. Install docker"
  echo "         or podman, or set CONTAINER_ENGINE, and run this again."
  exit 0
fi

if ! "$ENGINE" info >/dev/null 2>&1; then
  echo "skipped: $ENGINE is installed but not usable here ('$ENGINE info' failed)."
  echo "         start the daemon, or check your permissions, and run again."
  exit 0
fi

# --------------------------------------------------------------- the checking

PASSED=0
FAILED=0
SKIPPED=0
FAILED_IMAGES=""
CURRENT=""
PROBE=""

# skip <reason> - coverage this image cannot supply, counted and named.
#
# The whole point of running anything in a container is to find out what a
# real machine says. An image that cannot answer a question has to say so:
# a silent pass would be worth less than no check at all.
skip() {
  printf '  skip     %s\n' "$*"
  SKIPPED=$((SKIPPED + 1))
}

# value <key> - a key's value out of the probe output.
value() {
  printf '%s\n' "$PROBE" | sed -n "s/^$1=//p" | head -1
}

check() {
  local key="$1" expected="$2" actual
  actual="$(value "$key")"
  if [ "$actual" = "$expected" ]; then
    printf '  ok       %s = %s\n' "$key" "$actual"
    PASSED=$((PASSED + 1))
  else
    printf '  FAIL     %s: expected %s, got %s\n' "$key" "$expected" "${actual:-<nothing>}"
    FAILED=$((FAILED + 1))
    case "$FAILED_IMAGES" in
      *" $CURRENT"*) ;;
      *) FAILED_IMAGES="$FAILED_IMAGES $CURRENT" ;;
    esac
  fi
}

# check_contains <key> <substring>
check_contains() {
  local key="$1" needle="$2" actual
  actual="$(value "$key")"
  case "$actual" in
    *"$needle"*)
      printf '  ok       %s contains %s\n' "$key" "$needle"
      PASSED=$((PASSED + 1))
      ;;
    *)
      printf '  FAIL     %s: expected to contain %s, got %s\n' "$key" "$needle" "${actual:-<nothing>}"
      FAILED=$((FAILED + 1))
      case "$FAILED_IMAGES" in
        *" $CURRENT"*) ;;
        *) FAILED_IMAGES="$FAILED_IMAGES $CURRENT" ;;
      esac
      ;;
  esac
}

STARTED="$(date +%s)"

while IFS='|' read -r image family manager id; do
  [ -n "$image" ] || continue
  selected "$image" || continue
  CURRENT="$image"
  printf '\n== %s\n' "$image"

  PROBE="$("$ENGINE" run --rm -v "$REPO_ROOT:/repo:ro" -w /repo "$image" \
             bash tests/lib/platform_probe.sh 2>&1)"
  if [ "$?" != 0 ] || [ -z "$PROBE" ]; then
    printf '  FAIL     the probe did not run:\n'
    printf '%s\n' "$PROBE" | sed 's/^/           /'
    FAILED=$((FAILED + 1))
    FAILED_IMAGES="$FAILED_IMAGES $image"
    continue
  fi
  [ "$VERBOSE" = 1 ] && printf '%s\n' "$PROBE" | sed 's/^/           /'

  check os linux
  check id "$id"
  check family "$family"
  check pkg_manager "$manager"
  # Three of these images ship systemctl and none of them has a user session.
  check service_manager none
  check immutable no
  check known yes
  check can_act yes
  # The suite runs as root in a container, so no command may carry sudo.
  check privilege root
  check_contains install_command "$manager"

  # The query really did reach the real package database, in both directions.
  check present_installed yes
  check absent_installed no

  if [ "$RUN_INSTALL" = 1 ]; then
    printf '  -- installing for real inside %s\n' "$image"
    # Writable, unlike the probe above: this one installs. The repository stays
    # read-only, and the container is gone the moment it exits.
    PROBE="$("$ENGINE" run --rm -v "$REPO_ROOT:/repo:ro" -w /repo "$image" \
               bash tests/lib/install_probe.sh 2>&1)"
    if [ "$?" != 0 ] || [ -z "$PROBE" ]; then
      printf '  FAIL     the install probe did not run:\n'
      printf '%s\n' "$PROBE" | sed 's/^/           /'
      FAILED=$((FAILED + 1))
      FAILED_IMAGES="$FAILED_IMAGES $image"
      continue
    fi
    [ "$VERBOSE" = 1 ] && printf '%s\n' "$PROBE" | sed 's/^/           /'

    # The command really was this family's, and it really worked.
    check_contains install_command "$manager"
    check install_status 0
    # And the programs are there afterwards - which is the only claim worth
    # making, and the one an exit status of zero does not by itself support.
    for want in tmux python3 curl tar; do
      check "after_$want" yes
      check "db_$want" yes
    done
  fi

  # ------------------------------------------------------------- services
  #
  # A second probe, because this one is about what gets written rather than
  # about what the machine is. What a container can honestly answer:
  #
  #   whether a definition renders at all under this image's bash and
  #   userland - the tumbleweed image has neither find nor python3, and
  #   busybox coreutils are not GNU ones;
  #
  #   whether this distro's own systemd parser accepts what we wrote, which
  #   is the one thing no fixture and no golden file can tell us;
  #
  #   and that an image with systemctl installed and no session is detected
  #   as having no service manager, rather than being promised one.
  #
  # What it cannot answer is whether a unit can be enabled or started, because
  # there is no user session bus in any of these images. That is not worked
  # around here. It is reported as skipped, by name, every time.
  SERVICE_PROBE="$("$ENGINE" run --rm -v "$REPO_ROOT:/repo:ro" -w /repo "$image" \
                     bash tests/lib/service_probe.sh 2>&1)"
  if [ "$?" != 0 ] || [ -z "$SERVICE_PROBE" ]; then
    printf '  FAIL     the service probe did not run:\n'
    printf '%s\n' "$SERVICE_PROBE" | sed 's/^/           /'
    FAILED=$((FAILED + 1))
    FAILED_IMAGES="$FAILED_IMAGES $image"
  else
    [ "$VERBOSE" = 1 ] && printf '%s\n' "$SERVICE_PROBE" | sed 's/^/           /'
    PROBE="$SERVICE_PROBE"
    check rendered_ayeaye yes
    check rendered_whisper yes
    check placeholders none
    check exec_absolute yes
    check env_referenced yes
    check settings_leaked no
    # No session bus here, and the detection has to say so rather than
    # counting the systemctl binary as an answer.
    check user_bus absent
    check service_manager none
    case "$(value analyze)" in
      ok)
        printf '  ok       analyze = ok (this image'"'"'s own systemd parsed both units)\n'
        PASSED=$((PASSED + 1))
        ;;
      unavailable)
        skip "systemd-analyze: $image does not ship it, so nothing here parsed the units"
        ;;
      unusable)
        skip "systemd-analyze: $image ships it but it will not start in a container, so nothing here parsed the units"
        ;;
      *)
        printf '  FAIL     analyze: %s\n' "$(value analyze_note)"
        FAILED=$((FAILED + 1))
        FAILED_IMAGES="$FAILED_IMAGES $image"
        ;;
    esac
    skip "enabling and starting a user service: no session bus in a container"
  fi

  if [ "$RUN_SUITE" = 1 ]; then
    if "$ENGINE" run --rm -v "$REPO_ROOT:/repo:ro" -w /repo "$image" \
         sh -c 'command -v find >/dev/null 2>&1' >/dev/null 2>&1; then
      printf '  -- unit suite inside %s\n' "$image"
      if "$ENGINE" run --rm -v "$REPO_ROOT:/repo:ro" -w /repo "$image" \
           bash tests/run.sh platform_ 2>&1 | sed 's/^/     /'; then
        PASSED=$((PASSED + 1))
      else
        FAILED=$((FAILED + 1))
        FAILED_IMAGES="$FAILED_IMAGES $image"
      fi
    else
      # The tumbleweed image has neither find nor python3, so the runner's own
      # discovery cannot work there. The probe above still can, which is why
      # the probe is the primary check and this one is a flag.
      printf '  skip     unit suite: %s has no find for the runner to discover with\n' "$image"
    fi
  fi
done <<EOF
$IMAGES
EOF

ELAPSED=$(( $(date +%s) - STARTED ))
printf '\n%s checks passed, %s failed, %s skipped in %ss\n' \
  "$PASSED" "$FAILED" "$SKIPPED" "$ELAPSED"
if [ "$SKIPPED" -gt 0 ]; then
  printf 'skipped checks are coverage a container cannot supply; each is named above.\n'
fi
printf 'not covered anywhere: launchd. There is no Mac here, so the agents are\n'
printf '                     pinned by golden file and by plistlib only.\n'
if [ "$FAILED" -gt 0 ]; then
  printf 'failed images:%s\n' "$FAILED_IMAGES"
  exit 1
fi
exit 0
