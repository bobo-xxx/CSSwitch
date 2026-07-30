use std::cell::RefCell;
use std::collections::{BTreeMap, VecDeque};
use std::io::{Error, ErrorKind, Read, Write};
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
    static OPENAI_CHAT_DELIVERY_FAILURE: RefCell<Option<DeliveryFailure>> = const { RefCell::new(None) };
    static CANCEL_ON_RECORDED_WAIT: RefCell<bool> = const { RefCell::new(false) };
}

#[derive(Default)]
struct RecordingState {
    delays_ms: Vec<u64>,
    diagnostics: Vec<Value>,
    delivery_evidence: Option<DeliveryEvidence>,
}

pub(super) fn record_attempt_delay(delay_ms: u64) {
    API_KEY_ACCEPTANCE_RECORDING.with(|slot| {
        if let Some(state) = slot.borrow_mut().as_mut() {
            state.delays_ms.push(delay_ms);
        }
    });
}

pub(super) fn cancel_recorded_wait() -> bool {
    CANCEL_ON_RECORDED_WAIT.with(|slot| *slot.borrow())
}

fn set_cancel_on_recorded_wait(value: bool) {
    CANCEL_ON_RECORDED_WAIT.with(|slot| {
        *slot.borrow_mut() = value;
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

fn record_delivery_evidence(evidence: DeliveryEvidence) {
    API_KEY_ACCEPTANCE_RECORDING.with(|slot| {
        if let Some(state) = slot.borrow_mut().as_mut() {
            state.delivery_evidence = Some(evidence);
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
    QwenChat,
    OpenaiCustomChat,
    GeminiChat,
    GrokChat,
    OpenCodeGoChat,
    OpenaiResponses,
}

#[derive(Clone, Copy, Debug)]
enum OpenedFixture {
    MalformedJson,
    IncompleteBody,
    PartialSseThenClose,
}

#[derive(Clone, Copy, Debug)]
enum DeliveryFailure {
    FinalBodyWrite,
    ChunkWrite,
    TerminalChunkWrite,
    FinalFlush,
}

#[derive(Clone, Debug, Default)]
struct DeliveryEvidence {
    final_body_write_error: bool,
    chunk_write_error: bool,
    terminal_chunk_write_error: bool,
    terminal_write_seen: bool,
    final_flush_error: bool,
    response_head_seen: bool,
}

#[derive(Clone, Debug)]
struct ScriptedDeliveryWriter {
    failure: DeliveryFailure,
    evidence: DeliveryEvidence,
    response_head_buffer: Vec<u8>,
    final_body_pending: bool,
}

impl ScriptedDeliveryWriter {
    fn new(failure: DeliveryFailure) -> Self {
        Self {
            failure,
            evidence: DeliveryEvidence::default(),
            response_head_buffer: Vec::new(),
            final_body_pending: false,
        }
    }
}

impl Write for ScriptedDeliveryWriter {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        if matches!(self.failure, DeliveryFailure::FinalBodyWrite) {
            if self.final_body_pending && !buffer.is_empty() {
                self.evidence.final_body_write_error = true;
                return Err(Error::new(
                    ErrorKind::BrokenPipe,
                    "scripted final JSON body write failure",
                ));
            }
            self.response_head_buffer.extend_from_slice(buffer);
            if let Some(head_end) = self
                .response_head_buffer
                .windows(4)
                .position(|candidate| candidate == b"\r\n\r\n")
            {
                self.evidence.response_head_seen = true;
                if self.response_head_buffer.len() > head_end + 4 {
                    self.evidence.final_body_write_error = true;
                    return Err(Error::new(
                        ErrorKind::BrokenPipe,
                        "scripted final JSON body write failure",
                    ));
                }
                self.final_body_pending = true;
            }
        }
        if matches!(self.failure, DeliveryFailure::ChunkWrite)
            && buffer
                .windows(b"event: content_block_start".len())
                .any(|candidate| candidate == b"event: content_block_start")
        {
            self.evidence.chunk_write_error = true;
            return Err(Error::new(
                ErrorKind::BrokenPipe,
                "scripted OpenAI Chat chunk write failure",
            ));
        }
        if buffer.windows(5).any(|candidate| candidate == b"0\r\n\r\n") {
            self.evidence.terminal_write_seen = true;
            if matches!(self.failure, DeliveryFailure::TerminalChunkWrite) {
                self.evidence.terminal_chunk_write_error = true;
                return Err(Error::new(
                    ErrorKind::BrokenPipe,
                    "scripted terminal chunk write failure",
                ));
            }
        }
        Ok(buffer.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        if matches!(self.failure, DeliveryFailure::FinalFlush) && self.evidence.terminal_write_seen
        {
            self.evidence.final_flush_error = true;
            return Err(Error::new(
                ErrorKind::BrokenPipe,
                "scripted OpenAI Chat final flush failure",
            ));
        }
        Ok(())
    }
}

pub(super) fn provider_delivery_result<T>(deliver: impl FnOnce(&mut dyn Write) -> T) -> Option<T> {
    let failure = OPENAI_CHAT_DELIVERY_FAILURE.with(|slot| *slot.borrow());
    failure.map(|failure| {
        let mut writer = ScriptedDeliveryWriter::new(failure);
        let result = deliver(&mut writer);
        record_delivery_evidence(writer.evidence);
        result
    })
}

pub(super) fn delivery_failure_enabled() -> bool {
    OPENAI_CHAT_DELIVERY_FAILURE.with(|slot| slot.borrow().is_some())
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

#[derive(Clone, Copy, Debug)]
enum NetworkFixture {
    ConnectRefused,
    Timeout,
}

struct TimeoutUpstream {
    address: SocketAddr,
    requests: Arc<Mutex<Vec<CapturedRequest>>>,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl TimeoutUpstream {
    fn start() -> Self {
        let listener = bind_loopback();
        listener
            .set_nonblocking(true)
            .expect("make timeout upstream nonblocking");
        let address = listener
            .local_addr()
            .expect("read timeout upstream address");
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let requests_for_thread = Arc::clone(&requests);
        let stop_for_thread = Arc::clone(&stop);
        let thread = thread::spawn(move || {
            let mut workers = Vec::new();
            while !stop_for_thread.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let requests = Arc::clone(&requests_for_thread);
                        let stop = Arc::clone(&stop_for_thread);
                        workers.push(thread::spawn(move || {
                            if let Ok(request) = read_request(&mut stream) {
                                requests
                                    .lock()
                                    .expect("lock timeout requests")
                                    .push(request);
                            }
                            while !stop.load(Ordering::Acquire) {
                                thread::sleep(Duration::from_millis(1));
                            }
                        }));
                    }
                    Err(error) if error.kind() == ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(1));
                    }
                    Err(error) => panic!("timeout upstream accept failed: {error}"),
                }
            }
            for worker in workers {
                worker.join().expect("join timeout upstream worker");
            }
        });
        Self {
            address,
            requests,
            stop,
            thread: Some(thread),
        }
    }

    fn endpoint(&self, route: FixtureRoute) -> String {
        endpoint_for(self.address, route)
    }

    fn finish(mut self) -> Vec<CapturedRequest> {
        self.stop.store(true, Ordering::Release);
        self.thread
            .take()
            .expect("timeout upstream thread")
            .join()
            .expect("join timeout upstream");
        self.requests.lock().expect("lock timeout requests").clone()
    }
}

impl Drop for TimeoutUpstream {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
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

    fn endpoint(&self, route: FixtureRoute) -> String {
        endpoint_for(self.address, route)
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

fn endpoint_for(address: SocketAddr, route: FixtureRoute) -> String {
    let path = match route {
        FixtureRoute::RelayAnthropic
        | FixtureRoute::RelayKimi
        | FixtureRoute::DeepseekAnthropic => "/v1/messages",
        FixtureRoute::QwenChat
        | FixtureRoute::OpenaiCustomChat
        | FixtureRoute::GeminiChat
        | FixtureRoute::GrokChat
        | FixtureRoute::OpenCodeGoChat => "/v1/chat/completions",
        FixtureRoute::OpenaiResponses => "/responses",
    };
    format!("http://{address}{path}")
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
    delivery_evidence: Option<DeliveryEvidence>,
}

struct AcceptanceFixture {
    upstream: ScriptedUpstream,
    gateway: TcpListener,
}

impl AcceptanceFixture {
    fn new(route: FixtureRoute) -> Self {
        let upstream = ScriptedUpstream::start(Vec::new());
        let _endpoint = upstream.endpoint(route);
        let gateway = bind_loopback();
        Self { upstream, gateway }
    }

    fn upstream_port(&self) -> u16 {
        self.upstream.address.port()
    }

    fn gateway_port(&self) -> u16 {
        self.gateway
            .local_addr()
            .expect("read gateway fixture address")
            .port()
    }
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
        FixtureRoute::QwenChat => ("qwen", "qwen-native", "qwen-max", None),
        FixtureRoute::OpenaiCustomChat => (
            "openai-custom",
            "custom-openai-chat",
            "openai-custom-model",
            None,
        ),
        FixtureRoute::GeminiChat => (
            "openai-custom",
            "gemini-openai-chat",
            "gemini-2.5-pro",
            None,
        ),
        FixtureRoute::GrokChat => ("openai-custom", "grok-openai-chat", "grok-4", None),
        FixtureRoute::OpenCodeGoChat => (
            "openai-custom",
            "opencode-go-openai-chat",
            "opencode-go-chat",
            None,
        ),
        FixtureRoute::OpenaiResponses => (
            "openai-responses",
            "custom-openai-responses",
            "openai-responses-model",
            None,
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

fn network_fixture_config(route: FixtureRoute, upstream_url: String) -> GatewayConfig {
    let mut cfg = config(route, upstream_url);
    let contract = cfg
        .provider_contract
        .as_mut()
        .expect("network fixture requires managed provider contract");
    contract.connect_timeout = Duration::from_millis(20);
    contract.request_timeout = Duration::from_millis(20);
    contract.read_idle_timeout = Duration::from_millis(20);
    cfg
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
    run_with_downstream(route, mode, steps, |handler| capture_downstream(handler))
}

fn run_with_downstream(
    route: FixtureRoute,
    mode: AttemptMode,
    steps: Vec<UpstreamStep>,
    capture: impl FnOnce(Box<dyn FnOnce(&mut TcpStream) + Send>) -> Vec<u8>,
) -> AcceptanceResult {
    let upstream = ScriptedUpstream::start(steps);
    let cfg = config(route, upstream.endpoint(route));
    let request_nonces = RequestNonceGenerator::with_prefix([0x42; 16]);
    let relay_models = models::RelayModelCache::default();
    start_recording();
    let downstream = capture(Box::new(move |stream| {
        handle_messages(
            stream,
            &cfg,
            serde_json::to_vec(&request(
                mode,
                matches!(
                    route,
                    FixtureRoute::DeepseekAnthropic | FixtureRoute::OpenaiResponses
                ),
            ))
            .expect("serialize caller request"),
            Some(&request_nonces),
            &relay_models,
            CodexComponents::default(),
        );
    }));
    let recording = finish_recording();
    AcceptanceResult {
        downstream,
        script: upstream.finish(),
        delays_ms: recording.delays_ms,
        diagnostics: recording.diagnostics,
        delivery_evidence: recording.delivery_evidence,
    }
}

fn run_network_endpoint(route: FixtureRoute, endpoint: String) -> (Vec<u8>, RecordingState) {
    let cfg = network_fixture_config(route, endpoint);
    let request_nonces = RequestNonceGenerator::with_prefix([0x42; 16]);
    let relay_models = models::RelayModelCache::default();
    start_recording();
    let downstream = capture_downstream(|stream| {
        handle_messages(
            stream,
            &cfg,
            serde_json::to_vec(&request(
                AttemptMode::Nonstream,
                matches!(
                    route,
                    FixtureRoute::DeepseekAnthropic | FixtureRoute::OpenaiResponses
                ),
            ))
            .expect("serialize caller request"),
            Some(&request_nonces),
            &relay_models,
            CodexComponents::default(),
        );
    });
    (downstream, finish_recording())
}

fn run_with_openai_chat_delivery_failure(
    route: FixtureRoute,
    mode: AttemptMode,
    steps: Vec<UpstreamStep>,
    failure: DeliveryFailure,
) -> AcceptanceResult {
    OPENAI_CHAT_DELIVERY_FAILURE.with(|slot| {
        *slot.borrow_mut() = Some(failure);
    });
    let result = run(route, mode, steps);
    OPENAI_CHAT_DELIVERY_FAILURE.with(|slot| {
        *slot.borrow_mut() = None;
    });
    result
}

fn request_id_sentinel(route: FixtureRoute) -> &'static str {
    match route {
        FixtureRoute::RelayAnthropic | FixtureRoute::RelayKimi => "Req-secret-01",
        FixtureRoute::DeepseekAnthropic => "Req-secret-02",
        FixtureRoute::QwenChat
        | FixtureRoute::OpenaiCustomChat
        | FixtureRoute::GeminiChat
        | FixtureRoute::GrokChat
        | FixtureRoute::OpenCodeGoChat => "Req-secret-03",
        FixtureRoute::OpenaiResponses => "Req-secret-04",
    }
}

fn failure_body(route: FixtureRoute, status: u16, error_code: ErrorCode) -> Value {
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
        "request_id": request_id_sentinel(route),
        "cross_request_id": "Req-secret-cross-route",
        "host": PRIVATE_HOST_SENTINEL
    });
    if !code.is_empty() {
        error["code"] = json!(code);
    }
    json!({"error": error, "status": status})
}

fn assert_script(route: FixtureRoute, result: &AcceptanceResult, expected_posts: usize) {
    assert_eq!(result.script.requests.len(), expected_posts);
    assert_eq!(result.script.remaining_steps, 0);
    assert_eq!(result.script.unexpected_posts, 0);
    assert_eq!(result.script.unexpected_methods, 0);
    assert_eq!(result.script.rejected_requests, 0);
    for request in &result.script.requests {
        assert_eq!(request.method, "POST");
        match route {
            FixtureRoute::RelayAnthropic | FixtureRoute::RelayKimi => {
                assert_eq!(request.path, "/v1/messages");
                assert_eq!(
                    request.headers.get("x-api-key").map(String::as_str),
                    Some(API_KEY_SENTINEL),
                    "Anthropic relay dual auth must include synthetic x-api-key"
                );
                assert_eq!(
                    request.headers.get("authorization").map(String::as_str),
                    Some("Bearer fixture-api-key"),
                    "Anthropic relay dual auth must include synthetic bearer"
                );
            }
            FixtureRoute::DeepseekAnthropic => {
                assert_eq!(request.path, "/v1/messages");
                assert_eq!(
                    request.headers.get("x-api-key").map(String::as_str),
                    Some(API_KEY_SENTINEL),
                    "DeepSeek x-api-key auth must send synthetic API key"
                );
                assert!(
                    !request.headers.contains_key("authorization"),
                    "DeepSeek x-api-key auth must not add bearer auth"
                );
            }
            FixtureRoute::QwenChat
            | FixtureRoute::OpenaiCustomChat
            | FixtureRoute::GeminiChat
            | FixtureRoute::GrokChat
            | FixtureRoute::OpenCodeGoChat => {
                assert_eq!(request.path, "/v1/chat/completions");
                assert!(
                    !request.headers.contains_key("x-api-key"),
                    "OpenAI Chat bearer auth must not add x-api-key"
                );
                assert_eq!(
                    request.headers.get("authorization").map(String::as_str),
                    Some("Bearer fixture-api-key"),
                    "OpenAI Chat routes must send synthetic bearer"
                );
                let body: Value =
                    serde_json::from_slice(&request.body).expect("OpenAI Chat request JSON");
                assert_eq!(body["model"], expected_upstream_model(route));
                assert_eq!(body["messages"][0]["role"], "user");
                assert_eq!(body["messages"][0]["content"], "hello");
            }
            FixtureRoute::OpenaiResponses => {
                assert_eq!(request.path, "/responses");
                assert!(
                    !request.headers.contains_key("x-api-key"),
                    "OpenAI Responses bearer auth must not add x-api-key"
                );
                assert_eq!(
                    request.headers.get("authorization").map(String::as_str),
                    Some("Bearer fixture-api-key"),
                    "OpenAI Responses routes must send synthetic bearer"
                );
                let body: Value =
                    serde_json::from_slice(&request.body).expect("OpenAI Responses request JSON");
                assert_eq!(body["model"], expected_upstream_model(route));
                assert_eq!(body["input"][0]["role"], "user");
                assert_eq!(body["input"][0]["content"], "hello");
                assert_eq!(body["tools"][0]["type"], "function");
                assert_eq!(body["tools"][0]["name"], "web_search");
                assert_eq!(body["tool_choice"], "auto");
            }
        }
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

fn assert_diagnostic_keys(route: FixtureRoute, expected: &[&str]) {
    let result = run(
        route,
        AttemptMode::Nonstream,
        vec![success_step(route, AttemptMode::Nonstream)],
    );
    assert_script(route, &result, 1);
    assert_one_final_diagnostic(&result, "completed");
    let mut actual = result.diagnostics[0]
        .as_object()
        .expect("diagnostic object")
        .keys()
        .map(String::as_str)
        .collect::<Vec<_>>();
    actual.sort_unstable();
    let mut expected = expected.to_vec();
    expected.sort_unstable();
    assert_eq!(actual, expected);
}

fn assert_diagnostic_reason(route: FixtureRoute) {
    let completed = run(
        route,
        AttemptMode::Nonstream,
        vec![success_step(route, AttemptMode::Nonstream)],
    );
    assert_script(route, &completed, 1);
    assert_eq!(
        completed.diagnostics[0]["reason"],
        json!({"kind": "completed", "delay_count": 0})
    );

    let failed = run(
        route,
        AttemptMode::Nonstream,
        vec![
            UpstreamStep::json(
                503,
                "Unavailable",
                failure_body(route, 503, ErrorCode::RateLimitError),
            ),
            UpstreamStep::json(
                503,
                "Unavailable",
                failure_body(route, 503, ErrorCode::RateLimitError),
            ),
            UpstreamStep::json(
                503,
                "Unavailable",
                failure_body(route, 503, ErrorCode::RateLimitError),
            ),
        ],
    );
    assert_script(route, &failed, 3);
    assert_eq!(
        failed.diagnostics[0]["reason"],
        json!({
            "kind": "failed",
            "delay_count": 2,
            "mapped_status": 502,
            "upstream_status": 503,
            "failure_class": "transient",
            "retryable": true
        })
    );
}

fn assert_failure_response(route: FixtureRoute, result: &AcceptanceResult, status: u16) {
    assert_eq!(result.status(), status);
    let body = result.json();
    assert_eq!(body["type"], "error");
    let expected_route = match route {
        FixtureRoute::RelayAnthropic
        | FixtureRoute::RelayKimi
        | FixtureRoute::DeepseekAnthropic => "anthropic_messages",
        FixtureRoute::QwenChat
        | FixtureRoute::OpenaiCustomChat
        | FixtureRoute::GeminiChat
        | FixtureRoute::GrokChat
        | FixtureRoute::OpenCodeGoChat => "openai_chat",
        FixtureRoute::OpenaiResponses => "openai_responses",
    };
    assert_eq!(body["error"]["route"], expected_route);
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
            failure_body(route, status, error_code),
        )],
    );
    assert_script(route, &result, expected_posts);
    assert!(result.delays_ms.is_empty());
    assert_failure_response(route, &result, status);
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
                failure_body(route, first_status, ErrorCode::RateLimitError),
                headers,
            ),
            success_step(route, mode),
        ],
    );
    assert_script(route, &result, 2);
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
                failure_body(route, status, ErrorCode::RateLimitError),
            ),
            UpstreamStep::json(
                status,
                "Unavailable",
                failure_body(route, status, ErrorCode::RateLimitError),
            ),
            UpstreamStep::json(
                status,
                "Unavailable",
                failure_body(route, status, ErrorCode::RateLimitError),
            ),
        ],
    );
    assert_script(route, &result, 3);
    assert_eq!(result.delays_ms, expected_delays_ms);
    assert_failure_response(route, &result, 502);
    assert_one_final_diagnostic(&result, "failed");
}

