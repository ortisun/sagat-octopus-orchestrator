//! Capacity limits of the worker service as a whole: how many connections it accepts and how many
//! calls it runs at once.
//!
//! Budgets per tenant and per identity belong to the bearer pipeline the calls go through; these
//! protect the process from everybody together, and they act *before* anything is authenticated, so
//! a flood of calls with garbage tokens costs a counter increment and not a database lookup.
//!
//! When the engine is full it sheds, it does not queue: a queued heartbeat is a heartbeat that
//! arrives after its lease expired, which turns overload into retries and more overload. Calls that
//! find new work (`RegisterWorker`, `PollTasks`) are shed first, so the calls that keep work already
//! leased alive (`Heartbeat`, `CompleteStep`, `FailStep`) keep a reserve.

use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::{Context, Poll};
use std::time::Duration;

use sagat_api_http::error::ApiError;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio::time::Sleep;
use tokio_stream::Stream;
use tonic::codegen::{Service, http};
use tonic::transport::server::{Connected, TcpConnectInfo};
use tower_layer::Layer;

use crate::status::to_status;

/// How long to wait before accepting again after the listener failed (no file descriptors left, ...).
const ACCEPT_BACKOFF: Duration = Duration::from_millis(100);
/// Suggested wait of a caller that was shed.
const SHED_RETRY: Duration = Duration::from_secs(1);

// ------------------------------------------------------------------------------ connections

/// The connections of the listener, at most `max` open at a time. The next connection is not
/// accepted until one closes, so the kernel's backlog, not the process's memory, absorbs a flood.
pub(crate) struct Incoming {
    listener: TcpListener,
    permits: Arc<Semaphore>,
    acquiring: Option<Pin<Box<dyn Future<Output = Option<OwnedSemaphorePermit>> + Send>>>,
    permit: Option<OwnedSemaphorePermit>,
    backoff: Option<Pin<Box<Sleep>>>,
}

impl Incoming {
    pub(crate) fn new(listener: TcpListener, max: usize) -> Self {
        Self {
            listener,
            permits: Arc::new(Semaphore::new(max.max(1))),
            acquiring: None,
            permit: None,
            backoff: None,
        }
    }
}

impl Stream for Incoming {
    type Item = io::Result<Capped>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = &mut *self;
        if let Some(backoff) = this.backoff.as_mut() {
            if backoff.as_mut().poll(cx).is_pending() {
                return Poll::Pending;
            }
            this.backoff = None;
        }
        if this.permit.is_none() {
            let permits = Arc::clone(&this.permits);
            let acquiring = this
                .acquiring
                .get_or_insert_with(|| Box::pin(async move { permits.acquire_owned().await.ok() }));
            match acquiring.as_mut().poll(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(None) => return Poll::Ready(None),
                Poll::Ready(Some(permit)) => {
                    this.acquiring = None;
                    this.permit = Some(permit);
                }
            }
        }
        match this.listener.poll_accept(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Ok((socket, _))) => match this.permit.take() {
                Some(permit) => {
                    let _ = socket.set_nodelay(true);
                    metrics::counter!("grpc_connections_total").increment(1);
                    Poll::Ready(Some(Ok(Capped {
                        socket,
                        _permit: permit,
                    })))
                }
                // Cannot happen (the permit was acquired above); an error is safer than a hang.
                None => Poll::Ready(Some(Err(io::Error::other("no connection slot")))),
            },
            Poll::Ready(Err(error)) => {
                tracing::warn!(%error, "accepting a connection failed; backing off");
                this.backoff = Some(Box::pin(tokio::time::sleep(ACCEPT_BACKOFF)));
                cx.waker().wake_by_ref();
                Poll::Pending
            }
        }
    }
}

/// A connection that gives its place back when it closes.
pub(crate) struct Capped {
    socket: TcpStream,
    _permit: OwnedSemaphorePermit,
}

impl Connected for Capped {
    type ConnectInfo = TcpConnectInfo;

    fn connect_info(&self) -> Self::ConnectInfo {
        self.socket.connect_info()
    }
}

impl AsyncRead for Capped {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.socket).poll_read(cx, buf)
    }
}

impl AsyncWrite for Capped {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.socket).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.socket).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.socket).poll_shutdown(cx)
    }

    fn poll_write_vectored(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.socket).poll_write_vectored(cx, bufs)
    }

    fn is_write_vectored(&self) -> bool {
        self.socket.is_write_vectored()
    }
}

// ------------------------------------------------------------------------------ calls

/// What a call is for, which decides how early it is shed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Class {
    /// Finds new work: shed first.
    Discovery,
    /// Keeps leased work alive or reports it: shed only when the engine is completely full.
    Lease,
}

impl Class {
    fn of(path: &str) -> Self {
        match path.rsplit('/').next() {
            Some("Heartbeat" | "CompleteStep" | "FailStep" | "ResolveSecrets") => Self::Lease,
            // Registering and polling, and whatever a future method does until it is classified.
            _ => Self::Discovery,
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Discovery => "discovery",
            Self::Lease => "lease",
        }
    }
}

