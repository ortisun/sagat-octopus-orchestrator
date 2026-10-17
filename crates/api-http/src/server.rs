//! The HTTP server loop, with the connection-level protections `axum::serve` does not provide.
//!
//! * `header_read_timeout`: a client that opens a connection and dribbles (or never sends) its
//!   request head is disconnected. It also bounds idle keep-alive connections, because the timer
//!   restarts for every request head. Without a timer on the connection builder hyper silently
//!   drops this protection (Slowloris), which is why the loop is not `axum::serve`.
//! * `write_stall_timeout`: a client that takes no byte of a response for this long is
//!   disconnected. Without it a client that asks for a large body and never reads holds the
//!   connection, and everything the response keeps alive, for as long as it likes. A reader that
//!   makes progress is allowed to continue unless its response opts into an absolute deadline.
//! * `max_connections`: when the cap is reached the server stops accepting; new connections wait in
//!   the kernel backlog instead of exhausting file descriptors.
//! * `shutdown_grace`: on shutdown, in-flight requests may finish, but only for this long; a
//!   connection that never completes cannot keep the process alive forever.

use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use axum::Router;
use hyper::server::conn::http1;
use hyper_util::rt::{TokioIo, TokioTimer};
use hyper_util::service::TowerToHyperService;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpListener;
use tokio::sync::{OwnedSemaphorePermit, Semaphore, watch};
use tokio::time::{Instant, Sleep};

/// Connection-level limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServerConfig {
    /// Longest a client may take to send a request head (and the longest an idle keep-alive
    /// connection may wait for the next one).
    pub header_read_timeout: Duration,
    /// Maximum simultaneous connections.
    pub max_connections: usize,
    /// Longest a write may wait for the client to take bytes before the connection is closed.
    pub write_stall_timeout: Duration,
    /// How long shutdown waits for in-flight requests before dropping the remaining connections.
    pub shutdown_grace: Duration,
}

impl ServerConfig {
    /// Check timer and semaphore bounds before a listener starts serving.
    ///
    /// # Errors
    /// A static reason for a zero/overlarge cap or timeout.
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.max_connections == 0
            || self.max_connections > Semaphore::MAX_PERMITS.min(u32::MAX as usize)
        {
            return Err("max_connections must fit a nonzero semaphore and shutdown permit count");
        }
        for timeout in [
            self.header_read_timeout,
            self.write_stall_timeout,
            self.shutdown_grace,
        ] {
            if timeout.is_zero() || timeout > Duration::from_secs(86400) {
                return Err(
                    "HTTP connection timeouts must be greater than zero and at most one day",
                );
            }
        }
        Ok(())
    }
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            header_read_timeout: Duration::from_secs(10),
            max_connections: 1024,
            write_stall_timeout: Duration::from_secs(30),
            shutdown_grace: Duration::from_secs(30),
        }
    }
}

