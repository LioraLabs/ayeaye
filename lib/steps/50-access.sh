# How you reach ayeaye, and the one rule underneath all four answers.
#
# ayeaye starts, stops and answers coding agents. Whoever can open it can run
# anything on this computer, so reaching it is the security decision this whole
# project turns on. Two things follow, and neither is negotiable:
#
#   ayeaye itself never leaves this computer. It listens on 127.0.0.1 in every
#   mode below. Anything a phone talks to is a separate program in front of it,
#   chosen by name, and setup will not offer to put ayeaye on the open internet
#   in any of them.
#
#   The key and the address check stay on in all four. bin/ayeaye wants a token
#   on every request and refuses a Host or an Origin it was not told about;
#   what this file does is configure those correctly per mode. It has no way to
#   switch either of them off and must never grow one.
#
# The four ways, in the order they are offered:
#
#   1  tailscale     a private network only the user's own devices join. The
#                    recommended answer: nothing is opened to the internet, it
#                    works away from home, and tailscale terminates https.
#   2  this computer  a browser on this machine and nothing else. Honest, safe,
#                    and clearly labelled as not being phone access.
#   3  home network   caddy, on a high port, holding a certificate signed by a
#                    small certificate authority made on this machine. The
#                    phone has to be taught to trust it, which is a real piece
#                    of work and is written out here in full for both phones.
#   4  existing proxy the user already runs something answering https for this
#                    computer; setup names it in the allow list and writes the
#                    proxy configuration they need.
#
# Nothing here asks a question that has nothing behind it. When neither
# tailscale nor caddy is here and neither can be installed, there is no menu:
# the run says so in one paragraph and keeps ayeaye on this computer, exactly
# as lib/steps/60-packages.sh does when it has nothing to offer.
#
# Everything that changes the machine goes through lib/consent.sh, and every
# one of the three modes that puts ayeaye within reach of another device is
# chosen through wizard_may_expose - including the one deduced from an https
# address somebody typed a stage earlier, which is deduced but still asked
# about. `expose` is one of the three kinds no flag can answer, so an
# unattended run can only ever end up on mode 2 whatever the offered default
# was. There are two gates and not one: the step that asks, and the step that
# acts, which refuses a network mode outright when nobody is there - because a
# run can be interrupted after the choosing and picked up by a run that cannot
# ask anything.
#
# bash 3.2: no associative arrays, no ${var,,}, no mapfile, no local -n.

# --------------------------------------------------------------- where things are
#
# Read at call time, not captured on load: install.sh has set all of these by
# the time lib/steps is sourced, and a test drives the pure parts directly.

_access_conf_dir()  { printf '%s' "${CONF_DIR:-${XDG_CONFIG_HOME:-$HOME/.config}/ayeaye}"; }
_access_state_dir() { printf '%s' "${WIZARD_STATE_DIR:-${XDG_STATE_HOME:-$HOME/.local/state}/ayeaye}"; }
_access_data_dir()  { printf '%s' "${XDG_DATA_HOME:-$HOME/.local/share}/ayeaye"; }
_access_env_file()  { printf '%s' "${ENV_FILE:-$(_access_conf_dir)/env}"; }

# The generated files. Every one of them is on GUARDED_PATHS in tests/run.sh.
#
# ACCESS_CADDYFILE is the one path two files have to agree on: this one writes
# it, and lib/steps/70-service.sh names it in the service definition that runs
# caddy. Both read the same variable with the same default so there is one
# answer to the question and no way for them to drift apart.
ACCESS_CADDYFILE="${ACCESS_CADDYFILE:-}"
_access_caddyfile() {
  if [ -n "$ACCESS_CADDYFILE" ]; then
    printf '%s' "$ACCESS_CADDYFILE"
  else
    printf '%s/Caddyfile' "$(_access_conf_dir)"
  fi
}

# A directory of its own holding exactly one file: the certificate the phone
# needs. caddy serves this directory over plain http and nothing else, and the
# reason it is not the directory caddy keeps its certificate authority in is
# that that directory also holds the authority's private key.
_access_ca_dir()   { printf '%s' "$(_access_conf_dir)/ca"; }
_access_ca_file()  { printf '%s' "$(_access_ca_dir)/ayeaye-ca.crt"; }
_access_ca_help()  { printf '%s' "$(_access_state_dir)/phone-certificate.txt"; }
_access_caddy_data()    { printf '%s' "$(_access_data_dir)/caddy"; }
_access_caddy_ca_root() { printf '%s' "$(_access_caddy_data)/pki/authorities/local/root.crt"; }
_access_proxy_caddy_file() { printf '%s' "$(_access_conf_dir)/reverse-proxy.caddy"; }
_access_proxy_nginx_file() { printf '%s' "$(_access_conf_dir)/reverse-proxy.nginx"; }

# The ports caddy answers on. Both are above 1024 on purpose: a user service
# that needed a privileged port would need this whole mode to ask for a
# password, and there is nothing here worth that.
ACCESS_LAN_PORT="${ACCESS_LAN_PORT:-8443}"
ACCESS_CA_PORT="${ACCESS_CA_PORT:-8480}"

# The address ayeaye itself listens on, in every mode. Named once.
ACCESS_LOOPBACK="127.0.0.1"

# _access_app_port - the port ayeaye answers on. The settings file first,
# because that is what is true now; then what was chosen this run.
_access_app_port() {
  local port
  port="$(wizard_env_get "$(_access_env_file)" AYEAYE_PORT "")"
  [ -n "$port" ] || port="$(wizard_state_get answer.port "")"
  # Checked, not trusted. This value was typed at a prompt that does not
  # validate it, and from here it reaches both a generated configuration file
  # and a command line. Anything that is not a number is not a port, and the
  # default is a better answer than passing it on.
  case "$port" in
    ""|*[!0-9]*) port="8911" ;;
  esac
  printf '%s' "$port"
}

# ----------------------------------------------------------------- the modes

# _access_label <mode> - what the mode is called in a sentence.
_access_label() {
  case "${1:-}" in
    tailscale) printf 'through tailscale' ;;
    local)     printf 'on this computer only' ;;
    lan)       printf 'over your home network' ;;
    proxy)     printf 'through the https address you already have' ;;
    *)         return 2 ;;
  esac
  return 0
}

# _access_is_network <mode> - status 0 when choosing it puts ayeaye within
# reach of a device that is not this one. The list wizard_may_expose guards.
_access_is_network() {
  case "${1:-}" in
    tailscale|lan|proxy) return 0 ;;
    *) return 1 ;;
  esac
}

# ------------------------------------------------------------------ probing

_access_have() { command -v "${1:-}" >/dev/null 2>&1; }

# _access_can_install - status 0 when a package could be installed here. Asked
# rather than assumed: on a machine with no package manager, offering to
# install tailscale is offering something that cannot happen.
_access_can_install() { platform_pkg_can_act; }

_access_tailscale_possible() { _access_have tailscale || _access_can_install; }
_access_caddy_possible()     { _access_have caddy || _access_can_install; }

# _access_lan_possible - caddy, and an address on a private network to put it
# on. Without the second there is nothing to write a certificate for.
_access_lan_possible() {
  _access_caddy_possible || return 1
  [ -n "$(_access_lan_ip)" ]
}

# _access_private_v4 <address> - status 0 for an address on one of the three
# private ranges. Nothing else is a home network, and a public address is
# exactly what this project will not put a front end on.
_access_private_v4() {
  local addr="${1:-}" part rest count=0
  # Four numbers and three dots, checked before the range. Everything this
  # returns 0 for ends up inside a generated configuration file and in the
  # list of addresses ayeaye accepts, so "starts with 10." is not enough of a
  # test: what produces these is a command's output, and a probe that trusts
  # its input is the assumption worth not making.
  rest="$addr"
  while [ -n "$rest" ]; do
    part="${rest%%.*}"
    case "$rest" in
      *.*) rest="${rest#*.}" ;;
      *) rest="" ;;
    esac
    case "$part" in
      ""|*[!0-9]*) return 1 ;;
    esac
    [ "${#part}" -le 3 ] || return 1
    count=$((count + 1))
  done
  [ "$count" = 4 ] || return 1
  case "$addr" in
    10.*|192.168.*) return 0 ;;
    172.1[6-9].*|172.2[0-9].*|172.3[01].*) return 0 ;;
    *) return 1 ;;
  esac
}

