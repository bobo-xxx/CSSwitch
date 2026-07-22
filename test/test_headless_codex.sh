#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CONTROLLER="$ROOT/scripts/csswitch-codex"
FIXTURE="$(mktemp -d)"
trap 'rm -rf "$FIXTURE"' EXIT

fail() { echo "FAIL: $*" >&2; exit 1; }
assert_eq() { [ "$1" = "$2" ] || fail "expected '$2', got '$1'"; }

[ -x "$CONTROLLER" ] || fail "missing executable controller"

export HOME="$FIXTURE/home"
export CSSWITCH_HEADLESS_DIR="$HOME/.csswitch/headless"
export CSSWITCH_GATEWAY_BIN="$FIXTURE/csswitch-gateway"
export CSSWITCH_SCIENCE_BIN="$FIXTURE/claude-science"
export CSSWITCH_SS_BIN="$FIXTURE/ss"
export CSSWITCH_SCIENCE_DATA_DIR="$FIXTURE/science-data"
export CSSWITCH_PROC_ROOT="$FIXTURE/proc"
mkdir -p "$HOME" "$CSSWITCH_SCIENCE_DATA_DIR" "$CSSWITCH_PROC_ROOT"
printf '#!/usr/bin/env bash\n[ -n "${LISTENER_PID:-}" ] && echo "LISTEN users:((\\"fixture\\",pid=$LISTENER_PID,fd=3))"\n' >"$CSSWITCH_SS_BIN"
chmod 0755 "$CSSWITCH_SS_BIN"
: >"$CSSWITCH_GATEWAY_BIN"
: >"$CSSWITCH_SCIENCE_BIN"
chmod 0755 "$CSSWITCH_GATEWAY_BIN" "$CSSWITCH_SCIENCE_BIN"

# shellcheck source=../scripts/csswitch-codex
source "$CONTROLLER"

status_runtime >/dev/null
[ ! -e "$CSSWITCH_HEADLESS_DIR/runtime.env" ] || fail "status created a runtime secret"

ensure_private_state
load_or_create_secret
assert_eq "$(stat -c %a "$CSSWITCH_HEADLESS_DIR")" "700"
assert_eq "$(stat -c %a "$CSSWITCH_HEADLESS_DIR/runtime.env")" "600"
[[ "$CSSWITCH_AUTH_TOKEN" =~ ^[0-9a-f]{64}$ ]] || fail "runtime token shape"

pid=4242
mkdir -p "$CSSWITCH_PROC_ROOT/$pid"
printf 'Uid:\t%s\t%s\t%s\t%s\n' "$(id -u)" "$(id -u)" "$(id -u)" "$(id -u)" >"$CSSWITCH_PROC_ROOT/$pid/status"
printf '%s\0' "$CSSWITCH_GATEWAY_BIN" --provider codex --port 11434 >"$CSSWITCH_PROC_ROOT/$pid/cmdline"
ln -s "$CSSWITCH_GATEWAY_BIN" "$CSSWITCH_PROC_ROOT/$pid/exe"
export LISTENER_PID=$pid
verify_process gateway "$pid" 11434
if assert_port_available gateway 11434 "$CSSWITCH_HEADLESS_DIR/missing.pid" >/dev/null 2>&1; then
  fail "unknown listener owner was accepted"
fi
printf '%s\n' 9999 >"$CSSWITCH_HEADLESS_DIR/gateway.pid"
chmod 600 "$CSSWITCH_HEADLESS_DIR/gateway.pid"
if assert_port_available gateway 11434 "$CSSWITCH_HEADLESS_DIR/gateway.pid" >/dev/null 2>&1; then
  fail "mismatched PID record was accepted"
fi
rm -f "$CSSWITCH_HEADLESS_DIR/gateway.pid"
if stop_owned_gateway >/dev/null 2>&1; then
  fail "stop accepted an unknown listener owner"
fi
unset LISTENER_PID

events="$FIXTURE/events"
: >"$events"
ensure_private_state() { echo state >>"$events"; }
load_or_create_secret() { CSSWITCH_AUTH_TOKEN="$(printf 'a%.0s' {1..64})"; echo secret >>"$events"; }
require_runtime_inputs() { echo inputs >>"$events"; }
require_auth_ready() { echo auth >>"$events"; }
assert_start_ports_available() { echo ports >>"$events"; }
start_gateway() { gateway_started=1; echo gateway >>"$events"; }
wait_gateway_health() { echo health >>"$events"; }
require_dynamic_catalog() { echo catalog >>"$events"; }
replace_science() { echo science >>"$events"; }
start_runtime >/dev/null
assert_eq "$(tr '\n' ' ' <"$events")" "state secret inputs auth ports gateway health catalog science "

: >"$events"
require_dynamic_catalog() { echo catalog >>"$events"; return 1; }
stop_owned_gateway() { echo gateway-stop >>"$events"; gateway_started=0; }
if start_runtime >/dev/null 2>&1; then fail "catalog failure was accepted"; fi
assert_eq "$(tr '\n' ' ' <"$events")" "state secret inputs auth ports gateway health catalog gateway-stop "

status_output="$({
  auth_status_line() { echo 'auth=ready'; }
  gateway_status_line() { echo 'gateway=stopped health=unavailable catalog=unavailable'; }
  science_status_line() { echo 'science=stopped listener=absent'; }
  status_runtime
} 2>&1)"
if grep -Eq 'https://|CSSWITCH_AUTH_TOKEN|/v1/messages|[0-9a-f]{64}' <<<"$status_output"; then
  fail "status output exposed bearer material"
fi

if grep -Eiq 'kimi|pkill[[:space:]]+-f|killall|watchdog|systemctl' "$CONTROLLER"; then
  fail "controller contains forbidden fallback or broad process control"
fi

echo "headless codex controller tests: PASS"
