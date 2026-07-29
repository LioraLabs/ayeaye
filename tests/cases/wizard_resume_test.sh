# Being run twice: once because the first run was interrupted, and once
# because somebody wants to change something.
#
# Those are different things and the wizard has to tell them apart. An
# interrupted run picks up where it stopped and does not redo what it already
# did. A deliberate rerun walks every stage again, because walking them again
# is the only way to change an answer - and it still keeps the key, the
# settings and everything else it was not asked about.

_hard_deps_present() {
  require_host_command python3
  stub_command tmux
  stub_real python3
}

_state() { printf '%s' "$XDG_STATE_HOME/ayeaye/setup-state"; }
_token() { printf '%s' "$XDG_STATE_HOME/ayeaye/token"; }
_env() { printf '%s' "$XDG_CONFIG_HOME/ayeaye/env"; }

# A systemd user session whose daemon-reload is broken, which is how a run gets
# interrupted in the middle of stage seven with everything before it finished.
_systemd_that_will_not_reload() {
  stub_command systemctl --exit 1
  stub_when systemctl '--user show-environment' --exit 0
  stub_when systemctl '--user daemon-reload' --exit 1 --stderr "Failed to reload daemon"
}

_systemd_that_works() {
  stub_command systemctl --exit 1
  stub_when systemctl '--user show-environment' --exit 0
  stub_when systemctl '--user daemon-reload' --exit 0
  stub_when systemctl '--user enable --now *' --exit 0
}

# ----------------------------------------------------- an interrupted run

test_a_run_that_stopped_partway_leaves_itself_open() {
  _hard_deps_present
  _systemd_that_will_not_reload
  run_install --defaults
  assert_ne 0 "$RUN_STATUS" "it really did stop"
  assert_file_contains "$(_state)" "run.completed=0" \
    "a run that did not reach the end is not a finished run"
  assert_file_contains "$(_state)" "stage.install=done" \
    "and everything it did finish is on record"
}

test_the_next_run_says_it_is_carrying_on() {
  _hard_deps_present
  _systemd_that_will_not_reload
  run_install --defaults
  _systemd_that_works
  run_install --defaults
  assert_status 0 "$RUN_STATUS"
  assert_contains "$RUN_STDOUT" "The last run stopped before it finished." \
    "somebody who was interrupted should be told what is happening"
}

test_the_next_run_does_not_redo_what_the_first_one_did() {
  _hard_deps_present
  _systemd_that_will_not_reload
  run_install --defaults
  assert_contains "$RUN_STDOUT" "wrote $(_env)"

  _systemd_that_works
  run_install --defaults
  assert_status 0 "$RUN_STATUS"
  assert_not_contains "$RUN_STDOUT" "wrote $(_env)" \
    "the settings were already written; writing them again is not resuming"
  assert_contains "$RUN_STDOUT" "already done" \
    "and it says so rather than silently skipping"
}

test_the_next_run_finishes_the_work_that_was_left() {
  _hard_deps_present
  _systemd_that_will_not_reload
  run_install --defaults
  assert_file_contains "$(_state)" "step.service.unit=failed" \
    "the service stage is the one that did not get done"
  assert_not_contains "$RUN_STDOUT" "bookmark:" "so the run never reached the end"

  _systemd_that_works
  run_install --defaults
  assert_status 0 "$RUN_STATUS"
  assert_stub_called_with systemctl "systemctl --user daemon-reload" \
    "the work that failed is tried again"
  assert_file_exists "$XDG_CONFIG_HOME/systemd/user/ayeaye.service"
  assert_contains "$RUN_STDOUT" "bookmark:" "and this time it reaches the end"
  assert_file_contains "$(_state)" "run.completed=1"
}

