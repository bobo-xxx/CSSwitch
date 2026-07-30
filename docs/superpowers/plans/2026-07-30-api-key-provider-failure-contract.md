# API-Key Provider Failure Contract Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Extend the proven Provider Failure Contract through the shared API-key inference adapter for Anthropic Messages, OpenAI Chat, and OpenAI Responses while preserving route transformations, enforcing explicit bounded policies, and leaving the installed runtime untouched.

**Architecture:** Keep `messages.rs` responsible for exactly one API-key inference POST and bounded projection of closed failure facts. Add a focused API-key attempt runner around the existing pure Attempt Controller, source route policies from the validated provider-contract catalog, and keep request/response transforms in their existing server branches.

**Tech Stack:** Rust 2021, blocking `reqwest`, `serde`/`serde_json`, loopback TCP fixtures, Cargo unit/integration tests, Git linked worktree.

## Global Constraints

- Work only on `ticket07/provider-failure-routes` in the existing linked worktree based on Ticket 06 evidence head `30ec9e2d546dea72f6de7fd843a373657d24fa0b`.
- The approved design is `docs/superpowers/specs/2026-07-30-api-key-provider-failure-contract-design.md`; behavioral changes require a spec amendment and user approval.
- Do not install, replace, stop, restart, or reconfigure the running Gateway on `11535`, Claude Science on `9002`, sandbox on `9003`, or proxy on `2999`.
- Do not use or inspect live API-key credentials, OAuth records, installed profiles, or external provider endpoints.
- Do not implement fallback, model substitution, history deletion, Safe Repair outside Codex, model discovery, authentication, UI, or Science orchestration.
- Translate each inference request once; retry-only attempts reuse byte-identical translated bytes.
- API-key routes permit at most three POSTs with fallback delays `[500, 1000]` ms and a `Retry-After` delta-seconds cap of 60 seconds.
- Anthropic Messages retries only pre-success connect/timeout, `408`, `429`, and `5xx` observations.
- OpenAI Chat and OpenAI Responses additionally retry pre-success `409`.
- Exact `insufficient_quota` overrides `429` and is terminal.
- `401`, `403`, permanent `4xx`, redirects, malformed success, conversion failures, opened/partial responses, and cancellation never replay.
- Preserve existing caller `type`, `error.type`, and `error.message`; add only closed optional metadata and omit unknown fields.
- Emit exactly one final diagnostic only after downstream delivery is known; diagnostics never contain request IDs or arbitrary upstream/caller strings.
- Preserve the Codex harness at exactly 8/8 and Codex contract at exactly 27/27.
- No new dependencies.
- Every production behavior follows RED, verified RED, minimal GREEN, verified GREEN, then refactor.
- Loopback tests run with `--test-threads=1` and reject ports `2999`, `8765`, `9002`, `9003`, `11434`, and `11535`.

---

## File Structure

- Modify `catalog/provider-contracts.v1.json`: add explicit `inference_retry` policy to every API-key contract.
- Modify `desktop/gateway/src/provider_contracts.rs`: validate closed retry tokens and project them into runtime policy.
- Modify `desktop/gateway/src/provider_failure.rs`: add closed API-key provider/route variants and policy-driven observation authorization without changing Codex behavior.
- Modify `desktop/gateway/src/provider_failure/tests.rs`: pure policy, classification, budget, repair-exclusion, serialization, and diagnostic tests.
- Modify `desktop/gateway/src/messages.rs`: expose one-POST opened responses and bounded closed failure projection while preserving model-discovery behavior.
- Create `desktop/gateway/src/api_key_attempt.rs`: shared attempt sequence, cancellable waits, immutable request replay, and pending finalization ownership.
- Create `desktop/gateway/src/api_key_attempt/tests.rs`: fake-transport RED/GREEN tests independent of protocol conversion.
- Modify `desktop/gateway/src/lib.rs`: register the private API-key attempt module.
- Create `desktop/gateway/src/server/api_key_acceptance.rs`: real-handler loopback conformance for all three protocols.
- Modify `desktop/gateway/src/server.rs`: integrate the runner into existing Anthropic Messages, OpenAI Chat, and OpenAI Responses branches and use checked terminal delivery.
- Create `docs/evidence/investigations/2026-07-30-api-key-provider-failure-contract.md`: exact revision, commands, counts, matrices, redaction boundary, and limitations.
- Modify `docs/evidence/investigations/README.md`: link Ticket 07 evidence.
- Modify `.scratch/provider-failure-contract/issues/07-extend-selected-provider-routes.md`: resolve only after review and publication.
- Modify `.scratch/provider-failure-contract/map.md`: record the completed rollout without adding a new provider or live-credential claim.

### Task 1: Add explicit catalog policies and generalize the pure domain

**Files:**
- Modify: `catalog/provider-contracts.v1.json`
- Modify: `desktop/gateway/src/provider_contracts.rs`
- Modify: `desktop/gateway/src/provider_failure.rs`
- Modify: `desktop/gateway/src/provider_failure/tests.rs`

**Interfaces:**
- Consumes: current `ProviderRuntimeContract`, `RetryPolicy`, `RouteContext`, `FailureObservation`, and Codex-only provider/route enums.
- Produces: validated `ProviderRuntimeContract::inference_retry`, closed provider/route conversion, and policy-driven `RetryPolicy::allows(&FailureObservation)`.

- [ ] **Step 1: Write failing provider-contract policy tests**

Add to `provider_contracts.rs` tests:

```rust
#[test]
fn every_api_key_contract_has_the_exact_closed_retry_policy() {
    let catalog = parse_catalog().unwrap();
    for contract in catalog
        .contracts
        .iter()
        .filter(|contract| contract.auth_mode == "api_key")
    {
        let policy = contract.inference_retry.as_ref().expect("API-key retry policy");
        assert_eq!(policy.max_posts, 3, "{}", contract.id);
        assert_eq!(policy.fallback_delays_ms, [500, 1_000], "{}", contract.id);
        assert_eq!(policy.retry_after_cap_seconds, 60, "{}", contract.id);
        assert_eq!(policy.retry_network_kinds, ["connect", "timeout"], "{}", contract.id);
        let expected = match contract.transport.as_str() {
            "anthropic_messages" => vec!["408", "429", "5xx"],
            "openai_chat" | "openai_responses" => vec!["408", "409", "429", "5xx"],
            other => panic!("unexpected API-key transport {other}"),
        };
        assert_eq!(policy.retry_statuses, expected, "{}", contract.id);
    }
}

#[test]
fn retry_policy_tokens_reject_duplicates_unknowns_and_codex_attachment() {
    let duplicate = mutated_catalog_text(|contracts| {
        let contract = contracts
            .iter_mut()
            .find(|contract| contract["auth_mode"] == "api_key")
            .unwrap();
        contract["inference_retry"]["retry_statuses"] = json!(["429", "429"]);
    });
    assert!(parse_catalog_text(&duplicate).is_err());

    let unknown_status = mutated_catalog_text(|contracts| {
        let contract = contracts
            .iter_mut()
            .find(|contract| contract["auth_mode"] == "api_key")
            .unwrap();
        contract["inference_retry"]["retry_statuses"] = json!(["425"]);
    });
    assert!(parse_catalog_text(&unknown_status).is_err());

    let unknown_network = mutated_catalog_text(|contracts| {
        let contract = contracts
            .iter_mut()
            .find(|contract| contract["auth_mode"] == "api_key")
            .unwrap();
        contract["inference_retry"]["retry_network_kinds"] = json!(["read"]);
    });
    assert!(parse_catalog_text(&unknown_network).is_err());

    let codex_attachment = mutated_catalog_text(|contracts| {
        let codex = contracts
            .iter_mut()
            .find(|contract| contract["adapter"] == "codex")
            .unwrap();
        codex["inference_retry"] = json!({
            "max_posts": 3,
            "fallback_delays_ms": [500, 1000],
            "retry_after_cap_seconds": 60,
            "retry_statuses": ["408", "409", "429", "5xx"],
            "retry_network_kinds": ["connect", "timeout"]
        });
    });
    assert!(parse_catalog_text(&codex_attachment).is_err());
}
```

