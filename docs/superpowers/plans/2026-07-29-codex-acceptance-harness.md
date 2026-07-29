# Codex Acceptance Harness Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a feature-gated deterministic loopback harness that exercises the real Codex handler and transport, leaves the approved Provider Failure Contract expected-red, and produces reviewer-ready evidence without changing production behavior.

**Architecture:** `server/codex_acceptance.rs` owns a bounded scripted HTTP/SSE upstream, request capture, a real-handler runner, harness self-tests, and target contract tests. `server.rs` only declares that child module under `cfg(all(test, feature = "acceptance-build"))`; normal tests and non-test builds never compile it. A Markdown evidence report separates green harness validity and contract anchors from precise current production gaps.

**Tech Stack:** Rust 2021, `std::net` loopback TCP, existing `reqwest`-backed `CodexTransport`, `serde_json`, Cargo feature `acceptance-build`, Markdown Wayfinder evidence.

## Global Constraints

- Execute code work in an isolated worktree on branch `prototype/codex-acceptance-harness`, created with the `using-git-worktrees` skill from the reviewed plan commit or its reviewed descendant.
- Before editing code, invoke the `test-driven-development` skill and preserve its red-green discipline for harness infrastructure.
- Compile the harness only under `cfg(all(test, feature = "acceptance-build"))`.
- Do not change the production Codex handler, transport behavior, retry behavior, OAuth state, proxy configuration, dependencies, or Provider Routes in this ticket.
- Use loopback ephemeral ports only. Do not use external network endpoints, installed CSSwitch profiles, real OAuth caches, or real credentials.
- Never retain the authorization header or `ChatGPT-Account-ID` in captured requests.
- Use only synthetic sentinels in request bodies and fake upstream responses.
- Retryable target cases permit at most three total POSTs; the Safe Repair case permits exactly one replay and at most two total POSTs.
- Normalize a final `Retry-After` delta to at most 60 seconds; do not assert elapsed wall-clock time.
- Once downstream streaming bytes begin, the expected POST count remains one.
- Keep legacy caller fields `type`, `error.type`, and `error.message`; attempt counts belong to diagnostics, not the caller envelope.
- Contract tests encode the approved target and may fail. Do not weaken an assertion to make the current handler green and do not add artificial failure branches.
- Harness self-tests must pass before any contract failure is accepted as evidence.
- Do not resolve the Wayfinder ticket until the user gives a live verdict on the expected-red report.

---

## File structure

- Modify `desktop/gateway/src/server.rs:2199` — declare the test-only acceptance module and make no other change.
- Create `desktop/gateway/src/server/codex_acceptance.rs` — scripted upstream, handler runner, self-tests, and target contract tests.
- Create `docs/superpowers/reports/2026-07-29-codex-acceptance-harness-expected-red.md` — reproducible commands, green harness evidence, current contract gaps, and Ticket 05 mapping.
- Modify `.scratch/provider-failure-contract/issues/03-prototype-codex-acceptance-harness.md` only after the live verdict — record the accepted answer and branch revision.
- Modify `.scratch/provider-failure-contract/map.md` only after the live verdict — publish the resolved-ticket pointer.

### Task 1: Establish the isolated feature gate

**Files:**
- Modify: `desktop/gateway/src/server.rs:2199`
- Create: `desktop/gateway/src/server/codex_acceptance.rs`

**Interfaces:**
- Consumes: existing Cargo feature `acceptance-build` and private server-module visibility.
- Produces: child module `server::codex_acceptance`, compiled only for tests with the feature enabled.

- [ ] **Step 1: Create and verify the isolated worktree**

Invoke `using-git-worktrees`, create `prototype/codex-acceptance-harness`, and verify:

```bash
git status --short --branch
git rev-parse HEAD
```

Expected: a clean isolated branch containing the approved design and this plan.

- [ ] **Step 2: Add the smallest compile gate**

Insert before the existing `#[cfg(test)] mod tests` in `desktop/gateway/src/server.rs`:

```rust
#[cfg(all(test, feature = "acceptance-build"))]
mod codex_acceptance;
```

