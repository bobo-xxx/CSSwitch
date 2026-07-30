# Linux Launcher Persistence Hardening Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Harden this machine's installed `csswitch-codex` launcher so plain commands use Gateway port `11535` and start the Gateway through proxy port `2999`, without changing the portable repository launcher or restarting the healthy runtime.

**Architecture:** Keep `scripts/csswitch-codex` byte-identical and derive a machine-local candidate from the currently installed launcher. Add a reusable candidate contract test, patch only the candidate's port/proxy defaults and `start_gateway` child environment, then back up and atomically replace the installed launcher. Verify the already-running Gateway and Claude Science processes retain their PIDs, executable, proxy environment, listeners, and readiness.

**Tech Stack:** Bash 4+, coreutils (`cp`, `chmod`, `cmp`, `mv`, `sha256sum`, `stat`), `rg`, `ss`, `curl`, Git.

## Global Constraints

- The repository's generic `scripts/csswitch-codex` default remains `11434` and its SHA-256 remains `87894fda3898a54c90ce3baa3c7efaea8e419d89fabbad37c22ffd94f1b3cc17`.
- The installed default Gateway port is exactly `11535`.
- The installed Gateway child receives `HTTP_PROXY`, `HTTPS_PROXY`, `ALL_PROXY`, `http_proxy`, `https_proxy`, and `all_proxy` as exactly `http://127.0.0.1:2999` by default.
- The installed Gateway child receives `NO_PROXY` and `no_proxy` as exactly `127.0.0.1,localhost` by default.
- `CSSWITCH_GATEWAY_PORT`, `CSSWITCH_GATEWAY_PROXY`, and `CSSWITCH_GATEWAY_NO_PROXY` remain explicit overrides.
- Claude Science remains on port `9002`, sandbox port `9003`, and Gateway base port `11535`.
- Provider selection remains Codex-only; do not add Kimi or any provider/model fallback.
- Do not read, print, copy, or record credentials, authentication tokens, bearer paths, raw provider bodies, or account identifiers.
- Do not signal, stop, restart, or replace the running Gateway or Claude Science processes.
- The pre-change installed launcher must be backed up byte-for-byte at `/home/bio-13/.local/bin/csswitch-codex.backup-pre-persistence-20260730` before replacement.
- Use regression-first verification and invoke `verification-before-completion` before claiming success.

## File Structure

- Create `test/test_linux_launcher_persistence.sh`: deterministic machine-local candidate contract; it sources a supplied launcher and exercises the real `start_gateway` function with a fake child.
- Create `scratchpad/launcher-persistence/csswitch-codex.candidate`: ignored deployment candidate copied from the installed launcher; never commit it.
- Modify `/home/bio-13/.local/bin/csswitch-codex`: machine-local installed launcher only, installed atomically after all candidate checks pass.
- Create `/home/bio-13/.local/bin/csswitch-codex.backup-pre-persistence-20260730`: byte-identical rollback copy of the old launcher.
- Modify `.scratch/provider-failure-contract/issues/07-extend-selected-provider-routes.md`: append sanitized installation and verification evidence.
- Planning record: `docs/superpowers/specs/2026-07-30-linux-launcher-persistence-design.md` is marked approved before execution begins.

---

### Task 1: Capture the machine-local launcher contract as a failing test

**Files:**
- Create: `test/test_linux_launcher_persistence.sh`
- Test: `test/test_linux_launcher_persistence.sh`

**Interfaces:**
- Consumes: a launcher path as positional argument `$1`; the launcher must be sourceable and define `start_gateway`.
- Produces: exit `0` plus `Linux launcher persistence tests: PASS` only when defaults, overrides, exact child environment, and Codex-only arguments satisfy the approved contract.

- [ ] **Step 1: Add the deterministic launcher candidate test**

Create `test/test_linux_launcher_persistence.sh` with this complete content and mode `0755`:

```bash
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
```

- [ ] **Step 2: Prove the installed launcher has the expected old contract**

Run:

```bash
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
  ' _ /home/bio-13/.local/bin/csswitch-codex
```

Expected exact output:

```text
port=11434
proxy=unset
no_proxy=unset
```

- [ ] **Step 3: Run the new test and verify RED**

Run:

```bash
bash test/test_linux_launcher_persistence.sh /home/bio-13/.local/bin/csswitch-codex
```

