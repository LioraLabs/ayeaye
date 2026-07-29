# Starting ayeaye at login: systemd user units on Linux, launchd agents on a
# Mac, and honest instructions on a machine that has neither.
#
# One description of each service, two ways of writing it down. The facts about
# a service - what it is called, what it runs, where its output goes - live in
# `_service_spec` and nowhere else; `service_render_systemd` and
# `service_render_launchd` are two spellings of the same facts. That is why the
# `@…@` templates that used to live under `systemd/user/` are gone: a unit and
# a property list saying slightly different things about the same program is
# exactly the bug this arrangement cannot have.
#
# What a definition does *not* contain is any setting. The port, the address,
# the allowed hosts and the notification URL live in the environment file and
# only there: the unit points at it with `EnvironmentFile=`, and the agent -
# launchd has no such thing - is told where it is instead. Somebody changing
# the port edits one file. A definition installed months ago can never
# disagree with the settings in force.
#
# Two halves, and they depend on different things. The registry and the two
# renderers are standalone: they read the paths below, write to standard
# output, and can be exercised without a wizard at all. The step is not - it
# is what install.sh's `step_service` hands over to, and it uses that file's
# `_manual_instructions` and its `NO_SYSTEMD` flag rather than saying the same
# things again in its own words.
#
# bash 3.2: no associative arrays, no ${var,,}, no mapfile, no local -n.

# ------------------------------------------------------------------ where things are
#
# Read at call time rather than captured when this file is sourced: a test
# drives the renderers directly, and install.sh has already set every one of
# these by the time lib/steps is loaded.

_service_repo()       { printf '%s' "${REPO:-.}"; }
_service_conf_home()  { printf '%s' "${XDG_CONFIG_HOME:-$HOME/.config}"; }
_service_state_home() { printf '%s' "${XDG_STATE_HOME:-$HOME/.local/state}"; }
_service_env_file()   { printf '%s' "${ENV_FILE:-$(_service_conf_home)/ayeaye/env}"; }
_service_unit_dir()   { printf '%s' "${UNIT_DIR:-$(_service_conf_home)/systemd/user}"; }
_service_agent_dir()  { printf '%s' "${SERVICE_AGENT_DIR:-$HOME/Library/LaunchAgents}"; }
_service_log_dir()    { printf '%s' "${SERVICE_LOG_DIR:-$HOME/Library/Logs/ayeaye}"; }

# What a launchd agent is given as its PATH. Both Homebrew prefixes - arm64
# and Intel - and then the four directories launchd would have supplied on its
# own. A fixed list rather than this machine's PATH on purpose: a definition
# is compared against a golden file, and a copy of whatever happened to be
# exported the day setup ran is not a description of the machine.
SERVICE_LAUNCHD_PATH="${SERVICE_LAUNCHD_PATH:-/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin}"

# The reverse-DNS prefix a launchd label carries. lib/service.sh owns the
# default; naming it here as well would be a second answer to one question.
_service_label() { printf '%s.%s' "$PLATFORM_LAUNCHD_PREFIX" "${1:-}"; }

# ---------------------------------------------------------------- the registry
#
# _service_spec <name> <field> - one fact about one service. 2 for a service
# this project does not have, or a field it does not know, so that a typo is a
# caller bug rather than an empty unit file that installs cleanly and starts
# nothing.
#
#   title   the one-line Description a person reads in `systemctl status`
#   after   what has to be up first, on systemd. launchd has no equivalent
#           and does not want one: an agent is started when the user logs in.
#   argv    the program and its arguments, one per line, absolute. One per
#           line because a path may contain a space and this is the only
#           representation both formats can be built from without guessing
#           where one argument ends.
_service_spec() {
  local name="${1:-}" field="${2:-}" repo
  repo="$(_service_repo)"

  case "$name" in
    ayeaye)
      case "$field" in
        title) printf 'voice remote for tmux (phone web UI)' ;;
        # Nothing. The unit used to order itself After=network.target, which
        # is a *system* target: a user manager has never heard of it, so the
        # line was an ordering guarantee that did not exist. ayeaye binds when
        # it starts and the restart policy covers the rest.
        after) printf '' ;;
        argv)  printf '%s/bin/ayeaye\n' "$repo" ;;
        *)     return 2 ;;
      esac
      ;;
    voice-agent)
      case "$field" in
        title) printf 'voice-dictate mic recorder (local, for M-v when attached locally)' ;;
        after) printf 'graphical-session.target' ;;
        argv)  printf '%s/bin/voice-agent\n' "$repo" ;;
        *)     return 2 ;;
      esac
      ;;
    whisper-server)
      case "$field" in
        title) printf 'whisper.cpp server, kept resident for dictation' ;;
        after) printf '' ;;
        argv)
          # Not the binary directly. whisper-server takes its model, its
          # address and its thread count as command-line arguments, and
          # baking this machine's values into the definition would put the
          # settings in two places - the one thing every service here is
          # arranged to avoid. So the program is a short shell that reads the
          # settings file at the moment the service starts, and the port is
          # changed by editing that file and restarting, exactly as it is for
          # ayeaye.
          printf '%s\n' "$SERVICE_SH"
          printf -- '-c\n'
          printf '%s\n' "$_SERVICE_WHISPER_SCRIPT"
          printf 'ayeaye-whisper\n'
          printf '%s\n' "$(_service_env_file)"
          printf '%s\n' "$(_service_whisper_bin)"
          ;;
        *)     return 2 ;;
      esac
      ;;
    *)
      return 2
      ;;
  esac
  return 0
}

