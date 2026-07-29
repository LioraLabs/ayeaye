# Talking out loud: which listening model, and where it comes from.
#
# Voice is the one part of ayeaye that can cost somebody a gigabyte and an
# afternoon, so it is the one part that must never become a trap. The floor is
# text-only and the floor always works: a run that fetches nothing here still
# ends with a working ayeaye you type at. Everything below is an offer.
#
# This file attaches to two stages through the seam in lib/steps/README.md:
#
#   configure  the question - which experience, and the whole cost of it in
#              front of the person before a single byte moves
#   install    the work - ffmpeg, whisper, the model, and the settings that
#              make the app agree with what was just installed
#
# Both are registered `optional`. Voice failing is not the run failing.
#
# ------------------------------------------------- selection, not judgement
#
# lib/steps/20-hardware.sh has already decided what this machine can carry,
# and no gate in this file recomputes that verdict. `hw_tier_at_least` is
# asked, and its answer is turned into an offer or into a sentence explaining
# why there is no offer; there is no megabyte, disk or VRAM comparison behind
# any of them. `hw_acceleration` says what kind of acceleration exists and
# `hw_accel_usable` says whether it is worth acting on - a two gigabyte card
# is honestly `cuda` and is honestly no use for holding a model, and the
# difference between those two questions is exactly the "offered only on
# suitable hardware" gate this file implements.
#
# One number is chosen here rather than asked for, and it is worth naming so
# that it is not mistaken for a second opinion: `_voice_threads` clamps the
# measured core count into a range. That is a runtime setting for a program
# this file installs - how much of the machine transcription may take while it
# runs - and not a judgement about what the machine can do. Nothing branches
# on it and no offer depends on it.
#
# -------------------------------------------- what the app actually probes
#
# `bin/ayeaye`'s voice_available() asks for ffmpeg, and then for either the
# whisper server answering on VOICE_WHISPER_SERVER or the `whisper-cpp`
# command with the file named by VOICE_WHISPER_MODEL present on disk. Until
# this file existed nothing in setup ever wrote VOICE_WHISPER_MODEL, which had
# two consequences nobody had noticed: lib/steps/70-service.sh declines to
# install the whisper service unless that setting names a model, so it
# declined on every machine there is; and the app's fallback route could only
# ever look at a default path that setup had never put a file at. So the
# install step here writes that setting, pointing at the file it really
# fetched, and it does it in the install stage - before the service stage runs
# - so that the service step finds a model named and installs the service that
# the app's first probe route looks for.
#
# ------------------------------------------------ the voice-agent service
#
# There is deliberately no voice-agent service installed here, and that was a
# decision rather than an omission. voice-agent is the microphone recorder,
# and it belongs on the device you speak into - the laptop or phone you SSH in
# from - not on the machine ayeaye runs on. The phone web UI's own dictation
# path does not use it at all, voice_available() does not probe for it, and a
# unit that grabs a microphone and binds a port on a headless server is
# exactly the surprise this milestone exists to avoid. lib/steps/70-service.sh
# will keep an existing voice-agent definition up to date for somebody who
# wrote one; setup does not write the first one. bin/voice-dictate-setup
# prints what a client device needs instead.
#
# --------------------------------------------------------------- the rules
#
#   Nothing is installed, downloaded or overwritten except through
#   lib/consent.sh. tests/cases/wizard_contract_test.sh reads this file and
#   fails the suite if that stops being true.
#
#   No step reports OK for work that did not happen. "The user chose text
#   only" is SKIP - finished business. "The download was refused" is SKIP.
#   "It was fetched and it did not verify" is PENDING.
#
#   Standard input is fd 8. Every question goes through wizard_ask or
#   wizard_confirm and everything else runs with standard input closed.
#
# bash 3.2: no associative arrays, no ${var,,}, no mapfile, no local -n.

# ===================================================== 1. what there is to get
#
# whisper.cpp's weights are published as one file per model in one repository,
# and that repository is a git-lfs one - which is the whole reason a checksum
# is available to verify against. Every size and every sha256 below is the one
# upstream publishes in that file's own lfs pointer, not a figure measured
# here.
#
# The URL base is overridable so that a test can point it somewhere that does
# not exist and still assert the exact request that would have been made.

_VOICE_MODEL_BASE="${AYEAYE_WHISPER_MODEL_URL:-https://huggingface.co/ggerganov/whisper.cpp/resolve/main}"

# Said to a person, so it names a place rather than a URL.
_VOICE_MODEL_SOURCE="huggingface.co, where the people who make it publish it"

# Where whisper.cpp itself lives, for the machines that have no package for it.
_VOICE_WHISPER_HOME="https://github.com/ggerganov/whisper.cpp"

# voice_models - every model this file knows, smallest first.
voice_models() {
  printf 'tiny.en\nbase.en\nsmall.en\nmedium.en\nlarge-v3-turbo\n'
}

# _voice_model_field <model> <field> -> the value, or nothing and status 1.
#
# Fields:
#   bytes    the artifact's exact size, from the lfs pointer
#   sha256   the artifact's checksum, from the same place
#   ram_mb   room to hold it and work in while it runs, rounded up. Not the
#            file size: whisper.cpp needs the weights plus scratch, and the
#            figures here follow the project's own published memory table.
#   tier     the lowest verdict from 20-hardware.sh that this model is within.
#            Named rather than inferred: "is this bigger than the machine was
#            measured for" is a question about tiers, and answering it by
#            comparing file sizes would be this file forming an opinion about
#            hardware, which is the one thing it may not do.
#   words    what it is, to somebody who has never heard of a model
_voice_model_field() {
  case "${1:-}:${2:-}" in
    tiny.en:bytes)   printf '77704715' ;;
    tiny.en:sha256)  printf '921e4cf8686fdd993dcd081a5da5b6c365bfde1162e72b08d75ac75289920b1f' ;;
    tiny.en:ram_mb)  printf '390' ;;
    tiny.en:tier)          printf 'lightweight' ;;
    tiny.en:words)   printf 'the smallest one there is: quick everywhere, and it will mishear names' ;;

    base.en:bytes)   printf '147964211' ;;
    base.en:sha256)  printf 'a03779c86df3323075f5e796cb2ce5029f00ec8869eee3fdfb897afe36c6d002' ;;
    base.en:ram_mb)  printf '500' ;;
    base.en:tier)          printf 'lightweight' ;;
    base.en:words)   printf 'a little better than the smallest, and still small' ;;

    small.en:bytes)  printf '487614201' ;;
    small.en:sha256) printf 'c6138d6d58ecc8322097e0f987c32f1be8bb0a18532a3f88f734d1bbf9c41e5d' ;;
    small.en:ram_mb) printf '1000' ;;
    small.en:tier)         printf 'recommended' ;;
    small.en:words)  printf 'the balanced one: accurate enough to trust, small enough to be quick' ;;

    medium.en:bytes)  printf '1533774781' ;;
    medium.en:sha256) printf 'cc37e93478338ec7700281a7ac30a10128929eb8f427dda2e865faa8f6da4356' ;;
    medium.en:ram_mb) printf '2600' ;;
    medium.en:tier)        printf 'maximum' ;;
    medium.en:words)  printf 'more accurate again, and noticeably slower without a graphics card' ;;

    large-v3-turbo:bytes)  printf '1624555275' ;;
    large-v3-turbo:sha256) printf '1fc70f774d38eb169993ac391eea357ef47c88757ef72ee5943879b7e8e2bc69' ;;
    large-v3-turbo:ram_mb) printf '2800' ;;
    large-v3-turbo:tier)   printf 'maximum' ;;
    large-v3-turbo:words)  printf 'the most accurate one that is still fast, on a machine with room for it' ;;

    *) return 1 ;;
  esac
  return 0
}

