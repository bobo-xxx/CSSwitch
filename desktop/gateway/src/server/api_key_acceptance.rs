use std::cell::RefCell;
use std::collections::{BTreeMap, VecDeque};
use std::io::{ErrorKind, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use super::{handle_messages, CodexComponents, RequestNonceGenerator};
use crate::config::{GatewayConfig, GatewayIntent};
use crate::messages::AttemptMode;
use crate::models;
use crate::provider_contracts;
use crate::provider_failure::{AttemptDiagnostic, ErrorCode};
use crate::static_profile::StaticProfileResolver;

const RESERVED_PORTS: [u16; 6] = [2999, 8765, 9002, 9003, 11434, 11535];
const API_KEY_SENTINEL: &str = "fixture-api-key";
const SECRET_UPSTREAM_TEXT: &str = "secret upstream text";
const PRIVATE_HOST_SENTINEL: &str = "private.example";
const MAX_REQUEST_HEADER_BYTES: usize = 16 * 1024;
const MAX_REQUEST_BODY_BYTES: usize = 256 * 1024;
const MAX_REQUEST_TOTAL_BYTES: usize = MAX_REQUEST_HEADER_BYTES + MAX_REQUEST_BODY_BYTES;

thread_local! {
    static API_KEY_ACCEPTANCE_RECORDING: RefCell<Option<RecordingState>> = const { RefCell::new(None) };
}

#[derive(Default)]
struct RecordingState {
    delays_ms: Vec<u64>,
    diagnostics: Vec<Value>,
}

pub(super) fn record_attempt_delay(delay_ms: u64) {
    API_KEY_ACCEPTANCE_RECORDING.with(|slot| {
        if let Some(state) = slot.borrow_mut().as_mut() {
            state.delays_ms.push(delay_ms);
        }
    });
}

pub(super) fn record_attempt_diagnostic(diagnostic: AttemptDiagnostic) {
    API_KEY_ACCEPTANCE_RECORDING.with(|slot| {
        if let Some(state) = slot.borrow_mut().as_mut() {
            state
                .diagnostics
                .push(serde_json::to_value(diagnostic).expect("serialize attempt diagnostic"));
        }
    });
}

fn start_recording() {
    API_KEY_ACCEPTANCE_RECORDING.with(|slot| {
        *slot.borrow_mut() = Some(RecordingState::default());
    });
}

fn finish_recording() -> RecordingState {
    API_KEY_ACCEPTANCE_RECORDING.with(|slot| {
        slot.borrow_mut()
            .take()
            .expect("API-key acceptance recording was active")
    })
}

#[derive(Clone, Copy, Debug)]
enum FixtureRoute {
    RelayAnthropic,
    RelayKimi,
    DeepseekAnthropic,
}

#[derive(Clone, Copy, Debug)]
enum OpenedFixture {
    MalformedJson,
    PartialSseThenClose,
}

#[derive(Clone, Debug)]
enum UpstreamStep {
    Http {
        status: u16,
        reason: &'static str,
        content_type: &'static str,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    },
    PartialSseThenDrop(Vec<u8>),
}

impl UpstreamStep {
    fn json(status: u16, reason: &'static str, body: Value) -> Self {
        Self::json_with_headers(status, reason, body, Vec::new())
    }

    fn json_with_headers(
        status: u16,
        reason: &'static str,
        body: Value,
        headers: Vec<(&str, &str)>,
    ) -> Self {
        Self::Http {
            status,
            reason,
            content_type: "application/json",
            headers: headers
                .into_iter()
                .map(|(name, value)| (name.to_owned(), value.to_owned()))
                .collect(),
            body: serde_json::to_vec(&body).expect("serialize synthetic upstream JSON"),
        }
    }

    fn bytes(status: u16, reason: &'static str, content_type: &'static str, body: Vec<u8>) -> Self {
        Self::Http {
            status,
            reason,
            content_type,
            headers: Vec::new(),
            body,
        }
    }
}

#[derive(Clone, Debug)]
struct CapturedRequest {
    method: String,
    path: String,
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
}

#[derive(Default)]
struct SharedScript {
    steps: VecDeque<UpstreamStep>,
    requests: Vec<CapturedRequest>,
    unexpected_posts: usize,
    unexpected_methods: usize,
    rejected_requests: usize,
}

struct ScriptResult {
    requests: Vec<CapturedRequest>,
    remaining_steps: usize,
    unexpected_posts: usize,
    unexpected_methods: usize,
    rejected_requests: usize,
}

struct ScriptedUpstream {
    address: SocketAddr,
    shared: Arc<Mutex<SharedScript>>,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl ScriptedUpstream {
    fn start(steps: Vec<UpstreamStep>) -> Self {
        let listener = bind_loopback();
        listener
            .set_nonblocking(true)
            .expect("make scripted upstream nonblocking");
        let address = listener.local_addr().expect("read upstream address");
        let shared = Arc::new(Mutex::new(SharedScript {
            steps: steps.into(),
            ..SharedScript::default()
        }));
        let stop = Arc::new(AtomicBool::new(false));
        let shared_for_thread = Arc::clone(&shared);
        let stop_for_thread = Arc::clone(&stop);
        let thread = thread::spawn(move || {
            while !stop_for_thread.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let step = match read_request(&mut stream) {
                            Ok(request) => {
                                let mut state =
                                    shared_for_thread.lock().expect("lock upstream script");
                                let is_post = request.method == "POST";
                                state.requests.push(request);
                                if !is_post {
                                    state.unexpected_methods += 1;
                                    UpstreamStep::json(405, "Method Not Allowed", json!({}))
                                } else {
                                    state.steps.pop_front().unwrap_or_else(|| {
                                        state.unexpected_posts += 1;
                                        UpstreamStep::json(
                                            500,
                                            "Unexpected Post",
                                            json!({"error":{"message":"unexpected post"}}),
                                        )
                                    })
                                }
                            }
                            Err(_) => {
                                shared_for_thread
                                    .lock()
                                    .expect("lock upstream script")
                                    .rejected_requests += 1;
                                UpstreamStep::json(400, "Bad Request", json!({}))
                            }
                        };
                        write_step(&mut stream, step);
                    }
                    Err(error) if error.kind() == ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("scripted upstream accept failed: {error}"),
                }
            }
        });
        Self {
            address,
            shared,
            stop,
            thread: Some(thread),
        }
    }

    fn endpoint(&self) -> String {
        format!("http://{}/v1/messages", self.address)
    }

    fn finish(mut self) -> ScriptResult {
        self.stop.store(true, Ordering::Release);
        self.thread
            .take()
            .expect("upstream thread")
            .join()
            .expect("join upstream thread");
        let state = self.shared.lock().expect("lock final script");
        ScriptResult {
            requests: state.requests.clone(),
            remaining_steps: state.steps.len(),
            unexpected_posts: state.unexpected_posts,
            unexpected_methods: state.unexpected_methods,
            rejected_requests: state.rejected_requests,
        }
    }
}

