# Prototype the Codex acceptance harness

Type: prototype
Status: claimed
Blocked by: 01

## Question

What is the smallest deterministic fake-upstream or recorded-fixture harness that reproduces the observed Codex rejection and lets a reviewer judge the required behavior? Produce a disposable test skeleton and expected assertions for permanent 4xx, 401/403, 429 with `Retry-After`, 5xx/network retry exhaustion, Responses Lite capability normalization, one safe replay, schema compatibility, and diagnostic redaction.
