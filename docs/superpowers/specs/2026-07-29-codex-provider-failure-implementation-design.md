# Codex Provider Failure Implementation Design

Date: 2026-07-29

Status: approved in interactive design review; pending review of this written capture

## Problem

The Codex Provider Route currently performs one upstream POST and maps most failures to a generic Anthropic-compatible `api_error`. Permanent upstream 4xx responses lose their status and class, 401/403 keep their status but not their authentication semantics, retry and quota signals are dropped, and a proven Responses Lite repair cannot run. This leaves Claude Science retrying an undifferentiated “Claude unavailable” result even when the real failure is permanent or directly actionable.

Ticket 03 established a deterministic real-handler acceptance surface. Its 8 harness tests pass, while 23 target assertions remain authentically red against production. Ticket 05 must turn all 27 contract tests green without weakening the assertions, using credentials, changing proxy state, or restarting the installed service.

During this design review, three accepted fixtures were found to conflict with the already approved transient policy: the caller-redaction case and the malformed/oversized request-ID cases each scripted one `500` even though a pre-response `500` must consume the three-POST retry budget. The user approved strengthening those fixtures before production code is written.

## Approved inputs

This design implements the decisions already accepted in:

- `docs/superpowers/specs/2026-07-29-provider-failure-contract-seam-design.md` on `prototype/provider-failure-contract-seam`;
- `docs/superpowers/specs/2026-07-29-codex-acceptance-harness-design.md` on `prototype/codex-acceptance-harness`;
- `.scratch/provider-failure-contract/issues/04-decide-provider-rollout-boundary.md`;
- the user-approved fallback schedule of 500 ms, then 1,000 ms, with at most three total POSTs and a 60-second `Retry-After` cap.

## Goals

- Add the pure Provider Failure Contract as a production deep module.
- Integrate its first adapter through the real Codex handler and `CodexTransport`.
- Preserve provider-specific OAuth, model selection, request translation, cancellation, and SSE reduction.
- Classify permanent, authentication, authorization, quota, rate-limit, transient, network, capability, and protocol failures.
- Enforce bounded retries, one closed Safe Repair, cancellation finality, and the response-started replay barrier.
- Preserve the legacy Anthropic error fields while adding only sanitized structured metadata.
- Add cancellable scheduling and one structured diagnostic per completed attempt sequence.
- Turn the accepted Codex contract suite fully green without using live credentials.

## Non-goals

- Applying the contract to the shared API-key `messages.rs` adapter; that remains Ticket 07.
- Live installed-runtime or subscription verification; that remains Ticket 06.
- Provider or model fallback.
- Silent history deletion, model substitution, compaction, or semantic rewriting.
- Changing model discovery, login/refresh protocols, proxy settings, Claude Science UI, or orchestration.
- Adding dependencies or exposing raw upstream bodies, prompts, credentials, account identifiers, cookies, or private URLs.

## Alternatives considered

### 1. Pure controller with a handler-owned attempt loop — selected

Add `provider_failure.rs` as the pure policy module. `CodexTransport` performs exactly one POST and returns only bounded allowlisted facts. The Codex handler owns the loop that executes controller directives and writes terminal results. This matches the accepted seam, keeps transport mechanics local, and can accept the second `messages.rs` adapter in Ticket 07.

### 2. Handler-local policy — rejected

Putting classification, budgets, serialization, and redaction directly in `server.rs` is initially smaller but fails the deletion test. Ticket 07 would either duplicate the logic or require a risky extraction after Codex behavior ships.

### 3. Transport-owned retries — rejected

Hiding retries in `CodexTransport` prevents the handler from authorizing exact POST counts, request repair, final envelope serialization, diagnostics, and the downstream response-started barrier. It would also make the future blocking `messages.rs` adapter share the wrong interface.

## Module structure

### `provider_failure.rs`

The new pure module owns:

- closed provider, route, failure, observation, repair, network, and protocol enums;
- validated `RetryPolicy` and `RouteContext`;
- monotonic `AttemptState` and `AttemptController` transitions;
- `ProviderFailure` classification and Anthropic-compatible serialization;
- `AttemptDiagnostic` construction from sanitized state and the same final failure;
- validated bounded `RequestId` and normalized retry-delay values.

The module must not depend on reqwest, OAuth types, sockets, request/response bodies, server writers, or arbitrary error strings.

The first production enums contain only the Codex values needed now. Ticket 07 may add closed generic-route values without changing controller state semantics.

### `codex_transport.rs`

`CodexTransport::open_responses` continues to perform exactly one POST. A failed request returns a non-`Copy` `CodexTransportError` containing only:

- mapped transport status and optional upstream status;
- a closed network/timeout stage;
- optional normalized rate kind;
- optional validated request ID;
- optional parsed `Retry-After` delta seconds;
- closed error-code and parameter signals needed for quota and capability decisions;
- an existing stable static detail string for direct transport diagnostics;
- cancellation state.

