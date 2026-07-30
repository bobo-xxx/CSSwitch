use super::*;

fn context(route: RouteMode) -> RouteContext {
    RouteContext::codex(route, CorrelationId::new("corr-0001").unwrap())
}

fn api_context(provider: ProviderId, route: RouteMode, policy: RetryPolicy) -> RouteContext {
    RouteContext::api_key(
        provider,
        route,
        CorrelationId::new("api-corr-0001").unwrap(),
        policy,
    )
}

#[test]
fn api_key_adapter_and_transport_conversion_is_closed() {
    assert_eq!(ProviderId::from_adapter("codex"), Some(ProviderId::Codex));
    assert_eq!(
        ProviderId::from_adapter("deepseek"),
        Some(ProviderId::Deepseek)
    );
    assert_eq!(ProviderId::from_adapter("qwen"), Some(ProviderId::Qwen));
    assert_eq!(ProviderId::from_adapter("relay"), Some(ProviderId::Relay));
    assert_eq!(
        ProviderId::from_adapter("openai-custom"),
        Some(ProviderId::OpenaiCustom)
    );
    assert_eq!(
        ProviderId::from_adapter("openai-responses"),
        Some(ProviderId::OpenaiResponses)
    );
    assert_eq!(ProviderId::from_adapter("unknown"), None);

    assert_eq!(
        RouteMode::from_api_key_transport("anthropic_messages"),
        Some(RouteMode::AnthropicMessages)
    );
    assert_eq!(
        RouteMode::from_api_key_transport("openai_chat"),
        Some(RouteMode::OpenaiChat)
    );
    assert_eq!(
        RouteMode::from_api_key_transport("openai_responses"),
        Some(RouteMode::OpenaiResponses)
    );
    assert_eq!(
        RouteMode::from_api_key_transport("codex_responses_sse"),
        None
    );
    assert_eq!(RouteMode::from_api_key_transport("unknown"), None);
}

#[test]
fn request_id_uses_positive_allowlist_and_length_bound() {
    assert_eq!(
        RequestId::new("Req_01.a:b-c").unwrap().as_str(),
        "Req_01.a:b-c"
    );
    assert!(RequestId::new("").is_none());
    assert!(RequestId::new("request id").is_none());
    assert!(RequestId::new("request/id").is_none());
    assert!(RequestId::new(&"r".repeat(257)).is_none());
}

#[test]
fn permanent_failure_preserves_legacy_shape_and_omits_unknown_optionals() {
    let failure = ProviderFailure::from_observation(
        &context(RouteMode::Responses),
        &FailureObservation::Http {
            status: 422,
            rate_kind: None,
            retry_after_seconds: None,
            request_id: None,
            error_code: ErrorCode::Absent,
            error_param: ErrorParam::Absent,
        },
        false,
    );
    assert_eq!(failure.status(), 422);
    assert_eq!(
        failure.anthropic_json(),
        serde_json::json!({
            "type": "error",
            "error": {
                "type": "invalid_request_error",
                "message": "Provider rejected the request",
                "provider": "codex",
                "route": "responses",
                "failure_class": "invalid_request",
                "retryable": false,
                "correlation_id": "corr-0001",
                "recovery": "Correct the request or select a compatible model",
                "upstream_status": 422
            }
        })
    );
}

