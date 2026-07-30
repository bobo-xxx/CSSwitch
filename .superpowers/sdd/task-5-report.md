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

## Review fix follow-up

Commit subject: `fix: scope OpenAI Chat review regressions`

### RED evidence

Redirect domain RED:

```bash
cargo test --offline --manifest-path desktop/gateway/Cargo.toml \
  provider_failure::tests::redirect_projection_is_scoped_to_openai_chat -- --test-threads=1
```

Result before fix: failed, 0 passed / 1 failed. Evidence: Anthropic `RouteMode::AnthropicMessages` 307 returned caller status `307`; expected preserved pre-Task5 shared behavior was `502` with `upstream_status:307`.

Final-flush RED:

```bash
cargo test --offline --manifest-path desktop/gateway/Cargo.toml \
  --features acceptance-build \
  'server::api_key_acceptance::api_contract_openai_chat_stream_replay_delivery_controls_final_outcome' \
  -- --nocapture --test-threads=1
```

Result before fix: failed, 0 passed / 1 failed. Evidence: the delivery fixture emitted cancellation but recorded no exact delivery evidence, proving the old early-close/ping shortcut did not establish chunk-write vs terminal-final-flush behavior.

### GREEN evidence

Focused GREEN:

```bash
cargo test --offline --manifest-path desktop/gateway/Cargo.toml \
  provider_failure::tests::redirect_projection_is_scoped_to_openai_chat -- --test-threads=1
cargo test --offline --manifest-path desktop/gateway/Cargo.toml \
  --features acceptance-build \
  'server::api_key_acceptance::api_contract_openai_chat_stream_replay_delivery_controls_final_outcome' \
  -- --nocapture --test-threads=1
```

Results: redirect focused test passed, 1 passed / 0 failed; delivery focused test passed, 1 passed / 0 failed. Delivery diagnostics were: success replay `completed` with `posts:1`; chunk-write failure `cancelled` with `posts:1`; final-flush failure `cancelled` with `posts:1`.

Full required verification:

```bash
cargo test --offline --manifest-path desktop/gateway/Cargo.toml \
  --features acceptance-build 'server::api_key_acceptance::api_contract_openai_chat_' \
  -- --nocapture --test-threads=1
cargo test --offline --manifest-path desktop/gateway/Cargo.toml provider_failure::tests -- --test-threads=1
cargo test --offline --manifest-path desktop/gateway/Cargo.toml openai_chat::tests -- --test-threads=1
cargo test --offline --manifest-path desktop/gateway/Cargo.toml \
  --features acceptance-build 'server::api_key_acceptance::api_contract_anthropic_' \
  -- --nocapture --test-threads=1
cargo test --offline --manifest-path desktop/gateway/Cargo.toml \
  config::tests::endpoint_join_policies_cover_full_urls_xai_gemini_and_opencode -- --test-threads=1
cargo fmt --check --manifest-path desktop/gateway/Cargo.toml
cargo clippy --offline --manifest-path desktop/gateway/Cargo.toml --all-targets -- -D warnings
git diff --check
```

Results:

- OpenAI Chat acceptance: 5 passed / 0 failed.
- `provider_failure::tests`: 18 passed / 0 failed.
- `openai_chat::tests`: 11 passed / 0 failed.
- Anthropic API-key acceptance: 6 passed / 0 failed.
- endpoint join regression: 1 passed / 0 failed.
- `cargo fmt --check`: passed.
- clippy: passed warning-free.
- `git diff --check`: clean.

### Files changed by review fix

- `desktop/gateway/src/provider_failure.rs`
  - Scoped 3xx caller-status projection to `RouteMode::OpenaiChat`; Anthropic/shared routes now retain generic 502 projection for redirects.
- `desktop/gateway/src/provider_failure/tests.rs`
  - Added route-domain redirect regression proving Anthropic 307 stays caller 502 while OpenAI Chat 307 is caller 307.
- `desktop/gateway/src/server.rs`
  - Extracted OpenAI Chat downstream delivery into `deliver_openai_chat_response<W: Write>`.
  - The real handler finalizes completed/cancelled from this delivery result; the acceptance hook only supplies a writer, not an alternate finalizer.
- `desktop/gateway/src/server/api_key_acceptance.rs`
  - Replaced early RST/ping final-flush shortcut with a scripted writer.
  - Chunk-write fixture fails on a replay event write before terminal chunk.
  - Final-flush fixture accepts all replay chunks and the terminal `0\r\n\r\n` write, then fails specifically on the final flush.
  - Assertions require exact evidence: chunk error vs terminal write seen vs final flush error.

### Updated self-review

- Redirect behavior is now scoped to OpenAI Chat only; OpenAI Responses is not pre-implemented.
- FinalFlush evidence now uses the production delivery function and real handler finalizer with a deterministic writer.
- The previous early-RST/ping concern is resolved and superseded by the scripted final-flush evidence above.
