use std::fmt;

use serde::Serialize;
use serde_json::{json, Value};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ProviderId {
    Codex,
    Deepseek,
    Qwen,
    Relay,
    OpenaiCustom,
    OpenaiResponses,
}

impl ProviderId {
    pub(crate) fn from_adapter(adapter: &str) -> Option<Self> {
        match adapter {
            "codex" => Some(Self::Codex),
            "deepseek" => Some(Self::Deepseek),
            "qwen" => Some(Self::Qwen),
            "relay" => Some(Self::Relay),
            "openai-custom" => Some(Self::OpenaiCustom),
            "openai-responses" => Some(Self::OpenaiResponses),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RouteMode {
    Responses,
    ResponsesLite,
    AnthropicMessages,
    OpenaiChat,
    OpenaiResponses,
}

impl RouteMode {
    pub(crate) fn from_api_key_transport(transport: &str) -> Option<Self> {
        match transport {
            "anthropic_messages" => Some(Self::AnthropicMessages),
            "openai_chat" => Some(Self::OpenaiChat),
            "openai_responses" => Some(Self::OpenaiResponses),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct CorrelationId(String);

impl CorrelationId {
    pub(crate) fn new(value: impl Into<String>) -> Option<Self> {
        let value = value.into();
        RequestId::is_valid(&value).then_some(Self(value))
    }
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RequestId(String);

impl RequestId {
    pub(crate) fn new(value: &str) -> Option<Self> {
        Self::is_valid(value).then(|| Self(value.to_owned()))
    }
    fn is_valid(value: &str) -> bool {
        !value.is_empty()
            && value.len() <= 256
            && value.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':')
            })
    }
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RetryPolicy {
    pub(crate) max_posts: u8,
    pub(crate) fallback_delays_ms: [u64; 2],
    pub(crate) retry_after_cap_seconds: u64,
    retry_408: bool,
    retry_409: bool,
    retry_429: bool,
    retry_5xx: bool,
    retry_connect: bool,
    retry_timeout: bool,
    retry_unknown_429: bool,
}

impl RetryPolicy {
    pub(crate) const CODEX: Self = Self {
        max_posts: 3,
        fallback_delays_ms: [500, 1_000],
        retry_after_cap_seconds: 60,
        retry_408: true,
        retry_409: true,
        retry_429: true,
        retry_5xx: true,
        retry_connect: true,
        retry_timeout: true,
        retry_unknown_429: false,
    };

    pub(crate) const ANTHROPIC_MESSAGES: Self = Self {
        max_posts: 3,
        fallback_delays_ms: [500, 1_000],
        retry_after_cap_seconds: 60,
        retry_408: true,
        retry_409: false,
        retry_429: true,
        retry_5xx: true,
        retry_connect: true,
        retry_timeout: true,
        retry_unknown_429: true,
    };

    pub(crate) const OPENAI_CHAT: Self = Self {
        max_posts: 3,
        fallback_delays_ms: [500, 1_000],
        retry_after_cap_seconds: 60,
        retry_408: true,
        retry_409: true,
        retry_429: true,
        retry_5xx: true,
        retry_connect: true,
        retry_timeout: true,
        retry_unknown_429: true,
    };

    pub(crate) const OPENAI_RESPONSES: Self = Self::OPENAI_CHAT;

    pub(crate) fn allows(&self, observation: &FailureObservation) -> bool {
        match observation {
            FailureObservation::Http { status: 408, .. } => self.retry_408,
            FailureObservation::Http { status: 409, .. } => self.retry_409,
            FailureObservation::Http {
                status: 429,
                rate_kind,
                ..
            } => self.retry_429 && (rate_kind.is_some() || self.retry_unknown_429),
            FailureObservation::Http {
                status: 500..=599, ..
            } => self.retry_5xx,
            FailureObservation::Network(NetworkKind::Connect) => self.retry_connect,
            FailureObservation::Network(NetworkKind::Timeout) => self.retry_timeout,
            FailureObservation::Network(NetworkKind::Read) => *self == Self::CODEX,
            FailureObservation::Http { .. }
            | FailureObservation::Protocol(_)
            | FailureObservation::Cancelled => false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RouteContext {
    pub(crate) provider: ProviderId,
    pub(crate) route: RouteMode,
    pub(crate) correlation_id: CorrelationId,
    pub(crate) retry_policy: RetryPolicy,
}

impl RouteContext {
    pub(crate) fn codex(route: RouteMode, correlation_id: CorrelationId) -> Self {
        Self {
            provider: ProviderId::Codex,
            route,
            correlation_id,
            retry_policy: RetryPolicy::CODEX,
        }
    }

    // Introduced at the Task 1 interface boundary; production handlers consume it in later tasks.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn api_key(
        provider: ProviderId,
        route: RouteMode,
        correlation_id: CorrelationId,
        retry_policy: RetryPolicy,
    ) -> Self {
        assert!(
            provider != ProviderId::Codex,
            "API-key provider must be non-Codex"
        );
        assert!(
            !matches!(route, RouteMode::Responses | RouteMode::ResponsesLite),
            "API-key route must be non-Codex"
        );
        assert!(
            retry_policy != RetryPolicy::CODEX,
            "API-key retry policy must be non-Codex"
        );
        Self {
            provider,
            route,
            correlation_id,
            retry_policy,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RateKind {
    RateLimit,
    Quota,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ErrorCode {
    Absent,
    UnsupportedValue,
    InsufficientQuota,
    RateLimitError,
    RateLimitExceeded,
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ErrorParam {
    Absent,
    ToolChoice,
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NetworkKind {
    Connect,
    Timeout,
    Read,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProtocolKind {
    InvalidResponse,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum FailureObservation {
    Http {
        status: u16,
        rate_kind: Option<RateKind>,
        retry_after_seconds: Option<u64>,
        request_id: Option<RequestId>,
        error_code: ErrorCode,
        error_param: ErrorParam,
    },
    Network(NetworkKind),
    Protocol(ProtocolKind),
    Cancelled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum FailureClass {
    Authentication,
    Authorization,
    InvalidRequest,
    Capability,
    Quota,
    RateLimit,
    Transient,
    Network,
    Protocol,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AttemptOutcome {
    Completed,
    Failed,
    Cancelled,
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct ProviderFailure {
    status: u16,
    error_type: &'static str,
    message: &'static str,
    context: RouteContext,
    failure_class: FailureClass,
    upstream_status: Option<u16>,
    retryable: bool,
    recovery: &'static str,
    request_id: Option<RequestId>,
    retry_after_seconds: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct AttemptDiagnostic {
    outcome: AttemptOutcome,
    provider: ProviderId,
    route: RouteMode,
    correlation_id: CorrelationId,
    posts: u8,
    repairs: u8,
    delays_ms: Vec<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    mapped_status: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    upstream_status: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    failure_class: Option<FailureClass>,
    #[serde(skip_serializing_if = "Option::is_none")]
    retryable: Option<bool>,
}

impl ProviderFailure {
    pub(crate) fn from_observation(
        context: &RouteContext,
        observation: &FailureObservation,
        exhausted: bool,
    ) -> Self {
        let (upstream_status, request_id, retry_after_seconds) = match observation {
            FailureObservation::Http {
                status,
                request_id,
                retry_after_seconds,
                ..
            } => (Some(*status), request_id.clone(), *retry_after_seconds),
            FailureObservation::Network(_) | FailureObservation::Protocol(_) => (None, None, None),
            FailureObservation::Cancelled => unreachable!("cancellation has no caller failure"),
        };
        let (status, error_type, message, failure_class, retryable, recovery) = match observation {
            FailureObservation::Http { status: 401, .. } => (
                401,
                "authentication_error",
                "Provider authentication failed",
                FailureClass::Authentication,
                false,
                "Re-authenticate, then start a new caller request",
            ),
            FailureObservation::Http { status: 403, .. } => (
                403,
                "permission_error",
                "Provider authorization failed",
                FailureClass::Authorization,
                false,
                "Check account, workspace, geography, and model entitlement",
            ),
            FailureObservation::Http {
                status: 400,
                error_code: ErrorCode::UnsupportedValue,
                ..
            }
            | FailureObservation::Http {
                status: 400,
                error_param: ErrorParam::ToolChoice,
                ..
            } => (
                400,
                "invalid_request_error",
                "Provider Route cannot preserve this request",
                FailureClass::Capability,
                false,
                "Use equivalent supported request semantics or select a compatible model",
            ),
            FailureObservation::Http {
                status: 429,
                rate_kind: Some(RateKind::Quota),
                ..
            } => (
                429,
                "rate_limit_error",
                "Provider quota prevents this request",
                FailureClass::Quota,
                false,
                "Restore account quota before starting a new request",
            ),
            FailureObservation::Http {
                status: 429,
                rate_kind: Some(RateKind::RateLimit),
                ..
            } => (
                429,
                "rate_limit_error",
                "Provider rate limit prevents this request",
                FailureClass::RateLimit,
                exhausted,
                "Wait for rate capacity before starting a new request",
            ),
            FailureObservation::Http { status: 429, .. } => (
                429,
                "rate_limit_error",
                "Provider rate limit prevents this request",
                FailureClass::RateLimit,
                exhausted,
                "Wait for rate capacity before starting a new request",
            ),
            FailureObservation::Http { status: 408, .. } => (
                504,
                "api_error",
                "Provider transient failure exhausted retry budget",
                FailureClass::Transient,
                exhausted,
                "Start a new request after the provider recovers",
            ),
            FailureObservation::Http {
                status: 409 | 500..=599,
                ..
            } => (
                502,
                "api_error",
                "Provider transient failure exhausted retry budget",
                FailureClass::Transient,
                exhausted,
                "Start a new request after the provider recovers",
            ),
            FailureObservation::Http { status, .. } if (400..=499).contains(status) => (
                *status,
                "invalid_request_error",
                "Provider rejected the request",
                FailureClass::InvalidRequest,
                false,
                "Correct the request or select a compatible model",
            ),
            FailureObservation::Http { .. } => (
                502,
                "api_error",
                "Provider returned an unsupported status",
                FailureClass::Protocol,
                false,
                "Inspect sanitized diagnostics and start a new request",
            ),
            FailureObservation::Network(NetworkKind::Timeout) => (
                504,
                "api_error",
                "Provider network failure exhausted retry budget",
                FailureClass::Network,
                exhausted,
                "Check the network route before starting a new request",
            ),
            FailureObservation::Network(NetworkKind::Connect | NetworkKind::Read) => (
                502,
                "api_error",
                "Provider network failure exhausted retry budget",
                FailureClass::Network,
                exhausted,
                "Check the network route before starting a new request",
            ),
            FailureObservation::Protocol(ProtocolKind::InvalidResponse) => (
                502,
                "api_error",
                "Provider response violated the expected protocol",
                FailureClass::Protocol,
                false,
                "Inspect sanitized diagnostics and start a new request",
            ),
            FailureObservation::Cancelled => unreachable!("cancellation has no caller failure"),
        };
        Self {
            status,
            error_type,
            message,
            context: context.clone(),
            failure_class,
            upstream_status,
            retryable,
            recovery,
            request_id,
            retry_after_seconds,
        }
    }

    pub(crate) fn status(&self) -> u16 {
        self.status
    }
    #[cfg(test)]
    pub(crate) fn failure_class(&self) -> FailureClass {
        self.failure_class
    }
    #[cfg(test)]
    pub(crate) fn retryable(&self) -> bool {
        self.retryable
    }

    pub(crate) fn anthropic_json(&self) -> Value {
        let mut error = json!({
            "type": self.error_type,
            "message": self.message,
            "provider": self.context.provider,
            "route": self.context.route,
            "failure_class": self.failure_class,
            "retryable": self.retryable,
            "correlation_id": self.context.correlation_id.as_str(),
            "recovery": self.recovery,
        });
        if let Some(status) = self.upstream_status {
            error["upstream_status"] = json!(status);
        }
        if let Some(request_id) = &self.request_id {
            error["request_id"] = json!(request_id.as_str());
        }
        if let Some(seconds) = self.retry_after_seconds {
            error["retry_after_seconds"] = json!(seconds.min(60));
        }
        json!({ "type": "error", "error": error })
    }
}

impl fmt::Debug for ProviderFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderFailure")
            .field("status", &self.status)
            .field("error_type", &self.error_type)
            .field("failure_class", &self.failure_class)
            .field("upstream_status", &self.upstream_status)
            .field("retryable", &self.retryable)
            .field("correlation_id", &self.context.correlation_id.as_str())
            .finish()
    }
}

impl AttemptDiagnostic {
    fn without_failure(
        outcome: AttemptOutcome,
        context: &RouteContext,
        snapshot: AttemptSnapshot,
    ) -> Self {
        Self {
            outcome,
            provider: context.provider,
            route: context.route,
            correlation_id: context.correlation_id.clone(),
            posts: snapshot.posts,
            repairs: snapshot.repairs,
            delays_ms: snapshot.delays_ms,
            mapped_status: None,
            upstream_status: None,
            failure_class: None,
            retryable: None,
        }
    }

    pub(crate) fn failed(
        context: &RouteContext,
        posts: u8,
        repairs: u8,
        delays_ms: Vec<u64>,
        failure: &ProviderFailure,
    ) -> Self {
        Self {
            outcome: AttemptOutcome::Failed,
            provider: context.provider,
            route: context.route,
            correlation_id: context.correlation_id.clone(),
            posts,
            repairs,
            delays_ms,
            mapped_status: Some(failure.status),
            upstream_status: failure.upstream_status,
            failure_class: Some(failure.failure_class),
            retryable: Some(failure.retryable),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RepairKind {
    OmitAutomaticToolChoice,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AttemptPhase {
    Ready,
    InFlight,
    RetryAuthorized,
    RepairAuthorized,
    UpstreamOpen,
    TerminalPending(AttemptOutcome),
    Finalized(AttemptOutcome),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum AttemptDirective {
    RetryAfter(u64),
    RepairOnce(RepairKind),
    Fail(ProviderFailure),
    Cancel,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TransitionError {
    PostNotAuthorized,
    ObservationNotAuthorized,
    ResponseAlreadyStarted,
    FinalizationNotAuthorized,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AttemptSnapshot {
    pub(crate) posts: u8,
    pub(crate) repairs: u8,
    pub(crate) delays_ms: Vec<u64>,
    pub(crate) response_started: bool,
}

pub(crate) struct AttemptController {
    context: RouteContext,
    repair_enabled: bool,
    phase: AttemptPhase,
    posts: u8,
    repairs: u8,
    delays_ms: Vec<u64>,
    response_started: bool,
}

impl AttemptController {
    pub(crate) fn new(context: RouteContext, repair_enabled: bool) -> Self {
        Self {
            context,
            repair_enabled,
            phase: AttemptPhase::Ready,
            posts: 0,
            repairs: 0,
            delays_ms: Vec::new(),
            response_started: false,
        }
    }

    pub(crate) fn snapshot(&self) -> AttemptSnapshot {
        AttemptSnapshot {
            posts: self.posts,
            repairs: self.repairs,
            delays_ms: self.delays_ms.clone(),
            response_started: self.response_started,
        }
    }

    pub(crate) fn failed_diagnostic(
        &mut self,
        failure: &ProviderFailure,
    ) -> Result<AttemptDiagnostic, TransitionError> {
        if self.phase != AttemptPhase::TerminalPending(AttemptOutcome::Failed) {
            return Err(TransitionError::FinalizationNotAuthorized);
        }
        self.phase = AttemptPhase::Finalized(AttemptOutcome::Failed);
        let snapshot = self.snapshot();
        Ok(AttemptDiagnostic::failed(
            &self.context,
            snapshot.posts,
            snapshot.repairs,
            snapshot.delays_ms,
            failure,
        ))
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn internal_terminal_failure(&mut self) -> Result<(), TransitionError> {
        if matches!(
            self.phase,
            AttemptPhase::TerminalPending(_) | AttemptPhase::Finalized(_)
        ) {
            return Err(TransitionError::ObservationNotAuthorized);
        }
        self.phase = AttemptPhase::TerminalPending(AttemptOutcome::Failed);
        Ok(())
    }

    pub(crate) fn completed_diagnostic(&mut self) -> Result<AttemptDiagnostic, TransitionError> {
        if self.phase != AttemptPhase::UpstreamOpen {
            return Err(TransitionError::FinalizationNotAuthorized);
        }
        self.phase = AttemptPhase::Finalized(AttemptOutcome::Completed);
        Ok(AttemptDiagnostic::without_failure(
            AttemptOutcome::Completed,
            &self.context,
            self.snapshot(),
        ))
    }

    pub(crate) fn cancelled_diagnostic(&mut self) -> Result<AttemptDiagnostic, TransitionError> {
        if !matches!(
            self.phase,
            AttemptPhase::UpstreamOpen
                | AttemptPhase::TerminalPending(AttemptOutcome::Failed)
                | AttemptPhase::TerminalPending(AttemptOutcome::Cancelled)
        ) {
            return Err(TransitionError::FinalizationNotAuthorized);
        }
        self.phase = AttemptPhase::Finalized(AttemptOutcome::Cancelled);
        Ok(AttemptDiagnostic::without_failure(
            AttemptOutcome::Cancelled,
            &self.context,
            self.snapshot(),
        ))
    }

    pub(crate) fn begin_post(&mut self) -> Result<(), TransitionError> {
        let authorized = matches!(
            self.phase,
            AttemptPhase::Ready | AttemptPhase::RetryAuthorized | AttemptPhase::RepairAuthorized
        );
        let maximum = if self.repairs == 0 {
            self.context.retry_policy.max_posts
        } else {
            2
        };
        if !authorized || self.posts >= maximum {
            return Err(TransitionError::PostNotAuthorized);
        }
        self.posts += 1;
        self.phase = AttemptPhase::InFlight;
        Ok(())
    }

    pub(crate) fn mark_response_started(&mut self) -> Result<(), TransitionError> {
        if self.phase != AttemptPhase::InFlight || self.response_started {
            return Err(TransitionError::ResponseAlreadyStarted);
        }
        self.response_started = true;
        self.phase = AttemptPhase::UpstreamOpen;
        Ok(())
    }

    pub(crate) fn observe(
        &mut self,
        observation: FailureObservation,
    ) -> Result<AttemptDirective, TransitionError> {
        if matches!(&observation, FailureObservation::Cancelled) {
            if matches!(
                self.phase,
                AttemptPhase::TerminalPending(_) | AttemptPhase::Finalized(_)
            ) {
                return Err(TransitionError::ObservationNotAuthorized);
            }
            self.phase = AttemptPhase::TerminalPending(AttemptOutcome::Cancelled);
            return Ok(AttemptDirective::Cancel);
        }
        if !matches!(
            self.phase,
            AttemptPhase::InFlight | AttemptPhase::UpstreamOpen
        ) {
            return Err(TransitionError::ObservationNotAuthorized);
        }
        if self.response_started {
            self.phase = AttemptPhase::TerminalPending(AttemptOutcome::Failed);
            return Ok(AttemptDirective::Fail(ProviderFailure::from_observation(
                &self.context,
                &observation,
                false,
            )));
        }

        let exact_repair = matches!(
            &observation,
            FailureObservation::Http {
                status: 400,
                error_code: ErrorCode::UnsupportedValue,
                error_param: ErrorParam::ToolChoice,
                ..
            }
        );
        let can_repair = exact_repair
            && self.repair_enabled
            && self.context.route == RouteMode::ResponsesLite
            && self.posts == 1
            && self.repairs == 0
            && self.delays_ms.is_empty();
        if can_repair {
            self.repairs = 1;
            self.phase = AttemptPhase::RepairAuthorized;
            return Ok(AttemptDirective::RepairOnce(
                RepairKind::OmitAutomaticToolChoice,
            ));
        }

        let quota = matches!(
            &observation,
            FailureObservation::Http {
                status: 429,
                rate_kind: Some(RateKind::Quota),
                ..
            }
        );
        let policy_allows = self.context.retry_policy.allows(&observation);
        let retryable = !quota && policy_allows;
        let can_retry =
            self.repairs == 0 && self.posts < self.context.retry_policy.max_posts && retryable;
        if can_retry {
            let override_seconds = match &observation {
                FailureObservation::Http {
                    retry_after_seconds,
                    ..
                } => *retry_after_seconds,
                FailureObservation::Network(_)
                | FailureObservation::Protocol(_)
                | FailureObservation::Cancelled => None,
            };
            let delay_ms = override_seconds
                .map(|seconds| {
                    seconds.min(self.context.retry_policy.retry_after_cap_seconds) * 1_000
                })
                .unwrap_or(self.context.retry_policy.fallback_delays_ms[(self.posts - 1) as usize]);
            self.delays_ms.push(delay_ms);
            self.phase = AttemptPhase::RetryAuthorized;
            return Ok(AttemptDirective::RetryAfter(delay_ms));
        }

        let exhausted = retryable;
        self.phase = AttemptPhase::TerminalPending(AttemptOutcome::Failed);
        Ok(AttemptDirective::Fail(ProviderFailure::from_observation(
            &self.context,
            &observation,
            exhausted,
        )))
    }
}

#[cfg(test)]
mod tests;
