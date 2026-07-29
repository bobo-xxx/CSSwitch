use std::borrow::Cow;
use std::fmt;
use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use reqwest::header::{HeaderValue, ACCEPT, AUTHORIZATION, CONTENT_TYPE, USER_AGENT};
use reqwest::{Client, Response};
use serde::Deserialize;
use zeroize::Zeroizing;

use crate::codex_auth::InferenceSecrets;
use crate::codex_network::CodexHttpClientFactory;
use crate::config::DEFAULT_CODEX_UPSTREAM_URL;
use crate::provider_contracts::CodexRuntimeContract;
use crate::provider_failure::{
    ErrorCode, ErrorParam, FailureObservation, NetworkKind, ProtocolKind, RateKind, RequestId,
};

const CODEX_ORIGINATOR: &str = "codex_cli_rs";
// ChatGPT's Codex edge rejects some product/custom User-Agent values as
// automated traffic. Keep inference aligned with the first-party Codex
// originator; OAuth and the other provider transports retain CSSwitch's UA.
const CODEX_INFERENCE_UA: &str = "codex_cli_rs";
const RESPONSES_LITE_HEADER: &str = "x-openai-internal-codex-responses-lite";
const FAILURE_BODY_LIMIT: usize = 16 * 1024;
#[cfg(test)]
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
#[cfg(test)]
const READ_IDLE_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Clone)]
pub(crate) struct CodexTransport {
    client: Client,
    endpoint: String,
    request_timeout: Duration,
    read_idle_timeout: Duration,
}

#[derive(Clone, Default)]
pub(crate) struct CodexCancellation {
    cancelled: Arc<AtomicBool>,
}

impl CodexCancellation {
    pub(crate) fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
}

impl fmt::Debug for CodexTransport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CodexTransport")
            .field("endpoint", &self.endpoint)
            .finish_non_exhaustive()
    }
}

pub(crate) struct CodexUpstream {
    response: Response,
    runtime: tokio::runtime::Runtime,
    cancellation: CodexCancellation,
    pending: Vec<u8>,
    pending_offset: usize,
    read_idle_timeout: Duration,
}

impl Read for CodexUpstream {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        if self.pending_offset < self.pending.len() {
            let available = &self.pending[self.pending_offset..];
            let length = available.len().min(buffer.len());
            buffer[..length].copy_from_slice(&available[..length]);
            self.pending_offset += length;
            if self.pending_offset == self.pending.len() {
                self.pending.clear();
                self.pending_offset = 0;
            }
            return Ok(length);
        }
        let cancellation = self.cancellation.clone();
        let read_idle_timeout = self.read_idle_timeout;
        let response = &mut self.response;
        let next = self.runtime.block_on(async move {
            tokio::select! {
                _ = wait_for_cancel(cancellation) => Err(std::io::Error::new(
                    std::io::ErrorKind::ConnectionAborted,
                    "Codex request cancelled",
                )),
                result = tokio::time::timeout(read_idle_timeout, response.chunk()) => match result {
                    Ok(result) => result
                        .map_err(|_| std::io::Error::other("Codex upstream read failed")),
                    Err(_) => Err(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        "Codex upstream read timed out",
                    )),
                },
            }
        })?;
        let Some(chunk) = next else {
            return Ok(0);
        };
        let length = chunk.len().min(buffer.len());
        buffer[..length].copy_from_slice(&chunk[..length]);
        if length < chunk.len() {
            self.pending = chunk.to_vec();
            self.pending_offset = length;
        }
        Ok(length)
    }
}

#[derive(Clone)]
pub(crate) struct CodexTransportError {
    pub status: u16,
    pub upstream_status: Option<u16>,
    pub detail: &'static str,
    pub cancelled: bool,
    observation: FailureObservation,
}

impl CodexTransportError {
    pub(crate) fn observation(&self) -> FailureObservation {
        self.observation.clone()
    }
}

impl fmt::Debug for CodexTransportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CodexTransportError")
            .field("status", &self.status)
            .field("upstream_status", &self.upstream_status)
            .field("detail", &self.detail)
            .field("cancelled", &self.cancelled)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FailureBodyFacts {
    rate_kind: Option<RateKind>,
    error_code: ErrorCode,
    error_param: ErrorParam,
}

#[derive(Deserialize)]
struct FailureBodyDocument<'a> {
    #[serde(borrow, default)]
    error: Option<FailureBodyError<'a>>,
}

#[derive(Deserialize)]
struct FailureBodyError<'a> {
    #[serde(rename = "type", borrow, default)]
    kind: Option<Cow<'a, str>>,
    #[serde(borrow, default)]
    code: Option<Cow<'a, str>>,
    #[serde(borrow, default)]
    param: Option<Cow<'a, str>>,
}