fn assert_real_network_exhaustion(
    route: FixtureRoute,
    fixture: NetworkFixture,
    expected_status: u16,
) {
    let (downstream, recording, requests) = match fixture {
        NetworkFixture::ConnectRefused => {
            let listener = bind_loopback();
            let address = listener.local_addr().expect("read refused address");
            drop(listener);
            let (downstream, recording) = run_network_endpoint(route, endpoint_for(address, route));
            (downstream, recording, None)
        }
        NetworkFixture::Timeout => {
            let upstream = TimeoutUpstream::start();
            let endpoint = upstream.endpoint(route);
            let (downstream, recording) = run_network_endpoint(route, endpoint);
            (downstream, recording, Some(upstream.finish()))
        }
    };
    let result = AcceptanceResult {
        downstream,
        script: ScriptResult {
            requests: Vec::new(),
            remaining_steps: 0,
            unexpected_posts: 0,
            unexpected_methods: 0,
            rejected_requests: 0,
        },
        delays_ms: recording.delays_ms,
        diagnostics: recording.diagnostics,
        delivery_evidence: recording.delivery_evidence,
    };
    assert_eq!(result.delays_ms, [500, 1_000]);
    assert_failure_response(route, &result, expected_status);
    assert_one_final_diagnostic(&result, "failed");
    let diagnostic = &result.diagnostics[0];
    assert_eq!(diagnostic["posts"], 3);
    assert_eq!(diagnostic["repairs"], 0);
    assert_eq!(
        diagnostic["reason"],
        json!({
            "kind": "failed",
            "delay_count": 2,
            "mapped_status": expected_status,
            "failure_class": "network",
            "retryable": true
        })
    );
    assert!(!result.text().contains(API_KEY_SENTINEL));
    assert!(!result.text().contains(SECRET_UPSTREAM_TEXT));
    assert!(!result.text().contains(PRIVATE_HOST_SENTINEL));
    assert_no_diagnostic_sentinels_for_test(
        &result.diagnostics,
        &[
            API_KEY_SENTINEL,
            SECRET_UPSTREAM_TEXT,
            PRIVATE_HOST_SENTINEL,
        ],
    );
    let outcomes = result
        .diagnostics
        .iter()
        .filter_map(|value| value["outcome"].as_str())
        .collect::<Vec<_>>();
    assert_eq!(outcomes, ["failed"]);
    assert!(!outcomes.contains(&"completed"));
    assert!(!outcomes.contains(&"cancelled"));

    if let Some(requests) = requests {
        assert_eq!(
            requests.len(),
            3,
            "timeout fixture must receive three POSTs"
        );
        for request in &requests {
            assert_eq!(request.method, "POST");
            assert_eq!(endpoint_for_path(route), request.path);
        }
        for request in &requests[1..] {
            assert_eq!(request.body, requests[0].body, "retried body bytes changed");
        }
    }
}

