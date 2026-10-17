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

use std::collections::HashMap;
use std::future::Future;
use std::io;
use std::net::{IpAddr, Ipv6Addr};
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::task::Waker;
use std::task::{Context, Poll};
use std::time::Duration;

use sagat_api_http::error::ApiError;
use sagat_domain::identity::TenantId;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio::time::{Instant, Sleep};
use tokio_stream::Stream;
use tonic::codegen::{Service, http};
use tonic::transport::server::Connected;
use tower_layer::Layer;

use crate::status::to_status;

/// How long to wait before accepting again after the listener failed (no file descriptors left, ...).
const ACCEPT_BACKOFF: Duration = Duration::from_millis(100);
/// Suggested wait of a caller that was shed.
const SHED_RETRY: Duration = Duration::from_secs(1);

// ------------------------------------------------------------------------------ task streams

/// Dedicated response-body limits for finite task streams. They are independent from request
/// admission because tonic returns the streaming response headers before the body ends.
#[derive(Debug)]
pub(crate) struct TaskStreams {
    open: Mutex<TaskStreamCounts>,
}

#[derive(Debug, Default)]
struct TaskStreamCounts {
    process: usize,
    tenants: HashMap<TenantId, usize>,
    subjects: HashMap<(TenantId, String), usize>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum StreamRefused {
    Process,
    Tenant,
    Subject,
}

impl StreamRefused {
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Process => "process",
            Self::Tenant => "tenant",
            Self::Subject => "subject",
        }
    }
}

#[derive(Debug)]
pub(crate) struct TaskStreamTicket {
    owner: Arc<TaskStreams>,
    tenant: TenantId,
    subject: String,
}

impl TaskStreams {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            open: Mutex::new(TaskStreamCounts::default()),
        })
    }

    pub(crate) fn admit(
        self: &Arc<Self>,
        tenant: TenantId,
        subject: &str,
    ) -> Result<TaskStreamTicket, StreamRefused> {
        let mut open = self.open.lock().unwrap_or_else(PoisonError::into_inner);
        let key = (tenant, subject.to_owned());
        if open.process >= 32 {
            return Err(StreamRefused::Process);
        }
        if open.subjects.get(&key).copied().unwrap_or(0) >= 2 {
            return Err(StreamRefused::Subject);
        }
        if open.tenants.get(&tenant).copied().unwrap_or(0) >= 8 {
            return Err(StreamRefused::Tenant);
        }
        open.process += 1;
        *open.tenants.entry(tenant).or_insert(0) += 1;
        *open.subjects.entry(key).or_insert(0) += 1;
        metrics::gauge!("worker_task_streams_open").set(open.process as f64);
        Ok(TaskStreamTicket {
            owner: Arc::clone(self),
            tenant,
            subject: subject.to_owned(),
        })
    }
}

impl Drop for TaskStreamTicket {
    fn drop(&mut self) {
        let mut open = self
            .owner
            .open
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        open.process = open.process.saturating_sub(1);
        let key = (self.tenant, self.subject.clone());
        if let Some(count) = open.tenants.get_mut(&self.tenant) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                open.tenants.remove(&self.tenant);
            }
        }
        if let Some(count) = open.subjects.get_mut(&key) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                open.subjects.remove(&key);
            }
        }
        metrics::gauge!("worker_task_streams_open").set(open.process as f64);
    }
}

// ------------------------------------------------------------------------------ connections

/// What the listener enforces on every connection before tonic sees it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ConnectionLimits {
    /// Connections open at once.
    pub max: usize,
    /// Connections open at once from one peer address (an IPv6 peer counts as its /64).
    pub per_peer: usize,
    /// How long a new connection may stay silent before it is closed.
    pub first_byte: Duration,
    /// How long an established connection may stay silent before it is closed.
    pub idle: Duration,
}

/// How many connections each peer holds, so one address cannot fill the places of everybody.
#[derive(Default)]
struct Peers {
    counts: Mutex<HashMap<IpAddr, usize>>,
}

