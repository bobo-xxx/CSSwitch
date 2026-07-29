## Destination

Ship a general Provider Failure Contract in CSSwitch, proven first through Codex subscription routing on Linux headless, that prevents indefinite generic “Claude unavailable” loops, safely adapts request capabilities, preserves Anthropic-compatible error JSON, passes the relevant compatibility suites, and is ready for upstream handoff.

## Notes

- This effort explicitly carries implementation and validation through the map; it is not limited to producing a specification.
- Orient to `CONTEXT.md` and use its terms **Provider Route** and **Provider Failure Contract** consistently.
- Consult `systematic-debugging`, `codebase-design`, `test-driven-development`, and `verification-before-completion` when their stages are reached.
- The architecture must be provider-general, but the first implementation and acceptance proof is Codex-only on Linux headless.
- Permanent 4xx failures fail immediately with sanitized provider/status/category/recovery metadata; 401/403 are authentication or authorization failures, not generic 502s.
- A 429 honors `Retry-After` with bounded retries. Network and upstream 5xx failures use bounded exponential retries, then surface a typed provider failure.
- Capability normalization may omit unsupported request fields. At most one deterministic repair-and-replay is allowed for a known safe transformation.
- Never silently delete conversation history, switch the selected model, or fall back to another provider. Unpreservable context fails explicitly as `context_limit` with recovery guidance.
- Preserve `type`, `error.type`, and `error.message`; add only optional structured metadata such as provider, upstream status, retryability, correlation ID, and recovery guidance.
- Diagnostics may retain correlation ID, timing, attempt count, mapped status, and upstream status, but never raw upstream bodies, prompts, credentials, account identifiers, or private URLs.
- Codex live verification may use only the already configured subscription and must not inspect credentials. Use an isolated fake upstream or recorded fixtures for deterministic acceptance tests.
- macOS runtime compatibility remains required, though a real macOS run may be documented as pending when no runner is available.

## Decisions so far

- [Establish the Codex Responses Lite contract](issues/01-establish-codex-responses-lite-contract.md) — Responses Lite is a private capability surface; preserve non-retryable capability/permanent failures, bound only the evidenced transient retry set, and expose only allowlisted diagnostics.

## Not yet specified

- Provider-family rollout beyond the first Codex proof; the exact boundary depends on what the capability and transport prototypes reveal.
- Long-context and compaction recovery beyond safe request normalization; the correct boundary depends on the verified failure taxonomy.
- Upstream publication form and reviewer evidence; the exact handoff depends on repository permissions and the final diff.

## Out of scope

- Automatic fallback to Kimi, Claude, or any other provider or model.
- Silent history deletion, model substitution, or semantic rewriting to make a request pass.
- Reading, exporting, or recording subscription credentials.
- Changing Claude Science’s UI banner or multi-agent orchestration in this effort.
- Treating a public Claude service outage as the cause of a Codex Provider Route failure without provider-specific evidence.
