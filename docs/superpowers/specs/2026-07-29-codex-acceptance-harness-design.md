# Codex Acceptance Harness Design

Date: 2026-07-29

Status: approved in interactive design review and written-spec review

## Problem

CSSwitch's current Codex route has useful focused tests, but they do not exercise the full boundary from the real Anthropic-compatible handler through the real `CodexTransport` to a controllable upstream. The current implementation performs one POST, does not own a retry or Safe Repair loop, preserves only selected upstream statuses, does not normalize `Retry-After`, and does not expose the complete Provider Failure metadata designed in the Provider Failure Contract seam.

The production implementation ticket therefore needs a deterministic acceptance surface that can answer externally visible questions before production logic is changed: how many POSTs occurred, what changed between attempts, when replay stopped, which status and envelope reached the caller, and whether upstream or credential-bearing data leaked.

## Goals

- Exercise the real current Codex handler and the real `CodexTransport` against a deterministic loopback upstream.
- Encode the approved Codex-first Provider Failure Contract as target assertions.
- Cover non-streaming rejections and streaming response-started behavior.
- Prove exact POST counts and paths, ordered upstream responses, bounded retry and Safe Repair targets, schema compatibility, caller redaction, and the current transport error's redacted projection.
- Keep the default test suite and all non-test builds unaffected by the harness; compile it only when the existing `acceptance-build` feature is explicitly selected for tests.
- Produce precise expected-red evidence that the production implementation ticket can turn green without weakening assertions or redesigning the fake upstream.

## Non-goals

- Implementing the Provider Failure Contract, retry controller, Safe Repair, handler-level structured diagnostic sink, or production/test scheduler seam.
- Using a live Codex subscription, OAuth flow, external network endpoint, or real credential.
- Provider or model fallback.
- Exercising generic, Kimi, or non-Codex Provider Routes.
- Exhaustively combining a Safe Repair with later transient retries; this slice proves each budget and replay rule independently.
- Measuring elapsed wall-clock time.
- Preserving arbitrary upstream error text in caller or transport error output.
- Creating a reusable general-purpose HTTP mocking framework.

## Current evidence

- `codex_protocol::translate_anthropic_request` already rejects non-equivalent Responses Lite tool choices before transport.
- `codex_transport::open_responses` performs one POST with reqwest retries disabled.
- `server::handle_codex_messages_with_policy` invokes the transport once and maps failures through the current generic error helper.
- Existing focused Codex tests pass and establish current single-POST behavior, but they do not describe the target retry/controller contract.
- Existing test helpers can return one repeated response or a simple response sequence; neither models disconnects, partial SSE, per-attempt request differences, and the complete target result in one acceptance case.

## Decision

Add a feature-gated scripted loopback upstream under the server tests. The harness calls the real private Codex handler with a real `CodexTransport` configured for the loopback endpoint. It introduces no transport trait and no production abstraction.

The harness boundary is enabled only by:

```rust
#[cfg(all(test, feature = "acceptance-build"))]
```

The existing `acceptance-build` Cargo feature is reused, but the harness itself additionally requires `test`. Normal builds and default tests do not compile or execute it, and this ticket does not add a non-test behavior to that feature.

This is the deepest practical seam for the question. It observes externally meaningful behavior through the production handler and transport while replacing only the remote Codex service. It can detect accidental duplicate POSTs, unsafe mutations, response-started replay, and caller-envelope incompatibility that an in-memory controller or fixture-only test cannot prove.

### Rejected alternatives

1. **Introduce an in-memory transport trait first.** Rejected because it changes the production architecture before the acceptance behavior has justified that abstraction and can conceal real HTTP/stream behavior.
2. **Use recorded response fixtures without a server.** Rejected because fixtures cannot reliably prove request attempt counts, ordered response consumption, disconnect timing, or the no-replay barrier after SSE bytes.
3. **Drive a live Codex endpoint.** Rejected because it is non-deterministic, credential-bearing, externally mutable, and inappropriate for rejection and retry-exhaustion tests.

## Test-only architecture

### `ScriptedCodexUpstream`

A loopback server binds an ephemeral port and consumes an ordered queue of steps. Supported steps are deliberately bounded to the cases under review:

- `Http`: return a complete status, allowlisted headers, and bounded body.
- `Disconnect`: accept the request and close before a valid response is completed.
- `Sse`: return a complete SSE stream.
- `PartialSseThenDrop`: return successful streaming headers and at least one valid SSE event, then close before the stream completes.

