# Establish the Codex Responses Lite contract

Type: research
Status: resolved
Blocked by: none

## Question

What documented and observed request capabilities, status semantics, retry signals, and safe diagnostic fields must CSSwitch honor when its Codex Provider Route talks to the Codex/Responses Lite upstream? Compare current official OpenAI documentation with the local transport behavior, distinguish sourced facts from inference, and return a compact compatibility matrix covering `tool_choice`, authentication failures, rate limits and `Retry-After`, other permanent 4xx responses, transient 5xx/network failures, subscription authentication constraints, and redaction boundaries.

## Answer

Responses Lite is an undocumented private ChatGPT/Codex route rather than the public Responses API contract. CSSwitch itself rejects non-`auto` Anthropic `tool_choice` values before transport, so that message is a local non-retryable capability error; `none`, `required`, and forced-tool semantics must not be silently rewritten to `auto`.

The current transport preserves 401, 403, and 429 but collapses all other upstream rejections to a generic 502 and drops upstream status, retry/reset signals, and request IDs. The Provider Failure Contract should preserve sanitized permanent 4xx classes, keep 401/403 as authentication/authorization failures, distinguish rate-limit 429 from quota when safely possible, and consider only network/408/409/rate-limited 429/5xx for explicitly bounded retries. Because the private route has no published idempotency guarantee, retries must stop after response bytes begin.

Safe diagnostic metadata is limited to allowlisted provider/route, mapped and upstream status, failure class, retryability, normalized delay/reset, correlation/request ID, attempt count, and stage timing. Credentials, account identifiers, bodies, prompts/history, tool arguments, cookies, private URLs, and raw upstream challenge/error content remain fully redacted.

Context: [primary-source research report](../../../docs/research/codex-responses-lite-contract.md), integrated from `research/codex-responses-lite-contract` commit `06aafbdf304a20185699445feeabdd9ed125a557`.
