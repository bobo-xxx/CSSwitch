use std::collections::VecDeque;

use crate::api_key_attempt::{ApiKeyAttemptSequence, ApiKeyTransport, AttemptRuntime, OpenResult};
use crate::messages::AttemptMode;
use crate::provider_failure::{
    CorrelationId, ErrorCode, ErrorParam, FailureObservation, ProtocolKind, ProviderId, RateKind,
    RetryPolicy, RouteContext, RouteMode,
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
    assert!(matches!(result, OpenResult::Opened(_)));
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
    assert!(matches!(result, OpenResult::Cancelled(_)));
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
}
