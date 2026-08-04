# What ayeaye needs, what you talk to it with, and the board it can show you.
#
# Three things, in one file, because to the person running setup they are one
# conversation:
#
#   the requirements   tmux and python3, without which nothing works, plus the
#                      two tools setup needs to fetch anything at all
#   the coding agents  Claude Code and OpenAI Codex - what ayeaye drives, and
#                      the accounts they need you to sign in to
#   cliban             the project board. Explicitly optional: every control
#                      ayeaye puts on your phone works without it, and only the
#                      board page and the links to tickets need it.
#
# install.sh does not know this file exists. Everything here attaches through
# the seam in lib/steps/README.md, on three stages:
#
#   detect     what this computer already has
#   configure  the questions, and every plan item - so that stage five can say
#              what is about to happen before any of it happens
#   install    the four steps that change anything
#
# Rules this file may not break, and why each one is here:
#
#   Nothing installs, downloads or overwrites on its own. Each of those goes
#   through lib/consent.sh, which asks first and does nothing when refused.
#   tests/cases/wizard_contract_test.sh reads this file and fails the suite if
#   that stops being true.
#
#   No step reports OK for work that did not happen. A download that was
#   refused, an agent that was installed but not signed in, a board that could
#   not be fetched: each says so and lands on the closing checklist.
#
#   Standard input is fd 8, and this file is the one with three ways to lose
#   it: it pauses for the user to sign in to an account, it runs somebody
#   else's installer, and it starts two programs to ask them their version.
#   Every question goes through wizard_ask or wizard_confirm, and everything
#   else runs with standard input closed - because a program that reads a line
#   eats the answer to the question after it, and the question after it
#   defaults to yes.
#
#   Optional work that fails does not fail the run. The three optional steps
#   are registered `optional` for that reason.
#
# The agent CLIs and cliban come from their own projects rather than from a
# distro package, because that is where they are published: neither Claude Code
# nor Codex nor cliban is in Debian, Fedora, Arch or openSUSE. The prebuilt
# artifacts cover exactly the four platforms this project supports - Linux on
# x86_64 and arm64, macOS on Intel and Apple Silicon - and anything else is
# told so in words rather than handed a download that cannot run.
#
# bash 3.2: no associative arrays, no ${var,,}, no mapfile, no local -n.

# ------------------------------------------------------------------ what we
#                                                                    fetch from

# Anthropic's own installer for Claude Code. It puts the binary under $HOME and
# refuses to run with elevated privileges, which is why nothing here offers to
# give it any.
_PKGS_CLAUDE_INSTALLER="${AYEAYE_CLAUDE_INSTALLER_URL:-https://claude.ai/install.sh}"

# OpenAI publishes a single executable per platform on its releases page, under
# a name that carries no version, so "latest" resolves without asking an API
# what the latest is - one request, and it is the download itself.
_PKGS_CODEX_BASE="${AYEAYE_CODEX_RELEASE_URL:-https://github.com/openai/codex/releases/latest/download}"

# cliban releases often, and the board page in bin/ayeaye speaks its current
# CLI, so the release taken is always the latest one. That resolves without
# asking an API which version "latest" is, because cliban publishes every
# artifact under a versionless alias as well - the same property the Codex
# release has, and it matters here for the same reason: the URL is known
# before any question is asked, so consent still comes before the first byte
# moves.
#
# cliban publishes other routes as well - its own curl-pipe-sh installer, a
# Homebrew tap, crates.io, an AUR package - and this file still takes the
# release tarball, because every one of those routes installs cliband, the
# multi-user server, alongside it, and putting a server on somebody's computer
# is an explicit non-goal here. The tarball is the one route that lets setup
# take the single program it wants and nothing else.
#
# What arrives is compared against the SHA256SUMS published beside it, fetched
# on the same consent - the bootstrap's model, and worth what it is worth
# there: it catches a download that broke in transit, and it does not catch
# somebody in a position to replace both files at once.
_PKGS_CLIBAN_BASE="${AYEAYE_CLIBAN_RELEASE_URL:-https://github.com/LioraLabs/cliban/releases/latest/download}"

# Rounded up from the real artifacts, for the "about N MB" line in stage five.
# Codex really is that big: 110 MB compressed and around 300 MB unpacked.
_PKGS_CLAUDE_BYTES=30000000
_PKGS_CODEX_BYTES=115000000
_PKGS_CLIBAN_BYTES=11000000

# ------------------------------------------------------------------- where
#                                                                     things go

_pkgs_bin_dir() {
  printf '%s' "${XDG_BIN_HOME:-$HOME/.local/bin}"
}

_pkgs_data_dir() {
  printf '%s' "${XDG_DATA_HOME:-$HOME/.local/share}/ayeaye"
}

_pkgs_marker_script() {
  printf '%s/statusline-command.sh' "$(_pkgs_data_dir)"
}

_pkgs_claude_settings() {
  printf '%s/.claude/settings.json' "$HOME"
}

_pkgs_cliban_db() {
  if [ -n "${CLIBAN_DB:-}" ]; then
    printf '%s' "$CLIBAN_DB"
  else
    printf '%s/cliban/cliban.db' "${XDG_DATA_HOME:-$HOME/.local/share}"
  fi
}

# The settings file this run is writing. install.sh owns the name; reading it
# defensively means this file can also be sourced on its own by a test.
_pkgs_env_file() {
  printf '%s' "${ENV_FILE:-${XDG_CONFIG_HOME:-$HOME/.config}/ayeaye/env}"
}

# ----------------------------------------------------------------- small tools

_pkgs_have() { command -v "${1:-}" >/dev/null 2>&1; }

# _pkgs_find <name> - where this program is, or nothing.
#
# PATH is not the whole answer and never was: everything this file installs
# goes into a directory that may not be on PATH until the next terminal window,
# and claude.ai's installer always writes ~/.local/bin whatever XDG_BIN_HOME
# says. Every question about whether one of these three programs is here goes
# through this, so that the stage that reports, the stage that plans and the
# stage that installs can never give three different answers.
_pkgs_find() {
  local name="${1:-}" found candidate
  if found="$(command -v "$name" 2>/dev/null)" && [ -n "$found" ]; then
    printf '%s' "$found"
    return 0
  fi
  for candidate in "$(_pkgs_bin_dir)/$name" "$HOME/.local/bin/$name"; do
    if [ -x "$candidate" ]; then
      printf '%s' "$candidate"
      return 0
    fi
  done
  return 1
}

_pkgs_here() { _pkgs_find "${1:-}" >/dev/null 2>&1; }

# _pkgs_shell_word <word> - a word safe to paste into a command line.
#
# Everything wizard_privileged runs goes through eval, so a path with a space
# in it becomes two arguments and a path with a semicolon becomes two commands.
# Temporary directories are built from $TMPDIR, which belongs to the user, so
# this is not hypothetical. The platform layer already has the routine; this is
# a named door onto it rather than a second copy of it.
_pkgs_shell_word() {
  _platform_quote "${1:-}"
  printf '%s' "$_PLATFORM_S"
}

# _pkgs_workdir - somewhere to put an archive while it is being checked.
# Nothing here is kept: the binary is copied out and the directory goes.
_pkgs_workdir() {
  local dir
  dir="$(mktemp -d "${TMPDIR:-/tmp}/ayeaye-fetch.XXXXXX" 2>/dev/null)" || return 1
  [ -n "$dir" ] || return 1
  printf '%s' "$dir"
}

