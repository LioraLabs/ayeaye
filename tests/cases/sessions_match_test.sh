# Which tmux session an agent lands in, and how it gets there.
#
# Two rules, and nothing outside this file was pinning either of them. The
# first is the name: a project directory is turned into a session name by the
# same convention the ftz/ftn scripts use, so that an agent started from the
# phone and one started by hand in a terminal end up in the same place rather
# than in two sessions with almost the same name. The second is what happens
# when that name is already taken - a second window in the existing session,
# never a second session - because "everything for one project under one name"
# is the whole reason the naming rule exists.
#
# Nothing here runs tmux. tmux is a recording stub, and what is asserted is the
# argv the app would have handed it: that is the only honest way to test window
# creation from a suite that has no terminal and must leave no sessions behind.

setup() {
  require_host_command python3
  stub_real python3
  WORK="$HOME/work"
  mkdir -p "$WORK"
}

probe() {
  run_script "$TESTS_DIR/lib/project_probe.py" "$@"
}

# The value of one `key=value` line the probe printed.
probe_value() {
  printf '%s\n' "$RUN_STDOUT" | sed -n "s/^$1=//p"
}

# _tmux_with_sessions [name...] - a tmux that reports those sessions as live
# and hands back a pane id for anything that creates one.
#
# The pane id matters: spawn_agent treats empty output from new-window or
# new-session as "tmux would not create it" and gives up before sending any
# keys, so a stub that printed nothing would make every test here pass for the
# wrong reason. The list-sessions rule is set even when there are no sessions,
# rather than letting the default response answer it, because the default is
# the pane id - and a tmux that answered "%7" to list-sessions would be
# claiming a session called %7 exists.
_tmux_with_sessions() {
  local names=""
  while [ "$#" -gt 0 ]; do
    if [ -n "$names" ]; then names="$names
$1"; else names="$1"; fi
    shift
  done
  stub_command tmux --stdout "%7"
  stub_when tmux 'list-sessions *' --stdout "$names"
}

# ------------------------------------------------------------- the name rule

test_a_directory_name_is_the_session_name() {
  probe session-name "$WORK/thing"
  assert_status 0 "$RUN_STATUS" "$RUN_STDERR"
  assert_eq "thing" "$(probe_value name)" \
    "the basename, not the path: a session name is what a human types after -t"
}

test_a_dot_in_the_directory_name_becomes_an_underscore() {
  probe session-name "$WORK/web.api"
  assert_eq "web_api" "$(probe_value name)" \
    "tmux reads a dot in a target as a window separator, so a session called
web.api can never be addressed by name"
}

test_a_trailing_slash_names_the_same_session() {
  probe session-name "$WORK/thing/"
  assert_eq "thing" "$(probe_value name)" \
    "a path from a picker may or may not carry one, and it is not a different project"
}

test_a_project_can_name_its_own_session_in_tmux_yaml() {
  mkdir -p "$WORK/thing"
  printf 'session_name: dashboard\n' > "$WORK/thing/.tmux.yaml"
  probe session-name "$WORK/thing"
  assert_eq "dashboard" "$(probe_value name)" \
    "a project already driving tmux from a config file has already chosen a name"
}

test_a_named_session_is_still_subject_to_the_dot_rule() {
  # The substitution is not a fixup for basenames; it is what makes a name
  # addressable at all, so it applies to a name that came out of the file too.
  mkdir -p "$WORK/thing"
  printf 'session_name: my.dashboard\n' > "$WORK/thing/.tmux.yaml"
  probe session-name "$WORK/thing"
  assert_eq "my_dashboard" "$(probe_value name)"
}

test_a_tmux_yaml_without_a_session_name_changes_nothing() {
  # Most .tmux.yaml files are about windows and panes and never mention a
  # session name. Reading one must not cost the directory its own name.
  mkdir -p "$WORK/thing"
  printf 'windows:\n  - editor\n  - shell\n' > "$WORK/thing/.tmux.yaml"
  probe session-name "$WORK/thing"
  assert_eq "thing" "$(probe_value name)"
}

test_an_empty_session_name_falls_back_to_the_directory() {
  # A key with nothing after it is a half-finished edit, not a request for a
  # session with no name - which tmux would refuse anyway.
  mkdir -p "$WORK/thing"
  printf 'session_name:\n' > "$WORK/thing/.tmux.yaml"
  probe session-name "$WORK/thing"
  assert_eq "thing" "$(probe_value name)"
}

test_an_unreadable_tmux_yaml_is_not_an_error() {
  # The picker offers directories it has only listed, and a project may well
  # be one nobody can read into. Losing the override is fine; failing to name
  # the session at all would mean no agent could be started there.
  mkdir -p "$WORK/thing/.tmux.yaml"
  probe session-name "$WORK/thing"
  assert_status 0 "$RUN_STATUS" "$RUN_STDERR"
  assert_eq "thing" "$(probe_value name)"
  assert_not_contains "$RUN_STDERR" "Traceback"
}

# ----------------------------------------------------------------- spawning

