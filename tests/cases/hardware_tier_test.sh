# The cut-offs, and the arithmetic that turns them into a recommendation.
#
# This is the file the whole ticket exists to make possible: a machine either
# is or is not over a line, the lines are named constants in one place, and
# every one of them is pinned here one megabyte below and exactly at the line.
# A number that moves without this file moving with it is a number that moved
# by accident.
#
# It never runs a probe. hw_tier_for is pure - five values in, one word out -
# so the recommendation can be reasoned about without a machine to reason on.

setup() {
  # Nothing here reads the machine, but the file under test is a step file and
  # registers itself, so the two stages it attaches to have to exist first.
  WIZARD_STATE_DIR="$TEST_TMPDIR/state"
  WIZARD_STATE_FILE="$WIZARD_STATE_DIR/setup-state"
  WIZARD_LOG_FILE="$WIZARD_STATE_DIR/setup.log"
  . "$REPO_ROOT/lib/wizard.sh"
  wizard_stage detect "checking this computer"
  wizard_stage report "what ayeaye can do here"
  . "$REPO_ROOT/lib/steps/20-hardware.sh"
}

# A machine that is over every line except the one under test, so that a change
# of verdict can only have come from the value being varied.
_tier() { hw_tier_for "$1" "$2" "$3" "$4" "$5" "${6:-none}"; }

_big_ram()   { printf '%s' "$HW_RAM_MB_MAXIMUM"; }
_big_disk()  { printf '%s' "$HW_DISK_MB_MAXIMUM"; }
_big_cores() { printf '%s' "$HW_CORES_MAXIMUM"; }
_big_gpu()   { printf '%s' "$HW_VRAM_MB_MAXIMUM"; }

# ------------------------------------------------------ the ladder has an order

test_the_four_tiers_are_ranked_lowest_to_highest() {
  assert_eq 0 "$(hw_tier_rank text-only)"
  assert_eq 1 "$(hw_tier_rank lightweight)"
  assert_eq 2 "$(hw_tier_rank recommended)"
  assert_eq 3 "$(hw_tier_rank maximum)"
}

test_a_tier_name_nobody_defined_is_a_caller_bug_and_not_a_guess() {
  local out status
  out="$(hw_tier_rank enormous 2>/dev/null)"
  status=$?
  assert_status 2 "$status" "an unknown tier name is a bug in the caller"
  assert_eq "" "$out" "and it must not answer with a rank anyway"
}

test_the_lower_of_two_tiers_is_the_answer_whichever_way_round_it_is_asked() {
  assert_eq lightweight "$(hw_tier_min lightweight maximum)"
  assert_eq lightweight "$(hw_tier_min maximum lightweight)"
  assert_eq text-only "$(hw_tier_min text-only recommended)"
  assert_eq recommended "$(hw_tier_min recommended recommended)"
}

# ------------------------------------------------------------ memory boundaries

test_memory_one_megabyte_under_the_lightweight_line_is_text_only() {
  assert_eq text-only \
    "$(_tier "$((HW_RAM_MB_LIGHTWEIGHT - 1))" "$(_big_cores)" "$(_big_disk)" cpu 0)"
}

test_memory_exactly_on_the_lightweight_line_is_lightweight() {
  assert_eq lightweight \
    "$(_tier "$HW_RAM_MB_LIGHTWEIGHT" "$(_big_cores)" "$(_big_disk)" cpu 0)"
}

test_memory_one_megabyte_under_the_recommended_line_is_still_lightweight() {
  assert_eq lightweight \
    "$(_tier "$((HW_RAM_MB_RECOMMENDED - 1))" "$(_big_cores)" "$(_big_disk)" cpu 0)"
}

test_memory_exactly_on_the_recommended_line_is_recommended() {
  assert_eq recommended \
    "$(_tier "$HW_RAM_MB_RECOMMENDED" "$(_big_cores)" "$(_big_disk)" cpu 0)"
}

test_memory_one_megabyte_under_the_maximum_line_holds_at_recommended() {
  assert_eq recommended \
    "$(_tier "$((HW_RAM_MB_MAXIMUM - 1))" "$(_big_cores)" "$(_big_disk)" cuda "$(_big_gpu)")"
}