# service_names - every service this file knows how to write down.
service_names() {
  printf 'ayeaye\nvoice-agent\nwhisper-server\n'
}

# Where whisper.cpp's server is on this machine, if it is anywhere.
# Overridable so a test can pin what a definition would say without needing a
# real binary.
_service_whisper_bin() {
  if [ -n "${SERVICE_WHISPER_BIN:-}" ]; then
    printf '%s' "$SERVICE_WHISPER_BIN"
  else
    command -v whisper-server 2>/dev/null || true
  fi
}

SERVICE_SH="${SERVICE_SH:-/bin/sh}"

# Set by the step: 1 when a whisper definition was just written, 0 when one was
# already correct, empty when there is none. Given a value at file scope so
# that reading it is safe under `set -u` whatever path the step took.
_SERVICE_WHISPER_READY="${_SERVICE_WHISPER_READY:-}"

# The program the whisper service really runs: read the settings file, then
# become whisper-server with what it said.
#
# Written out here rather than shipped as a script of its own, because a
# generated definition that depends on a second generated file has twice as
# many ways to be half-installed. Read with a `case` loop rather than by
# sourcing, because sourcing would execute whatever is in the file: systemd's
# EnvironmentFile does not execute its input, and neither does this, so a
# value means the same thing on both platforms.
#
# 78 is EX_CONFIG, which says to anybody reading the log that the service is
# not broken, it is unconfigured.
_SERVICE_WHISPER_SCRIPT='E="$1"; B="$2"; M=""; S=""; T=""; if [ -r "$E" ]; then while IFS= read -r L || [ -n "$L" ]; do case "$L" in VOICE_WHISPER_MODEL=*) M="${L#*=}" ;; VOICE_WHISPER_SERVER=*) S="${L#*=}" ;; VOICE_WHISPER_THREADS=*) T="${L#*=}" ;; esac; done < "$E"; fi; M="${M#\"}"; M="${M%\"}"; S="${S#\"}"; S="${S%\"}"; T="${T#\"}"; T="${T%\"}"; [ -n "$S" ] || S=127.0.0.1:8910; [ -n "$T" ] || T=8; if [ -z "$M" ]; then echo "set VOICE_WHISPER_MODEL in $E, then restart this service" >&2; exit 78; fi; exec "$B" --model "$M" --host "${S%%:*}" --port "${S##*:}" --threads "$T" --language en'

# ------------------------------------------------------------------- quoting
#
# Each format mangles a path in its own way, and both of them are silent about
# it: a unit with a split ExecStart installs cleanly and never starts, and a
# plist with a bare ampersand in it is not XML at all.

# _service_systemd_arg <arg> - one ExecStart argument, as systemd parses it.
#
# Left bare when it is made only of characters systemd treats as themselves,
# which keeps the ordinary case readable. Otherwise double-quoted, and inside
# those quotes a backslash, a double quote, a dollar (systemd expands $NAME
# there) and a percent (systemd's own specifiers, %h and friends) all have to
# be doubled or escaped to arrive at execve intact.
_service_systemd_arg() {
  local arg="${1:-}" out
  case "$arg" in
    "") printf '""'; return 0 ;;
    *[!A-Za-z0-9_./:=@+,-]*) ;;
    *) printf '%s' "$arg"; return 0 ;;
  esac
  out="$arg"
  out="${out//\\/\\\\}"
  out="${out//\"/\\\"}"
  out="${out//\$/\$\$}"
  out="${out//%/%%}"
  printf '"%s"' "$out"
}

