# The service half of install.sh: whether a systemd user session is detected,
# what unit gets written, and exactly which systemctl commands would run.
#
# No unit is ever installed and no service is ever started by these tests. The
# assertions are made against the argv every stub records, which is the whole
# reason the stub mechanism exists.

_hard_deps_present() {
  require_host_command python3
  stub_command tmux
  stub_real python3
}

# A host with a live systemd user session. The detection probe is
# `systemctl --user show-environment`, so that is what has to succeed.
_systemd_session() {
  stub_command systemctl --exit 1 --stderr "unknown systemctl invocation"
  stub_when systemctl '--user show-environment' --exit 0
  stub_when systemctl '--user daemon-reload' --exit 0
  stub_when systemctl '--user enable --now *' --exit 0
}

_unit_file() {
  printf '%s' "$XDG_CONFIG_HOME/systemd/user/ayeaye.service"
}

# ------------------------------------------------------------- detection

test_a_live_user_session_is_detected_through_show_environment() {
  _hard_deps_present
  _systemd_session
  run_install --defaults
  assert_status 0 "$RUN_STATUS"
  assert_stub_called_with systemctl "systemctl --user show-environment"
  assert_contains "$RUN_STDOUT" "installed $(_unit_file)"
}

test_systemctl_present_without_a_user_session_is_not_enough() {
  _hard_deps_present
  stub_command systemctl --exit 1
  run_install --defaults
  assert_status 0 "$RUN_STATUS"
  assert_contains "$RUN_STDOUT" "no systemd user session detected"
  assert_file_missing "$(_unit_file)" "a failed probe must not install a unit"
  assert_stub_called_with systemctl "systemctl --user show-environment"
  assert_not_contains "$(stub_calls systemctl)" "daemon-reload"
}

test_no_systemctl_at_all_falls_back_to_the_manual_command() {
  _hard_deps_present
  run_install --defaults
  assert_status 0 "$RUN_STATUS"
  assert_contains "$RUN_STDOUT" "no systemd user session detected"
  assert_contains "$RUN_STDOUT" "run the server with: $REPO_ROOT/bin/ayeaye"
  assert_contains "$RUN_STDOUT" "(it reads $XDG_CONFIG_HOME/ayeaye/env by itself)"
  assert_matches "$RUN_STDOUT" "logs[[:space:]]*: stderr of "
  assert_contains "$RUN_STDOUT" "stderr of $REPO_ROOT/bin/ayeaye"
}

test_no_systemd_installs_and_starts_nothing() {
  # --no-systemd used to mean "do not even ask", because asking and acting were
  # the same piece of code. The platform layer now answers "what is the service
  # manager here" for the machine report whatever the flag says, and the flag
  # means what its name says: nothing is installed and nothing is started.
  _hard_deps_present
  _systemd_session
  run_install --defaults --no-systemd
  assert_status 0 "$RUN_STATUS"
  assert_contains "$RUN_STDOUT" "skipping systemd (--no-systemd)"
  assert_file_missing "$(_unit_file)" "no unit may be written"
  assert_not_contains "$(stub_calls systemctl)" "daemon-reload" "nothing is loaded"
  assert_not_contains "$(stub_calls systemctl)" "enable" "and nothing is started"
  assert_contains "$RUN_STDOUT" "run the server with: $REPO_ROOT/bin/ayeaye"
}

# ---------------------------------------------------------- the unit file

test_the_unit_is_written_from_the_template_with_both_paths_filled_in() {
  _hard_deps_present
  _systemd_session
  run_install --defaults
  local unit
  unit="$(_unit_file)"
  assert_file_exists "$unit"
  assert_file_contains "$unit" "ExecStart=$REPO_ROOT/bin/ayeaye"
  assert_file_contains "$unit" "EnvironmentFile=$XDG_CONFIG_HOME/ayeaye/env"
  assert_file_not_contains "$unit" "@REPO@"
  assert_file_not_contains "$unit" "@ENV_FILE@"
  # Nothing else about the unit's contents is asserted here: the rest comes from
  # systemd/user/ayeaye.service.template, and editing that file is not an
  # install.sh regression.
}

