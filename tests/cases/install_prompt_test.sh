# How install.sh asks, and what it does with the answer.
#
# `ask` renders "<prompt> [<default>]: " and treats empty input as the default;
# `confirm` layers a yes/no reading on top of it. Both are about to be replaced
# by a multi-stage wizard, so their current semantics are pinned here in full.

_hard_deps_present() {
  require_host_command python3
  stub_command tmux
  stub_real python3
}

# A config that is obviously not the default, so "kept" and "rewritten" can
# never be confused.
_existing_config() {
  mkdir -p "$XDG_CONFIG_HOME/ayeaye"
  cat > "$XDG_CONFIG_HOME/ayeaye/env" <<'ENV'
AYEAYE_BIND=10.9.8.7
AYEAYE_PORT=7777
AYEAYE_ALLOWED_HOSTS=kept.example
VOICE_NTFY_URL=https://ntfy.example/kept
ENV
}

# The three configuration questions, answered by name rather than by position so
# a misplaced answer cannot pass unnoticed. The address ayeaye answers on used
# to be the first of them and is no longer asked at all: lib/steps/50-access.sh
# owns it now, so there is nothing here to type at it.
_answer_config_prompts() {
  pty_expect "port [" "$1"
  pty_expect "allowed hosts" "$2"
  pty_expect "ntfy topic URL" "$3"
}

test_the_three_questions_are_asked_in_order_with_their_defaults() {
  _hard_deps_present
  pty_answers "" "" ""
  pty_install --no-systemd
  assert_contains "$PTY_TRANSCRIPT" "port [8911]:"
  assert_contains "$PTY_TRANSCRIPT" "allowed hosts"
  assert_contains "$PTY_TRANSCRIPT" "ntfy topic URL for push notifications (empty disables) [empty]:"

  local port_line hosts_line ntfy_line
  port_line="$(printf '%s\n' "$PTY_TRANSCRIPT" | grep -n "port \[8911\]" | head -1 | cut -d: -f1)"
  hosts_line="$(printf '%s\n' "$PTY_TRANSCRIPT" | grep -n "allowed hosts" | head -1 | cut -d: -f1)"
  ntfy_line="$(printf '%s\n' "$PTY_TRANSCRIPT" | grep -n "ntfy topic URL" | head -1 | cut -d: -f1)"
  [ "$port_line" -lt "$hosts_line" ] || fail "port must be asked before allowed hosts:
$PTY_TRANSCRIPT"
  [ "$hosts_line" -lt "$ntfy_line" ] || fail "allowed hosts must be asked before ntfy:
$PTY_TRANSCRIPT"
}

test_an_empty_answer_takes_the_offered_default() {
  _hard_deps_present
  pty_answers "" "" ""
  pty_install --no-systemd
  assert_status 0 "$PTY_STATUS"
  assert_file_contains "$XDG_CONFIG_HOME/ayeaye/env" "AYEAYE_BIND=127.0.0.1"
  assert_file_contains "$XDG_CONFIG_HOME/ayeaye/env" "AYEAYE_PORT=8911"
  assert_file_contains "$XDG_CONFIG_HOME/ayeaye/env" "AYEAYE_ALLOWED_HOSTS="
  assert_file_contains "$XDG_CONFIG_HOME/ayeaye/env" "VOICE_NTFY_URL="
}

test_a_typed_answer_overrides_the_default() {
  _hard_deps_present
  _answer_config_prompts "9000" "box.example,other.example" "https://ntfy.sh/mine"
  # Naming an https address of your own is now asked about once - it is what
  # lets something that is not this computer reach ayeaye - so the answer
  # to that question belongs here too.
  pty_expect "may box.example,other.example reach ayeaye?" "y"
  pty_install --no-systemd
  assert_status 0 "$PTY_STATUS"
  assert_file_contains "$XDG_CONFIG_HOME/ayeaye/env" "AYEAYE_PORT=9000"
  assert_file_contains "$XDG_CONFIG_HOME/ayeaye/env" "AYEAYE_ALLOWED_HOSTS=box.example,other.example"
  assert_file_contains "$XDG_CONFIG_HOME/ayeaye/env" "VOICE_NTFY_URL=https://ntfy.sh/mine"
  assert_file_contains "$XDG_CONFIG_HOME/ayeaye/env" "AYEAYE_BIND=127.0.0.1" \
    "the address is not one of the questions, so no answer can move it off this computer"
}

