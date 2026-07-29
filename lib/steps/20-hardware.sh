# What this computer can actually do, and how to say it to somebody who does
# not care what a gigabyte is.
#
# Registers into the `detect` and `report` stages. Read-only from end to end:
# nothing here installs, downloads, elevates, opens a port or writes anywhere
# but the setup state file and the setup log.
#
# ---------------------------------------------------------------- the contract
#
# The stages after this one act on a verdict rather than on numbers, so that
# each of them implements *selection* and only this file implements
# *judgement*. Two of these are the verdict and the rest describe it:
#
#   hw_acceleration    metal | cuda | rocm | cpu           <- the verdict
#   hw_voice_tier      text-only | lightweight |
#                      recommended | maximum               <- the verdict
#   hw_accel_usable    status 0 when that acceleration is worth acting on. A
#                      card can honestly be `cuda` and honestly be too small
#                      to hold anything, and a caller picking a build wants
#                      the first answer while a caller picking a model size
#                      wants the second
#   hw_tier_reason     one plain sentence saying why the tier is not higher,
#                      or nothing at all when nothing held it back
#   hw_tier_cause      one word naming what held it back, for a caller that
#                      wants to branch rather than to print: ram | disk |
#                      cores | ram-unknown | disk-unknown | cores-unknown |
#                      graphics-none | graphics-small | graphics-unknown |
#                      container, or empty
#   hw_tier_at_least <tier>
#                      status 0 when this machine reached that tier or better
#   hw_gpu_name        the card's own name, or nothing
#
# Every one of them reads the cache first, the state file second, and probes
# only if neither has an answer - so a later stage or a resumed run, which
# skips the step that worked all this out, gets the same answers rather than a
# default. The state keys behind them:
#
#   step.detect.hardware.acceleration    the acceleration verdict
#   step.detect.hardware.tier            the recommended voice tier
#   step.detect.hardware.tier_reason     the sentence, or empty
#   step.detect.hardware.tier_cause      the one-word cause, or empty
#   step.detect.hardware.ram_mb          whole-megabyte figures, or "unknown"
#   step.detect.hardware.cores
#   step.detect.hardware.disk_mb         free space where models would land
#   step.detect.hardware.vram_mb         graphics memory, or "unknown"
#   step.detect.hardware.gpu             the card's own name, or empty
#   step.detect.hardware.accel_usable    1 when the acceleration is worth
#                                        acting on, 0 when it is not
#   step.detect.hardware.container       1 inside a container, 0 outside
#   step.detect.hardware.limits          none | known | unknown - whether a
#                                        container's share could be read
#   step.detect.hardware.network         online | offline | unknown
#   step.detect.tools.<name>             1 when present, 0 when absent, for
#                                        ffmpeg whisper ollama tailscale cliban
#                                        and each agent command
#   step.detect.tools.whisper_command    which whisper binary was found
#   step.detect.tools.agents             the agent commands found, space
#                                        separated, or empty
#
# The accessors read the cache first and the state file second, which means
# they are meant to be read through `$(...)`. A command substitution is a
# subshell and a cache filled inside one dies with it, so a consumer that will
# ask more than once calls `hw_detect` in its own shell first. Correct either
# way, only free the first way.
#
# The acceleration verdict is a statement of fact and not of size: a machine
# with a small NVIDIA card still answers `cuda`, because CUDA is what is there,
# and a card whose size will not read is still a card. Whether it is big enough
# to be worth using is a second question, asked through hw_accel_usable and
# answered against the same constants the tier uses. Both live here so that no
# later stage has to know what a gigabyte of graphics memory is worth.
#
# ------------------------------------------------------------------ honesty
#
# Every probe answers "unknown" rather than guessing, and every unknown lowers
# the recommendation rather than raising it. A container is the sharp case: the
# core count and the memory figure a container reports are the host's, so a
# 512 MB share of a 64-core server reads as a 64-core server unless somebody
# looks at the cgroup limits. This file looks, clamps the numbers to the share
# when it can read it, and caps the tier when it cannot.
#
# bash 3.2: no associative arrays, no ${var,,}, no mapfile, no local -n.

# ----------------------------------------------------------------- tunables
#
# Every file-shaped input is a variable so that a test can point it at a
# fixture and never at the machine running the test. Commands are found on
# PATH, which the suite empties, so an unstubbed command is genuinely absent.
#
# Plain `${var-default}` assignment rather than `: "${var:=}"`: bash 3.2 does
# not create a variable when the := default is empty, and the first read of it
# under `set -u` would then be fatal.
HW_MEMINFO_FILE="${HW_MEMINFO_FILE-/proc/meminfo}"
HW_CPUINFO_FILE="${HW_CPUINFO_FILE-/proc/cpuinfo}"
HW_ROUTE_FILE="${HW_ROUTE_FILE-/proc/net/route}"
HW_ROUTE6_FILE="${HW_ROUTE6_FILE-/proc/net/ipv6_route}"

# Container detection: any one of the three is enough.
HW_DOCKERENV_FILE="${HW_DOCKERENV_FILE-/.dockerenv}"
HW_CONTAINERENV_FILE="${HW_CONTAINERENV_FILE-/run/.containerenv}"
HW_PROC1_CGROUP_FILE="${HW_PROC1_CGROUP_FILE-/proc/1/cgroup}"
HW_SYSTEMD_CONTAINER_FILE="${HW_SYSTEMD_CONTAINER_FILE-/run/systemd/container}"
HW_MOUNTINFO_FILE="${HW_MOUNTINFO_FILE-/proc/self/mountinfo}"

# Where this process's own cgroup is, and where the tree is mounted. The two
# together are how a container reads its own limits rather than the machine's:
# under a shared cgroup namespace the absolute paths below are the *host's*
# root, which reports no limit at all, and believing that is how a 512 MB share
# gets told it can hold a three-gigabyte model.
HW_PROC_SELF_CGROUP_FILE="${HW_PROC_SELF_CGROUP_FILE-/proc/self/cgroup}"
HW_CGROUP_ROOT="${HW_CGROUP_ROOT-/sys/fs/cgroup}"

# A container's actual share. v2 first, v1 second; both are read, neither is
# required.
HW_CGROUP_MEM_MAX_FILE="${HW_CGROUP_MEM_MAX_FILE-/sys/fs/cgroup/memory.max}"
HW_CGROUP_MEM_LIMIT_FILE="${HW_CGROUP_MEM_LIMIT_FILE-/sys/fs/cgroup/memory/memory.limit_in_bytes}"
HW_CGROUP_CPU_MAX_FILE="${HW_CGROUP_CPU_MAX_FILE-/sys/fs/cgroup/cpu.max}"
HW_CGROUP_CPU_QUOTA_FILE="${HW_CGROUP_CPU_QUOTA_FILE-/sys/fs/cgroup/cpu/cpu.cfs_quota_us}"
HW_CGROUP_CPU_PERIOD_FILE="${HW_CGROUP_CPU_PERIOD_FILE-/sys/fs/cgroup/cpu/cpu.cfs_period_us}"

# Free space is only interesting where the models would actually land. The
# probe walks up from here to the first directory that exists, because asking
# df about a directory nobody has created yet answers nothing.
HW_MODEL_DIR="${HW_MODEL_DIR-${HOME-}/whisper-models}"

# --- the threshold table -----------------------------------------------------
#
# The only place in this repository where a cut-off is written down. Every
# number is in whole megabytes, and every one of them is deliberately shy of
# the round figure it is named after, because a machine sold as 16 GB reports
# something between 15.2 and 15.9 to /proc/meminfo once the firmware has taken
# its share. A cut-off written as 16384 would put every 16 GB machine on the
# wrong side of its own line.
#
# What each line is protecting the user from:
#
#   lightweight   below this, speech-to-text is slower than typing and the
#                 machine is better off being told so than discovering it
#   recommended   the comfortable middle: a small transcription model and a
#                 7B instruct model to tidy up what it heard
#   maximum       the largest transcription model resident in memory, which is
#                 only comfortable with a card to hold it

HW_RAM_MB_LIGHTWEIGHT=3584      # a nominal 4 GB machine
HW_RAM_MB_RECOMMENDED=7168      # a nominal 8 GB machine
HW_RAM_MB_MAXIMUM=15360         # a nominal 16 GB machine

HW_CORES_LIGHTWEIGHT=2
HW_CORES_RECOMMENDED=4
HW_CORES_MAXIMUM=8

