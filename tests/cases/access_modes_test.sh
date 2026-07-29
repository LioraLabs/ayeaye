# Choosing how you reach ayeaye: what is offered, what is not, and what an
# unattended run is allowed to decide on somebody's behalf.
#
# The whole installer is driven here rather than a step in isolation, because
# the properties worth pinning are about the conversation: that a run with
# nobody watching never opens anything, that a menu is only shown when there is
# something on it, and that saying no leaves the machine on this computer only.

_hard_deps_present() {
  require_host_command python3
  stub_command tmux
  stub_real python3
}

_tailscale_online() {
  stub_script tailscale <<SH
case "\$*" in
  "status --json") cat "$FIXTURES_DIR/tailscale/status-online" ;;
  "serve status") echo "https://my-box.tail1a2b3c.ts.net (tailnet only)" ;;
  *) exit 0 ;;
esac
SH
}

_env() { printf '%s' "$XDG_CONFIG_HOME/ayeaye/env"; }
_ledger() { printf '%s' "$XDG_STATE_HOME/ayeaye/setup-consent.log"; }
_state() { printf '%s' "$XDG_STATE_HOME/ayeaye/setup-state"; }

# The four questions install.sh asks before this one.
_answer_config_prompts() {
  pty_expect "bind address" "$1"
  pty_expect "port [" "$2"
  pty_expect "allowed hosts" "$3"
  pty_expect "ntfy topic URL" "$4"
}

# ------------------------------------------------------- nobody watching

test_an_unattended_run_keeps_ayeaye_on_this_computer() {
  _hard_deps_present
  _tailscale_online
  run_install --defaults --no-systemd
  assert_status 0 "$RUN_STATUS"
  assert_file_contains "$(_env)" "AYEAYE_BIND=127.0.0.1"
  assert_contains "$RUN_STDOUT" "ayeaye will answer on this computer only"
  assert_not_contains "$RUN_STDOUT" "which one?" "an unattended run asks nothing"
  assert_not_contains "$(stub_calls tailscale)" "serve --bg" \
    "nothing was put in front of ayeaye"
}

test_an_unattended_run_records_that_it_refused_to_open_anything() {
  _hard_deps_present
  _tailscale_online
  run_install --defaults --no-systemd
  assert_file_contains "$(_ledger)" \
    "expose	refused	a run with nobody watching never opens ayeaye up"
}

test_blanket_yes_is_not_a_way_to_reach_a_network_mode() {
  # --yes exists so automation can install packages. Exposure is one of the
  # three kinds it is not allowed to answer, and this is the end-to-end proof.
  _hard_deps_present
  _tailscale_online
  run_install --defaults --yes --no-systemd
  assert_status 0 "$RUN_STATUS"
  assert_file_contains "$(_env)" "AYEAYE_BIND=127.0.0.1"
  assert_not_contains "$(cat "$(_ledger)")" "expose	granted"
  assert_not_contains "$(stub_calls tailscale)" "serve --bg"
}

# --------------------------------------------------- nothing worth asking

test_a_machine_that_can_offer_nothing_says_so_instead_of_asking() {
  # Four answers of which three cannot be carried out is worse than one
  # paragraph. The same judgement lib/steps/60-packages.sh makes.
  _hard_deps_present
  pty_answers "" "" "" ""
  pty_install --no-systemd
  assert_status 0 "$PTY_STATUS"
  assert_not_contains "$PTY_TRANSCRIPT" "which one?"
  assert_contains "$PTY_TRANSCRIPT" "computer has no way to make one yet"
  assert_contains "$PTY_TRANSCRIPT" "Install tailscale"
  assert_file_contains "$(_env)" "AYEAYE_BIND=127.0.0.1"
}

