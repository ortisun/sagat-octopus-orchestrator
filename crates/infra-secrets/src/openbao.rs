//! OpenBao KV v2, immutable CAS writes and agent-renewed token-file authentication.
//! No HTTP error body is logged or propagated. Values and tokens use zeroizing buffers.
use std::{
    future::Future,
    net::IpAddr,
    path::PathBuf,
    time::{Duration, Instant},
};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use bytes::Bytes;
use reqwest::{Client, Method, Response, StatusCode, Url, header::HeaderValue};
use sagat_application::secret_provider::{
    ProviderError, ProviderFuture, ProviderReference, SecretProvider,
};
use sagat_domain::secrets::SecretValue;
use serde::{Deserialize, Serialize};
use tokio::io::AsyncReadExt;
use zeroize::{Zeroize, Zeroizing};

const MAX_RESPONSE: usize = 64 * 1024;
const MAX_TOKEN: u64 = 8192;

/// Deployment configuration. Callers cannot select addresses, mounts or token files in API input.
/// `development_loopback` is explicitly opt-in; production always verifies TLS and token policy.
pub struct OpenBaoConfig {
    pub address: String,
    pub mount: String,
    pub token_file: PathBuf,
    pub timeout: Duration,
    pub development_loopback: bool,
    pub ca_pem: Option<Vec<u8>>,
}

pub struct OpenBao {
    client: Client,
    base: Url,
    mount: String,
    token_file: PathBuf,
    timeout: Duration,
    development: bool,
}

impl std::fmt::Debug for OpenBao {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenBao")
            .field("development", &self.development)
            .finish_non_exhaustive()
    }
}

impl OpenBao {
    pub fn new(config: OpenBaoConfig) -> Result<Self, ProviderError> {
        let base = Url::parse(&config.address).map_err(|_| ProviderError::Configuration)?;
        let loopback = base.host_str().is_some_and(|host| {
            host.trim_matches(['[', ']'])
                .parse::<IpAddr>()
                .is_ok_and(|ip| ip.is_loopback())
        });
        let mount_valid = !config.mount.is_empty()
            && config.mount.len() <= 64
            && config
                .mount
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'));
        if !mount_valid
            || !base.username().is_empty()
            || base.password().is_some()
            || base.query().is_some()
            || base.fragment().is_some()
            || base.path() != "/"
            || !config.token_file.is_absolute()
            || config.timeout.is_zero()
            || config.timeout > Duration::from_secs(10)
            || if config.development_loopback {
                !loopback || !matches!(base.scheme(), "http" | "https")
            } else {
                base.scheme() != "https"
            }
        {
            return Err(ProviderError::Configuration);
        }
        // This workspace explicitly uses ring; avoid process-global implicit provider selection.
        let tls = rustls::crypto::ring::default_provider();
        let _ = tls.install_default();
        let mut builder = Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(config.timeout)
            .timeout(config.timeout)
            .pool_max_idle_per_host(2);
        if let Some(pem) = config.ca_pem {
            builder = builder.add_root_certificate(
                reqwest::Certificate::from_pem(&pem).map_err(|_| ProviderError::Configuration)?,
            );
        }
        Ok(Self {
            client: builder.build().map_err(|_| ProviderError::Configuration)?,
            base,
            mount: config.mount,
            token_file: config.token_file,
            timeout: config.timeout,
            development: config.development_loopback,
        })
    }

    fn url(&self, reference: &ProviderReference, endpoint: &str) -> Result<Url, ProviderError> {
        self.base
            .join(&format!(
                "v1/{}/{endpoint}/sagat/credentials/{}/{}/{}",
                self.mount, reference.tenant, reference.namespace, reference.operation
            ))
            .map_err(|_| ProviderError::Configuration)
    }