const FAILURE_PARSED_STORAGE: usize =
    std::mem::size_of::<FailureBodyDocument<'static>>() + std::mem::size_of::<FailureBodyFacts>();
// In the worst case all three allowlisted strings contain JSON escapes and
// serde_json must own their decoded bytes. Their combined size cannot exceed
// the input size, so reserving a second input-sized budget keeps caller-owned
// input plus parsed storage within FAILURE_BODY_LIMIT.
const FAILURE_BODY_INPUT_LIMIT: usize = (FAILURE_BODY_LIMIT - FAILURE_PARSED_STORAGE) / 2;

struct FailureBodyCollector {
    body: Vec<u8>,
    declared_length: usize,
}

impl FailureBodyCollector {
    fn new(declared_length: Option<u64>) -> Option<Self> {
        let declared_length = usize::try_from(declared_length?).ok()?;
        (declared_length <= FAILURE_BODY_INPUT_LIMIT).then(|| Self {
            body: Vec::with_capacity(declared_length),
            declared_length,
        })
    }

    fn remaining(&self) -> usize {
        self.declared_length.saturating_sub(self.body.len())
    }

    fn retain(&mut self, chunk: &[u8]) -> bool {
        if chunk.len() > self.remaining() {
            return false;
        }
        self.body.extend_from_slice(chunk);
        true
    }

    fn facts(self) -> FailureBodyFacts {
        reduce_failure_body(&self.body)
    }
}

fn reduce_failure_body(body: &[u8]) -> FailureBodyFacts {
    let bounded = &body[..body.len().min(FAILURE_BODY_INPUT_LIMIT)];
    let parsed: FailureBodyDocument<'_> = match serde_json::from_slice(bounded) {
        Ok(value) => value,
        Err(_) => {
            return FailureBodyFacts {
                rate_kind: None,
                error_code: ErrorCode::Absent,
                error_param: ErrorParam::Absent,
            };
        }
    };
    let error = parsed.error;
    let error_type = error.as_ref().and_then(|value| value.kind.as_deref());
    let code = error.as_ref().and_then(|value| value.code.as_deref());
    let param = error.as_ref().and_then(|value| value.param.as_deref());
    let quota = error_type == Some("insufficient_quota") || code == Some("insufficient_quota");
    let rate = error_type == Some("rate_limit_error") || code == Some("rate_limit_exceeded");
    FailureBodyFacts {
        rate_kind: if quota {
            Some(RateKind::Quota)
        } else if rate {
            Some(RateKind::RateLimit)
        } else {
            None
        },
        error_code: match code {
            Some("unsupported_value") => ErrorCode::UnsupportedValue,
            Some("insufficient_quota") => ErrorCode::InsufficientQuota,
            Some("rate_limit_exceeded") => ErrorCode::RateLimitExceeded,
            _ if error_type == Some("insufficient_quota") => ErrorCode::InsufficientQuota,
            Some(_) => ErrorCode::Other,
            None => ErrorCode::Absent,
        },
        error_param: match param {
            Some("tool_choice") => ErrorParam::ToolChoice,
            Some(_) => ErrorParam::Other,
            None => ErrorParam::Absent,
        },
    }
}

fn parse_retry_after(value: Option<&str>) -> Option<u64> {
    let value = value?;
    (!value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()))
        .then(|| value.parse().ok())
        .flatten()
}

fn parse_request_id(value: Option<&str>) -> Option<RequestId> {
    RequestId::new(value?)
}

fn challenge_detected(response: &Response) -> bool {
    response
        .headers()
        .get("cf-mitigated")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.eq_ignore_ascii_case("challenge"))
}

fn response_media_type(response: &Response) -> Option<&str> {
    response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn unexpected_response_detail(response: &Response) -> &'static str {
    if challenge_detected(response) {
        return "Codex upstream returned a Cloudflare challenge response";
    }

    match response_media_type(response) {
        Some(value) if value.eq_ignore_ascii_case("application/json") => {
            "Codex upstream returned JSON instead of an event stream"
        }
        Some(value) if value.eq_ignore_ascii_case("text/html") => {
            "Codex upstream returned HTML instead of an event stream"
        }
        None => "Codex upstream returned no response content type",
        Some(_) => "Codex upstream returned an unsupported response content type",
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ResponsePrefixKind {
    Sse,
    Html,
    Json,
    NeedMore,
    Other,
}

fn classify_response_prefix(prefix: &[u8]) -> ResponsePrefixKind {
    let prefix = prefix.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(prefix);
    let prefix = prefix
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
        .map(|start| &prefix[start..])
        .unwrap_or_default();
    if prefix.is_empty() {
        return ResponsePrefixKind::NeedMore;
    }
    const SSE_PREFIXES: [&[u8]; 5] = [b"data:", b"event:", b"id:", b"retry:", b":"];
    if SSE_PREFIXES
        .iter()
        .any(|candidate| prefix.starts_with(candidate))
    {
        return ResponsePrefixKind::Sse;
    }
    if SSE_PREFIXES
        .iter()
        .any(|candidate| candidate.starts_with(prefix))
    {
        return ResponsePrefixKind::NeedMore;
    }
    match prefix[0] {
        b'<' => ResponsePrefixKind::Html,
        b'{' | b'[' => ResponsePrefixKind::Json,
        _ => ResponsePrefixKind::Other,
    }
}

impl fmt::Display for CodexTransportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.detail)
    }
}