Add the production seam `parse_catalog_text(input: &str) -> Result<ProviderContractCatalog, String>` and keep `parse_catalog()` as its static-catalog wrapper. Add this complete test helper so invalid-policy tests always start from the real catalog rather than maintaining a second schema fixture:

```rust
fn mutated_catalog_text(mutator: impl FnOnce(&mut Vec<serde_json::Value>)) -> String {
    let mut root: serde_json::Value =
        serde_json::from_str(STATIC_PROVIDER_CONTRACTS_JSON).unwrap();
    let contracts = root["contracts"].as_array_mut().unwrap();
    mutator(contracts);
    serde_json::to_string(&root).unwrap()
}
```

- [ ] **Step 2: Write failing pure-domain policy tests**

Add API-key context helpers and tests to `provider_failure/tests.rs`:

```rust
fn api_context(provider: ProviderId, route: RouteMode, policy: RetryPolicy) -> RouteContext {
    RouteContext::api_key(
        provider,
        route,
        CorrelationId::new("api-corr-0001").unwrap(),
        policy,
    )
}

#[test]
fn anthropic_and_openai_policies_differ_only_on_conflict() {
    assert!(!RetryPolicy::ANTHROPIC_MESSAGES.allows(&http(409, None, None)));
    assert!(RetryPolicy::OPENAI_CHAT.allows(&http(409, None, None)));
    assert!(RetryPolicy::OPENAI_RESPONSES.allows(&http(409, None, None)));
    for policy in [
        RetryPolicy::ANTHROPIC_MESSAGES,
        RetryPolicy::OPENAI_CHAT,
        RetryPolicy::OPENAI_RESPONSES,
    ] {
        assert!(policy.allows(&http(408, None, None)));
        assert!(policy.allows(&http(429, None, None)));
        assert!(policy.allows(&http(529, None, None)));
        assert!(policy.allows(&FailureObservation::Network(NetworkKind::Connect)));
        assert!(policy.allows(&FailureObservation::Network(NetworkKind::Timeout)));
        assert!(!policy.allows(&FailureObservation::Network(NetworkKind::Read)));
        assert!(!policy.allows(&http(422, None, None)));
    }
}

#[test]
fn api_key_quota_is_terminal_and_repair_is_structurally_disabled() {
    let context = api_context(
        ProviderId::Relay,
        RouteMode::AnthropicMessages,
        RetryPolicy::ANTHROPIC_MESSAGES,
    );
    let mut controller = AttemptController::new(context, false);
    controller.begin_post().unwrap();
    let quota = FailureObservation::Http {
        status: 429,
        rate_kind: Some(RateKind::Quota),
        retry_after_seconds: Some(0),
        request_id: None,
        error_code: ErrorCode::InsufficientQuota,
        error_param: ErrorParam::Absent,
    };
    assert!(matches!(controller.observe(quota).unwrap(), AttemptDirective::Fail(_)));
    assert_eq!(controller.snapshot().posts, 1);
    assert_eq!(controller.snapshot().repairs, 0);
}
```

- [ ] **Step 3: Run the focused tests and verify RED**

```bash
cargo test --offline --manifest-path desktop/gateway/Cargo.toml \
  provider_contracts::tests::every_api_key_contract_has_the_exact_closed_retry_policy \
  -- --test-threads=1
cargo test --offline --manifest-path desktop/gateway/Cargo.toml \
  provider_failure::tests::anthropic_and_openai_policies_differ_only_on_conflict \
  -- --test-threads=1
```

Expected: compile/test failure because `inference_retry`, API-key variants, constructors and policy constants do not exist.

- [ ] **Step 4: Add the closed catalog schema and runtime projection**

Add the private catalog shape:

```rust
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct InferenceRetryConfig {
    max_posts: u8,
    fallback_delays_ms: [u64; 2],
    retry_after_cap_seconds: u64,
    retry_statuses: Vec<String>,
    retry_network_kinds: Vec<String>,
}
```

Add `inference_retry: Option<InferenceRetryConfig>` to `ProviderContract`, and `pub inference_retry: Option<RetryPolicy>` to `ProviderRuntimeContract`. Validate exact list membership, uniqueness, ordering, bounds and auth-mode ownership before converting to the corresponding domain constant.

Add these objects to every API-key catalog contract:

```json
"inference_retry": {
  "max_posts": 3,
  "fallback_delays_ms": [500, 1000],
  "retry_after_cap_seconds": 60,
  "retry_statuses": ["408", "429", "5xx"],
  "retry_network_kinds": ["connect", "timeout"]
}
```

Use `["408", "409", "429", "5xx"]` for `openai_chat` and `openai_responses`. Codex has no `inference_retry` object.

- [ ] **Step 5: Generalize closed domain values without changing Codex policy**

Extend the enums and policy:

```rust
pub(crate) enum ProviderId {
    Codex,
    Deepseek,
    Qwen,
    Relay,
    OpenaiCustom,
    OpenaiResponses,
}

pub(crate) enum RouteMode {
    Responses,
    ResponsesLite,
    AnthropicMessages,
    OpenaiChat,
    OpenaiResponses,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RetryPolicy {
    pub(crate) max_posts: u8,
    pub(crate) fallback_delays_ms: [u64; 2],
    pub(crate) retry_after_cap_seconds: u64,
    retry_408: bool,
    retry_409: bool,
    retry_429: bool,
    retry_5xx: bool,
    retry_connect: bool,
    retry_timeout: bool,
    retry_unknown_429: bool,
}
```

Add constants `ANTHROPIC_MESSAGES`, `OPENAI_CHAT`, and `OPENAI_RESPONSES`. Keep `CODEX` behavior byte-for-byte equivalent, including unknown-429 terminal behavior and its existing network set. Replace hardcoded retry classification in `AttemptController::observe` with `self.context.retry_policy.allows(&observation)` while explicitly rejecting `RateKind::Quota` first.

Add `RouteContext::api_key(provider: ProviderId, route: RouteMode, correlation_id: CorrelationId, retry_policy: RetryPolicy) -> Self`, and assert inside the constructor that the provider and route are non-Codex and the policy is not `CODEX`. Do not add a repair flag to catalog policy.

- [ ] **Step 6: Run pure and catalog tests GREEN**