fn endpoint_for_path(route: FixtureRoute) -> &'static str {
    match route {
        FixtureRoute::RelayAnthropic
        | FixtureRoute::RelayKimi
        | FixtureRoute::DeepseekAnthropic => "/v1/messages",
        FixtureRoute::QwenChat
        | FixtureRoute::OpenaiCustomChat
        | FixtureRoute::GeminiChat
        | FixtureRoute::GrokChat
        | FixtureRoute::OpenCodeGoChat => "/v1/chat/completions",
        FixtureRoute::OpenaiResponses => "/responses",
    }
}

fn assert_opened_failure(route: FixtureRoute, mode: AttemptMode, opened_fixture: OpenedFixture) {
    let step = match opened_fixture {
        OpenedFixture::MalformedJson => {
            UpstreamStep::bytes(200, "OK", "application/json", b"{not-json".to_vec())
        }
        OpenedFixture::IncompleteBody => {
            UpstreamStep::PartialSseThenDrop(b"{\"id\":\"resp_partial\"".to_vec())
        }
        OpenedFixture::PartialSseThenClose => {
            UpstreamStep::PartialSseThenDrop(b"event: message_start\n".to_vec())
        }
    };
    let result = run(route, mode, vec![step]);
    assert_script(route, &result, 1);
    assert!(result.delays_ms.is_empty());
    assert_one_final_diagnostic(&result, "failed");
}

