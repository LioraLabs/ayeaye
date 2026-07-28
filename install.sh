#!/usr/bin/env bash
# One-command setup for voice-remote. Safe to re-run: it never overwrites an
# existing config without asking, and regenerating everything else is
# harmless.
#
#   ./install.sh              interactive, sane defaults
#   ./install.sh --defaults   accept every default, no prompts
#   ./install.sh --no-systemd skip the unit; prints the manual run command
#
# What it does:
#   1. checks hard deps (tmux, python3); reports optional voice-tier deps
#   2. writes ${XDG_CONFIG_HOME:-~/.config}/voice-remote/env from env.template
#   3. ensures the auth token exists (0600) and prints the bookmark URL
#   4. installs a systemd user unit running bin/voice-remote from this repo
set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CONF_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/voice-remote"
ENV_FILE="$CONF_DIR/env"
STATE_DIR="${XDG_STATE_HOME:-$HOME/.local/state}/voice-remote"
TOKEN_FILE="$STATE_DIR/token"
UNIT_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"

DEFAULTS=0
NO_SYSTEMD=0
for arg in "$@"; do
  case "$arg" in
    --defaults)   DEFAULTS=1 ;;
    --no-systemd) NO_SYSTEMD=1 ;;
    -h|--help)    sed -n '2,12p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "unknown option: $arg (try --help)" >&2; exit 2 ;;
  esac
done

say()  { printf '%s\n' "$*"; }
head_() { printf '\n\033[1m%s\033[0m\n' "$*"; }

# ask "prompt" "default" -> $REPLY. Empty input (or --defaults, or no
# stdin) takes the default, so `yes '' | ./install.sh` works too.
ask() {
  local prompt="$1" def="$2"
  REPLY=""
  if [ "$DEFAULTS" = 0 ]; then
    read -r -p "$prompt [${def:-empty}]: " REPLY || REPLY=""
  fi
  [ -n "$REPLY" ] || REPLY="$def"
}

# confirm "prompt" "y|n" -> 0 for yes
confirm() {
  local def="$2"
  ask "$1" "$def"
  case "$REPLY" in y|Y|yes|YES) return 0 ;; *) return 1 ;; esac
}

# ------------------------------------------------------------- dependencies
head_ "voice-remote setup"
say "repo: $REPO"

missing=0
for dep in tmux python3; do
  if command -v "$dep" >/dev/null 2>&1; then
    say "  ok       $dep"
  else
    say "  MISSING  $dep (required)"
    missing=1
  fi
done
[ "$missing" = 0 ] || { say ""; say "install the missing packages and re-run."; exit 1; }

# Optional tiers. Nothing here fails the install: the server probes voice at
# runtime and runs text-only until these appear.
notify_hint=""
voice_missing=""
command -v tailscale >/dev/null 2>&1 || say "  note     tailscale not found: you will need another way to serve https to the phone"
for dep in ffmpeg whisper-server ollama; do
  command -v "$dep" >/dev/null 2>&1 || voice_missing="$voice_missing $dep"
done
if [ -n "$voice_missing" ]; then
  say "  voice tier: missing$voice_missing (optional; text-only mode works without them)"
else
  say "  voice tier: all present (ffmpeg, whisper-server, ollama)"
fi

# ----------------------------------------------------------------- env file
head_ "configuration"

ts_host=""
if command -v tailscale >/dev/null 2>&1; then
  ts_host=$(tailscale status --json 2>/dev/null \
    | python3 -c 'import sys,json;print(json.load(sys.stdin)["Self"]["DNSName"].rstrip("."))' \
    2>/dev/null) || ts_host=""
fi

write_env=1
if [ -s "$ENV_FILE" ]; then
  say "config exists at $ENV_FILE"
  if confirm "rewrite it? (n keeps the current file)" "n"; then
    write_env=1
  else
    write_env=0
    say "keeping existing config"
  fi
fi

if [ "$write_env" = 1 ]; then
  ask "bind address" "127.0.0.1"
  bind="$REPLY"
  ask "port" "8911"
  port="$REPLY"
  if [ -n "$ts_host" ]; then
    ask "allowed hosts (your https front)" "$ts_host"
  else
    ask "allowed hosts (your https front, comma separated; empty for none)" ""
  fi
  hosts="$REPLY"
  ask "ntfy topic URL for push notifications (empty disables)" ""
  ntfy="$REPLY"

  mkdir -p "$CONF_DIR"
  BIND="$bind" PORT="$port" HOSTS="$hosts" NTFY="$ntfy" \
  python3 - "$REPO/env.template" > "$ENV_FILE.tmp" <<'PY'
