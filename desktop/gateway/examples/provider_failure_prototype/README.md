# Provider Failure Contract Seam Prototype

PROTOTYPE — throwaway terminal shell; never ship this example as gateway runtime code.

## Question

Does a pure Attempt Controller provide a sufficiently small interface to centralize Provider Failure classification, retry and Safe Repair budgets, response-started rules, and Anthropic-compatible serialization while leaving Provider Route adapters in control of protocol-specific normalization and I/O?

## Run

```bash
cargo run --offline --manifest-path desktop/gateway/Cargo.toml --example provider_failure_prototype
```

The prototype is in-memory only. It performs no network, OAuth, proxy, or filesystem-persistence operations. The shell renders the typed authorization phase, the last accepted observation/directive, and any rejected transition.

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
- `8`: allowlisted protocol Safe Repair observation
- `9`: unrepairable protocol failure
- `408`: upstream HTTP 408 request timeout
- `409`: upstream HTTP 409 conflict
- `c`: cancellation
- `r`: reset in-memory state
- `q`: quit

## Validation state

Decision: pending re-validation

The previous acceptance predates the typed attempt-authorization and forbidden-data hardening. Final review must re-drive the documented scenarios before deciding whether to accept the seam.
