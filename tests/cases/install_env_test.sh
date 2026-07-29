# The env-file round trip and the auth token.
#
# Two artefacts survive an install and are the contract with everything else in
# the repo: $XDG_CONFIG_HOME/ayeaye/env, rendered from env.template, and
# $XDG_STATE_HOME/ayeaye/token, which the bookmark URL is built from. These
# pin what a run puts in them, what a re-run does to them, and the promise that
# templating only ever touches the four prompted values.
#
# Everything here uses --no-systemd: the unit is another case file's job.

# The minimum for install.sh to get past its hard-dependency gate. python3 is
# the real one because the installer genuinely runs python for templating and
# for the token; tmux is only ever looked up, never executed.
_hard_deps_present() {
  stub_command tmux
  stub_real python3
}

# The four configuration questions, answered by name rather than by position so
# a misplaced answer cannot pass unnoticed.
_answer_config_prompts() {
  pty_expect "bind address" "$1"
  pty_expect "port [" "$2"
  pty_expect "allowed hosts" "$3"
  pty_expect "ntfy topic URL" "$4"
}

# Every @NAME@ still present in a file, deduplicated. Deliberately generic: a
# fifth placeholder added to env.template later is found without editing this.
_placeholders_in() {
  grep -o -E '@[A-Za-z0-9_]+@' "$1" | sort -u
}

# A file with the four substituted lines removed, i.e. the part templating is
# not allowed to change.
_untouched_part() {
  grep -v -E '^(AYEAYE_BIND|AYEAYE_PORT|AYEAYE_ALLOWED_HOSTS|VOICE_NTFY_URL)=' "$1"
}

_line_count() {
  wc -l < "$1" | tr -d ' '
}

# Content signature, filename excluded so two paths can be compared.
_sum() {
  cksum < "$1"
}

# ------------------------------------------------------------- templating

test_a_defaults_run_writes_the_config_and_creates_its_directory() {
  _hard_deps_present
  assert_file_missing "$XDG_CONFIG_HOME/ayeaye" \
    "the sandbox must start without a config directory, or this proves nothing"
  run_install --defaults --no-systemd
  assert_status 0 "$RUN_STATUS"
  assert_file_exists "$XDG_CONFIG_HOME/ayeaye/env"
  assert_contains "$RUN_STDOUT" "wrote $XDG_CONFIG_HOME/ayeaye/env"
}

test_no_placeholder_survives_into_the_written_config() {
  _hard_deps_present
  run_install --defaults --no-systemd

  local in_template left
  in_template="$(_placeholders_in "$REPO_ROOT/env.template")"
  assert_ne "" "$in_template" \
    "the scan must find the template's own placeholders, or it can never fail"
  left="$(_placeholders_in "$XDG_CONFIG_HOME/ayeaye/env")"
  assert_eq "" "$left" \
    "every @NAME@ must be substituted, including any placeholder added to env.template after this test was written"
}

test_the_four_answers_land_in_their_own_keys() {
  _hard_deps_present
  stdin_lines "10.1.2.3" "9911" "one.example,two.example" "https://ntfy.example/topic"
  run_install --no-systemd
  assert_status 0 "$RUN_STATUS"

  local env_file="$XDG_CONFIG_HOME/ayeaye/env"
  assert_file_contains "$env_file" "AYEAYE_BIND=10.1.2.3"
  assert_file_contains "$env_file" "AYEAYE_PORT=9911"
  assert_file_contains "$env_file" "AYEAYE_ALLOWED_HOSTS=one.example,two.example"
  assert_file_contains "$env_file" "VOICE_NTFY_URL=https://ntfy.example/topic"
}

test_everything_the_installer_was_not_asked_about_survives_verbatim() {
  _hard_deps_present
  run_install --defaults --no-systemd

  local env_file="$XDG_CONFIG_HOME/ayeaye/env"
  assert_eq "$(_line_count "$REPO_ROOT/env.template")" "$(_line_count "$env_file")" \
    "templating must not add or drop a line"
  assert_eq "$(_untouched_part "$REPO_ROOT/env.template" | cksum)" \
            "$(_untouched_part "$env_file" | cksum)" \
    "outside the four substituted lines the written file must be the template, byte for byte"

  # The commented-out defaults are the file's documentation of every other
  # setting; losing them would be silent.
  assert_file_contains "$env_file" "#AYEAYE_TOKEN="
  assert_file_contains "$env_file" "#AYEAYE_SHARE="
  assert_file_contains "$env_file" "#AYEAYE_LINES=24"
  assert_file_contains "$env_file" \
    "# Values are literal. No shell expansion happens (systemd does none either),"
}

