# Codex Provider Failure Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Turn all 27 accepted Codex Provider Failure Contract tests green through the real Codex handler while preserving single-POST transport mechanics, enforcing bounded retries and one closed Safe Repair, and emitting only sanitized caller errors and diagnostics.

**Architecture:** Add a pure `provider_failure` deep module that classifies closed observations and owns attempt state, directives, envelopes, and diagnostic snapshots. Keep `CodexTransport::open_responses` responsible for exactly one POST and bounded extraction of allowlisted upstream facts; keep the Codex handler responsible for the attempt loop, cancellable waits, repair mutation, downstream replay barrier, and final diagnostic emission.

**Tech Stack:** Rust 2021, `tiny_http`, `reqwest` blocking client, `serde`/`serde_json`, Cargo unit and integration tests, Git worktrees.

## Global Constraints

- Work only on a new `feature/codex-provider-failure` branch created from `linux-headless-oauth` in a project-owned isolated worktree.
- Merge `prototype/codex-acceptance-harness` verbatim before the separately approved three-fixture correction.
- Do not use live credentials, external endpoints, installed profiles, proxy mutations, or service restarts.
- Do not implement fallback, provider/model substitution, model discovery, authentication flows, UI work, or the generic API-key adapter.
- `CodexTransport::open_responses` performs exactly one POST and may read at most 16 KiB of a failed response body.
- A retry-only sequence permits at most three total POSTs, using 500 ms then 1,000 ms when no valid `Retry-After` is present.
- An ASCII decimal `Retry-After` delta overrides that transition's fallback and is capped at 60 seconds; zero is valid and recorded.
- Safe Repair is Responses Lite only, first failure only, exact `unsupported_value` plus `tool_choice`, caller absent/auto only, and removes only top-level `"tool_choice": "auto"`.
- A repair sequence permits exactly one additional POST and can never consume a retry budget; a retry sequence can never authorize repair.
- Once an upstream response is accepted or downstream bytes are written, replay and repair are permanently forbidden.
- Cancellation is terminal and authorizes no additional POST.
- Preserve legacy top-level `type`, `error.type`, and `error.message`; add only the approved structured error metadata and omit unknown optionals rather than serializing `null`.
- A request ID is valid only at 1-256 ASCII characters drawn from `[A-Za-z0-9._:-]`.
- Emit exactly one final structured diagnostic per attempt sequence with outcome `completed`, `failed`, or `cancelled`; never include request IDs or arbitrary upstream/caller strings.
- No new dependencies.
- Every production change follows a focused RED-GREEN cycle; do not weaken accepted assertions.
- Run loopback/socket-heavy tests with `--test-threads=1` to avoid known ephemeral-port contention.

---

## File Structure

- Create `desktop/gateway/src/provider_failure.rs`: closed Provider Failure vocabulary, classification, attempt controller, caller envelope, and diagnostic snapshot construction.
- Create `desktop/gateway/src/provider_failure/tests.rs`: exhaustive pure-module state, classification, serialization, budget, and redaction tests.
- Modify `desktop/gateway/src/lib.rs`: register the private `provider_failure` module.
- Modify `desktop/gateway/src/codex_transport.rs`: retain one-POST behavior while extracting bounded, allowlisted failure facts and validating `Retry-After` and request IDs.
- Modify `desktop/gateway/src/server.rs`: execute controller directives, use a cancellable attempt runtime, apply the one exact repair, and defer the sole diagnostic until the reducer reaches a final outcome.
- Modify `desktop/gateway/src/server/codex_acceptance.rs`: preserve the accepted harness and amend only the three approved transient fixtures before making the production contract green.
- Create `docs/evidence/investigations/2026-07-29-codex-provider-failure-implementation.md`: record revisions, toolchain, commands, counts, retry/repair evidence, and redaction boundaries.
- Modify `docs/evidence/investigations/README.md`: link the new Ticket 05 evidence report.
- Modify `.scratch/provider-failure-contract/issues/05-implement-codex-first-slice.md`: mark only Ticket 05 resolved after every gate passes.
- Modify `.scratch/provider-failure-contract/map.md`: record Ticket 05 evidence and leave Tickets 06 and 07 open and unclaimed.

### Task 1: Establish the isolated accepted-harness RED baseline

**Files:**
- Modify: `desktop/gateway/src/server.rs`
- Create: `desktop/gateway/src/server/codex_acceptance.rs`
- Modify: `desktop/gateway/src/server/codex_acceptance.rs:826-845,1253-1283`

**Interfaces:**
- Consumes: accepted commits `fe57d97` (`prototype/codex-acceptance-harness`) and the current `linux-headless-oauth` tip containing this plan.
- Produces: branch `feature/codex-provider-failure`, an isolated worktree, the exact accepted harness, and three corrected transient fixtures that require three POSTs.

- [ ] **Step 1: Create the execution worktree from the reviewed main tip**

Run from the repository root:

```bash
git worktree add .worktrees/codex-provider-failure -b feature/codex-provider-failure linux-headless-oauth
git -C .worktrees/codex-provider-failure status --short --branch
```

Expected: the new branch is clean and based on the commit containing this plan.

Run every remaining Task 1 step and Tasks 2-9 from `.worktrees/codex-provider-failure`; commands without `git -C` assume that worktree is the current directory. Task 10 returns to the repository's main worktree.

- [ ] **Step 2: Merge the accepted harness verbatim**

```bash
git -C .worktrees/codex-provider-failure merge --no-ff prototype/codex-acceptance-harness -m "test: merge accepted Codex contract harness"
git -C .worktrees/codex-provider-failure diff fe57d97 -- desktop/gateway/src/server/codex_acceptance.rs
```

Expected: the merge succeeds; the second command prints no diff for the accepted harness file.

- [ ] **Step 3: Reproduce the historical accepted RED baseline before amending fixtures**

```bash
cargo test --manifest-path desktop/gateway/Cargo.toml --features acceptance-build 'server::codex_acceptance::harness_' -- --test-threads=1
cargo test --manifest-path desktop/gateway/Cargo.toml --features acceptance-build 'server::codex_acceptance::contract_' -- --test-threads=1
```

Expected: all 8 `harness_` tests pass; exactly 4 of 27 `contract_` tests pass and 23 fail for the missing production behavior documented by Ticket 03.

- [ ] **Step 4: Strengthen the three approved transient fixtures**

Replace the `steps` field in `contract_failure_schema_and_caller_redaction` with:

```rust
steps: vec![redaction_step(), redaction_step(), redaction_step()],
```

Replace `assert_invalid_request_id_is_omitted` with:

```rust
fn invalid_request_id_step(request_id: &str) -> UpstreamStep {
    UpstreamStep::json_with_headers(
        500,
        "Internal Server Error",
        serde_json::json!({"error":{"code":"synthetic_transient"}}),
        vec![("x-request-id", request_id)],
    )
}

fn assert_invalid_request_id_is_omitted(request_id: &str) {
    let result = run_case(AcceptanceCase {
        request: anthropic_request(false),
        is_stream: false,
        use_responses_lite: false,
        endpoint_path: "/responses",
        steps: vec![
            invalid_request_id_step(request_id),
            invalid_request_id_step(request_id),
            invalid_request_id_step(request_id),
        ],
    });
    assert_eq!(result.script.requests.len(), 3);
    assert_eq!(result.script.remaining_steps, 0);
    assert_eq!(result.script.unexpected_posts, 0);
    assert_optional_metadata_absent(&result, &["request_id", "retry_after_seconds"]);
    assert_failure_envelope(
        &result,
        502,
        "api_error",
        "responses",
        "transient",
        Some(500),
        true,
    );
}
```

Add the same exact POST-count assertions immediately after `run_case` in `contract_failure_schema_and_caller_redaction`:

```rust
assert_eq!(result.script.requests.len(), 3);
assert_eq!(result.script.remaining_steps, 0);
assert_eq!(result.script.unexpected_posts, 0);
```

- [ ] **Step 5: Verify the corrected fixtures remain RED against unchanged production code**

```bash
cargo test --manifest-path desktop/gateway/Cargo.toml --features acceptance-build 'server::codex_acceptance::contract_failure_schema_and_caller_redaction' -- --test-threads=1
cargo test --manifest-path desktop/gateway/Cargo.toml --features acceptance-build 'server::codex_acceptance::contract_malformed_request_id_is_omitted' -- --test-threads=1
cargo test --manifest-path desktop/gateway/Cargo.toml --features acceptance-build 'server::codex_acceptance::contract_oversized_request_id_is_omitted' -- --test-threads=1
```

Expected: each test fails because the current handler makes one POST rather than the required three; no production file differs from the accepted harness merge.

- [ ] **Step 6: Commit the approved fixture correction**

```bash
git -C .worktrees/codex-provider-failure add desktop/gateway/src/server/codex_acceptance.rs
git -C .worktrees/codex-provider-failure commit -m "test: align Codex transient fixtures with retry policy"
```

Expected: one test-only commit follows the verbatim accepted-harness merge.

### Task 2: Add closed Provider Failure values and envelope serialization

**Files:**
- Create: `desktop/gateway/src/provider_failure.rs`
- Create: `desktop/gateway/src/provider_failure/tests.rs`
- Modify: `desktop/gateway/src/lib.rs:1-18`

**Interfaces:**
- Consumes: no transport or server types.
- Produces: `ProviderId`, `RouteMode`, `CorrelationId`, `RequestId`, `RetryPolicy`, `RouteContext`, `RateKind`, `ErrorCode`, `ErrorParam`, `NetworkKind`, `ProtocolKind`, `FailureObservation`, `FailureClass`, `ProviderFailure`, `AttemptOutcome`, and `AttemptDiagnostic`.

- [ ] **Step 1: Register the private module and write failing value/envelope tests**

Add to `desktop/gateway/src/lib.rs` after `provider_contracts`:

```rust
pub(crate) mod provider_failure;
```

Create `desktop/gateway/src/provider_failure.rs` with only:

```rust
#[cfg(test)]
mod tests;
```

Create `desktop/gateway/src/provider_failure/tests.rs`:

```rust
use super::*;

fn context(route: RouteMode) -> RouteContext {
    RouteContext::codex(route, CorrelationId::new("corr-0001").unwrap())
}

#[test]
fn request_id_uses_positive_allowlist_and_length_bound() {
    assert_eq!(RequestId::new("Req_01.a:b-c").unwrap().as_str(), "Req_01.a:b-c");
    assert!(RequestId::new("").is_none());
    assert!(RequestId::new("request id").is_none());
    assert!(RequestId::new("request/id").is_none());
    assert!(RequestId::new(&"r".repeat(257)).is_none());
}

#[test]
fn permanent_failure_preserves_legacy_shape_and_omits_unknown_optionals() {
    let failure = ProviderFailure::from_observation(
        &context(RouteMode::Responses),
        &FailureObservation::Http {
            status: 422,
            rate_kind: None,
            retry_after_seconds: None,
            request_id: None,
            error_code: ErrorCode::Absent,
            error_param: ErrorParam::Absent,
        },
        false,
    );
    assert_eq!(failure.status(), 422);
    assert_eq!(
        failure.anthropic_json(),
        serde_json::json!({
            "type": "error",
            "error": {
                "type": "invalid_request_error",
                "message": "Provider rejected the request",
                "provider": "codex",
                "route": "responses",
                "failure_class": "invalid_request",
                "retryable": false,
                "correlation_id": "corr-0001",
                "recovery": "Correct the request or select a compatible model",
                "upstream_status": 422
            }
        })
    );
}

#[test]
fn transient_failure_projects_only_valid_optional_metadata() {
    let failure = ProviderFailure::from_observation(
        &context(RouteMode::ResponsesLite),
        &FailureObservation::Http {
            status: 500,
            rate_kind: None,
            retry_after_seconds: Some(7),
            request_id: RequestId::new("req-safe-500"),
            error_code: ErrorCode::Absent,
            error_param: ErrorParam::Absent,
        },
        true,
    );
    let json = failure.anthropic_json();
    assert_eq!(failure.status(), 502);
    assert_eq!(json["error"]["type"], "api_error");
    assert_eq!(json["error"]["route"], "responses_lite");
    assert_eq!(json["error"]["failure_class"], "transient");
    assert_eq!(json["error"]["retryable"], true);
    assert_eq!(json["error"]["upstream_status"], 500);
    assert_eq!(json["error"]["request_id"], "req-safe-500");
    assert_eq!(json["error"]["retry_after_seconds"], 7);
}

#[test]
fn quota_and_rate_limit_are_distinct() {
    let quota = ProviderFailure::from_observation(
        &context(RouteMode::Responses),
        &FailureObservation::Http {
            status: 429,
            rate_kind: Some(RateKind::Quota),
            retry_after_seconds: None,
            request_id: None,
            error_code: ErrorCode::InsufficientQuota,
            error_param: ErrorParam::Absent,
        },
        false,
    );
    let rate = ProviderFailure::from_observation(
        &context(RouteMode::Responses),
        &FailureObservation::Http {
            status: 429,
            rate_kind: Some(RateKind::RateLimit),
            retry_after_seconds: None,
            request_id: None,
            error_code: ErrorCode::RateLimitExceeded,
            error_param: ErrorParam::Absent,
        },
        true,
    );
    assert_eq!(quota.failure_class(), FailureClass::Quota);
    assert!(!quota.retryable());
    assert_eq!(rate.failure_class(), FailureClass::RateLimit);
    assert!(rate.retryable());
}

#[test]
fn diagnostic_schema_has_no_arbitrary_string_slot() {
    let diagnostic = AttemptDiagnostic::failed(
        &context(RouteMode::Responses),
        3,
        0,
        vec![500, 1_000],
        &ProviderFailure::from_observation(
            &context(RouteMode::Responses),
            &FailureObservation::Network(NetworkKind::Connect),
            true,
        ),
    );
    assert_eq!(
        serde_json::to_value(diagnostic).unwrap(),
        serde_json::json!({
            "outcome": "failed",
            "provider": "codex",
            "route": "responses",
            "correlation_id": "corr-0001",
            "posts": 3,
            "repairs": 0,
            "delays_ms": [500, 1000],
            "mapped_status": 502,
            "failure_class": "network",
            "retryable": true
        })
    );
}
```

