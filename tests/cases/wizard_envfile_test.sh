# The settings file: writing it the first time, and changing it every time
# after that.
#
# The behaviour being replaced is a whole-file rewrite. It threw away anything
# the user had put in the file, it re-expanded its own substitutions, it left a
# half-written file behind when it failed, and it created the file with
# whatever umask happened to be in force - in a file documented as a place to
# put an auth token.

setup() {
  require_host_command python3
  stub_real python3
  WIZARD_STATE_DIR="$TEST_TMPDIR/state"
  WIZARD_STATE_FILE="$WIZARD_STATE_DIR/setup-state"
  WIZARD_CONSENT_LEDGER="$WIZARD_STATE_DIR/setup-consent.log"
  WIZARD_LOG_FILE="$WIZARD_STATE_DIR/setup.log"
  WIZARD_BACKUP_DIR="$WIZARD_STATE_DIR/backups"
  PLATFORM_OS_RELEASE_FILES=""
  export PLATFORM_OS_RELEASE_FILES
  . "$REPO_ROOT/lib/state.sh"
  . "$REPO_ROOT/lib/ui.sh"
  . "$REPO_ROOT/lib/platform.sh"
  . "$REPO_ROOT/lib/consent.sh"
  . "$REPO_ROOT/lib/envfile.sh"
  ENV_FILE="$TEST_TMPDIR/env"
  TEMPLATE="$REPO_ROOT/env.template"
}

_leftovers() { find "$TEST_TMPDIR" -name 'env.tmp*'; }

# ------------------------------------------------------------- rendering

test_a_rendered_file_has_the_answers_in_it() {
  wizard_env_render "$TEMPLATE" "$ENV_FILE" \
    "AYEAYE_BIND=10.1.2.3" "AYEAYE_PORT=9911" \
    "AYEAYE_ALLOWED_HOSTS=one.example" "VOICE_NTFY_URL=https://ntfy.example/t"
  assert_status 0 "$?"
  assert_file_contains "$ENV_FILE" "AYEAYE_BIND=10.1.2.3"
  assert_file_contains "$ENV_FILE" "AYEAYE_PORT=9911"
  assert_file_contains "$ENV_FILE" "AYEAYE_ALLOWED_HOSTS=one.example"
  assert_file_contains "$ENV_FILE" "VOICE_NTFY_URL=https://ntfy.example/t"
}

test_nothing_the_template_documents_is_lost() {
  wizard_env_render "$TEMPLATE" "$ENV_FILE" \
    "AYEAYE_BIND=127.0.0.1" "AYEAYE_PORT=8911" \
    "AYEAYE_ALLOWED_HOSTS=" "VOICE_NTFY_URL="
  assert_eq "$(wc -l < "$TEMPLATE" | tr -d ' ')" "$(wc -l < "$ENV_FILE" | tr -d ' ')" \
    "rendering must not add or drop a line"
  assert_file_contains "$ENV_FILE" "#AYEAYE_TOKEN="
}

test_no_placeholder_survives() {
  wizard_env_render "$TEMPLATE" "$ENV_FILE" \
    "AYEAYE_BIND=127.0.0.1" "AYEAYE_PORT=8911" \
    "AYEAYE_ALLOWED_HOSTS=" "VOICE_NTFY_URL="
  assert_eq "" "$(grep -o -E '@[A-Za-z0-9_]+@' "$ENV_FILE" | sort -u)"
}

test_an_answer_shaped_like_a_placeholder_is_written_literally() {
  # The defect this replaces: substituting the placeholders one after another
  # over the whole text made a value substituted early eligible for the
  # substitutions that followed, so answering "@PORT@" to the address question
  # put the port number in the address.
  wizard_env_render "$TEMPLATE" "$ENV_FILE" \
    "AYEAYE_BIND=@PORT@" "AYEAYE_PORT=9911" \
    "AYEAYE_ALLOWED_HOSTS=" "VOICE_NTFY_URL="
  assert_file_contains "$ENV_FILE" "AYEAYE_BIND=@PORT@" \
    "an answer is a value, never another instruction"
  assert_file_not_contains "$ENV_FILE" "AYEAYE_BIND=9911"
}