#[test]
fn redirect_projection_is_scoped_to_openai_chat() {
    let redirect = FailureObservation::Http {
        status: 307,
        rate_kind: None,
        retry_after_seconds: None,
        request_id: None,
        error_code: ErrorCode::Absent,
        error_param: ErrorParam::Absent,
    };

    let anthropic_failure = ProviderFailure::from_observation(
        &api_context(
            ProviderId::Relay,
            RouteMode::AnthropicMessages,
            RetryPolicy::ANTHROPIC_MESSAGES,
        ),
        &redirect,
        false,
    );
    assert_eq!(anthropic_failure.status(), 502);
    assert_eq!(
        anthropic_failure.anthropic_json()["error"]["route"],
        "anthropic_messages"
    );
    assert_eq!(
        anthropic_failure.anthropic_json()["error"]["upstream_status"],
        307
    );

    let openai_chat_failure = ProviderFailure::from_observation(
        &api_context(
            ProviderId::OpenaiCustom,
            RouteMode::OpenaiChat,
            RetryPolicy::OPENAI_CHAT,
        ),
        &redirect,
        false,
    );
    assert_eq!(openai_chat_failure.status(), 307);
    assert_eq!(
        openai_chat_failure.anthropic_json()["error"]["route"],
        "openai_chat"
    );
    assert_eq!(
        openai_chat_failure.anthropic_json()["error"]["upstream_status"],
        307
    );
}

#[test]
fn transient_failure_projects_only_valid_optional_metadata() {
    let failure = ProviderFailure::from_observation(
        &context(RouteMode::ResponsesLite),
        &FailureObservation::Http {
            status: 500,
            rate_kind: None,
            retry_after_seconds: Some(7),
            request_id: RequestId::new("req-safe-500"),
            error_code: ErrorCode::Absent,
            error_param: ErrorParam::Absent,
        },
        true,
    );
    let json = failure.anthropic_json();
    assert_eq!(failure.status(), 502);
    assert_eq!(json["error"]["type"], "api_error");
    assert_eq!(json["error"]["route"], "responses_lite");
    assert_eq!(json["error"]["failure_class"], "transient");
    assert_eq!(json["error"]["retryable"], true);
    assert_eq!(json["error"]["upstream_status"], 500);
    assert_eq!(json["error"]["request_id"], "req-safe-500");
    assert_eq!(json["error"]["retry_after_seconds"], 7);
}

#[test]
fn quota_and_rate_limit_are_distinct() {
    let quota = ProviderFailure::from_observation(
        &context(RouteMode::Responses),
        &FailureObservation::Http {
            status: 429,
            rate_kind: Some(RateKind::Quota),
            retry_after_seconds: None,
            request_id: None,
            error_code: ErrorCode::InsufficientQuota,
            error_param: ErrorParam::Absent,
        },
        false,
    );
    let rate = ProviderFailure::from_observation(
        &context(RouteMode::Responses),
        &FailureObservation::Http {
            status: 429,
            rate_kind: Some(RateKind::RateLimit),
            retry_after_seconds: None,
            request_id: None,
            error_code: ErrorCode::RateLimitExceeded,
            error_param: ErrorParam::Absent,
        },
        true,
    );
    assert_eq!(quota.failure_class(), FailureClass::Quota);
    assert!(!quota.retryable());
    assert_eq!(rate.failure_class(), FailureClass::RateLimit);
    assert!(rate.retryable());
}

#[test]
fn diagnostic_schema_has_no_arbitrary_string_slot() {
    let diagnostic = AttemptDiagnostic::failed(
        &context(RouteMode::Responses),
        3,
        0,
        vec![500, 1_000],
        &ProviderFailure::from_observation(
            &context(RouteMode::Responses),
            &FailureObservation::Network(NetworkKind::Connect),
            true,
        ),
    );
    assert_eq!(
        serde_json::to_value(diagnostic).unwrap(),
        serde_json::json!({
            "outcome": "failed",
            "provider": "codex",
            "route": "responses",
            "correlation_id": "corr-0001",
            "posts": 3,
            "repairs": 0,
            "delays_ms": [500, 1000],
            "mapped_status": 502,
            "failure_class": "network",
            "retryable": true
        })
    );
}

fn http(
    status: u16,
    rate_kind: Option<RateKind>,
    retry_after_seconds: Option<u64>,
) -> FailureObservation {
    FailureObservation::Http {
        status,
        rate_kind,
        retry_after_seconds,
        request_id: None,
        error_code: ErrorCode::Absent,
        error_param: ErrorParam::Absent,
    }
}

