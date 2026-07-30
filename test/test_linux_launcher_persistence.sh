#!/usr/bin/env bash
set -euo pipefail

CONTROLLER=${1:?usage: test_linux_launcher_persistence.sh /path/to/csswitch-codex}
FIXTURE="$(mktemp -d)"
trap 'rm -rf "$FIXTURE"' EXIT

fail() { echo "FAIL: $*" >&2; exit 1; }
assert_eq() {
  [ "$1" = "$2" ] || fail "$3: expected '$2', got '$1'"
}
assert_line() {
  grep -Fxq -- "$2" "$1" || fail "$3: missing '$2'"
}

[ -f "$CONTROLLER" ] || fail "controller is not a regular file"
[ ! -L "$CONTROLLER" ] || fail "controller must not be a symlink"

contract="$({
  env \
    -u CSSWITCH_GATEWAY_PORT \
    -u CSSWITCH_GATEWAY_PROXY \
    -u CSSWITCH_GATEWAY_NO_PROXY \
    bash -c '
      set -euo pipefail
      source "$1"
      printf "port=%s\n" "$gateway_port"
      printf "proxy=%s\n" "${gateway_proxy-unset}"
      printf "no_proxy=%s\n" "${gateway_no_proxy-unset}"
    ' _ "$CONTROLLER"
} 2>&1)"
assert_eq "$(sed -n 's/^port=//p' <<<"$contract")" "11535" "default gateway port"
assert_eq "$(sed -n 's/^proxy=//p' <<<"$contract")" "http://127.0.0.1:2999" "default Gateway proxy"
assert_eq "$(sed -n 's/^no_proxy=//p' <<<"$contract")" "127.0.0.1,localhost" "default Gateway no_proxy"

fake_gateway="$FIXTURE/csswitch-gateway"
printf '%s\n' \
  '#!/usr/bin/env bash' \
  'set -euo pipefail' \
  '{' \
  '  printf "arg=%s\n" "$@"' \
  '  for name in HTTP_PROXY HTTPS_PROXY ALL_PROXY http_proxy https_proxy all_proxy NO_PROXY no_proxy; do' \
  '    printf "%s=%s\n" "$name" "${!name-}"' \
  '  done' \
  '} >"${CSSWITCH_TEST_CAPTURE:?}"' >"$fake_gateway"
chmod 0755 "$fake_gateway"

run_scenario() {
  local name="$1" expected_port="$2" expected_proxy="$3" expected_no_proxy="$4"
  shift 4
  local state="$FIXTURE/$name-state" capture="$FIXTURE/$name.capture"
  mkdir -p "$state"

  env \
    -u HTTP_PROXY -u HTTPS_PROXY -u ALL_PROXY \
    -u http_proxy -u https_proxy -u all_proxy \
    -u NO_PROXY -u no_proxy \
    "$@" \
    CSSWITCH_HEADLESS_DIR="$state" \
    CSSWITCH_GATEWAY_BIN="$fake_gateway" \
    CSSWITCH_NOHUP_BIN=/usr/bin/env \
    CSSWITCH_SETSID_BIN=/usr/bin/env \
    CSSWITCH_TEST_CAPTURE="$capture" \
    bash -c '
      set -euo pipefail
      source "$1"
      ensure_private_state
      CSSWITCH_AUTH_TOKEN="$(printf "a%.0s" {1..64})"
      start_gateway
      pid="$(<"$gateway_pid_file")"
      wait "$pid"
    ' _ "$CONTROLLER"

  assert_line "$capture" "arg=--provider" "$name provider flag"
  assert_line "$capture" "arg=codex" "$name provider"
  assert_line "$capture" "arg=--port" "$name port flag"
  assert_line "$capture" "arg=$expected_port" "$name port"
  for name in HTTP_PROXY HTTPS_PROXY ALL_PROXY http_proxy https_proxy all_proxy; do
    assert_line "$capture" "$name=$expected_proxy" "$name proxy"
  done
  for name in NO_PROXY no_proxy; do
    assert_line "$capture" "$name=$expected_no_proxy" "$name bypass"
  done
}

run_scenario \
  default \
  11535 \
  http://127.0.0.1:2999 \
  127.0.0.1,localhost \
  -u CSSWITCH_GATEWAY_PORT \
  -u CSSWITCH_GATEWAY_PROXY \
  -u CSSWITCH_GATEWAY_NO_PROXY

run_scenario \
  override \
  12535 \
  http://127.0.0.1:3999 \
  127.0.0.1,localhost,science.local \
  CSSWITCH_GATEWAY_PORT=12535 \
  CSSWITCH_GATEWAY_PROXY=http://127.0.0.1:3999 \
  CSSWITCH_GATEWAY_NO_PROXY=127.0.0.1,localhost,science.local

echo "Linux launcher persistence tests: PASS"