Create `desktop/gateway/src/server/codex_acceptance.rs`:

```rust
#[test]
fn harness_feature_gate_compiles() {
    assert!(cfg!(all(test, feature = "acceptance-build")));
}
```

- [ ] **Step 3: Prove both sides of the gate**

Run:

```bash
cargo test --offline --manifest-path desktop/gateway/Cargo.toml \
  --features acceptance-build 'server::codex_acceptance::harness_feature_gate_compiles'
cargo test --offline --manifest-path desktop/gateway/Cargo.toml \
  'server::codex_acceptance::' -- --list
```

Expected: the first command reports one passing test; the second reports zero matching tests because the module is not compiled without the feature.

- [ ] **Step 4: Commit the feature gate**

```bash
git add desktop/gateway/src/server.rs desktop/gateway/src/server/codex_acceptance.rs
git commit -m "test: gate Codex acceptance harness"
```

### Task 2: Build the scripted loopback upstream

**Files:**
- Modify: `desktop/gateway/src/server/codex_acceptance.rs`

**Interfaces:**
- Consumes: `std::net::TcpListener`, ordered `VecDeque<UpstreamStep>`, and bounded JSON request bodies.
- Produces: `ScriptedCodexUpstream::start(Vec<UpstreamStep>)`, `endpoint(&str) -> String`, and `finish() -> ScriptResult`.

- [ ] **Step 1: Write the failing request-capture self-test**

Append this test. `manual_post` deliberately sends both secret-bearing headers:

```rust
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
assert!(!result.requests[0].headers.contains_key("chatgpt-account-id"));
}
```

Run:

```bash
cargo test --offline --manifest-path desktop/gateway/Cargo.toml \
  --features acceptance-build \
  'server::codex_acceptance::harness_records_ordered_requests_without_secret_headers'
```

Expected: compilation fails because the scripted-upstream types do not exist yet.

- [ ] **Step 2: Add the bounded script vocabulary and capture result**

Replace the top of `codex_acceptance.rs` with these declarations, keeping the feature-gate test at the bottom:

```rust
use std::collections::{BTreeMap, VecDeque};
use std::io::{ErrorKind, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use serde_json::Value;

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
```

- [ ] **Step 3: Implement server start and deterministic shutdown**

Add these exact method signatures and behaviors:

```rust
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
        self.thread.take().expect("upstream thread").join().expect("join upstream");
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
```

- [ ] **Step 4: Implement bounded request capture**

Add the bounded request parser. It reads through the declared `Content-Length`, lowercases header names, and discards exactly the two secret-bearing headers before capture:

```rust
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
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().expect("numeric content length"))
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
```

- [ ] **Step 5: Implement scripted responses and the manual client**

Add complete and partial response writers:

```rust
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

fn manual_post(address: SocketAddr, body: Value) {
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
    stream.read_to_end(&mut response).expect("read manual response");
    assert!(response.starts_with(b"HTTP/1.1"));
}
```

- [ ] **Step 6: Add unused-step and extra-POST self-tests**

Add:

```rust
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
```

- [ ] **Step 7: Run self-tests and commit**

```bash
cargo test --offline --manifest-path desktop/gateway/Cargo.toml \
  --features acceptance-build 'server::codex_acceptance::harness_'
git add desktop/gateway/src/server/codex_acceptance.rs
git commit -m "test: add scripted Codex upstream"
```

Expected: all `harness_` tests pass; the commit changes only the test module.

### Task 3: Add the real-handler acceptance runner

**Files:**
- Modify: `desktop/gateway/src/server/codex_acceptance.rs`

**Interfaces:**
- Consumes: `handle_codex_messages_with_policy`, `CodexRequestPolicy`, `InferenceSecrets::for_test`, and `CodexTransport::for_test`.
- Produces: `run_case(AcceptanceCase) -> AcceptanceResult`, parsed downstream status/body helpers, and synthetic request/SSE fixtures.

- [ ] **Step 1: Write a failing real-handler success test**

Add this test before the runner implementation:

```rust
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
}
```

