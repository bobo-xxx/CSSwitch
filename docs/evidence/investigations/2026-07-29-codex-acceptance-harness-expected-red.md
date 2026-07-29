# Codex Acceptance Harness Expected-Red Audit

Date: 2026-07-29 (Asia/Shanghai)

## Evidence boundary

- Branch: `prototype/codex-acceptance-harness`
- Tested code revision: `ebb46b3b2cb6fcb183a83bc6f0f3deea42b6f808` (`test: harden Codex acceptance review surface`)
- Test layer: source/unit acceptance against deterministic loopback HTTP; not an installed runtime or live provider test
- Acceptance inventory: 8 `harness_` tests and 27 `contract_` tests
- Result: formatting and Clippy passed; 8/8 harness tests passed; the contract command produced its intended 4 anchors and 23 authentic expected-red target failures; the existing focused Codex suite passed 16/16.

The evidence document is committed after the tested code revision, following the repository's existing convention for dated investigations. No code changed between the tested revision and the recorded commands.

## Environment

- OS: Ubuntu 22.04.4 LTS (Jammy), Linux `6.8.0-90-generic`, `x86_64`
- Rust: `rustc 1.97.0 (2d8144b78 2026-07-07)`, host `x86_64-unknown-linux-gnu`, LLVM 22.1.6
- Cargo: `cargo 1.97.0 (c980f4866 2026-06-30)`
- Build context: Cargo development test profile (`unoptimized + debuginfo`), dependency resolution forced offline with `--offline`
- Harness compilation: existing `acceptance-build` feature plus `test`; default/non-test production behavior is unchanged
- Network context: ephemeral `127.0.0.1` listeners only; the real `CodexTransport` used its direct test client against the scripted loopback endpoint

One interim parallel invocation of the existing focused suite hit the sandbox's concurrent ephemeral-bind limit (`EPERM` at `TcpListener::bind`). The same 16 tests passed serially without a code change, and the exact prescribed parallel command then passed after the sockets drained. The authoritative result below is that final exact command, exit `0`; the transient environment event is not classified as a code failure.

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

Exit `0`: 8 passed, 0 failed, 308 filtered out.

Expected-red contract audit:

```bash
cargo test --offline --manifest-path desktop/gateway/Cargo.toml \
  --features acceptance-build 'server::codex_acceptance::contract_' -- --nocapture
```

Exit `101`, intentionally: 4 passed, 23 failed, 289 filtered out. Every failure reached its named production target assertion. There was no harness panic, hang, malformed HTTP, compiler warning, invalid-method/request counter, or unrelated failure.

Existing focused Codex suite:

```bash
cargo test --offline --manifest-path desktop/gateway/Cargo.toml \
  'server::tests::codex_' -- --nocapture
```

Exit `0`: 16 passed, 0 failed, 265 filtered out.

## Green harness-validity evidence

- `harness_feature_gate_compiles`: compiled only under `test + acceptance-build`.
- `harness_runner_uses_real_handler_and_transport`: drove the real private handler and `CodexTransport`, captured one exact `POST /responses`, and returned the expected Anthropic response.
- `harness_records_ordered_requests_without_secret_headers`: captured two ordered POST bodies and retained only the positive `accept`/`content-type` header allowlist.
- `harness_reports_unused_and_extra_script_steps`: reported both an unused step and an extra POST without hiding either condition.
- `harness_serves_sse_disconnect_and_partial_responses`: served complete SSE, disconnect, and deliberately truncated SSE with exact wire framing.
- `harness_rejects_oversized_request_headers_and_bodies`: proved the 16 KiB header and 256 KiB body ceilings reject before unbounded allocation.
- `harness_captures_only_allowlisted_request_headers`: removed authorization, account, and arbitrary private headers from the request projection.
- `harness_rejects_and_separately_accounts_non_post_requests`: returned 405, incremented the method counter, and did not consume a scripted POST step.

Every real-handler case executes a shared integrity assertion before its contract assertion. It requires zero unexpected POSTs, methods, or malformed requests, and verifies every captured attempt is exactly `POST` to the configured path. This makes the POST counts in the contract table method- and path-specific rather than generic connection counts.

The stream anchor additionally dechunks and parses downstream SSE. It proves a visible prefix, exactly one terminal `event: error`, terminal JSON with legacy `type/error.type/error.message`, and that the terminal error is the final event. Its upstream path, headers, and unforwarded partial body contain unique synthetic sentinels; the complete downstream wire response contains none of them.