# _voice_model_url <model> - where that file is.
_voice_model_url() {
  local model="${1:-}"
  _voice_model_field "$model" bytes >/dev/null || return 1
  printf '%s/ggml-%s.bin' "$_VOICE_MODEL_BASE" "$model"
}

# _voice_model_path <model> - where it lands.
#
# HW_MODEL_DIR, and not a name of this file's own: the hardware step measured
# the free space on the filesystem that directory is on in order to reach its
# verdict, and putting the file somewhere else would make that measurement a
# measurement of the wrong disk.
_voice_model_path() {
  printf '%s/ggml-%s.bin' "${HW_MODEL_DIR:-$HOME/whisper-models}" "${1:-}"
}

# ======================================================== 2. the four presets
#
# A person picks an experience. The model is a consequence of the experience,
# and is named in the details rather than in the question.

# voice_presets - in the order they are offered.
voice_presets() {
  printf 'text-only\nlightweight\nrecommended\nmaximum\n'
}

# _voice_preset_model <preset> -> the model it means, or nothing for text-only.
_voice_preset_model() {
  case "${1:-}" in
    text-only)   printf '' ;;
    lightweight) printf 'tiny.en' ;;
    recommended) printf 'small.en' ;;
    maximum)     printf 'large-v3-turbo' ;;
    *) return 1 ;;
  esac
  return 0
}

# _voice_preset_words <preset> - the offer, in one line.
_voice_preset_words() {
  case "${1:-}" in
    text-only)   printf 'type to your agents, and download nothing' ;;
    lightweight) printf 'talk to your agents, with a small quick listener' ;;
    recommended) printf 'talk to your agents, and have what you said tidied up' ;;
    maximum)     printf 'the most accurate listening this computer has room for' ;;
    *) return 1 ;;
  esac
  return 0
}

# _voice_preset_tier <preset> - the tier a machine has to have reached.
_voice_preset_tier() {
  case "${1:-}" in
    text-only)   printf 'text-only' ;;
    lightweight) printf 'lightweight' ;;
    recommended) printf 'recommended' ;;
    maximum)     printf 'maximum' ;;
    *) return 1 ;;
  esac
  return 0
}

# ================================================ 3. which backend, really
#
# Two questions, and they are not the same one. hw_acceleration says what kind
# of acceleration this machine has, as a statement of fact. hw_accel_usable
# says whether that acceleration is worth acting on. A two gigabyte NVIDIA
# card answers `cuda` to the first and no to the second, and the honest
# backend for it is the processor.

_voice_backend() {
  if hw_accel_usable; then
    hw_acceleration
  else
    printf 'cpu'
  fi
  return 0
}

# _voice_backend_words <backend> - what that means, without a library name.
_voice_backend_words() {
  case "${1:-}" in
    metal) printf 'the graphics built into this Mac' ;;
    cuda)  printf 'your NVIDIA graphics card' ;;
    rocm)  printf 'your AMD graphics card' ;;
    *)     printf "this computer's processor" ;;
  esac
  return 0
}

# _voice_threads - how many processor threads whisper should use.
#
# From the core count the hardware step already measured, never from a guess,
# and never more than eight: whisper.cpp stops getting faster somewhere around
# there and a machine that gives all of itself to transcription is a machine
# that stutters at everything else. Two is the floor, and an unreadable core
# count gets the floor rather than an invention.
_voice_threads() {
  local cores
  cores="$(wizard_state_get step.detect.hardware.cores unknown)"
  case "$cores" in
    ""|*[!0-9]*) printf '2'; return 0 ;;
  esac
  cores=$((10#$cores))
  [ "$cores" -lt 2 ] && cores=2
  [ "$cores" -gt 8 ] && cores=8
  printf '%s' "$cores"
  return 0
}

# ==================================================== 4. can whisper be had
#
# The program, as opposed to the weights. A model with nothing to run it is
# half a gigabyte of nothing, so this is asked before any preset is offered.
#
# Two families really package whisper.cpp and the rest do not, which is a
# statement about the world rather than about this project: Debian, Fedora and
# openSUSE have no package for it at the time of writing, and offering to
# install one that does not exist would be a promise this file cannot keep.
# Where there is none, the presets that need it are blocked and the reason is
# said out loud with somewhere to go next.

# _voice_whisper_here - the whisper program this computer already has, or
# nothing. Read out of the state file: the detect stage looked for exactly
# this and a resumed run must reach the same answer without probing again.
_voice_whisper_here() {
  wizard_state_get step.detect.tools.whisper_command ""
}

# _voice_whisper_package - the package name for this family and this backend,
# or nothing when this family has none.
_voice_whisper_package() {
  local family backend
  family="$(platform_family)"
  backend="${1:-$(_voice_backend)}"
  case "$family" in
    macos)
      platform_has_brew || return 0
      # Homebrew builds it with Metal on Apple Silicon; there is one formula.
      printf 'whisper-cpp'
      ;;
    arch)
      case "$backend" in
        cuda) printf 'whisper.cpp-cuda' ;;
        rocm) printf 'whisper.cpp-hipblas' ;;
        *)    printf 'whisper.cpp' ;;
      esac
      ;;
    *) return 0 ;;
  esac
  return 0
}

# _voice_whisper_obtainable - status 0 when this machine either has whisper or
# can be given it.
_voice_whisper_obtainable() {
  [ -n "$(_voice_whisper_here)" ] && return 0
  [ -n "$(_voice_whisper_package)" ] || return 1
  platform_pkg_can_act
}

