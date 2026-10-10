//! The control plane as the runner sees it: a handful of REST calls, each scoped to the tenant of
//! the runner's identity. The runner has no other way to learn what to build or run.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use reqwest::{Method, StatusCode};
use sagat_domain::scripts::{Digest, Language, Policy};
use serde::Deserialize;
use serde_json::{Value, json};

/// What went wrong talking to the control plane.
#[derive(Debug, thiserror::Error)]
pub enum RunnerError {
    /// The request did not complete.
    #[error("the control plane could not be reached")]
    Transport,
    /// The control plane answered with an error. `code` is its stable code, `reason` the detail
    /// that tells apart errors with the same code (never a payload).
    #[error("the control plane answered {status} ({code}, {reason})")]
    Status {
        status: u16,
        code: String,
        reason: String,
    },
    /// The answer is not what the contract says.
    #[error("the control plane's answer could not be read")]
    Decode,
    /// The credentials could not be read.
    #[error("the credentials could not be read")]
    Credentials,
    /// The container engine, or the artifact, could not be used.
    #[error("{0}")]
    Sandbox(String),
}

impl RunnerError {
    /// Whether the control plane said that the lease is no longer this runner's.
    #[must_use]
    pub fn is_stale(&self) -> bool {
        matches!(self, Self::Status { status: 409, reason, .. } if reason == "STALE_ATTEMPT")
    }
}

/// How the runner proves who it is: a token for the identity provider's audience, read at every
/// request so a rotated token is used at once.
#[derive(Clone)]
pub enum Credentials {
    Static(String),
    File(PathBuf),
    /// For tests and embedders: mints a token on demand.
    Provider(Arc<dyn Fn() -> String + Send + Sync>),
}

impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // A token never prints.
        f.write_str(match self {
            Self::Static(_) => "Credentials::Static(<redacted>)",
            Self::File(_) => "Credentials::File(..)",
            Self::Provider(_) => "Credentials::Provider(..)",
        })
    }
}

