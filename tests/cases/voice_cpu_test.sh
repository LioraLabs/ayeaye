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
