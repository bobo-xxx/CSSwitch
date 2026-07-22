# Parallel Kimi Coding Plan + Claude Science Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a second, fully isolated Linux headless CSSwitch Gateway and Claude Science instance backed by a Kimi Coding Plan subscription, while leaving the running Codex Gateway and Science instance unchanged.

**Architecture:** Keep one provider per Gateway. Add a distinct `kimi-code` preset for Kimi's Anthropic-compatible Coding Plan API, then run it behind a second Gateway on port `12434` and a second Science daemon on ports `9102`/`9103`. The second controller owns a separate state directory, path secret, Kimi API-key file, Science data directory, Science config, PID records, and logs; it never reads, rewrites, stops, or adopts the Codex controller's state or processes.

**Tech Stack:** Rust Gateway, Tauri Rust control-plane catalogs, Bash headless controller, Claude Science `0.1.15-dev.20260701`, Anthropic-compatible Kimi Code API, existing shell/Rust integration tests.

## Global Constraints

- This is a future plan. Do not execute any task until the user explicitly authorizes implementation.
- Existing Codex runtime remains on Gateway `11434`, Science `9002`, preview `9003`, state `~/.csswitch/headless`, and its current Science data directory.
- New Kimi defaults are Gateway `12434`, Science `9102`, preview `9103`, state `~/.csswitch/headless-kimi-code`, Science data `~/.claude-science-kimi-code`, and config `~/.claude-science-kimi-code/config.toml`.
- Kimi Coding Plan base URL is exactly `https://api.kimi.com/coding/`; the effective Anthropic endpoint is `https://api.kimi.com/coding/v1/messages`.
- Kimi Coding Plan credentials come from the Kimi Code Console and are not interchangeable with Moonshot Open Platform keys.
- Supported API model IDs are `k3`, `kimi-for-coding`, and `kimi-for-coding-highspeed`. Never send `k3[1m]` to the API.
- Preserve the existing `kimi` preset for Moonshot Platform users; add a new `kimi-code` preset instead of silently changing billing systems.
- All API keys, path secrets, generated login URLs, response bodies, account identifiers, prompts, and conversation history are private and must not appear in logs, tests, status output, commits, or terminal transcripts.
- Kimi and Codex may share the outbound proxy `http://127.0.0.1:2999`; both Gateway processes must have `HTTP_PROXY`, `HTTPS_PROXY`, `http_proxy`, and `https_proxy` set to that URL.
- Both stacks use `NO_PROXY=no_proxy=127.0.0.1,localhost,::1` for local Science-to-Gateway traffic.
- Do not add fallback routing between Kimi and Codex. A Kimi failure fails the Kimi request and does not consume Codex quota.
- Do not use broad process controls (`pkill -f`, `killall`, unscoped `systemctl`) or adopt unknown listeners.
- Keep Claude Science bound to `127.0.0.1`; collaborators connect through the existing tunnel/reverse-proxy practice and receive only a single-use `claude-science url` for the Kimi data directory.
- Use TDD for every behavior change and make one local commit per task. Do not push without separate authorization.
- Official compatibility references: <https://www.kimi.com/code/docs/en/> and <https://www.kimi.com/code/docs/en/kimi-code/models.html>.

---

## File Structure

- `catalog/provider-contracts.v1.json`: add a contract whose identity distinguishes Kimi Coding Plan from Moonshot Platform relay traffic.
- `catalog/model-presets.v1.json`: add the three official Kimi Coding Plan API model IDs and safe default role bindings.
- `desktop/src-tauri/src/templates.rs`: expose a separate editable `kimi-code` profile template with the correct subscription endpoint.
- `desktop/gateway/src/config.rs`: prove that `/coding/` joins to `/coding/v1/messages` and `/coding/v1/models` without duplicated segments.
- `desktop/gateway/src/anthropic_compat.rs`: select Kimi protocol normalization from the trusted provider contract, not from a model-name substring.
- `desktop/gateway/src/server.rs`: apply Kimi stream filtering to `k3` and `kimi-for-coding*` requests through the explicit compatibility flag.
- `scripts/csswitch-kimi-code`: own and manage only the isolated Kimi Gateway and Science processes.
- `test/test_headless_kimi_code.sh`: test state isolation, exact process ownership, proxy environment, endpoint/model policy, lifecycle, and redacted output.
- `test/run-scripts.sh`: include the new controller test.
- `docs/features/kimi-code-science-bridge.md`: document setup, operation, sharing boundary, validation, and rollback.

