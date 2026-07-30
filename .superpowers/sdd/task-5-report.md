# Task 5 report: OpenAI Chat API-key routes through shared attempt sequence

## Status

Implemented and verified. OpenAI Chat routes now use the shared API-key attempt sequence for Qwen, custom OpenAI Chat, Gemini, Grok, and OpenCode Go Chat. OpenAI Responses remains on the pre-existing path for Task 6.

## RED evidence

Command:

```bash
cargo test --offline --manifest-path desktop/gateway/Cargo.toml \
  --features acceptance-build 'server::api_key_acceptance::api_contract_openai_chat_' \
  -- --nocapture --test-threads=1
```

Result before production changes: failed as expected, 0 passed / 5 failed. Failures showed the intended missing behavior:

- retry case observed one POST instead of two;
- malformed opened response and success cases emitted zero final diagnostics;
- failure shape still reflected the old one-shot path;
- stream replay success returned 502 instead of downstream SSE success.

## GREEN evidence

Focused Task 5 acceptance after implementation and formatting:

```bash
cargo test --offline --manifest-path desktop/gateway/Cargo.toml \
  --features acceptance-build 'server::api_key_acceptance::api_contract_openai_chat_' \
  -- --nocapture --test-threads=1
```

Result: passed, 5 passed / 0 failed. Evidence from diagnostics:

- OpenAI custom 409 retry completed with `posts:2`, `delays_ms:[500]`.
- Qwen 429 retry completed with `posts:2`, `delays_ms:[500]`.
- Gemini 500 exhaustion failed with `posts:3`, `delays_ms:[500,1000]`, `mapped_status:502`.
- Permanent/auth/quota/redirect cases were terminal with `posts:1`; quota remained `retryable:false`; redirect mapped terminal `307`.
- Malformed opened success failed with `posts:1`, no replay.
- Stream local SSE replay completed with `posts:1`; both downstream delivery closures produced exactly one `cancelled` diagnostic and no upstream retry.

Compatibility/regression commands:

```bash
cargo test --offline --manifest-path desktop/gateway/Cargo.toml openai_chat::tests -- --test-threads=1
cargo test --offline --manifest-path desktop/gateway/Cargo.toml \
  config::tests::endpoint_join_policies_cover_full_urls_xai_gemini_and_opencode -- --test-threads=1
cargo test --offline --manifest-path desktop/gateway/Cargo.toml api_key_attempt::tests -- --test-threads=1
cargo test --offline --manifest-path desktop/gateway/Cargo.toml provider_failure::tests -- --test-threads=1
cargo test --offline --manifest-path desktop/gateway/Cargo.toml \
  --features acceptance-build 'server::api_key_acceptance::api_contract_anthropic_' \
  -- --nocapture --test-threads=1
cargo clippy --offline --manifest-path desktop/gateway/Cargo.toml --all-targets -- -D warnings
git diff --check
```

Results:

- `openai_chat::tests`: 11 passed / 0 failed.
- endpoint join regression: 1 passed / 0 failed.
- `api_key_attempt::tests`: 5 passed / 0 failed.
- `provider_failure::tests`: 17 passed / 0 failed.
- Anthropic API-key acceptance: 6 passed / 0 failed.
- clippy: passed warning-free.
- `git diff --check`: clean.

## Files changed

- `desktop/gateway/src/server.rs`
  - Added `handle_openai_chat_api_key`.
  - Routes Qwen/custom OpenAI Chat providers through `ApiKeyAttemptSequence`.
  - Serializes the transformed OpenAI Chat request once before the shared sequence.
  - Marks response-open via the shared sequence before body read/conversion.
  - Converts opened OpenAI Chat success bodies with existing `openai_to_anthropic`.
  - Replays local SSE downstream-only for caller `stream=true`; delivery failures finalize as cancelled without upstream replay.
  - Leaves `openai-responses` on the existing path for Task 6.

- `desktop/gateway/src/server/api_key_acceptance.rs`
  - Added guarded real-handler OpenAI Chat fixture routes for Qwen, custom OpenAI Chat, Gemini, Grok, and OpenCode Go Chat.
  - Added route-aware path/auth/body assertions.
  - Added literal success fixtures and retry/exhaustion/terminal/malformed/delivery tests.
  - Added RST-style downstream close fixture for deterministic delivery cancellation classification.

- `desktop/gateway/src/provider_failure.rs`
  - Added explicit terminal 3xx projection so the specified OpenAI Chat redirect acceptance case returns/matches upstream `307` while remaining non-retryable protocol failure.

## Retry/body/delivery evidence

- Retried requests compare captured body bytes against the first request, so the fixture fails if serialization changes between attempts.
- Captured OpenAI Chat requests assert `/v1/chat/completions`, bearer auth only, target model mapping, user message role/content, and no API key leakage in body bytes.
- Stream callers still use one upstream OpenAI Chat JSON response and local downstream SSE replay only.
- Delivery cancellation tests assert one upstream POST, no delays, and exactly one final `cancelled` diagnostic.

## Self-review

- Scope is limited to OpenAI Chat API-key routes plus the redirect status projection needed by the Task 5 acceptance requirement.
- OpenAI Responses intentionally remains unchanged.
- No live keys/providers or reserved ports were used; fixtures bind ephemeral loopback ports excluding the reserved list.
- No fallback or Safe Repair path was added.
- Existing translation/signing/mapping functions are reused.
- Literal success bytes are asserted for all new success routes.

## Concerns

- The task brief listed only `server.rs` and `api_key_acceptance.rs`, but the requested redirect assertion (`307` terminal) required a small shared mapper update in `provider_failure.rs`; existing provider-failure tests still pass.
- The final-flush delivery fixture uses an early RST close at replay `ping` for deterministic cancellation on loopback, because closing after `message_stop` can be fully buffered before the handler observes the close.