# _pkgs_target - the release-artifact name for this computer, or nothing.
#
# Empty is a real answer and the only honest one on a machine none of these
# projects builds for: a 32-bit Raspberry Pi, a riscv board, a BSD. The caller
# says so and offers what can be done by hand instead of downloading a binary
# that cannot run.
_pkgs_target() {
  local os arch
  os="$(platform_os)"
  arch="$(platform_arch)"
  case "$os:$arch" in
    linux:x86_64) printf 'x86_64-unknown-linux-musl' ;;
    linux:arm64)  printf 'aarch64-unknown-linux-musl' ;;
    macos:x86_64) printf 'x86_64-apple-darwin' ;;
    macos:arm64)  printf 'aarch64-apple-darwin' ;;
    *)            return 1 ;;
  esac
}

# _pkgs_ask_yes <state-key> <question> <built-in-default> - a yes/no answer that
# remembers itself.
#
# The default offered is whatever was answered last time, so a rerun keeps the
# choices somebody already made rather than asking them to make them again -
# the same rule the address and port questions follow in install.sh. That is
# also what lets a resumed unattended run finish the work the first one
# started, since it takes the default without reading.
#
# A run that cannot ask has its built-in default forced to no, whatever this
# file thinks the sensible answer is. Installing Claude Code is the right
# suggestion to make to somebody who is there to hear it and a decision made on
# their behalf when they are not, so --defaults installs nothing it was not
# already told to. An answer a previous run recorded is such a telling, which
# is what lets an interrupted unattended run finish what it started.
_pkgs_ask_yes() {
  local key="${1:-}" question="${2:-}" def="${3:-n}" previous
  previous="$(wizard_state_get "$key" "")"
  case "$previous" in
    1) def=y ;;
    0) def=n ;;
    *) [ "${WIZARD_INTERACTIVE:-1}" = 0 ] && def=n ;;
  esac
  if wizard_confirm "$question" "$def"; then
    wizard_remember "$key" 1
    return 0
  fi
  wizard_remember "$key" 0
  return 1
}

_pkgs_wants() { [ "$(wizard_state_get "${1:-}" 0)" = 1 ]; }

# _pkgs_installed <logical> - is this program usable on this computer?
#
# The command first, because that is what everything here actually runs, and
# the package database second, for the case where a package is installed under
# a name the shell has not been told about yet. One predicate, used by the
# stage that plans and the stage that installs, so the two can never disagree
# about what "missing" means.
_pkgs_installed() {
  _pkgs_have "${1:-}" && return 0
  platform_pkg_is_installed "${1:-}"
}

# _pkgs_can_get <tool> - is this tool here, or could setup get it?
#
# gzip is not on this list, and it is not an oversight: bsdtar decompresses by
# itself and GNU tar shells out to gzip, which is part of the base system of
# every distribution this project supports - Debian marks it Essential, and the
# other three ship it in their minimal install. A machine that has GNU tar and
# no gzip says so through tar's own error, in the log, and the step reports it
# rather than claiming an archive was unpacked.
_pkgs_can_get() {
  _pkgs_installed "${1:-}" && return 0
  platform_pkg_can_act
}

# _pkgs_can_download - could anything be fetched from the internet at all?
_pkgs_can_download() { _pkgs_can_get curl; }

# _pkgs_can_unpack - and could an archive then be opened?
#
# Separate from the above because the two are needed by different things: the
# Claude Code installer is a script and needs only the first, while Codex and
# cliban arrive as archives and need both. A computer with one and not the
# other is offered exactly what it can have.
_pkgs_can_unpack() { _pkgs_can_download && _pkgs_can_get tar; }

# _pkgs_probe <path-or-name> - does this program start, and what does it say it
# is? The whole of "validating" an agent, deliberately.
#
# A version flag costs nothing and runs nothing. Asking an agent to answer a
# real prompt would confirm rather more - and would spend the user's money to
# do it, on an account they may have set up ninety seconds ago. So this proves
# the program is installed and runs, and the caller says exactly that and no
# more.
# Standard input is closed for it, deliberately. A step body's standard input
# is the wizard's own, and a program that reads it - a version flag that turns
# out to be interactive, a pager - would eat the answer to the question after
# this one.
_pkgs_probe() {
  local cmd="${1:-}" out
  [ -n "$cmd" ] || return 2
  if out="$("$cmd" --version </dev/null 2>&1)"; then
    printf '%s' "$out"
    return 0
  fi
  wizard_detail "probe failed: $cmd --version"
  wizard_detail "$out"
  return 1
}

# _pkgs_install_binary <source> <name> - put a program where the user can run
# it. Prints nothing; the caller reports.
#
# Replacing something already at that name goes through wizard_replace, which
# asks and takes a copy first: a program the user put there by hand is theirs,
# whatever it is called.
_pkgs_install_binary() {
  local src="${1:-}" name="${2:-}" dir dest tmp
  [ -f "$src" ] || return 1
  dir="$(_pkgs_bin_dir)"
  dest="$dir/$name"
  mkdir -p "$dir" 2>/dev/null || return 1
  if [ -e "$dest" ]; then
    wizard_replace "$dest" "you already have a program called $name at $dest. May I replace it?" \
      || return "$?"
  fi
  # Moved rather than copied: an agent CLI is a third of a gigabyte unpacked,
  # and two copies of it at once is a real amount of somebody's disk. The
  # destination is still only ever written atomically, by the rename below.
  tmp="$dest.tmp.$$"
  mv "$src" "$tmp" 2>/dev/null || cp "$src" "$tmp" 2>/dev/null || return 1
  chmod 755 "$tmp" 2>/dev/null || true
  mv -f "$tmp" "$dest" 2>/dev/null || {
    rm -f "$tmp" 2>/dev/null
    return 1
  }
  return 0
}

_PKGS_PATH_NOTED="${_PKGS_PATH_NOTED:-0}"

# _pkgs_say_path_note <path-of-something-installed> - said once, about the
# directory the program really went into, and only when it is true.
#
# The argument matters: nothing is gained by telling somebody about a directory
# nothing was put in, and telling them about the wrong one is worse than
# silence.
_pkgs_say_path_note() {
  local installed="${1:-}" dir
  [ -n "$installed" ] || return 0
  [ "$_PKGS_PATH_NOTED" = 0 ] || return 0
  dir="${installed%/*}"
  [ -n "$dir" ] || return 0
  case ":${PATH:-}:" in
    *":$dir:"*) return 0 ;;
  esac
  _PKGS_PATH_NOTED=1
  wizard_blank
  wizard_say "note: $dir is not somewhere your shell looks for programs yet."
  wizard_say "open a new terminal window and it usually is. If it still is not,"
  wizard_say "add this line to the end of your shell's startup file:"
  wizard_say "  export PATH=\"$dir:\$PATH\""
  return 0
}

# _pkgs_digest <file> - the sha256 of a file, using whatever this machine has.
# 1 when it has none of them, which is a thing to say out loud rather than a
# thing to fail over - the same shape as the bootstrap's own digest.
_pkgs_digest() {
  if _pkgs_have sha256sum; then
    sha256sum "$1" | awk '{print $1}'
  elif _pkgs_have shasum; then
    shasum -a 256 "$1" | awk '{print $1}'
  elif _pkgs_have openssl; then
    openssl dgst -sha256 "$1" | awk '{print $NF}'
  else
    return 1
  fi
}

# _pkgs_verify_archive <archive> <member> - is this really the archive it should
# be, and does it contain what it should?
#
# Codex publishes no checksums, so for it this is what verification can
# honestly be: the bytes are a real gzip stream, the archive is a real tar, and
# the one file that is wanted is in it under the name it should have. cliban's
# checksum is pinned above and compared before this is called; this still runs
# after it, because an archive that matches its checksum can still be missing
# the one file this step is about to take out of it. The program is then run
# with a harmless flag before anything reports success.
_pkgs_verify_archive() {
  local archive="${1:-}" member="${2:-}" listing
  [ -s "$archive" ] || {
    wizard_detail "verify: $archive is empty or missing"
    return 1
  }
  if ! listing="$(tar -tzf "$archive" 2>&1)"; then
    wizard_detail "verify: $archive is not a readable archive"
    wizard_detail "$listing"
    return 1
  fi
  # A whole line, not a prefix: an archive containing "cliban-x/cliban-notes"
  # is not an archive containing "cliban-x/cliban".
  case "
$listing
" in
    *"
$member
"*) return 0 ;;
  esac
  wizard_detail "verify: $archive does not contain $member"
  wizard_detail "$listing"
  return 1
}

