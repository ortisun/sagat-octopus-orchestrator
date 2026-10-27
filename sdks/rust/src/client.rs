use super::{OperationResult, Run, SagatError, ScopeSegment, SignalResult, TokenSource};
use futures_util::StreamExt as _;
use reqwest::{
    Method,
    header::{AUTHORIZATION, HeaderValue},
};
use serde_json::{Value, json};
use std::time::Duration;
use time::{OffsetDateTime, UtcOffset, format_description::well_known::Rfc3339};
use tokio_util::sync::CancellationToken;
use url::Url;
use uuid::Uuid;

const MAX_RESPONSE: usize = 32 * 1024 * 1024;
#[derive(Clone, Debug)]
pub struct RetryPolicy {
    pub max_attempts: u32,
    pub initial_delay: Duration,
    pub max_delay: Duration,
}
impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 4,
            initial_delay: Duration::from_millis(200),
            max_delay: Duration::from_secs(5),
        }
    }
}
impl RetryPolicy {
    pub(crate) fn after(&self, attempt: u32, hint: Option<Duration>) -> Duration {
        let ceiling = self
            .initial_delay
            .saturating_mul(2_u32.saturating_pow(attempt.min(31)))
            .min(self.max_delay);
        let random = Uuid::new_v4();
        let bytes = random.as_bytes();
        let factor = 0.5 + f64::from(u16::from_be_bytes([bytes[0], bytes[1]])) / 131_072.0;
        ceiling
            .mul_f64(factor)
            .max(hint.unwrap_or_default().min(Duration::from_secs(60)))
    }
}
/// A relative deadline is frozen once, before the first request attempt.
#[derive(Clone, Debug)]
pub enum RunDeadline {
    At(OffsetDateTime),
    After(Duration),
}
#[derive(Clone, Debug, Default)]
pub struct StartOptions {
    pub call: CallOptions,
    pub version_id: Option<String>,
    pub correlation_id: Option<String>,
    pub deadline: Option<RunDeadline>,
}
impl From<CallOptions> for StartOptions {
    fn from(call: CallOptions) -> Self {
        Self {
            call,
            ..Self::default()
        }
    }
}
#[derive(Clone, Copy, Debug, serde::Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum StepOutcome {
    Succeeded,
    Failed,
}
#[derive(Clone, Debug, Default)]
pub struct OperationOptions {
    pub call: CallOptions,
    pub step_id: Option<String>,
    pub outcome: Option<StepOutcome>,
    pub error_code: Option<String>,
    pub instance_path: Option<Vec<ScopeSegment>>,
    pub expected_run_version: Option<u64>,
}
impl From<CallOptions> for OperationOptions {
    fn from(call: CallOptions) -> Self {
        Self {
            call,
            ..Self::default()
        }
    }
}

struct WireReply {
    value: Value,
    status: u16,
    trace_id: Option<String>,
}
impl WireReply {
    fn parse<T>(
        self,
        parser: impl FnOnce(Value) -> Result<T, SagatError>,
    ) -> Result<T, SagatError> {
        parser(self.value).map_err(|mut error| {
            error.status = Some(self.status);
            error.trace_id = self.trace_id;
            error
        })
    }
}

