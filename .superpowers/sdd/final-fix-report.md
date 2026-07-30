# Ticket 07 Final Fix Report

## Result

Added deterministic real-handler conformance for pre-response connect and timeout exhaustion across representative Anthropic Messages, OpenAI Chat, and OpenAI Responses routes. Removed the brittle `include_str!("../server.rs")` source-shape publication guard; the existing checked-delivery behavioral tests remain the enforcement.

## TDD record

- RED: `cargo test --offline --manifest-path desktop/gateway/Cargo.toml --all-features api_contract_real_transport_connect_and_timeout_exhaustion_is_closed -- --exact` failed to compile with four expected missing-fixture errors (`NetworkFixture` and `assert_real_network_exhaustion`). The `--exact` selector itself matched zero tests, so subsequent commands use the unqualified substring selector.
- GREEN: `cargo test --offline --manifest-path desktop/gateway/Cargo.toml --all-features api_contract_real_transport_connect_and_timeout_exhaustion_is_closed -- --nocapture` passed 1/1 (six route/failure cases).
- Sensitivity: temporarily mutated production timeout mapping from 504 to 502; `cargo test --offline --manifest-path desktop/gateway/Cargo.toml --all-features api_contract_real_transport_connect_and_timeout_exhaustion_is_closed` failed 0/1 at the caller-status assertion (`502 != 504`). The mutation was reverted and the focused test returned to 1/1 green.

## Fixture mechanics and assertions

- Uses `handle_messages`, the production API-key attempt runner, and the real `messages::post_once`/reqwest classification path.
- Connect exhaustion targets a freshly allocated, released, non-reserved loopback address; the final diagnostic proves three production POST invocations. Timeout exhaustion uses a loopback listener that captures three real POST requests and deliberately never opens a response.
- Timeout tests override only the fixture's cloned managed runtime contract to 20 ms connect/total/read-idle bounds. Retry waits remain the acceptance-build recording seam, recording `[500, 1000]` without sleeping.
- All loopback allocation goes through the existing reserved-port rejection for `2999`, `8765`, `9002`, `9003`, `11434`, and `11535`.
- Each of the six cases asserts caller 502 (connect) or 504 (timeout), three posts, delays `[500, 1000]`, one `failed` diagnostic, closed `network` reason/mapped status/retryability, zero repairs, no completed/cancelled outcome, and no raw sentinel leakage. Timeout cases additionally assert exact POST path and byte-identical translated request bodies.

## Verification

- API-key acceptance: 22/22 green.
- `provider_failure::tests`: 19/19 green.
- `messages::tests`: 13/13 green.
- `api_key_attempt::tests`: 5/5 green.
- Codex harness: exactly 8/8 green.
- Codex contract: exactly 27/27 green.
- `cargo fmt --manifest-path desktop/gateway/Cargo.toml -- --check`: green.
- `cargo clippy --offline --manifest-path desktop/gateway/Cargo.toml --all-targets --all-features -- -D warnings`: green, warning-free.
- `git diff --check`: green.
- A monolithic all-target/all-feature run could not complete in this managed sandbox: after 265/391 passes, 126 existing network tests failed because loopback `bind` returned OS `EPERM`, both at default concurrency and with `--test-threads=1`. The required focused network and compatibility suites above ran successfully in separate processes.

## Files and triage

- Modified `desktop/gateway/src/server/api_key_acceptance.rs` only for fixture/test behavior.
- Added this report. No production runtime, catalog, evidence, deployment, install, or service state changed.
- Removed the source-text guard without replacement; behavioral delivery fault-injection coverage remains green.
- Deferred as reviewed debt: catalog-versus-literal retry-policy duplication and residual OpenAI Chat/Responses lifecycle duplication. Neither is necessary for this focused publication fix; architecture churn would increase risk.

## Commit and self-review

Commit subject: `test: cover API-key network exhaustion in real handlers`. The full SHA is returned in the handoff after the commit is created. Self-review found no production behavior change, no live provider/key access, bounded fixture threads, reserved-port protection, exact final-state assertions, and no warning or whitespace defect.