# ================================================== 2. what is already here

_pkgs_detect_step() {
  local tool found

  if found="$(_pkgs_find claude)"; then
    wizard_item ok "Claude Code"
    wizard_detail "Claude Code: $found"
  else
    wizard_item "-" "Claude Code (not installed)"
  fi
  if found="$(_pkgs_find codex)"; then
    wizard_item ok "OpenAI Codex"
    wizard_detail "OpenAI Codex: $found"
  else
    wizard_item "-" "OpenAI Codex (not installed)"
  fi
  if found="$(_pkgs_find cliban)"; then
    wizard_item ok "cliban (the project board)"
    wizard_detail "cliban: $found"
  else
    wizard_item "-" "cliban (optional; the project board needs it)"
  fi

  # curl and tar are not ayeaye's requirements, they are setup's, and only when
  # there is something to fetch. Said here as a note rather than a verdict; the
  # install stage is where it becomes a fact worth acting on.
  for tool in curl tar; do
    _pkgs_have "$tool" \
      || wizard_detail "$tool is not installed; anything downloaded will need it"
  done
  return "$WIZARD_STAGE_OK"
}

# =================================================== 4. what you want here

# Is there still something to fetch for each of the three? "Still", because a
# program that is already here needs nothing fetched however the question was
# answered - and an answer somebody gave is theirs and is not overwritten to
# record a fact about the machine.
_pkgs_fetching_claude() { _pkgs_here claude && return 1; _pkgs_wants answer.agent.claude; }
_pkgs_fetching_codex()  { _pkgs_here codex  && return 1; _pkgs_wants answer.agent.codex; }
_pkgs_fetching_board()  { _pkgs_here cliban && return 1; _pkgs_wants answer.board; }

# _pkgs_fetch_tools - the logical package names *this* step installs.
#
# Not tmux and python3. Those are ayeaye's own requirements and install.sh asks
# about them in the detect stage, three stages earlier, because a run without
# them cannot get far enough to reach this one. Installing them again here
# would be a second password prompt for something already done. This step
# checks them and installs what setup itself needs to fetch what was chosen:
# curl for anything at all, and tar as well for the two that arrive in an
# archive. Neither is asked for by a text-only setup that chose nothing.
_pkgs_fetch_tools() {
  local list=""
  if _pkgs_fetching_claude || _pkgs_fetching_codex || _pkgs_fetching_board; then
    list="curl"
  fi
  if _pkgs_fetching_codex || _pkgs_fetching_board; then
    list="$list tar"
  fi
  printf '%s' "${list# }"
}

_pkgs_choose_step() {
  local have_claude=0 have_codex=0 have_board=0 want_claude=0 want_codex=0
  local missing="" logical

  _pkgs_here claude && have_claude=1
  _pkgs_here codex  && have_codex=1
  _pkgs_here cliban && have_board=1

  if ! _pkgs_can_download && [ "$have_claude$have_codex$have_board" = "000" ]; then
    # Nothing here can be offered, so nothing here is asked. Four questions
    # whose answers could not be acted on are worse than one sentence, and the
    # tools by name belong in the log rather than in front of somebody who has
    # never opened a terminal.
    wizard_blank
    wizard_say "this computer has no way to fetch a program from the internet,"
    wizard_say "and no way to add one, so setup will not offer to install a"
    wizard_say "coding agent or the project board here."
    wizard_say "Once it can, run ./install.sh again and it will offer."
    wizard_detail "cannot fetch: no curl, and packages cannot be installed"
    return "$WIZARD_STAGE_SKIP"
  fi

  wizard_blank
  wizard_say "ayeaye shows you the coding agents running on this computer and"
  wizard_say "lets you talk to them from your phone. It drives two of them:"
  wizard_say "Claude Code, and OpenAI Codex. You need at least one."
  wizard_blank

  if [ "$have_claude" = 1 ]; then
    wizard_say "Claude Code is already installed here."
  elif _pkgs_can_download; then
    if _pkgs_ask_yes answer.agent.claude "install Claude Code?" "y"; then
      want_claude=1
    fi
  fi

  if [ "$have_codex" = 1 ]; then
    wizard_say "OpenAI Codex is already installed here."
  elif _pkgs_can_unpack; then
    wizard_say "Codex is a large download, around 110 MB."
    if _pkgs_ask_yes answer.agent.codex "install OpenAI Codex?" "n"; then
      want_codex=1
    fi
  fi

  # The marker is only worth anything with Claude Code, and only worth asking
  # about when there will be a Claude Code to mark.
  if [ "$have_claude" = 1 ] || [ "$want_claude" = 1 ]; then
    wizard_blank
    wizard_say "Claude Code can print a short tag in its status line saying which"
    wizard_say "conversation a window is showing. ayeaye reads that tag, and it is"
    wizard_say "what lets the phone show you the conversation rather than just the"
    wizard_say "screen. Nothing else about Claude Code changes, and everything you"
    wizard_say "have already set stays exactly as it is."
    _pkgs_ask_yes answer.agent.marker "set that up?" "y" || true
  else
    wizard_remember answer.agent.marker 0
  fi

  wizard_blank
  wizard_say "ayeaye can also show you a project board: your tickets, and what"
  wizard_say "each agent is working on. That needs a separate small program"
  wizard_say "called cliban."
  wizard_say "Everything else works without it - starting agents, reading them,"
  wizard_say "talking to them, approving what they ask. Only the board page and"
  wizard_say "the links to tickets need cliban, and they are the only things that"
  wizard_say "stay empty without it."
  if [ "$have_board" = 1 ]; then
    wizard_say "It is already installed here."
  elif _pkgs_can_unpack; then
    _pkgs_ask_yes answer.board "install cliban as well?" "n" || true
  else
    wizard_say "This computer has no way to unpack it yet, so setup will not"
    wizard_say "offer it here. Nothing else is affected."
  fi

  # The plan, filled in here so stage five can show all of it before stage six
  # does any of it.
  for logical in $(_pkgs_fetch_tools); do
    _pkgs_installed "$logical" && continue
    missing="$missing $logical"
  done
  if [ -n "$missing" ]; then
    wizard_plan_add package "what setup needs to fetch things:$missing"
  fi
  if [ "$want_claude" = 1 ]; then
    wizard_plan_add download "Claude Code, from claude.ai" "$_PKGS_CLAUDE_BYTES"
  fi
  if [ "$want_codex" = 1 ]; then
    wizard_plan_add download "OpenAI Codex, from its releases page" "$_PKGS_CODEX_BYTES"
  fi
  if _pkgs_wants answer.agent.marker; then
    wizard_plan_add config "a status line for Claude Code, added to $(_pkgs_claude_settings)"
  fi
  if _pkgs_fetching_board; then
    wizard_plan_add download "cliban (its latest release), for the project board" \
      "$_PKGS_CLIBAN_BYTES"
  fi
  return "$WIZARD_STAGE_OK"
}

# ================================ 6a. what setup needs to do what was chosen

