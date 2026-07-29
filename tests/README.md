# Tests

A shell test suite with no dependencies beyond bash, coreutils and python3 —
all three of which ayeaye already requires. No bats, no npm, no cargo, nothing
to install.

```sh
tests/run.sh                      # everything
tests/run.sh install_env          # only tests whose id contains "install_env"
tests/run.sh tests/cases/install_env_test.sh
tests/run.sh --list               # print test ids, run nothing
tests/run.sh -v                   # show output of passing tests too
tests/run.sh --timeout 30         # per-test watchdog, seconds
```

A test id is `<case-file-without-.sh>::<test-function>`, for example
`install_env_test::test_a_second_run_keeps_the_config`. Filters are substrings
matched against that id, and several may be given. That is also how a subset is
run inside a container:

```sh
docker run --rm -v "$PWD:/repo:ro" -w /repo debian:12 bash tests/run.sh install_args
```

Read-only is deliberate: a suite that needs to write into the checkout is a
suite that is writing where it should not. A filter that matches nothing is an
error, not an empty pass.

The suite exits 0 only when every selected test passed or was skipped.

Two images are worth running against, and both have caught bugs the development
host could not:

```sh
# bash 3.2 with a busybox userland - what macOS portability actually means
docker run --rm -v "$PWD:/repo:ro" -w /repo bash:3.2 \
  bash tests/run.sh harness_ install_args

# everything, on a machine that does not export USER and has no desktop
docker run --rm -v "$PWD:/repo:ro" -w /repo python:3.12-slim bash tests/run.sh
```

The first image has no python3, so the tests that need one — the pty driver and
anything that runs the installer — report themselves as skipped rather than
failing. `require_host_command python3` is what does that, and it is the right
tool for any coverage that cannot run on the machine it finds itself on.

## Container checks

`tests/containers.sh` is a separate, opt-in runner. It goes into real
`debian`, `fedora`, `archlinux` and `opensuse/tumbleweed` images and does two
things there.

```sh
tests/containers.sh              # all four images: what it says, and what it does
tests/containers.sh arch fedora  # by substring
tests/containers.sh --list
tests/containers.sh --quick      # only the questions; install nothing
tests/containers.sh --suite      # also run the unit tests inside each image
tests/containers.sh -v           # print every probed value
```

**`tests/lib/platform_probe.sh` only asks questions.** It sources the platform
layer and asserts that the family, package manager, distro id and package
queries match a machine rather than a fixture. Nothing is installed: the
repository is mounted read-only and the commands the layer would run are
asserted as strings.

**`tests/lib/install_probe.sh` really installs.** It calls
`wizard_install_packages` — the same door `install.sh` goes through — for
tmux, python3, curl and tar, and then checks that each program is on `PATH`
*and* that the package database agrees. That is the coverage a stub cannot
give: that the command generated for a family is one that family's package
manager accepts, and that the name table names packages that exist. It runs
inside a container that is thrown away when it exits, and the repository is
still mounted read-only. `--quick` leaves it out; do not run it on a laptop.

It is not part of `tests/run.sh` on purpose — the fast suite stays runnable
with nothing but bash, coreutils and python3. With no container engine it says
so and exits 0, because "could not check" is not "found a problem". The
tumbleweed image ships neither `find` nor `python3`, so `--suite` skips it and
reports why; both probes still run there, and the install probe is in fact how
python3 gets there.

`tests/lib/platform_probe.sh` is what it runs inside each image, and it is also
the quickest way to see what the platform layer makes of your own machine.
`tests/lib/service_probe.sh` is the second one, and it answers a different
question: what a real distro's own `systemd-analyze verify` makes of the unit
files this project generates. No golden file can tell you that.

A container cannot tell you everything, and the runner says which parts out
loud rather than passing them quietly. There is no user session bus in any of
these images, so nothing there proves a unit can be enabled or started; that
is printed as a named `skip` for every image, and the summary counts skips
separately. launchd is not covered at all — there is no Mac — so the agents
are pinned by golden file and by `plistlib` and nothing else.

## Adding a test

Create `tests/cases/<area>_test.sh`. Every function named `test_*` in it is a
test. There is no boilerplate, no registration and no `main`:

```sh
# What this file pins, in a sentence.

test_the_port_is_written_to_the_config() {
  stub_command tmux
  stub_real python3
  run_install --defaults --no-systemd
  assert_status 0 "$RUN_STATUS"
  assert_file_contains "$XDG_CONFIG_HOME/ayeaye/env" "AYEAYE_PORT=8911"
}
```

Rules of the road:

- **A case file defines functions and nothing else.** Discovery sources it to
  find out which functions really exist, before any test has run — a top-level
  statement executes there, outside the per-test sandbox. Discovery has a
  scratch home of its own so a stray statement cannot reach your machine, but it
  still has no stubs, no fixtures and no `$TEST_TMPDIR`. Put setup in `setup`.