# _access_real_interface <name> - status 0 for an interface that could carry a
# home network.
#
# Docker's bridge, a virtual machine network and a tailscale interface all have
# private addresses and none of them is the wifi. On a machine where the bridge
# comes first - docker installed before a dock or a USB adapter, which is
# ordinary - taking the first private address would put the certificate on
# 172.17.0.1, tell the phone to use an address it can never reach, and offer to
# open the firewall to every container on the machine instead of to the house.
_access_real_interface() {
  case "${1:-}" in
    ""|lo|lo0) return 1 ;;
    docker*|br-*|virbr*|vmnet*|veth*|tailscale*|tun*|tap*|wg*|zt*|utun*|bridge*|podman*|cni*)
      return 1 ;;
    *) return 0 ;;
  esac
}

# _access_lan_addresses - "<address> <prefix-length> <interface>" per private
# address this computer has, one per line. Empty when there is none.
#
# Written against `ip` and `ifconfig` by hand rather than with awk: the test
# sandbox has exactly the commands a test asked for, and a probe that needed a
# second tool to read the first one's output would be untestable on the machine
# it has to work on.
_access_lan_addresses() {
  local line="" word prev iface addr prefix
  if _access_have ip; then
    while IFS= read -r line || [ -n "$line" ]; do
      [ -n "$line" ] || continue
      # "2: eth0    inet 192.168.1.42/24 brd … scope global …"
      iface=""
      addr=""
      prev=""
      for word in $line; do
        case "$prev" in
          inet) addr="$word" ;;
        esac
        [ -n "$iface" ] || case "$word" in
          *:) ;;
          *) iface="$word" ;;
        esac
        prev="$word"
      done
      [ -n "$addr" ] || continue
      prefix="${addr#*/}"
      addr="${addr%%/*}"
      [ "$prefix" != "$addr" ] || prefix=32
      _access_private_v4 "$addr" || continue
      _access_real_interface "$iface" || continue
      printf '%s %s %s\n' "$addr" "$prefix" "$iface"
    done <<EOF
$(ip -o -4 addr show 2>/dev/null)
EOF
    return 0
  fi
  if _access_have ifconfig; then
    iface=""
    while IFS= read -r line || [ -n "$line" ]; do
      case "$line" in
        " "*|"	"*) ;;
        *:*) iface="${line%%:*}" ;;
      esac
      case "$line" in
        *inet\ *) ;;
        *) continue ;;
      esac
      addr=""
      prev=""
      for word in $line; do
        case "$prev" in
          inet) addr="$word" ;;
        esac
        prev="$word"
      done
      [ -n "$addr" ] || continue
      _access_private_v4 "$addr" || continue
      # The mask ifconfig really printed, not the one the address class would
      # suggest: a 192.168.1.x machine is almost always on a /24, and calling
      # it a /16 would put half a million addresses into a firewall rule that
      # says it is scoped to this network. A mask this does not recognise
      # produces no prefix, and _access_lan_subnet then generates nothing.
      prefix=""
      prev=""
      for word in $line; do
        case "$prev" in
          netmask)
            case "$word" in
              0xffffff00|255.255.255.0) prefix=24 ;;
              0xffff0000|255.255.0.0)   prefix=16 ;;
              0xff000000|255.0.0.0)     prefix=8 ;;
            esac
            ;;
        esac
        prev="$word"
      done
      [ -n "$prefix" ] || prefix=32
      _access_real_interface "$iface" || continue
      printf '%s %s %s\n' "$addr" "$prefix" "$iface"
    done <<EOF
$(ifconfig 2>/dev/null)
EOF
  fi
  return 0
}

# _access_lan_ip - the first private address, or nothing.
_access_lan_ip() {
  local line=""
  while IFS= read -r line || [ -n "$line" ]; do
    [ -n "$line" ] || continue
    printf '%s' "${line%% *}"
    return 0
  done <<EOF
$(_access_lan_addresses)
EOF
  return 0
}

# _access_lan_subnet - the network that address is on, as "10.0.0.0/8", or
# nothing at all.
#
# Only whole-octet prefixes are answered. A rule for half an octet would have
# to be computed with arithmetic this shell can get subtly wrong, and a
# firewall rule that is subtly wrong is worse than no rule: the honest answer
# on anything else is to generate nothing and say so.
_access_lan_subnet() {
  local line="" addr prefix a b c rest
  while IFS= read -r line || [ -n "$line" ]; do
    [ -n "$line" ] || continue
    addr="${line%% *}"
    rest="${line#* }"
    prefix="${rest%% *}"
    a="${addr%%.*}"; rest="${addr#*.}"
    b="${rest%%.*}"; rest="${rest#*.}"
    c="${rest%%.*}"
    case "$prefix" in
      8)  printf '%s.0.0.0/8' "$a" ;;
      16) printf '%s.%s.0.0/16' "$a" "$b" ;;
      24) printf '%s.%s.%s.0/24' "$a" "$b" "$c" ;;
      *)  return 0 ;;
    esac
    return 0
  done <<EOF
$(_access_lan_addresses)
EOF
  return 0
}

# _access_mdns_name - "thishost.local", the name a phone on the same wifi can
# usually resolve without anything being configured, or nothing.
ACCESS_MDNS_NAME="${ACCESS_MDNS_NAME:-}"
_access_mdns_name() {
  local name=""
  if [ -n "$ACCESS_MDNS_NAME" ]; then
    printf '%s' "$ACCESS_MDNS_NAME"
    return 0
  fi
  if _access_have hostname; then
    name="$(hostname 2>/dev/null || true)"
  fi
  [ -n "$name" ] || name="${HOSTNAME:-}"
  [ -n "$name" ] || return 0
  name="${name%%.*}"
  case "$name" in
    ""|localhost) return 0 ;;
    *[!A-Za-z0-9-]*) return 0 ;;
  esac
  printf '%s.local' "$name"
}

# ----------------------------------------------------------------- tailscale
#
# One reader for the daemon's own answer, so "is it logged in" and "what is it
# called" cannot disagree.

# _access_tailscale_field <key> - a top-level or Self field of the status
# document, or nothing. Always 0.
_access_tailscale_field() {
  local key="${1:-}" out
  _access_have tailscale || return 0
  _access_have python3 || return 0
  out="$(tailscale status --json 2>/dev/null </dev/null | python3 -c '
import json, sys
key = sys.argv[1]
try:
    doc = json.load(sys.stdin)
except Exception:
    raise SystemExit(0)
value = doc.get(key)
if value is None:
    value = (doc.get("Self") or {}).get(key)
if isinstance(value, str):
    sys.stdout.write(value.rstrip("."))
' "$key" 2>/dev/null)" || out=""
  printf '%s' "$out"
}

_access_tailscale_hostname() { _access_tailscale_field DNSName; }
_access_tailscale_state()    { _access_tailscale_field BackendState; }

# _access_tailscale_serve_command <port> - what would be run to put tailscale's
# https front in front of ayeaye. A string, run by nobody here.
_access_tailscale_serve_command() {
  printf 'tailscale serve --bg http://%s:%s' "$ACCESS_LOOPBACK" "${1:-8911}"
}

# _access_tailscale_off_command - and what takes it away again.
_access_tailscale_off_command() { printf 'tailscale serve --https=443 off'; }

# ------------------------------------------------------------- the renderers
#
# Pure: they write to standard output and touch nothing, so a test can assert
# on every byte of what would be installed without installing it.

# access_render_caddyfile <app-port> <lan-address> [mdns-name]
#
# Two sites and no third. The https one is the only thing on the network that
# can reach ayeaye, and it reaches it on loopback. The plain-http one exists
# for one file - the certificate a phone needs before it can check the first
# one - and answers 404 to everything else, because a phone that does not have
# the certificate yet has no way to fetch it over https.
#
# What is deliberately not here: the admin endpoint, which caddy would
# otherwise open on this machine and which needs no password.
access_render_caddyfile() {
  local port="${1:-8911}" lan="${2:-}" mdns="${3:-}" hosts
  [ -n "$lan" ] || return 2
  hosts="https://$lan:$ACCESS_LAN_PORT"
  [ -z "$mdns" ] || hosts="$hosts, https://$mdns:$ACCESS_LAN_PORT"
  cat <<CADDY
# ayeaye's https front for your home network.
#
# Generated by setup and replaced whole every time it runs, so put anything of
# your own in a file of your own rather than in here.
#
# ayeaye itself answers on $ACCESS_LOOPBACK and nothing else can reach it. This
# file is the only thing on your network, it only forwards, and the address
# check and the key in ayeaye are both still in force behind it.
{
	# No admin endpoint. caddy would otherwise listen on this machine for
	# commands, with nothing asked of whoever sends them.
	admin off
	# And no redirect from plain http, which caddy would otherwise put on port
	# 80. Nothing here runs as root, port 80 needs root, and caddy that cannot
	# open a listener it was told to open does not start at all - so the whole
	# https front would be lost to a convenience nobody asked for.
	auto_https disable_redirects
	storage file_system "$(_access_caddy_data)"
	pki {
		ca local {
			name "ayeaye on $(_access_mdns_name_or "$lan")"
		}
	}
}

$hosts {
	# One interface, not all of them. A site address in caddy is a name to
	# match, not an address to listen on: without this line caddy opens these
	# ports on every address this computer has, including a public one if it
	# has one. Checked against a real caddy, whose listener is the wildcard
	# without this line and $lan with it.
	bind $lan
	tls internal
	reverse_proxy $ACCESS_LOOPBACK:$port
}

# The certificate your phone has to trust, and nothing else at all. Plain
# http, because a phone that does not have the certificate yet cannot check
# one, and it hands out one public file: no part of ayeaye is behind this.
http://$lan:$ACCESS_CA_PORT {
	bind $lan
	root * "$(_access_ca_dir)"
	@certificate path /ayeaye-ca.crt
	handle @certificate {
		file_server
	}
	handle {
		respond "this address only hands out /ayeaye-ca.crt" 404
	}
}
CADDY
  return 0
}