Run it and expect compilation to fail because the case runner is not defined.

- [ ] **Step 2: Add the case and result types**

Add:

```rust
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
```

- [ ] **Step 3: Implement the real handler invocation**

Implement:

```rust
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
```

- [ ] **Step 4: Implement downstream response capture**

Implement the downstream capture with a second loopback listener:

```rust
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
```

- [ ] **Step 5: Add exact synthetic fixtures**

Add exact synthetic fixtures:

```rust
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
```

- [ ] **Step 6: Run the runner self-test and existing focused Codex tests**

```bash
cargo test --offline --manifest-path desktop/gateway/Cargo.toml \
  --features acceptance-build 'server::codex_acceptance::harness_'
cargo test --offline --manifest-path desktop/gateway/Cargo.toml \
  'server::tests::codex_' -- --nocapture
```

Expected: harness self-tests pass; the existing focused command retains its 16 passing tests and no failures.

- [ ] **Step 7: Commit the runner**

```bash
git add desktop/gateway/src/server/codex_acceptance.rs
git commit -m "test: drive real Codex handler from harness"
```

### Task 4: Encode permanent and capability contract cases

**Files:**
- Modify: `desktop/gateway/src/server/codex_acceptance.rs`

**Interfaces:**
- Consumes: `run_case`, synthetic requests, and scripted JSON responses.
- Produces: green pre-POST capability anchors plus precise expected-red permanent 4xx and auth assertions.

- [ ] **Step 1: Add a reusable target-envelope assertion**

Add:

```rust
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
    assert!(body["error"]["message"].as_str().is_some_and(|value| !value.is_empty()));
    assert_eq!(body["error"]["provider"], "codex");
    assert_eq!(body["error"]["route"], route);
    assert_eq!(body["error"]["failure_class"], failure_class);
    assert_eq!(body["error"]["upstream_status"].as_u64(), upstream_status.map(u64::from));
    assert_eq!(body["error"]["retryable"], retryable);
    assert!(body["error"]["correlation_id"]
        .as_str()
        .is_some_and(|value| !value.is_empty()));
    assert!(body["error"]["recovery"]
        .as_str()
        .is_some_and(|value| !value.is_empty()));
    assert!(body["error"].get("attempt_count").is_none());
}
```

- [ ] **Step 2: Add the zero-POST Responses Lite anchors**

Add the shared tool fixture and the contract anchor:

```rust
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
```

Run only this test. Expected: PASS against the current translator.

- [ ] **Step 3: Add separate permanent 400, 404, and 422 tests**

Implement the helper and separate wrappers:

```rust
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
```

Run them. Expected now: FAIL because the real transport maps these rejections to 502 `api_error`, while captured POST count remains one.

- [ ] **Step 4: Add separate 401 and 403 tests**

Add:

```rust
fn assert_auth_failure(
    status: u16,
    reason: &'static str,
    error_type: &str,
    failure_class: &str,
) {
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
    assert_auth_failure(401, "Unauthorized", "authentication_error", "authentication");
}

#[test]
fn contract_403_is_authorization_and_not_retried() {
    assert_auth_failure(403, "Forbidden", "permission_error", "authorization");
}
```

Run them. Expected now: FAIL at `error.type` because the current handler emits `api_error`; status and one-POST assertions remain green.

- [ ] **Step 5: Commit the target cases without weakening them**

```bash
git add desktop/gateway/src/server/codex_acceptance.rs
git commit -m "test: specify permanent Codex failure contract"
```

### Task 5: Encode rate-limit, quota, and transient budgets

**Files:**
- Modify: `desktop/gateway/src/server/codex_acceptance.rs`

**Interfaces:**
- Consumes: ordered scripts and target-envelope assertion.
- Produces: exact attempt-budget tests for 429, disconnect, 408, 409, and 5xx behavior.

- [ ] **Step 1: Add rate-limit success and exhaustion tests**

Add the rate-limit fixture and two tests:

```rust
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
```

Expected now: both tests FAIL because the handler stops after the first POST and lacks normalized metadata.

- [ ] **Step 2: Add quota classification**

