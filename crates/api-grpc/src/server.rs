//! Serving the worker service: connection and request limits.

use std::future::Future;
use std::time::Duration;

use sagat_application::execution::MAX_PAYLOAD_BYTES;
use tokio::net::TcpListener;
use tokio_stream::wrappers::TcpListenerStream;
use tonic::transport::Server;

use crate::pb::worker_service_server::{WorkerService, WorkerServiceServer};

/// Largest message the server accepts: an inline output (256 KiB) plus the envelope.
const MAX_MESSAGE_BYTES: usize = MAX_PAYLOAD_BYTES + 64 * 1024;

/// Connection-level limits of the gRPC server.
#[derive(Debug, Clone)]
pub struct GrpcConfig {
    /// Upper bound for one call, including waiting for the run's lock.
    pub request_timeout: Duration,
    /// Calls running at once on one connection.
    pub concurrency_per_connection: usize,
    /// HTTP/2 streams a client may open at once.
    pub max_concurrent_streams: u32,
    /// Ping interval that detects dead peers behind a NAT or a silent load balancer.
    pub keepalive_interval: Duration,
    /// How long to wait for the ping answer before dropping the connection.
    pub keepalive_timeout: Duration,
}

impl Default for GrpcConfig {
    fn default() -> Self {
        Self {
            request_timeout: Duration::from_secs(30),
            concurrency_per_connection: 64,
            max_concurrent_streams: 128,
            keepalive_interval: Duration::from_secs(30),
            keepalive_timeout: Duration::from_secs(10),
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
    Server::builder()
        .timeout(config.request_timeout)
        .concurrency_limit_per_connection(config.concurrency_per_connection)
        .max_concurrent_streams(config.max_concurrent_streams)
        .http2_keepalive_interval(Some(config.keepalive_interval))
        .http2_keepalive_timeout(Some(config.keepalive_timeout))
        .add_service(service)
        .serve_with_incoming_shutdown(TcpListenerStream::new(listener), shutdown)
        .await
}