test_memory_exactly_on_the_maximum_line_reaches_maximum() {
  assert_eq maximum \
    "$(_tier "$HW_RAM_MB_MAXIMUM" "$(_big_cores)" "$(_big_disk)" cuda "$(_big_gpu)")"
}

# ------------------------------------------------------------- core boundaries

test_one_core_under_the_lightweight_line_is_text_only() {
  assert_eq text-only \
    "$(_tier "$(_big_ram)" "$((HW_CORES_LIGHTWEIGHT - 1))" "$(_big_disk)" cpu 0)"
}

test_exactly_the_lightweight_core_count_is_lightweight() {
  assert_eq lightweight \
    "$(_tier "$(_big_ram)" "$HW_CORES_LIGHTWEIGHT" "$(_big_disk)" cpu 0)"
}

test_one_core_under_the_recommended_line_holds_at_lightweight() {
  assert_eq lightweight \
    "$(_tier "$(_big_ram)" "$((HW_CORES_RECOMMENDED - 1))" "$(_big_disk)" cpu 0)"
}

test_exactly_the_recommended_core_count_is_recommended() {
  assert_eq recommended \
    "$(_tier "$(_big_ram)" "$HW_CORES_RECOMMENDED" "$(_big_disk)" cpu 0)"
}

test_one_core_under_the_maximum_line_holds_at_recommended() {
  assert_eq recommended \
    "$(_tier "$(_big_ram)" "$((HW_CORES_MAXIMUM - 1))" "$(_big_disk)" cuda "$(_big_gpu)")"
}

test_exactly_the_maximum_core_count_reaches_maximum() {
  assert_eq maximum \
    "$(_tier "$(_big_ram)" "$HW_CORES_MAXIMUM" "$(_big_disk)" cuda "$(_big_gpu)")"
}

# ------------------------------------------------------------- disk boundaries

test_one_megabyte_of_free_space_under_the_lightweight_line_is_text_only() {
  assert_eq text-only \
    "$(_tier "$(_big_ram)" "$(_big_cores)" "$((HW_DISK_MB_LIGHTWEIGHT - 1))" cpu 0)"
}

test_exactly_the_lightweight_free_space_is_lightweight() {
  assert_eq lightweight \
    "$(_tier "$(_big_ram)" "$(_big_cores)" "$HW_DISK_MB_LIGHTWEIGHT" cpu 0)"
}

test_one_megabyte_under_the_recommended_free_space_holds_at_lightweight() {
  assert_eq lightweight \
    "$(_tier "$(_big_ram)" "$(_big_cores)" "$((HW_DISK_MB_RECOMMENDED - 1))" cpu 0)"
}

test_exactly_the_recommended_free_space_is_recommended() {
  assert_eq recommended \
    "$(_tier "$(_big_ram)" "$(_big_cores)" "$HW_DISK_MB_RECOMMENDED" cpu 0)"
}

test_one_megabyte_under_the_maximum_free_space_holds_at_recommended() {
  assert_eq recommended \
    "$(_tier "$(_big_ram)" "$(_big_cores)" "$((HW_DISK_MB_MAXIMUM - 1))" cuda "$(_big_gpu)")"
}

test_exactly_the_maximum_free_space_reaches_maximum() {
  assert_eq maximum \
    "$(_tier "$(_big_ram)" "$(_big_cores)" "$HW_DISK_MB_MAXIMUM" cuda "$(_big_gpu)")"
}

# ---------------------------------------------------- graphics memory boundaries

test_a_card_one_megabyte_under_the_useful_line_does_not_lift_the_tier() {
  assert_eq recommended \
    "$(_tier "$(_big_ram)" "$(_big_cores)" "$(_big_disk)" cuda "$((HW_VRAM_MB_ACCELERATED - 1))")"
}

test_a_card_exactly_on_the_useful_line_is_still_not_enough_for_maximum() {
  # Useful and sufficient are two different lines. This one only says the card
  # is worth having; the tier still waits for the second.
  assert_eq recommended \
    "$(_tier "$(_big_ram)" "$(_big_cores)" "$(_big_disk)" cuda "$HW_VRAM_MB_ACCELERATED")"
}

