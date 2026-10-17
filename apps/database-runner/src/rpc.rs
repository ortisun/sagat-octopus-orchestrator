//! Bounded authenticated worker calls. Tokens are reread for every RPC and diagnostics are closed.
use crate::{
    RunnerError,
    config::{Config, bounded_file},
};
use sagat_api_grpc::pb::{self, worker_service_client::WorkerServiceClient};
use std::{path::PathBuf, time::Duration};
use tonic::{
    Request,
    metadata::MetadataValue,
    transport::{Certificate, Channel, ClientTlsConfig, Endpoint},
};
use zeroize::Zeroizing;

#[derive(Clone)]
pub(crate) struct Rpc {
    pub client: WorkerServiceClient<Channel>,
    token_file: PathBuf,
    timeout: Duration,
}
#[derive(Debug, Clone, Copy)]
pub(crate) struct Failure {
    pub code: tonic::Code,
    pub quota: bool,
}
impl From<tonic::Status> for Failure {
    fn from(status: tonic::Status) -> Self {
        Self {
            code: status.code(),
            quota: status
                .metadata()
                .get("x-sagat-error")
                .is_some_and(|v| v == "QUOTA_EXCEEDED"),
        }
    }
}
impl Failure {
    pub fn retryable(self) -> bool {
        !self.quota
            && matches!(
                self.code,
                tonic::Code::Unavailable
                    | tonic::Code::DeadlineExceeded
                    | tonic::Code::ResourceExhausted
            )
    }
}
impl Rpc {
    pub async fn connect(config: &Config) -> Result<Self, RunnerError> {
        let mut endpoint = Endpoint::from_shared(config.grpc_url.clone())
            .map_err(|_| RunnerError::Configuration)?
            .connect_timeout(config.rpc_timeout())
            .timeout(config.rpc_timeout());
        if config.grpc_url.starts_with("https://") {
            let tls = if let Some(path) = &config.ca_file {
                let ca = bounded_file(path, 256 * 1024).await?;
                ClientTlsConfig::new().ca_certificate(Certificate::from_pem(ca.as_slice()))
            } else {
                ClientTlsConfig::new().with_native_roots()
            };
            endpoint = endpoint
                .tls_config(tls)
                .map_err(|_| RunnerError::Configuration)?;
        }
        let channel = tokio::time::timeout(config.rpc_timeout(), endpoint.connect())
            .await
            .map_err(|_| RunnerError::Transport)?
            .map_err(|_| RunnerError::Transport)?;
        Ok(Self {
            client: WorkerServiceClient::new(channel)
                .max_decoding_message_size(2 * 1024 * 1024)
                .max_encoding_message_size(2 * 1024 * 1024),
            token_file: config.token_file.clone(),
            timeout: config.rpc_timeout(),
        })
    }
    pub async fn request<T>(&self, body: T) -> Result<Request<T>, Failure> {
        let fail = || Failure {
            code: tonic::Code::Unauthenticated,
            quota: false,
        };
        let bytes = tokio::time::timeout(self.timeout, bounded_file(&self.token_file, 8192))
            .await
            .map_err(|_| fail())?
            .map_err(|_| fail())?;
        let token = std::str::from_utf8(&bytes).map_err(|_| fail())?.trim();
        if token.is_empty() || token.chars().any(char::is_whitespace) {
            return Err(fail());
        }
        let bearer = Zeroizing::new(format!("Bearer {token}"));
        let mut value = MetadataValue::try_from(bearer.as_str()).map_err(|_| fail())?;
        value.set_sensitive(true);
        let mut request = Request::new(body);
        request.set_timeout(self.timeout);
        request.metadata_mut().insert("authorization", value);
        Ok(request)
    }
}
pub(crate) fn task(lease: &pb::TaskLease) -> pb::ResolveSecretsRequest {
    pb::ResolveSecretsRequest {
        task_id: lease.task_id.clone(),
        run_id: lease.run_id.clone(),
        step_id: lease.step_id.clone(),
        attempt: lease.attempt,
        generation: lease.generation,
        lease_token: lease.lease_token.clone(),
        traceparent: lease.traceparent.clone(),
    }
}
