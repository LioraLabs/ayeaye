# The setup state file: what a run remembers, and how it survives being
# interrupted.
#
# Resumability is the property this file pins. Everything the wizard learns -
# which stages finished, which steps are still outstanding, what the user
# answered - goes through these five functions, so an interrupted run can pick
# up where it stopped instead of starting over.
#
# The store is deliberately dumb: one key=value per line, written by machine,
# read by machine. It is not a config file and nobody is expected to edit it.

setup() {
  . "$REPO_ROOT/lib/state.sh"
  WIZARD_STATE_DIR="$TEST_TMPDIR/state"
  WIZARD_STATE_FILE="$WIZARD_STATE_DIR/setup-state"
  wizard_state_load
}

# ------------------------------------------------------------- round trip

test_a_value_that_was_set_reads_back() {
  wizard_state_set answer.port 8911
  assert_eq "8911" "$(wizard_state_get answer.port)"
}

test_a_key_that_was_never_set_reads_back_as_its_default() {
  assert_eq "127.0.0.1" "$(wizard_state_get answer.bind 127.0.0.1)"
  assert_eq "" "$(wizard_state_get answer.bind)" "no default means the empty string"
}

test_reading_an_absent_key_succeeds() {
  # Callers assign this bare under `set -e`; a non-zero status would be fatal
  # at every single call site.
  wizard_state_get answer.nothing >/dev/null
  assert_status 0 "$?"
}

test_setting_a_key_twice_replaces_rather_than_appends() {
  wizard_state_set answer.port 8911
  wizard_state_set answer.port 9000
  assert_eq "9000" "$(wizard_state_get answer.port)"
  assert_eq "1" "$(grep -c '^answer.port=' "$WIZARD_STATE_FILE")" \
    "the file must hold one line for the key, not a history of it"
}

test_an_unset_key_is_gone_from_the_file_and_from_the_answer() {
  wizard_state_set answer.port 8911
  wizard_state_unset answer.port
  assert_eq "fallback" "$(wizard_state_get answer.port fallback)"
  assert_file_not_contains "$WIZARD_STATE_FILE" "answer.port"
}

test_values_survive_the_characters_a_line_oriented_file_would_eat() {
  wizard_state_set answer.hosts 'a.example,b.example'
  wizard_state_set answer.ntfy 'https://ntfy.example/a/b?x=1&y=2'
  wizard_state_set answer.odd 'back\slash and = sign'
  assert_eq 'a.example,b.example' "$(wizard_state_get answer.hosts)"
  assert_eq 'https://ntfy.example/a/b?x=1&y=2' "$(wizard_state_get answer.ntfy)"
  assert_eq 'back\slash and = sign' "$(wizard_state_get answer.odd)"
}

test_a_value_with_a_newline_in_it_round_trips() {
  # Nothing prompts for one today, but a stored value that could end the line
  # would silently truncate the whole file from that point on.
  local multi='first
second'
  wizard_state_set answer.multi "$multi"
  assert_eq "$multi" "$(wizard_state_get answer.multi)"
  assert_eq "1" "$(grep -c '^answer.multi=' "$WIZARD_STATE_FILE")" \
    "it must occupy exactly one physical line on disk"
}

test_a_second_reader_sees_what_the_first_wrote() {
  wizard_state_set stage.detect done
  ( . "$REPO_ROOT/lib/state.sh"
    WIZARD_STATE_DIR="$TEST_TMPDIR/state"
    WIZARD_STATE_FILE="$WIZARD_STATE_DIR/setup-state"
    wizard_state_load
    assert_eq "done" "$(wizard_state_get stage.detect)" "a fresh shell must see the same state" )
}

# ------------------------------------------------------------ the file itself

test_the_state_file_is_private_to_its_owner() {
  # It holds the answers, and an answer may be a notification URL that is a
  # bearer credential in its own right.
  wizard_state_set answer.ntfy https://ntfy.example/secret-topic
  assert_file_mode 600 "$WIZARD_STATE_FILE"
}

test_the_state_file_is_created_with_its_directory() {
  assert_file_missing "$WIZARD_STATE_DIR" "the sandbox must start without one"
  wizard_state_set run.started 1
  assert_file_exists "$WIZARD_STATE_FILE"
}

test_writing_leaves_no_temporary_file_beside_it() {
  wizard_state_set run.started 1
  wizard_state_set run.count 2
  local leftovers
  leftovers="$(find "$WIZARD_STATE_DIR" -name '*.tmp*')"
  assert_eq "" "$leftovers" \
    "the write is a rename over the real file, not a file left half-written"
}

test_clearing_removes_the_file_and_the_memory_of_it() {
  wizard_state_set stage.detect done
  wizard_state_clear
  assert_file_missing "$WIZARD_STATE_FILE"
  assert_eq "" "$(wizard_state_get stage.detect)"
}

# ------------------------------------------------------------------ keys

test_keys_can_be_listed_by_prefix() {
  wizard_state_set stage.welcome done
  wizard_state_set stage.detect pending
  wizard_state_set answer.port 8911
  local keys
  keys="$(wizard_state_keys stage. | sort)"
  assert_eq "stage.detect
stage.welcome" "$keys"
}

test_a_prefix_can_be_cleared_in_one_call() {
  # This is what a rerun does: forget which stages finished, keep the answers.
  wizard_state_set stage.welcome done
  wizard_state_set step.detect.deps done
  wizard_state_set answer.port 8911
  wizard_state_unset_prefix stage.
  wizard_state_unset_prefix step.
  assert_eq "" "$(wizard_state_get stage.welcome)"
  assert_eq "" "$(wizard_state_get step.detect.deps)"
  assert_eq "8911" "$(wizard_state_get answer.port)" "answers are not run state"
}

test_a_malformed_key_is_refused_as_a_caller_bug() {
  # 2 rather than 1, by the same convention the platform layer uses: this is a
  # mistake in the code, not a fact about the machine.
  wizard_state_set "answer port" x
  assert_status 2 "$?"
  wizard_state_set "answer=port" x
  assert_status 2 "$?"
  wizard_state_set "" x
  assert_status 2 "$?"
}

# ------------------------------------------------------------- degrading

test_a_state_file_full_of_rubbish_is_survived_not_propagated() {
  mkdir -p "$WIZARD_STATE_DIR"
  printf 'this is not a key value pair\n\n=novalue\nanswer.port=8911\n' \
    > "$WIZARD_STATE_FILE"
  wizard_state_load
  assert_eq "8911" "$(wizard_state_get answer.port)" \
    "the lines it can read are still read"
  assert_eq "" "$(wizard_state_get nonsense)"
}

test_no_state_file_at_all_is_an_ordinary_starting_point() {
  assert_file_missing "$WIZARD_STATE_FILE"
  wizard_state_load
  assert_status 0 "$?"
  assert_eq "" "$(wizard_state_keys)"
}

test_state_can_be_told_it_exists() {
  assert_eq "1" "$(wizard_state_is_new && echo 1)" "no file yet means a new run"
  wizard_state_set run.started 1
  wizard_state_load
  assert_eq "" "$(wizard_state_is_new && echo 1)" "once written it is not new"
}