- [ ] **Step 2: Run the focused tests to verify RED**

```bash
cargo test --manifest-path desktop/gateway/Cargo.toml provider_failure::tests -- --test-threads=1
```

Expected: compilation fails because the closed types and constructors do not exist.

- [ ] **Step 3: Implement the closed values and validated wrappers**

Replace `desktop/gateway/src/provider_failure.rs` with these public-to-crate declarations and exact labels:

```rust
use std::fmt;

use serde::Serialize;
use serde_json::{json, Value};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ProviderId { Codex }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RouteMode { Responses, ResponsesLite }

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct CorrelationId(String);

impl CorrelationId {
    pub(crate) fn new(value: impl Into<String>) -> Option<Self> {
        let value = value.into();
        RequestId::is_valid(&value).then_some(Self(value))
    }
    pub(crate) fn as_str(&self) -> &str { &self.0 }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RequestId(String);

impl RequestId {
    pub(crate) fn new(value: &str) -> Option<Self> {
        Self::is_valid(value).then(|| Self(value.to_owned()))
    }
    fn is_valid(value: &str) -> bool {
        !value.is_empty()
            && value.len() <= 256
            && value.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':')
            })
    }
    pub(crate) fn as_str(&self) -> &str { &self.0 }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RetryPolicy {
    pub(crate) max_posts: u8,
    pub(crate) fallback_delays_ms: [u64; 2],
    pub(crate) retry_after_cap_seconds: u64,
}

impl RetryPolicy {
    pub(crate) const CODEX: Self = Self {
        max_posts: 3,
        fallback_delays_ms: [500, 1_000],
        retry_after_cap_seconds: 60,
    };
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RouteContext {
    pub(crate) provider: ProviderId,
    pub(crate) route: RouteMode,
    pub(crate) correlation_id: CorrelationId,
    pub(crate) retry_policy: RetryPolicy,
}

impl RouteContext {
    pub(crate) fn codex(route: RouteMode, correlation_id: CorrelationId) -> Self {
        Self { provider: ProviderId::Codex, route, correlation_id, retry_policy: RetryPolicy::CODEX }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RateKind { RateLimit, Quota }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ErrorCode { Absent, UnsupportedValue, InsufficientQuota, RateLimitExceeded, Other }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ErrorParam { Absent, ToolChoice, Other }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NetworkKind { Connect, Timeout, Read }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProtocolKind { InvalidResponse }

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum FailureObservation {
    Http {
        status: u16,
        rate_kind: Option<RateKind>,
        retry_after_seconds: Option<u64>,
        request_id: Option<RequestId>,
        error_code: ErrorCode,
        error_param: ErrorParam,
    },
    Network(NetworkKind),
    Protocol(ProtocolKind),
    Cancelled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum FailureClass {
    Authentication, Authorization, InvalidRequest, Capability, Quota,
    RateLimit, Transient, Network, Protocol,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AttemptOutcome { Completed, Failed, Cancelled }

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct ProviderFailure {
    status: u16,
    error_type: &'static str,
    message: &'static str,
    context: RouteContext,
    failure_class: FailureClass,
    upstream_status: Option<u16>,
    retryable: bool,
    recovery: &'static str,
    request_id: Option<RequestId>,
    retry_after_seconds: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct AttemptDiagnostic {
    outcome: AttemptOutcome,
    provider: ProviderId,
    route: RouteMode,
    correlation_id: CorrelationId,
    posts: u8,
    repairs: u8,
    delays_ms: Vec<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    mapped_status: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    upstream_status: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    failure_class: Option<FailureClass>,
    #[serde(skip_serializing_if = "Option::is_none")]
    retryable: Option<bool>,
}

#[cfg(test)]
mod tests;
```

Implement `ProviderFailure::from_observation`, its accessors, its bounded debug representation, and caller serialization exactly as:

```rust
impl ProviderFailure {
    pub(crate) fn from_observation(
        context: &RouteContext,
        observation: &FailureObservation,
        exhausted: bool,
    ) -> Self {
        let (upstream_status, request_id, retry_after_seconds) = match observation {
            FailureObservation::Http { status, request_id, retry_after_seconds, .. } => {
                (Some(*status), request_id.clone(), *retry_after_seconds)
            }
            FailureObservation::Network(_) | FailureObservation::Protocol(_) => (None, None, None),
            FailureObservation::Cancelled => unreachable!("cancellation has no caller failure"),
        };
        let (status, error_type, message, failure_class, retryable, recovery) = match observation {
            FailureObservation::Http { status: 401, .. } => (
                401, "authentication_error", "Provider authentication failed",
                FailureClass::Authentication, false,
                "Re-authenticate, then start a new caller request",
            ),
            FailureObservation::Http { status: 403, .. } => (
                403, "permission_error", "Provider authorization failed",
                FailureClass::Authorization, false,
                "Check account, workspace, geography, and model entitlement",
            ),
            FailureObservation::Http {
                status: 400,
                error_code: ErrorCode::UnsupportedValue,
                ..
            }
            | FailureObservation::Http {
                status: 400,
                error_param: ErrorParam::ToolChoice,
                ..
            } => (
                400, "invalid_request_error", "Provider Route cannot preserve this request",
                FailureClass::Capability, false,
                "Use equivalent supported request semantics or select a compatible model",
            ),
            FailureObservation::Http { status: 429, rate_kind: Some(RateKind::Quota), .. } => (
                429, "rate_limit_error", "Provider quota prevents this request",
                FailureClass::Quota, false,
                "Restore account quota before starting a new request",
            ),
            FailureObservation::Http { status: 429, rate_kind: Some(RateKind::RateLimit), .. } => (
                429, "rate_limit_error", "Provider rate limit prevents this request",
                FailureClass::RateLimit, exhausted,
                "Wait for rate capacity before starting a new request",
            ),
            FailureObservation::Http { status: 429, .. } => (
                429, "rate_limit_error", "Provider rate limit prevents this request",
                FailureClass::RateLimit, false,
                "Wait for rate capacity before starting a new request",
            ),
            FailureObservation::Http { status: 408, .. } => (
                504, "api_error", "Provider transient failure exhausted retry budget",
                FailureClass::Transient, exhausted,
                "Start a new request after the provider recovers",
            ),
            FailureObservation::Http { status: 409 | 500..=599, .. } => (
                502, "api_error", "Provider transient failure exhausted retry budget",
                FailureClass::Transient, exhausted,
                "Start a new request after the provider recovers",
            ),
            FailureObservation::Http { status, .. } if (400..=499).contains(status) => (
                *status, "invalid_request_error", "Provider rejected the request",
                FailureClass::InvalidRequest, false,
                "Correct the request or select a compatible model",
            ),
            FailureObservation::Http { .. } => (
                502, "api_error", "Provider returned an unsupported status",
                FailureClass::Protocol, false,
                "Inspect sanitized diagnostics and start a new request",
            ),
            FailureObservation::Network(NetworkKind::Timeout) => (
                504, "api_error", "Provider network failure exhausted retry budget",
                FailureClass::Network, exhausted,
                "Check the network route before starting a new request",
            ),
            FailureObservation::Network(NetworkKind::Connect | NetworkKind::Read) => (
                502, "api_error", "Provider network failure exhausted retry budget",
                FailureClass::Network, exhausted,
                "Check the network route before starting a new request",
            ),
            FailureObservation::Protocol(ProtocolKind::InvalidResponse) => (
                502, "api_error", "Provider response violated the expected protocol",
                FailureClass::Protocol, false,
                "Inspect sanitized diagnostics and start a new request",
            ),
            FailureObservation::Cancelled => unreachable!("cancellation has no caller failure"),
        };
        Self {
            status, error_type, message, context: context.clone(), failure_class,
            upstream_status, retryable, recovery, request_id, retry_after_seconds,
        }
    }

    pub(crate) fn status(&self) -> u16 { self.status }
    pub(crate) fn failure_class(&self) -> FailureClass { self.failure_class }
    pub(crate) fn retryable(&self) -> bool { self.retryable }

    pub(crate) fn anthropic_json(&self) -> Value {
        let mut error = json!({
            "type": self.error_type,
            "message": self.message,
            "provider": self.context.provider,
            "route": self.context.route,
            "failure_class": self.failure_class,
            "retryable": self.retryable,
            "correlation_id": self.context.correlation_id.as_str(),
            "recovery": self.recovery,
        });
        if let Some(status) = self.upstream_status { error["upstream_status"] = json!(status); }
        if let Some(request_id) = &self.request_id { error["request_id"] = json!(request_id.as_str()); }
        if let Some(seconds) = self.retry_after_seconds { error["retry_after_seconds"] = json!(seconds.min(60)); }
        json!({ "type": "error", "error": error })
    }
}

impl fmt::Debug for ProviderFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderFailure")
            .field("status", &self.status)
            .field("error_type", &self.error_type)
            .field("failure_class", &self.failure_class)
            .field("upstream_status", &self.upstream_status)
            .field("retryable", &self.retryable)
            .field("correlation_id", &self.context.correlation_id.as_str())
            .finish()
    }
}
```

The capability mapping deliberately recognizes either closed typed signal for classification, while Task 3 authorizes repair only for their exact pair. Message prose is never inspected.

Implement `AttemptDiagnostic::failed` exactly as:

```rust
impl AttemptDiagnostic {
    pub(crate) fn failed(
        context: &RouteContext,
        posts: u8,
        repairs: u8,
        delays_ms: Vec<u64>,
        failure: &ProviderFailure,
    ) -> Self {
        Self {
            outcome: AttemptOutcome::Failed,
            provider: context.provider,
            route: context.route,
            correlation_id: context.correlation_id.clone(),
            posts,
            repairs,
            delays_ms,
            mapped_status: Some(failure.status),
            upstream_status: failure.upstream_status,
            failure_class: Some(failure.failure_class),
            retryable: Some(failure.retryable),
        }
    }
}
```

- [ ] **Step 4: Run value/envelope tests to verify GREEN**

```bash
cargo test --manifest-path desktop/gateway/Cargo.toml provider_failure::tests -- --test-threads=1
```

Expected: the five new tests pass.

- [ ] **Step 5: Format, inspect the public surface, and commit**

```bash
cargo fmt --manifest-path desktop/gateway/Cargo.toml
rg -n "String|&str" desktop/gateway/src/provider_failure.rs
git add desktop/gateway/src/lib.rs desktop/gateway/src/provider_failure.rs desktop/gateway/src/provider_failure/tests.rs
git commit -m "feat: add closed provider failure values"
```

Expected: the only owned strings are validated `CorrelationId` and `RequestId`; no failure observation or diagnostic accepts an arbitrary string.