_pkgs_requirements_step() {
  local logical missing="" nothing_to_install="" still="" status
  local can_download can_unpack

  # ayeaye's own two, checked and not installed: install.sh asks about these in
  # the detect stage, three stages earlier, because a run without them cannot
  # get far enough to reach this one. Checking is worth a line each; asking a
  # second time for the same password is not.
  for logical in tmux python3; do
    if _pkgs_installed "$logical"; then
      wizard_item ok "$logical"
      continue
    fi
    wizard_item MISSING "$logical"
    wizard_blank
    wizard_say "ayeaye cannot run without $logical."
    wizard_say "install it and run ./install.sh again; it carries on from here."
    return "$WIZARD_STAGE_FAIL"
  done

  for logical in $(_pkgs_fetch_tools); do
    if _pkgs_installed "$logical"; then
      wizard_item ok "$logical"
      continue
    fi
    wizard_item MISSING "$logical"
    # A name this family has no package for is not something to install. macOS
    # has no formula called tar and needs none, because it ships one; asking
    # Homebrew for it would be asking for a thing that does not exist. Noted
    # and set aside, so whatever else is missing still gets installed.
    if [ "$(platform_family)" != unknown ] && ! platform_pkg_has_mapping "$logical"; then
      nothing_to_install="$nothing_to_install $logical"
      continue
    fi
    missing="$missing $logical"
  done

  if [ -n "$missing" ]; then
    wizard_blank
    wizard_say "setup needs one or two more things to fetch what you chose."
    wizard_detail "missing:$missing"
    wizard_blank

    # The one sanctioned way to install anything: it asks, it shows the
    # commands, and on a machine that cannot install it prints what to do by
    # hand instead.
    # shellcheck disable=SC2086
    wizard_install_packages $missing
    status=$?

    case "$status" in
      0) ;;
      2)
        # A bug here, not news about this computer, and saying "your platform
        # is unsupported" would send somebody looking for a fault that is not
        # theirs.
        wizard_say "setup asked for software without naming any. That is a fault"
        wizard_say "in setup itself; nothing on this computer has been changed."
        _pkgs_remember_fetching 0 0
        return "$WIZARD_STAGE_FAIL"
        ;;
      3)
        wizard_say "nothing was installed, and this computer is exactly as it was."
        ;;
      *)
        wizard_say "that software could not be installed automatically."
        ;;
    esac

    for logical in $missing; do
      if _pkgs_installed "$logical"; then
        wizard_item ok "$logical"
        continue
      fi
      wizard_item MISSING "$logical"
      still="$still $logical"
    done
  fi

  # What is possible now, per capability, so the steps after this one offer
  # exactly what this computer can still have and nothing else.
  can_download=0
  can_unpack=0
  _pkgs_installed curl && can_download=1
  if [ "$can_download" = 1 ] && _pkgs_installed tar; then
    can_unpack=1
  fi
  _pkgs_remember_fetching "$can_download" "$can_unpack"

  if [ -z "$still$nothing_to_install" ]; then
    if [ -n "$missing" ]; then
      wizard_say "installed."
    else
      wizard_say "everything setup needs is already on this computer."
    fi
    return "$WIZARD_STAGE_OK"
  fi

  wizard_blank
  if [ -n "$nothing_to_install" ]; then
    wizard_say "this computer has no package for$nothing_to_install, so setup"
    wizard_say "cannot add it."
  fi
  wizard_say "some of what you chose cannot be fetched, so that part is left"
  wizard_say "for another run. Everything that can still be done, still will be."
  wizard_detail "still missing:$still$nothing_to_install"
  return "$WIZARD_STAGE_PENDING"
}

# _pkgs_remember_fetching <can-download> <can-unpack> - what the steps after
# this one may do.
#
# Two facts, not one, because they gate different things: the Claude Code
# installer is a script and needs only curl, while Codex and cliban arrive in
# an archive and need tar as well. Collapsing them into a single "can fetch"
# would withhold the main agent from a computer that can perfectly well install
# it, and say so in a sentence that is not true.
#
# Through the state file rather than a variable, because the step that finds
# out is one a resumed run skips: a later step reading a shell variable would
# read its default and try a download this run already knows cannot work.
_pkgs_remember_fetching() {
  wizard_remember step.install.packages.download "${1:-0}"
  wizard_remember step.install.packages.unpack "${2:-0}"
}

# _pkgs_may_fetch <download|unpack>
_pkgs_may_fetch() {
  case "${1:-download}" in
    unpack) [ "$(wizard_state_get step.install.packages.unpack 1)" = 1 ] ;;
    *)      [ "$(wizard_state_get step.install.packages.download 1)" = 1 ] ;;
  esac
}

# ============================================== 6b. the coding agents

# _pkgs_fetch_claude - Anthropic's installer, downloaded and then run.
#
# Two questions rather than one, and deliberately: fetching a script from the
# internet and running it are different things to agree to, and the second is
# the one worth being able to say no to on its own.
_pkgs_fetch_claude() {
  local work script status
  work="$(_pkgs_workdir)" || {
    wizard_say "could not make a temporary folder to download into."
    return 1
  }
  script="$work/claude-install.sh"
  # A few kilobytes: it is the installer, not Claude Code. What the installer
  # then fetches is counted in the plan, where the number is read by a person
  # deciding whether to start.
  wizard_download "may I download the Claude Code installer from claude.ai?" \
    "$_PKGS_CLAUDE_INSTALLER" "$script" 8000
  status=$?
  if [ "$status" != 0 ]; then
    rm -rf "$work" 2>/dev/null
    return "$status"
  fi
  if [ ! -s "$script" ]; then
    wizard_say "the download arrived empty, so nothing was run."
    rm -rf "$work" 2>/dev/null
    return 1
  fi

  # What is said here is what is true: this hands over to somebody else's
  # program. It is Anthropic's own and it is documented to install into your
  # home folder without a password, and setup does not control what it does.
  wizard_say "the next step hands over to Anthropic's own installer, which is"
  wizard_say "what claude.ai publishes for this. It puts Claude Code in your"
  wizard_say "home folder and asks for no password. Setup cannot promise what"
  wizard_say "else it changes, which is why this is a question."
  # </dev/null: the installer inherits this step's standard input, which is the
  # wizard's own, and one that read a line would eat the answer to the next
  # question. Quoted: $TMPDIR belongs to the user and may contain a space.
  wizard_privileged "run the Claude Code installer now?" \
    "bash $(_pkgs_shell_word "$script") </dev/null"
  status=$?
  rm -rf "$work" 2>/dev/null
  return "$status"
}

# _pkgs_fetch_codex - one executable, from the releases page.
#
# Status 4 for "nobody builds one for this computer", which is neither a
# failure nor a refusal: trying again cannot help, so the caller reports it as
# unfinished rather than letting the lifecycle offer three more attempts at
# something that can never work.
_pkgs_fetch_codex() {
  local work target url archive member status
  if ! target="$(_pkgs_target)"; then
    wizard_say "OpenAI does not publish a ready-made Codex for this kind of"
    wizard_say "computer ($(platform_os), $(platform_arch)), so setup has nothing"
    wizard_say "safe to download."
    wizard_say "if this computer has npm, this installs it:"
    wizard_say "  npm install -g @openai/codex"
    return 4
  fi
  work="$(_pkgs_workdir)" || {
    wizard_say "could not make a temporary folder to download into."
    return 1
  }
  member="codex-$target"
  url="$_PKGS_CODEX_BASE/$member.tar.gz"
  archive="$work/$member.tar.gz"

  wizard_download "may I download OpenAI Codex? It is about 110 MB." \
    "$url" "$archive" "$_PKGS_CODEX_BYTES"
  status=$?
  if [ "$status" != 0 ]; then
    rm -rf "$work" 2>/dev/null
    return "$status"
  fi
  if ! _pkgs_verify_archive "$archive" "$member"; then
    wizard_say "what was downloaded is not the Codex release it should be, so"
    wizard_say "nothing has been unpacked or installed."
    rm -rf "$work" 2>/dev/null
    return 1
  fi
  if ! tar -xzf "$archive" -C "$work" "$member" 2>/dev/null; then
    wizard_say "the download could not be unpacked."
    rm -rf "$work" 2>/dev/null
    return 1
  fi
  _pkgs_install_binary "$work/$member" codex
  status=$?
  rm -rf "$work" 2>/dev/null
  return "$status"
}