/// Serves `app` on `listener` until `shutdown` completes, then drains.
///
/// # Errors
/// Returns an error only for an unusable configuration; transient `accept` failures are logged
/// and retried.
pub async fn serve(
    listener: TcpListener,
    app: Router,
    config: ServerConfig,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> io::Result<()> {
    serve_inner(listener, app, config, None, shutdown).await
}

/// Serves HTTP over native TLS with verified, optional client identities.
///
/// # Errors
/// Returns an error for an unusable server configuration.
pub async fn serve_tls(
    listener: TcpListener,
    app: Router,
    config: ServerConfig,
    tls: HttpTlsConfig,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> io::Result<()> {
    serve_inner(listener, app, config, Some(tls), shutdown).await
}

async fn serve_inner(
    listener: TcpListener,
    app: Router,
    config: ServerConfig,
    tls: Option<HttpTlsConfig>,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> io::Result<()> {
    config
        .validate()
        .map_err(|reason| io::Error::new(io::ErrorKind::InvalidInput, reason))?;
    let permits = Arc::new(Semaphore::new(config.max_connections));
    let mut connections = tokio::task::JoinSet::new();
    let (stop_tx, stop_rx) = watch::channel(false);
    tokio::pin!(shutdown);

    loop {
        while connections.try_join_next().is_some() {}
        // Take a slot first: at the cap we simply stop accepting.
        let permit = tokio::select! {
            permit = Arc::clone(&permits).acquire_owned() => match permit {
                Ok(permit) => permit,
                Err(_) => break, // the semaphore is never closed; defensive
            },
            () = &mut shutdown => break,
        };
        let stream = tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok((stream, _peer)) => stream,
                Err(error) => {
                    tracing::warn!(%error, "accept failed; retrying");
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    continue;
                }
            },
            () = &mut shutdown => break,
        };
        // Small responses should not wait for Nagle; failure to set it is harmless.
        let _ = stream.set_nodelay(true);
        if let Some(tls) = tls.clone() {
            let app = app.clone();
            let mut stop = stop_rx.clone();
            connections.spawn(async move {
                let acceptor = tokio_rustls::TlsAcceptor::from(tls.server);
                let accepted = tokio::select! {
                    result = tokio::time::timeout(config.header_read_timeout, acceptor.accept(stream)) => result,
                    _ = stop.changed() => return,
                };
                let Ok(Ok(stream)) = accepted else {
                    metrics::counter!("http_tls_handshakes_total", "outcome" => "refused")
                        .increment(1);
                    return;
                };
                metrics::counter!("http_tls_handshakes_total", "outcome" => "accepted")
                    .increment(1);
                let app = match stream.get_ref().1.peer_certificates() {
                    Some(chain) if !chain.is_empty() => {
                        let identity = ClientIdentity {
                            chain: chain.to_vec(),
                            verifier: tls.client_verifier,
                        };
                        app.layer(axum::middleware::from_fn(
                            move |mut request: axum::extract::Request,
                                  next: axum::middleware::Next| {
                                let identity = identity.clone();
                                async move {
                                    // Recheck validity on each keep-alive request, including certificate expiry.
                                    // An expired identity is absent: profile admission then emits
                                    // the normal typed denial, audit and request telemetry.
                                    if let Some(certificate) = identity.verified() {
                                        request.extensions_mut().insert(certificate);
                                    }
                                    next.run(request).await
                                }
                            },
                        ))
                    }
                    _ => app,
                };
                serve_connection(
                    TokioIo::new(StallGuard::new(stream, config.write_stall_timeout)),
                    app,
                    config.header_read_timeout,
                    stop,
                    permit,
                )
                .await;
            });
        } else {
            connections.spawn(serve_connection(
                TokioIo::new(StallGuard::new(stream, config.write_stall_timeout)),
                app.clone(),
                config.header_read_timeout,
                stop_rx.clone(),
                permit,
            ));
        }
    }

    // Stop: let every connection finish its current request, then close.
    let _ = stop_tx.send(true);
    let all = u32::try_from(config.max_connections).unwrap_or(u32::MAX);
    match tokio::time::timeout(config.shutdown_grace, permits.acquire_many(all)).await {
        Ok(_) => {}
        Err(_) => tracing::warn!(
            grace_secs = config.shutdown_grace.as_secs(),
            "shutdown grace period elapsed; dropping the remaining connections"
        ),
    }
    // Dropping a JoinHandle detaches its task. Explicitly cancel and drain the owned tasks so
    // sockets, request bodies and handler resources are released before this listener returns.
    connections.shutdown().await;
    Ok(())
}

async fn serve_connection<S: AsyncRead + AsyncWrite + Unpin + Send + 'static>(
    io: TokioIo<StallGuard<S>>,
    app: Router,
    header_read_timeout: Duration,
    mut stop: watch::Receiver<bool>,
    _permit: OwnedSemaphorePermit,
) {
    let mut builder = http1::Builder::new();
    builder
        .timer(TokioTimer::new())
        .header_read_timeout(header_read_timeout)
        .keep_alive(true);
    let (abort, mut aborted) = watch::channel(false);
    let app = app.layer(axum::Extension(ConnectionAbort(abort)));
    let connection = builder.serve_connection(io, TowerToHyperService::new(app));
    tokio::pin!(connection);

    let mut stopping = false;
    loop {
        tokio::select! {
            result = connection.as_mut() => {
                if let Err(error) = result {
                    tracing::debug!(%error, "connection closed with an error");
                }
                return;
            }
            _ = aborted.changed() => {
                metrics::counter!("http_response_deadlines_total").increment(1);
                return;
            }
            _ = stop.changed(), if !stopping => {
                stopping = true;
                connection.as_mut().graceful_shutdown();
            }
        }
    }
}

/// Available only from the native HTTP server. A bounded response owns the deadline guard in
/// its Bytes owner so the timer covers Hyper's buffers too, independently of body polling.
#[derive(Clone)]
pub(crate) struct ConnectionAbort(watch::Sender<bool>);
pub(crate) struct DeliveryDeadline(tokio::task::JoinHandle<()>);
impl ConnectionAbort {
    pub(crate) fn deadline(&self, deadline: Instant) -> DeliveryDeadline {
        let abort = self.0.clone();
        DeliveryDeadline(tokio::spawn(async move {
            tokio::time::sleep_until(deadline).await;
            let _ = abort.send(true);
        }))
    }
}
impl Drop for DeliveryDeadline {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// A connection whose writes are given up on when the client takes nothing for `timeout`: a write
/// that cannot make progress starts a timer, any progress stops it, and when it runs out the write
/// fails with `TimedOut`, which closes the connection and everything its response held.
struct StallGuard<S> {
    inner: S,
    timeout: Duration,
    stalled: Pin<Box<Sleep>>,
    waiting: bool,
}

impl<S> StallGuard<S> {
    fn new(inner: S, timeout: Duration) -> Self {
        Self {
            inner,
            timeout,
            stalled: Box::pin(tokio::time::sleep(timeout)),
            waiting: false,
        }
    }