impl Peers {
    /// A place for one more connection of `ip`, unless it already holds `cap`.
    fn admit(self: &Arc<Self>, ip: IpAddr, cap: usize) -> Option<PeerSlot> {
        let ip = peer_key(ip);
        let mut counts = self.counts.lock().unwrap_or_else(PoisonError::into_inner);
        let held = counts.entry(ip).or_insert(0);
        if *held >= cap {
            return None;
        }
        *held += 1;
        Some(PeerSlot {
            peers: Arc::clone(self),
            ip,
        })
    }
}

/// One connection's place in its peer's count; given back when the connection closes.
struct PeerSlot {
    peers: Arc<Peers>,
    ip: IpAddr,
}

impl Drop for PeerSlot {
    fn drop(&mut self) {
        let mut counts = self
            .peers
            .counts
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(held) = counts.get_mut(&self.ip) {
            *held = held.saturating_sub(1);
            if *held == 0 {
                counts.remove(&self.ip);
            }
        }
    }
}

/// The address a connection is counted under: an IPv4-mapped IPv6 address is the IPv4 address, and
/// any other IPv6 address stands for its /64 (one subscriber is routinely given a whole one).
fn peer_key(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V4(_) => ip,
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or_else(
            || {
                let [a, b, c, d, ..] = v6.segments();
                IpAddr::V6(Ipv6Addr::new(a, b, c, d, 0, 0, 0, 0))
            },
            IpAddr::V4,
        ),
    }
}

/// The connections of the listener, at most `max` open at a time. The next connection is not
/// accepted until one closes, so the kernel's backlog, not the process's memory, absorbs a flood.
/// A connection that says nothing is closed (see [`ConnectionLimits`]), so a client that opens
/// sockets and never speaks cannot hold the places for good.
pub(crate) struct Incoming {
    listener: TcpListener,
    limits: ConnectionLimits,
    peers: Arc<Peers>,
    permits: Arc<Semaphore>,
    acquiring: Option<Pin<Box<dyn Future<Output = Option<OwnedSemaphorePermit>> + Send>>>,
    permit: Option<OwnedSemaphorePermit>,
    backoff: Option<Pin<Box<Sleep>>>,
}

impl Incoming {
    pub(crate) fn new(listener: TcpListener, limits: ConnectionLimits) -> Self {
        Self {
            listener,
            limits,
            peers: Arc::new(Peers::default()),
            permits: Arc::new(Semaphore::new(limits.max.max(1))),
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
            Poll::Ready(Ok((socket, address))) => {
                // A peer at its share is turned away without taking a place from anybody else.
                let Some(slot) = this.peers.admit(address.ip(), this.limits.per_peer) else {
                    metrics::counter!("grpc_connections_refused_total", "reason" => "per_peer")
                        .increment(1);
                    drop(socket);
                    cx.waker().wake_by_ref();
                    return Poll::Pending;
                };
                match this.permit.take() {
                    Some(permit) => {
                        let _ = socket.set_nodelay(true);
                        metrics::counter!("grpc_connections_total").increment(1);
                        Poll::Ready(Some(Ok(Capped {
                            socket,
                            _permit: permit,
                            _slot: slot,
                            watchdog: Arc::new(ConnectionWatchdog::default()),
                            deadline: Box::pin(tokio::time::sleep(this.limits.first_byte)),
                            idle: this.limits.idle,
                            heard: false,
                        })))
                    }
                    // Cannot happen (the permit was acquired above); an error is safer than a hang.
                    None => Poll::Ready(Some(Err(io::Error::other("no connection slot")))),
                }
            }
            Poll::Ready(Err(error)) => {
                tracing::warn!(%error, "accepting a connection failed; backing off");
                this.backoff = Some(Box::pin(tokio::time::sleep(ACCEPT_BACKOFF)));
                cx.waker().wake_by_ref();
                Poll::Pending
            }
        }
    }
}

