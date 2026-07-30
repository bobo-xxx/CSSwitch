# Extend the contract to the selected Provider Routes

Type: task
Status: resolved
Blocked by: 04, 06

## Question

Apply the proven Provider Failure Contract to the provider families selected by “Decide the provider rollout boundary,” preserving their protocol-specific semantics while centralizing shared retry, normalization, serialization, and redaction rules. Add conformance cases before each provider change.

## Resolution

Implemented and independently reviewed the shared API-key Provider Failure Contract, then merged [PR #1](https://github.com/bobo-xxx/CSSwitch/pull/1) into `ticket06/codex-linux-headless` as `5374d40716a5ee32a8e1b23bc55c3a94ac057565`. The final reviewed feature head and evidence commit is `f507ffad01592f72a17e0820a1b0bff7444d8363`; the primary tested implementation is `2b5dcb804e9d18079309db1d5974b626d39123f6`, and the post-review oversized-body remediation is `378d02063c3719396e6b1cc8795865ed077ae338`. Commands, route assertions, closed schemas, redaction boundaries, remediation proof, and limitations are recorded in `docs/evidence/investigations/2026-07-30-api-key-provider-failure-contract.md` at the merged revision.

Every API-key Provider Route permits at most three pre-response POST attempts with fallback delays `[500, 1000]` ms, accepts only unsigned-decimal `Retry-After` delta seconds capped at 60 seconds, retries pre-response connect and timeout failures, and maps exhausted connect failures to `502` and timeouts to `504`. Anthropic Messages routes (`deepseek` and `relay`) retry `408`, `429`, and `500..=599`, while `409` and other permanent 4xx responses terminate. OpenAI Chat routes (`qwen` and `openai_custom`) and OpenAI Responses retry `408`, `409`, `429`, and `500..=599`; redirects and other permanent 4xx responses terminate. Exact `insufficient_quota` evidence overrides `429` and terminates on every route. API-key Safe Repair is disabled, response-open and downstream-delivery barriers prevent replay, and no provider or model fallback was added.

Fresh deterministic verification passed provider contracts 6/6, Provider Failure Contract 19/19, Messages 14/14, API-key attempt runner 5/5, and API-key acceptance 22/22; preserved Codex suites passed 8/8 and 27/27, protocol compatibility passed 15/15, 11/11, and 6/6, the full Gateway passed 392/392 library plus 3/3 CLI and 0 documentation tests, and all-target/all-feature offline Clippy completed with warnings denied. The final independent whole-branch review found no Critical or Important issues and assessed the branch ready to publish/merge. The only GitHub inline finding was fixed with regression-first proof, and no unresolved review thread remained at merge. Two non-blocking maintainability debts remain for follow-up: duplicated retry-policy authorities and duplicated OpenAI non-stream lifecycle logic.

This resolution is deterministic Linux evidence only. It used no live credentials or provider endpoints and made no installation, deployment, profile, proxy, or running-runtime change; the installed Gateway and Claude Science services remain untouched. Real macOS runtime verification remains pending.
