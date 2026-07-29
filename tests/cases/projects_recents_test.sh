# The pick store: where AyeAye keeps its own record of which directories you
# actually start agents in. It is the replacement for zoxide's frecency, so
# what matters is that it survives the process that wrote it, that it orders
# by use rather than by luck, and that nothing about it can break the picker.

setup() {
  require_host_command python3
  stub_real python3
  STORE="$XDG_STATE_HOME/ayeaye/projects.json"
}

probe() {
  run_script "$TESTS_DIR/lib/project_probe.py" "$@"
}

probe_value() {
  printf '%s\n' "$RUN_STDOUT" | sed -n "s/^$1=//p"
}

# assert_below <value> <bound> <message> - numeric, because scores are floats.
assert_below() {
  awk -v v="$1" -v b="$2" 'BEGIN { exit !(v + 0 < b + 0) }' \
    || fail "$3 (got $1, expected below $2)"
}

score_for() {   # score_for <count> <age-in-days>
  probe recents score --count "$1" --age-days "$2"
  probe_value score
}

test_a_pick_is_recorded_in_the_state_directory() {
  probe recents note /srv/one
  assert_status 0 "$RUN_STATUS" "$RUN_STDERR"
  assert_file_exists "$STORE"
  assert_file_contains "$STORE" "/srv/one"
}

test_a_pick_survives_the_process_that_recorded_it() {
  probe recents note /srv/one /srv/two /srv/one
  probe recents dump
  assert_eq "2" "$(probe_value entries)"
  assert_contains "$RUN_STDOUT" "pick	/srv/one	2" \
    "picking the same directory twice counts twice"
  assert_contains "$RUN_STDOUT" "pick	/srv/two	1"
}

test_a_trailing_slash_is_the_same_directory() {
  probe recents note /srv/one/
  probe recents note /srv/one
  probe recents dump
  assert_eq "1" "$(probe_value entries)"
  assert_contains "$RUN_STDOUT" "pick	/srv/one	2"
}

test_more_picks_score_higher_than_fewer() {
  local many few
  many="$(score_for 5 0)"
  few="$(score_for 1 0)"
  assert_below "$few" "$many" "five picks today must outrank one"
}

test_an_older_pick_scores_lower_than_a_newer_one() {
  local fresh stale
  fresh="$(score_for 1 0)"
  stale="$(score_for 1 30)"
  assert_below "$stale" "$fresh" "a month-old pick must not outrank today's"
}

test_a_habit_outranks_a_flurry_from_last_spring() {
  local daily burst
  daily="$(score_for 3 1)"
  burst="$(score_for 10 180)"
  assert_below "$burst" "$daily" \
    "frecency, not frequency: ten picks half a year ago are stale"
}

test_a_missing_store_is_simply_no_history() {
  assert_file_missing "$STORE"
  probe recents dump
  assert_status 0 "$RUN_STATUS" "$RUN_STDERR"
  assert_eq "0" "$(probe_value entries)"
}

test_a_corrupt_store_degrades_to_no_history_and_heals() {
  mkdir -p "$XDG_STATE_HOME/ayeaye"
  printf '{"picks": {"/srv/one": {"n": ' > "$STORE"
  probe recents dump
  assert_status 0 "$RUN_STATUS" "$RUN_STDERR"
  assert_eq "0" "$(probe_value entries)" "half a file is no history, not an error"
  assert_not_contains "$RUN_STDERR" "Traceback"

  probe recents note /srv/two
  probe recents dump
  assert_eq "1" "$(probe_value entries)" "the next pick rewrites the store"
}

test_a_store_of_the_wrong_shape_is_ignored() {
  mkdir -p "$XDG_STATE_HOME/ayeaye"
  printf '{"picks": ["not", "a", "map"]}' > "$STORE"
  probe recents dump
  assert_status 0 "$RUN_STATUS" "$RUN_STDERR"
  assert_eq "0" "$(probe_value entries)"
}

test_the_store_stays_capped_however_many_projects_you_visit() {
  local args="" i=1
  while [ "$i" -le 250 ]; do
    args="$args /srv/p$i"
    i=$((i + 1))
  done
  # shellcheck disable=SC2086
  probe recents note $args
  assert_status 0 "$RUN_STATUS" "$RUN_STDERR"
  probe recents dump
  assert_eq "200" "$(probe_value entries)" \
    "the store is a ranking signal, not a history file"
}