```bash
cargo test --offline --manifest-path desktop/gateway/Cargo.toml \
  provider_contracts::tests -- --test-threads=1
cargo test --offline --manifest-path desktop/gateway/Cargo.toml \
  provider_failure::tests -- --test-threads=1
cargo test --offline --manifest-path desktop/gateway/Cargo.toml \
  server::codex_acceptance::contract_ --features acceptance-build -- --test-threads=1
```

Expected: provider-contract and pure suites pass; Codex contract remains exactly 27/27.

- [ ] **Step 7: Commit Task 1**

```bash
git add catalog/provider-contracts.v1.json \
  desktop/gateway/src/provider_contracts.rs \
  desktop/gateway/src/provider_failure.rs \
  desktop/gateway/src/provider_failure/tests.rs
git commit -m "feat: define API-key provider retry policies"
```

### Task 2: Make the shared transport exactly one POST with closed failure facts

**Files:**
- Modify: `desktop/gateway/src/messages.rs`
- Modify: `desktop/gateway/src/provider_failure.rs`

**Interfaces:**
- Consumes: validated `GatewayConfig::provider_contract`, closed failure enums and existing timeout/auth code.
- Produces: `messages::post_once`, `OpenedInferenceResponse`, `into_nonstream`, `into_stream`, strict retry/request-ID projection, and no policy decisions.

- [ ] **Step 1: Write failing one-POST projection tests**

Add loopback tests in `messages.rs`:

```rust
#[test]
fn api_key_post_once_projects_only_closed_failure_facts() {
    let body = br#"{"error":{"code":"insufficient_quota","message":"secret upstream text"}}"#;
    let response = response_with_headers(
        429,
        body,
        &[("Retry-After", "90"), ("x-request-id", "Req_01.a:b-c")],
    );
    let (url, count, upstream) = spawn_counted_response(response);
    let observation = super::post_once(&test_config(url), b"{}", AttemptMode::Nonstream)
        .expect_err("429 is a closed observation");
    assert_eq!(count.load(Ordering::SeqCst), 1);
    assert_eq!(
        observation,
        FailureObservation::Http {
            status: 429,
            rate_kind: Some(RateKind::Quota),
            retry_after_seconds: Some(90),
            request_id: RequestId::new("Req_01.a:b-c"),
            error_code: ErrorCode::InsufficientQuota,
            error_param: ErrorParam::Absent,
        }
    );
    assert!(!format!("{observation:?}").contains("secret upstream text"));
    upstream.join().unwrap();
}

#[test]
fn successful_headers_open_before_body_or_protocol_failure() {
    let incomplete = response_with_declared_length(200, "application/json", 100, b"{}");
    let (url, count, upstream) = spawn_counted_response(incomplete);
    let opened = super::post_once(&test_config(url), b"{}", AttemptMode::Nonstream)
        .expect("2xx response opens");
    let observation = opened.into_nonstream().unwrap_err();
    assert!(matches!(observation, FailureObservation::Network(NetworkKind::Read)));
    assert_eq!(count.load(Ordering::SeqCst), 1);
    upstream.join().unwrap();
}

#[test]
fn failure_projection_bounds_body_and_validates_retry_headers() {
    let oversized = oversized_error_body_with_late_insufficient_quota();
    assert_eq!(project_api_failure_body(&oversized), (None, ErrorCode::Absent));
    assert_eq!(parse_retry_after(Some("60")), Some(60));
    assert_eq!(parse_retry_after(Some(" 7 ")), None);
    assert_eq!(parse_retry_after(Some("Wed, 30 Jul 2026 00:00:00 GMT")), None);
    assert!(RequestId::new("bad/request").is_none());
}
```

Reuse the existing `spawn_counted_response` and `test_config` helpers. Add these exact fixture builders next to them:

```rust
fn response_with_headers(status: u16, body: &[u8], headers: &[(&str, &str)]) -> Vec<u8> {
    let mut head = format!(
        "HTTP/1.1 {status} fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    for (name, value) in headers {
        head.push_str(name);
        head.push_str(": ");
        head.push_str(value);
        head.push_str("\r\n");
    }
    head.push_str("\r\n");
    let mut response = head.into_bytes();
    response.extend_from_slice(body);
    response
}

fn response_with_declared_length(
    status: u16,
    content_type: &str,
    declared_len: usize,
    body: &[u8],
) -> Vec<u8> {
    let mut response = format!(
        "HTTP/1.1 {status} fixture\r\nContent-Type: {content_type}\r\nContent-Length: {declared_len}\r\nConnection: close\r\n\r\n"
    )
    .into_bytes();
    response.extend_from_slice(body);
    response
}

fn oversized_error_body_with_late_insufficient_quota() -> Vec<u8> {
    let mut body = vec![b' '; MAX_ERROR_BODY_BYTES as usize + 1];
    body.extend_from_slice(br#"{"error":{"code":"insufficient_quota"}}"#);
    body
}
```

- [ ] **Step 2: Run focused transport tests and verify RED**

```bash
cargo test --offline --manifest-path desktop/gateway/Cargo.toml \
  messages::tests::api_key_post_once_projects_only_closed_failure_facts \
  -- --test-threads=1
```

Expected: compile failure because the one-attempt API and projection helpers do not exist.

- [ ] **Step 3: Add the one-attempt opened-response API**

Introduce:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AttemptMode {
    Nonstream,
    Stream,
}

pub(crate) struct OpenedInferenceResponse {
    response: Response,
    started: Instant,
    timeouts: InferenceTimeouts,
}

pub(crate) fn post_once(
    cfg: &GatewayConfig,
    body: &[u8],
    mode: AttemptMode,
) -> Result<OpenedInferenceResponse, FailureObservation>;

impl OpenedInferenceResponse {
    pub(crate) fn into_nonstream(self) -> Result<UpstreamBody, FailureObservation>;
    pub(crate) fn into_stream(self) -> Result<UpstreamStream, FailureObservation>;
}
```

`post_once` sends exactly once and returns only after either a non-success observation is projected or a 2xx response object is available. It never sleeps or retries. `into_nonstream` and `into_stream` are called only after the runner marks response-open.

Keep `get`, model-discovery error strings and their tests unchanged. Convert current public inference wrappers into temporary compatibility shims around one attempt until Task 4-6 remove their call sites.

- [ ] **Step 4: Implement bounded closed error projection**

Add strict helpers:

```rust
fn parse_retry_after(value: Option<&str>) -> Option<u64>;
fn parse_request_id(headers: &HeaderMap) -> Option<RequestId>;
fn project_api_failure_body(bytes: &[u8]) -> (Option<RateKind>, ErrorCode);
fn project_http_failure(response: Response, started: Instant, total: Duration)
    -> FailureObservation;
```

Read at most `MAX_ERROR_BODY_BYTES + 1`, pass at most the first 16 KiB to `serde_json`, and project only exact `error.code` or `error.type` values `insufficient_quota`, `rate_limit_exceeded`, and `rate_limit_error`. Unknown, malformed, duplicate or oversized semantics produce closed absent/unknown facts. No body string enters `UpstreamError`, `ProviderFailure`, diagnostics or Debug output for inference paths.

- [ ] **Step 5: Run Task 2 GREEN and existing message regressions**

```bash
cargo test --offline --manifest-path desktop/gateway/Cargo.toml \
  messages::tests -- --test-threads=1