Expected: non-zero exit with this first contract mismatch:

```text
FAIL: default gateway port: expected '11535', got '11434'
```

- [ ] **Step 4: Verify the test script itself is syntactically clean**

Run:

```bash
bash -n test/test_linux_launcher_persistence.sh
```

Expected: exit `0` with no output.

- [ ] **Step 5: Commit the RED test**

```bash
git add test/test_linux_launcher_persistence.sh
git commit -m "test: capture Linux launcher persistence contract"
```

Expected: one commit containing only the new test.

---

### Task 2: Build the minimal candidate and make the contract GREEN

**Files:**
- Create: `scratchpad/launcher-persistence/csswitch-codex.candidate`
- Read only: `scripts/csswitch-codex`
- Test: `test/test_linux_launcher_persistence.sh`
- Test: `test/test_headless_codex.sh`

**Interfaces:**
- Consumes: installed launcher SHA-256 `87894fda3898a54c90ce3baa3c7efaea8e419d89fabbad37c22ffd94f1b3cc17`.
- Produces: candidate SHA-256 `b7fa8c4598740c3bf351298a1902663c3f09be21dba5bc11dac6442f46c91c4e`, mode `0755`, and size `12567` bytes.

- [ ] **Step 1: Gate candidate creation on the known installed source**

Run:

```bash
sha256sum /home/bio-13/.local/bin/csswitch-codex scripts/csswitch-codex
cmp -s /home/bio-13/.local/bin/csswitch-codex scripts/csswitch-codex
```

Expected: both hashes are `87894fda3898a54c90ce3baa3c7efaea8e419d89fabbad37c22ffd94f1b3cc17`, and `cmp` exits `0`. Stop without editing if either assertion differs.

- [ ] **Step 2: Copy the installed launcher into the ignored candidate path**

Run:

```bash
mkdir -p scratchpad/launcher-persistence
cp --preserve=mode /home/bio-13/.local/bin/csswitch-codex scratchpad/launcher-persistence/csswitch-codex.candidate
```

Expected: a regular, non-symlink candidate with mode `0755`.

- [ ] **Step 3: Add only the machine defaults and Gateway-child environment**

Apply this exact patch to `scratchpad/launcher-persistence/csswitch-codex.candidate`:

```diff
-gateway_port=${CSSWITCH_GATEWAY_PORT:-11434}
+gateway_port=${CSSWITCH_GATEWAY_PORT:-11535}
+gateway_proxy=${CSSWITCH_GATEWAY_PROXY:-http://127.0.0.1:2999}
+gateway_no_proxy=${CSSWITCH_GATEWAY_NO_PROXY:-127.0.0.1,localhost}
 science_port=${CSSWITCH_SCIENCE_PORT:-9002}
 sandbox_port=${CSSWITCH_SANDBOX_PORT:-9003}
@@
   : >"$gateway_log"
   chmod 600 "$gateway_log"
   CSSWITCH_AUTH_TOKEN="$CSSWITCH_AUTH_TOKEN" \
+  HTTP_PROXY="$gateway_proxy" \
+  HTTPS_PROXY="$gateway_proxy" \
+  ALL_PROXY="$gateway_proxy" \
+  http_proxy="$gateway_proxy" \
+  https_proxy="$gateway_proxy" \
+  all_proxy="$gateway_proxy" \
+  NO_PROXY="$gateway_no_proxy" \
+  no_proxy="$gateway_no_proxy" \
     "$nohup_bin" "$setsid_bin" "$gateway_bin" --provider codex --port "$gateway_port" \
```

- [ ] **Step 4: Run the focused candidate checks and verify GREEN**

Run:

```bash
bash -n scratchpad/launcher-persistence/csswitch-codex.candidate
bash test/test_linux_launcher_persistence.sh scratchpad/launcher-persistence/csswitch-codex.candidate
```

Expected exact test output:

```text
Linux launcher persistence tests: PASS
```

- [ ] **Step 5: Prove the portable repository launcher is unchanged and healthy**

Run:

```bash
sha256sum scripts/csswitch-codex
bash test/test_headless_codex.sh
```

Expected hash: `87894fda3898a54c90ce3baa3c7efaea8e419d89fabbad37c22ffd94f1b3cc17`.

Expected test output:

```text
headless codex controller tests: PASS
```

- [ ] **Step 6: Review the exact candidate diff and safety contract**

Run:

```bash
diff -u /home/bio-13/.local/bin/csswitch-codex scratchpad/launcher-persistence/csswitch-codex.candidate
sha256sum scratchpad/launcher-persistence/csswitch-codex.candidate
stat -c 'mode=%a uid=%u gid=%g size=%s' scratchpad/launcher-persistence/csswitch-codex.candidate
rg -ni 'kimi|pkill[[:space:]]+-f|killall|watchdog|systemctl|sk-[A-Za-z0-9]{8,}|Bearer[[:space:]]+[A-Za-z0-9_-]{16,}' scratchpad/launcher-persistence/csswitch-codex.candidate
```

Expected: the diff contains only Step 3's changes; SHA-256 is `b7fa8c4598740c3bf351298a1902663c3f09be21dba5bc11dac6442f46c91c4e`; mode is `755`; size is `12567`; `rg` exits `1` with no matches.

---

### Task 3: Back up and atomically install the verified candidate

**Files:**
- Read: `/home/bio-13/.csswitch/headless/gateway.pid`
- Read: `/home/bio-13/.csswitch/headless/science.pid`
- Create: `/home/bio-13/.local/bin/csswitch-codex.backup-pre-persistence-20260730`
- Create then rename: `/home/bio-13/.local/bin/csswitch-codex.stage-persistence-20260730`
- Modify atomically: `/home/bio-13/.local/bin/csswitch-codex`

**Interfaces:**
- Consumes: verified candidate hash `b7fa8c4598740c3bf351298a1902663c3f09be21dba5bc11dac6442f46c91c4e`.
- Produces: installed launcher with the same hash and a rollback backup with old hash `87894fda3898a54c90ce3baa3c7efaea8e419d89fabbad37c22ffd94f1b3cc17`.

- [ ] **Step 1: Capture sanitized pre-install runtime identity without signalling processes**

Run:

```bash
gateway_pid="$(< /home/bio-13/.csswitch/headless/gateway.pid)"
science_pid="$(< /home/bio-13/.csswitch/headless/science.pid)"
printf 'gateway_pid=%s\nscience_pid=%s\n' "$gateway_pid" "$science_pid"
sha256sum "/proc/$gateway_pid/exe"
tr '\0' '\n' <"/proc/$gateway_pid/environ" | \
  rg '^(HTTP_PROXY|HTTPS_PROXY|ALL_PROXY|http_proxy|https_proxy|all_proxy|NO_PROXY|no_proxy)=' | \
  sort
ss -H -ltnp 'sport = :11535'
ss -H -ltnp 'sport = :9002'
```

Expected: both PIDs are live and own their respective ports; the Gateway executable hash is `300e65867fedd68ed42aa07839fad005d3e20c0f70827332a055ee31bf753d49`; six proxy values use `http://127.0.0.1:2999`; both bypass values contain `127.0.0.1` and `localhost`. Keep the two PID values in the execution record for the post-install equality check; do not print any other environment variables.

- [ ] **Step 2: Create and verify the byte-identical rollback backup**

First require that the fixed backup path is absent:

```bash
test ! -e /home/bio-13/.local/bin/csswitch-codex.backup-pre-persistence-20260730
```

Then copy and verify:

```bash
cp --preserve=mode,ownership,timestamps \
  /home/bio-13/.local/bin/csswitch-codex \
  /home/bio-13/.local/bin/csswitch-codex.backup-pre-persistence-20260730
cmp -s \
  /home/bio-13/.local/bin/csswitch-codex \
  /home/bio-13/.local/bin/csswitch-codex.backup-pre-persistence-20260730
sha256sum /home/bio-13/.local/bin/csswitch-codex.backup-pre-persistence-20260730
stat -c 'mode=%a uid=%u gid=%g size=%s' \
  /home/bio-13/.local/bin/csswitch-codex.backup-pre-persistence-20260730
```

Expected: `cmp` exits `0`; backup hash is `87894fda3898a54c90ce3baa3c7efaea8e419d89fabbad37c22ffd94f1b3cc17`; mode is `755`; size is `12179`.

- [ ] **Step 3: Stage the candidate beside the installed launcher**

Run:

