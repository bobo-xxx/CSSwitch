# Extend the contract to the selected Provider Routes

Type: task
Status: resolved
Blocked by: 04, 06

## Question

Apply the proven Provider Failure Contract to the provider families selected by “Decide the provider rollout boundary,” preserving their protocol-specific semantics while centralizing shared retry, normalization, serialization, and redaction rules. Add conformance cases before each provider change.

## Resolution

Implemented, independently reviewed, and published the shared API-key Provider Failure Contract on branch `ticket07/provider-failure-routes` at evidence/head commit `b1ff00821c3ab110adaecd684abad3a4d442a844`. The exact tested implementation is `2b5dcb804e9d18079309db1d5974b626d39123f6`; commands, route assertions, closed schemas, redaction boundaries, and limitations are recorded in `docs/evidence/investigations/2026-07-30-api-key-provider-failure-contract.md` at the published evidence commit.

Every API-key Provider Route permits at most three pre-response POST attempts with fallback delays `[500, 1000]` ms, accepts only unsigned-decimal `Retry-After` delta seconds capped at 60 seconds, retries pre-response connect and timeout failures, and maps exhausted connect failures to `502` and timeouts to `504`. Anthropic Messages routes (`deepseek` and `relay`) retry `408`, `429`, and `500..=599`, while `409` and other permanent 4xx responses terminate. OpenAI Chat routes (`qwen` and `openai_custom`) and OpenAI Responses retry `408`, `409`, `429`, and `500..=599`; redirects and other permanent 4xx responses terminate. Exact `insufficient_quota` evidence overrides `429` and terminates on every route. API-key Safe Repair is disabled, response-open and downstream-delivery barriers prevent replay, and no provider or model fallback was added.

Fresh deterministic verification passed provider contracts 6/6, Provider Failure Contract 19/19, Messages 13/13, API-key attempt runner 5/5, and API-key acceptance 22/22; preserved Codex suites passed 8/8 and 27/27, protocol compatibility passed 15/15, 11/11, and 6/6, the full Gateway passed 391/391 library plus 3/3 CLI and 0 documentation tests, and all-target/all-feature offline Clippy completed with warnings denied. The final independent whole-branch review found no Critical or Important issues and assessed the branch ready to publish/merge. It accepted two non-blocking maintainability debts for follow-up: duplicated retry-policy authorities and duplicated OpenAI non-stream lifecycle logic.

This resolution is deterministic Linux evidence only. It used no live credentials or provider endpoints and made no installation, deployment, profile, proxy, or running-runtime change; the installed Gateway and Claude Science services remain untouched. Real macOS runtime verification remains pending.
