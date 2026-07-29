# Does ayeaye actually answer?
#
# A seam, and a small working one. Health checking is its own ticket; what is
# here is the least dishonest thing this file can do until that lands: try,
# and when it cannot try, say so and report the check as unfinished rather
# than as passed.
#
# It runs last in the service stage, after whatever installed and started
# things, which is what the 80 in the filename is for.
#
# Note what it does *not* do: read the shell variables the service step set.
# A resumed run skips that step, so those variables would hold their defaults
# and this check would quietly decide there was nothing to check. Everything
# one stage tells another goes through the state file and the settings file.

_health_check_step() {
  local url status body bind port started

  started="$(wizard_state_get step.service.unit.started 0)"
  if [ "$started" != 1 ]; then
    wizard_say "ayeaye is not running yet, so there is nothing to check."
    wizard_say "start it, then open the address below on your phone."
    return "$WIZARD_STAGE_SKIP"
  fi

  bind="$(wizard_env_get "$ENV_FILE" AYEAYE_BIND 127.0.0.1)"
  port="$(wizard_env_get "$ENV_FILE" AYEAYE_PORT 8911)"
  url="http://$bind:$port/"

  if ! command -v curl >/dev/null 2>&1; then
    # Not a failure and not a success: the check did not happen. Saying it
    # passed would be the worst answer available.
    wizard_say "cannot check that ayeaye is answering: this computer has no curl."
    wizard_say "open the address below on your phone to find out."
    return "$WIZARD_STAGE_PENDING"
  fi

  wizard_detail "health check: GET $url"
  # Without a key the app is expected to refuse, and a refusal is proof it is
  # listening and doing its job. Anything that is an HTTP answer at all counts.
  if body="$(curl -sS -o /dev/null -w '%{http_code}' --max-time 5 "$url" 2>&1)"; then
    status="$body"
  else
    wizard_detail "$body"
    wizard_say "ayeaye was started but is not answering on $url yet."
    wizard_say "give it a moment and open the address below; if it still does"
    wizard_say "not answer, the log has the details."
    return "$WIZARD_STAGE_PENDING"
  fi

  case "$status" in
    2*|3*|401|403)
      wizard_item ok "ayeaye is answering on $url"
      return "$WIZARD_STAGE_OK"
      ;;
    *)
      wizard_detail "health check: unexpected status $status"
      wizard_say "ayeaye answered on $url, but not in the way it should have."
      return "$WIZARD_STAGE_PENDING"
      ;;
  esac
}

wizard_step service health _health_check_step "Checking that ayeaye answers" optional