# =================================================== 5. what is on offer here
#
# One function decides every gate in this file, and it decides them by asking
# the hardware step rather than by measuring anything.

# _voice_preset_blocker <preset> -> one word, or nothing when the preset is on
# offer. `tier` the machine was measured smaller than this needs, `whisper`
# there is no program to run a model with, `network` this computer is offline.
_voice_preset_blocker() {
  local preset="${1:-}" tier
  tier="$(_voice_preset_tier "$preset")" || return 2
  [ "$preset" = "text-only" ] && return 0

  _voice_whisper_obtainable || { printf 'whisper'; return 0; }
  if [ "$(wizard_state_get step.detect.hardware.network unknown)" = offline ]; then
    printf 'network'
    return 0
  fi
  hw_tier_at_least "$tier" || { printf 'tier'; return 0; }
  return 0
}

# _voice_blocker_sentence <preset> <word> - why that option is not there.
#
# Never left to be inferred. An option that is simply absent reads as a
# limitation of the software; an option that is named and explained reads as a
# measurement of this computer, which is what it is.
_voice_blocker_sentence() {
  local preset="${1:-}" word="${2:-}" reason
  case "$word" in
    tier)
      reason="$(hw_tier_reason)"
      if [ -n "$reason" ]; then
        printf 'not offered here: %s' "$reason"
      else
        printf 'not offered here: this computer was measured smaller than it needs'
      fi
      ;;
    whisper)
      # The reason differs by machine and the difference is actionable: on a
      # Mac the program exists and Homebrew is what is missing, everywhere
      # else there is no package at all and the project's own page is where
      # to go. Telling a Mac user "there is no package for it" would be
      # false, and would send them looking in the wrong place.
      if [ "$(platform_family)" = macos ] && ! platform_has_brew; then
        printf 'not offered here: nothing on this computer can turn speech into words yet, and installing it needs Homebrew, which is not on this Mac. Homebrew is at https://brew.sh'
      else
        printf 'not offered here: nothing on this computer can turn speech into words yet, and this system has no package for one. whisper.cpp is at %s' \
          "$_VOICE_WHISPER_HOME"
      fi
      ;;
    network)
      printf 'not offered here: this computer has no way to reach the internet right now'
      ;;
    *)
      printf 'not offered here'
      ;;
  esac
  return 0
}

# ============================================ 5b. tidying up what was heard
#
# ollama is optional even inside voice: bin/voice-dictate degrades to the raw
# transcription when it is not there, and nothing about ayeaye stops working.
# Three things are true about it and all three shape what is offered:
#
#   Setup will not install ollama. It is published as a shell script piped
#   into a shell, and this project has exactly one exemption from the "ask
#   before you fetch" rule, in install.sh's bootstrap. So the cleanup model is
#   offered where ollama is already here, and where it is not, ollama is named
#   with somewhere to go rather than installed behind somebody's back.
#
#   Only instruct models. A coder model answers the dictation instead of
#   tidying it - ask it to rewrite "check why fetch user throws" and it writes
#   you a function. env.template has warned about this for as long as it has
#   existed; this is the same warning at the point the choice is made.
#
#   It is not offered on a weak processor-only machine. A rewrite model is
#   several gigabytes and runs after every single thing you say, and on a
#   machine with no graphics card and not much of anything else that turns a
#   two second dictation into a fifteen second one.

_VOICE_OLLAMA_HOME="https://ollama.com"

# _voice_cleanup_model - the instruct model to offer here, sized to what the
# hardware step measured. Never a coder model, and never a base model.
_voice_cleanup_model() {
  if hw_tier_at_least recommended; then
    printf 'qwen2.5:7b-instruct'
  else
    printf 'qwen2.5:3b-instruct'
  fi
  return 0
}

# _voice_cleanup_bytes <model> - roughly what ollama will pull, for the plan
# estimate. The quantised weights ollama publishes, rounded up.
_voice_cleanup_bytes() {
  case "${1:-}" in
    qwen2.5:7b-instruct) printf '4700000000' ;;
    qwen2.5:3b-instruct) printf '1900000000' ;;
    *)                   printf '0' ;;
  esac
  return 0
}

# _voice_cleanup_ram <model> - room to hold it and work in, in megabytes.
# The weights plus ollama's own working set, rounded up.
_voice_cleanup_ram() {
  case "${1:-}" in
    qwen2.5:7b-instruct) printf '5600' ;;
    qwen2.5:3b-instruct) printf '2500' ;;
    *)                   printf '0' ;;
  esac
  return 0
}

# _voice_show_cleanup_cost <model> - the same summary the listening model
# gets, for a download that is larger than any of them.
#
# It gets the whole treatment and not a sentence for exactly that reason: the
# cleanup model is the biggest thing this file will ever fetch, and "show the
# sizes before downloading" was never a rule about the first download only.
_voice_show_cleanup_cost() {
  local model="${1:-}" bytes ram
  bytes="$(_voice_cleanup_bytes "$model")"
  [ "$bytes" = 0 ] && return 1
  ram="$(_voice_cleanup_ram "$model")"

  wizard_detail "voice: cleanup model $model, about $bytes bytes, via ollama pull"

  wizard_blank
  wizard_say "Before anything is downloaded, here is the whole of it:"
  wizard_item "download" "about $(_voice_mb "$bytes") MB, once"
  wizard_item "disk" "about $(_voice_mb "$bytes") MB kept on this computer"
  wizard_item "memory" "about $ram MB while it is tidying"
  wizard_item "using" "$(_voice_backend_words "$(_voice_backend)")"
  wizard_item "from" "ollama, which fetches it from its own library"
  wizard_item "remove" "run: ollama rm $model"
  wizard_item "change" "run this setup again and answer differently"
  return 0
}

# _voice_cleanup_blocker -> one word, or nothing when it can be offered.
#   ollama  it is not installed here, and setup will not install it
#   weak    no graphics card and not enough machine to carry it
_voice_cleanup_blocker() {
  [ "$(wizard_state_get step.detect.tools.ollama 0)" = 1 ] || { printf 'ollama'; return 0; }
  if [ "$(_voice_backend)" = cpu ] && ! hw_tier_at_least recommended; then
    printf 'weak'
    return 0
  fi
  return 0
}

_voice_cleanup_sentence() {
  case "${1:-}" in
    ollama)
      printf 'not offered here: that needs a separate program called ollama, which is not on this computer. It is at %s, and setup will notice it next time.' \
        "$_VOICE_OLLAMA_HOME" ;;
    weak)
      printf 'not offered here: it would run after everything you say, and on a computer this size with no graphics card that would make talking slower than typing.' ;;
    *) printf 'not offered here' ;;
  esac
  return 0
}