#[test]
fn anthropic_and_openai_policies_differ_only_on_conflict() {
    assert!(!RetryPolicy::ANTHROPIC_MESSAGES.allows(&http(409, None, None)));
    assert!(RetryPolicy::OPENAI_CHAT.allows(&http(409, None, None)));
    assert!(RetryPolicy::OPENAI_RESPONSES.allows(&http(409, None, None)));
    for policy in [
        RetryPolicy::ANTHROPIC_MESSAGES,
        RetryPolicy::OPENAI_CHAT,
        RetryPolicy::OPENAI_RESPONSES,
    ] {
        assert!(policy.allows(&http(408, None, None)));
        assert!(policy.allows(&http(429, None, None)));
        assert!(policy.allows(&http(529, None, None)));
        assert!(policy.allows(&FailureObservation::Network(NetworkKind::Connect)));
        assert!(policy.allows(&FailureObservation::Network(NetworkKind::Timeout)));
        assert!(!policy.allows(&FailureObservation::Network(NetworkKind::Read)));
        assert!(!policy.allows(&http(422, None, None)));
    }
}

#[test]
fn anthropic_messages_conflict_is_permanent_invalid_request() {
    let context = api_context(
        ProviderId::Relay,
        RouteMode::AnthropicMessages,
        RetryPolicy::ANTHROPIC_MESSAGES,
    );
    let failure = ProviderFailure::from_observation(&context, &http(409, None, None), false);
    assert_eq!(failure.status(), 409);
    assert_eq!(failure.failure_class(), FailureClass::InvalidRequest);
    assert!(!failure.retryable());
    assert_eq!(
        failure.anthropic_json(),
        serde_json::json!({
            "type": "error",
            "error": {
                "type": "invalid_request_error",
                "message": "Provider rejected the request",
                "provider": "relay",
                "route": "anthropic_messages",
                "failure_class": "invalid_request",
                "retryable": false,
                "correlation_id": "api-corr-0001",
                "recovery": "Correct the request or select a compatible model",
                "upstream_status": 409
            }
        })
    );
}

#[test]
fn api_key_quota_is_terminal_and_repair_is_structurally_disabled() {
    let context = api_context(
        ProviderId::Relay,
        RouteMode::AnthropicMessages,
        RetryPolicy::ANTHROPIC_MESSAGES,
    );
    let mut controller = AttemptController::new(context, false);
    controller.begin_post().unwrap();
    let quota = FailureObservation::Http {
        status: 429,
        rate_kind: Some(RateKind::Quota),
        retry_after_seconds: Some(0),
        request_id: None,
        error_code: ErrorCode::InsufficientQuota,
        error_param: ErrorParam::Absent,
    };
    assert!(matches!(
        controller.observe(quota).unwrap(),
        AttemptDirective::Fail(_)
    ));
    assert_eq!(controller.snapshot().posts, 1);
    assert_eq!(controller.snapshot().repairs, 0);
}

#[test]
fn api_key_unknown_429_is_retryable_after_policy_budget_exhaustion() {
    let context = api_context(
        ProviderId::Relay,
        RouteMode::AnthropicMessages,
        RetryPolicy::ANTHROPIC_MESSAGES,
    );
    let mut controller = AttemptController::new(context, false);
    for expected_delay in [500, 1_000] {
        controller.begin_post().unwrap();
        assert_eq!(
            controller.observe(http(429, None, None)).unwrap(),
            AttemptDirective::RetryAfter(expected_delay)
        );
    }
    controller.begin_post().unwrap();
    let AttemptDirective::Fail(failure) = controller.observe(http(429, None, None)).unwrap() else {
        panic!("exhausted API-key 429 must fail");
    };
    assert!(failure.retryable());
    assert_eq!(controller.snapshot().posts, 3);
}