# Free space where the models land. A base transcription model is about 150 MB,
# a small one 500 MB, the largest 3 GB, and the instruct model that cleans up
# dictation is about 4.7 GB. Each line is the models plus room to download them
# before they are unpacked.
HW_DISK_MB_LIGHTWEIGHT=1536
HW_DISK_MB_RECOMMENDED=6144
HW_DISK_MB_MAXIMUM=12288

# Graphics memory. The first line is "this card is worth using at all"; the
# second is "this card can hold the largest model". They are different
# questions and a single number would answer neither well.
HW_VRAM_MB_ACCELERATED=3584     # a nominal 4 GB card
HW_VRAM_MB_MAXIMUM=7168         # a nominal 8 GB card
#
# --- end of the threshold table ----------------------------------------------

# The cache. Filled by hw_detect, read by everything else.
_HW_DETECTED="${_HW_DETECTED:-0}"
_HW_RAM_MB="${_HW_RAM_MB:-unknown}"
_HW_CORES="${_HW_CORES:-unknown}"
_HW_DISK_MB="${_HW_DISK_MB:-unknown}"
_HW_VRAM_MB="${_HW_VRAM_MB:-unknown}"
_HW_ACCEL="${_HW_ACCEL:-cpu}"
_HW_GPU_NAME="${_HW_GPU_NAME:-}"
_HW_CONTAINER="${_HW_CONTAINER:-0}"
_HW_LIMITS="${_HW_LIMITS:-none}"
_HW_NETWORK="${_HW_NETWORK:-unknown}"
_HW_TIER="${_HW_TIER:-}"
_HW_TIER_CAUSE="${_HW_TIER_CAUSE:-}"
_HW_TIER_REASON="${_HW_TIER_REASON:-}"

# ============================================================ tier arithmetic

# _hw_is_number <value> - status 0 for a run of digits and nothing else.
# "unknown", "", "lots" and "8 cores" are all not numbers, and all of them
# arrive here at some point from a command that answered something unexpected.
_hw_is_number() {
  case "${1:-}" in
    "") return 1 ;;
    *[!0-9]*) return 1 ;;
    *) return 0 ;;
  esac
}

# hw_tier_rank <tier> -> 0..3 on stdout.
#
# Status 2 and no output for a word that is not one of the four: a caller that
# invented a tier name has a bug, and answering it with a number would hide it.
hw_tier_rank() {
  case "${1:-}" in
    text-only)   printf '0' ;;
    lightweight) printf '1' ;;
    recommended) printf '2' ;;
    maximum)     printf '3' ;;
    *) return 2 ;;
  esac
  return 0
}

# hw_tier_min <a> <b> -> the lower of the two. Status 2 if either is not a tier.
hw_tier_min() {
  local ra rb
  ra="$(hw_tier_rank "${1:-}")" || return 2
  rb="$(hw_tier_rank "${2:-}")" || return 2
  if [ "$ra" -le "$rb" ]; then
    printf '%s' "$1"
  else
    printf '%s' "$2"
  fi
  return 0
}

# _hw_dim_tier <value> <lightweight> <recommended> <maximum> -> the tier this
# one measurement supports. A value that is not a number is the caller's
# problem to have checked; here it reads as the floor.
_hw_dim_tier() {
  local v="${1:-0}"
  _hw_is_number "$v" || { printf 'text-only'; return 0; }
  if   [ "$v" -ge "$4" ]; then printf 'maximum'
  elif [ "$v" -ge "$3" ]; then printf 'recommended'
  elif [ "$v" -ge "$2" ]; then printf 'lightweight'
  else                         printf 'text-only'
  fi
  return 0
}

# _hw_cap <tier> <cause> <reason> - lower the working answer, and say why.
#
# Strictly lower, so the first constraint to bind is the one that gets to
# explain itself and a later constraint of equal weight cannot overwrite a more
# specific sentence with a vaguer one.
_hw_cap() {
  local rank_new rank_now
  # Never non-zero. This is called bare from nine places inside the one
  # function whose whole contract is that nothing here ends a setup run, and a
  # tier name nobody defined is a reason to leave the answer alone rather than
  # a reason to stop.
  rank_new="$(hw_tier_rank "$1")" || return 0
  rank_now="$(hw_tier_rank "$_HW_TIER")" || return 0
  [ "$rank_new" -lt "$rank_now" ] || return 0
  _HW_TIER="$1"
  _HW_TIER_CAUSE="$2"
  _HW_TIER_REASON="$3"
  return 0
}

# _hw_tier_compute <ram_mb> <cores> <disk_mb> <accel> <vram_mb> <limits>
#
# Sets $_HW_TIER and $_HW_TIER_REASON. Answers in variables rather than on
# stdout because two answers do not fit down one pipe, and because the caller
# that matters - the step - wants both without a subshell.
#
# <limits> is none when this is not a container, known when it is one whose
# share of the machine was read, and unknown when it is one whose share could
# not be. Anything else is read as unknown, which is the safe direction.
#
# The shape of it: start at the top, and let every constraint push down. A
# constraint can only ever lower the answer. There is deliberately no path by
# which a graphics card raises a tier that memory or free space did not already
# support - a card cannot hold a model there is no room to download.
_hw_tier_compute() {
  local ram="${1:-}" cores="${2:-}" disk="${3:-}" accel="${4:-cpu}"
  local vram="${5:-}" limits="${6:-none}"

  _HW_TIER=maximum
  _HW_TIER_CAUSE=""
  _HW_TIER_REASON=""

  # Every sentence below is short enough that "Why not more than that: " and a
  # full stop still fit on one line of a narrow terminal. A reason that wraps
  # is a reason that looks like an error message.
  if _hw_is_number "$ram"; then
    _hw_cap "$(_hw_dim_tier "$ram" "$HW_RAM_MB_LIGHTWEIGHT" \
      "$HW_RAM_MB_RECOMMENDED" "$HW_RAM_MB_MAXIMUM")" ram \
      "there is not much memory in this computer"
  else
    _hw_cap lightweight ram-unknown \
      "setup could not read this computer's memory"
  fi

  if _hw_is_number "$disk"; then
    _hw_cap "$(_hw_dim_tier "$disk" "$HW_DISK_MB_LIGHTWEIGHT" \
      "$HW_DISK_MB_RECOMMENDED" "$HW_DISK_MB_MAXIMUM")" disk \
      "there is not much free space left on the disk"
  else
    _hw_cap lightweight disk-unknown \
      "setup could not read the disk's free space"
  fi

  if _hw_is_number "$cores"; then
    _hw_cap "$(_hw_dim_tier "$cores" "$HW_CORES_LIGHTWEIGHT" \
      "$HW_CORES_RECOMMENDED" "$HW_CORES_MAXIMUM")" cores \
      "this computer has only a few processors"
  else
    _hw_cap recommended cores-unknown \
      "setup could not count the processors"
  fi

  # Acceleration caps and never lifts.
  case "$accel" in
    metal)
      # One pool of memory shared between the processor and the graphics, so
      # the memory line above has already said everything there is to say and
      # there is no second figure to be ignorant of. This is the one branch
      # that caps nothing, and it is deliberate: a Mac reaches the top tier
      # only by having the memory for it, which is the same test every other
      # machine passes. An 8 GB Mac is capped by the memory line like anything
      # else.
      : ;;
    cuda|rocm)
      if ! _hw_is_number "$vram"; then
        _hw_cap recommended graphics-unknown \
          "setup could not read the graphics card's size"
      elif [ "$vram" -ge "$HW_VRAM_MB_MAXIMUM" ]; then
        :
      elif [ "$vram" -ge "$HW_VRAM_MB_ACCELERATED" ]; then
        _hw_cap recommended graphics-small \
          "the graphics card cannot hold the largest model"
      else
        # Below the first line the card is not worth using at all, and the
        # machine is a processor-only machine that happens to have a card in
        # it. Saying so is the difference between a true sentence and a
        # useful one.
        _hw_cap recommended graphics-small \
          "the graphics card is too small to be any use"
      fi
      ;;
    *)
      # cpu, and anything this code does not recognise, which is read as cpu.
      _hw_cap recommended graphics-none \
        "there is no graphics card ayeaye can use here"
      ;;
  esac

  case "$limits" in
    none|known) : ;;
    *)
      _hw_cap recommended container \
        "only part of this computer is yours to use"
      ;;
  esac
  return 0
}

# hw_tier_for <ram_mb> <cores> <disk_mb> <accel> <vram_mb> [limits] -> the tier.
#
# The same thing with the answer on stdout, for a caller that wants only the
# word and does not mind the subshell.
hw_tier_for() {
  _hw_tier_compute "${1:-}" "${2:-}" "${3:-}" "${4:-cpu}" "${5:-}" "${6:-none}"
  printf '%s' "$_HW_TIER"
  return 0
}