# _voice_ask_cleanup - the question, and the plan item behind it.
_voice_ask_cleanup() {
  local blocker model bytes def
  blocker="$(_voice_cleanup_blocker)"
  model="$(_voice_cleanup_model)"

  wizard_blank
  wizard_say "ayeaye can also tidy up what it heard before it types it: capital"
  wizard_say "letters, punctuation, and the names of things on your screen spelled"
  wizard_say "the way they are spelled."
  if [ -n "$blocker" ]; then
    wizard_item "-" "$(_voice_cleanup_sentence "$blocker")"
    wizard_remember answer.voice.cleanup 0
    return 0
  fi

  bytes="$(_voice_cleanup_bytes "$model")"
  wizard_say "That needs a second model, downloaded by ollama, which is already"
  wizard_say "on this computer. Without it you still get everything you said,"
  wizard_say "just as it was heard."
  _voice_show_cleanup_cost "$model" || {
    wizard_remember answer.voice.cleanup 0
    return 0
  }
  wizard_blank

  # An answer given before is what this run offers, rather than what this run
  # assumes: a person who said no last time is asked again with no already in
  # the brackets, which is a question they can answer with the return key and
  # still change their mind about.
  case "$(wizard_state_get answer.voice.cleanup "")" in
    0) def=n ;;
    1) def=y ;;
    *) def=y ;;
  esac
  if ! wizard_confirm "tidy up what you say?" "$def"; then
    wizard_remember answer.voice.cleanup 0
    return 0
  fi
  wizard_remember answer.voice.cleanup 1
  wizard_remember answer.voice.cleanup_model "$model"
  wizard_plan_add download \
    "a tidying-up model, about $(_voice_mb "$bytes") MB, fetched by ollama" "$bytes"
  return 0
}

# ======================================================= 6. the conversation

# The settings file this run is writing. install.sh owns the name; reading it
# defensively means this file can also be sourced on its own by a test.
_voice_env_file() {
  printf '%s' "${ENV_FILE:-${XDG_CONFIG_HOME:-$HOME/.config}/ayeaye/env}"
}

_voice_have() { command -v "${1:-}" >/dev/null 2>&1; }

# _voice_mb <bytes> - whole megabytes, rounded up, for a person to read.
#
# A million bytes to the megabyte, which is what the file listing beside the
# download says and therefore what somebody comparing the two will see. A
# leading zero in a size would otherwise be read as octal, so the base is
# forced the way every other numeric path in this milestone forces it.
_voice_mb() {
  local bytes="${1:-0}"
  case "$bytes" in
    ""|*[!0-9]*) printf '0'; return 0 ;;
  esac
  printf '%s' "$(( ((10#$bytes) + 999999) / 1000000 ))"
}

# _voice_speed_words <model> <backend> - what "slow" means, in the time it
# takes rather than in a benchmark number.
#
# The processor is never hidden and never dressed up. A slow honest option
# beats an absent one, and somebody who was told it would be slow and finds it
# slow has been dealt with fairly; somebody who was not has not.
_voice_speed_words() {
  local model="${1:-}" backend="${2:-}"
  if [ "$backend" != cpu ]; then
    printf 'the words appear a moment after you stop talking'
    return 0
  fi
  case "$model" in
    tiny.en|base.en)
      printf 'on this processor, expect a few seconds after you stop talking' ;;
    small.en)
      printf 'on this processor, expect about as long again as you spoke' ;;
    *)
      printf 'on this processor, expect several times as long as you spoke - minutes for a long sentence' ;;
  esac
  return 0
}

# _voice_show_cost <model> - the whole cost, before anything is asked.
#
# Everything the requirement names, in one place and in this order: what it
# takes to download, what it takes on disk, what it takes to run, what will do
# the work, where it comes from, how to get rid of it and how to change your
# mind. Raw sizes and the model's own id go to the log; what a person reads is
# megabytes and a sentence.
_voice_show_cost() {
  local model="${1:-}" bytes ram backend path
  bytes="$(_voice_model_field "$model" bytes)" || return 1
  ram="$(_voice_model_field "$model" ram_mb)"
  backend="$(_voice_backend)"
  path="$(_voice_model_path "$model")"

  wizard_detail "voice: model $model, $bytes bytes, sha256 $(_voice_model_field "$model" sha256)"
  wizard_detail "voice: $(_voice_model_url "$model") -> $path"

  wizard_blank
  wizard_say "Before anything is downloaded, here is the whole of it:"
  wizard_item "download" "about $(_voice_mb "$bytes") MB, once"
  wizard_item "disk" "about $(_voice_mb "$bytes") MB kept on this computer"
  wizard_item "memory" "about $ram MB while it is listening"
  wizard_item "using" "$(_voice_backend_words "$backend")"
  wizard_item "speed" "$(_voice_speed_words "$model" "$backend")"
  wizard_item "from" "$_VOICE_MODEL_SOURCE"
  wizard_item "remove" "delete $path, and nothing else changes"
  wizard_item "change" "run this setup again and pick a different one"
  return 0
}

# _voice_default_preset - the preset the question offers when the answer is
# just a newline.
#
# The recommendation is the preset called recommended, when this machine can
# carry it; a smaller machine is offered the small one.
#
# Two things this deliberately does *not* do. It does not hand back an answer
# from a previous run without putting it through the gate again: a computer
# that lost a disk, or a card, or its internet between two runs would
# otherwise be offered last week's answer as its default, and a default is an
# answer somebody gets by pressing return. And it does not consult the
# previous answer at all when the run may not ask - --defaults means take the
# defaults, and a default that quietly downloads a gigabyte is not one.
_voice_default_preset() {
  local previous
  if [ "${WIZARD_INTERACTIVE:-1}" = 0 ]; then
    printf 'text-only'
    return 0
  fi
  previous="$(wizard_state_get answer.voice.preset "")"
  case "$previous" in
    text-only|lightweight|recommended|maximum)
      if [ -z "$(_voice_preset_blocker "$previous")" ]; then
        printf '%s' "$previous"
        return 0
      fi
      ;;
  esac
  if [ -z "$(_voice_preset_blocker recommended)" ]; then
    printf 'recommended'
  elif [ -z "$(_voice_preset_blocker lightweight)" ]; then
    printf 'lightweight'
  else
    printf 'text-only'
  fi
  return 0
}

# _voice_offered_number <offered> <preset> - which number that preset was
# given, or nothing when it was not offered one.
_voice_offered_number() {
  local choice="" name=""
  while IFS=' ' read -r choice name || [ -n "$choice" ]; do
    [ -n "$choice" ] || continue
    if [ "$name" = "$2" ]; then
      printf '%s' "$choice"
      return 0
    fi
  done <<EOF
$1
EOF
  return 1
}