impl std::error::Error for CodexTransportError {}

impl CodexTransport {
    pub(crate) fn production(contract: &CodexRuntimeContract) -> Result<Self, CodexTransportError> {
        let factory =
            CodexHttpClientFactory::from_environment().map_err(|_| CodexTransportError {
                status: 502,
                upstream_status: None,
                detail: "Codex network route initialization failed",
                cancelled: false,
                observation: FailureObservation::Network(NetworkKind::Connect),
            })?;
        Self::new_with_factory(
            DEFAULT_CODEX_UPSTREAM_URL.to_string(),
            contract.connect_timeout,
            contract.request_timeout,
            contract.read_idle_timeout,
            &factory,
        )
    }

    #[cfg(test)]
    pub(crate) fn for_test(endpoint: String) -> Result<Self, CodexTransportError> {
        Self::new_with_factory(
            endpoint,
            CONNECT_TIMEOUT,
            READ_IDLE_TIMEOUT,
            READ_IDLE_TIMEOUT,
            &CodexHttpClientFactory::direct_for_test(),
        )
    }

    fn new_with_factory(
        endpoint: String,
        connect_timeout: Duration,
        request_timeout: Duration,
        read_idle_timeout: Duration,
        factory: &CodexHttpClientFactory,
    ) -> Result<Self, CodexTransportError> {
        let client = factory
            .async_builder()
            .map_err(|_| CodexTransportError {
                status: 502,
                upstream_status: None,
                detail: "Codex network route initialization failed",
                cancelled: false,
                observation: FailureObservation::Network(NetworkKind::Connect),
            })?
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .pool_max_idle_per_host(0)
            .connect_timeout(connect_timeout)
            .build()
            .map_err(|_| CodexTransportError {
                status: 502,
                upstream_status: None,
                detail: "Codex transport initialization failed",
                cancelled: false,
                observation: FailureObservation::Network(NetworkKind::Connect),
            })?;
        Ok(Self {
            client,
            endpoint,
            request_timeout,
            read_idle_timeout,
        })
    }