#[derive(Clone)]
pub struct ClientOptions {
    pub api_url: String,
    pub namespace: String,
    pub token: TokenSource,
    pub timeout: Duration,
    pub retry: RetryPolicy,
    pub max_response_bytes: usize,
    pub tls: crate::TlsOptions,
    pub user_agent_suffix: Option<String>,
}
impl ClientOptions {
    pub fn new(
        api_url: impl Into<String>,
        namespace: impl Into<String>,
        token: TokenSource,
    ) -> Self {
        Self {
            api_url: api_url.into(),
            namespace: namespace.into(),
            token,
            timeout: Duration::from_secs(30),
            retry: RetryPolicy::default(),
            max_response_bytes: MAX_RESPONSE,
            tls: crate::TlsOptions::default(),
            user_agent_suffix: None,
        }
    }
}
impl std::fmt::Debug for ClientOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClientOptions")
            .field("endpoint", &"<configured>")
            .field("token", &self.token)
            .field("timeout", &self.timeout)
            .field("retry", &self.retry)
            .finish_non_exhaustive()
    }
}
#[derive(Clone, Debug, Default)]
pub struct CallOptions {
    pub idempotency_key: Option<String>,
    pub traceparent: Option<String>,
    pub timeout: Option<Duration>,
    pub cancel: CancellationToken,
}
#[derive(Clone)]
pub struct Client {
    api: Url,
    options: ClientOptions,
    http: reqwest::Client,
}
impl std::fmt::Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Client")
            .field("endpoint", &"<configured>")
            .finish_non_exhaustive()
    }
}
struct WireRequest {
    method: Method,
    path: Vec<String>,
    body: Option<Vec<u8>>,
    key: Option<String>,
    repeatable: bool,
    expected: u16,
}
impl WireRequest {
    fn new(method: Method, path: &[&str], expected: u16) -> Self {
        let repeatable = method == Method::GET;
        Self {
            method,
            path: path.iter().map(|s| (*s).to_owned()).collect(),
            body: None,
            key: None,
            repeatable,
            expected,
        }
    }
    fn json(mut self, value: &Value) -> Result<Self, SagatError> {
        self.body = Some(serde_json::to_vec(value).map_err(|_| validation())?);
        Ok(self)
    }
    fn keyed(mut self, key: String) -> Self {
        self.key = Some(key);
        self.repeatable = true;
        self
    }
}
impl Client {
    pub fn new(options: ClientOptions) -> Result<Self, SagatError> {
        if options.user_agent_suffix.as_deref().is_some_and(|s| {
            s.len() > 128 || !s.is_ascii() || s.bytes().any(|b| b.is_ascii_control())
        }) {
            return Err(validation());
        }
        let mut api = Url::parse(&options.api_url).map_err(|_| validation())?;
        if !matches!(api.scheme(), "http" | "https")
            || api.host_str().is_none()
            || api.port() == Some(0)
            || !api.username().is_empty()
            || api.password().is_some()
            || api.query().is_some()
            || api.fragment().is_some()
            || options.namespace.is_empty()
            || options.namespace.len() > 63
            || !options
                .namespace
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            || !valid_timeout(options.timeout)
            || !(1..=100).contains(&options.retry.max_attempts)
            || options.retry.initial_delay > options.retry.max_delay
            || options.retry.max_delay > Duration::from_secs(60)
            || !(1..=MAX_RESPONSE).contains(&options.max_response_bytes)
        {
            return Err(validation());
        }
        api.path_segments_mut()
            .map_err(|_| validation())?
            .pop_if_empty();
        let tls = crate::tls::configuration(&options.tls)?;
        let http = reqwest::Client::builder()
            .tls_backend_preconfigured(tls)
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(options.timeout)
            .build()
            .map_err(|_| validation())?;
        if api.scheme() == "http" && !is_loopback(&api) {
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                tracing::warn!(
                    host = api.host_str().unwrap_or_default(),
                    "Sagat SDK uses plaintext HTTP"
                );
            }));
        }
        Ok(Self { api, options, http })
    }
    pub async fn start(
        &self,
        workflow_id: &str,
        input: &Value,
        options: impl Into<StartOptions>,
    ) -> Result<Run, SagatError> {
        uuid(workflow_id)?;
        if !input.is_object() {
            return Err(validation());
        }
        let options = options.into();
        let mut body = json!({"workflowId":workflow_id,"input":input});
        if let Some(version) = options.version_id {
            uuid(&version)?;
            body["versionId"] = Value::String(version);
        }
        if let Some(correlation) = options.correlation_id {
            body["correlationId"] = Value::String(correlation);
        }
        if let Some(deadline) = options.deadline {
            let instant = match deadline {
                RunDeadline::At(instant) => instant,
                RunDeadline::After(duration) => OffsetDateTime::now_utc()
                    .checked_add(time::Duration::try_from(duration).map_err(|_| validation())?)
                    .ok_or_else(validation)?,
            };
            body["deadline"] = Value::String(
                instant
                    .to_offset(UtcOffset::UTC)
                    .format(&Rfc3339)
                    .map_err(|_| validation())?,
            );
        }
        let request = WireRequest::new(Method::POST, &["runs"], 202)
            .json(&body)?
            .keyed(key(&options.call)?);
        self.execute(request, options.call).await?.parse(Run::parse)
    }
    pub async fn get(&self, run_id: &str, options: CallOptions) -> Result<Run, SagatError> {
        uuid(run_id)?;
        self.execute(
            WireRequest::new(Method::GET, &["runs", run_id], 200),
            options,
        )
        .await?
        .parse(Run::parse)
    }
    pub async fn cancel(
        &self,
        run_id: &str,
        reason: &str,
        options: impl Into<OperationOptions>,
    ) -> Result<OperationResult, SagatError> {
        self.operate(run_id, "CANCEL", reason, options).await
    }
    pub async fn operate(
        &self,
        run_id: &str,
        action: &str,
        reason: &str,
        options: impl Into<OperationOptions>,
    ) -> Result<OperationResult, SagatError> {
        uuid(run_id)?;
        let options = options.into();
        if !matches!(
            action,
            "CANCEL"
                | "PAUSE"
                | "RESUME"
                | "RERUN_AS_NEW"
                | "RETRY_STEP"
                | "RETRY_COMPENSATION"
                | "SKIP_COMPENSATION"
                | "RESOLVE_STEP"
                | "MARK_MANUAL_REVIEW"
        ) || !(8..=2000).contains(&reason.trim().chars().count())
        {
            return Err(validation());
        }
        let step = matches!(
            action,
            "RETRY_STEP"
                | "RETRY_COMPENSATION"
                | "SKIP_COMPENSATION"
                | "RESOLVE_STEP"
                | "MARK_MANUAL_REVIEW"
        );
        if step != options.step_id.is_some()
            || (action == "RESOLVE_STEP") != options.outcome.is_some()
            || (options.error_code.is_some()
                != matches!(options.outcome, Some(StepOutcome::Failed)))
            || options.error_code.as_deref().is_some_and(|c| {
                c.is_empty()
                    || c.len() > 64
                    || !c.bytes().all(|b| {
                        b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b':' | b'-')
                    })
            })
            || (!step && options.instance_path.is_some())
            || options
                .expected_run_version
                .is_some_and(|v| v > i64::MAX as u64)
        {
            return Err(validation());
        }
        let mut body = json!({"action":action,"reason":reason});
        if let Some(step) = options.step_id {
            if step.is_empty() {
                return Err(validation());
            }
            body["stepId"] = Value::String(step);
        }
        if let Some(outcome) = options.outcome {
            body["outcome"] = serde_json::to_value(outcome).map_err(|_| validation())?;
        }
        if let Some(code) = options.error_code {
            body["errorCode"] = Value::String(code);
        }
        if let Some(path) = options.instance_path {
            if path.len() > 8
                || path.iter().any(|p| {
                    p.scope_node_id.is_empty()
                        || p.scope_node_id.chars().count() > 64
                        || p.iteration_index > 9999
                })
            {
                return Err(validation());
            }
            body["instancePath"] = serde_json::to_value(path).map_err(|_| validation())?;
        }
        if let Some(version) = options.expected_run_version {
            body["expectedRunVersion"] = json!(version);
        }
        let request = WireRequest::new(Method::POST, &["runs", run_id, "operations"], 202)
            .json(&body)?
            .keyed(key(&options.call)?);
        self.execute(request, options.call).await?.parse(|raw| {
            let result = OperationResult::parse(raw)?;
            if (result.accepted && action == "RERUN_AS_NEW" && result.new_run_id.is_none())
                || (action != "RERUN_AS_NEW" && result.new_run_id.is_some())
            {
                return Err(protocol());
            }
            Ok(result)
        })
    }
    pub async fn signal(
        &self,
        run_id: &str,
        name: &str,
        payload: &Value,
        options: CallOptions,
    ) -> Result<SignalResult, SagatError> {
        uuid(run_id)?;
        path_name(name)?;
        if !payload.is_object() {
            return Err(validation());
        }
        let request = WireRequest::new(Method::POST, &["runs", run_id, "signals", name], 202)
            .json(payload)?
            .keyed(key(&options)?);
        self.execute(request, options)
            .await?
            .parse(SignalResult::parse)
    }
    async fn execute(
        &self,
        request: WireRequest,
        options: CallOptions,
    ) -> Result<WireReply, SagatError> {
        if options.cancel.is_cancelled() {
            return Err(cancelled());
        }
        if options
            .traceparent
            .as_deref()
            .is_some_and(|value| !valid_traceparent(value))
        {
            return Err(SagatError::new(
                "BAD_REQUEST",
                "the trace context is invalid",
            ));
        }
        let timeout = options.timeout.unwrap_or(self.options.timeout);
        if !valid_timeout(timeout) {
            return Err(validation());
        }
        let attempts = if request.repeatable {
            self.options.retry.max_attempts
        } else {
            1
        };
        for attempt in 0..attempts {
            let result = self.once(&request, &options, timeout).await;
            match result {
                Ok(value) => return Ok(value),
                Err(error) if error.may_retry() && attempt + 1 < attempts => {
                    let wait = self.options.retry.after(attempt, error.retry_after);
                    tokio::select! { biased; _=options.cancel.cancelled()=>return Err(cancelled()), _=tokio::time::sleep(wait)=>{} }
                }
                Err(error) => return Err(error),
            }
        }
        Err(protocol())
    }
    async fn once(
        &self,
        request: &WireRequest,
        options: &CallOptions,
        timeout: Duration,
    ) -> Result<WireReply, SagatError> {
        let deadline = tokio::time::Instant::now() + timeout;
        let token = tokio::select! {
            biased;
            _=options.cancel.cancelled()=>return Err(cancelled()),
            result=tokio::time::timeout_at(deadline,self.options.token.get())=>result.map_err(|_| SagatError::provider_failure())??,
        };
        let mut authorization = HeaderValue::from_str(&format!("Bearer {}", token.as_str()))
            .map_err(|_| SagatError::provider_failure())?;
        authorization.set_sensitive(true);
        let mut target = self.api.clone();
        {
            let mut segments = target.path_segments_mut().map_err(|_| validation())?;
            for segment in &request.path {
                segments.push(segment);
            }
        }
        let mut builder = self
            .http
            .request(request.method.clone(), target)
            .header(AUTHORIZATION, authorization)
            .header("accept", "application/json")
            .header("x-octopus-namespace", &self.options.namespace)
            .header(
                "user-agent",
                self.options
                    .user_agent_suffix
                    .as_deref()
                    .filter(|s| !s.is_empty())
                    .map_or_else(
                        || concat!("sagat-sdk-rust/", env!("CARGO_PKG_VERSION")).to_owned(),
                        |s| format!("sagat-sdk-rust/{} {s}", env!("CARGO_PKG_VERSION")),
                    ),
            );
        if let Some(key) = &request.key {
            builder = builder.header("idempotency-key", key);
        }
        if let Some(trace) = &options.traceparent {
            builder = builder.header("traceparent", trace);
        }
        if let Some(body) = &request.body {
            builder = builder
                .header("content-type", "application/json")
                .body(body.clone());
        }
        let response = tokio::select! {
            biased;
            _=options.cancel.cancelled()=>return Err(cancelled()),
            result=tokio::time::timeout_at(deadline,builder.send())=>result.map_err(|_|transport())?.map_err(|_|transport())?,
        };
        self.consume_response(response, request.expected, deadline, &options.cancel)
            .await
    }
    async fn consume_response(
        &self,
        response: reqwest::Response,
        expected: u16,
        deadline: tokio::time::Instant,
        cancel: &CancellationToken,
    ) -> Result<WireReply, SagatError> {
        let status = response.status().as_u16();
        let headers = response.headers().clone();
        let trace_id = headers
            .get("x-trace-id")
            .and_then(|v| v.to_str().ok())
            .and_then(crate::error::trace_id);
        if (300..400).contains(&status) {
            return Err(SagatError::from_http(status, &headers, &[]));
        }
        let consume = async {
            if response
                .content_length()
                .is_some_and(|n| n > self.options.max_response_bytes as u64)
            {
                return Err(protocol());
            }
            let stream = response.bytes_stream();
            futures_util::pin_mut!(stream);
            let mut bytes = Vec::new();
            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(|_| transport())?;
                if chunk.len() > self.options.max_response_bytes.saturating_sub(bytes.len()) {
                    return Err(protocol());
                }
                bytes.extend_from_slice(&chunk);
            }
            if !(200..300).contains(&status) {
                return Err(SagatError::from_http(status, &headers, &bytes));
            }
            if status != expected {
                return Err(protocol());
            }
            let value = if bytes.is_empty() {
                Value::Null
            } else {
                serde_json::from_slice(&bytes).map_err(|_| protocol())?
            };
            Ok(WireReply {
                value,
                status,
                trace_id: trace_id.clone(),
            })
        };
        let result = tokio::select! {
            biased;
            _=cancel.cancelled()=>Err(cancelled()),
            result=tokio::time::timeout_at(deadline,consume)=>result.map_err(|_|transport()).and_then(|r|r),
        };
        result.map_err(|mut error| {
            error.status = Some(status);
            if error.trace_id.is_none() {
                error.trace_id = trace_id;
            }
            error
        })
    }
}
fn valid_timeout(value: Duration) -> bool {
    !value.is_zero() && value <= Duration::from_secs(86_400)
}
fn uuid(value: &str) -> Result<(), SagatError> {
    Uuid::parse_str(value).map(|_| ()).map_err(|_| validation())
}
fn path_name(value: &str) -> Result<(), SagatError> {
    if value.is_empty() || matches!(value, "." | "..") {
        return Err(validation());
    }
    Ok(())
}
fn key(options: &CallOptions) -> Result<String, SagatError> {
    let key = options
        .idempotency_key
        .clone()
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    if !(8..=160).contains(&key.len()) || !key.bytes().all(|b| (0x21..=0x7e).contains(&b)) {
        return Err(validation());
    }
    Ok(key)
}
pub(crate) fn valid_traceparent(value: &str) -> bool {
    let fields: Vec<_> = value.split('-').collect();
    fields.len() == 4
        && fields[0] == "00"
        && fields[1].len() == 32
        && fields[2].len() == 16
        && fields[3].len() == 2
        && fields[1..]
            .iter()
            .all(|field| field.bytes().all(|b| b.is_ascii_hexdigit()))
        && fields[1].bytes().any(|b| b != b'0')
        && fields[2].bytes().any(|b| b != b'0')
}
fn validation() -> SagatError {
    SagatError::new("VALIDATION_FAILED", "a client argument is invalid")
}
fn protocol() -> SagatError {
    SagatError::new(
        "PROTOCOL",
        "the server response does not match the contract",
    )
}
fn transport() -> SagatError {
    SagatError::new("TRANSPORT", "no usable answer from the server")
}
fn cancelled() -> SagatError {
    SagatError::new("CANCELLED", "the call was cancelled")
}