test_an_address_the_person_already_named_is_treated_as_their_answer() {
  # Somebody who types their own https address into the question a page
  # earlier has told setup what is in front of this computer. Asking again
  # would be asking the same thing twice.
  _hard_deps_present
  _answer_config_prompts "" "" "ayeaye.example.com" ""
  pty_expect "may ayeaye.example.com reach ayeaye?" "y"
  pty_install --no-systemd
  assert_status 0 "$PTY_STATUS"
  assert_not_contains "$PTY_TRANSCRIPT" "which one?" \
    "there is nothing to choose between, so there is no menu"
  assert_contains "$PTY_TRANSCRIPT" "You said ayeaye.example.com is the https address"
  assert_file_contains "$(_env)" "AYEAYE_ALLOWED_HOSTS=ayeaye.example.com"
  assert_file_contains "$XDG_CONFIG_HOME/ayeaye/reverse-proxy.nginx" \
    "proxy_pass http://127.0.0.1:"
  assert_file_contains "$(_ledger)" "expose	granted	may ayeaye.example.com reach ayeaye?"
}

# ------------------------------------------------------------- the menu

test_a_machine_with_tailscale_is_offered_all_four_ways() {
  _hard_deps_present
  _tailscale_online
  _answer_config_prompts "" "" "" ""
  pty_expect "which one?" "2"
  pty_install --no-systemd
  assert_status 0 "$PTY_STATUS"
  assert_contains "$PTY_TRANSCRIPT" "How will your phone reach ayeaye?"
  assert_contains "$PTY_TRANSCRIPT" "1  tailscale"
  assert_contains "$PTY_TRANSCRIPT" "2  this computer only"
  assert_contains "$PTY_TRANSCRIPT" "3  your home network"
  assert_contains "$PTY_TRANSCRIPT" "4  the https address you already have"
  assert_contains "$PTY_TRANSCRIPT" "which one? (1, 2, 3 or 4) [1]:" \
    "tailscale is what it offers when tailscale is possible"
}

test_the_menu_says_what_is_at_stake_before_it_asks() {
  _hard_deps_present
  _tailscale_online
  _answer_config_prompts "" "" "" ""
  pty_expect "which one?" "2"
  pty_install --no-systemd
  assert_contains "$PTY_TRANSCRIPT" \
    "whoever opens it can do anything on this computer"
  assert_contains "$PTY_TRANSCRIPT" "will not put it on the open internet"
}

test_choosing_this_computer_only_says_it_is_not_phone_access() {
  _hard_deps_present
  _tailscale_online
  _answer_config_prompts "" "" "" ""
  pty_expect "which one?" "2"
  pty_install --no-systemd
  assert_contains "$PTY_TRANSCRIPT" "it is not how"
  assert_contains "$PTY_TRANSCRIPT" "your phone will not find it"
  assert_file_contains "$(_env)" "AYEAYE_ALLOWED_HOSTS="
  assert_not_contains "$(stub_calls tailscale)" "serve --bg"
}

test_choosing_tailscale_and_then_saying_no_leaves_nothing_open() {
  # The menu is not the permission. Exposure is asked as its own question and
  # answering no lands on this computer only, with nothing run.
  _hard_deps_present
  _tailscale_online
  _answer_config_prompts "" "" "" ""
  pty_expect "which one?" "1"
  pty_expect "may your own tailscale devices reach ayeaye?" "n"
  pty_install --no-systemd
  assert_status 0 "$PTY_STATUS"
  assert_contains "$PTY_TRANSCRIPT" "Nothing has been opened."
  assert_not_contains "$(stub_calls tailscale)" "serve --bg"
  assert_file_contains "$(_env)" "AYEAYE_ALLOWED_HOSTS="
  assert_file_contains "$(_ledger)" "expose	refused"
}

test_choosing_tailscale_and_saying_yes_sets_it_up() {
  _hard_deps_present
  _tailscale_online
  _answer_config_prompts "" "" "" ""
  pty_expect "which one?" "1"
  pty_expect "may your own tailscale devices reach ayeaye?" "y"
  pty_install --no-systemd
  assert_status 0 "$PTY_STATUS"
  assert_stub_called_with tailscale "tailscale serve --bg http://127.0.0.1:8911"
  assert_file_contains "$(_env)" "AYEAYE_ALLOWED_HOSTS=my-box.tail1a2b3c.ts.net"
  assert_file_contains "$(_env)" "AYEAYE_BIND=127.0.0.1"
  assert_file_contains "$(_ledger)" "expose	granted	may your own tailscale devices reach ayeaye?"
}

