# Codex Provider Failure Contract Implementation Evidence

Date: 2026-07-30

## Scope

Ticket 05 implements the Provider Failure Contract through the real Codex handler and single-POST transport. This verification used only deterministic loopback fixtures: no live credential, installed profile, proxy mutation, external endpoint, installed runtime, or running service was used. No fallback or generic adapter was exercised.

## Revision and environment

The structurally bounded parser, terminal-delivery implementation, and reserved-port-safe acceptance fixtures were tested from a clean worktree at revision `9591a869f7268b5ff6baee00eb1365b36c54a618` before this evidence-only commit. The literal revision and environment commands and outputs were:

```text
$ git status --short
[no output]
exit 0

$ git rev-parse HEAD
9591a869f7268b5ff6baee00eb1365b36c54a618
exit 0

$ rustc --version
rustc 1.97.0 (2d8144b78 2026-07-07)
exit 0

$ cargo --version
cargo 1.97.0 (c980f4866 2026-06-30)
exit 0

$ uname -a
Linux shengxin02 6.8.0-90-generic #91~22.04.1-Ubuntu SMP PREEMPT_DYNAMIC Thu Nov 20 15:20:45 UTC 2 x86_64 x86_64 x86_64 GNU/Linux
exit 0
```

The acceptance-fixture reserved-port remediation was committed before the immutable gate run:

```text
$ git add desktop/gateway/src/server/codex_acceptance.rs
exit 0

$ git commit -m "test: avoid reserved Science port in Codex fixtures"
[feature/codex-provider-failure 9591a86] test: avoid reserved Science port in Codex fixtures
1 file changed, 30 insertions(+), 2 deletions(-)
exit 0
```

## Verification results

Every required verification gate was run fresh against the tested revision. Literal commands, exit statuses, and Cargo counts follow.

```text
$ cargo fmt --check --manifest-path desktop/gateway/Cargo.toml
[no output]
exit 0

$ git diff --check linux-headless-oauth...HEAD
[no output]
exit 0
```

```text
$ cargo test --offline --manifest-path desktop/gateway/Cargo.toml provider_failure::tests -- --test-threads=1
lib: 12 passed; 0 failed; 0 ignored; 0 measured; 301 filtered out
main: 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
codex_auth_cli: 0 passed; 0 failed; 0 ignored; 0 measured; 3 filtered out
exit 0
```

```text
$ cargo test --offline --manifest-path desktop/gateway/Cargo.toml codex_transport::tests -- --test-threads=1
lib: 18 passed; 0 failed; 0 ignored; 0 measured; 295 filtered out
main: 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
codex_auth_cli: 0 passed; 0 failed; 0 ignored; 0 measured; 3 filtered out
exit 0
```

```text
$ cargo test --offline --manifest-path desktop/gateway/Cargo.toml server::tests::codex_failed_ -- --test-threads=1
lib: 5 passed; 0 failed; 0 ignored; 0 measured; 308 filtered out
main: 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
codex_auth_cli: 0 passed; 0 failed; 0 ignored; 0 measured; 3 filtered out
exit 0
```

```text
$ cargo test --offline --manifest-path desktop/gateway/Cargo.toml --features acceptance-build 'server::codex_acceptance::harness_' -- --nocapture --test-threads=1
lib: 8 passed; 0 failed; 0 ignored; 0 measured; 343 filtered out
main: 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
codex_auth_cli: 0 passed; 0 failed; 0 ignored; 0 measured; 3 filtered out
exit 0
```

```text
$ cargo test --offline --manifest-path desktop/gateway/Cargo.toml --features acceptance-build 'server::codex_acceptance::contract_' -- --nocapture --test-threads=1
lib: 27 passed; 0 failed; 0 ignored; 0 measured; 324 filtered out
main: 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
codex_auth_cli: 0 passed; 0 failed; 0 ignored; 0 measured; 3 filtered out
exit 0
```

```text
$ cargo test --offline --manifest-path desktop/gateway/Cargo.toml --features acceptance-build bind_loopback_avoids_reserved_science_port -- --nocapture --test-threads=1
lib: 1 passed; 0 failed; 0 ignored; 0 measured; 350 filtered out
main: 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
codex_auth_cli: 0 passed; 0 failed; 0 ignored; 0 measured; 3 filtered out
exit 0
```

```text
$ cargo test --offline --manifest-path desktop/gateway/Cargo.toml codex_ -- --test-threads=1
lib: 166 passed; 0 failed; 0 ignored; 0 measured; 147 filtered out
main: 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
codex_auth_cli: 1 passed; 0 failed; 0 ignored; 0 measured; 2 filtered out
exit 0
```

