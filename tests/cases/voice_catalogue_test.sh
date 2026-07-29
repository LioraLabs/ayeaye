# The catalogue: what there is to fetch, which experience means which of it,
# and which experiences this computer is allowed to be offered.
#
# None of it runs a probe. Every gate here is a question put to the hardware
# step's published verdict, so the answer can be reasoned about by writing the
# verdict into the state file and asking - which is exactly what a resumed run
# does, and is the only way to cover four machine profiles without four
# machines.

setup() {
  WIZARD_STATE_DIR="$TEST_TMPDIR/state"
  WIZARD_STATE_FILE="$WIZARD_STATE_DIR/setup-state"
  WIZARD_LOG_FILE="$WIZARD_STATE_DIR/setup.log"
  . "$REPO_ROOT/lib/wizard.sh"
  wizard_stage detect    "checking this computer"
  wizard_stage report    "what ayeaye can do here"
  wizard_stage configure "configuration"
  wizard_stage install   "setting it up"
  . "$REPO_ROOT/lib/steps/20-hardware.sh"
  . "$REPO_ROOT/lib/steps/40-voice.sh"
  # The verdict is read from the state file rather than probed, which is what
  # _hw_ready does whenever this shell has not detected anything itself.
  _HW_DETECTED=0
}

_stub_uname() {
  stub_command uname --exit 1
  stub_when uname '-s' --stdout "$1"
  stub_when uname '-m' --stdout "$2"
}

# _on <os-release fixture> <package manager> - a Linux family with its manager.
_on() {
  _stub_uname Linux x86_64
  stub_command id --stdout "1000"
  stub_command sudo
  stub_command "$2"
  assert_fixture_exists "os-release/$1"
  export PLATFORM_OS_RELEASE_FILES="$(fixture_file "os-release/$1")"
  platform_reset
}

_on_macos_with_brew() {
  _stub_uname Darwin arm64
  stub_command_from_fixture sw_vers sw_vers/macos-15.1
  stub_command id --stdout "501"
  stub_command sudo
  stub_command brew
  export PLATFORM_OS_RELEASE_FILES=""
  platform_reset
}

# _machine <tier> <acceleration> <vram> [network] - a published verdict.
_machine() {
  wizard_remember step.detect.hardware.tier "$1"
  wizard_remember step.detect.hardware.acceleration "$2"
  wizard_remember step.detect.hardware.vram_mb "$3"
  wizard_remember step.detect.hardware.network "${4:-online}"
  wizard_remember step.detect.tools.whisper_command "whisper-server"
}

# ================================================== the table is complete

test_every_model_carries_a_size_a_checksum_and_a_memory_figure() {
  local model bytes sum ram words
  for model in $(voice_models); do
    bytes="$(_voice_model_field "$model" bytes)"
    sum="$(_voice_model_field "$model" sha256)"
    ram="$(_voice_model_field "$model" ram_mb)"
    words="$(_voice_model_field "$model" words)"
    assert_matches "$bytes" '^[0-9]+$' "$model has a size in bytes"
    assert_matches "$sum" '^[0-9a-f]{64}$' "$model has a sha256 upstream published"
    assert_matches "$ram" '^[0-9]+$' "$model says how much room it needs"
    assert_ne "" "$words" "$model can be described without naming a model"
  done
}

test_a_model_needs_more_room_to_run_in_than_it_takes_on_disk() {
  # The figure the summary shows is working memory, not file size. whisper.cpp
  # holds the weights and works beside them, so a table that quietly reported
  # the file size would understate every model - and the check has to be
  # strictly greater than the file, not greater than half of it, or it would
  # pass for exactly the mistake it is here to catch.
  local model bytes ram
  for model in $(voice_models); do
    bytes="$(_voice_model_field "$model" bytes)"
    ram="$(_voice_model_field "$model" ram_mb)"
    assert_eq 1 "$([ "$((ram * 1000000))" -gt "$bytes" ] && echo 1 || echo 0)" \
      "$model needs more room to run in ($ram MB) than it takes on disk"
  done
}