# _pkgs_sign_in <label> <command> - explain the account, and wait.
#
# The pause is the point. An agent installs in a minute and takes rather longer
# to sign in to, and a wizard that raced past it would go on to check something
# the user has not had the chance to do yet. wizard_ask reads the wizard's own
# standard input, so the answer to this lands here and the question after it
# still gets its own.
_pkgs_sign_in() {
  local label="${1:-}" cmd="${2:-}"
  wizard_blank
  wizard_say "$label needs an account before it will answer you."
  wizard_say "In a terminal window of your own, run:"
  wizard_say "  $cmd"
  wizard_say "It opens your web browser, you sign in there, and it remembers."
  wizard_say "Nothing about that goes through ayeaye and setup never sees it."
  if [ "${WIZARD_INTERACTIVE:-1}" = 0 ]; then
    wizard_blank
    wizard_say "this run cannot wait for that, so it has not happened yet."
    return 1
  fi
  wizard_blank
  wizard_ask "press return once you have signed in, or say skip" "signed in"
  case "$REPLY" in
    skip|s|later|no|n) return 1 ;;
  esac
  return 0
}

# _pkgs_one_agent <name> <label> <login-command> <state-key> <download|unpack>
#
# Sets _PKGS_AGENT_RESULT to finished | pending | failed | skipped, so the step can
# hold the worst of them without a subshell losing the answer.
_pkgs_one_agent() {
  local name="${1:-}" label="${2:-}" login="${3:-}" key="${4:-}" need="${5:-download}"
  local dest version status found
  _PKGS_AGENT_RESULT=skipped
  dest="$(_pkgs_bin_dir)/$name"

  if found="$(_pkgs_find "$name")"; then
    wizard_item ok "$label is already here"
    wizard_detail "$label: $found"
    _PKGS_AGENT_RESULT=finished
    return 0
  fi

  if ! _pkgs_wants "$key"; then
    wizard_item "-" "$label (not chosen)"
    return 0
  fi

  # The step before this one is where setup found out what it can fetch.
  # Trying anyway would end in "could not be installed" one screen after "left
  # for another run", which is two different explanations for one fact. Claude
  # Code arrives as a script and Codex as an archive, so they are not the same
  # question.
  if ! _pkgs_may_fetch "$need"; then
    wizard_item "-" "$label (nothing here can fetch it yet)"
    _PKGS_AGENT_RESULT=pending
    return 0
  fi

  case "$name" in
    claude) _pkgs_fetch_claude ;;
    codex)  _pkgs_fetch_codex ;;
    *)      return 2 ;;
  esac
  status=$?
  case "$status" in
    0) ;;
    3)
      wizard_item "-" "$label (you said no)"
      return 0
      ;;
    4)
      wizard_item "-" "$label (nothing is published for this computer)"
      _PKGS_AGENT_RESULT=pending
      return 0
      ;;
    *)
      wizard_item FAILED "$label could not be installed"
      _PKGS_AGENT_RESULT=failed
      return 0
      ;;
  esac

  if ! found="$(_pkgs_find "$name")"; then
    wizard_item FAILED "$label was installed but cannot be found"
    _PKGS_AGENT_RESULT=failed
    return 0
  fi

  # Harmless, and the report says exactly what it proves and what it does not.
  if version="$(_pkgs_probe "$found")"; then
    wizard_item ok "$label runs: $version"
  else
    wizard_item FAILED "$label was installed but will not start"
    _PKGS_AGENT_RESULT=failed
    return 0
  fi

  _pkgs_say_path_note "$found"
  if _pkgs_sign_in "$label" "$login"; then
    wizard_say "$label is installed and you have signed in."
    wizard_say "setup checked that it starts; whether the account works is"
    wizard_say "something only $label itself can tell you, and asking it would"
    wizard_say "have cost you money."
    _PKGS_AGENT_RESULT=finished
  else
    wizard_say "$label is installed but not signed in yet. Run this when you can:"
    wizard_say "  $login"
    _PKGS_AGENT_RESULT=pending
  fi
  return 0
}

_pkgs_agents_step() {
  local worst=finished

  # Status 2 from _pkgs_one_agent is a name it has no channel for, which is a
  # fault in this file rather than news about the computer. It cannot happen
  # today and would be silent if it started to.
  if ! _pkgs_one_agent claude "Claude Code" "claude" answer.agent.claude download; then
    wizard_say "setup does not know how to install that. That is a fault in"
    wizard_say "setup itself; nothing on this computer has been changed."
    return "$WIZARD_STAGE_FAIL"
  fi
  case "$_PKGS_AGENT_RESULT" in
    failed) worst=failed ;;
    pending) [ "$worst" = "failed" ] || worst=pending ;;
  esac

  if ! _pkgs_one_agent codex "OpenAI Codex" "codex login" answer.agent.codex unpack; then
    wizard_say "setup does not know how to install that. That is a fault in"
    wizard_say "setup itself; nothing on this computer has been changed."
    return "$WIZARD_STAGE_FAIL"
  fi
  case "$_PKGS_AGENT_RESULT" in
    failed) worst=failed ;;
    pending) [ "$worst" = "failed" ] || worst=pending ;;
  esac

  # Only when nothing was asked for and nothing went wrong. Telling somebody
  # whose install just failed to "run setup again when you want one" would be
  # answering a question they did not ask with news they already have.
  if [ "$worst" = finished ] && ! _pkgs_here claude && ! _pkgs_here codex; then
    wizard_blank
    wizard_say "there is no coding agent on this computer yet, so ayeaye will"
    wizard_say "have nothing to show you until there is. Everything else setup"
    wizard_say "did is still good; run ./install.sh again when you want one."
    # A choice, and a choice is finished business.
    return "$WIZARD_STAGE_SKIP"
  fi

  case "$worst" in
    failed)  return "$WIZARD_STAGE_FAIL" ;;
    pending) return "$WIZARD_STAGE_PENDING" ;;
  esac
  return "$WIZARD_STAGE_OK"
}

# ====================================== 6c. the status line marker