    /// Sends exactly one inference POST. Retry and repair policy belongs to the
    /// handler-owned Attempt Controller; this method never replays internally.
    pub(crate) fn open_responses(
        &self,
        secrets: &InferenceSecrets,
        body: Vec<u8>,
        use_responses_lite: bool,
        cancellation: CodexCancellation,
    ) -> Result<CodexUpstream, CodexTransportError> {
        let authorization = Zeroizing::new(format!("Bearer {}", secrets.access_token()));
        let mut authorization_header =
            HeaderValue::from_str(&authorization).map_err(|_| CodexTransportError {
                status: 401,
                upstream_status: None,
                detail: "Codex authorization is invalid",
                cancelled: false,
                observation: FailureObservation::Http {
                    status: 401,
                    rate_kind: None,
                    retry_after_seconds: None,
                    request_id: None,
                    error_code: ErrorCode::Absent,
                    error_param: ErrorParam::Absent,
                },
            })?;
        authorization_header.set_sensitive(true);
        let mut account_header =
            HeaderValue::from_str(secrets.account_id()).map_err(|_| CodexTransportError {
                status: 401,
                upstream_status: None,
                detail: "Codex account authorization is invalid",
                cancelled: false,
                observation: FailureObservation::Http {
                    status: 401,
                    rate_kind: None,
                    retry_after_seconds: None,
                    request_id: None,
                    error_code: ErrorCode::Absent,
                    error_param: ErrorParam::Absent,
                },
            })?;
        account_header.set_sensitive(true);
        let mut request = self
            .client
            .post(&self.endpoint)
            .header(CONTENT_TYPE, "application/json")
            .header(ACCEPT, "text/event-stream")
            .header(USER_AGENT, CODEX_INFERENCE_UA)
            .header("originator", CODEX_ORIGINATOR)
            .header("ChatGPT-Account-ID", account_header)
            .header(AUTHORIZATION, authorization_header)
            .body(body);
        if use_responses_lite {
            request = request.header(RESPONSES_LITE_HEADER, "true");
        }
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_io()
            .enable_time()
            .build()
            .map_err(|_| CodexTransportError {
                status: 502,
                upstream_status: None,
                detail: "Codex transport runtime failed",
                cancelled: false,
                observation: FailureObservation::Network(NetworkKind::Connect),
            })?;
        let mut response = runtime.block_on(async {
            tokio::select! {
                _ = wait_for_cancel(cancellation.clone()) => Err(CodexTransportError {
                    status: 499,
                    upstream_status: None,
                    detail: "Codex request was cancelled",
                    cancelled: true,
                    observation: FailureObservation::Cancelled,
                }),
                result = tokio::time::timeout(self.request_timeout, request.send()) => match result {
                    Ok(result) => result.map_err(|_| CodexTransportError {
                        status: 502,
                        upstream_status: None,
                        detail: "Codex upstream request failed",
                        cancelled: false,
                        observation: FailureObservation::Network(NetworkKind::Connect),
                    }),
                    Err(_) => Err(CodexTransportError {
                        status: 504,
                        upstream_status: None,
                        detail: "Codex upstream response timed out",
                        cancelled: false,
                        observation: FailureObservation::Network(NetworkKind::Timeout),
                    }),
                },
            }
        })?;
        let status = response.status().as_u16();
        let retry_after = response
            .headers()
            .get("retry-after")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| parse_retry_after(Some(value)));
        let request_id = response
            .headers()
            .get("x-request-id")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| parse_request_id(Some(value)));
        if !response.status().is_success() {
            enum BodyRead {
                Facts(FailureBodyFacts),
                Cancelled,
                Unavailable,
            }
            // Reqwest exposes already-framed `Bytes`; it cannot accept a caller-sized
            // read buffer. Therefore an unknown or oversized failure body is never
            // polled. For an admitted response, hyper's validated Content-Length
            // bounds every materialized frame plus previously retained input.
            let body_read = match FailureBodyCollector::new(response.content_length()) {
                Some(mut collector) => {
                    let cancellation_for_body = cancellation.clone();
                    runtime.block_on(async {
                        match tokio::time::timeout(self.request_timeout, async {
                            while collector.remaining() > 0 {
                                let next = tokio::select! {
                                    _ = wait_for_cancel(cancellation_for_body.clone()) => {
                                        return BodyRead::Cancelled;
                                    }
                                    next = response.chunk() => match next {
                                        Ok(next) => next,
                                        Err(_) => return BodyRead::Unavailable,
                                    },
                                };
                                let Some(chunk) = next else {
                                    break;
                                };
                                if !collector.retain(&chunk) {
                                    return BodyRead::Unavailable;
                                }
                            }
                            BodyRead::Facts(collector.facts())
                        })
                        .await
                        {
                            Ok(result) => result,
                            Err(_) => BodyRead::Unavailable,
                        }
                    })
                }
                None if cancellation.is_cancelled() => BodyRead::Cancelled,
                None => BodyRead::Facts(FailureBodyFacts {
                    rate_kind: None,
                    error_code: ErrorCode::Absent,
                    error_param: ErrorParam::Absent,
                }),
            };
            let facts = match body_read {
                BodyRead::Facts(facts) => facts,
                BodyRead::Unavailable => FailureBodyFacts {
                    rate_kind: None,
                    error_code: ErrorCode::Absent,
                    error_param: ErrorParam::Absent,
                },
                BodyRead::Cancelled => {
                    return Err(CodexTransportError {
                        status: 499,
                        upstream_status: Some(status),
                        detail: "Codex request was cancelled",
                        cancelled: true,
                        observation: FailureObservation::Cancelled,
                    });
                }
            };
            let observation = FailureObservation::Http {
                status,
                rate_kind: facts.rate_kind,
                retry_after_seconds: retry_after,
                request_id,
                error_code: facts.error_code,
                error_param: facts.error_param,
            };
            return Err(CodexTransportError {
                status: match status {
                    401 | 403 | 429 => status,
                    408 => 504,
                    _ => 502,
                },
                upstream_status: Some(status),
                detail: "Codex upstream rejected the request",
                cancelled: false,
                observation,
            });
        }
        let mut pending = Vec::new();
        let is_event_stream = response_media_type(&response)
            .is_some_and(|value| value.eq_ignore_ascii_case("text/event-stream"));
        if !is_event_stream {
            if challenge_detected(&response) || response_media_type(&response).is_some() {
                return Err(CodexTransportError {
                    status: 502,
                    upstream_status: Some(status),
                    detail: unexpected_response_detail(&response),
                    cancelled: false,
                    observation: FailureObservation::Protocol(ProtocolKind::InvalidResponse),
                });
            }

            // Some Codex edges omit Content-Type on an otherwise valid SSE
            // response. Sniff only a bounded prefix and accept it solely when
            // it has a valid SSE field prefix. This consumes no second POST and
            // never exposes the upstream body in diagnostics.
            let cancellation_for_sniff = cancellation.clone();
            let sniffed = runtime.block_on(async {
                tokio::time::timeout(self.request_timeout, async {
                    let mut prefix = Vec::with_capacity(512);
                    let mut consumed = Vec::new();
                    loop {
                        let next = tokio::select! {
                            _ = wait_for_cancel(cancellation_for_sniff.clone()) => {
                                return Err(CodexTransportError {
                                    status: 499,
                                    upstream_status: Some(status),
                                    detail: "Codex request was cancelled",
                                    cancelled: true,
                                    observation: FailureObservation::Cancelled,
                                });
                            }
                            result = response.chunk() => result.map_err(|_| CodexTransportError {
                                status: 502,
                                upstream_status: Some(status),
                                detail: "Codex upstream read failed",
                                cancelled: false,
                                observation: FailureObservation::Network(NetworkKind::Read),
                            })?,
                        };
                        let Some(chunk) = next else {
                            return Err(CodexTransportError {
                                status: 502,
                                upstream_status: Some(status),
                                detail: "Codex upstream returned an empty response",
                                cancelled: false,
                                observation: FailureObservation::Protocol(
                                    ProtocolKind::InvalidResponse,
                                ),
                            });
                        };
                        consumed.extend_from_slice(&chunk);
                        let remaining = 512_usize.saturating_sub(prefix.len());
                        prefix.extend_from_slice(&chunk[..chunk.len().min(remaining)]);
                        match classify_response_prefix(&prefix) {
                            ResponsePrefixKind::Sse => return Ok(consumed),
                            ResponsePrefixKind::Html => {
                                return Err(CodexTransportError {
                                    status: 502,
                                    upstream_status: Some(status),
                                    detail:
                                        "Codex upstream returned HTML instead of an event stream",
                                    cancelled: false,
                                    observation: FailureObservation::Protocol(
                                        ProtocolKind::InvalidResponse,
                                    ),
                                });
                            }
                            ResponsePrefixKind::Json => {
                                return Err(CodexTransportError {
                                    status: 502,
                                    upstream_status: Some(status),
                                    detail:
                                        "Codex upstream returned JSON instead of an event stream",
                                    cancelled: false,
                                    observation: FailureObservation::Protocol(
                                        ProtocolKind::InvalidResponse,
                                    ),
                                });
                            }
                            ResponsePrefixKind::Other => {
                                return Err(CodexTransportError {
                                    status: 502,
                                    upstream_status: Some(status),
                                    detail: "Codex upstream returned an unsupported response body",
                                    cancelled: false,
                                    observation: FailureObservation::Protocol(
                                        ProtocolKind::InvalidResponse,
                                    ),
                                });
                            }
                            ResponsePrefixKind::NeedMore if prefix.len() < 512 => {}
                            ResponsePrefixKind::NeedMore => {
                                return Err(CodexTransportError {
                                    status: 502,
                                    upstream_status: Some(status),
                                    detail: "Codex upstream returned an unsupported response body",
                                    cancelled: false,
                                    observation: FailureObservation::Protocol(
                                        ProtocolKind::InvalidResponse,
                                    ),
                                });
                            }
                        }
                    }
                })
                .await
                .map_err(|_| CodexTransportError {
                    status: 504,
                    upstream_status: Some(status),
                    detail: "Codex upstream response body timed out",
                    cancelled: false,
                    observation: FailureObservation::Network(NetworkKind::Timeout),
                })?
            })?;
            pending = sniffed;
        }
        Ok(CodexUpstream {
            response,
            runtime,
            cancellation,
            pending,
            pending_offset: 0,
            read_idle_timeout: self.read_idle_timeout,
        })
    }
}