test_answers_are_reflected_in_the_bookmark_url() {
  # The address is no longer promptable, so the one in the settings file is
  # what the closing lines have to be read out of. The port still is, and both
  # halves of the bookmark block have to agree about it.
  _hard_deps_present
  mkdir -p "$XDG_CONFIG_HOME/ayeaye"
  printf 'AYEAYE_BIND=192.168.1.10\n' > "$XDG_CONFIG_HOME/ayeaye/env"
  pty_expect "rewrite it?" "y"
  _answer_config_prompts "9000" "box.example,other.example" ""
  pty_expect "may box.example,other.example reach ayeaye?" "y"
  pty_install --no-systemd
  assert_contains "$PTY_TRANSCRIPT" "bookmark: https://box.example/?token=" \
    "only the first allowed host is offered as the https front"
  assert_contains "$PTY_TRANSCRIPT" \
    "in a browser on this computer: http://192.168.1.10:9000/?token="
}

test_a_notification_url_shows_up_in_what_works() {
  _hard_deps_present
  _answer_config_prompts "" "" "https://ntfy.sh/mine"
  pty_install --no-systemd
  assert_matches "$PTY_TRANSCRIPT" \
    "what works: .*, with notifications to your phone"
}

test_defaults_mode_asks_nothing_and_reads_no_stdin() {
  _hard_deps_present
  stdin_lines "9999" "should-not-be-read" "should-not-be-read"
  run_install --defaults --no-systemd
  assert_status 0 "$RUN_STATUS"
  assert_not_contains "$RUN_OUTPUT" "port [8911]:" "--defaults must not prompt"
  assert_file_contains "$XDG_CONFIG_HOME/ayeaye/env" "AYEAYE_BIND=127.0.0.1"
  assert_file_contains "$XDG_CONFIG_HOME/ayeaye/env" "AYEAYE_PORT=8911" \
    "queued input must be ignored entirely, not consumed as answers"
}

test_a_piped_run_without_answers_still_takes_every_default() {
  # Documented in the script's own header: `yes '' | ./install.sh` works
  # because a failed read leaves REPLY empty and empty means default.
  _hard_deps_present
  run_install --no-systemd
  assert_status 0 "$RUN_STATUS"
  assert_file_contains "$XDG_CONFIG_HOME/ayeaye/env" "AYEAYE_BIND=127.0.0.1"
  assert_file_contains "$XDG_CONFIG_HOME/ayeaye/env" "AYEAYE_PORT=8911"
}

test_piped_answers_are_consumed_in_prompt_order() {
  _hard_deps_present
  stdin_lines "9999" "piped.example" ""
  run_install --no-systemd
  assert_status 0 "$RUN_STATUS"
  assert_file_contains "$XDG_CONFIG_HOME/ayeaye/env" "AYEAYE_PORT=9999"
  assert_file_contains "$XDG_CONFIG_HOME/ayeaye/env" "AYEAYE_ALLOWED_HOSTS=piped.example"
}

# ------------------------------------------------------------------- confirm

test_an_existing_config_is_announced_before_the_question() {
  _hard_deps_present
  _existing_config
  pty_expect "rewrite it?" "n"
  pty_install --no-systemd
  assert_contains "$PTY_TRANSCRIPT" "config exists at $XDG_CONFIG_HOME/ayeaye/env"
  assert_contains "$PTY_TRANSCRIPT" "rewrite it? (n keeps the current file) [n]:"
}