```bash
cp scratchpad/launcher-persistence/csswitch-codex.candidate \
  /home/bio-13/.local/bin/csswitch-codex.stage-persistence-20260730
chmod 0755 /home/bio-13/.local/bin/csswitch-codex.stage-persistence-20260730
sha256sum /home/bio-13/.local/bin/csswitch-codex.stage-persistence-20260730
bash -n /home/bio-13/.local/bin/csswitch-codex.stage-persistence-20260730
```

Expected staged hash: `b7fa8c4598740c3bf351298a1902663c3f09be21dba5bc11dac6442f46c91c4e` and syntax exit `0`.

- [ ] **Step 4: Atomically replace only the installed launcher**

Run:

```bash
mv \
  /home/bio-13/.local/bin/csswitch-codex.stage-persistence-20260730 \
  /home/bio-13/.local/bin/csswitch-codex
```

Expected: rename exits `0`. Do not invoke `start`, `stop`, `kill`, or any service manager.

- [ ] **Step 5: Verify the installed bytes immediately or restore atomically**

Run:

```bash
sha256sum /home/bio-13/.local/bin/csswitch-codex
stat -c 'mode=%a uid=%u gid=%g size=%s' /home/bio-13/.local/bin/csswitch-codex
bash -n /home/bio-13/.local/bin/csswitch-codex
```

Expected: hash `b7fa8c4598740c3bf351298a1902663c3f09be21dba5bc11dac6442f46c91c4e`, mode `755`, size `12567`, and syntax exit `0`.

If any assertion from Steps 3–5 fails, restore with these exact commands and stop:

```bash
cp /home/bio-13/.local/bin/csswitch-codex.backup-pre-persistence-20260730 \
  /home/bio-13/.local/bin/csswitch-codex.stage-persistence-rollback-20260730
chmod 0755 /home/bio-13/.local/bin/csswitch-codex.stage-persistence-rollback-20260730
mv \
  /home/bio-13/.local/bin/csswitch-codex.stage-persistence-rollback-20260730 \
  /home/bio-13/.local/bin/csswitch-codex
sha256sum /home/bio-13/.local/bin/csswitch-codex
```

Expected rollback hash: `87894fda3898a54c90ce3baa3c7efaea8e419d89fabbad37c22ffd94f1b3cc17`.

---

### Task 4: Verify unchanged runtime state and publish sanitized evidence

**Files:**
- Modify: `.scratch/provider-failure-contract/issues/07-extend-selected-provider-routes.md:28`
- Delete after verification: `scratchpad/launcher-persistence/csswitch-codex.candidate`
- Test: `test/test_linux_launcher_persistence.sh`
- Test: `test/test_headless_codex.sh`

**Interfaces:**
- Consumes: installed launcher hash `b7fa8c4598740c3bf351298a1902663c3f09be21dba5bc11dac6442f46c91c4e` and the pre-install Gateway/Science PID values.
- Produces: plain launcher status at `auth=ready`, `gateway=running health=ready catalog=ready`, `science=running listener=ready`, plus a committed and pushed sanitized verification record.

- [ ] **Step 1: Run the installed and portable launcher test gates**

Run:

```bash
bash test/test_linux_launcher_persistence.sh /home/bio-13/.local/bin/csswitch-codex
bash test/test_headless_codex.sh
sha256sum scripts/csswitch-codex /home/bio-13/.local/bin/csswitch-codex
```

Expected:

```text
Linux launcher persistence tests: PASS
headless codex controller tests: PASS
87894fda3898a54c90ce3baa3c7efaea8e419d89fabbad37c22ffd94f1b3cc17  scripts/csswitch-codex
b7fa8c4598740c3bf351298a1902663c3f09be21dba5bc11dac6442f46c91c4e  /home/bio-13/.local/bin/csswitch-codex
```

- [ ] **Step 2: Verify plain status recognizes the existing runtime**

Run with no `CSSWITCH_GATEWAY_PORT`, proxy, or no-proxy overrides:

```bash
env \
  -u CSSWITCH_GATEWAY_PORT \
  -u CSSWITCH_GATEWAY_PROXY \
  -u CSSWITCH_GATEWAY_NO_PROXY \
  /home/bio-13/.local/bin/csswitch-codex status
```

Expected exact output:

```text
auth=ready
gateway=running health=ready catalog=ready
science=running listener=ready
```

