# CSSwitch Provider Routing

This context names the provider-routing concepts CSSwitch exposes consistently across Claude Science integrations.

## Language

**Provider Route**:
The configured path that adapts a Claude Science inference request to one selected upstream provider and model.
_Avoid_: Backend, proxy mode

**Provider Failure Contract**:
The provider-independent, sanitized classification CSSwitch exposes when an inference request fails, including whether retrying the same request can succeed.
_Avoid_: Generic 502, Claude outage