test_no_placeholder_survives_in_the_unit() {
  _hard_deps_present
  _systemd_session
  run_install --defaults
  local leftovers
  assert_file_exists "$(_unit_file)" \
    "a run that installed no unit has no placeholders in it either"
  leftovers="$(grep -o '@[A-Z_]*@' "$(_unit_file)" || true)"
  assert_eq "" "$leftovers" "an unsubstituted placeholder would install a broken unit"
}

test_the_unit_directory_is_created_when_missing() {
  _hard_deps_present
  _systemd_session
  assert_file_missing "$XDG_CONFIG_HOME/systemd/user"
  run_install --defaults
  assert_file_exists "$XDG_CONFIG_HOME/systemd/user"
}

test_the_unit_is_regenerated_on_every_run_and_says_so() {
  _hard_deps_present
  _systemd_session
  run_install --defaults
  printf 'hand-edited nonsense\n' > "$(_unit_file)"
  run_install --defaults
  assert_file_not_contains "$(_unit_file)" "hand-edited nonsense" \
    "the unit is a build artefact, not a config file"
  assert_contains "$RUN_STDOUT" "(regenerated on every run; put settings in"
}

test_the_daemon_is_reloaded_after_the_unit_is_written() {
  _hard_deps_present
  _systemd_session
  run_install --defaults
  assert_stub_called_with systemctl "systemctl --user daemon-reload"
}

# ------------------------------------------------------- enabling the unit

test_defaults_mode_never_starts_the_service() {
  _hard_deps_present
  _systemd_session
  run_install --defaults
  assert_not_contains "$(stub_calls systemctl)" "enable" \
    "an unattended run must not start a service behind the user's back"
  assert_contains "$RUN_STDOUT" "start it with: systemctl --user enable --now ayeaye"
  assert_not_contains "$RUN_OUTPUT" "enable and start it now?"
}

test_confirming_the_prompt_enables_and_starts_the_unit() {
  _hard_deps_present
  _systemd_session
  pty_answers "" "" "" ""          # the four configuration questions
  pty_expect "enable and start it now?" "y"
  pty_install
  assert_status 0 "$PTY_STATUS"
  assert_stub_called_with systemctl "systemctl --user enable --now ayeaye.service"
  assert_contains "$PTY_TRANSCRIPT" "started. status: systemctl --user status ayeaye"
}

test_declining_the_prompt_leaves_the_service_alone() {
  _hard_deps_present
  _systemd_session
  pty_answers "" "" "" ""
  pty_expect "enable and start it now?" "n"
  pty_install
  assert_status 0 "$PTY_STATUS"
  assert_not_contains "$(stub_calls systemctl)" "enable"
  assert_contains "$PTY_TRANSCRIPT" "start it with: systemctl --user enable --now ayeaye"
  assert_stub_called_with systemctl "systemctl --user daemon-reload" \
    "declining to start it must still leave the unit installed and loaded"
}

test_the_start_prompt_defaults_to_yes() {
  _hard_deps_present
  _systemd_session
  pty_answers "" "" "" ""
  pty_expect "enable and start it now?" ""
  pty_install
  assert_contains "$PTY_TRANSCRIPT" "enable and start it now? [y]:"
  assert_stub_called_with systemctl "systemctl --user enable --now ayeaye.service" \
    "an empty answer takes the y default"
}

test_the_summary_points_at_the_journal_when_a_unit_was_installed() {
  _hard_deps_present
  _systemd_session
  run_install --defaults
  assert_matches "$RUN_STDOUT" "logs[[:space:]]*: journalctl --user -u ayeaye -f"
}

# ------------------------------------------------------------ linger hint

test_a_user_without_linger_is_told_to_enable_it() {
  _hard_deps_present
  _systemd_session
  stub_command loginctl --stdout "no"
  run_install --defaults
  assert_stub_called_with loginctl "loginctl show-user $USER -P Linger"
  assert_contains "$RUN_STDOUT" "recommended: loginctl enable-linger $USER"
  assert_contains "$RUN_STDOUT" "(without it, user services stop when your last session ends)"
}

test_a_user_with_linger_is_not_nagged() {
  _hard_deps_present
  _systemd_session
  stub_command loginctl --stdout "yes"
  run_install --defaults
  # Without the anchor this cannot tell "linger is already on" from "linger was
  # never checked", which is the mistake a rewrite would actually make.
  assert_stub_called_with loginctl "loginctl show-user" "anchor: linger really was checked"
  assert_contains "$RUN_STDOUT" "installed $(_unit_file)"
  assert_not_contains "$RUN_STDOUT" "enable-linger"
}

