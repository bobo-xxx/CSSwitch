use serde_json::{json, Value};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProviderId {
    Codex,
}

impl ProviderId {
    fn safe_label(self) -> &'static str {
        match self {
            Self::Codex => "codex",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RouteMode {
    ResponsesLite,
}

impl RouteMode {
    fn safe_label(self) -> &'static str {
        match self {
            Self::ResponsesLite => "responses_lite",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CorrelationId {
    Prototype0001,
}

impl CorrelationId {
    fn safe_label(self) -> &'static str {
        match self {
            Self::Prototype0001 => "prototype-0001",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RetryPolicy {
    max_posts: u8,
    base_delay_ms: u64,
    max_delay_ms: u64,
    retry_after_cap_ms: u64,
}

impl RetryPolicy {
    fn prototype() -> Self {
        Self {
            max_posts: 3,
            base_delay_ms: 500,
            max_delay_ms: 2_000,
            retry_after_cap_ms: 60_000,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RouteContext {
    provider: ProviderId,
    route: RouteMode,
    correlation_id: CorrelationId,
    policy: RetryPolicy,
}

impl RouteContext {
    pub fn codex_responses_lite_prototype() -> Self {
        Self {
            provider: ProviderId::Codex,
            route: RouteMode::ResponsesLite,
            correlation_id: CorrelationId::Prototype0001,
            policy: RetryPolicy::prototype(),
        }
    }
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProtocolClass {
    UnsupportedAutomaticToolChoice,
    InvalidResponse,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FailureObservation {
    Capability,
    Http {
        status: u16,
        rate_kind: Option<RateKind>,
        retry_after_ms: Option<u64>,
    },
    Network,
    Protocol(ProtocolClass),
    Cancelled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObservationKind {
    Capability,
    Http,
    Network,
    Protocol,
    Cancelled,
}

impl FailureObservation {
    fn kind(&self) -> ObservationKind {
        match self {
            Self::Capability => ObservationKind::Capability,
            Self::Http { .. } => ObservationKind::Http,
            Self::Network => ObservationKind::Network,
            Self::Protocol(_) => ObservationKind::Protocol,
            Self::Cancelled => ObservationKind::Cancelled,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttemptPhase {
    ReadyInitial,
    InFlight,
    RetryAuthorized,
    RepairAuthorized(RepairKind),
    Terminal,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttemptState {
    posts_started: u8,
    repairs_used: u8,
    response_started: bool,
    phase: AttemptPhase,
}

impl Default for AttemptState {
    fn default() -> Self {
        Self {
            posts_started: 0,
            repairs_used: 0,
            response_started: false,
            phase: AttemptPhase::ReadyInitial,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderFailure {
    status: u16,
    error_type: &'static str,
    message: &'static str,
    provider: ProviderId,
    route: RouteMode,
    failure_class: &'static str,
    upstream_status: Option<u16>,
    retryable: bool,
    correlation_id: CorrelationId,
    recovery: &'static str,
}

impl ProviderFailure {
    // The terminal shell renders JSON but does not execute the adapter's HTTP response write.
    #[allow(dead_code)]
    pub fn status(&self) -> u16 {
        self.status
    }

    pub fn anthropic_json(&self) -> Value {
        let mut error = json!({
            "type": self.error_type,
            "message": self.message,
            "provider": self.provider.safe_label(),
            "route": self.route.safe_label(),
            "failure_class": self.failure_class,
            "retryable": self.retryable,
            "correlation_id": self.correlation_id.safe_label(),
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransitionError {
    PostNotAuthorized {
        phase: AttemptPhase,
    },
    PostCapacityExhausted {
        phase: AttemptPhase,
    },
    ResponseStartNotAllowed {
        phase: AttemptPhase,
    },
    ObservationNotAllowed {
        observation: ObservationKind,
        phase: AttemptPhase,
    },
}

#[derive(Clone, Debug)]
pub struct AttemptController {
    context: RouteContext,
    state: AttemptState,
    enabled_repair: Option<RepairKind>,
    last_observation: Option<FailureObservation>,
    last_directive: Option<AttemptDirective>,
    last_transition_error: Option<TransitionError>,
}

impl AttemptController {
    pub fn new(context: RouteContext, enabled_repair: Option<RepairKind>) -> Self {
        Self {
            context,
            state: AttemptState::default(),
            enabled_repair,
            last_observation: None,
            last_directive: None,
            last_transition_error: None,
        }
    }

    pub fn provider_label(&self) -> &'static str {
        self.context.provider.safe_label()
    }

    pub fn route_label(&self) -> &'static str {
        self.context.route.safe_label()
    }

    pub fn correlation_id_label(&self) -> &'static str {
        self.context.correlation_id.safe_label()
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

    pub fn last_transition_error(&self) -> Option<&TransitionError> {
        self.last_transition_error.as_ref()
    }

    pub fn begin_post(&mut self) -> Result<(), TransitionError> {
        let phase = self.state.phase;
        if !matches!(
            phase,
            AttemptPhase::ReadyInitial
                | AttemptPhase::RetryAuthorized
                | AttemptPhase::RepairAuthorized(_)
        ) {
            return self.reject(TransitionError::PostNotAuthorized { phase });
        }
        if self.state.posts_started >= self.context.policy.max_posts {
            return self.reject(TransitionError::PostCapacityExhausted { phase });
        }

        self.state.posts_started += 1;
        self.state.phase = AttemptPhase::InFlight;
        self.last_observation = None;
        self.last_directive = None;
        self.last_transition_error = None;
        Ok(())
    }

    pub fn mark_response_started(&mut self) -> Result<(), TransitionError> {
        let phase = self.state.phase;
        if phase != AttemptPhase::InFlight {
            return self.reject(TransitionError::ResponseStartNotAllowed { phase });
        }

        self.state.response_started = true;
        self.last_transition_error = None;
        Ok(())
    }

    pub fn reset(&mut self) {
        self.state = AttemptState::default();
        self.last_observation = None;
        self.last_directive = None;
        self.last_transition_error = None;
    }

    pub fn observe(
        &mut self,
        observation: FailureObservation,
    ) -> Result<AttemptDirective, TransitionError> {
        let phase = self.state.phase;
        if !self.observation_allowed(&observation) {
            return self.reject(TransitionError::ObservationNotAllowed {
                observation: observation.kind(),
                phase,
            });
        }

        let directive = self.decide(&observation);
        self.state.phase = match &directive {
            AttemptDirective::Fail(_) | AttemptDirective::Cancel => AttemptPhase::Terminal,
            AttemptDirective::RetryAfter(_) => AttemptPhase::RetryAuthorized,
            AttemptDirective::RepairOnce(repair) => AttemptPhase::RepairAuthorized(*repair),
        };
        self.last_observation = Some(observation);
        self.last_directive = Some(directive.clone());
        self.last_transition_error = None;
        Ok(directive)
    }

    fn observation_allowed(&self, observation: &FailureObservation) -> bool {
        match observation {
            FailureObservation::Capability => self.state.phase == AttemptPhase::ReadyInitial,
            FailureObservation::Http { .. }
            | FailureObservation::Network
            | FailureObservation::Protocol(_) => self.state.phase == AttemptPhase::InFlight,
            FailureObservation::Cancelled => self.state.phase != AttemptPhase::Terminal,
        }
    }

    fn decide(&mut self, observation: &FailureObservation) -> AttemptDirective {
        match observation {
            FailureObservation::Capability => self.fail(
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
            FailureObservation::Protocol(ProtocolClass::UnsupportedAutomaticToolChoice)
                if self.can_repair(RepairKind::OmitUnsupportedAutomaticToolChoice) =>
            {
                self.state.repairs_used += 1;
                AttemptDirective::RepairOnce(RepairKind::OmitUnsupportedAutomaticToolChoice)
            }
            FailureObservation::Protocol(_) => self.fail(
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
        self.state.phase == AttemptPhase::InFlight
            && !self.state.response_started
            && self.state.posts_started < self.context.policy.max_posts
    }

    fn can_repair(&self, repair: RepairKind) -> bool {
        self.state.phase == AttemptPhase::InFlight
            && !self.state.response_started
            && self.state.repairs_used == 0
            && self.enabled_repair == Some(repair)
            && self.state.posts_started < self.context.policy.max_posts
    }

    fn backoff_ms(&self) -> u64 {
        let mut delay = self.context.policy.base_delay_ms;
        for _ in 1..self.state.posts_started {
            delay = delay.saturating_mul(2);
        }
        delay.min(self.context.policy.max_delay_ms)
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
            provider: self.context.provider,
            route: self.context.route,
            failure_class,
            upstream_status,
            retryable: false,
            correlation_id: self.context.correlation_id,
            recovery,
        })
    }

    fn reject<T>(&mut self, error: TransitionError) -> Result<T, TransitionError> {
        self.last_transition_error = Some(error);
        Err(error)
    }
}