    /// What a write that could not proceed does: arm the timer if it is not, and fail once it ran out.
    fn stall<T>(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<T>> {
        if !self.waiting {
            self.stalled.as_mut().reset(Instant::now() + self.timeout);
            self.waiting = true;
        }
        if self.stalled.as_mut().poll(cx).is_ready() {
            self.waiting = false;
            metrics::counter!("http_write_stalls_total").increment(1);
            tracing::warn!(
                timeout_secs = self.timeout.as_secs(),
                "a client took no bytes of a response in time; the connection is closed"
            );
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "the client took no bytes of the response in time",
            )));
        }
        Poll::Pending
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for StallGuard<S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for StallGuard<S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        match Pin::new(&mut self.inner).poll_write(cx, buf) {
            Poll::Pending => self.stall(cx),
            ready => {
                self.waiting = false;
                ready
            }
        }
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match Pin::new(&mut self.inner).poll_flush(cx) {
            Poll::Pending => self.stall(cx),
            ready => {
                self.waiting = false;
                ready
            }
        }
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }

    fn poll_write_vectored(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        match Pin::new(&mut self.inner).poll_write_vectored(cx, bufs) {
            Poll::Pending => self.stall(cx),
            ready => {
                self.waiting = false;
                ready
            }
        }
    }

    fn is_write_vectored(&self) -> bool {
        self.inner.is_write_vectored()
    }
}

/// Identity inserted by the native TLS listener after certificate validation.
#[derive(Clone)]
pub(crate) struct VerifiedClientCertificate {
    pub(crate) sha256: String,
}

/// Native HTTP TLS configuration. Private keys are deliberately not printable.
#[derive(Clone)]
pub struct HttpTlsConfig {
    server: Arc<rustls::ServerConfig>,
    client_verifier: Arc<dyn rustls::server::danger::ClientCertVerifier>,
}

impl std::fmt::Debug for HttpTlsConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("HttpTlsConfig([redacted])")
    }
}

impl HttpTlsConfig {
    /// Build native TLS with an optional client certificate, checked against these roots.
    /// Profile-specific handlers decide whether a verified client identity is mandatory.
    ///
    /// # Errors
    /// A static error without key or certificate bytes if configuration is invalid.
    pub fn from_pem(
        certificate: &[u8],
        private_key: &[u8],
        client_roots: &[u8],
    ) -> Result<Self, &'static str> {
        use rustls::pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject};
        crate::auth::install_crypto_provider();
        if [certificate.len(), private_key.len(), client_roots.len()]
            .into_iter()
            .any(|size| size == 0 || size > 256 * 1024)
        {
            return Err("HTTP TLS PEM files must be non-empty and at most 256 KiB");
        }
        let chain = CertificateDer::pem_slice_iter(certificate)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| "invalid HTTP TLS certificate")?;
        let key = PrivateKeyDer::from_pem_slice(private_key)
            .map_err(|_| "invalid HTTP TLS private key")?;
        let mut roots = rustls::RootCertStore::empty();
        for root in CertificateDer::pem_slice_iter(client_roots) {
            roots
                .add(root.map_err(|_| "invalid HTTP TLS client roots")?)
                .map_err(|_| "invalid HTTP TLS client roots")?;
        }
        let client_verifier = rustls::server::WebPkiClientVerifier::builder(Arc::new(roots))
            .allow_unauthenticated()
            .build()
            .map_err(|_| "invalid HTTP TLS client roots")?;
        let mut server = rustls::ServerConfig::builder()
            .with_client_cert_verifier(client_verifier.clone())
            .with_single_cert(chain, key)
            .map_err(|_| "invalid HTTP TLS certificate and key")?;
        server.alpn_protocols = vec![b"http/1.1".to_vec()];
        // Resumed sessions must not retain a client identity across certificate replacement.
        server.session_storage = Arc::new(rustls::server::NoServerSessionStorage {});
        server.send_tls13_tickets = 0;
        Ok(Self {
            server: Arc::new(server),
            client_verifier,
        })
    }
}

#[derive(Clone)]
struct ClientIdentity {
    chain: Vec<rustls::pki_types::CertificateDer<'static>>,
    verifier: Arc<dyn rustls::server::danger::ClientCertVerifier>,
}

impl ClientIdentity {
    fn verified(&self) -> Option<VerifiedClientCertificate> {
        use sha2::Digest as _;
        let (leaf, intermediates) = self.chain.split_first()?;
        self.verifier
            .verify_client_cert(leaf, intermediates, rustls::pki_types::UnixTime::now())
            .ok()?;
        Some(VerifiedClientCertificate {
            sha256: sha2::Sha256::digest(leaf.as_ref())
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect(),
        })
    }
}

#[cfg(test)]
#[path = "server_tls_tests.rs"]
mod tls_tests;

#[cfg(test)]
#[path = "server_deadline_tests.rs"]
mod deadline_tests;