test_the_temporary_file_is_not_left_next_to_the_config() {
  _hard_deps_present
  run_install --defaults --no-systemd

  local leftovers
  leftovers="$(find "$XDG_CONFIG_HOME/ayeaye" -name '*env.tmp*')"
  assert_eq "" "$leftovers" \
    "the render target must be renamed into place, not left behind beside the config"
}

test_answers_typed_at_a_terminal_reach_the_same_keys() {
  _hard_deps_present
  _answer_config_prompts "192.0.2.77" "8123" "typed.example" "https://ntfy.example/typed"
  pty_install --no-systemd
  assert_status 0 "$PTY_STATUS"

  local env_file="$XDG_CONFIG_HOME/ayeaye/env"
  assert_file_contains "$env_file" "AYEAYE_BIND=192.0.2.77"
  assert_file_contains "$env_file" "AYEAYE_PORT=8123"
  assert_file_contains "$env_file" "AYEAYE_ALLOWED_HOSTS=typed.example"
  assert_file_contains "$env_file" "VOICE_NTFY_URL=https://ntfy.example/typed"
}

test_answers_full_of_separators_reach_the_file_unmangled() {
  # The values a substitution engine is most likely to eat: a comma list, and a
  # URL with the slashes and ampersands that would need escaping if this were
  # sed rather than python.
  _hard_deps_present
  stdin_lines "127.0.0.1" "8911" "a.example,b.example,c.example" \
              "https://ntfy.example/a/b?x=1&y=2"
  run_install --no-systemd
  assert_status 0 "$RUN_STATUS"

  local env_file="$XDG_CONFIG_HOME/ayeaye/env"
  assert_file_contains "$env_file" "AYEAYE_ALLOWED_HOSTS=a.example,b.example,c.example"
  assert_file_contains "$env_file" "VOICE_NTFY_URL=https://ntfy.example/a/b?x=1&y=2"
  assert_contains "$RUN_STDOUT" "tier    : text-only +notifications" \
    "the ntfy URL must also read back out of the file it was just written to"
}

test_an_answer_shaped_like_a_placeholder_is_substituted_a_second_time() {
  # Documented, not endorsed: install.sh:125-127 replaces the four placeholders
  # in sequence over the whole text, so a value substituted early is still
  # eligible for the substitutions that follow. Answering "@PORT@" to the bind
  # question puts the port in AYEAYE_BIND.
  _hard_deps_present
  stdin_lines "@PORT@" "9911" "" ""
  run_install --no-systemd
  assert_status 0 "$RUN_STATUS"
  assert_file_contains "$XDG_CONFIG_HOME/ayeaye/env" "AYEAYE_BIND=9911" \
    "today the injected placeholder is expanded rather than written literally"
}

# ----------------------------------------------------------- re-running

test_a_second_run_asks_before_rewriting_the_config_it_wrote() {
  _hard_deps_present
  run_install --defaults --no-systemd
  assert_file_exists "$XDG_CONFIG_HOME/ayeaye/env"

  pty_expect "rewrite it?" "n"
  pty_install --no-systemd
  assert_contains "$PTY_TRANSCRIPT" "config exists at $XDG_CONFIG_HOME/ayeaye/env"
  assert_contains "$PTY_TRANSCRIPT" "rewrite it? (n keeps the current file) [n]:"
}

test_declining_the_rewrite_leaves_the_file_byte_for_byte_identical() {
  _hard_deps_present
  stdin_lines "10.1.2.3" "9911" "kept.example" "https://ntfy.example/kept"
  run_install --no-systemd

  local env_file="$XDG_CONFIG_HOME/ayeaye/env" before before_sum
  before="$(cat "$env_file")"
  before_sum="$(_sum "$env_file")"

  stdin_lines "n"
  run_install --no-systemd
  assert_status 0 "$RUN_STATUS"
  assert_contains "$RUN_STDOUT" "keeping existing config"
  assert_eq "$before_sum" "$(_sum "$env_file")" \
    "a declined rewrite must not touch a single byte"
  assert_eq "$before" "$(cat "$env_file")"
}

test_accepting_the_rewrite_replaces_the_answers() {
  _hard_deps_present
  run_install --defaults --no-systemd
  assert_file_contains "$XDG_CONFIG_HOME/ayeaye/env" "AYEAYE_PORT=8911"

  stdin_lines "y" "10.1.2.3" "9911" "second.example" "https://ntfy.example/second"
  run_install --no-systemd
  assert_status 0 "$RUN_STATUS"

  local env_file="$XDG_CONFIG_HOME/ayeaye/env"
  assert_file_contains "$env_file" "AYEAYE_BIND=10.1.2.3"
  assert_file_contains "$env_file" "AYEAYE_PORT=9911"
  assert_file_contains "$env_file" "AYEAYE_ALLOWED_HOSTS=second.example"
  assert_file_contains "$env_file" "VOICE_NTFY_URL=https://ntfy.example/second"
  assert_file_not_contains "$env_file" "AYEAYE_PORT=8911" \
    "the first run's answers must be gone, not appended to"
}

