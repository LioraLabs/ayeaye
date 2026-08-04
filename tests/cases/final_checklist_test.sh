# The last screen of a run, and the one thing it may never be: wrong about
# this computer.
#
# Everything on it is built from what actually happened - which service manager
# was really used, which way in was really set up, which capabilities the
# health check could really confirm - rather than from what usually happens.
# A closing screen that tells a Mac owner to run systemctl has told them
# something false about their own machine, and a screen that lists a phone
# address on a run that set up no way in has told them something worse.

_hard_deps_present() {
  require_host_command python3
  stub_command tmux
  stub_real python3
}

# A host with a live systemd user session, and every command it needs.
_systemd_session() {
  stub_command systemctl --exit 1 --stderr "unknown systemctl invocation"
  stub_when systemctl '--user show-environment' --exit 0
  stub_when systemctl '--user daemon-reload' --exit 0
  stub_when systemctl '--user enable --now *' --exit 0
}

# A Mac with launchd, the same Darwin costume service_launchd_test.sh wears.
_mac_with_launchd() {
  stub_command uname --exit 1
  stub_when uname '-s' --stdout "Darwin"
  stub_when uname '-m' --stdout "arm64"
  stub_command_from_fixture sw_vers sw_vers/macos-15.1
  PLATFORM_OS_RELEASE_FILES=""
  export PLATFORM_OS_RELEASE_FILES
  stub_script launchctl <<'SH'
case "$*" in
  print*)
    [ -f "$HOME/launchctl-registered" ] || exit 1
    printf 'state = running\n'
    ;;
  bootstrap*)
    if [ -f "$HOME/launchctl-registered" ]; then
      printf 'Bootstrap failed: 5: Input/output error\n' >&2
      exit 5
    fi
    : > "$HOME/launchctl-registered"
    ;;
  bootout*) rm -f "$HOME/launchctl-registered" ;;
esac
exit 0
SH
}

_env_file()   { printf '%s' "$XDG_CONFIG_HOME/ayeaye/env"; }
_token_file() { printf '%s' "$XDG_STATE_HOME/ayeaye/token"; }
_unit_file()  { printf '%s' "$XDG_CONFIG_HOME/systemd/user/ayeaye.service"; }
_plist()      { printf '%s' "$HOME/Library/LaunchAgents/dev.ayeaye.plist"; }

# _removal_block <output> - the closing removal instructions, and nothing else,
# so that an assertion about them cannot accidentally be satisfied by something
# an earlier stage said.
_removal_block() {
  printf '%s\n' "$1" | sed -n '/^to remove it:/,/^$/p'
}

# ================================================ removal, per platform, exactly

test_a_linux_run_is_told_how_to_remove_a_systemd_unit() {
  _hard_deps_present
  _systemd_session
  run_install --defaults
  assert_status 0 "$RUN_STATUS"
  local block
  block="$(_removal_block "$RUN_STDOUT")"
  assert_contains "$block" "systemctl --user disable --now ayeaye.service"
  assert_contains "$block" "$(_unit_file)"
  assert_not_contains "$block" "launchctl"
}

test_a_mac_run_is_never_told_to_run_systemctl() {
  # The defect this test exists for: the closing block used to print systemd
  # removal steps unconditionally, ten lines after the service stage had
  # printed the correct launchd ones.
  _hard_deps_present
  _mac_with_launchd
  run_install --defaults
  assert_status 0 "$RUN_STATUS"
  local block
  block="$(_removal_block "$RUN_STDOUT")"
  assert_contains "$block" "launchctl bootout gui/$(id -u)/dev.ayeaye"
  assert_contains "$block" "$(_plist)"
  assert_not_contains "$block" "systemctl"
  assert_not_contains "$block" "systemd/user"
}

test_a_mac_run_names_the_launchd_log_and_not_a_journal() {
  _hard_deps_present
  _mac_with_launchd
  run_install --defaults
  assert_matches "$RUN_STDOUT" "logs[[:space:]]*: tail -f $HOME/Library/Logs/ayeaye/ayeaye.log"
  assert_not_contains "$RUN_STDOUT" "journalctl"
}

test_a_machine_that_starts_nothing_for_you_is_told_the_truth_about_removal() {
  _hard_deps_present
  run_install --defaults --no-systemd
  assert_status 0 "$RUN_STATUS"
  local block
  block="$(_removal_block "$RUN_STDOUT")"
  assert_contains "$block" "stop the ayeaye you started by hand"
  assert_not_contains "$block" "systemctl"
  assert_not_contains "$block" "launchctl"
  assert_not_contains "$RUN_STDOUT" "journalctl"
}

test_every_removal_block_names_the_settings_and_the_state_directory() {
  _hard_deps_present
  _systemd_session
  run_install --defaults
  local block
  block="$(_removal_block "$RUN_STDOUT")"
  assert_contains "$block" "$(_env_file)"
  assert_contains "$block" "$XDG_STATE_HOME/ayeaye"
}

# ==================================================== the address for the phone

