# The release pipeline: the pieces of "pushing a tag produces every artifact
# and its checksums" that can be held to account without pushing a tag. The
# checksum script is exercised for real; the workflow file is pinned by shape,
# which is weak but is what keeps a row from being deleted without this suite
# noticing.

setup() {
  require_host_command sha256sum
  stub_real sha256sum
  CHECKSUMS="$REPO_ROOT/scripts/release-checksums.sh"
  WORKFLOW="$REPO_ROOT/.github/workflows/release.yml"
  DIST="$TEST_TMPDIR/dist"
  mkdir -p "$DIST"
}

# AYEAYE-59
test_checksums_cover_every_published_name() {
  printf 'one' > "$DIST/ayeaye-v9.9.9-x86_64-unknown-linux-musl"
  printf 'two' > "$DIST/ayeaye-x86_64-unknown-linux-musl"
  printf 'three' > "$DIST/ayeaye-v9.9.9.tar.gz"
  run_script "$CHECKSUMS" \
    "$DIST/ayeaye-v9.9.9-x86_64-unknown-linux-musl" \
    "$DIST/ayeaye-x86_64-unknown-linux-musl" \
    "$DIST/ayeaye-v9.9.9.tar.gz" \
    "$DIST/SHA256SUMS"
  assert_status 0 "$RUN_STATUS" "$RUN_OUTPUT"
  sums="$(cat "$DIST/SHA256SUMS")"
  assert_contains "$sums" "ayeaye-v9.9.9-x86_64-unknown-linux-musl" \
    "the versioned name must be verifiable"
  assert_contains "$sums" "ayeaye-x86_64-unknown-linux-musl" \
    "the versionless alias is a published name too, not a footnote"
  assert_contains "$sums" "ayeaye-v9.9.9.tar.gz"
  assert_not_contains "$sums" "$TEST_TMPDIR" \
    "bare basenames, no paths - the bootstrap looks names up as the release page shows them"
  # sha256 of the literal bytes 'one', computed independently of the script.
  assert_contains "$sums" "7692c3ad3540bb803c020b3aee66cd8887123234ea0c6e7143c0add73ff431ed" \
    "the sums must be of the artifacts, not of anything else"
  assert_eq 3 "$(printf '%s\n' "$sums" | wc -l | tr -d ' ')" \
    "one line per published name, nothing extra"
}

# _workflow - the workflow's text, for shape assertions. A missing file is an
# empty shape, which every assertion then names loudly.
_workflow() { cat "$WORKFLOW" 2>/dev/null || true; }

# _require_pyyaml - the structural assertions parse the YAML rather than grep
# it; skip them, honestly, where the host cannot parse YAML.
_require_pyyaml() {
  require_host_command python3
  stub_real python3
  python3 -c 'import yaml' 2>/dev/null || skip "this host has no pyyaml"
}

# AYEAYE-59
test_the_workflow_triggers_on_version_tags() {
  _require_pyyaml
  [ -f "$WORKFLOW" ]
  assert_status 0 $? "no workflow at .github/workflows/release.yml - pushing a tag builds nothing"
  run_script python3 -c '
import sys, yaml
w = yaml.safe_load(open(sys.argv[1]))
trigger = w.get("on", w.get(True))          # yaml 1.1 reads a bare `on` as True
tags = trigger["push"]["tags"]
sys.exit(0 if "v*" in tags else 1)
' "$WORKFLOW"
  assert_status 0 "$RUN_STATUS" "$RUN_OUTPUT"
}

# AYEAYE-59
test_the_matrix_is_exactly_the_five_rows_the_spec_names() {
  _require_pyyaml
  run_script python3 -c '
import sys, yaml
w = yaml.safe_load(open(sys.argv[1]))
rows = w["jobs"]["build"]["strategy"]["matrix"]["include"]
targets = sorted(r["target"] for r in rows)
want = sorted([
    "x86_64-unknown-linux-musl",     # static musl, both Linux architectures
    "aarch64-unknown-linux-musl",
    "x86_64-apple-darwin",           # both Apple architectures
    "aarch64-apple-darwin",
    "x86_64-unknown-linux-gnu",      # the one NVIDIA row, system C library
])
if targets != want:
    sys.exit("matrix targets are %r, the spec names %r" % (targets, want))
gnu = [r for r in rows if r["target"] == "x86_64-unknown-linux-gnu"]
if "cuda" not in gnu[0].get("features", ""):
    sys.exit("the glibc row is only there for NVIDIA; it must build with cuda")
apple = [r for r in rows if "apple" in r["target"]]
if any("metal" not in r.get("features", "") for r in apple):
    sys.exit("Apple rows must say the acceleration they carry")
' "$WORKFLOW"
  assert_status 0 "$RUN_STATUS" "$RUN_OUTPUT"
}

# AYEAYE-59
test_verify_gates_the_matrix_and_the_matrix_gates_publish() {
  _require_pyyaml
  run_script python3 -c '
import sys, yaml
w = yaml.safe_load(open(sys.argv[1]))
jobs = w["jobs"]
if jobs["build"].get("needs") != "verify":
    sys.exit("a row must not build before verify passes")
if jobs["publish"].get("needs") != "build":
    sys.exit("nothing publishes before every row built")
' "$WORKFLOW"
  assert_status 0 "$RUN_STATUS" "$RUN_OUTPUT"
}

# AYEAYE-59
test_the_verify_job_runs_the_suites_the_linter_and_the_version_gate() {
  text="$(_workflow)"
  assert_contains "$text" "cargo test --workspace --locked" \
    "the verify job runs the test suite"
  assert_contains "$text" "bash tests/run.sh" \
    "both suites, not just the Rust one"
  assert_contains "$text" "-D warnings" \
    "the linter is an error gate, not advice"
  assert_contains "$text" 'check-release-version.sh "$GITHUB_REF_NAME"' \
    "every version claim in the tree must agree with the tag being pushed"
}

# AYEAYE-59
test_every_artifact_is_published_under_both_names() {
  text="$(_workflow)"
  assert_contains "$text" 'ayeaye-$tag-$row' \
    "the versioned name"
  assert_contains "$text" 'ayeaye-$row' \
    "the versionless alias, so latest needs no API call"
  assert_contains "$text" "release-archive.sh" \
    "the source tarball is a release artifact too, built from the tag"
  assert_contains "$text" "release-checksums.sh" \
    "checksums come from the tested script, not an inline sha256sum"
  assert_contains "$text" "SHA256SUMS"
  assert_contains "$text" "release create"
  assert_contains "$text" "verify-tag"
  assert_contains "$text" "clobber" \
    "converge with a release the local cook publish already created"
}

# AYEAYE-59
test_checksums_still_take_a_single_artifact() {
  # The Cookfile's dist recipe calls this with one artifact and one output;
  # going variadic must not break that call.
  printf 'one' > "$DIST/ayeaye-v9.9.9.tar.gz"
  run_script "$CHECKSUMS" "$DIST/ayeaye-v9.9.9.tar.gz" "$DIST/SHA256SUMS"
  assert_status 0 "$RUN_STATUS" "$RUN_OUTPUT"
  assert_eq "7692c3ad3540bb803c020b3aee66cd8887123234ea0c6e7143c0add73ff431ed  ayeaye-v9.9.9.tar.gz" \
    "$(cat "$DIST/SHA256SUMS")"
}
