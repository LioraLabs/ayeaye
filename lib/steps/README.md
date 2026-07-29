# Adding work to setup

`install.sh` owns the lifecycle: eight stages, in order, resumable, and one
place where permission is asked. It does not own the work inside them. A
change that adds work to setup adds a file here instead of editing
`install.sh`.

Every `*.sh` in this directory is sourced by `wizard_load_steps`, in filename
order, after the core steps are registered and before the walk begins. A
numeric prefix is how a step chooses where it lands inside its stage.

## The shape of a step file

```sh
# lib/steps/30-hardware.sh
#
# One sentence saying what this adds and which ticket owns it.

detect_hardware_step() {
  wizard_item "graphics" "…"
  return "$WIZARD_STAGE_OK"
}

wizard_step detect hardware detect_hardware_step "What this computer has"
```

`wizard_step <stage> <step> <function> <label> [policy] [resume]`

| | |
| --- | --- |
| `stage` | one of `welcome detect report configure plan install service finish` |
| `step` | a name unique within that stage, `[A-Za-z0-9_-]+` |
| `function` | must already be defined when `wizard_step` runs |
| `label` | what a person reads when it is skipped or when it fails. No tabs, no newlines |
| `policy` | `required` (default) — a failure stops the run · `optional` — a failure is reported and the run carries on |
| `resume` | `once` (default) — remembered, and not repeated by a resumed run · `always` — runs every time and records nothing |

Registration is checked eagerly. A stage that does not exist, a step name
already taken, or a function that is not defined all return 2 and the run stops
before it has printed anything, rather than two stages in.

## What a step answers

| | | |
| --- | --- | --- |
| `$WIZARD_STAGE_OK` | 0 | the work is finished |
| `$WIZARD_STAGE_PENDING` | 10 | it ran, and work remains |
| `$WIZARD_STAGE_SKIP` | 11 | it does not apply here, or the user said no |
| `$WIZARD_STAGE_FAIL` | 12 | it tried and could not |

Anything else non-zero is read as a failure, so a step that forgets to say what
happened is treated as having failed rather than as having succeeded.

**`PENDING` is what an unfinished seam returns.** A stage holding a pending step
is never recorded as `done`, it is listed under "not finished, and worth coming
back to" in the closing summary, and the next run picks it up. Returning `OK`
for work that did not happen is the one thing a step must never do.

## What a step may read

| | |
| --- | --- |
| `WIZARD_STAGE_ID` | the stage it is running in |
| `WIZARD_STEP_ID` | `<stage>.<step>` — key any state of your own under this |
| `WIZARD_INTERACTIVE` | 1 when questions may be asked, 0 when they may not |
| `WIZARD_RESUMING` | 1 when this run is picking up an interrupted one |

Anything one stage tells a later stage goes through `wizard_remember`, never a
shell variable: a resumed run skips the step that set the variable, and the
later stage would read its default instead of the decision.

## Talking and asking

    wizard_say <text>…                one line of plain language
    wizard_blank                      one empty line
    wizard_head <title>               a heading
    wizard_item <mark> <text>         a checklist line, "  ok       tmux"
    wizard_detail <text>…             a raw command or raw output: logged
                                      always, shown only with --details
    wizard_ask <prompt> <default>     -> $REPLY. Always 0.
    wizard_confirm <prompt> <default> status 0 for yes

Jargon in a prompt is a defect. The audience has never used a terminal: a
question names the thing in the words they already have, and never a unit name,
a flag or a path they did not choose. Raw commands go to `wizard_detail`.

`wizard_ask` renders `<prompt> [<default>]: ` and takes an empty answer as the
default. When the run may not ask it takes the default without reading, so
nothing on standard input can steer an unattended run.

## Doing anything to the machine

**Nothing privileged runs, nothing is downloaded, no firewall is opened, no
certificate is trusted and no existing file is replaced except through one of
these.** They ask first, and do nothing whatsoever when refused.

    wizard_privileged <question> <command-string>
    wizard_download   <question> <url> <destination> [bytes]
    wizard_firewall   <question> <command-string>
    wizard_trust      <question> <command-string>
    wizard_may_expose <question> [detail]…
    wizard_replace    <path> [question]     0 when the caller may now write
    wizard_install_packages <logical>…      the package layer, with consent
    wizard_backup     <path>                the backup path on stdout

    0  done
    1  it was allowed to run and it did not work, or this machine cannot
    2  the call was wrong. A bug in the caller.
    3  refused. Nothing happened, and nothing is wrong.

`platform_pkg_install` and `platform_service_run` change the machine and are
**not** wrappers. Do not call them: use `wizard_install_packages`, and build a
service command with `platform_service_command` and run it through a wrapper.
`tests/cases/wizard_contract_test.sh` fails the suite if you do.

Whatever a step is about to do, add it to the plan in stage four or earlier so
that stage five can say so before it happens:

    wizard_plan_add <category> <text> [bytes]
    # package download privileged network trust config service

## Remembering

    wizard_remember <key> <value>          persist. Always 0.
    wizard_state_get <key> [default]       read it back. Always 0.

Use `answer.<name>` for something the user chose and `step.<stage>.<step>.<x>`
for a step's own bookkeeping. Never write into `run.*`, `stage.*` or the bare
`step.<stage>.<step>` key — those belong to the lifecycle.

## Testing a step

`tests/README.md` is the guide. Two things specific to steps:

- **Add every new path your step writes to `GUARDED_PATHS` in `tests/run.sh`,
  in the same commit.** The suite declares a run void if anything on that list
  changes, and a path that is not on it is a path a test can escape through.
- A prompt is only emitted to a terminal, so anything about a question needs
  the pty driver: `pty_expect`, `pty_answers`, `pty_install`.
