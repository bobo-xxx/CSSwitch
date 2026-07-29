# Provider Failure Contract Seam Design

Date: 2026-07-29

Status: approved in interactive design review; pending review of this written capture

## Problem

CSSwitch currently splits provider failure behavior across request translators, `codex_transport`, the generic `messages` transport, and response writers in `server`. Codex preserves only selected upstream statuses and maps most other rejections to a generic 502, while generic providers expose a different error shape and may include redacted upstream body text. Retry eligibility, capability failures, and caller-visible serialization therefore lack one authoritative contract.

The design must make provider failures deterministic and actionable without forcing Codex's asynchronous, cancellation-aware SSE transport and the generic blocking transport through one shallow I/O abstraction.

## Goals

- Give every Provider Route one Provider Failure Contract.
- Centralize failure classification, retry and repair budgets, response-started rules, caller metadata, and diagnostic redaction.
- Preserve provider-specific request normalization, authentication, transport, cancellation, and stream handling in adapters.
- Prove the seam first with Codex on Linux headless, while keeping the interface usable by generic providers.
- Preserve the existing Anthropic-compatible `type`, `error.type`, and `error.message` fields.

## Non-goals

- Provider or model fallback.
- Silent history deletion, model substitution, or semantic rewriting.
- A common HTTP client or common sync/async executor.
- Enabling a Safe Repair without adapter-specific evidence.
- Changing Claude Science's UI banner or orchestration.
- Selecting final production backoff durations; the acceptance-harness ticket supplies the validated retry policy values.

## Current seams

- `codex_protocol` performs Codex capability-aware request translation and returns `ProtocolError`.
- `codex_transport` owns OAuth-bearing asynchronous I/O, cancellation, SSE response validation, and `CodexTransportError`.
- `messages` owns blocking API-key transport and `UpstreamError` for non-Codex providers.
- `server` currently maps those unrelated errors into Anthropic-compatible responses.
- `provider_contracts` supplies validated static Provider Route configuration and timeout values; it does not own per-request state.

## Decision

Add a pure, in-process deep module at a new `provider_failure` seam. The module owns deterministic policy and attempt state. Provider adapters retain protocol-specific transformations and I/O, and submit only sanitized Failure Observations to the module.

This passes the deletion test: without the module, classification, retry budgets, repair limits, redaction, and envelope mapping would reappear in each adapter and in `server`. The module's interface remains small because it does not expose HTTP clients, credentials, request bodies, response bodies, or transport-specific stream types.

### Rejected alternatives

1. **Extend `provider_contracts` with request-state behavior.** Rejected because it mixes static configuration identity with mutable per-attempt state and response serialization.
2. **Route every provider through one shared HTTP executor.** Rejected because blocking generic I/O and asynchronous cancellation-aware Codex SSE would require a large common interface and weaken existing guarantees.
3. **Only enrich `UpstreamError` and `CodexTransportError`.** Rejected as a shallow change: retry, repair, and serialization decisions would remain duplicated across callers.

## Module interface

The production names may follow Rust conventions, but the interface must preserve this shape:

```rust
struct RouteContext {
    provider: ProviderId,
    route: RouteMode,
    correlation_id: CorrelationId,
    policy: RetryPolicy,
}

struct AttemptState {
    posts_started: u8,
    repairs_used: u8,
    response_started: bool,
}

enum FailureObservation {
    Capability { class: CapabilityClass, repair: Option<RepairKind> },
    Http { status: u16, rate_kind: Option<RateKind>, retry_after: Option<Duration>, request_id: Option<RequestId> },
    Network { stage: NetworkStage },
    Protocol { stage: ProtocolStage },
    Cancelled,
}

enum AttemptDirective {
    Fail(ProviderFailure),
    RetryAfter(Duration),
    RepairOnce(RepairKind),
    Cancel,
}

impl AttemptController {
    fn observe(&mut self, observation: FailureObservation) -> AttemptDirective;
}
```

`FailureObservation` accepts only allowlisted facts. It cannot contain request or response bodies, prompts, credentials, account identifiers, cookies, private URLs, or arbitrary upstream error strings.

Adapters execute an `AttemptDirective`; they do not decide it. They may duplicate transport mechanics such as sleeping or reopening an HTTP request, but not retry eligibility, limits, classification, or redaction policy.

## State invariants

- Pre-POST normalization does not increment `posts_started` and does not consume the Safe Repair budget.
- `posts_started`, `repairs_used`, and `response_started` are monotonic.
- A Safe Repair is available only through a closed `RepairKind` enum and only when the active Provider Route explicitly enables it.
- At most one Safe Repair and replay is allowed for one caller request.
- A retry is allowed only for an allowlisted transient class and while the validated retry budget remains.
- Once response bytes begin, neither retry nor repair is legal.
- Cancellation returns `Cancel` and never starts another attempt.
- The Codex-first slice enables no upstream-triggered Safe Repair until the acceptance harness supplies evidence. Equivalent pre-POST normalization remains allowed.

## Classification

