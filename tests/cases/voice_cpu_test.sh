# The processor-only path, which must never be the missing one.
#
# A slow honest option beats an absent one, so the thing pinned here is that
# the offer survives every way a machine can end up without usable graphics -
# no card at all, a card too small to hold a model, a card setup could not
# measure - and that every one of them is told what slow means in the time it
# takes rather than in a number.

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
  _HW_DETECTED=0
}

# _machine <tier> <acceleration> <vram> - a published verdict, with a whisper
# on the machine so that the graphics are the only thing under test.
_machine() {
  wizard_remember step.detect.hardware.tier "$1"
  wizard_remember step.detect.hardware.acceleration "$2"
  wizard_remember step.detect.hardware.vram_mb "$3"
  wizard_remember step.detect.hardware.network online
  wizard_remember step.detect.tools.whisper_command whisper-cli
}

# ================================= every way to end up without graphics

test_a_computer_with_no_card_at_all_still_gets_the_offer() {
  _machine recommended cpu unknown
  assert_eq "cpu" "$(_voice_backend)"
  assert_eq "" "$(_voice_preset_blocker lightweight)"
  assert_eq "" "$(_voice_preset_blocker recommended)"
}

test_a_card_too_small_to_hold_a_model_still_gets_the_offer() {
  _machine recommended cuda 2048
  assert_eq "cpu" "$(_voice_backend)" "the promise is not made for it"
  assert_eq "" "$(_voice_preset_blocker recommended)" \
    "and the offer is still there, on the processor"
}

test_a_two_gigabyte_card_is_offered_the_smallest_model_on_the_processor() {
  # Written down because it was raised as a doubt at the close of the
  # milestone: does the tier ladder leave a two gigabyte graphics board with no
  # listening option at all, not even the 78 MB one?
  #
  # It does not, and this is the proof rather than a reading of the code. A
  # card below the useful line caps the tier at "recommended" and never below
  # it - a card cannot take away memory or disk the machine has - so both the
  # smallest experience and the balanced one stay on offer, on the processor,
  # with the speed warning the tests below pin. Only memory, free space or
  # processor count can put a machine on text-only.
  _machine recommended cuda 2048
  assert_eq "" "$(_voice_preset_blocker lightweight)" \
    "the 78 MB model is on offer"
  assert_eq "tiny.en" "$(_voice_preset_model lightweight)" \
    "and lightweight is what that model is called here"
  assert_eq "recommended" \
    "$(hw_tier_for "$(_big_ram)" 8 "$(_big_disk)" cuda 2048)" \
    "a small card caps the tier and never lowers it to text-only"
}

test_only_the_machine_itself_can_take_listening_away() {
  # The other half of the same claim: text-only is reached by not having the
  # memory, the space or the processors, and by nothing a graphics card does or
  # does not do.
  assert_eq "text-only" "$(hw_tier_for 1024 8 "$(_big_disk)" cuda 24576)" \
    "not enough memory, however big the card"
  assert_eq "text-only" "$(hw_tier_for "$(_big_ram)" 8 512 cuda 24576)" \
    "not enough room on the disk, however big the card"
  assert_eq "maximum" "$(hw_tier_for "$(_big_ram)" 8 "$(_big_disk)" cuda 24576)" \
    "and a machine with all three reaches the top"
}

# _big_ram / _big_disk - comfortably above every line in the ladder, so that
# the one dimension under test is the only one that can be binding.
_big_ram()  { printf '65536'; }
_big_disk() { printf '102400'; }

test_a_card_setup_could_not_measure_still_gets_the_offer() {
  _machine recommended rocm unknown
  assert_eq "cpu" "$(_voice_backend)"
  assert_eq "" "$(_voice_preset_blocker lightweight)"
}

test_the_smallest_experience_survives_a_machine_with_nothing_going_for_it() {
  # Lightweight is the floor of listening, and it is reachable on a machine
  # with no card, no measurement and only just enough of itself.
  _machine lightweight cpu unknown
  assert_eq "" "$(_voice_preset_blocker lightweight)"
  assert_eq "cpu" "$(_voice_backend)"
}

# ================================== and every one of them is told the cost

test_every_model_on_a_processor_says_what_slow_means_in_time() {
  local model words
  for model in $(voice_models); do
    words="$(_voice_speed_words "$model" cpu)"
    assert_contains "$words" "on this processor" "$model warns about the processor"
    assert_matches "$words" "seconds|as long|minutes" \
      "$model says what slow means in time, not in a benchmark number"
  done
}

test_the_warning_gets_worse_as_the_model_gets_bigger() {
  assert_contains "$(_voice_speed_words tiny.en cpu)" "a few seconds"
  assert_contains "$(_voice_speed_words small.en cpu)" "about as long again"
  assert_contains "$(_voice_speed_words large-v3-turbo cpu)" "several times as long"
}

test_a_machine_with_usable_graphics_is_not_warned_about_a_processor() {
  local words
  words="$(_voice_speed_words large-v3-turbo cuda)"
  assert_not_contains "$words" "processor"
  assert_contains "$words" "a moment after you stop talking"
}

# ================================================ threads, from the measurement

test_the_processor_path_uses_the_cores_that_were_counted() {
  local cores
  for cores in 2 4 8; do
    wizard_remember step.detect.hardware.cores "$cores"
    assert_eq "$cores" "$(_voice_threads)" "$cores cores means $cores threads"
  done
}

test_a_very_large_machine_does_not_give_all_of_itself_to_listening() {
  wizard_remember step.detect.hardware.cores 128
  assert_eq "8" "$(_voice_threads)" \
    "a machine that stutters at everything else while transcribing is not faster"
}