test_an_interrupted_run_does_not_ask_the_questions_again() {
  # The point of resuming: the answers given before the interruption are kept,
  # so nobody has to type them twice.
  _hard_deps_present
  _systemd_that_will_not_reload
  stdin_lines "10.1.2.3" "9911" "kept.example" ""
  run_install
  assert_ne 0 "$RUN_STATUS"

  _systemd_that_works
  run_install
  assert_status 0 "$RUN_STATUS"
  # Not "the prompt is absent from the output": a prompt is only ever written
  # to a terminal, so that assertion could not fail under a pipe. What proves
  # it is that the stage was kept rather than walked.
  assert_contains "$RUN_STDOUT" "Your choices (already done)" \
    "the questions were answered already, and it says so"
  assert_file_contains "$(_env)" "AYEAYE_PORT=9911"
  assert_contains "$RUN_STDOUT" "bookmark: http://10.1.2.3:9911/?token=" \
    "and the answers still drive the summary"
}

test_a_stopped_run_keeps_the_key_it_made() {
  _hard_deps_present
  _systemd_that_will_not_reload
  run_install --defaults
  local first
  first="$(cat "$(_token)")"
  assert_ne "" "$first"

  _systemd_that_works
  run_install --defaults
  assert_eq "$first" "$(cat "$(_token)")" \
    "an interruption must never cost somebody the bookmark on their phone"
}

test_a_run_interrupted_before_it_wrote_anything_starts_from_the_top() {
  # Stage two fails, so nothing after it ran. There is nothing to keep and
  # nothing to skip - and the second run must still ask the questions.
  require_host_command python3
  stub_real python3           # no tmux: the run stops in stage two
  run_install --defaults --no-systemd
  assert_ne 0 "$RUN_STATUS"
  assert_file_missing "$(_env)"

  stub_command tmux
  stdin_lines "10.1.2.3" "9911" "" ""
  run_install --no-systemd
  assert_status 0 "$RUN_STATUS"
  assert_file_contains "$(_env)" "AYEAYE_BIND=10.1.2.3" \
    "nothing was decided last time, so everything is asked this time"
}

# ------------------------------------------------------ a deliberate rerun

test_a_finished_run_is_walked_again_from_the_start() {
  _hard_deps_present
  run_install --defaults --no-systemd
  assert_file_contains "$(_state)" "run.completed=1"

  stdin_lines "n"
  run_install --no-systemd
  assert_status 0 "$RUN_STATUS"
  assert_contains "$RUN_STDOUT" "config exists at $(_env)" \
    "a rerun is how settings get changed, so it must offer to change them"
  assert_not_contains "$RUN_STDOUT" "The last run stopped before it finished."
  assert_file_contains "$(_state)" "run.count=2"
}

test_a_rerun_never_makes_a_new_key() {
  _hard_deps_present
  run_install --defaults --no-systemd
  local first
  first="$(cat "$(_token)")"
  assert_ne "" "$first"

  run_install --defaults --no-systemd
  assert_status 0 "$RUN_STATUS"
  assert_eq "$first" "$(cat "$(_token)")" \
    "re-running setup must never invalidate the bookmark already on the phone"
  assert_not_contains "$RUN_OUTPUT" "generated auth token" \
    "and it does not pretend to have made one"
  assert_contains "$RUN_STDOUT" "keeping the key you already have"
}

test_a_rerun_that_changes_one_answer_keeps_the_rest_of_the_file() {
  _hard_deps_present
  stdin_lines "10.1.2.3" "9911" "first.example" "https://ntfy.example/first"
  run_install --no-systemd
  printf '\n# something I added by hand\nAYEAYE_NAME=the-workshop\n' \
    >> "$(_env)"

  stdin_lines "y" "10.1.2.3" "9000" "first.example" "https://ntfy.example/first"
  run_install --no-systemd
  assert_status 0 "$RUN_STATUS"
  assert_file_contains "$(_env)" "AYEAYE_PORT=9000" "the changed answer changed"
  assert_file_contains "$(_env)" "AYEAYE_NAME=the-workshop" \
    "and everything else is still there"
  assert_file_contains "$(_env)" "# something I added by hand"
}

