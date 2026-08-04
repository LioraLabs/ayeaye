<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/readme/mascot-dark.svg">
    <img src="assets/readme/mascot-light.svg" width="128" alt="the aye-aye">
  </picture>
</p>

# Aye, Aye

**Your agents ask. You tap. Aye, aye.**

A phone-shaped web app for the coding agents running in your tmux sessions.
It lists every pane, tells you which agent is working and which is waiting on
you, renders the conversation, answers permission prompts, and lets you talk
instead of type. Nothing is installed on the phone: the browser records, and
transcription happens on the machine with the GPU.

Works with **Claude Code** and **OpenAI Codex**.

Named for the [aye-aye](https://en.wikipedia.org/wiki/Aye-aye): enormous
watchful eyes, and it taps to find the spot that needs attention. Same job.

```
┌──────────────────────────┐
│  devbox           ● live │
├──────────────────────────┤
│ + new agent              │
├──────────────────────────┤
│ cook:1  [codex]  codex   │
│ ◆ needs you              │
│  ┌─────────────────────┐ │
│  │ Run `git restore`?  │ │
│  │ 1  Yes, proceed     │ │
│  │ 2  Yes, don't ask   │ │
│  │ 3  No, tell it why  │ │
│  │ [↑][↓][⏎ enter][esc]│ │
│  └─────────────────────┘ │
│                          │
│ dev:1  [claude]  claude  │
│ ● working          10s   │
├──────────────────────────┤
│ [⏎] [esc] [ 🎤 talk    ] │
└──────────────────────────┘
```

## Security model

Be clear about what this app is before you deploy it: it types into live
agent sessions, approves the commands those agents want to run, spawns new
agents, and streams full agent transcripts. **Treat it as equivalent to shell
access on the machine it runs on.**

Defense is layered. The network layer alone is not enough:

- **Token auth on every `/api/*` endpoint.** A shared secret comes from
  `AYEAYE_TOKEN` or, if unset, is generated with mode 0600 at
  `$XDG_STATE_HOME/ayeaye/token` on first run. Requests present it as
  an `X-Voice-Token` header or as the `voice_token` cookie; comparison is
  constant-time. Failures get a bare 401.
- **Host/Origin validation.** Requests whose `Host` header is not the bind
  address, `localhost`, `127.0.0.1`, or an entry in
  `AYEAYE_ALLOWED_HOSTS` are rejected with a 403, which defeats DNS
  rebinding. POSTs with a cross-site `Origin` or `Sec-Fetch-Site: cross-site`
  are also rejected, which defeats CSRF even if a token ever leaked to a page.
- **Network placement still matters.** Run this only on a trusted network
  (a tailnet, or loopback behind `tailscale serve`). **Never bind 0.0.0.0.**
  The token is a second lock on the door, not a reason to open the door.

Token bootstrap for a phone:

1. Start `ayeaye` once; note the token
   (`cat ~/.local/state/ayeaye/token`).
2. On the phone, open `https://<your-front>/?token=<token>` once. The server
   sets an HttpOnly, SameSite=Strict cookie and redirects to `/`: bookmark
   that login URL and you never type the token again.
3. Non-browser clients send the token as an `X-Voice-Token` header instead.

The static pages (`/`, `/board`) are served without auth; they contain no
data and never embed the token; every API call they make is what is gated.

## Why this exists

Agents spend a lot of their time waiting for you: a permission prompt, a
question, a choice. That's fine at the keyboard and miserable anywhere else.
This turns "go back to the desk" into "tap the notification you were going to
look at anyway".

## How it works

Everything is derived from things that already exist. There is no daemon on
the phone and no agent-side plugin.

| what | where it comes from |
|---|---|
| the pane list | `tmux list-panes` |
| what an agent said | the agent's own JSONL transcript, tailed live |
| which session a pane is running | see [Session mapping](#session-mapping) |
| pending permission prompts | `tmux capture-pane`; they exist only in the TUI |
| sending text and keys | `tmux send-keys` |
| speech to text | `whisper.cpp` server, held resident on the GPU |
| cleaning up dictation | a local LLM via ollama |

Three processes:

- **`ayeaye`**: the web app and its API. Reads tmux, tails transcripts,
  streams events to the browser over SSE.
- **`voice-agent`**: a small recorder daemon. Only needed if you want the
  `M-v` tmux binding to record from a *different* machine you're SSH'd in
  from. The web app does not need it.
- **`whisper-server`**: `whisper.cpp` with a model resident in VRAM.
  Optional: without it the app runs text-only ([Setup](#setup)).

## Install

One command, on a machine that has no copy of ayeaye on it:

```sh
curl -fsSL https://raw.githubusercontent.com/LioraLabs/ayeaye/main/install.sh | bash
```

It fetches a **pinned release** — the version is printed before anything is
downloaded and is visible in `--help` — into `~/.local/share/ayeaye`, checks
what arrived against the checksum published with that release, and then runs
exactly the setup described below from the copy it unpacked. It asks before
it downloads anything, and answering no leaves the machine as it was. When a
release publishes no checksum, or the machine has no `sha256sum`, it says so
in as many words rather than implying that something was verified.

Arguments go after `-s --`:

```sh
curl -fsSL https://raw.githubusercontent.com/LioraLabs/ayeaye/main/install.sh | bash -s -- --yes
```

An interrupted bootstrap is resumable: a part-finished download carries on
from where it stopped, and a release already unpacked is used as it is,
without touching the network at all. Running it from a clone downloads
nothing whatsoever — see [Setup](#setup).

## Setup

One command gets you the whole of the base install: the phone page, the pane
list, transcripts, prompt answering, spawning agents and typed input.
Everything else — voice, push notifications, the coding agents themselves, the
ticket board — is optional, is offered during the same conversation, and is
detected at runtime: add a piece and the feature lights up, remove it and it
degrades cleanly instead of breaking.

Hard requirements on the machine running your agents:

- Linux or macOS (session mapping reads process start times: `/proc` on
  Linux, `ps` and `lsof` on a Mac)
- `tmux`
- `python3` (3.9+, standard library only: no pip install)

Setup installs both of those for you if they are missing — after showing you
that it is going to, in stage five, along with everything else it would change.

Then:

```sh
git clone https://github.com/LioraLabs/ayeaye ~/dev/ayeaye
cd ~/dev/ayeaye
./install.sh
```

Setup is an eight-stage conversation:

1. it explains what it may change, and what it will never change
2. it works out what this machine is and what it already has
3. it says in plain words what ayeaye can do here
4. it asks how you want to reach it, and what to switch on
5. **it lists everything it is about to install or change, and asks** — nothing
   is installed before this point, including the two programs ayeaye cannot run
   without
6. it installs and configures what you chose
7. it starts ayeaye in the background and **checks that what you chose works**
8. it prints the address to open on your phone, and anything left to do

It writes one config file at `~/.config/ayeaye/env`, creates the auth token,
installs a background service — a systemd user unit on Linux, a launchd agent
on a Mac — that runs `bin/ayeaye` straight from the clone, and prints a
bookmark URL. Open that URL once on the phone; it sets the auth cookie and you
never type the token again. That's it.

Nothing privileged runs, nothing is downloaded, no firewall is opened, no
certificate is trusted and no file you already have is replaced without a
question first — and answering no to any of them leaves the machine exactly
as it was. Every one of those decisions is recorded at
`~/.local/state/ayeaye/setup-consent.log`, so "did that script do anything to
my machine" has one place to look.

Installer flags:

| flag | |
|---|---|
| `--defaults` | accept every default, ask nothing. Grants no permission of any kind, so it can never expose ayeaye to a network |
| `--yes` | answer the install and configuration questions in advance — but never one about the network, the firewall or the certificate store |
| `--no-systemd` | skip the background service; print the manual run command instead (`bin/ayeaye` reads the config file by itself) |
| `--details` | echo the raw commands as they run; they are logged either way, at `~/.local/state/ayeaye/setup.log` |
| `--fresh` | forget what earlier runs recorded and start over |
| `--help` | all of the above, plus the pinned version |

In the base install the pane list, transcripts, prompt answering, spawning
agents and typed input all work with no ffmpeg, no whisper.cpp and no ollama
installed. The talk button greys out with "voice not configured".
The project picker needs nothing installed: it searches below your home
directory itself, offers Git repositories first, and learns from the
directories you actually start agents in. Tune it, if you ever need to,
with the `AYEAYE_PROJECT_*` settings in `~/.config/ayeaye/env`.

One more thing worth doing on Linux: `loginctl enable-linger $USER`, so user
services survive logout, not just reboot. The installer reminds you when it is
off.

### The four ways to reach it

Stage four asks one question — how will your phone reach ayeaye — and there
are four answers. **ayeaye itself never leaves this computer in any of them:**
it listens on `127.0.0.1`, and whatever is on the network is a separate program
in front of it. The token and the `Host`/`Origin` check stay on in all four,
and setup has no way to switch either off.

| | | |
|---|---|---|
| **1** | **Tailscale** | A private network only your own devices join. The recommended answer: nothing is opened to the internet, it works away from home, and Tailscale terminates HTTPS with a real certificate. Setup runs `tailscale serve` for you and adds the tailnet name to the allow list. |
| **2** | **this computer only** | A browser on this machine and nothing else. Safe, honest, and clearly labelled as *not* phone access. This is what every unattended run gets, whatever else it was told. |
| **3** | **your home network** | Caddy on port 8443, holding a certificate signed by a small certificate authority made on this machine. Your phone has to be taught to trust it — a few minutes of tapping, once per phone — and setup writes the walkthrough out to `~/.local/state/ayeaye/phone-certificate.txt` so you can read it again. Works on your own wifi and nowhere else. |
| **4** | **an HTTPS address you already have** | You already run something answering HTTPS for this machine. Setup names it as the one address allowed to hand requests to ayeaye and writes the proxy configuration you need into `~/.config/ayeaye/reverse-proxy.caddy` and `.nginx`. |

Choosing a different one later takes the old front end down before the new one
goes up: two ways in, one of them forgotten, is the outcome a re-run must not
produce. See [Exposure](#exposure) for why HTTPS matters at all.

### Optional components

None of these is needed for the phone page to work, and the app detects each
one at runtime — add it and the feature lights up, remove it and it degrades
cleanly.

| | |
|---|---|
| **Push notifications** | one setting, `VOICE_NTFY_URL`. [Push notifications](#push-notifications) below. |
| **Voice** | whisper.cpp to listen, ollama to tidy up. Setup sizes the model to the machine, downloads it with your say-so, and installs a service that keeps it loaded. [Voice](#voice) below. |
| **Coding agents** | Claude Code and Codex. Setup offers to fetch either, and to set up the [session marker](#session-mapping) Claude Code needs. |
| **The project board** | `cliban`, for [`/board`](#the-board). Setup offers to fetch it. |

Anything that could not be finished is listed at the end of the run, with how
to pick each one up, and `./install.sh` again picks up exactly those.

### What the last stage checks

Stage seven does not assume the install worked. For **the options you actually
chose**, it checks the service is running, that ayeaye answers on this
computer, that **a request carrying no key is refused**, that the coding agents
you asked for are installed, that the addresses in your allow list are accepted
and one that is not is refused, that HTTPS through your chosen front end
answers, that the talk button is live, and that the board answers. Each check
reports one of four things, and they are four distinct marks — `ok`, `FAILED`,
`skipped` (you did not ask for it) and `unknown` (setup could not tell). A
check that was skipped is never rendered as one that passed.

What it cannot tell you: whether your phone can reach the address, whether the
service will still be there after a reboot, and whether a front end you have
not configured yet will work once you have. Those need a phone, a reboot and a
proxy respectively.

### Re-running, and changing your mind

Re-running is safe, and it is how anything gets changed — including switching
between the four ways in. An existing config file is never overwritten without
asking, and when you do agree to a change only the settings you were asked
about are rewritten: everything else in the file, including comments and
settings the wizard has never heard of, survives byte for byte, and the
previous version is copied to `~/.local/state/ayeaye/backups/` first. The auth
token is never regenerated, so a bookmark already on your phone keeps working.

A run that is interrupted picks up where it stopped rather than starting over:
what it finished is recorded at `~/.local/state/ayeaye/setup-state`.

Check it by hand, if you want to:

```sh
systemctl --user is-active ayeaye          # launchctl print gui/$UID/dev.ayeaye on a Mac
journalctl --user -u ayeaye -f             # tail -f ~/Library/Logs/ayeaye/ayeaye.log on a Mac
curl -s -H "X-Voice-Token: $(cat ~/.local/state/ayeaye/token)" \
  localhost:8911/api/overview | python3 -m json.tool
```

### Removing it

The closing screen prints this for the machine you are actually on; it is
repeated here for the two it can be.

```sh
# Linux
systemctl --user disable --now ayeaye.service
rm ~/.config/systemd/user/ayeaye.service

# macOS
launchctl bootout gui/$(id -u)/dev.ayeaye
rm ~/Library/LaunchAgents/dev.ayeaye.plist
```

Then, on either:

```sh
rm -r ~/.config/ayeaye ~/.local/state/ayeaye
```

If you chose the home-network way in, there is a second service
(`ayeaye-caddy`) and a certificate: disable that service the same way, and
**remove the certificate from every phone you installed it on** — nothing on
this computer can do that for you. If you let setup install Claude Code, Codex
or cliban, they are in `~/.local/bin` and are yours to keep or delete.

### Push notifications

One env var. Point `VOICE_NTFY_URL` in `~/.config/ayeaye/env` at any
[ntfy](https://ntfy.sh) topic (the installer asks for it too), restart the
service, and put the ntfy app on the phone. Self-hosting and buffering
details are in [Notifications](#notifications).

### Voice

Two local models on a GPU box: whisper.cpp transcribes, ollama cleans up.
Install them (next two sections) and enable the whisper unit; there is
nothing to configure in the app itself. The server probes the whisper
endpoint at runtime (cached 30 s, `VOICE_PROBE_TTL`), reports it as a
`voice` boolean in `/api/overview`, and the talk button comes alive when
the probe answers. `/api/dictate` answers 503 while it does not.

There is nothing to copy or edit by hand: once `whisper-server` is on your
`PATH` and `VOICE_WHISPER_MODEL` in `~/.config/ayeaye/env` says where the
model is, `./install.sh` writes the service itself - a systemd user unit on
Linux, a launchd agent on a Mac - and asks whether to keep the model loaded.
It reads the model, the address and the thread count out of that settings
file every time it starts, so changing the port means editing one file and
restarting, and never editing a unit.

For the `M-v` binding that dictates into the pane you are sitting in, and
for recording from another device you SSH in from, see
[The tmux binding](#the-tmux-binding-optional) and run
`voice-dictate-setup`: it prints per-device instructions.

### Whisper

Model size is nearly free on a modern GPU: on an RTX 5090, `large-v3` encodes
in ~68 ms, the same as `small.en`. What costs time is *loading* it: 1.5 s for
`large-v3` on every invocation. So run it as a server and keep it resident.
The service `./install.sh` writes does that.

```sh
# build with CUDA (adjust the arch for your card; 120 = Blackwell)
cmake -B build -DGGML_CUDA=ON -DCMAKE_CUDA_ARCHITECTURES=120
cmake --build build -j --target whisper-server
curl -L -o ~/whisper-models/ggml-large-v3.bin \
  https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3.bin
```

If `whisper-server` is unreachable, `voice-dictate` falls back to the
whisper.cpp command line with a smaller model: correct, just slower. Which
command that is comes from `VOICE_WHISPER_CLI`, or from the first of
`whisper-cli`, `whisper-cpp` and `whisper` on `PATH` -- the project renamed
its binaries and both names are still out there. `bin/ayeaye` greys out the
talk button on the same question, so the two cannot disagree.

### Ollama

Used to clean up dictation: filler removal, punctuation, joining spoken
identifiers (`"parse underscore config"` → `parse_config`).

```sh
ollama pull qwen2.5:7b-instruct
```

**Use an instruct model, not a coder model.** A code-tuned model *answers*
dictated requests instead of rewriting them: `"write a function called parse
underscore config"` comes back as an actual `def` block, which then gets typed
into your pane. Same speed, wrong behaviour.

The model is pinned with `keep_alive` (default 1h), because ollama's default
5-minute idle unload turns the next dictation into a 3 s model load for 0.2 s
of work.

## Session mapping

The hard part: given a tmux pane, which agent conversation is it?

**Claude** advertises it. Its statusline command receives `session_id` on
stdin, so print a short marker and `capture-pane` can read it back. Add this
to your statusline script (full example in `examples/statusline-command.sh`):

```sh
session_id=$(echo "$input" | jq -r '.session_id // empty')
[ -n "$session_id" ] && printf '\033[90m⟪cc:%s⟫\033[0m\n' "${session_id:0:8}"
```

Eight hex characters is enough to glob the transcript uniquely, and it's dim
enough to ignore on screen. `capture-pane` strips the colour but keeps the
text.

`./install.sh` offers to set this up for you: it writes a small status line
script to `~/.local/share/ayeaye/statusline-command.sh` and points
`~/.claude/settings.json` at it, after showing you the change and taking a copy
of the file. A status line you already have is never replaced without being
shown to you first.

**Print it on its own line, first on that line.** Appended to a path segment
it gets truncated the moment the working directory is long, and a clipped
marker fails silently: the pane still works, the transcript button just
goes quiet.

**Codex** has no statusline, so it's matched by **process start time**:
find the codex process in the pane, read its `cwd` and start time, then pick
the rollout whose filename timestamp lands within a couple of seconds of it.
That lookup is the one platform-specific corner in the server -- `/proc` on
Linux, `ps` and `lsof` on macOS -- behind a single internal interface. That stays exact with two codex agents in one directory, and
works however codex was launched.

Codex hooks are *not* used. They do fire, but only on the first turn, by
which point the transcript exists and the timing match already works.

## Exposure

**HTTPS is required.** Browsers refuse `getUserMedia` on an insecure origin,
so the page will load over plain HTTP and the talk button will fail.

`./install.sh` sets one of [the four ways in](#the-four-ways-to-reach-it) up
for you, and three of them terminate HTTPS. What follows is the same thing by
hand, for a machine that was set up before any of that existed.

Easiest, if you use Tailscale:

```sh
tailscale serve --bg http://127.0.0.1:8911
```

That publishes `https://<machine>.<tailnet>.ts.net/` with a real certificate,
reachable only from your tailnet. For a reverse proxy instead, see
`examples/Caddyfile.snippet`.

### Endpoint hardening

See [Security model](#security-model) at the top for the auth and
Host/Origin rules. On top of those, the endpoints themselves are narrow:

- The key endpoint accepts digits `1`-`9` and
  `up down left right enter esc tab space`. Nothing else: it is a
  prompt-answering endpoint, not a remote keyboard.
- Spawning is limited to a fixed `{claude, codex}` map, so the agent name is
  never interpolated from a request.
- Request bodies are capped (1 MB, 32 MB for dictation audio), and dictation
  uploads must carry a known audio extension.
- If you front it with a proxy, additionally restrict by source IP
  (see `examples/Caddyfile.snippet`). DNS scoping is not access control:
  if your resolver points a hostname at the proxy, anything using that
  resolver can reach it.

For identity-level auth on top, put it behind an identity-aware proxy
(Authentik, Tailscale Serve + ACLs, oauth2-proxy).

## The tmux binding (optional)

Separate from the web app: dictate straight into the pane you're in.

```tmux
bind -n M-v run-shell -b "voice-dictate #{pane_id} #{client_pid}"
set -g @voice_rec ""
```

Press once to start, again to stop; polished text is typed into the pane
**without** pressing Enter, so you review before submitting. Show the
indicator by referencing `#{@voice_rec}` somewhere in your status line.

This is `run-shell`, not `display-popup`, on purpose. A popup has to own the
terminal to read a keypress, and with `mouse on` the escape sequences that
generates read as cancel: it closes the instant it appears.

Recording for `M-v` comes from `voice-agent`. If the tmux client is local,
that's this machine. If you're SSH'd in from a laptop, the request goes to
`voice-agent` on *that* machine over the tailnet, which is what
`voice-dictate-setup` configures. The web app needs none of this.

## Configuration

Everything lives in one file: `~/.config/ayeaye/env`, written by
`install.sh` from `env.template`. The systemd unit reads it through
`EnvironmentFile=`, and the `bin/` scripts read it themselves when run by
hand, so both paths see the same settings. A variable set in the real
environment always wins over the file. No setting requires editing a
script or a unit file. Legacy `VOICE_REMOTE_*` variable names and the old
`~/.config/voice-remote/env` path keep working, so an existing install
survives the rename untouched.

The template documents every variable with its default; the highlights:

| variable | default | |
|---|---|---|
| `AYEAYE_BIND` | `127.0.0.1` | bind address; see Security |
| `AYEAYE_PORT` | `8911` | |
| `AYEAYE_TOKEN` | generated | auth token; unset = generated 0600 in `$XDG_STATE_HOME/ayeaye/token` |
| `AYEAYE_ALLOWED_HOSTS` | unset | extra allowed `Host` values (your https front), comma separated |
| `AYEAYE_SHARE` | auto | dir holding `app.html`; defaults to the repo's `share/` |
| `AYEAYE_LINES` | `24` | history lines above the visible screen in the terminal view |
| `AYEAYE_FIT_TTL` | `12` | seconds an auto-fit lease survives without the pane being polled |
| `VOICE_TX_ROWS` | `200` | transcript entries sent on connect |
| `VOICE_IDLE_AFTER` | `300` | seconds before an agent reads as idle |
| `VOICE_SEND_DELAY` | `0.4` | gap between typing text and Enter |
| `VOICE_PROBE_TTL` | `30` | seconds the voice probe result is cached |
| `VOICE_WHISPER_SERVER` | `127.0.0.1:8910` | probed at runtime; unreachable = text-only |
| `VOICE_WHISPER_MODEL` | `~/whisper-models/ggml-small.en.bin` | model for the CLI fallback |
| `VOICE_WHISPER_THREADS` | `16` | threads for the CLI fallback |
| `VOICE_OLLAMA_HOST` | `localhost:11434` | |
| `VOICE_OLLAMA_MODEL` | `qwen2.5:7b-instruct` | |
| `VOICE_KEEP_ALIVE` | `1h` | how long ollama holds the model |
| `VOICE_SILENCE_RMS` | `1000` | below this a clip is treated as silence |
| `VOICE_NO_CONTEXT` | unset | set to disable pane vocabulary |
| `VOICE_CONTEXT_LINES` | `40` | pane lines scanned for vocabulary |
| `VOICE_LOG` | `$XDG_STATE_HOME/voice-dictate.jsonl` | dictation log |
| `VOICE_NTFY_URL` | unset | ntfy topic URL; empty disables notifications |
| `VOICE_NTFY_CLICK` | unset | URL opened when a notification is tapped |
| `VOICE_NOTIFY_EVERY` | `10` | seconds between checks for blocked agents |
| `VOICE_NOTIFY_STATES` | `blocked` | comma separated; `waiting` is noisy |
| `VOICE_CLIBAN` | auto | path to the cliban binary for `/board` |
| `VOICE_BIND` | `127.0.0.1` | voice-agent bind address (client device) |
| `VOICE_PORT` | `8787` | voice-agent port |
| `VOICE_SOURCE` | auto | recording source for the ffmpeg backend |
| `VOICE_HOST` | tailscale name | hostname `voice-dictate-setup` prints for clients |

`VOICE_SEND_DELAY` exists because Codex's composer reads an Enter arriving in
the same input burst as the text as a newline rather than a submit. Claude
tolerates it; the gap makes both behave.

`VOICE_SILENCE_RMS` gates dictation on loudness, not duration: handed silence
whisper confidently returns "Thank you.", and a duration cutoff would throw
away legitimately short commands like "run the tests".

## The board

`/board` is a read-mostly view over cliban: projects, milestones with
progress, and issues grouped by status. Tap a
ticket and its markdown body renders in place; tap **run** and the ticket is
handed to a fresh agent: pick a directory (the same project picker as the
main page) and the agent starts with an opening prompt pointing it at the issue:
`cliban issue show` for the spec, `log`/`tick`/`mv` to keep the ticket
honest while it works.

Data comes from shelling out to `cliban … --json`, not from reading its
SQLite file, so list semantics stay cliban's. The binary is found on `PATH`
or at `~/.cargo/bin/cliban`; override with `VOICE_CLIBAN`.

## Notifications

An agent that stops to ask for permission is the whole reason you'd look at
your phone, so `ayeaye` can push when that happens.

It polls its own overview and fires on the **transition** into `blocked`,
keyed on the pane plus the question: a new prompt in the same pane notifies
again, the same prompt sitting unanswered stays quiet.

Delivery is [ntfy](https://ntfy.sh). Self-host it and nothing leaves your
network:

```sh
docker run -d --name ntfy -p 127.0.0.1:2586:80 \
  -e NTFY_BASE_URL=https://ntfy.example.com \
  -e NTFY_BEHIND_PROXY=true \
  -v /srv/ntfy:/var/lib/ntfy \
  binwiederhier/ntfy serve
```

Then point the service at it in `~/.config/ayeaye/env` and install
the ntfy app on your phone, configured for your server, subscribed to the
topic:

```sh
VOICE_NTFY_URL=http://127.0.0.1:2586/agents
VOICE_NTFY_CLICK=https://agents.example.com/
```

Publish to the container directly rather than out through your proxy: fewer
moving parts, and notifications keep working if the proxy is down.

`VOICE_NOTIFY_STATES` defaults to `blocked`. Adding `waiting` notifies at the
end of every turn, which for an active agent is constant; it's there if you
want it.

If you proxy ntfy, disable response buffering (`flush_interval -1` in Caddy).
The phone holds a long-lived connection for instant delivery, and a buffering
proxy makes notifications arrive in clumps.

## Logs

`~/.local/state/voice-dictate.jsonl`, one JSON object per dictation:

```json
{"outcome":"ok","asr":0.26,"llm":0.22,"rms":3228.9,"seconds":6.6,
 "raw":"...rerun pass config tests.",
 "final":"...rerun parse_config tests.","changed":true}
```

Enough to tell apart a bad microphone (`rms`), a bad transcription (`raw`),
and a bad rewrite (`final`). Other outcomes: `silence`, `empty`,
`decode_failed`, `record_failed`, `unreachable`.

## Known limits

- **Permission prompts are read from the TUI.** Neither agent logs them to its
  transcript, so detection is pattern-matching on pane text. It handles the
  shapes both agents use today; a redesign upstream would need a tweak here.
- **A freshly spawned agent shows no transcript** until it does something.
  That's correct, not a bug: there's nothing written yet.
- **Codex without a rollout can't be mapped.** Same reason.
- **The terminal view resizes the real window.** tmux keeps one grid per
  pane, so the phone and the desktop cannot each get their own width;
  opening the terminal view fits the window to the phone's screen exactly
  (`resize-window`, which sets `window-size manual`), reflowing it for
  every attached client. The fit is held as a lease -- renewed by the pane
  poll, released on leaving the view, swept server-side within
  ~`AYEAYE_FIT_TTL` seconds if the browser simply dies, and persisted so a
  server restart restores any window it was holding. The desktop always
  gets its window back. Both agent TUIs reflow cleanly, so this is safe to
  round-trip.
- The transcript view shows the last `VOICE_TX_ROWS` entries. There is no
  pagination; scrolling back further means reading the JSONL directly.
- Looking at processes is the only platform-specific part, and all of it is in
  `bin/process_inspect.py`: which agent is behind a pane, and which tmux client
  is at a microphone. The macOS half has been exercised only against canned
  `ps` and `lsof` output; `tests/macos_probe.py` is what to run on a real Mac.

## Layout

```
install.sh                one-command setup: the eight-stage wizard
lib/platform.sh           what this machine is, and who to ask to change it
lib/state.sh              what a setup run remembers between invocations
lib/ui.sh                 how setup talks, and how it listens
lib/consent.sh            permission, and the only wrappers allowed to act
lib/envfile.sh            the settings file: render once, merge ever after
lib/stage.sh              the eight-stage lifecycle and its steps
lib/steps/                work registered onto a stage; see its README
lib/steps/50-access.sh    the four ways to reach ayeaye, and one rule under them
lib/steps/70-service.sh   what a service is, in both platforms' formats
lib/steps/80-health.sh    does what you chose actually work; the closing checks
env.template              every setting, documented; install.sh fills it in
bin/voice-dictate         pipeline: record → whisper → polish → send-keys
bin/voice-agent           recorder daemon for remote tmux clients
bin/process_inspect.py    processes, per platform: /proc, or ps and lsof
bin/ayeaye          web app + API
bin/voice-dictate-setup   prints per-device setup for voice-agent
share/app.html            the entire front end, one file, no build step
share/board.html          cliban issue board at /board, same deal
examples/                 statusline marker, reverse-proxy snippet
tests/run.sh              the whole suite, one command, bash and coreutils only
tests/containers.sh       the same layer against four real Linux distributions
tests/smoke.sh            the whole wizard on a real machine; see tests/smoke-hosts
```

No build, no dependencies beyond the standard library. `app.html` is served
as-is.

`tests/README.md` is the guide to all three. The one thing worth knowing from
outside: **the six real machines the onboarding is supposed to work on have
never been tried on one.** `tests/smoke-hosts` is the record, every line of it
says `no`, and the ordinary suite prints a named skip for each of them so that
the gap is counted rather than quietly absent.