For every accepted POST, the server records the method, path, positively allowlisted `accept`/`content-type` headers, and parsed JSON body. Request headers are capped at 16 KiB, bodies at 256 KiB, and total-size arithmetic is checked. Non-POST and malformed requests are rejected and counted separately without consuming a scripted POST step. At test completion it reports consumed and unconsumed script steps so an early or extra attempt is explicit.

### `AcceptanceCase`

Each table case contains:

- a synthetic Anthropic request fixture;
- an ordered upstream script;
- the expected POST count and response sequence;
- optional per-attempt request-body assertions;
- the expected downstream status, compatible error envelope, and caller retry metadata;
- forbidden sentinel values for caller and transport-projection redaction checks.

Ticket 03 does not inject retry delays or collect handler-level attempt diagnostics. The expected POST budget is a target assertion against the authentic handler result. Deterministic scheduling, consumed-delay sequences, and structured attempt diagnostics require production seams and are explicit Ticket 05 work.

### `run_case`

The runner constructs test-only route state with a synthetic token, points a real `CodexTransport` at the scripted loopback URL, and invokes the real private server handler. It captures the downstream HTTP response or stream, captured POST summaries, and script-consumption result. A direct `CodexTransportError` Debug/Display check supplies supplemental redaction evidence; it is not represented as a handler-level structured diagnostic collector.

The runner makes no outbound connection other than its loopback server. It does not read the installed CSSwitch profile, OAuth cache, process proxy settings, or user credential files.

## Behavior matrix

| Scenario | Script and expected attempts | Required result |
|---|---|---|
| Responses Lite `tool_choice: none` | No upstream response; zero POSTs | Local 400 `invalid_request_error`; explain incompatibility without rewriting semantics |
| Responses Lite required/forced tool | No upstream response; zero POSTs | Local 400 `invalid_request_error`; never convert to automatic choice |
| Permanent upstream 400, 404, or 422 | One rejection; one POST | Preserve the exact 400, 404, or 422 status with `invalid_request_error`; non-retryable |
| Authentication 401 | One rejection; one POST | 401 `authentication_error`; no replay of the current caller request |
| Authorization 403 | One rejection; one POST | 403 `permission_error`; no replay of the current caller request |
| Rate-limit 429 | Two retryable responses followed by success or final rejection; at most three POSTs | `rate_limit_error` if exhausted; normalized/capped caller delay and ordered attempts |
| Quota 429 | One typed quota rejection; one POST | 429 `rate_limit_error`; explicitly non-retryable |
| Network, 408, 409, or 5xx before response bytes | Failure sequence followed by success or exhaustion; at most three POSTs | Retry only within budget; 504 for timeout-class exhaustion and 502 `api_error` for other transient exhaustion |
| Proven automatic-choice Safe Repair | Allowlisted typed rejection followed by success; exactly two POSTs | Omit only redundant automatic `tool_choice` on the second POST; all other request semantics unchanged |
| Unauthorized repair signal | Prose, unknown code, wrong `param`, or route-disabled typed rejection; one POST | No repair and no replay |
| Used repair budget | Allowlisted typed rejection twice, then a scripted success; exactly two POSTs | The second rejection is terminal; the third script step remains unused |
| Partial SSE then failure | Successful streaming prefix, at least one forwarded event, then drop; one POST | Preserve emitted prefix, produce a sanitized terminal stream failure, and never replay |
| Schema compatibility | Any final failure | Preserve `type`, `error.type`, and `error.message`; add only the approved Provider Failure fields |
| Optional metadata boundary | Network-only exhaustion or malformed/oversized request-ID headers | Unknown fields are absent rather than null; invalid request IDs are omitted |
| Redaction | Error body and forbidden headers contain unique synthetic sentinels | No sentinel, token, body, cookie, or private upstream URL in caller output or the supplemental transport error projection |

Some target assertions may already pass, especially pre-POST capability rejection. They remain green contract anchors. “Expected red” means the focused suite truthfully fails where the current handler lacks the approved behavior; tests must never contain artificial failure branches.

## Safe Repair evidence

The only repair exercised by this Codex-first harness is omission of a redundant automatic `tool_choice` representation:

1. The caller's tool choice is absent or automatic.
2. The first POST receives a bounded structured rejection with an allowlisted error code and `param` identifying `tool_choice`.
3. The active Provider Route explicitly enables the closed repair kind.
4. No response byte has reached the caller and no prior repair was used.
5. The second POST omits only the redundant automatic field.