# _service_xml <text> - text safe to put between two XML tags.
#
# sed, and not ${var//&/&amp;}, because that expansion cannot be written to
# mean the same thing in both shells this project runs under. bash 5 reads an
# unescaped & in the replacement as "the text that matched" and bash 3.2 - the
# one macOS ships, and the only platform that reads the output of this
# function - does not, so every spelling is wrong in one of them:
#
#   ${o//</&lt;}    bash 5: <lt;      bash 3.2: &lt;
#   ${o//</\&lt;}   bash 5: &lt;      bash 3.2: \&lt;
#
# The second is what a Mac would have installed: still well-formed XML, so
# plistlib is happy, and an agent pointing at a path with a stray backslash in
# it that does not exist.
_service_xml() {
  printf '%s' "${1:-}" | sed -e 's/&/\&amp;/g' -e 's/</\&lt;/g' -e 's/>/\&gt;/g'
}

# _service_systemd_value <text> - a directive value that is not a command.
#
# systemd runs specifier expansion (%h, %i, %%…) over EnvironmentFile= just as
# it does over ExecStart=, and an unrecognised one is not an error: the whole
# directive is dropped with a warning nobody reads. A settings file under a
# path containing a percent sign would leave a unit that starts perfectly and
# runs with none of the user's settings.
_service_systemd_value() {
  local out="${1:-}"
  out="${out//%/%%}"
  printf '%s' "$out"
}

# _service_systemd_can_express <path>… - status 0 when systemd can name all of
# them in a unit.
#
# There is no escaping for these. systemd unquotes ExecStart and then rejects
# the result if it contains a quote, a backslash or a control character -
# "Executable path contains special characters", a fatal unit error - so the
# only honest answers are to say so or to write a unit that can never start.
_service_systemd_can_express() {
  local path
  for path in "$@"; do
    case "$path" in
      *\"*|*\'*|*\\*|*"
"*) return 1 ;;
    esac
  done
  return 0
}

# _service_exec_line <name> - the whole ExecStart value.
_service_exec_line() {
  local name="${1:-}" arg out="" first=1
  while IFS= read -r arg || [ -n "$arg" ]; do
    [ "$first" = 1 ] || out="$out "
    first=0
    out="$out$(_service_systemd_arg "$arg")"
  done <<EOF
$(_service_spec "$name" argv)
EOF
  printf '%s' "$out"
}

# ----------------------------------------------------------------- renderers
#
# Both write to standard output and touch nothing else, so a test can compare
# what would be installed against a golden file without installing anything.

# service_render_systemd <name>
service_render_systemd() {
  local name="${1:-}" title after
  title="$(_service_spec "$name" title)" || return 2
  after="$(_service_spec "$name" after)" || return 2
  cat <<UNIT
[Unit]
Description=$title
UNIT
  [ -n "$after" ] && printf 'After=%s\n' "$after"
  cat <<UNIT

[Service]
Type=simple
# Generated by setup, and replaced whole every time it runs. Nothing is
# configured here: every setting lives in the environment file below. Edit
# that, then \`systemctl --user restart $name\`.
#
# The leading "-" means a settings file that is not there yet is not a reason
# to refuse to start: ayeaye has a default for everything in it, and a restart
# loop would be a worse answer than running on the defaults.
EnvironmentFile=-$(_service_systemd_value "$(_service_env_file)")
ExecStart=$(_service_exec_line "$name")
Restart=on-failure
RestartSec=5

[Install]
WantedBy=default.target
UNIT
  return 0
}

# service_render_launchd <name>
#
# The differences from the unit above are forced by the platform and are not
# choices.
#
# launchd has no EnvironmentFile, so the agent is handed the location of the
# settings - XDG_CONFIG_HOME and XDG_STATE_HOME, which is what bin/ayeaye
# reads that same file through - rather than any value out of it.
#
# It also has no journal, so the agent names the file it writes and the step
# tells the person that path.
#
# And it starts an agent with almost no PATH: /usr/bin:/bin:/usr/sbin:/sbin,
# which on a Mac does not include tmux. Homebrew is where tmux comes from
# there, under one of two prefixes depending on the architecture, and both are
# named. Without this the agent starts, answers, passes its health check, and
# shows an empty list of sessions forever - bin/ayeaye treats a tmux that
# cannot be run as a tmux with nothing in it.
service_render_launchd() {
  local name="${1:-}" arg logs
  _service_spec "$name" title >/dev/null || return 2
  logs="$(_service_log_dir)"
  cat <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <!-- Generated by setup, and replaced whole every time it runs. Nothing is
       configured here: every setting lives in the environment file this
       agent reads. Edit that, then restart the agent. -->
  <key>Label</key>
  <string>$(_service_xml "$(_service_label "$name")")</string>
  <key>ProgramArguments</key>
  <array>
PLIST
  while IFS= read -r arg || [ -n "$arg" ]; do
    printf '    <string>%s</string>\n' "$(_service_xml "$arg")"
  done <<EOF
$(_service_spec "$name" argv)
EOF
  cat <<PLIST
  </array>
  <key>EnvironmentVariables</key>
  <dict>
    <key>PATH</key>
    <string>$(_service_xml "$SERVICE_LAUNCHD_PATH")</string>
    <key>XDG_CONFIG_HOME</key>
    <string>$(_service_xml "$(_service_conf_home)")</string>
    <key>XDG_STATE_HOME</key>
    <string>$(_service_xml "$(_service_state_home)")</string>
  </dict>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <dict>
    <key>SuccessfulExit</key>
    <false/>
  </dict>
  <key>ThrottleInterval</key>
  <integer>5</integer>
  <key>StandardOutPath</key>
  <string>$(_service_xml "$logs/$name.log")</string>
  <key>StandardErrorPath</key>
  <string>$(_service_xml "$logs/$name.log")</string>
</dict>
</plist>
PLIST
  return 0
}