/// A connection that gives its places back when it closes, and that closes itself when its peer
/// stays silent for too long: first for [`ConnectionLimits::first_byte`], afterwards for
/// [`ConnectionLimits::idle`] after the last byte it read.
pub(crate) struct Capped {
    socket: TcpStream,
    _permit: OwnedSemaphorePermit,
    _slot: PeerSlot,
    watchdog: Arc<ConnectionWatchdog>,
    deadline: Pin<Box<Sleep>>,
    idle: Duration,
    heard: bool,
}

/// Per-connection escape hatch used by finite response streams. HTTP/2 stops polling a response
/// body while its peer withholds flow-control credit, so a timer inside the body cannot tear down a
/// stalled send. The task-stream deadline trips this handle instead; waking the connection's I/O
/// future makes hyper drop every body on that connection and release their lifetime tickets.
#[derive(Clone, Debug)]
pub(crate) struct CappedConnectInfo {
    watchdog: Arc<ConnectionWatchdog>,
}

#[derive(Debug, Default)]
struct ConnectionWatchdog {
    tripped: AtomicUsize,
    waker: Mutex<Option<Waker>>,
}

impl ConnectionWatchdog {
    fn poll(&self, cx: &Context<'_>) -> bool {
        if self.tripped.load(Ordering::Acquire) != 0 {
            return true;
        }
        let mut waker = self.waker.lock().unwrap_or_else(PoisonError::into_inner);
        if !waker
            .as_ref()
            .is_some_and(|stored| stored.will_wake(cx.waker()))
        {
            *waker = Some(cx.waker().clone());
        }
        self.tripped.load(Ordering::Acquire) != 0
    }

    fn trip(&self) {
        if self.tripped.swap(1, Ordering::AcqRel) == 0
            && let Some(waker) = self
                .waker
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take()
        {
            waker.wake();
        }
    }
}

impl CappedConnectInfo {
    /// Closes the whole HTTP/2 connection at `deadline` unless the returned guard is dropped first.
    /// Connection-wide cancellation is deliberate: h2 exposes no per-stream reset handle to a
    /// tonic response body once flow control has stopped polling that body.
    pub(crate) fn arm(&self, deadline: Instant) -> ConnectionDeadline {
        let watchdog = Arc::clone(&self.watchdog);
        ConnectionDeadline {
            task: tokio::spawn(async move {
                tokio::time::sleep_until(deadline).await;
                metrics::counter!(
                    "grpc_connections_closed_total",
                    "reason" => "task_stream_deadline"
                )
                .increment(1);
                watchdog.trip();
            }),
        }
    }
}

/// Cancels a pending connection deadline when its response body completed normally.
pub(crate) struct ConnectionDeadline {
    task: tokio::task::JoinHandle<()>,
}

impl Drop for ConnectionDeadline {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Connected for Capped {
    type ConnectInfo = CappedConnectInfo;

    fn connect_info(&self) -> Self::ConnectInfo {
        CappedConnectInfo {
            watchdog: Arc::clone(&self.watchdog),
        }
    }
}

impl AsyncRead for Capped {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = &mut *self;
        if this.watchdog.poll(cx) {
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "a task stream exceeded its receive window",
            )));
        }
        if this.deadline.as_mut().poll(cx).is_ready() {
            let reason = if this.heard { "idle" } else { "silent" };
            metrics::counter!("grpc_connections_closed_total", "reason" => reason).increment(1);
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "the peer was silent for too long",
            )));
        }
        let before = buf.filled().len();
        let polled = Pin::new(&mut this.socket).poll_read(cx, buf);
        if matches!(polled, Poll::Ready(Ok(()))) && buf.filled().len() > before {
            this.heard = true;
            this.deadline.as_mut().reset(Instant::now() + this.idle);
        }
        polled
    }
}