# The certificate authority's name as it will appear on the phone. A name
# somebody can recognise in a list of profiles is the difference between
# trusting the right thing and trusting whatever was there.
_access_mdns_name_or() {
  local name
  name="$(_access_mdns_name)"
  [ -n "$name" ] || name="${1:-this computer}"
  printf '%s' "$name"
}

# access_render_proxy_caddy <hostname> <app-port>
access_render_proxy_caddy() {
  local host="${1:-}" port="${2:-8911}"
  [ -n "$host" ] || return 2
  cat <<CADDY
# ayeaye, behind the https address you already have.
#
# Add the last line of this file to the block in your Caddyfile that already
# answers https for $host, and reload caddy:
#
#   $host {
#       ... whatever you already have ...
#       reverse_proxy $ACCESS_LOOPBACK:$port
#   }
#
# Not as a second block of its own. Two blocks for one address is something
# caddy reads as a mistake and refuses the whole file for, which would take
# the site you already have down with it.
#
# ayeaye answers on $ACCESS_LOOPBACK only, so this is the only thing that can
# reach it. caddy passes the address the browser asked for through unchanged,
# which is what ayeaye checks against $host - leave that alone or requests
# get a 403.
reverse_proxy $ACCESS_LOOPBACK:$port
CADDY
  return 0
}

# access_render_proxy_nginx <hostname> <app-port>
#
# ayeaye streams what your agents are doing as it happens, which nginx buffers
# by default: without proxy_buffering off the page sits blank and nothing in
# any log says why.
access_render_proxy_nginx() {
  local host="${1:-}" port="${2:-8911}"
  [ -n "$host" ] || return 2
  cat <<NGINX
# ayeaye, behind the https address you already have.
#
# Put this inside the server block that answers https for $host, then reload
# nginx. ayeaye answers on $ACCESS_LOOPBACK only, so this is the only thing
# that can reach it.
location / {
    proxy_pass http://$ACCESS_LOOPBACK:$port;
    proxy_http_version 1.1;

    # \$http_host, not \$server_name and not \$host: this is the address the
    # browser really asked for, port included. ayeaye checks it against $host
    # and answers 403 to anything else, which is what stops another site
    # sending your browser here - and nginx's \$host has the port taken off,
    # so an https address on a port other than 443 would be refused by the
    # very check this line exists to satisfy.
    proxy_set_header Host \$http_host;
    proxy_set_header X-Forwarded-Proto \$scheme;
    proxy_set_header X-Forwarded-For \$proxy_add_x_forwarded_for;

    # ayeaye sends what your agents do as it happens. Both of these are what
    # make it arrive rather than sit in a buffer.
    proxy_buffering off;
    proxy_read_timeout 3600s;
}
NGINX
  return 0
}

# _access_mobile_ca_help <certificate-url> <app-url> - the whole walkthrough.
#
# Written for somebody who has never done this, because they have not. The two
# phones are genuinely different and both are given every tap; the iPhone one
# has two halves and the second half is the one everybody misses - a
# certificate installed and not switched on in Certificate Trust Settings
# looks exactly like a certificate that does not work.
_access_mobile_ca_help() {
  local cert_url="${1:-}" app_url="${2:-}"
  cat <<HELP
Your phone has to be told, once, that it can trust the certificate this
computer just made for itself. Until you do, every page says the connection
is not private.

Both phones start the same way: put the phone on the same wifi as this
computer, and open this address in its browser:

    $cert_url

ON AN IPHONE OR IPAD - two halves, and the second is the one people miss

  1. Open the address above in Safari. It says a website is trying to show
     you a configuration profile. Tap Allow.
  2. Open Settings. Near the very top, under your name, a new line says
     "Profile Downloaded". Tap it.
     (If it is not there: Settings > General > VPN & Device Management.)
  3. Tap Install, top right. Type the phone's passcode. Tap Install again,
     then Install once more on the warning, then Done.
  4. Now the half that is easy to miss. Go to
     Settings > General > About > Certificate Trust Settings.
  5. Under "Enable full trust for root certificates" there is a switch with
     the name of this computer next to it. Turn it on, and tap Continue.
  6. Open $app_url in Safari. No warning means it worked.

  To undo all of it later: Settings > General > VPN & Device Management,
  tap the profile, then Remove Profile.

ON AN ANDROID PHONE

  1. Open the address above in Chrome. The file downloads; a notification
     says so. Nothing happens automatically - installing it is step 2.
  2. Open Settings and go to:
       Security & privacy > More security & privacy > Encryption &
       credentials > Install a certificate > CA certificate
     (On a Samsung: Settings > Security and privacy > Other security
     settings > Install from device storage > CA certificate.)
  3. It warns you in red that someone could monitor your traffic, and offers
     "Install anyway". That warning is about certificates in general and is
     expected here; tap Install anyway and type your PIN.
  4. Pick the downloaded file - it is in Downloads and is called
     ayeaye-ca.crt.
  5. Open $app_url in Chrome. No warning means it worked.

  Android leaves a standing note saying your network may be monitored. That
  is what it always says about a certificate you installed yourself.

  To undo it later: Settings > Security & privacy > More security & privacy >
  Encryption & credentials > Trusted credentials > User, then remove it.

This only affects the phone you do it on, and only for names this computer
signs for. It is not a certificate for anything else on the internet.
HELP
  return 0
}

# ------------------------------------------------------------------ running

# _access_run <command-string> - run something this file built, with its output
# kept for the log and put in front of the reader only when it failed.
#
# Not a consent wrapper and not a way around one: everything routed through it
# is either a read-only probe or an operation on this user's own service, and
# anything privileged, downloaded, exposed, trusted or replaced has already
# been through lib/consent.sh by the time it gets here.
_access_run() {
  local cmd="${1:-}" out
  [ -n "$cmd" ] || return 2
  wizard_detail "running: $cmd"
  if out="$(eval "$cmd" 2>&1 </dev/null)"; then
    [ -n "$out" ] && wizard_detail "$out"
    return 0
  fi
  wizard_detail "$out"
  [ -n "$out" ] && printf '%s\n' "$out" >&2
  return 1
}

# _access_probe_url <url> [ca-file] - the http status a request gets, or
# nothing when the request could not be made at all.
#
# Deliberately without the key. A request that comes back 401 has proved
# everything this needs to prove - something is listening, it is ayeaye, and
# it is still asking for the key - and a token on a command line ends up in
# the log, in the process list, and in this computer's shell history.
_access_probe_url() {
  local url="${1:-}" ca="${2:-}" out
  _access_have curl || return 1
  wizard_detail "checking: $url"
  if [ -n "$ca" ]; then
    out="$(curl -sS -o /dev/null -w '%{http_code}' --max-time 8 \
      --cacert "$ca" "$url" 2>&1 </dev/null)" || out=""
  else
    out="$(curl -sS -o /dev/null -w '%{http_code}' --max-time 8 \
      "$url" 2>&1 </dev/null)" || out=""
  fi
  [ -n "$out" ] || return 1
  case "$out" in
    *[!0-9]*) wizard_detail "$out"; return 1 ;;
  esac
  printf '%s' "$out"
  return 0
}