For non-success responses, the transport reads no more than 16 KiB of the body. It parses only `error.type`, `error.code`, and `error.param`, immediately reduces them to closed enums, and discards the body. Quota wins if either type or code equals `insufficient_quota`. Rate limit is proven only by `rate_limit_error` or `rate_limit_exceeded`. A typed capability signal exists when the allowlisted code/parameter vocabulary is present; repair requires the exact pair `unsupported_value` plus `tool_choice`.

`Retry-After` accepts only an ASCII decimal delta-seconds value. The controller caps it at 60 seconds. A request ID is accepted only when it contains 1–256 ASCII alphanumeric characters or `-`, `_`, `.`, or `:`. All other values are omitted. No other header is projected.

### `server.rs`

The Codex handler retains request translation, auth invalidation, response reduction, and downstream I/O. It adds the attempt loop:

1. Translate the caller request once to a mutable JSON value.
2. Construct the Codex `RouteContext`, correlation ID, controller, cancellation monitor, and attempt runtime.
3. Ask the controller to authorize a POST.
4. Serialize the current translated value and invoke the single-POST transport.
5. Convert a transport error into a closed `FailureObservation` and execute the returned directive.
6. For `RetryAfter`, wait through the cancellable runtime, then repeat the same body.
7. For `RepairOnce`, remove only the top-level translated `tool_choice: "auto"`, then repeat.
8. For `Fail`, write the controller-owned envelope and emit the terminal diagnostic.
9. For `Cancel`, emit a cancellation diagnostic and write no new caller response.
10. For an opened upstream, permanently close replay and continue through the existing SSE/non-stream reducer. Emit the one final diagnostic only when that reducer completes, fails, or is cancelled.

## Attempt state and budgets

The controller phases are initial-ready, in-flight, retry-authorized, repair-authorized, and terminal. The following values are monotonic: POSTs started, repairs used, response started, and consumed delay sequence.

- At most three POSTs may start for a retry-only sequence.
- Missing valid `Retry-After` values use 500 ms after POST one and 1,000 ms after POST two.
- Valid delta seconds override the fallback for that transition and are capped at 60 seconds.
- Zero is a valid delay and is still recorded as a consumed scheduling directive.
- Safe Repair is legal only after the first POST, before any retry delay, with no response started and no prior repair.
- A repair sequence may start exactly one additional POST, so it is permanently capped at two POSTs.
- After the repair POST, every further failure is terminal; it cannot consume a retry budget.
- A capability signal discovered after a retry is terminal and cannot authorize repair.
- Once an upstream response is accepted or any downstream response bytes are written, retries and repair are permanently forbidden.
- Cancellation is terminal and can never authorize another POST.

These rules intentionally prevent a request from combining retry and repair budgets into more than the approved maximum.

## Safe Repair

Safe Repair is enabled only when all conditions hold:

- the active route is Codex Responses Lite;
- the caller's tool choice was absent or explicitly automatic;
- the translated first body contains top-level `tool_choice: "auto"`;
- the first upstream rejection contains exact closed facts `unsupported_value` and `tool_choice`;
- this is the first POST, no delay or repair was consumed, and no response began.

POST two removes only that top-level field. Parsed bodies before and after removal must otherwise be equal. Prose, an unknown code, a wrong parameter, a non-Lite route, a prior retry, a prior repair, or response-started state cannot authorize repair. A second typed rejection is terminal and leaves a scripted third success unused.

## Classification and caller envelope

| Observation | Internal action | Terminal status/type/class | Caller `retryable` |
|---|---|---|---|
| 401 | Fail; invoke existing auth callback for the next request | 401 / `authentication_error` / `authentication` | false |
| 403 | Fail; invoke existing auth callback | 403 / `permission_error` / `authorization` | false |
| Other permanent 4xx | Fail without replay | Preserve status / `invalid_request_error` / `invalid_request` | false |
| Typed capability rejection not repaired | Fail without replay | 400 / `invalid_request_error` / `capability` | false |
| Proven quota 429 | Fail after one POST | 429 / `rate_limit_error` / `quota` | false |
| Proven rate-limit 429 | Retry within budget | 429 / `rate_limit_error` / `rate_limit` | true |
| Upstream 408 exhaustion | Retry within budget | 504 / `api_error` / `transient` | true |
| Upstream 409 or 5xx exhaustion | Retry within budget | 502 / `api_error` / `transient` | true |
| Network timeout exhaustion | Retry within budget | 504 / `api_error` / `network` | true |
| Other network exhaustion | Retry within budget | 502 / `api_error` / `network` | true |
| Protocol failure | Never retry after response start | 502 / `api_error` / `protocol` | false |
| Cancellation | Stop | No new response | false |

An unknown 429 is terminal and conservative: it uses `rate_limit_error`/`rate_limit`, but is not internally retried and sets `retryable: false` because neither rate capacity nor quota semantics were proven.

Every JSON failure preserves:

```json
{
  "type": "error",
  "error": {
    "type": "api_error",
    "message": "stable sanitized message"
  }
}
```

