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
  # The key travels in the URL exactly once, and the closing screen is the only
  # place that says why - and what to bookmark instead of the address with the
  # key in it.
  _hard_deps_present
  run_install --defaults --no-systemd
  assert_contains "$RUN_STDOUT" "signing in, once per device:"
  assert_contains "$RUN_STDOUT" "open that address once. It puts a key in the browser and sends you"
  assert_contains "$RUN_STDOUT" "to the page; bookmark what you land on and you never type the key"
  assert_contains "$RUN_STDOUT" "X-Voice-Token instead; the key itself is in $XDG_STATE_HOME/ayeaye/token"
}

test_a_configured_https_front_becomes_the_phone_bookmark() {
  # The phone cannot open a loopback address, so where a way in has really been
  # set up that is the address the closing screen leads with - and the local one
  # is still named, marked as being for a browser on this computer.
  #
  # The fourth answer is the one that matters: naming an https address is not
  # the same as agreeing that it may reach ayeaye, and until that question is
  # answered nothing has been set up for a phone at all.
  _hard_deps_present
  stdin_lines "" "front.example" "" "y"
  run_install --no-systemd
  assert_contains "$RUN_STDOUT" "bookmark: https://front.example/?token="
  assert_contains "$RUN_STDOUT" "in a browser on this computer: http://127.0.0.1:8911/?token="
}

test_naming_an_address_is_not_the_same_as_setting_one_up() {
  # The defect this pins: install.sh offers a tailscale name it noticed as the
  # default for the allowed-hosts question, so pressing return puts a name in
  # the settings file with no front end anywhere. A closing screen that read
  # that list as "here is your phone address" would invent one, on a run that
  # set nothing up and said so two screens earlier.
  _hard_deps_present
  stdin_lines "" "front.example" "" "n"
  run_install --no-systemd
  assert_file_contains "$XDG_CONFIG_HOME/ayeaye/env" "AYEAYE_ALLOWED_HOSTS=front.example" \
    "the name they typed is still theirs and is still written down"
  assert_not_contains "$RUN_STDOUT" "bookmark: https://front.example" \
    "but nothing was set up, so there is no address to lead with"
  assert_not_contains "$RUN_STDOUT" "open that one on your phone." \
    "and nobody is told to open one"
  assert_contains "$RUN_STDOUT" "setup did not put it there and cannot see it"
}

test_the_two_halves_of_the_bookmark_block_agree_about_the_port() {
  # The https line and the local line used to be built from different values,
  # so for any port but the default the two contradicted each other and one of
  # them was wrong. They agree now.
  _hard_deps_present
  stdin_lines "9000" "front.example" "" "y"
  run_install --no-systemd
  assert_contains "$RUN_STDOUT" "bookmark: https://front.example/?token="
  assert_contains "$RUN_STDOUT" "in a browser on this computer: http://127.0.0.1:9000/?token="
  assert_not_contains "$RUN_STDOUT" "http://127.0.0.1:8911/" \
    "the two lines must not disagree about where ayeaye is listening"
}

test_no_allowed_host_means_no_https_front_is_offered() {
  _hard_deps_present
  run_install --defaults --no-systemd
  assert_contains "$RUN_STDOUT" "bookmark: http://127.0.0.1:8911/?token=" "anchor"
  assert_not_contains "$RUN_STDOUT" "open that one on your phone." \
    "with nothing in front of it there is no address a phone could open"
  assert_contains "$RUN_STDOUT" "cannot reach ayeaye yet. Run ./install.sh again and pick a" \
    "and the closing screen says so rather than printing a loopback address at a phone"
}

test_the_first_allowed_host_becomes_the_https_front() {
  _hard_deps_present
  stdin_lines "" "front.example,second.example" "" "y"
  run_install --no-systemd
  assert_contains "$RUN_STDOUT" "bookmark: https://front.example/?token="
  assert_not_contains "$RUN_STDOUT" "https://second.example" \
    "only the first of the allowed hosts is offered as the front"
}

test_the_summary_names_the_config_file_it_wrote() {
  _hard_deps_present
  run_install --defaults --no-systemd
  assert_matches "$RUN_STDOUT" "config[[:space:]]*: "
  assert_contains "$RUN_STDOUT" "$XDG_CONFIG_HOME/ayeaye/env"
}

test_the_run_says_that_software_added_later_needs_no_re_run() {
  # The closing screen used to carry this as "(probed live; the app adapts at
  # runtime)" beside a tier line that no longer exists. The promise is the part
  # that mattered - install ffmpeg next month and nothing has to be re-run -
  # and it is now made once, beside the checklist of what is missing.
  _hard_deps_present
  run_install --defaults --no-systemd
  assert_contains "$RUN_STDOUT" "ayeaye notices each of them the moment"
  assert_contains "$RUN_STDOUT" "it is installed, so none of this has to be decided now."
}

test_the_closing_screen_says_what_works_here() {
  # One line, describing what this run set up, and it is the first thing on
  # the last screen. Typing always works; anything more had to be chosen.
  _hard_deps_present
  run_install --defaults --no-systemd
  assert_contains "$RUN_STDOUT" "what works: typing"
}