cargo test --offline --manifest-path desktop/gateway/Cargo.toml \
  codex_transport::tests -- --test-threads=1
```

Expected: all message and Codex transport tests pass; one-POST tests assert count `1`.

- [ ] **Step 6: Commit Task 2**

```bash
git add desktop/gateway/src/messages.rs desktop/gateway/src/provider_failure.rs
git commit -m "refactor: expose one-post API-key transport facts"
```

### Task 3: Add the shared API-key attempt sequence

**Files:**
- Create: `desktop/gateway/src/api_key_attempt.rs`
- Create: `desktop/gateway/src/api_key_attempt/tests.rs`
- Modify: `desktop/gateway/src/lib.rs`

**Interfaces:**
- Consumes: immutable translated bytes, `RouteContext`, `AttemptController`, `messages::post_once`, and cancellation/runtime hooks.
- Produces: `ApiKeyAttemptSequence::open`, `OpenedAttempt`, `TerminalAttempt`, and checked finalization methods used by every server branch.

- [ ] **Step 1: Register an empty module and write fake-transport RED tests**

Add to `lib.rs`:

```rust
pub(crate) mod api_key_attempt;
```

Create the module with `#[cfg(test)] mod tests;`. In `tests.rs`, define the fake seam completely: `ScriptedTransport` owns a `VecDeque<Result<FakeOpened, FailureObservation>>` and a `Vec<Vec<u8>>`; each `post_once` records `body.to_vec()` and pops exactly one result. `RecordingRuntime` owns `delays: Vec<u64>` plus `cancel_on_wait: Option<usize>`; its `wait` records the delay and returns `false` only at that indexed wait. Use these exact helpers:

```rust
#[derive(Debug)]
struct FakeOpened;

fn http_observation(status: u16, rate_kind: Option<RateKind>) -> FailureObservation {
    FailureObservation::Http {
        status,
        rate_kind,
        retry_after_seconds: None,
        request_id: None,
        error_code: ErrorCode::Absent,
        error_param: ErrorParam::Absent,
    }
}

fn quota_observation() -> FailureObservation {
    FailureObservation::Http {
        status: 429,
        rate_kind: Some(RateKind::Quota),
        retry_after_seconds: Some(0),
        request_id: None,
        error_code: ErrorCode::InsufficientQuota,
        error_param: ErrorParam::Absent,
    }
}

fn failure(status: u16) -> Result<FakeOpened, FailureObservation> {
    Err(http_observation(status, None))
}

fn opened_success() -> Result<FakeOpened, FailureObservation> {
    Ok(FakeOpened)
}

fn openai_context() -> RouteContext {
    RouteContext::api_key(
        ProviderId::OpenaiCustom,
        RouteMode::OpenaiChat,
        CorrelationId::new("api-corr-openai").unwrap(),
        RetryPolicy::OPENAI_CHAT,
    )
}

fn anthropic_context() -> RouteContext {
    RouteContext::api_key(
        ProviderId::Relay,
        RouteMode::AnthropicMessages,
        CorrelationId::new("api-corr-anthropic").unwrap(),
        RetryPolicy::ANTHROPIC_MESSAGES,
    )
}
```

Then add these sequence tests:

```rust
#[test]
fn openai_conflict_retries_three_identical_posts_but_anthropic_stops_at_one() {
    let body = br#"{"model":"fixed","messages":[]}"#.to_vec();
    let mut openai = ScriptedTransport::fail_http([409, 409, 409]);
    let result = ApiKeyAttemptSequence::open(
        openai_context(),
        &body,
        AttemptMode::Nonstream,
        &mut openai,
        &mut RecordingRuntime::default(),
    );
    assert!(matches!(result, OpenResult::Failed(_)));
    assert_eq!(openai.bodies(), &[body.clone(), body.clone(), body.clone()]);

    let mut anthropic = ScriptedTransport::fail_http([409]);
    let result = ApiKeyAttemptSequence::open(
        anthropic_context(),
        &body,
        AttemptMode::Nonstream,
        &mut anthropic,
        &mut RecordingRuntime::default(),
    );
    assert!(matches!(result, OpenResult::Failed(_)));
    assert_eq!(anthropic.bodies(), &[body.clone()]);
}

#[test]
fn unknown_rate_limit_retries_but_quota_stops_without_repair() {
    let mut rate = ScriptedTransport::script([
        Err(http_observation(429, None)),
        opened_success(),
    ]);
    let mut runtime = RecordingRuntime::default();
    let result = ApiKeyAttemptSequence::open(
        anthropic_context(),
        b"{}",
        AttemptMode::Nonstream,
        &mut rate,
        &mut runtime,
    );
    assert!(matches!(result, OpenResult::Opened(_)));
    assert_eq!(runtime.delays, [500]);

    let mut quota = ScriptedTransport::script([Err(quota_observation())]);
    let result = ApiKeyAttemptSequence::open(
        anthropic_context(),
        b"{}",
        AttemptMode::Nonstream,
        &mut quota,
        &mut RecordingRuntime::default(),
    );
    let OpenResult::Failed(terminal) = result else { panic!("terminal quota") };
    assert_eq!(terminal.snapshot().posts, 1);
    assert_eq!(terminal.snapshot().repairs, 0);
}

#[test]
fn cancellation_during_wait_and_opened_response_are_final_barriers() {
    let mut runtime = RecordingRuntime::cancel_during_first_wait();
    let mut transport = ScriptedTransport::script([failure(503), opened_success()]);
    let result = ApiKeyAttemptSequence::open(
        openai_context(),
        b"{}",
        AttemptMode::Nonstream,
        &mut transport,
        &mut runtime,
    );
    assert!(matches!(result, OpenResult::Cancelled(_)));
    assert_eq!(transport.posts(), 1);

    let mut opened_transport = ScriptedTransport::script([opened_success()]);
    let OpenResult::Opened(opened) = ApiKeyAttemptSequence::open(
        openai_context(),
        b"{}",
        AttemptMode::Nonstream,
        &mut opened_transport,
        &mut RecordingRuntime::default(),
    ) else {
        panic!("fixture must open");
    };
    let failure = opened.fail_after_open(FailureObservation::Protocol(
        ProtocolKind::InvalidResponse,
    ));
    assert_eq!(failure.snapshot().posts, 1);
    assert_eq!(opened_transport.posts(), 1);
}
```

Implement `ScriptedTransport::script`, `ScriptedTransport::fail_http`, `bodies`, and `posts` as direct constructors/accessors over those two fields; do not add timers, sockets, or environment reads to this unit fixture.

- [ ] **Step 2: Verify Task 3 RED**

```bash
cargo test --offline --manifest-path desktop/gateway/Cargo.toml \
  api_key_attempt::tests -- --test-threads=1
```

Expected: compile failure because the sequence, runtime, transport and result types are absent.

- [ ] **Step 3: Implement the minimal sequence interfaces**

Create these production interfaces:

```rust
pub(crate) trait ApiKeyTransport {
    type Opened;
    fn post_once(
        &mut self,
        body: &[u8],
        mode: AttemptMode,
    ) -> Result<Self::Opened, FailureObservation>;
}

pub(crate) trait AttemptRuntime {
    fn cancelled(&mut self) -> bool;
    fn wait(&mut self, delay_ms: u64) -> bool;
}

pub(crate) enum OpenResult<O> {
    Opened(OpenedAttempt<O>),
    Failed(TerminalAttempt),
    Cancelled(TerminalAttempt),
}

pub(crate) struct OpenedAttempt<O> {
    pub(crate) opened: O,
    controller: AttemptController,
}

pub(crate) struct TerminalAttempt {
    pub(crate) failure: Option<ProviderFailure>,
    controller: AttemptController,
}
```

`ApiKeyAttemptSequence::open` loops only on `AttemptDirective::RetryAfter`. `RepairOnce` is unreachable for API-key contexts and becomes a fail-closed internal terminal error in release code plus a pure assertion. `wait` returns `false` on cancellation; no sleep occurs in tests.

`OpenedAttempt` exposes `fail_after_open`, `completed_diagnostic`, and `cancelled_diagnostic`. `TerminalAttempt` exposes checked `failed_diagnostic` or `cancelled_diagnostic` after caller delivery is known. No diagnostic is emitted inside the runner.

- [ ] **Step 4: Add the production adapter without server integration**

Implement `ApiKeyTransport` for a small wrapper around `(&GatewayConfig, messages::post_once)`. It owns no credential copy and implements custom Debug that prints only provider and attempt mode.

- [ ] **Step 5: Run Task 3 GREEN**

```bash
cargo test --offline --manifest-path desktop/gateway/Cargo.toml \
  api_key_attempt::tests -- --test-threads=1
cargo test --offline --manifest-path desktop/gateway/Cargo.toml \
  provider_failure::tests -- --test-threads=1
```

Expected: fake sequence and pure controller suites pass with exact post/delay vectors and no repair.

- [ ] **Step 6: Commit Task 3**

```bash
git add desktop/gateway/src/lib.rs \
  desktop/gateway/src/api_key_attempt.rs \
  desktop/gateway/src/api_key_attempt/tests.rs
git commit -m "feat: add shared API-key attempt sequence"
```

### Task 4: Integrate Anthropic Messages routes through real-handler conformance

**Files:**
- Create: `desktop/gateway/src/server/api_key_acceptance.rs`
- Modify: `desktop/gateway/src/server.rs`
- Modify: `desktop/gateway/src/messages.rs`

**Interfaces:**
- Consumes: shared attempt sequence, Anthropic Messages policy, existing relay/Kimi/DeepSeek transforms and stream filters.
- Produces: real-handler Anthropic conformance for DeepSeek and relay routes with identical success behavior.

- [ ] **Step 1: Build the guarded API-key acceptance fixture**

Register only under acceptance tests:

```rust
#[cfg(all(test, feature = "acceptance-build"))]
mod api_key_acceptance;
```

The fixture owns an ordered upstream script, captured requests, downstream response, recording wait runtime and final diagnostics. Its binder loops until the assigned port is not in:

```rust
const RESERVED_PORTS: [u16; 6] = [2999, 8765, 9002, 9003, 11434, 11535];
```

It constructs managed `GatewayConfig` values using exact catalog contract IDs and synthetic API keys that are asserted absent from every captured caller/diagnostic value.

Give the fixture these assertion entry points: `assert_terminal_http(route, mode, status, error_code, expected_posts)`, `assert_retry_then_success(route, mode, first_status, retry_after_seconds, expected_delay_ms)`, `assert_exhausted(route, mode, status, expected_delays_ms)`, `assert_opened_failure(route, mode, opened_fixture)`, `assert_success_fixture(route, mode, fixture_name)`, and `assert_redacted(route, sentinels)`. Every entry point must invoke the real `/v1/messages` handler and assert captured request count, unused script count, byte-identical bodies for repeated POSTs, exact recorded delays, exact caller status/body, and exactly one final diagnostic. `assert_success_fixture` records the current pre-change response as a literal test constant during RED setup and compares byte-for-byte after integration.

- [ ] **Step 2: Write Anthropic Messages conformance RED tests**

Add named cases:

```rust
#[test]
fn api_contract_anthropic_permanent_auth_and_quota_are_one_post() {
    for status in [400, 401, 403, 422] {
        assert_terminal_http(FixtureRoute::RelayAnthropic, AttemptMode::Nonstream, status, ErrorCode::Absent, 1);
    }
    assert_terminal_http(FixtureRoute::RelayAnthropic, AttemptMode::Nonstream, 429, ErrorCode::InsufficientQuota, 1);
}

#[test]
fn api_contract_anthropic_rate_and_5xx_are_bounded_and_byte_identical() {
    assert_retry_then_success(FixtureRoute::RelayAnthropic, AttemptMode::Nonstream, 429, None, 500);
    assert_retry_then_success(FixtureRoute::DeepseekAnthropic, AttemptMode::Nonstream, 429, Some(90), 60_000);
    assert_exhausted(FixtureRoute::DeepseekAnthropic, AttemptMode::Nonstream, 503, &[500, 1_000]);
}

#[test]
fn api_contract_anthropic_409_is_terminal() {
    assert_terminal_http(FixtureRoute::RelayAnthropic, AttemptMode::Nonstream, 409, ErrorCode::Absent, 1);
}

#[test]
fn api_contract_anthropic_opened_or_partial_stream_never_replays() {
    assert_opened_failure(FixtureRoute::DeepseekAnthropic, AttemptMode::Nonstream, OpenedFixture::MalformedJson);
    assert_opened_failure(FixtureRoute::RelayAnthropic, AttemptMode::Stream, OpenedFixture::PartialSseThenClose);
}

#[test]
fn api_contract_anthropic_kimi_and_deepseek_success_fixtures_are_unchanged() {
    assert_success_fixture(FixtureRoute::RelayKimi, AttemptMode::Nonstream, "kimi-nonstream-filter");
    assert_success_fixture(FixtureRoute::RelayKimi, AttemptMode::Stream, "kimi-stream-filter");
    assert_success_fixture(FixtureRoute::DeepseekAnthropic, AttemptMode::Nonstream, "deepseek-dsml-nonstream");
    assert_success_fixture(FixtureRoute::DeepseekAnthropic, AttemptMode::Stream, "deepseek-dsml-stream");
}

#[test]
fn api_contract_anthropic_failures_and_diagnostics_are_redacted() {
    assert_redacted(FixtureRoute::RelayAnthropic, &["fixture-api-key", "secret upstream text", "private.example", "Req-secret-01"]);
    assert_redacted(FixtureRoute::DeepseekAnthropic, &["fixture-api-key", "secret upstream text", "private.example", "Req-secret-02"]);
}
```

Every case asserts exact POST count, unused script steps, byte-identical translated bodies where retried, delays, caller status/envelope, and exactly one final diagnostic.

- [ ] **Step 3: Run Anthropic cases and verify RED**

```bash
cargo test --offline --manifest-path desktop/gateway/Cargo.toml \
  --features acceptance-build 'server::api_key_acceptance::api_contract_anthropic_' \
  -- --nocapture --test-threads=1
```

