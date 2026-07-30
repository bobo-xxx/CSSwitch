# Verify Codex routing on Linux headless

Type: task
Status: resolved
Blocked by: 05

## Question

Run the deterministic Codex acceptance suite, existing gateway unit and contract tests, and a narrowly scoped live check through the already configured Codex subscription on Linux headless. Record exact commands, sanitized evidence, remaining limitations, and whether the original generic-retry failure now terminates or recovers according to contract.

## Resolution

Verified and published on branch `ticket06/codex-linux-headless` at evidence commit `30ec9e2`. The exact Provider Failure Contract implementation remains `9591a869f7268b5ff6baee00eb1365b36c54a618`; its release Gateway was built offline, backed up over the previous installed binary, and installed with SHA-256 `623d0e26cd1084f6c4f9de5c7175cb193eb6deeb056dc89af38797628797fad9`. The recoverable previous binary remains at `/home/bio-13/.local/bin/csswitch-gateway.backup-ticket06-20260730`.

Fresh Linux-headless verification passed the full Gateway suite 351/351 plus CLI 3/3, accepted harness 8/8, accepted Provider Failure Contract 27/27, focused Codex library 166/166 plus CLI 1/1, and offline Clippy with warnings denied. Through the configured proxy at `127.0.0.1:2999`, both the isolated candidate and installed managed Gateway completed a live `gpt-5.6-sol` request with HTTP 200, exact expected text, one POST, zero repairs, and one `completed` diagnostic. Dynamic discovery returned seven Codex aliases. Claude Science is running on the tunnel-preserved loopback port `9002`; Gateway uses `11535` because existing port `11434` was left untouched.

The historical generic-retry upstream condition did not recur live, so this resolution does not claim to reproduce it. The deterministic real-handler contract proves that eligible transient failures terminate within three POSTs or recover, permanent/auth/quota failures return immediately, and opened/partial responses never replay. Sanitized commands, topology, hashes, results, rollback artifact, and limitations are recorded in `docs/evidence/investigations/2026-07-30-codex-linux-headless-live-verification.md` on the published evidence branch. No OAuth credential contents were inspected or recorded; account identifiers, local path secrets, and Science nonces were excluded from evidence; no profile, fallback, or conflicting service was changed. macOS and deliberately induced live failure remain unverified. Ticket 07 remains open for the shared API-key Provider Route adapter.