# _access_run_words <program> <argument>… - run something with arguments that
# were never a string, so a path containing a space or a quote is passed as
# itself rather than as two words.
_access_run_words() {
  local out
  wizard_detail "running: $*"
  if out="$("$@" 2>&1 </dev/null)"; then
    [ -n "$out" ] && wizard_detail "$out"
    return 0
  fi
  wizard_detail "$out"
  [ -n "$out" ] && printf '%s\n' "$out" >&2
  return 1
}

# _access_status_is_answer <status> - status 0 when an http status proves
# ayeaye answered. A refusal counts: without the key it is supposed to refuse,
# and a refusal is proof the address reaches it and the key is still wanted.
_access_status_is_answer() {
  case "${1:-}" in
    2*|3*|401|403) return 0 ;;
    *) return 1 ;;
  esac
}

# ----------------------------------------------------------------- settings
#
# The two settings every mode has an opinion about, and the only two it may
# touch. AYEAYE_PORT and everything else in the file belong to whoever put
# them there.

# _access_bind_is_open <value> - status 0 when a bind address would put ayeaye
# on the network by itself. This is the one setting this project will not
# write, in any mode, for any answer.
_access_bind_is_open() {
  case "${1:-}" in
    0.0.0.0|"::"|"*"|"[::]") return 0 ;;
    *) return 1 ;;
  esac
}

# _access_apply_settings <hosts> - merge, never rewrite.
#
# 0 when the settings on disk now say what this mode needs. The address ayeaye
# listens on comes from what the choosing step decided: loopback when it chose
# the mode, and nothing at all when it only deduced one, because an address
# somebody typed themselves in an earlier question is theirs.
_access_apply_settings() {
  local hosts="${1:-}" bind env_file
  bind="$(wizard_state_get answer.access.bind "")"
  env_file="$(_access_env_file)"
  if [ ! -f "$env_file" ]; then
    wizard_detail "no settings file at $env_file yet"
    return 1
  fi
  if [ -n "$bind" ]; then
    wizard_env_merge "$env_file" "AYEAYE_BIND=$bind" "AYEAYE_ALLOWED_HOSTS=$hosts" || return 1
  else
    wizard_env_merge "$env_file" "AYEAYE_ALLOWED_HOSTS=$hosts" || return 1
  fi
  return 0
}

# _access_warn_open_bind - say something when ayeaye is set to answer on every
# address this computer has and this step is not the one that decided it.
#
# What is about to be written first, because on a first run that is a typed
# answer that has not reached the settings file yet, and then what is already
# on disk.
_access_warn_open_bind() {
  local bind
  bind="$(wizard_state_get answer.bind "")"
  [ -n "$bind" ] || bind="$(wizard_env_get "$(_access_env_file)" AYEAYE_BIND "$ACCESS_LOOPBACK")"
  _access_bind_is_open "$bind" || return 0
  wizard_blank
  wizard_say "warning: ayeaye is set to answer on every address this computer"
  wizard_say "has, which means anything that can reach this machine can reach"
  wizard_say "your agents. The key is then the only thing in the way, and none"
  wizard_say "of it is encrypted."
  wizard_say "Nothing here has changed it, because that address is already in"
  wizard_say "your settings and it is not this step's to overwrite. To undo it,"
  wizard_say "change AYEAYE_BIND to $ACCESS_LOOPBACK in"
  wizard_say "$(_access_env_file), and run setup again."
  return 0
}

# ============================================================ 4. your choices

_access_choose_step() {
  local change default reply tries=0 hosts

  change="$(wizard_state_get answer.change_settings 1)"
  if [ "$change" = 0 ]; then
    wizard_say "keeping the way you reach ayeaye exactly as it is."
    _access_warn_open_bind
    return "$WIZARD_STAGE_SKIP"
  fi

  hosts="$(wizard_state_get answer.hosts "")"

  # Belt and braces, and the same belt install.sh already wears. An unattended
  # run cannot be asked, so it cannot answer the one question that puts ayeaye
  # within reach of another device - and wizard_may_expose would refuse below
  # even if this were removed.
  if [ "${WIZARD_INTERACTIVE:-1}" = 0 ]; then
    wizard_say "ayeaye will answer on this computer only."
    wizard_say "Run setup again from a terminal to reach it from your phone."
    # Nothing was chosen here, so nothing of anybody's is moved: an unattended
    # run records the refusal and leaves the settings it found. install.sh has
    # already forced the address to this computer for the same reason.
    _access_settle local "a run with nobody watching never opens ayeaye up" 0
    return "$WIZARD_STAGE_OK"
  fi

  # Nothing to offer is not a question. When neither of the two programs that
  # could give this computer an https address is here or installable, a menu
  # would be four answers of which three cannot be carried out.
  if ! _access_tailscale_possible && ! _access_lan_possible; then
    _access_no_front_end "$hosts"
    return "$WIZARD_STAGE_OK"
  fi

  _access_explain
  default=2
  _access_tailscale_possible && default=1

  while [ "$tries" -lt 3 ]; do
    tries=$((tries + 1))
    wizard_ask "which one? (1, 2, 3 or 4)" "$default"
    reply="$REPLY"
    case "$reply" in
      1|tailscale)
        if _access_tailscale_possible; then
          _access_offer tailscale
          return "$WIZARD_STAGE_OK"
        fi
        wizard_say "tailscale is not on this computer and cannot be installed here."
        ;;
      2|local|this)
        _access_settle local "you chose to keep ayeaye on this computer"
        return "$WIZARD_STAGE_OK"
        ;;
      3|lan|home)
        if _access_lan_possible; then
          _access_offer lan
          return "$WIZARD_STAGE_OK"
        fi
        if ! _access_caddy_possible; then
          wizard_say "caddy is not on this computer and cannot be installed here."
        else
          wizard_say "this computer is not on a home network setup can see, so there"
          wizard_say "is no address to put a certificate on."
        fi
        ;;
      4|proxy|existing)
        _access_offer proxy
        return "$WIZARD_STAGE_OK"
        ;;
      *)
        wizard_say "please answer 1, 2, 3 or 4."
        ;;
    esac
  done

  wizard_say "keeping ayeaye on this computer for now. Run setup again to change it."
  # Nothing was chosen, so nothing of anybody's is moved - the same reason the
  # unattended path above passes 0.
  _access_settle local "no answer that could be carried out, so nothing was opened" 0
  return "$WIZARD_STAGE_OK"
}

_access_explain() {
  wizard_blank
  wizard_say "How will your phone reach ayeaye?"
  wizard_blank
  wizard_say "ayeaye can start your coding agents, answer them and stop them, so"
  wizard_say "whoever opens it can do anything on this computer. That is why"
  wizard_say "setup will not put it on the open internet, whichever of these you"
  wizard_say "pick, and why all four keep the key and the address check."
  wizard_blank
  wizard_say "  1  tailscale - a private network that only your own devices"
  wizard_say "     join. Recommended: nothing is opened to the internet, and it"
  wizard_say "     works from anywhere, not only at home."
  if ! _access_tailscale_possible; then
    wizard_say "     (not possible here: tailscale is not installed and this"
    wizard_say "     computer has no way to install it)"
  fi
  wizard_say "  2  this computer only - a browser here, and nothing else. Safe,"
  wizard_say "     and your phone cannot reach it this way."
  wizard_say "  3  your home network - your phone reaches it on your own wifi"
  wizard_say "     and nowhere else. Your phone has to be taught to trust a"
  wizard_say "     certificate this computer makes, which takes a few minutes"
  wizard_say "     and has to be done once per phone."
  if ! _access_lan_possible; then
    wizard_say "     (not possible here yet)"
  fi
  wizard_say "  4  the https address you already have - you already run"
  wizard_say "     something that answers https for this computer."
  wizard_blank
  return 0
}