The harness compares the parsed first and second request bodies after removing that single permitted field. The remainder must be equal. `none`, required, and forced-tool inputs fail before any POST and are never repair candidates. Error prose, keyword matching, an unknown code, or a mismatched parameter cannot authorize a repair.

For Ticket 03, Responses Lite is the only route treated as the target-enabled repair case. A non-Lite Responses case supplies the route-disabled negative. Ticket 05 must add the actual closed route policy and keep the negative green.

## Retry policy and timing assertions

- A caller request may start at most three upstream POSTs for retryable rate-limit or transient failures.
- A Safe Repair may replay exactly once, for at most two POSTs in that case.
- `Retry-After` delta seconds are parsed into a normalized duration and capped at 60 seconds.
- Tests assert normalized caller delay values, caps, ordered response consumption, and total POST count.
- Ticket 03 supplies zero-second `Retry-After` values on scripted intermediate responses and does not inject or observe sleeps. A rate-limit exhaustion case puts an over-cap delay on the final response, where it can be normalized to 60 seconds without another sleep.
- Ticket 05 owns validated fallback-delay inputs, the deterministic scheduler seam, consumed-delay diagnostics, and positive-delay sequencing tests. Ticket 03 does not claim those production behaviors are complete.

## Response-started barrier

The fake upstream's partial-stream step ensures the handler receives a successful response and forwards a valid SSE event before the upstream fails. Once those bytes are observed, the expected upstream POST count is permanently one. Neither a transport retry nor the Safe Repair path may reopen the request. Any terminal error representation must be stream-compatible and sanitized rather than a second JSON HTTP response.

## Schema and redaction assertions

Every final non-streaming failure retains the legacy Anthropic-compatible fields:

```json
{
  "type": "error",
  "error": {
    "type": "api_error",
    "message": "sanitized stable message"
  }
}
```

Target contract cases additionally assert the approved provider, route, failure class, retryability, correlation ID, optional upstream status, optional allowlisted request ID, optional normalized retry delay, and recovery guidance. Optional fields are absent rather than null when unknown. Attempt count and the consumed delay sequence do not belong to the caller envelope; their future structured diagnostic projection is Ticket 05 work. For a final rate-limit response, `retryable` describes whether a new caller request may succeed after the normalized delay; it does not imply that the exhausted controller will start a fourth internal POST.

The upstream may place unique synthetic secrets in its body, cookies, non-allowlisted headers, and URL path. The harness checks serialized caller/stream output and the supplemental transport error projection for those sentinels. Only a syntactically valid, bounded, allowlisted request-ID header may cross the future Provider Failure boundary. The request-capture projection uses a positive header allowlist and never records authorization or account headers, so the harness itself does not become a credential recorder.

## Test organization

The intended production-tree test layout is:

```text
desktop/gateway/src/server.rs
desktop/gateway/src/server/codex_acceptance.rs
```

`server.rs` declares the child module only for test plus `acceptance-build`. The child module owns the fake upstream, case runner, harness self-tests, and contract tests. Bounded synthetic fixtures stay inline unless readability proves a separate fixture file necessary.

Harness self-tests must be green before contract results are interpreted. They prove script ordering, extra/missing attempt detection, request capture, disconnect behavior, partial-stream behavior, and sentinel scanning. Contract tests encode the target behavior and are allowed to be red against the current implementation.

The focused command is:

```bash
cargo test --offline --manifest-path desktop/gateway/Cargo.toml \
  --features acceptance-build 'server::codex_acceptance::' -- --nocapture
```

The expected-red report under `docs/evidence/investigations/` separates:

- harness self-test results;
- already-green contract anchors;
- each failing target assertion;
- observed versus expected status, envelope, attempts, or mutation;
- the corresponding production responsibility for the later implementation ticket.

## Completion and handoff

This prototype ticket is complete only when:

1. The feature-gated harness compiles.
2. All harness self-tests pass.
3. The focused contract run is deterministic and its current failures are precise rather than incidental.
4. The evidence report maps every red assertion to the approved behavior matrix.
5. No production behavior, real credential, or external endpoint was used.
6. The reviewer gives a live verdict that the attempt behavior and expected outputs are judgeable.

An overall red contract run is expected and is not itself a harness defect. The later production implementation adopts these target tests and turns them green without relaxing attempt counts, replay barriers, schema, or redaction assertions. The fake upstream remains test-only and can be simplified or removed after equivalent production acceptance coverage exists.