test_no_loginctl_means_no_linger_hint() {
  _hard_deps_present
  _systemd_session
  run_install --defaults
  assert_contains "$RUN_STDOUT" "installed $(_unit_file)"
  assert_not_contains "$RUN_STDOUT" "enable-linger"
}

test_a_failing_loginctl_is_read_as_no_linger() {
  _hard_deps_present
  _systemd_session
  stub_command loginctl --exit 1 --stderr "Failed to get user: no such user"
  run_install --defaults
  assert_contains "$RUN_STDOUT" "recommended: loginctl enable-linger" \
    "an unreadable linger state is treated as not lingering"
  assert_not_contains "$RUN_STDERR" "Failed to get user" "the probe's noise stays hidden"
}

test_an_unset_user_variable_is_survived_at_the_linger_hint() {
  # This used to end the run. install.sh runs under `set -u` and read $USER
  # bare, twice, at the very end: a host that does not export it - which is
  # every container - lost the whole summary after the unit was already
  # installed. It now reads ${USER:-} and falls back to `id -un`, so the hint
  # still names a real user.
  _hard_deps_present
  _systemd_session
  stub_command loginctl --stdout "no"
  unset USER
  run_install --defaults
  assert_status 0 "$RUN_STATUS" "a host without USER exported is an ordinary host"
  assert_not_contains "$RUN_STDERR" "unbound variable"
  assert_matches "$RUN_STDOUT" "recommended: loginctl enable-linger .+" \
    "and the hint still names somebody to enable it for"
  assert_matches "$RUN_STDOUT" "logs[[:space:]]*: journalctl" \
    "anchor: it ran all the way to the summary"
}

test_an_unset_user_variable_is_harmless_without_loginctl() {
  # The same run is fine when loginctl is absent, which is why this has gone
  # unnoticed: the fatal line is only reached through the linger branch.
  _hard_deps_present
  _systemd_session
  unset USER
  run_install --defaults
  assert_status 0 "$RUN_STATUS"
  assert_matches "$RUN_STDOUT" "logs[[:space:]]*: journalctl" \
    "anchor: it ran all the way to the summary, not just far enough to not crash"
}

test_a_failing_daemon_reload_stops_the_run_after_the_unit_is_written() {
  # A systemd that refuses to reload cannot be asked to start anything, so the
  # run stops there rather than promising a service it could not install. The
  # unit is on disk and the state file records how far it got, so running setup
  # again picks up from exactly here - which wizard_resume_test.sh pins.
  # systemctl's own words still reach stderr, because at the moment something
  # fails they are worth more to the reader than any paraphrase.
  _hard_deps_present
  stub_command systemctl --exit 1
  stub_when systemctl '--user show-environment' --exit 0
  stub_when systemctl '--user daemon-reload' --exit 1 --stderr "Failed to reload daemon"
  run_install --defaults
  assert_ne 0 "$RUN_STATUS"
  assert_file_exists "$(_unit_file)" "the unit was already written when it gave up"
  assert_contains "$RUN_STDERR" "Failed to reload daemon"
  assert_not_contains "$RUN_STDOUT" "bookmark:" "the summary is never reached"
}

test_the_repo_path_may_contain_a_space() {
  # macOS paths routinely do, and the unit is rendered with sed, where an
  # unquoted expansion would split the path.
  _hard_deps_present
  _systemd_session
  local repo="$TEST_TMPDIR/my repo"
  mkdir -p "$repo"
  cp "$REPO_ROOT/install.sh" "$REPO_ROOT/env.template" "$repo/"
  cp -R "$REPO_ROOT/lib" "$repo/lib"
  mkdir -p "$repo/systemd/user"
  cp "$REPO_ROOT/systemd/user/ayeaye.service.template" "$repo/systemd/user/"

  run_script "$repo/install.sh" --defaults
  assert_status 0 "$RUN_STATUS"
  assert_contains "$RUN_STDOUT" "repo: $repo"
  assert_file_contains "$(_unit_file)" "ExecStart=$repo/bin/ayeaye"
}