Expected: failures show the existing real handler performs one POST and emits legacy string-based `api_error` output without the shared attempt diagnostic.

- [ ] **Step 4: Integrate relay and DeepSeek branches minimally**

Add a `ProviderAttemptHooks` runtime in `server.rs` that polls downstream cancellation during waits and emits no diagnostic itself. Add one helper that builds API-key `RouteContext` from `cfg.provider`, `cfg.provider_contract.transport`, and the validated retry policy.

For relay and DeepSeek:

1. Keep current validation and transformation code in place.
2. Serialize once.
3. Call the shared sequence for both non-stream and stream.
4. On opened response, mark the success barrier before reading/conversion/filtering.
5. Deliver terminal failure with `write_json_checked` and choose failed versus cancelled diagnostic from the write result.
6. Finalize completed only after final body/chunk/flush delivery succeeds.

Do not move or change Kimi filters, DSML nonce generation, DeepSeek policy transforms, model selection or success payload bytes.

- [ ] **Step 5: Run Anthropic GREEN and compatibility suites**

```bash
cargo test --offline --manifest-path desktop/gateway/Cargo.toml \
  --features acceptance-build 'server::api_key_acceptance::api_contract_anthropic_' \
  -- --nocapture --test-threads=1
cargo test --offline --manifest-path desktop/gateway/Cargo.toml \
  anthropic_compat::tests -- --test-threads=1
cargo test --offline --manifest-path desktop/gateway/Cargo.toml \
  anthropic_sse::tests -- --test-threads=1
cargo test --offline --manifest-path desktop/gateway/Cargo.toml \
  dsml_shim::tests -- --test-threads=1
```

Expected: all Anthropic conformance and existing transform/filter suites pass.

- [ ] **Step 6: Commit Task 4**

```bash
git add desktop/gateway/src/server.rs \
  desktop/gateway/src/server/api_key_acceptance.rs \
  desktop/gateway/src/messages.rs
git commit -m "feat: apply provider failure contract to Anthropic routes"
```

### Task 5: Integrate OpenAI Chat routes

**Files:**
- Modify: `desktop/gateway/src/server/api_key_acceptance.rs`
- Modify: `desktop/gateway/src/server.rs`

**Interfaces:**
- Consumes: shared sequence, OpenAI Chat policy, Qwen/custom transforms, reasoning signer and local SSE replay.
- Produces: Qwen/custom/Gemini/Grok/OpenCode Go Chat conformance with route-specific `409` retry.

- [ ] **Step 1: Write OpenAI Chat RED cases**

Add:

```rust
#[test]
fn api_contract_openai_chat_conflict_rate_and_5xx_retry_with_identical_body() {
    assert_retry_then_success(FixtureRoute::OpenaiCustomChat, AttemptMode::Nonstream, 409, None, 500);
    assert_retry_then_success(FixtureRoute::QwenChat, AttemptMode::Nonstream, 429, None, 500);
    assert_exhausted(FixtureRoute::GeminiChat, AttemptMode::Nonstream, 500, &[500, 1_000]);
}

#[test]
fn api_contract_openai_chat_permanent_auth_quota_and_redirect_are_terminal() {
    for status in [400, 401, 403, 422, 307] {
        assert_terminal_http(FixtureRoute::OpenaiCustomChat, AttemptMode::Nonstream, status, ErrorCode::Absent, 1);
    }
    assert_terminal_http(FixtureRoute::QwenChat, AttemptMode::Nonstream, 429, ErrorCode::InsufficientQuota, 1);
}

#[test]
fn api_contract_openai_chat_malformed_success_never_replays() {
    assert_opened_failure(FixtureRoute::OpenaiCustomChat, AttemptMode::Nonstream, OpenedFixture::MalformedJson);
}

#[test]
fn api_contract_openai_chat_qwen_custom_gemini_grok_success_is_compatible() {
    for (route, fixture) in [
        (FixtureRoute::QwenChat, "qwen-chat"),
        (FixtureRoute::OpenaiCustomChat, "openai-custom-chat"),
        (FixtureRoute::GeminiChat, "gemini-chat"),
        (FixtureRoute::GrokChat, "grok-chat"),
        (FixtureRoute::OpenCodeGoChat, "opencode-go-chat"),
    ] {
        assert_success_fixture(route, AttemptMode::Nonstream, fixture);
    }
}

#[test]
fn api_contract_openai_chat_stream_replay_delivery_controls_final_outcome() {
    assert_success_fixture(FixtureRoute::OpenaiCustomChat, AttemptMode::Stream, "openai-chat-local-sse-replay");
    assert_delivery_failure(FixtureRoute::OpenaiCustomChat, AttemptMode::Stream, DeliveryFailure::ChunkWrite);
    assert_delivery_failure(FixtureRoute::OpenaiCustomChat, AttemptMode::Stream, DeliveryFailure::FinalFlush);
}
```

- [ ] **Step 2: Verify OpenAI Chat RED**

```bash
cargo test --offline --manifest-path desktop/gateway/Cargo.toml \
  --features acceptance-build 'server::api_key_acceptance::api_contract_openai_chat_' \
  -- --nocapture --test-threads=1
```

Expected: retry cases fail at one POST and diagnostic assertions fail; existing success output remains the baseline oracle.

- [ ] **Step 3: Route OpenAI Chat through the shared sequence**

Preserve `anthropic_to_openai`, `anthropic_to_openai_custom`, reasoning-signature checks and `openai_to_anthropic`. Translate/serialize once, then open with `RouteMode::OpenaiChat`. After successful headers, mark response-open before body read and conversion. For caller `stream=true`, local SSE replay remains downstream-only and never causes another upstream POST.

Use the same checked failure/completed/cancelled finalization helper introduced for Anthropic routes; do not duplicate an attempt loop.

- [ ] **Step 4: Run OpenAI Chat GREEN and compatibility suites**

```bash
cargo test --offline --manifest-path desktop/gateway/Cargo.toml \
  --features acceptance-build 'server::api_key_acceptance::api_contract_openai_chat_' \
  -- --nocapture --test-threads=1
cargo test --offline --manifest-path desktop/gateway/Cargo.toml \
  openai_chat::tests -- --test-threads=1
cargo test --offline --manifest-path desktop/gateway/Cargo.toml \
  config::tests::endpoint_join_policies_cover_full_urls_xai_gemini_and_opencode \
  -- --test-threads=1
```

Expected: OpenAI Chat conformance and all existing translation fixtures pass.

- [ ] **Step 5: Commit Task 5**

```bash
git add desktop/gateway/src/server.rs desktop/gateway/src/server/api_key_acceptance.rs
git commit -m "feat: apply provider failure contract to OpenAI Chat routes"
```

### Task 6: Integrate OpenAI Responses routes

**Files:**
- Modify: `desktop/gateway/src/server/api_key_acceptance.rs`
- Modify: `desktop/gateway/src/server.rs`

**Interfaces:**
- Consumes: shared sequence, OpenAI Responses policy and existing Responses request/response metadata conversion.
- Produces: custom OpenAI Responses conformance with the same bounded lifecycle and no Codex-only repair semantics.

- [ ] **Step 1: Write OpenAI Responses RED cases**