test_a_run_with_no_way_in_says_the_phone_cannot_reach_it_yet() {
  _hard_deps_present
  run_install --defaults --no-systemd
  assert_status 0 "$RUN_STATUS"
  assert_contains "$RUN_STDOUT" "your phone"
  assert_contains "$RUN_STDOUT" "cannot reach ayeaye yet"
  assert_matches "$RUN_STDOUT" "bookmark: http://127\.0\.0\.1:8911/\?token="
  assert_not_contains "$RUN_STDOUT" "open that one on your phone."
}

test_a_way_in_that_was_really_set_up_gives_the_phone_that_address() {
  # The address a phone is handed comes from what the way-in step actually
  # installed, not from the list of addresses ayeaye accepts. Here one really
  # is set up: the run is asked, it says yes, and the proxy mode writes the
  # configuration and the allow list together.
  _hard_deps_present
  stdin_lines "" "box.example.ts.net" "" "y"
  run_install --no-systemd
  assert_status 0 "$RUN_STATUS"
  assert_matches "$RUN_STDOUT" "bookmark: https://box\.example\.ts\.net/\?token="
  assert_matches "$RUN_STDOUT" "in a browser on this computer: http://127\.0\.0\.1:8911/\?token="
}

test_an_address_in_the_settings_is_not_a_way_in_that_was_set_up() {
  # The defect: install.sh offers a tailscale name it noticed as the default
  # for the allowed-hosts question, so a settings file can name an https
  # address that nothing is serving - and the closing screen used to read that
  # list as "here is the address for your phone" and invent one. It is still
  # printed, because losing somebody's own address would be unhelpful, and it
  # is neither led with nor called theirs to open.
  _hard_deps_present
  mkdir -p "$XDG_CONFIG_HOME/ayeaye"
  cat > "$(_env_file)" <<'ENV'
AYEAYE_BIND=127.0.0.1
AYEAYE_PORT=8911
AYEAYE_ALLOWED_HOSTS=box.example.ts.net
ENV
  run_install --defaults --no-systemd
  assert_status 0 "$RUN_STATUS"
  assert_matches "$RUN_STDOUT" "bookmark: http://127\.0\.0\.1:8911/\?token="
  assert_not_contains "$RUN_STDOUT" "bookmark: https://box.example.ts.net" \
    "nothing was set up, so nothing may be led with"
  assert_not_contains "$RUN_STDOUT" "open that one on your phone." \
    "and nobody is told to open it"
  assert_contains "$RUN_STDOUT" "setup did not put it there and cannot see it" \
    "the address is still shown, with what is and is not known about it"
}

test_a_way_in_that_is_not_finished_is_not_handed_to_a_phone_as_working() {
  # The proxy mode writes the configuration and then cannot reach the address,
  # because the person still has to add that configuration to their proxy. The
  # address is right; it is not yet an address to open from the sofa.
  _hard_deps_present
  stdin_lines "" "not-yet.example" "" "y"
  run_install --no-systemd
  assert_status 0 "$RUN_STATUS"
  assert_matches "$RUN_STDOUT" "bookmark: https://not-yet\.example/\?token="
  assert_contains "$RUN_STDOUT" "It is not finished yet, and what it is waiting"
  assert_not_contains "$RUN_STDOUT" "open that one on your phone." \
    "a front end waiting for its configuration is not one to send somebody to"
}

test_the_address_carries_the_key_that_is_really_on_this_machine() {
  _hard_deps_present
  run_install --defaults --no-systemd
  local token
  token="$(cat "$(_token_file)")"
  assert_ne "" "$token"
  assert_contains "$RUN_STDOUT" "?token=$token"
}

# ======================================================== signing in, in words

test_the_closing_screen_explains_the_bookmark_flow_and_the_header() {
  _hard_deps_present
  run_install --defaults --no-systemd
  assert_contains "$RUN_STDOUT" "signing in, once per device:"
  assert_contains "$RUN_STDOUT" "It puts a key in the browser and sends you"
  assert_contains "$RUN_STDOUT" "X-Voice-Token"
  assert_contains "$RUN_STDOUT" "$(_token_file)"
}

test_the_key_itself_is_named_by_its_file_and_not_pasted_into_a_command() {
  # The token belongs in the address, which is a thing to open once, and in a
  # file, which is a thing to read. Not in a command line somebody is invited
  # to paste, where it would end up in their shell history.
  _hard_deps_present
  run_install --defaults --no-systemd
  assert_not_contains "$RUN_STDOUT" "X-Voice-Token: $(cat "$(_token_file)")"
}

# ==================================================== what this computer can do

test_the_closing_screen_reports_what_was_set_up_not_what_was_measured() {
  # It used to print "tier : text-only" whatever the machine was, from a probe
  # of its own that had nothing to do with the measurement three stages back.
  # Reading the measurement instead is the opposite mistake and a worse one:
  # the hardware tier says what this computer has room for, and a run that set
  # voice up for nobody would announce that talking works.
  _hard_deps_present
  run_install --defaults --no-systemd
  assert_contains "$RUN_STDOUT" "what works: typing"
  assert_not_contains "$RUN_STDOUT" "probed live"
  assert_not_contains "$RUN_STDOUT" "what works: typing, and talking out loud" \
    "nothing about voice was chosen in this run"
}

