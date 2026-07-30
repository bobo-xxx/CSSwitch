# API-Key Provider Failure Contract Design

Date: 2026-07-30

Status: approved in conversation and written-spec review

## Purpose

Ticket 07 extends the proven Provider Failure Contract from the Codex Provider Route to every existing API-key inference route that uses the shared Gateway transport. The rollout preserves each protocol adapter's request and response semantics while centralizing bounded attempts, stable caller failures, cancellation finality, redaction, and attempt diagnostics.

The selected routes are:

- Anthropic Messages: DeepSeek native and relay contracts, including Kimi, custom Anthropic, OpenCode Go Anthropic, GLM, Xiaomi, SiliconFlow, MiniMax and OpenRouter templates.
- OpenAI Chat: Qwen native, custom OpenAI Chat, OpenCode Go OpenAI, Grok and Gemini templates.
- OpenAI Responses: custom OpenAI Responses templates.

The installed Codex Gateway on port `11535`, Claude Science on port `9002`, sandbox on port `9003`, and proxy on port `2999` remain untouched during design and implementation. Deployment is a separate user-approved operation after deterministic verification and review.

## Goals

- Give every selected API-key Provider Route an explicit, default-deny retry policy.
- Reuse the pure Attempt Controller for budgets, terminal state and exactly-once diagnostics.
- Translate each request once and reuse byte-identical translated bytes for retry-only attempts.
- Preserve existing Kimi, DeepSeek, Qwen, OpenAI Chat and OpenAI Responses transformations.
- Preserve Anthropic-compatible caller error shape while adding only optional closed metadata.
- Prevent replay after a successful upstream response opens, after downstream bytes begin, or after cancellation.
- Keep Safe Repair and capability-field removal Codex-only.
- Prove each protocol stage with deterministic real-handler loopback conformance before enabling it.

## Non-goals

- No automatic provider or model fallback.
- No silent history deletion, model substitution or semantic request rewriting.
- No Safe Repair outside Codex.
- No model-discovery, login, OAuth, profile, UI, Science-orchestration or proxy changes.
- No new provider families or transport formats.
- No live API-key credentials or live calls to Kimi, TaoToken, DeepSeek, Qwen, Gemini, Grok or custom endpoints.
- No claim of real macOS execution when no macOS runner is available.

## Existing Baseline

The branch starts from Ticket 06 evidence head `30ec9e2d546dea72f6de7fd843a373657d24fa0b`, containing the reviewed Provider Failure Contract implementation `9591a869f7268b5ff6baee00eb1365b36c54a618`. A fresh baseline run passed 351 Gateway library tests and 3 CLI integration tests.

The current shared `messages.rs` adapter performs one inference POST for non-stream and stream routes, but returns string-based `UpstreamError` details and has no inference retry loop. Route transformations and response conversions currently live in `server.rs`. The provider contract catalog already resolves the exact transport and provider contract for managed and standalone launches.

## Considered Approaches

### Selected: shared attempt runner plus explicit catalog policy

Keep protocol I/O as a single attempt, place retry and terminal lifecycle in one runner around the Attempt Controller, and load a validated policy from each provider contract. This gives the seam a second real transport adapter without duplicating controller policy across server branches.

### Rejected: independent loops in each server branch

Separate retry loops for relay, OpenAI Chat and OpenAI Responses would minimize initial refactoring, but would duplicate cancellation, redaction, delay, response-open and diagnostic rules. The branches would drift and could accidentally classify the same failure differently.

### Rejected: broad generic transport trait rewrite

A new trait hierarchy for all transports could unify more code, but would force unrelated Codex, model-discovery and route-conversion changes. Ticket 07 needs one bounded API-key adapter, not a Gateway-wide transport framework.

## Architecture

### Provider contract policy

`catalog/provider-contracts.v1.json` becomes the source of truth for an `inference_retry` object on every API-key contract. The runtime loader validates and exposes:

- `max_posts`: exactly `3` for the selected Ticket 07 routes.
- `fallback_delays_ms`: exactly `[500, 1000]`.
- `retry_after_cap_seconds`: exactly `60`.
- `retry_statuses`: an explicit closed list per protocol contract.
- `retry_network_kinds`: exactly pre-response `connect` and `timeout`.

Both lists use validated string tokens rather than free-form values. `retry_statuses` accepts only `"408"`, `"409"`, `"429"`, and `"5xx"`; `retry_network_kinds` accepts only `"connect"` and `"timeout"`. Each list rejects duplicates. The Anthropic Messages contracts store `["408", "429", "5xx"]`; OpenAI Chat and OpenAI Responses store `["408", "409", "429", "5xx"]`.

