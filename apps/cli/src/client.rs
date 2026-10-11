//! The platform's REST API, as the command line speaks it.
//!
//! One bearer token, one namespace header, a deadline on every call, and no redirect: a token is
//! sent where the person said and nowhere else. A failure carries the platform's own words (the
//! code, the reason, the message, what to do about it); the token is never printed.

use std::time::Duration;

use serde_json::Value;

/// What the platform answered when it refused.
#[derive(Debug, Clone, PartialEq)]
pub struct Failure {
    pub status: u16,
    pub code: String,
    pub reason: Option<String>,
    pub message: String,
    pub remediation: Option<String>,
    pub details: Value,
    pub trace_id: Option<String>,
}

impl Failure {
    #[must_use]
    pub fn is(&self, reason: &str) -> bool {
        self.reason.as_deref() == Some(reason)
    }

    fn from_body(status: u16, body: &Value) -> Self {
        let text = |key: &str| body.get(key).and_then(Value::as_str).map(str::to_owned);
        Self {
            status,
            code: text("code").unwrap_or_else(|| format!("HTTP_{status}")),
            reason: body
                .pointer("/details/reason")
                .and_then(Value::as_str)
                .map(str::to_owned),
            message: text("message")
                .unwrap_or_else(|| "the platform refused the request".to_owned()),
            remediation: text("remediation"),
            details: body.get("details").cloned().unwrap_or(Value::Null),
            trace_id: text("traceId"),
        }
    }
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({}", self.message, self.code)?;
        if let Some(reason) = &self.reason {
            write!(f, ", {reason}")?;
        }
        write!(f, ")")?;
        if let Some(todo) = &self.remediation {
            write!(f, ". {todo}")?;
        }
        if let Some(trace) = &self.trace_id {
            write!(f, " Trace {trace}.")?;
        }
        Ok(())
    }
}

/// Why a call did not give an answer to use.
#[derive(Debug, Clone, PartialEq)]
pub enum CallError {
    /// The platform could not be reached, or did not answer in time.
    Transport(String),
    /// The platform answered with a refusal.
    Refused(Box<Failure>),
}

impl std::fmt::Display for CallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Transport(why) => write!(f, "the platform could not be reached ({why})"),
            Self::Refused(failure) => failure.fmt(f),
        }
    }
}

/// The most of an answer this tool reads: a page of drafts of the largest size the platform keeps.
const MAX_ANSWER: usize = 128 * 1024 * 1024;

/// The platform, a token and a namespace.
pub struct Client {
    http: reqwest::Client,
    base: String,
    token: String,
    namespace: String,
}

/// An `https` address, or an `http` one to this machine: a bearer token is never sent in the clear
/// across a network.
fn is_safe_address(base: &str) -> bool {
    if base.starts_with("https://") {
        return base.len() > "https://".len();
    }
    let Some(rest) = base.strip_prefix("http://") else {
        return false;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = if authority.starts_with('[') {
        authority.split(']').next().map(|h| format!("{h}]"))
    } else {
        authority.split(':').next().map(str::to_owned)
    };
    matches!(host.as_deref(), Some("localhost" | "127.0.0.1" | "[::1]"))
}

/// A path with its query string, the values percent-encoded.
#[must_use]
pub fn with_query(path: &str, params: &[(&str, &str)]) -> String {
    let mut out = path.to_owned();
    for (i, (name, value)) in params.iter().enumerate() {
        out.push(if i == 0 { '?' } else { '&' });
        out.push_str(name);
        out.push('=');
        for byte in value.bytes() {
            if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
                out.push(char::from(byte));
            } else {
                out.push_str(&format!("%{byte:02X}"));
            }
        }
    }
    out
}

fn install_crypto_provider() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

impl Client {
    /// # Errors
    /// A message when the address is not an http(s) URL or the token cannot travel in a header.
    pub fn new(server: &str, token: &str, namespace: &str) -> Result<Self, String> {
        let base = server.trim_end_matches('/').to_owned();
        if !is_safe_address(&base) {
            return Err(
                "the server is an https URL (plain http only for localhost, 127.0.0.1 or [::1]): \
                 the token must not travel in the clear"
                    .to_owned(),
            );
        }
        if token.is_empty() || !token.bytes().all(|b| b.is_ascii_graphic()) {
            return Err(
                "the token is empty or has characters that cannot travel in a header".to_owned(),
            );
        }
        install_crypto_provider();
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| "the HTTP client could not be built".to_owned())?;
        Ok(Self {
            http,
            base,
            token: token.to_owned(),
            namespace: namespace.to_owned(),
        })
    }

    /// Sends a request; any status is an answer.
    async fn send(
        &self,
        method: &str,
        path: &str,
        body: Option<&Value>,
        key: Option<&str>,
    ) -> Result<(u16, Value), CallError> {
        let method = reqwest::Method::from_bytes(method.as_bytes())
            .map_err(|_| CallError::Transport("a method".to_owned()))?;
        let mut request = self
            .http
            .request(method, format!("{}{path}", self.base))
            .bearer_auth(&self.token)
            .header("x-octopus-namespace", &self.namespace);
        if let Some(key) = key {
            request = request.header("idempotency-key", key);
        }
        if let Some(body) = body {
            request = request
                .header("content-type", "application/json")
                .body(serde_json::to_vec(body).map_err(|e| CallError::Transport(e.to_string()))?);
        }
        let response = request.send().await.map_err(|e| {
            CallError::Transport(if e.is_timeout() {
                "it took too long".to_owned()
            } else if e.is_connect() {
                "connection refused or unreachable".to_owned()
            } else {
                "transport".to_owned()
            })
        })?;
        let status = response.status().as_u16();
        let mut response = response;
        let mut bytes: Vec<u8> = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| CallError::Transport("the answer was cut short".to_owned()))?
        {
            if bytes.len() + chunk.len() > MAX_ANSWER {
                return Err(CallError::Transport(
                    "the answer is larger than this tool reads".to_owned(),
                ));
            }
            bytes.extend_from_slice(&chunk);
        }
        let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        Ok((status, value))
    }

    /// Sends a request and gives the body of a success.
    ///
    /// # Errors
    /// [`CallError`] for no answer or a refusal.
    pub async fn call(
        &self,
        method: &str,
        path: &str,
        body: Option<&Value>,
        key: Option<&str>,
    ) -> Result<(u16, Value), CallError> {
        let (status, value) = self.send(method, path, body, key).await?;
        if (200..300).contains(&status) {
            Ok((status, value))
        } else {
            Err(CallError::Refused(Box::new(Failure::from_body(
                status, &value,
            ))))
        }
    }
}