### Task 3: Add the pure monotonic Attempt Controller

**Files:**
- Modify: `desktop/gateway/src/provider_failure.rs`
- Modify: `desktop/gateway/src/provider_failure/tests.rs`

**Interfaces:**
- Consumes: `RouteContext`, `FailureObservation`, `ProviderFailure`, and closed enums from Task 2.
- Produces: `AttemptController::new`, `begin_post`, `mark_response_started`, `observe`, `snapshot`, `AttemptDirective`, `RepairKind`, and `AttemptSnapshot`.

- [ ] **Step 1: Write failing state-machine tests**

Append to `desktop/gateway/src/provider_failure/tests.rs`:

```rust
fn http(status: u16, rate_kind: Option<RateKind>, retry_after_seconds: Option<u64>) -> FailureObservation {
    FailureObservation::Http {
        status,
        rate_kind,
        retry_after_seconds,
        request_id: None,
        error_code: ErrorCode::Absent,
        error_param: ErrorParam::Absent,
    }
}

#[test]
fn transient_sequence_uses_override_then_fallback_and_stops_at_three_posts() {
    let mut controller = AttemptController::new(context(RouteMode::Responses), false);
    controller.begin_post().unwrap();
    assert_eq!(controller.observe(http(500, None, Some(90))).unwrap(), AttemptDirective::RetryAfter(60_000));
    controller.begin_post().unwrap();
    assert_eq!(controller.observe(http(500, None, None)).unwrap(), AttemptDirective::RetryAfter(1_000));
    controller.begin_post().unwrap();
    let failure = match controller.observe(http(500, None, None)).unwrap() {
        AttemptDirective::Fail(failure) => failure,
        directive => panic!("expected terminal failure, got {directive:?}"),
    };
    assert_eq!(failure.status(), 502);
    assert!(failure.retryable());
    assert_eq!(controller.snapshot().posts, 3);
    assert_eq!(controller.snapshot().delays_ms, vec![60_000, 1_000]);
    assert!(controller.begin_post().is_err());
}

#[test]
fn quota_and_unknown_429_do_not_retry() {
    let mut quota = AttemptController::new(context(RouteMode::Responses), false);
    quota.begin_post().unwrap();
    let quota_failure = match quota.observe(http(429, Some(RateKind::Quota), Some(0))).unwrap() {
        AttemptDirective::Fail(failure) => failure,
        directive => panic!("expected quota failure, got {directive:?}"),
    };
    assert_eq!(quota_failure.failure_class(), FailureClass::Quota);
    assert!(!quota_failure.retryable());
    assert_eq!(quota.snapshot().posts, 1);

    let mut unknown = AttemptController::new(context(RouteMode::Responses), false);
    unknown.begin_post().unwrap();
    let unknown_failure = match unknown.observe(http(429, None, None)).unwrap() {
        AttemptDirective::Fail(failure) => failure,
        directive => panic!("expected unknown-rate failure, got {directive:?}"),
    };
    assert_eq!(unknown_failure.failure_class(), FailureClass::RateLimit);
    assert!(!unknown_failure.retryable());
    assert_eq!(unknown.snapshot().posts, 1);
}

#[test]
fn exact_first_post_lite_capability_can_repair_once_without_retry_budget() {
    let mut controller = AttemptController::new(context(RouteMode::ResponsesLite), true);
    controller.begin_post().unwrap();
    let observation = FailureObservation::Http {
        status: 400,
        rate_kind: None,
        retry_after_seconds: None,
        request_id: None,
        error_code: ErrorCode::UnsupportedValue,
        error_param: ErrorParam::ToolChoice,
    };
    assert_eq!(controller.observe(observation.clone()).unwrap(), AttemptDirective::RepairOnce(RepairKind::OmitAutomaticToolChoice));
    controller.begin_post().unwrap();
    assert!(matches!(controller.observe(observation).unwrap(), AttemptDirective::Fail(_)));
    assert_eq!(controller.snapshot().posts, 2);
    assert_eq!(controller.snapshot().repairs, 1);
    assert!(controller.snapshot().delays_ms.is_empty());
    assert!(controller.begin_post().is_err());
}

#[test]
fn repair_is_forbidden_after_retry_or_response_start() {
    let capability = FailureObservation::Http {
        status: 400,
        rate_kind: None,
        retry_after_seconds: None,
        request_id: None,
        error_code: ErrorCode::UnsupportedValue,
        error_param: ErrorParam::ToolChoice,
    };

    let mut after_retry = AttemptController::new(context(RouteMode::ResponsesLite), true);
    after_retry.begin_post().unwrap();
    assert!(matches!(after_retry.observe(http(500, None, None)).unwrap(), AttemptDirective::RetryAfter(500)));
    after_retry.begin_post().unwrap();
    assert!(matches!(after_retry.observe(capability.clone()).unwrap(), AttemptDirective::Fail(_)));

    let mut after_start = AttemptController::new(context(RouteMode::ResponsesLite), true);
    after_start.begin_post().unwrap();
    after_start.mark_response_started().unwrap();
    assert!(matches!(after_start.observe(FailureObservation::Protocol(ProtocolKind::InvalidResponse)).unwrap(), AttemptDirective::Fail(_)));
    assert!(after_start.begin_post().is_err());
}

#[test]
fn cancellation_is_terminal_from_ready_retry_and_inflight() {
    for mut controller in [
        AttemptController::new(context(RouteMode::Responses), false),
        {
            let mut value = AttemptController::new(context(RouteMode::Responses), false);
            value.begin_post().unwrap();
            value.observe(http(500, None, None)).unwrap();
            value
        },
        {
            let mut value = AttemptController::new(context(RouteMode::Responses), false);
            value.begin_post().unwrap();
            value
        },
    ] {
        assert_eq!(controller.observe(FailureObservation::Cancelled).unwrap(), AttemptDirective::Cancel);
        assert!(controller.begin_post().is_err());
    }
}
```

- [ ] **Step 2: Run the controller tests to verify RED**

```bash
cargo test --manifest-path desktop/gateway/Cargo.toml provider_failure::tests::transient_sequence_uses_override_then_fallback_and_stops_at_three_posts -- --test-threads=1
cargo test --manifest-path desktop/gateway/Cargo.toml provider_failure::tests::quota_and_unknown_429_do_not_retry -- --test-threads=1
cargo test --manifest-path desktop/gateway/Cargo.toml provider_failure::tests::exact_first_post_lite_capability_can_repair_once_without_retry_budget -- --test-threads=1
cargo test --manifest-path desktop/gateway/Cargo.toml provider_failure::tests::repair_is_forbidden_after_retry_or_response_start -- --test-threads=1
cargo test --manifest-path desktop/gateway/Cargo.toml provider_failure::tests::cancellation_is_terminal_from_ready_retry_and_inflight -- --test-threads=1
```

Expected: compilation fails because controller types and methods are absent.

- [ ] **Step 3: Add the state, directive, and transition types**