# --------------------------------------------------------------- where it lands

# service_definition_path <name> <systemd|launchd>
service_definition_path() {
  local name="${1:-}" kind="${2:-}"
  [ -n "$name" ] || return 2
  case "$kind" in
    systemd) printf '%s/%s.service' "$(_service_unit_dir)" "$name" ;;
    launchd) printf '%s/%s.plist' "$(_service_agent_dir)" "$(_service_label "$name")" ;;
    *) return 2 ;;
  esac
  return 0
}

# ------------------------------------------------------------------- the plan
#
# Stage four, so that stage five can say a service is about to be installed
# before it is. `service` is a plan category that is shown and does not gate:
# a user unit of one's own is not the kind of thing a person should have to
# stop and approve.
_service_plan_step() {
  local kind
  if [ "${NO_SYSTEMD:-0}" = 1 ]; then
    return "$WIZARD_STAGE_SKIP"
  fi
  kind="$(platform_service_manager)"
  case "$kind" in
    systemd)
      wizard_plan_add service "ayeaye will start when you log in, and restart if it stops"
      ;;
    launchd)
      wizard_plan_add service "ayeaye will start when you log in, and restart if it stops"
      ;;
    *)
      # Nothing to promise. The service step will say so in full when it gets
      # there; saying it twice would be worse than saying it once.
      return "$WIZARD_STAGE_SKIP"
      ;;
  esac
  return "$WIZARD_STAGE_OK"
}

wizard_step configure service _service_plan_step "How ayeaye will start"

# =============================================================== installing it
#
# What install.sh's `step_service` hands over to. One step, three outcomes:
# a systemd user unit, a launchd agent, or written instructions for running it
# by hand - and that last one is a supported way to use this program, not a
# failure.

# _service_same <a> <b> - status 0 when two files hold the same bytes.
#
# cksum rather than cmp: the test harness's PATH is exactly the stubs it was
# given, cmp is not among the coreutils it links in by default, and a
# comparison that silently answered "different" would rewrite a file on every
# run and back it up every time.
_service_same() {
  local a b
  a="$(cksum < "${1:-}" 2>/dev/null)" || return 1
  b="$(cksum < "${2:-}" 2>/dev/null)" || return 1
  [ "$a" = "$b" ]
}