impl Credentials {
    /// The token to present now.
    ///
    /// # Errors
    /// [`RunnerError::Credentials`] when the file cannot be read.
    pub async fn token(&self) -> Result<String, RunnerError> {
        match self {
            Self::Static(token) => Ok(token.clone()),
            Self::File(path) => tokio::fs::read_to_string(path)
                .await
                .map(|text| text.trim().to_owned())
                .map_err(|_| RunnerError::Credentials),
            Self::Provider(mint) => Ok(mint()),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildJob {
    pub job_id: String,
    pub lease_token: String,
    pub lease_seconds: u64,
    pub language: String,
    pub source: String,
    #[serde(default)]
    pub lockfile: Option<String>,
    pub policy: String,
    #[serde(default)]
    pub input_schema: Option<Value>,
    #[serde(default)]
    pub output_schema: Option<Value>,
    pub build_key: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TestJob {
    pub run_id: String,
    pub lease_token: String,
    pub lease_seconds: u64,
    pub namespace_id: String,
    pub language: String,
    pub source: String,
    #[serde(default)]
    pub lockfile: Option<String>,
    #[serde(default)]
    pub input_schema: Option<Value>,
    #[serde(default)]
    pub output_schema: Option<Value>,
    pub policy: String,
    pub input: Value,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactView {
    pub tenant_id: String,
    pub digest: String,
    pub language: String,
    pub artifact: String,
    pub signature: String,
    pub key_id: String,
}

impl ArtifactView {
    /// The artifact's bytes.
    pub fn bytes(&self) -> Result<Vec<u8>, RunnerError> {
        STANDARD
            .decode(&self.artifact)
            .map_err(|_| RunnerError::Decode)
    }

    pub fn signature_bytes(&self) -> Result<Vec<u8>, RunnerError> {
        STANDARD
            .decode(&self.signature)
            .map_err(|_| RunnerError::Decode)
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EgressView {
    pub rules: Vec<String>,
    pub ports: Vec<u16>,
    pub allow_http: bool,
}

/// What a test run came to.
#[derive(Debug, Clone, Default)]
pub struct TestResult {
    pub result: Option<Value>,
    pub failure: Option<String>,
    pub log: String,
    pub log_truncated: bool,
    pub denied: Vec<String>,
    pub exit_code: Option<i64>,
    pub wall_ms: u64,
    pub start_ms: u64,
    pub memory_peak_bytes: Option<u64>,
    pub inferred_input_schema: Option<Value>,
    pub inferred_output_schema: Option<Value>,
}

/// What a production step came to, for the record kept of it.
#[derive(Debug, Clone)]
pub struct ExecutionReport {
    pub namespace_id: String,
    pub digest: String,
    pub run_id: String,
    pub node: String,
    pub attempt: u32,
    pub failure: Option<String>,
    pub exit_code: Option<i64>,
    pub log: String,
    pub log_truncated: bool,
    pub denied: Vec<String>,
    pub wall_ms: u64,
    pub start_ms: u64,
    pub memory_peak_bytes: Option<u64>,
    pub trace_id: Option<String>,
}

#[derive(Clone)]
pub struct ControlPlane {
    base: String,
    http: reqwest::Client,
    credentials: Credentials,
}

impl std::fmt::Debug for ControlPlane {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ControlPlane")
            .field("base", &self.base)
            .finish_non_exhaustive()
    }
}

impl ControlPlane {
    /// `base` is the API root, e.g. `https://control-plane/v1`. `ca` is an extra root certificate
    /// (PEM) for a control plane behind a private authority.
    ///
    /// # Errors
    /// [`RunnerError::Transport`] if the client cannot be built.
    pub fn new(
        base: &str,
        credentials: Credentials,
        ca: Option<&[u8]>,
    ) -> Result<Self, RunnerError> {
        sagat_api_http::auth::install_crypto_provider();
        let mut builder = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::none())
            // A proxy from the environment would carry the runner's credentials somewhere else.
            .no_proxy();
        if let Some(pem) = ca {
            let certificate =
                reqwest::Certificate::from_pem(pem).map_err(|_| RunnerError::Transport)?;
            builder = builder.add_root_certificate(certificate);
        }
        Ok(Self {
            base: base.trim_end_matches('/').to_owned(),
            http: builder.build().map_err(|_| RunnerError::Transport)?,
            credentials,
        })
    }

    async fn call(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<Option<Value>, RunnerError> {
        let token = self.credentials.token().await?;
        let mut request = self
            .http
            .request(method, format!("{}{path}", self.base))
            .bearer_auth(token);
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request.send().await.map_err(|_| RunnerError::Transport)?;
        let status = response.status();
        if status == StatusCode::NO_CONTENT {
            return Ok(None);
        }
        let text = response.text().await.map_err(|_| RunnerError::Transport)?;
        let value: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        if status.is_success() {
            return Ok(Some(value));
        }
        Err(RunnerError::Status {
            status: status.as_u16(),
            code: value["code"].as_str().unwrap_or_default().to_owned(),
            reason: value["details"]["reason"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
        })
    }

    fn decode<T: for<'de> Deserialize<'de>>(
        value: Option<Value>,
    ) -> Result<Option<T>, RunnerError> {
        value
            .map(|v| serde_json::from_value(v).map_err(|_| RunnerError::Decode))
            .transpose()
    }

    /// The oldest build for one of the toolchains, if there is one.
    ///
    /// # Errors
    /// [`RunnerError`] if the call fails.
    pub async fn claim_build(
        &self,
        toolchains: &BTreeMap<String, String>,
    ) -> Result<Option<BuildJob>, RunnerError> {
        let answer = self
            .call(
                Method::POST,
                "/script-runner/builds/claim",
                Some(json!({ "toolchains": toolchains })),
            )
            .await?;
        Self::decode(answer)
    }

    /// Extends the lease. `false` when the build is not this runner's any more.
    ///
    /// # Errors
    /// [`RunnerError`] for anything but a lost lease.
    pub async fn heartbeat_build(&self, job: &str, lease: &str) -> Result<bool, RunnerError> {
        self.heartbeat(&format!("/script-runner/builds/{job}/heartbeat"), lease)
            .await
    }

    async fn heartbeat(&self, path: &str, lease: &str) -> Result<bool, RunnerError> {
        match self
            .call(Method::POST, path, Some(json!({ "leaseToken": lease })))
            .await
        {
            Ok(_) => Ok(true),
            Err(error) if error.is_stale() => Ok(false),
            Err(error) => Err(error),
        }
    }

    /// Reports a finished build.
    ///
    /// # Errors
    /// [`RunnerError`]; a lost lease is [`RunnerError::is_stale`].
    pub async fn complete_build_built(
        &self,
        job: &str,
        lease: &str,
        artifact: &[u8],
        sbom: &[u8],
        toolchain: &str,
        log: &str,
    ) -> Result<(), RunnerError> {
        self.call(
            Method::POST,
            &format!("/script-runner/builds/{job}/complete"),
            Some(json!({
                "leaseToken": lease,
                "outcome": "built",
                "artifact": STANDARD.encode(artifact),
                "sbom": serde_json::from_slice::<Value>(sbom).unwrap_or(json!({})),
                "toolchain": toolchain,
                "log": log,
            })),
        )
        .await
        .map(|_| ())
    }

    /// Reports a build that made nothing. `kind` is `rejected`, `lockfile`, `limit`, `unsupported`
    /// or `unavailable`.
    ///
    /// # Errors
    /// [`RunnerError`].
    pub async fn complete_build_failed(
        &self,
        job: &str,
        lease: &str,
        kind: &str,
        message: &str,
        log: &str,
    ) -> Result<(), RunnerError> {
        self.call(
            Method::POST,
            &format!("/script-runner/builds/{job}/complete"),
            Some(json!({
                "leaseToken": lease,
                "outcome": "failed",
                "kind": kind,
                "message": message,
                "log": log,
            })),
        )
        .await
        .map(|_| ())
    }

    /// The oldest test run of one of the languages.
    ///
    /// # Errors
    /// [`RunnerError`].
    pub async fn claim_test(&self, languages: &[Language]) -> Result<Option<TestJob>, RunnerError> {
        let names: Vec<&str> = languages.iter().map(|l| l.as_str()).collect();
        let answer = self
            .call(
                Method::POST,
                "/script-runner/test-runs/claim",
                Some(json!({ "languages": names })),
            )
            .await?;
        Self::decode(answer)
    }

    /// # Errors
    /// [`RunnerError`] for anything but a lost lease.
    pub async fn heartbeat_test(&self, run: &str, lease: &str) -> Result<bool, RunnerError> {
        self.heartbeat(&format!("/script-runner/test-runs/{run}/heartbeat"), lease)
            .await
    }

    /// # Errors
    /// [`RunnerError`].
    pub async fn complete_test(
        &self,
        run: &str,
        lease: &str,
        result: &TestResult,
    ) -> Result<(), RunnerError> {
        self.call(
            Method::POST,
            &format!("/script-runner/test-runs/{run}/complete"),
            Some(json!({
                "leaseToken": lease,
                "result": result.result,
                "failure": result.failure,
                "log": result.log,
                "logTruncated": result.log_truncated,
                "denied": result.denied,
                "exitCode": result.exit_code,
                "wallMs": result.wall_ms,
                "startMs": result.start_ms,
                "memoryPeakBytes": result.memory_peak_bytes,
                "inferredInputSchema": result.inferred_input_schema,
                "inferredOutputSchema": result.inferred_output_schema,
            })),
        )
        .await
        .map(|_| ())
    }

    /// An artifact of the tenant, with its signature.
    ///
    /// # Errors
    /// [`RunnerError`]; a digest the tenant does not have is a `404`.
    pub async fn artifact(&self, digest: &Digest) -> Result<ArtifactView, RunnerError> {
        Self::decode(
            self.call(
                Method::GET,
                &format!("/script-runner/artifacts/{digest}"),
                None,
            )
            .await?,
        )?
        .ok_or(RunnerError::Decode)
    }

    /// The public key the control plane signs artifacts with.
    ///
    /// # Errors
    /// [`RunnerError`].
    pub async fn signing_key(&self) -> Result<Vec<u8>, RunnerError> {
        let answer = self
            .call(Method::GET, "/script-runner/signing-key", None)
            .await?
            .ok_or(RunnerError::Decode)?;
        STANDARD
            .decode(answer["publicKey"].as_str().unwrap_or_default())
            .map_err(|_| RunnerError::Decode)
    }

    /// The egress policy of a namespace.
    ///
    /// # Errors
    /// [`RunnerError`].
    pub async fn egress_policy(&self, namespace: &str) -> Result<EgressView, RunnerError> {
        Self::decode(
            self.call(
                Method::GET,
                &format!("/script-runner/namespaces/{namespace}/egress-policy"),
                None,
            )
            .await?,
        )?
        .ok_or(RunnerError::Decode)
    }

    /// The script activities with a step ready in a namespace.
    ///
    /// # Errors
    /// [`RunnerError`].
    pub async fn pending(&self, namespace: &str) -> Result<Vec<String>, RunnerError> {
        let answer = self
            .call(
                Method::GET,
                &format!("/script-runner/namespaces/{namespace}/pending-activities"),
                None,
            )
            .await?
            .ok_or(RunnerError::Decode)?;
        Ok(answer["activities"]
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default())
    }

    /// Keeps the record of an execution.
    ///
    /// # Errors
    /// [`RunnerError`].
    pub async fn record_execution(&self, report: &ExecutionReport) -> Result<(), RunnerError> {
        self.call(
            Method::POST,
            "/script-runner/executions",
            Some(json!({
                "namespaceId": report.namespace_id,
                "digest": report.digest,
                "runId": report.run_id,
                "node": report.node,
                "attempt": report.attempt,
                "status": if report.failure.is_none() { "SUCCEEDED" } else { "FAILED" },
                "failure": report.failure,
                "exitCode": report.exit_code,
                "log": report.log,
                "logTruncated": report.log_truncated,
                "denied": report.denied,
                "wallMs": report.wall_ms,
                "startMs": report.start_ms,
                "memoryPeakBytes": report.memory_peak_bytes,
                "traceId": report.trace_id,
            })),
        )
        .await
        .map(|_| ())
    }
}

/// The policy text of a job, read as input from outside.
///
/// # Errors
/// [`RunnerError::Decode`] if it is not a policy the platform writes.
pub fn policy_of(text: &str) -> Result<Policy, RunnerError> {
    Policy::parse_canonical(text).map_err(|_| RunnerError::Decode)
}