# ================================================================== probing
#
# Every probe here answers on stdout with a whole number or the word "unknown",
# and every one of them exits 0. There is nothing a probe can discover that is
# worth ending a setup run over, and a probe that returns non-zero under a
# `set -e` caller would do exactly that.
#
# Nothing below uses grep, sed or awk. The suite empties PATH, so a probe that
# reached for one would be testing the harness rather than the machine, and the
# parsing here is small enough that shell pattern matching does it plainly.

# _hw_trim <text> -> $_HW_S with leading and trailing blanks removed.
# Answered in a variable rather than on stdout: these run several times per
# parsed file, and a command substitution per line is a fork per line.
_HW_S="${_HW_S:-}"
_HW_CR="${_HW_CR:-$'\r'}"
_hw_trim() {
  local s="${1:-}"
  # A carriage return counts as blank. A file that arrived through a Windows
  # editor, or a command answering over a serial console, would otherwise fail
  # every numeric check in this file for a reason nobody would ever guess.
  while :; do
    case "$s" in
      " "*|"	"*|"$_HW_CR"*) s="${s#?}" ;;
      *) break ;;
    esac
  done
  while :; do
    case "$s" in
      *" "|*"	"|*"$_HW_CR") s="${s%?}" ;;
      *) break ;;
    esac
  done
  _HW_S="$s"
}

# _hw_readable <path> - a readable regular file, and not a directory.
#
# `[ -r ]` alone is true of a directory, and `read` from one prints a raw
# diagnostic on stderr in the middle of a wizard that has promised to be
# quiet. lib/platform.sh guards its own file reads the same way.
_hw_readable() {
  [ -f "${1:-}" ] && [ -r "$1" ]
}

# _hw_first_word <text> -> the first whitespace-separated word, or nothing.
#
# Through `read` and not through `set -- $text`: an unquoted expansion is
# subject to pathname expansion as well as to word splitting, and rocminfo
# prints lines of asterisks. A parser that turned one of those into the
# contents of the working directory would be a very confusing bug.
_hw_first_word() {
  local w="" rest="" IFS=" 	
"
  read -r w rest <<EOF
${1:-}
EOF
  printf '%s' "$w"
}

# _hw_last_word <text> -> the final whitespace-separated word, or nothing.
_hw_last_word() {
  _hw_trim "${1:-}"
  case "$_HW_S" in
    *[!\ ]*) : ;;
    *) printf ''; return 0 ;;
  esac
  printf '%s' "${_HW_S##* }"
}

# _hw_field <n> <line> -> the nth whitespace-separated field, or nothing.
# n above five is the whole remainder, which is all any caller here wants.
_hw_field() {
  local n="${1:-1}" a="" b="" c="" d="" e="" IFS=" 	
"
  read -r a b c d e <<EOF
${2:-}
EOF
  case "$n" in
    1) printf '%s' "$a" ;;
    2) printf '%s' "$b" ;;
    3) printf '%s' "$c" ;;
    4) printf '%s' "$d" ;;
    *) printf '%s' "$e" ;;
  esac
}

# _hw_positive <value> - status 0 for a whole number greater than zero.
# Zero cores and zero bytes of memory are not machines; they are a probe that
# answered something it did not mean.
_hw_positive() {
  _hw_is_number "${1:-}" || return 1
  [ "$1" -gt 0 ]
}

# ------------------------------------------------------------- how many cores

_hw_cores_from_lscpu() {
  local out="" line=""
  command -v lscpu >/dev/null 2>&1 || return 1
  # LC_ALL=C because util-linux is translated: a French machine prints
  # "Processeur(s) :" and every pattern below would miss it.
  out="$(LC_ALL=C lscpu 2>/dev/null)" || return 1
  while IFS= read -r line || [ -n "$line" ]; do
    # At the left margin only. lscpu prints "CPU(s):" three times - once as the
    # count and twice indented, for the on-line list and for the NUMA node -
    # and an unanchored match picks up whichever comes last.
    case "$line" in
      "CPU(s):"*)
        _hw_trim "${line#CPU\(s\):}"
        _hw_positive "$_HW_S" || return 1
        printf '%s' "$_HW_S"
        return 0
        ;;
    esac
  done <<EOF
$out
EOF
  return 1
}

_hw_cores_from_nproc() {
  local out=""
  command -v nproc >/dev/null 2>&1 || return 1
  out="$(LC_ALL=C nproc 2>/dev/null)" || return 1
  _hw_trim "$out"
  _hw_positive "$_HW_S" || return 1
  printf '%s' "$_HW_S"
}

_hw_cores_from_cpuinfo() {
  local line="" n=0
  _hw_readable "$HW_CPUINFO_FILE" || return 1
  while IFS= read -r line || [ -n "$line" ]; do
    case "$line" in
      processor*:*) n=$((n + 1)) ;;
    esac
  done < "$HW_CPUINFO_FILE"
  [ "$n" -gt 0 ] || return 1
  printf '%s' "$n"
}

_hw_cores_from_sysctl() {
  local out=""
  command -v sysctl >/dev/null 2>&1 || return 1
  out="$(sysctl -n hw.ncpu 2>/dev/null)" || return 1
  _hw_trim "$out"
  _hw_positive "$_HW_S" || return 1
  printf '%s' "$_HW_S"
}

# system_profiler is the slowest of the four and the last asked for that
# reason. "Total Number of Cores: 8 (4 performance and 4 efficiency)" is eight
# cores, not eight-something: the first word after the colon is the whole
# answer and the parenthesis is commentary.
_hw_from_system_profiler() {
  local want="$1" out="" line=""
  command -v system_profiler >/dev/null 2>&1 || return 1
  out="$(system_profiler SPHardwareDataType 2>/dev/null)" || return 1
  while IFS= read -r line || [ -n "$line" ]; do
    _hw_trim "$line"
    case "$_HW_S" in
      "$want:"*)
        _hw_trim "${_HW_S#"$want":}"
        printf '%s' "$_HW_S"
        return 0
        ;;
    esac
  done <<EOF
$out
EOF
  return 1
}

_hw_cores_from_system_profiler() {
  local value=""
  value="$(_hw_from_system_profiler "Total Number of Cores")" || return 1
  value="$(_hw_first_word "$value")"
  _hw_positive "$value" || return 1
  printf '%s' "$value"
}

# hw_probe_cores -> a whole number, or "unknown". Always 0.
hw_probe_cores() {
  local n=""
  # nproc first, and lscpu second. nproc answers what this process may
  # actually be scheduled on - which is what --cpuset-cpus, taskset and a
  # kubernetes cpuset each change - while lscpu answers what the kernel can
  # see, which inside a pinned container is the whole node.
  if n="$(_hw_cores_from_nproc)" \
     || n="$(_hw_cores_from_lscpu)" \
     || n="$(_hw_cores_from_cpuinfo)" \
     || n="$(_hw_cores_from_sysctl)" \
     || n="$(_hw_cores_from_system_profiler)"; then
    printf '%s' "$n"
  else
    printf 'unknown'
  fi
  return 0
}

# ------------------------------------------------------------ how much memory

_hw_ram_from_meminfo() {
  local line="" kb=""
  _hw_readable "$HW_MEMINFO_FILE" || return 1
  while IFS= read -r line || [ -n "$line" ]; do
    case "$line" in
      MemTotal:*)
        _hw_trim "${line#MemTotal:}"
        kb="$(_hw_first_word "$_HW_S")"
        _hw_positive "$kb" || return 1
        printf '%s' "$((10#$kb / 1024))"
        return 0
        ;;
    esac
  done < "$HW_MEMINFO_FILE"
  return 1
}

_hw_ram_from_free() {
  local out="" line="" mb=""
  command -v free >/dev/null 2>&1 || return 1
  out="$(LC_ALL=C free -m 2>/dev/null)" || return 1
  while IFS= read -r line || [ -n "$line" ]; do
    case "$line" in
      Mem:*)
        mb="$(_hw_field 2 "$line")"
        _hw_positive "$mb" || return 1
        printf '%s' "$mb"
        return 0
        ;;
    esac
  done <<EOF
$out
EOF
  return 1
}

_hw_ram_from_sysctl() {
  local out=""
  command -v sysctl >/dev/null 2>&1 || return 1
  out="$(sysctl -n hw.memsize 2>/dev/null)" || return 1
  _hw_trim "$out"
  _hw_positive "$_HW_S" || return 1
  printf '%s' "$((10#$_HW_S / 1048576))"
}