| Observation | Downstream status/type | Directive |
|---|---|---|
| Non-equivalent capability, 400, 404, 422 | 400-series / `invalid_request_error` | Fail immediately |
| Context limit | 400 or existing 413 / `invalid_request_error` | Fail with recovery guidance |
| 401 | 401 / `authentication_error` | Fail current request; refresh may prepare a new caller request |
| 403 | 403 / `permission_error` | Fail with entitlement guidance |
| Rate-limited 429 | 429 / `rate_limit_error` | Bounded retry before response bytes |
| Quota 429 | 429 / `rate_limit_error` | Fail as non-retryable quota exhaustion |
| Network, 408, 409, 5xx | 502/504 / `api_error` | Bounded retry before response bytes |
| Protocol failure before bytes | 502 / `api_error` | Fail unless a separately proven Safe Repair applies |
| Failure after response bytes | Stream terminal error / `api_error` | Fail; never replay |
| Cancellation | No new response when downstream is gone | Cancel |

The controller accepts `RetryPolicy` as validated input. Numeric attempt counts, backoff values, and delay caps are not embedded in the seam and are selected by the acceptance-harness decision.

## Capability normalization and Safe Repair

Provider adapters remain responsible for semantic request translation because they understand the target protocol. They report a typed capability Failure Observation when semantics cannot be preserved.

For Codex Responses Lite:

- An absent or automatic tool choice may normalize to the provider's automatic representation before the first POST.
- `none`, `required`, and forced-tool choices are not equivalent to automatic selection and fail before transport.
- Arbitrary upstream error text never selects a repair.
- An upstream-triggered repair requires an enabled `RepairKind`, an unused repair budget, no response bytes, and an adapter implementation with evidence that the transformation preserves semantics.

## Provider Failure and serialization

`ProviderFailure` owns the sanitized downstream status, Anthropic error type, stable message, failure class, retryability, optional upstream status, correlation ID, optional allowlisted request ID, optional normalized retry delay, and recovery guidance.

Provider failures extend, rather than replace, the existing envelope:

```json
{
  "type": "error",
  "error": {
    "type": "invalid_request_error",
    "message": "Codex request is incompatible with Responses Lite",
    "provider": "codex",
    "route": "responses_lite",
    "failure_class": "capability",
    "upstream_status": 400,
    "retryable": false,
    "correlation_id": "local-generated-id",
    "recovery": "Use automatic tool choice or select a compatible model"
  }
}
```

Optional fields are omitted when unknown. `server` receives a fully classified Provider Failure and writes it; it no longer invents provider-specific status mappings. Local routing and syntax errors may continue using existing non-provider helpers.

Diagnostics derive from the same Provider Failure but may add stage timing and attempt count. Caller-visible and diagnostic representations both exclude raw bodies and secret-bearing identifiers.

## Adapter integration

### Codex

- `codex_protocol` maps non-equivalent capabilities to typed observations.
- `codex_transport` extracts only status and allowlisted retry/request-ID headers before discarding the error body.
- The Codex handler keeps authentication invalidation and next-request refresh as adapter effects after a classified 401/403.
- Codex cancellation and SSE reducers remain unchanged behind their existing interfaces.

### Generic providers

- `messages` maps blocking transport results into the same Failure Observation vocabulary.
- Generic adapters stop returning caller-facing raw or redacted upstream body text.
- Streaming response validation remains local, but response-started state is reported to the controller so replay is impossible.

## Throwaway logic prototype

The prototype lives only on `prototype/provider-failure-contract-seam` under:

```text
desktop/gateway/examples/provider_failure_prototype/
├── main.rs
└── controller.rs
```

Run it with:

```bash
cargo run --offline --manifest-path desktop/gateway/Cargo.toml --example provider_failure_prototype
```

The terminal exposes the full in-memory state and actions for capability rejection, 401, 403, rate-limit or quota 429, network/408/409/5xx failures, a known repair, response-started, cancellation, and reset. It shows the resulting directive, budgets, delay, and final envelope after every action. It performs no network, OAuth, filesystem persistence, or production gateway mutation.

The prototype has no automated tests. Its purpose is interactive validation of the state model and interface. The validated controller behavior later moves into production code; the terminal shell remains only on the throwaway branch as primary evidence.

## Production verification surface

- Table-driven tests through the `provider_failure` interface for every classification and state invariant.
- Serializer compatibility tests proving the three legacy fields remain present.
- Redaction tests proving forbidden data cannot be represented by Failure Observation or Provider Failure.
- Codex adapter tests proving exact POST counts, no current-request replay after 401, no replay after bytes, and allowlisted header extraction.
- Generic adapter conformance tests using the same observation/directive cases.
- The separate Codex acceptance-harness ticket owns fake-upstream scenarios and final retry timing values.

## Rollout

1. Validate the pure state model with the throwaway prototype.
2. Use the separate acceptance-harness prototype to validate external attempt counts and response envelopes.
3. Implement the Codex-first production slice through tests.
4. Verify Linux headless with fixtures and the already configured subscription, without inspecting credentials.
5. Decide the provider rollout boundary before changing generic Provider Routes.
