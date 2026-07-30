#!/usr/bin/env bash
set -euo pipefail

CONTROLLER=${1:?usage: test_linux_launcher_persistence.sh /path/to/csswitch-codex}
FIXTURE="$(mktemp -d)"
trap 'rm -rf "$FIXTURE"' EXIT

fixture_home="$FIXTURE/home"
contract_state="$FIXTURE/contract-state"
fake_gateway="$FIXTURE/csswitch-gateway"
mkdir -p "$fixture_home" "$contract_state"

PROXY_ENV_NAMES=(HTTP_PROXY HTTPS_PROXY ALL_PROXY http_proxy https_proxy all_proxy)
NO_PROXY_ENV_NAMES=(NO_PROXY no_proxy)
GATEWAY_ENV_NAMES=("${PROXY_ENV_NAMES[@]}" "${NO_PROXY_ENV_NAMES[@]}")
GATEWAY_ENV_UNSET_ARGS=()
for env_name in "${GATEWAY_ENV_NAMES[@]}"; do
  GATEWAY_ENV_UNSET_ARGS+=(-u "$env_name")
done

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
    -u BASH_ENV \
    -u ENV \
    -u CSSWITCH_GATEWAY_PORT \
    -u CSSWITCH_GATEWAY_PROXY \
    -u CSSWITCH_GATEWAY_NO_PROXY \
    HOME="$fixture_home" \
    CSSWITCH_HEADLESS_DIR="$contract_state" \
    CSSWITCH_GATEWAY_BIN="$fake_gateway" \
    CSSWITCH_NOHUP_BIN=/usr/bin/env \
    CSSWITCH_SETSID_BIN=/usr/bin/env \
    bash --noprofile --norc -c '
      set -euo pipefail
      source "$1"
      printf "port=%s\n" "$gateway_port"
      printf "proxy=%s\n" "${gateway_proxy-unset}"
      printf "no_proxy=%s\n" "${gateway_no_proxy-unset}"
      printf "state=%s\n" "$state_dir"
      printf "gateway_bin=%s\n" "$gateway_bin"
      printf "nohup_bin=%s\n" "$nohup_bin"
      printf "setsid_bin=%s\n" "$setsid_bin"
    ' _ "$CONTROLLER"
} 2>&1)"
assert_eq "$(sed -n 's/^port=//p' <<<"$contract")" "11535" "default gateway port"
assert_eq "$(sed -n 's/^proxy=//p' <<<"$contract")" "http://127.0.0.1:2999" "default Gateway proxy"
assert_eq "$(sed -n 's/^no_proxy=//p' <<<"$contract")" "127.0.0.1,localhost" "default Gateway no_proxy"
assert_eq "$(sed -n 's/^state=//p' <<<"$contract")" "$contract_state" "contract state directory"
assert_eq "$(sed -n 's/^gateway_bin=//p' <<<"$contract")" "$fake_gateway" "contract Gateway binary"
assert_eq "$(sed -n 's/^nohup_bin=//p' <<<"$contract")" "/usr/bin/env" "contract nohup binary"
assert_eq "$(sed -n 's/^setsid_bin=//p' <<<"$contract")" "/usr/bin/env" "contract setsid binary"

{
  printf '%s\n' \
    '#!/usr/bin/env bash' \
    'set -euo pipefail' \
    '{' \
    '  printf "argc=%d\n" "$#"' \
    '  arg_index=0' \
    '  for arg_value in "$@"; do' \
    '    printf "arg[%d]=%q\n" "$arg_index" "$arg_value"' \
    '    arg_index=$((arg_index + 1))' \
    '  done'
  printf '  for env_name in'
  printf ' %q' "${GATEWAY_ENV_NAMES[@]}"
  printf '; do\n'
  printf '%s\n' \
    '    printf "%s=%s\n" "$env_name" "${!env_name-}"' \
    '  done' \
    '} >"${CSSWITCH_TEST_CAPTURE:?}"'
} >"$fake_gateway"
chmod 0755 "$fake_gateway"

run_scenario() {
  local scenario_name="$1" expected_port="$2" expected_proxy="$3" expected_no_proxy="$4"
  shift 4
  local state="$FIXTURE/$scenario_name-state" capture="$FIXTURE/$scenario_name.capture"
  local expected_args=(--provider codex --port "$expected_port")
  local expected_arg_vector actual_arg_vector
  mkdir -p "$state"

  env \
    -u BASH_ENV \
    -u ENV \
    "${GATEWAY_ENV_UNSET_ARGS[@]}" \
    "$@" \
    HOME="$fixture_home" \
    CSSWITCH_HEADLESS_DIR="$state" \
    CSSWITCH_GATEWAY_BIN="$fake_gateway" \
    CSSWITCH_NOHUP_BIN=/usr/bin/env \
    CSSWITCH_SETSID_BIN=/usr/bin/env \
    CSSWITCH_TEST_CAPTURE="$capture" \
    bash --noprofile --norc -c '
      set -euo pipefail
      assert_selected_path() {
        [ "$1" = "$2" ] || {
          printf "FAIL: %s selection: expected '%s', got '%s'\n" "$3" "$2" "$1" >&2
          exit 1
        }
      }
      source "$1"
      assert_selected_path "$gateway_bin" "$2" "Gateway binary"
      assert_selected_path "$state_dir" "$3" "state directory"
      assert_selected_path "$nohup_bin" "$4" "nohup binary"
      assert_selected_path "$setsid_bin" "$5" "setsid binary"
      ensure_private_state
      CSSWITCH_AUTH_TOKEN="$(printf "a%.0s" {1..64})"
      start_gateway
      pid="$(<"$gateway_pid_file")"
      wait "$pid"
    ' _ "$CONTROLLER" "$fake_gateway" "$state" /usr/bin/env /usr/bin/env

  expected_arg_vector="$(
    printf 'argc=%d\n' "${#expected_args[@]}"
    for arg_index in "${!expected_args[@]}"; do
      printf 'arg[%d]=%q\n' "$arg_index" "${expected_args[$arg_index]}"
    done
  )"
  actual_arg_vector="$(
    sed -n \
      -e '/^argc=/p' \
      -e '/^arg\[[0-9][0-9]*\]=/p' \
      "$capture"
  )"
  assert_eq "$actual_arg_vector" "$expected_arg_vector" "$scenario_name exact argument vector"
  for env_name in "${PROXY_ENV_NAMES[@]}"; do
    assert_line "$capture" "$env_name=$expected_proxy" "$scenario_name $env_name proxy"
  done
  for env_name in "${NO_PROXY_ENV_NAMES[@]}"; do
    assert_line "$capture" "$env_name=$expected_no_proxy" "$scenario_name $env_name bypass"
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