test_every_model_names_the_verdict_it_is_within() {
  # "Is this bigger than the machine was measured for" is a question about
  # tiers. Answering it by comparing file sizes would be this file forming an
  # opinion about hardware, which is the one thing it may not do.
  local model tier
  for model in $(voice_models); do
    tier="$(_voice_model_field "$model" tier)"
    assert_matches "$tier" '^(lightweight|recommended|maximum)$' \
      "$model names a tier the hardware step publishes"
  done
  assert_eq "lightweight" "$(_voice_model_field tiny.en tier)"
  assert_eq "recommended" "$(_voice_model_field small.en tier)"
  assert_eq "maximum" "$(_voice_model_field large-v3-turbo tier)"
}

test_a_preset_asks_for_a_model_its_own_tier_can_carry() {
  # The two tables have to agree, and nothing but this notices when they stop:
  # a preset offered at "recommended" whose model wanted "maximum" would be
  # offered to a machine and then warned about on the machine it was offered to.
  local preset model
  for preset in lightweight recommended maximum; do
    model="$(_voice_preset_model "$preset")"
    assert_eq "$preset" "$(_voice_model_field "$model" tier)" \
      "$preset offers $model, which is a $preset model"
  done
}

test_an_unknown_model_is_not_invented() {
  local status
  _voice_model_field "small.fr" bytes >/dev/null
  status=$?
  assert_status 1 "$status" "a model the table has never heard of is refused"
  _voice_model_url "small.fr" >/dev/null
  status=$?
  assert_status 1 "$status" "and has no address"
}

test_the_download_address_is_the_published_one() {
  assert_eq \
    "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.en.bin" \
    "$(_voice_model_url small.en)"
}

test_the_model_lands_where_the_hardware_step_measured_the_free_space() {
  HW_MODEL_DIR="$TEST_TMPDIR/models"
  assert_eq "$TEST_TMPDIR/models/ggml-tiny.en.bin" "$(_voice_model_path tiny.en)"
}

# ==================================================== presets name models

test_every_preset_but_text_only_names_a_model_the_table_has() {
  local preset model
  for preset in $(voice_presets); do
    model="$(_voice_preset_model "$preset")"
    if [ "$preset" = "text-only" ]; then
      assert_eq "" "$model" "text only downloads nothing"
      continue
    fi
    assert_ne "" "$model" "$preset names a model"
    assert_ne "" "$(_voice_model_field "$model" bytes)" \
      "$preset names a model the table knows: $model"
  done
}

test_every_preset_can_be_described_without_naming_a_model() {
  local preset words
  for preset in $(voice_presets); do
    words="$(_voice_preset_words "$preset")"
    assert_ne "" "$words" "$preset has an offer line"
    assert_not_contains "$words" "ggml" "the offer is an experience, not a file"
    assert_not_contains "$words" "whisper" "nor a library name"
  done
}

test_the_presets_get_larger_in_the_order_they_are_offered() {
  local preset last=0 bytes model
  for preset in $(voice_presets); do
    model="$(_voice_preset_model "$preset")"
    [ -n "$model" ] || continue
    bytes="$(_voice_model_field "$model" bytes)"
    assert_eq 1 "$([ "$bytes" -gt "$last" ] && echo 1 || echo 0)" \
      "$preset is bigger than the one before it"
    last="$bytes"
  done
}

# ============================================ which presets a machine gets

test_the_top_machine_is_offered_everything() {
  _machine maximum cuda 24576
  local preset
  for preset in $(voice_presets); do
    assert_eq "" "$(_voice_preset_blocker "$preset")" "$preset is on offer"
  done
}

test_a_middling_machine_is_offered_everything_but_the_largest() {
  _machine recommended cuda 8192
  assert_eq "" "$(_voice_preset_blocker text-only)"
  assert_eq "" "$(_voice_preset_blocker lightweight)"
  assert_eq "" "$(_voice_preset_blocker recommended)"
  assert_eq "tier" "$(_voice_preset_blocker maximum)"
}

test_a_small_machine_is_offered_the_small_listener_and_nothing_above_it() {
  _machine lightweight cpu unknown
  assert_eq "" "$(_voice_preset_blocker text-only)"
  assert_eq "" "$(_voice_preset_blocker lightweight)"
  assert_eq "tier" "$(_voice_preset_blocker recommended)"
  assert_eq "tier" "$(_voice_preset_blocker maximum)"
}

test_a_machine_measured_as_text_only_is_offered_only_text() {
  _machine text-only cpu unknown
  assert_eq "" "$(_voice_preset_blocker text-only)"
  assert_eq "tier" "$(_voice_preset_blocker lightweight)"
  assert_eq "tier" "$(_voice_preset_blocker recommended)"
  assert_eq "tier" "$(_voice_preset_blocker maximum)"
}

