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
        after) printf 'network.target' ;;
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
    *)
      return 2
      ;;
  esac
  return 0
}

# service_names - every service this file knows how to write down.
service_names() {
  printf 'ayeaye\nvoice-agent\n'
}

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
_service_xml() {
  local out="${1:-}"
  out="${out//&/\&amp;}"
  out="${out//</\&lt;}"
  out="${out//>/\&gt;}"
  printf '%s' "$out"
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
After=$after

[Service]
Type=simple
# Generated by setup, and replaced whole every time it runs. Nothing is
# configured here: every setting lives in the environment file below. Edit
# that, then \`systemctl --user restart $name\`.
EnvironmentFile=$(_service_env_file)
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
# The two differences from the unit above are forced by the platform and are
# not choices. launchd has no EnvironmentFile, so the agent is handed the
# location of the settings - XDG_CONFIG_HOME and XDG_STATE_HOME, which is what
# bin/ayeaye reads the file through - rather than any value out of it. And
# launchd has no journal, so the agent says where its output goes and the step
# tells the person that path.
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
  local name="${1:-}" kind="${2:-}" path dir tmp backup
  SERVICE_WROTE=0
  path="$(service_definition_path "$name" "$kind")" || return 1
  dir="${path%/*}"

  if ! mkdir -p "$dir" 2>/dev/null; then
    wizard_say "cannot create $dir, so ayeaye cannot be started for you."
    return 1
  fi
  tmp="$path.tmp.$$"
  if ! "service_render_$kind" "$name" > "$tmp" 2>/dev/null; then
    rm -f "$tmp" 2>/dev/null
    wizard_say "cannot write into $dir, so ayeaye cannot be started for you."
    return 1
  fi

  # Already exactly right. Leaving the file alone is not an optimisation, it
  # is the difference between repairing a service and disturbing one: no new
  # timestamp, and no backup of a file that is identical to its replacement.
  if [ -f "$path" ] && _service_same "$path" "$tmp"; then
    rm -f "$tmp" 2>/dev/null
    return 0
  fi

  # A definition that is there and different is somebody's - an older version
  # of this program wrote it, or a person edited it. It is a generated file
  # and it is going to be replaced, but the bytes are cheap to keep and the
  # only copy of a hand edit is worth more than the space.
  if [ -f "$path" ]; then
    if backup="$(wizard_backup "$path")"; then
      wizard_say "saved a copy of the previous $name definition at $backup"
    fi
  fi

  chmod 644 "$tmp" 2>/dev/null
  if ! mv -f "$tmp" "$path" 2>/dev/null; then
    rm -f "$tmp" 2>/dev/null
    wizard_say "cannot write into $dir, so ayeaye cannot be started for you."
    return 1
  fi
  SERVICE_WROTE=1
  return 0
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
  local cmd path
  path="$(service_definition_path ayeaye systemd)"

  _service_write_definition ayeaye systemd || return "$WIZARD_STAGE_FAIL"
  wizard_say "installed $path"
  wizard_say "(regenerated on every run; put settings in $(_service_env_file), not the unit)"

  _service_install_whisper systemd

  if cmd="$(platform_service_command ayeaye reload)"; then
    if ! _service_run "$cmd"; then
      wizard_say "this computer's service manager would not reload its files, so"
      wizard_say "ayeaye cannot be started for you."
      return "$WIZARD_STAGE_FAIL"
    fi
  fi

  _service_enable systemd || return $?
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
  local path logs
  path="$(service_definition_path ayeaye launchd)"
  logs="$(_service_log_dir)"

  # The agent names the file it writes, and launchd will not start something
  # whose log directory does not exist.
  mkdir -p "$logs" 2>/dev/null || true

  _service_write_definition ayeaye launchd || return "$WIZARD_STAGE_FAIL"
  wizard_say "installed $path"
  wizard_say "(regenerated on every run; put settings in $(_service_env_file), not the agent)"

  _service_install_whisper launchd

  # There is no daemon-reload here and this does not invent one: launchd
  # re-reads a property list by being handed it again, which is what enabling
  # does. lib/service.sh says so by answering "this machine cannot" to reload.

  _service_enable launchd || return $?
  wizard_say "to read its log: tail -f $logs/ayeaye.log"
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
  local kind="${1:-}" cmd path uid
  path="$(service_definition_path ayeaye "$kind")"

  if [ "$WIZARD_INTERACTIVE" = 1 ] && wizard_confirm "enable and start it now?" "y"; then
    [ "$kind" = launchd ] && _service_unregister_launchd ayeaye
    if ! cmd="$(platform_service_command ayeaye enable "$path")"; then
      wizard_say "this computer cannot be asked to start ayeaye for you."
      return "$WIZARD_STAGE_FAIL"
    fi
    if _service_run "$cmd"; then
      STARTED=1
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
    uid="$(platform_uid)"
    wizard_say "start it with: launchctl bootstrap gui/$uid $path"
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

# A seam for the ticket that owns voice: nothing is generated for whisper
# until that lands, and saying nothing is better than saying something wrong.
_service_install_whisper() {
  return 0
}
