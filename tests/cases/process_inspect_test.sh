# How ayeaye works out which agent is running behind a tmux pane, on both
# platforms it supports.
#
# The behaviour lives in python, so the assertions do too: this file is the
# bridge that puts tests/python/process_inspect.py under `tests/run.sh`, one
# bash test per group of behaviours. That python file names each behaviour
# individually and its output is printed in full when anything here fails.

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

test_the_backend_is_chosen_by_platform_and_never_raises() {
  run_proc_tests SelectionTest
  assert_status 0 "$RUN_STATUS" "$RUN_OUTPUT"
}

test_the_claude_marker_never_inspects_a_process() {
  run_proc_tests ClaudeMarkerTest
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
m = SourceFileLoader('a', os.path.join(sys.argv[1], 'bin', 'ayeaye')).load_module()
builtins.open, os.readlink, os.listdir = real_open, real_readlink, real_listdir
print(type(m._make_process_info('darwin')).__name__)
print('reached for: %s' % denied)" "$REPO_ROOT"
  assert_status 0 "$RUN_STATUS" "$RUN_OUTPUT"
  assert_contains "$RUN_STDOUT" "_DarwinProcessInfo"
  assert_contains "$RUN_STDOUT" "reached for: []"
}