Codex retains `RetryPolicy::CODEX`; its contract and behavior do not inherit API-key policy. Missing, duplicated, out-of-range or incompatible API-key policy fields fail provider-contract loading rather than enabling a default retry set.

The policy matrices are:

| Transport | Retryable HTTP status before successful response-open |
|---|---|
| `anthropic_messages` | `408`, `429`, and `500..=599` |
| `openai_chat` | `408`, `409`, `429`, and `500..=599` |
| `openai_responses` | `408`, `409`, `429`, and `500..=599` |

An exact closed `insufficient_quota` error code overrides status `429` and is terminal. Unknown or absent `429` body codes follow the route's explicit rate-limit policy. `Retry-After` accepts only unsigned decimal delta seconds, is capped at 60 seconds, and otherwise uses the route's fallback delay.

### Provider Failure domain

The Provider Failure module gains closed provider and route variants for the selected adapters while preserving all Codex variants and tests:

- Providers: `deepseek`, `qwen`, `relay`, `openai_custom`, `openai_responses`, and `codex`.
- Routes: `anthropic_messages`, `openai_chat`, `openai_responses`, `responses`, and `responses_lite`.

`RouteContext` is created only from a validated runtime provider contract plus a generated allowlisted correlation ID. API-key contexts disable repair authorization structurally. The Attempt Controller continues to own post count, delay count, response-open state, cancellation state and single-use terminal diagnostics.

### Single-attempt API-key transport

`messages.rs` exposes single-attempt inference operations. Each operation:

1. Builds one request with the existing resolved authentication scheme and exact translated bytes.
2. Disables reqwest automatic retries and redirects.
3. Returns a successful response or a closed `FailureObservation` plus bounded caller metadata.
4. Never decides whether another POST is allowed.

HTTP failure projection reads at most 16 KiB for semantics, accepts only closed `error.code` and `error.type` values needed for quota/rate classification, and allowlists request IDs and `Retry-After`. Raw upstream bodies, messages, headers, API keys, URLs and arbitrary strings do not cross into the controller or diagnostic types.

The current string-based redacted detail remains available only where model-discovery code still owns its existing contract. Ticket 07 inference paths stop serializing upstream body text into caller failures.

### Shared API-key attempt runner

A focused API-key attempt module coordinates the single-attempt transport and Attempt Controller. It receives the already translated immutable body and a cancellation/runtime seam, then returns one of:

- a non-stream success response;
- an opened stream response;
- a typed terminal `ProviderFailure`;
- cancellation with no caller failure body.

Retry-only attempts reuse byte-identical translated bytes. The runner never invokes request translation, response conversion, Kimi filters, DSML rewriting or OpenAI response mapping.

The runner marks response-open as soon as a successful 2xx response is accepted. After that point, body-read failure, invalid content type, malformed JSON, response conversion failure, incomplete SSE, partial downstream delivery or disconnect cannot authorize replay.

### Server integration

`server.rs` keeps protocol-specific transformation and response-conversion ownership:

- Anthropic relay requests retain Kimi thinking/tool filters and target-model behavior.
- DeepSeek retains policy transforms and DSML modes.
- Qwen and custom OpenAI Chat retain their current translation, signing and SSE replay behavior.
- Custom OpenAI Responses retains its current request metadata and response mapping.

Each branch translates once, calls the shared runner, converts only a final success, and emits the shared Provider Failure envelope for a terminal failure. Checked downstream writers determine the final diagnostic: delivery failure produces exactly one `cancelled` outcome and cannot also produce `completed` or `failed`.

## Request Data Flow

### Non-stream

1. Validate the Anthropic request and resolve the selected static model route.
2. Apply the existing protocol-specific request transformation once.
3. Serialize the translated request once.
4. Create the route context and begin the shared attempt sequence.
5. Retry only a policy-authorized pre-success observation, using identical bytes.
6. On 2xx, close the replay barrier before reading or converting the body.
7. Read under existing cumulative bounds and run the existing response converter.
8. Deliver the final Anthropic response with checked writes.
9. Emit exactly one `completed`, `failed` or `cancelled` diagnostic.

### Stream