# _service_write_definition <name> <systemd|launchd>
#
# 0 when the definition on disk is now the generated one, 1 when it could not
# be. Sets SERVICE_WROTE to 1 when the file actually changed.
#
# Every step of it is checked. A step body runs with errexit suspended, so an
# unchecked redirection into a directory that cannot be written would leave
# this saying "installed" about a file that was never created.
_service_write_definition() {
  # backup and rendered are given values rather than merely declared: install.sh
  # runs under `set -u`, where reading a declared-but-unset local ends the
  # whole run rather than the step.
  local name="${1:-}" kind="${2:-}" path dir tmp backup="" rendered=""
  SERVICE_WROTE=0
  if ! path="$(service_definition_path "$name" "$kind")"; then
    wizard_say "setup does not know how to write down a service called \"$name\"."
    return 1
  fi
  dir="${path%/*}"

  # Rendered into memory first, so that a name this file does not have and a
  # directory that cannot be written are told apart. They are two different
  # problems and only one of them is about the disk.
  if ! rendered="$("service_render_$kind" "$name" 2>/dev/null)"; then
    wizard_say "setup does not know how to write down a service called \"$name\"."
    return 1
  fi

  # Compared before anything is created, and in memory. Leaving an unchanged
  # file alone is not an optimisation, it is the difference between repairing
  # a service and disturbing one: no new timestamp, and no backup of a file
  # identical to its replacement. Doing the comparison without a temporary
  # file also means a directory somebody has made read-only, holding a
  # definition that is already correct, is not reported as a failure.
  if [ -f "$path" ] \
     && [ "$(printf '%s\n' "$rendered" | cksum)" = "$(cksum < "$path" 2>/dev/null)" ]; then
    return 0
  fi

  if ! mkdir -p "$dir" 2>/dev/null; then
    wizard_say "cannot create $dir, so $name cannot be started for you."
    return 1
  fi
  tmp="$path.tmp.$$"
  if ! printf '%s\n' "$rendered" > "$tmp" 2>/dev/null; then
    rm -f "$tmp" 2>/dev/null
    wizard_say "cannot write into $dir, so $name cannot be started for you."
    return 1
  fi

  # A definition that is there and different is somebody's - an older version
  # of this program wrote it, or a person edited it. It is a generated file
  # and this run is going to replace it, which is a decision taken when they
  # asked for ayeaye to be set up rather than one worth stopping for; but the
  # bytes are cheap to keep, and a copy that could not be made is a reason to
  # leave the file alone rather than a detail.
  if [ -f "$path" ]; then
    if ! backup="$(wizard_backup "$path")"; then
      rm -f "$tmp" 2>/dev/null
      wizard_say "could not save a copy of $path, so it has been left alone."
      return 1
    fi
    wizard_say "saved a copy of the previous $name definition at $backup"
  fi

  chmod 644 "$tmp" 2>/dev/null
  if ! mv -f "$tmp" "$path" 2>/dev/null; then
    rm -f "$tmp" 2>/dev/null
    wizard_say "cannot write into $dir, so $name cannot be started for you."
    return 1
  fi
  # Recorded after the fact, not before it: a ledger saying a file was
  # replaced when it was not is worse than a ledger that is a line shorter.
  # This is the documented way to put a decision in the trail that was settled
  # by an answer given earlier - asking to set ayeaye up - rather than now.
  [ -n "$backup" ] && \
    wizard_consent_record replace granted "replaced the service definition at $path"
  SERVICE_WROTE=1
  return 0
}

# _service_migrate_siblings <kind> - regenerate a definition somebody already
# has, and install nothing new.
#
# voice-agent is the case this exists for: its unit used to be a template a
# person copied by hand, and that template is gone. Somebody who copied it
# still has the file, and a rerun of setup should leave them with a correct
# one rather than a stale one naming a path the repository has since moved
# from. What it will not do is install a service nobody asked for: a
# definition that is not already there is not written.
_service_migrate_siblings() {
  local kind="${1:-}" name path running
  for name in $(service_names); do
    [ "$name" = ayeaye ] && continue
    path="$(service_definition_path "$name" "$kind")" || continue
    [ -f "$path" ] || continue
    running=0
    _service_is_running "$name" && running=1
    if _service_write_definition "$name" "$kind" && [ "$SERVICE_WROTE" = 1 ]; then
      wizard_say "brought your $name definition up to date as well"
      # And made it take effect, for the same reason ayeaye's does: a running
      # process does not re-read the file it was started from, so a rewritten
      # definition nobody restarts is a definition that describes something
      # other than what is running.
      if [ "$running" = 1 ] && _service_restart "$name" "$kind"; then
        wizard_say "and restarted it, since it was running"
      fi
    fi
  done
  # Never the reason a run fails: this is somebody else's service and setup is
  # only being tidy about it.
  SERVICE_WROTE=0
  return 0
}

# _service_is_running <name> - status 0 when the service manager says the
# service is up, or - on launchd, which has no finer answer - that it is
# registered. Quiet: this is a probe, and its output is noise.
_service_is_running() {
  local cmd
  cmd="$(platform_service_command "${1:-}" status)" || return 1
  eval "$cmd" >/dev/null 2>&1
}

# _service_restart <name> <kind> - make a rewritten definition take effect.
#
# Two platforms, two spellings of one idea. systemd stops and starts, which
# lib/service.sh defines as the pair that leaves enablement alone. launchd has
# no restart at all: a property list is re-read by being handed to launchd
# again, and a label it already knows has to be booted out first or the
# bootstrap fails outright.
_service_restart() {
  local name="${1:-}" kind="${2:-}" cmd path
  if [ "$kind" = launchd ]; then
    _service_unregister_launchd "$name"
    path="$(service_definition_path "$name" launchd)" || return 1
    cmd="$(platform_service_command "$name" enable "$path")" || return 1
    _service_run "$cmd"
    return $?
  fi
  cmd="$(platform_service_command "$name" stop)" && _service_run "$cmd"
  cmd="$(platform_service_command "$name" start)" || return 1
  _service_run "$cmd"
}

