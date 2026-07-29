# Codex Provider Failure Contract Implementation Evidence

Date: 2026-07-30

## Scope

Ticket 05 implements the Provider Failure Contract through the real Codex handler and single-POST transport. This verification used only deterministic loopback fixtures: no live credential, installed profile, proxy mutation, external endpoint, installed runtime, or running service was used. No fallback or generic adapter was exercised.

## Revision and environment

The parser-bound-remediated implementation was tested from a clean worktree at revision `2b3a1f4ce77dccd0c14e44f89ab72a588eb14581` before this evidence-only commit. The literal revision and environment commands and outputs were:

```text
$ git status --short
[no output]
exit 0

$ git rev-parse HEAD
2b3a1f4ce77dccd0c14e44f89ab72a588eb14581
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

The parser-bound remediation was committed before the immutable gate run:

```text
$ git add desktop/gateway/src/codex_transport.rs
exit 0

$ git commit -m "fix: bound Codex failure parser memory"
[feature/codex-provider-failure 2b3a1f4] fix: bound Codex failure parser memory
1 file changed, 157 insertions(+), 43 deletions(-)
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
lib: 12 passed; 0 failed; 0 ignored; 0 measured; 293 filtered out
main: 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
codex_auth_cli: 0 passed; 0 failed; 0 ignored; 0 measured; 3 filtered out
exit 0
```

```text
$ cargo test --offline --manifest-path desktop/gateway/Cargo.toml codex_transport::tests -- --test-threads=1
lib: 15 passed; 0 failed; 0 ignored; 0 measured; 290 filtered out
main: 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
codex_auth_cli: 0 passed; 0 failed; 0 ignored; 0 measured; 3 filtered out
exit 0
```

```text
$ cargo test --offline --manifest-path desktop/gateway/Cargo.toml --features acceptance-build 'server::codex_acceptance::harness_' -- --nocapture --test-threads=1
lib: 8 passed; 0 failed; 0 ignored; 0 measured; 334 filtered out
main: 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
codex_auth_cli: 0 passed; 0 failed; 0 ignored; 0 measured; 3 filtered out
exit 0
```

```text
$ cargo test --offline --manifest-path desktop/gateway/Cargo.toml --features acceptance-build 'server::codex_acceptance::contract_' -- --nocapture --test-threads=1
lib: 27 passed; 0 failed; 0 ignored; 0 measured; 315 filtered out
main: 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
codex_auth_cli: 0 passed; 0 failed; 0 ignored; 0 measured; 3 filtered out
exit 0
```

```text
$ cargo test --offline --manifest-path desktop/gateway/Cargo.toml codex_ -- --test-threads=1
lib: 158 passed; 0 failed; 0 ignored; 0 measured; 147 filtered out
main: 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
codex_auth_cli: 1 passed; 0 failed; 0 ignored; 0 measured; 2 filtered out
exit 0
```

```text
$ cargo test --offline --manifest-path desktop/gateway/Cargo.toml --all-features -- --test-threads=1
lib: 342 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
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
Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.13s
exit 0; no warnings
```

The accepted harness passes 8/8, the accepted Provider Failure Contract passes 27/27, and the non-contract attempt integrations pass 2/2. The pure controller suite passes 12/12 and the transport suite passes 15/15.

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

## Redaction boundary

Caller envelopes preserve legacy Anthropic fields and add only closed structured metadata. Diagnostics contain only outcome, provider, route, correlation ID, POST/repair/delay counts, and failure-only mapped status, optional upstream status, class, and retryability. Request IDs are positive-allowlisted for caller output but excluded from diagnostics. Raw bodies, prompts, credentials, account identifiers, cookies, headers, and private URLs cannot enter the controller or diagnostic types.

Failed-body projection now admits body collection only when reqwest exposes a validated `Content-Length` and the entire declared body fits a conservative memory budget below 16 KiB. The calculation reserves three input-sized regions plus fixed typed storage: one for the actual retained `Vec` capacity and two for serde_json's escaped-string scratch contents and amortized capacity slack. The collector rechecks the actual allocated input capacity before parsing. serde_json's private scratch capacity is not directly exposed for measurement, so the bound deliberately uses this conservative reserve under the pinned Vec-backed parser rather than claiming runtime observation of that scratch allocation.

The three projected strings are visited directly into closed enums; no decoded `String` or `Cow` survives a visitor. Unknown-length, invalid-length, and over-budget bodies are not parsed or projected by CSSwitch and conservatively produce no body-derived facts. Focused regressions prove the admission arithmetic, exercise a late JSON escape at the exact admitted limit, and prove that a valid repair-shaped body one byte over the limit is rejected before its capability facts can be projected.

## Explicit limitations

This evidence does not verify an installed runtime or live Codex subscription; Ticket 06 owns that work. It does not add the shared API-key adapter; Ticket 07 owns that work. No macOS runtime was available, so macOS source compatibility is covered by the shared Rust build/test surface rather than a real macOS run.

Reqwest exposes already-materialized `Bytes` frames and does not provide a caller-sized read API for `Response::chunk()`. CSSwitch therefore enforces the strict bound across its retained input, pinned parser scratch allowance, and fixed projection storage; it does not claim to observe or cap private reqwest/hyper socket buffers or allocations that may occur while response headers are produced. The focused oversized-frame test proves the collector retains zero bytes from a frame larger than 16 KiB and that an oversized declared loopback response yields no parsed capability facts.
