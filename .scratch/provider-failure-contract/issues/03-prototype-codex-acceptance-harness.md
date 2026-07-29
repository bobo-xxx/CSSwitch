# Prototype the Codex acceptance harness

Type: prototype
Status: resolved
Blocked by: 01

## Question

What is the smallest deterministic fake-upstream or recorded-fixture harness that reproduces the observed Codex rejection and lets a reviewer judge the required behavior? Produce a disposable test skeleton and expected assertions for permanent 4xx, 401/403, 429 with `Retry-After`, 5xx/network retry exhaustion, Responses Lite capability normalization, one safe replay, schema compatibility, and diagnostic redaction.

## Resolution

Use the feature-gated deterministic loopback harness on branch `prototype/codex-acceptance-harness` at final revision `fe57d97`. The tested code revision is `ebb46b3`; the following documentation-only revision publishes the accepted evidence under `docs/evidence/investigations/2026-07-29-codex-acceptance-harness-expected-red.md`.

The focused harness command is:

```bash
cargo test --offline --manifest-path desktop/gateway/Cargo.toml \
  --features acceptance-build 'server::codex_acceptance::harness_' -- --nocapture
```

It passes 8/8 tests. The matching `contract_` command reaches 4 green anchors and 23 authentic expected-red Ticket 05 assertions; the existing focused Codex suite passes 16/16. The harness drives the real Codex handler and transport, proves exact POST/path counts, bounded capture, replay barriers, repair authorization, and sanitized caller/stream output without production changes.

Human verdict on 2026-07-29: tested and behaved as expected; the surface is accepted as sufficiently judgeable for Ticket 05.

Ticket 05 owns all production changes, including Provider Failure classification/metadata, bounded retry integration, closed Safe Repair, response-fact projection, handler-level structured diagnostics, and deterministic scheduler injection.
