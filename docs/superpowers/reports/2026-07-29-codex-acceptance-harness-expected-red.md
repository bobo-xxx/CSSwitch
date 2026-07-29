# Codex Acceptance Harness Expected-Red Audit

Date: 2026-07-29 (Asia/Shanghai)

## Audit identity

- Branch: `prototype/codex-acceptance-harness`
- Tested Git revision: `d71aae5ca70a85bdb5efd9705ea11f0ad2761b9e` (`test: satisfy Codex harness clippy gate`)
- Acceptance inventory: 5 `harness_` tests and 20 `contract_` tests
- Result: both quality gates passed; 5/5 harness tests passed; the contract command produced its intended 4 anchors passed and 16 expected-red target failures; the existing focused Codex suite passed 16/16.

The starting revision, `2b32d672ba2929de0c48cdbd41442077c85c4c61`, exposed one genuine harness-only quality defect before the audit: Clippy rejected the feature-gate self-test's runtime syntax for a compile-time assertion. Commit `d71aae5` changes only that assertion to a const assertion. It does not alter a target assertion or production behavior. All evidence below is from the committed corrected revision.

## Exact commands and results

Quality gates:

```bash
cargo fmt --manifest-path desktop/gateway/Cargo.toml -- --check
cargo clippy --offline --manifest-path desktop/gateway/Cargo.toml \
  --features acceptance-build --lib --tests -- -D warnings
```

Both exited `0`. Clippy completed without warnings.

Harness validity:

```bash
cargo test --offline --manifest-path desktop/gateway/Cargo.toml \
  --features acceptance-build 'server::codex_acceptance::harness_' -- --nocapture
```

Exit `0`: 5 passed, 0 failed, 301 filtered out.

Expected-red contract audit:

```bash
cargo test --offline --manifest-path desktop/gateway/Cargo.toml \
  --features acceptance-build 'server::codex_acceptance::contract_' -- --nocapture
```

Exit `101`, intentionally: 4 passed, 16 failed, 286 filtered out. Every failure reached its named target assertion. There was no harness panic, timeout, malformed HTTP, compiler warning, unused/extra-step failure, or unrelated failure.

Existing focused Codex suite:

```bash
cargo test --offline --manifest-path desktop/gateway/Cargo.toml \
  'server::tests::codex_' -- --nocapture
```

Exit `0`: 16 passed, 0 failed, 265 filtered out.

## Green harness-validity evidence

- Script ordering and request capture: `harness_records_ordered_requests_without_secret_headers` captured two requests in order with their parsed bodies and removed authorization/account headers from the stored projection.
- Unused/extra-step detection: `harness_reports_unused_and_extra_script_steps` reported one remaining step for an early stop and one unexpected POST for an overrun.
- Real handler routing: `harness_runner_uses_real_handler_and_transport` drove the real private Codex handler and `CodexTransport`, consumed one scripted SSE response, and returned the expected HTTP 200 Anthropic response.
- Disconnect and partial SSE: `harness_serves_sse_disconnect_and_partial_responses` served complete SSE, a clean disconnect, and a deliberately truncated response with an overstated content length, in order.
- Feature-gate compilation: `harness_feature_gate_compiles` passed under `test + acceptance-build`.
- Sentinel scanning: the green `contract_transport_diagnostic_projection_is_redacted` rejected all five body/cookie/path/access/account sentinels from the real transport error's Debug and Display projections. The caller-side sentinel scan in `contract_failure_schema_and_caller_redaction` also passed before that test reached its intended `error.provider` target mismatch.

These checks make the red rows below interpretable as production-contract gaps rather than fake-upstream or capture failures.

## Contract observations

For the table, “type” is downstream `error.type` when a JSON error exists, `success` for a successful response, and the stream/transport outcome for cases without a downstream JSON error. “Not constrained” means the named anchor deliberately specifies replay or redaction only; it is not an omitted target assertion. The observations combine the final full-suite output with the exact source-backed result-boundary diagnostics recorded while Tasks 4–7 established these unchanged contracts.