impl AsyncWrite for Capped {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        if self.watchdog.poll(cx) {
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "a task stream exceeded its receive window",
            )));
        }
        Pin::new(&mut self.socket).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        if self.watchdog.poll(cx) {
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "a task stream exceeded its receive window",
            )));
        }
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
        if self.watchdog.poll(cx) {
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "a task stream exceeded its receive window",
            )));
        }
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
    /// Moves a blob for a step that is running: shed like a lease call, but it owns its time limit
    /// (a transfer legitimately outlasts a unary call) and its own cap on how many run at once.
    Transfer,
}

impl Class {
    fn of(path: &str) -> Self {
        match path.rsplit('/').next() {
            Some(
                "Heartbeat" | "CompleteStep" | "FailStep" | "ResolveSecrets" | "ResolveDatabase",
            ) => Self::Lease,
            Some("WriteBlob" | "ReadBlob") => Self::Transfer,
            // Registering and polling, and whatever a future method does until it is classified.
            _ => Self::Discovery,
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Discovery => "discovery",
            Self::Lease => "lease",
            Self::Transfer => "transfer",
        }
    }
}

struct Shared {
    in_flight: AtomicUsize,
    /// Calls running at once, of any class.
    max: AtomicUsize,
    /// Calls running at once above which discovery calls are shed.
    discovery_max: AtomicUsize,
    /// The longest a call other than a blob transfer may take. The transport's own timeout has to
    /// allow the longest transfer, so this is where the shorter limit of everything else is kept.
    unary_timeout_ms: AtomicU64,
}