```text
$ cargo test --offline --manifest-path desktop/gateway/Cargo.toml --all-features -- --test-threads=1
lib: 351 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
main: 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
codex_auth_cli: 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
doc tests: 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
exit 0
```

The two non-contract attempt integration tests passed within the complete library suite:

```text
server::codex_acceptance::attempt_retry_wait_cancellation_stops_without_failure_output ... ok
server::codex_acceptance::attempt_wait_cancellation_emits_one_final_diagnostic_without_replay ... ok
```

```text
$ cargo clippy --offline --manifest-path desktop/gateway/Cargo.toml --all-targets --all-features -- -D warnings
Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.15s
exit 0; no warnings
```

The accepted harness passes 8/8, the accepted Provider Failure Contract passes 27/27, the reserved-port fixture regression passes 1/1, and the non-contract attempt integrations pass 2/2. The pure controller suite passes 12/12, the transport suite passes 18/18, and the terminal-delivery suite passes 5/5.

## Behavioral evidence

- Retry-only sequences make at most three POSTs.
- Missing `Retry-After` consumes 500 ms then 1,000 ms; valid delta seconds override a transition and are capped at 60 seconds.
- The recording runtime performs no sleep and captures exact delay vectors.
- Safe Repair is restricted to Responses Lite, absent/automatic caller choice, and exact `unsupported_value` plus `tool_choice`; it removes only top-level translated `tool_choice: "auto"` and makes at most two POSTs.
- Retry and repair budgets never combine.
- Permanent 4xx, authentication, authorization, and quota failures do not replay.
- No replay occurs after an upstream response opens or downstream bytes begin.
- Every started attempt sequence emits one final `completed`, `failed`, or `cancelled` diagnostic.
- Final diagnostic construction is checked and single-use: duplicate finalization and phase-incompatible outcomes return `FinalizationNotAuthorized` rather than overwriting terminal state or constructing a second diagnostic.
- Final diagnostics are emitted only after the terminal downstream delivery result is known. A failed final non-stream response, provider-failure response, terminal SSE error, stream terminator, or terminal flush produces exactly one `cancelled` diagnostic and cannot also produce `completed` or `failed`.
- Codex acceptance upstream and downstream loopback fixtures use one guarded binder that rejects the reserved Science port `8765` without changing accepted harness or contract membership.

## Redaction boundary

Caller envelopes preserve legacy Anthropic fields and add only closed structured metadata. Diagnostics contain only outcome, provider, route, correlation ID, POST/repair/delay counts, and failure-only mapped status, optional upstream status, class, and retryability. Request IDs are positive-allowlisted for caller output but excluded from diagnostics. Raw bodies, prompts, credentials, account identifiers, cookies, headers, and private URLs cannot enter the controller or diagnostic types.

Failed-body projection admits body collection only when reqwest exposes a validated `Content-Length` no greater than 16 KiB. CSSwitch retains admitted bytes in a fixed `[u8; 16384]` buffer; the collector has no `Vec`, `String`, `Box`, or other heap-owning field. Unknown-length and oversized bodies are not polled for projection and conservatively produce no body-derived facts.

Projection uses a safe recursive-descent scanner over a borrowed byte slice. Its state contains only that borrow and scalar cursor/field counters, returns only `Copy` closed facts, and performs no parser heap allocation. It validates JSON string escapes and surrogate pairs, compares decoded field/token values without retaining decoded strings, skips irrelevant data without retaining it, and rejects malformed, duplicate closed-field, over-depth, and over-field-limit inputs conservatively. Recursion is capped at 16 container levels and object fields at 128. Focused regressions cover a late escape at exactly 16 KiB, nested unknown objects/arrays, malformed JSON and Unicode escapes, fixed depth/field limits, an over-limit body, and the no-drop fixed/borrowed parser-state types.

## Explicit limitations

This evidence does not verify an installed runtime or live Codex subscription; Ticket 06 owns that work. It does not add the shared API-key adapter; Ticket 07 owns that work. No macOS runtime was available, so macOS source compatibility is covered by the shared Rust build/test surface rather than a real macOS run.

Reqwest exposes already-materialized `Bytes` frames and does not provide a caller-sized read API for `Response::chunk()`. The 16 KiB statement covers CSSwitch's own admitted failure-body buffer and allocation-free scanner/projection state; it does not claim to observe or cap private reqwest/hyper socket buffers or allocations. The focused oversized-frame test proves the CSSwitch collector retains zero bytes from a frame larger than the declared bound and that an oversized declared loopback response yields no parsed capability facts.