test_confirm_reads_y_upper_y_yes_and_upper_yes_as_yes() {
  # Saying yes opens the questions; it is the answers that change anything.
  # Each question now offers what is already configured, so answering yes and
  # then pressing return deliberately leaves the file as it was - this used to
  # reset every setting to the factory defaults, which is not what pressing
  # return looks like it means. test_a_typed_answer_overrides_the_default is
  # where changing them is pinned.
  _hard_deps_present
  local answer
  for answer in y Y yes YES; do
    _existing_config
    pty_expect "rewrite it?" "$answer"
    _answer_config_prompts "9000" "" ""
    # The config kept from before names an https address, which setup asks
    # about once before it lets anything through it.
    pty_expect "may kept.example reach ayeaye?" "y"
    pty_install --no-systemd
    assert_status 0 "$PTY_STATUS"
    assert_file_contains "$XDG_CONFIG_HOME/ayeaye/env" "AYEAYE_PORT=9000" \
      "\"$answer\" must be taken as yes and let the answers through"
    assert_file_contains "$XDG_CONFIG_HOME/ayeaye/env" "AYEAYE_BIND=10.9.8.7" \
      "and the address nobody was asked about is still the one that was there"
  done
}

test_the_questions_offer_back_what_is_already_configured() {
  _hard_deps_present
  _existing_config
  pty_expect "rewrite it?" "y"
  _answer_config_prompts "" "" ""
  pty_expect "may kept.example reach ayeaye?" "y"
  pty_install --no-systemd
  assert_contains "$PTY_TRANSCRIPT" "port [7777]:"
  assert_contains "$PTY_TRANSCRIPT" "allowed hosts (your https front) [kept.example]:"
  assert_contains "$PTY_TRANSCRIPT" \
    "ntfy topic URL for push notifications (empty disables) [https://ntfy.example/kept]:"
}

test_confirm_reads_anything_else_as_no() {
  _hard_deps_present
  local answer
  # The empty answer is the interesting one: it falls through to the "n"
  # default rather than being re-asked.
  for answer in n N no NO "" maybe yeah Yes.; do
    _existing_config
    pty_expect "rewrite it?" "$answer"
    pty_install --no-systemd
    assert_status 0 "$PTY_STATUS"
    assert_file_contains "$XDG_CONFIG_HOME/ayeaye/env" "AYEAYE_PORT=7777" \
      "\"$answer\" must be taken as no and keep the config"
    assert_contains "$PTY_TRANSCRIPT" "keeping existing config"
  done
}

test_keeping_a_config_asks_nothing_further() {
  _hard_deps_present
  _existing_config
  pty_expect "rewrite it?" "n"
  pty_install --no-systemd
  assert_not_contains "$PTY_TRANSCRIPT" "port [" \
    "declining the rewrite must skip the three configuration questions"
}

test_a_kept_config_still_drives_the_summary() {
  _hard_deps_present
  _existing_config
  pty_expect "rewrite it?" "n"
  pty_install --no-systemd
  assert_contains "$PTY_TRANSCRIPT" "https://kept.example/?token=" \
    "the summary is read back out of the kept file, not out of the defaults"
  assert_contains "$PTY_TRANSCRIPT" "bookmark: http://10.9.8.7:7777/?token="
  assert_contains "$PTY_TRANSCRIPT" "setup did not put it there and cannot see it" \
    "a name in a kept settings file is not a way in this run set up"
  assert_contains "$PTY_TRANSCRIPT" "with notifications to your phone" \
    "the kept ntfy URL counts towards what works here"
}

test_defaults_mode_keeps_an_existing_config_without_asking() {
  _hard_deps_present
  _existing_config
  run_install --defaults --no-systemd
  assert_status 0 "$RUN_STATUS"
  assert_contains "$RUN_STDOUT" "keeping existing config"
  assert_file_contains "$XDG_CONFIG_HOME/ayeaye/env" "AYEAYE_PORT=7777"
  assert_not_contains "$RUN_OUTPUT" "rewrite it?"
}

test_an_empty_config_file_is_not_treated_as_an_existing_one() {
  _hard_deps_present
  mkdir -p "$XDG_CONFIG_HOME/ayeaye"
  : > "$XDG_CONFIG_HOME/ayeaye/env"
  run_install --defaults --no-systemd
  assert_status 0 "$RUN_STATUS"
  assert_not_contains "$RUN_OUTPUT" "config exists at"
  assert_file_contains "$XDG_CONFIG_HOME/ayeaye/env" "AYEAYE_PORT=8911" \
    "a zero-byte config is rewritten without asking"
}