impl Shared {
    fn admit(self: &Arc<Self>, class: Class) -> Option<Ticket> {
        let limit = match class {
            Class::Discovery => self.discovery_max.load(Ordering::Acquire),
            Class::Lease | Class::Transfer => self.max.load(Ordering::Acquire),
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
pub(crate) struct Ticket {
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
#[doc(hidden)]
pub struct Admission {
    shared: Arc<Shared>,
}

impl Admission {
    /// `max` calls at once; discovery calls are shed above three quarters of it, so a reserve is
    /// left for the calls that keep leased work alive.
    pub(crate) fn new(max: usize, unary_timeout: Duration) -> Self {
        let max = max.max(1);
        Self {
            shared: Arc::new(Shared {
                in_flight: AtomicUsize::new(0),
                max: AtomicUsize::new(max),
                discovery_max: AtomicUsize::new((max * 3 / 4).max(1)),
                unary_timeout_ms: AtomicU64::new(
                    u64::try_from(unary_timeout.as_millis()).unwrap_or(u64::MAX),
                ),
            }),
        }
    }

    /// Reconciles the shared gate with the server's runtime configuration before it accepts calls.
    pub(crate) fn configure(&self, max: usize, unary_timeout: Duration) {
        let max = max.max(1);
        self.shared.max.store(max, Ordering::Release);
        self.shared
            .discovery_max
            .store((max * 3 / 4).max(1), Ordering::Release);
        self.shared.unary_timeout_ms.store(
            u64::try_from(unary_timeout.as_millis()).unwrap_or(u64::MAX),
            Ordering::Release,
        );
    }

    /// Uses the exact discovery reserve shared with unary registration/polling. The returned ticket
    /// is held only through one stream authentication and database claim.
    pub(crate) fn admit_discovery(&self) -> Option<Ticket> {
        self.shared.admit(Class::Discovery)
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
#[doc(hidden)]
pub struct Admit<S> {
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
        let limit = (class != Class::Transfer)
            .then(|| Duration::from_millis(self.shared.unary_timeout_ms.load(Ordering::Acquire)));
        Box::pin(async move {
            let response = match limit {
                None => call.await,
                Some(limit) => match tokio::time::timeout(limit, call).await {
                    Ok(response) => response,
                    Err(_) => {
                        metrics::counter!("grpc_timeouts_total", "class" => class.label())
                            .increment(1);
                        Ok(to_status(&ApiError::request_timeout()).into_http())
                    }
                },
            };
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
        assert_eq!(class("WriteBlob"), Class::Transfer);
        assert_eq!(class("ResolveDatabase"), Class::Lease);
        assert_eq!(class("ReadBlob"), Class::Transfer);
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
        let admission = Admission::new(8, Duration::from_secs(30));
        let stream_claims = admission.clone();
        let shared = admission.shared;
        let tickets: Vec<_> = (0..6)
            .map(|_| {
                shared
                    .admit(Class::Discovery)
                    .expect("below the discovery limit")
            })
            .collect();
        assert!(
            stream_claims.admit_discovery().is_none(),
            "a stream claim shares unary discovery saturation and cannot borrow the two lease-report places"
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

    #[test]
    fn task_stream_lifetime_caps_are_exact_and_release_on_drop() {
        let streams = TaskStreams::new();
        let tenant = TenantId::new(uuid::Uuid::now_v7());
        let first = streams.admit(tenant, "alice").expect("first subject slot");
        let second = streams.admit(tenant, "alice").expect("second subject slot");
        assert!(matches!(
            streams.admit(tenant, "alice"),
            Err(StreamRefused::Subject)
        ));
        drop((first, second));
        assert!(streams.admit(tenant, "alice").is_ok(), "slots return");

        let tenant_streams = TaskStreams::new();
        let held: Vec<_> = (0..8)
            .map(|index| {
                tenant_streams
                    .admit(tenant, &format!("subject-{index}"))
                    .expect("tenant place")
            })
            .collect();
        assert!(matches!(
            tenant_streams.admit(tenant, "ninth"),
            Err(StreamRefused::Tenant)
        ));
        drop(held);

        let process_streams = TaskStreams::new();
        let process_held: Vec<_> = (0..32)
            .map(|_| {
                process_streams
                    .admit(TenantId::new(uuid::Uuid::now_v7()), "one")
                    .expect("process place")
            })
            .collect();
        assert!(matches!(
            process_streams.admit(TenantId::new(uuid::Uuid::now_v7()), "one"),
            Err(StreamRefused::Process)
        ));
        drop(process_held);
    }

    fn ip(text: &str) -> IpAddr {
        text.parse().expect("an address")
    }

    #[test]
    fn a_peer_holds_at_most_its_share_and_gives_each_place_back() {
        let peers = Arc::new(Peers::default());
        let first = peers.admit(ip("203.0.113.7"), 2).expect("first");
        let second = peers.admit(ip("203.0.113.7"), 2).expect("second");
        assert!(
            peers.admit(ip("203.0.113.7"), 2).is_none(),
            "a third connection of the same address is turned away"
        );
        assert!(
            peers.admit(ip("203.0.113.8"), 2).is_some(),
            "another address is not affected"
        );
        drop(first);
        let again = peers
            .admit(ip("203.0.113.7"), 2)
            .expect("a place was freed");
        drop((second, again));
        let held = peers.counts.lock().expect("lock").len();
        assert_eq!(held, 0, "a peer with no connection left costs no memory");
    }

    #[test]
    fn an_address_stands_for_the_range_a_subscriber_can_use() {
        // IPv4 as itself, also when it arrives in IPv6 clothes.
        assert_eq!(peer_key(ip("203.0.113.7")), ip("203.0.113.7"));
        assert_eq!(peer_key(ip("::ffff:203.0.113.7")), ip("203.0.113.7"));
        // Every address of one /64 is one peer; another /64 is another.
        assert_eq!(
            peer_key(ip("2001:db8:1:2:aaaa:bbbb:cccc:dddd")),
            peer_key(ip("2001:db8:1:2::1"))
        );
        assert_ne!(
            peer_key(ip("2001:db8:1:2::1")),
            peer_key(ip("2001:db8:1:3::1"))
        );
        let peers = Arc::new(Peers::default());
        let _held = peers.admit(ip("2001:db8:1:2::1"), 1).expect("first");
        assert!(
            peers.admit(ip("2001:db8:1:2:ffff::9"), 1).is_none(),
            "the same /64 shares one count"
        );
    }
}