    async fn token(&self) -> Result<HeaderValue, ProviderError> {
        let file = tokio::fs::File::open(&self.token_file)
            .await
            .map_err(|_| ProviderError::Unavailable)?;
        let mut bytes = Zeroizing::new(Vec::new());
        file.take(MAX_TOKEN + 1)
            .read_to_end(&mut bytes)
            .await
            .map_err(|_| ProviderError::Unavailable)?;
        if bytes.len() > MAX_TOKEN as usize {
            return Err(ProviderError::Configuration);
        }
        let text = std::str::from_utf8(&bytes)
            .map_err(|_| ProviderError::Configuration)?
            .trim();
        if text.is_empty() || !text.bytes().all(|b| b.is_ascii_graphic()) {
            return Err(ProviderError::Configuration);
        }
        let mut token = HeaderValue::from_str(text).map_err(|_| ProviderError::Configuration)?;
        token.set_sensitive(true);
        if !self.development {
            let url = self
                .base
                .join("v1/auth/token/lookup-self")
                .map_err(|_| ProviderError::Configuration)?;
            let response = self
                .client
                .get(url)
                .header("X-Vault-Token", token.clone())
                .send()
                .await
                .map_err(|_| ProviderError::Unavailable)?;
            check_status(response.status())?;
            let body = bounded_body(response).await?;
            let lookup: TokenLookup =
                serde_json::from_slice(&body).map_err(|_| ProviderError::InvalidResponse)?;
            if lookup.data.ttl == 0
                || lookup.data.ttl > 86400
                || lookup.data.policies.iter().any(|p| p == "root")
            {
                return Err(ProviderError::Denied);
            }
        }
        Ok(token)
    }

    async fn request(
        &self,
        method: Method,
        url: Url,
        body: Option<Bytes>,
    ) -> Result<Response, ProviderError> {
        let token = self.token().await?;
        let mut request = self
            .client
            .request(method, url)
            .header("X-Vault-Token", token);
        if let Some(body) = body {
            request = request
                .header("Content-Type", "application/json")
                .body(body);
        }
        request.send().await.map_err(|_| ProviderError::Unavailable)
    }

    async fn measured<T>(
        &self,
        operation: &'static str,
        future: impl Future<Output = Result<T, ProviderError>>,
    ) -> Result<T, ProviderError> {
        let started = Instant::now();
        let result = tokio::time::timeout(self.timeout, future)
            .await
            .unwrap_or(Err(ProviderError::Unavailable));
        let outcome = match &result {
            Ok(_) => "ok",
            Err(ProviderError::Denied) => "denied",
            Err(ProviderError::Conflict) => "conflict",
            Err(_) => "failed",
        };
        metrics::counter!("secret_provider_requests_total", "provider" => "openbao", "operation" => operation, "outcome" => outcome).increment(1);
        metrics::histogram!("secret_provider_request_seconds", "provider" => "openbao", "operation" => operation).record(started.elapsed().as_secs_f64());
        result
    }

    #[tracing::instrument(name = "secret_provider.read", skip_all, fields(provider = "openbao"))]
    async fn read_inner(
        &self,
        reference: &ProviderReference,
    ) -> Result<Option<SecretValue>, ProviderError> {
        let mut url = self.url(reference, "data")?;
        url.set_query(Some("version=1"));
        let response = self.request(Method::GET, url, None).await?;
        if response.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        check_status(response.status())?;
        let body = bounded_body(response).await?;
        let value: ReadResponse =
            serde_json::from_slice(&body).map_err(|_| ProviderError::InvalidResponse)?;
        if value.data.metadata.version != 1
            || value.data.metadata.destroyed
            || !value.data.metadata.deletion_time.is_empty()
        {
            return Err(ProviderError::InvalidResponse);
        }
        let bytes = STANDARD
            .decode(&value.data.data.value)
            .map_err(|_| ProviderError::InvalidResponse)?;
        Ok(Some(
            SecretValue::new(bytes).map_err(|_| ProviderError::InvalidResponse)?,
        ))
    }