#[test]
fn transient_sequence_uses_override_then_fallback_and_stops_at_three_posts() {
    let mut controller = AttemptController::new(context(RouteMode::Responses), false);
    controller.begin_post().unwrap();
    assert_eq!(
        controller.observe(http(500, None, Some(90))).unwrap(),
        AttemptDirective::RetryAfter(60_000)
    );
    controller.begin_post().unwrap();
    assert_eq!(
        controller.observe(http(500, None, None)).unwrap(),
        AttemptDirective::RetryAfter(1_000)
    );
    controller.begin_post().unwrap();
    let failure = match controller.observe(http(500, None, None)).unwrap() {
        AttemptDirective::Fail(failure) => failure,
        directive => panic!("expected terminal failure, got {directive:?}"),
    };
    assert_eq!(failure.status(), 502);
    assert!(failure.retryable());
    assert_eq!(controller.snapshot().posts, 3);
    assert_eq!(controller.snapshot().delays_ms, vec![60_000, 1_000]);
    assert!(controller.begin_post().is_err());
}

#[test]
fn quota_and_unknown_429_do_not_retry() {
    let mut quota = AttemptController::new(context(RouteMode::Responses), false);
    quota.begin_post().unwrap();
    let quota_failure = match quota
        .observe(http(429, Some(RateKind::Quota), Some(0)))
        .unwrap()
    {
        AttemptDirective::Fail(failure) => failure,
        directive => panic!("expected quota failure, got {directive:?}"),
    };
    assert_eq!(quota_failure.failure_class(), FailureClass::Quota);
    assert!(!quota_failure.retryable());
    assert_eq!(quota.snapshot().posts, 1);

    let mut unknown = AttemptController::new(context(RouteMode::Responses), false);
    unknown.begin_post().unwrap();
    let unknown_failure = match unknown.observe(http(429, None, None)).unwrap() {
        AttemptDirective::Fail(failure) => failure,
        directive => panic!("expected unknown-rate failure, got {directive:?}"),
    };
    assert_eq!(unknown_failure.failure_class(), FailureClass::RateLimit);
    assert!(!unknown_failure.retryable());
    assert_eq!(unknown.snapshot().posts, 1);
}

#[test]
fn exact_first_post_lite_capability_can_repair_once_without_retry_budget() {
    let mut controller = AttemptController::new(context(RouteMode::ResponsesLite), true);
    controller.begin_post().unwrap();
    let observation = FailureObservation::Http {
        status: 400,
        rate_kind: None,
        retry_after_seconds: None,
        request_id: None,
        error_code: ErrorCode::UnsupportedValue,
        error_param: ErrorParam::ToolChoice,
    };
    assert_eq!(
        controller.observe(observation.clone()).unwrap(),
        AttemptDirective::RepairOnce(RepairKind::OmitAutomaticToolChoice)
    );
    controller.begin_post().unwrap();
    assert!(matches!(
        controller.observe(observation).unwrap(),
        AttemptDirective::Fail(_)
    ));
    assert_eq!(controller.snapshot().posts, 2);
    assert_eq!(controller.snapshot().repairs, 1);
    assert!(controller.snapshot().delays_ms.is_empty());
    assert!(controller.begin_post().is_err());
}

#[test]
fn repair_is_forbidden_after_retry_or_response_start() {
    let capability = FailureObservation::Http {
        status: 400,
        rate_kind: None,
        retry_after_seconds: None,
        request_id: None,
        error_code: ErrorCode::UnsupportedValue,
        error_param: ErrorParam::ToolChoice,
    };

    let mut after_retry = AttemptController::new(context(RouteMode::ResponsesLite), true);
    after_retry.begin_post().unwrap();
    assert!(matches!(
        after_retry.observe(http(500, None, None)).unwrap(),
        AttemptDirective::RetryAfter(500)
    ));
    after_retry.begin_post().unwrap();
    assert!(matches!(
        after_retry.observe(capability.clone()).unwrap(),
        AttemptDirective::Fail(_)
    ));

    let mut after_start = AttemptController::new(context(RouteMode::ResponsesLite), true);
    after_start.begin_post().unwrap();
    after_start.mark_response_started().unwrap();
    assert!(matches!(
        after_start
            .observe(FailureObservation::Protocol(ProtocolKind::InvalidResponse))
            .unwrap(),
        AttemptDirective::Fail(_)
    ));
    assert!(after_start.begin_post().is_err());
}