test_a_notification_url_is_part_of_what_works() {
  _hard_deps_present
  mkdir -p "$XDG_CONFIG_HOME/ayeaye"
  cat > "$(_env_file)" <<'ENV'
AYEAYE_PORT=8911
VOICE_NTFY_URL=https://ntfy.example/mine
ENV
  run_install --defaults --no-systemd
  assert_contains "$RUN_STDOUT" "with notifications to your phone"
}

test_only_one_thing_in_a_run_says_whether_this_computer_can_listen() {
  # Three verdicts about one capability, in two framings, is what this run used
  # to give: a "voice tier: missing …" sweep in stage two, a "talking to them
  # out loud: not yet" paragraph in stage three, and a stale "tier : text-only"
  # word in stage eight that meant something else entirely.
  #
  # One verdict now, from the step that measured the machine. Stage two still
  # lists which programs are installed, and that is deliberately not a verdict:
  # it is an inventory, it says "Nothing above is required", and it is the
  # difference between "you do not have ffmpeg" and "this computer cannot
  # listen".
  _hard_deps_present
  run_install --defaults --no-systemd
  assert_not_contains "$RUN_STDOUT" "voice tier:"
  assert_not_contains "$RUN_STDOUT" "What that means for you:"
  assert_not_contains "$RUN_STDOUT" "talking to them out loud"
  assert_not_contains "$RUN_STDOUT" "probed live"
  assert_eq "1" \
    "$(printf '%s\n' "$RUN_STDOUT" | grep -c 'Talking to your agents out loud')" \
    "the hardware step is the one place that says what talking here would be like"
  assert_eq "1" \
    "$(printf '%s\n' "$RUN_STDOUT" | grep -c '^what works: ')" \
    "and the closing line says what this run set up, once"
}

# =============================================== unfinished work, and how to resume

test_every_unfinished_item_says_how_to_pick_it_up() {
  # Structural rather than by name: which items are outstanding depends on the
  # machine, and asserting on the whole block would pin something about this
  # sandbox rather than about the wizard.
  _hard_deps_present
  run_install --defaults --no-systemd
  local block seen=0 line previous=""
  block="$(printf '%s\n' "$RUN_STDOUT" | sed -n '/^not finished, and worth coming back to:/,$p')"
  [ -n "$block" ] || skip "this run finished everything, so there is nothing to check"
  while IFS= read -r line; do
    case "$previous" in
      "  - "*)
        seen=$((seen + 1))
        case "$line" in
          "    "?*) ;;
          *) fail "an unfinished item with no way to resume it: $previous" ;;
        esac
        ;;
    esac
    previous="$line"
  done <<EOF
$block
EOF
  assert_ne 0 "$seen" "the block was printed, so it had at least one item in it"
}

test_a_health_check_that_could_not_confirm_something_names_it() {
  # A run that really started the service, on a machine with no curl: every
  # check that needs the network can only answer "could not tell", and the
  # closing screen has to say which capabilities those were rather than
  # "the health check reported a problem".
  _hard_deps_present
  _systemd_session
  pty_answers "" "" ""
  pty_expect "enable and start it now?" "y"
  pty_install
  assert_status 0 "$PTY_STATUS"
  assert_contains "$PTY_TRANSCRIPT" "Checking that what you chose works (not finished)"
  assert_contains "$PTY_TRANSCRIPT" "still to sort out:"
  assert_contains "$PTY_TRANSCRIPT" "ayeaye answering on this computer"
}

test_the_health_check_marks_are_explained_before_the_checklist() {
  _hard_deps_present
  _systemd_session
  pty_answers "" "" ""
  pty_expect "enable and start it now?" "y"
  pty_install
  assert_contains "$PTY_TRANSCRIPT" "Checking what you asked for."
  assert_contains "$PTY_TRANSCRIPT" "skipped  you did not ask"
}

# ============================================================= changing your mind

test_the_closing_screen_says_rerunning_is_how_anything_is_changed() {
  _hard_deps_present
  run_install --defaults --no-systemd
  assert_contains "$RUN_STDOUT" "to change any of this later: run ./install.sh again."
  assert_contains "$RUN_STDOUT" "you switch between the four ways of reaching ayeaye."
}

# ======================================================= the public one-liner

test_help_quotes_the_public_one_liner_and_no_stale_caveat() {
  # There used to be a caveat here saying the release did not exist. The
  # release exists, so the caveat itself would now be the false claim.
  run_script "$REPO_ROOT/install.sh" --help
  assert_status 0 "$RUN_STATUS"
  assert_contains "$RUN_STDOUT" "curl -fsSL"
  assert_not_contains "$RUN_STDOUT" "has not been published"
}