fn assert_success_fixture(route: FixtureRoute, mode: AttemptMode, fixture_name: &str) {
    let result = run(route, mode, vec![success_step(route, mode)]);
    assert_script(route, &result, 1);
    assert!(result.delays_ms.is_empty());
    assert_success_body(route, mode, &result);
    let actual = match mode {
        AttemptMode::Nonstream => result.body().to_vec(),
        AttemptMode::Stream => result.sse_body(),
    };
    assert_eq!(
        String::from_utf8_lossy(&actual),
        expected_success_fixture(fixture_name),
        "literal success fixture {fixture_name} changed"
    );
    assert_one_final_diagnostic(&result, "completed");
}

fn assert_delivery_failure(route: FixtureRoute, mode: AttemptMode, failure: DeliveryFailure) {
    let result = run_with_openai_chat_delivery_failure(
        route,
        mode,
        vec![success_step(route, mode)],
        failure,
    );
    assert_script(route, &result, 1);
    assert!(result.delays_ms.is_empty());
    let evidence = result
        .delivery_evidence
        .as_ref()
        .expect("delivery failure fixture must record exact delivery evidence");
    match failure {
        DeliveryFailure::FinalBodyWrite => {
            assert!(evidence.final_body_write_error);
            assert!(!evidence.chunk_write_error);
            assert!(!evidence.terminal_chunk_write_error);
            assert!(!evidence.terminal_write_seen);
            assert!(!evidence.final_flush_error);
        }
        DeliveryFailure::ChunkWrite => {
            assert!(!evidence.final_body_write_error);
            assert!(evidence.chunk_write_error);
            assert!(!evidence.terminal_chunk_write_error);
            assert!(!evidence.terminal_write_seen);
            assert!(!evidence.final_flush_error);
        }
        DeliveryFailure::TerminalChunkWrite => {
            assert!(!evidence.final_body_write_error);
            assert!(!evidence.chunk_write_error);
            assert!(evidence.terminal_write_seen);
            assert!(evidence.terminal_chunk_write_error);
            assert!(!evidence.final_flush_error);
        }
        DeliveryFailure::FinalFlush => {
            assert!(!evidence.final_body_write_error);
            assert!(!evidence.chunk_write_error);
            assert!(!evidence.terminal_chunk_write_error);
            assert!(evidence.terminal_write_seen);
            assert!(evidence.final_flush_error);
        }
    }
    assert_one_final_diagnostic(&result, "cancelled");
    let outcomes = result
        .diagnostics
        .iter()
        .filter_map(|diagnostic| diagnostic["outcome"].as_str())
        .collect::<Vec<_>>();
    assert_eq!(outcomes, vec!["cancelled"]);
    assert!(!outcomes.contains(&"failed"));
    assert!(!outcomes.contains(&"completed"));
}

