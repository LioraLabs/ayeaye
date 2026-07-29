# What install.sh does with its arguments today. This is a regression net, not
# a wish list: where the current behaviour looks wrong the test pins the wrong
# behaviour and says so, so that the wizard rewrite has to change it on
# purpose rather than by accident.
#
# Argument handling runs before any dependency check, so none of these runs
# needs a stubbed tmux or python3.

test_help_prints_the_usage_banner_and_exits_zero() {
  run_install --help
  assert_status 0 "$RUN_STATUS"
  assert_contains "$RUN_STDOUT" "One-command setup for ayeaye"
  assert_contains "$RUN_STDOUT" "./install.sh              interactive, sane defaults"
  assert_contains "$RUN_STDOUT" "./install.sh --defaults   accept every default, no prompts"
  assert_contains "$RUN_STDOUT" "./install.sh --no-systemd skip the unit; prints the manual run command"
  assert_eq "" "$RUN_STDERR" "help is not a diagnostic"
}

test_short_help_is_the_same_as_long_help() {
  run_install -h
  local short="$RUN_STDOUT"
  run_install --help
  assert_eq "$short" "$RUN_STDOUT"
  assert_status 0 "$RUN_STATUS"
}

test_the_help_banner_is_stripped_of_its_comment_markers() {
  run_install --help
  assert_not_contains "$RUN_STDOUT" "#!/usr/bin/env bash"
  case "$RUN_STDOUT" in
    "#"*) fail "the banner still starts with a comment marker: $RUN_STDOUT" ;;
  esac
}

test_help_stops_short_of_the_full_what_it_does_list() {
  # Documented, not endorsed: --help lifts a fixed line range out of the
  # script's own header, and the header has since grown past it. Steps 3 and 4
  # of "what it does" are in the file but never printed.
  run_install --help
  assert_contains "$RUN_STDOUT" "1. checks hard deps"
  assert_contains "$RUN_STDOUT" "2. writes"
  assert_not_contains "$RUN_STDOUT" "3. ensures the auth token exists" \
    "today's help output is truncated mid-list; a rewrite should fix it deliberately"
}

test_help_touches_nothing() {
  run_install --help
  assert_status 0 "$RUN_STATUS"
  assert_file_missing "$XDG_CONFIG_HOME/ayeaye/env"
  assert_file_missing "$XDG_STATE_HOME/ayeaye/token"
  assert_file_missing "$XDG_CONFIG_HOME/systemd/user/ayeaye.service"
  assert_eq "" "$(ls -A "$XDG_CONFIG_HOME")" "help must not create so much as a directory"
}

test_an_unknown_option_exits_two_and_says_so_on_stderr() {
  run_install --frobnicate
  assert_status 2 "$RUN_STATUS"
  assert_eq "unknown option: --frobnicate (try --help)" "$RUN_STDERR"
  assert_eq "" "$RUN_STDOUT" "the complaint belongs on stderr, not stdout"
}

test_a_bare_word_is_an_unknown_option_too() {
  run_install install
  assert_status 2 "$RUN_STATUS"
  assert_contains "$RUN_STDERR" "unknown option: install"
}

test_an_empty_argument_is_rejected() {
  run_install ""
  assert_status 2 "$RUN_STATUS"
  assert_contains "$RUN_STDERR" "unknown option:"
}

test_a_rejected_option_stops_before_anything_is_written() {
  run_install --frobnicate
  assert_status 2 "$RUN_STATUS"
  assert_file_missing "$XDG_CONFIG_HOME/ayeaye/env"
  assert_file_missing "$XDG_STATE_HOME/ayeaye/token"
}

test_argument_parsing_is_one_pass_over_every_argument() {
  # Every argument is inspected, so a bad one anywhere is fatal even when a
  # good one precedes it.
  run_install --defaults --frobnicate
  assert_status 2 "$RUN_STATUS"
  assert_contains "$RUN_STDERR" "unknown option: --frobnicate"
}

test_help_wins_when_it_comes_first() {
  # The mirror image: --help exits from inside the loop, so a bad option after
  # it is never reached.
  run_install --help --frobnicate
  assert_status 0 "$RUN_STATUS"
  assert_contains "$RUN_STDOUT" "One-command setup for ayeaye"
  assert_eq "" "$RUN_STDERR"
}

test_the_known_flags_are_accepted_together() {
  # Accepted here means "not rejected by the parser": both flags get past
  # argument handling and the run proceeds to the dependency check, which
  # fails because this sandbox has no tmux.
  run_install --defaults --no-systemd
  assert_ne 2 "$RUN_STATUS" "neither flag may be treated as unknown"
  assert_not_contains "$RUN_STDERR" "unknown option"
}
