# Provider Failure Contract Seam Prototype

PROTOTYPE — throwaway terminal shell; never ship this example as gateway runtime code.

## Question

Does a pure Attempt Controller provide a sufficiently small interface to centralize Provider Failure classification, retry and Safe Repair budgets, response-started rules, and Anthropic-compatible serialization while leaving Provider Route adapters in control of protocol-specific normalization and I/O?

## Run

```bash
cargo run --offline --manifest-path desktop/gateway/Cargo.toml --example provider_failure_prototype
```

The prototype is in-memory only. It performs no network, OAuth, proxy, or filesystem-persistence operations.

## Actions

- `p`: begin a POST attempt
- `b`: mark response bytes as started
- `1`: non-equivalent capability rejection
- `2`: HTTP 401 authentication rejection
- `3`: HTTP 403 authorization rejection
- `4`: rate-limited HTTP 429 with a 1500 ms Retry-After
- `5`: quota HTTP 429
- `6`: network failure
- `7`: upstream HTTP 500
- `8`: allowlisted Safe Repair observation
- `9`: unrepairable protocol failure
- `c`: cancellation
- `r`: reset in-memory state
- `q`: quit

## Verdict

Decision: accepted

The user validated the pure Attempt Controller seam after driving the permanent-failure, retry-exhaustion, one-repair, response-started, rate/quota, authentication/authorization, and cancellation scenarios.

Observation: The controller centralizes policy and state without taking Provider Route I/O away from adapters.
