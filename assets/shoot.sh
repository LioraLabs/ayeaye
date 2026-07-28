#!/usr/bin/env bash
# Phone-framed screenshot of a running voice-remote.
# usage: shoot.sh [url] [out.png]   (url defaults to localhost + your token)
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
TOKEN_FILE="${XDG_STATE_HOME:-$HOME/.local/state}/voice-remote/token"
URL=${1:-"http://127.0.0.1:8911/?token=$(cat "$TOKEN_FILE")"}
OUT=${2:-"$HERE/phone.png"}
WORK=$(mktemp -d); trap 'rm -rf "$WORK"' EXIT
chromium --headless=new --disable-gpu --screenshot="$WORK/raw.png" \
  --window-size=430,880 --force-device-scale-factor=3 --hide-scrollbars \
  --user-agent="Mozilla/5.0 (Linux; Android 15; Pixel 9) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126 Mobile Safari/537.36" \
  "$URL" 2>/dev/null
cat > "$WORK/frame.html" <<HTML
<!doctype html><meta charset="utf-8"><style>
html,body{margin:0;height:100%}
body{display:flex;align-items:center;justify-content:center;
  background:radial-gradient(120% 130% at 20% 0%,#3b4261 0%,#1a1b26 55%,#0f0f17 100%)}
.phone{border-radius:44px;padding:12px;background:#0b0b12;
  box-shadow:0 40px 90px rgba(0,0,0,.7),0 6px 18px rgba(0,0,0,.5),
  inset 0 0 0 2px rgba(255,255,255,.08)}
.phone img{display:block;width:400px;border-radius:32px}
</style><body><div class="phone"><img src="raw.png"></div></body>
HTML
chromium --headless=new --disable-gpu --screenshot="$OUT" \
  --window-size=900,1050 --force-device-scale-factor=2 --hide-scrollbars \
  "file://$WORK/frame.html" 2>/dev/null
echo "wrote $OUT"
