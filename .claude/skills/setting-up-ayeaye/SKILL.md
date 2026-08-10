---
name: setting-up-ayeaye
description: Set up and troubleshoot ayeaye by driving the binary's own verbs. Use when installing ayeaye on a machine (including one that has never had it), deciding where it sits on the network and what may reach it, choosing which speech model suits this machine or its graphics card, or reading why `ayeaye check` reported a failed health check.
---

# Setting up ayeaye

Walk a person through the judgement calls of an ayeaye install, and drive the
same commands they would type. Two ground rules govern every step:

- **Only the binary's own verbs**, plus read-only probes (`command -v`,
  `tailscale status`). There is no privileged path here: every command is one
  the user could run by hand, and anything consequential is shown before it
  runs.
- **Consent is relayed, never assumed.** Run from a shell, `ayeaye setup` has
  no terminal to ask on, so it declines both acts with a consequence —
  downloading a model, starting a login service — and prints the exact by-hand
  command under "not done, because you did not ask for it". Lean on that: run
  setup bare, put its questions to the user in conversation with what each
  means, and run the printed command (or re-run with `--yes`) only after the
  user has said yes to that specific act.

The binary *detects and verifies* network exposure, reverse proxies, mesh
networks, coding agents, and tmux — it configures none of them. This skill
keeps the same boundary: the user's network and packages change only by the
user's hand or explicit say-so, and every file edit is shown first.

## The flow

1. **Is ayeaye here at all?** `command -v ayeaye`. On a machine that has never
   had it, offer the README's one-line installer, or fetch the artifact for
   this OS and architecture from the GitHub releases page and put it on PATH.
   Builds differ by acceleration: a machine with a usable NVIDIA card wants the
   CUDA build, a Mac gets Metal in the Apple artifacts, and everything else the
   static CPU build. Step 4's acceleration check is what catches a mismatch.
2. **`ayeaye setup`**, bare. Read what it prints back to the user:
   - the machine summary and its **tier verdict** — `text-only`,
     `lightweight`, `recommended`, or `maximum` — with the reason line naming
     the constraint that held it back;
   - what it did: key minted, settings file written (`~/.config/ayeaye/env`),
     service definition written;
   - the "not done" list: the consent questions, each with its by-hand command.
3. **Relay consent.** For each declined step, say what it costs — the model
   download's size goes over the network; the service runs whenever they log
   in — and on a yes, run the printed command (`ayeaye model pull <id>`,
   `ayeaye service enable`).
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

The tier verdict already accounts for RAM, disk, cores, and the card; its
suggested model (printed by setup) is the right default. The judgement the
verdict cannot make:

- **Language.** The suggested `.en` models are English-only. Someone dictating
  another language wants the multilingual twin — drop the `.en`
  (`openai/whisper-small`), or `openai/whisper-large-v3-turbo` at the top
  tier. Model IDs are `owner/name`, and the architecture allowlist is checked
  at `pull` time, so trying one is cheap: an unsupported model is refused
  before the weights download, never at first inference.
- **The constraint names the fix.** A tier held back by `disk` rises after
  freeing space where models land and re-running setup; `ram` or `cores` is
  the machine's ceiling. Read the reason line rather than guessing.
- **A slow machine that wants accuracy anyway** can hold a bigger model than
  its tier suggests — say out loud that transcription latency is the price,
  then `ayeaye model pull <id>` and `ayeaye model use <id>`.
- **The card is not a model choice.** A CPU build beside a usable NVIDIA card
  is reported by the acceleration check as `FAILED` — the fix is the CUDA
  build artifact, not a smaller model. An AMD card runs on the processor
  (candle has no ROCm backend); that is the machine's ceiling, reported as
  skipped with its reason, and no setting changes it.

`ayeaye model ls` shows what is installed and which model is in use;
`ayeaye model rm <id>` frees the space of one that did not work out.

## Where settings live

Setup writes `~/.config/ayeaye/env`; the daemon and `ayeaye check` both read
the environment first (`AYEAYE_*`), then that file, then defaults — so an edit
to the file is the durable move, and an `AYEAYE_*` variable wins over it.
After changing bind, port, or hosts, restart the service (`ayeaye service
stop`, then `start`) so the daemon rereads the file, and verify with
`ayeaye check`. The full variable list is in `ayeaye --help`.
