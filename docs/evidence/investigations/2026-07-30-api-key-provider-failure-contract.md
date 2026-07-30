# API-Key Provider Failure Contract deterministic evidence

Date: 2026-07-30

## Scope and evidence boundary

Ticket 07 extends the shared Provider Failure Contract to the existing API-key Anthropic Messages, OpenAI Chat, and OpenAI Responses routes. This evidence binds the deterministic implementation to revision `7842d31d71c2334c67db6fe5ee37c5ce762a2a05` and records only fresh offline source/unit and real-handler loopback-fixture verification performed from that revision.

This run did not use a live API key, live provider, external endpoint, installed profile, installed runtime, proxy, or service. It did not install, replace, stop, restart, or reconfigure the Gateway, Claude Science, sandbox, or proxy. Fixtures used ephemeral loopback listeners and explicitly rejected reserved runtime ports `2999`, `8765`, `9002`, `9003`, `11434`, and `11535`.

## Revision and environment

The literal pre-evidence commands and outputs were:

```text
$ git status --short
[no output]
exit 0

$ git rev-parse HEAD
7842d31d71c2334c67db6fe5ee37c5ce762a2a05
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

## Provider and route policy

Every catalog contract with `auth_mode: api_key` is validated against an explicit closed policy. All rows permit at most three POSTs, use fallback delays `[500, 1000]` ms, cap unsigned-decimal `Retry-After` delta seconds at `60`, and retry only pre-response `connect` and `timeout` network observations. Duplicate/unknown policy tokens, missing API-key policy, policy attached to Codex, and incompatible adapter-only resolution fail closed.

| Serialized provider | Route | Catalog contract and template coverage | Retryable pre-open HTTP statuses | Explicit exclusions |
|---|---|---|---|---|
| `deepseek` | `anthropic_messages` | `deepseek-native` (`deepseek`) | `408`, `429`, `500..=599` | `409` and other permanent 4xx |
| `relay` | `anthropic_messages` | `anthropic-relay` (`glm`, `xiaomi`, `siliconflow`, `minimax`, `openrouter`); `kimi-anthropic-relay` (`kimi`); `custom-anthropic` (`custom`); `opencode-go-anthropic` | `408`, `429`, `500..=599` | `409` and other permanent 4xx |
| `qwen` | `openai_chat` | `qwen-native` (`qwen`) | `408`, `409`, `429`, `500..=599` | other permanent 4xx and redirects |
| `openai_custom` | `openai_chat` | `custom-openai-chat` (`custom-openai`, `custom`); `opencode-go-openai-chat`; `grok-openai-chat`; `gemini-openai-chat` | `408`, `409`, `429`, `500..=599` | other permanent 4xx and redirects |
| `openai_responses` | `openai_responses` | `custom-openai-responses` (`custom-openai-responses`, `custom`) | `408`, `409`, `429`, `500..=599` | other permanent 4xx and redirects |

For every route, exact closed `insufficient_quota` evidence overrides status `429` and is terminal. Unknown/absent 429 codes follow the route's explicit rate policy. API-key contexts structurally disable Safe Repair; the OpenAI Responses `unsupported_value` plus `tool_choice` case makes one POST, records `repairs: 0`, and terminates. Codex keeps its separate policy and serialization contract.

## Literal verification commands and counts

Formatting and diff integrity:

```text
$ cargo fmt --check --manifest-path desktop/gateway/Cargo.toml
[no output]
exit 0

$ git diff --check 30ec9e2d546dea72f6de7fd843a373657d24fa0b..HEAD
[no output]
exit 0
```

Focused policy, domain, transport, runner, and API-key acceptance gates:

```text
$ cargo test --offline --manifest-path desktop/gateway/Cargo.toml provider_contracts::tests -- --test-threads=1
lib: 6 passed; 0 failed; 0 ignored; 0 measured; 325 filtered out
main: 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
codex_auth_cli: 0 passed; 0 failed; 0 ignored; 0 measured; 3 filtered out
exit 0

$ cargo test --offline --manifest-path desktop/gateway/Cargo.toml provider_failure::tests -- --test-threads=1
lib: 19 passed; 0 failed; 0 ignored; 0 measured; 312 filtered out
main: 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
codex_auth_cli: 0 passed; 0 failed; 0 ignored; 0 measured; 3 filtered out
exit 0