# The script Claude Code will run. Written here rather than pointed at
# examples/, because a setting pointing into a checkout is a setting that
# breaks the day the checkout moves.
#
# No jq: it is not a dependency of this project and asking for one to print
# eight characters would be absurd. sed reads the two fields that are wanted
# out of the JSON that arrives on standard input.
_pkgs_write_marker_script() {
  local path="${1:-}" dir tmp
  dir="${path%/*}"
  mkdir -p "$dir" 2>/dev/null || return 1
  tmp="$path.tmp.$$"
  cat > "$tmp" <<'MARKER' || { rm -f "$tmp" 2>/dev/null; return 1; }
#!/bin/sh
# The status line ayeaye asked Claude Code for.
#
# Claude Code runs this and shows whatever it prints. ayeaye reads terminal
# windows as plain text and has no other way to tell which conversation a
# window is showing, so the second line below is a short tag naming it. Eight
# characters is enough to find the conversation on disk, and it is dim enough
# to ignore on screen.
#
# The tag is on a line of its own, and first on that line, on purpose: added to
# the end of the folder line it would be cut off the moment a folder name got
# long, and a cut-off tag fails silently.
#
# Safe to edit and safe to delete. Without it the window still works and only
# the conversation view on the phone goes quiet.
input=$(cat | tr -d '\n')

_field() {
  printf '%s' "$input" \
    | sed -n "s/.*\"$1\"[[:space:]]*:[[:space:]]*\"\\([^\"]*\\)\".*/\\1/p" \
    | head -1
}

dir=$(_field current_dir)
[ -n "$dir" ] || dir=$(_field cwd)
case "$dir" in
  "$HOME")   dir="~" ;;
  "$HOME"/*) dir="~${dir#"$HOME"}" ;;
esac
session=$(_field session_id)

[ -n "$dir" ] && printf '\033[34m%s\033[0m\n' "$dir"
if [ -n "$session" ]; then
  printf '\033[90m⟪cc:%s⟫\033[0m\n' "$(printf '%s' "$session" | cut -c1-8)"
fi
exit 0
MARKER
  chmod 755 "$tmp" 2>/dev/null || true
  mv -f "$tmp" "$path" 2>/dev/null || {
    rm -f "$tmp" 2>/dev/null
    return 1
  }
  return 0
}

# _pkgs_settings_probe <path> - what is in the settings file now, as key=value
# lines. Always 0; the state line is the answer.
#
#   state=missing | unreadable | empty | corrupt | ok
#   keys=<the other top-level names, comma separated>
#   hasstatusline=1  when there is a status line of any shape at all
#   statusline=<the command it names, when it names one>
#
# The last two are separate answers on purpose. A status line configured as
# something other than a command - a fixed line of text, say - names no
# command, and reporting only the command would make it invisible: the caller
# would print "everything you have stays exactly as it is" and then replace it.
_pkgs_settings_probe() {
  _pkgs_have python3 || { printf 'state=nopython\n'; return 0; }
  python3 - "${1:-}" <<'PY' 2>/dev/null || printf 'state=nopython\n'
import json, os, sys

path = sys.argv[1]
if not os.path.exists(path):
    print("state=missing")
    sys.exit(0)
try:
    with open(path) as handle:
        raw = handle.read()
except Exception:
    # It is there and it cannot be read. Saying "you have no settings" about a
    # file full of somebody's settings is the one answer that must not be given.
    print("state=unreadable")
    sys.exit(0)
if not raw.strip():
    print("state=empty")
    sys.exit(0)
try:
    data = json.loads(raw)
except Exception:
    print("state=corrupt")
    sys.exit(0)
if not isinstance(data, dict):
    print("state=corrupt")
    sys.exit(0)
print("state=ok")
print("keys=%s" % ",".join(k for k in data.keys() if k != "statusLine"))
if "statusLine" in data:
    print("hasstatusline=1")
    line = data["statusLine"]
    if isinstance(line, dict):
        command = line.get("command")
        if command:
            print("statusline=%s" % command)
    elif line is not None:
        print("statusline=%s" % line)
PY
  return 0
}

# _pkgs_settings_write <path> <command> - add the status line and change nothing
# else.
#
# Everything already in the file comes back out of it, in the order it was
# written in: the file is read, one key is set, and it is written again. A file
# that could not be read at all has already been backed up and agreed to by the
# time this is called.
_pkgs_settings_write() {
  local errors status
  _pkgs_have python3 || return 1
  # Its own words go to the log rather than to the screen: a stack trace in
  # front of somebody who has never opened a terminal explains nothing.
  errors="$(mktemp "${TMPDIR:-/tmp}/ayeaye-settings.XXXXXX" 2>/dev/null)" \
    || errors="${TMPDIR:-/tmp}/ayeaye-settings.$$.err"
  python3 - "${1:-}" "${2:-}" 2>"$errors" <<'PY' 
import json, os, sys

path, command = sys.argv[1], sys.argv[2]

# A settings file that is a symlink - which is what every dotfiles manager
# makes - must stay one, or the file they edit and the file Claude Code reads
# quietly become two different files.
path = os.path.realpath(path)

data = {}
try:
    with open(path) as handle:
        raw = handle.read()
    if raw.strip():
        loaded = json.loads(raw)
        if isinstance(loaded, dict):
            data = loaded
except Exception:
    data = {}

# The other keys of a status line that already runs a command - its padding,
# how often it refreshes - are settings somebody chose and are kept. A status
# line of any other shape is not a command line at all, and merging into it
# would leave half of each; it is replaced, which is what the user was asked.
line = data.get("statusLine")
if not isinstance(line, dict) or line.get("type") != "command":
    line = {}
line["type"] = "command"
line["command"] = command
data["statusLine"] = line

directory = os.path.dirname(path)
if directory and not os.path.isdir(directory):
    os.makedirs(directory)

# Whatever mode the file had, it keeps: a settings file somebody made private
# must not become world-readable because setup added a line to it.
mode = None
try:
    mode = os.stat(path).st_mode & 0o7777
except OSError:
    pass

# Beside the destination and then renamed over it, so an interruption leaves
# either the old file or the new one and never half of either - and never a
# stray .tmp beside somebody's configuration.
tmp = "%s.tmp.%d" % (path, os.getpid())
try:
    with open(tmp, "w") as handle:
        json.dump(data, handle, indent=2)
        handle.write("\n")
    if mode is not None:
        os.chmod(tmp, mode)
    os.rename(tmp, path)
except Exception as exc:
    try:
        os.unlink(tmp)
    except OSError:
        pass
    sys.stderr.write("%s\n" % exc)
    sys.exit(1)
PY
  status=$?
  if [ "$status" != 0 ] && [ -s "$errors" ]; then
    wizard_detail "settings: $(cat "$errors" 2>/dev/null)"
  fi
  rm -f "$errors" 2>/dev/null
  return "$status"
}

_pkgs_probe_value() {
  printf '%s\n' "$2" | sed -n "/^$1=/{s/^$1=//;p;q;}"
}

_pkgs_marker_step() {
  local script settings probe state keys current has_line question

  if ! _pkgs_wants answer.agent.marker; then
    wizard_say "leaving Claude Code's status line alone."
    wizard_say "(Codex needs nothing set up: ayeaye finds its conversations by"
    wizard_say "itself.)"
    return "$WIZARD_STAGE_SKIP"
  fi

  if ! _pkgs_have python3; then
    wizard_say "reading and writing Claude Code's settings needs python3, which"
    wizard_say "is not here, so the settings file has been left alone."
    return "$WIZARD_STAGE_PENDING"
  fi

  script="$(_pkgs_marker_script)"
  settings="$(_pkgs_claude_settings)"

  # Ours, regenerated - unless somebody has edited it, in which case it is
  # theirs and replacing it is a question.
  if [ -e "$script" ]; then
    if ! _pkgs_write_marker_script "$script.check.$$" ; then
      wizard_say "could not write to $(_pkgs_data_dir)."
      rm -f "$script.check.$$" 2>/dev/null
      return "$WIZARD_STAGE_FAIL"
    fi
    if cmp -s "$script" "$script.check.$$" 2>/dev/null; then
      rm -f "$script.check.$$" 2>/dev/null
    else
      rm -f "$script.check.$$" 2>/dev/null
      wizard_replace "$script" "you have changed $script since setup wrote it. May I write it again?"
      case "$?" in
        0) _pkgs_write_marker_script "$script" || {
             wizard_say "could not write $script."
             return "$WIZARD_STAGE_FAIL"
           } ;;
        2)
          wizard_say "setup asked about a file without naming one. That is a"
          wizard_say "fault in setup itself; nothing has been changed."
          return "$WIZARD_STAGE_FAIL"
          ;;
        *) wizard_say "keeping your version of $script." ;;
      esac
    fi
  else
    _pkgs_write_marker_script "$script" || {
      wizard_say "could not write $script."
      return "$WIZARD_STAGE_FAIL"
    }
    wizard_say "wrote $script"
  fi

  probe="$(_pkgs_settings_probe "$settings")"
  state="$(_pkgs_probe_value state "$probe")"
  keys="$(_pkgs_probe_value keys "$probe")"
  current="$(_pkgs_probe_value statusline "$probe")"
  has_line="$(_pkgs_probe_value hasstatusline "$probe")"

  if [ "$current" = "$script" ]; then
    wizard_item ok "Claude Code is already set up to print the marker"
    return "$WIZARD_STAGE_OK"
  fi

  wizard_blank
  case "$state" in
    missing|empty)
      wizard_say "Claude Code has no settings of yours to change: setup would"
      wizard_say "create $settings and put one thing in it."
      ;;
    unreadable)
      wizard_say "$settings is there and setup is not allowed to read it, so it"
      wizard_say "has been left exactly as it is."
      wizard_say "Nothing about Claude Code has changed. To get the marker, point"
      wizard_say "its status line at $script yourself."
      return "$WIZARD_STAGE_PENDING"
      ;;
    corrupt)
      wizard_say "$settings is there but is not something this can read."
      wizard_say "Setup will not guess at what it means. It can save a copy of it"
      wizard_say "and write a fresh one containing only the status line, and"
      wizard_say "whatever was in it is then in the copy and nowhere else."
      ;;
    ok)
      # The list is everything except the status line, because the status line
      # is the one thing that is not about to stay exactly as it is.
      wizard_say "$settings already has: ${keys:-nothing else}"
      wizard_say "All of that stays exactly as it is."
      if [ "$has_line" = 1 ]; then
        if [ -n "$current" ]; then
          wizard_say "It also already has a status line of its own, which runs:"
          wizard_say "  $current"
        else
          wizard_say "It also already has a status line of its own, which shows"
          wizard_say "something other than the output of a command."
        fi
        wizard_say "ayeaye's prints the folder and the marker, and nothing else,"
        wizard_say "so replacing yours would lose whatever else yours shows."
      fi
      ;;
  esac
  wizard_say "The one change: Claude Code would run $script"
  wizard_say "and show what it prints."
  wizard_blank

  if [ "$has_line" = 1 ] && [ "$state" = ok ]; then
    if ! wizard_confirm "replace your status line with ayeaye's?" "n"; then
      wizard_say "keeping your status line, and $settings is unchanged."
      wizard_say "To get the marker without losing what your own status line"
      wizard_say "shows, copy the last few lines of"
      wizard_say "  $script"
      if [ -n "$current" ]; then
        wizard_say "onto the end of"
        wizard_say "  $current"
      else
        wizard_say "into whatever draws your status line."
      fi
      wizard_say "The marker has to be printed on a line of its own."
      return "$WIZARD_STAGE_PENDING"
    fi
  fi

  if [ -e "$settings" ]; then
    # Asks, and takes the copy itself. 0 means the old version is already safe.
    if [ "$state" = corrupt ]; then
      question="may I save a copy of $settings and write a fresh one with only the status line in it?"
    else
      question="may I add the status line to $settings?"
    fi
    wizard_replace "$settings" "$question"
    case "$?" in
      0) ;;
      3)
        wizard_say "leaving $settings alone. Nothing about Claude Code changed."
        return "$WIZARD_STAGE_SKIP"
        ;;
      2)
        wizard_say "setup asked about a file without naming one. That is a fault"
        wizard_say "in setup itself; nothing has been changed."
        return "$WIZARD_STAGE_FAIL"
        ;;
      *)
        wizard_say "could not save a copy of $settings, so it was left alone."
        return "$WIZARD_STAGE_FAIL"
        ;;
    esac
  else
    if ! wizard_confirm "create $settings with the status line in it?" "y"; then
      wizard_say "leaving Claude Code's settings alone."
      return "$WIZARD_STAGE_SKIP"
    fi
  fi

  if ! _pkgs_settings_write "$settings" "$script"; then
    wizard_say "could not write $settings."
    return "$WIZARD_STAGE_FAIL"
  fi
  wizard_item ok "Claude Code will print the marker from now on"
  wizard_say "wrote $settings"
  wizard_say "(Codex needs nothing set up: ayeaye finds its conversations by"
  wizard_say "itself.)"
  return "$WIZARD_STAGE_OK"
}

# ============================================== 6d. the project board

# _pkgs_cliban_ready <path> - it runs, its database exists, and ayeaye can find
# it. The three things "installed" has to mean before this says so.
_pkgs_cliban_ready() {
  local path="${1:-}" db out status=0
  if ! out="$("$path" --help </dev/null 2>&1)"; then
    wizard_detail "$out"
    wizard_say "cliban is on this computer but will not start."
    return 1
  fi
  wizard_item ok "cliban runs"

  db="$(_pkgs_cliban_db)"
  if [ -s "$db" ]; then
    wizard_item ok "its board is at $db"
  else
    # cliban makes its database the first time it is asked anything. Asking is
    # a read; it creates nothing but its own empty board.
    wizard_detail "creating the cliban database: $path project ls --json"
    if out="$("$path" project ls --json </dev/null 2>&1)" && [ -s "$db" ]; then
      wizard_item ok "started an empty board at $db"
    else
      wizard_detail "$out"
      wizard_say "cliban runs, but its board could not be created at $db."
      status=1
    fi
  fi

  # bin/ayeaye looks on PATH, and the background service does not have the same
  # PATH your terminal does - so it is told where the program is rather than
  # left to find it.
  _pkgs_cliban_tell_ayeaye "$path" || status=1
  return "$status"
}

_pkgs_cliban_tell_ayeaye() {
  local path="${1:-}" env_file backup
  env_file="$(_pkgs_env_file)"
  if [ ! -s "$env_file" ]; then
    wizard_say "ayeaye has no settings file yet, so put this line in it when"
    wizard_say "there is one:  VOICE_CLIBAN=$path"
    return 1
  fi
  if [ "$(wizard_env_get "$env_file" VOICE_CLIBAN "")" = "$path" ]; then
    wizard_item ok "ayeaye knows where cliban is"
    return 0
  fi
  # A copy first, and no copy means no change. This is the user's settings
  # file, it has been theirs since the settings stage wrote it, and the same
  # rule install.sh follows for the same file applies here.
  if ! backup="$(wizard_backup "$env_file")"; then
    wizard_say "could not save a copy of your settings, so they have been left"
    wizard_say "alone. To make the board work, add this line to $env_file:"
    wizard_say "  VOICE_CLIBAN=$path"
    return 1
  fi
  wizard_detail "settings backed up to $backup"
  if wizard_env_merge "$env_file" "VOICE_CLIBAN=$path"; then
    wizard_item ok "told ayeaye where cliban is"
    return 0
  fi
  wizard_say "could not record where cliban is in $env_file."
  wizard_say "add this line to it by hand:  VOICE_CLIBAN=$path"
  return 1
}

# _pkgs_cliban_cargo - the advanced route, offered and never assumed.
#
# It compiles the program from its source - cliban is published on crates.io,
# so this is the release's own source rather than whatever is on a branch -
# which takes several minutes and a rust toolchain. Somebody who already has
# cargo knows what that means; anybody else is better served by being told the
# download did not work.
_pkgs_cliban_cargo() {
  if ! _pkgs_have cargo; then
    wizard_say "the other way to install cliban is to build it from its source,"
    wizard_say "which needs the rust toolchain from https://rustup.rs and takes"
    wizard_say "several minutes:"
    wizard_say "  cargo install cliban"
    return 1
  fi
  if [ "${WIZARD_INTERACTIVE:-1}" = 0 ]; then
    # Several minutes of this computer's whole attention is not something to
    # start on somebody's behalf while they are not watching.
    wizard_say "this computer could build cliban from its source instead, which"
    wizard_say "takes several minutes. Run ./install.sh again to be asked."
    return 1
  fi
  wizard_blank
  wizard_say "this computer can build cliban from its source instead. That is"
  wizard_say "the long way round: it compiles the program, which takes several"
  wizard_say "minutes and a lot of this computer's attention."
  # One question, not two. wizard_privileged asks it and shows the command.
  wizard_privileged "build cliban from source now? It takes several minutes." \
    "cargo install cliban </dev/null"
}

# _pkgs_verify_checksum <archive> <artifact-name> <sums-file> - 0 to carry on,
# 1 to stop.
#
# Says which of the three things happened, every time, because silence here
# reads as "verified" and is the one thing this must never mean: the checksum
# matched; there was nothing to compare, and which half was missing; or a
# mismatch - which stops the board, and only the board.
_pkgs_verify_checksum() {
  local archive="${1:-}" name="${2:-}" sums="${3:-}" expected actual
  if [ ! -s "$sums" ]; then
    wizard_say "not checked: the checksums cliban publishes beside its releases"
    wizard_say "did not arrive, so the only thing protecting this download was"
    wizard_say "the encrypted connection to the server."
    return 0
  fi
  expected="$(awk -v want="$name" \
    '{ n = $2; sub(/^\*/, "", n); if (n == want) { print $1; exit } }' "$sums")"
  if [ -z "$expected" ]; then
    wizard_say "not checked: the checksums published with this cliban release say"
    wizard_say "nothing about $name, so there was nothing to compare against."
    return 0
  fi
  actual="$(_pkgs_digest "$archive")" || actual=""
  if [ -z "$actual" ]; then
    wizard_say "not checked: this computer has no sha256sum, shasum or openssl,"
    wizard_say "so the checksum cliban publishes could not be compared."
    return 0
  fi
  if [ "$actual" != "$expected" ]; then
    wizard_say "what was downloaded is not what cliban published, so nothing has"
    wizard_say "been unpacked or installed. This can be an interrupted download,"
    wizard_say "and it can be somebody in the way of this one. Try again, and if"
    wizard_say "it happens twice do not run it."
    wizard_detail "checksum expected $expected"
    wizard_detail "checksum received $actual"
    return 1
  fi
  wizard_item ok "the download matches the checksums published beside it"
  return 0
}