# _access_no_front_end <hosts> - the honest paragraph, and no question.
#
# Somebody who already named an https address has told us what is in front of
# this computer; that is an answer to the question, given a moment ago, so it
# is acted on rather than asked again. Everybody else gets this computer only,
# and what would change that.
_access_no_front_end() {
  local hosts="${1:-}"
  wizard_blank
  if [ -n "$hosts" ]; then
    wizard_say "You said $hosts is the https address for this computer. ayeaye can"
    wizard_say "stay on this computer and let just that address hand requests to"
    wizard_say "it - which is how your phone would reach it."
    # Asked, not assumed. There is a person here by definition, and letting an
    # address that is not this computer through is the decision this whole
    # file exists to put in one place. Nothing about the deduction is a reason
    # to skip the one question, and the answer costs one keystroke.
    if wizard_may_expose "may $hosts reach ayeaye?" \
        "an existing https front in front of $ACCESS_LOOPBACK"; then
      _access_settle proxy "" 0
    else
      wizard_blank
      # No `why`: wizard_may_expose has just recorded the refusal, and one
      # decision that produced two lines would be a trail that overstates.
      wizard_say "leaving everything as it is, then. Nothing has been opened."
      _access_settle local "" 0
    fi
    return 0
  fi
  wizard_say "Your phone reaches ayeaye through an https address, and this"
  wizard_say "computer has no way to make one yet."
  if _access_caddy_possible; then
    # caddy is here; what is missing is a home network to put it on.
    wizard_say "caddy could answer https on your home network, but this computer"
    wizard_say "is not on one setup can see - so there is no address to put a"
    wizard_say "certificate on - and tailscale is not installed either."
  else
    wizard_say "tailscale is not installed, caddy is not installed, and neither"
    wizard_say "can be installed from here."
  fi
  wizard_say "So ayeaye is set up for this computer only, which is safe and is"
  wizard_say "enough to try it out in a browser here."
  wizard_say "Install tailscale - the least work by a distance - and run setup"
  wizard_say "again, and it will offer to do the rest."
  _access_settle local "nothing here can give this computer an https address yet" 0
  return 0
}

# _access_offer <mode> - the consequence in words, then the one question that
# can put ayeaye within reach of another device.
#
# Refusing is not a failure and does not end anything: it settles on this
# computer only, which is a supported way to use ayeaye and the one every
# unattended run gets.
_access_offer() {
  local mode="${1:-}" question detail
  case "$mode" in
    tailscale)
      wizard_blank
      wizard_say "tailscale gives this computer a private https address that only"
      wizard_say "the devices signed in to your own tailscale account can open."
      wizard_say "Nothing is opened to the internet and nothing on your wifi can"
      wizard_say "reach it either."
      question="may your own tailscale devices reach ayeaye?"
      detail="tailscale serve in front of $ACCESS_LOOPBACK"
      ;;
    lan)
      wizard_blank
      wizard_say "This puts an https address on your home network. Anything on"
      wizard_say "your wifi can then open ayeaye, and the key is what keeps it"
      wizard_say "out - so it is only as private as your wifi is."
      wizard_say "It also means teaching each phone to trust a certificate this"
      wizard_say "computer makes, which is a few minutes of tapping per phone."
      question="may devices on your home network reach ayeaye?"
      detail="caddy on port $ACCESS_LAN_PORT in front of $ACCESS_LOOPBACK"
      ;;
    proxy)
      wizard_blank
      wizard_say "You already run something that answers https for this computer."
      wizard_say "Setup will name it as the one address allowed to hand requests"
      wizard_say "to ayeaye, and write the configuration you need to add to it."
      wizard_say "Whatever that thing is reachable from can then reach ayeaye,"
      wizard_say "with the key in the way."
      question="may that address reach ayeaye?"
      detail="an existing https front in front of $ACCESS_LOOPBACK"
      ;;
    *) return 2 ;;
  esac

  if wizard_may_expose "$question" "$detail"; then
    _access_settle "$mode" ""
    return 0
  fi
  wizard_blank
  wizard_say "leaving ayeaye on this computer only, then. Nothing has been opened."
  _access_settle local ""
  return 1
}

# _access_settle <mode> [why] [owns-address] - the decision, written down once.
#
# The ledger gets a line either way. wizard_may_expose has already recorded the
# grant for a mode that was asked about, so `why` is given only for a decision
# settled without a question - which is the documented use of
# wizard_consent_record and is how "did anything put this machine on a network"
# stays answerable from one file.
#
# `owns-address` is 1 when this step chose the mode rather than deduced it, and
# it is the only case in which the address ayeaye listens on is moved. A person
# who typed one into install.sh's own question a page earlier was answering a
# different question, and quietly overwriting their answer because a later step
# deduced something from a different one would be worse than the thing it
# prevents. Where it is left alone and it is an address that puts ayeaye on the
# network by itself, this says so out loud instead.
_access_settle() {
  local mode="${1:-}" why="${2:-}" owns="${3:-1}"
  wizard_remember answer.access.mode "$mode"
  if [ -n "$why" ]; then
    if _access_is_network "$mode"; then
      wizard_consent_record expose granted "$why"
    else
      wizard_consent_record expose refused "$why"
    fi
  fi

  # ayeaye itself goes back to loopback whenever this step chose the mode. The
  # front end is the thing on the network; the program behind it has no reason
  # to be, and an address typed into an earlier question would leave two of
  # them.
  if [ "$owns" = 1 ]; then
    wizard_remember answer.bind "$ACCESS_LOOPBACK"
    wizard_remember answer.access.bind "$ACCESS_LOOPBACK"
  else
    wizard_remember answer.access.bind ""
    _access_warn_open_bind
  fi

  case "$mode" in
    local)
      # install.sh used to put this line in the plan itself, from an address
      # question it no longer asks. It belongs here now: this is the step that
      # decides how ayeaye is reached, in all four answers, and the plan should
      # say what was decided rather than what somebody typed.
      wizard_plan_add network \
        "ayeaye will answer on $ACCESS_LOOPBACK:$(_access_app_port), this computer only"
      ;;
    tailscale)
      _access_have tailscale || wizard_plan_add package "tailscale, for a private https address"
      wizard_plan_add network "your own tailscale devices will be able to reach ayeaye"
      ;;
    lan)
      _access_have caddy || wizard_plan_add package "caddy, to answer https on your home network"
      wizard_plan_add network "devices on your home network will be able to reach ayeaye"
      wizard_plan_add trust "a certificate made on this computer, for your phone to trust"
      wizard_plan_add config "the https front's settings, at $(_access_caddyfile)"
      wizard_plan_add service "caddy will start when you log in, to answer https"
      ;;
    proxy)
      wizard_plan_add network "your existing https address will be able to reach ayeaye"
      wizard_plan_add config "proxy configuration for you to add, in $(_access_conf_dir)"
      ;;
  esac
  return 0
}

# ========================================================== 6. do the work

# _access_note_installed <mode> <ready> - what is really in front of ayeaye now.
#
# Two keys and not one, because the closing screen has two different things to
# say. `installed` is which way in this machine has; `ready` is whether it was
# finished. A tailscale that is installed but not signed in is `tailscale` and
# `0`: the address is the right one to write down and the wrong one to tell
# somebody to open.
#
# Both live under step.install.access, which a deliberate rerun clears along
# with every other step key - which is why the paths that change nothing
# re-derive them from _access_previous_mode rather than assuming last run's
# answer survived.
_access_note_installed() {
  wizard_remember step.install.access.installed "${1:-}"
  wizard_remember step.install.access.ready "${2:-0}"
  return 0
}

# _access_keep_what_is_there - for the paths that leave the way in alone. What
# is on this machine is a fact about the machine, and a run that touched
# nothing must still be able to say what that fact is.
_access_keep_what_is_there() {
  local previous
  previous="$(_access_previous_mode)"
  [ -n "$previous" ] || return 0
  _access_note_installed "$previous" 1
  return 0
}

_access_install_step() {
  local mode previous status

  if [ "$(wizard_state_get answer.change_settings 1)" = 0 ]; then
    wizard_say "leaving the way you reach ayeaye as it is."
    _access_keep_what_is_there
    return "$WIZARD_STAGE_SKIP"
  fi
  mode="$(wizard_state_get answer.access.mode "")"
  if [ -z "$mode" ]; then
    wizard_say "leaving the way you reach ayeaye as it is."
    _access_keep_what_is_there
    return "$WIZARD_STAGE_SKIP"
  fi

  # The last gate, and it is here rather than only in the step that asks
  # because a run picks up where an earlier one stopped. A person can choose a
  # way in, stop at the summary before anything happens, and the next run -
  # which may be `--defaults` in somebody's script - skips the stage that
  # asked and arrives here with the answer already in the state file. Setting
  # up something that puts ayeaye within reach of another device is not a
  # thing to do with nobody watching, whatever an earlier run recorded.
  if [ "${WIZARD_INTERACTIVE:-1}" = 0 ] && _access_is_network "$mode"; then
    wizard_say "you chose to reach ayeaye $(_access_label "$mode"), and setting that"
    wizard_say "up is not something to do with nobody there."
    wizard_say "Run setup again from a terminal and it will finish it off."
    wizard_consent_record expose refused \
      "a run with nobody watching will not set up a way in chosen by an earlier one"
    return "$WIZARD_STAGE_PENDING"
  fi

  # Changing your mind takes the old front end away before the new one goes
  # up. Two of them at once is the one outcome a re-run must not produce.
  previous="$(_access_previous_mode)"
  if [ -n "$previous" ] && [ "$previous" != "$mode" ]; then
    _access_teardown "$previous"
  fi

  case "$mode" in
    local)     _access_setup_local ;;
    tailscale) _access_setup_tailscale ;;
    lan)       _access_setup_lan ;;
    proxy)     _access_setup_proxy ;;
    *)
      wizard_detail "unknown access mode: $mode"
      return "$WIZARD_STAGE_SKIP"
      ;;
  esac
  status=$?
  if [ "$status" = "$WIZARD_STAGE_OK" ]; then
    _access_note_installed "$mode" 1
  elif [ "$status" = "$WIZARD_STAGE_PENDING" ]; then
    _access_note_installed "$mode" 0
  fi
  return "$status"
}