#[test]
fn cancellation_is_terminal_from_ready_retry_and_inflight() {
    for mut controller in [
        AttemptController::new(context(RouteMode::Responses), false),
        {
            let mut value = AttemptController::new(context(RouteMode::Responses), false);
            value.begin_post().unwrap();
            value.observe(http(500, None, None)).unwrap();
            value
        },
        {
            let mut value = AttemptController::new(context(RouteMode::Responses), false);
            value.begin_post().unwrap();
            value
        },
    ] {
        assert_eq!(
            controller.observe(FailureObservation::Cancelled).unwrap(),
            AttemptDirective::Cancel
        );
        assert!(controller.begin_post().is_err());
    }
}

#[test]
fn completed_and_cancelled_diagnostics_omit_failure_fields() {
    let mut completed = AttemptController::new(context(RouteMode::Responses), false);
    completed.begin_post().unwrap();
    completed.mark_response_started().unwrap();
    assert_eq!(
        serde_json::to_value(completed.completed_diagnostic().unwrap()).unwrap(),
        serde_json::json!({
            "outcome":"completed", "provider":"codex", "route":"responses",
            "correlation_id":"corr-0001", "posts":1, "repairs":0, "delays_ms":[]
        })
    );

    let mut cancelled = AttemptController::new(context(RouteMode::Responses), false);
    cancelled.begin_post().unwrap();
    cancelled.observe(FailureObservation::Cancelled).unwrap();
    assert_eq!(
        serde_json::to_value(cancelled.cancelled_diagnostic().unwrap()).unwrap(),
        serde_json::json!({
            "outcome":"cancelled", "provider":"codex", "route":"responses",
            "correlation_id":"corr-0001", "posts":1, "repairs":0, "delays_ms":[]
        })
    );
}

#[test]
fn final_diagnostics_are_single_use_and_phase_checked() {
    let mut failed = AttemptController::new(context(RouteMode::Responses), false);
    failed.begin_post().unwrap();
    let AttemptDirective::Fail(failure) = failed.observe(http(422, None, None)).unwrap() else {
        panic!("permanent rejection must fail");
    };
    assert!(failed.failed_diagnostic(&failure).is_ok());
    assert_eq!(
        failed.failed_diagnostic(&failure),
        Err(TransitionError::FinalizationNotAuthorized)
    );
    assert_eq!(
        failed.completed_diagnostic(),
        Err(TransitionError::FinalizationNotAuthorized)
    );
    assert_eq!(
        failed.cancelled_diagnostic(),
        Err(TransitionError::FinalizationNotAuthorized)
    );

    let mut in_flight = AttemptController::new(context(RouteMode::Responses), false);
    in_flight.begin_post().unwrap();
    assert_eq!(
        in_flight.completed_diagnostic(),
        Err(TransitionError::FinalizationNotAuthorized)
    );
    assert_eq!(
        in_flight.cancelled_diagnostic(),
        Err(TransitionError::FinalizationNotAuthorized)
    );

    let mut completed = AttemptController::new(context(RouteMode::Responses), false);
    completed.begin_post().unwrap();
    completed.mark_response_started().unwrap();
    assert!(completed.completed_diagnostic().is_ok());
    assert_eq!(
        completed.completed_diagnostic(),
        Err(TransitionError::FinalizationNotAuthorized)
    );

    let mut cancelled = AttemptController::new(context(RouteMode::Responses), false);
    cancelled.begin_post().unwrap();
    cancelled.observe(FailureObservation::Cancelled).unwrap();
    assert!(cancelled.cancelled_diagnostic().is_ok());
    assert_eq!(
        cancelled.cancelled_diagnostic(),
        Err(TransitionError::FinalizationNotAuthorized)
    );
}
