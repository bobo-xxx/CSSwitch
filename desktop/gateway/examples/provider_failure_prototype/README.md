# Provider Failure Contract Seam Prototype

PROTOTYPE — throwaway terminal shell; never ship this example as gateway runtime code.

## Question

Does a pure Attempt Controller provide a sufficiently small interface to centralize Provider Failure classification, retry and Safe Repair budgets, response-started rules, and Anthropic-compatible serialization while leaving Provider Route adapters in control of protocol-specific normalization and I/O?

## Run

```bash
cargo run --offline --manifest-path desktop/gateway/Cargo.toml --example provider_failure_prototype
```

The prototype is in-memory only. It performs no network, OAuth, proxy, or filesystem-persistence operations.
