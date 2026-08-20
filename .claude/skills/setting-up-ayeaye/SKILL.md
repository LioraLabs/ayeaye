---
name: setting-up-ayeaye
description: Set up and troubleshoot ayeaye by driving the binary's own verbs. Use when installing ayeaye on a machine (including one that has never had it), deciding where it sits on the network and what may reach it, pointing it at the llama-swap that serves its speech and cleanup models, or reading why `ayeaye check` reported a failed health check.
---

# Setting up ayeaye

Walk a person through the judgement calls of an ayeaye install, and drive the
same commands they would type. Two ground rules govern every step:

- **Only the binary's own verbs**, plus read-only probes (`command -v`,
  `tailscale status`). There is no privileged path here: every command is one
  the user could run by hand, and anything consequential is shown before it
  runs.
- **Consent is relayed, never assumed.** Run from a shell, `ayeaye setup` has
  no terminal to ask on, so it declines the one act with a consequence —
  starting a login service — and prints the exact by-hand command under "not
  done". Lean on that: run setup bare, put its
  question to the user in conversation with what it means, and run the printed
  command (or re-run with `--yes`) only after the user has said yes to that
  specific act. Choosing models is *not* gated: it downloads nothing, and it is
  itself a question, so it simply reports that it had nobody to ask.

The binary *detects and verifies* network exposure, reverse proxies, mesh
networks, coding agents, and tmux — it configures none of them. This skill
keeps the same boundary: the user's network and packages change only by the
user's hand or explicit say-so, and every file edit is shown first.

## The flow

1. **Is ayeaye here at all?** `command -v ayeaye`. On a machine that has never
   had it, offer the README's one-line installer, or fetch the artifact for
   this OS and architecture from the GitHub releases page and put it on PATH.
   There is one artifact per OS and architecture — no acceleration variants,
   because this binary runs no model. Speech and cleanup happen in a
   [llama-swap](https://github.com/mostlygeek/llama-swap) the user runs
   themselves, and the graphics card is that process's business.
2. **`ayeaye setup`**, bare. Read what it prints back to the user:
   - the machine summary — the operating system, what installs software here,
     and where a user service can live. **Not a hardware verdict.** There used
     to be a tier — `text-only` through `maximum` — computed from memory,
     cores, disk and the graphics card; it went with the models, because it was
     only ever answering "which speech model fits here" and the models are not
     here;
   - what it did: key minted, settings file written (`~/.config/ayeaye/env`),
     models chosen, service definition written;
   - the "not done" list: the consent question, with its by-hand command, and
     any step that could not finish — a backend that was not running says so
     here.
3. **Relay consent.** For each declined step, say what it costs — the service
   runs whenever they log in — and on a yes, run the printed command
   (`ayeaye service enable`).
4. **`ayeaye check`** to finish, and after every later change. Exit 0 is done;
   exit 1 means something is unfinished; exit 2 means the lock is off — stop
   and read [references/health-checks.md](references/health-checks.md) before
   anything else. That file triages every `FAILED` and `unknown` line.
5. **Placement.** When the question is who else should reach this machine —
   a phone, another computer, teammates — read
   [references/network-placement.md](references/network-placement.md) and have
   that conversation before touching any setting.

Setup is re-runnable and keeps hand-edits to the settings file, so "run it
again and read the report" is always a safe move.

## Choosing a model

**ayeaye does not download, store, size or run models.** It asks a llama-swap
what it is serving and calls two of them by name. So the questions this section
used to answer — which model fits this machine's RAM, whether the architecture
is supported, how big the download is — all belong to whoever configured
llama-swap. ayeaye no longer even measures the memory, disk or graphics card it
would have needed to answer them.

What is left here:

- **Is the backend reachable and serving both parts?** `ayeaye check`'s
  **backend** line answers exactly that. A `FAILED` there names the models the
  proxy does not have; `unknown` means nothing answered at the address.
- **Which model plays which part.** `ayeaye model ls` lists what the backend
  serves and marks the two in use. `ayeaye model choose` walks both roles and
  smoke-tests each before writing anything down; `ayeaye model use speech NAME`
  sets one. The names are the keys in llama-swap's `config.yaml`, not
  `owner/name` ids.
- **Where the backend is.** `AYEAYE_LLAMA_SWAP`, defaulting to
  `http://127.0.0.1:8080`. `https://` and a path prefix both work, and a
  backend on another machine is an ordinary setup.
- **A speech model has to be one.** The proxy will happily list a language
  model, and nothing but the smoke test can tell that it is the wrong kind —
  which is why `choose` runs one. On the llama-swap side a speech model is
  whisper.cpp's `whisper-server` with `--request-path /v1/audio/transcriptions
  --inference-path ""`.
- **Cleanup is optional.** Without `AYEAYE_CLEANUP_MODEL` dictation types the
  raw transcript, which is what it degrades to when the model is unreachable
  anyway. That is a worse dictation, not a broken one.

## Where settings live

Setup writes `~/.config/ayeaye/env`; the daemon and `ayeaye check` both read
the environment first (`AYEAYE_*`), then that file, then defaults — so an edit
to the file is the durable move, and an `AYEAYE_*` variable wins over it.
After changing bind, port, or hosts, restart the service (`ayeaye service
stop`, then `start`) so the daemon rereads the file, and verify with
`ayeaye check`. The full variable list is in `ayeaye --help`.