fn assert_wait_cancellation(
    route: FixtureRoute,
    status: u16,
    expected_posts: usize,
    expected_delays_ms: &[u64],
) {
    set_cancel_on_recorded_wait(true);
    let result = run_with_downstream(
        route,
        AttemptMode::Nonstream,
        vec![
            UpstreamStep::json(
                status,
                "Retryable",
                failure_body(route, status, ErrorCode::RateLimitError),
            ),
            success_step(route, AttemptMode::Nonstream),
        ],
        |handler| {
            let listener = bind_loopback();
            let address = listener.local_addr().expect("read downstream address");
            let reader = thread::spawn(move || {
                let stream = TcpStream::connect(address).expect("connect downstream reader");
                thread::sleep(Duration::from_millis(25));
                drop(stream);
                Vec::new()
            });
            let (mut stream, _) = listener.accept().expect("accept downstream");
            handler(&mut stream);
            drop(stream);
            reader.join().expect("join downstream reader")
        },
    );
    set_cancel_on_recorded_wait(false);
    assert_eq!(result.script.requests.len(), expected_posts);
    assert_eq!(result.script.remaining_steps, 1);
    assert_eq!(result.script.unexpected_posts, 0);
    assert_eq!(result.script.unexpected_methods, 0);
    assert_eq!(result.script.rejected_requests, 0);
    assert_eq!(result.delays_ms, expected_delays_ms);
    assert_one_final_diagnostic(&result, "cancelled");
}

fn assert_unsupported_tool_choice_is_terminal(
    route: FixtureRoute,
    status: u16,
    expected_posts: usize,
    expected_delays: usize,
) {
    let result = run(
        route,
        AttemptMode::Nonstream,
        vec![UpstreamStep::json(
            status,
            "Unsupported Tool Choice",
            json!({
                "error": {
                    "message": "automatic tool_choice is unsupported",
                    "code": "unsupported_value",
                    "param": "tool_choice",
                    "request_id": request_id_sentinel(route),
                    "host": PRIVATE_HOST_SENTINEL
                }
            }),
        )],
    );
    assert_script(route, &result, expected_posts);
    assert_eq!(result.delays_ms.len(), expected_delays);
    assert_failure_response(route, &result, status);
    assert_one_final_diagnostic(&result, "failed");
}

fn assert_redacted(route: FixtureRoute, sentinels: &[&str]) {
    let result = run(
        route,
        AttemptMode::Nonstream,
        vec![
            UpstreamStep::json(
                503,
                "Unavailable",
                failure_body(route, 503, ErrorCode::RateLimitError),
            ),
            UpstreamStep::json(
                503,
                "Unavailable",
                failure_body(route, 503, ErrorCode::RateLimitError),
            ),
            UpstreamStep::json(
                503,
                "Unavailable",
                failure_body(route, 503, ErrorCode::RateLimitError),
            ),
        ],
    );
    assert_script(route, &result, 3);
    let caller_text = result.text();
    for sentinel in sentinels {
        assert!(
            !caller_text.contains(sentinel),
            "downstream leaked sentinel {sentinel}: {caller_text}"
        );
    }
    assert_no_diagnostic_sentinels_for_test(&result.diagnostics, sentinels);
    assert_one_final_diagnostic(&result, "failed");
}

#[cfg(test)]
fn assert_no_diagnostic_sentinels_for_test(diagnostics: &[Value], sentinels: &[&str]) {
    let diagnostics_text =
        serde_json::to_string(diagnostics).expect("serialize acceptance diagnostics");
    for sentinel in sentinels {
        assert!(
            !diagnostics_text.contains(sentinel),
            "diagnostic leaked sentinel {sentinel}: {diagnostics_text}"
        );
    }
}