struct Shared {
    in_flight: AtomicUsize,
    /// Calls running at once, of any class.
    max: usize,
    /// Calls running at once above which discovery calls are shed.
    discovery_max: usize,
}

impl Shared {
    fn admit(self: &Arc<Self>, class: Class) -> Option<Ticket> {
        let limit = match class {
            Class::Discovery => self.discovery_max,
            Class::Lease => self.max,
        };
        let running = self.in_flight.fetch_add(1, Ordering::AcqRel);
        if running >= limit {
            self.in_flight.fetch_sub(1, Ordering::AcqRel);
            return None;
        }
        #[allow(clippy::cast_precision_loss)]
        metrics::gauge!("grpc_in_flight").set((running + 1) as f64);
        Some(Ticket {
            shared: Arc::clone(self),
        })
    }
}

/// A place among the calls running; released when the call ends, however it ends.
struct Ticket {
    shared: Arc<Shared>,
}

impl Drop for Ticket {
    fn drop(&mut self) {
        let before = self.shared.in_flight.fetch_sub(1, Ordering::AcqRel);
        #[allow(clippy::cast_precision_loss)]
        metrics::gauge!("grpc_in_flight").set(before.saturating_sub(1) as f64);
    }
}

/// Limits the calls running at once across all connections. Cloning shares the counter.
#[derive(Clone)]
pub(crate) struct Admission {
    shared: Arc<Shared>,
}

impl Admission {
    /// `max` calls at once; discovery calls are shed above three quarters of it, so a reserve is
    /// left for the calls that keep leased work alive.
    pub(crate) fn new(max: usize) -> Self {
        let max = max.max(1);
        Self {
            shared: Arc::new(Shared {
                in_flight: AtomicUsize::new(0),
                max,
                discovery_max: (max * 3 / 4).max(1),
            }),
        }
    }
}

impl<S> Layer<S> for Admission {
    type Service = Admit<S>;

    fn layer(&self, inner: S) -> Admit<S> {
        Admit {
            inner,
            shared: Arc::clone(&self.shared),
        }
    }
}

#[derive(Clone)]
pub(crate) struct Admit<S> {
    inner: S,
    shared: Arc<Shared>,
}

impl<S, B, R> Service<http::Request<B>> for Admit<S>
where
    S: Service<http::Request<B>, Response = http::Response<R>> + Send + 'static,
    S::Future: Send + 'static,
    S::Error: Send + 'static,
    R: Default + Send + 'static,
{
    type Response = http::Response<R>;
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, request: http::Request<B>) -> Self::Future {
        let class = Class::of(request.uri().path());
        let Some(ticket) = self.shared.admit(class) else {
            metrics::counter!("grpc_shed_total", "class" => class.label()).increment(1);
            let shed = to_status(&ApiError::rate_limited(SHED_RETRY)).into_http();
            return Box::pin(std::future::ready(Ok(shed)));
        };
        let call = self.inner.call(request);
        Box::pin(async move {
            let response = call.await;
            drop(ticket);
            response
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_calls_that_keep_leased_work_alive_are_reserved_capacity() {
        let class =
            |m: &str| Class::of(&format!("/sagat_octopus_orchestrator.v1.WorkerService/{m}"));
        assert_eq!(class("PollTasks"), Class::Discovery);
        assert_eq!(class("RegisterWorker"), Class::Discovery);
        assert_eq!(class("Heartbeat"), Class::Lease);
        assert_eq!(class("CompleteStep"), Class::Lease);
        assert_eq!(class("FailStep"), Class::Lease);
        assert_eq!(
            class("ResolveSecrets"),
            Class::Lease,
            "a step that is running is waiting for its secrets"
        );
        assert_eq!(
            class("SomethingNew"),
            Class::Discovery,
            "unknown is shed first"
        );
    }

    #[test]
    fn a_reserve_is_left_for_leased_work_and_every_ticket_is_given_back() {
        let shared = Admission::new(8).shared;
        let tickets: Vec<_> = (0..6)
            .map(|_| {
                shared
                    .admit(Class::Discovery)
                    .expect("below the discovery limit")
            })
            .collect();
        assert!(
            shared.admit(Class::Discovery).is_none(),
            "discovery is shed at 6 of 8"
        );
        let reserve: Vec<_> = (0..2)
            .map(|_| shared.admit(Class::Lease).expect("the reserve"))
            .collect();
        assert!(shared.admit(Class::Lease).is_none(), "full is full");
        assert_eq!(shared.in_flight.load(Ordering::Acquire), 8);
        drop(tickets);
        drop(reserve);
        assert_eq!(shared.in_flight.load(Ordering::Acquire), 0, "nothing leaks");
        assert!(shared.admit(Class::Discovery).is_some());
    }
}