test_answers_full_of_separators_arrive_unmangled() {
  wizard_env_render "$TEMPLATE" "$ENV_FILE" \
    "AYEAYE_BIND=127.0.0.1" "AYEAYE_PORT=8911" \
    "AYEAYE_ALLOWED_HOSTS=a.example,b.example,c.example" \
    "VOICE_NTFY_URL=https://ntfy.example/a/b?x=1&y=2"
  assert_file_contains "$ENV_FILE" "AYEAYE_ALLOWED_HOSTS=a.example,b.example,c.example"
  assert_file_contains "$ENV_FILE" "VOICE_NTFY_URL=https://ntfy.example/a/b?x=1&y=2"
}

test_the_settings_file_is_private_to_its_owner() {
  # It is documented as a place to put AYEAYE_TOKEN, which is a bearer
  # credential. A loose umask must not be able to make it world-readable.
  umask 000
  wizard_env_render "$TEMPLATE" "$ENV_FILE" \
    "AYEAYE_BIND=127.0.0.1" "AYEAYE_PORT=8911" \
    "AYEAYE_ALLOWED_HOSTS=" "VOICE_NTFY_URL="
  assert_file_mode 600 "$ENV_FILE"
}

test_the_directory_is_created_if_it_is_missing() {
  local nested="$TEST_TMPDIR/a/b/env"
  wizard_env_render "$TEMPLATE" "$nested" "AYEAYE_PORT=8911"
  assert_status 0 "$?"
  assert_file_exists "$nested"
}

test_a_failed_render_leaves_nothing_behind() {
  # The defect this replaces: the render target was created next to the
  # settings file and only ever removed by the rename that followed it, so a
  # template that could not be read left a file nothing afterwards looked at.
  wizard_env_render "$TEST_TMPDIR/no-such-template" "$ENV_FILE" "AYEAYE_PORT=8911"
  assert_ne 0 "$?" "a template it cannot read is not a success"
  assert_file_missing "$ENV_FILE"
  assert_eq "" "$(_leftovers)" "no half-written file may survive a failed render"
}

test_a_render_that_fails_halfway_removes_what_it_had_written() {
  # The missing-template path gives up before it creates anything, so it cannot
  # prove the cleanup. This one gets past that check and fails at the write: a
  # template that is there and a destination directory that cannot be written
  # into.
  [ "$(id -u)" = 0 ] && skip "root can write into a directory it has no write bit for"
  local locked="$TEST_TMPDIR/locked"
  mkdir -p "$locked"
  printf 'AYEAYE_PORT=@PORT@\n' > "$locked/template"
  chmod 500 "$locked"
  wizard_env_render "$locked/template" "$locked/env" "AYEAYE_PORT=8911"
  local status=$?
  chmod 700 "$locked"
  assert_ne 0 "$status" "a render it could not write is not a success"
  assert_file_missing "$locked/env"
  assert_eq "" "$(find "$locked" -name 'env.tmp*')" \
    "and the temporary file it did create is gone"
}

test_a_failed_render_does_not_touch_the_file_that_was_there() {
  printf 'AYEAYE_PORT=7777\n' > "$ENV_FILE"
  wizard_env_render "$TEST_TMPDIR/no-such-template" "$ENV_FILE" "AYEAYE_PORT=8911"
  assert_ne 0 "$?"
  assert_file_contains "$ENV_FILE" "AYEAYE_PORT=7777" \
    "the previous settings must survive a render that never happened"
  assert_eq "" "$(_leftovers)"
}

# --------------------------------------------------------------- merging

test_a_merge_changes_only_the_keys_it_was_given() {
  cat > "$ENV_FILE" <<'ENV'
# my own note about this machine
AYEAYE_BIND=10.9.8.7
AYEAYE_PORT=7777
AYEAYE_NAME=the-workshop
SOMETHING_ELSE=kept
ENV
  wizard_env_merge "$ENV_FILE" "AYEAYE_PORT=9000"
  assert_status 0 "$?"
  assert_file_contains "$ENV_FILE" "AYEAYE_PORT=9000"
  assert_file_contains "$ENV_FILE" "AYEAYE_BIND=10.9.8.7" "an untouched key survives"
  assert_file_contains "$ENV_FILE" "AYEAYE_NAME=the-workshop" \
    "a setting this wizard never asks about survives"
  assert_file_contains "$ENV_FILE" "SOMETHING_ELSE=kept" \
    "a setting this project has never heard of survives"
  assert_file_contains "$ENV_FILE" "# my own note about this machine" \
    "and so does what the user wrote in the margin"
}