---

### Task 1: Add a distinct Kimi Coding Plan catalog identity

**Files:**
- Modify: `catalog/provider-contracts.v1.json`
- Modify: `catalog/model-presets.v1.json`
- Modify: `desktop/src-tauri/src/templates.rs`
- Test: `desktop/src-tauri/src/templates.rs`
- Test: `desktop/src-tauri/src/provider_contracts.rs`

**Interfaces:**
- Consumes: existing provider-contract, model-preset, and template loaders.
- Produces: template ID `kimi-code`, preset ID `kimi-code`, and contract ID `kimi-code-anthropic-relay` using adapter `relay`.

- [ ] **Step 1: Write failing template and contract tests**

Add assertions alongside the existing Kimi tests:

```rust
let template = by_id("kimi-code").expect("Kimi Code template");
assert_eq!(template.api_format, "anthropic");
assert_eq!(template.base_url, "https://api.kimi.com/coding/");
assert_eq!(template.preset_catalog_id, Some("kimi-code"));

let contract = crate::provider_contracts::contract_for("kimi-code", "anthropic")
    .expect("Kimi Code provider contract");
assert_eq!(contract.id, "kimi-code-anthropic-relay");
assert_eq!(contract.adapter, "relay");
assert_eq!(contract.thinking_policy, "enabled");

let preset = crate::model_catalog::preset("kimi-code").expect("Kimi Code preset");
assert_eq!(preset.default_upstream_model, "kimi-for-coding");
assert_eq!(
    preset.models.iter().map(|model| model.upstream_model.as_str()).collect::<Vec<_>>(),
    vec!["k3", "kimi-for-coding", "kimi-for-coding-highspeed"]
);
```

- [ ] **Step 2: Run the focused tests and confirm RED**

Run:

```bash
cargo test --manifest-path desktop/src-tauri/Cargo.toml templates::tests --lib
cargo test --manifest-path desktop/src-tauri/Cargo.toml provider_contracts::tests --lib
```

Expected: failures reporting that `kimi-code` is absent.

- [ ] **Step 3: Add the provider contract**

Add this contract without changing the existing `kimi-anthropic-relay` entry:

```json
{
  "id": "kimi-code-anthropic-relay",
  "template_ids": ["kimi-code"],
  "api_formats": ["anthropic"],
  "adapter": "relay",
  "auth_mode": "api_key",
  "auth_scheme": "anthropic_dual",
  "credential_sources": ["api_key"],
  "default_credential_source": "api_key",
  "model_policies": ["saved_catalog"],
  "default_model_policy": "saved_catalog",
  "model_discovery": "manual",
  "transport": "anthropic_messages",
  "endpoint_policy": "profile_required",
  "endpoint_join": "anthropic_v1",
  "api_key_env": "CSSWITCH_RELAY_KEY",
  "scratch_policy": "upstream_probe",
  "thinking_policy": "enabled",
  "timeouts": { "connect_ms": 10000, "total_ms": 30000, "read_idle_ms": 300000 },
  "cache": { "normal_ttl_seconds": 0, "stale_ttl_seconds": 0 }
}
```

Use `model_discovery: "manual"` because the Coding Plan contract guarantees the documented IDs but does not require startup to depend on `/v1/models`.

- [ ] **Step 4: Add the model preset**

```json
{
  "id": "kimi-code",
  "default_upstream_model": "kimi-for-coding",
  "role_bindings": {
    "sonnet": "kimi-for-coding",
    "opus": "kimi-for-coding",
    "haiku": "kimi-for-coding",
    "fable": "kimi-for-coding"
  },
  "models": [
    {"upstream_model": "k3", "display_name": "Kimi K3 (Coding Plan)", "supports_tools": true},
    {"upstream_model": "kimi-for-coding", "display_name": "Kimi K2.7 Code (Coding Plan)", "supports_tools": true},
    {"upstream_model": "kimi-for-coding-highspeed", "display_name": "Kimi K2.7 Code HighSpeed (Coding Plan)", "supports_tools": true}
  ]
}
```

