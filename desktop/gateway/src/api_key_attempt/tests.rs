use std::collections::VecDeque;

use crate::api_key_attempt::{
    ApiKeyAttemptSequence, ApiKeyPostOnceTransport, ApiKeyTransport, AttemptRuntime, OpenResult,
};
use crate::config::{GatewayConfig, GatewayIntent};
use crate::messages::AttemptMode;
use crate::provider_failure::{
    AttemptController, CorrelationId, ErrorCode, ErrorParam, FailureObservation, ProtocolKind,
    ProviderId, RateKind, RetryPolicy, RouteContext, RouteMode, TransitionError,
};

#[derive(Debug)]
struct FakeOpened;

struct ScriptedTransport {
    script: VecDeque<Result<FakeOpened, FailureObservation>>,
    bodies: Vec<Vec<u8>>,
}

impl ScriptedTransport {
    fn script<const N: usize>(script: [Result<FakeOpened, FailureObservation>; N]) -> Self {
        Self {
            script: VecDeque::from(script),
            bodies: Vec::new(),
        }
    }

    fn fail_http<const N: usize>(statuses: [u16; N]) -> Self {
        Self::script(statuses.map(failure))
    }

    fn bodies(&self) -> &[Vec<u8>] {
        &self.bodies
    }

    fn posts(&self) -> usize {
        self.bodies.len()
    }
}

impl ApiKeyTransport for ScriptedTransport {
    type Opened = FakeOpened;

    fn post_once(
        &mut self,
        body: &[u8],
        _mode: AttemptMode,
    ) -> Result<Self::Opened, FailureObservation> {
        self.bodies.push(body.to_vec());
        self.script
            .pop_front()
            .expect("scripted transport must have one result per post")
    }
}

#[derive(Default)]
struct RecordingRuntime {
    delays: Vec<u64>,
    cancel_on_wait: Option<usize>,
}

impl RecordingRuntime {
    fn cancel_during_first_wait() -> Self {
        Self {
            delays: Vec::new(),
            cancel_on_wait: Some(0),
        }
    }
}

impl AttemptRuntime for RecordingRuntime {
    fn cancelled(&mut self) -> bool {
        false
    }

    fn wait(&mut self, delay_ms: u64) -> bool {
        let index = self.delays.len();
        self.delays.push(delay_ms);
        self.cancel_on_wait != Some(index)
    }
}

fn http_observation(status: u16, rate_kind: Option<RateKind>) -> FailureObservation {
    FailureObservation::Http {
        status,
        rate_kind,
        retry_after_seconds: None,
        request_id: None,
        error_code: ErrorCode::Absent,
        error_param: ErrorParam::Absent,
    }
}

fn quota_observation() -> FailureObservation {
    FailureObservation::Http {
        status: 429,
        rate_kind: Some(RateKind::Quota),
        retry_after_seconds: Some(0),
        request_id: None,
        error_code: ErrorCode::InsufficientQuota,
        error_param: ErrorParam::Absent,
    }
}

fn repair_observation() -> FailureObservation {
    FailureObservation::Http {
        status: 400,
        rate_kind: None,
        retry_after_seconds: None,
        request_id: None,
        error_code: ErrorCode::UnsupportedValue,
        error_param: ErrorParam::ToolChoice,
    }
}

fn failure(status: u16) -> Result<FakeOpened, FailureObservation> {
    Err(http_observation(status, None))
}

fn opened_success() -> Result<FakeOpened, FailureObservation> {
    Ok(FakeOpened)
}

fn openai_context() -> RouteContext {
    RouteContext::api_key(
        ProviderId::OpenaiCustom,
        RouteMode::OpenaiChat,
        CorrelationId::new("api-corr-openai").unwrap(),
        RetryPolicy::OPENAI_CHAT,
    )
}

fn anthropic_context() -> RouteContext {
    RouteContext::api_key(
        ProviderId::Relay,
        RouteMode::AnthropicMessages,
        CorrelationId::new("api-corr-anthropic").unwrap(),
        RetryPolicy::ANTHROPIC_MESSAGES,
    )
}

fn repair_enabled_test_context() -> RouteContext {
    RouteContext {
        provider: ProviderId::OpenaiCustom,
        route: RouteMode::ResponsesLite,
        correlation_id: CorrelationId::new("api-corr-repair-defensive").unwrap(),
        retry_policy: RetryPolicy::OPENAI_CHAT,
    }
}

fn adapter_debug_config() -> GatewayConfig {
    GatewayConfig {
        provider: "openai-custom".to_owned(),
        port: 0,
        auth_secret: None,
        api_key: Some("secret-must-not-leak".to_owned()),
        upstream_url: "http://127.0.0.1/unused".to_owned(),
        models_url: None,
        relay_thinking: None,
        provider_contract: None,
        intent: GatewayIntent::Formal,
        static_model_resolver: None,
        shim_mode: "off".to_owned(),
        codex_state_root: None,
        codex_contract: None,
        launch_id: String::new(),
        skill_data_dir: None,
        skill_bridge_dir: None,
        skill_bridge_token: None,
        science_host_context: None,
    }
}