# _access_previous_mode - the way in that is set up on this machine now.
#
# What an earlier run recorded, and then what is actually on the disk. The
# second is not belt and braces: `--fresh` forgets everything a previous run
# wrote down, and so does deleting the state directory, and a run that then
# settled on "this computer only" would say your phone cannot reach ayeaye
# while a caddy nobody remembers is still answering on the wifi. What is
# installed is a fact about the machine, not about what setup remembers.
_access_previous_mode() {
  local recorded kind path
  recorded="$(wizard_state_get step.install.access.installed "")"
  if [ -n "$recorded" ]; then
    printf '%s' "$recorded"
    return 0
  fi
  if [ -f "$(_access_caddyfile)" ]; then
    printf 'lan'
    return 0
  fi
  kind="$(platform_service_manager)"
  case "$kind" in
    systemd|launchd)
      if path="$(service_definition_path ayeaye-caddy "$kind")" && [ -f "$path" ]; then
        printf 'lan'
        return 0
      fi
      ;;
  esac
  if [ -f "$(_access_proxy_caddy_file)" ] || [ -f "$(_access_proxy_nginx_file)" ]; then
    printf 'proxy'
    return 0
  fi
  # A tailscale that is serving this computer's ayeaye is the one way in with
  # nothing on the disk to find, so the daemon is asked instead.
  if _access_have tailscale; then
    # The port as well as the address. Somebody serving something of their own
    # on loopback is not ayeaye on tailscale, and reading it as one would end
    # with setup turning their serve off to make room for a mode they chose
    # for this program.
    case "$(tailscale serve status 2>/dev/null </dev/null || true)" in
      *"$ACCESS_LOOPBACK:$(_access_app_port)"*) printf 'tailscale'; return 0 ;;
    esac
  fi
  return 0
}

# --------------------------------------------------------------- 2. this computer

_access_setup_local() {
  local port
  port="$(_access_app_port)"
  # Only when this step was the one that chose. Where it settled here because
  # there was nothing else on offer, or because nobody was watching, the
  # settings that were already there are somebody else's answer to a different
  # question - and clearing the list of addresses ayeaye accepts would undo a
  # setting that puts nothing at risk on its own.
  if [ -n "$(wizard_state_get answer.access.bind "")" ]; then
    if ! _access_apply_settings ""; then
      wizard_say "could not write your settings, so nothing about how you reach"
      wizard_say "ayeaye has changed."
      return "$WIZARD_STAGE_FAIL"
    fi
  fi
  wizard_item ok "ayeaye will answer at http://$ACCESS_LOOPBACK:$port on this computer"
  wizard_blank
  wizard_say "This is a browser on this computer and nothing else - it is not how"
  wizard_say "you reach it from your phone, and your phone will not find it."
  wizard_say "Run setup again when you want to add tailscale or an https address"
  wizard_say "on your home network."
  return "$WIZARD_STAGE_OK"
}

# ------------------------------------------------------------------ 1. tailscale

_access_setup_tailscale() {
  local port host state cmd code

  port="$(_access_app_port)"
  if ! _access_have tailscale; then
    if ! wizard_install_packages tailscale; then
      wizard_blank
      wizard_say "tailscale is not here, so there is no private address to put in"
      wizard_say "front of ayeaye yet. Install it from https://tailscale.com/download"
      wizard_say "and run setup again; nothing else about your setup is affected."
      return "$WIZARD_STAGE_PENDING"
    fi
  fi

  state="$(_access_tailscale_state)"
  host="$(_access_tailscale_hostname)"
  if [ -z "$host" ]; then
    wizard_blank
    wizard_say "tailscale is installed here but this computer has not been signed"
    wizard_say "in to your tailscale account yet, so it has no address."
    wizard_say "Open a terminal and run:"
    wizard_say "    tailscale up"
    wizard_say "It prints a link; open it and sign in. Then run setup again and"
    wizard_say "it will finish this off."
    wizard_detail "tailscale backend state: ${state:-unknown}"
    return "$WIZARD_STAGE_PENDING"
  fi

  if ! _access_apply_settings "$host"; then
    wizard_say "could not write your settings, so nothing has been put in front"
    wizard_say "of ayeaye."
    return "$WIZARD_STAGE_FAIL"
  fi

  cmd="$(_access_tailscale_serve_command "$port")"
  if ! _access_run "$cmd"; then
    wizard_blank
    wizard_say "tailscale would not put its https address in front of ayeaye."
    wizard_say "Your settings are correct and ayeaye is unaffected; run this in a"
    wizard_say "terminal to finish it off:"
    wizard_say "    $cmd"
    return "$WIZARD_STAGE_PENDING"
  fi
  wizard_item ok "tailscale is answering https for $host"

  # Two checks, and they answer different questions. The first is whether
  # tailscale really holds the mapping that was just asked for; the second is
  # whether anything comes back from the address a phone would open.
  if ! _access_run "tailscale serve status"; then
    wizard_say "tailscale accepted the change but will not say what it is now"
    wizard_say "serving, so setup could not confirm it."
    return "$WIZARD_STAGE_PENDING"
  fi

  if ! code="$(_access_probe_url "https://$host/")"; then
    wizard_blank
    wizard_say "could not check the address from here, so it is worth opening"
    wizard_say "https://$host/ on your phone to be sure."
    return "$WIZARD_STAGE_PENDING"
  fi
  if _access_status_is_answer "$code"; then
    wizard_item ok "https://$host/ answers"
    wizard_blank
    wizard_say "Open that on your phone. Anything signed in to your tailscale"
    wizard_say "account can reach it; nothing else on the internet can."
    return "$WIZARD_STAGE_OK"
  fi
  wizard_detail "https://$host/ answered $code"
  wizard_blank
  wizard_say "https://$host/ is set up but did not answer the way it should yet."
  wizard_say "ayeaye itself is started a moment from now, which is usually all"
  wizard_say "that is missing; open the address on your phone once it has."
  return "$WIZARD_STAGE_PENDING"
}

# -------------------------------------------------------------- 3. home network