- [ ] **Step 5: Add the separate template**

```rust
Template {
    id: "kimi-code",
    name: "Kimi Coding Plan",
    category: "cn_official",
    api_format: "anthropic",
    base_url: "https://api.kimi.com/coding/",
    base_url_editable: false,
    preset_catalog_id: Some("kimi-code"),
    model_catalog_source: "preset",
    website_url: "https://www.kimi.com/code/console",
    icon: "kimi",
    icon_color: "#16182F",
    compatibility_notice: Some("Uses Kimi membership Coding Plan quota; requires a Kimi Code Console API key."),
},
```

- [ ] **Step 6: Run focused tests and confirm GREEN**

Run the two Task 1 commands again. Expected: all focused tests pass.

- [ ] **Step 7: Commit**

```bash
git add catalog/provider-contracts.v1.json catalog/model-presets.v1.json desktop/src-tauri/src/templates.rs desktop/src-tauri/src/provider_contracts.rs
git commit -m "feat: add Kimi Coding Plan provider preset"
```

---

### Task 2: Route Kimi compatibility by contract rather than model spelling

**Files:**
- Modify: `desktop/gateway/src/config.rs`
- Modify: `desktop/gateway/src/anthropic_compat.rs`
- Modify: `desktop/gateway/src/server.rs`
- Test: `desktop/gateway/src/anthropic_compat.rs`
- Test: `desktop/gateway/src/server.rs`

**Interfaces:**
- Consumes: `ProviderRuntimeContract.contract_id` and the request's resolved upstream model.
- Produces: `GatewayConfig.kimi_compat: bool`, passed explicitly into relay request transformation and response filtering.

- [ ] **Step 1: Write failing tests for `k3` and Coding Plan model IDs**

Add tests proving that Kimi behavior is not inferred from `model.contains("kimi")`:

```rust
#[test]
fn kimi_code_contract_enables_kimi_compat_for_k3() {
    let cfg = gateway_config_for_contract("kimi-code-anthropic-relay", "k3");
    assert!(cfg.kimi_compat);
}

#[test]
fn k3_uses_enabled_thinking_and_removes_forced_tool_choice() {
    let request = json!({
        "model": "claude-csswitch-kimi-code-k3",
        "tool_choice": {"type": "tool", "name": "bash"},
        "tools": [{"name": "bash", "input_schema": {"type": "object"}}],
        "messages": [{"role": "user", "content": "inspect"}]
    });
    let (out, metadata) = transform_relay_request_with_compat(
        request,
        "k3",
        Some("enabled"),
        "https://api.kimi.com/coding/v1/messages",
        true,
    ).unwrap();
    assert_eq!(out["thinking"]["type"], "enabled");
    assert!(out.get("tool_choice").is_none());
    assert!(metadata.kimi_compat);
}
```

Add a server test that feeds Kimi-style `server_tool_use` SSE for model `k3` and asserts it is filtered and indices remain compact.

- [ ] **Step 2: Run Gateway tests and confirm RED**

```bash
cargo test --manifest-path desktop/gateway/Cargo.toml anthropic_compat::tests::k3_uses_enabled_thinking_and_removes_forced_tool_choice
cargo test --manifest-path desktop/gateway/Cargo.toml server::tests::k3_stream_filters_server_tool_blocks
```

Expected: compile failure because the explicit compatibility interface does not exist.

- [ ] **Step 3: Add the trusted compatibility flag**

Extend `GatewayConfig`:

```rust
pub kimi_compat: bool,
```

Derive it only from the validated contract identity:

```rust
let kimi_compat = matches!(
    provider_contract.contract_id.as_str(),
    "kimi-anthropic-relay" | "kimi-code-anthropic-relay"
);
```

Do not activate Kimi normalization for an arbitrary custom model whose name merely contains `kimi`.

- [ ] **Step 4: Thread the flag through request and response handling**

Replace each Kimi decision based on `target_model.to_ascii_lowercase().contains("kimi")` with `cfg.kimi_compat` or an explicit `kimi_compat` argument. Extend `AnthropicMetadata`:

```rust
pub struct AnthropicMetadata {
    pub target_model: String,
    pub rule_ids: Vec<String>,
    pub kimi_compat: bool,
}
```

