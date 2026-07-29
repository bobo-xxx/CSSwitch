# Choose the Provider Failure Contract seam

Type: prototype
Status: resolved
Blocked by: 01

## Question

Where is the smallest deep module boundary in the gateway for capability normalization, one safe repair-and-replay, retry classification, and sanitized Provider Failure Contract serialization across transports? Produce a disposable code-level sketch or contract-test outline comparing plausible seams, then review it with the user before choosing one. The design must preserve provider-specific details without duplicating retry and redaction policy in every transport.

## Answer

Choose a pure Attempt Controller as the Provider Failure Contract seam. Provider Route adapters retain protocol-specific normalization and I/O, but submit sanitized Failure Observations to the controller. The controller alone owns classification, retry and Safe Repair budgets, response-started invariants, and Provider Failure serialization.

The interactive prototype validated permanent failures, bounded retry exhaustion, one Safe Repair, the prohibition on replay after response bytes, rate versus quota behavior, authentication and authorization failures, and cancellation without using network traffic or credentials.

Context: branch `prototype/provider-failure-contract-seam`, commit `3726405e17919876f2e8bce6bcfbaed8ccb1b4c8`, path `desktop/gateway/examples/provider_failure_prototype/README.md`.