_access_setup_lan() {
  local port lan mdns caddyfile rendered app_url cert_url status wrote_help=0 replaced

  port="$(_access_app_port)"
  lan="$(_access_lan_ip)"
  mdns="$(_access_mdns_name)"
  if [ -z "$lan" ]; then
    wizard_say "this computer is not on a home network setup can see, so there is"
    wizard_say "no address to answer https on."
    return "$WIZARD_STAGE_PENDING"
  fi

  # The certificate is the whole of this mode, and making one is a decision
  # about what this computer's phones will believe. It goes through the
  # consent layer, which never grants it on anybody's behalf, and nothing is
  # written when it is refused.
  if ! wizard_consent trust \
      "may this computer make a certificate for your home network, for your phone to trust?" \
      "a local certificate authority under $(_access_caddy_data)"; then
    wizard_blank
    wizard_say "nothing has been made, and nothing on your network can reach"
    wizard_say "ayeaye. It is still there in a browser on this computer."
    return "$WIZARD_STAGE_SKIP"
  fi

  if ! _access_have caddy; then
    if ! wizard_install_packages caddy; then
      wizard_blank
      wizard_say "caddy is what would answer https on your home network and it is"
      wizard_say "not here. Install it from https://caddyserver.com/docs/install"
      wizard_say "and run setup again."
      return "$WIZARD_STAGE_PENDING"
    fi
  fi

  caddyfile="$(_access_caddyfile)"
  if ! rendered="$(access_render_caddyfile "$port" "$lan" "$mdns")"; then
    wizard_detail "could not work out what the https front should say"
    return "$WIZARD_STAGE_FAIL"
  fi
  wizard_replace "$caddyfile" \
    "there are already https front settings at $caddyfile. May I replace them?"
  replaced=$?
  if [ "$replaced" != 0 ]; then
    wizard_say "leaving $caddyfile alone, so nothing on your network has changed."
    # Refused is finished business; a copy that could not be made is a thing
    # that was tried and did not work, and the difference is the whole reason
    # those are two different statuses.
    [ "$replaced" = 1 ] && return "$WIZARD_STAGE_FAIL"
    return "$WIZARD_STAGE_SKIP"
  fi
  mkdir -p "$(_access_conf_dir)" 2>/dev/null
  mkdir -p "$(_access_ca_dir)" 2>/dev/null
  if ! printf '%s\n' "$rendered" > "$caddyfile" 2>/dev/null; then
    wizard_say "could not write $caddyfile, so nothing on your network has changed."
    return "$WIZARD_STAGE_FAIL"
  fi
  chmod 644 "$caddyfile" 2>/dev/null
  wizard_item ok "wrote $caddyfile"

  if ! _access_run_words caddy validate --adapter caddyfile --config "$caddyfile"; then
    wizard_blank
    wizard_say "caddy would not accept those settings, so nothing has been"
    wizard_say "started. The file is at $caddyfile and the log says why."
    return "$WIZARD_STAGE_PENDING"
  fi

  if ! _access_apply_settings "$(_access_lan_hosts "$lan" "$mdns")"; then
    wizard_say "could not write your settings, so nothing has been started."
    return "$WIZARD_STAGE_FAIL"
  fi

  _access_start_caddy || return $?

  app_url="https://${mdns:-$lan}:$ACCESS_LAN_PORT/"
  cert_url="http://$lan:$ACCESS_CA_PORT/ayeaye-ca.crt"

  if ! _access_export_ca; then
    wizard_blank
    wizard_say "the certificate your phone needs has not appeared yet. Run setup"
    wizard_say "again in a minute and it will pick this up."
    return "$WIZARD_STAGE_PENDING"
  fi

  _access_offer_firewall
  wrote_help=0
  _access_write_ca_help "$cert_url" "$app_url" && wrote_help=1

  status="$WIZARD_STAGE_OK"
  if ! _access_verify_lan "$lan"; then
    status="$WIZARD_STAGE_PENDING"
  fi

  wizard_blank
  _access_mobile_ca_help "$cert_url" "$app_url"
  wizard_blank
  if [ "$wrote_help" = 1 ]; then
    wizard_say "That is also written down at $(_access_ca_help), so you can read it"
    wizard_say "again without running setup."
  fi
  wizard_say "This works on your own wifi and nowhere else: away from home the"
  wizard_say "address does not resolve and nothing answers."
  wizard_say "One thing to know: it is tied to this computer's address on your"
  wizard_say "network, $lan. If your router hands it a different one later, run"
  wizard_say "setup again and it will make a new certificate for the new address."
  return "$status"
}

# _access_lan_hosts <lan> [mdns] - what ayeaye is told to accept.
#
# With the port, and that is the whole point of writing it out. bin/ayeaye
# matches an entry exactly first and only then with the port stripped, so a
# bare "192.168.1.42" here would accept requests claiming to come from that
# address on any port - including the plain-http one this same mode opens two
# lines further down to hand out a certificate. The front end is one address
# on one port, and that is what goes in the list.
_access_lan_hosts() {
  local lan="${1:-}" mdns="${2:-}"
  if [ -n "$mdns" ]; then
    printf '%s:%s,%s:%s' "$lan" "$ACCESS_LAN_PORT" "$mdns" "$ACCESS_LAN_PORT"
  else
    printf '%s:%s' "$lan" "$ACCESS_LAN_PORT"
  fi
}

# _access_start_caddy - write the definition and start it, through the service
# layer that already knows how to do both on either platform.
_access_start_caddy() {
  local kind path cmd
  if [ "${NO_SYSTEMD:-0}" = 1 ]; then
    _access_manual_caddy
    return "$WIZARD_STAGE_PENDING"
  fi
  kind="$(platform_service_manager)"
  case "$kind" in
    systemd|launchd) ;;
    *)
      _access_manual_caddy
      return "$WIZARD_STAGE_PENDING"
      ;;
  esac
  if ! command -v _service_write_definition >/dev/null 2>&1; then
    _access_manual_caddy
    return "$WIZARD_STAGE_PENDING"
  fi
  if ! _service_write_definition ayeaye-caddy "$kind"; then
    _access_manual_caddy
    return "$WIZARD_STAGE_PENDING"
  fi
  if ! path="$(service_definition_path ayeaye-caddy "$kind")"; then
    _access_manual_caddy
    return "$WIZARD_STAGE_PENDING"
  fi
  wizard_item ok "wrote $path"

  # Bootstrapping a launchd label it already knows fails outright, so a second
  # run has to take the old one out first. systemd needs its files re-read.
  if [ "$kind" = launchd ]; then
    command -v _service_unregister_launchd >/dev/null 2>&1 && _service_unregister_launchd ayeaye-caddy
  elif cmd="$(platform_service_command ayeaye-caddy reload)"; then
    _access_run "$cmd" || true
  fi

  if ! cmd="$(platform_service_command ayeaye-caddy enable "$path")"; then
    _access_manual_caddy
    return "$WIZARD_STAGE_PENDING"
  fi
  if ! _access_run "$cmd"; then
    wizard_blank
    wizard_say "the https front is configured but would not start."
    _access_manual_caddy
    return "$WIZARD_STAGE_PENDING"
  fi
  wizard_item ok "the https front is running and will start again when you log in"
  return "$WIZARD_STAGE_OK"
}

_access_manual_caddy() {
  wizard_say "start the https front by hand with:"
  wizard_say "    caddy run --adapter caddyfile --config $(_access_caddyfile)"
  wizard_say "(it has to be running for your phone to reach ayeaye)"
  wizard_say "Once it is running, run setup again: the certificate your phone"
  wizard_say "needs only exists after the first start, and the next run picks it"
  wizard_say "up and tells you how to get it onto the phone."
  return 0
}

# _access_export_ca - copy the certificate authority's certificate, and only
# that, somewhere a phone can fetch it.
#
# Never the key beside it. That file is what makes this computer able to sign
# for names on your network, and the directory this serves holds one file for
# exactly that reason.
ACCESS_CA_WAIT="${ACCESS_CA_WAIT:-20}"
_access_export_ca() {
  local root waited=0
  root="$(_access_caddy_ca_root)"
  while [ "$waited" -lt "$ACCESS_CA_WAIT" ]; do
    [ -s "$root" ] && break
    sleep 1
    waited=$((waited + 1))
  done
  [ -s "$root" ] || { wizard_detail "no certificate at $root after ${waited}s"; return 1; }
  mkdir -p "$(_access_ca_dir)" 2>/dev/null || return 1
  cp "$root" "$(_access_ca_file)" 2>/dev/null || return 1
  # Public, and meant to be handed out: this is the certificate, not the key.
  chmod 644 "$(_access_ca_file)" 2>/dev/null
  wizard_item ok "the certificate your phone needs is at $(_access_ca_file)"
  return 0
}

# _access_write_ca_help <certificate-url> <app-url> - the same walkthrough,
# kept where it can be read again without running setup. 1 when it could not
# be written, so that nobody is sent to a file that is not there.
_access_write_ca_help() {
  local file
  file="$(_access_ca_help)"
  mkdir -p "$(_access_state_dir)" 2>/dev/null
  _access_mobile_ca_help "${1:-}" "${2:-}" > "$file" 2>/dev/null || return 1
  chmod 644 "$file" 2>/dev/null
  return 0
}

# _access_verify_lan <lan> - does the front end really answer, with the
# certificate this computer made, from this computer?
_access_verify_lan() {
  local lan="${1:-}" code
  if ! code="$(_access_probe_url "https://$lan:$ACCESS_LAN_PORT/" "$(_access_ca_file)")"; then
    wizard_blank
    wizard_say "setup could not check the new address from here, so try"
    wizard_say "https://$lan:$ACCESS_LAN_PORT/ in a browser on this computer."
    return 1
  fi
  if _access_status_is_answer "$code"; then
    wizard_item ok "https://$lan:$ACCESS_LAN_PORT/ answers, with the new certificate"
    return 0
  fi
  wizard_detail "https://$lan:$ACCESS_LAN_PORT/ answered $code"
  wizard_say "the https front is up and ayeaye behind it is not answering yet."
  wizard_say "It is started a moment from now, which is usually all that is"
  wizard_say "missing."
  return 1
}