test_text_only_is_never_blocked_even_with_nothing_installed_and_no_network() {
  # The floor. Whatever is wrong with this computer, typing still works, and
  # an option that could be taken away is not a floor.
  _machine text-only cpu unknown offline
  wizard_remember step.detect.tools.whisper_command ""
  assert_eq "" "$(_voice_preset_blocker text-only)"
}

test_no_program_to_run_a_model_blocks_the_listening_presets() {
  _machine maximum cuda 24576
  wizard_remember step.detect.tools.whisper_command ""
  _on debian-12 apt-get
  assert_eq "whisper" "$(_voice_preset_blocker lightweight)" \
    "a model with nothing to run it is half a gigabyte of nothing"
  assert_eq "whisper" "$(_voice_preset_blocker maximum)"
  assert_eq "" "$(_voice_preset_blocker text-only)"
}

test_an_offline_computer_is_not_offered_a_download() {
  _machine maximum cuda 24576 offline
  assert_eq "network" "$(_voice_preset_blocker lightweight)"
  assert_eq "" "$(_voice_preset_blocker text-only)"
}

test_a_preset_that_does_not_exist_is_a_caller_bug() {
  _machine maximum cuda 24576
  local status
  _voice_preset_blocker "enormous" >/dev/null
  status=$?
  assert_status 2 "$status"
}

# ============================================= every block carries a reason

test_every_blocked_preset_has_a_sentence_saying_why() {
  local preset word sentence
  _machine lightweight cpu unknown
  wizard_remember step.detect.hardware.tier_reason \
    "this computer has 4 GB of memory, and the larger listeners need 8 GB"
  for preset in recommended maximum; do
    word="$(_voice_preset_blocker "$preset")"
    assert_ne "" "$word" "$preset is blocked here"
    sentence="$(_voice_blocker_sentence "$preset" "$word")"
    assert_contains "$sentence" "not offered here"
    assert_contains "$sentence" "4 GB of memory" \
      "the explanation is the hardware step's own words, not a second opinion"
  done
}

test_a_block_with_no_recorded_reason_still_explains_itself() {
  _machine lightweight cpu unknown
  wizard_remember step.detect.hardware.tier_reason ""
  assert_contains "$(_voice_blocker_sentence recommended tier)" \
    "measured smaller than it needs"
}

test_the_missing_program_explanation_says_where_to_go_next() {
  # A blocked option with no destination in the sentence is a dead end.
  _on debian-12 apt-get
  local sentence
  sentence="$(_voice_blocker_sentence lightweight whisper)"
  assert_contains "$sentence" "no package for one"
  assert_contains "$sentence" "https://github.com/ggerganov/whisper.cpp" \
    "and somewhere to go about it"
}

test_a_mac_is_not_told_there_is_no_package_when_there_is_one() {
  # On a Mac the program exists and Homebrew is what is missing. Telling
  # somebody there is no package would be false, and would send them looking
  # in the wrong place.
  _stub_uname Darwin arm64
  stub_command_from_fixture sw_vers sw_vers/macos-15.1
  stub_command id --stdout "501"
  export PLATFORM_OS_RELEASE_FILES=""
  export PLATFORM_BREW_PREFIXES=""
  platform_reset
  assert_eq "macos" "$(platform_family)"
  local sentence
  sentence="$(_voice_blocker_sentence lightweight whisper)"
  assert_contains "$sentence" "Homebrew, which is not on this Mac"
  assert_contains "$sentence" "https://brew.sh"
  assert_not_contains "$sentence" "no package for one"
}

# ================================================== which backend, really

test_a_large_nvidia_card_is_used() {
  _machine maximum cuda 24576
  assert_eq "cuda" "$(_voice_backend)"
  assert_contains "$(_voice_backend_words cuda)" "NVIDIA"
}

test_a_card_too_small_to_hold_a_model_is_honestly_cuda_and_is_not_used() {
  # The distinction this whole gate exists for: hw_acceleration is a statement
  # of fact and a 2 GB card really is cuda. Promising it will do the work is
  # the lie, and the backend is what carries the promise.
  _machine lightweight cuda 2048
  assert_eq "cuda" "$(hw_acceleration)" "the fact is unchanged"
  assert_eq "cpu" "$(_voice_backend)" "and the promise is not made"
}