fn is_loopback(url: &Url) -> bool {
    match url.host() {
        Some(url::Host::Domain(host)) => host.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    }
}

#[cfg(test)]
mod response_cancellation_tests {
    #![allow(clippy::expect_used)]
    use super::*;
    #[tokio::test]
    async fn cancellation_during_a_held_body_keeps_received_status_and_trace() {
        let polled = std::sync::Arc::new(tokio::sync::Notify::new());
        let observed = polled.clone();
        let stream = futures_util::stream::poll_fn(move |_| {
            observed.notify_one();
            std::task::Poll::Pending::<Option<Result<bytes::Bytes, std::io::Error>>>
        });
        let response = axum::http::Response::builder()
            .status(200)
            .header("x-trace-id", "0123456789abcdef0123456789abcdef")
            .body(reqwest::Body::wrap_stream(stream))
            .expect("response");
        let client = Client::new(ClientOptions::new(
            "http://localhost:1/v1",
            "demo",
            TokenSource::bearer("fixture-token"),
        ))
        .expect("client");
        let cancel = CancellationToken::new();
        let signal = cancel.clone();
        let action = async move {
            client
                .consume_response(
                    response.into(),
                    200,
                    tokio::time::Instant::now() + Duration::from_secs(10),
                    &signal,
                )
                .await
                .err()
                .expect("cancelled body")
        };
        let cancel_body = async move {
            polled.notified().await;
            cancel.cancel();
        };
        let (error, ()) = tokio::join!(action, cancel_body);
        assert_eq!(error.code, "CANCELLED");
        assert_eq!(error.status, Some(200));
        assert_eq!(
            error.trace_id.as_deref(),
            Some("0123456789abcdef0123456789abcdef")
        );
    }
}
