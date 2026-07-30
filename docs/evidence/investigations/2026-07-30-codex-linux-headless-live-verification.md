# Codex Provider Failure Contract Linux-headless live verification

Date: 2026-07-30

## Scope

Ticket 06 verifies the reviewed Codex Provider Failure Contract on this Linux headless host. The check covered deterministic real-handler acceptance, a source-built isolated candidate, a backed-up installed Gateway replacement, live model discovery and two minimal live inferences through the already configured CSSwitch Codex subscription. It did not inspect, copy, print, or modify OAuth credential contents.

No provider fallback, model substitution, profile change, proxy mutation, or Claude Science data migration was performed. Claude Science stayed on loopback port `9002` for the user's existing SSH tunnel.

## Revisions and artifacts

- Published Ticket 05 evidence head: `1fc249b684aeb8e034a510e44be0bf6582b08112`.
- Exact implementation revision: `9591a869f7268b5ff6baee00eb1365b36c54a618`.
- Installed release Gateway SHA-256: `623d0e26cd1084f6c4f9de5c7175cb193eb6deeb056dc89af38797628797fad9`.
- Recoverable previous Gateway backup: `/home/bio-13/.local/bin/csswitch-gateway.backup-ticket06-20260730`.
- Previous Gateway SHA-256: `07e0d0687a456d8b9c81fd01102a94b193c3700762210ea0786db47211efbb4c`.

The release binary was built from the clean published feature source with:

```text
$ cargo build --release --offline --manifest-path desktop/gateway/Cargo.toml --bin csswitch-gateway
Finished release profile; exit 0

$ desktop/gateway/target/release/csswitch-gateway codex-auth status | [closed-field projection]
authenticated=true
reason=ready
expiry_state=valid
exit 0
```

Account hashes, auth epochs/generations, expiry timestamps, path secrets, OAuth records, response headers and Science access nonces were deliberately excluded from evidence.

## Deterministic verification

The full all-feature suite was rerun immediately before the live/install phase:

```text
$ cargo test --offline --manifest-path desktop/gateway/Cargo.toml --all-features
library: 351 passed; 0 failed
CLI integration: 3 passed; 0 failed
doc tests: 0 passed; 0 failed
exit 0
```

The named Ticket 06 gates were then rerun against the same source:

```text
$ cargo test --offline --manifest-path desktop/gateway/Cargo.toml \
    --features acceptance-build 'server::codex_acceptance::harness_' \
    -- --test-threads=1
8 passed; 0 failed
exit 0

$ cargo test --offline --manifest-path desktop/gateway/Cargo.toml \
    --features acceptance-build 'server::codex_acceptance::contract_' \
    -- --test-threads=1
27 passed; 0 failed
exit 0

$ cargo test --offline --manifest-path desktop/gateway/Cargo.toml \
    codex_ -- --test-threads=1
library: 166 passed; 0 failed
CLI integration: 1 passed; 0 failed
exit 0

$ cargo clippy --offline --manifest-path desktop/gateway/Cargo.toml \
    --all-targets --all-features -- -D warnings
exit 0; no warnings
```

The contract suite proves the bounded failure behavior that cannot be induced safely against a live subscription: network/5xx replay stops within three POSTs, bounded rate-limit recovery can succeed, exhausted 408 maps to 504, exhausted 409/500/503 maps to 502, permanent/auth/quota failures do not replay, Safe Repair is limited to one mutually exclusive replay, partial streams never replay, and caller/diagnostic projections remain redacted.

## Network and port topology

The live candidate Gateway received an explicit proxy environment:

```text
HTTP_PROXY=http://127.0.0.1:2999
HTTPS_PROXY=http://127.0.0.1:2999
NO_PROXY=127.0.0.1,localhost,::1
```

Loopback proxy port `2999` was listening before launch. Port `11434` remained owned by an existing Python service and was not stopped or modified. The installed controller was therefore launched with these explicit port overrides:

```text
CSSWITCH_GATEWAY_PORT=11535
CSSWITCH_SCIENCE_PORT=9002
CSSWITCH_SANDBOX_PORT=9003
```

Final listeners and controller state were:

```text
auth=ready
gateway=running health=ready catalog=ready
science=running listener=ready

127.0.0.1:11535  csswitch-gateway
127.0.0.1:9002   claude-science
127.0.0.1:9003   claude-science sandbox
```

Because the installed controller script's default remains `11434`, subsequent `start`, `status`, and `stop` operations for this runtime must retain `CSSWITCH_GATEWAY_PORT=11535` unless the conflicting service is removed. This is an operational port override, not a provider fallback.

## Live subscription evidence

Live model discovery through the isolated reviewed Gateway returned HTTP success and seven dynamic aliases:

```text
claude-csswitch-codex-gpt-5.6-sol
claude-csswitch-codex-gpt-5.6-terra
claude-csswitch-codex-gpt-5.6-luna
claude-csswitch-codex-gpt-5.5
claude-csswitch-codex-gpt-5.4
claude-csswitch-codex-gpt-5.4-mini
claude-csswitch-codex-gpt-5.3-codex-spark
```

The isolated source-built candidate then completed a minimal non-stream request:

```text
HTTP status: 200
response type: message
model: gpt-5.6-sol
content: TICKET06_OK
stop_reason: end_turn
final diagnostic: outcome=completed, route=responses_lite, posts=1, repairs=0, delays=[]
```

After backup and installation, the byte-identified release Gateway repeated the live check through the managed runtime:

```text
HTTP status: 200
response type: message
model: gpt-5.6-sol
content: TICKET06_INSTALLED_OK
stop_reason: end_turn
final diagnostic: outcome=completed, route=responses_lite, posts=1, repairs=0, delays=[]
```

Claude Science `0.1.15-dev.20260701.t220242.shaaa553de` subsequently reported release health on port `9002`. A single-use Science URL was generated but is not recorded because it contains a short-lived access nonce.

## Historical failure conclusion

The historical generic `Claude is temporarily unavailable — retrying` condition did not recur during either minimal live request: both recovered as first-attempt success, so this evidence does not claim to reproduce the same upstream fault. If that fault recurs, the deterministic real-handler contract establishes that the Codex Provider Route cannot loop indefinitely: eligible transient classes have at most three POSTs and then return their typed terminal envelope; permanent, authentication, authorization and quota failures return immediately; an opened/partial response is never replayed.

Therefore Ticket 06 verifies both sides honestly: the installed live route succeeds through the configured subscription, and the original failure family has a deterministic bounded terminal contract. It does not claim that a live upstream failure was forced or observed.

## Remaining limitations

- The optional browser/UI prompt through Claude Science was offered to the user but is not required for the automated Gateway live result and is not asserted here unless separately reported.
- No macOS runtime was available.
- No live transient failure, rate limit, auth rejection, Safe Repair, disconnect, or partial stream was deliberately induced against the subscription.
- The shared API-key Provider Route adapter remains Ticket 07.
- The backup was retained; rollback was not exercised because both installed health and live inference passed.