1. Perform the same validation, resolution, single translation and serialization.
2. Retry only network/HTTP failure observations before accepting a successful SSE response.
3. Close the replay barrier on accepted 2xx SSE headers.
4. Run the existing protocol stream/filter path.
5. Never replay after any accepted success, SSE event, terminal event, partial frame or downstream byte.
6. Finalize only after the terminal chunk and flush outcome is known.

## Failure Mapping

Caller failures retain the legacy Anthropic-compatible shape:

```json
{
  "type": "error",
  "error": {
    "type": "api_error",
    "message": "Provider transient failure exhausted retry budget"
  }
}
```

Only closed optional metadata may be added: provider, upstream status, failure class, retryability, allowlisted request ID, capped retry-after seconds and recovery guidance.

The mapping rules are:

- `401`: caller `401`, `authentication_error`, terminal.
- `403`: caller `403`, `permission_error`, terminal.
- Other permanent `400..=499`: preserve status, stable invalid-request message, terminal.
- Exact `insufficient_quota`: caller `429`, quota class, terminal.
- Policy-approved `429`: rate-limit class; retry until success or budget exhaustion, then caller `429`.
- Exhausted `408`: caller `504`, transient class.
- Exhausted retryable `409` or `5xx`: caller `502`, transient class.
- Pre-response connect/timeout exhaustion: caller `504` for timeout and `502` for connect/network.
- Successful-response body/protocol/conversion failure: caller `502`, protocol class, no replay.
- Cancellation: no caller failure generated after the downstream is gone.

## Diagnostics and Redaction

Every started attempt sequence emits one final diagnostic after downstream delivery is known. The diagnostic contains only:

- outcome;
- closed provider and route;
- allowlisted correlation ID;
- post and delay counts;
- mapped and optional upstream status;
- closed failure class and retryability.

It cannot contain request IDs, raw bodies, prompts, API keys, authorization headers, account identifiers, private URLs, model responses or arbitrary upstream messages. Caller-visible request IDs use the existing positive allowlist `[A-Za-z0-9._:-]{1,256}`.

## Deterministic Conformance

Tests are added before each provider-stage behavior change and use real handler plus loopback transport fixtures. Fixture listeners bind ephemeral loopback ports and reject reserved/runtime ports `2999`, `8765`, `9002`, `9003`, `11434`, and `11535`.

Every protocol stage covers:

- `401`, `403`, `400` and `422` with one POST;
- rate-limit recovery and exhaustion with exact delays and 60-second cap;
- exact quota code with one POST;
- network and `5xx` exhaustion within three POSTs;
- `409` retry only for OpenAI Chat and OpenAI Responses;
- byte-identical translated request bodies across retry-only attempts;
- no retry after successful response-open, malformed success body or partial SSE;
- checked downstream disconnect/cancellation and exactly one final diagnostic;
- backward-compatible caller envelope and closed metadata;
- absence of secrets, raw bodies, private URLs and arbitrary upstream strings;
- unchanged protocol-specific request and response fixtures.

The implementation sequence is:

1. Provider contract schema, pure policy validation and Provider Failure domain generalization.
2. Shared single-attempt facts and API-key attempt runner.
3. Anthropic Messages conformance and integration, including Kimi/DeepSeek compatibility.
4. OpenAI Chat conformance and integration, including Qwen/custom/Gemini/Grok compatibility.
5. OpenAI Responses conformance and integration.
6. Full regression, Clippy, evidence, independent review and publication.

No stage is enabled before its expected-red conformance case is observed and its focused tests are green.

## Acceptance Criteria

- All selected provider contracts contain a validated explicit inference retry policy.
- Codex's existing 8/8 harness, 27/27 contract and focused suites remain unchanged and green.
- API-key retry-only attempts are capped at three and use identical translated bytes.
- No retry occurs for permanent/auth/quota failures, after success response-open, after partial stream or after cancellation.
- Safe Repair remains unavailable to every API-key route.
- Each terminal attempt sequence emits exactly one redacted final diagnostic.
- Existing route transformations and successful response shapes remain fixture-compatible.
- Full Gateway and CLI suites pass offline; all-target/all-feature Clippy passes with warnings denied.
- Evidence identifies the exact tested revision and records macOS runtime execution as pending when unavailable.
- The installed runtime is not changed by Ticket 07 implementation or deterministic verification.

## Operational Handoff

Development uses branch `ticket07/provider-failure-routes` in the existing linked worktree. The running installed Gateway and Claude Science remain available to the user throughout development. After tests and final review, the feature branch may be published, but installation/restart requires a separate explicit user approval and a recoverable backup plan.
