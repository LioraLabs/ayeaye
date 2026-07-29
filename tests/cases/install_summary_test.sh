# The last thing on screen: the closing summary and the advice under it.
#
# This is onboarding copy — the part of the installer a user actually reads —
# and the wizard rewrite is certain to touch it. Nothing else in this suite
# pins it, so a rewrite could quietly drop the instruction that makes the
# bookmark work and every other test would stay green.

_hard_deps_present() {
  require_host_command python3
  stub_command tmux
  stub_real python3
}

test_the_run_is_divided_into_named_sections() {
  _hard_deps_present
  run_install --defaults --no-systemd
  assert_contains "$RUN_STDOUT" "ayeaye setup"
  assert_contains "$RUN_STDOUT" "configuration"
  assert_contains "$RUN_STDOUT" "service"
  assert_contains "$RUN_STDOUT" "done"
}

test_a_clean_run_says_nothing_on_stderr() {
  # The probes shell out to tailscale, systemctl and python. Every one of them
  # is silenced today, and a wizard that lets probe noise through would be
  # invisible to the rest of this suite.
  _hard_deps_present
  stub_command_from_fixture tailscale tailscale/status-online
  run_install --defaults --no-systemd
  assert_status 0 "$RUN_STATUS"
  assert_eq "" "$RUN_STDERR" "a successful install is silent on stderr"
}

test_the_bookmark_instruction_explains_why_the_token_is_in_the_url() {
  _hard_deps_present
  run_install --defaults --no-systemd
  assert_contains "$RUN_STDOUT" "open the bookmark URL once on the phone; it sets a cookie and"
  assert_contains "$RUN_STDOUT" "redirects to /. Bookmark it and you never type the token again."
}

test_the_https_hint_names_the_command_that_provides_it() {
  # The mic needs a secure origin, so this line is the difference between a
  # working install and a phone that cannot record.
  _hard_deps_present
  run_install --defaults --no-systemd
  assert_contains "$RUN_STDOUT" "https for the phone (mic needs a secure origin):"
  assert_contains "$RUN_STDOUT" "tailscale serve --bg http://127.0.0.1:8911"
}

test_the_https_hint_names_the_address_that_was_chosen() {
  # The serve hint used to hardcode 127.0.0.1 while the bookmark above it used
  # the address the user actually chose, so for any bind but the default the
  # two lines contradicted each other and one of them was wrong. They agree
  # now.
  _hard_deps_present
  stdin_lines "10.1.2.3" "9000" "" ""
  run_install --no-systemd
  assert_contains "$RUN_STDOUT" "bookmark: http://10.1.2.3:9000/?token="
  assert_contains "$RUN_STDOUT" "tailscale serve --bg http://10.1.2.3:9000"
  assert_not_contains "$RUN_STDOUT" "tailscale serve --bg http://127.0.0.1:9000" \
    "the two lines must not disagree about where ayeaye is listening"
}

test_no_allowed_host_means_no_https_front_is_offered() {
  _hard_deps_present
  run_install --defaults --no-systemd
  assert_contains "$RUN_STDOUT" "bookmark: http://127.0.0.1:8911/?token=" "anchor"
  assert_not_contains "$RUN_STDOUT" "behind tailscale serve or a proxy"
}

test_the_first_allowed_host_becomes_the_https_front() {
  _hard_deps_present
  stdin_lines "" "" "front.example,second.example" ""
  run_install --no-systemd
  assert_contains "$RUN_STDOUT" "behind tailscale serve or a proxy: https://front.example/?token="
  assert_not_contains "$RUN_STDOUT" "https://second.example" \
    "only the first of the allowed hosts is offered as the front"
}

test_the_summary_names_the_config_file_it_wrote() {
  _hard_deps_present
  run_install --defaults --no-systemd
  assert_matches "$RUN_STDOUT" "config[[:space:]]*: "
  assert_contains "$RUN_STDOUT" "$XDG_CONFIG_HOME/ayeaye/env"
}

test_the_tier_line_says_it_was_probed_live() {
  # The wording matters more than it looks: it is what tells a user that
  # installing ffmpeg later needs no re-run.
  _hard_deps_present
  run_install --defaults --no-systemd
  assert_contains "$RUN_STDOUT" "(probed live; the app adapts at runtime)"
}