Add:

```rust
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
```

Expected now: FAIL at the typed envelope while the no-retry count remains green.

- [ ] **Step 3: Add transient success and exhaustion helpers**

Add the mixed-success case and the exhaustion helper:

```rust
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

fn assert_transient_exhaustion(
    status: u16,
    reason: &'static str,
    downstream_status: u16,
) {
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
```

Expected now: each test FAILS because the real handler performs one POST.

- [ ] **Step 4: Run this group and commit**

```bash
cargo test --offline --manifest-path desktop/gateway/Cargo.toml \
  --features acceptance-build 'server::codex_acceptance::contract_rate_limit_' -- --nocapture
cargo test --offline --manifest-path desktop/gateway/Cargo.toml \
  --features acceptance-build 'server::codex_acceptance::contract_5' -- --nocapture
git add desktop/gateway/src/server/codex_acceptance.rs
git commit -m "test: specify Codex retry budgets"
```

Expected: focused contract commands are red for the stated production gaps, not for harness panics, timeouts, or malformed HTTP.

### Task 6: Encode the single Safe Repair

**Files:**
- Modify: `desktop/gateway/src/server/codex_acceptance.rs`

**Interfaces:**
- Consumes: Responses Lite handler policy and captured parsed request bodies.
- Produces: exact two-POST semantic-diff assertions and a green negative anchor for arbitrary error prose.

- [ ] **Step 1: Add the typed repair fixture**

Reuse `tool_request` from Task 4 and add only the typed rejection fixture:

```rust
fn typed_automatic_choice_rejection() -> UpstreamStep {
    UpstreamStep::json(
        400,
        "Bad Request",
        serde_json::json!({
            "error": {
                "type": "invalid_request_error",
                "code": "unsupported_value",
                "param": "tool_choice",
                "message": "synthetic typed rejection"
            }
        }),
    )
}
```

- [ ] **Step 2: Add absent and explicit-auto repair tests**

Implement `assert_safe_repair(choice: Option<Value>)` with this setup before comparing request bodies:

```rust
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
assert_eq!(first.remove("tool_choice"), Some(Value::String("auto".into())));
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
```

Expected now: FAIL because only the first POST occurs and returns 502.

- [ ] **Step 3: Add the unproven-signal anchor**

Add this negative test. It does not assert a target envelope because its sole question is whether arbitrary prose can trigger replay:

```rust
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
```

Expected now: PASS, proving the current handler does not accidentally replay on keywords.

- [ ] **Step 4: Run and commit**

```bash
cargo test --offline --manifest-path desktop/gateway/Cargo.toml \
  --features acceptance-build 'server::codex_acceptance::contract_safe_repair_' -- --nocapture
cargo test --offline --manifest-path desktop/gateway/Cargo.toml \
  --features acceptance-build 'server::codex_acceptance::contract_unproven_' -- --nocapture
git add desktop/gateway/src/server/codex_acceptance.rs
git commit -m "test: specify Codex automatic-choice repair"
```

### Task 7: Prove the stream barrier, schema, and redaction boundary

**Files:**
- Modify: `desktop/gateway/src/server/codex_acceptance.rs`

**Interfaces:**
- Consumes: partial-SSE script, raw downstream bytes, direct real-transport errors, and synthetic sentinels.
- Produces: no-replay streaming anchor and caller/transport diagnostic redaction evidence.

- [ ] **Step 1: Add the partial-stream no-replay test**

Add the fixture and test:

```rust
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

#[test]
fn contract_partial_stream_failure_never_replays() {
    let result = run_case(AcceptanceCase {
        request: anthropic_request(true),
        is_stream: true,
        use_responses_lite: false,
        endpoint_path: "/responses",
        steps: vec![UpstreamStep::PartialSseThenDrop(partial_sse())],
    });
assert_eq!(result.status(), 200);
assert_eq!(result.script.requests.len(), 1);
assert_eq!(result.script.remaining_steps, 0);
assert_eq!(result.text().matches("partial-visible").count(), 1);
assert_eq!(result.text().matches("event: error").count(), 1);
}
```

