# CSSwitch Provider Routing

This context names the provider-routing concepts CSSwitch exposes consistently across Claude Science integrations.

## Language

**Provider Route**:
The configured path that adapts a Claude Science inference request to one selected upstream provider and model.
_Avoid_: Backend, proxy mode

**Provider Failure Contract**:
The provider-independent, sanitized classification CSSwitch exposes when an inference request fails, including whether retrying the same request can succeed.
_Avoid_: Generic 502, Claude outage

**Failure Observation**:
The sanitized facts CSSwitch knows about one unsuccessful attempt through a Provider Route before applying the Provider Failure Contract.
_Avoid_: Raw upstream error, exception

**Provider Failure**:
The final sanitized outcome CSSwitch exposes after the Provider Failure Contract determines that no further safe action remains.
_Avoid_: Upstream error, Claude outage

**Safe Repair**:
An explicitly allowlisted, semantics-preserving request transformation that CSSwitch may apply at most once before replaying a Provider Route attempt.
_Avoid_: Fallback, silent rewrite

**Provider Retry Policy**:
The explicit per-Provider Route allowlist of Failure Observation classes that may consume a bounded replay budget.
_Avoid_: Global retry policy, retry every provider error