#[test]
fn openai_conflict_retries_three_identical_posts_but_anthropic_stops_at_one() {
    let body = br#"{"model":"fixed","messages":[]}"#.to_vec();
    let mut openai = ScriptedTransport::fail_http([409, 409, 409]);
    let result = ApiKeyAttemptSequence::open(
        openai_context(),
        &body,
        AttemptMode::Nonstream,
        &mut openai,
        &mut RecordingRuntime::default(),
    );
    assert!(matches!(result, OpenResult::Failed(_)));
    assert_eq!(openai.bodies(), &[body.clone(), body.clone(), body.clone()]);

    let mut anthropic = ScriptedTransport::fail_http([409]);
    let result = ApiKeyAttemptSequence::open(
        anthropic_context(),
        &body,
        AttemptMode::Nonstream,
        &mut anthropic,
        &mut RecordingRuntime::default(),
    );
    assert!(matches!(result, OpenResult::Failed(_)));
    assert_eq!(anthropic.bodies(), &[body.clone()]);
}

#[test]
fn unknown_rate_limit_retries_but_quota_stops_without_repair() {
    let mut rate = ScriptedTransport::script([Err(http_observation(429, None)), opened_success()]);
    let mut runtime = RecordingRuntime::default();
    let result = ApiKeyAttemptSequence::open(
        anthropic_context(),
        b"{}",
        AttemptMode::Nonstream,
        &mut rate,
        &mut runtime,
    );
    let OpenResult::Opened(mut opened) = result else {
        panic!("unknown rate limit should retry then open")
    };
    let _opened_payload = &opened.opened;
    assert_eq!(opened.snapshot().posts, 2);
    assert!(opened.completed_diagnostic().is_ok());
    assert_eq!(runtime.delays, [500]);

    let mut quota = ScriptedTransport::script([Err(quota_observation())]);
    let result = ApiKeyAttemptSequence::open(
        anthropic_context(),
        b"{}",
        AttemptMode::Nonstream,
        &mut quota,
        &mut RecordingRuntime::default(),
    );
    let OpenResult::Failed(terminal) = result else {
        panic!("terminal quota")
    };
    assert_eq!(terminal.snapshot().posts, 1);
    assert_eq!(terminal.snapshot().repairs, 0);
}

#[test]
fn cancellation_during_wait_and_opened_response_are_final_barriers() {
    let mut runtime = RecordingRuntime::cancel_during_first_wait();
    let mut transport = ScriptedTransport::script([failure(503), opened_success()]);
    let result = ApiKeyAttemptSequence::open(
        openai_context(),
        b"{}",
        AttemptMode::Nonstream,
        &mut transport,
        &mut runtime,
    );
    let OpenResult::Cancelled(mut terminal) = result else {
        panic!("wait cancellation should stop the sequence")
    };
    assert!(terminal.cancelled_diagnostic().is_ok());
    assert_eq!(transport.posts(), 1);

    let mut opened_transport = ScriptedTransport::script([opened_success()]);
    let OpenResult::Opened(opened) = ApiKeyAttemptSequence::open(
        openai_context(),
        b"{}",
        AttemptMode::Nonstream,
        &mut opened_transport,
        &mut RecordingRuntime::default(),
    ) else {
        panic!("fixture must open");
    };
    let failure =
        opened.fail_after_open(FailureObservation::Protocol(ProtocolKind::InvalidResponse));
    assert_eq!(failure.snapshot().posts, 1);
    assert_eq!(opened_transport.posts(), 1);

    let mut opened_transport = ScriptedTransport::script([opened_success()]);
    let OpenResult::Opened(mut opened) = ApiKeyAttemptSequence::open(
        openai_context(),
        b"{}",
        AttemptMode::Nonstream,
        &mut opened_transport,
        &mut RecordingRuntime::default(),
    ) else {
        panic!("fixture must open");
    };
    assert!(opened.cancelled_diagnostic().is_ok());
}

#[test]
fn unexpected_repair_directive_fails_closed_and_finalizes_once() {
    let context = repair_enabled_test_context();
    let controller = AttemptController::new(context.clone(), true);
    let mut transport = ScriptedTransport::script([Err(repair_observation())]);
    let result = ApiKeyAttemptSequence::open_with_controller_for_test(
        context,
        controller,
        b"{}",
        AttemptMode::Nonstream,
        &mut transport,
        &mut RecordingRuntime::default(),
    );

    let OpenResult::Failed(mut terminal) = result else {
        panic!("unexpected repair directive must fail closed")
    };
    assert_eq!(terminal.snapshot().posts, 1);
    assert_eq!(terminal.snapshot().repairs, 1);
    assert_eq!(transport.posts(), 1);
    assert!(terminal.failed_diagnostic().is_ok());
    assert!(matches!(
        terminal.failed_diagnostic(),
        Err(TransitionError::FinalizationNotAuthorized)
    ));
}

#[test]
fn production_adapter_debug_names_provider_and_mode_without_secret() {
    let cfg = adapter_debug_config();
    let transport = ApiKeyPostOnceTransport::new(&cfg, AttemptMode::Nonstream);
    let debug = format!("{transport:?}");

    assert!(debug.contains("openai-custom"));
    assert!(debug.contains("Nonstream"));
    assert!(!debug.contains("secret-must-not-leak"));
}
