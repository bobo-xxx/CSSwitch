use std::collections::{BTreeMap, VecDeque};
use std::io::{ErrorKind, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use serde_json::Value;

use super::{handle_codex_messages_with_policy, CodexRequestPolicy};
use crate::codex_auth::InferenceSecrets;
use crate::codex_transport::CodexTransport;

const ACCESS_SENTINEL: &str = "ACCEPTANCE_ACCESS_TOKEN_SENTINEL";
const ACCOUNT_SENTINEL: &str = "ACCEPTANCE_ACCOUNT_SENTINEL";
const BODY_SENTINEL: &str = "UPSTREAM_BODY_SECRET_SENTINEL";
const COOKIE_SENTINEL: &str = "UPSTREAM_COOKIE_SECRET_SENTINEL";
const URL_SENTINEL: &str = "PRIVATE_UPSTREAM_PATH_SENTINEL";
const MAX_REQUEST_HEADER_BYTES: usize = 16 * 1024;
const MAX_REQUEST_BODY_BYTES: usize = 256 * 1024;
const MAX_REQUEST_TOTAL_BYTES: usize = MAX_REQUEST_HEADER_BYTES + MAX_REQUEST_BODY_BYTES;
const CAPTURED_REQUEST_HEADERS: [&str; 2] = ["accept", "content-type"];

struct AcceptanceCase {
    request: Value,
    is_stream: bool,
    use_responses_lite: bool,
    endpoint_path: &'static str,
    steps: Vec<UpstreamStep>,
}

struct AcceptanceResult {
    downstream: Vec<u8>,
    script: ScriptResult,
    auth_rejections: Vec<u16>,
}

impl AcceptanceResult {
    fn status(&self) -> u16 {
        String::from_utf8_lossy(&self.downstream)
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .and_then(|status| status.parse().ok())
            .expect("downstream HTTP status")
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
        let head = String::from_utf8_lossy(&self.downstream[..head_end]);
        assert!(head.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(head.contains("content-type: text/event-stream\r\n"));
        assert!(head.contains("transfer-encoding: chunked\r\n"));
        decode_chunked_body(&self.downstream[head_end + 4..])
    }

    fn sse_text(&self) -> String {
        String::from_utf8(self.sse_body()).expect("downstream SSE is UTF-8")
    }

    fn sse_events(&self) -> Vec<ParsedSseEvent> {
        parse_sse_events(&self.sse_body())
    }
}

#[derive(Debug, PartialEq)]
struct ParsedSseEvent {
    event: Option<String>,
    data: Value,
}

fn decode_chunked_body(mut encoded: &[u8]) -> Vec<u8> {
    let mut decoded = Vec::new();
    loop {
        let line_end = encoded
            .windows(2)
            .position(|part| part == b"\r\n")
            .expect("chunk size terminator");
        let size_text = std::str::from_utf8(&encoded[..line_end]).expect("ASCII chunk size");
        let size = usize::from_str_radix(size_text, 16).expect("hexadecimal chunk size");
        encoded = &encoded[line_end + 2..];
        if size == 0 {
            assert_eq!(encoded, b"\r\n", "terminal chunk must end the response");
            break;
        }
        assert!(encoded.len() >= size + 2, "complete chunk payload required");
        decoded.extend_from_slice(&encoded[..size]);
        assert_eq!(&encoded[size..size + 2], b"\r\n");
        encoded = &encoded[size + 2..];
    }
    decoded
}

fn parse_sse_events(body: &[u8]) -> Vec<ParsedSseEvent> {
    let text = std::str::from_utf8(body).expect("synthetic downstream SSE is UTF-8");
    text.split("\n\n")
        .filter(|block| !block.is_empty())
        .map(|block| {
            let mut event = None;
            let mut data = Vec::new();
            for line in block.lines() {
                if let Some(value) = line.strip_prefix("event:") {
                    event = Some(value.trim().to_string());
                } else if let Some(value) = line.strip_prefix("data:") {
                    data.push(value.trim());
                }
            }
            assert!(!data.is_empty(), "SSE event must carry JSON data");
            ParsedSseEvent {
                event,
                data: serde_json::from_str(&data.join("\n")).expect("SSE data JSON"),
            }
        })
        .collect()
}

fn assert_forbidden_sentinels_absent(text: &str) {
    for forbidden in [
        BODY_SENTINEL,
        COOKIE_SENTINEL,
        URL_SENTINEL,
        ACCESS_SENTINEL,
        ACCOUNT_SENTINEL,
    ] {
        assert!(!text.contains(forbidden), "forbidden sentinel leaked");
    }
}

fn assert_request_projection_safe(request: &CapturedRequest) {
    assert!(!request.headers.contains_key("authorization"));
    assert!(!request.headers.contains_key("chatgpt-account-id"));
    assert!(!request.headers.contains_key("x-private-sentinel"));
    assert!(request
        .headers
        .keys()
        .all(|name| CAPTURED_REQUEST_HEADERS.contains(&name.as_str())));
}

fn assert_sse_wire_response(response: &[u8], declared_length: usize, body: &[u8]) {
    let head_end = response
        .windows(4)
        .position(|part| part == b"\r\n\r\n")
        .expect("SSE response head");
    let head = String::from_utf8_lossy(&response[..head_end]);
    assert!(head.starts_with("HTTP/1.1 200 OK\r\n"));
    assert!(head.contains("content-type: text/event-stream\r\n"));
    assert!(head.contains(&format!("content-length: {declared_length}")));
    assert_eq!(&response[head_end + 4..], body);
}

fn assert_failure_envelope(
    result: &AcceptanceResult,
    status: u16,
    error_type: &str,
    route: &str,
    failure_class: &str,
    upstream_status: Option<u16>,
    retryable: bool,
) {
    assert_eq!(result.status(), status);
    let body = result.json();
    assert_eq!(body["type"], "error");
    assert_eq!(body["error"]["type"], error_type);
    assert!(body["error"]["message"]
        .as_str()
        .is_some_and(|value| !value.is_empty()));
    assert_eq!(body["error"]["provider"], "codex");
    assert_eq!(body["error"]["route"], route);
    assert_eq!(body["error"]["failure_class"], failure_class);
    let error = body["error"].as_object().expect("error object");
    match upstream_status {
        Some(upstream_status) => {
            assert_eq!(
                error["upstream_status"].as_u64(),
                Some(upstream_status.into())
            );
        }
        None => assert!(
            !error.contains_key("upstream_status"),
            "unknown upstream status must be absent, not null"
        ),
    }
    assert_eq!(body["error"]["retryable"], retryable);
    assert!(body["error"]["correlation_id"]
        .as_str()
        .is_some_and(|value| !value.is_empty()));
    assert!(body["error"]["recovery"]
        .as_str()
        .is_some_and(|value| !value.is_empty()));
    assert!(body["error"].get("attempt_count").is_none());
}

fn assert_optional_metadata_absent(result: &AcceptanceResult, fields: &[&str]) {
    let body = result.json();
    let error = body["error"].as_object().expect("error object");
    for field in fields {
        assert!(
            !error.contains_key(*field),
            "unknown optional field {field} must be absent, not null"
        );
    }
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
    Sse(Vec<u8>),
    Disconnect,
    PartialSseThenDrop {
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    },
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
                .map(|(name, value)| (name.to_string(), value.to_string()))
                .collect(),
            body: serde_json::to_vec(&body).expect("serialize synthetic upstream JSON"),
        }
    }

    fn partial_sse(body: Vec<u8>, headers: Vec<(&str, &str)>) -> Self {
        Self::PartialSseThenDrop {
            headers: headers
                .into_iter()
                .map(|(name, value)| (name.to_string(), value.to_string()))
                .collect(),
            body,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
struct CapturedRequest {
    method: String,
    path: String,
    headers: BTreeMap<String, String>,
    body: Value,
}

#[derive(Debug)]
struct ScriptResult {
    requests: Vec<CapturedRequest>,
    remaining_steps: usize,
    unexpected_posts: usize,
    unexpected_methods: usize,
    rejected_requests: usize,
}

#[derive(Default)]
struct SharedScript {
    steps: VecDeque<UpstreamStep>,
    requests: Vec<CapturedRequest>,
    unexpected_posts: usize,
    unexpected_methods: usize,
    rejected_requests: usize,
}

struct ScriptedCodexUpstream {
    address: SocketAddr,
    shared: Arc<Mutex<SharedScript>>,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl ScriptedCodexUpstream {
    fn start(steps: Vec<UpstreamStep>) -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind loopback upstream");
        listener
            .set_nonblocking(true)
            .expect("make loopback upstream nonblocking");
        let address = listener.local_addr().expect("read loopback address");
        let shared = Arc::new(Mutex::new(SharedScript {
            steps: steps.into(),
            requests: Vec::new(),
            unexpected_posts: 0,
            unexpected_methods: 0,
            rejected_requests: 0,
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
                                    shared_for_thread.lock().expect("lock script state");
                                let is_post = request.method == "POST";
                                state.requests.push(request);
                                if !is_post {
                                    state.unexpected_methods += 1;
                                    UpstreamStep::json(
                                        405,
                                        "Method Not Allowed",
                                        serde_json::json!({"error":"POST required"}),
                                    )
                                } else {
                                    match state.steps.pop_front() {
                                        Some(step) => step,
                                        None => {
                                            state.unexpected_posts += 1;
                                            UpstreamStep::Http {
                                                status: 500,
                                                reason: "Unexpected Post",
                                                content_type: "application/json",
                                                headers: Vec::new(),
                                                body: br#"{"error":"unexpected post"}"#.to_vec(),
                                            }
                                        }
                                    }
                                }
                            }
                            Err(_) => {
                                shared_for_thread
                                    .lock()
                                    .expect("lock script state")
                                    .rejected_requests += 1;
                                UpstreamStep::json(
                                    400,
                                    "Bad Request",
                                    serde_json::json!({"error":"invalid synthetic request"}),
                                )
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

    fn endpoint(&self, path: &str) -> String {
        format!("http://{}{path}", self.address)
    }

    fn address(&self) -> SocketAddr {
        self.address
    }

    fn finish(mut self) -> ScriptResult {
        self.stop.store(true, Ordering::Release);
        self.thread
            .take()
            .expect("upstream thread")
            .join()
            .expect("join upstream");
        let state = self.shared.lock().expect("lock final script state");
        ScriptResult {
            requests: state.requests.clone(),
            remaining_steps: state.steps.len(),
            unexpected_posts: state.unexpected_posts,
            unexpected_methods: state.unexpected_methods,
            rejected_requests: state.rejected_requests,
        }
    }
}

impl Drop for ScriptedCodexUpstream {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RequestReadError {
    Io,
    Incomplete,
    HeadersTooLarge,
    BodyTooLarge,
    TotalTooLarge,
    InvalidHead,
    InvalidContentLength,
    InvalidJson,
}

fn request_extent(raw: &[u8]) -> Result<Option<usize>, RequestReadError> {
    let Some(head_end) = raw.windows(4).position(|part| part == b"\r\n\r\n") else {
        return if raw.len() > MAX_REQUEST_HEADER_BYTES {
            Err(RequestReadError::HeadersTooLarge)
        } else {
            Ok(None)
        };
    };
    let header_bytes = head_end
        .checked_add(4)
        .ok_or(RequestReadError::TotalTooLarge)?;
    if header_bytes > MAX_REQUEST_HEADER_BYTES {
        return Err(RequestReadError::HeadersTooLarge);
    }
    let head = std::str::from_utf8(&raw[..head_end]).map_err(|_| RequestReadError::InvalidHead)?;
    let content_length = head
        .lines()
        .skip(1)
        .filter_map(|line| line.split_once(':'))
        .find_map(|(name, value)| {
            name.eq_ignore_ascii_case("content-length")
                .then_some(value.trim())
        })
        .map(|value| {
            value
                .parse::<usize>()
                .map_err(|_| RequestReadError::InvalidContentLength)
        })
        .transpose()?
        .unwrap_or(0);
    if content_length > MAX_REQUEST_BODY_BYTES {
        return Err(RequestReadError::BodyTooLarge);
    }
    let total = header_bytes
        .checked_add(content_length)
        .ok_or(RequestReadError::TotalTooLarge)?;
    if total > MAX_REQUEST_TOTAL_BYTES {
        return Err(RequestReadError::TotalTooLarge);
    }
    Ok(Some(total))
}

fn parse_complete_request(raw: &[u8]) -> Result<CapturedRequest, RequestReadError> {
    let expected = request_extent(raw)?.ok_or(RequestReadError::Incomplete)?;
    if raw.len() < expected {
        return Err(RequestReadError::Incomplete);
    }
    let head_end = raw
        .windows(4)
        .position(|part| part == b"\r\n\r\n")
        .ok_or(RequestReadError::Incomplete)?;
    let head = std::str::from_utf8(&raw[..head_end]).map_err(|_| RequestReadError::InvalidHead)?;
    let mut lines = head.lines();
    let mut request_line = lines
        .next()
        .ok_or(RequestReadError::InvalidHead)?
        .split_whitespace();
    let method = request_line
        .next()
        .ok_or(RequestReadError::InvalidHead)?
        .to_string();
    let path = request_line
        .next()
        .ok_or(RequestReadError::InvalidHead)?
        .to_string();
    let headers = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.to_ascii_lowercase(), value.trim().to_string()))
        .filter(|(name, _)| CAPTURED_REQUEST_HEADERS.contains(&name.as_str()))
        .collect();
    let body = serde_json::from_slice(&raw[head_end + 4..expected])
        .map_err(|_| RequestReadError::InvalidJson)?;
    Ok(CapturedRequest {
        method,
        path,
        headers,
        body,
    })
}

fn read_request(stream: &mut TcpStream) -> Result<CapturedRequest, RequestReadError> {
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .map_err(|_| RequestReadError::Io)?;
    let mut raw = Vec::new();
    let mut expected = None;
    let mut buffer = [0_u8; 1024];
    loop {
        let read = stream.read(&mut buffer).map_err(|_| RequestReadError::Io)?;
        if read == 0 {
            return Err(RequestReadError::Incomplete);
        }
        if raw
            .len()
            .checked_add(read)
            .is_none_or(|length| length > MAX_REQUEST_TOTAL_BYTES)
        {
            return Err(RequestReadError::TotalTooLarge);
        }
        raw.extend_from_slice(&buffer[..read]);
        if expected.is_none() {
            expected = request_extent(&raw)?;
        }
        if expected.is_some_and(|length| raw.len() >= length) {
            return parse_complete_request(&raw);
        }
    }
}

fn write_complete_response(
    stream: &mut TcpStream,
    status: u16,
    reason: &str,
    content_type: &str,
    headers: &[(String, String)],
    body: &[u8],
) {
    write!(
        stream,
        "HTTP/1.1 {status} {reason}\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\n",
        body.len()
    )
    .expect("write response head");
    for (name, value) in headers {
        write!(stream, "{name}: {value}\r\n").expect("write synthetic response header");
    }
    stream
        .write_all(b"connection: close\r\n\r\n")
        .expect("finish response head");
    stream.write_all(body).expect("write response body");
    stream.flush().expect("flush complete response");
}

fn write_step(stream: &mut TcpStream, step: UpstreamStep) {
    match step {
        UpstreamStep::Http {
            status,
            reason,
            content_type,
            headers,
            body,
        } => write_complete_response(stream, status, reason, content_type, &headers, &body),
        UpstreamStep::Sse(body) => {
            write_complete_response(stream, 200, "OK", "text/event-stream", &[], &body)
        }
        UpstreamStep::Disconnect => {}
        UpstreamStep::PartialSseThenDrop { headers, body } => {
            write!(
                stream,
                "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\n",
                body.len() + 128
            )
            .expect("write partial SSE head");
            for (name, value) in headers {
                write!(stream, "{name}: {value}\r\n").expect("write partial SSE header");
            }
            stream
                .write_all(b"connection: close\r\n\r\n")
                .expect("finish partial SSE head");
            stream.write_all(&body).expect("write partial SSE body");
            stream.flush().expect("flush partial SSE body");
        }
    }
}

fn manual_post(address: SocketAddr, body: Value) -> Vec<u8> {
    let response = manual_post_raw(address, body);
    assert!(response.starts_with(b"HTTP/1.1"));
    response
}

fn manual_post_raw(address: SocketAddr, body: Value) -> Vec<u8> {
    let body = serde_json::to_vec(&body).expect("serialize manual request");
    manual_request(address, "POST", "/responses", &body)
}

fn manual_request(address: SocketAddr, method: &str, path: &str, body: &[u8]) -> Vec<u8> {
    let mut stream = TcpStream::connect(address).expect("connect manual client");
    write!(
        stream,
        "{method} {path} HTTP/1.1\r\nhost: {address}\r\ncontent-type: application/json\r\naccept: text/event-stream\r\nauthorization: Bearer SYNTHETIC\r\nChatGPT-Account-ID: SYNTHETIC\r\nx-private-sentinel: NEVER_CAPTURE_THIS\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        body.len()
    )
    .expect("write manual request head");
    stream.write_all(body).expect("write manual request body");
    stream.flush().expect("flush manual request");
    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .expect("read manual response");
    response
}

fn run_case(case: AcceptanceCase) -> AcceptanceResult {
    let expected_path = case.endpoint_path;
    let upstream = ScriptedCodexUpstream::start(case.steps);
    let transport = CodexTransport::for_test(upstream.endpoint(case.endpoint_path))
        .expect("construct real test Codex transport");
    let auth_rejections = Arc::new(Mutex::new(Vec::new()));
    let auth_rejections_for_handler = Arc::clone(&auth_rejections);
    let downstream = capture_downstream(|stream| {
        handle_codex_messages_with_policy(
            stream,
            &case.request,
            case.is_stream,
            InferenceSecrets::for_test(ACCESS_SENTINEL, ACCOUNT_SENTINEL),
            &transport,
            CodexRequestPolicy {
                use_responses_lite: case.use_responses_lite,
                ..CodexRequestPolicy::default()
            },
            |status, _generation| {
                auth_rejections_for_handler
                    .lock()
                    .expect("lock auth rejections")
                    .push(status);
            },
        );
    });
    let script = upstream.finish();
    assert_script_transport_integrity(&script, expected_path);
    let auth_rejections = auth_rejections
        .lock()
        .expect("lock final auth rejections")
        .clone();
    AcceptanceResult {
        downstream,
        script,
        auth_rejections,
    }
}

fn assert_script_transport_integrity(script: &ScriptResult, expected_path: &str) {
    assert_eq!(script.unexpected_posts, 0, "script received an extra POST");
    assert_eq!(
        script.unexpected_methods, 0,
        "script received a non-POST request"
    );
    assert_eq!(
        script.rejected_requests, 0,
        "script rejected a malformed request"
    );
    for request in &script.requests {
        assert_eq!(request.method, "POST", "captured attempt must be POST");
        assert_eq!(
            request.path, expected_path,
            "captured attempt must use the configured endpoint path"
        );
    }
}

fn capture_downstream(handler: impl FnOnce(&mut TcpStream)) -> Vec<u8> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind downstream capture");
    let address = listener.local_addr().expect("read downstream address");
    let reader = thread::spawn(move || {
        let mut stream = TcpStream::connect(address).expect("connect downstream reader");
        let mut response = Vec::new();
        stream
            .read_to_end(&mut response)
            .expect("read downstream response");
        response
    });
    let (mut stream, _) = listener.accept().expect("accept downstream reader");
    handler(&mut stream);
    drop(stream);
    reader.join().expect("join downstream reader")
}

fn anthropic_request(stream: bool) -> Value {
    serde_json::json!({
        "model": "gpt-test",
        "max_tokens": 128,
        "stream": stream,
        "messages": [{"role": "user", "content": "hello"}]
    })
}

fn tool_request(choice: Option<Value>) -> Value {
    let mut request = anthropic_request(false);
    request["tools"] = serde_json::json!([{
        "name": "read",
        "description": "read a synthetic file",
        "input_schema": {"type": "object", "properties": {}}
    }]);
    if let Some(choice) = choice {
        request["tool_choice"] = choice;
    }
    request
}

fn typed_choice_rejection(code: &str, param: &str) -> UpstreamStep {
    UpstreamStep::json(
        400,
        "Bad Request",
        serde_json::json!({
            "error": {
                "type": "invalid_request_error",
                "code": code,
                "param": param,
                "message": "synthetic typed rejection"
            }
        }),
    )
}

fn typed_automatic_choice_rejection() -> UpstreamStep {
    typed_choice_rejection("unsupported_value", "tool_choice")
}

fn complete_sse() -> Vec<u8> {
    [
        serde_json::json!({"type":"response.created","response":{"id":"resp"}}),
        serde_json::json!({"type":"response.output_text.delta","item_id":"msg","delta":"hello"}),
        serde_json::json!({"type":"response.output_item.done","item":{"type":"message","id":"msg","content":[{"type":"output_text","text":"hello"}]}}),
        serde_json::json!({"type":"response.completed","response":{"usage":{"input_tokens":2,"output_tokens":1}}}),
    ]
    .into_iter()
    .map(|event| format!("data: {event}\n\n"))
    .collect::<String>()
    .into_bytes()
}

fn partial_sse() -> Vec<u8> {
    [
        serde_json::json!({"type":"response.created","response":{"id":"resp-partial"}}),
        serde_json::json!({"type":"response.output_text.delta","item_id":"msg","delta":"partial-visible"}),
    ]
    .into_iter()
    .map(|event| format!("data: {event}\n\n"))
    .collect::<String>()
    .into_bytes()
}

fn partial_sse_with_sentinels() -> UpstreamStep {
    let mut body = partial_sse();
    body.extend_from_slice(format!("data: {{\"secret\":\"{BODY_SENTINEL}\"").as_bytes());
    UpstreamStep::partial_sse(body, vec![("set-cookie", COOKIE_SENTINEL)])
}

fn redaction_step() -> UpstreamStep {
    UpstreamStep::json_with_headers(
        500,
        "Internal Server Error",
        serde_json::json!({"error":{"message":BODY_SENTINEL}}),
        vec![
            ("set-cookie", COOKIE_SENTINEL),
            ("x-request-id", "req-safe-500"),
        ],
    )
}

#[test]
fn contract_partial_stream_failure_never_replays() {
    let result = run_case(AcceptanceCase {
        request: anthropic_request(true),
        is_stream: true,
        use_responses_lite: false,
        endpoint_path: "/PRIVATE_UPSTREAM_PATH_SENTINEL/responses",
        steps: vec![partial_sse_with_sentinels()],
    });
    assert_eq!(result.status(), 200);
    assert_eq!(result.script.requests.len(), 1);
    assert_eq!(result.script.remaining_steps, 0);
    let events = result.sse_events();
    assert!(
        events.len() >= 2,
        "visible prefix and terminal event required"
    );
    assert_eq!(result.sse_text().matches("partial-visible").count(), 1);
    let terminal_errors: Vec<_> = events
        .iter()
        .enumerate()
        .filter(|(_, event)| event.event.as_deref() == Some("error"))
        .collect();
    assert_eq!(terminal_errors.len(), 1);
    let (terminal_index, terminal) = terminal_errors[0];
    assert_eq!(
        terminal_index,
        events.len() - 1,
        "terminal error must be last"
    );
    assert_eq!(terminal.data["type"], "error");
    assert_eq!(terminal.data["error"]["type"], "api_error");
    assert!(terminal.data["error"]["message"]
        .as_str()
        .is_some_and(|message| !message.is_empty()));
    assert_forbidden_sentinels_absent(&result.text());
}

#[test]
fn contract_failure_schema_and_caller_redaction() {
    let result = run_case(AcceptanceCase {
        request: anthropic_request(false),
        is_stream: false,
        use_responses_lite: false,
        endpoint_path: "/PRIVATE_UPSTREAM_PATH_SENTINEL/responses",
        steps: vec![redaction_step(), redaction_step(), redaction_step()],
    });
    assert_eq!(result.script.requests.len(), 3);
    assert_eq!(result.script.remaining_steps, 0);
    assert_eq!(result.script.unexpected_posts, 0);
    let raw = result.text();
    assert_forbidden_sentinels_absent(&raw);
    assert_failure_envelope(
        &result,
        502,
        "api_error",
        "responses",
        "transient",
        Some(500),
        true,
    );
    assert_eq!(result.json()["error"]["request_id"], "req-safe-500");
}

#[test]
fn contract_transport_diagnostic_projection_is_redacted() {
    let upstream = ScriptedCodexUpstream::start(vec![redaction_step()]);
    let transport =
        CodexTransport::for_test(upstream.endpoint("/PRIVATE_UPSTREAM_PATH_SENTINEL/responses"))
            .expect("construct diagnostic transport");
    let error = match transport.open_responses(
        &InferenceSecrets::for_test(ACCESS_SENTINEL, ACCOUNT_SENTINEL),
        serde_json::to_vec(&serde_json::json!({"prompt": BODY_SENTINEL})).unwrap(),
        false,
        crate::codex_transport::CodexCancellation::default(),
    ) {
        Ok(_) => panic!("synthetic 500 must return a transport error"),
        Err(error) => error,
    };
    let diagnostic = format!("{error:?} {error}");
    assert_forbidden_sentinels_absent(&diagnostic);
    let result = upstream.finish();
    assert_eq!(result.requests.len(), 1);
    assert_eq!(result.unexpected_posts, 0);
    assert_request_projection_safe(&result.requests[0]);
}

fn rate_limit_step(retry_after: &str, request_id: Option<&str>) -> UpstreamStep {
    let mut headers = vec![("Retry-After", retry_after)];
    if let Some(request_id) = request_id {
        headers.push(("x-request-id", request_id));
    }
    UpstreamStep::json_with_headers(
        429,
        "Too Many Requests",
        serde_json::json!({"error":{"type":"rate_limit_error","code":"rate_limit_exceeded"}}),
        headers,
    )
}

#[test]
fn contract_rate_limit_retries_twice_then_succeeds() {
    let result = run_case(AcceptanceCase {
        request: anthropic_request(false),
        is_stream: false,
        use_responses_lite: false,
        endpoint_path: "/responses",
        steps: vec![
            rate_limit_step("0", None),
            rate_limit_step("0", None),
            UpstreamStep::Sse(complete_sse()),
        ],
    });
    assert_eq!(result.status(), 200);
    assert_eq!(result.script.requests.len(), 3);
    assert_eq!(result.script.remaining_steps, 0);
    assert_eq!(result.script.unexpected_posts, 0);
}

#[test]
fn contract_rate_limit_exhaustion_caps_retry_after() {
    let result = run_case(AcceptanceCase {
        request: anthropic_request(false),
        is_stream: false,
        use_responses_lite: false,
        endpoint_path: "/responses",
        steps: vec![
            rate_limit_step("0", None),
            rate_limit_step("0", None),
            rate_limit_step("120", Some("req-acceptance-429")),
        ],
    });
    assert_eq!(result.script.requests.len(), 3);
    assert_failure_envelope(
        &result,
        429,
        "rate_limit_error",
        "responses",
        "rate_limit",
        Some(429),
        true,
    );
    let body = result.json();
    assert_eq!(body["error"]["retry_after_seconds"], 60);
    assert_eq!(body["error"]["request_id"], "req-acceptance-429");
}

#[test]
fn contract_quota_429_is_not_retried() {
    let result = run_case(AcceptanceCase {
        request: anthropic_request(false),
        is_stream: false,
        use_responses_lite: false,
        endpoint_path: "/responses",
        steps: vec![
            UpstreamStep::json(
                429,
                "Too Many Requests",
                serde_json::json!({"error":{"type":"insufficient_quota","code":"insufficient_quota"}}),
            ),
            UpstreamStep::Sse(complete_sse()),
        ],
    });
    assert_eq!(result.script.requests.len(), 1);
    assert_eq!(result.script.remaining_steps, 1);
    assert_failure_envelope(
        &result,
        429,
        "rate_limit_error",
        "responses",
        "quota",
        Some(429),
        false,
    );
}

fn transient_step(status: u16, reason: &'static str) -> UpstreamStep {
    UpstreamStep::json_with_headers(
        status,
        reason,
        serde_json::json!({"error":{"code":"synthetic_transient"}}),
        vec![("Retry-After", "0")],
    )
}

#[test]
fn contract_network_and_5xx_retry_within_three_posts() {
    let result = run_case(AcceptanceCase {
        request: anthropic_request(false),
        is_stream: false,
        use_responses_lite: false,
        endpoint_path: "/responses",
        steps: vec![
            UpstreamStep::Disconnect,
            transient_step(503, "Service Unavailable"),
            UpstreamStep::Sse(complete_sse()),
        ],
    });
    assert_eq!(result.status(), 200);
    assert_eq!(result.script.requests.len(), 3);
    assert_eq!(result.script.remaining_steps, 0);
}

#[test]
fn contract_network_only_exhaustion_omits_unknown_upstream_metadata() {
    let result = run_case(AcceptanceCase {
        request: anthropic_request(false),
        is_stream: false,
        use_responses_lite: false,
        endpoint_path: "/responses",
        steps: vec![
            UpstreamStep::Disconnect,
            UpstreamStep::Disconnect,
            UpstreamStep::Disconnect,
        ],
    });
    assert_eq!(result.script.requests.len(), 3);
    assert_optional_metadata_absent(
        &result,
        &["upstream_status", "request_id", "retry_after_seconds"],
    );
    assert_failure_envelope(
        &result,
        502,
        "api_error",
        "responses",
        "network",
        None,
        true,
    );
}

fn assert_transient_exhaustion(status: u16, reason: &'static str, downstream_status: u16) {
    let result = run_case(AcceptanceCase {
        request: anthropic_request(false),
        is_stream: false,
        use_responses_lite: false,
        endpoint_path: "/responses",
        steps: vec![
            transient_step(status, reason),
            transient_step(status, reason),
            transient_step(status, reason),
        ],
    });
    assert_eq!(result.script.requests.len(), 3);
    assert_failure_envelope(
        &result,
        downstream_status,
        "api_error",
        "responses",
        "transient",
        Some(status),
        true,
    );
}

#[test]
fn contract_408_exhaustion_returns_504() {
    assert_transient_exhaustion(408, "Request Timeout", 504);
}

#[test]
fn contract_409_exhaustion_returns_502() {
    assert_transient_exhaustion(409, "Conflict", 502);
}

#[test]
fn contract_500_exhaustion_returns_502() {
    assert_transient_exhaustion(500, "Internal Server Error", 502);
}

#[test]
fn contract_503_exhaustion_returns_502() {
    assert_transient_exhaustion(503, "Service Unavailable", 502);
}

#[test]
fn contract_lite_non_equivalent_tool_choices_never_post() {
    for choice in [
        serde_json::json!({"type": "none"}),
        serde_json::json!({"type": "any"}),
        serde_json::json!({"type": "required"}),
        serde_json::json!({"type": "tool", "name": "read"}),
    ] {
        let result = run_case(AcceptanceCase {
            request: tool_request(Some(choice)),
            is_stream: false,
            use_responses_lite: true,
            endpoint_path: "/responses",
            steps: Vec::new(),
        });
        assert_eq!(result.status(), 400);
        assert_eq!(result.json()["error"]["type"], "invalid_request_error");
        assert_eq!(result.script.requests.len(), 0);
        assert_eq!(result.script.remaining_steps, 0);
        assert_eq!(result.script.unexpected_posts, 0);
    }
}

fn assert_safe_repair(choice: Option<Value>) {
    let result = run_case(AcceptanceCase {
        request: tool_request(choice),
        is_stream: false,
        use_responses_lite: true,
        endpoint_path: "/responses",
        steps: vec![
            typed_automatic_choice_rejection(),
            UpstreamStep::Sse(complete_sse()),
        ],
    });
    assert_eq!(result.status(), 200);
    assert_eq!(result.script.requests.len(), 2);
    assert_eq!(result.script.remaining_steps, 0);
    let mut first = result.script.requests[0]
        .body
        .as_object()
        .expect("first request object")
        .clone();
    let second = result.script.requests[1]
        .body
        .as_object()
        .expect("second request object")
        .clone();
    assert_eq!(
        first.remove("tool_choice"),
        Some(Value::String("auto".into()))
    );
    assert!(!second.contains_key("tool_choice"));
    assert_eq!(first, second);
}

#[test]
fn contract_safe_repair_for_absent_choice() {
    assert_safe_repair(None);
}

#[test]
fn contract_safe_repair_for_explicit_auto() {
    assert_safe_repair(Some(serde_json::json!({"type": "auto"})));
}

fn assert_typed_rejection_does_not_authorize_repair(
    code: &str,
    param: &str,
    use_responses_lite: bool,
    route: &str,
) {
    let result = run_case(AcceptanceCase {
        request: tool_request(Some(serde_json::json!({"type": "auto"}))),
        is_stream: false,
        use_responses_lite,
        endpoint_path: "/responses",
        steps: vec![
            typed_choice_rejection(code, param),
            UpstreamStep::Sse(complete_sse()),
        ],
    });
    assert_eq!(result.script.requests.len(), 1);
    assert_eq!(result.script.remaining_steps, 1);
    assert_failure_envelope(
        &result,
        400,
        "invalid_request_error",
        route,
        "capability",
        Some(400),
        false,
    );
}

#[test]
fn contract_unknown_typed_code_does_not_authorize_repair() {
    assert_typed_rejection_does_not_authorize_repair(
        "unknown_capability_code",
        "tool_choice",
        true,
        "responses_lite",
    );
}

#[test]
fn contract_allowlisted_code_with_wrong_param_does_not_authorize_repair() {
    assert_typed_rejection_does_not_authorize_repair(
        "unsupported_value",
        "parallel_tool_calls",
        true,
        "responses_lite",
    );
}

#[test]
fn contract_route_disabled_does_not_authorize_repair() {
    assert_typed_rejection_does_not_authorize_repair(
        "unsupported_value",
        "tool_choice",
        false,
        "responses",
    );
}

#[test]
fn contract_used_repair_budget_stops_after_second_typed_rejection() {
    let result = run_case(AcceptanceCase {
        request: tool_request(Some(serde_json::json!({"type": "auto"}))),
        is_stream: false,
        use_responses_lite: true,
        endpoint_path: "/responses",
        steps: vec![
            typed_automatic_choice_rejection(),
            typed_automatic_choice_rejection(),
            UpstreamStep::Sse(complete_sse()),
        ],
    });
    assert_eq!(result.script.requests.len(), 2);
    assert_eq!(result.script.remaining_steps, 1);
    let mut first = result.script.requests[0]
        .body
        .as_object()
        .expect("first request object")
        .clone();
    let second = result.script.requests[1]
        .body
        .as_object()
        .expect("second request object")
        .clone();
    assert_eq!(
        first.remove("tool_choice"),
        Some(Value::String("auto".into()))
    );
    assert!(!second.contains_key("tool_choice"));
    assert_eq!(first, second);
    assert_failure_envelope(
        &result,
        400,
        "invalid_request_error",
        "responses_lite",
        "capability",
        Some(400),
        false,
    );
}

#[test]
fn contract_unproven_error_text_does_not_authorize_repair() {
    let result = run_case(AcceptanceCase {
        request: tool_request(Some(serde_json::json!({"type": "auto"}))),
        is_stream: false,
        use_responses_lite: true,
        endpoint_path: "/responses",
        steps: vec![
            UpstreamStep::json(
                400,
                "Bad Request",
                serde_json::json!({
                    "error": {"message": "automatic tool_choice is unsupported"}
                }),
            ),
            UpstreamStep::Sse(complete_sse()),
        ],
    });
    assert_eq!(result.script.requests.len(), 1);
    assert_eq!(result.script.remaining_steps, 1);
    assert_eq!(result.script.unexpected_posts, 0);
}

fn invalid_request_id_step(request_id: &str) -> UpstreamStep {
    UpstreamStep::json_with_headers(
        500,
        "Internal Server Error",
        serde_json::json!({"error":{"code":"synthetic_transient"}}),
        vec![("x-request-id", request_id)],
    )
}

fn assert_invalid_request_id_is_omitted(request_id: &str) {
    let result = run_case(AcceptanceCase {
        request: anthropic_request(false),
        is_stream: false,
        use_responses_lite: false,
        endpoint_path: "/responses",
        steps: vec![
            invalid_request_id_step(request_id),
            invalid_request_id_step(request_id),
            invalid_request_id_step(request_id),
        ],
    });
    assert_eq!(result.script.requests.len(), 3);
    assert_eq!(result.script.remaining_steps, 0);
    assert_eq!(result.script.unexpected_posts, 0);
    assert_optional_metadata_absent(&result, &["request_id", "retry_after_seconds"]);
    assert_failure_envelope(
        &result,
        502,
        "api_error",
        "responses",
        "transient",
        Some(500),
        true,
    );
}

#[test]
fn contract_malformed_request_id_is_omitted() {
    assert_invalid_request_id_is_omitted("request id contains spaces");
}

#[test]
fn contract_oversized_request_id_is_omitted() {
    assert_invalid_request_id_is_omitted(&"r".repeat(257));
}

fn assert_permanent_4xx(status: u16, reason: &'static str) {
    let result = run_case(AcceptanceCase {
        request: anthropic_request(false),
        is_stream: false,
        use_responses_lite: false,
        endpoint_path: "/responses",
        steps: vec![UpstreamStep::json(
            status,
            reason,
            serde_json::json!({"error":{"type":"invalid_request_error","code":"invalid_request"}}),
        )],
    });
    assert_eq!(result.script.requests.len(), 1);
    assert_eq!(result.script.remaining_steps, 0);
    assert_eq!(result.script.unexpected_posts, 0);
    assert_failure_envelope(
        &result,
        status,
        "invalid_request_error",
        "responses",
        "invalid_request",
        Some(status),
        false,
    );
}

#[test]
fn contract_permanent_400_is_not_retried() {
    assert_permanent_4xx(400, "Bad Request");
}

#[test]
fn contract_permanent_404_is_not_retried() {
    assert_permanent_4xx(404, "Not Found");
}

#[test]
fn contract_permanent_422_is_not_retried() {
    assert_permanent_4xx(422, "Unprocessable Entity");
}

fn assert_auth_failure(status: u16, reason: &'static str, error_type: &str, failure_class: &str) {
    let result = run_case(AcceptanceCase {
        request: anthropic_request(false),
        is_stream: false,
        use_responses_lite: false,
        endpoint_path: "/responses",
        steps: vec![UpstreamStep::json(
            status,
            reason,
            serde_json::json!({"error":{"code":"synthetic_auth_rejection"}}),
        )],
    });
    assert_eq!(result.script.requests.len(), 1);
    assert_eq!(result.auth_rejections, vec![status]);
    assert_failure_envelope(
        &result,
        status,
        error_type,
        "responses",
        failure_class,
        Some(status),
        false,
    );
}

#[test]
fn contract_401_is_authentication_and_not_retried() {
    assert_auth_failure(
        401,
        "Unauthorized",
        "authentication_error",
        "authentication",
    );
}

#[test]
fn contract_403_is_authorization_and_not_retried() {
    assert_auth_failure(403, "Forbidden", "permission_error", "authorization");
}

#[test]
fn harness_feature_gate_compiles() {
    const { assert!(cfg!(all(test, feature = "acceptance-build"))) };
}

#[test]
fn harness_runner_uses_real_handler_and_transport() {
    let result = run_case(AcceptanceCase {
        request: anthropic_request(false),
        is_stream: false,
        use_responses_lite: false,
        endpoint_path: "/responses",
        steps: vec![UpstreamStep::Sse(complete_sse())],
    });

    assert_eq!(result.status(), 200);
    assert_eq!(result.script.requests.len(), 1);
    assert_eq!(result.script.remaining_steps, 0);
    assert_eq!(result.script.requests[0].body["model"], "gpt-test");
    assert_eq!(result.json()["content"][0]["text"], "hello");
    assert!(result.auth_rejections.is_empty());
    assert!(result.text().starts_with("HTTP/1.1 200"));
}

#[test]
fn harness_records_ordered_requests_without_secret_headers() {
    let upstream = ScriptedCodexUpstream::start(vec![
        UpstreamStep::json(400, "Bad Request", serde_json::json!({"n": 1})),
        UpstreamStep::json(422, "Unprocessable Entity", serde_json::json!({"n": 2})),
    ]);
    manual_post(upstream.address(), serde_json::json!({"attempt": 1}));
    manual_post(upstream.address(), serde_json::json!({"attempt": 2}));
    let result = upstream.finish();

    assert_eq!(result.requests.len(), 2);
    assert_eq!(result.remaining_steps, 0);
    assert_eq!(result.unexpected_posts, 0);
    assert_eq!(result.requests[0].path, "/responses");
    assert_eq!(result.requests[0].body["attempt"], 1);
    assert_eq!(result.requests[1].body["attempt"], 2);
    assert_request_projection_safe(&result.requests[0]);
    assert_request_projection_safe(&result.requests[1]);
}

#[test]
fn harness_reports_unused_and_extra_script_steps() {
    let unused = ScriptedCodexUpstream::start(vec![
        UpstreamStep::json(400, "Bad Request", serde_json::json!({"n": 1})),
        UpstreamStep::json(400, "Bad Request", serde_json::json!({"n": 2})),
    ]);
    manual_post(unused.address(), serde_json::json!({"attempt": 1}));
    let unused_result = unused.finish();
    assert_eq!(unused_result.remaining_steps, 1);
    assert_eq!(unused_result.unexpected_posts, 0);

    let extra = ScriptedCodexUpstream::start(vec![UpstreamStep::json(
        400,
        "Bad Request",
        serde_json::json!({"n": 1}),
    )]);
    manual_post(extra.address(), serde_json::json!({"attempt": 1}));
    manual_post(extra.address(), serde_json::json!({"attempt": 2}));
    let extra_result = extra.finish();
    assert_eq!(extra_result.remaining_steps, 0);
    assert_eq!(extra_result.unexpected_posts, 1);
}

#[test]
fn harness_serves_sse_disconnect_and_partial_responses() {
    let complete_body = b"data: complete\n\n".to_vec();
    let partial_body = b"data: partial\n\n".to_vec();
    let upstream = ScriptedCodexUpstream::start(vec![
        UpstreamStep::Sse(complete_body.clone()),
        UpstreamStep::Disconnect,
        UpstreamStep::partial_sse(partial_body.clone(), Vec::new()),
    ]);
    assert_eq!(
        upstream.endpoint("/responses"),
        format!("http://{}/responses", upstream.address())
    );

    let complete_response = manual_post(upstream.address(), serde_json::json!({"attempt": 1}));
    assert_sse_wire_response(&complete_response, complete_body.len(), &complete_body);

    let disconnect_response =
        manual_post_raw(upstream.address(), serde_json::json!({"attempt": 2}));
    assert!(disconnect_response.is_empty());

    let partial_response = manual_post(upstream.address(), serde_json::json!({"attempt": 3}));
    assert_sse_wire_response(&partial_response, partial_body.len() + 128, &partial_body);

    let result = upstream.finish();
    assert_eq!(result.requests.len(), 3);
    assert_eq!(result.remaining_steps, 0);
    assert_eq!(result.unexpected_posts, 0);
    for request in result.requests {
        assert_request_projection_safe(&request);
    }
}

#[test]
fn harness_rejects_oversized_request_headers_and_bodies() {
    let oversized_header = format!(
        "POST /responses HTTP/1.1\r\nx-padding: {}\r\n\r\n",
        "x".repeat(MAX_REQUEST_HEADER_BYTES)
    );
    assert_eq!(
        parse_complete_request(oversized_header.as_bytes()),
        Err(RequestReadError::HeadersTooLarge)
    );

    let oversized_body = format!(
        "POST /responses HTTP/1.1\r\ncontent-length: {}\r\n\r\n",
        MAX_REQUEST_BODY_BYTES + 1
    );
    assert_eq!(
        parse_complete_request(oversized_body.as_bytes()),
        Err(RequestReadError::BodyTooLarge)
    );
}

#[test]
fn harness_captures_only_allowlisted_request_headers() {
    let raw = b"POST /responses HTTP/1.1\r\ncontent-type: application/json\r\naccept: text/event-stream\r\nauthorization: Bearer SYNTHETIC\r\nChatGPT-Account-ID: SYNTHETIC\r\nx-private-sentinel: NEVER_CAPTURE_THIS\r\ncontent-length: 2\r\n\r\n{}";
    let request = parse_complete_request(raw).expect("parse bounded synthetic request");

    assert_eq!(request.method, "POST");
    assert_eq!(request.path, "/responses");
    assert_eq!(
        request.headers,
        BTreeMap::from([
            ("accept".into(), "text/event-stream".into()),
            ("content-type".into(), "application/json".into()),
        ])
    );
}

#[test]
fn harness_rejects_and_separately_accounts_non_post_requests() {
    let upstream = ScriptedCodexUpstream::start(vec![UpstreamStep::json(
        400,
        "Bad Request",
        serde_json::json!({"n": 1}),
    )]);
    let response = manual_request(upstream.address(), "GET", "/responses", b"{}");
    assert!(response.starts_with(b"HTTP/1.1 405 Method Not Allowed"));

    let result = upstream.finish();
    assert_eq!(result.requests.len(), 1);
    assert_eq!(result.unexpected_methods, 1);
    assert_eq!(result.rejected_requests, 0);
    assert_eq!(result.remaining_steps, 1);
    assert_eq!(result.unexpected_posts, 0);
}
