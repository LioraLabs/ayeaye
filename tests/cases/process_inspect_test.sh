# How ayeaye works out which agent is running behind a tmux pane, on both
# platforms it supports.
#
# The behaviour lives in python, so the assertions do too: this file is the
# bridge that puts tests/python/process_inspect.py under `tests/run.sh`, one
# bash test per group of behaviours. That python file names each behaviour
# individually and its output is printed in full when anything here fails.
#
# The backend itself lives in bin/process_inspect.py, because bin/voice-dictate
# has to ask the same platform questions about tmux clients;
# tests/cases/dictate_client_test.sh is that half. Everything here still
# reaches it through bin/ayeaye, which is where it used to live and where it
# still reads as living.

setup() {
  require_host_command python3
  stub_real python3
  PROC_TESTS="$REPO_ROOT/tests/python/process_inspect.py"
}

# run_proc_tests <test-id>... - run those python tests and remember the result.
run_proc_tests() {
  run_script "$PROC_TESTS" "$@"
}

test_the_proc_backend_reproduces_the_pre_refactor_parsing() {
  run_proc_tests LinuxProcTest.test_start_time_is_boot_time_plus_field_22 \
                 LinuxProcTest.test_start_time_keeps_sub_second_resolution \
                 LinuxProcTest.test_start_time_survives_a_parenthesis_in_the_process_name
  assert_status 0 "$RUN_STATUS" "$RUN_OUTPUT"
}

test_the_proc_backend_agrees_with_the_real_proc_on_this_host() {
  case "$OSTYPE" in
    linux*) ;;
    *) skip "the /proc oracle needs a linux host" ;;
  esac
  run_proc_tests LinuxProcTest.test_the_live_proc_parsing_matches_the_reference_implementation
  assert_status 0 "$RUN_STATUS" "$RUN_OUTPUT"
}

test_the_proc_backend_walks_and_reads_a_fake_process_tree() {
  run_proc_tests LinuxProcTest
  assert_status 0 "$RUN_STATUS" "$RUN_OUTPUT"
}

test_a_codex_rollout_is_matched_by_start_time_and_directory() {
  run_proc_tests CodexSessionTest
  assert_status 0 "$RUN_STATUS" "$RUN_OUTPUT"
}

test_the_macos_backend_reads_canned_ps_and_lsof_output() {
  run_proc_tests DarwinProcTest
  assert_status 0 "$RUN_STATUS" "$RUN_OUTPUT"
}

test_a_macos_pane_resolves_its_codex_rollout_with_no_proc_anywhere() {
  run_proc_tests DarwinEndToEndTest
  assert_status 0 "$RUN_STATUS" "$RUN_OUTPUT"
}

test_lstart_and_proc_put_this_process_at_the_same_moment() {
  # The only cross-backend check this host can run. GNU ps is not BSD ps, so
  # it says nothing about the flags -- but the two start times are arithmetic,
  # and they have to agree.
  case "$OSTYPE" in
    linux*) ;;
    *) skip "needs /proc to compare against" ;;
  esac
  require_host_command ps
  stub_real ps
  run_proc_tests BothBackendsTest
  assert_status 0 "$RUN_STATUS" "$RUN_OUTPUT"
}

test_the_backend_is_chosen_by_platform_and_never_raises() {
  run_proc_tests SelectionTest
  assert_status 0 "$RUN_STATUS" "$RUN_OUTPUT"
}

test_the_claude_marker_resolves_a_pane_the_session_file_cannot() {
  run_proc_tests ClaudeMarkerTest
  assert_status 0 "$RUN_STATUS" "$RUN_OUTPUT"
}

test_a_claude_pane_with_no_marker_resolves_through_its_session_file() {
  run_proc_tests ClaudeSessionTest
  assert_status 0 "$RUN_STATUS" "$RUN_OUTPUT"
}

test_a_codex_rollout_held_open_beats_guessing_from_a_start_time() {
  run_proc_tests CodexHeldRolloutTest
  assert_status 0 "$RUN_STATUS" "$RUN_OUTPUT"
}

test_whether_a_pid_is_alive_is_answered_on_both_platforms() {
  run_proc_tests \
    LinuxProcTest.test_exists_is_true_for_a_pid_with_a_directory_under_proc \
    LinuxProcTest.test_exists_is_false_for_a_pid_that_is_not_there \
    DarwinProcTest.test_a_pid_ps_reports_back_exists \
    DarwinProcTest.test_a_pid_ps_says_nothing_about_does_not_exist \
    DarwinProcTest.test_a_pid_is_believed_when_ps_could_not_be_asked_at_all
  assert_status 0 "$RUN_STATUS" "$RUN_OUTPUT"
}

