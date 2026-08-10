# Reading the health report

`ayeaye check` (and the end of `ayeaye setup`) prints one line per capability
under four marks, never collapsed into fewer:

```
ok       it was checked and it works
FAILED   it was checked and it does not
skipped  not part of what you asked for — nothing to check
unknown  part of what you asked for, and setup could not tell
```

`skipped` and `unknown` are different facts and neither is ever rendered as a
pass. The exit code is the verdict: **0** nothing outstanding, **1** something
failed or could not be checked, **2** the lock is off. Triage in that order:
exit 2 first, then each `FAILED`, then each `unknown`. After every repair, run
`ayeaye check` again — the report is the proof, not the repair.

## Exit 2: the lock is off

A request carrying no key was answered in full. Anybody who can reach that
address can drive the coding agents on this computer. Tell the user to keep
the page closed until this is settled, then find which of the two causes it
is:

1. **Something else is listening on that port.** `ss -tlnp | grep 8912` (or
   `lsof -i :8912` on a Mac) names the process. If it is not ayeaye, the check
   graded a stranger; move one of them.
2. **The key has been switched off.** Restart it clean — `ayeaye service
   stop`, `ayeaye service start`, `ayeaye check` — and if it still answers
   without the key, read what is actually running before going further.

## The checks, one by one

**service** — *ayeaye starts when you log in.*
`FAILED`: the manager would not report it running — stopped, crashed, or
never installed. `ayeaye service status` prints the manager's own words;
`ayeaye service repair` rewrites the definition and restarts a running
service. `skipped`: this machine has no user
service manager — not broken; `ayeaye serve` by hand is a supported life, and
`ayeaye service status` prints the exact commands for it. `unknown`: a manager
that could not be addressed (a Mac whose launchd domain would not answer).

**tmux** — *what ayeaye reads your agents through.*
`FAILED`: not on PATH. The detail line is the install command for this
platform; installing a package is the user's consent to get, then theirs or
yours with a yes. Nothing else works until this does.

**acceleration** — *what this build can use of what this machine has.*
`FAILED`: a usable card beside a build not compiled for it — everything works
but transcription runs on the processor and is mysteriously slow. The fix is
the matching artifact (the CUDA build for NVIDIA), never a setting. `skipped`
with a reason: the card cannot be used (AMD — candle has no ROCm backend — or
too small); that is the machine's ceiling, said plainly. `unknown`: the card
would not say how much memory it has.

**local** — *ayeaye answers on its own address.*
`unknown`: nothing answered at all — the daemon is not up (`ayeaye service
start`, or `ayeaye serve` to watch it start and read its complaint), or the
check dialed an address nothing listens on: compare `AYEAYE_BIND` /
`AYEAYE_DEV_PORT` between environment and `~/.config/ayeaye/env`, remembering
environment wins. `FAILED`: something answered with an error status — read the
code in the detail line, and suspect a stranger on the port.

**auth** — *ayeaye refuses anyone without your key.*
`ok` here means the request was refused with 401 — refusal is the pass. A full
answer instead is the exit-2 case above. `unknown` with 403: the host gate
refused before the lock could be tried; fix `hosts` first and this resolves.

**authorised** — *your key opens the page.*
The complement of **auth**: that one proves the lock is on, this proves it is
*your* lock. `FAILED`: the key in the state file and the key the daemon holds
disagree — the classic cause is a token file rewritten while a daemon was
already running. Restart (`ayeaye service stop`, `start`) so the daemon
rereads it. `unknown`: no key on this computer to try — `ayeaye setup` mints
one.

**hosts** — *the named addresses answer, and a stranger is refused.*
The check dials ayeaye's own address with forged Host headers, so both
directions grade the **daemon's** host gate, not the proxy — and only the
second direction proves anything: a gate accepting every name passes the
first half perfectly. `FAILED`'s detail says which broke. A stranger being
answered is the classic sign of a daemon started before
`AYEAYE_ALLOWED_HOSTS` was written — restart (`ayeaye service stop`,
`start`). A named host refused means the file and the name the proxy serves
disagree — make them match. `skipped`: no hosts configured, nothing claimed.

**https** — *the address the phone opens answers.*
A 401/403 refusal counts as `ok` — it proves the address reaches ayeaye,
which is all this check claims. `FAILED`: the address answered with a bad
status, a proxy's own 502 being the classic — the front end is up and its
route to ayeaye is not. `unknown` is either nothing answering at all
(certificate refused, proxy down, DNS) or an exposed bind with no host named
in `AYEAYE_ALLOWED_HOSTS`, so there was no address to dial — name one, or
return to loopback.

**mesh** — *the mesh network you reach this machine over.*
`FAILED`: tailscale is installed and its own status says down — `tailscale
up`, run by the user (it changes their network membership). `skipped`: not
installed, and not a fault; a mesh is one placement among several.

**board** — *the ticket board on the phone.*
`FAILED`: the app answered and its answer held no projects — the daemon may
predate cliban's arrival (restart it), or `AYEAYE_CLIBAN` points somewhere
stale. `unknown`: the request could not be made at all — no key here, or the
daemon is down; fix **local** and **authorised** first and this resolves.
`skipped`: no cliban here, and the board tab simply stays off.

**agents** — *a coding agent for ayeaye to show you.*
`FAILED`: neither claude nor codex is on PATH — there is nothing to put in
the panel. Installing one is the user's call; ayeaye detects it at runtime, no
re-setup needed.

**every network check `unknown` at once, "no curl"** — the checks ask over
HTTP through curl, and this machine has none. Install curl with the platform's
package command; nothing is wrong with ayeaye itself, and nothing has been
verified either.
