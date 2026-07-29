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
                .map(|(name, value)| (name.to_string(), value.to_string()))
                .collect(),
            body: serde_json::to_vec(&body).expect("serialize synthetic upstream JSON"),
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
}

#[derive(Default)]
struct SharedScript {
    steps: VecDeque<UpstreamStep>,
    requests: Vec<CapturedRequest>,
    unexpected_posts: usize,
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
        }));
        let stop = Arc::new(AtomicBool::new(false));
        let shared_for_thread = Arc::clone(&shared);
        let stop_for_thread = Arc::clone(&stop);
        let thread = thread::spawn(move || {
            while !stop_for_thread.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let request = read_request(&mut stream);
                        let step = {
                            let mut state = shared_for_thread.lock().expect("lock script state");
                            state.requests.push(request);
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

fn read_request(stream: &mut TcpStream) -> CapturedRequest {
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("set request read timeout");
    let mut raw = Vec::new();
    let mut expected = None;
    let mut buffer = [0_u8; 1024];
    loop {
        let read = stream.read(&mut buffer).expect("read scripted request");
        assert!(read > 0, "scripted request ended before its declared body");
        raw.extend_from_slice(&buffer[..read]);
        if expected.is_none() {
            if let Some(head_end) = raw.windows(4).position(|part| part == b"\r\n\r\n") {
                let head = String::from_utf8_lossy(&raw[..head_end]);
                let content_length = head
                    .lines()
                    .skip(1)
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length").then(|| {
                            value
                                .trim()
                                .parse::<usize>()
                                .expect("numeric content length")
                        })
                    })
                    .unwrap_or(0);
                expected = Some(head_end + 4 + content_length);
            }
        }
        if expected.is_some_and(|length| raw.len() >= length) {
            break;
        }
    }

    let head_end = raw
        .windows(4)
        .position(|part| part == b"\r\n\r\n")
        .expect("request header terminator");
    let head = String::from_utf8_lossy(&raw[..head_end]);
    let mut lines = head.lines();
    let mut request_line = lines.next().expect("request line").split_whitespace();
    let method = request_line.next().expect("request method").to_string();
    let path = request_line.next().expect("request path").to_string();
    let headers = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.to_ascii_lowercase(), value.trim().to_string()))
        .filter(|(name, _)| name != "authorization" && name != "chatgpt-account-id")
        .collect();
    let body = serde_json::from_slice(&raw[head_end + 4..expected.expect("request length")])
        .expect("synthetic request JSON");
    CapturedRequest {
        method,
        path,
        headers,
        body,
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
        UpstreamStep::PartialSseThenDrop(body) => {
            write!(
                stream,
                "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                body.len() + 128
            )
            .expect("write partial SSE head");
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
    let mut stream = TcpStream::connect(address).expect("connect manual client");
    write!(
        stream,
        "POST /responses HTTP/1.1\r\nhost: {address}\r\ncontent-type: application/json\r\nauthorization: Bearer SYNTHETIC\r\nChatGPT-Account-ID: SYNTHETIC\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        body.len()
    )
    .expect("write manual request head");
    stream.write_all(&body).expect("write manual request body");
    stream.flush().expect("flush manual request");
    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .expect("read manual response");
    response
}

fn run_case(case: AcceptanceCase) -> AcceptanceResult {
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

#[test]
fn harness_feature_gate_compiles() {
    assert!(cfg!(all(test, feature = "acceptance-build")));
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
    assert!(!result.requests[0].headers.contains_key("authorization"));
    assert!(!result.requests[0]
        .headers
        .contains_key("chatgpt-account-id"));
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
        UpstreamStep::PartialSseThenDrop(partial_body.clone()),
    ]);
    assert_eq!(
        upstream.endpoint("/responses"),
        format!("http://{}/responses", upstream.address())
    );

    let complete_response = manual_post(upstream.address(), serde_json::json!({"attempt": 1}));
    let complete_head_end = complete_response
        .windows(4)
        .position(|part| part == b"\r\n\r\n")
        .expect("complete SSE response head");
    let complete_head = String::from_utf8_lossy(&complete_response[..complete_head_end]);
    let complete_length = format!("content-length: {}", complete_body.len());
    assert!(complete_head.starts_with("HTTP/1.1 200 OK\r\n"));
    assert!(complete_head.contains("content-type: text/event-stream\r\n"));
    assert!(complete_head.contains(&complete_length));
    assert_eq!(&complete_response[complete_head_end + 4..], complete_body);

    let disconnect_response =
        manual_post_raw(upstream.address(), serde_json::json!({"attempt": 2}));
    assert!(disconnect_response.is_empty());

    let partial_response = manual_post(upstream.address(), serde_json::json!({"attempt": 3}));
    let partial_head_end = partial_response
        .windows(4)
        .position(|part| part == b"\r\n\r\n")
        .expect("partial SSE response head");
    let partial_head = String::from_utf8_lossy(&partial_response[..partial_head_end]);
    let declared_partial_length = format!("content-length: {}", partial_body.len() + 128);
    assert!(partial_head.starts_with("HTTP/1.1 200 OK\r\n"));
    assert!(partial_head.contains("content-type: text/event-stream\r\n"));
    assert!(partial_head.contains(&declared_partial_length));
    assert_eq!(&partial_response[partial_head_end + 4..], partial_body);

    let result = upstream.finish();
    assert_eq!(result.requests.len(), 3);
    assert_eq!(result.remaining_steps, 0);
    assert_eq!(result.unexpected_posts, 0);
    for request in result.requests {
        assert!(!request.headers.contains_key("authorization"));
        assert!(!request.headers.contains_key("chatgpt-account-id"));
    }
}