# _service_run <question-free command> - run a command this layer built, with
# its output kept for the log and, when it fails, put in front of the reader.
#
# At the moment something fails, the service manager's own words are worth
# more to whoever has to fix it than any paraphrase of them.
_service_run() {
  local cmd="${1:-}" out
  wizard_detail "running: $cmd"
  if out="$(eval "$cmd" 2>&1)"; then
    [ -n "$out" ] && wizard_detail "$out"
    return 0
  fi
  wizard_detail "$out"
  [ -n "$out" ] && printf '%s\n' "$out" >&2
  return 1
}

# ------------------------------------------------------------------- systemd

_service_install_systemd() {
  local cmd path changed running=0
  path="$(service_definition_path ayeaye systemd)"

  # There is no escaping that makes these work, so the only honest thing left
  # is to say which character it is and offer the way that does work. PENDING,
  # not FAIL: everything else about the install succeeded, and this is waiting
  # on somebody moving a folder rather than on setup trying again.
  if ! _service_systemd_can_express "$(_service_repo)" "$(_service_env_file)"; then
    wizard_say "ayeaye is in a folder whose name has a quote, a backslash or a"
    wizard_say "line break in it, and systemd cannot name such a folder in a"
    wizard_say "service. Move it somewhere with a simpler name and run this again."
    wizard_detail "unrepresentable in a unit: $(_service_repo)"
    _service_manual_fallback
    return "$WIZARD_STAGE_PENDING"
  fi

  # Asked before the file is replaced, because afterwards there is no way to
  # tell "it was already running" from "we just wrote this".
  _service_is_running ayeaye && running=1

  _service_write_definition ayeaye systemd || return "$WIZARD_STAGE_FAIL"
  changed="$SERVICE_WROTE"
  wizard_say "installed $path"
  wizard_say "(regenerated on every run; put settings in $(_service_env_file), not the unit)"

  _service_migrate_siblings systemd
  _service_install_whisper systemd

  if cmd="$(platform_service_command ayeaye reload)"; then
    if ! _service_run "$cmd"; then
      wizard_say "this computer's service manager would not reload its files, so"
      wizard_say "ayeaye cannot be started for you."
      return "$WIZARD_STAGE_FAIL"
    fi
  fi

  # A running service does not pick up a rewritten unit by itself, and
  # `enable --now` on something already active does nothing at all - so
  # without this, moving the repository and running setup again would report
  # success while the old process carried on from the old path. This is the
  # whole of what "repair" means here, and it is the same code as an install.
  if [ "$running" = 1 ]; then
    if [ "$changed" = 1 ]; then
      if _service_restart ayeaye systemd; then
        wizard_say "ayeaye was already running, so it has been restarted on the new"
        wizard_say "definition."
      else
        wizard_say "ayeaye was already running and would not start again."
        return "$WIZARD_STAGE_FAIL"
      fi
    else
      wizard_say "ayeaye was already running, and its definition has not changed."
    fi
    wizard_remember step.service.unit.started 1

    # Running is not the same as starting at login, and stop-then-start is
    # deliberately the pair that leaves enablement alone. A unit somebody
    # started by hand is running and not enabled, and this stage has already
    # promised that ayeaye will come back when they log in - so make that
    # true rather than assume it. `enable --now` on a unit that is already
    # both is a no-op, which is why it is safe to run unconditionally.
    if cmd="$(platform_service_command ayeaye enable)"; then
      if ! _service_run "$cmd"; then
        wizard_say "ayeaye is running, but this computer would not agree to start it"
        wizard_say "again when you log in. It will need starting by hand until that"
        wizard_say "is sorted out."
        wizard_say "to read its log: journalctl --user -u ayeaye -f"
        _service_linger_hint
        return "$WIZARD_STAGE_PENDING"
      fi
    fi
  else
    _service_enable systemd || return $?
  fi

  _service_enable_whisper systemd
  wizard_say "to read its log: journalctl --user -u ayeaye -f"
  _service_linger_hint
  return "$WIZARD_STAGE_OK"
}

# The one thing about a Linux user service that surprises everybody: it is
# tied to the login session, so closing the last one stops it. Worth saying in
# words rather than leaving as a command to paste.
_service_linger_hint() {
  local who
  # $USER is not exported on every machine - a container shell does not have
  # it - and reading it bare under `set -u` used to end the run right here,
  # after the unit was already installed.
  who="${USER:-}"
  [ -n "$who" ] || who="$(id -un 2>/dev/null || true)"
  [ -n "$who" ] || return 0
  command -v loginctl >/dev/null 2>&1 || return 0
  [ "$(loginctl show-user "$who" -P Linger 2>/dev/null || true)" = "yes" ] && return 0
  wizard_say "recommended: loginctl enable-linger $who"
  wizard_say "(without it, user services stop when your last session ends)"
  return 0
}

