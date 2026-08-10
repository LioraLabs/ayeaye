# Network placement

Where ayeaye sits on the network is a judgement made of assumptions, which is
why the binary refuses to make it: it detects and verifies, and a person
decides. This file is the conversation that surfaces the assumptions, then the
ladder of placements from least exposure up. Every rung ends the same way:
`ayeaye check`, and read the report.

## The three questions first

Ask before proposing anything, because the answers pick the rung:

1. **What needs to reach it?** Just the browser on this machine? The user's
   phone? Other people?
2. **From what network?** The same room, the same tailnet, or anywhere?
3. **What already exists?** `tailscale status` says whether there is a mesh;
   the user knows whether a reverse proxy (caddy, nginx) already fronts this
   machine. The best placement is almost always the infrastructure they
   already trust.

Default to the lowest rung that satisfies the answers. Moving up a rung is
easy later; walking back an exposure is not, because the address is already in
someone's phone.

## The ladder

### 1. Loopback — the default, and a complete story

`127.0.0.1:8912`, the binary's own default, which setup leaves in place.
Nothing to do, nothing exposed. From another machine, an SSH forward reaches it without
opening anything:

```sh
ssh -L 8912:127.0.0.1:8912 user@machine
```

then `http://127.0.0.1:8912` on the near end. Right for occasional use from a
laptop; wrong for a phone, which has no ssh worth living with.

`ayeaye check`: `hosts` and `https` report `skipped` — nothing was asked for,
so there was nothing to check. That is the correct report, not a gap.

### 2. A mesh network the machine is already on

If `tailscale status` answers, every device on the tailnet can already reach
this machine privately; the `mesh` health check reports it. Two shapes:

- **`tailscale serve`** proxies to loopback with https and a tailnet name.
  ayeaye's bind stays loopback — nothing new listens on any network — and the
  phone gets a real `https://` address. Prefer this one. Add the name to
  `AYEAYE_ALLOWED_HOSTS` so the `hosts` and `https` checks verify it.
- **Bind the tailnet address** (`AYEAYE_BIND=100.x.y.z`). Plain http, no new
  software, private to the tailnet. The `https` check will report `unknown`
  until a host name is named — exposed with no address to try is exactly what
  that mark means.

Running `tailscale up`, or installing tailscale, is the user's command to run:
it changes their network membership, which is beyond even relayed consent to a
single yes — show the command and let them run it.

### 3. A reverse proxy with https

For a name the phone can open from anywhere the proxy is reachable. ayeaye's
bind stays loopback; the proxy the user already runs terminates TLS and
forwards to `127.0.0.1:8912`. Then set, in `~/.config/ayeaye/env`:

```sh
AYEAYE_ALLOWED_HOSTS=ayeaye.example.com
```

That name is what the `hosts` check asserts **in both directions** — the named
address must answer and a stranger's Host header must be refused with 403 —
and what the `https` check dials. The gate doing the refusing is the daemon's
own, and it reads the file at start: a `hosts` failure after editing the name
usually means the daemon predates the edit — restart it.

Show the proxy stanza before writing it, and write it only where the user
points. The binary never touches proxy configuration; neither does this skill
unasked.

### 4. A bare LAN bind — the last resort

`AYEAYE_BIND=0.0.0.0` or the LAN address. Say this out loud first, in these
terms: the token becomes the only lock, it crosses the network readable
because there is no TLS, and everyone on that network can try the door.
Driving ayeaye means running commands on this computer. A rung above this one
almost always fits; reach this one only when the user hears that and still
wants it — a trusted, isolated network, or a deliberate short-lived test.

`ayeaye check` treats any non-loopback bind as exposed and expects a front
end: `https` reporting `unknown` here is the report saying what this rung
gave up.

## After any change

The daemon reads the settings file at start, so: `ayeaye service stop`,
`ayeaye service start`, then `ayeaye check`. Exit 2 at any point —
something answered without the key — is a stop-everything moment; see
[health-checks.md](health-checks.md).
