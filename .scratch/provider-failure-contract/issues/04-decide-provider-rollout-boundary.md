# Decide the provider rollout boundary

Type: grilling
Status: resolved
Blocked by: 02, 03

## Question

After reviewing the chosen contract seam and Codex acceptance harness, which gateway transports and provider families must adopt the Provider Failure Contract in this effort, and which compatibility work should remain a follow-up? Preserve the approved provider-general architecture and Codex-first verification sequence while avoiding an unbounded rewrite.

## Answer

Use a two-stage, two-adapter rollout.

Stage 1 is the Codex-first slice: implement the full Provider Failure Contract through `CodexTransport`, then verify it with the deterministic acceptance harness and the already configured Codex subscription on Linux headless.

Stage 2 extends the base contract through the existing shared `messages.rs` inference adapter to every current API-key Provider Route: DeepSeek, Qwen, Anthropic relays including Kimi, OpenAI Chat routes including Gemini/Grok/custom, and custom OpenAI Responses. This gives the seam two real adapters without duplicating the controller across provider templates.

All selected routes receive backward-compatible failure envelopes, sanitized classification, redaction, cancellation finality, and the no-replay-after-response barrier. Each Provider Route supplies an explicit retry policy; the Codex retry set is never enabled globally. Permanent 4xx and 401/403 remain terminal, and only route-approved transient classes may consume the bounded attempt budget. Safe Repair and capability-field removal remain Codex-only unless a future route has its own documented and fixture-proven semantics.

Non-Codex acceptance uses deterministic conformance fixtures for Anthropic Messages, OpenAI Chat, and OpenAI Responses. Live verification is required only for Codex; no other provider credentials need to be used or inspected.

This rollout excludes model-discovery requests, login and authentication flows, UI work, automatic fallback, new provider families, and live testing of every API-key provider. macOS source compatibility remains required, while unavailable macOS runtime proof may be recorded as an explicit limitation.

Human verdict on 2026-07-29: accepted as the complete rollout boundary.
