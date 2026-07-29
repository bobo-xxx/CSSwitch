use super::*;

fn context(route: RouteMode) -> RouteContext {
    RouteContext::codex(route, CorrelationId::new("corr-0001").unwrap())
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