test_apple_silicon_uses_the_graphics_built_into_it() {
  _machine maximum metal unknown
  assert_eq "metal" "$(_voice_backend)"
  assert_contains "$(_voice_backend_words metal)" "built into this Mac"
}

test_a_large_amd_card_is_used() {
  _machine recommended rocm 16384
  assert_eq "rocm" "$(_voice_backend)"
  assert_contains "$(_voice_backend_words rocm)" "AMD"
}

test_a_machine_with_no_graphics_at_all_uses_its_processor() {
  _machine lightweight cpu unknown
  assert_eq "cpu" "$(_voice_backend)"
  assert_contains "$(_voice_backend_words cpu)" "processor"
}

# ====================================================== threads, measured

test_the_thread_count_follows_the_cores_that_were_measured() {
  wizard_remember step.detect.hardware.cores 4
  assert_eq "4" "$(_voice_threads)"
}

test_the_thread_count_never_takes_the_whole_machine() {
  wizard_remember step.detect.hardware.cores 64
  assert_eq "8" "$(_voice_threads)"
}

test_an_unreadable_core_count_gets_a_floor_rather_than_an_invention() {
  wizard_remember step.detect.hardware.cores unknown
  assert_eq "2" "$(_voice_threads)"
}

# ==================================================== packages for whisper

test_homebrew_has_a_formula_and_it_is_offered() {
  _on_macos_with_brew
  assert_eq "macos" "$(platform_family)"
  assert_eq "whisper-cpp" "$(_voice_whisper_package)"
}

test_arch_names_the_build_that_matches_the_backend() {
  _on arch pacman
  assert_eq "arch" "$(platform_family)"
  assert_eq "whisper.cpp" "$(_voice_whisper_package cpu)"
  assert_eq "whisper.cpp-cuda" "$(_voice_whisper_package cuda)"
  assert_eq "whisper.cpp-hipblas" "$(_voice_whisper_package rocm)"
}

test_a_family_with_no_package_says_so_rather_than_guessing_a_name() {
  # Guessing would be worse than nothing: an install command naming a package
  # that does not exist fails after asking for a password.
  _on debian-12 apt-get
  assert_eq "debian" "$(platform_family)"
  assert_eq "" "$(_voice_whisper_package)"
}

# ================================================= tidying up what was heard

test_only_instruct_models_are_ever_named_for_the_cleanup_step() {
  # A coder model answers the dictation instead of rewriting it: ask one to
  # tidy "check why fetch user throws" and it writes you a function.
  local tier model
  for tier in lightweight recommended maximum; do
    _machine "$tier" cuda 24576
    model="$(_voice_cleanup_model)"
    assert_contains "$model" "instruct" "$tier is offered an instruct model"
    assert_not_contains "$model" "coder" "and never a coder model"
    assert_ne "0" "$(_voice_cleanup_bytes "$model")" \
      "and its size is known before it is offered"
  done
}

test_a_smaller_machine_is_offered_a_smaller_cleanup_model() {
  _machine maximum cuda 24576
  assert_eq "qwen2.5:7b-instruct" "$(_voice_cleanup_model)"
  _machine lightweight cuda 24576
  assert_eq "qwen2.5:3b-instruct" "$(_voice_cleanup_model)"
}

test_the_cleanup_model_is_not_offered_without_the_program_that_fetches_it() {
  _machine maximum cuda 24576
  wizard_remember step.detect.tools.ollama 0
  assert_eq "ollama" "$(_voice_cleanup_blocker)"
  wizard_remember step.detect.tools.ollama 1
  assert_eq "" "$(_voice_cleanup_blocker)"
}

test_the_cleanup_model_is_not_offered_on_a_weak_processor_only_computer() {
  _machine lightweight cpu unknown
  wizard_remember step.detect.tools.ollama 1
  assert_eq "weak" "$(_voice_cleanup_blocker)"
  assert_contains "$(_voice_cleanup_sentence weak)" "slower than typing"
}

test_a_processor_only_computer_with_room_for_it_is_still_offered_it() {
  # The rule is about weak machines, not about processors. A workstation with
  # no graphics card and plenty of everything else can carry it.
  _machine recommended cpu unknown
  wizard_remember step.detect.tools.ollama 1
  assert_eq "" "$(_voice_cleanup_blocker)"
}
