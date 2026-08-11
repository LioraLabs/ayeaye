# ayeaye

See every local Claude Code and Codex session from one phone-sized page, open
their transcripts, answer prompts, start agents, and dictate into tmux panes.

The server is one Rust binary. It contains the web app, setup, configuration,
service management, process inspection, speech transcription, and cleanup
inference. The server needs no Python interpreter, C++ inference runtime, or
files beside the executable.

## Install

Linux and macOS builds are published for x86-64 and arm64:

```sh
curl -fsSL https://raw.githubusercontent.com/LioraLabs/ayeaye/main/install.sh | sh
```

The downloader verifies the release checksum, installs `ayeaye` in
`~/.local/bin`, then runs `ayeaye setup`. Setup writes the configuration and
token, can install the user service, and reports anything still missing. It is
safe to run again after an upgrade:

```sh
ayeaye setup
ayeaye check
```

The only required runtime tool is `tmux`. Voice additionally needs `ffmpeg` to
turn a recording into audio samples. A model is downloaded separately
because the binary ships inference, not model weights:

```sh
ayeaye model pull openai/whisper-small.en
ayeaye model use openai/whisper-small.en
```

## Run

Setup normally installs and starts a systemd user service on Linux or a launchd
agent on macOS. To run it directly:

```sh
ayeaye serve
```

The default address is `http://127.0.0.1:8911`. Open it once as
`http://127.0.0.1:8911/?token=<token>`; the browser stores the token locally.
The token is in `$XDG_STATE_HOME/ayeaye/token`, or
`~/.local/state/ayeaye/token` when `XDG_STATE_HOME` is unset.

Useful commands:

```text
ayeaye setup [--yes] [--no-service] [--no-model] [--model ID]
ayeaye check
ayeaye service <install|repair|enable|disable|start|stop|status|remove>
ayeaye model <ls|pull ID|use ID|rm ID>
ayeaye dictate <host/pane> [client-pid]
```

Run `ayeaye --help` for every flag and environment setting. Configuration lives
in `$XDG_CONFIG_HOME/ayeaye/env`, or `~/.config/ayeaye/env` by default.
Environment variables override that file; the common ones are `AYEAYE_BIND`,
`AYEAYE_PORT`, `AYEAYE_ALLOWED_HOSTS`, and `AYEAYE_TOKEN`.

## Voice from another device

`bin/voice-agent` is the one deliberate exception to the server's one-binary
shape. It records on the client device you SSH from, not on the server, and is
therefore not installed by `ayeaye setup`. `bin/voice-dictate-setup` prints the
client-side setup instructions. Server-side transcription and cleanup still run
inside the `ayeaye` binary.

Bind dictation in tmux with a qualified pane id:

```tmux
bind -n M-v run-shell -b "ayeaye dictate #{host}/#{pane_id} #{client_pid}"
```

## Development

Rust 1.88 or newer is required to build:

```sh
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
```

`cook test` runs the same cached release gates. `cook dist` builds the source
archive and checksums; tagged releases build the platform binaries in CI.

License: MIT.