test_the_two_graphics_lines_give_two_different_reasons() {
  # Both answers are "recommended", and they are not the same answer: one card
  # is too small to hold the biggest model and the other is too small to be
  # worth using at all. A single sentence for both would tell somebody with a
  # perfectly good card to go and buy one.
  _hw_tier_compute "$(_big_ram)" "$(_big_cores)" "$(_big_disk)" cuda \
    "$HW_VRAM_MB_ACCELERATED" none
  assert_eq recommended "$_HW_TIER"
  assert_contains "$_HW_TIER_REASON" "cannot hold the largest model"

  _hw_tier_compute "$(_big_ram)" "$(_big_cores)" "$(_big_disk)" cuda \
    "$((HW_VRAM_MB_ACCELERATED - 1))" none
  assert_eq recommended "$_HW_TIER"
  assert_contains "$_HW_TIER_REASON" "too small to be any use"
}

test_a_card_one_megabyte_under_the_maximum_line_holds_at_recommended() {
  assert_eq recommended \
    "$(_tier "$(_big_ram)" "$(_big_cores)" "$(_big_disk)" cuda "$((HW_VRAM_MB_MAXIMUM - 1))")"
}

test_a_card_exactly_on_the_maximum_line_reaches_maximum() {
  assert_eq maximum \
    "$(_tier "$(_big_ram)" "$(_big_cores)" "$(_big_disk)" cuda "$HW_VRAM_MB_MAXIMUM")"
}

# ------------------------------------------------- what acceleration does and does not

test_a_machine_with_no_graphics_card_is_never_recommended_the_largest_models() {
  assert_eq recommended \
    "$(_tier "$((HW_RAM_MB_MAXIMUM * 4))" "$((HW_CORES_MAXIMUM * 4))" "$((HW_DISK_MB_MAXIMUM * 4))" cpu 0)" \
    "a large CPU-only machine can transcribe, just not with the biggest model"
}

test_the_reason_a_large_machine_stops_at_recommended_names_the_graphics_card() {
  _hw_tier_compute "$((HW_RAM_MB_MAXIMUM * 4))" "$((HW_CORES_MAXIMUM * 4))" \
    "$((HW_DISK_MB_MAXIMUM * 4))" cpu 0 none
  assert_eq recommended "$_HW_TIER"
  assert_contains "$_HW_TIER_REASON" "graphics"
}

test_rocm_is_treated_exactly_like_cuda_at_the_same_size() {
  assert_eq maximum \
    "$(_tier "$(_big_ram)" "$(_big_cores)" "$(_big_disk)" rocm "$(_big_gpu)")"
  assert_eq recommended \
    "$(_tier "$(_big_ram)" "$(_big_cores)" "$(_big_disk)" rocm "$((HW_VRAM_MB_MAXIMUM - 1))")"
}

test_metal_needs_no_separate_graphics_memory_because_it_shares_the_machines() {
  # Apple Silicon has one pool of memory. Asking for a VRAM figure it does not
  # have would cap every Mac at recommended for no reason.
  assert_eq maximum \
    "$(_tier "$(_big_ram)" "$(_big_cores)" "$(_big_disk)" metal 0)"
}

test_metal_on_a_small_mac_still_obeys_the_memory_line() {
  assert_eq lightweight \
    "$(_tier "$HW_RAM_MB_LIGHTWEIGHT" "$(_big_cores)" "$(_big_disk)" metal 0)"
}

test_a_graphics_card_cannot_rescue_a_machine_with_no_room_for_the_model() {
  assert_eq text-only \
    "$(_tier "$(_big_ram)" "$(_big_cores)" "$((HW_DISK_MB_LIGHTWEIGHT - 1))" cuda "$(_big_gpu)")" \
    "acceleration caps the tier, it never lifts it"
}

test_an_acceleration_word_nobody_defined_is_treated_as_no_acceleration() {
  assert_eq recommended \
    "$(_tier "$(_big_ram)" "$(_big_cores)" "$(_big_disk)" vulkan 999999)" \
    "the conservative reading of a word this code does not know"
}

# ------------------------------------------------------------ unknown inputs cap

test_not_knowing_how_much_memory_there_is_caps_the_recommendation() {
  local tier
  tier="$(_tier unknown "$(_big_cores)" "$(_big_disk)" cuda "$(_big_gpu)")"
  assert_eq lightweight "$tier" \
    "an unreadable memory figure must never read as a large machine"
}