import os, sys
text = open(sys.argv[1]).read()
for ph, var in (("@BIND@", "BIND"), ("@PORT@", "PORT"),
                ("@ALLOWED_HOSTS@", "HOSTS"), ("@NTFY_URL@", "NTFY")):
    text = text.replace(ph, os.environ.get(var, ""))
sys.stdout.write(text)
PY
  mv "$ENV_FILE.tmp" "$ENV_FILE"
  say "wrote $ENV_FILE"
fi

# Read the effective values back out of the file, so a kept config still
# gets a correct summary and unit.
get_env() {
  python3 - "$ENV_FILE" "$1" "$2" <<'PY'
import sys
val = sys.argv[3]
for line in open(sys.argv[1]):
    line = line.strip()
    if line.startswith(sys.argv[2] + "="):
        val = line.partition("=")[2].strip().strip('"').strip("'")
sys.stdout.write(val)
PY
}
bind=$(get_env VOICE_REMOTE_BIND 127.0.0.1)
port=$(get_env VOICE_REMOTE_PORT 8911)
hosts=$(get_env VOICE_REMOTE_ALLOWED_HOSTS "")
ntfy=$(get_env VOICE_NTFY_URL "")

# -------------------------------------------------------------------- token
# Same convention as the server: it would generate this file itself on first
# run; creating it here just means the bookmark URL is printable now.
if [ ! -s "$TOKEN_FILE" ]; then
  mkdir -p "$STATE_DIR"
  (umask 077; python3 -c 'import secrets;print(secrets.token_urlsafe(32))' > "$TOKEN_FILE")
  say "generated auth token at $TOKEN_FILE"
fi
chmod 600 "$TOKEN_FILE"
token=$(cat "$TOKEN_FILE")

# ------------------------------------------------------------------ systemd
head_ "service"

manual_cmd="$REPO/bin/voice-remote"
have_systemd=0
if [ "$NO_SYSTEMD" = 0 ] && command -v systemctl >/dev/null 2>&1 \
   && systemctl --user show-environment >/dev/null 2>&1; then
  have_systemd=1
fi

if [ "$have_systemd" = 1 ]; then
  mkdir -p "$UNIT_DIR"
  sed -e "s|@REPO@|$REPO|g" -e "s|@ENV_FILE@|$ENV_FILE|g" \
    "$REPO/systemd/user/voice-remote.service.template" \
    > "$UNIT_DIR/voice-remote.service"
  systemctl --user daemon-reload
  say "installed $UNIT_DIR/voice-remote.service"
  say "(regenerated on every run; put settings in $ENV_FILE, not the unit)"
  if [ "$DEFAULTS" = 0 ] && confirm "enable and start it now?" "y"; then
    systemctl --user enable --now voice-remote.service
    say "started. status: systemctl --user status voice-remote"
  else
    say "start it with: systemctl --user enable --now voice-remote"
  fi
  if command -v loginctl >/dev/null 2>&1 \
     && [ "$(loginctl show-user "$USER" -P Linger 2>/dev/null)" != "yes" ]; then
    say "recommended: loginctl enable-linger $USER"
    say "(without it, user services stop when your last session ends)"
  fi
else
  [ "$NO_SYSTEMD" = 1 ] && say "skipping systemd (--no-systemd)" \
                        || say "no systemd user session detected"
  say "run the server with: $manual_cmd"
  say "(it reads $ENV_FILE by itself)"
fi

# ------------------------------------------------------------------ summary
tier="text-only"
[ -n "$ntfy" ] && tier="$tier +notifications"
if command -v ffmpeg >/dev/null 2>&1 \
   && { command -v whisper-server >/dev/null 2>&1 \
        || command -v whisper-cpp >/dev/null 2>&1; }; then
  tier="$tier +voice"
fi

url_host="$bind"
first_host="${hosts%%,*}"

head_ "done"
say "tier    : $tier (probed live; the app adapts at runtime)"
say "config  : $ENV_FILE"
say "bookmark: http://$url_host:$port/?token=$token"
if [ -n "$first_host" ]; then
  say "          behind tailscale serve or a proxy: https://$first_host/?token=$token"
fi
say "open the bookmark URL once on the phone; it sets a cookie and"
say "redirects to /. Bookmark it and you never type the token again."
if [ "$have_systemd" = 1 ]; then
  say "logs    : journalctl --user -u voice-remote -f"
else
  say "logs    : stderr of $manual_cmd"
fi
say "https for the phone (mic needs a secure origin):"
say "  tailscale serve --bg http://127.0.0.1:$port"