test_a_project_with_no_session_gets_one_of_its_own() {
  mkdir -p "$WORK/thing"
  _tmux_with_sessions
  probe spawn "$WORK/thing"
  assert_status 0 "$RUN_STATUS" "$RUN_STDERR"
  assert_eq "session" "$(probe_value created)"
  assert_eq "thing" "$(probe_value session)"
  assert_stub_called_with tmux "tmux new-session -d -s thing -c $WORK/thing"
  assert_not_contains "$(stub_calls tmux)" "new-window" \
    "there was nothing to put a window in"
}

test_a_project_with_a_live_session_gets_a_window_in_it() {
  mkdir -p "$WORK/thing"
  _tmux_with_sessions thing other
  probe spawn "$WORK/thing"
  assert_status 0 "$RUN_STATUS" "$RUN_STDERR"
  assert_eq "window" "$(probe_value created)"
  assert_eq "thing" "$(probe_value session)"
  assert_stub_called_with tmux "tmux new-window -t =thing -c $WORK/thing"
  assert_not_contains "$(stub_calls tmux)" "new-session" \
    "a second session for the same project is the thing the naming rule exists to prevent"
}

test_the_window_target_forces_an_exact_session_match() {
  # The "=" is load-bearing and is commented as such in bin/ayeaye. tmux
  # resolves a bare -t target as a prefix, so a project called "web" would
  # otherwise open its window inside a session called "website" that happens
  # to sort first - an agent running in someone else's project, with no error
  # anywhere to say so.
  mkdir -p "$WORK/web"
  _tmux_with_sessions web website
  probe spawn "$WORK/web"
  assert_stub_called_with tmux "new-window -t =web" \
    "without the = this target is a prefix, and prefixes are not projects"
  assert_not_contains "$(stub_calls tmux)" "new-window -t web " \
    "the unguarded form must not appear at all"
}

test_a_session_that_merely_starts_with_the_name_is_a_different_project() {
  # The other half of the same idea, one layer up: whether a session exists is
  # exact set membership, so "website" being live says nothing about "web".
  mkdir -p "$WORK/web"
  _tmux_with_sessions website webhooks
  probe spawn "$WORK/web"
  assert_eq "session" "$(probe_value created)" \
    "web has no session of its own, whatever else is running"
  assert_stub_called_with tmux "tmux new-session -d -s web -c $WORK/web"
}

test_the_session_a_dotted_project_is_spawned_into_is_the_substituted_one() {
  # The name rule and the spawn have to agree, or an agent started twice in
  # the same directory lands in two sessions.
  mkdir -p "$WORK/web.api"
  _tmux_with_sessions web_api
  probe spawn "$WORK/web.api"
  assert_eq "window" "$(probe_value created)" \
    "the live session is this project's, spelled the way tmux was told to spell it"
  assert_stub_called_with tmux "new-window -t =web_api"
}

test_the_session_named_in_tmux_yaml_is_the_one_spawned_into() {
  mkdir -p "$WORK/thing"
  printf 'session_name: dashboard\n' > "$WORK/thing/.tmux.yaml"
  _tmux_with_sessions dashboard
  probe spawn "$WORK/thing"
  assert_eq "dashboard" "$(probe_value session)"
  assert_eq "window" "$(probe_value created)"
  assert_stub_called_with tmux "new-window -t =dashboard"
}

test_a_directory_that_is_not_there_starts_nothing() {
  _tmux_with_sessions
  probe spawn "$WORK/never-existed"
  assert_status 0 "$RUN_STATUS" "$RUN_STDERR"
  assert_eq "no such directory" "$(probe_value error)"
  assert_stub_not_called tmux "not even to ask what sessions exist"
}

# -------------------------------------------------------- which agents exist

test_only_the_two_known_agents_may_be_spawned() {
  # This endpoint starts processes on the machine, and the agent arrives in an
  # HTTP request. The name is looked up in a table rather than interpolated
  # into a command line, so anything not in the table is refused before tmux
  # is reached at all - and the command finally typed is the table's value,
  # not the request's.
  mkdir -p "$WORK/thing"
  _tmux_with_sessions
  probe spawn "$WORK/thing" --agent claude
  assert_eq "claude" "$(probe_value agent)"
  assert_stub_called_with tmux "tmux send-keys -t %7 -l claude"

  _tmux_with_sessions
  probe spawn "$WORK/thing" --agent codex
  assert_eq "codex" "$(probe_value agent)"
  assert_stub_called_with tmux "tmux send-keys -t %7 -l codex"
}

test_an_agent_the_table_does_not_know_is_refused_before_tmux() {
  mkdir -p "$WORK/thing"
  _tmux_with_sessions
  probe spawn "$WORK/thing" --agent bash
  assert_status 0 "$RUN_STATUS" "$RUN_STDERR"
  assert_eq "unknown agent" "$(probe_value error)"
  assert_stub_not_called tmux "refused before anything could be started"
}

test_an_agent_name_carrying_a_shell_is_not_a_command() {
  # The failure this pins is not "the semicolon runs": it is the name reaching
  # a command line at all. A table lookup cannot be escaped out of, so the
  # answer is the same "unknown agent" any other typo would get.
  mkdir -p "$WORK/thing"
  _tmux_with_sessions
  probe spawn "$WORK/thing" --agent "claude; touch $TEST_TMPDIR/pwned"
  assert_eq "unknown agent" "$(probe_value error)"
  assert_file_missing "$TEST_TMPDIR/pwned"
  assert_stub_not_called tmux
}