impl Drop for ScriptedUpstream {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

struct AcceptanceResult {
    downstream: Vec<u8>,
    script: ScriptResult,
    delays_ms: Vec<u64>,
    diagnostics: Vec<Value>,
}

impl AcceptanceResult {
    fn status(&self) -> u16 {
        String::from_utf8_lossy(&self.downstream)
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .and_then(|status| status.parse().ok())
            .expect("downstream status")
    }

    fn body(&self) -> &[u8] {
        let start = self
            .downstream
            .windows(4)
            .position(|part| part == b"\r\n\r\n")
            .expect("downstream HTTP headers");
        &self.downstream[start + 4..]
    }

    fn json(&self) -> Value {
        serde_json::from_slice(self.body()).expect("downstream JSON")
    }

    fn text(&self) -> String {
        String::from_utf8_lossy(&self.downstream).into_owned()
    }

    fn sse_body(&self) -> Vec<u8> {
        let head_end = self
            .downstream
            .windows(4)
            .position(|part| part == b"\r\n\r\n")
            .expect("downstream stream headers");
        decode_chunked_body(&self.downstream[head_end + 4..])
    }
}

fn bind_loopback() -> TcpListener {
    loop {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind loopback fixture");
        let port = listener
            .local_addr()
            .expect("read loopback fixture address")
            .port();
        if !RESERVED_PORTS.contains(&port) {
            return listener;
        }
    }
}

fn read_request(stream: &mut TcpStream) -> Result<CapturedRequest, ()> {
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .map_err(|_| ())?;
    let mut raw = Vec::new();
    let mut buffer = [0_u8; 1024];
    loop {
        let read = stream.read(&mut buffer).map_err(|_| ())?;
        if read == 0 {
            return Err(());
        }
        raw.extend_from_slice(&buffer[..read]);
        if raw.len() > MAX_REQUEST_TOTAL_BYTES {
            return Err(());
        }
        if let Some(head_end) = raw.windows(4).position(|part| part == b"\r\n\r\n") {
            if head_end > MAX_REQUEST_HEADER_BYTES {
                return Err(());
            }
            let head = std::str::from_utf8(&raw[..head_end]).map_err(|_| ())?;
            let content_length = head
                .lines()
                .skip(1)
                .filter_map(|line| line.split_once(':'))
                .find_map(|(name, value)| {
                    name.eq_ignore_ascii_case("content-length")
                        .then_some(value.trim())
                })
                .map(|value| value.parse::<usize>().map_err(|_| ()))
                .transpose()?
                .unwrap_or(0);
            if content_length > MAX_REQUEST_BODY_BYTES {
                return Err(());
            }
            let expected = head_end + 4 + content_length;
            if raw.len() >= expected {
                let mut lines = head.lines();
                let mut request_line = lines.next().ok_or(())?.split_whitespace();
                let method = request_line.next().ok_or(())?.to_owned();
                let path = request_line.next().ok_or(())?.to_owned();
                let headers = lines
                    .filter_map(|line| line.split_once(':'))
                    .map(|(name, value)| (name.to_ascii_lowercase(), value.trim().to_owned()))
                    .filter(|(name, _)| {
                        matches!(
                            name.as_str(),
                            "accept" | "content-type" | "anthropic-version"
                        )
                    })
                    .collect();
                return Ok(CapturedRequest {
                    method,
                    path,
                    headers,
                    body: raw[head_end + 4..expected].to_vec(),
                });
            }
        }
    }
}

fn write_step(stream: &mut TcpStream, step: UpstreamStep) {
    match step {
        UpstreamStep::Http {
            status,
            reason,
            content_type,
            headers,
            body,
        } => {
            write!(
                stream,
                "HTTP/1.1 {status} {reason}\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\n",
                body.len()
            )
            .expect("write response head");
            for (name, value) in headers {
                write!(stream, "{name}: {value}\r\n").expect("write response header");
            }
            stream
                .write_all(b"connection: close\r\n\r\n")
                .expect("finish response head");
            stream.write_all(&body).expect("write response body");
            stream.flush().expect("flush response");
        }
        UpstreamStep::PartialSseThenDrop(body) => {
            write!(
                stream,
                "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                body.len() + 128
            )
            .expect("write partial SSE head");
            stream.write_all(&body).expect("write partial SSE body");
            stream.flush().expect("flush partial SSE");
        }
    }
}

fn capture_downstream(handler: impl FnOnce(&mut TcpStream)) -> Vec<u8> {
    let listener = bind_loopback();
    let address = listener.local_addr().expect("read downstream address");
    let reader = thread::spawn(move || {
        let mut stream = TcpStream::connect(address).expect("connect downstream reader");
        let mut response = Vec::new();
        stream
            .read_to_end(&mut response)
            .expect("read downstream response");
        response
    });
    let (mut stream, _) = listener.accept().expect("accept downstream");
    handler(&mut stream);
    drop(stream);
    reader.join().expect("join downstream reader")
}

fn fingerprint_text(digest: &mut Sha256, value: &str) {
    digest.update((value.len() as u32).to_be_bytes());
    digest.update(value.as_bytes());
}

fn resolver(adapter: &str, upstream_model: &str) -> StaticProfileResolver {
    let selector = format!("claude-csswitch-{adapter}-acceptance-00000001");
    let mut digest = Sha256::new();
    digest.update(b"csswitch-static-catalog-fp-v1\0");
    digest.update(1_u32.to_be_bytes());
    fingerprint_text(&mut digest, adapter);
    fingerprint_text(&mut digest, &selector);
    digest.update(1_u32.to_be_bytes());
    fingerprint_text(&mut digest, &selector);
    fingerprint_text(&mut digest, "Acceptance");
    fingerprint_text(&mut digest, upstream_model);
    digest.update([2]);
    fingerprint_text(&mut digest, "none");
    digest.update([0, 0, 0]);
    for _ in 0..4 {
        fingerprint_text(&mut digest, &selector);
    }
    digest.update(1_u32.to_be_bytes());
    fingerprint_text(&mut digest, "claude-sonnet-4-20250514");
    fingerprint_text(&mut digest, &selector);
    let fp = digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let value = json!({
        "schema_version": 1,
        "adapter": adapter,
        "catalog_fp": fp,
        "default_selector_id": selector,
        "routes": [{
            "selector_id": selector,
            "display_name": "Acceptance",
            "upstream_model": upstream_model,
            "supports_tools": true
        }],
        "role_bindings": {
            "sonnet": selector,
            "opus": selector,
            "haiku": selector,
            "fable": selector
        },
        "legacy_aliases": [{
            "alias": "claude-sonnet-4-20250514",
            "selector_id": selector
        }]
    });
    StaticProfileResolver::from_json(&value.to_string()).expect("build static resolver")
}

fn config(route: FixtureRoute, upstream_url: String) -> GatewayConfig {
    let (provider, contract_id, upstream_model, relay_thinking) = match route {
        FixtureRoute::DeepseekAnthropic => ("deepseek", "deepseek-native", "deepseek-chat", None),
        FixtureRoute::RelayAnthropic => ("relay", "anthropic-relay", "relay-claude", None),
        FixtureRoute::RelayKimi => (
            "relay",
            "kimi-anthropic-relay",
            "kimi-k2.7-code",
            Some("enabled".to_owned()),
        ),
    };
    let digest = provider_contract_catalog_digest();
    let contract =
        provider_contracts::load_runtime_contract(provider, Some(contract_id), Some(&digest))
            .expect("load provider contract");
    GatewayConfig {
        provider: provider.to_owned(),
        port: 0,
        auth_secret: None,
        api_key: Some(API_KEY_SENTINEL.to_owned()),
        upstream_url,
        models_url: None,
        relay_thinking,
        provider_contract: Some(contract),
        intent: GatewayIntent::Formal,
        static_model_resolver: Some(resolver(provider, upstream_model)),
        shim_mode: if matches!(route, FixtureRoute::DeepseekAnthropic) {
            "rewrite".to_owned()
        } else {
            "off".to_owned()
        },
        codex_state_root: None,
        codex_contract: None,
        launch_id: "api-key-acceptance".to_owned(),
        skill_data_dir: None,
        skill_bridge_dir: None,
        skill_bridge_token: None,
        science_host_context: None,
    }
}

fn provider_contract_catalog_digest() -> String {
    let text = include_str!("../../../../catalog/provider-contracts.v1.json");
    format!("{:x}", Sha256::digest(text.as_bytes()))
}

fn request(mode: AttemptMode, include_tools: bool) -> Value {
    let mut value = json!({
        "model": "claude-sonnet-4-20250514",
        "max_tokens": 128,
        "stream": matches!(mode, AttemptMode::Stream),
        "messages": [{"role": "user", "content": "hello"}]
    });
    if include_tools {
        value["tools"] = json!([{
            "name": "web_search",
            "description": "synthetic search",
            "input_schema": {"type":"object","properties":{"query":{"type":"string"}}}
        }]);
    }
    value
}

fn run(route: FixtureRoute, mode: AttemptMode, steps: Vec<UpstreamStep>) -> AcceptanceResult {
    let upstream = ScriptedUpstream::start(steps);
    let cfg = config(route, upstream.endpoint());
    let request_nonces = RequestNonceGenerator::with_prefix([0x42; 16]);
    let relay_models = models::RelayModelCache::default();
    start_recording();
    let downstream = capture_downstream(|stream| {
        handle_messages(
            stream,
            &cfg,
            serde_json::to_vec(&request(
                mode,
                matches!(route, FixtureRoute::DeepseekAnthropic),
            ))
            .expect("serialize caller request"),
            Some(&request_nonces),
            &relay_models,
            CodexComponents::default(),
        );
    });
    let recording = finish_recording();
    AcceptanceResult {
        downstream,
        script: upstream.finish(),
        delays_ms: recording.delays_ms,
        diagnostics: recording.diagnostics,
    }
}

fn failure_body(status: u16, error_code: ErrorCode) -> Value {
    let code = match error_code {
        ErrorCode::InsufficientQuota => "insufficient_quota",
        ErrorCode::RateLimitError => "rate_limit_error",
        ErrorCode::RateLimitExceeded => "rate_limit_exceeded",
        ErrorCode::Absent => "",
        ErrorCode::UnsupportedValue | ErrorCode::Other => "other",
    };
    let mut error = json!({
        "type": "api_error",
        "message": SECRET_UPSTREAM_TEXT,
        "request_id": "Req-secret-01",
        "host": PRIVATE_HOST_SENTINEL
    });
    if !code.is_empty() {
        error["code"] = json!(code);
    }
    json!({"error": error, "status": status})
}

fn assert_script(result: &AcceptanceResult, expected_posts: usize) {
    assert_eq!(result.script.requests.len(), expected_posts);
    assert_eq!(result.script.remaining_steps, 0);
    assert_eq!(result.script.unexpected_posts, 0);
    assert_eq!(result.script.unexpected_methods, 0);
    assert_eq!(result.script.rejected_requests, 0);
    for request in &result.script.requests {
        assert_eq!(request.method, "POST");
        assert_eq!(request.path, "/v1/messages");
        assert!(!request.headers.contains_key("authorization"));
        assert!(!request.headers.contains_key("x-api-key"));
        assert!(!String::from_utf8_lossy(&request.body).contains(API_KEY_SENTINEL));
    }
    if let Some((first, rest)) = result.script.requests.split_first() {
        for request in rest {
            assert_eq!(request.body, first.body, "retried body bytes changed");
        }
    }
}

fn assert_one_final_diagnostic(result: &AcceptanceResult, expected_outcome: &str) {
    assert_eq!(
        result.diagnostics.len(),
        1,
        "exactly one final diagnostic required"
    );
    assert_eq!(result.diagnostics[0]["outcome"], expected_outcome);
}

fn assert_failure_response(result: &AcceptanceResult, status: u16) {
    let expected_status = if status == 409 { 502 } else { status };
    assert_eq!(result.status(), expected_status);
    let body = result.json();
    assert_eq!(body["type"], "error");
    assert_eq!(body["error"]["route"], "anthropic_messages");
    assert!(body["error"]["provider"].as_str().is_some());
    assert!(body["error"]["correlation_id"].as_str().is_some());
    assert!(body["error"]["message"]
        .as_str()
        .is_some_and(|text| !text.is_empty()));
}

fn assert_terminal_http(
    route: FixtureRoute,
    mode: AttemptMode,
    status: u16,
    error_code: ErrorCode,
    expected_posts: usize,
) {
    let result = run(
        route,
        mode,
        vec![UpstreamStep::json(
            status,
            "Synthetic Failure",
            failure_body(status, error_code),
        )],
    );
    assert_script(&result, expected_posts);
    assert!(result.delays_ms.is_empty());
    assert_failure_response(&result, status);
    assert_one_final_diagnostic(&result, "failed");
}

fn assert_retry_then_success(
    route: FixtureRoute,
    mode: AttemptMode,
    first_status: u16,
    retry_after_seconds: Option<u64>,
    expected_delay_ms: u64,
) {
    let headers = retry_after_seconds
        .map(|value| {
            vec![(
                "retry-after",
                Box::leak(value.to_string().into_boxed_str()) as &str,
            )]
        })
        .unwrap_or_default();
    let result = run(
        route,
        mode,
        vec![
            UpstreamStep::json_with_headers(
                first_status,
                "Too Many Requests",
                failure_body(first_status, ErrorCode::RateLimitError),
                headers,
            ),
            success_step(route, mode),
        ],
    );
    assert_script(&result, 2);
    assert_eq!(result.delays_ms, vec![expected_delay_ms]);
    assert_success_body(route, mode, &result);
    assert_one_final_diagnostic(&result, "completed");
}

fn assert_exhausted(
    route: FixtureRoute,
    mode: AttemptMode,
    status: u16,
    expected_delays_ms: &[u64],
) {
    let result = run(
        route,
        mode,
        vec![
            UpstreamStep::json(
                status,
                "Unavailable",
                failure_body(status, ErrorCode::RateLimitError),
            ),
            UpstreamStep::json(
                status,
                "Unavailable",
                failure_body(status, ErrorCode::RateLimitError),
            ),
            UpstreamStep::json(
                status,
                "Unavailable",
                failure_body(status, ErrorCode::RateLimitError),
            ),
        ],
    );
    assert_script(&result, 3);
    assert_eq!(result.delays_ms, expected_delays_ms);
    assert_failure_response(&result, 502);
    assert_one_final_diagnostic(&result, "failed");
}

fn assert_opened_failure(route: FixtureRoute, mode: AttemptMode, opened_fixture: OpenedFixture) {
    let step = match opened_fixture {
        OpenedFixture::MalformedJson => {
            UpstreamStep::bytes(200, "OK", "application/json", b"{not-json".to_vec())
        }
        OpenedFixture::PartialSseThenClose => {
            UpstreamStep::PartialSseThenDrop(b"event: message_start\n".to_vec())
        }
    };
    let result = run(route, mode, vec![step]);
    assert_script(&result, 1);
    assert!(result.delays_ms.is_empty());
    assert_one_final_diagnostic(&result, "failed");
}

fn assert_success_fixture(route: FixtureRoute, mode: AttemptMode, _fixture_name: &str) {
    let result = run(route, mode, vec![success_step(route, mode)]);
    assert_script(&result, 1);
    assert!(result.delays_ms.is_empty());
    assert_success_body(route, mode, &result);
    assert_one_final_diagnostic(&result, "completed");
}

fn assert_redacted(route: FixtureRoute, sentinels: &[&str]) {
    let result = run(
        route,
        AttemptMode::Nonstream,
        vec![
            UpstreamStep::json(
                503,
                "Unavailable",
                failure_body(503, ErrorCode::RateLimitError),
            ),
            UpstreamStep::json(
                503,
                "Unavailable",
                failure_body(503, ErrorCode::RateLimitError),
            ),
            UpstreamStep::json(
                503,
                "Unavailable",
                failure_body(503, ErrorCode::RateLimitError),
            ),
        ],
    );
    assert_script(&result, 3);
    let text = result.text();
    for sentinel in sentinels {
        assert!(
            !text.contains(sentinel),
            "downstream/diagnostic leaked sentinel {sentinel}: {text}"
        );
    }
    assert_one_final_diagnostic(&result, "failed");
}

fn success_step(route: FixtureRoute, mode: AttemptMode) -> UpstreamStep {
    match mode {
        AttemptMode::Nonstream => {
            let body = if matches!(route, FixtureRoute::RelayKimi) {
                json!({
                    "id": "msg_kimi",
                    "type": "message",
                    "role": "assistant",
                    "model": "kimi-k2.7-code",
                    "content": [
                        {"type":"thinking","thinking":"","signature":""},
                        {"type":"text","text":"kimi ok"}
                    ],
                    "stop_reason": "end_turn",
                    "stop_sequence": null,
                    "usage": {"input_tokens": 1, "output_tokens": 2}
                })
            } else {
                json!({
                    "id": "msg_ok",
                    "type": "message",
                    "role": "assistant",
                    "model": "upstream",
                    "content": [{"type":"text","text":"hello <｜DSML｜tool_calls> <｜DSML｜invoke name=\"web_search\"><｜DSML｜parameter name=\"query\">q</｜DSML｜parameter></｜DSML｜invoke> </｜DSML｜tool_calls> done"}],
                    "stop_reason": "end_turn",
                    "stop_sequence": null,
                    "usage": {"input_tokens": 1, "output_tokens": 2}
                })
            };
            UpstreamStep::json(200, "OK", body)
        }
        AttemptMode::Stream => UpstreamStep::bytes(
            200,
            "OK",
            "text/event-stream",
            b"event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_stream\",\"type\":\"message\",\"role\":\"assistant\",\"model\":\"upstream\",\"content\":[],\"stop_reason\":null,\"stop_sequence\":null,\"usage\":{\"input_tokens\":1,\"output_tokens\":0}}}\n\nevent: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\nevent: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"stream ok\"}}\n\nevent: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\nevent: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\",\"stop_sequence\":null},\"usage\":{\"output_tokens\":2}}\n\nevent: message_stop\ndata: {\"type\":\"message_stop\"}\n\n".to_vec(),
        ),
    }
}

fn assert_success_body(route: FixtureRoute, mode: AttemptMode, result: &AcceptanceResult) {
    assert_eq!(result.status(), 200);
    match mode {
        AttemptMode::Nonstream => {
            let body = result.json();
            assert_eq!(body["type"], "message");
            let text = serde_json::to_string(&body).expect("serialize downstream JSON");
            if matches!(route, FixtureRoute::DeepseekAnthropic) {
                assert!(text.contains("tool_use"));
                assert!(
                    text.contains("toolu_dsml_424242424242424242424242424242420000000000000001_1")
                );
            }
            if matches!(route, FixtureRoute::RelayKimi) {
                assert!(!text.contains("\"type\":\"thinking\""));
                assert!(text.contains("kimi ok"));
            }
        }
        AttemptMode::Stream => {
            let text = String::from_utf8(result.sse_body()).expect("SSE UTF-8");
            assert!(text.contains("message_start"));
            assert!(text.contains("message_stop"));
        }
    }
}

fn decode_chunked_body(mut encoded: &[u8]) -> Vec<u8> {
    let mut decoded = Vec::new();
    loop {
        let line_end = encoded
            .windows(2)
            .position(|part| part == b"\r\n")
            .expect("chunk size terminator");
        let size_text = std::str::from_utf8(&encoded[..line_end]).expect("ASCII chunk size");
        let size = usize::from_str_radix(size_text, 16).expect("hex chunk size");
        encoded = &encoded[line_end + 2..];
        if size == 0 {
            break;
        }
        assert!(encoded.len() >= size + 2);
        decoded.extend_from_slice(&encoded[..size]);
        encoded = &encoded[size + 2..];
    }
    decoded
}

#[test]
fn api_contract_anthropic_permanent_auth_and_quota_are_one_post() {
    for status in [400, 401, 403, 422] {
        assert_terminal_http(
            FixtureRoute::RelayAnthropic,
            AttemptMode::Nonstream,
            status,
            ErrorCode::Absent,
            1,
        );
    }
    assert_terminal_http(
        FixtureRoute::RelayAnthropic,
        AttemptMode::Nonstream,
        429,
        ErrorCode::InsufficientQuota,
        1,
    );
}

#[test]
fn api_contract_anthropic_rate_and_5xx_are_bounded_and_byte_identical() {
    assert_retry_then_success(
        FixtureRoute::RelayAnthropic,
        AttemptMode::Nonstream,
        429,
        None,
        500,
    );
    assert_retry_then_success(
        FixtureRoute::DeepseekAnthropic,
        AttemptMode::Nonstream,
        429,
        Some(90),
        60_000,
    );
    assert_exhausted(
        FixtureRoute::DeepseekAnthropic,
        AttemptMode::Nonstream,
        503,
        &[500, 1_000],
    );
}

#[test]
fn api_contract_anthropic_409_is_terminal() {
    assert_terminal_http(
        FixtureRoute::RelayAnthropic,
        AttemptMode::Nonstream,
        409,
        ErrorCode::Absent,
        1,
    );
}

#[test]
fn api_contract_anthropic_opened_or_partial_stream_never_replays() {
    assert_opened_failure(
        FixtureRoute::DeepseekAnthropic,
        AttemptMode::Nonstream,
        OpenedFixture::MalformedJson,
    );
    assert_opened_failure(
        FixtureRoute::RelayAnthropic,
        AttemptMode::Stream,
        OpenedFixture::PartialSseThenClose,
    );
}

#[test]
fn api_contract_anthropic_kimi_and_deepseek_success_fixtures_are_unchanged() {
    assert_success_fixture(
        FixtureRoute::RelayKimi,
        AttemptMode::Nonstream,
        "kimi-nonstream-filter",
    );
    assert_success_fixture(
        FixtureRoute::RelayKimi,
        AttemptMode::Stream,
        "kimi-stream-filter",
    );
    assert_success_fixture(
        FixtureRoute::DeepseekAnthropic,
        AttemptMode::Nonstream,
        "deepseek-dsml-nonstream",
    );
    assert_success_fixture(
        FixtureRoute::DeepseekAnthropic,
        AttemptMode::Stream,
        "deepseek-dsml-stream",
    );
}

#[test]
fn api_contract_anthropic_failures_and_diagnostics_are_redacted() {
    assert_redacted(
        FixtureRoute::RelayAnthropic,
        &[
            "fixture-api-key",
            "secret upstream text",
            "private.example",
            "Req-secret-01",
        ],
    );
    assert_redacted(
        FixtureRoute::DeepseekAnthropic,
        &[
            "fixture-api-key",
            "secret upstream text",
            "private.example",
            "Req-secret-02",
        ],
    );
}