Select the stream filter with:

```rust
let filter = metadata
    .kimi_compat
    .then(|| StreamFilter::Kimi(KimiServerToolFilter::new()));
```

- [ ] **Step 5: Run focused tests and confirm GREEN**

Run the two Task 2 commands again. Expected: both pass.

- [ ] **Step 6: Run all Gateway tests**

```bash
cargo test --offline --manifest-path desktop/gateway/Cargo.toml
```

Expected: no new failures relative to the recorded filesystem-constrained baseline.

- [ ] **Step 7: Commit**

```bash
git add desktop/gateway/src/config.rs desktop/gateway/src/anthropic_compat.rs desktop/gateway/src/server.rs
git commit -m "fix: recognize Kimi Coding Plan protocol by contract"
```

---

### Task 3: Pin the Coding Plan endpoint construction and headers

**Files:**
- Modify: `desktop/gateway/src/config.rs`
- Modify: `desktop/gateway/src/messages.rs`
- Test: `desktop/gateway/src/config.rs`
- Test: `desktop/gateway/src/messages.rs`

**Interfaces:**
- Consumes: base URL `https://api.kimi.com/coding/` and `AuthScheme::AnthropicDual`.
- Produces: inference URL `https://api.kimi.com/coding/v1/messages`; the Kimi Code API key is sent only in the existing authorized Anthropic headers.

- [ ] **Step 1: Add endpoint table tests**

```rust
#[test]
fn kimi_code_anthropic_base_joins_once() {
    let cases = [
        "https://api.kimi.com/coding/",
        "https://api.kimi.com/coding/v1",
        "https://api.kimi.com/coding/v1/messages",
    ];
    for base in cases {
        let (messages, models) = joined_endpoints(
            EndpointJoin::AnthropicV1,
            "anthropic_messages",
            base,
        ).unwrap();
        assert_eq!(messages, "https://api.kimi.com/coding/v1/messages");
        assert_eq!(models.as_deref(), Some("https://api.kimi.com/coding/v1/models"));
    }
}
```

- [ ] **Step 2: Run the endpoint test**

```bash
cargo test --manifest-path desktop/gateway/Cargo.toml config::tests::kimi_code_anthropic_base_joins_once
```

Expected: pass if current normalization remains compatible; if it fails, change only `normalize_anthropic_v1_base` and retain all existing endpoint tests.

- [ ] **Step 3: Add a mock-upstream header test**

Start the existing loopback mock, send one request, and assert:

```rust
assert_eq!(captured.path, "/coding/v1/messages");
assert_eq!(captured.headers.get("x-api-key"), Some(&"test-kimi-code-key".into()));
assert!(!captured.headers.contains_key("x-csswitch-auth-token"));
assert_eq!(captured.body["model"], "k3");
```

The test fixture key must remain a literal test value and never read the user's environment.

- [ ] **Step 4: Run the focused header test**

```bash
cargo test --manifest-path desktop/gateway/Cargo.toml messages::tests::kimi_code_uses_anthropic_auth_without_local_secret
```

Expected: pass after any minimal header correction.

- [ ] **Step 5: Commit**

```bash
git add desktop/gateway/src/config.rs desktop/gateway/src/messages.rs
git commit -m "test: pin Kimi Coding Plan endpoint contract"
```

---

### Task 4: Add an isolated Linux Kimi controller

**Files:**
- Create: `scripts/csswitch-kimi-code`
- Create: `test/test_headless_kimi_code.sh`
- Modify: `test/run-scripts.sh`

**Interfaces:**
- Consumes: installed `csswitch-gateway`, installed `claude-science`, protected Kimi key file, static `kimi-code` catalog, proxy `127.0.0.1:2999`.
- Produces: `csswitch-kimi-code start|stop|status` with redacted output and exact PID ownership.

- [ ] **Step 1: Write the failing isolation test**

The test must use a temporary HOME and fake `/proc`/`ss` fixtures. Assert exact defaults:

```bash
assert_eq "$CSSWITCH_KIMI_STATE_DIR" "$HOME/.csswitch/headless-kimi-code"
assert_eq "$CSSWITCH_KIMI_GATEWAY_PORT" "12434"
assert_eq "$CSSWITCH_KIMI_SCIENCE_PORT" "9102"
assert_eq "$CSSWITCH_KIMI_SANDBOX_PORT" "9103"
assert_eq "$CSSWITCH_KIMI_SCIENCE_DATA" "$HOME/.claude-science-kimi-code"
assert_eq "$CSSWITCH_KIMI_SCIENCE_CONFIG" "$HOME/.claude-science-kimi-code/config.toml"
```

Also assert that the script contains none of the Codex-owned paths as writable defaults and rejects listeners whose PID record or executable does not match.

- [ ] **Step 2: Run the test and confirm RED**

```bash
bash test/test_headless_kimi_code.sh
```

Expected: failure because `scripts/csswitch-kimi-code` does not exist.

- [ ] **Step 3: Implement protected input loading**

Use these defaults:

```bash
state_dir=${CSSWITCH_KIMI_STATE_DIR:-"${HOME:?}/.csswitch/headless-kimi-code"}
key_file=${CSSWITCH_KIMI_KEY_FILE:-"${HOME:?}/.csswitch/kimi-code.env"}
science_data=${CSSWITCH_KIMI_SCIENCE_DATA:-"${HOME:?}/.claude-science-kimi-code"}
science_config=${CSSWITCH_KIMI_SCIENCE_CONFIG:-"$science_data/config.toml"}
gateway_port=${CSSWITCH_KIMI_GATEWAY_PORT:-12434}
science_port=${CSSWITCH_KIMI_SCIENCE_PORT:-9102}
sandbox_port=${CSSWITCH_KIMI_SANDBOX_PORT:-9103}
outbound_proxy=${CSSWITCH_KIMI_OUTBOUND_PROXY:-http://127.0.0.1:2999}
```

Require `state_dir` mode `0700`; require `key_file` to be a non-symlink regular file mode `0600` containing exactly one `CSSWITCH_RELAY_KEY=<non-whitespace>` line. Never echo the value or source arbitrary shell syntax.

- [ ] **Step 4: Start the isolated Gateway**

Generate a unique 64-hex local path secret under the Kimi state directory. Launch detached with `nohup` + `setsid`, provider `relay`, the exact `kimi-code-anthropic-relay` catalog identity, the protected key, base URL, static model catalog, and proxy variables. The launched environment must include:

```text
CSSWITCH_RELAY_BASE_URL=https://api.kimi.com/coding/
CSSWITCH_RELAY_THINKING=enabled
HTTP_PROXY=http://127.0.0.1:2999
HTTPS_PROXY=http://127.0.0.1:2999
http_proxy=http://127.0.0.1:2999
https_proxy=http://127.0.0.1:2999
NO_PROXY=127.0.0.1,localhost,::1
no_proxy=127.0.0.1,localhost,::1
```

The controller must obtain or embed the exact provider catalog SHA-256 during the build/install step and reject a mismatch instead of selecting the generic relay contract.

- [ ] **Step 5: Start the isolated Science daemon**

Launch with all identity-bearing flags explicit:

```bash
claude-science serve \
  --data-dir "$science_data" \
  --config "$science_config" \
  --host 127.0.0.1 \
  --port "$science_port" \
  --sandbox-port "$sandbox_port" \
  --no-browser --no-auto-update --detached
```

Set its `ANTHROPIC_BASE_URL` to the protected local Kimi Gateway URL. Do not alter the existing Codex Science process or its environment.

- [ ] **Step 6: Implement status and stop**

Status prints only:

```text
key=ready|missing
gateway=running|stopped health=ready|unavailable catalog=ready|unavailable proxy=ready|invalid
science=running|stopped listener=ready|absent
```

Stop verifies UID, resolved executable, exact port argument, exact data directory argument, PID record, and listener ownership before sending `TERM`; use `KILL` only after a bounded wait and a second identity verification.

- [ ] **Step 7: Complete controller tests**

Cover:

- separate state/data/config paths;
- modes `0700` and `0600`;
- refusal of symlinks and malformed key files;
- refusal of ports `11434`, `9002`, and `9003` as Kimi defaults;
- refusal to stop unknown owners;
- correct provider contract, endpoint, model catalog, and proxy environment;
- process survival after the launching shell exits;
- cleanup of a newly started Gateway when Science startup fails;
- absence of key, local path secret, URL token, or response body in output/logs.