Expected now: PASS. This is the green response-started barrier anchor.

- [ ] **Step 2: Add caller schema and redaction assertions**

Use these unique sentinels:

```rust
const BODY_SENTINEL: &str = "UPSTREAM_BODY_SECRET_SENTINEL";
const COOKIE_SENTINEL: &str = "UPSTREAM_COOKIE_SECRET_SENTINEL";
const URL_SENTINEL: &str = "PRIVATE_UPSTREAM_PATH_SENTINEL";
```

Add the shared response and caller test:

```rust
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
fn contract_failure_schema_and_caller_redaction() {
    let result = run_case(AcceptanceCase {
        request: anthropic_request(false),
        is_stream: false,
        use_responses_lite: false,
        endpoint_path: "/PRIVATE_UPSTREAM_PATH_SENTINEL/responses",
        steps: vec![redaction_step()],
    });
    let raw = result.text();
    for forbidden in [
        BODY_SENTINEL,
        COOKIE_SENTINEL,
        URL_SENTINEL,
        ACCESS_SENTINEL,
        ACCOUNT_SENTINEL,
    ] {
        assert!(!raw.contains(forbidden));
    }
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
```

Expected now: redaction and legacy fields PASS; new Provider Failure fields FAIL.

- [ ] **Step 3: Add the real-transport diagnostic projection test**

Start the same 500 response and call `CodexTransport::open_responses` directly with synthetic secrets:

```rust
#[test]
fn contract_transport_diagnostic_projection_is_redacted() {
let upstream = ScriptedCodexUpstream::start(vec![redaction_step()]);
let transport = CodexTransport::for_test(
    upstream.endpoint("/PRIVATE_UPSTREAM_PATH_SENTINEL/responses"),
)
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
for forbidden in [
    BODY_SENTINEL,
    COOKIE_SENTINEL,
    URL_SENTINEL,
    ACCESS_SENTINEL,
    ACCOUNT_SENTINEL,
] {
    assert!(!diagnostic.contains(forbidden));
}
let result = upstream.finish();
assert_eq!(result.requests.len(), 1);
assert_eq!(result.unexpected_posts, 0);
assert!(!result.requests[0].headers.contains_key("authorization"));
assert!(!result.requests[0].headers.contains_key("chatgpt-account-id"));
}
```

Finish the scripted server and assert one request, no stored authorization/account headers, and no unexpected POST. This is a supplemental transport diagnostic assertion because the current handler has no structured diagnostic sink; the evidence report records that missing structured sink as Ticket 05 work instead of fabricating one in the harness.

- [ ] **Step 4: Run all green harness/anchor filters and commit**

```bash
cargo test --offline --manifest-path desktop/gateway/Cargo.toml \
  --features acceptance-build 'server::codex_acceptance::harness_' -- --nocapture
cargo test --offline --manifest-path desktop/gateway/Cargo.toml \
  --features acceptance-build 'server::codex_acceptance::contract_partial_' -- --nocapture
cargo test --offline --manifest-path desktop/gateway/Cargo.toml \
  --features acceptance-build 'server::codex_acceptance::contract_unproven_' -- --nocapture
cargo test --offline --manifest-path desktop/gateway/Cargo.toml \
  --features acceptance-build 'server::codex_acceptance::contract_transport_diagnostic_' -- --nocapture
git add desktop/gateway/src/server/codex_acceptance.rs
git commit -m "test: prove Codex stream and redaction barriers"
```

Expected: every listed green filter passes.

### Task 8: Run the expected-red audit and publish evidence

**Files:**
- Create: `docs/superpowers/reports/2026-07-29-codex-acceptance-harness-expected-red.md`

**Interfaces:**
- Consumes: all harness and contract tests from Tasks 1–7.
- Produces: deterministic reviewer evidence and a precise Ticket 05 implementation map.

- [ ] **Step 1: Format and verify compilation quality**

```bash
cargo fmt --manifest-path desktop/gateway/Cargo.toml -- --check
cargo clippy --offline --manifest-path desktop/gateway/Cargo.toml \
  --features acceptance-build --lib --tests -- -D warnings
```