# _voice_choose_step - stage four. One question, and the whole cost of every
# answer to it.
_voice_choose_step() {
  local preset model blocker offered="" n=0 choice reply tries backend
  local custom_ok=0 chosen="" default_preset default_n label

  hw_detect

  wizard_blank
  wizard_say "ayeaye can let you talk to your agents instead of typing at them."
  wizard_say "It listens on this computer - what you say is turned into words"
  wizard_say "here and goes nowhere else."

  # Which of them this machine may be offered, and one sentence for each of
  # the ones it may not. A missing option reads as a limitation of the
  # software; a named and explained one reads as a measurement of this
  # computer, which is what it is.
  wizard_blank
  while IFS= read -r preset || [ -n "$preset" ]; do
    [ -n "$preset" ] || continue
    blocker="$(_voice_preset_blocker "$preset")"
    if [ -n "$blocker" ]; then
      wizard_item "-" "$(_voice_preset_words "$preset") - $(_voice_blocker_sentence "$preset" "$blocker")"
      continue
    fi
    n=$((n + 1))
    offered="$offered$n $preset
"
    model="$(_voice_preset_model "$preset")"
    label="$(_voice_preset_words "$preset")"
    if [ -n "$model" ]; then
      label="$label (about $(_voice_mb "$(_voice_model_field "$model" bytes)") MB)"
    fi
    wizard_item "$n" "$label"
  done <<EOF
$(voice_presets)
EOF

  # Choosing the model yourself is an override of the measurement, and an
  # override is only on offer where there is a measurement to override. A
  # computer with no program to run a model, no internet, or too little of
  # itself to run even the smallest listener is not being kept from a
  # decision - there is no decision there to make. It is said out loud for the
  # same reason every blocked preset is: an option that simply vanishes reads
  # as a limitation of the software.
  blocker="$(_voice_preset_blocker lightweight)"
  if [ -z "$blocker" ]; then
    custom_ok=1
    n=$((n + 1))
    offered="$offered$n custom
"
    wizard_item "$n" "choose the listening model yourself"
  else
    wizard_item "-" "choose the listening model yourself - $(_voice_blocker_sentence lightweight "$blocker")"
  fi

  if [ "$n" -le 1 ]; then
    # Only text-only survived. Nothing here is a failure and nothing here is
    # unfinished: this computer has been measured, the answer is typing, and
    # typing works.
    wizard_blank
    wizard_say "So this computer will be set up for typing, which is everything"
    wizard_say "ayeaye does apart from listening. Nothing will be downloaded."
    wizard_remember answer.voice.preset text-only
    wizard_remember answer.voice.model ""
    return "$WIZARD_STAGE_SKIP"
  fi

  # The default is resolved against the list that was just printed, and not
  # against the machine a second time. Anything the list does not contain
  # cannot be the default, cannot be in the brackets, and cannot be what an
  # unanswerable question falls back to - which is the whole of what stops a
  # preset the screen has just called unavailable from being taken silently.
  default_preset="$(_voice_default_preset)"
  if ! default_n="$(_voice_offered_number "$offered" "$default_preset")"; then
    default_preset="text-only"
    default_n="1"
  fi

  wizard_blank
  tries=0
  chosen=""
  while [ "$tries" -lt 3 ]; do
    tries=$((tries + 1))
    wizard_ask "which one?" "$default_n"
    reply="$REPLY"
    chosen=""
    while IFS=' ' read -r choice preset || [ -n "$choice" ]; do
      [ -n "$choice" ] || continue
      if [ "$choice" = "$reply" ] || [ "$preset" = "$reply" ]; then
        chosen="$preset"
        break
      fi
    done <<EOF
$offered
EOF
    [ -n "$chosen" ] && break
    wizard_say "that is not one of the numbers above."
    [ "${WIZARD_INTERACTIVE:-1}" = 0 ] && break
  done
  if [ -z "$chosen" ]; then
    chosen="$default_preset"
    wizard_say "taking the one offered: $(_voice_preset_words "$chosen")"
  fi

  if [ "$chosen" = custom ]; then
    _voice_ask_custom || {
      wizard_remember answer.voice.preset text-only
      wizard_remember answer.voice.model ""
      return "$WIZARD_STAGE_SKIP"
    }
    model="$_VOICE_CUSTOM_MODEL"
    wizard_remember answer.voice.preset custom
  else
    model="$(_voice_preset_model "$chosen")"
    wizard_remember answer.voice.preset "$chosen"
    # An override belongs to the answer that was overridden. Left behind from
    # a previous run it would say something untrue about this one.
    wizard_remember answer.voice.override 0
  fi

  if [ -z "$model" ]; then
    wizard_blank
    wizard_say "Nothing will be downloaded, and everything else about ayeaye"
    wizard_say "works exactly the same."
    wizard_remember answer.voice.model ""
    return "$WIZARD_STAGE_SKIP"
  fi

  # A model the catalogue cannot price is a model nothing may be planned for.
  if ! _voice_show_cost "$model"; then
    wizard_say "setup does not know that listening model, so nothing has been"
    wizard_say "chosen and nothing will be downloaded."
    wizard_remember answer.voice.preset text-only
    wizard_remember answer.voice.model ""
    return "$WIZARD_STAGE_SKIP"
  fi

  backend="$(_voice_backend)"
  wizard_remember answer.voice.model "$model"
  wizard_remember answer.voice.backend "$backend"

  _voice_plan "$model"
  _voice_ask_cleanup
  return "$WIZARD_STAGE_OK"
}