# "Memory: 24 GB" - the only place in this file where a unit has to be read
# rather than assumed, because system_profiler is the only source that prints
# one.
_hw_ram_from_system_profiler() {
  local value="" n="" unit=""
  value="$(_hw_from_system_profiler "Memory")" || return 1
  n="$(_hw_field 1 "$value")"
  unit="$(_hw_field 2 "$value")"
  _hw_positive "$n" || return 1
  case "$unit" in
    GB|gb|GiB) printf '%s' "$((10#$n * 1024))" ;;
    MB|mb|MiB) printf '%s' "$((10#$n))" ;;
    TB|tb|TiB) printf '%s' "$((10#$n * 1024 * 1024))" ;;
    *) return 1 ;;
  esac
}

# hw_probe_ram_mb -> whole megabytes, or "unknown". Always 0.
hw_probe_ram_mb() {
  local n=""
  if n="$(_hw_ram_from_meminfo)" \
     || n="$(_hw_ram_from_free)" \
     || n="$(_hw_ram_from_sysctl)" \
     || n="$(_hw_ram_from_system_profiler)"; then
    printf '%s' "$n"
  else
    printf 'unknown'
  fi
  return 0
}

# --------------------------------------------------------- how much free space

# _hw_disk_dir -> the nearest ancestor of the model directory that exists.
# On a first run nothing has created it yet, and df has nothing to say about a
# path that is not there.
_hw_disk_dir() {
  local d="$HW_MODEL_DIR"
  while [ -n "$d" ] && [ "$d" != "/" ]; do
    if [ -d "$d" ]; then
      printf '%s' "$d"
      return 0
    fi
    case "$d" in
      */*) d="${d%/*}" ;;
      *)   d="" ;;
    esac
    [ -n "$d" ] || d="/"
  done
  printf '/'
}

# hw_probe_disk_mb -> whole megabytes free where the models would land, or
# "unknown". Always 0.
#
# `df -Pk` and not `df -h`: POSIX output never wraps a long device name onto a
# second line, and kilobytes are a number rather than "1.4G".
hw_probe_disk_mb() {
  local dir="" out="" line="" last="" blocks=""
  command -v df >/dev/null 2>&1 || { printf 'unknown'; return 0; }
  dir="$(_hw_disk_dir)"
  out="$(LC_ALL=C df -Pk "$dir" 2>/dev/null)" || { printf 'unknown'; return 0; }
  while IFS= read -r line || [ -n "$line" ]; do
    [ -n "$line" ] || continue
    last="$line"
  done <<EOF
$out
EOF
  blocks="$(_hw_field 4 "$last")"
  if _hw_is_number "$blocks"; then
    printf '%s' "$((10#$blocks / 1024))"
  else
    printf 'unknown'
  fi
  return 0
}

# ---------------------------------------------------------------- what graphics

# _hw_nvidia - the card's name into $_HW_S and its megabytes into $_HW_N, or
# status 1. Asking for both in one query means one exec and one parse.
_HW_N="${_HW_N:-}"
_hw_nvidia() {
  local out="" line="" mem="" name="" best=0 found=0
  command -v nvidia-smi >/dev/null 2>&1 || return 1
  out="$(LC_ALL=C nvidia-smi --query-gpu=name,memory.total --format=csv,noheader \
    2>/dev/null)" || return 1
  while IFS= read -r line || [ -n "$line" ]; do
    case "$line" in
      # A driver that is not loaded prints a paragraph of prose. One comma per
      # card is the shape that tells a listing from an apology; a card whose
      # name contains a comma would put it in ${line%,*} where it belongs.
      *,*) : ;;
      *) continue ;;
    esac
    _hw_trim "${line%,*}"
    [ -n "$_HW_S" ] || continue
    found=1
    # The first card's name, whatever else is or is not readable about it: a
    # listing that says nothing but [N/A] still has a card in it to name.
    [ -n "$name" ] || name="$_HW_S"
    mem="$(_hw_first_word "${line##*,}")"
    # "[N/A]" is what vGPU, MIG and WSL answer. The card is still a card and
    # the verdict is still cuda; it is the size that is unknown, and unknown
    # is what caps the tier.
    _hw_positive "$mem" || continue
    if [ "$((10#$mem))" -gt "$best" ]; then
      best="$((10#$mem))"
      name="$_HW_S"
    fi
  done <<EOF
$out
EOF
  [ "$found" = 1 ] || return 1
  _HW_S="$name"
  if [ "$best" -gt 0 ]; then
    _HW_N="$best"
  else
    _HW_N="unknown"
  fi
  return 0
}

# _hw_rocm - the same for an AMD card.
#
# rocminfo prints one block per HSA agent and the processor is an agent too, so
# the parse only starts believing what it reads once it has seen an agent whose
# name is a gfx target. The size is best-effort: the pool sizes are the only
# figure there is, the largest of them is the card's memory, and a layout this
# code does not recognise answers "unknown" rather than a number - which costs
# the machine a tier and never gains it one.
_hw_rocm() {
  local out="" line="" in_gpu=0 found=0 name="" kb="" best=0 cpu_best=0 raw=""
  command -v rocminfo >/dev/null 2>&1 || return 1
  out="$(LC_ALL=C rocminfo 2>/dev/null)" || return 1
  while IFS= read -r line || [ -n "$line" ]; do
    _hw_trim "$line"
    case "$_HW_S" in
      "Agent "*) in_gpu=0 ;;
      "Name:"*)
        _hw_trim "${_HW_S#Name:}"
        case "$_HW_S" in
          gfx*) in_gpu=1; found=1 ;;
        esac
        ;;
      "Marketing Name:"*)
        [ "$in_gpu" = 1 ] || continue
        _hw_trim "${_HW_S#Marketing Name:}"
        [ -n "$name" ] || name="$_HW_S"
        ;;
      "Size:"*)
        _hw_trim "${_HW_S#Size:}"
        raw="$_HW_S"
        kb="${raw%%(*}"
        _hw_trim "$kb"
        kb="$(_hw_first_word "$_HW_S")"
        _hw_positive "$kb" || continue
        kb="$((10#$kb))"
        if [ "$in_gpu" = 1 ]; then
          [ "$kb" -gt "$best" ] && best="$kb"
        else
          # The processor's own pool, kept so that the card's can be compared
          # against it below.
          [ "$kb" -gt "$cpu_best" ] && cpu_best="$kb"
        fi
        ;;
    esac
  done <<EOF
$out
EOF
  [ "$found" = 1 ] || return 1
  _HW_S="$name"
  # Graphics built into the processor is an HSA agent like any other, and the
  # pool it reports is the machine's own memory - the same gigabytes the memory
  # line has already counted. Reporting them again as graphics memory would let
  # one pool of memory satisfy two independent checks, which is exactly the
  # double-count the size check exists to prevent. Within a tenth of the
  # processor's pool means shared, and shared means there is no separate figure.
  if [ "$best" -gt 0 ] && [ "$cpu_best" -gt 0 ] \
     && [ "$((best + best / 10))" -ge "$cpu_best" ]; then
    best=0
  fi
  if [ "$best" -gt 0 ]; then
    _HW_N="$((best / 1024))"
  else
    _HW_N="unknown"
  fi
  return 0
}

# _hw_chip_name - what the Apple Silicon processor calls itself.
_hw_chip_name() {
  local out=""
  if command -v sysctl >/dev/null 2>&1; then
    if out="$(LC_ALL=C sysctl -n machdep.cpu.brand_string 2>/dev/null)"; then
      _hw_trim "$out"
      if [ -n "$_HW_S" ]; then
        printf '%s' "$_HW_S"
        return 0
      fi
    fi
  fi
  _hw_from_system_profiler "Chip"
}

# hw_probe_acceleration - sets $_HW_ACCEL, $_HW_VRAM_MB and $_HW_GPU_NAME.
#
# Precedence, and why: Apple Silicon first, because a Mac's answer cannot be
# anything else and asking two more commands would only find a way to be wrong;
# then NVIDIA, then AMD, because a machine with both is a machine where CUDA is
# the better-supported of the two for what this project runs.
#
# The verdict is a statement of fact and not of size. A two-gigabyte NVIDIA
# card still answers "cuda"; whether it is big enough to be worth using is the
# tier's business, and the tier is where the sizes live.
hw_probe_acceleration() {
  _HW_ACCEL=cpu
  _HW_VRAM_MB=unknown
  _HW_GPU_NAME=""

  platform_detect
  if [ "$(platform_os)" = macos ]; then
    # Intel Macs are deliberately not Metal here. They have a Metal device, but
    # the accelerated builds this project points people at are Apple Silicon
    # builds, and promising acceleration that is not there costs more than
    # declining to promise it.
    if [ "$(platform_arch)" = arm64 ]; then
      _HW_ACCEL=metal
      # One pool of memory, shared with the processor. There is no separate
      # figure to report and inventing one would be a lie in either direction.
      # sysctl answers "Apple M3" immediately; system_profiler answers the
      # same thing one to three seconds later, and is only worth waiting for
      # when sysctl has nothing to say.
      _HW_GPU_NAME="$(_hw_chip_name)" || _HW_GPU_NAME=""
    fi
    return 0
  fi

  if _hw_nvidia; then
    _HW_ACCEL=cuda
    _HW_GPU_NAME="$_HW_S"
    _HW_VRAM_MB="$_HW_N"
    return 0
  fi
  if _hw_rocm; then
    _HW_ACCEL=rocm
    _HW_GPU_NAME="$_HW_S"
    _HW_VRAM_MB="$_HW_N"
    return 0
  fi
  return 0
}

# ------------------------------------------------------------- what container

# Anything at or above this is a cgroup saying "no limit" in the old layout's
# idiom of a number as large as it can count to.
_HW_NO_LIMIT_FLOOR=4611686018427387904

_HW_LIMIT_RAM_MB="${_HW_LIMIT_RAM_MB:-none}"
_HW_LIMIT_CORES="${_HW_LIMIT_CORES:-none}"

# _hw_read_line <path> -> the first line, or status 1.
_hw_read_line() {
  local line=""
  _hw_readable "$1" || return 1
  IFS= read -r line < "$1" 2>/dev/null || [ -n "$line" ] || return 1
  _hw_trim "$line"
  [ -n "$_HW_S" ] || return 1
  printf '%s' "$_HW_S"
}

# _hw_cgroup_rel <subsystem> -> this process's own cgroup path, or status 1.
#
# An empty subsystem asks for the unified hierarchy, whose line in
# /proc/self/cgroup is "0::/some/path". A named one asks for the version-one
# hierarchy that controls it, whose line is "7:memory,cpuset:/some/path" - the
# controller field is a comma-separated list and the subsystem has to be a
# whole element of it, or "cpu" would match "cpuacct".
_hw_cgroup_rel() {
  local want="${1:-}" line="" hier="" ctrls="" path="" c=""
  _hw_readable "$HW_PROC_SELF_CGROUP_FILE" || return 1
  while IFS= read -r line || [ -n "$line" ]; do
    hier="${line%%:*}"
    line="${line#*:}"
    ctrls="${line%%:*}"
    path="${line#*:}"
    [ -n "$path" ] || continue
    if [ -z "$want" ]; then
      if [ "$hier" = 0 ] && [ -z "$ctrls" ]; then
        printf '%s' "$path"
        return 0
      fi
      continue
    fi
    # shellcheck disable=SC2086
    for c in $(printf '%s' "$ctrls" | tr , ' ' 2>/dev/null); do
      if [ "$c" = "$want" ]; then
        printf '%s' "$path"
        return 0
      fi
    done
  done < "$HW_PROC_SELF_CGROUP_FILE"
  return 1
}

# _hw_cgroup_path <subsystem> <leaf> -> where that file is for this process.
#
# "/sys/fs/cgroup/memory.max" is the *host's* root under a shared cgroup
# namespace, and the root has no limit on anything - which is a true statement
# about the host and a dangerous one about a container. Composing the path from
# /proc/self/cgroup asks the question about this process instead.
_hw_cgroup_path() {
  local want="${1:-}" leaf="${2:-}" rel="" base=""
  rel="$(_hw_cgroup_rel "$want")" || return 1
  case "$rel" in
    /) rel="" ;;
    */) rel="${rel%/}" ;;
  esac
  if [ -z "$want" ]; then
    base="$HW_CGROUP_ROOT$rel"
  else
    base="$HW_CGROUP_ROOT/$want$rel"
  fi
  printf '%s/%s' "$base" "$leaf"
}