- [ ] **Step 8: Run controller tests and confirm GREEN**

```bash
bash test/test_headless_kimi_code.sh
bash test/run-scripts.sh
```

Expected: both pass.

- [ ] **Step 9: Commit**

```bash
git add scripts/csswitch-kimi-code test/test_headless_kimi_code.sh test/run-scripts.sh
git commit -m "feat: manage isolated Kimi Code Science runtime"
```

---

### Task 5: Document safe setup, operation, sharing, and rollback

**Files:**
- Create: `docs/features/kimi-code-science-bridge.md`
- Modify: `docs/README.md`
- Test: `test/test_headless_kimi_code.sh`

**Interfaces:**
- Consumes: `csswitch-kimi-code start|stop|status` and explicit Science `--data-dir`/`--config` targeting.
- Produces: a credential-safe operator runbook.

- [ ] **Step 1: Add documentation assertions**

```bash
grep -Fq 'https://api.kimi.com/coding/' "$DOC"
grep -Fq 'kimi-for-coding' "$DOC"
grep -Fq 'csswitch-kimi-code status' "$DOC"
grep -Fq -- '--data-dir ~/.claude-science-kimi-code' "$DOC"
grep -Fq -- '--config ~/.claude-science-kimi-code/config.toml' "$DOC"
grep -Fq 'Do not share the Kimi API key' "$DOC"
grep -Fq 'does not fall back to Codex' "$DOC"
```

- [ ] **Step 2: Run the test and confirm RED**

```bash
bash test/test_headless_kimi_code.sh
```

Expected: documentation assertions fail.

- [ ] **Step 3: Write the operator runbook**

Document:

1. Obtain a Kimi Coding Plan key from the Kimi Code Console.
2. Create `~/.csswitch/kimi-code.env` as mode `0600` without entering the key into shell history.
3. Validate proxy `127.0.0.1:2999` before startup.
4. Run `csswitch-kimi-code start` and `csswitch-kimi-code status`.
5. Generate the correct URL with both selectors:

```bash
claude-science url \
  --data-dir ~/.claude-science-kimi-code \
  --config ~/.claude-science-kimi-code/config.toml
```

6. Share only that single-use Science URL with trusted collaborators.
7. Stop only the Kimi stack with `csswitch-kimi-code stop`.
8. Confirm `csswitch-codex status` remains healthy before and after every lifecycle operation.

- [ ] **Step 4: Run documentation/controller tests and confirm GREEN**

```bash
bash test/test_headless_kimi_code.sh
```

Expected: pass.

- [ ] **Step 5: Commit**

```bash
git add docs/features/kimi-code-science-bridge.md docs/README.md test/test_headless_kimi_code.sh
git commit -m "docs: add parallel Kimi Code operations"
```

---

### Task 6: Build and perform non-live isolation verification

**Files:**
- Modify only if a test exposes a defect in files from Tasks 1–5.

**Interfaces:**
- Consumes: completed implementation and existing Codex runtime.
- Produces: build/test evidence without using the user's Kimi key.

- [ ] **Step 1: Capture redacted Codex identity before testing**

Record only PID hashes or exact PIDs in a private temporary test variable; terminal output should show:

```text
codex_before=running
codex_health=ready
codex_catalog=ready
```

- [ ] **Step 2: Run formatting and lint checks**

```bash
cargo fmt --manifest-path desktop/gateway/Cargo.toml --check
cargo clippy --offline --manifest-path desktop/gateway/Cargo.toml --all-targets -- -D warnings
```

Expected: exit `0`.

- [ ] **Step 3: Run focused and full available tests**

```bash
cargo test --offline --manifest-path desktop/gateway/Cargo.toml
bash test/test_headless_codex.sh
bash test/test_headless_kimi_code.sh
bash test/run_all.sh
```

Expected: new tests pass; any pre-existing environment-only failures are compared exactly against the recorded baseline and no new failure is accepted.

- [ ] **Step 4: Build the Gateway**

```bash
cargo build --release --manifest-path desktop/gateway/Cargo.toml
```

Expected: exit `0` and a release `csswitch-gateway` artifact.