test_a_defaults_rerun_keeps_the_existing_file_without_asking() {
  _hard_deps_present
  stdin_lines "10.1.2.3" "9911" "" ""
  run_install --no-systemd

  local env_file="$XDG_CONFIG_HOME/ayeaye/env" before_sum
  before_sum="$(_sum "$env_file")"

  run_install --defaults --no-systemd
  assert_status 0 "$RUN_STATUS"
  assert_contains "$RUN_STDOUT" "keeping existing config"
  assert_not_contains "$RUN_OUTPUT" "rewrite it?" \
    "--defaults answers the rewrite question with the safe option instead of asking it"
  assert_eq "$before_sum" "$(_sum "$env_file")"
  assert_contains "$RUN_STDOUT" "bookmark: http://10.1.2.3:9911/?token=" \
    "the summary of a kept config is read back out of the file, not out of the defaults"
}

# --------------------------------------------------------------- the token

test_the_first_run_creates_a_private_non_empty_token() {
  _hard_deps_present
  assert_file_missing "$XDG_STATE_HOME/ayeaye" \
    "the sandbox must start without a state directory, or this proves nothing"
  run_install --defaults --no-systemd
  assert_status 0 "$RUN_STATUS"

  local token_file="$XDG_STATE_HOME/ayeaye/token" token
  assert_file_exists "$token_file"
  assert_file_mode 600 "$token_file" \
    "the token is a bearer credential: nothing but the owner may read it"
  token="$(cat "$token_file")"
  assert_ne "" "$token" "an empty token would authenticate nobody and lock nothing"
  assert_matches "$token" '^[A-Za-z0-9_-]+$' \
    "url-safe, because it goes straight into the bookmark query string"
}

test_the_bookmark_url_carries_that_exact_token_and_the_chosen_address() {
  _hard_deps_present
  stdin_lines "10.1.2.3" "9911" "" ""
  run_install --no-systemd
  assert_status 0 "$RUN_STATUS"

  local token
  token="$(cat "$XDG_STATE_HOME/ayeaye/token")"
  assert_ne "" "$token"
  assert_contains "$RUN_STDOUT" "bookmark: http://10.1.2.3:9911/?token=$token" \
    "the printed URL must carry the token that was actually stored, not a second one"
}

test_a_second_run_does_not_regenerate_the_token() {
  _hard_deps_present
  run_install --defaults --no-systemd
  local token_file="$XDG_STATE_HOME/ayeaye/token" first
  first="$(cat "$token_file")"
  assert_ne "" "$first"

  run_install --defaults --no-systemd
  assert_status 0 "$RUN_STATUS"
  assert_eq "$first" "$(cat "$token_file")" \
    "re-running the installer must not invalidate the bookmark already on the phone"
}

test_a_world_readable_token_is_tightened_by_a_run() {
  _hard_deps_present
  local token_file="$XDG_STATE_HOME/ayeaye/token"
  mkdir -p "$XDG_STATE_HOME/ayeaye"
  printf '%s\n' "preexisting-token" > "$token_file"
  chmod 644 "$token_file"
  assert_file_mode 644 "$token_file" "the loose mode must really be in place first"

  run_install --defaults --no-systemd
  assert_status 0 "$RUN_STATUS"
  assert_file_mode 600 "$token_file" \
    "a re-run repairs the permissions of a token left readable by others"
  assert_eq "preexisting-token" "$(cat "$token_file")" \
    "repairing the mode must not change the value"
}

test_a_zero_byte_token_file_is_replaced_rather_than_kept() {
  # The guard is `-s`, not `-e` (install.sh:155): an empty file counts as no
  # token at all, which is what makes an interrupted first run recoverable.
  _hard_deps_present
  local token_file="$XDG_STATE_HOME/ayeaye/token"
  mkdir -p "$XDG_STATE_HOME/ayeaye"
  : > "$token_file"

  run_install --defaults --no-systemd
  assert_status 0 "$RUN_STATUS"
  assert_ne "" "$(cat "$token_file")" "a zero-byte token must be filled in, not accepted"
  assert_file_mode 600 "$token_file"
  assert_contains "$RUN_STDOUT" "generated auth token at $token_file"
}

test_the_token_is_announced_on_the_run_that_creates_it_and_no_other() {
  _hard_deps_present
  run_install --defaults --no-systemd
  assert_contains "$RUN_STDOUT" "generated auth token at $XDG_STATE_HOME/ayeaye/token"

  run_install --defaults --no-systemd
  assert_not_contains "$RUN_OUTPUT" "generated auth token" \
    "the second run has nothing to generate, so it must stay quiet about it"
}