$ cargo test --offline --manifest-path desktop/gateway/Cargo.toml messages::tests -- --test-threads=1
lib: 13 passed; 0 failed; 0 ignored; 0 measured; 318 filtered out
main: 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
codex_auth_cli: 0 passed; 0 failed; 0 ignored; 0 measured; 3 filtered out
exit 0

$ cargo test --offline --manifest-path desktop/gateway/Cargo.toml api_key_attempt::tests -- --test-threads=1
lib: 5 passed; 0 failed; 0 ignored; 0 measured; 326 filtered out
main: 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
codex_auth_cli: 0 passed; 0 failed; 0 ignored; 0 measured; 3 filtered out
exit 0

$ cargo test --offline --manifest-path desktop/gateway/Cargo.toml --features acceptance-build 'server::api_key_acceptance::api_' -- --nocapture --test-threads=1
lib: 22 passed; 0 failed; 0 ignored; 0 measured; 369 filtered out
main: 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
codex_auth_cli: 0 passed; 0 failed; 0 ignored; 0 measured; 3 filtered out
exit 0
```

Codex invariants and protocol compatibility:

```text
$ cargo test --offline --manifest-path desktop/gateway/Cargo.toml --features acceptance-build 'server::codex_acceptance::harness_' -- --test-threads=1
lib: 8 passed; 0 failed; 0 ignored; 0 measured; 383 filtered out
main: 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
codex_auth_cli: 0 passed; 0 failed; 0 ignored; 0 measured; 3 filtered out
exit 0

$ cargo test --offline --manifest-path desktop/gateway/Cargo.toml --features acceptance-build 'server::codex_acceptance::contract_' -- --test-threads=1
lib: 27 passed; 0 failed; 0 ignored; 0 measured; 364 filtered out
main: 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
codex_auth_cli: 0 passed; 0 failed; 0 ignored; 0 measured; 3 filtered out
exit 0

$ cargo test --offline --manifest-path desktop/gateway/Cargo.toml anthropic_compat::tests -- --test-threads=1
lib: 15 passed; 0 failed; 0 ignored; 0 measured; 316 filtered out
main: 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
codex_auth_cli: 0 passed; 0 failed; 0 ignored; 0 measured; 3 filtered out
exit 0

$ cargo test --offline --manifest-path desktop/gateway/Cargo.toml openai_chat::tests -- --test-threads=1
lib: 11 passed; 0 failed; 0 ignored; 0 measured; 320 filtered out
main: 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
codex_auth_cli: 0 passed; 0 failed; 0 ignored; 0 measured; 3 filtered out
exit 0

$ cargo test --offline --manifest-path desktop/gateway/Cargo.toml openai_responses::tests -- --test-threads=1
lib: 6 passed; 0 failed; 0 ignored; 0 measured; 325 filtered out
main: 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
codex_auth_cli: 0 passed; 0 failed; 0 ignored; 0 measured; 3 filtered out
exit 0
```

Complete offline tests and lint:

```text
$ cargo test --offline --manifest-path desktop/gateway/Cargo.toml --all-features -- --test-threads=1
lib: 391 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
main: 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
codex_auth_cli: 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
doc tests: 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
exit 0