- Name a test after the behaviour it pins, as a sentence. `test_a_missing_tmux_stops_the_install`
  reads better in the output than `test_deps_2`.
- Tests run in source order, one process each. Nothing leaks between them: not
  variables, not functions, not the working directory, not `PATH`, not files.
- Optional `setup` and `teardown` functions run around each test in the file.
  `teardown` runs even when the test fails.
- `skip "reason"` ends a test as skipped rather than failed — the way to mark
  coverage that only makes sense on another platform or inside a container.
- There is no `set -e`. A command that fails mid-test does not end it; assert
  on the thing you care about, and capture exit status explicitly with
  `run_script`.
- There *is* `set -u`. A misspelled variable is an error, not an empty string,
  so `assert_eq "" "$RUN_STDOTU"` fails loudly instead of passing.
- An assertion that fails inside a subshell — a pipeline, a command
  substitution — still fails the test. `exit` alone could not do that, so the
  failure is also recorded on disk and the runner acts on it. If you mean to
  capture a failure on purpose, run it with `ASSERT_EXPECT_FAILURE=1`.

## Assertions

Every one takes an optional last argument, a message explaining what the
assertion is really about. Use it whenever the assertion is not self-evident.
A failure prints the assertion, the file and line, and the expected and actual
values in full — multi-line values are shown as indented blocks, never
truncated.

| Assertion | Checks |
| --- | --- |
| `assert_eq <expected> <actual>` | exact string equality |
| `assert_ne <not-expected> <actual>` | inequality |
| `assert_contains <haystack> <needle>` | substring |
| `assert_not_contains <haystack> <needle>` | absence of a substring |
| `assert_matches <string> <extended-regex>` | `grep -E` match |
| `assert_status <expected> <actual>` | exit code |
| `assert_file_exists` / `assert_file_missing` | presence on disk |
| `assert_file_contains` / `assert_file_not_contains` | file body substring |
| `assert_file_mode <octal> <path>` | permissions, e.g. `600` |
| `assert_fixture_exists <category/name>` | a fixture is present |
| `assert_stub_called <name>` | a stubbed command ran at least once |
| `assert_stub_not_called <name>` | it never ran |
| `assert_stub_called_with <name> <argv-substring>` | it ran with these arguments |
| `assert_stub_call_count <name> <n>` | it ran exactly n times |
| `assert_command_absent <name>` | it cannot be found on `PATH` at all |
| `fail <message>` | end the test as a failure |

## Isolation: what a test may touch

Every test gets a fresh `$TEST_TMPDIR`, removed when it ends. Before the first
line of the test runs, everything that would otherwise resolve to your real
home directory is redirected inside it:

    HOME  XDG_CONFIG_HOME  XDG_STATE_HOME  XDG_DATA_HOME  XDG_CACHE_HOME
    XDG_RUNTIME_DIR  TMPDIR

`AYEAYE_*` and `VOICE_*` variables are scrubbed from the environment, and the
locale is pinned, so a contributor with `AYEAYE_PORT` exported gets the same
results as a clean machine. `USER` and `LOGNAME` are pinned too — a container
shell does not export `USER` and a desktop one does, which by itself is enough
to make a test pass on a laptop and fail in CI. A test that cares about that
difference says `unset USER` and means it.

**A test may write anywhere under `$TEST_TMPDIR` and nowhere else.** The runner
enforces it: it takes a signature of every path an install writes or could
plausibly write (`~/.config/ayeaye/env`, the systemd unit, the token, the setup
state file, the consent ledger, the setup log and the backup directory, plus
`~/.local/bin/ayeaye`, a launchd plist and the shell rc files) before and after
the run, and if any of them changed it declares the whole run void no matter
what the assertions said. **When the wizard learns to write somewhere new, add
it to `GUARDED_PATHS` in `tests/run.sh` in the same commit** —
`wizard_contract_test.sh` fails if a known setup path is missing from it.

`KEEP_TMPDIR=1 tests/run.sh <filter>` leaves the sandbox in place and prints
its path, which is how you look at what a failing test actually wrote.

Useful paths: `$REPO_ROOT`, `$TESTS_DIR`, `$FIXTURES_DIR`, `$TEST_TMPDIR`,
`$STUB_BIN`, `$STUB_LOG_DIR`.

## Command stubs

**`PATH` inside a test is exactly `$STUB_BIN`.** A command that was not stubbed
and not deliberately linked in is genuinely absent, whatever the host has
installed. That is what makes detection testable: a test asserting "tailscale
is not installed" gives the same answer on a laptop that has it and in a bare
container.

