# Implement the Codex-first contract slice

Type: task
Status: resolved
Blocked by: 02, 03

## Question

Implement the approved Provider Failure Contract seam for the Codex Provider Route using test-driven development. Add capability normalization, at most one known-safe repair-and-replay, bounded retry classification, sanitized diagnostics, and the backward-compatible error envelope proven by the acceptance harness. Do not add fallback or inspect credentials.

## Resolution

Implemented and published the Codex-first Provider Failure Contract on branch `feature/codex-provider-failure` at evidence/head commit `1fc249b684aeb8e034a510e44be0bf6582b08112`. The real handler now uses the pure Attempt Controller, bounded single-POST transport fact projection, at most three retry POSTs, one mutually exclusive Responses Lite Safe Repair, backward-compatible structured caller failures, the response-started replay barrier, cancellation-aware scheduling, and exactly one final sanitized attempt diagnostic.

The exact tested implementation is `9591a869f7268b5ff6baee00eb1365b36c54a618`; its independent final code/security review is clean. Deterministic verification passes the pure provider suite 12/12, transport suite 18/18, terminal-delivery suite 5/5, accepted harness 8/8, accepted contract 27/27, full gateway library 351/351, CLI 3/3, and offline Clippy with no warnings. Commands, redaction boundaries, and limitations are recorded in `docs/evidence/investigations/2026-07-29-codex-provider-failure-implementation.md` on the published feature branch.

This is deterministic evidence only: no installed runtime, live Codex subscription, or macOS runtime was verified, and no credential, profile, proxy, external endpoint, or running service was used or changed. No fallback was added. Ticket 06 remains responsible for installed-runtime and live-subscription verification; Ticket 07 remains responsible for the shared API-key Provider Route adapter. Both remain open and unclaimed.
