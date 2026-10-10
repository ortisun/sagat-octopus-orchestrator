//! The HTTP server loop, with the connection-level protections `axum::serve` does not provide.
//!
//! * `header_read_timeout`: a client that opens a connection and dribbles (or never sends) its
//!   request head is disconnected. It also bounds idle keep-alive connections, because the timer
//!   restarts for every request head. Without a timer on the connection builder hyper silently
//!   drops this protection (Slowloris), which is why the loop is not `axum::serve`.
//! * `write_stall_timeout`: a client that takes no byte of a response for this long is
//!   disconnected. Without it a client that asks for a large body and never reads holds the
//!   connection, and everything the response keeps alive, for as long as it likes. A reader that
//!   makes progress, however slowly, is never cut.
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
    if config.max_connections == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "max_connections must be at least 1",
        ));
    }
    let permits = Arc::new(Semaphore::new(config.max_connections));
    let (stop_tx, stop_rx) = watch::channel(false);
    tokio::pin!(shutdown);

    loop {
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
        tokio::spawn(serve_connection(
            TokioIo::new(StallGuard::new(stream, config.write_stall_timeout)),
            app.clone(),
            config.header_read_timeout,
            stop_rx.clone(),
            permit,
        ));
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
    Ok(())
}

async fn serve_connection(
    io: TokioIo<StallGuard<tokio::net::TcpStream>>,
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
            _ = stop.changed(), if !stopping => {
                stopping = true;
                connection.as_mut().graceful_shutdown();
            }
        }
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