test_not_knowing_how_much_memory_there_is_says_so() {
  _hw_tier_compute unknown "$(_big_cores)" "$(_big_disk)" cuda "$(_big_gpu)" none
  assert_eq ram-unknown "$_HW_TIER_CAUSE"
  assert_contains "$_HW_TIER_REASON" "could not"
  assert_contains "$_HW_TIER_REASON" "memory"
}

test_not_knowing_how_much_free_space_there_is_caps_the_recommendation() {
  assert_eq lightweight \
    "$(_tier "$(_big_ram)" "$(_big_cores)" unknown cuda "$(_big_gpu)")"
}

test_not_knowing_the_core_count_caps_at_recommended() {
  assert_eq recommended \
    "$(_tier "$(_big_ram)" unknown "$(_big_disk)" cuda "$(_big_gpu)")" \
    "cores are the least of the three, so not knowing them costs the least"
}

test_an_empty_value_is_read_as_unknown_and_not_as_zero() {
  assert_eq lightweight "$(_tier "" "$(_big_cores)" "$(_big_disk)" cuda "$(_big_gpu)")"
}

test_a_value_that_is_not_a_number_is_read_as_unknown() {
  assert_eq lightweight \
    "$(_tier "lots" "$(_big_cores)" "$(_big_disk)" cuda "$(_big_gpu)")"
}

test_a_card_whose_size_could_not_be_read_does_not_reach_maximum() {
  local tier
  tier="$(_tier "$(_big_ram)" "$(_big_cores)" "$(_big_disk)" rocm unknown)"
  assert_eq recommended "$tier"
  _hw_tier_compute "$(_big_ram)" "$(_big_cores)" "$(_big_disk)" rocm unknown none
  assert_contains "$_HW_TIER_REASON" "graphics"
}

test_a_calculation_about_some_other_machine_never_becomes_this_machines_verdict() {
  # hw_tier_for is pure and callable by anybody. If the verdict were "whatever
  # was computed last" rather than "whatever was detected", one what-if
  # anywhere in the wizard would silently rewrite what the voice ticket reads.
  _HW_DETECTED=0
  _HW_TIER=""
  hw_tier_for "$(_big_ram)" "$(_big_cores)" "$(_big_disk)" cuda "$(_big_gpu)" >/dev/null
  assert_eq maximum "$_HW_TIER" "the calculation did happen"
  assert_eq 0 "$_HW_DETECTED" "and it was not a detection"
  assert_eq "" "$(wizard_state_get step.detect.hardware.tier "")" \
    "and nothing was published"
}

# --------------------------------------------------------------- containers cap

test_a_container_whose_limits_could_not_be_read_is_capped() {
  assert_eq recommended \
    "$(_tier "$(_big_ram)" "$(_big_cores)" "$(_big_disk)" cuda "$(_big_gpu)" unknown)" \
    "inside a container the numbers describe the host, not the share of it"
}

test_a_container_whose_limits_were_read_is_not_capped_for_being_one() {
  assert_eq maximum \
    "$(_tier "$(_big_ram)" "$(_big_cores)" "$(_big_disk)" cuda "$(_big_gpu)" known)"
}

test_the_reason_a_container_is_capped_says_the_machine_is_shared() {
  # And says it without the word "container", which the audience does not have.
  _hw_tier_compute "$(_big_ram)" "$(_big_cores)" "$(_big_disk)" cuda "$(_big_gpu)" unknown
  assert_eq container "$_HW_TIER_CAUSE"
  assert_contains "$_HW_TIER_REASON" "only part of this computer is yours"
  assert_not_contains "$_HW_TIER_REASON" "container"
}

test_every_reason_fits_on_one_line_beside_the_words_that_introduce_it() {
  # The report renders "Why not more than that: <reason>." A reason that wraps
  # is a reason that looks like an error message, and the wrap depends on the
  # length of a string nobody looks at again once it is written.
  local lead="Why not more than that: " ram cores disk accel vram limits reason
  for ram in unknown "$((HW_RAM_MB_LIGHTWEIGHT - 1))" "$(_big_ram)"; do
    for cores in unknown "$((HW_CORES_LIGHTWEIGHT - 1))" "$(_big_cores)"; do
      for disk in unknown "$((HW_DISK_MB_LIGHTWEIGHT - 1))" "$(_big_disk)"; do
        for accel in cpu cuda metal; do
          for vram in unknown 0 "$(_big_gpu)"; do
            for limits in none known unknown; do
              _hw_tier_compute "$ram" "$cores" "$disk" "$accel" "$vram" "$limits"
              reason="$_HW_TIER_REASON"
              [ -n "$reason" ] || continue
              if [ "${#lead}" -gt 0 ] && [ "$(( ${#lead} + ${#reason} + 1 ))" -gt 72 ]; then
                fail "too long for one line: \"$lead$reason.\""
              fi
              case "$reason" in
                *[!\ ]*) : ;;
                *) fail "empty reason for cause $_HW_TIER_CAUSE" ;;
              esac
            done
          done
        done
      done
    done
  done
}