The `error` object then adds `provider`, `route`, `failure_class`, `retryable`, `correlation_id`, and `recovery`. It adds `upstream_status`, `request_id`, and `retry_after_seconds` only when known and valid. Unknown optionals are absent, never `null`. Attempt count, repair count, and delay sequence never enter the caller envelope.

The route label is `responses_lite` when the selected model uses that private capability surface and `responses` otherwise. The provider label is always `codex` for this slice. The correlation ID is generated locally per caller request and cannot derive from a prompt, credential, account identity, upstream body, URL, or cookie.

## Scheduler and diagnostics

An internal attempt-runtime interface has two responsibilities:

- `wait(delay, cancellation)` records or performs a delay and returns early when cancellation is observed;
- `emit(diagnostic)` receives a structurally sanitized `AttemptDiagnostic`.

The production runtime waits in short bounded intervals so the existing downstream cancellation monitor remains effective even during a 60-second `Retry-After`. Tests inject a recording runtime and never sleep.

Every completed attempt sequence emits exactly one final diagnostic with outcome `completed`, `failed`, or `cancelled`. An opened upstream is not itself completion; the diagnostic is deferred until the existing reducer completes, fails, or is cancelled.

The diagnostic may contain only:

- outcome, provider, route, and correlation ID;
- POST count, repair count, and consumed delay milliseconds;
- final mapped status, optional upstream status, failure class, and retryability.

It omits the request ID as unnecessary log data and structurally cannot contain arbitrary body/header strings. Production emits compact JSON with a fixed `provider_attempt` prefix. Tests assert exact attempt/delay values and scan all formatted diagnostics for synthetic credential, account, body, cookie, and URL sentinels.

Protocol failures that occur after the upstream opens retain the no-replay behavior. Streaming callers receive exactly one final sanitized SSE error event; non-streaming callers receive the Provider Failure JSON. Success emits `completed`, a reducer failure emits `failed`, and an early downstream disconnect emits `cancelled`; none of these paths emits an earlier opening diagnostic.

## Security and redaction

- Observation and diagnostic types contain no arbitrary upstream strings.
- Error bodies are bounded to 16 KiB before parsing and immediately reduced to closed facts.
- Authorization, ChatGPT account ID, cookies, raw bodies, prompt/history/tool data, private endpoint paths, and challenge content never enter the controller, caller envelope, or diagnostic.
- `Debug` and `Display` for transport and controller errors expose only stable static messages and sanitized typed fields.
- Request ID validation is positive allowlisting; invalid and oversized values are omitted.
- The accepted harness sentinel checks remain unchanged and must pass after production integration.

## Branch and TDD strategy

After this design and its implementation plan are reviewed, create `feature/codex-provider-failure` from the current `linux-headless-oauth` commit in a project-owned isolated worktree. Merge the accepted `prototype/codex-acceptance-harness` branch verbatim before making the separately approved Ticket 05 fixture amendments. This establishes the required initial red baseline: 8/8 harness tests, 4/27 green contract anchors, and 23/27 authentic production failures.

Implement in red-green slices:

1. Strengthen `contract_failure_schema_and_caller_redaction`, `contract_malformed_request_id_is_omitted`, and `contract_oversized_request_id_is_omitted` to script three `500` responses and require exactly three POSTs. Confirm they remain authentically red against the unchanged handler. Do not rewrite the historical Ticket 03 report, whose tested revision remains accurate.
2. Add pure controller classification, state, envelope, and redaction tests.
3. Add bounded transport fact extraction and safe header/body parsing tests.
4. Integrate permanent/auth/quota handler behavior.
5. Integrate the retry scheduler and attempt loop.
6. Add the closed Safe Repair path and its negative discriminators.
7. Add structured diagnostics, cancellation-aware waiting, and response-started lifecycle.
8. Run the full acceptance turn-green and regression audit.

No production code may be added before its focused test has failed for the intended missing behavior.

## Verification

The implementation gate requires:

- pure `provider_failure` unit tests green;
- `CodexTransport` extraction/redaction tests green;
- accepted harness tests 8/8 green;
- accepted contract tests 27/27 green with no relaxed assertion;
- existing focused Codex tests 16/16 green;
- the complete gateway test suite green;
- `cargo fmt --check` green;
- offline Clippy for all gateway targets and features at `-D warnings` green;
- `git diff --check` green;
- an independent final code/security review with no Critical or Important findings.

The evidence report records the tested code revision, environment/toolchain, exact commands, attempt/delay results, redaction boundary, and the explicit fact that no live credential, proxy mutation, installed profile, or running service was used.

## Completion boundary

Ticket 05 is complete when the production module and Codex adapter turn the deterministic suite fully green and the implementation branch is published for review. It does not restart or replace the installed CSSwitch gateway. Ticket 06 alone owns installed-runtime and live subscription verification.

Ticket 07 later adds the shared `messages.rs` adapter using explicit Provider Retry Policies. It reuses the controller, envelope, diagnostic, and state invariants, but enables no Codex-specific Safe Repair.