test_a_merge_keeps_the_order_of_the_file() {
  cat > "$ENV_FILE" <<'ENV'
AYEAYE_BIND=10.9.8.7
AYEAYE_PORT=7777
AYEAYE_NAME=the-workshop
ENV
  wizard_env_merge "$ENV_FILE" "AYEAYE_PORT=9000"
  assert_eq "AYEAYE_BIND=10.9.8.7
AYEAYE_PORT=9000
AYEAYE_NAME=the-workshop" "$(cat "$ENV_FILE")"
}

test_a_key_the_file_does_not_have_is_added() {
  printf 'AYEAYE_PORT=7777\n' > "$ENV_FILE"
  wizard_env_merge "$ENV_FILE" "AYEAYE_ALLOWED_HOSTS=new.example"
  assert_file_contains "$ENV_FILE" "AYEAYE_PORT=7777"
  assert_file_contains "$ENV_FILE" "AYEAYE_ALLOWED_HOSTS=new.example"
}

test_a_commented_out_key_is_uncommented_rather_than_duplicated() {
  # The template ships every optional setting commented out with its default.
  # Setting one for the first time must produce one line, not a commented one
  # and a live one that disagree.
  printf '#AYEAYE_NAME=\nAYEAYE_PORT=7777\n' > "$ENV_FILE"
  wizard_env_merge "$ENV_FILE" "AYEAYE_NAME=the-workshop"
  assert_file_contains "$ENV_FILE" "AYEAYE_NAME=the-workshop"
  assert_eq "1" "$(grep -c 'AYEAYE_NAME' "$ENV_FILE")" \
    "two lines for one setting is how a file starts disagreeing with itself"
}

test_a_duplicated_key_is_left_with_one_live_value() {
  printf 'AYEAYE_PORT=7777\nAYEAYE_PORT=8888\n' > "$ENV_FILE"
  wizard_env_merge "$ENV_FILE" "AYEAYE_PORT=9000"
  assert_eq "1" "$(grep -c '^AYEAYE_PORT=' "$ENV_FILE")"
  assert_file_contains "$ENV_FILE" "AYEAYE_PORT=9000"
}

test_merging_into_a_file_that_is_not_there_is_a_caller_bug() {
  wizard_env_merge "$TEST_TMPDIR/absent" "AYEAYE_PORT=9000"
  assert_ne 0 "$?"
}

test_a_merge_leaves_nothing_behind_and_keeps_the_mode() {
  printf 'AYEAYE_PORT=7777\n' > "$ENV_FILE"
  chmod 600 "$ENV_FILE"
  wizard_env_merge "$ENV_FILE" "AYEAYE_PORT=9000"
  assert_file_mode 600 "$ENV_FILE"
  assert_eq "" "$(_leftovers)"
}

# ------------------------------------------------------------- reading back

test_a_value_is_read_back_with_its_quotes_stripped() {
  cat > "$ENV_FILE" <<'ENV'
AYEAYE_BIND="10.0.0.5"
AYEAYE_PORT='9100'
ENV
  assert_eq "10.0.0.5" "$(wizard_env_get "$ENV_FILE" AYEAYE_BIND)"
  assert_eq "9100" "$(wizard_env_get "$ENV_FILE" AYEAYE_PORT)"
}

test_the_last_assignment_wins() {
  printf 'AYEAYE_PORT=9100\nAYEAYE_PORT=9200\n' > "$ENV_FILE"
  assert_eq "9200" "$(wizard_env_get "$ENV_FILE" AYEAYE_PORT)" \
    "which is what systemd's EnvironmentFile does too"
}

test_a_missing_key_reads_back_as_the_default() {
  printf 'AYEAYE_PORT=9100\n' > "$ENV_FILE"
  assert_eq "127.0.0.1" "$(wizard_env_get "$ENV_FILE" AYEAYE_BIND 127.0.0.1)"
}

test_a_missing_file_reads_back_as_the_default() {
  assert_eq "8911" "$(wizard_env_get "$TEST_TMPDIR/absent" AYEAYE_PORT 8911)"
}

test_reading_back_always_succeeds() {
  # Assigned bare at every call site under `set -e`.
  wizard_env_get "$TEST_TMPDIR/absent" AYEAYE_PORT >/dev/null
  assert_status 0 "$?"
}

test_a_commented_key_is_not_a_value() {
  printf '#AYEAYE_PORT=9100\n' > "$ENV_FILE"
  assert_eq "8911" "$(wizard_env_get "$ENV_FILE" AYEAYE_PORT 8911)" \
    "the template ships every optional setting commented out with its default"
}