# _hw_cgroup_read <subsystem> <leaf> <fallback-path> -> the value, or status 1.
# The composed path first, the plain tunable second: the tunable is what a test
# points at a fixture, and what a machine with no /proc/self/cgroup falls back
# to.
_hw_cgroup_read() {
  local path=""
  if path="$(_hw_cgroup_path "${1:-}" "${2:-}")"; then
    if _hw_read_line "$path"; then
      return 0
    fi
  fi
  _hw_read_line "${3:-}"
}

# Anything at or above this is a cgroup saying "no limit" in the version-one
# idiom of a number as large as it can count to.
_HW_NO_LIMIT_FLOOR=4611686018427387904

_HW_LIMIT_RAM_MB="${_HW_LIMIT_RAM_MB:-none}"
_HW_LIMIT_CORES="${_HW_LIMIT_CORES:-none}"

# _hw_cgroup_memory -> megabytes, or "none" for no limit, or "unknown".
_hw_cgroup_memory() {
  local v=""
  if v="$(_hw_cgroup_read "" memory.max "$HW_CGROUP_MEM_MAX_FILE")"; then
    case "$v" in
      max) printf 'none'; return 0 ;;
    esac
  elif v="$(_hw_cgroup_read memory memory.limit_in_bytes "$HW_CGROUP_MEM_LIMIT_FILE")"; then
    :
  else
    printf 'unknown'
    return 0
  fi
  if ! _hw_positive "$v"; then
    printf 'unknown'
    return 0
  fi
  if [ "$((10#$v))" -ge "$_HW_NO_LIMIT_FLOOR" ]; then
    printf 'none'
    return 0
  fi
  printf '%s' "$((10#$v / 1048576))"
}

# _hw_cgroup_cores -> whole cores, or "none" for no limit, or "unknown".
#
# A share of a core and a half is one core you can count on. Rounding it up to
# two would be counting on a core that is not there, which is the direction
# this file never rounds in; a share of less than a whole core still counts as
# one, because there is no such thing as most of a processor to schedule on.
_hw_cgroup_cores() {
  local v="" quota="" period="" n=""
  if v="$(_hw_cgroup_read "" cpu.max "$HW_CGROUP_CPU_MAX_FILE")"; then
    quota="$(_hw_field 1 "$v")"
    period="$(_hw_field 2 "$v")"
    case "$quota" in
      max) printf 'none'; return 0 ;;
    esac
  elif quota="$(_hw_cgroup_read cpu cpu.cfs_quota_us "$HW_CGROUP_CPU_QUOTA_FILE")"; then
    period="$(_hw_cgroup_read cpu cpu.cfs_period_us "$HW_CGROUP_CPU_PERIOD_FILE")" \
      || period=""
    case "$quota" in
      -1) printf 'none'; return 0 ;;
    esac
  else
    printf 'unknown'
    return 0
  fi
  if ! _hw_positive "$quota" || ! _hw_positive "$period"; then
    printf 'unknown'
    return 0
  fi
  n="$((10#$quota / 10#$period))"
  [ "$n" -ge 1 ] || n=1
  printf '%s' "$n"
}