## Contract observations

“Type” is downstream `error.type` for JSON failures, `success` for a successful response, and the stream/transport outcome where no downstream JSON error exists. “Not constrained” means the anchor deliberately specifies replay or redaction only. All one-or-more-attempt cases passed the shared exact POST/path integrity assertion before reaching the listed result.

| Contract test | Observed POST / status / type | Target POST / status / type | Verdict and first named gap |
|---|---|---|---|
| `contract_partial_stream_failure_never_replays` | 1 / 200 / parsed terminal SSE `api_error` | 1 / 200 / parsed terminal SSE `api_error` | `anchor` — visible prefix once, one terminal error last, no replay or sentinel |
| `contract_failure_schema_and_caller_redaction` | 1 / 502 / `api_error` | 1 / 502 / `api_error` | `expected-red` — `error.provider` absent, target `codex`; sentinel and legacy fields passed first |
| `contract_transport_diagnostic_projection_is_redacted` | 1 / transport 502 (upstream 500) / redacted `CodexTransportError` | 1 / not constrained / redacted transport projection | `anchor` — no sentinel or non-allowlisted captured header |
| `contract_rate_limit_retries_twice_then_succeeds` | 1 / 429 / `api_error` | 3 / 200 / `success` | `expected-red` — status 429, target 200; two steps remain |
| `contract_rate_limit_exhaustion_caps_retry_after` | 1 / 429 / `api_error` | 3 / 429 / `rate_limit_error` | `expected-red` — POST count 1, target 3; later targets include delay cap 60 and safe request ID |
| `contract_quota_429_is_not_retried` | 1 / 429 / `api_error` | 1 / 429 / `rate_limit_error` | `expected-red` — type mismatch; one unused success step proves no replay |
| `contract_network_and_5xx_retry_within_three_posts` | 1 / 502 / `api_error` | 3 / 200 / `success` | `expected-red` — status 502, target 200; two steps remain |
| `contract_network_only_exhaustion_omits_unknown_upstream_metadata` | 1 / 502 / `api_error` | 3 / 502 / `api_error` | `expected-red` — POST count 1, target 3; terminal target requires absent upstream status/request ID/delay |
| `contract_408_exhaustion_returns_504` | 1 / 502 / `api_error` | 3 / 504 / `api_error` | `expected-red` — POST count 1, target 3 |
| `contract_409_exhaustion_returns_502` | 1 / 502 / `api_error` | 3 / 502 / `api_error` | `expected-red` — POST count 1, target 3 |
| `contract_500_exhaustion_returns_502` | 1 / 502 / `api_error` | 3 / 502 / `api_error` | `expected-red` — POST count 1, target 3 |
| `contract_503_exhaustion_returns_502` | 1 / 502 / `api_error` | 3 / 502 / `api_error` | `expected-red` — POST count 1, target 3 |
| `contract_lite_non_equivalent_tool_choices_never_post` | 0 each / 400 / `invalid_request_error` | 0 each / 400 / `invalid_request_error` | `anchor` — `none`, `any`, `required`, and named tool fail before POST |
| `contract_safe_repair_for_absent_choice` | 1 / 502 / `api_error` | 2 / 200 / `success` | `expected-red` — status 502, target 200; one success step remains |
| `contract_safe_repair_for_explicit_auto` | 1 / 502 / `api_error` | 2 / 200 / `success` | `expected-red` — status 502, target 200; one success step remains |
| `contract_unproven_error_text_does_not_authorize_repair` | 1 / 502 / `api_error` | 1 / not constrained / not constrained | `anchor` — arbitrary prose does not authorize replay |
| `contract_unknown_typed_code_does_not_authorize_repair` | 1 / 502 / `api_error` | 1 / 400 / `invalid_request_error` | `expected-red` — no replay passed; terminal typed capability envelope is missing |
| `contract_allowlisted_code_with_wrong_param_does_not_authorize_repair` | 1 / 502 / `api_error` | 1 / 400 / `invalid_request_error` | `expected-red` — no replay passed; terminal typed capability envelope is missing |
| `contract_route_disabled_does_not_authorize_repair` | 1 / 502 / `api_error` | 1 / 400 / `invalid_request_error` | `expected-red` — non-Lite route does not replay; target route-aware terminal envelope is missing |
| `contract_used_repair_budget_stops_after_second_typed_rejection` | 1 / 502 / `api_error` | 2 / 400 / `invalid_request_error` | `expected-red` — POST count 1, target 2; target leaves the scripted third success unused |
| `contract_malformed_request_id_is_omitted` | 1 / 502 / `api_error`, ID absent | 1 / 502 / `api_error`, ID absent | `expected-red` — malformed-ID omission passed; `error.provider` is absent |
| `contract_oversized_request_id_is_omitted` | 1 / 502 / `api_error`, ID absent | 1 / 502 / `api_error`, ID absent | `expected-red` — oversized-ID omission passed; `error.provider` is absent |
| `contract_permanent_400_is_not_retried` | 1 / 502 / `api_error` | 1 / 400 / `invalid_request_error` | `expected-red` — status mismatch |
| `contract_permanent_404_is_not_retried` | 1 / 502 / `api_error` | 1 / 404 / `invalid_request_error` | `expected-red` — status mismatch |
| `contract_permanent_422_is_not_retried` | 1 / 502 / `api_error` | 1 / 422 / `invalid_request_error` | `expected-red` — status mismatch |
| `contract_401_is_authentication_and_not_retried` | 1 / 401 / `api_error` | 1 / 401 / `authentication_error` | `expected-red` — type mismatch after exact auth callback and no replay passed |
| `contract_403_is_authorization_and_not_retried` | 1 / 403 / `api_error` | 1 / 403 / `permission_error` | `expected-red` — type mismatch after exact auth callback and no replay passed |