Add to `desktop/gateway/src/provider_failure.rs` before the test module:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RepairKind { OmitAutomaticToolChoice }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AttemptPhase { Ready, InFlight, RetryAuthorized, RepairAuthorized, UpstreamOpen, Terminal }

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum AttemptDirective {
    RetryAfter(u64),
    RepairOnce(RepairKind),
    Fail(ProviderFailure),
    Cancel,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TransitionError { PostNotAuthorized, ObservationNotAuthorized, ResponseAlreadyStarted }

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AttemptSnapshot {
    pub(crate) posts: u8,
    pub(crate) repairs: u8,
    pub(crate) delays_ms: Vec<u64>,
    pub(crate) response_started: bool,
}

pub(crate) struct AttemptController {
    context: RouteContext,
    repair_enabled: bool,
    phase: AttemptPhase,
    posts: u8,
    repairs: u8,
    delays_ms: Vec<u64>,
    response_started: bool,
}
```

- [ ] **Step 4: Implement the controller transitions**

Implement `AttemptController` with these exact methods and decision order:

```rust
impl AttemptController {
    pub(crate) fn new(context: RouteContext, repair_enabled: bool) -> Self {
        Self { context, repair_enabled, phase: AttemptPhase::Ready, posts: 0, repairs: 0, delays_ms: Vec::new(), response_started: false }
    }

    pub(crate) fn snapshot(&self) -> AttemptSnapshot {
        AttemptSnapshot { posts: self.posts, repairs: self.repairs, delays_ms: self.delays_ms.clone(), response_started: self.response_started }
    }

    pub(crate) fn begin_post(&mut self) -> Result<(), TransitionError> {
        let authorized = matches!(self.phase, AttemptPhase::Ready | AttemptPhase::RetryAuthorized | AttemptPhase::RepairAuthorized);
        let maximum = if self.repairs == 0 { self.context.retry_policy.max_posts } else { 2 };
        if !authorized || self.posts >= maximum { return Err(TransitionError::PostNotAuthorized); }
        self.posts += 1;
        self.phase = AttemptPhase::InFlight;
        Ok(())
    }

    pub(crate) fn mark_response_started(&mut self) -> Result<(), TransitionError> {
        if self.phase != AttemptPhase::InFlight || self.response_started { return Err(TransitionError::ResponseAlreadyStarted); }
        self.response_started = true;
        self.phase = AttemptPhase::UpstreamOpen;
        Ok(())
    }

    pub(crate) fn observe(&mut self, observation: FailureObservation) -> Result<AttemptDirective, TransitionError> {
        if matches!(&observation, FailureObservation::Cancelled) {
            if self.phase == AttemptPhase::Terminal { return Err(TransitionError::ObservationNotAuthorized); }
            self.phase = AttemptPhase::Terminal;
            return Ok(AttemptDirective::Cancel);
        }
        if !matches!(self.phase, AttemptPhase::InFlight | AttemptPhase::UpstreamOpen) {
            return Err(TransitionError::ObservationNotAuthorized);
        }
        if self.response_started {
            self.phase = AttemptPhase::Terminal;
            return Ok(AttemptDirective::Fail(ProviderFailure::from_observation(&self.context, &observation, false)));
        }

        let exact_repair = matches!(
            &observation,
            FailureObservation::Http { status: 400, error_code: ErrorCode::UnsupportedValue, error_param: ErrorParam::ToolChoice, .. }
        );
        let can_repair = exact_repair
            && self.repair_enabled
            && self.context.route == RouteMode::ResponsesLite
            && self.posts == 1
            && self.repairs == 0
            && self.delays_ms.is_empty();
        if can_repair {
            self.repairs = 1;
            self.phase = AttemptPhase::RepairAuthorized;
            return Ok(AttemptDirective::RepairOnce(RepairKind::OmitAutomaticToolChoice));
        }

        let proven_rate = matches!(&observation, FailureObservation::Http { status: 429, rate_kind: Some(RateKind::RateLimit), .. });
        let transient_http = matches!(&observation, FailureObservation::Http { status: 408 | 409 | 500..=599, .. });
        let transient_network = matches!(&observation, FailureObservation::Network(_));
        let can_retry = self.repairs == 0
            && self.posts < self.context.retry_policy.max_posts
            && (proven_rate || transient_http || transient_network);
        if can_retry {
            let override_seconds = match &observation {
                FailureObservation::Http { retry_after_seconds, .. } => *retry_after_seconds,
                FailureObservation::Network(_) | FailureObservation::Protocol(_) | FailureObservation::Cancelled => None,
            };
            let delay_ms = override_seconds
                .map(|seconds| seconds.min(self.context.retry_policy.retry_after_cap_seconds) * 1_000)
                .unwrap_or(self.context.retry_policy.fallback_delays_ms[(self.posts - 1) as usize]);
            self.delays_ms.push(delay_ms);
            self.phase = AttemptPhase::RetryAuthorized;
            return Ok(AttemptDirective::RetryAfter(delay_ms));
        }

        let exhausted = proven_rate || transient_http || transient_network;
        self.phase = AttemptPhase::Terminal;
        Ok(AttemptDirective::Fail(ProviderFailure::from_observation(&self.context, &observation, exhausted)))
    }
}
```

Also map an HTTP 400 carrying either the `unsupported_value` code or the `tool_choice` parameter to `FailureClass::Capability` inside `ProviderFailure::from_observation` before the generic permanent-4xx arm. The exact pair remains the only repair authorization.

- [ ] **Step 5: Run all pure tests to verify GREEN**

```bash
cargo test --manifest-path desktop/gateway/Cargo.toml provider_failure::tests -- --test-threads=1
```

Expected: all Task 2 and Task 3 pure tests pass; the retry snapshots are exactly `[60000, 1000]`, repair snapshots contain no delay, and cancellation permits no next POST.

- [ ] **Step 6: Format and commit**

```bash
cargo fmt --manifest-path desktop/gateway/Cargo.toml
git add desktop/gateway/src/provider_failure.rs desktop/gateway/src/provider_failure/tests.rs
git commit -m "feat: add provider attempt controller"
```

### Task 4: Project bounded allowlisted facts from one Codex POST

**Files:**
- Modify: `desktop/gateway/src/codex_transport.rs:1-455`
- Modify: `desktop/gateway/src/codex_transport.rs:469-760`

**Interfaces:**
- Consumes: `ErrorCode`, `ErrorParam`, `FailureObservation`, `NetworkKind`, `ProtocolKind`, `RateKind`, and `RequestId` from Task 2.
- Produces: `CodexTransportError::observation()` returning a cloned closed `FailureObservation`; preserves `status`, `upstream_status`, `detail`, and `cancelled` for existing callers during staged integration.

- [ ] **Step 1: Write failing extraction and redaction tests**

Inside `codex_transport.rs`'s existing test module, import the closed types and append:

```rust
use crate::provider_failure::{ErrorCode, ErrorParam, FailureObservation, RateKind};

#[test]
fn failed_response_extracts_only_allowlisted_facts() {
    let body = r#"{"error":{"type":"rate_limit_error","code":"rate_limit_exceeded","param":"other","message":"BODY_SECRET"}}"#;
    let response = format!(
        "HTTP/1.1 429 Too Many Requests\r\nRetry-After: 90\r\nx-request-id: Req_01.a:b-c\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    ).into_bytes();
    let (endpoint, request_rx, handle) = mock_server(response);
    let transport = CodexTransport::for_test(endpoint).unwrap();
    let error = transport
        .open_responses(&secrets(), b"{}".to_vec(), false, CodexCancellation::default())
        .err()
        .expect("synthetic 429 must fail");
    assert_eq!(
        error.observation(),
        FailureObservation::Http {
            status: 429,
            rate_kind: Some(RateKind::RateLimit),
            retry_after_seconds: Some(90),
            request_id: crate::provider_failure::RequestId::new("Req_01.a:b-c"),
            error_code: ErrorCode::RateLimitExceeded,
            error_param: ErrorParam::Other,
        }
    );
    let diagnostic = format!("{error:?} {error}");
    assert!(!diagnostic.contains("BODY_SECRET"));
    assert!(!diagnostic.contains("Req_01.a:b-c"));
    assert!(request_rx.recv().unwrap().starts_with(b"POST "));
    handle.join().unwrap();
}

#[test]
fn quota_wins_when_type_or_code_proves_insufficient_quota() {
    for body in [
        r#"{"error":{"type":"insufficient_quota","code":"other"}}"#,
        r#"{"error":{"type":"other","code":"insufficient_quota"}}"#,
    ] {
        let facts = reduce_failure_body(body.as_bytes());
        assert_eq!(facts.rate_kind, Some(RateKind::Quota));
        assert_eq!(facts.error_code, ErrorCode::InsufficientQuota);
    }
}

#[test]
fn repair_signal_requires_exact_code_and_parameter() {
    let exact = reduce_failure_body(br#"{"error":{"code":"unsupported_value","param":"tool_choice"}}"#);
    assert_eq!(exact.error_code, ErrorCode::UnsupportedValue);
    assert_eq!(exact.error_param, ErrorParam::ToolChoice);

    for body in [
        br#"{"error":{"message":"unsupported_value tool_choice"}}"#.as_slice(),
        br#"{"error":{"code":"unsupported_value","param":"tools"}}"#.as_slice(),
        br#"{"error":{"code":"other","param":"tool_choice"}}"#.as_slice(),
    ] {
        let facts = reduce_failure_body(body);
        assert!(facts.error_code != ErrorCode::UnsupportedValue || facts.error_param != ErrorParam::ToolChoice);
    }
}

#[test]
fn retry_after_and_request_id_parsers_are_strict() {
    assert_eq!(parse_retry_after(Some("0")), Some(0));
    assert_eq!(parse_retry_after(Some("60")), Some(60));
    assert_eq!(parse_retry_after(Some(" 7 ")), None);
    assert_eq!(parse_retry_after(Some("1.5")), None);
    assert_eq!(parse_retry_after(Some("Wed, 29 Jul 2026 00:00:00 GMT")), None);
    assert_eq!(parse_retry_after(None), None);
    assert_eq!(parse_request_id(Some("req:01-A_b.c")).unwrap().as_str(), "req:01-A_b.c");
    assert!(parse_request_id(Some("request id")).is_none());
    assert!(parse_request_id(Some(&"r".repeat(257))).is_none());
}

#[test]
fn failure_body_reduction_never_reads_semantics_beyond_16_kib() {
    let mut body = vec![b' '; FAILURE_BODY_LIMIT];
    body.extend_from_slice(br#"{"error":{"code":"unsupported_value","param":"tool_choice"}}"#);
    let facts = reduce_failure_body(&body);
    assert_eq!(facts.error_code, ErrorCode::Absent);
    assert_eq!(facts.error_param, ErrorParam::Absent);
}
```

- [ ] **Step 2: Run the transport tests to verify RED**

```bash
cargo test --manifest-path desktop/gateway/Cargo.toml codex_transport::tests::failed_response_extracts_only_allowlisted_facts -- --test-threads=1
cargo test --manifest-path desktop/gateway/Cargo.toml codex_transport::tests::quota_wins_when_type_or_code_proves_insufficient_quota -- --test-threads=1
cargo test --manifest-path desktop/gateway/Cargo.toml codex_transport::tests::repair_signal_requires_exact_code_and_parameter -- --test-threads=1
cargo test --manifest-path desktop/gateway/Cargo.toml codex_transport::tests::retry_after_and_request_id_parsers_are_strict -- --test-threads=1
cargo test --manifest-path desktop/gateway/Cargo.toml codex_transport::tests::failure_body_reduction_never_reads_semantics_beyond_16_kib -- --test-threads=1
```

Expected: compilation fails because projection helpers and the observation field do not exist.

- [ ] **Step 3: Add the closed transport projection**

Add these imports and declarations near the existing transport constants:

```rust
use crate::provider_failure::{
    ErrorCode, ErrorParam, FailureObservation, NetworkKind, ProtocolKind, RateKind, RequestId,
};

const FAILURE_BODY_LIMIT: usize = 16 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FailureBodyFacts {
    rate_kind: Option<RateKind>,
    error_code: ErrorCode,
    error_param: ErrorParam,
}

fn reduce_failure_body(body: &[u8]) -> FailureBodyFacts {
    let bounded = &body[..body.len().min(FAILURE_BODY_LIMIT)];
    let parsed: serde_json::Value = match serde_json::from_slice(bounded) {
        Ok(value) => value,
        Err(_) => return FailureBodyFacts { rate_kind: None, error_code: ErrorCode::Absent, error_param: ErrorParam::Absent },
    };
    let error = &parsed["error"];
    let error_type = error["type"].as_str();
    let code = error["code"].as_str();
    let param = error["param"].as_str();
    let quota = error_type == Some("insufficient_quota") || code == Some("insufficient_quota");
    let rate = error_type == Some("rate_limit_error") || code == Some("rate_limit_exceeded");
    FailureBodyFacts {
        rate_kind: if quota { Some(RateKind::Quota) } else if rate { Some(RateKind::RateLimit) } else { None },
        error_code: match code {
            Some("unsupported_value") => ErrorCode::UnsupportedValue,
            Some("insufficient_quota") => ErrorCode::InsufficientQuota,
            Some("rate_limit_exceeded") => ErrorCode::RateLimitExceeded,
            _ if error_type == Some("insufficient_quota") => ErrorCode::InsufficientQuota,
            Some(_) => ErrorCode::Other,
            None => ErrorCode::Absent,
        },
        error_param: match param {
            Some("tool_choice") => ErrorParam::ToolChoice,
            Some(_) => ErrorParam::Other,
            None => ErrorParam::Absent,
        },
    }
}

fn parse_retry_after(value: Option<&str>) -> Option<u64> {
    let value = value?;
    (!value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()))
        .then(|| value.parse().ok())
        .flatten()
}

fn parse_request_id(value: Option<&str>) -> Option<RequestId> {
    RequestId::new(value?)
}
```

Change `CodexTransportError` from `Clone, Copy, Debug` to `Clone` and add a private `observation: FailureObservation` field. Implement a deliberately bounded debug view and the observation accessor:

```rust
impl CodexTransportError {
    pub(crate) fn observation(&self) -> FailureObservation { self.observation.clone() }
}

impl fmt::Debug for CodexTransportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CodexTransportError")
            .field("status", &self.status)
            .field("upstream_status", &self.upstream_status)
            .field("detail", &self.detail)
            .field("cancelled", &self.cancelled)
            .finish()
    }
}
```

Every local construction site must supply a closed observation:

- invalid local authorization/account headers: HTTP 401 with no optional facts;
- cancellation: `FailureObservation::Cancelled`;
- request timeout: `FailureObservation::Network(NetworkKind::Timeout)`;
- send/connect failure: `FailureObservation::Network(NetworkKind::Connect)`;
- response read failure: `FailureObservation::Network(NetworkKind::Read)`;
- successful-status content-type/prefix failures: `FailureObservation::Protocol(ProtocolKind::InvalidResponse)`.

Use the same closed observation for initialization errors as `NetworkKind::Connect`. Keep all existing `detail` values static and do not add request ID, body text, endpoint, headers, or credentials to `Debug` or `Display`.

Replace the stale method comment with:

```rust
/// Sends exactly one inference POST. Retry and repair policy belongs to the
/// handler-owned Attempt Controller; this method never replays internally.
```

- [ ] **Step 4: Replace the non-success branch with bounded async collection**

Immediately before consuming the response body, clone only the two allowlisted headers:

```rust
let retry_after = response.headers().get("retry-after").and_then(|value| value.to_str().ok()).and_then(|value| parse_retry_after(Some(value)));
let request_id = response.headers().get("x-request-id").and_then(|value| value.to_str().ok()).and_then(|value| parse_request_id(Some(value)));
```

Then replace the current `if !response.status().is_success()` block with:

```rust
if !response.status().is_success() {
    enum BodyRead {
        Body(Vec<u8>),
        Cancelled,
        Unavailable,
    }
    let cancellation_for_body = cancellation.clone();
    let body_read = runtime.block_on(async {
        match tokio::time::timeout(self.request_timeout, async {
            let mut bounded = Vec::with_capacity(FAILURE_BODY_LIMIT);
            while bounded.len() < FAILURE_BODY_LIMIT {
                let next = tokio::select! {
                    _ = wait_for_cancel(cancellation_for_body.clone()) => return BodyRead::Cancelled,
                    next = response.chunk() => match next {
                        Ok(next) => next,
                        Err(_) => return BodyRead::Unavailable,
                    },
                };
                let Some(chunk) = next else { break; };
                let remaining = FAILURE_BODY_LIMIT - bounded.len();
                bounded.extend_from_slice(&chunk[..chunk.len().min(remaining)]);
            }
            BodyRead::Body(bounded)
        }).await {
            Ok(result) => result,
            Err(_) => BodyRead::Unavailable,
        }
    });
    let body = match body_read {
        BodyRead::Body(body) => body,
        BodyRead::Unavailable => Vec::new(),
        BodyRead::Cancelled => {
            return Err(CodexTransportError {
                status: 499,
                upstream_status: Some(status),
                detail: "Codex request was cancelled",
                cancelled: true,
                observation: FailureObservation::Cancelled,
            });
        }
    };
    let facts = reduce_failure_body(&body);
    let observation = FailureObservation::Http {
        status,
        rate_kind: facts.rate_kind,
        retry_after_seconds: retry_after,
        request_id,
        error_code: facts.error_code,
        error_param: facts.error_param,
    };
    return Err(CodexTransportError {
        status: match status { 401 | 403 | 429 => status, 408 => 504, _ => 502 },
        upstream_status: Some(status),
        detail: "Codex upstream rejected the request",
        cancelled: false,
        observation,
    });
}
```

This loop retains and parses at most 16 KiB and stops requesting chunks once that bound is reached. Dropping the response closes the remaining body; it does not retain, parse, or log the suffix. The existing request timeout also bounds a stalled failure body.

- [ ] **Step 5: Run focused and existing transport tests to verify GREEN**

```bash
cargo test --manifest-path desktop/gateway/Cargo.toml codex_transport::tests -- --test-threads=1
```

Expected: all existing transport tests and all five projection tests pass; each mock receives exactly one POST.

- [ ] **Step 6: Run Clippy for the new extraction surface and commit**

```bash
cargo fmt --manifest-path desktop/gateway/Cargo.toml
cargo clippy --offline --manifest-path desktop/gateway/Cargo.toml --lib -- -D warnings
git add desktop/gateway/src/codex_transport.rs
git commit -m "feat: project bounded Codex failure facts"
```

### Task 5: Route terminal Codex failures through the real handler

**Files:**
- Modify: `desktop/gateway/src/provider_failure.rs`
- Modify: `desktop/gateway/src/server.rs:1-25,1070-1175`
- Test: `desktop/gateway/src/server/codex_acceptance.rs:932-963,1290-1370`

**Interfaces:**
- Consumes: `AttemptController`, `AttemptDirective`, `RouteContext`, `RouteMode`, and `CodexTransportError::observation()`.
- Produces: one locally generated correlation ID per caller request and `write_provider_failure` for the backward-compatible structured caller envelope.

- [ ] **Step 1: Run the terminal acceptance cases to capture RED**

```bash
cargo test --manifest-path desktop/gateway/Cargo.toml --features acceptance-build 'server::codex_acceptance::contract_quota_429_is_not_retried' -- --test-threads=1
cargo test --manifest-path desktop/gateway/Cargo.toml --features acceptance-build 'server::codex_acceptance::contract_permanent_' -- --test-threads=1
cargo test --manifest-path desktop/gateway/Cargo.toml --features acceptance-build 'server::codex_acceptance::contract_401_is_authentication_and_not_retried' -- --test-threads=1
cargo test --manifest-path desktop/gateway/Cargo.toml --features acceptance-build 'server::codex_acceptance::contract_403_is_authorization_and_not_retried' -- --test-threads=1
```

Expected: failures show the current generic `api_error` envelope, wrong permanent status/type, and absent structured fields.

- [ ] **Step 2: Add correlation and failure-writing helpers**

Extend the atomic import at the top of `server.rs` to include `AtomicU64`, then add next to `CodexRequestPolicy`:

```rust
static CODEX_CORRELATION_SEQUENCE: AtomicU64 = AtomicU64::new(1);

fn next_codex_correlation_id() -> crate::provider_failure::CorrelationId {
    let sequence = CODEX_CORRELATION_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    crate::provider_failure::CorrelationId::new(format!("codex-{sequence:016x}"))
        .expect("generated Codex correlation ID is allowlisted")
}

fn write_provider_failure(
    stream: &mut TcpStream,
    failure: &crate::provider_failure::ProviderFailure,
) {
    write_json(
        stream,
        failure.status(),
        status_reason(failure.status()),
        failure.anthropic_json(),
    );
}
```

- [ ] **Step 3: Construct the controller and map the first terminal observation**

After request translation succeeds and before encoding, create the route context:

```rust
let route = if policy.use_responses_lite {
    crate::provider_failure::RouteMode::ResponsesLite
} else {
    crate::provider_failure::RouteMode::Responses
};
let route_context = crate::provider_failure::RouteContext::codex(route, next_codex_correlation_id());
let mut controller = crate::provider_failure::AttemptController::new(route_context, false);
```

Immediately before `open_responses`, authorize the POST:

```rust
controller.begin_post().expect("initial Codex POST is authorized");
```

Replace the existing transport error branch with:

```rust
Err(error) => {
    let observation = error.observation();
    if let Some(status @ (401 | 403)) = error.upstream_status {
        auth_rejected(status, generation);
    }
    match controller.observe(observation).expect("in-flight Codex observation is authorized") {
        crate::provider_failure::AttemptDirective::Fail(failure) => {
            write_provider_failure(stream, &failure);
            return;
        }
        crate::provider_failure::AttemptDirective::Cancel => return,
        crate::provider_failure::AttemptDirective::RetryAfter(_) => {
            api_error_json(stream, 500, "Codex retry runtime is not installed");
            return;
        }
        crate::provider_failure::AttemptDirective::RepairOnce(_) => {
            api_error_json(stream, 500, "Codex repair runtime is not installed");
            return;
        }
    }
}
```

The temporary two internal branches are exercised by no Task 5 GREEN test and are deleted, not retained, in Tasks 6 and 7.

- [ ] **Step 4: Verify the handler uses only the Task 2 failure surface**

```bash
rg -n "failure\.(status|anthropic_json)\(\)" desktop/gateway/src/server.rs
rg -n "request_id|retry_after_seconds|upstream_status" desktop/gateway/src/server.rs
```

Expected: the handler calls only `status()` and `anthropic_json()`; it does not reconstruct optional metadata or inspect the upstream error body.

- [ ] **Step 5: Run the terminal acceptance cases to verify GREEN**

```bash
cargo test --manifest-path desktop/gateway/Cargo.toml --features acceptance-build 'server::codex_acceptance::contract_quota_429_is_not_retried' -- --test-threads=1
cargo test --manifest-path desktop/gateway/Cargo.toml --features acceptance-build 'server::codex_acceptance::contract_permanent_' -- --test-threads=1
cargo test --manifest-path desktop/gateway/Cargo.toml --features acceptance-build 'server::codex_acceptance::contract_401_is_authentication_and_not_retried' -- --test-threads=1
cargo test --manifest-path desktop/gateway/Cargo.toml --features acceptance-build 'server::codex_acceptance::contract_403_is_authorization_and_not_retried' -- --test-threads=1
```

Expected: quota posts once with `quota`/`retryable:false`; 400, 404, and 422 preserve status and use `invalid_request_error`; 401 and 403 each post once, invoke the existing auth callback once, and use authentication/permission types.

- [ ] **Step 6: Run existing focused Codex tests and commit**

```bash
cargo test --manifest-path desktop/gateway/Cargo.toml codex_ -- --test-threads=1
cargo fmt --manifest-path desktop/gateway/Cargo.toml
git add desktop/gateway/src/provider_failure.rs desktop/gateway/src/server.rs
git commit -m "feat: return structured terminal Codex failures"
```

Expected: focused tests outside the intentionally red retry/repair contract remain green.

### Task 6: Execute bounded retries through a cancellable testable runtime

**Files:**
- Modify: `desktop/gateway/src/provider_failure.rs`
- Modify: `desktop/gateway/src/server.rs:1070-1185`
- Modify: `desktop/gateway/src/server/codex_acceptance.rs:1-680,826-1059,1253-1283`

**Interfaces:**
- Consumes: `AttemptDirective::RetryAfter`, `AttemptController::snapshot`, and the existing downstream cancellation watch.
- Produces: private `CodexAttemptRuntime::{wait,emit}`, `ProductionCodexAttemptRuntime`, a recording acceptance runtime, and a real handler attempt loop that reuses the exact translated body.

- [ ] **Step 1: Extend the acceptance runner with a no-sleep runtime and exact delay assertions**

In `server/codex_acceptance.rs`, import `AttemptDiagnostic` and the new private runtime trait from `super`. Add:

```rust
#[derive(Default)]
struct RecordingAttemptRuntime {
    delays_ms: Vec<u64>,
    diagnostics: Vec<Value>,
}

impl CodexAttemptRuntime for RecordingAttemptRuntime {
    fn wait(&mut self, delay_ms: u64, cancellation: &CodexCancellation) -> bool {
        self.delays_ms.push(delay_ms);
        !cancellation.is_cancelled()
    }

    fn emit(&mut self, diagnostic: AttemptDiagnostic) {
        self.diagnostics.push(serde_json::to_value(diagnostic).expect("serialize attempt diagnostic"));
    }
}
```

Add `delays_ms: Vec<u64>` and `diagnostics: Vec<Value>` to `AcceptanceResult`. In `run_case`, construct a mutable `RecordingAttemptRuntime`, call `handle_codex_messages_with_policy_and_runtime`, and move both vectors into the result.

Add these assertions to the named accepted cases:

```rust
// contract_rate_limit_retries_twice_then_succeeds
assert_eq!(result.delays_ms, vec![0, 0]);

// contract_rate_limit_exhaustion_caps_retry_after
assert_eq!(result.delays_ms, vec![0, 0]);

// contract_network_and_5xx_retry_within_three_posts
assert_eq!(result.delays_ms, vec![500, 0]);

// contract_network_only_exhaustion_omits_unknown_upstream_metadata
assert_eq!(result.delays_ms, vec![500, 1_000]);

// contract_failure_schema_and_caller_redaction
assert_eq!(result.delays_ms, vec![500, 1_000]);
```

Inside `assert_invalid_request_id_is_omitted`, add:

```rust
assert_eq!(result.delays_ms, vec![500, 1_000]);
```

- [ ] **Step 2: Run the retry acceptance cases to verify RED without sleeping**

```bash
cargo test --manifest-path desktop/gateway/Cargo.toml --features acceptance-build 'server::codex_acceptance::contract_rate_limit_' -- --test-threads=1
cargo test --manifest-path desktop/gateway/Cargo.toml --features acceptance-build 'server::codex_acceptance::contract_network_' -- --test-threads=1
cargo test --manifest-path desktop/gateway/Cargo.toml --features acceptance-build 'server::codex_acceptance::contract_408_exhaustion_returns_504' -- --test-threads=1
cargo test --manifest-path desktop/gateway/Cargo.toml --features acceptance-build 'server::codex_acceptance::contract_409_exhaustion_returns_502' -- --test-threads=1
cargo test --manifest-path desktop/gateway/Cargo.toml --features acceptance-build 'server::codex_acceptance::contract_500_exhaustion_returns_502' -- --test-threads=1
cargo test --manifest-path desktop/gateway/Cargo.toml --features acceptance-build 'server::codex_acceptance::contract_503_exhaustion_returns_502' -- --test-threads=1
cargo test --manifest-path desktop/gateway/Cargo.toml --features acceptance-build 'server::codex_acceptance::contract_failure_schema_and_caller_redaction' -- --test-threads=1
cargo test --manifest-path desktop/gateway/Cargo.toml --features acceptance-build 'server::codex_acceptance::contract_malformed_request_id_is_omitted' -- --test-threads=1
cargo test --manifest-path desktop/gateway/Cargo.toml --features acceptance-build 'server::codex_acceptance::contract_oversized_request_id_is_omitted' -- --test-threads=1
```

Expected: compilation fails because the injectable runtime entry point does not exist.

- [ ] **Step 3: Add diagnostic construction from controller-owned state**

Add to `AttemptController`:

```rust
pub(crate) fn failed_diagnostic(&self, failure: &ProviderFailure) -> AttemptDiagnostic {
    AttemptDiagnostic::failed(
        &self.context,
        self.posts,
        self.repairs,
        self.delays_ms.clone(),
        failure,
    )
}
```

- [ ] **Step 4: Add the runtime seam and cancellation-aware production wait**

In `server.rs`, add:

```rust
trait CodexAttemptRuntime {
    fn wait(&mut self, delay_ms: u64, cancellation: &codex_transport::CodexCancellation) -> bool;
    fn emit(&mut self, diagnostic: crate::provider_failure::AttemptDiagnostic);
}

struct ProductionCodexAttemptRuntime;

impl CodexAttemptRuntime for ProductionCodexAttemptRuntime {
    fn wait(&mut self, delay_ms: u64, cancellation: &codex_transport::CodexCancellation) -> bool {
        let deadline = std::time::Instant::now() + Duration::from_millis(delay_ms);
        while std::time::Instant::now() < deadline {
            if cancellation.is_cancelled() { return false; }
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            thread::sleep(remaining.min(Duration::from_millis(25)));
        }
        !cancellation.is_cancelled()
    }

    fn emit(&mut self, diagnostic: crate::provider_failure::AttemptDiagnostic) {
        if let Ok(value) = serde_json::to_string(&diagnostic) {
            eprintln!("provider_attempt {value}");
        }
    }
}
```

Add this focused unit test inside `server.rs`'s existing test module:

```rust
#[test]
fn production_codex_retry_wait_observes_existing_cancellation() {
    let cancellation = crate::codex_transport::CodexCancellation::default();
    cancellation.cancel();
    let mut runtime = ProductionCodexAttemptRuntime;
    let started = std::time::Instant::now();
    assert!(!runtime.wait(60_000, &cancellation));
    assert!(started.elapsed() < Duration::from_millis(100));
}
```

- [ ] **Step 5: Split the production wrapper from the injectable core**

Keep the current `handle_codex_messages_with_policy` signature and replace its body with:

```rust
let mut runtime = ProductionCodexAttemptRuntime;
handle_codex_messages_with_policy_and_runtime(
    stream,
    raw,
    is_stream,
    secrets,
    transport,
    policy,
    &mut auth_rejected,
    &mut runtime,
);
```

Move the former body into:

```rust
fn handle_codex_messages_with_policy_and_runtime(
    stream: &mut TcpStream,
    raw: &Value,
    is_stream: bool,
    secrets: codex_auth::InferenceSecrets,
    transport: &codex_transport::CodexTransport,
    policy: CodexRequestPolicy<'_>,
    mut auth_rejected: impl FnMut(u16, u64),
    runtime: &mut dyn CodexAttemptRuntime,
) {
```

The wrapper remains the only production call path; the acceptance module, which is a test-only child module, calls the injectable core.

- [ ] **Step 6: Replace the single POST with the bounded loop**

Keep `translated` as a `Value`, remove the one-time `body` binding, and replace the current initial POST/error match with:

```rust
let upstream = loop {
    controller.begin_post().expect("controller authorized Codex POST");
    let body = match serde_json::to_vec(&translated) {
        Ok(body) => body,
        Err(_) => {
            invalid_request_json(stream, "Codex request encoding failed");
            return;
        }
    };
    match transport.open_responses(&secrets, body, policy.use_responses_lite, cancellation.clone()) {
        Ok(upstream) => {
            controller.mark_response_started().expect("opened Codex response follows in-flight POST");
            break upstream;
        }
        Err(error) => {
            if let Some(status @ (401 | 403)) = error.upstream_status {
                auth_rejected(status, generation);
            }
            match controller.observe(error.observation()).expect("in-flight Codex observation is authorized") {
                crate::provider_failure::AttemptDirective::RetryAfter(delay_ms) => {
                    if !runtime.wait(delay_ms, &cancellation) {
                        controller.observe(crate::provider_failure::FailureObservation::Cancelled).expect("cancellation is terminal");
                        return;
                    }
                }
                crate::provider_failure::AttemptDirective::Fail(failure) => {
                    runtime.emit(controller.failed_diagnostic(&failure));
                    write_provider_failure(stream, &failure);
                    return;
                }
                crate::provider_failure::AttemptDirective::Cancel => return,
                crate::provider_failure::AttemptDirective::RepairOnce(_) => {
                    api_error_json(stream, 500, "Codex repair runtime is not installed");
                    return;
                }
            }
        }
    }
};
```

This deletes the temporary retry branch from Task 5. Do not mutate `translated` in this task, so every retry serializes equal JSON.

- [ ] **Step 7: Run retry, transport, and controller suites to verify GREEN**

```bash
cargo test --manifest-path desktop/gateway/Cargo.toml --features acceptance-build 'server::codex_acceptance::contract_rate_limit_' -- --test-threads=1
cargo test --manifest-path desktop/gateway/Cargo.toml --features acceptance-build 'server::codex_acceptance::contract_network_' -- --test-threads=1
cargo test --manifest-path desktop/gateway/Cargo.toml --features acceptance-build 'server::codex_acceptance::contract_408_exhaustion_returns_504' -- --test-threads=1
cargo test --manifest-path desktop/gateway/Cargo.toml --features acceptance-build 'server::codex_acceptance::contract_409_exhaustion_returns_502' -- --test-threads=1
cargo test --manifest-path desktop/gateway/Cargo.toml --features acceptance-build 'server::codex_acceptance::contract_500_exhaustion_returns_502' -- --test-threads=1
cargo test --manifest-path desktop/gateway/Cargo.toml --features acceptance-build 'server::codex_acceptance::contract_503_exhaustion_returns_502' -- --test-threads=1
cargo test --manifest-path desktop/gateway/Cargo.toml --features acceptance-build 'server::codex_acceptance::contract_failure_schema_and_caller_redaction' -- --test-threads=1
cargo test --manifest-path desktop/gateway/Cargo.toml --features acceptance-build 'server::codex_acceptance::contract_malformed_request_id_is_omitted' -- --test-threads=1
cargo test --manifest-path desktop/gateway/Cargo.toml --features acceptance-build 'server::codex_acceptance::contract_oversized_request_id_is_omitted' -- --test-threads=1
cargo test --manifest-path desktop/gateway/Cargo.toml codex_transport::tests -- --test-threads=1
cargo test --manifest-path desktop/gateway/Cargo.toml provider_failure::tests -- --test-threads=1
```

Expected: exact POST and delay vectors pass; fallback cases do not sleep under the recording runtime; transport still makes one POST per call.

- [ ] **Step 8: Format and commit**

```bash
cargo fmt --manifest-path desktop/gateway/Cargo.toml
git add desktop/gateway/src/provider_failure.rs desktop/gateway/src/server.rs desktop/gateway/src/server/codex_acceptance.rs
git commit -m "feat: retry Codex failures through bounded runtime"
```

### Task 7: Add the one closed Responses Lite Safe Repair

**Files:**
- Modify: `desktop/gateway/src/provider_failure.rs`
- Modify: `desktop/gateway/src/provider_failure/tests.rs`
- Modify: `desktop/gateway/src/codex_transport.rs`
- Modify: `desktop/gateway/src/server.rs:1070-1195`
- Test: `desktop/gateway/src/server/codex_acceptance.rs:1061-1236`

**Interfaces:**
- Consumes: translated request `Value`, closed `ErrorCode`/`ErrorParam` facts, `AttemptDirective::RepairOnce`, and controller mutual-exclusion rules.
- Produces: `caller_allows_automatic_tool_choice`, exact repair eligibility, and a second body differing only by removal of top-level string `tool_choice: "auto"`.

- [ ] **Step 1: Strengthen repair outcome observations and run accepted repair tests RED**

Inside `assert_safe_repair`, after its existing body-equality assertions, add:

```rust
assert!(result.delays_ms.is_empty());
```

Inside `contract_used_repair_budget_stops_after_second_typed_rejection`, add:

```rust
assert!(result.delays_ms.is_empty());
```

Run:

```bash
cargo test --manifest-path desktop/gateway/Cargo.toml --features acceptance-build 'server::codex_acceptance::contract_lite_non_equivalent_tool_choices_never_post' -- --test-threads=1
cargo test --manifest-path desktop/gateway/Cargo.toml --features acceptance-build 'server::codex_acceptance::contract_safe_repair_' -- --test-threads=1
cargo test --manifest-path desktop/gateway/Cargo.toml --features acceptance-build 'server::codex_acceptance::contract_unknown_typed_code_does_not_authorize_repair' -- --test-threads=1
cargo test --manifest-path desktop/gateway/Cargo.toml --features acceptance-build 'server::codex_acceptance::contract_allowlisted_code_with_wrong_param_does_not_authorize_repair' -- --test-threads=1
cargo test --manifest-path desktop/gateway/Cargo.toml --features acceptance-build 'server::codex_acceptance::contract_route_disabled_does_not_authorize_repair' -- --test-threads=1
cargo test --manifest-path desktop/gateway/Cargo.toml --features acceptance-build 'server::codex_acceptance::contract_used_repair_budget_stops_after_second_typed_rejection' -- --test-threads=1
cargo test --manifest-path desktop/gateway/Cargo.toml --features acceptance-build 'server::codex_acceptance::contract_unproven_error_text_does_not_authorize_repair' -- --test-threads=1
```

Expected: non-equivalent caller choices already pass without a POST; both safe-repair cases fail because only one POST occurs; negative typed cases fail their structured capability envelope until normalization is complete.

- [ ] **Step 2: Verify absent and present-but-unrecognized facts remain distinct**

Task 2 defines `Absent` separately from `Other`, Task 4's reducer emits `Absent` for missing fields and `Other` for present unknown strings, and `ProviderFailure::from_observation` classifies HTTP 400 as capability when either the allowlisted code or parameter signal is present. Confirm the exact source with:

```bash
rg -n "enum Error(Code|Param)|Some\(_\) => Error(Code|Param)::Other|None => Error(Code|Param)::Absent|FailureClass::Capability" desktop/gateway/src/provider_failure.rs desktop/gateway/src/codex_transport.rs
```

Expected: wrong-code/right-parameter and right-code/wrong-parameter are terminal capability failures, a generic `invalid_request` code remains `InvalidRequest`, and only the exact pair reaches the controller's `RepairOnce` arm.

- [ ] **Step 3: Add a caller-eligibility unit test**

Inside `server.rs`'s existing tests, add:

```rust
#[test]
fn safe_repair_caller_eligibility_is_closed_to_absent_or_auto() {
    assert!(caller_allows_automatic_tool_choice(&serde_json::json!({})));
    assert!(caller_allows_automatic_tool_choice(&serde_json::json!({"tool_choice":{"type":"auto"}})));
    for request in [
        serde_json::json!({"tool_choice":{"type":"none"}}),
        serde_json::json!({"tool_choice":{"type":"any"}}),
        serde_json::json!({"tool_choice":{"type":"tool","name":"read"}}),
        serde_json::json!({"tool_choice":"auto"}),
    ] {
        assert!(!caller_allows_automatic_tool_choice(&request));
    }
}
```

Run:

```bash
cargo test --manifest-path desktop/gateway/Cargo.toml server::tests::safe_repair_caller_eligibility_is_closed_to_absent_or_auto -- --exact --test-threads=1
```

Expected: compilation fails because the helper is absent.

- [ ] **Step 4: Implement exact caller and translated-body eligibility**

Add beside `CodexRequestPolicy`:

```rust
fn caller_allows_automatic_tool_choice(raw: &Value) -> bool {
    match raw.get("tool_choice") {
        None => true,
        Some(Value::Object(choice)) => choice.get("type").and_then(Value::as_str) == Some("auto"),
        Some(_) => false,
    }
}
```

After translation, compute:

```rust
let repair_enabled = policy.use_responses_lite
    && caller_allows_automatic_tool_choice(raw)
    && translated.get("tool_choice").and_then(Value::as_str) == Some("auto");
```

Pass `repair_enabled` rather than `false` to `AttemptController::new`.

- [ ] **Step 5: Execute the single authorized mutation**

Replace the temporary repair branch in the attempt loop with:

```rust
crate::provider_failure::AttemptDirective::RepairOnce(
    crate::provider_failure::RepairKind::OmitAutomaticToolChoice,
) => {
    let removed = translated
        .as_object_mut()
        .and_then(|object| object.remove("tool_choice"));
    assert_eq!(
        removed,
        Some(Value::String("auto".to_owned())),
        "controller may authorize only the exact top-level automatic tool choice repair"
    );
}
```

No other key may be removed or rewritten. The next loop iteration serializes the mutated `Value`; controller state caps that sequence at two POSTs and forbids every retry.

- [ ] **Step 6: Run pure, extraction, and accepted repair suites GREEN**

```bash
cargo test --manifest-path desktop/gateway/Cargo.toml provider_failure::tests -- --test-threads=1
cargo test --manifest-path desktop/gateway/Cargo.toml codex_transport::tests -- --test-threads=1
cargo test --manifest-path desktop/gateway/Cargo.toml --features acceptance-build 'server::codex_acceptance::contract_lite_non_equivalent_tool_choices_never_post' -- --test-threads=1
cargo test --manifest-path desktop/gateway/Cargo.toml --features acceptance-build 'server::codex_acceptance::contract_safe_repair_' -- --test-threads=1
cargo test --manifest-path desktop/gateway/Cargo.toml --features acceptance-build 'server::codex_acceptance::contract_unknown_typed_code_does_not_authorize_repair' -- --test-threads=1
cargo test --manifest-path desktop/gateway/Cargo.toml --features acceptance-build 'server::codex_acceptance::contract_allowlisted_code_with_wrong_param_does_not_authorize_repair' -- --test-threads=1
cargo test --manifest-path desktop/gateway/Cargo.toml --features acceptance-build 'server::codex_acceptance::contract_route_disabled_does_not_authorize_repair' -- --test-threads=1
cargo test --manifest-path desktop/gateway/Cargo.toml --features acceptance-build 'server::codex_acceptance::contract_used_repair_budget_stops_after_second_typed_rejection' -- --test-threads=1
cargo test --manifest-path desktop/gateway/Cargo.toml --features acceptance-build 'server::codex_acceptance::contract_unproven_error_text_does_not_authorize_repair' -- --test-threads=1
```

Expected: both safe cases POST twice with bodies equal after removing only `tool_choice`; negative cases POST once; the double rejection leaves the third scripted success unused; every repair delay vector is empty.

- [ ] **Step 7: Format and commit**

```bash
cargo fmt --manifest-path desktop/gateway/Cargo.toml
git add desktop/gateway/src/provider_failure.rs desktop/gateway/src/provider_failure/tests.rs desktop/gateway/src/codex_transport.rs desktop/gateway/src/server.rs desktop/gateway/src/server/codex_acceptance.rs
git commit -m "feat: repair one safe Codex Lite capability"
```

### Task 8: Emit exactly one final diagnostic after reducer completion

**Files:**
- Modify: `desktop/gateway/src/provider_failure.rs`
- Modify: `desktop/gateway/src/provider_failure/tests.rs`
- Modify: `desktop/gateway/src/server.rs:850-1185`
- Modify: `desktop/gateway/src/server/codex_acceptance.rs:788-845,885-929,970-1012,1080-1150`

**Interfaces:**
- Consumes: controller snapshots, runtime diagnostic sink, existing stream/non-stream reducers, and the response-started replay barrier.
- Produces: `AttemptController::{completed_diagnostic,cancelled_diagnostic}`, `CodexStreamOutcome`, and one final diagnostic for every started attempt sequence.

- [ ] **Step 1: Add exact final-diagnostic assertions to acceptance cases**

Add this helper to `server/codex_acceptance.rs`:

```rust
fn only_diagnostic(result: &AcceptanceResult) -> &Value {
    assert_eq!(result.diagnostics.len(), 1, "one final attempt diagnostic required");
    &result.diagnostics[0]
}
```

Add these assertions to the named cases:

```rust
// harness_runner_uses_real_handler_and_transport
assert_eq!(only_diagnostic(&result)["outcome"], "completed");
assert_eq!(only_diagnostic(&result)["posts"], 1);

// contract_rate_limit_retries_twice_then_succeeds
assert_eq!(only_diagnostic(&result)["outcome"], "completed");
assert_eq!(only_diagnostic(&result)["posts"], 3);
assert_eq!(only_diagnostic(&result)["delays_ms"], serde_json::json!([0, 0]));

// assert_safe_repair
assert_eq!(only_diagnostic(&result)["outcome"], "completed");
assert_eq!(only_diagnostic(&result)["posts"], 2);
assert_eq!(only_diagnostic(&result)["repairs"], 1);
assert_eq!(only_diagnostic(&result)["delays_ms"], serde_json::json!([]));

// contract_partial_stream_failure_never_replays
assert_eq!(only_diagnostic(&result)["outcome"], "failed");
assert_eq!(only_diagnostic(&result)["posts"], 1);
assert_eq!(only_diagnostic(&result)["failure_class"], "protocol");
assert_eq!(only_diagnostic(&result)["retryable"], false);

// contract_failure_schema_and_caller_redaction
assert_eq!(only_diagnostic(&result)["outcome"], "failed");
assert_eq!(only_diagnostic(&result)["posts"], 3);
assert_eq!(only_diagnostic(&result)["delays_ms"], serde_json::json!([500, 1000]));
let rendered_diagnostics = serde_json::to_string(&result.diagnostics).unwrap();
assert_forbidden_sentinels_absent(&rendered_diagnostics);
assert!(!rendered_diagnostics.contains("req-safe-500"));
```

Add a non-contract integration test so the accepted 8/27 counts remain unchanged:

```rust
#[derive(Default)]
struct CancellingAttemptRuntime {
    diagnostics: Vec<Value>,
}

impl CodexAttemptRuntime for CancellingAttemptRuntime {
    fn wait(&mut self, _delay_ms: u64, cancellation: &CodexCancellation) -> bool {
        cancellation.cancel();
        false
    }

    fn emit(&mut self, diagnostic: AttemptDiagnostic) {
        self.diagnostics.push(serde_json::to_value(diagnostic).unwrap());
    }
}

#[test]
fn attempt_wait_cancellation_emits_one_final_diagnostic_without_replay() {
    let upstream = ScriptedCodexUpstream::start(vec![transient_step(500, "Internal Server Error")]);
    let transport = CodexTransport::for_test(upstream.endpoint("/responses")).unwrap();
    let mut runtime = CancellingAttemptRuntime::default();
    let downstream = capture_downstream(|stream| {
        handle_codex_messages_with_policy_and_runtime(
            stream,
            &anthropic_request(false),
            false,
            InferenceSecrets::for_test(ACCESS_SENTINEL, ACCOUNT_SENTINEL),
            &transport,
            CodexRequestPolicy::default(),
            |_status, _generation| {},
            &mut runtime,
        );
    });
    let script = upstream.finish();
    assert!(downstream.is_empty());
    assert_eq!(script.requests.len(), 1);
    assert_eq!(script.remaining_steps, 0);
    assert_eq!(runtime.diagnostics.len(), 1);
    assert_eq!(runtime.diagnostics[0]["outcome"], "cancelled");
    assert_eq!(runtime.diagnostics[0]["posts"], 1);
    assert_eq!(runtime.diagnostics[0]["delays_ms"], serde_json::json!([0]));
}
```

- [ ] **Step 2: Add pure completed/cancelled schema tests**

Append to `provider_failure/tests.rs`:

```rust
#[test]
fn completed_and_cancelled_diagnostics_omit_failure_fields() {
    let mut completed = AttemptController::new(context(RouteMode::Responses), false);
    completed.begin_post().unwrap();
    completed.mark_response_started().unwrap();
    assert_eq!(
        serde_json::to_value(completed.completed_diagnostic()).unwrap(),
        serde_json::json!({
            "outcome":"completed", "provider":"codex", "route":"responses",
            "correlation_id":"corr-0001", "posts":1, "repairs":0, "delays_ms":[]
        })
    );

    let mut cancelled = AttemptController::new(context(RouteMode::Responses), false);
    cancelled.begin_post().unwrap();
    cancelled.observe(FailureObservation::Cancelled).unwrap();
    assert_eq!(
        serde_json::to_value(cancelled.cancelled_diagnostic()).unwrap(),
        serde_json::json!({
            "outcome":"cancelled", "provider":"codex", "route":"responses",
            "correlation_id":"corr-0001", "posts":1, "repairs":0, "delays_ms":[]
        })
    );
}
```

Run:

```bash
cargo test --manifest-path desktop/gateway/Cargo.toml provider_failure::tests::completed_and_cancelled_diagnostics_omit_failure_fields -- --test-threads=1
cargo test --manifest-path desktop/gateway/Cargo.toml --features acceptance-build 'server::codex_acceptance::harness_runner_uses_real_handler_and_transport' -- --test-threads=1
cargo test --manifest-path desktop/gateway/Cargo.toml --features acceptance-build 'server::codex_acceptance::contract_partial_stream_failure_never_replays' -- --test-threads=1
cargo test --manifest-path desktop/gateway/Cargo.toml --features acceptance-build 'server::codex_acceptance::attempt_wait_cancellation_emits_one_final_diagnostic_without_replay' -- --test-threads=1
```

Expected: RED because success/cancellation constructors and reducer outcomes do not exist and success currently emits no final diagnostic.

- [ ] **Step 3: Add success and cancellation diagnostic constructors**

Add to `AttemptDiagnostic`:

```rust
fn without_failure(outcome: AttemptOutcome, context: &RouteContext, snapshot: AttemptSnapshot) -> Self {
    Self {
        outcome,
        provider: context.provider,
        route: context.route,
        correlation_id: context.correlation_id.clone(),
        posts: snapshot.posts,
        repairs: snapshot.repairs,
        delays_ms: snapshot.delays_ms,
        mapped_status: None,
        upstream_status: None,
        failure_class: None,
        retryable: None,
    }
}
```

Add to `AttemptController`:

```rust
pub(crate) fn completed_diagnostic(&mut self) -> AttemptDiagnostic {
    self.phase = AttemptPhase::Terminal;
    AttemptDiagnostic::without_failure(AttemptOutcome::Completed, &self.context, self.snapshot())
}

pub(crate) fn cancelled_diagnostic(&mut self) -> AttemptDiagnostic {
    self.phase = AttemptPhase::Terminal;
    AttemptDiagnostic::without_failure(AttemptOutcome::Cancelled, &self.context, self.snapshot())
}
```

- [ ] **Step 4: Make streaming return a closed reducer outcome**

Add:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CodexStreamOutcome { Completed, Failed, Cancelled }
```

Change `forward_codex_stream` to return `CodexStreamOutcome`. Use this exact outcome mapping:

```rust
if write!(
    stream,
    "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ntransfer-encoding: chunked\r\nconnection: close\r\n\r\n"
)
.and_then(|_| stream.flush())
.is_err()
{
    return CodexStreamOutcome::Cancelled;
}
match pump_codex_stream(&mut upstream, reducer, |chunk| write_chunk(stream, chunk)) {
    Ok(()) => {
        let _ = stream.write_all(b"0\r\n\r\n");
        let _ = stream.flush();
        CodexStreamOutcome::Completed
    }
    Err(CodexPumpError::DownstreamWrite | CodexPumpError::Cancelled) => CodexStreamOutcome::Cancelled,
    Err(CodexPumpError::UpstreamRead | CodexPumpError::Protocol) => {
        finish_codex_stream_error(stream);
        CodexStreamOutcome::Failed
    }
}
```

- [ ] **Step 5: Finalize diagnostics only after stream/non-stream reduction**

Replace the handler's final stream/non-stream block with:

```rust
if is_stream {
    match forward_codex_stream(stream, upstream, &mut reducer) {
        CodexStreamOutcome::Completed => runtime.emit(controller.completed_diagnostic()),
        CodexStreamOutcome::Cancelled => runtime.emit(controller.cancelled_diagnostic()),
        CodexStreamOutcome::Failed => {
            let directive = controller
                .observe(crate::provider_failure::FailureObservation::Protocol(
                    crate::provider_failure::ProtocolKind::InvalidResponse,
                ))
                .expect("response-started protocol failure is terminal");
            let crate::provider_failure::AttemptDirective::Fail(failure) = directive else {
                panic!("response-started failure cannot replay");
            };
            runtime.emit(controller.failed_diagnostic(&failure));
        }
    }
} else {
    match collect_codex_nonstream(upstream, stream, &mut reducer) {
        Err(CodexNonstreamError::DownstreamClosed) => runtime.emit(controller.cancelled_diagnostic()),
        Err(CodexNonstreamError::UpstreamRead | CodexNonstreamError::Protocol) => {
            let directive = controller
                .observe(crate::provider_failure::FailureObservation::Protocol(
                    crate::provider_failure::ProtocolKind::InvalidResponse,
                ))
                .expect("response-started protocol failure is terminal");
            let crate::provider_failure::AttemptDirective::Fail(failure) = directive else {
                panic!("response-started failure cannot replay");
            };
            runtime.emit(controller.failed_diagnostic(&failure));
            write_provider_failure(stream, &failure);
        }
        Ok(()) => match reducer.nonstream_response() {
            Ok(response) => {
                write_json(stream, 200, "OK", response);
                runtime.emit(controller.completed_diagnostic());
            }
            Err(_) => {
                let directive = controller
                    .observe(crate::provider_failure::FailureObservation::Protocol(
                        crate::provider_failure::ProtocolKind::InvalidResponse,
                    ))
                    .expect("response-started reducer failure is terminal");
                let crate::provider_failure::AttemptDirective::Fail(failure) = directive else {
                    panic!("response-started failure cannot replay");
                };
                runtime.emit(controller.failed_diagnostic(&failure));
                write_provider_failure(stream, &failure);
            }
        },
    }
}
```

In the retry-wait cancellation branch from Task 6, emit exactly once before returning:

```rust
controller.observe(crate::provider_failure::FailureObservation::Cancelled).expect("cancellation is terminal");
runtime.emit(controller.cancelled_diagnostic());
return;
```

In the direct transport `AttemptDirective::Cancel` branch, emit `controller.cancelled_diagnostic()` before returning. Failure branches already emit once. Do not emit when an upstream merely opens.

- [ ] **Step 6: Run diagnostic, replay-barrier, and full acceptance tests GREEN**

```bash
cargo test --manifest-path desktop/gateway/Cargo.toml provider_failure::tests -- --test-threads=1
cargo test --manifest-path desktop/gateway/Cargo.toml --features acceptance-build 'server::codex_acceptance::contract_partial_stream_failure_never_replays' -- --test-threads=1
cargo test --manifest-path desktop/gateway/Cargo.toml --features acceptance-build 'server::codex_acceptance::contract_failure_schema_and_caller_redaction' -- --test-threads=1
cargo test --manifest-path desktop/gateway/Cargo.toml --features acceptance-build 'server::codex_acceptance::contract_rate_limit_retries_twice_then_succeeds' -- --test-threads=1
cargo test --manifest-path desktop/gateway/Cargo.toml --features acceptance-build 'server::codex_acceptance::contract_safe_repair_' -- --test-threads=1
cargo test --manifest-path desktop/gateway/Cargo.toml --features acceptance-build 'server::codex_acceptance::harness_runner_uses_real_handler_and_transport' -- --test-threads=1
cargo test --manifest-path desktop/gateway/Cargo.toml --features acceptance-build 'server::codex_acceptance::' -- --test-threads=1
```

Expected: partial stream performs one POST, preserves its visible prefix once, writes one terminal sanitized SSE error, and emits one failed diagnostic; all 8 harness and all 27 contract tests pass.

- [ ] **Step 7: Prove diagnostics cannot contain forbidden sentinels and commit**

```bash
rg -n "request_id|detail|message|body|header|cookie|token|account|endpoint" desktop/gateway/src/provider_failure.rs
cargo fmt --manifest-path desktop/gateway/Cargo.toml
git add desktop/gateway/src/provider_failure.rs desktop/gateway/src/provider_failure/tests.rs desktop/gateway/src/server.rs desktop/gateway/src/server/codex_acceptance.rs
git commit -m "feat: finalize Codex attempt diagnostics"
```

Expected: any matches are confined to caller-envelope construction or closed static labels; `AttemptDiagnostic` has no request-ID or arbitrary-string field.

### Task 9: Run every gate, record evidence, review, and publish the feature branch

**Files:**
- Create: `docs/evidence/investigations/2026-07-29-codex-provider-failure-implementation.md`
- Modify: `docs/evidence/investigations/README.md`
- Modify if review finds a defect: the exact production/test file implicated by the finding.

**Interfaces:**
- Consumes: the complete feature implementation and accepted deterministic harness.
- Produces: immutable test evidence, a clean full-gate run, an independent code/security review with no Critical or Important findings, and published `feature/codex-provider-failure`.

- [ ] **Step 1: Run formatting and whitespace gates**

```bash
cargo fmt --check --manifest-path desktop/gateway/Cargo.toml
git diff --check linux-headless-oauth...HEAD
```

Expected: both commands exit 0 with no output.

- [ ] **Step 2: Run pure, transport, accepted harness, accepted contract, and focused Codex gates**

```bash
cargo test --offline --manifest-path desktop/gateway/Cargo.toml provider_failure::tests -- --test-threads=1
cargo test --offline --manifest-path desktop/gateway/Cargo.toml codex_transport::tests -- --test-threads=1
cargo test --offline --manifest-path desktop/gateway/Cargo.toml --features acceptance-build 'server::codex_acceptance::harness_' -- --nocapture --test-threads=1
cargo test --offline --manifest-path desktop/gateway/Cargo.toml --features acceptance-build 'server::codex_acceptance::contract_' -- --nocapture --test-threads=1
cargo test --offline --manifest-path desktop/gateway/Cargo.toml codex_ -- --test-threads=1
```

Expected: all pure tests pass, all transport tests pass, harness is 8/8, contract is 27/27, and the existing focused Codex suite has no regression. Record the exact filtered counts printed by Cargo rather than assuming counts for newly added unit tests.

- [ ] **Step 3: Run the complete gateway and lint gates**

```bash
cargo test --offline --manifest-path desktop/gateway/Cargo.toml --all-features -- --test-threads=1
cargo clippy --offline --manifest-path desktop/gateway/Cargo.toml --all-targets --all-features -- -D warnings
```

Expected: the complete gateway suite passes and Clippy exits 0 with no warnings.

- [ ] **Step 4: Commit the verified implementation before writing immutable evidence**

```bash
git status --short
git add desktop/gateway/src/lib.rs desktop/gateway/src/provider_failure.rs desktop/gateway/src/provider_failure/tests.rs desktop/gateway/src/codex_transport.rs desktop/gateway/src/server.rs desktop/gateway/src/server/codex_acceptance.rs
git commit -m "feat: complete Codex provider failure contract"
git rev-parse HEAD
rustc --version
cargo --version
uname -a
```

Expected: if earlier task commits already contain every implementation file, `git status --short` is clean and no extra implementation commit is needed. In either case, capture the literal tested HEAD and toolchain/environment output for the evidence file.

- [ ] **Step 5: Write the evidence report with the observed literal values**

Create `docs/evidence/investigations/2026-07-29-codex-provider-failure-implementation.md` using `apply_patch`. The report must contain these exact headings and factual statements, with the literal Step 2-4 outputs copied into the revision/environment/result fields:

```markdown
# Codex Provider Failure Contract Implementation Evidence

Date: 2026-07-29

## Scope

Ticket 05 implements the Provider Failure Contract through the real Codex handler and single-POST transport. This verification used only deterministic loopback fixtures: no live credential, installed profile, proxy mutation, external endpoint, or running service was used.

## Revision and environment

The tested revision is the literal `git rev-parse HEAD` output captured immediately after the final implementation commit. The Rust, Cargo, kernel, and architecture values below are the literal outputs captured in the same clean worktree.

## Verification results

The report lists every command from implementation-plan Task 9 verbatim, followed by its literal exit status and Cargo test count. The accepted harness passes 8/8 and the accepted Provider Failure Contract passes 27/27.

## Behavioral evidence

- Retry-only sequences make at most three POSTs.
- Missing `Retry-After` consumes 500 ms then 1,000 ms; valid delta seconds override a transition and are capped at 60 seconds.
- The recording runtime performs no sleep and captures exact delay vectors.
- Safe Repair is restricted to Responses Lite, absent/automatic caller choice, and exact `unsupported_value` plus `tool_choice`; it removes only top-level translated `tool_choice: "auto"` and makes at most two POSTs.
- Retry and repair budgets never combine.
- Permanent 4xx, authentication, authorization, and quota failures do not replay.
- No replay occurs after an upstream response opens or downstream bytes begin.
- Every started attempt sequence emits one final `completed`, `failed`, or `cancelled` diagnostic.

## Redaction boundary

Caller envelopes preserve legacy Anthropic fields and add only closed structured metadata. Diagnostics contain only outcome, provider, route, correlation ID, POST/repair/delay counts, and failure-only mapped status, optional upstream status, class, and retryability. Request IDs are positive-allowlisted for caller output but excluded from diagnostics. Raw bodies, prompts, credentials, account identifiers, cookies, headers, and private URLs cannot enter the controller or diagnostic types.

## Explicit limitations

This evidence does not verify an installed runtime or live Codex subscription; Ticket 06 owns that work. It does not add the shared API-key adapter; Ticket 07 owns that work. No macOS runtime was available, so macOS source compatibility is covered by the shared Rust build/test surface rather than a real macOS run.
```

Replace the two explanatory sentences in “Revision and environment” and “Verification results” with the actual revision, version strings, command lines, exit status, and counts before committing; do not leave symbolic wording in the final report.

Add one dated bullet linking the report to `docs/evidence/investigations/README.md`.

- [ ] **Step 6: Commit the evidence and rerun diff checks**

```bash
git add docs/evidence/investigations/2026-07-29-codex-provider-failure-implementation.md docs/evidence/investigations/README.md
git commit -m "docs: record Codex provider failure evidence"
git diff --check linux-headless-oauth...HEAD
git status --short --branch
```

Expected: evidence is committed, diff check is clean, and the feature worktree has no uncommitted files.

- [ ] **Step 7: Request an independent code and security review**

Use the `requesting-code-review` skill. Give the reviewer the approved design, this implementation plan, the Ticket 05 diff base `linux-headless-oauth`, and these review priorities:

```text
1. Any path exceeding three retry POSTs or two repair POSTs.
2. Any retry after repair, repair after retry, or replay after response start/cancellation.
3. Any body/header/credential/account/private-URL leakage into caller JSON, Debug/Display, or diagnostics.
4. Any unbounded failed-body read or request-ID acceptance outside [A-Za-z0-9._:-]{1,256}.
5. Any mismatch in 401/403/permanent 4xx/quota/rate/transient/network/protocol status and retryability.
6. Any attempt sequence emitting zero or multiple final diagnostics.
7. Any weakening of the accepted harness assertions.
```

Expected: the reviewer reports no Critical or Important findings. If such a finding exists, write a focused failing regression test, confirm RED, implement the minimal fix, rerun Steps 1-3, update the evidence with the new literal revision/results, and request re-review before continuing.

- [ ] **Step 8: Publish the reviewed feature branch**

```bash
git push -u fork feature/codex-provider-failure
git status --short --branch
```

Expected: `fork/feature/codex-provider-failure` points to the reviewed evidence commit and the worktree is clean.

### Task 10: Resolve only Wayfinder Ticket 05 on the main branch

**Files:**
- Modify: `.scratch/provider-failure-contract/issues/05-implement-codex-first-slice.md`
- Modify: `.scratch/provider-failure-contract/map.md`

**Interfaces:**
- Consumes: published feature branch and its evidence report.
- Produces: a truthful Ticket 05 resolution on `linux-headless-oauth`; Ticket 06 and Ticket 07 remain open and unclaimed.

- [ ] **Step 1: Return to the main worktree and verify the published feature tip**

```bash
git status --short --branch
git rev-parse feature/codex-provider-failure
git rev-parse fork/feature/codex-provider-failure
```

Expected: the two feature revisions are identical; the main worktree contains only the already approved plan/design documentation state, if any.

- [ ] **Step 2: Resolve Ticket 05 with concrete evidence**

Change `Status: claimed` to `Status: resolved` in `.scratch/provider-failure-contract/issues/05-implement-codex-first-slice.md`, then append:

```markdown
## Resolution

Implemented and published the Codex-first Provider Failure Contract on branch `feature/codex-provider-failure`. The real handler now uses the pure Attempt Controller, bounded single-POST transport fact projection, at most three retry POSTs, one mutually exclusive Responses Lite Safe Repair, backward-compatible structured caller failures, the response-started replay barrier, cancellation-aware scheduling, and exactly one final sanitized attempt diagnostic.

Deterministic verification passes the accepted harness 8/8 and accepted contract 27/27, plus the pure controller, transport, focused Codex, full gateway, formatting, and offline Clippy gates. The immutable tested revision, literal commands/counts, redaction boundary, and limitations are recorded in `docs/evidence/investigations/2026-07-29-codex-provider-failure-implementation.md` on the published feature branch.

No live credential, installed profile, proxy setting, external endpoint, or running service was used or changed. Ticket 06 remains responsible for installed-runtime and live subscription verification. Ticket 07 remains responsible for the shared API-key Provider Route adapter.
```

- [ ] **Step 3: Add the Ticket 05 decision to the Wayfinder map**

Append under `## Decisions so far`:

```markdown
- [Implement the Codex-first contract slice](issues/05-implement-codex-first-slice.md) — The reviewed feature branch implements the full deterministic Codex Provider Failure Contract and passes 8/8 harness plus 27/27 contract tests; installed/live verification and the generic adapter remain Tickets 06 and 07.
```

Do not edit or claim Ticket 06 or Ticket 07 in this session.

- [ ] **Step 4: Validate, commit, and publish the Wayfinder resolution**

```bash
git diff --check
git diff -- .scratch/provider-failure-contract/issues/05-implement-codex-first-slice.md .scratch/provider-failure-contract/map.md
git add .scratch/provider-failure-contract/issues/05-implement-codex-first-slice.md .scratch/provider-failure-contract/map.md
git commit -m "docs: resolve Codex contract implementation"
git push fork linux-headless-oauth
git status --short --branch
```

Expected: main is clean and synchronized with `fork/linux-headless-oauth`; Ticket 05 is resolved; Tickets 06 and 07 remain open/unclaimed; no installed CSSwitch or Claude Science process was restarted or changed.