test_a_rerun_offers_back_what_was_chosen_last_time() {
  # Pressing return through a rerun means "leave it as it is". It used to mean
  # "put everything back to the factory defaults", which is the opposite, and
  # which nobody would have chosen on purpose.
  _hard_deps_present
  _answer_config_prompts "10.1.2.3" "9911" "one.example" "https://ntfy.example/t"
  # Naming an https address of your own is now asked about once - it is what
  # lets something that is not this computer reach ayeaye - so the answer
  # to that question belongs here too.
  pty_expect "may one.example reach ayeaye?" "y"
  pty_install --no-systemd
  assert_status 0 "$PTY_STATUS"

  pty_expect "rewrite it?" "y"
  _answer_config_prompts "" "" "" ""
  pty_expect "may one.example reach ayeaye?" "y"
  pty_install --no-systemd
  assert_status 0 "$PTY_STATUS"
  assert_contains "$PTY_TRANSCRIPT" "bind address [10.1.2.3]:"
  assert_contains "$PTY_TRANSCRIPT" "port [9911]:"
  local env_file
  env_file="$(_env)"
  assert_file_contains "$env_file" "AYEAYE_BIND=10.1.2.3"
  assert_file_contains "$env_file" "AYEAYE_PORT=9911"
  assert_file_contains "$env_file" "AYEAYE_ALLOWED_HOSTS=one.example"
  assert_file_contains "$env_file" "VOICE_NTFY_URL=https://ntfy.example/t"
}

# The four configuration questions, answered by name rather than by position so
# a misplaced answer cannot pass unnoticed.
_answer_config_prompts() {
  pty_expect "bind address" "$1"
  pty_expect "port [" "$2"
  pty_expect "allowed hosts" "$3"
  pty_expect "ntfy topic URL" "$4"
}

# ------------------------------------ what a resumed run must not throw away

test_a_resumed_run_still_knows_what_it_was_going_to_do() {
  # Stage five summarises what stage four decided. A resumed run skips stage
  # four, so a plan kept only in memory would empty itself - and stage five
  # would say "nothing needs to be installed or changed" immediately before
  # installing something.
  _hard_deps_present
  _systemd_that_will_not_reload
  stdin_lines "10.1.2.3" "9911" "" ""
  run_install
  assert_ne 0 "$RUN_STATUS"
  assert_contains "$RUN_STDOUT" "ayeaye will answer on 10.1.2.3:9911"

  _systemd_that_works
  run_install
  assert_status 0 "$RUN_STATUS"
  assert_contains "$RUN_STDOUT" "ayeaye will answer on 10.1.2.3:9911" \
    "the plan is what stage five shows, and it must survive being resumed"
  assert_not_contains "$RUN_STDOUT" "nothing needs to be installed or changed."
}

test_a_resumed_run_keeps_the_audit_trail_of_the_first_one() {
  # "Did that script do anything to my machine" has one place to look, and an
  # interruption must not empty it.
  _hard_deps_present
  _systemd_that_will_not_reload
  stdin_lines "0.0.0.0" "9911" "" ""
  run_install
  assert_ne 0 "$RUN_STATUS"
  local ledger="$XDG_STATE_HOME/ayeaye/setup-consent.log"
  assert_file_contains "$ledger" "expose	granted"

  _systemd_that_works
  run_install
  assert_status 0 "$RUN_STATUS"
  assert_file_contains "$ledger" "expose	granted" \
    "the decision was made, and it is still on the record"
}

test_a_deliberate_rerun_starts_the_audit_trail_over() {
  _hard_deps_present
  stdin_lines "0.0.0.0" "9911" "" ""
  run_install --no-systemd
  local ledger="$XDG_STATE_HOME/ayeaye/setup-consent.log"
  assert_file_contains "$ledger" "expose	granted"

  stdin_lines "n"
  run_install --no-systemd
  assert_status 0 "$RUN_STATUS"
  assert_file_not_contains "$ledger" "expose	granted" \
    "a new pass is a new set of decisions; last week's are not this run's"
}

test_forgetting_everything_and_starting_over_is_possible() {
  _hard_deps_present
  run_install --defaults --no-systemd
  assert_file_contains "$(_state)" "answer.port=8911"

  stdin_lines "n"
  run_install --no-systemd --fresh
  assert_status 0 "$RUN_STATUS"
  assert_file_not_contains "$(_state)" "answer.hosts=one.example"
  assert_file_contains "$(_state)" "run.count=1"
  assert_file_exists "$(_token)" "--fresh forgets the run, not the key"
  assert_file_exists "$(_env)" "and not the settings either"
}