fn expected_success_fixture(fixture_name: &str) -> &'static str {
    match fixture_name {
        "kimi-nonstream-filter" => "{\"id\":\"msg_kimi\",\"type\":\"message\",\"role\":\"assistant\",\"model\":\"kimi-k2.7-code\",\"content\":[{\"type\":\"text\",\"text\":\"kimi ok\"}],\"stop_reason\":\"end_turn\",\"stop_sequence\":null,\"usage\":{\"input_tokens\":1,\"output_tokens\":2}}",
        "kimi-stream-filter" => "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_stream\",\"type\":\"message\",\"role\":\"assistant\",\"model\":\"upstream\",\"content\":[],\"stop_reason\":null,\"stop_sequence\":null,\"usage\":{\"input_tokens\":1,\"output_tokens\":0}}}\n\nevent: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\nevent: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"stream ok\"}}\n\nevent: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\nevent: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\",\"stop_sequence\":null},\"usage\":{\"output_tokens\":2}}\n\nevent: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
        "deepseek-dsml-nonstream" => "{\"id\":\"msg_ok\",\"type\":\"message\",\"role\":\"assistant\",\"model\":\"upstream\",\"content\":[{\"type\":\"text\",\"text\":\"hello \"},{\"type\":\"tool_use\",\"id\":\"toolu_dsml_424242424242424242424242424242420000000000000001_1\",\"name\":\"web_search\",\"input\":{\"query\":\"q\"}},{\"type\":\"text\",\"text\":\" done\"}],\"stop_reason\":\"tool_use\",\"stop_sequence\":null,\"usage\":{\"input_tokens\":1,\"output_tokens\":2}}",
        "deepseek-dsml-stream" => "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_stream\",\"type\":\"message\",\"role\":\"assistant\",\"model\":\"upstream\",\"content\":[],\"stop_reason\":null,\"stop_sequence\":null,\"usage\":{\"input_tokens\":1,\"output_tokens\":0}}}\n\nevent: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\nevent: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"stream ok\"}}\n\nevent: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\nevent: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\",\"stop_sequence\":null},\"usage\":{\"output_tokens\":2}}\n\nevent: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
        "qwen-chat" => "{\"id\":\"chatcmpl_qwen\",\"type\":\"message\",\"role\":\"assistant\",\"model\":\"claude-sonnet-4-20250514\",\"content\":[{\"type\":\"text\",\"text\":\"qwen ok\"}],\"stop_reason\":\"end_turn\",\"stop_sequence\":null,\"usage\":{\"input_tokens\":3,\"output_tokens\":4}}",
        "openai-custom-chat" => "{\"id\":\"chatcmpl_custom\",\"type\":\"message\",\"role\":\"assistant\",\"model\":\"claude-sonnet-4-20250514\",\"content\":[{\"type\":\"text\",\"text\":\"custom ok\"}],\"stop_reason\":\"end_turn\",\"stop_sequence\":null,\"usage\":{\"input_tokens\":3,\"output_tokens\":4}}",
        "gemini-chat" => "{\"id\":\"chatcmpl_gemini\",\"type\":\"message\",\"role\":\"assistant\",\"model\":\"claude-sonnet-4-20250514\",\"content\":[{\"type\":\"text\",\"text\":\"gemini ok\"}],\"stop_reason\":\"end_turn\",\"stop_sequence\":null,\"usage\":{\"input_tokens\":3,\"output_tokens\":4}}",
        "grok-chat" => "{\"id\":\"chatcmpl_grok\",\"type\":\"message\",\"role\":\"assistant\",\"model\":\"claude-sonnet-4-20250514\",\"content\":[{\"type\":\"text\",\"text\":\"grok ok\"}],\"stop_reason\":\"end_turn\",\"stop_sequence\":null,\"usage\":{\"input_tokens\":3,\"output_tokens\":4}}",
        "opencode-go-chat" => "{\"id\":\"chatcmpl_opencode_go\",\"type\":\"message\",\"role\":\"assistant\",\"model\":\"claude-sonnet-4-20250514\",\"content\":[{\"type\":\"text\",\"text\":\"opencode go ok\"}],\"stop_reason\":\"end_turn\",\"stop_sequence\":null,\"usage\":{\"input_tokens\":3,\"output_tokens\":4}}",
        "openai-chat-local-sse-replay" => "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"chatcmpl_custom\",\"type\":\"message\",\"role\":\"assistant\",\"model\":\"claude-sonnet-4-20250514\",\"content\":[],\"stop_reason\":null,\"stop_sequence\":null,\"usage\":{\"input_tokens\":3,\"output_tokens\":4}}}\n\nevent: ping\ndata: {\"type\":\"ping\"}\n\nevent: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\nevent: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"custom ok\"}}\n\nevent: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\nevent: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\",\"stop_sequence\":null},\"usage\":{\"output_tokens\":4}}\n\nevent: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
        "openai-responses-metadata-map" => "{\"id\":\"resp_ok\",\"type\":\"message\",\"role\":\"assistant\",\"model\":\"claude-sonnet-4-20250514\",\"content\":[{\"type\":\"text\",\"text\":\"responses ok\"},{\"type\":\"tool_use\",\"id\":\"call_resp\",\"name\":\"web_search\",\"input\":{\"query\":\"q\"}}],\"stop_reason\":\"tool_use\",\"stop_sequence\":null,\"usage\":{\"input_tokens\":5,\"output_tokens\":6}}",
        "openai-responses-local-sse-replay" => "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"resp_ok\",\"type\":\"message\",\"role\":\"assistant\",\"model\":\"claude-sonnet-4-20250514\",\"content\":[],\"stop_reason\":null,\"stop_sequence\":null,\"usage\":{\"input_tokens\":5,\"output_tokens\":6}}}\n\nevent: ping\ndata: {\"type\":\"ping\"}\n\nevent: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\nevent: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"responses ok\"}}\n\nevent: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\nevent: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"tool_use\",\"id\":\"call_resp\",\"name\":\"web_search\",\"input\":{}}}\n\nevent: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"query\\\": \\\"q\\\"}\"}}\n\nevent: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":1}\n\nevent: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\",\"stop_sequence\":null},\"usage\":{\"output_tokens\":6}}\n\nevent: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
        other => panic!("unknown success fixture {other}"),
    }
}