- [ ] **Step 5: Prove isolation with mock credentials**

Run the Kimi controller against loopback fake Gateway/Science fixtures on `12434`/`9102`/`9103`. Confirm the Codex PIDs, `/proc` identities, listeners, state-file hashes, and redacted status are unchanged.

- [ ] **Step 6: Commit any evidence-only documentation**

If the repository convention requires dated evidence, add only redacted commands/results:

```bash
git add docs/evidence/investigations/2026-07-23-kimi-code-isolation-evidence.md
git commit -m "test: record Kimi Code isolation evidence"
```

---

### Task 7: User-authorized live acceptance and rollback proof

**Files:**
- No source changes unless acceptance identifies a reproduced defect with a new failing test.

**Interfaces:**
- Consumes: user-created protected Kimi Code API key file and installed tested binaries.
- Produces: a running isolated Kimi stack or a complete rollback to the untouched Codex-only state.

- [ ] **Step 1: Obtain explicit live-test authorization**

Before reading the protected Kimi key file, starting processes, consuming membership quota, or writing outside the repository, ask for explicit approval. Do not request that the user paste the key into chat.

- [ ] **Step 2: Resolve and verify all target ports**

Read-only checks must prove `12434`, `9102`, and `9103` are free. Recheck existing `11434`, `9002`, and `9003` and record only redacted health/ownership results.

- [ ] **Step 3: Install without overwriting Codex controls**

Install the updated Gateway only after hash verification. Install the new controller as `/home/bio-13/.local/bin/csswitch-kimi-code`; do not overwrite `/home/bio-13/.local/bin/csswitch-codex` unless its source changed and its regression test passed.

- [ ] **Step 4: Start and validate the Kimi stack**

Run:

```bash
csswitch-kimi-code start
csswitch-kimi-code status
```

Expected redacted result:

```text
key=ready
gateway=running health=ready catalog=ready proxy=ready
science=running listener=ready
```

- [ ] **Step 5: Perform minimum live inference acceptance**

Using the protected local Gateway URL internally, perform one non-streaming response requiring the exact text `CSSWITCH_KIMI_OK`. Print only:

```text
selected_model=<public model id>
inference=ready
```

Then test one streamed tool-use turn and one Claude Science terminal auto-review checkpoint. Do not print response bodies, tool inputs, or reviewer prompts.

- [ ] **Step 6: Verify model-specific behavior**

Test only models allowed by the user's membership tier:

- `kimi-for-coding` for every plan;
- `k3` only when the account tier exposes it;
- `kimi-for-coding-highspeed` only when the account tier exposes it.

An unavailable tier model is reported as unavailable and is not replaced by Codex or another Kimi model.

- [ ] **Step 7: Prove Codex remained unchanged**

Run `csswitch-codex status`, compare its exact owned PIDs/listeners and protected state-file hashes with Step 1, and confirm one Codex health/catalog request. Do not send an additional Codex inference unless separately authorized.

- [ ] **Step 8: Exercise isolated rollback**

Run `csswitch-kimi-code stop`. Confirm ports `12434`, `9102`, and `9103` are clear while Codex ports and PIDs remain unchanged. Preserve Kimi Science data and the protected API-key file unless the user explicitly requests their deletion.

- [ ] **Step 9: Final branch verification**

```bash
git diff --check
git status --short --branch
git log --oneline --decorate -8
```

Expected: clean named feature branch containing only reviewed local commits. Present merge/push/keep/discard options; default to keeping the branch when the user has not authorized integration.

---

## Self-Review Checklist

- [x] Separate Kimi Coding Plan from Moonshot Platform billing and model IDs.
- [x] Preserve the current Codex controller, ports, state, credentials, processes, and Science data.
- [x] Use a second Gateway rather than mixed-provider routing.
- [x] Route `k3` through Kimi compatibility without relying on the substring `kimi`.
- [x] Cover text, streaming tools, thinking, and Science auto-review traffic.
- [x] Require exact process ownership and redacted status output.
- [x] Include proxy `2999`, localhost bypass, safe URL sharing, and rollback.
- [x] Require explicit authorization before credentials, quota, installation, or live processes are touched.
- [x] Every implementation task names its required code, command, and expected result.