Add:

```rust
#[test]
fn api_contract_openai_responses_retries_conflict_rate_and_5xx_only() {
    assert_retry_then_success(FixtureRoute::OpenaiResponses, AttemptMode::Nonstream, 409, None, 500);
    assert_retry_then_success(FixtureRoute::OpenaiResponses, AttemptMode::Nonstream, 429, Some(90), 60_000);
    assert_exhausted(FixtureRoute::OpenaiResponses, AttemptMode::Nonstream, 502, &[500, 1_000]);
}

#[test]
fn api_contract_openai_responses_quota_and_permanent_errors_are_terminal() {
    for status in [400, 401, 403, 422, 307] {
        assert_terminal_http(FixtureRoute::OpenaiResponses, AttemptMode::Nonstream, status, ErrorCode::Absent, 1);
    }
    assert_terminal_http(FixtureRoute::OpenaiResponses, AttemptMode::Nonstream, 429, ErrorCode::InsufficientQuota, 1);
}

#[test]
fn api_contract_openai_responses_malformed_or_incomplete_success_never_replays() {
    assert_opened_failure(FixtureRoute::OpenaiResponses, AttemptMode::Nonstream, OpenedFixture::MalformedJson);
    assert_opened_failure(FixtureRoute::OpenaiResponses, AttemptMode::Nonstream, OpenedFixture::IncompleteBody);
}

#[test]
fn api_contract_openai_responses_success_metadata_and_mapping_are_unchanged() {
    assert_success_fixture(FixtureRoute::OpenaiResponses, AttemptMode::Nonstream, "openai-responses-metadata-map");
    assert_success_fixture(FixtureRoute::OpenaiResponses, AttemptMode::Stream, "openai-responses-local-sse-replay");
}

#[test]
fn api_contract_openai_responses_never_authorizes_safe_repair() {
    assert_unsupported_tool_choice_is_terminal(FixtureRoute::OpenaiResponses, 400, 1, 0);
}
```

- [ ] **Step 2: Verify OpenAI Responses RED**

```bash
cargo test --offline --manifest-path desktop/gateway/Cargo.toml \
  --features acceptance-build 'server::api_key_acceptance::api_contract_openai_responses_' \
  -- --nocapture --test-threads=1
```

Expected: retry/diagnostic assertions fail against the current single-call server branch.

- [ ] **Step 3: Route OpenAI Responses through the shared sequence**

Keep `anthropic_to_openai`, DashScope endpoint detection, metadata logging and `openai_to_anthropic` unchanged. Use `RouteMode::OpenaiResponses`, immutable serialized bytes, response-open before body read, and the same terminal delivery/finalization helper. Construct every API-key controller with repair disabled and assert no `RepairOnce` directive can escape the shared runner.

- [ ] **Step 4: Run OpenAI Responses GREEN and compatibility suites**

```bash
cargo test --offline --manifest-path desktop/gateway/Cargo.toml \
  --features acceptance-build 'server::api_key_acceptance::api_contract_openai_responses_' \
  -- --nocapture --test-threads=1
cargo test --offline --manifest-path desktop/gateway/Cargo.toml \
  openai_responses::tests -- --test-threads=1
```

Expected: all Responses conformance and existing mapping fixtures pass.

- [ ] **Step 5: Commit Task 6**

```bash
git add desktop/gateway/src/server.rs desktop/gateway/src/server/api_key_acceptance.rs
git commit -m "feat: apply provider failure contract to OpenAI Responses routes"
```

### Task 7: Close cross-route lifecycle, redaction and regression gaps

**Files:**
- Modify: `desktop/gateway/src/server/api_key_acceptance.rs`
- Modify: `desktop/gateway/src/server.rs`
- Modify: `desktop/gateway/src/api_key_attempt.rs`
- Modify: `desktop/gateway/src/provider_failure/tests.rs`

**Interfaces:**
- Consumes: all three integrated protocol stages.
- Produces: one cross-route acceptance gate proving exactly-once terminal diagnostics, cancellation finality, reserved-port safety, and no arbitrary-string leakage.

- [ ] **Step 1: Add cross-route RED regressions**

Add tests:

```rust
#[test]
fn api_contract_cross_route_downstream_failure_finalizes_only_cancelled() {
    for route in [FixtureRoute::RelayAnthropic, FixtureRoute::QwenChat, FixtureRoute::OpenaiResponses] {
        assert_delivery_failure(route, AttemptMode::Nonstream, DeliveryFailure::FinalBodyWrite);
        assert_delivery_failure(route, AttemptMode::Stream, DeliveryFailure::FinalFlush);
    }
}

#[test]
fn api_contract_cross_route_diagnostic_schema_has_only_closed_fields() {
    for route in [FixtureRoute::RelayAnthropic, FixtureRoute::QwenChat, FixtureRoute::OpenaiResponses] {
        assert_diagnostic_keys(route, &["schema_version", "provider", "route", "correlation_id", "outcome", "posts", "repairs", "reason"]);
    }
}

#[test]
fn api_contract_cross_route_raw_body_key_header_url_and_request_id_never_reach_diagnostic() {
    for route in [FixtureRoute::RelayAnthropic, FixtureRoute::QwenChat, FixtureRoute::OpenaiResponses] {
        assert_redacted(route, &["fixture-api-key", "secret upstream text", "private.example", "Req-secret-cross-route"]);
    }
}

#[test]
fn api_contract_cross_route_retry_wait_cancellation_stops_before_next_post() {
    for route in [FixtureRoute::RelayAnthropic, FixtureRoute::QwenChat, FixtureRoute::OpenaiResponses] {
        assert_wait_cancellation(route, 503, 1, &[500]);
    }
}

#[test]
fn api_harness_avoids_every_reserved_runtime_port() {
    for _ in 0..64 {
        let fixture = AcceptanceFixture::new(FixtureRoute::RelayAnthropic);
        assert!(!RESERVED_PORTS.contains(&fixture.upstream_port()));
        assert!(!RESERVED_PORTS.contains(&fixture.gateway_port()));
    }
}
```

Use a fault-injecting `Write` implementation for final JSON, SSE chunk and flush failures. Assert exactly one `cancelled` diagnostic and absence of `failed`/`completed` for each delivery failure.

- [ ] **Step 2: Verify cross-route RED**

```bash
cargo test --offline --manifest-path desktop/gateway/Cargo.toml \
  --features acceptance-build 'server::api_key_acceptance::api_contract_cross_route_' \
  -- --nocapture --test-threads=1
```

Expected: any remaining duplicated/unobserved delivery path fails its exact outcome or schema assertion.

- [ ] **Step 3: Make minimal lifecycle corrections and remove duplication**

Extract only helpers shared by at least two now-green route branches:

```rust
fn deliver_provider_failure<W: Write>(
    writer: &mut W,
    terminal: TerminalAttempt,
) -> AttemptDiagnostic;

fn finalize_opened_delivery(
    opened: OpenedAttempt<messages::OpenedInferenceResponse>,
    delivery: io::Result<()>,
) -> AttemptDiagnostic;
```

Both helpers choose one checked finalizer from the actual delivery result. They accept no raw upstream/caller strings and do not perform transport or conversion.

