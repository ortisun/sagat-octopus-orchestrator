//! Serving the worker service: connection and request limits.

use std::future::Future;
use std::time::Duration;

use sagat_application::execution::MAX_PAYLOAD_BYTES;
use tokio::net::TcpListener;
use tonic::transport::{Certificate, Identity, Server, ServerTlsConfig};

use crate::limits::{Admission, Incoming};
use crate::pb::worker_service_server::{WorkerService, WorkerServiceServer};

/// Largest message the server accepts: an inline output (256 KiB) plus the envelope.
const MAX_MESSAGE_BYTES: usize = MAX_PAYLOAD_BYTES + 64 * 1024;

/// The server's certificate and key, and (for mutual TLS) the CA that signs the clients' certificates,
/// all PEM. The private key never prints.
#[derive(Clone)]
pub struct TlsConfig {
    pub certificate_pem: Vec<u8>,
    pub private_key_pem: Vec<u8>,
    /// When set, a client must present a certificate signed by this CA (mutual TLS). The channel is
    /// then authenticated as well as encrypted; the bearer token still names the worker.
    pub client_ca_pem: Option<Vec<u8>>,
}

impl std::fmt::Debug for TlsConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TlsConfig")
            .field("private_key_pem", &"<redacted>")
            .field("mutual", &self.client_ca_pem.is_some())
            .finish_non_exhaustive()
    }
}

/// Connection-level limits of the gRPC server.
#[derive(Debug, Clone)]
pub struct GrpcConfig {
    /// Upper bound for one call, including waiting for the run's lock.
    pub request_timeout: Duration,
    /// Calls running at once on one connection.
    pub concurrency_per_connection: usize,
    /// HTTP/2 streams a client may open at once.
    pub max_concurrent_streams: u32,
    /// Connections open at once; the next one is accepted when one closes.
    pub max_connections: usize,
    /// Calls running at once across all connections. Above three quarters of it the calls that look
    /// for work (`RegisterWorker`, `PollTasks`) are shed with `RESOURCE_EXHAUSTED`, the rest only
    /// when it is full, so a reserve stays for the calls that keep leased work alive.
    pub max_in_flight: usize,
    /// Ping interval that detects dead peers behind a NAT or a silent load balancer.
    pub keepalive_interval: Duration,
    /// How long to wait for the ping answer before dropping the connection.
    pub keepalive_timeout: Duration,
    /// TLS on the listener. `None` serves plaintext HTTP/2, which is only right behind something
    /// that terminates TLS (a mesh sidecar, an ingress) or in development.
    pub tls: Option<TlsConfig>,
}

impl Default for GrpcConfig {
    fn default() -> Self {
        Self {
            request_timeout: Duration::from_secs(30),
            concurrency_per_connection: 64,
            max_concurrent_streams: 128,
            max_connections: 2048,
            max_in_flight: 1024,
            keepalive_interval: Duration::from_secs(30),
            keepalive_timeout: Duration::from_secs(10),
            tls: None,
        }
    }
}

/// Wraps a handler into the tonic service, applying the message size limit.
#[must_use]
pub fn worker_service<T: WorkerService>(handler: T) -> WorkerServiceServer<T> {
    WorkerServiceServer::new(handler).max_decoding_message_size(MAX_MESSAGE_BYTES)
}

/// Serves `service` on `listener` until `shutdown` completes, then drains in-flight calls.
///
/// # Errors
/// The transport error that stopped the server.
pub async fn serve<T: WorkerService>(
    listener: TcpListener,
    service: WorkerServiceServer<T>,
    config: GrpcConfig,
    shutdown: impl Future<Output = ()>,
) -> Result<(), tonic::transport::Error> {
    let mut builder = Server::builder();
    if let Some(tls) = &config.tls {
        let mut server_tls = ServerTlsConfig::new().identity(Identity::from_pem(
            &tls.certificate_pem,
            &tls.private_key_pem,
        ));
        if let Some(ca) = &tls.client_ca_pem {
            server_tls = server_tls.client_ca_root(Certificate::from_pem(ca));
        }
        builder = builder.tls_config(server_tls)?;
    }
    builder
        .timeout(config.request_timeout)
        .concurrency_limit_per_connection(config.concurrency_per_connection)
        .max_concurrent_streams(config.max_concurrent_streams)
        .http2_keepalive_interval(Some(config.keepalive_interval))
        .http2_keepalive_timeout(Some(config.keepalive_timeout))
        .layer(Admission::new(config.max_in_flight))
        .add_service(service)
        .serve_with_incoming_shutdown(Incoming::new(listener, config.max_connections), shutdown)
        .await
}
