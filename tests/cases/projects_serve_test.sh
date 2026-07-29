# What the picker actually answers with: the ranking, the row shape the page
# consumes, and the two properties that make it usable from a phone -- a
# request never waits on the filesystem, and a query the user has already
# typed past delivers nothing.

setup() {
  require_host_command python3
  stub_real python3
  stub_command tmux                 # no live sessions unless a test says so
  TREE="$HOME/tree"
  mkdir -p "$TREE"
  AYEAYE_PROJECT_ROOTS="$TREE"
  # Long enough that a small tree is always walked to completion, so the
  # ranking tests are about ranking and not about timing.
  AYEAYE_PROJECT_WAIT="5"
  export AYEAYE_PROJECT_ROOTS AYEAYE_PROJECT_WAIT
}

probe() {
  run_script "$TESTS_DIR/lib/project_probe.py" "$@"
}

probe_value() {
  printf '%s\n' "$RUN_STDOUT" | sed -n "s/^$1=//p"
}

# The nth result directory, 1-based.
probe_row() {
  printf '%s\n' "$RUN_STDOUT" | grep "^$TREE" | sed -n "$1p"
}

assert_below() {
  local why="${3:-a bound did not hold}"
  awk -v v="$1" -v b="$2" 'BEGIN { exit !(v + 0 < b + 0) }' \
    || fail "$why (expected $1 to be below $2)"
}

test_the_row_shape_the_page_consumes_is_unchanged() {
  mkdir -p "$TREE/one"
  probe shape
  assert_status 0 "$RUN_STATUS" "$RUN_STDERR"
  assert_contains "$(probe_value keys)" "dir"
  assert_contains "$(probe_value keys)" "short"
  assert_contains "$(probe_value keys)" "session"
  assert_contains "$(probe_value keys)" "exists"
}

test_a_path_under_home_is_shortened_for_display() {
  mkdir -p "$TREE/one"
  probe projects --field short
  assert_contains "$RUN_STDOUT" "~/tree/one"
}

test_a_git_repository_outranks_a_plain_directory() {
  mkdir -p "$TREE/aaa-plain" "$TREE/zzz-repo/.git"
  probe projects --field dir
  assert_eq "2" "$(probe_value count)"
  assert_eq "$TREE/zzz-repo" "$(probe_row 1)" \
    "being a repository is the strongest signal a directory is a project"
  assert_eq "$TREE/aaa-plain" "$(probe_row 2)"
}

test_a_recent_pick_outranks_a_repository_you_never_opened() {
  mkdir -p "$TREE/aaa-repo/.git" "$TREE/zzz-repo/.git"
  probe recents note "$TREE/zzz-repo"
  probe projects --field dir
  assert_eq "$TREE/zzz-repo" "$(probe_row 1)"
  assert_eq "$TREE/aaa-repo" "$(probe_row 2)"
}

test_a_pick_outside_the_search_roots_is_still_offered() {
  mkdir -p "$TREE/here" "$HOME/elsewhere/project"
  probe recents note "$HOME/elsewhere/project"
  probe projects --field dir
  assert_contains "$RUN_STDOUT" "$HOME/elsewhere/project" \
    "a directory you have used stays reachable even when the walk cannot see it"
}

test_a_pick_that_no_longer_exists_is_not_offered() {
  mkdir -p "$TREE/here"
  probe recents note "$HOME/deleted-last-week"
  probe projects --field dir
  assert_not_contains "$RUN_STDOUT" "deleted-last-week"
}

test_a_name_match_beats_a_match_buried_in_the_path() {
  mkdir -p "$TREE/nova" "$TREE/nova-parent/child"
  probe projects --query nova --field dir
  assert_eq "$TREE/nova" "$(probe_row 1)"
}

test_a_query_narrows_the_list() {
  mkdir -p "$TREE/alpha" "$TREE/beta"
  probe projects --query alph --field dir
  assert_eq "1" "$(probe_value count)"
  assert_contains "$RUN_STDOUT" "$TREE/alpha"
}

test_the_limit_is_respected() {
  probe maketree "$TREE" --width 4 --depth 3
  probe projects --limit 3 --field dir
  assert_eq "3" "$(probe_value count)"
}

test_a_session_that_already_exists_is_marked() {
  mkdir -p "$TREE/myproj"
  stub_command tmux --stdout "myproj"
  probe projects
  assert_contains "$RUN_STDOUT" '"exists": true'
  assert_contains "$RUN_STDOUT" '"session": "myproj"'
}

