# The bounds on the project picker's filesystem walk: what it refuses to
# enter, how deep it goes, and which of depth, time, cancellation or the
# result limit ends it. Every tree here is synthetic and lives inside the
# sandbox; the walk never sees a real home directory.

setup() {
  require_host_command python3
  stub_real python3
  TREE="$HOME/tree"
  mkdir -p "$TREE"
  AYEAYE_PROJECT_ROOTS="$TREE"
  export AYEAYE_PROJECT_ROOTS
}

probe() {
  run_script "$TESTS_DIR/lib/project_probe.py" "$@"
}

# The value of one `key=value` line the probe printed.
probe_value() {
  printf '%s\n' "$RUN_STDOUT" | sed -n "s/^$1=//p"
}

# assert_below <value> <bound> <message> - a numeric comparison that reads as
# an assertion, for the counts and durations a bound is supposed to hold down.
assert_below() {
  awk -v v="$1" -v b="$2" 'BEGIN { exit !(v + 0 < b + 0) }' \
    || fail "$3 (got $1, expected below $2)"
}

test_the_walk_finds_directories_below_the_root() {
  mkdir -p "$TREE/alpha" "$TREE/beta/gamma"
  probe walk
  assert_status 0 "$RUN_STATUS" "$RUN_STDERR"
  assert_contains "$RUN_STDOUT" "$TREE/alpha"
  assert_contains "$RUN_STDOUT" "$TREE/beta/gamma"
  assert_eq "exhausted" "$(probe_value stopped)" \
    "a small tree is walked to the end, no bound needed"
}

test_a_query_selects_by_substring_of_the_path() {
  mkdir -p "$TREE/alpha" "$TREE/beta"
  probe walk --query alph
  assert_contains "$RUN_STDOUT" "$TREE/alpha"
  assert_not_contains "$RUN_STDOUT" "$TREE/beta"
}

test_an_excluded_name_is_neither_returned_nor_entered() {
  mkdir -p "$TREE/proj/node_modules/left-pad/inner"
  mkdir -p "$TREE/proj/target/debug"
  mkdir -p "$TREE/proj/.venv/lib/site-packages"
  mkdir -p "$TREE/proj/vendor/bundle"
  mkdir -p "$TREE/proj/src"
  probe walk
  assert_contains "$RUN_STDOUT" "$TREE/proj/src"
  assert_not_contains "$RUN_STDOUT" "node_modules"
  assert_not_contains "$RUN_STDOUT" "left-pad" \
    "an excluded directory must not be descended into either"
  assert_not_contains "$RUN_STDOUT" "target"
  assert_not_contains "$RUN_STDOUT" "vendor"
  assert_not_contains "$RUN_STDOUT" "site-packages"
}

test_a_hidden_directory_is_tool_internals_not_a_project() {
  mkdir -p "$TREE/.cache/huggingface" "$TREE/.local/share/thing" "$TREE/work"
  probe walk
  assert_contains "$RUN_STDOUT" "$TREE/work"
  assert_not_contains "$RUN_STDOUT" ".cache"
  assert_not_contains "$RUN_STDOUT" ".local"
}

test_the_exclusion_set_is_extensible_from_the_environment() {
  mkdir -p "$TREE/scratchpad" "$TREE/work"
  AYEAYE_PROJECT_SKIP="scratchpad"
  export AYEAYE_PROJECT_SKIP
  probe walk
  assert_contains "$RUN_STDOUT" "$TREE/work"
  assert_not_contains "$RUN_STDOUT" "scratchpad"
}

test_the_depth_bound_stops_the_walk() {
  mkdir -p "$TREE/a/b/c/d"
  probe walk --depth 2
  assert_contains "$RUN_STDOUT" "$TREE/a"
  assert_contains "$RUN_STDOUT" "$TREE/a/b"
  assert_not_contains "$RUN_STDOUT" "$TREE/a/b/c"
  assert_eq "depth" "$(probe_value stopped)"
}

test_a_symlinked_directory_is_not_followed() {
  mkdir -p "$TREE/real/inside"
  ln -s "$TREE/real" "$TREE/mirror"
  probe walk
  assert_contains "$RUN_STDOUT" "$TREE/real/inside"
  assert_not_contains "$RUN_STDOUT" "mirror" \
    "a symlinked directory is a second name for something already walked"
}

test_a_git_repository_is_flagged_however_dot_git_is_stored() {
  mkdir -p "$TREE/repo/.git" "$TREE/plain"
  mkdir -p "$TREE/worktree"
  printf 'gitdir: /elsewhere\n' > "$TREE/worktree/.git"
  probe walk
  assert_contains "$RUN_STDOUT" "git	1	$TREE/repo"
  assert_contains "$RUN_STDOUT" "git	1	$TREE/worktree" \
    "a worktree and a submodule keep .git as a file, not a directory"
  assert_contains "$RUN_STDOUT" "dir	1	$TREE/plain"
}

test_the_result_limit_stops_the_walk_early() {
  probe maketree "$TREE" --width 6 --depth 5
  assert_status 0 "$RUN_STATUS" "$RUN_STDERR"
  probe walk --query L1n --limit 5
  assert_eq "limit" "$(probe_value stopped)"
  assert_eq "5" "$(probe_value count)"
  assert_below "$(probe_value listings)" 5 \
    "stopping at the limit means the rest of the tree is never listed"
}

test_a_huge_tree_returns_within_the_time_bound() {
  probe maketree "$TREE" --width 6 --depth 5
  assert_status 0 "$RUN_STATUS" "$RUN_STDERR"
  assert_eq "9330" "$(probe_value made)" "the tree really is that big"
  # Half a millisecond per listing stands in for a filesystem that is not a
  # warm tmpfs, so the deadline is what ends this walk on any host.
  probe --slow 0.0005 walk --query nothingmatchesthis --budget 0.25
  assert_eq "deadline" "$(probe_value stopped)"
  assert_eq "0" "$(probe_value count)"
  assert_below "$(probe_value elapsed)" 2.0 "the time bound must actually bind"
  assert_below "$(probe_value listings)" 3000 \
    "a bounded walk visits a fraction of a 9330-directory tree"
}

test_one_enormous_directory_cannot_hold_the_walk_hostage() {
  # The bound that matters is not "how many directories" but "how many
  # entries in one of them": a single cache directory with a hundred
  # thousand children would otherwise be listed to the end before anything
  # could check the clock or the cancel flag.
  probe maketree "$TREE/huge" --width 6000 --depth 1
  assert_eq "6000" "$(probe_value made)"
  mkdir -p "$TREE/small"
  probe walk
  assert_status 0 "$RUN_STATUS" "$RUN_STDERR"
  assert_contains "$RUN_STDOUT" "$TREE/small"
  local hits
  hits="$(printf '%s\n' "$RUN_STDOUT" | grep -c "^hit")"
  assert_below "$hits" 5100 \
    "one directory may not contribute more entries than the per-listing cap"
}

test_a_cancelled_walk_stops_walking() {
  probe maketree "$TREE" --width 6 --depth 4
  probe --slow 0.01 walk --cancel-after 0.2
  assert_eq "cancel" "$(probe_value stopped)"
  assert_below "$(probe_value elapsed)" 2.0 \
    "cancellation ends the walk, it does not merely mark it"
}