fn expected_upstream_model(route: FixtureRoute) -> &'static str {
    match route {
        FixtureRoute::RelayAnthropic => "relay-claude",
        FixtureRoute::RelayKimi => "kimi-k2.7-code",
        FixtureRoute::DeepseekAnthropic => "deepseek-chat",
        FixtureRoute::QwenChat => "qwen-max",
        FixtureRoute::OpenaiCustomChat => "openai-custom-model",
        FixtureRoute::GeminiChat => "gemini-2.5-pro",
        FixtureRoute::GrokChat => "grok-4",
        FixtureRoute::OpenCodeGoChat => "opencode-go-chat",
        FixtureRoute::OpenaiResponses => "openai-responses-model",
    }
}

fn openai_chat_success_body(route: FixtureRoute) -> Value {
    let (id, text) = match route {
        FixtureRoute::QwenChat => ("chatcmpl_qwen", "qwen ok"),
        FixtureRoute::OpenaiCustomChat => ("chatcmpl_custom", "custom ok"),
        FixtureRoute::GeminiChat => ("chatcmpl_gemini", "gemini ok"),
        FixtureRoute::GrokChat => ("chatcmpl_grok", "grok ok"),
        FixtureRoute::OpenCodeGoChat => ("chatcmpl_opencode_go", "opencode go ok"),
        _ => unreachable!("OpenAI Chat success body requires OpenAI Chat route"),
    };
    json!({
        "id": id,
        "object": "chat.completion",
        "created": 1_700_000_000_u64,
        "model": expected_upstream_model(route),
        "choices": [{
            "index": 0,
            "message": {"role": "assistant", "content": text},
            "finish_reason": "stop"
        }],
        "usage": {"prompt_tokens": 3, "completion_tokens": 4, "total_tokens": 7}
    })
}

fn openai_responses_success_body() -> Value {
    json!({
        "id": "resp_ok",
        "status": "completed",
        "output": [
            {
                "type": "message",
                "content": [{
                    "type": "output_text",
                    "text": "responses ok"
                }]
            },
            {
                "type": "function_call",
                "call_id": "call_resp",
                "name": "web_search",
                "arguments": "{\"query\":\"q\"}"
            }
        ],
        "usage": {
            "input_tokens": 5,
            "output_tokens": 6
        }
    })
}