$ cargo clippy --offline --manifest-path desktop/gateway/Cargo.toml --all-targets --all-features -- -D warnings
Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.13s
exit 0; no warnings
```

## Exact attempt, delay, and body assertions

The acceptance harness requires every captured request to be `POST` to the route's exact path, with zero unexpected POSTs, methods, or rejected requests. For every attempt after the first, it directly asserts `request.body == first.body`; retry-only translations are therefore byte-identical, not merely JSON-equivalent.

| Route fixture | Scripted observation | Exact asserted result |
|---|---|---|
| Relay Anthropic | `429` then success, no valid `Retry-After` | 2 POSTs; delays `[500]`; byte-identical bodies; `completed` |
| DeepSeek Anthropic | `429` then success, `Retry-After: 90` | 2 POSTs; delays `[60000]`; byte-identical bodies; `completed` |
| DeepSeek Anthropic | three `503` responses | 3 POSTs; delays `[500, 1000]`; byte-identical bodies; terminal `failed` |
| OpenAI custom Chat | `409` then success | 2 POSTs; delays `[500]`; byte-identical bodies; `completed` |
| Qwen Chat | `429` then success | 2 POSTs; delays `[500]`; byte-identical bodies; `completed` |
| Gemini Chat | three `500` responses | 3 POSTs; delays `[500, 1000]`; byte-identical bodies; terminal `failed` |
| OpenAI Responses | `409` then success | 2 POSTs; delays `[500]`; byte-identical bodies; `completed` |
| OpenAI Responses | `429` then success, `Retry-After: 90` | 2 POSTs; delays `[60000]`; byte-identical bodies; `completed` |
| OpenAI Responses | three `502` responses | 3 POSTs; delays `[500, 1000]`; byte-identical bodies; terminal `failed` |

Anthropic `400`, `401`, `403`, `409`, `422`, and exact quota `429` cases assert one POST and no delay. OpenAI Chat and OpenAI Responses `400`, `401`, `403`, `422`, `307`, and exact quota `429` cases assert one POST and no delay. Existing literal success fixtures remain unchanged for Kimi non-stream/stream filtering, DeepSeek DSML non-stream/stream rewriting, Qwen, custom OpenAI Chat, Gemini, Grok, OpenCode Go, OpenAI Chat local SSE replay, and OpenAI Responses metadata mapping/local SSE replay.

## Response-open and cancellation barriers

- The runner accepts retry observations only before a successful response opens. Malformed JSON after 2xx, incomplete body, and partial SSE fixtures each assert exactly one POST, no delay, no replay, and one final `failed` diagnostic.
- Downstream final-body and final-flush failures are exercised across Relay Anthropic, Qwen Chat, and OpenAI Responses. OpenAI Chat also exercises content-chunk, terminal-chunk, and final-flush failures. Each asserts exactly one final `cancelled` diagnostic and explicitly excludes `failed` and `completed` outcomes.
- Retry-wait cancellation is exercised on all three route families. Each fixture records delay vector `[500]`, makes one POST, leaves the scripted success step unconsumed, and emits exactly one `cancelled` diagnostic before a second POST.
- Successful opened responses finalize only after checked downstream delivery. The acceptance suite verifies the same finalizer is used by multiple production branches; it does not use a separate acceptance-only delivery path.

## Closed caller and diagnostic schemas

Caller failures preserve the Anthropic-compatible top-level shape with exactly `type: "error"` and an `error` object. The required closed `error` fields are stable `type` and `message`, `provider`, `route`, `failure_class`, `retryable`, allowlisted `correlation_id`, and stable `recovery`. Only `upstream_status`, positive-allowlisted `request_id`, and capped `retry_after_seconds` may be added when known. Caller request and correlation IDs accept only `[A-Za-z0-9._:-]{1,256}`; unknown optional facts are omitted.

API-key diagnostics have exactly these top-level keys:

```text
schema_version, provider, route, correlation_id, outcome, posts, repairs, reason
```

For completion, `reason` is exactly `{"kind":"completed","delay_count":0}`. For a three-POST `503` exhaustion it is exactly `{"kind":"failed","delay_count":2,"mapped_status":502,"upstream_status":503,"failure_class":"transient","retryable":true}`. Cancelled/completed reasons omit failure-only fields. Diagnostics never contain request IDs, raw bodies, prompts, credentials, account identifiers, cookies, headers, URLs, provider error messages, or arbitrary caller strings.

Cross-route fixtures inject the sentinels `fixture-api-key`, `secret upstream text`, `private.example`, `Req-secret-cross-route`, and route-specific request IDs. Both caller output and serialized diagnostics assert that every sentinel is absent. The one-POST transport projection separately bounds retained HTTP failure-body semantics and exposes only closed rate/code/parameter facts plus allowlisted retry/request-ID metadata; raw upstream text does not enter the controller.

## Limitations and review note

- This is deterministic Linux-headless evidence, not live-provider or installed-runtime evidence. No transient failure, quota, authentication rejection, disconnect, or repair condition was induced against a real provider.
- No macOS runtime was available. Shared Rust compilation/tests cover source compatibility, but real macOS runtime execution remains pending.
- One acceptance regression uses an `include_str!` source-shape guard to prevent reintroducing a test-only delivery bypass and dead finalizer. It passed here, but remains a brittle Minor review concern rather than runtime behavioral proof; the real-handler delivery/cancellation fixtures provide the behavioral evidence.
- Publication and independent whole-branch review are controller-owned follow-up work. This evidence task does not push or deploy the branch.