# ------------------------------------------------------------------- launchd

_service_install_launchd() {
  local path logs cmd changed registered=0
  path="$(service_definition_path ayeaye launchd)"
  logs="$(_service_log_dir)"

  # The agent names the files it writes, and launchd refuses to start a job
  # whose StandardOutPath cannot be opened. Checked rather than hoped for:
  # this is the difference between an agent that runs and one that is
  # installed, reported as installed, and never starts.
  if ! mkdir -p "$logs" 2>/dev/null; then
    wizard_say "cannot create $logs, so ayeaye cannot be started for you."
    return "$WIZARD_STAGE_FAIL"
  fi

  _service_is_running ayeaye && registered=1

  _service_write_definition ayeaye launchd || return "$WIZARD_STAGE_FAIL"
  changed="$SERVICE_WROTE"
  wizard_say "installed $path"
  wizard_say "(regenerated on every run; put settings in $(_service_env_file), not the agent)"

  _service_migrate_siblings launchd
  _service_install_whisper launchd

  # There is no daemon-reload here and this does not invent one: launchd
  # re-reads a property list by being handed it again, which is what
  # bootstrapping does. lib/service.sh says so by answering "this machine
  # cannot" to reload.
  if [ "$registered" = 1 ]; then
    if [ "$changed" = 1 ]; then
      if _service_restart ayeaye launchd; then
        wizard_say "ayeaye was already set up to start when you log in, so it has"
        wizard_say "been restarted on the new definition."
      else
        wizard_say "ayeaye was already registered and would not start again."
        return "$WIZARD_STAGE_FAIL"
      fi
    else
      # "Registered", and deliberately not "running": launchctl print answers
      # for a job that is loaded, which includes one that has crashed or
      # exited, and there is no cheap way from here to tell those apart.
      # Claiming it is up is the health check's business, one step later.
      wizard_say "ayeaye is already set up to start when you log in."
    fi
    wizard_remember step.service.unit.started 1
  else
    _service_enable launchd || return $?
  fi

  _service_enable_whisper launchd
  wizard_say "to read its log: tail -f $logs/ayeaye.log"
  wizard_say "to remove it later: launchctl bootout gui/$(platform_uid)/$(_service_label ayeaye),"
  wizard_say "then delete $path"
  return "$WIZARD_STAGE_OK"
}

# _service_unregister_launchd <name> - bootout, but only if it is registered.
#
# Bootstrapping a label launchd already knows fails outright, which would turn
# every second run into an error and make repairing a broken agent impossible.
# The status probe is what tells the two cases apart.
_service_unregister_launchd() {
  local name="${1:-}" cmd
  cmd="$(platform_service_command "$name" status)" || return 0
  eval "$cmd" >/dev/null 2>&1 || return 0
  cmd="$(platform_service_command "$name" disable)" || return 0
  wizard_detail "running: $cmd"
  eval "$cmd" >/dev/null 2>&1 || true
  return 0
}

# ------------------------------------------------------- starting it, or not

_service_enable() {
  local kind="${1:-}" cmd path uid=""
  path="$(service_definition_path ayeaye "$kind")"

  if [ "${WIZARD_INTERACTIVE:-1}" = 1 ] && wizard_confirm "enable and start it now?" "y"; then
    [ "$kind" = launchd ] && _service_unregister_launchd ayeaye
    if ! cmd="$(platform_service_command ayeaye enable "$path")"; then
      wizard_say "this computer cannot be asked to start ayeaye for you."
      return "$WIZARD_STAGE_FAIL"
    fi
    if _service_run "$cmd"; then
      wizard_remember step.service.unit.started 1
      if [ "$kind" = systemd ]; then
        wizard_say "started. status: systemctl --user status ayeaye"
      else
        uid="$(platform_uid)"
        wizard_say "started. status: launchctl print gui/$uid/$(_service_label ayeaye)"
      fi
      return 0
    fi
    wizard_say "ayeaye is installed but would not start."
    return "$WIZARD_STAGE_FAIL"
  fi

  wizard_remember step.service.unit.started 0
  if [ "$kind" = systemd ]; then
    wizard_say "start it with: systemctl --user enable --now ayeaye"
  else
    # Built by the platform layer rather than written out here, so the command
    # somebody is told to paste is the same one setup would have run - quoting
    # of the path included.
    if cmd="$(platform_service_command ayeaye enable "$path")"; then
      wizard_say "start it with: $cmd"
    fi
  fi
  return 0
}