# _voice_ask_custom - the override. Sets $_VOICE_CUSTOM_MODEL; status 1 when
# the answer was to change your mind.
#
# This is the one place a model is named to a person, because naming one is
# what was asked for. A model above what this computer was measured at is
# warned about and then allowed: an explicit override is not a silent one, and
# somebody who has read the warning and typed the number anyway has made a
# decision this file has no business overruling.
_VOICE_CUSTOM_MODEL=""
_voice_ask_custom() {
  local model n=0 offered="" reply choice need
  _VOICE_CUSTOM_MODEL=""

  wizard_blank
  wizard_say "The bigger the listener, the better it hears and the longer it"
  wizard_say "takes. These are the ones ayeaye knows:"
  while IFS= read -r model || [ -n "$model" ]; do
    [ -n "$model" ] || continue
    n=$((n + 1))
    offered="$offered$n $model
"
    wizard_item "$n" "$(_voice_model_field "$model" words) (about $(_voice_mb "$(_voice_model_field "$model" bytes)") MB)"
  done <<EOF
$(voice_models)
EOF
  wizard_blank
  wizard_ask "which one?" "1"
  reply="$REPLY"
  while IFS=' ' read -r choice model || [ -n "$choice" ]; do
    [ -n "$choice" ] || continue
    if [ "$choice" = "$reply" ] || [ "$model" = "$reply" ]; then
      _VOICE_CUSTOM_MODEL="$model"
      break
    fi
  done <<EOF
$offered
EOF
  if [ -z "$_VOICE_CUSTOM_MODEL" ]; then
    wizard_say "that is not one of the numbers above, so nothing has been chosen."
    return 1
  fi

  # Above what this machine was measured at? Every model in the catalogue
  # names the verdict it is within, so this is one question put to the
  # hardware step rather than an opinion formed here out of file sizes.
  need="$(_voice_model_field "$_VOICE_CUSTOM_MODEL" tier)"
  wizard_remember answer.voice.override 0
  if ! hw_tier_at_least "$need"; then
    wizard_blank
    wizard_say "That is larger than this computer was measured for."
    if [ -n "$(hw_tier_reason)" ]; then
      wizard_say "$(hw_tier_reason)"
    fi
    wizard_say "It will still be downloaded and it will still work; it will be"
    wizard_say "slower than the ones above it in that list, and on a computer"
    wizard_say "this size it may run out of memory and stop."
    if ! wizard_confirm "use it anyway?" "n"; then
      wizard_say "nothing has been chosen."
      return 1
    fi
    wizard_remember answer.voice.override 1
  fi
  return 0
}

# _voice_plan <model> - everything this file will do, before stage five, so
# stage five can say so before stage six does any of it.
#
# The byte count is what feeds the estimate at the plan gate, and that is how
# "show the sizes before downloading" is met for somebody who never reads a
# summary twice.
_voice_plan() {
  local model="${1:-}" bytes pkg missing=""
  bytes="$(_voice_model_field "$model" bytes)" || return 0

  _voice_have ffmpeg || missing="ffmpeg"
  if [ -z "$(_voice_whisper_here)" ]; then
    pkg="$(_voice_whisper_package)"
    [ -n "$pkg" ] && missing="${missing:+$missing }$pkg"
  fi
  [ -n "$missing" ] && wizard_plan_add package "what listening needs: $missing"

  wizard_plan_add download \
    "a listening model, about $(_voice_mb "$bytes") MB, from huggingface.co" "$bytes"
  wizard_plan_add config \
    "where the listening model is, added to $(_voice_env_file)"
  return 0
}

wizard_step configure voice _voice_choose_step "Choosing how you talk to ayeaye" optional

# ================================================== 7. doing it, in stage six
#
# The order is programs first and weights second, and that is not arbitrary:
# half a gigabyte fetched onto a computer that turns out to have no way to
# record or decode audio is half a gigabyte wasted, and "it downloaded and
# then told me it could not work" is exactly the afternoon this ticket exists
# to prevent.

# _voice_sha256 <path> -> the checksum, or nothing.
#
#   0  here it is
#   1  there is a program here to do it and it did not work
#   2  this computer has nothing that can take a checksum
#
# Two spellings, because the two userlands this project supports disagree:
# coreutils ships sha256sum and macOS ships shasum. Those two failures are
# kept apart deliberately. A machine with no hasher can only have its
# downloads checked for length, which is worth saying out loud; a machine
# whose hasher ran and failed is a machine where something is wrong, and
# quietly downgrading it to a length check would both weaken the check and put
# a false sentence in the log about why.
_voice_sha256() {
  local path="${1:-}" out=""
  [ -f "$path" ] || return 1
  if command -v sha256sum >/dev/null 2>&1; then
    out="$(sha256sum "$path" 2>/dev/null)" || return 1
  elif command -v shasum >/dev/null 2>&1; then
    out="$(shasum -a 256 "$path" 2>/dev/null)" || return 1
  else
    return 2
  fi
  out="${out%% *}"
  [ -n "$out" ] || return 1
  printf '%s' "$out"
}

# _voice_file_size <path> -> bytes, or nothing and status 1.
#
# wc rather than stat: BSD and GNU stat take different flags for this and the
# byte count is the same on both.
_voice_file_size() {
  local out
  [ -f "${1:-}" ] || return 1
  out="$(wc -c < "$1" 2>/dev/null)" || return 1
  out="${out##* }"
  case "$out" in
    ""|*[!0-9]*) return 1 ;;
  esac
  printf '%s' "$out"
}

# _voice_verify <path> <model> - is the file on disk the artifact we wanted?
#
# Sets $_VOICE_VERIFIED to `checksum` when the published sha256 matched,
# `size` when there was no way to take a checksum here and the byte count
# matched, and `no` otherwise. Status 0 for the first two.
#
# The size check is not a substitute for the checksum and is not described as
# one. It is what catches the case this actually has to catch: a fetch that
# stopped halfway, leaving a file that is the right name and the wrong length.
_VOICE_VERIFIED=""
_voice_verify() {
  local path="${1:-}" model="${2:-}" want_sum want_size got_sum got_size status
  _VOICE_VERIFIED=no
  [ -f "$path" ] || return 1
  want_sum="$(_voice_model_field "$model" sha256)" || return 1
  want_size="$(_voice_model_field "$model" bytes)"

  # Assigned through an `if`, never bare: this runs where errexit is not in
  # force and the status is the answer, not an accident.
  if got_sum="$(_voice_sha256 "$path")"; then
    status=0
  else
    status=$?
  fi
  if [ "$status" = 0 ]; then
    if [ "$got_sum" = "$want_sum" ]; then
      _VOICE_VERIFIED=checksum
      return 0
    fi
    wizard_detail "voice: $path has sha256 $got_sum, expected $want_sum"
    return 1
  fi
  if [ "$status" = 1 ]; then
    # A hasher that is here and did not work is a fact about this machine, not
    # a licence to check something weaker instead.
    wizard_detail "voice: taking a checksum of $path did not work"
    return 1
  fi

  wizard_detail "voice: no sha256sum or shasum here, checking the size only"
  if got_size="$(_voice_file_size "$path")" && [ "$got_size" = "$want_size" ]; then
    _VOICE_VERIFIED=size
    return 0
  fi
  wizard_detail "voice: $path is $got_size bytes, expected $want_size"
  return 1
}