fn success_step(route: FixtureRoute, mode: AttemptMode) -> UpstreamStep {
    match mode {
        AttemptMode::Nonstream => {
            let body = match route {
                FixtureRoute::RelayKimi => json!({
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
                }),
                FixtureRoute::RelayAnthropic | FixtureRoute::DeepseekAnthropic => json!({
                    "id": "msg_ok",
                    "type": "message",
                    "role": "assistant",
                    "model": "upstream",
                    "content": [{"type":"text","text":"hello <｜DSML｜tool_calls> <｜DSML｜invoke name=\"web_search\"><｜DSML｜parameter name=\"query\">q</｜DSML｜parameter></｜DSML｜invoke> </｜DSML｜tool_calls> done"}],
                    "stop_reason": "end_turn",
                    "stop_sequence": null,
                    "usage": {"input_tokens": 1, "output_tokens": 2}
                }),
                FixtureRoute::QwenChat
                | FixtureRoute::OpenaiCustomChat
                | FixtureRoute::GeminiChat
                | FixtureRoute::GrokChat
                | FixtureRoute::OpenCodeGoChat => openai_chat_success_body(route),
                FixtureRoute::OpenaiResponses => openai_responses_success_body(),
            };
            UpstreamStep::json(200, "OK", body)
        }
        AttemptMode::Stream => match route {
            FixtureRoute::QwenChat
            | FixtureRoute::OpenaiCustomChat
            | FixtureRoute::GeminiChat
            | FixtureRoute::GrokChat
            | FixtureRoute::OpenCodeGoChat => UpstreamStep::json(200, "OK", openai_chat_success_body(route)),
            FixtureRoute::OpenaiResponses => {
                UpstreamStep::json(200, "OK", openai_responses_success_body())
            }
            FixtureRoute::RelayAnthropic | FixtureRoute::RelayKimi | FixtureRoute::DeepseekAnthropic => {
                UpstreamStep::bytes(
                    200,
                    "OK",
                    "text/event-stream",
                    b"event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_stream\",\"type\":\"message\",\"role\":\"assistant\",\"model\":\"upstream\",\"content\":[],\"stop_reason\":null,\"stop_sequence\":null,\"usage\":{\"input_tokens\":1,\"output_tokens\":0}}}\n\nevent: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\nevent: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"stream ok\"}}\n\nevent: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\nevent: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\",\"stop_sequence\":null},\"usage\":{\"output_tokens\":2}}\n\nevent: message_stop\ndata: {\"type\":\"message_stop\"}\n\n".to_vec(),
                )
            }
        },
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
            if matches!(
                route,
                FixtureRoute::QwenChat
                    | FixtureRoute::OpenaiCustomChat
                    | FixtureRoute::GeminiChat
                    | FixtureRoute::GrokChat
                    | FixtureRoute::OpenCodeGoChat
            ) {
                assert_eq!(body["model"], "claude-sonnet-4-20250514");
                assert_eq!(body["content"][0]["type"], "text");
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
fn api_contract_real_transport_connect_and_timeout_exhaustion_is_closed() {
    for route in [
        FixtureRoute::RelayAnthropic,
        FixtureRoute::OpenaiCustomChat,
        FixtureRoute::OpenaiResponses,
    ] {
        assert_real_network_exhaustion(route, NetworkFixture::ConnectRefused, 502);
        assert_real_network_exhaustion(route, NetworkFixture::Timeout, 504);
    }
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
fn api_contract_openai_chat_conflict_rate_and_5xx_retry_with_identical_body() {
    assert_retry_then_success(
        FixtureRoute::OpenaiCustomChat,
        AttemptMode::Nonstream,
        409,
        None,
        500,
    );
    assert_retry_then_success(
        FixtureRoute::QwenChat,
        AttemptMode::Nonstream,
        429,
        None,
        500,
    );
    assert_exhausted(
        FixtureRoute::GeminiChat,
        AttemptMode::Nonstream,
        500,
        &[500, 1_000],
    );
}

#[test]
fn api_contract_openai_chat_permanent_auth_quota_and_redirect_are_terminal() {
    for status in [400, 401, 403, 422, 307] {
        assert_terminal_http(
            FixtureRoute::OpenaiCustomChat,
            AttemptMode::Nonstream,
            status,
            ErrorCode::Absent,
            1,
        );
    }
    assert_terminal_http(
        FixtureRoute::QwenChat,
        AttemptMode::Nonstream,
        429,
        ErrorCode::InsufficientQuota,
        1,
    );
}

#[test]
fn api_contract_openai_chat_malformed_success_never_replays() {
    assert_opened_failure(
        FixtureRoute::OpenaiCustomChat,
        AttemptMode::Nonstream,
        OpenedFixture::MalformedJson,
    );
}

#[test]
fn api_contract_openai_chat_qwen_custom_gemini_grok_success_is_compatible() {
    for (route, fixture) in [
        (FixtureRoute::QwenChat, "qwen-chat"),
        (FixtureRoute::OpenaiCustomChat, "openai-custom-chat"),
        (FixtureRoute::GeminiChat, "gemini-chat"),
        (FixtureRoute::GrokChat, "grok-chat"),
        (FixtureRoute::OpenCodeGoChat, "opencode-go-chat"),
    ] {
        assert_success_fixture(route, AttemptMode::Nonstream, fixture);
    }
}

#[test]
fn api_contract_openai_chat_stream_replay_delivery_controls_final_outcome() {
    assert_success_fixture(
        FixtureRoute::OpenaiCustomChat,
        AttemptMode::Stream,
        "openai-chat-local-sse-replay",
    );
    assert_delivery_failure(
        FixtureRoute::OpenaiCustomChat,
        AttemptMode::Stream,
        DeliveryFailure::ChunkWrite,
    );
    assert_delivery_failure(
        FixtureRoute::OpenaiCustomChat,
        AttemptMode::Stream,
        DeliveryFailure::TerminalChunkWrite,
    );
    assert_delivery_failure(
        FixtureRoute::OpenaiCustomChat,
        AttemptMode::Stream,
        DeliveryFailure::FinalFlush,
    );
}

#[test]
fn api_contract_cross_route_downstream_failure_finalizes_only_cancelled() {
    for route in [
        FixtureRoute::RelayAnthropic,
        FixtureRoute::QwenChat,
        FixtureRoute::OpenaiResponses,
    ] {
        assert_delivery_failure(
            route,
            AttemptMode::Nonstream,
            DeliveryFailure::FinalBodyWrite,
        );
        assert_delivery_failure(route, AttemptMode::Stream, DeliveryFailure::FinalFlush);
    }
}

#[test]
fn api_contract_cross_route_diagnostic_schema_has_only_closed_fields() {
    for route in [
        FixtureRoute::RelayAnthropic,
        FixtureRoute::QwenChat,
        FixtureRoute::OpenaiResponses,
    ] {
        assert_diagnostic_keys(
            route,
            &[
                "schema_version",
                "provider",
                "route",
                "correlation_id",
                "outcome",
                "posts",
                "repairs",
                "reason",
            ],
        );
        assert_diagnostic_reason(route);
    }
}

#[test]
fn api_contract_cross_route_raw_body_key_header_url_and_request_id_never_reach_diagnostic() {
    for route in [
        FixtureRoute::RelayAnthropic,
        FixtureRoute::QwenChat,
        FixtureRoute::OpenaiResponses,
    ] {
        assert_redacted(
            route,
            &[
                "fixture-api-key",
                "secret upstream text",
                "private.example",
                "Req-secret-cross-route",
                request_id_sentinel(route),
            ],
        );
    }
}

#[test]
fn api_contract_cross_route_retry_wait_cancellation_stops_before_next_post() {
    for route in [
        FixtureRoute::RelayAnthropic,
        FixtureRoute::QwenChat,
        FixtureRoute::OpenaiResponses,
    ] {
        assert_wait_cancellation(route, 503, 1, &[500]);
    }
}

#[test]
fn api_harness_avoids_every_reserved_runtime_port() {
    for _ in 0..64 {
        let fixture = AcceptanceFixture::new(FixtureRoute::RelayAnthropic);
        assert!(!RESERVED_PORTS.contains(&fixture.upstream_port()));
        assert!(!RESERVED_PORTS.contains(&fixture.gateway_port()));
    }
}

#[test]
fn api_contract_openai_responses_retries_conflict_rate_and_5xx_only() {
    assert_retry_then_success(
        FixtureRoute::OpenaiResponses,
        AttemptMode::Nonstream,
        409,
        None,
        500,
    );
    assert_retry_then_success(
        FixtureRoute::OpenaiResponses,
        AttemptMode::Nonstream,
        429,
        Some(90),
        60_000,
    );
    assert_exhausted(
        FixtureRoute::OpenaiResponses,
        AttemptMode::Nonstream,
        502,
        &[500, 1_000],
    );
}

#[test]
fn api_contract_openai_responses_quota_and_permanent_errors_are_terminal() {
    for status in [400, 401, 403, 422, 307] {
        assert_terminal_http(
            FixtureRoute::OpenaiResponses,
            AttemptMode::Nonstream,
            status,
            ErrorCode::Absent,
            1,
        );
    }
    assert_terminal_http(
        FixtureRoute::OpenaiResponses,
        AttemptMode::Nonstream,
        429,
        ErrorCode::InsufficientQuota,
        1,
    );
}

#[test]
fn api_contract_openai_responses_malformed_or_incomplete_success_never_replays() {
    assert_opened_failure(
        FixtureRoute::OpenaiResponses,
        AttemptMode::Nonstream,
        OpenedFixture::MalformedJson,
    );
    assert_opened_failure(
        FixtureRoute::OpenaiResponses,
        AttemptMode::Nonstream,
        OpenedFixture::IncompleteBody,
    );
}

#[test]
fn api_contract_openai_responses_success_metadata_and_mapping_are_unchanged() {
    assert_success_fixture(
        FixtureRoute::OpenaiResponses,
        AttemptMode::Nonstream,
        "openai-responses-metadata-map",
    );
    assert_success_fixture(
        FixtureRoute::OpenaiResponses,
        AttemptMode::Stream,
        "openai-responses-local-sse-replay",
    );
}

#[test]
fn api_contract_openai_responses_never_authorizes_safe_repair() {
    assert_unsupported_tool_choice_is_terminal(FixtureRoute::OpenaiResponses, 400, 1, 0);
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