`PATH` really is bare, and that applies to the test's own helper code as much as
to the code under test: `stat`, `readlink`, `tee`, `xargs`, `diff`, `seq`, `tar`,
`curl` and `getent` are all absent until you ask for them. If a test dies with
"command not found", `stub_real <name>` is the answer.

Common coreutils are linked in automatically. Deliberately *not* linked:
`python3`, `tmux`, `uname`, `systemctl`, `tailscale`, package managers —
anything a detection step is supposed to probe. Ask for what you need:

```sh
stub_command tmux            # a recording shim, exit 0, no output
stub_real python3            # the host's real python3, because the installer runs it
```

Add a stub:

```sh
stub_command apt-get --exit 0 --stdout "Reading package lists..."
stub_command nvidia-smi --exit 1 --stderr "command not found"
stub_command_from_fixture uname uname/darwin-arm64
stub_remove python3                            # make a command absent again
```

Scripted responses per argument pattern, tried in the order declared, with the
`stub_command` response as the fallback:

```sh
stub_command systemctl --exit 1
stub_when systemctl '--user show-environment' --exit 0
stub_when systemctl 'enable --now *' --exit 0 --stdout "started"
```

Anything more involved gets a body of its own:

```sh
stub_script tailscale <<'SH'
case "$*" in
  "status --json") cat "$FIXTURES_DIR/tailscale/status-online" ;;
  *) exit 1 ;;
esac
SH
```

`require_host_command python3` skips the test when the machine cannot supply
something the code under test genuinely needs.

Every invocation of every stub is recorded, one line per call, as the command
name followed by its argv. An argument containing whitespace, a quote or a
non-printable character is single-quoted, so a recorded line re-parses to the
arguments that were really passed:

```sh
run_install --defaults
assert_stub_called_with systemctl "systemctl --user daemon-reload"
assert_stub_not_called systemctl                  # nothing was started
stub_calls systemctl                              # every call, in order
stub_call systemctl 2                             # just the second one
```

**This is how generated commands are tested.** Assert on what the installer
*would* run; never run it. Two things to know:

- A stub records what was *executed*. `command -v foo` looks a command up
  without running it and records nothing.
- `assert_stub_called_with` matches within a single recorded call, so a needle
  cannot span two of them.

## Fixtures

Canned bytes standing in for what a real command or system file produces.
Layout is `tests/fixtures/<category>/<scenario>`, where the category is the
command or file being canned:

```
tests/fixtures/os-release/ubuntu-24.04
tests/fixtures/uname/darwin-arm64
tests/fixtures/tailscale/status-online
```

Names carry no extension: the repository's `.gitignore` drops `*.log` and a
file named `token`, and extensionless names sidestep all of it. The content is
exactly what the command writes, nothing else — no comments, no headers.

Add one by dropping the file in. Use it with:

```sh
stub_command_from_fixture lscpu lscpu/x86_64-8core   # as a command's stdout
fixture_path os-release/debian-12                    # absolute path
fixture_cat os-release/debian-12                     # its bytes
fixture_file os-release/debian-12                    # copy into the sandbox, echo the path
fixture_list os-release                              # the scenarios in a category
```

A missing fixture complains on stderr, names the path it looked for, lists what
the category does contain, and returns a path that cannot exist — so whatever
consumes it fails with the name in the message.

For **file-shaped input** such as `/etc/os-release`, `fixture_file` materialises
the fixture in the sandbox and prints where it landed. Code under test must
therefore take such paths from an overridable variable rather than hard-coding
`/etc/os-release`; that is the one thing the platform adapters have to design
for in order to be testable.

## Prompt flow

Bash writes a `read -p` prompt **only when standard input is a terminal**.
Under a pipe the prompt text is never emitted anywhere, so a piped run can
prove what a script did but never what it asked. There are therefore two ways
to run the code under test.

A pipe, for everything that is not about prompts:

```sh
stdin_lines "0.0.0.0" "9000"        # optional; queued for the next run only
run_script "$REPO_ROOT/install.sh" --defaults
run_install --defaults              # the same thing, shorter

assert_status 0 "$RUN_STATUS"
assert_contains "$RUN_STDOUT" "wrote"
assert_eq "" "$RUN_STDERR"
# also: $RUN_OUTPUT (both streams), $RUN_STDOUT_FILE, $RUN_STDERR_FILE
```

A real terminal, whenever a prompt, a default or the order of questions is the
thing under test:

```sh
pty_answers "" "9000" "" ""                       # answer each prompt in turn
pty_expect "enable and start it now?" "y"         # or wait for a specific one
pty_install --no-systemd                          # or pty_run <script> [args...]

assert_contains "$PTY_TRANSCRIPT" "port [8911]: 9000"
assert_status 0 "$PTY_STATUS"
```