test_the_address_a_process_was_reached_from_is_read_on_both_platforms() {
  # The same answer from two entirely different sources: a NUL-separated
  # environ block on Linux, and the environment ps appends to a command line
  # on macOS. Whichever one is wrong, the user is offered the wrong
  # microphone and nothing says so.
  run_proc_tests \
    LinuxProcTest.test_the_ssh_peer_is_the_first_field_of_ssh_connection \
    LinuxProcTest.test_a_process_with_no_ssh_connection_has_no_peer \
    LinuxProcTest.test_a_variable_whose_name_merely_ends_in_it_is_not_it \
    DarwinProcTest.test_the_ssh_peer_comes_out_of_the_process_environment \
    DarwinProcTest.test_a_client_sitting_at_the_machine_has_no_peer \
    DarwinProcTest.test_a_variable_whose_name_merely_ends_in_it_is_not_it \
    DarwinProcTest.test_the_client_probes_ask_for_what_they_need
  assert_status 0 "$RUN_STATUS" "$RUN_OUTPUT"
}

test_a_missing_tool_or_a_refused_read_never_reaches_the_caller() {
  run_proc_tests \
    LinuxProcTest.test_a_peer_lookup_on_a_process_that_is_gone_is_not_an_error \
    LinuxProcTest.test_a_peer_lookup_that_is_refused_is_not_an_error \
    LinuxProcTest.test_an_undecodable_environment_does_not_cost_the_peer \
    DarwinProcTest.test_an_ssh_connection_with_no_value_is_not_the_next_variable \
    DarwinProcTest.test_a_peer_lookup_with_no_ps_at_all_is_not_an_error
  assert_status 0 "$RUN_STATUS" "$RUN_OUTPUT"
}

test_one_backend_serves_both_commands() {
  # The failure this guards is not a crash. Two copies of a platform backend
  # work on the day they are written, and then one gets a fix.
  run_proc_tests SharedModuleTest
  assert_status 0 "$RUN_STATUS" "$RUN_OUTPUT"
}

test_the_whole_python_suite_passes() {
  run_proc_tests
  assert_status 0 "$RUN_STATUS" "$RUN_OUTPUT"
  assert_not_contains "$RUN_OUTPUT" "FAILED"
}

test_ayeaye_imports_with_proc_taken_away() {
  # An import that reaches for /proc cannot run on a Mac, where there is
  # nothing there to reach for. This host has one, so it is taken away: every
  # path under /proc is made to fail before the module is loaded.
  run_script "$STUB_BIN/python3" -c \
    "import builtins, os, sys
from importlib.machinery import SourceFileLoader
from importlib.util import module_from_spec, spec_from_file_location

# The tree under test is not a place to write to, and loading a .py sibling
# out of bin/ would otherwise leave a __pycache__ in it.
sys.dont_write_bytecode = True

real_open, real_readlink, real_listdir = open, os.readlink, os.listdir
denied = []

def refuse(fn):
    def wrapper(path, *a, **kw):
        if str(path).startswith('/proc'):
            denied.append(str(path))
            raise FileNotFoundError(path)
        return fn(path, *a, **kw)
    return wrapper

builtins.open = refuse(real_open)
os.readlink, os.listdir = refuse(real_readlink), refuse(real_listdir)
os.environ['AYEAYE_TOKEN'] = 't'
# Loaded, not imported, and with a named loader: bin/ayeaye has no .py
# extension for importlib to infer one from. exec_module rather than the
# load_module() that goes away in python 3.15, registered under its name
# first because that is what load_module() did.
path = os.path.join(sys.argv[1], 'bin', 'ayeaye')
spec = spec_from_file_location('a', path, loader=SourceFileLoader('a', path))
m = module_from_spec(spec)
sys.modules['a'] = m
spec.loader.exec_module(m)
builtins.open, os.readlink, os.listdir = real_open, real_readlink, real_listdir
print(type(m._make_process_info('darwin')).__name__)
print('reached for: %s' % denied)" "$REPO_ROOT"
  assert_status 0 "$RUN_STATUS" "$RUN_OUTPUT"
  assert_contains "$RUN_STDOUT" "_DarwinProcessInfo"
  assert_contains "$RUN_STDOUT" "reached for: []"
}