- [ ] **Step 4: Run all focused gates GREEN**

```bash
cargo test --offline --manifest-path desktop/gateway/Cargo.toml \
  provider_failure::tests -- --test-threads=1
cargo test --offline --manifest-path desktop/gateway/Cargo.toml \
  messages::tests -- --test-threads=1
cargo test --offline --manifest-path desktop/gateway/Cargo.toml \
  api_key_attempt::tests -- --test-threads=1
cargo test --offline --manifest-path desktop/gateway/Cargo.toml \
  --features acceptance-build 'server::api_key_acceptance::api_' \
  -- --nocapture --test-threads=1
cargo test --offline --manifest-path desktop/gateway/Cargo.toml \
  --features acceptance-build 'server::codex_acceptance::harness_' \
  -- --test-threads=1
cargo test --offline --manifest-path desktop/gateway/Cargo.toml \
  --features acceptance-build 'server::codex_acceptance::contract_' \
  -- --test-threads=1
```

Expected: all API-key gates pass; Codex remains exactly 8/8 and 27/27.

- [ ] **Step 5: Commit Task 7**

```bash
git add desktop/gateway/src/server.rs \
  desktop/gateway/src/server/api_key_acceptance.rs \
  desktop/gateway/src/api_key_attempt.rs \
  desktop/gateway/src/provider_failure/tests.rs
git commit -m "test: close API-key provider lifecycle contract"
```

### Task 8: Run immutable verification and publish Ticket 07 evidence

**Files:**
- Create: `docs/evidence/investigations/2026-07-30-api-key-provider-failure-contract.md`
- Modify: `docs/evidence/investigations/README.md`

**Interfaces:**
- Consumes: clean reviewed implementation branch.
- Produces: exact-SHA deterministic evidence with no live/provider credential claim.

- [ ] **Step 1: Record the clean implementation revision and environment**

```bash
git status --short
git rev-parse HEAD
rustc --version
cargo --version
uname -a
```

Expected: clean status; record literal outputs before creating evidence.

- [ ] **Step 2: Run formatting, diff and focused gates**

```bash
cargo fmt --check --manifest-path desktop/gateway/Cargo.toml
git diff --check 30ec9e2d546dea72f6de7fd843a373657d24fa0b..HEAD
cargo test --offline --manifest-path desktop/gateway/Cargo.toml \
  provider_contracts::tests -- --test-threads=1
cargo test --offline --manifest-path desktop/gateway/Cargo.toml \
  provider_failure::tests -- --test-threads=1
cargo test --offline --manifest-path desktop/gateway/Cargo.toml \
  messages::tests -- --test-threads=1
cargo test --offline --manifest-path desktop/gateway/Cargo.toml \
  api_key_attempt::tests -- --test-threads=1
cargo test --offline --manifest-path desktop/gateway/Cargo.toml \
  --features acceptance-build 'server::api_key_acceptance::api_' \
  -- --nocapture --test-threads=1
```

Expected: all focused gates pass with exact counts recorded in evidence.

- [ ] **Step 3: Re-run Codex invariants and compatibility suites**

```bash
cargo test --offline --manifest-path desktop/gateway/Cargo.toml \
  --features acceptance-build 'server::codex_acceptance::harness_' \
  -- --test-threads=1
cargo test --offline --manifest-path desktop/gateway/Cargo.toml \
  --features acceptance-build 'server::codex_acceptance::contract_' \
  -- --test-threads=1
cargo test --offline --manifest-path desktop/gateway/Cargo.toml \
  anthropic_compat::tests -- --test-threads=1
cargo test --offline --manifest-path desktop/gateway/Cargo.toml \
  openai_chat::tests -- --test-threads=1
cargo test --offline --manifest-path desktop/gateway/Cargo.toml \
  openai_responses::tests -- --test-threads=1
```

Expected: Codex remains exactly 8/8 and 27/27; all existing protocol compatibility fixtures pass.

- [ ] **Step 4: Run full tests and Clippy**

```bash
cargo test --offline --manifest-path desktop/gateway/Cargo.toml \
  --all-features -- --test-threads=1
cargo clippy --offline --manifest-path desktop/gateway/Cargo.toml \
  --all-targets --all-features -- -D warnings
```

Expected: all library, CLI and doc tests pass; Clippy exits zero with no warnings.

- [ ] **Step 5: Write and commit immutable evidence**

The report must bind the exact implementation SHA from Step 1 and include:

- provider/route policy table;
- literal commands and counts;
- exact post/delay/body-equality assertions;
- response-open and cancellation barriers;
- caller/diagnostic closed schemas;
- confirmation that no installed runtime, live key, external endpoint, profile, proxy or service was used or changed;
- macOS runtime pending limitation.

```bash
git add docs/evidence/investigations/README.md \
  docs/evidence/investigations/2026-07-30-api-key-provider-failure-contract.md
git commit -m "docs: record API-key provider failure evidence"
```

- [ ] **Step 6: Obtain independent whole-branch review**

Review `git diff 30ec9e2..HEAD` for policy correctness, duplicate POSTs, response-open/cancellation finality, raw-string leakage, compatibility regressions, catalog validation and evidence integrity. Correct every Critical or Important finding with a new RED/GREEN cycle and repeat the review until clean.

- [ ] **Step 7: Publish the feature branch without deployment**

```bash
git push -u fork ticket07/provider-failure-routes
git rev-parse HEAD
git rev-parse fork/ticket07/provider-failure-routes
git status --short --branch
```

Expected: local and remote heads match; worktree is clean. Do not build or install a replacement `/home/bio-13/.local/bin/csswitch-gateway` in this task.

### Task 9: Resolve the Wayfinder rollout record

**Files:**
- Modify in main checkout: `.scratch/provider-failure-contract/issues/07-extend-selected-provider-routes.md`
- Modify in main checkout: `.scratch/provider-failure-contract/map.md`

**Interfaces:**
- Consumes: published clean Ticket 07 branch and immutable evidence.
- Produces: resolved Ticket 07 with accurate deterministic-only limitations.

- [ ] **Step 1: Update only Ticket 07 and the map**

Change Ticket 07 status from `open` to `resolved`. Add a resolution containing the published branch/head, exact tested implementation SHA, evidence path, test counts, policy matrix, final review result, no-live/no-install limitation and macOS pending limitation. Add one matching map decision. Do not rewrite Tickets 01-06.

- [ ] **Step 2: Verify and commit the resolution**

```bash
git diff --check
git diff --stat
rg -n '^Status:' .scratch/provider-failure-contract/issues/*.md
git add .scratch/provider-failure-contract/issues/07-extend-selected-provider-routes.md \
  .scratch/provider-failure-contract/map.md
git commit -m "docs: resolve API-key provider failure rollout"
```

Expected: only Ticket 07 and the map are changed; earlier tickets remain resolved without content edits.

- [ ] **Step 3: Review and publish the main record**

Perform a read-only accuracy review against the published feature/evidence refs. Then:

```bash
git push fork linux-headless-oauth
git rev-parse HEAD
git rev-parse fork/linux-headless-oauth
git status --short --branch
```

Expected: main local/remote refs match and remain clean. The installed runtime is still the Ticket 06 build until a separate deployment approval.