# _hw_container_marked - status 0 when something on this machine says outright
# that it is a container.
#
# Five marks, because no one of them is reliable. Docker writes /.dockerenv;
# podman and lxc write /run/.containerenv and the `container` environment
# variable; systemd-nspawn writes /run/systemd/container. A containerd or CRI
# pod writes none of them, which is what the limits below are for.
_hw_container_marked() {
  local line=""
  _hw_readable "$HW_DOCKERENV_FILE" && return 0
  _hw_readable "$HW_CONTAINERENV_FILE" && return 0
  _hw_readable "$HW_SYSTEMD_CONTAINER_FILE" && return 0
  [ -n "${container-}" ] && return 0
  if _hw_readable "$HW_PROC1_CGROUP_FILE"; then
    while IFS= read -r line || [ -n "$line" ]; do
      case "$line" in
        *docker*|*lxc*|*kubepods*|*libpod*|*containerd*|*/machine.slice/*)
          return 0
          ;;
      esac
    done < "$HW_PROC1_CGROUP_FILE"
  fi
  if _hw_readable "$HW_MOUNTINFO_FILE"; then
    while IFS= read -r line || [ -n "$line" ]; do
      case "$line" in
        */docker/containers/*|*/kubepods/*|*/containers/storage/*)
          return 0
          ;;
      esac
    done < "$HW_MOUNTINFO_FILE"
  fi
  return 1
}

# hw_probe_container - sets $_HW_CONTAINER, $_HW_LIMITS, $_HW_LIMIT_RAM_MB and
# $_HW_LIMIT_CORES.
#
# The limits are read whether or not a mark was found, and for two reasons. A
# kubernetes pod under containerd has no marker file, an unshared cgroup
# namespace that makes /proc/1/cgroup read "0::/", and a memory limit - so the
# limit is the only evidence there is, and a run that only looked for marks
# would report a 512 MB share of a 64-core node as a 64-core machine. And on a
# machine that is not a container at all, a limit that is really there is worth
# obeying anyway.
#
# $_HW_LIMITS is "none" outside a container, "known" inside one whose share of
# the machine was read - including reading that there is no limit - and
# "unknown" inside one whose share could not be. The third case is the
# dangerous one and the only one that costs a tier.
hw_probe_container() {
  _HW_CONTAINER=0
  _HW_LIMITS=none
  _HW_LIMIT_RAM_MB="$(_hw_cgroup_memory)"
  _HW_LIMIT_CORES="$(_hw_cgroup_cores)"

  if _hw_container_marked; then
    _HW_CONTAINER=1
  elif _hw_is_number "$_HW_LIMIT_RAM_MB" || _hw_is_number "$_HW_LIMIT_CORES"; then
    # A real limit is a real share of a machine, whatever is or is not written
    # in /.dockerenv.
    _HW_CONTAINER=1
  fi

  if [ "$_HW_CONTAINER" != 1 ]; then
    _HW_LIMITS=none
    return 0
  fi
  if [ "$_HW_LIMIT_RAM_MB" = unknown ] || [ "$_HW_LIMIT_CORES" = unknown ]; then
    _HW_LIMITS=unknown
  else
    _HW_LIMITS=known
  fi
  return 0
}

# ----------------------------------------------------------------- what network

# hw_probe_network -> online | offline | unknown. Always 0.
#
# Read, never dialled. Setup asks this before it offers to download anything,
# and a wizard that quietly contacts a server while working out whether it can
# contact a server has already done the thing it was checking permission for.
# A default route is the strongest claim that can honestly be made without
# sending a packet, and it is a claim about routing rather than about the
# internet - which is why the sentence the user reads never says "you are
# online", only that there is nothing obviously in the way.
hw_probe_network() {
  local line="" iface="" dest="" prefix="" seen=0
  if _hw_readable "$HW_ROUTE_FILE"; then
    seen=1
    while IFS= read -r line || [ -n "$line" ]; do
      iface="$(_hw_field 1 "$line")"
      dest="$(_hw_field 2 "$line")"
      [ "$iface" = Iface ] && continue
      if [ "$dest" = "00000000" ] && [ "$iface" != lo ]; then
        printf 'online'
        return 0
      fi
    done < "$HW_ROUTE_FILE"
  fi
  # A machine with only an IPv6 address has no entry in the table above at all,
  # and calling it offline would be a positive claim made out of having looked
  # in one place. The v6 table's default route is the all-zero destination with
  # a zero prefix length, in the first two columns, and the device it goes out
  # of is the last.
  #
  # The device is why this is not two lines shorter. A container started with
  # no network at all still has two ::/0 entries pointing at loopback - they
  # are the unreachable routes that make a send fail immediately rather than
  # hang - and reading those as a way out to the internet would suppress the
  # one warning somebody about to download a model needs.
  if _hw_readable "$HW_ROUTE6_FILE"; then
    seen=1
    while IFS= read -r line || [ -n "$line" ]; do
      dest="$(_hw_field 1 "$line")"
      prefix="$(_hw_field 2 "$line")"
      iface="$(_hw_last_word "$line")"
      [ "$iface" != lo ] || continue
      case "$dest" in
        00000000000000000000000000000000)
          if [ "$prefix" = 00 ]; then
            printf 'online'
            return 0
          fi
          ;;
      esac
    done < "$HW_ROUTE6_FILE"
  fi
  if [ "$seen" = 1 ]; then
    printf 'offline'
    return 0
  fi
  if command -v route >/dev/null 2>&1; then
    if route -n get default >/dev/null 2>&1; then
      printf 'online'
    else
      printf 'offline'
    fi
    return 0
  fi
  printf 'unknown'
  return 0
}

# ============================================================ the whole picture

# _hw_clamp <measured> <limit> -> the smaller of the two.
#
# Only ever downwards. A cgroup limit larger than the machine it is on is not
# more machine, and a limit of "none" is not a measurement at all.
_hw_clamp() {
  local measured="${1:-unknown}" limit="${2:-none}"
  _hw_is_number "$limit" || { printf '%s' "$measured"; return 0; }
  # An upper bound is not a measurement. A container that was allowed 32 GB and
  # whose /proc could not be read has not been measured at 32 GB, and answering
  # 32768 here would turn "we could not tell" - which caps the tier - into a
  # large machine, which does not.
  if ! _hw_is_number "$measured"; then
    printf 'unknown'
    return 0
  fi
  if [ "$limit" -lt "$measured" ]; then
    printf '%s' "$limit"
  else
    printf '%s' "$measured"
  fi
  return 0
}

# hw_detect - probe everything, once, and work out the verdict.
#
# Call it in your own shell before reading anything. The accessors below fall
# back to calling it, which is correct but not free: a command substitution is
# a subshell, so a cache filled inside one dies with it and the next question
# probes all over again.
hw_detect() {
  [ "$_HW_DETECTED" = 1 ] && return 0
  _HW_DETECTED=1

  platform_detect
  hw_probe_container
  _HW_CORES="$(hw_probe_cores)"
  _HW_RAM_MB="$(hw_probe_ram_mb)"
  _HW_DISK_MB="$(hw_probe_disk_mb)"
  _HW_NETWORK="$(hw_probe_network)"
  hw_probe_acceleration

  # The container correction, applied to the two figures a container lies
  # about. Free space is not one of them: df inside a container reports the
  # filesystem the container can actually write to.
  _HW_CORES="$(_hw_clamp "$_HW_CORES" "$_HW_LIMIT_CORES")"
  _HW_RAM_MB="$(_hw_clamp "$_HW_RAM_MB" "$_HW_LIMIT_RAM_MB")"

  _hw_tier_compute "$_HW_RAM_MB" "$_HW_CORES" "$_HW_DISK_MB" \
    "$_HW_ACCEL" "$_HW_VRAM_MB" "$_HW_LIMITS"
  return 0
}

# ================================================== what the other stages read
#
# All six answer the same way: the cache when this shell has probed, the state
# file when an earlier run did, and a probe of their own when neither has. The
# uniformity is the point. An accessor that answered its default instead would
# not be empty, it would be *wrong* - an empty reason means "nothing held this
# machine back", an empty card name means "there is no card", and a caller has
# no way to tell either of those from "nobody has asked yet".

# _hw_ready <state-key> - make sure there is an answer to read, one way or
# another. Status 0 when the cache is the place to read it, 1 when the state
# file is.
_hw_ready() {
  [ "$_HW_DETECTED" = 1 ] && return 0
  wizard_state_has "${1:-}" && return 1
  hw_detect
  return 0
}

# hw_acceleration -> metal | cuda | rocm | cpu. Always 0.
hw_acceleration() {
  if _hw_ready step.detect.hardware.acceleration; then
    printf '%s' "$_HW_ACCEL"
  else
    printf '%s' "$(wizard_state_get step.detect.hardware.acceleration cpu)"
  fi
  return 0
}

# hw_voice_tier -> text-only | lightweight | recommended | maximum. Always 0.
#
# Gated on the detection having happened, and deliberately not on $_HW_TIER
# being set: _hw_tier_compute is a pure calculation any caller may run to ask
# "what would a machine like this be", and a what-if must never become the
# verdict.
hw_voice_tier() {
  if _hw_ready step.detect.hardware.tier; then
    printf '%s' "$_HW_TIER"
  else
    printf '%s' "$(wizard_state_get step.detect.hardware.tier text-only)"
  fi
  return 0
}

# hw_tier_reason -> one sentence, or nothing at all when the machine reached
# everything its numbers allow. Always 0.
hw_tier_reason() {
  if _hw_ready step.detect.hardware.tier; then
    printf '%s' "$_HW_TIER_REASON"
  else
    printf '%s' "$(wizard_state_get step.detect.hardware.tier_reason "")"
  fi
  return 0
}

# hw_tier_cause -> one word naming what held the tier back, or nothing.
# For a caller that wants to branch on it rather than print it.
hw_tier_cause() {
  if _hw_ready step.detect.hardware.tier; then
    printf '%s' "$_HW_TIER_CAUSE"
  else
    printf '%s' "$(wizard_state_get step.detect.hardware.tier_cause "")"
  fi
  return 0
}

# hw_gpu_name -> the card's own name, or nothing at all. Always 0.
hw_gpu_name() {
  if _hw_ready step.detect.hardware.acceleration; then
    printf '%s' "$_HW_GPU_NAME"
  else
    printf '%s' "$(wizard_state_get step.detect.hardware.gpu "")"
  fi
  return 0
}

# hw_accel_usable - status 0 when the acceleration verdict is worth acting on.
#
# The verdict and the size are different questions, and this is where they
# meet. A two-gigabyte NVIDIA card is honestly `cuda` and is honestly no use
# for holding a listening model, so every sentence and every later stage that
# would otherwise say "your graphics card will do this" asks here first.
#
# Apple Silicon is always usable: its memory is the machine's memory, there is
# no second figure to compare, and how much of it there is has already been
# answered by the tier.
hw_accel_usable() {
  local accel="" vram=""
  if _hw_ready step.detect.hardware.acceleration; then
    accel="$_HW_ACCEL"
    vram="$_HW_VRAM_MB"
  else
    accel="$(wizard_state_get step.detect.hardware.acceleration cpu)"
    vram="$(wizard_state_get step.detect.hardware.vram_mb unknown)"
  fi
  case "$accel" in
    metal) return 0 ;;
    cuda|rocm) : ;;
    *) return 1 ;;
  esac
  _hw_is_number "$vram" || return 1
  [ "$((10#$vram))" -ge "$HW_VRAM_MB_ACCELERATED" ]
}

# hw_tier_at_least <tier> - status 0 when this machine reached that tier or
# better, 2 when the argument is not one of the four.
hw_tier_at_least() {
  local want="" have=""
  want="$(hw_tier_rank "${1:-}")" || return 2
  have="$(hw_tier_rank "$(hw_voice_tier)")" || return 1
  [ "$have" -ge "$want" ]
}

# ============================================================== the two steps

# _hw_tier_phrase - what the tier means, said to somebody who has never heard
# of a model. Never a number, never a unit: raw specifications are what
# wizard_detail is for. All four are the same grammatical shape, so that the
# stem they hang off reads as one sentence whichever one arrives.
_hw_tier_phrase() {
  case "$(hw_voice_tier)" in
    maximum)     printf 'able to do everything ayeaye offers' ;;
    recommended) printf 'comfortable with listening and typing' ;;
    lightweight) printf 'able to do a little listening, and all the typing' ;;
    *)           printf 'able to show you your agents and let you type to them' ;;
  esac
}

# _hw_graphics_state -> usable | small | unsized | none. Which of the four
# things there is to say about graphics, in one word, so that the detect item
# and the report paragraph cannot drift apart.
#
# "unsized" is its own answer rather than a kind of "small": a card whose size
# would not read is not a small card, and telling somebody their card is too
# small when what happened is that a command did not answer is a lie about
# their hardware rather than about ours.
_hw_graphics_state() {
  local vram=""
  if hw_accel_usable; then
    printf 'usable'
    return 0
  fi
  case "$(hw_acceleration)" in
    cuda|rocm) : ;;
    *) printf 'none'; return 0 ;;
  esac
  if [ "$_HW_DETECTED" = 1 ]; then
    vram="$_HW_VRAM_MB"
  else
    vram="$(wizard_state_get step.detect.hardware.vram_mb unknown)"
  fi
  if _hw_is_number "$vram"; then
    printf 'small'
  else
    printf 'unsized'
  fi
  return 0
}

_hw_detect_step() {
  local unreadable="" gpu

  hw_detect

  # Said before the items and not after them: everything below is derived from
  # what could be read, and a caveat that arrives afterwards is a caveat
  # somebody has already acted on.
  unreadable=""
  _hw_is_number "$_HW_RAM_MB"  || unreadable="$unreadable memory"
  _hw_is_number "$_HW_CORES"   || unreadable="$unreadable processors"
  _hw_is_number "$_HW_DISK_MB" || unreadable="$unreadable free-space"
  if [ -n "$unreadable" ]; then
    wizard_say "Setup could not read everything about this computer, so what"
    wizard_say "follows is the least it could be rather than a measurement."
  fi

  wizard_item "hardware" "this computer is $(_hw_tier_phrase)"

  gpu="$_HW_GPU_NAME"
  case "$(_hw_graphics_state)" in
    usable)
      if [ "$_HW_ACCEL" = metal ]; then
        wizard_item "graphics" "built into this Mac, and ayeaye can use it"
      elif [ -n "$gpu" ]; then
        wizard_item "graphics" "$gpu"
      else
        wizard_item "graphics" "a graphics card ayeaye can use"
      fi
      ;;
    small)
      if [ -n "$gpu" ]; then
        wizard_item "graphics" "$gpu, too small for the listening work"
      else
        wizard_item "graphics" "a graphics card, too small for the listening work"
      fi
      ;;
    unsized)
      if [ -n "$gpu" ]; then
        wizard_item "graphics" "$gpu, and it will not say how big it is"
      else
        wizard_item "graphics" "a graphics card that will not say how big it is"
      fi
      ;;
    *)
      wizard_item "graphics" "none that ayeaye can use; the processor does the work" ;;
  esac

  case "$_HW_NETWORK" in
    online)  : ;;
    offline) wizard_item "internet" "no way out to the internet from here right now" ;;
    *)       wizard_item "internet" "cannot tell from here" ;;
  esac

  if [ "$_HW_CONTAINER" = 1 ]; then
    wizard_item "shared" "only part of this computer is yours to use"
  fi

  # Everything raw, in one line, to the log. This is the line to paste into a
  # bug report, and the reason none of the above has a number in it.
  wizard_detail "hardware: cores=$_HW_CORES ram=${_HW_RAM_MB}MB free=${_HW_DISK_MB}MB \
accel=$_HW_ACCEL vram=${_HW_VRAM_MB}MB gpu=${_HW_GPU_NAME:-none} \
container=$_HW_CONTAINER limits=$_HW_LIMITS network=$_HW_NETWORK"
  wizard_detail "hardware: tier=$_HW_TIER cause=${_HW_TIER_CAUSE:-none} \
reason=${_HW_TIER_REASON:-none}"

  wizard_remember step.detect.hardware.acceleration "$_HW_ACCEL"
  wizard_remember step.detect.hardware.tier "$_HW_TIER"
  wizard_remember step.detect.hardware.tier_reason "$_HW_TIER_REASON"
  wizard_remember step.detect.hardware.tier_cause "$_HW_TIER_CAUSE"
  wizard_remember step.detect.hardware.ram_mb "$_HW_RAM_MB"
  wizard_remember step.detect.hardware.cores "$_HW_CORES"
  wizard_remember step.detect.hardware.disk_mb "$_HW_DISK_MB"
  wizard_remember step.detect.hardware.vram_mb "$_HW_VRAM_MB"
  wizard_remember step.detect.hardware.gpu "$_HW_GPU_NAME"
  wizard_remember step.detect.hardware.accel_usable \
    "$(hw_accel_usable && printf 1 || printf 0)"
  wizard_remember step.detect.hardware.container "$_HW_CONTAINER"
  wizard_remember step.detect.hardware.limits "$_HW_LIMITS"
  wizard_remember step.detect.hardware.network "$_HW_NETWORK"

  # A measurement that did not happen keeps the stage out of "done". The
  # verdict above is real - it is the lowest one the missing figure allows,
  # which is what not knowing earns - but recording the check as finished would
  # say this computer was looked at, and part of it was not. The label on this
  # step is worded to be read under "not finished, and worth coming back to",
  # because on a machine whose /proc cannot be read that is where it will live.
  if [ -n "$unreadable" ]; then
    wizard_blank
    wizard_say "What setup could not read:$unreadable."
    wizard_say "Nothing is broken. ayeaye will not offer anything it cannot be"
    wizard_say "sure of, and reading these again is all a later run has to do."
    return "$WIZARD_STAGE_PENDING"
  fi
  return "$WIZARD_STAGE_OK"
}

# ------------------------------------------------------------- the toolbox
#
# The flat `command -v` sweep this replaces answered "ffmpeg: missing", which
# is a true sentence that helps nobody who does not already know what ffmpeg
# is for. Every line below names the job first and the command second, and the
# jobs are grouped so that a missing piece is visibly a missing piece of one
# thing rather than an item on an undifferentiated list.
#
# The mark on a missing one is "not yet" and not "MISSING". MISSING is what
# the stage above uses for tmux and python3, which setup cannot continue
# without; six more of them for things that are all optional is how somebody
# on their first run decides the whole thing is broken.
#
# Nothing here is run. `command -v` looks a command up without executing it,
# which is the whole difference between detecting ollama and starting it.

# Every binary whisper.cpp has shipped its server and its one-shot tool under,
# newest first. The name is worth recording as well as the fact: the ticket
# that installs models has to invoke whichever one is there.
_HW_WHISPER_COMMANDS="whisper-server whisper-cli whisper-cpp whisper"
_HW_AGENT_COMMANDS="claude codex"

_hw_have() { command -v "$1" >/dev/null 2>&1; }

# _hw_report_tool <command> <what it is for> - one checklist line, and one
# state key. Status 0 when it is there.
_hw_report_tool() {
  local cmd="$1" purpose="$2"
  if _hw_have "$cmd"; then
    wizard_item ok "$purpose ($cmd)"
    wizard_remember "step.detect.tools.$cmd" 1
    return 0
  fi
  wizard_item "not yet" "$purpose ($cmd)"
  wizard_remember "step.detect.tools.$cmd" 0
  return 1
}

_hw_tools_step() {
  local cmd whisper="" agents=""

  wizard_blank
  wizard_say "For talking to your agents out loud:"
  _hw_report_tool ffmpeg "recording what you say" || true
  for cmd in $_HW_WHISPER_COMMANDS; do
    if _hw_have "$cmd"; then
      whisper="$cmd"
      break
    fi
  done
  if [ -n "$whisper" ]; then
    wizard_item ok "turning what you said into words ($whisper)"
  else
    wizard_item "not yet" "turning what you said into words (whisper)"
  fi
  wizard_remember step.detect.tools.whisper "$([ -n "$whisper" ] && printf 1 || printf 0)"
  wizard_remember step.detect.tools.whisper_command "$whisper"
  _hw_report_tool ollama "tidying up what it heard" || true

  wizard_blank
  wizard_say "For reaching this computer from your phone:"
  _hw_report_tool tailscale "a private address only you can use" || true

  wizard_blank
  for cmd in $_HW_AGENT_COMMANDS; do
    if _hw_have "$cmd"; then
      agents="${agents:+$agents }$cmd"
      wizard_remember "step.detect.tools.$cmd" 1
    else
      wizard_remember "step.detect.tools.$cmd" 0
    fi
  done
  if [ -n "$agents" ]; then
    wizard_item ok "coding agents on this computer: $agents"
  else
    # Not a failure. ayeaye shows what is running in tmux, and an agent
    # installed tomorrow appears without setup being run again.
    wizard_item note "no coding agent found yet; ayeaye will show one as soon as you start it"
  fi
  wizard_remember step.detect.tools.agents "$agents"

  if _hw_have cliban; then
    wizard_item ok "the ticket board on your phone (cliban)"
    wizard_remember step.detect.tools.cliban 1
  else
    wizard_remember step.detect.tools.cliban 0
  fi

  wizard_blank
  wizard_say "Nothing above is required. ayeaye notices each of them the moment"
  wizard_say "it is installed, so none of this has to be decided now."
  return "$WIZARD_STAGE_OK"
}

# --------------------------------------------------- what that means for you
#
# The third stage answers one question - will this work for me - and it
# answers it in the words the person asking already has. No unit, no library,
# no file name, no number. All of that went to the log in the step above.
#
# Everything here is in the conditional: "would", not "can". None of it is
# installed yet, the stage above has just finished listing what is missing,
# and the difference between "this computer can listen" and "this computer
# would be able to listen once you set it up" is the difference between a
# description and a promise. Only one of them is this stage's to make.
#
# The other half of the job is the sentence after the answer: when this
# machine is being offered less than its raw numbers suggest, it is told why
# in the same breath, because an answer with no reason attached reads as an
# arbitrary limit rather than as a measurement.

_hw_report_step() {
  local tier cause reason gpu network graphics

  tier="$(hw_voice_tier)"
  cause="$(hw_tier_cause)"
  reason="$(hw_tier_reason)"
  gpu="$(hw_gpu_name)"
  graphics="$(_hw_graphics_state)"
  network="$(wizard_state_get step.detect.hardware.network unknown)"

  wizard_blank
  wizard_say "Talking to your agents out loud, once it is set up:"
  case "$tier" in
    maximum)
      wizard_say "  yes, and with room for the best of it."
      wizard_say "  This computer has room to keep the biggest and most accurate"
      wizard_say "  listening there is ready and waiting."
      ;;
    recommended)
      wizard_say "  yes, comfortably."
      wizard_say "  This computer has room to listen well, and to tidy up what it"
      wizard_say "  heard before it reaches your agent."
      ;;
    lightweight)
      wizard_say "  yes, a sentence at a time."
      wizard_say "  This computer has room for the smallest of the listening"
      wizard_say "  models. Expect to wait a moment after each sentence."
      ;;
    *)
      wizard_say "  not as things stand."
      wizard_say "  This computer can still show you what your agents are doing and"
      wizard_say "  let you type to them from your phone. Everything except the"
      wizard_say "  talking works."
      ;;
  esac

  # Only when there is listening to do. Naming the thing that would have done
  # the work on a machine that is not going to do the work is an offer, and
  # this stage does not make offers it cannot keep.
  if [ "$tier" != text-only ]; then
    wizard_blank
    case "$graphics" in
      usable)
        if [ "$(hw_acceleration)" = metal ]; then
          wizard_say "The graphics built into this Mac would do the listening work."
        elif [ -n "$gpu" ]; then
          wizard_say "The graphics card in this computer - $gpu - would do the"
          wizard_say "listening work."
        else
          wizard_say "The graphics card in this computer would do the listening work."
        fi
        ;;
      small)
        # There is a card, it is real, and it is not big enough. Saying "there
        # is no graphics card here" to somebody who paid for one is how a tool
        # stops being believed about anything else.
        if [ -n "$gpu" ]; then
          wizard_say "There is a graphics card in this computer - $gpu - but it"
          wizard_say "is too small for the listening, so the processor would do it."
        else
          wizard_say "There is a graphics card in this computer, but it is too small"
          wizard_say "for the listening, so the processor would do it."
        fi
        wizard_say "That is slower, and it works."
        ;;
      unsized)
        if [ -n "$gpu" ]; then
          wizard_say "There is a graphics card in this computer - $gpu - but it"
          wizard_say "will not say how big it is, so the processor would do the"
          wizard_say "listening."
        else
          wizard_say "There is a graphics card in this computer, but it will not say"
          wizard_say "how big it is, so the processor would do the listening."
        fi
        wizard_say "That is slower, and it works."
        ;;
      *)
        wizard_say "There is no graphics card here that ayeaye can use for the"
        wizard_say "listening, so the processor would do it instead."
        wizard_say "That is slower, and it works."
        ;;
    esac
  fi

  # The graphics paragraph above has just said all of this, at more length and
  # in more useful words. Repeating it as a reason is how a screen stops being
  # read.
  case "$cause" in
    graphics-none|graphics-small|graphics-unknown)
      [ "$tier" = text-only ] || reason="" ;;
  esac
  if [ -n "$reason" ]; then
    wizard_blank
    wizard_say "Why not more than that: $reason."
  fi
  case "$cause" in
    *-unknown)
      wizard_say "That is the least this computer could be, not what it measured."
      ;;
  esac

  # Said here, in the stage that describes, rather than in the stage that
  # downloads: somebody about to carry a laptop to where the network is would
  # rather know now.
  if [ "$tier" != text-only ] && [ "$network" != online ]; then
    wizard_blank
    wizard_say "The part that does the listening has to be downloaded first, and"
    if [ "$network" = offline ]; then
      wizard_say "this computer does not seem to have a way out to the internet"
      wizard_say "right now. Everything else here works without it."
    else
      wizard_say "this computer will not say whether it has a way out to the"
      wizard_say "internet. Everything else here works without it."
    fi
  fi

  wizard_blank
  wizard_say "None of this is being installed now. Setup is only saying what would"
  wizard_say "work here, so that nothing is downloaded that this computer cannot use."
  wizard_detail "report: tier=$tier cause=${cause:-none} accel=$(hw_acceleration) \
graphics=$graphics network=$network"
  return "$WIZARD_STAGE_OK"
}

# The labels are written to be read twice: once beside the step as it runs, and
# once under "not finished, and worth coming back to" in the closing summary.
wizard_step detect hardware _hw_detect_step "Reading this computer's size"
wizard_step detect tools    _hw_tools_step  "What is already installed"
wizard_step report capability _hw_report_step "What talking out loud would need" required always
