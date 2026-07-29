use serde_json::{json, Value};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RetryPolicy {
    pub max_posts: u8,
    pub base_delay_ms: u64,
    pub max_delay_ms: u64,
    pub retry_after_cap_ms: u64,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_posts: 3,
            base_delay_ms: 500,
            max_delay_ms: 2_000,
            retry_after_cap_ms: 60_000,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AttemptState {
    pub posts_started: u8,
    pub repairs_used: u8,
    pub response_started: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RouteContext {
    pub provider: String,
    pub route: String,
    pub correlation_id: String,
    pub policy: RetryPolicy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RateKind {
    RateLimit,
    Quota,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RepairKind {
    OmitUnsupportedAutomaticToolChoice,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FailureObservation {
    Capability {
        repair: Option<RepairKind>,
    },
    Http {
        status: u16,
        rate_kind: Option<RateKind>,
        retry_after_ms: Option<u64>,
        repair: Option<RepairKind>,
    },
    Network,
    Protocol {
        repair: Option<RepairKind>,
    },
    Cancelled,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderFailure {
    pub status: u16,
    pub error_type: &'static str,
    pub message: &'static str,
    pub provider: String,
    pub route: String,
    pub failure_class: &'static str,
    pub upstream_status: Option<u16>,
    pub retryable: bool,
    pub correlation_id: String,
    pub recovery: &'static str,
}

impl ProviderFailure {
    pub fn anthropic_json(&self) -> Value {
        let mut error = json!({
            "type": self.error_type,
            "message": self.message,
            "provider": self.provider,
            "route": self.route,
            "failure_class": self.failure_class,
            "retryable": self.retryable,
            "correlation_id": self.correlation_id,
            "recovery": self.recovery,
        });
        if let Some(status) = self.upstream_status {
            error["upstream_status"] = json!(status);
        }
        json!({ "type": "error", "error": error })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AttemptDirective {
    Fail(ProviderFailure),
    RetryAfter(u64),
    RepairOnce(RepairKind),
    Cancel,
}

#[derive(Clone, Debug)]
pub struct AttemptController {
    context: RouteContext,
    state: AttemptState,
    enabled_repairs: Vec<RepairKind>,
    last_observation: Option<FailureObservation>,
    last_directive: Option<AttemptDirective>,
}

impl AttemptController {
    pub fn new(context: RouteContext, enabled_repairs: Vec<RepairKind>) -> Self {
        Self {
            context,
            state: AttemptState::default(),
            enabled_repairs,
            last_observation: None,
            last_directive: None,
        }
    }

    pub fn context(&self) -> &RouteContext {
        &self.context
    }

    pub fn state(&self) -> &AttemptState {
        &self.state
    }

    pub fn last_observation(&self) -> Option<&FailureObservation> {
        self.last_observation.as_ref()
    }

    pub fn last_directive(&self) -> Option<&AttemptDirective> {
        self.last_directive.as_ref()
    }

    pub fn begin_post(&mut self) -> bool {
        if self.state.response_started || self.state.posts_started >= self.context.policy.max_posts
        {
            return false;
        }
        self.state.posts_started += 1;
        self.last_observation = None;
        self.last_directive = None;
        true
    }

    pub fn mark_response_started(&mut self) -> bool {
        if self.state.posts_started == 0 {
            return false;
        }
        self.state.response_started = true;
        true
    }

    pub fn reset(&mut self) {
        self.state = AttemptState::default();
        self.last_observation = None;
        self.last_directive = None;
    }

    pub fn observe(&mut self, observation: FailureObservation) -> AttemptDirective {
        let directive = self.decide(&observation);
        self.last_observation = Some(observation);
        self.last_directive = Some(directive.clone());
        directive
    }

    fn decide(&mut self, observation: &FailureObservation) -> AttemptDirective {
        if matches!(observation, FailureObservation::Cancelled) {
            return AttemptDirective::Cancel;
        }
        if let Some(repair) = self.repair_from(observation) {
            if !self.state.response_started
                && self.state.repairs_used == 0
                && self.enabled_repairs.contains(&repair)
            {
                self.state.repairs_used = 1;
                return AttemptDirective::RepairOnce(repair);
            }
        }
        match observation {
            FailureObservation::Capability { .. } => self.fail(
                400,
                "invalid_request_error",
                "Provider Route capability cannot preserve this request",
                "capability",
                None,
                "Use equivalent supported request semantics or select a compatible model",
            ),
            FailureObservation::Http { status: 401, .. } => self.fail(
                401,
                "authentication_error",
                "Provider authentication failed",
                "authentication",
                Some(401),
                "Re-authenticate, then start a new caller request",
            ),
            FailureObservation::Http { status: 403, .. } => self.fail(
                403,
                "permission_error",
                "Provider authorization failed",
                "authorization",
                Some(403),
                "Check account, workspace, geography, and model entitlement",
            ),
            FailureObservation::Http {
                status: 429,
                rate_kind: Some(RateKind::RateLimit),
                retry_after_ms,
                ..
            } if self.can_retry() => AttemptDirective::RetryAfter(
                retry_after_ms
                    .unwrap_or_else(|| self.backoff_ms())
                    .min(self.context.policy.retry_after_cap_ms),
            ),
            FailureObservation::Http {
                status: 429,
                rate_kind,
                ..
            } => self.fail(
                429,
                "rate_limit_error",
                "Provider rate or quota limit prevents this request",
                if *rate_kind == Some(RateKind::Quota) {
                    "quota"
                } else {
                    "rate_limit"
                },
                Some(429),
                "Wait for rate capacity or restore account quota before a new request",
            ),
            FailureObservation::Http { status, .. }
                if matches!(*status, 408 | 409) || (500..=599).contains(status) =>
            {
                if self.can_retry() {
                    AttemptDirective::RetryAfter(self.backoff_ms())
                } else {
                    self.fail(
                        if *status == 408 { 504 } else { 502 },
                        "api_error",
                        "Provider transient failure exhausted the retry budget",
                        "transient",
                        Some(*status),
                        "Start a new request after the provider recovers",
                    )
                }
            }
            FailureObservation::Http { status, .. } if (400..=499).contains(status) => self.fail(
                *status,
                "invalid_request_error",
                "Provider rejected the request",
                "invalid_request",
                Some(*status),
                "Correct the request or select a compatible model",
            ),
            FailureObservation::Http { status, .. } => self.fail(
                502,
                "api_error",
                "Provider returned an unsupported status",
                "upstream",
                Some(*status),
                "Inspect sanitized diagnostics before starting a new request",
            ),
            FailureObservation::Network if self.can_retry() => {
                AttemptDirective::RetryAfter(self.backoff_ms())
            }
            FailureObservation::Network => self.fail(
                502,
                "api_error",
                "Provider network failure exhausted the retry budget",
                "network",
                None,
                "Check the network route before starting a new request",
            ),
            FailureObservation::Protocol { .. } => self.fail(
                502,
                "api_error",
                "Provider response violated the expected protocol",
                "protocol",
                None,
                "Inspect sanitized diagnostics and start a new request",
            ),
            FailureObservation::Cancelled => AttemptDirective::Cancel,
        }
    }

    fn can_retry(&self) -> bool {
        !self.state.response_started
            && self.state.posts_started > 0
            && self.state.posts_started < self.context.policy.max_posts
    }

    fn backoff_ms(&self) -> u64 {
        let mut delay = self.context.policy.base_delay_ms;
        for _ in 1..self.state.posts_started {
            delay = delay.saturating_mul(2);
        }
        delay.min(self.context.policy.max_delay_ms)
    }

    fn repair_from(&self, observation: &FailureObservation) -> Option<RepairKind> {
        match observation {
            FailureObservation::Capability { repair }
            | FailureObservation::Protocol { repair }
            | FailureObservation::Http { repair, .. } => *repair,
            FailureObservation::Network | FailureObservation::Cancelled => None,
        }
    }

    fn fail(
        &self,
        status: u16,
        error_type: &'static str,
        message: &'static str,
        failure_class: &'static str,
        upstream_status: Option<u16>,
        recovery: &'static str,
    ) -> AttemptDirective {
        AttemptDirective::Fail(ProviderFailure {
            status,
            error_type,
            message,
            provider: self.context.provider.clone(),
            route: self.context.route.clone(),
            failure_class,
            upstream_status,
            retryable: false,
            correlation_id: self.context.correlation_id.clone(),
            recovery,
        })
    }
}