Use `pty_await <substring>` to wait for a milestone that is not a question —
"Detecting dependencies…", "wrote ~/.config/ayeaye/env". It types nothing;
`pty_expect` with an empty answer would send a newline, and that newline would
be swallowed by whatever asks the next question.

`pty_expect` waits for its substring to appear before typing its answer, which
is what keeps the transcript deterministic — no answer can be typed before its
question was asked. An empty answer types just the newline, which is how a
script's own default gets exercised. An expectation that never arrives, or a
script that never exits, fails the test with the transcript so far instead of
hanging the suite; `PTY_TIMEOUT` (default 20s) bounds it.

`pty_answers` pairs each answer with `"]: "`, the terminator every prompt in
this project ends with. Prefer `pty_expect` with a distinctive substring when
the prompts are easy to confuse — an answer landing in the wrong prompt would
still satisfy a bare "contains" check.

## Constraints

- **bash 3.2.** macOS still ships it. No `declare -A`, no `mapfile`/`readarray`,
  no `${var,,}` or `${var^^}`, no `local -n`. The harness itself observes this.
- **BSD and GNU userland both.** No `sed -i`, no `grep -P`, no `readlink -f`,
  no `find -printf`, no GNU-only `stat` or `date` flags.
- **No new dependencies.** bash, coreutils, and python3 for the pty driver.
- Watch out for `local a="$1" b="$STUB_BIN/$a"` — bash expands `$a` before the
  assignment to `a` takes effect, and under dynamic scoping it silently picks up
  the caller's variable of the same name. Declare on separate lines.

## What lives where

| Path | |
| --- | --- |
| `tests/run.sh` | the entry point: discovery, filtering, reporting, the sandbox tripwire |
| `tests/lib/runner.sh` | runs one test in one process |
| `tests/lib/list_functions.sh` | which test functions a case file really defines |
| `tests/lib/harness.sh` | the sandbox and the environment redirection |
| `tests/lib/assert.sh` | assertions |
| `tests/lib/stub.sh` | command stubs and their recordings |
| `tests/lib/fixture.sh` | fixture lookup |
| `tests/lib/script.sh` | `run_script`, and the bash side of the pty driver |
| `tests/lib/pty_run.py` | the pty driver itself |
| `tests/lib/project_probe.py` | drives `bin/ayeaye`'s project discovery from bash |
| `tests/cases/harness_*_test.sh` | the harness testing itself |
| `tests/cases/install_*_test.sh` | what `install.sh` does today |
| `tests/cases/wizard_*_test.sh` | the setup layer: state, consent, lifecycle, flow, resume |
| `tests/cases/projects_*_test.sh` | how the project picker finds and ranks projects |
| `tests/cases/install_packages_test.sh` | the requirements, and the exact install command per family |
| `tests/cases/install_agents_test.sh` | finding, fetching and checking Claude Code and Codex |
| `tests/cases/install_marker_test.sh` | the Claude Code status line, and the settings merge |
| `tests/cases/install_cliban_test.sh` | cliban, and what it means for a component to be optional |

## Testing a setup step

`lib/steps/README.md` is the contract for adding work to the wizard. Three
things about testing one:

- A step is registered, not called: `install.sh` sources every `lib/steps/*.sh`
  in filename order, so a case file drives it by running the whole installer.
- A step that could not do its work returns `$WIZARD_STAGE_PENDING`, which
  keeps its stage out of `done` and puts it in the closing summary. Pin that,
  not just the message: a seam that reports success is the failure mode the
  whole lifecycle exists to prevent.
- `wizard_contract_test.sh` reads the source rather than a run. It fails if
  anything outside `lib/consent.sh` installs a package, reaches for `sudo`,
  downloads a file or touches a firewall or a trust store. That is a lint, and
  it is the only thing that can see a step routing around consent.

## Testing the Python

`bin/ayeaye` is Python and this suite is bash, so something has to sit
between them: `tests/lib/project_probe.py`. It imports the code under test,
runs one thing, and prints `key=value` lines for a case file to assert on.

The reason it exists rather than a separate Python test runner is that the
properties worth pinning are invisible from outside an HTTP request. Which
bound ended a search, how many directories it really listed, whether a
superseded search delivered anything — none of that is in the JSON a client
sees, and a test that cannot see them can only assert that something
plausible came back. A probe keeps one command running everything while
still reaching inside.

A case file drives it through `run_script`, which needs the interpreter the
sandbox deliberately withholds:

```sh
setup() {
  require_host_command python3     # skip where there is none, e.g. bash:3.2
  stub_real python3
}

probe() { run_script "$TESTS_DIR/lib/project_probe.py" "$@"; }
```

Build the trees such a test walks inside `$TEST_TMPDIR` or the sandbox
`$HOME`, never the real one — `probe maketree "$HOME/tree" --width 6 --depth 5`
makes nine thousand directories in about a second, which is enough for a
bound to be worth asserting on.
