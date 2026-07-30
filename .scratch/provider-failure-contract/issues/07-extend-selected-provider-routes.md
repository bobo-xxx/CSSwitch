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

The API-key failure scenarios remain deterministic Linux evidence only: no live API-key provider was induced to fail, and real macOS runtime verification remains pending.

## Installed Linux verification

On 2026-07-30, the exact merged revision `5374d40716a5ee32a8e1b23bc55c3a94ac057565` was built offline in an isolated detached worktree and deployed as the production Gateway executable. The installed binary SHA-256 is `300e65867fedd68ed42aa07839fad005d3e20c0f70827332a055ee31bf753d49`; the byte-identical pre-deployment backup is `/home/bio-13/.local/bin/csswitch-gateway.backup-pre-ticket07-20260730` with SHA-256 `623d0e26cd1084f6c4f9de5c7175cb193eb6deeb056dc89af38797628797fad9`.

Only the owned Gateway on `127.0.0.1:11535` was stopped and restarted. Claude Science remained on its existing process and port `9002`; proxy port `2999` remained listening. The restarted Gateway process used `HTTP_PROXY`, `HTTPS_PROXY`, `ALL_PROXY` and lowercase equivalents at loopback port `2999`, while both `NO_PROXY` forms retained `127.0.0.1` and `localhost`. Authentication, health, and live Codex model discovery were ready after restart.

A minimal Codex-only subscription inference through the installed Gateway selected `claude-csswitch-codex-gpt-5.6-sol`, returned the expected marker, and recorded one completed `responses_lite` attempt with one POST, zero repairs, and no retry delay. Science continued to answer on `9002` with its expected unauthenticated-root `401` boundary. No provider fallback, profile, credential, Science data, tunnel, or port configuration was changed. Live API-key route failure induction and real macOS verification remain pending.

## Installed Linux launcher persistence

On 2026-07-30, the machine-local `/home/bio-13/.local/bin/csswitch-codex` launcher was hardened without restarting or signalling the running Gateway or Claude Science. The byte-identical rollback backup is `/home/bio-13/.local/bin/csswitch-codex.backup-pre-persistence-20260730` with SHA-256 `87894fda3898a54c90ce3baa3c7efaea8e419d89fabbad37c22ffd94f1b3cc17`, mode `0755`, and size `12179` bytes. The installed launcher has SHA-256 `b7fa8c4598740c3bf351298a1902663c3f09be21dba5bc11dac6442f46c91c4e`, mode `0755`, and size `12567` bytes.

A regression-first fake-child contract proved the old launcher defaulted to port `11434` and owned no Gateway proxy default. The installed launcher now defaults to Gateway port `11535`, passes proxy `http://127.0.0.1:2999` through all six uppercase/lowercase proxy variables, passes `127.0.0.1,localhost` through both no-proxy variables, and honors explicit port, proxy, and no-proxy overrides. The portable repository launcher remains byte-identical with its generic `11434` default.

Plain launcher status reported Codex authentication, Gateway health, dynamic catalog, and Claude Science ready. Gateway and Science retained their pre-install PIDs and listeners; the running Gateway executable retained SHA-256 `300e65867fedd68ed42aa07839fad005d3e20c0f70827332a055ee31bf753d49` and its proxy environment on loopback port `2999`. No provider, model, credential, profile, fallback, Science data, tunnel, firewall, or runtime process was changed. This proves Linux launcher restart configuration without performing a restart; live API-key failure induction and real macOS verification remain pending.
