<p align="center"><img src="assets/readme/mascot.png" width="180" alt="ayeaye"></p>

<h1 align="center">ayeaye</h1>

<p align="center"><b>Your coding agents, in your pocket.</b></p>

ayeaye puts every local Claude Code and Codex session on one phone-sized page.
See who is working, who is waiting, and who delegated. Open a live terminal or
read the clean transcript. Answer prompts, start agents, and dictate replies
without returning to your desk.

<p align="center"><img src="assets/readme/sessions.jpg" width="380" alt="Claude Code and Codex sessions with live status in ayeaye"></p>
<p align="center"><sub>the whole fleet: working · waiting on you · agents working</sub></p>

It is one self-contained Rust binary: web app, agent discovery, terminal
control, transcript rendering, setup, service management, notifications, and
local voice inference. Your sessions and transcripts stay on your machine.

## Install

Linux and macOS, x86-64 and arm64:

```sh
curl -fsSL https://raw.githubusercontent.com/LioraLabs/ayeaye/main/install.sh | sh
```

The installer verifies the release, installs `ayeaye`, and runs setup. `tmux`
is the only required runtime tool. On x86-64 Linux with an NVIDIA driver, it
installs the GPU build with its matching CUDA runtime included; no distro CUDA
toolkit package is required.

```sh
ayeaye setup
ayeaye check
```

Setup normally installs and starts a user service. Open the address it prints,
or run directly with `ayeaye serve`. The default is
`http://127.0.0.1:8911`.

## Run the room from your phone

The sessions view derives live state from the agents and their panes—no agent
plugin or check-in protocol required. Tap any session to move between its real
terminal and a readable, live transcript.

<p align="center"><img src="assets/readme/terminal.jpg" width="380" alt="A live Claude Code terminal controlled from a phone"></p>
<p align="center"><sub>the real terminal: inspect it, type into it, answer prompts</sub></p>

<p align="center"><img src="assets/readme/transcript.jpg" width="380" alt="A formatted Claude Code transcript on a phone"></p>
<p align="center"><sub>the same session as a readable conversation</sub></p>

From the same screen you can:

- see Claude Code and Codex sessions across local tmux panes;
- distinguish working, waiting, delegated, idle, and finished agents;
- answer interactive prompts and send terminal keys;
- find a project and launch a new agent there;
- speak a reply and have local models transcribe and clean it up;
- receive a Web Push notification when an agent needs you, then tap straight
  into that session.

## HTTPS for notifications

Install ayeaye to your phone's home screen, serve it over HTTPS, then tap
**enable notifications**. The watcher runs locally and uses Web Push only when
an agent needs attention.

The shortest HTTPS setup is [Tailscale Serve](https://tailscale.com/kb/1242/tailscale-serve):

```sh
tailscale serve --bg http://127.0.0.1:8911
```

For local TLS, [Caddy](https://caddyserver.com/docs/automatic-https#local-https)
works too:

```caddyfile
ayeaye.localhost {
	reverse_proxy 127.0.0.1:8911
}
```

Plain HTTP still runs the app; browsers simply refuse notifications there.

## Voice

Voice needs `ffmpeg` and separately downloaded models—the binary contains the
inference runtime, not model weights. Setup can search Hugging Face for models
that fit this machine, smoke-test them, and keep the previous configuration if
either role fails:

```sh
ayeaye model choose
```

The same workflow is available as small, scriptable commands. Discovery is
read-only; `add` imports models already on disk; selections are role-specific:

```sh
ayeaye model search whisper
ayeaye model pull openai/whisper-small.en
ayeaye model add ./model.gguf
ayeaye model use speech openai/whisper-small.en
ayeaye model use cleanup local/model@revision
ayeaye model ls
```

Speech models use the supported Whisper/Safetensors layout. Cleanup models use
a supported GGUF plus its tokenizer. Hub revisions are pinned, and downloads,
imports, and configuration changes are staged before becoming active.

The phone records in the web app. For dictation from another SSH client,
`bin/voice-dictate-setup` prints the client-side setup and `bin/voice-agent`
captures that device's microphone. Transcription and cleanup still happen
inside ayeaye on your machine.

## Configuration

```text
ayeaye setup [--yes] [--no-service] [--no-model]
ayeaye check
ayeaye service <install|repair|enable|disable|start|stop|status|remove>
ayeaye model search [QUERY]
ayeaye model choose [speech|cleanup]
ayeaye model <ls|pull ID|add PATH|use [speech|cleanup] ID|rm ID>
ayeaye dictate <host/pane> [client-pid]
```

Run `ayeaye --help` for all settings. Configuration lives at
`~/.config/ayeaye/env` by default; the access token lives at
`~/.local/state/ayeaye/token`. `XDG_CONFIG_HOME` and `XDG_STATE_HOME` are
respected.

## Build

Rust 1.88 or newer:

```sh
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
```

MIT licensed.