- [ ] **Step 3: Prove no runtime process or environment changed**

Re-read the two PID files and compare them with Task 3 Step 1's values, then run:

```bash
gateway_pid="$(< /home/bio-13/.csswitch/headless/gateway.pid)"
science_pid="$(< /home/bio-13/.csswitch/headless/science.pid)"
printf 'gateway_pid=%s\nscience_pid=%s\n' "$gateway_pid" "$science_pid"
sha256sum "/proc/$gateway_pid/exe"
tr '\0' '\n' <"/proc/$gateway_pid/environ" | \
  rg '^(HTTP_PROXY|HTTPS_PROXY|ALL_PROXY|http_proxy|https_proxy|all_proxy|NO_PROXY|no_proxy)=' | \
  sort
ss -H -ltnp 'sport = :11535'
ss -H -ltnp 'sport = :9002'
curl --silent --show-error --max-time 3 \
  --dump-header - --output /dev/null http://127.0.0.1:9002/
```

Expected: both PID values equal their pre-install values; Gateway executable hash remains `300e65867fedd68ed42aa07839fad005d3e20c0f70827332a055ee31bf753d49`; proxy values remain on loopback `2999`; both listeners retain the same owners; Science returns its expected unauthenticated-root `401` boundary.

- [ ] **Step 4: Append the sanitized launcher-persistence evidence**

Append this exact section to `.scratch/provider-failure-contract/issues/07-extend-selected-provider-routes.md`:

```markdown

## Installed Linux launcher persistence

On 2026-07-30, the machine-local `/home/bio-13/.local/bin/csswitch-codex` launcher was hardened without restarting or signalling the running Gateway or Claude Science. The byte-identical rollback backup is `/home/bio-13/.local/bin/csswitch-codex.backup-pre-persistence-20260730` with SHA-256 `87894fda3898a54c90ce3baa3c7efaea8e419d89fabbad37c22ffd94f1b3cc17`, mode `0755`, and size `12179` bytes. The installed launcher has SHA-256 `b7fa8c4598740c3bf351298a1902663c3f09be21dba5bc11dac6442f46c91c4e`, mode `0755`, and size `12567` bytes.

A regression-first fake-child contract proved the old launcher defaulted to port `11434` and owned no Gateway proxy default. The installed launcher now defaults to Gateway port `11535`, passes proxy `http://127.0.0.1:2999` through all six uppercase/lowercase proxy variables, passes `127.0.0.1,localhost` through both no-proxy variables, and honors explicit port, proxy, and no-proxy overrides. The portable repository launcher remains byte-identical with its generic `11434` default.

Plain launcher status reported Codex authentication, Gateway health, dynamic catalog, and Claude Science ready. Gateway and Science retained their pre-install PIDs and listeners; the running Gateway executable retained SHA-256 `300e65867fedd68ed42aa07839fad005d3e20c0f70827332a055ee31bf753d49` and its proxy environment on loopback port `2999`. No provider, model, credential, profile, fallback, Science data, tunnel, firewall, or runtime process was changed. This proves Linux launcher restart configuration without performing a restart; live API-key failure induction and real macOS verification remain pending.
```

- [ ] **Step 5: Invoke completion verification and commit the evidence**

Use the `verification-before-completion` skill, rerun its required fresh checks, then run:

```bash
git diff --check
git diff -- .scratch/provider-failure-contract/issues/07-extend-selected-provider-routes.md
git status --short
git add .scratch/provider-failure-contract/issues/07-extend-selected-provider-routes.md
git commit -m "docs: record Linux launcher persistence verification"
```

Expected: the commit contains only the sanitized Ticket 07 evidence update; the approved design/test commit remains separate.

- [ ] **Step 6: Clean the ignored candidate and prepare the reviewed branch handoff**

Delete `scratchpad/launcher-persistence/csswitch-codex.candidate` with `apply_patch`, remove the now-empty `scratchpad/launcher-persistence` directory, then run:

```bash
git status --short --branch
git rev-parse HEAD
```

Expected: a clean `launcher-persistence-hardening` branch. The rollback backup remains installed and is not deleted. After the final whole-branch review, use the `finishing-a-development-branch` workflow to integrate this branch into `linux-headless-oauth` and push the durable commits to `fork/linux-headless-oauth`.