_pkgs_board_step() {
  local work target name url archive member sums status found dest

  if ! _pkgs_wants answer.board; then
    wizard_say "no project board. Everything ayeaye puts on your phone works"
    wizard_say "without it: starting agents, reading them, talking to them and"
    wizard_say "approving what they ask. Only the board page and the links to"
    wizard_say "tickets need cliban, and setup can add it whenever you want."
    return "$WIZARD_STAGE_SKIP"
  fi

  if ! _pkgs_may_fetch unpack && ! _pkgs_here cliban; then
    wizard_say "setup has no way to fetch cliban yet, so the project board is"
    wizard_say "left for another run. Everything else works."
    return "$WIZARD_STAGE_PENDING"
  fi

  dest="$(_pkgs_bin_dir)/cliban"
  if found="$(_pkgs_find cliban)"; then
    wizard_item ok "cliban is already here"
    wizard_detail "cliban: $found"
    _pkgs_cliban_ready "$found" || return "$WIZARD_STAGE_PENDING"
    return "$WIZARD_STAGE_OK"
  fi

  if ! target="$(_pkgs_target)"; then
    wizard_say "cliban does not publish a ready-made program for this kind of"
    wizard_say "computer ($(platform_os), $(platform_arch))."
    if _pkgs_cliban_cargo; then
      if found="$(_pkgs_find cliban)"; then
        _pkgs_cliban_ready "$found" || return "$WIZARD_STAGE_PENDING"
        return "$WIZARD_STAGE_OK"
      fi
    fi
    return "$WIZARD_STAGE_PENDING"
  fi

  work="$(_pkgs_workdir)" || {
    wizard_say "could not make a temporary folder to download into."
    return "$WIZARD_STAGE_FAIL"
  }
  # The versionless alias of the latest release, and the checksums beside it
  # on the same consent. The alias holds a versionless directory too, which is
  # what lets the member be named here without knowing the version.
  name="cliban-$target"
  member="$name/cliban"
  url="$_PKGS_CLIBAN_BASE/$name.tar.gz"
  archive="$work/$name.tar.gz"
  sums="$work/SHA256SUMS"

  wizard_download "may I download cliban, the project board program?" \
    "$url" "$archive" "$_PKGS_CLIBAN_BYTES" "$_PKGS_CLIBAN_BASE/SHA256SUMS" "$sums"
  status=$?
  if [ "$status" = 3 ]; then
    rm -rf "$work" 2>/dev/null
    wizard_say "nothing was downloaded. The board page will stay empty, and"
    wizard_say "everything else works."
    return "$WIZARD_STAGE_SKIP"
  fi
  if [ "$status" != 0 ]; then
    rm -rf "$work" 2>/dev/null
    wizard_say "cliban could not be downloaded, so there is no project board."
    wizard_say "Everything else ayeaye does is unaffected."
    # The route cliban itself documents first, for whoever wants to do this by
    # hand - with the one thing about it setup would never do on its own said
    # out loud: the formula installs cliband, the multi-user server, as well.
    if platform_has_brew; then
      wizard_say "Homebrew on this computer can install it too, though that way"
      wizard_say "also installs cliband, its multi-user server:"
      wizard_say "  brew install lioralabs/tap/cliban"
    fi
    _pkgs_cliban_cargo || return "$WIZARD_STAGE_FAIL"
    if found="$(_pkgs_find cliban)"; then
      _pkgs_cliban_ready "$found" || return "$WIZARD_STAGE_PENDING"
      return "$WIZARD_STAGE_OK"
    fi
    return "$WIZARD_STAGE_FAIL"
  fi

  if ! _pkgs_verify_checksum "$archive" "$name.tar.gz" "$sums"; then
    rm -rf "$work" 2>/dev/null
    return "$WIZARD_STAGE_FAIL"
  fi

  if ! _pkgs_verify_archive "$archive" "$member"; then
    rm -rf "$work" 2>/dev/null
    wizard_say "what was downloaded is not the cliban release it should be, so"
    wizard_say "nothing has been unpacked or installed."
    return "$WIZARD_STAGE_FAIL"
  fi

  # One member out of four, by name. cliban's archive also carries cliband, the
  # multi-user server, and this project has no business putting a server on
  # somebody's computer - so it is never even written to disk.
  if ! tar -xzf "$archive" -C "$work" "$member" 2>/dev/null; then
    rm -rf "$work" 2>/dev/null
    wizard_say "the download could not be unpacked."
    return "$WIZARD_STAGE_FAIL"
  fi

  _pkgs_install_binary "$work/$member" cliban
  status=$?
  rm -rf "$work" 2>/dev/null
  if [ "$status" = 3 ]; then
    wizard_say "leaving the cliban you already have alone."
    return "$WIZARD_STAGE_SKIP"
  fi
  if [ "$status" != 0 ]; then
    wizard_say "cliban was downloaded but could not be put in $(_pkgs_bin_dir)."
    return "$WIZARD_STAGE_FAIL"
  fi
  wizard_say "installed $dest"

  if ! found="$(_pkgs_find cliban)"; then
    wizard_say "cliban was installed and then could not be found."
    return "$WIZARD_STAGE_FAIL"
  fi
  _pkgs_say_path_note "$found"
  _pkgs_cliban_ready "$found" || return "$WIZARD_STAGE_PENDING"
  return "$WIZARD_STAGE_OK"
}

# ============================================================ registration

wizard_step detect    software _pkgs_detect_step \
  "Coding agents and the project board"                  required always
wizard_step configure software _pkgs_choose_step \
  "Which agents, and whether you want the board"
wizard_step install   packages _pkgs_requirements_step \
  "The software ayeaye needs"
wizard_step install   agents   _pkgs_agents_step \
  "Your coding agents"                                   optional
wizard_step install   marker   _pkgs_marker_step \
  "The Claude Code session marker"                       optional
wizard_step install   board    _pkgs_board_step \
  "cliban, for the project board"                        optional