| Contract test | Observed POST / status / type | Target POST / status / type | Verdict and first named gap |
|---|---|---|---|
| `contract_partial_stream_failure_never_replays` | 1 / 200 / one terminal SSE `event: error` | 1 / 200 / one terminal SSE `event: error` | `anchor` — visible prefix once, no replay |
| `contract_failure_schema_and_caller_redaction` | 1 / 502 / `api_error` | 1 / 502 / `api_error` | `expected-red` — `error.provider` is `null`, target `codex`; all sentinel and legacy-envelope assertions before it passed |
| `contract_transport_diagnostic_projection_is_redacted` | 1 / transport 502 (upstream 500) / redacted `CodexTransportError` | 1 / not constrained / redacted transport projection | `anchor` — no sentinel or captured secret header |
| `contract_rate_limit_retries_twice_then_succeeds` | 1 / 429 / `api_error` | 3 / 200 / `success` | `expected-red` — status 429, target 200; 2 steps remain |
| `contract_rate_limit_exhaustion_caps_retry_after` | 1 / 429 / `api_error` | 3 / 429 / `rate_limit_error` | `expected-red` — POST count 1, target 3; later target includes capped delay 60 and request ID |
| `contract_quota_429_is_not_retried` | 1 / 429 / `api_error` | 1 / 429 / `rate_limit_error` | `expected-red` — type mismatch; one unused success step proves no replay |
| `contract_network_and_5xx_retry_within_three_posts` | 1 / 502 / `api_error` | 3 / 200 / `success` | `expected-red` — status 502, target 200; 2 steps remain |
| `contract_408_exhaustion_returns_504` | 1 / 502 / `api_error` | 3 / 504 / `api_error` | `expected-red` — POST count 1, target 3 |
| `contract_409_exhaustion_returns_502` | 1 / 502 / `api_error` | 3 / 502 / `api_error` | `expected-red` — POST count 1, target 3 |
| `contract_500_exhaustion_returns_502` | 1 / 502 / `api_error` | 3 / 502 / `api_error` | `expected-red` — POST count 1, target 3 |
| `contract_503_exhaustion_returns_502` | 1 / 502 / `api_error` | 3 / 502 / `api_error` | `expected-red` — POST count 1, target 3 |
| `contract_lite_non_equivalent_tool_choices_never_post` | 0 each / 400 / `invalid_request_error` | 0 each / 400 / `invalid_request_error` | `anchor` — `none`, `any`, `required`, and named tool all fail before POST |
| `contract_safe_repair_for_absent_choice` | 1 / 502 / `api_error` | 2 / 200 / `success` | `expected-red` — status 502, target 200; one success step remains |
| `contract_safe_repair_for_explicit_auto` | 1 / 502 / `api_error` | 2 / 200 / `success` | `expected-red` — status 502, target 200; one success step remains |
| `contract_unproven_error_text_does_not_authorize_repair` | 1 / 502 / `api_error` | 1 / not constrained / not constrained | `anchor` — arbitrary prose does not authorize replay; one success step remains |
| `contract_permanent_400_is_not_retried` | 1 / 502 / `api_error` | 1 / 400 / `invalid_request_error` | `expected-red` — status 502, target 400 |
| `contract_permanent_404_is_not_retried` | 1 / 502 / `api_error` | 1 / 404 / `invalid_request_error` | `expected-red` — status 502, target 404 |
| `contract_permanent_422_is_not_retried` | 1 / 502 / `api_error` | 1 / 422 / `invalid_request_error` | `expected-red` — status 502, target 422 |
| `contract_401_is_authentication_and_not_retried` | 1 / 401 / `api_error` | 1 / 401 / `authentication_error` | `expected-red` — type mismatch after exact auth-rejection callback passed |
| `contract_403_is_authorization_and_not_retried` | 1 / 403 / `api_error` | 1 / 403 / `permission_error` | `expected-red` — type mismatch after exact auth-rejection callback passed |

All scripted cases recorded zero unexpected POSTs. Where retry or repair is absent, remaining scripted steps match the stated observation. The complete run's 16 failures correspond exactly to permanent typing, Provider Failure metadata, bounded attempt counts, typed rate/quota handling, and the one Safe Repair.

## Ticket 05 responsibility map

| Current gap | Ticket 05 production responsibility | Acceptance evidence that turns green |
|---|---|---|
| Permanent/auth/rate/quota/transient classification and missing caller metadata | Add the pure `provider_failure` module. Accept only sanitized `FailureObservation` values; classify the final `ProviderFailure`; serialize the legacy `type`, `error.type`, and `error.message` plus provider, route, failure class, upstream status, retryability, correlation ID, optional request ID/delay, and recovery. Keep `attempt_count` out of caller JSON. | Permanent 400/404/422, 401/403, quota 429, rate exhaustion, transient exhaustion, and caller schema rows |
| One current POST where three attempts are required | Integrate an `AttemptController` into the Codex Provider Route around the existing single-POST transport call. The controller owns monotonic post/repair/response-started state, the maximum three-POST retry budget, classification eligibility, and terminal decision; the adapter executes directives. Preserve the already-green no-replay-after-bytes anchor. | Rate-limit retry/success, rate exhaustion, mixed network/503/success, and 408/409/500/503 exhaustion rows |
| `Retry-After` and safe request ID are not projected | Extend Codex transport observation extraction to parse only bounded allowlisted response facts before discarding the body: upstream status, normalized/capped `Retry-After`, and a syntactically valid request-ID header. Do not retain cookies, arbitrary headers, URLs, or body text. | Rate-limit exhaustion's delay 60 and `req-acceptance-429`; caller schema request ID |
| Typed automatic-choice rejection is terminal | Add the closed, route-enabled Safe Repair path: only absent/automatic caller choice plus the allowlisted structured rejection may produce `RepairOnce`; consume the one-repair budget; omit only redundant `tool_choice` on the second POST; compare all remaining semantics unchanged. Prose, non-equivalent choices, response-started state, or a used budget must remain terminal. | Two Safe Repair rows; keep Lite, unproven-signal, and partial-stream anchors green |
| No handler-level structured diagnostic sink | Add the future structured diagnostic projection derived from the same sanitized `ProviderFailure`, optionally adding attempt count, delay sequence, stage timing, and mapped/upstream status. It must be structurally unable to carry bodies, prompts, credentials, account IDs, cookies, or private URLs. The direct transport projection remains supplemental redaction evidence, not a substitute. | Future diagnostic assertion plus continued sentinel-scan anchors |

## Resource and production boundary

No live credential, external network endpoint, installed profile, OAuth cache, proxy mutation, or live production behavior was used. The audit used only synthetic sentinels, deterministic ephemeral loopback sockets, and the feature-gated test module. It invoked the real handler and real transport in test configuration, but did not change production code, production state, or non-test behavior.

The overall red contract command is the intended Ticket 03 result. It is expected-red evidence for Ticket 05, not a claim that the Provider Failure Contract passes in production. Production completion requires implementing the responsibility map without weakening these target assertions, then making all 20 contract tests green while retaining the 5 harness tests and existing focused Codex suite.
