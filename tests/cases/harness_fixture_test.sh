# Self-tests for the fixture convention, plus a shape check over the shipped
# corpus so that a fixture cannot rot unnoticed.

test_fixture_path_resolves_under_the_fixtures_directory() {
  local path
  path="$(fixture_path os-release/ubuntu-24.04)"
  assert_eq "$FIXTURES_DIR/os-release/ubuntu-24.04" "$path"
  assert_file_exists "$path"
}

test_fixture_cat_returns_the_bytes() {
  local body
  body="$(fixture_cat os-release/ubuntu-24.04)"
  assert_contains "$body" "ID=ubuntu"
  assert_contains "$body" "VERSION_ID="
}

test_a_missing_fixture_complains_loudly_and_yields_an_unusable_path() {
  local path output status
  output="$( (fixture_path os-release/no-such-distro >/dev/null) 2>&1 )"
  path="$(fixture_path os-release/no-such-distro 2>/dev/null)"; status=$?

  assert_status 1 "$status"
  assert_contains "$output" "no such fixture: os-release/no-such-distro"
  assert_contains "$output" "$FIXTURES_DIR/os-release/no-such-distro" \
    "the message must name the path it looked for"
  assert_contains "$output" "ubuntu-24.04" "and list what the category does hold"
  assert_file_missing "$path" "the returned path must not resolve to anything"
}

test_fixture_file_materialises_into_the_sandbox() {
  local path
  path="$(fixture_file os-release/debian-12)"
  case "$path" in
    "$TEST_TMPDIR"/*) : ;;
    *) fail "a materialised fixture must live in the sandbox, got: $path" ;;
  esac
  assert_file_contains "$path" "ID=debian"
  assert_eq "$(fixture_cat os-release/debian-12)" "$(cat "$path")"
}

test_fixture_file_honours_an_explicit_destination() {
  local path
  path="$(fixture_file os-release/debian-12 "$TEST_TMPDIR/etc/os-release")"
  assert_eq "$TEST_TMPDIR/etc/os-release" "$path"
  assert_file_contains "$path" "ID=debian"
}

test_fixture_list_names_the_scenarios_in_a_category() {
  local listing
  listing="$(fixture_list os-release)"
  assert_contains "$listing" "ubuntu-24.04"
  assert_contains "$listing" "debian-12"
  assert_eq "" "$(fixture_list no-such-category)"
}

test_every_shipped_fixture_is_a_non_empty_file_without_an_extension() {
  local path rel
  for path in $(find "$FIXTURES_DIR" -type f | sort); do
    rel="${path#$FIXTURES_DIR/}"
    [ -s "$path" ] || fail "empty fixture: $rel"
    case "${rel##*/}" in
      *.*)
        case "$rel" in
          os-release/*|sw_vers/*|uname/*) : ;;   # version numbers, not extensions
          *) fail "fixture names carry no extension: $rel" ;;
        esac
        ;;
    esac
  done
}

test_the_platform_detection_corpus_is_present() {
  # The eleven onboarding tickets all start from these. Losing one silently
  # would turn their detection tests green for the wrong reason.
  assert_fixture_exists os-release/ubuntu-24.04
  assert_fixture_exists os-release/debian-12
  assert_fixture_exists os-release/fedora-40
  assert_fixture_exists os-release/arch
  assert_fixture_exists os-release/alpine-3.20
  assert_fixture_exists uname/linux-x86_64
  assert_fixture_exists uname/darwin-arm64
  assert_fixture_exists lscpu/x86_64-8core
  assert_fixture_exists free/16gb
  assert_fixture_exists nvidia-smi/rtx-4090
  assert_fixture_exists nvidia-smi/no-devices
  assert_fixture_exists sw_vers/macos-15.1
  assert_fixture_exists system_profiler/apple-m3-24gb
  assert_fixture_exists tailscale/status-online
  assert_fixture_exists tailscale/status-logged-out
  assert_fixture_exists tailscale/status-malformed
}

test_the_tailscale_fixtures_are_shaped_the_way_the_installer_reads_them() {
  require_host_command python3
  stub_real python3
  local dnsname status
  dnsname="$(python3 -c 'import json,sys;print(json.load(open(sys.argv[1]))["Self"]["DNSName"])' \
    "$(fixture_path tailscale/status-online)")"
  assert_matches "$dnsname" '\.$' "the online fixture must keep the trailing dot the installer strips"

  python3 -c 'import json,sys;json.load(open(sys.argv[1]))' \
    "$(fixture_path tailscale/status-logged-out)" >/dev/null 2>&1
  status=$?
  assert_status 0 "$status" "the logged-out fixture must still be valid JSON"

  python3 -c 'import json,sys;json.load(open(sys.argv[1]))' \
    "$(fixture_path tailscale/status-malformed)" >/dev/null 2>&1
  status=$?
  assert_ne 0 "$status" "the malformed fixture must not parse, that is its whole job"
}