# _access_offer_firewall - open the port for the home network, or say why not.
#
# The rule is built by lib/consent.sh, which is the only file in this project
# allowed to name a firewall program at all, and it is scoped to the subnet
# this computer is actually on. A rule that cannot be scoped is not offered:
# "allow this port from anywhere" is not a smaller version of what was asked
# for, it is a different and much worse thing.
_access_offer_firewall() {
  local subnet rule
  subnet="$(_access_lan_subnet)"
  if [ -z "$subnet" ]; then
    wizard_detail "no whole-octet private subnet found; no firewall rule generated"
    return 0
  fi
  if ! rule="$(wizard_firewall_rule "$subnet" "$ACCESS_LAN_PORT")" || [ -z "$rule" ]; then
    wizard_blank
    wizard_say "if your phone cannot reach ayeaye, this computer's firewall is the"
    wizard_say "likeliest reason. Setup does not know how to write a rule for the"
    wizard_say "firewall here, and would rather say so than write the wrong one:"
    wizard_say "allow incoming connections on port $ACCESS_LAN_PORT from $subnet."
    return 0
  fi
  if wizard_firewall \
      "may I let devices on $subnet reach this computer on port $ACCESS_LAN_PORT?" \
      "$rule"; then
    wizard_item ok "your firewall now lets $subnet reach port $ACCESS_LAN_PORT"
    return 0
  fi
  wizard_blank
  wizard_say "nothing about your firewall has changed. If your phone cannot"
  wizard_say "reach ayeaye, that is the first thing to look at: it needs to"
  wizard_say "allow port $ACCESS_LAN_PORT from $subnet."
  return 0
}

# ------------------------------------------------------- 4. an https front you have

_access_setup_proxy() {
  local host port hosts code caddy_snippet nginx_snippet

  hosts="$(wizard_state_get answer.hosts "")"
  [ -n "$hosts" ] || hosts="$(wizard_env_get "$(_access_env_file)" AYEAYE_ALLOWED_HOSTS "")"
  # Only asked when it is not already known. Somebody who typed the address
  # into the question a page ago has answered this one, and asking again would
  # be asking the same thing twice.
  if [ -z "$hosts" ] && [ "${WIZARD_INTERACTIVE:-1}" != 0 ]; then
    wizard_blank
    wizard_say "What is the https address that already answers for this computer?"
    wizard_say "Just the name, the way you type it into a browser."
    wizard_ask "https address" ""
    hosts="$REPLY"
  fi
  if [ -z "$hosts" ]; then
    wizard_say "without that address ayeaye has no way to know which requests to"
    wizard_say "accept, so nothing has been changed."
    return "$WIZARD_STAGE_PENDING"
  fi
  # A name, and nothing that is not one. What is typed here is written into
  # two configuration files for other programs to read and into the list of
  # addresses ayeaye accepts; a space or a brace in it would end up as a line
  # of somebody's proxy configuration that they did not write.
  case "$hosts" in
    *[!A-Za-z0-9.,:_-]*)
      wizard_blank
      wizard_say "that does not look like an address: an https address is a name"
      wizard_say "like ayeaye.example.com, with no spaces and no punctuation"
      wizard_say "beyond dots and dashes. Nothing has been changed."
      wizard_detail "rejected https address: $hosts"
      return "$WIZARD_STAGE_PENDING"
      ;;
  esac
  host="${hosts%%,*}"
  port="$(_access_app_port)"

  if ! _access_apply_settings "$hosts"; then
    wizard_say "could not write your settings, so nothing has changed."
    return "$WIZARD_STAGE_FAIL"
  fi
  wizard_item ok "ayeaye will accept requests that arrive as $host"

  # Rendered into memory and then written, with both writes checked. A
  # redirection into a directory that cannot be written does not stop a step,
  # and telling somebody where to find a file that was never created would be
  # the worst kind of success.
  caddy_snippet="$(access_render_proxy_caddy "$host" "$port")" || return "$WIZARD_STAGE_FAIL"
  nginx_snippet="$(access_render_proxy_nginx "$host" "$port")" || return "$WIZARD_STAGE_FAIL"
  mkdir -p "$(_access_conf_dir)" 2>/dev/null
  if ! printf '%s\n' "$caddy_snippet" > "$(_access_proxy_caddy_file)" 2>/dev/null \
     || ! printf '%s\n' "$nginx_snippet" > "$(_access_proxy_nginx_file)" 2>/dev/null; then
    wizard_blank
    wizard_say "ayeaye will accept that address, but setup could not write the"
    wizard_say "configuration your proxy needs into $(_access_conf_dir)."
    wizard_say "Point it at $ACCESS_LOOPBACK:$port yourself, passing the address"
    wizard_say "the browser asked for through unchanged."
    return "$WIZARD_STAGE_PENDING"
  fi
  chmod 644 "$(_access_proxy_caddy_file)" "$(_access_proxy_nginx_file)" 2>/dev/null
  wizard_blank
  wizard_say "Your proxy has to be told to hand requests to ayeaye. Setup has"
  wizard_say "written both versions out; add the one that matches what you run:"
  wizard_say "    $(_access_proxy_caddy_file)"
  wizard_say "    $(_access_proxy_nginx_file)"
  wizard_say "Both point at $ACCESS_LOOPBACK:$port, which is where ayeaye stays."

  if ! code="$(_access_probe_url "https://$host/")"; then
    wizard_blank
    wizard_say "setup could not reach https://$host/ from here, which is what you"
    wizard_say "would expect until your proxy has been given the configuration"
    wizard_say "above. Add it, reload it, and open the address on your phone."
    return "$WIZARD_STAGE_PENDING"
  fi
  if _access_status_is_answer "$code"; then
    wizard_item ok "https://$host/ answers"
    return "$WIZARD_STAGE_OK"
  fi
  wizard_detail "https://$host/ answered $code"
  wizard_blank
  wizard_say "https://$host/ answered, but not as ayeaye - so the configuration"
  wizard_say "above has not been added to your proxy yet."
  return "$WIZARD_STAGE_PENDING"
}

# ------------------------------------------------------------ changing your mind

# _access_teardown <mode> - take the previous front end down.
#
# Layering a second one on top of the first is the failure this exists to
# prevent: two ways in, one of them forgotten, and a certificate on a phone
# nobody remembers installing.
_access_teardown() {
  local mode="${1:-}" kind path cmd
  case "$mode" in
    tailscale)
      _access_have tailscale || return 0
      wizard_say "taking the tailscale address away from ayeaye first."
      _access_run "$(_access_tailscale_off_command)" || true
      ;;
    lan)
      # Recorded and not there is a run that got as far as choosing and no
      # further. There is nothing to take down, and saying a certificate has
      # been removed from a computer that never made one would send somebody
      # looking for a profile on their phone that is not there either.
      if [ ! -f "$(_access_caddyfile)" ]; then
        wizard_detail "no https front on disk to take down"
        return 0
      fi
      wizard_say "taking the https front off your home network first."
      kind="$(platform_service_manager)"
      case "$kind" in
        systemd|launchd)
          if cmd="$(platform_service_command ayeaye-caddy disable)"; then
            _access_run "$cmd" || true
          fi
          if path="$(service_definition_path ayeaye-caddy "$kind")"; then
            rm -f "$path" 2>/dev/null
          fi
          if cmd="$(platform_service_command ayeaye-caddy reload)"; then
            _access_run "$cmd" || true
          fi
          ;;
      esac
      rm -f "$(_access_caddyfile)" "$(_access_ca_file)" "$(_access_ca_help)" 2>/dev/null
      rmdir "$(_access_ca_dir)" 2>/dev/null
      wizard_say "the certificate is gone from this computer. Remove it from any"
      wizard_say "phone you installed it on as well - only you can do that."
      wizard_say "If you started the https front by hand rather than letting setup"
      wizard_say "do it, stop that as well: it is still running with the settings"
      wizard_say "it read when it started."
      ;;
    proxy)
      rm -f "$(_access_proxy_caddy_file)" "$(_access_proxy_nginx_file)" 2>/dev/null
      wizard_say "removed the proxy configuration setup wrote for you. Your own"
      wizard_say "proxy still has whatever you added to it."
      ;;
    *)
      # Nothing was there to take down, so nothing goes in the ledger. A trail
      # saying a thing was removed when it never existed is worse than a trail
      # that is a line shorter.
      return 0
      ;;
  esac
  wizard_consent_record replace granted "removed the previous way in: $(_access_label "$mode")"
  return 0
}

# --------------------------------------------------------------- registration

wizard_step configure access _access_choose_step "How you will reach ayeaye"
wizard_step install   access _access_install_step "Setting up how you reach ayeaye"