test_an_answer_that_is_not_one_of_the_four_is_asked_again() {
  _hard_deps_present
  _tailscale_online
  _answer_config_prompts "" "" "" ""
  pty_expect "which one?" "banana"
  pty_expect "which one?" "2"
  pty_install --no-systemd
  assert_status 0 "$PTY_STATUS"
  assert_contains "$PTY_TRANSCRIPT" "please answer 1, 2, 3 or 4"
}

# ------------------------------------------------------ a bind of one's own

test_choosing_a_way_in_puts_ayeaye_back_on_this_computer() {
  # An address typed into install.sh's own question a page earlier would leave
  # two things on the network: the front end, and ayeaye behind it. The front
  # end is the one that is supposed to be there.
  _hard_deps_present
  _tailscale_online
  _answer_config_prompts "192.168.1.10" "" "" ""
  pty_expect "which one?" "1"
  pty_expect "may your own tailscale devices reach ayeaye?" "y"
  pty_install --no-systemd
  assert_file_contains "$(_env)" "AYEAYE_BIND=127.0.0.1" \
    "the app stays on loopback whatever was typed earlier"
}

# ---------------------------------------------------------- keeping it all

test_a_run_that_keeps_the_settings_does_not_touch_the_way_in() {
  _hard_deps_present
  _tailscale_online
  mkdir -p "$XDG_CONFIG_HOME/ayeaye"
  cat > "$(_env)" <<'ENV'
AYEAYE_BIND=127.0.0.1
AYEAYE_PORT=7777
AYEAYE_ALLOWED_HOSTS=kept.example
ENV
  pty_expect "rewrite it?" "n"
  pty_install --no-systemd
  assert_status 0 "$PTY_STATUS"
  assert_not_contains "$PTY_TRANSCRIPT" "which one?"
  assert_file_contains "$(_env)" "AYEAYE_ALLOWED_HOSTS=kept.example"
  assert_file_contains "$(_env)" "AYEAYE_PORT=7777"
}

test_a_settings_file_open_to_the_network_is_warned_about_even_when_kept() {
  # Setup will not change a file the person just said to leave alone. It will
  # say what is in it.
  _hard_deps_present
  mkdir -p "$XDG_CONFIG_HOME/ayeaye"
  cat > "$(_env)" <<'ENV'
AYEAYE_BIND=0.0.0.0
AYEAYE_PORT=8911
AYEAYE_ALLOWED_HOSTS=
ENV
  pty_expect "rewrite it?" "n"
  pty_install --no-systemd
  assert_contains "$PTY_TRANSCRIPT" "answer on every address this"
  assert_file_contains "$(_env)" "AYEAYE_BIND=0.0.0.0" \
    "a file the person kept is theirs, warning or no warning"
}

# ------------------------------------------------------------ the plan

test_what_is_about_to_happen_is_said_before_it_happens() {
  _hard_deps_present
  _tailscale_online
  _answer_config_prompts "" "" "" ""
  pty_expect "which one?" "1"
  pty_expect "may your own tailscale devices reach ayeaye?" "y"
  pty_install --no-systemd
  local plan_line serve_line
  plan_line="$(printf '%s\n' "$PTY_TRANSCRIPT" | grep -n "your own tailscale devices will be able to reach ayeaye" | head -1 | cut -d: -f1)"
  serve_line="$(printf '%s\n' "$PTY_TRANSCRIPT" | grep -n "tailscale is answering https" | head -1 | cut -d: -f1)"
  [ -n "$plan_line" ] || fail "the plan never mentioned the network:
$PTY_TRANSCRIPT"
  [ -n "$serve_line" ] || fail "it never got set up:
$PTY_TRANSCRIPT"
  [ "$plan_line" -lt "$serve_line" ] || fail "the plan must come first:
$PTY_TRANSCRIPT"
}