    #[tracing::instrument(
        name = "secret_provider.create",
        skip_all,
        fields(provider = "openbao")
    )]
    async fn create_inner(
        &self,
        reference: &ProviderReference,
        value: &SecretValue,
    ) -> Result<(), ProviderError> {
        let encoded = Zeroizing::new(STANDARD.encode(value.expose()));
        let payload = WriteRequest {
            options: Cas { cas: 0 },
            data: WriteData { value: &encoded },
        };
        let body =
            Zeroizing::new(serde_json::to_vec(&payload).map_err(|_| ProviderError::Configuration)?);
        let response = self
            .request(
                Method::POST,
                self.url(reference, "data")?,
                Some(Bytes::from_owner(body)),
            )
            .await?;
        if response.status() == StatusCode::BAD_REQUEST {
            // Do not trust or inspect provider diagnostics to classify CAS. An existing immutable
            // receipt proves the conflict; absence means a refused operation, not success.
            return Err(if self.read_inner(reference).await?.is_some() {
                ProviderError::Conflict
            } else {
                ProviderError::Rejected
            });
        }
        check_status(response.status())?;
        let body = bounded_body(response).await?;
        let receipt: WriteResponse =
            serde_json::from_slice(&body).map_err(|_| ProviderError::InvalidResponse)?;
        if receipt.data.version != 1 {
            return Err(ProviderError::InvalidResponse);
        }
        Ok(())
    }
}

impl SecretProvider for OpenBao {
    fn create<'a>(
        &'a self,
        reference: &'a ProviderReference,
        value: &'a SecretValue,
    ) -> ProviderFuture<'a, ()> {
        Box::pin(async move {
            self.measured("create", self.create_inner(reference, value))
                .await
        })
    }
    fn read<'a>(
        &'a self,
        reference: &'a ProviderReference,
    ) -> ProviderFuture<'a, Option<SecretValue>> {
        Box::pin(async move { self.measured("read", self.read_inner(reference)).await })
    }
    fn delete<'a>(&'a self, reference: &'a ProviderReference) -> ProviderFuture<'a, ()> {
        Box::pin(async move {
            self.measured("delete", async {
                let response = self
                    .request(Method::DELETE, self.url(reference, "metadata")?, None)
                    .await?;
                if response.status() == StatusCode::NOT_FOUND {
                    Ok(())
                } else {
                    check_status(response.status())
                }
            })
            .await
        })
    }
}

fn check_status(status: StatusCode) -> Result<(), ProviderError> {
    match status.as_u16() {
        200 | 204 => Ok(()),
        401 | 403 => Err(ProviderError::Denied),
        429 | 500..=599 => Err(ProviderError::Unavailable),
        400 | 404 | 405 => Err(ProviderError::Rejected),
        _ => Err(ProviderError::InvalidResponse),
    }
}

async fn bounded_body(mut response: Response) -> Result<Zeroizing<Vec<u8>>, ProviderError> {
    if response
        .content_length()
        .is_some_and(|n| n > MAX_RESPONSE as u64)
    {
        return Err(ProviderError::InvalidResponse);
    }
    let mut result = Zeroizing::new(Vec::new());
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| ProviderError::Unavailable)?
    {
        if chunk.len() > MAX_RESPONSE - result.len() {
            return Err(ProviderError::InvalidResponse);
        }
        result.extend_from_slice(&chunk);
    }
    Ok(result)
}

#[derive(Serialize)]
struct Cas {
    cas: u32,
}
#[derive(Serialize)]
struct WriteData<'a> {
    value: &'a str,
}
#[derive(Serialize)]
struct WriteRequest<'a> {
    options: Cas,
    data: WriteData<'a>,
}
#[derive(Deserialize)]
struct WriteResponse {
    data: Version,
}
#[derive(Deserialize)]
struct Version {
    version: u32,
}
#[derive(Deserialize)]
struct ReadResponse {
    data: ReadData,
}
#[derive(Deserialize)]
struct ReadData {
    data: EncodedValue,
    metadata: Metadata,
}
#[derive(Deserialize)]
struct EncodedValue {
    value: String,
}
impl Drop for EncodedValue {
    fn drop(&mut self) {
        self.value.zeroize();
    }
}
#[derive(Deserialize)]
struct Metadata {
    version: u32,
    destroyed: bool,
    deletion_time: String,
}
#[derive(Deserialize)]
struct TokenLookup {
    data: TokenData,
}
#[derive(Deserialize)]
struct TokenData {
    ttl: u64,
    policies: Vec<String>,
}