test_a_request_answers_from_a_walk_that_has_not_finished() {
  probe maketree "$TREE" --width 6 --depth 5
  assert_eq "9330" "$(probe_value made)"
  AYEAYE_PROJECT_WAIT="0.3"
  export AYEAYE_PROJECT_WAIT
  # Two milliseconds a listing, and a query that matches one subtree rather
  # than everything: too few matches to end the walk early, but the first of
  # them are found almost immediately. So the walk is certainly still going
  # when the wait expires, and the answer has to come from a walk in progress.
  probe --slow 0.002 projects --query L1n0/L2n0 --field dir
  assert_below 0.25 "$(probe_value elapsed)" \
    "the walk really did outlast the wait, or this proves nothing"
  assert_below "$(probe_value elapsed)" 1.5 \
    "the wait bounds the response; the walk's own budget does not"
  assert_below 0 "$(probe_value count)" \
    "a walk still running must still deliver what it has found so far"
  assert_contains "$RUN_STDOUT" "$TREE/L1n0/L2n0"
}

test_two_callers_asking_the_same_thing_share_one_walk() {
  probe maketree "$TREE" --width 5 --depth 4
  AYEAYE_PROJECT_WAIT="10"
  export AYEAYE_PROJECT_WAIT
  probe --slow 0.002 join --query L2n1 --gap 0.2
  assert_status 0 "$RUN_STATUS" "$RUN_STDERR"
  assert_eq "1" "$(probe_value walks)" \
    "a second caller joins the walk in flight rather than restarting it"
  assert_eq "$(probe_value first_rows)" "$(probe_value second_rows)"
  assert_below 0 "$(probe_value first_rows)" \
    "and neither caller is starved by the other"
}

test_a_superseded_search_delivers_nothing() {
  probe maketree "$TREE" --width 6 --depth 4
  AYEAYE_PROJECT_WAIT="10"
  export AYEAYE_PROJECT_WAIT
  # Queries narrow enough that neither can reach the candidate cap and stop
  # early, so the first is certainly still walking when the second arrives.
  probe --slow 0.004 supersede --query-a L1n0/L2n0 --query-b L1n1/L2n0 --gap 0.25
  assert_status 0 "$RUN_STATUS" "$RUN_STDERR"
  assert_eq "0" "$(probe_value first_rows)" \
    "the query the user has typed past must not race the current one"
  assert_below 0 "$(probe_value second_rows)" "the current query still answers"
  assert_eq "yes" "$(probe_value stopped_walking)" \
    "cancellation stops the walking thread, not merely the waiting one"
}

test_a_walk_that_finishes_late_is_still_kept() {
  probe maketree "$TREE" --width 5 --depth 4
  AYEAYE_PROJECT_WAIT="0.15"
  export AYEAYE_PROJECT_WAIT
  # The first call gives up waiting long before the walk ends. The second
  # asks the same thing once the walk has had time to finish, and must be
  # answered from that finished walk rather than starting another.
  probe --slow 0.002 cached --query L2n1 --settle 3
  assert_status 0 "$RUN_STATUS" "$RUN_STDERR"
  assert_below 0 "$(probe_value settle_listings)" \
    "the walk carries on after the request that started it gave up"
  assert_below 0 "$(probe_value second_count)" \
    "a walk nobody was still waiting for is not thrown away"
  assert_eq "0" "$(probe_value second_listings)" \
    "and the query that outlived its waiter is not walked twice"
}

test_the_limit_is_a_limit_even_at_zero() {
  mkdir -p "$TREE/one" "$TREE/two"
  probe projects --limit 0 --field dir
  assert_eq "0" "$(probe_value count)"
}

test_a_sibling_of_home_is_not_mistaken_for_a_path_inside_it() {
  mkdir -p "$HOME-ish/tree/proj"
  AYEAYE_PROJECT_ROOTS="$HOME-ish/tree"
  export AYEAYE_PROJECT_ROOTS
  probe projects --field short
  assert_contains "$RUN_STDOUT" "$HOME-ish/tree/proj" \
    "collapsing to ~ is a path prefix, not a string prefix"
  assert_not_contains "$RUN_STDOUT" "~" \
    "nothing here is under home, so nothing may be shortened as if it were"
}

test_a_repeated_query_is_answered_without_walking_again() {
  mkdir -p "$TREE/alpha" "$TREE/beta"
  probe cached --query alph
  assert_status 0 "$RUN_STATUS" "$RUN_STDERR"
  assert_eq "$(probe_value first_count)" "$(probe_value second_count)"
  assert_below 0 "$(probe_value first_listings)"
  assert_eq "0" "$(probe_value second_listings)" \
    "typing does not re-walk what a completed search already answered"
}