async fn wait_for_cancel(cancellation: CodexCancellation) {
    while !cancellation.is_cancelled() {
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{mpsc, Arc, Mutex};
    use std::thread;
    use std::time::{Duration, Instant};

    use super::{
        parse_request_id, parse_retry_after, reduce_failure_body, CodexCancellation,
        CodexTransport, FailureBodyCollector, FAILURE_BODY_INPUT_LIMIT, FAILURE_BODY_LIMIT,
        FAILURE_PARSED_STORAGE,
    };
    use crate::codex_auth::InferenceSecrets;
    use crate::provider_failure::{ErrorCode, ErrorParam, FailureObservation, RateKind};

    fn bind_loopback() -> TcpListener {
        loop {
            let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
            if listener.local_addr().unwrap().port() != 8765 {
                return listener;
            }
        }
    }

    fn read_request(stream: &mut TcpStream) -> Vec<u8> {
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut request = Vec::new();
        let mut expected = None;
        let mut buffer = [0_u8; 1024];
        loop {
            let read = stream.read(&mut buffer).unwrap();
            assert!(read > 0);
            request.extend_from_slice(&buffer[..read]);
            if expected.is_none() {
                if let Some(end) = request.windows(4).position(|part| part == b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&request[..end]);
                    let length = head
                        .lines()
                        .find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            name.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse::<usize>().unwrap())
                        })
                        .unwrap_or(0);
                    expected = Some(end + 4 + length);
                }
            }
            if expected.is_some_and(|length| request.len() >= length) {
                return request;
            }
        }
    }

    fn mock_server(response: Vec<u8>) -> (String, mpsc::Receiver<Vec<u8>>, thread::JoinHandle<()>) {
        let listener = bind_loopback();
        let address = listener.local_addr().unwrap();
        let (tx, rx) = mpsc::channel();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let request = read_request(&mut stream);
            tx.send(request).unwrap();
            stream.write_all(&response).unwrap();
            stream.flush().unwrap();
        });
        (format!("http://{address}/responses"), rx, handle)
    }

    fn secrets() -> InferenceSecrets {
        InferenceSecrets::for_test("access-secret", "account-secret")
    }

    #[test]
    fn single_post_uses_codex_headers_and_preserves_body() {
        let body = b"{\"stream\":true}".to_vec();
        let event = b"data: {\"type\":\"response.created\"}\n\n";
        let response = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream; charset=utf-8\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
            event.len()
        )
        .into_bytes();
        let mut response = response;
        response.extend_from_slice(event);
        let (endpoint, request_rx, handle) = mock_server(response);
        let transport = CodexTransport::for_test(endpoint).unwrap();
        let mut upstream = transport
            .open_responses(
                &secrets(),
                body.clone(),
                false,
                CodexCancellation::default(),
            )
            .unwrap();
        let mut received_body = Vec::new();
        upstream.read_to_end(&mut received_body).unwrap();
        assert_eq!(received_body, event);

        let request = request_rx.recv().unwrap();
        let head_end = request
            .windows(4)
            .position(|part| part == b"\r\n\r\n")
            .unwrap();
        let head = String::from_utf8_lossy(&request[..head_end]).to_ascii_lowercase();
        assert!(head.starts_with("post /responses http/1.1"));
        assert!(head.contains("authorization: bearer access-secret"));
        assert!(head.contains("chatgpt-account-id: account-secret"));
        assert!(head.contains("originator: codex_cli_rs"));
        assert!(head.contains("user-agent: codex_cli_rs"));
        assert!(head.contains("accept: text/event-stream"));
        assert_eq!(&request[head_end + 4..], body);
        handle.join().unwrap();
    }

    #[test]
    fn responses_lite_request_sets_lite_header() {
        let event = b"data: {\"type\":\"response.created\"}\n\n";
        let mut response = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
            event.len()
        )
        .into_bytes();
        response.extend_from_slice(event);
        let (endpoint, request_rx, handle) = mock_server(response);
        let transport = CodexTransport::for_test(endpoint).unwrap();
        let mut upstream = transport
            .open_responses(
                &secrets(),
                b"{}".to_vec(),
                true,
                CodexCancellation::default(),
            )
            .unwrap();
        let mut received = Vec::new();
        upstream.read_to_end(&mut received).unwrap();
        assert_eq!(received, event);
        let request = String::from_utf8_lossy(&request_rx.recv().unwrap()).to_ascii_lowercase();
        assert!(request.contains("x-openai-internal-codex-responses-lite: true"));
        handle.join().unwrap();
    }

    #[test]
    fn upstream_errors_and_content_type_fail_without_a_second_post() {
        for response in [
            b"HTTP/1.1 401 Unauthorized\r\ncontent-length: 0\r\nconnection: close\r\n\r\n".to_vec(),
            b"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 2\r\nconnection: close\r\n\r\n{}".to_vec(),
        ] {
            let (endpoint, request_rx, handle) = mock_server(response);
            let transport = CodexTransport::for_test(endpoint).unwrap();
            let error = transport
                .open_responses(
                    &secrets(),
                    b"{}".to_vec(),
                    false,
                    CodexCancellation::default(),
                )
                .err()
                .unwrap();
            let request = request_rx.recv().unwrap();
            assert!(request.starts_with(b"POST "));
            if error.upstream_status == Some(401) {
                assert_eq!(error.status, 401);
            } else {
                assert_eq!(error.status, 502);
            }
            handle.join().unwrap();
        }
    }

    #[test]
    fn unexpected_content_type_is_classified_without_exposing_the_body() {
        for (headers, body, expected) in [
            (
                "content-type: text/html\r\n",
                "private-html-body",
                "Codex upstream returned HTML instead of an event stream",
            ),
            (
                "content-type: application/json\r\n",
                "private-json-body",
                "Codex upstream returned JSON instead of an event stream",
            ),
            (
                "content-type: text/html\r\ncf-mitigated: challenge\r\n",
                "private-challenge-body",
                "Codex upstream returned a Cloudflare challenge response",
            ),
            (
                "",
                "private-missing-content-type-body",
                "Codex upstream returned an unsupported response body",
            ),
        ] {
            let response = format!(
                "HTTP/1.1 200 OK\r\n{headers}content-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            )
            .into_bytes();
            let (endpoint, request_rx, handle) = mock_server(response);
            let transport = CodexTransport::for_test(endpoint).unwrap();
            let error = transport
                .open_responses(
                    &secrets(),
                    b"{}".to_vec(),
                    false,
                    CodexCancellation::default(),
                )
                .err()
                .unwrap();
            assert_eq!(error.detail, expected);
            assert!(!error.detail.contains(body));
            let _ = request_rx.recv().unwrap();
            handle.join().unwrap();
        }
    }

    #[test]
    fn missing_content_type_accepts_a_bounded_sse_prefix_without_reposting() {
        let event = b"data: {\"type\":\"response.created\"}\n\n";
        let mut response = format!(
            "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
            event.len()
        )
        .into_bytes();
        response.extend_from_slice(event);
        let (endpoint, request_rx, handle) = mock_server(response);
        let transport = CodexTransport::for_test(endpoint).unwrap();
        let mut upstream = transport
            .open_responses(
                &secrets(),
                b"{}".to_vec(),
                false,
                CodexCancellation::default(),
            )
            .unwrap();
        let mut received = Vec::new();
        upstream.read_to_end(&mut received).unwrap();
        assert_eq!(received, event);
        let request = request_rx.recv().unwrap();
        assert!(request.starts_with(b"POST "));
        handle.join().unwrap();
    }

    #[test]
    fn cancellation_aborts_first_byte_wait_without_reposting() {
        let listener = bind_loopback();
        let address = listener.local_addr().unwrap();
        let requests = Arc::new(AtomicUsize::new(0));
        let captured = Arc::new(Mutex::new(Vec::new()));
        let requests_for_server = Arc::clone(&requests);
        let captured_for_server = Arc::clone(&captured);
        let (request_seen_tx, request_seen_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let request = read_request(&mut stream);
            requests_for_server.fetch_add(1, Ordering::SeqCst);
            *captured_for_server.lock().unwrap() = request;
            request_seen_tx.send(()).unwrap();
            let _ = release_rx.recv_timeout(Duration::from_secs(2));
        });
        let transport = CodexTransport::for_test(format!("http://{address}/responses")).unwrap();
        let cancellation = CodexCancellation::default();
        let cancellation_for_thread = cancellation.clone();
        let canceller = thread::spawn(move || {
            request_seen_rx
                .recv_timeout(Duration::from_secs(2))
                .unwrap();
            cancellation_for_thread.cancel();
        });

        let started = Instant::now();
        let error = transport
            .open_responses(&secrets(), b"{}".to_vec(), false, cancellation)
            .err()
            .unwrap();
        assert!(error.cancelled);
        assert!(started.elapsed() < Duration::from_secs(1));
        assert_eq!(requests.load(Ordering::SeqCst), 1);
        assert!(captured.lock().unwrap().starts_with(b"POST "));

        release_tx.send(()).unwrap();
        canceller.join().unwrap();
        server.join().unwrap();
    }

    #[test]
    fn failed_response_extracts_only_allowlisted_facts() {
        let body = r#"{"error":{"type":"rate_limit_error","code":"rate_limit_exceeded","param":"other","message":"BODY_SECRET"}}"#;
        let response = format!(
            "HTTP/1.1 429 Too Many Requests\r\nRetry-After: 90\r\nx-request-id: Req_01.a:b-c\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        )
        .into_bytes();
        let (endpoint, request_rx, handle) = mock_server(response);
        let transport = CodexTransport::for_test(endpoint).unwrap();
        let error = transport
            .open_responses(
                &secrets(),
                b"{}".to_vec(),
                false,
                CodexCancellation::default(),
            )
            .err()
            .expect("synthetic 429 must fail");
        assert_eq!(
            error.observation(),
            FailureObservation::Http {
                status: 429,
                rate_kind: Some(RateKind::RateLimit),
                retry_after_seconds: Some(90),
                request_id: crate::provider_failure::RequestId::new("Req_01.a:b-c"),
                error_code: ErrorCode::RateLimitExceeded,
                error_param: ErrorParam::Other,
            }
        );
        let diagnostic = format!("{error:?} {error}");
        assert!(!diagnostic.contains("BODY_SECRET"));
        assert!(!diagnostic.contains("Req_01.a:b-c"));
        assert!(request_rx.recv().unwrap().starts_with(b"POST "));
        handle.join().unwrap();
    }

    #[test]
    fn quota_wins_when_type_or_code_proves_insufficient_quota() {
        for body in [
            r#"{"error":{"type":"insufficient_quota","code":"other"}}"#,
            r#"{"error":{"type":"other","code":"insufficient_quota"}}"#,
        ] {
            let facts = reduce_failure_body(body.as_bytes());
            assert_eq!(facts.rate_kind, Some(RateKind::Quota));
            assert_eq!(facts.error_code, ErrorCode::InsufficientQuota);
        }
    }

    #[test]
    fn repair_signal_requires_exact_code_and_parameter() {
        let exact =
            reduce_failure_body(br#"{"error":{"code":"unsupported_value","param":"tool_choice"}}"#);
        assert_eq!(exact.error_code, ErrorCode::UnsupportedValue);
        assert_eq!(exact.error_param, ErrorParam::ToolChoice);
        let escaped = reduce_failure_body(
            br#"{"error":{"code":"unsupported_v\u0061lue","param":"tool_cho\u0069ce"}}"#,
        );
        assert_eq!(escaped.error_code, ErrorCode::UnsupportedValue);
        assert_eq!(escaped.error_param, ErrorParam::ToolChoice);

        for body in [
            br#"{"error":{"message":"unsupported_value tool_choice"}}"#.as_slice(),
            br#"{"error":{"code":"unsupported_value","param":"tools"}}"#.as_slice(),
            br#"{"error":{"code":"other","param":"tool_choice"}}"#.as_slice(),
        ] {
            let facts = reduce_failure_body(body);
            assert!(
                facts.error_code != ErrorCode::UnsupportedValue
                    || facts.error_param != ErrorParam::ToolChoice
            );
        }
    }

    #[test]
    fn retry_after_and_request_id_parsers_are_strict() {
        assert_eq!(parse_retry_after(Some("0")), Some(0));
        assert_eq!(parse_retry_after(Some("60")), Some(60));
        assert_eq!(parse_retry_after(Some(" 7 ")), None);
        assert_eq!(parse_retry_after(Some("1.5")), None);
        assert_eq!(
            parse_retry_after(Some("Wed, 29 Jul 2026 00:00:00 GMT")),
            None
        );
        assert_eq!(parse_retry_after(None), None);
        assert_eq!(
            parse_request_id(Some("req:01-A_b.c")).unwrap().as_str(),
            "req:01-A_b.c"
        );
        assert!(parse_request_id(Some("request id")).is_none());
        assert!(parse_request_id(Some(&"r".repeat(257))).is_none());
    }

    #[test]
    fn failure_body_reduction_never_reads_semantics_beyond_16_kib() {
        let mut body = vec![b' '; FAILURE_BODY_LIMIT];
        body.extend_from_slice(br#"{"error":{"code":"unsupported_value","param":"tool_choice"}}"#);
        let facts = reduce_failure_body(&body);
        assert_eq!(facts.error_code, ErrorCode::Absent);
        assert_eq!(facts.error_param, ErrorParam::Absent);
    }

    #[test]
    fn oversized_failure_frame_is_not_retained_or_parsed_by_the_caller() {
        let mut body = br#"{"error":{"code":"unsupported_value","param":"tool_choice"}}"#.to_vec();
        body.resize(FAILURE_BODY_LIMIT + 1, b' ');
        let mut collector = FailureBodyCollector::new(Some(FAILURE_BODY_INPUT_LIMIT as u64))
            .expect("the maximum bounded input must be admitted");
        assert!(!collector.retain(&body));
        assert_eq!(collector.body.len(), 0);
        assert!(collector.body.capacity() + FAILURE_PARSED_STORAGE <= FAILURE_BODY_LIMIT);
        assert!(FailureBodyCollector::new(Some(body.len() as u64)).is_none());

        let response = format!(
            "HTTP/1.1 400 Bad Request\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
            body.len()
        )
        .into_bytes()
        .into_iter()
        .chain(body)
        .collect();
        let (endpoint, request_rx, handle) = mock_server(response);
        let transport = CodexTransport::for_test(endpoint).unwrap();
        let error = transport
            .open_responses(
                &secrets(),
                b"{}".to_vec(),
                false,
                CodexCancellation::default(),
            )
            .err()
            .expect("synthetic oversized rejection must fail");

        assert_eq!(
            error.observation(),
            FailureObservation::Http {
                status: 400,
                rate_kind: None,
                retry_after_seconds: None,
                request_id: None,
                error_code: ErrorCode::Absent,
                error_param: ErrorParam::Absent,
            }
        );
        assert!(request_rx.recv().unwrap().starts_with(b"POST "));
        handle.join().unwrap();
    }
}