test_every_cap_names_a_cause_a_caller_can_branch_on() {
  _hw_tier_compute unknown "$(_big_cores)" "$(_big_disk)" cuda "$(_big_gpu)" none
  assert_eq ram-unknown "$_HW_TIER_CAUSE"
  _hw_tier_compute "$((HW_RAM_MB_LIGHTWEIGHT - 1))" "$(_big_cores)" "$(_big_disk)" cuda "$(_big_gpu)" none
  assert_eq ram "$_HW_TIER_CAUSE"
  _hw_tier_compute "$(_big_ram)" "$(_big_cores)" "$(_big_disk)" cpu 0 none
  assert_eq graphics-none "$_HW_TIER_CAUSE"
  _hw_tier_compute "$(_big_ram)" "$(_big_cores)" "$(_big_disk)" cuda unknown none
  assert_eq graphics-unknown "$_HW_TIER_CAUSE"
  _hw_tier_compute "$(_big_ram)" "$(_big_cores)" "$(_big_disk)" cuda "$(_big_gpu)" none
  assert_eq "" "$_HW_TIER_CAUSE" "nothing capped it, so nothing is named"
}

# ----------------------------------------------------- a machine at its ceiling

test_a_machine_over_every_line_and_nothing_capping_it_has_nothing_to_explain() {
  _hw_tier_compute "$(_big_ram)" "$(_big_cores)" "$(_big_disk)" cuda "$(_big_gpu)" none
  assert_eq maximum "$_HW_TIER"
  assert_eq "" "$_HW_TIER_REASON" \
    "a reason is what is said when the answer is lower than the numbers suggest"
}

test_the_weakest_of_the_three_measurements_is_the_one_that_decides() {
  assert_eq text-only \
    "$(_tier "$((HW_RAM_MB_LIGHTWEIGHT - 1))" "$(_big_cores)" "$(_big_disk)" cuda "$(_big_gpu)")"
  assert_eq text-only \
    "$(_tier "$(_big_ram)" "$((HW_CORES_LIGHTWEIGHT - 1))" "$(_big_disk)" cuda "$(_big_gpu)")"
}

# --------------------------------------------------------- the comparison helper

test_at_least_answers_yes_for_the_tier_itself_and_everything_under_it() {
  _HW_DETECTED=1
  _HW_TIER=recommended
  hw_tier_at_least text-only    || fail "recommended is at least text-only"
  hw_tier_at_least lightweight  || fail "recommended is at least lightweight"
  hw_tier_at_least recommended  || fail "recommended is at least recommended"
  hw_tier_at_least maximum      && fail "recommended is not at least maximum"
  return 0
}

# ------------------------------------------------ the numbers live in one place

test_no_cut_off_is_written_down_anywhere_but_the_constant_block() {
  # A threshold copied into a probe or into a sentence is a threshold that will
  # drift. The constants are the only place a number belongs. The core counts
  # are not scanned for: 2, 4 and 8 are every other number in a shell script
  # too, and a lint that cries wolf is a lint somebody deletes.
  stub_real grep sed
  local body offenders
  body="$(sed '/^# --- the threshold table/,/^# --- end of the threshold table/d' \
    "$REPO_ROOT/lib/steps/20-hardware.sh")"
  offenders="$(printf '%s\n' "$body" \
    | grep -n -E '(^|[^A-Za-z0-9_])(1536|3584|6144|7168|12288|15360)([^A-Za-z0-9_]|$)' \
    | grep -v -E '^[0-9]+:[[:space:]]*#' || true)"
  assert_eq "" "$offenders" \
    "every cut-off belongs to the constant block; this is a copy of one"
}