The 23 failures correspond exactly to permanent/auth/rate/quota/transient classification, Provider Failure metadata, bounded retry counts, malformed optional metadata policy, and the closed one-repair path. They are assertions against natural real-handler output; the tests contain no artificial failure branches.

## Ticket 05 responsibility map

| Ticket 03 observation | Ticket 05 production responsibility | Acceptance evidence that turns green |
|---|---|---|
| Permanent/auth/rate/quota/transient results lack classification and caller metadata | Add the pure Provider Failure module and serialize the legacy fields plus provider, route, failure class, retryability, correlation ID, optional allowlisted facts, and recovery. Keep unknown optional fields absent, not null. | Permanent, auth, quota, caller schema, request-ID, and transient terminal rows |
| Retryable cases stop after one POST | Integrate the Attempt Controller around the Codex transport call with a maximum three-POST budget and the existing response-started barrier. | Rate success/exhaustion, mixed network/503 success, network-only exhaustion, and 408/409/500/503 rows |
| Retry and request-ID headers are not projected | Extract only bounded allowlisted response facts, cap `Retry-After` at 60 seconds, accept only syntactically valid bounded request IDs, and omit invalid/unknown fields. | Rate exhaustion plus malformed/oversized request-ID rows |
| Typed automatic-choice rejection is terminal | Add the closed Responses Lite route-enabled repair. Require allowlisted code plus exact `tool_choice` param, consume the one-repair budget, and omit only redundant automatic choice on POST two. Keep prose, unknown code, wrong param, non-Lite route, response-started state, and used budget terminal. | Two repair-success rows, three typed negative rows, used-budget row, and existing prose/stream anchors |
| No handler-level structured diagnostic sink or deterministic scheduler seam exists | Add both in Ticket 05. Diagnostics derive only from sanitized Provider Failure and may add attempt count/delay sequence; scheduler injection proves positive delay order without wall-clock sleeps. Neither seam is fabricated in Ticket 03. | Future Ticket 05 diagnostic/scheduler tests while all Ticket 03 caller, attempt, and redaction assertions stay unchanged |

## Resource and production boundary

No live credential, external endpoint, installed profile, OAuth cache, proxy mutation, or production runtime behavior was used. The audit used synthetic sentinels, bounded ephemeral loopback sockets, and the feature-gated test module. It invoked the real handler and real transport in test configuration but did not change production modules, dependencies, production state, or non-test behavior.

The overall red contract command is the intended Ticket 03 result. It is expected-red evidence for Ticket 05, not a claim that the Provider Failure Contract passes in production. Ticket 03 also does not claim handler-level structured diagnostics, deterministic scheduler injection, fallback-delay policy, consumed-delay sequences, or positive-delay sequencing; those remain explicit Ticket 05 production work.