# ------------------------------------------------------ nothing to install into

# The documented manual mode. Not an apology and not a failure: ayeaye runs
# perfectly well started by hand, and a machine with no user service manager -
# a container, a stripped-down Linux, a Mac with launchctl missing - is a
# machine where saying so plainly is the whole of the right answer.
_service_manual_fallback() {
  _manual_instructions
  wizard_say "its log is whatever the terminal you start it in shows."
  wizard_remember step.service.unit.started 0
  return 0
}

# ---------------------------------------------------------------- the step

# service_step - what install.sh's step_service calls.
#
# SKIP rather than OK on the manual path, and the distinction is the point.
# OK would mean a service was installed, and the closing summary would tell
# somebody their program starts when they log in on a machine where nothing
# does. SKIP is the wizard's word for "does not apply here" - finished
# business, nothing outstanding, and a run that ends successfully.
service_step() {
  local kind
  # Nothing has been started yet, and saying so first makes that structural
  # rather than a property of having remembered to say it on every one of the
  # paths below - including the ones that give up partway. A run that fails
  # after the service was stopped must not leave a "1" from last week for the
  # health check to believe.
  wizard_remember step.service.unit.started 0
  _SERVICE_WHISPER_READY=""
  kind="$(platform_service_manager)"

  if [ "${NO_SYSTEMD:-0}" = 1 ]; then
    wizard_say "skipping systemd (--no-systemd)"
    _service_manual_fallback
    return "$WIZARD_STAGE_SKIP"
  fi

  case "$kind" in
    systemd)
      _service_install_systemd
      return $?
      ;;
    launchd)
      _service_install_launchd
      return $?
      ;;
    *)
      if [ "$(platform_os)" = "macos" ]; then
        wizard_say "no launchd session detected"
      else
        wizard_say "no systemd user session detected"
      fi
      _service_manual_fallback
      return "$WIZARD_STAGE_SKIP"
      ;;
  esac
}

# _service_install_whisper <kind> - the transcription service, when there is
# one to install.
#
# Only when this machine really has whisper.cpp's server and the settings file
# really names a model. A definition pointing at a binary that is not there
# would fail at every login and leave a red line in somebody's status output
# forever, which is a worse answer than not writing one.
#
# It is never enabled without being asked, and an unattended run does not
# enable it: whether a GPU-resident transcription server runs all the time is
# a decision, not a detail.
_service_install_whisper() {
  local kind="${1:-}" bin model path cmd
  bin="$(_service_whisper_bin)"
  [ -n "$bin" ] || return 0

  model="$(wizard_env_get "$(_service_env_file)" VOICE_WHISPER_MODEL "")"
  if [ -z "$model" ]; then
    wizard_say "this computer has whisper, but your settings do not say where the"
    wizard_say "model is, so it has not been set up to start on its own. Put its"
    wizard_say "location in VOICE_WHISPER_MODEL in $(_service_env_file), then run"
    wizard_say "this again."
    return 0
  fi

  if [ "$kind" = systemd ] && ! _service_systemd_can_express "$bin"; then
    wizard_detail "whisper-server is at a path systemd cannot name: $bin"
    return 0
  fi

  _service_write_definition whisper-server "$kind" || return 0
  path="$(service_definition_path whisper-server "$kind")"
  wizard_say "installed $path"
  wizard_say "(it reads the model, the address and the thread count from your"
  wizard_say "settings when it starts, so changing them means editing that file"
  wizard_say "and restarting it)"

  _SERVICE_WHISPER_READY="$SERVICE_WROTE"
  return 0
}

# _service_enable_whisper <kind> - asked after ayeaye's own question, not
# before it. Whichever order the files are written in, the main thing setup
# was asked for is the thing a person should be asked about first.
_service_enable_whisper() {
  local kind="${1:-}" path cmd
  [ -n "${_SERVICE_WHISPER_READY:-}" ] || return 0
  path="$(service_definition_path whisper-server "$kind")" || return 0
  [ -f "$path" ] || return 0

  if _service_is_running whisper-server; then
    if [ "$_SERVICE_WHISPER_READY" = 1 ] && _service_restart whisper-server "$kind"; then
      wizard_say "and restarted whisper, since it was running"
    fi
    return 0
  fi
  if [ "${WIZARD_INTERACTIVE:-1}" = 1 ] \
     && wizard_confirm "keep the transcription model loaded and ready?" "y"; then
    if cmd="$(platform_service_command whisper-server enable "$path")"; then
      _service_run "$cmd" || wizard_say "whisper is installed but would not start."
    fi
  fi
  return 0
}