# _voice_programs - ffmpeg, whisper and whatever it takes to fetch.
#
#   0  everything listening needs is now on this computer
#   1  it is not, and that is something that went wrong
#   3  the user said no. Nothing happened, and nothing is wrong.
#
# The three are kept apart all the way up to the step's return value, because
# they are three different things to say to somebody and two different things
# for the closing checklist to do. One question covering every package rather
# than one each: they are one decision to the person answering, and three
# password prompts for one decision is how a wizard teaches somebody to stop
# reading them.
_voice_programs() {
  local want="" pkg status
  _voice_have ffmpeg || want="ffmpeg"
  _voice_have curl   || want="${want:+$want }curl"
  if [ -z "$(_voice_whisper_here)" ]; then
    pkg="$(_voice_whisper_package)"
    [ -n "$pkg" ] && want="${want:+$want }$pkg"
  fi

  if [ -n "$want" ]; then
    wizard_detail "voice: installing $want"
    # shellcheck disable=SC2086
    wizard_install_packages $want
    status=$?
    case "$status" in
      0) ;;
      "$WIZARD_REFUSED")
        wizard_say "nothing has been installed, so this computer cannot listen yet."
        return "$WIZARD_REFUSED"
        ;;
      *)
        wizard_say "could not install what listening needs on this computer."
        return 1
        ;;
    esac
  fi

  if ! _voice_have ffmpeg; then
    wizard_item "not yet" "recording what you say (ffmpeg)"
    wizard_say "ayeaye needs ffmpeg to hear you, and it is still not here."
    return 1
  fi
  wizard_item ok "recording what you say"

  if [ -z "$(_voice_whisper_here)" ] && ! _voice_have whisper-server \
     && ! _voice_have whisper-cli && ! _voice_have whisper-cpp; then
    wizard_item "not yet" "turning what you said into words"
    wizard_say "there is still no program here that turns speech into words."
    wizard_say "whisper.cpp is at $_VOICE_WHISPER_HOME; install it and run this"
    wizard_say "again, and the model will be waiting."
    return 1
  fi
  wizard_item ok "turning what you said into words"
  return 0
}

# _voice_fetch <model> - the weights. Status 0 fetched or already here, 3 the
# download was refused, 1 anything else.
_voice_fetch() {
  local model="${1:-}" path url bytes status
  path="$(_voice_model_path "$model")"
  url="$(_voice_model_url "$model")"
  bytes="$(_voice_model_field "$model" bytes)"

  # Already here and correct? Then there is nothing to fetch, and saying so is
  # how a run that was interrupted picks up where it stopped: the second run
  # asks nothing, downloads nothing, and carries straight on to the settings.
  if _voice_verify "$path" "$model"; then
    wizard_item ok "the listening model is already on this computer"
    wizard_detail "voice: $path verified by $_VOICE_VERIFIED, not fetched again"
    return 0
  fi

  # Something is there and it is not what was wanted. Which of two things it
  # is decides what may happen to it, and setup is not entitled to guess:
  #
  #   a fetch of its own that stopped halfway. Setup wrote it, setup knows it
  #   is half a file, and a half file at the right name is exactly what makes
  #   the next run believe it has a model. Removed.
  #
  #   somebody else's file. A re-quantised model, a symlink to a shared copy,
  #   a model for a language this catalogue has never heard of. Not setup's to
  #   delete, and deleting it before asking for permission to download the
  #   replacement would leave a refusal worse off than never having run.
  if [ -e "$path" ]; then
    if [ "$(wizard_state_get step.install.voice.fetching "")" = "$path" ]; then
      wizard_say "there is a listening model here already, and it is not complete."
      wizard_say "it will be fetched again."
      wizard_detail "voice: removing the incomplete $path, which setup started"
      rm -f "$path" 2>/dev/null || true
    else
      wizard_say "there is already a file where the listening model goes, and it"
      wizard_say "is not the one setup was going to fetch."
      wizard_replace "$path" "you already have a file at $path. May I replace it?"
      case "$?" in
        0) rm -f "$path" 2>/dev/null || true ;;
        2) return 1 ;;
        *)
          wizard_say "left alone, so nothing has been downloaded."
          return "$WIZARD_REFUSED"
          ;;
      esac
    fi
  fi

  # Written down before the fetch and cleared after it, so that the run which
  # picks up an interrupted one can tell its own half-file from a stranger's.
  wizard_remember step.install.voice.fetching "$path"
  wizard_download \
    "may I download the listening model? It is about $(_voice_mb "$bytes") MB." \
    "$url" "$path" "$bytes"
  status=$?
  case "$status" in
    0) ;;
    "$WIZARD_REFUSED") return "$WIZARD_REFUSED" ;;
    *) return 1 ;;
  esac

  if ! _voice_verify "$path" "$model"; then
    wizard_say "the listening model downloaded, but it is not the file it should"
    wizard_say "be, so it has been deleted rather than used."
    wizard_say "run this setup again to try once more."
    rm -f "$path" 2>/dev/null || true
    wizard_remember step.install.voice.fetching ""
    return 1
  fi
  wizard_remember step.install.voice.fetching ""
  if [ "$_VOICE_VERIFIED" = checksum ]; then
    wizard_item ok "the listening model, checked against the checksum its authors published"
  else
    wizard_item ok "the listening model"
    wizard_say "(this computer has no way to check a file's fingerprint, so only"
    wizard_say "its size was checked)"
  fi
  return 0
}

# _voice_whisper_cli - the command-line transcriber on this machine, or
# nothing when the only whisper here is the resident server.
#
# whisper.cpp renamed its binaries and both names are still in the world: its
# own builds and Homebrew's formula install `whisper-cli`, while older builds
# and some packages carry `whisper-cpp`. bin/voice-dictate used to name one of
# them and nothing else, so a computer with the other one had a transcriber
# that setup had found and the app could not run.
_voice_whisper_cli() {
  local name
  for name in whisper-cli whisper-cpp whisper; do
    if _voice_have "$name"; then
      command -v "$name" 2>/dev/null
      return 0
    fi
  done
  return 1
}

# _voice_settings <model> - tell the app where the model is.
#
# This is the whole of the reconciliation, and it is short because the
# disagreement was never complicated: the app looks for VOICE_WHISPER_MODEL,
# nothing in setup had ever written it, and so lib/steps/70-service.sh
# declined to install the transcription service on every machine there is
# while the app's fallback route looked at a default path nobody had put a
# file at.
#
# Merged rather than rewritten: the settings file is the user's, and
# everything in it this run did not ask about comes through byte for byte.
_voice_settings() {
  local model="${1:-}" env_file path cli
  env_file="$(_voice_env_file)"
  path="$(_voice_model_path "$model")"
  if [ ! -f "$env_file" ]; then
    wizard_detail "voice: no settings file at $env_file to write the model into"
    return 1
  fi
  cli="$(_voice_whisper_cli)" || cli=""
  if wizard_env_merge "$env_file" \
      "VOICE_WHISPER_MODEL=$path" \
      "VOICE_WHISPER_THREADS=$(_voice_threads)" \
      ${cli:+"VOICE_WHISPER_CLI=$cli"}; then
    wizard_remember step.install.voice.model_path "$path"
    wizard_detail "voice: VOICE_WHISPER_MODEL=$path in $env_file"
    [ -n "$cli" ] && wizard_detail "voice: VOICE_WHISPER_CLI=$cli in $env_file"
    return 0
  fi
  wizard_say "could not write where the listening model is into your settings."
  return 1
}