Expected: both commands pass. Fix only harness formatting or lint findings; do not alter target assertions.

- [ ] **Step 2: Run the validity suite separately from the contract suite**

```bash
cargo test --offline --manifest-path desktop/gateway/Cargo.toml \
  --features acceptance-build 'server::codex_acceptance::harness_' -- --nocapture
cargo test --offline --manifest-path desktop/gateway/Cargo.toml \
  --features acceptance-build 'server::codex_acceptance::contract_' -- --nocapture
cargo test --offline --manifest-path desktop/gateway/Cargo.toml \
  'server::tests::codex_' -- --nocapture
```

Expected:

- harness self-tests: all pass;
- contract run: capability/no-replay/redaction anchors pass, and permanent typing, Provider Failure metadata, retry budgets, and Safe Repair fail at their named target assertions;
- existing focused Codex tests: 16 pass, zero fail.

If a harness test fails, a test hangs, malformed HTTP appears, or a contract fails before reaching its named target assertion, use `systematic-debugging` and correct the harness before documenting evidence.

- [ ] **Step 3: Write the expected-red report**

Create the report with:

- the three exact commands above and the tested Git revision;
- a green harness-validity section listing script ordering, request capture, unused/extra step detection, real handler routing, disconnect, partial SSE, and sentinel scanning;
- a contract table with one row per test, its observed POST count/status/type, target POST count/status/type, and `anchor` or `expected-red` verdict;
- current-gap mappings: classification/metadata to the Provider Failure module, attempt counts to the Attempt Controller integration, header parsing to Codex transport observation extraction, Safe Repair to the closed route-enabled repair, and structured diagnostics to the future diagnostic projection;
- an explicit statement that no live credential, external network endpoint, installed profile, OAuth cache, proxy mutation, or production behavior was used;
- an explicit statement that an overall red contract command is the intended Ticket 03 result and is not claimed as a production pass.

Record the observed values emitted by the test run. If any observation differs from the source-backed expectations in Step 2, diagnose it before writing the row.

- [ ] **Step 4: Commit the evidence**

```bash
git add -f docs/superpowers/reports/2026-07-29-codex-acceptance-harness-expected-red.md
git commit -m "docs: record Codex acceptance expected-red"
git status --short --branch
```

Expected: clean prototype branch with the feature-gated harness and evidence report; the Wayfinder ticket remains claimed.

### Task 9: Human verdict and Wayfinder resolution

**Files:**
- Modify after acceptance: `.scratch/provider-failure-contract/issues/03-prototype-codex-acceptance-harness.md`
- Modify after acceptance: `.scratch/provider-failure-contract/map.md`

**Interfaces:**
- Consumes: prototype branch revision and expected-red report.
- Produces: a human-reviewed Ticket 03 answer that unblocks the rollout-boundary and production-implementation tickets.

- [ ] **Step 1: Present the live review surface**

Give the user the prototype branch revision, evidence-report link, green harness results, green anchors, and each expected-red mismatch. Ask exactly:

> Does this harness make the real Codex attempt behavior, no-replay barriers, sanitized output, and target Provider Failure result sufficiently judgeable for Ticket 05?

- [ ] **Step 2: Wait for the verdict**

Do not modify the Wayfinder ticket or map before the user accepts. If the user requests changes, keep Ticket 03 claimed, update the harness/evidence on the prototype branch, rerun the affected filters, and ask again.

- [ ] **Step 3: Resolve only after acceptance**

After an affirmative verdict, invoke the `wayfinder` skill, return to the clean `linux-headless-oauth` worktree, and update Ticket 03 with:

- `Status: resolved`;
- the accepted harness branch and final revision;
- the exact focused command;
- the green harness result and expected-red contract result;
- the user's verdict;
- the statement that Ticket 05 owns production changes.

Append the one-line resolved decision pointer to `.scratch/provider-failure-contract/map.md`, commit those tracker-only changes, and push the fork branch. Do not resolve any other non-research ticket in this Wayfinder session.