# _voice_talk_button_works - status 0 when the app will really turn talking
# on after this run, 1 when it will not, whatever else went right.
#
# This is the handshake, checked rather than assumed, and it is checked
# against what bin/ayeaye actually asks for rather than against what this file
# has just done. voice_available() wants ffmpeg, and then either a whisper
# server answering on VOICE_WHISPER_SERVER - which the service stage installs
# once the settings name a model, which is what this step has just written -
# or the command spelled exactly `whisper-cpp` on PATH.
#
# That second spelling is the one this cannot satisfy from here. whisper.cpp
# renamed its binaries and the current builds, Homebrew's formula and Arch's
# package all install `whisper-cli`; a computer with that name and no server
# transcribes perfectly through bin/voice-dictate and is still refused by the
# probe. Saying so is the only honest thing available: the file that holds the
# probe belongs to another ticket, and reporting the work finished when the
# talk button will be grey is the one thing a step must never do.
_voice_talk_button_works() {
  _voice_have ffmpeg || return 1
  _voice_have whisper-cpp && return 0
  # A server the service stage can install and start counts: the probe's first
  # route is a socket, and it does not care what the program is called.
  _voice_have whisper-server && return 0
  return 1
}

_voice_install_step() {
  local model status

  model="$(wizard_state_get answer.voice.model "")"
  if [ -z "$model" ]; then
    # Nothing was chosen, which is finished business rather than work left
    # undone. The closing checklist has nothing to say about a decision.
    return "$WIZARD_STAGE_SKIP"
  fi

  hw_detect || wizard_detail "voice: reading this computer again did not work"
  wizard_blank

  _voice_programs
  status=$?
  if [ "$status" = "$WIZARD_REFUSED" ]; then
    # A refusal is a finished conversation. Nothing was downloaded and nothing
    # is outstanding: the answer was no.
    wizard_say "so nothing has been downloaded. Everything else about ayeaye"
    wizard_say "works, and you can type to your agents in the meantime."
    return "$WIZARD_STAGE_SKIP"
  fi
  if [ "$status" != 0 ]; then
    wizard_say "so nothing has been downloaded. Everything else about ayeaye"
    wizard_say "works, and you can type to your agents in the meantime."
    return "$WIZARD_STAGE_PENDING"
  fi

  _voice_fetch "$model"
  status=$?
  if [ "$status" = "$WIZARD_REFUSED" ]; then
    wizard_say "nothing was downloaded. ayeaye will run text-only until it is."
    return "$WIZARD_STAGE_SKIP"
  fi
  if [ "$status" != 0 ]; then
    return "$WIZARD_STAGE_PENDING"
  fi

  if ! _voice_settings "$model"; then
    return "$WIZARD_STAGE_PENDING"
  fi

  _voice_pull_cleanup
  status=$?
  [ "$status" = 1 ] && return "$WIZARD_STAGE_PENDING"

  if ! _voice_talk_button_works; then
    wizard_blank
    wizard_say "The listening model is here and your settings point at it, but"
    wizard_say "ayeaye will still show the talk button greyed out for now: it"
    wizard_say "looks for the listening program under one particular name, and"
    wizard_say "this computer has it under another."
    wizard_say "The fix is to let setup start the listening program in the"
    wizard_say "background - answer yes to that in a moment - or run this again"
    wizard_say "once whisper.cpp's server is installed."
    wizard_detail "voice: bin/ayeaye's probe wants whisper-cpp on PATH or a whisper server; this machine has $(_voice_whisper_cli || printf none)"
    return "$WIZARD_STAGE_PENDING"
  fi
  return "$WIZARD_STAGE_OK"
}

# _voice_pull_cleanup - the instruct model, through ollama, if one was chosen.
#
# Status 0 when there was nothing to do, it was done, or it was refused - all
# three are finished business. 1 when it was asked for and did not arrive,
# which is the one case the closing checklist should mention.
#
# Fetching bytes from the internet is consented to as a download like anything
# else, even though it is a program doing the fetching rather than curl: the
# rule is about what lands on the machine, not about which process put it
# there. There is no wrapper shaped like this, so the primitive the wrappers
# are built on is used directly and the command goes in the ledger with it.
_voice_pull_cleanup() {
  local model bytes out status
  [ "$(wizard_state_get answer.voice.cleanup 0)" = 1 ] || return 0
  model="$(wizard_state_get answer.voice.cleanup_model "")"
  [ -n "$model" ] || return 0

  if ! _voice_have ollama; then
    wizard_say "ollama is no longer on this computer, so what you say will be"
    wizard_say "typed exactly as it was heard. Nothing else is affected."
    return 0
  fi

  bytes="$(_voice_cleanup_bytes "$model")"
  wizard_consent download \
    "may I download the tidying-up model? It is about $(_voice_mb "$bytes") MB." \
    "ollama pull $model"
  status=$?
  if [ "$status" = 2 ]; then
    # A bug here, not an answer. Reporting it as "you said no" would hide it
    # behind a sentence that reads perfectly well.
    wizard_detail "voice: wizard_consent rejected the cleanup question as malformed"
    return 1
  fi
  if [ "$status" != 0 ]; then
    wizard_say "nothing was downloaded. What you say will be typed exactly as it"
    wizard_say "was heard, which is what ayeaye does without it anyway."
    return 0
  fi

  # Standard input closed. ollama pull draws a progress bar and this step runs
  # with the real standard input on fd 8; a program that reads a line here
  # would eat the answer to the question after it.
  wizard_detail "running: ollama pull $model"
  if out="$(ollama pull "$model" 2>&1 </dev/null)"; then
    wizard_detail "$out"
  else
    wizard_detail "$out"
    wizard_say "the tidying-up model would not download. What you say will be"
    wizard_say "typed as it was heard until it does; run this setup again to try."
    return 1
  fi

  if wizard_env_merge "$(_voice_env_file)" "VOICE_OLLAMA_MODEL=$model"; then
    wizard_item ok "tidying up what you said"
    return 0
  fi
  wizard_say "could not write the tidying-up model into your settings."
  return 1
}

wizard_step install voice _voice_install_step "Setting up talking out loud" optional
