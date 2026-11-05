//! The relay: the only route out of a lake sandbox (feature 035, D035-049, D035-061, FR-035-004).
//!
//! The container has no network interface but loopback. An in-container forwarder turns the client libraries'
//! loopback connections into connections to the attempt's Unix socket, where this relay decides, one connection at
//! a time, whether the sandbox may reach the destination it asks for:
//!
//! 1. **The token.** The first line is `SAGAT-RELAY/1 <token>`. Anything else, or a wrong token, closes the connection
//!    in silence, so a process that is not the attempt's host learns nothing.
//! 2. **The request.** `CONNECT host:port HTTP/1.1` and its headers (an HTTP proxy tunnel), or `TCP <name>` (a mapping
//!    the connection declared, for a destination that is not HTTP: a Hive metastore, a catalog database).
//! 3. **The name.** The host and port must be one the connection declared, or a regional endpoint of its storage type
//!    ([`EndpointPolicy`]). A catalog or a vended credential cannot add one. No pattern and no wildcard.
//! 4. **The address.** The name is resolved **once**, and every address of the answer must pass the managed executor's
//!    network policy (`sagat_infra_database::NetworkPolicy`: no loopback, link-local, multicast or cloud metadata
//!    address, no platform state network). The relay then connects to the address it checked, so a second answer from
//!    a hostile DNS server (rebinding) is never asked for.
//! 5. **The tunnel.** After the answer the relay copies bytes in both directions and reads none of them. It never
//!    terminates TLS: the client verifies the certificate of the declared host name end to end.
//!
//! Each refusal is counted by a closed kind ([`RelayDenials`]) and returned with the result for the audit. The refusal
//! text never repeats what the client asked for.
//!
//! The resolver and the connector are traits so that a test scripts a DNS answer and records the address connected to.
//! The control conversation (`CONTROL`, the `sagat.lake.v1` lines) is served by the attempt driver (T035-035), not here.

use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::io;
use std::net::{IpAddr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use ipnet::IpNet;
use sagat_infra_database::{DestinationScope, NetworkPolicy};
use sagat_telemetry::LakeRelayDenial;
use sagat_telemetry::lake::record_relay_denial;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, copy_bidirectional};
use tokio::net::{TcpStream, UnixListener, UnixStream};
use tokio::sync::Semaphore;
use tokio::time::{Instant, timeout_at};
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use crate::protocol::RelayToken;

/// The first word of the first line of every connection.
const PREFACE: &[u8] = b"SAGAT-RELAY/1 ";
/// The longest first line: the word, the token and the newline.
const MAX_PREFACE_BYTES: usize = 256;
/// The longest request: the request line and the headers together.
const MAX_REQUEST_BYTES: usize = 8192;
const MAX_MAPPING_NAME: usize = 32;
const MAX_RESOLVED: usize = 17;
const HTTPS: u16 = 443;
/// Connections served at once by one relay: a sandbox that opens more waits in the kernel's backlog.
const DEFAULT_MAX_CONNECTIONS: usize = 64;
/// How long a request has to be resolved and connected (D035-079).
const DEFAULT_ADMISSION_TIMEOUT: Duration = Duration::from_secs(10);

const ESTABLISHED: &[u8] = b"HTTP/1.1 200 Connection Established\r\n\r\n";
const FORBIDDEN: &[u8] = b"HTTP/1.1 403 Forbidden\r\n\r\n";
const BAD_GATEWAY: &[u8] = b"HTTP/1.1 502 Bad Gateway\r\n\r\n";
const BAD_REQUEST: &[u8] = b"HTTP/1.1 400 Bad Request\r\n\r\n";
const TCP_OK: &[u8] = b"OK\n";
const TCP_DENIED: &[u8] = b"DENIED\n";

/// What a relay is configured with that is wrong. These are faults of the runner, not of the sandbox.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RelayConfigError {
    #[error("not a host name or an address")]
    Host,
    #[error("not a port")]
    Port,
    #[error("not a region")]
    Region,
    #[error("not a bucket name")]
    Bucket,
    #[error("a mapping has an invalid or a repeated name")]
    Mapping,
    #[error("the network policy is not valid")]
    Network,
    #[error("a limit is zero")]
    Limit,
}

/// A host (a lower-case DNS name without its trailing dot, or an address in its canonical form) and a port.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Endpoint {
    host: String,
    port: u16,
}

impl Endpoint {
    pub fn new(host: &str, port: u16) -> Result<Self, RelayConfigError> {
        if port == 0 {
            return Err(RelayConfigError::Port);
        }
        let host = normalize_host(host).ok_or(RelayConfigError::Host)?;
        Ok(Self { host, port })
    }
}

/// `text` as the relay compares hosts: lower case, one trailing dot removed, an IP literal in its canonical form
/// (bracketed IPv6 accepted). `None` when it is neither a DNS name of valid labels nor an address.
fn normalize_host(text: &str) -> Option<String> {
    let text = text.strip_suffix('.').unwrap_or(text);
    if let Some(inner) = text
        .strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
    {
        return inner
            .parse::<Ipv6Addr>()
            .ok()
            .map(|address| address.to_string());
    }
    if let Ok(address) = text.parse::<IpAddr>() {
        return Some(address.to_string());
    }
    let name = text.to_ascii_lowercase();
    let label = |label: &str| {
        (1..=63).contains(&label.len())
            && label
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
            && !label.starts_with('-')
            && !label.ends_with('-')
    };
    (!name.is_empty() && name.len() <= 253 && name.split('.').all(label)).then_some(name)
}

/// The names the sandbox may ask for.
#[derive(Debug, Clone, Default)]
pub struct EndpointPolicy {
    allowed: BTreeSet<Endpoint>,
}

impl EndpointPolicy {
    /// The endpoints the pinned connection declares: the catalog, its token endpoint, an S3-compatible store.
    #[must_use]
    pub fn new(declared: impl IntoIterator<Item = Endpoint>) -> Self {
        Self {
            allowed: declared.into_iter().collect(),
        }
    }

    /// Adds the official regional endpoints of S3 for `region` and for each bucket the warehouse locations name: the
    /// path-style and dual-stack hosts, and the virtual-hosted ones for a bucket without a dot (a dotted bucket cannot
    /// be addressed that way over TLS). The relay sees host names, not buckets (G035-021), so a bucket that is not
    /// named here is not allowed.
    pub fn with_s3_regional(
        mut self,
        region: &str,
        buckets: &[&str],
    ) -> Result<Self, RelayConfigError> {
        if !valid_region(region) {
            return Err(RelayConfigError::Region);
        }
        let domain = if region.starts_with("cn-") {
            "amazonaws.com.cn"
        } else {
            "amazonaws.com"
        };
        let mut hosts = vec![
            format!("s3.{region}.{domain}"),
            format!("s3.dualstack.{region}.{domain}"),
        ];
        for bucket in buckets {
            if !valid_bucket(bucket) {
                return Err(RelayConfigError::Bucket);
            }
            if !bucket.contains('.') {
                hosts.push(format!("{bucket}.s3.{region}.{domain}"));
                hosts.push(format!("{bucket}.s3.dualstack.{region}.{domain}"));
            }
        }
        for host in hosts {
            self.allowed.insert(Endpoint::new(&host, HTTPS)?);
        }
        Ok(self)
    }

    /// Whether the sandbox may reach `host:port`. `host` is normalized here, so the caller's case and trailing dot do
    /// not matter, and a string that is not a host is not allowed.
    #[must_use]
    pub fn allows(&self, host: &str, port: u16) -> bool {
        normalize_host(host).is_some_and(|host| self.allowed.contains(&Endpoint { host, port }))
    }

    fn ports(&self) -> BTreeSet<u16> {
        self.allowed.iter().map(|endpoint| endpoint.port).collect()
    }
}

fn valid_region(region: &str) -> bool {
    let bytes = region.as_bytes();
    (3..=32).contains(&bytes.len())
        && bytes[0].is_ascii_lowercase()
        && bytes[bytes.len() - 1].is_ascii_alphanumeric()
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-')
}

fn valid_bucket(bucket: &str) -> bool {
    let bytes = bucket.as_bytes();
    (3..=63).contains(&bytes.len())
        && bytes[0].is_ascii_alphanumeric()
        && bytes[bytes.len() - 1].is_ascii_alphanumeric()
        && bytes.iter().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-' || *byte == b'.'
        })
        && !bucket.contains("..")
}

/// A TCP destination the connection declared for something that is not HTTP, named so the container asks for it by
/// name and never by address.
#[derive(Debug, Clone)]
pub struct Mapping {
    name: String,
    endpoint: Endpoint,
}

impl Mapping {
    #[must_use]
    pub fn new(name: &str, endpoint: Endpoint) -> Self {
        Self {
            name: name.to_owned(),
            endpoint,
        }
    }
}

fn valid_mapping_name(name: &str) -> bool {
    (1..=MAX_MAPPING_NAME).contains(&name.len())
        && name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

/// What the relay is configured with for one attempt.
#[derive(Debug, Clone)]
pub struct RelayConfig {
    token: RelayToken,
    endpoints: EndpointPolicy,
    mappings: Vec<Mapping>,
    allow: Vec<IpNet>,
    deny: Vec<IpNet>,
    development: bool,
    handshake_timeout: Duration,
    admission_timeout: Duration,
    max_connections: usize,
    peer_uid: Option<u32>,
}

impl RelayConfig {
    /// `allow` is the operator's list of networks the sandbox may reach, as for the managed database executor: an
    /// address outside it is refused whatever name it came from, and an empty list reaches nothing. It is a required
    /// argument so that a relay cannot be built open: a declared name that a tenant's DNS points at the cluster's
    /// own addresses is stopped here.
    #[must_use]
    pub fn new(
        token: RelayToken,
        endpoints: EndpointPolicy,
        allow: impl IntoIterator<Item = IpNet>,
    ) -> Self {
        Self {
            token,
            endpoints,
            mappings: Vec::new(),
            allow: allow.into_iter().collect(),
            deny: Vec::new(),
            development: false,
            handshake_timeout: Duration::from_secs(5),
            admission_timeout: DEFAULT_ADMISSION_TIMEOUT,
            max_connections: DEFAULT_MAX_CONNECTIONS,
            peer_uid: None,
        }
    }

    #[must_use]
    pub fn with_mappings(mut self, mappings: impl IntoIterator<Item = Mapping>) -> Self {
        self.mappings.extend(mappings);
        self
    }

    /// Networks the sandbox may never reach whatever the connection declares: the platform's own state (PostgreSQL,
    /// the broker, the secret store).
    #[must_use]
    pub fn with_deny(mut self, networks: impl IntoIterator<Item = IpNet>) -> Self {
        self.deny.extend(networks);
        self
    }

    /// Development allows the loopback interface (the fixtures of the tests run on it) and nothing else.
    #[must_use]
    pub fn development(mut self, development: bool) -> Self {
        self.development = development;
        self
    }

    /// How long a connection has to present its token and its request.
    #[must_use]
    pub fn with_handshake_timeout(mut self, limit: Duration) -> Self {
        self.handshake_timeout = limit;
        self
    }

    /// How long the relay has to resolve a name and connect to the address it checked. A name server that never
    /// answers is the tenant's to arrange, so it must not hold a thread of the runner.
    #[must_use]
    pub fn with_admission_timeout(mut self, limit: Duration) -> Self {
        self.admission_timeout = limit;
        self
    }

    /// How many connections are served at once (64 by default). The others wait in the kernel's backlog.
    #[must_use]
    pub fn with_max_connections(mut self, limit: usize) -> Self {
        self.max_connections = limit;
        self
    }

    /// Only a peer with this user id is served. The token proves a connection is the attempt's, but the sandboxed code
    /// holds it too; the user id of the container's slot (a distinct one per slot) keeps another attempt's container
    /// out if it could reach the socket. Read from the connection by the kernel, not by the client.
    #[must_use]
    pub fn with_peer_uid(mut self, uid: u32) -> Self {
        self.peer_uid = Some(uid);
        self
    }
}

/// Resolves a name. It is called once per connection.
pub trait Resolver: Send + Sync + 'static {
    fn resolve(
        &self,
        host: &str,
        port: u16,
    ) -> impl Future<Output = io::Result<Vec<IpAddr>>> + Send;
}

/// The operating system's resolver.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemResolver;

impl Resolver for SystemResolver {
    async fn resolve(&self, host: &str, port: u16) -> io::Result<Vec<IpAddr>> {
        Ok(tokio::net::lookup_host((host, port))
            .await?
            .take(MAX_RESOLVED)
            .map(|address| address.ip())
            .collect())
    }
}

/// Opens the connection to the address the relay checked.
pub trait Connector: Send + Sync + 'static {
    type Stream: AsyncRead + AsyncWrite + Unpin + Send + 'static;

    fn connect(&self, address: SocketAddr)
    -> impl Future<Output = io::Result<Self::Stream>> + Send;
}

/// Plain TCP with a connect timeout.
#[derive(Debug, Clone, Copy)]
pub struct TcpConnector {
    timeout: Duration,
}

impl TcpConnector {
    #[must_use]
    pub const fn new(timeout: Duration) -> Self {
        Self { timeout }
    }
}

impl Connector for TcpConnector {
    type Stream = TcpStream;

    async fn connect(&self, address: SocketAddr) -> io::Result<TcpStream> {
        let stream = tokio::time::timeout(self.timeout, TcpStream::connect(address))
            .await
            .map_err(|_| io::Error::from(io::ErrorKind::TimedOut))??;
        stream.set_nodelay(true)?;
        Ok(stream)
    }
}

/// How many connections the relay refused, by kind, for the audit (the `relayDenials` of the result line is the total).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RelayDenials {
    pub endpoint_not_declared: u32,
    pub address_forbidden: u32,
    pub resolution_failed: u32,
    pub token_refused: u32,
    pub request_unsupported: u32,
}

impl RelayDenials {
    #[must_use]
    pub const fn total(&self) -> u32 {
        self.endpoint_not_declared
            .saturating_add(self.address_forbidden)
            .saturating_add(self.resolution_failed)
            .saturating_add(self.token_refused)
            .saturating_add(self.request_unsupported)
    }
}

#[derive(Debug, Default)]
struct Counters {
    endpoint_not_declared: AtomicU32,
    address_forbidden: AtomicU32,
    resolution_failed: AtomicU32,
    token_refused: AtomicU32,
    request_unsupported: AtomicU32,
}

/// A count that stops at the maximum instead of starting again from zero.
fn increment(counter: &AtomicU32) {
    let _ = counter.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |count| {
        Some(count.saturating_add(1))
    });
}

enum Target {
    Connect(Endpoint),
    Tcp(String),
}

/// What a connection asked for and was refused, before the reply.
enum Refusal {
    Denied(LakeRelayDenial),
    /// The destination was allowed and could not be reached: not a denial, and not counted.
    Unreachable,
}

struct Inner<R, C> {
    token: RelayToken,
    endpoints: EndpointPolicy,
    mappings: BTreeMap<String, Endpoint>,
    policy: NetworkPolicy,
    scope: DestinationScope,
    handshake_timeout: Duration,
    admission_timeout: Duration,
    peer_uid: Option<u32>,
    max_connections: usize,
    slots: Arc<Semaphore>,
    resolver: R,
    connector: C,
    counters: Counters,
}

/// The relay of one attempt.
pub struct Relay<R, C> {
    inner: Arc<Inner<R, C>>,
}

impl<R, C> Clone for Relay<R, C> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<R: Resolver, C: Connector> Relay<R, C> {
    pub fn new(config: RelayConfig, resolver: R, connector: C) -> Result<Self, RelayConfigError> {
        let mut mappings = BTreeMap::new();
        let mut ports = config.endpoints.ports();
        for mapping in config.mappings {
            if !valid_mapping_name(&mapping.name) {
                return Err(RelayConfigError::Mapping);
            }
            ports.insert(mapping.endpoint.port);
            if mappings.insert(mapping.name, mapping.endpoint).is_some() {
                return Err(RelayConfigError::Mapping);
            }
        }
        if config.max_connections == 0
            || config.handshake_timeout.is_zero()
            || config.admission_timeout.is_zero()
        {
            return Err(RelayConfigError::Limit);
        }
        // The name policy says where the sandbox may go; the operator's networks and the policy below say which
        // addresses a name may resolve to.
        let scope = DestinationScope::new(config.allow, ports.into_iter().collect())
            .map_err(|_| RelayConfigError::Network)?;
        let policy = NetworkPolicy::new(scope.clone(), config.deny, config.development)
            .map_err(|_| RelayConfigError::Network)?;
        Ok(Self {
            inner: Arc::new(Inner {
                token: config.token,
                endpoints: config.endpoints,
                mappings,
                policy,
                scope,
                handshake_timeout: config.handshake_timeout,
                admission_timeout: config.admission_timeout,
                peer_uid: config.peer_uid,
                max_connections: config.max_connections,
                slots: Arc::new(Semaphore::new(config.max_connections)),
                resolver,
                connector,
                counters: Counters::default(),
            }),
        })
    }

    /// What the relay has refused so far.
    #[must_use]
    pub fn denials(&self) -> RelayDenials {
        let counters = &self.inner.counters;
        RelayDenials {
            endpoint_not_declared: counters.endpoint_not_declared.load(Ordering::Relaxed),
            address_forbidden: counters.address_forbidden.load(Ordering::Relaxed),
            resolution_failed: counters.resolution_failed.load(Ordering::Relaxed),
            token_refused: counters.token_refused.load(Ordering::Relaxed),
            request_unsupported: counters.request_unsupported.load(Ordering::Relaxed),
        }
    }

    /// The connections being served by [`Relay::serve`] now.
    #[must_use]
    pub fn open_connections(&self) -> usize {
        self.inner
            .max_connections
            .saturating_sub(self.inner.slots.available_permits())
    }

    fn deny(&self, kind: LakeRelayDenial) {
        let counters = &self.inner.counters;
        let counter = match kind {
            LakeRelayDenial::EndpointNotDeclared => &counters.endpoint_not_declared,
            LakeRelayDenial::AddressForbidden => &counters.address_forbidden,
            LakeRelayDenial::ResolutionFailed => &counters.resolution_failed,
            LakeRelayDenial::TokenRefused => &counters.token_refused,
            LakeRelayDenial::RequestUnsupported => &counters.request_unsupported,
        };
        increment(counter);
        record_relay_denial(kind);
    }

    /// Accepts connections on the attempt's socket until `shutdown`, then waits for the connections, which `shutdown` ends.
    ///
    /// A slot is taken before a connection is accepted, so a sandbox that opens more than the limit waits in the
    /// kernel's backlog instead of in this process; a finished connection leaves nothing behind.
    pub async fn serve(&self, listener: UnixListener, shutdown: CancellationToken) {
        let tracker = TaskTracker::new();
        loop {
            let slot = tokio::select! {
                () = shutdown.cancelled() => break,
                slot = Arc::clone(&self.inner.slots).acquire_owned() => match slot {
                    Ok(slot) => slot,
                    Err(_) => break,
                },
            };
            let accepted = tokio::select! {
                () = shutdown.cancelled() => break,
                accepted = listener.accept() => accepted,
            };
            match accepted {
                Ok((stream, _)) if !self.peer_allowed(&stream) => {
                    self.deny(LakeRelayDenial::TokenRefused);
                    drop(stream);
                }
                Ok((stream, _)) => {
                    let relay = self.clone();
                    let shutdown = shutdown.clone();
                    tracker.spawn(async move {
                        relay.handle(stream, &shutdown).await;
                        drop(slot);
                    });
                }
                // A descriptor limit or a reset: back off instead of spinning.
                Err(_) => tokio::time::sleep(Duration::from_millis(10)).await,
            }
        }
        tracker.close();
        tracker.wait().await;
    }

    /// Whether the kernel says the peer runs as the configured user (every peer, when none is configured).
    fn peer_allowed(&self, stream: &UnixStream) -> bool {
        self.inner
            .peer_uid
            .is_none_or(|uid| stream.peer_cred().is_ok_and(|peer| peer.uid() == uid))
    }

    /// One connection of the sandbox, from its token to the end of its tunnel. A shutdown ends it at any point: while
    /// it waits for its request, for a name server or for a destination, as well as while it copies.
    pub async fn handle<S>(&self, stream: S, shutdown: &CancellationToken)
    where
        S: AsyncRead + AsyncWrite + Unpin + Send,
    {
        tokio::select! {
            () = shutdown.cancelled() => {}
            () = self.converse(stream) => {}
        }
    }

    async fn converse<S>(&self, mut stream: S)
    where
        S: AsyncRead + AsyncWrite + Unpin + Send,
    {
        let deadline = Instant::now() + self.inner.handshake_timeout;
        if !matches!(
            timeout_at(deadline, self.authenticate(&mut stream)).await,
            Ok(true)
        ) {
            self.deny(LakeRelayDenial::TokenRefused);
            return;
        }
        let (mode, target) = match timeout_at(deadline, read_request(&mut stream)).await {
            Ok(Some(request)) => request,
            Ok(None) | Err(_) => {
                self.deny(LakeRelayDenial::RequestUnsupported);
                let _ = stream.write_all(BAD_REQUEST).await;
                return;
            }
        };
        let admission = Instant::now() + self.inner.admission_timeout;
        let address = match self.admit(target, admission).await {
            Ok(address) => address,
            Err(refusal) => {
                if let Refusal::Denied(kind) = refusal {
                    self.deny(kind);
                }
                let _ = stream.write_all(refusal_reply(mode, &refusal)).await;
                return;
            }
        };
        let Ok(Ok(mut remote)) = timeout_at(admission, self.inner.connector.connect(address)).await
        else {
            let _ = stream
                .write_all(refusal_reply(mode, &Refusal::Unreachable))
                .await;
            return;
        };
        let established = match mode {
            Mode::Connect => ESTABLISHED,
            Mode::Tcp => TCP_OK,
        };
        if stream.write_all(established).await.is_err() {
            return;
        }
        let _ = copy_bidirectional(&mut stream, &mut remote).await;
    }

    /// The first line: the word and the attempt's token. Anything else is refused without a reply.
    async fn authenticate<S: AsyncRead + Unpin>(&self, stream: &mut S) -> bool {
        let Some(line) = read_line(stream, MAX_PREFACE_BYTES).await else {
            return false;
        };
        line.strip_prefix(PREFACE)
            .is_some_and(|presented| same(presented, self.inner.token.as_bytes()))
    }

    /// The name policy, one resolution, the address policy, and the address to connect to.
    async fn admit(&self, target: Target, deadline: Instant) -> Result<SocketAddr, Refusal> {
        let endpoint = match target {
            Target::Connect(endpoint) => endpoint,
            Target::Tcp(name) => self
                .inner
                .mappings
                .get(&name)
                .cloned()
                .ok_or(Refusal::Denied(LakeRelayDenial::EndpointNotDeclared))?,
        };
        if !self.inner.endpoints.allows(&endpoint.host, endpoint.port) && !self.maps(&endpoint) {
            return Err(Refusal::Denied(LakeRelayDenial::EndpointNotDeclared));
        }
        let addresses = match endpoint.host.parse::<IpAddr>() {
            Ok(address) => vec![address],
            Err(_) => {
                // Asked for as a fully qualified name: without its trailing dot the resolver's search domains (`ndots`)
                // could turn `catalog.example` into an in-cluster name (D035-079).
                let name = format!("{}.", endpoint.host);
                match timeout_at(deadline, self.inner.resolver.resolve(&name, endpoint.port)).await
                {
                    Ok(Ok(answer)) if !answer.is_empty() => {
                        answer.into_iter().take(MAX_RESOLVED).collect()
                    }
                    _ => return Err(Refusal::Denied(LakeRelayDenial::ResolutionFailed)),
                }
            }
        };
        self.inner
            .policy
            .validate_destination(&addresses, endpoint.port, &self.inner.scope)
            .map_err(|_| Refusal::Denied(LakeRelayDenial::AddressForbidden))?;
        let pinned = addresses
            .iter()
            .map(IpAddr::to_canonical)
            .find(IpAddr::is_ipv4)
            .unwrap_or_else(|| addresses[0].to_canonical());
        Ok(SocketAddr::new(pinned, endpoint.port))
    }

    /// Whether `endpoint` is the destination of a declared mapping (a mapping is asked for by name, and is allowed).
    fn maps(&self, endpoint: &Endpoint) -> bool {
        self.inner
            .mappings
            .values()
            .any(|mapped| mapped == endpoint)
    }
}

/// The reply of a refusal, in the language of the request: HTTP for a tunnel, a word for a mapping.
fn refusal_reply(mode: Mode, refusal: &Refusal) -> &'static [u8] {
    match (mode, refusal) {
        (Mode::Tcp, _) => TCP_DENIED,
        (
            Mode::Connect,
            Refusal::Denied(LakeRelayDenial::ResolutionFailed) | Refusal::Unreachable,
        ) => BAD_GATEWAY,
        (Mode::Connect, Refusal::Denied(LakeRelayDenial::RequestUnsupported)) => BAD_REQUEST,
        (Mode::Connect, Refusal::Denied(_)) => FORBIDDEN,
    }
}

#[derive(Clone, Copy)]
enum Mode {
    Connect,
    Tcp,
}

/// Equality that does not stop at the first different byte.
fn same(left: &[u8], right: &[u8]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .fold(0_u8, |difference, (a, b)| difference | (a ^ b))
            == 0
}

/// One line, without its newline, of at most `limit` bytes. A byte at a time: a buffered reader could take bytes of
/// the tunnel that follow the request, and the request is a few hundred bytes.
async fn read_line<S: AsyncRead + Unpin>(stream: &mut S, limit: usize) -> Option<Vec<u8>> {
    let mut line = Vec::new();
    loop {
        let byte = stream.read_u8().await.ok()?;
        if byte == b'\n' {
            return Some(line);
        }
        if line.len() >= limit {
            return None;
        }
        line.push(byte);
    }
}

/// The request of a connection that presented its token, or `None` when the relay does not serve it.
async fn read_request<S: AsyncRead + Unpin>(stream: &mut S) -> Option<(Mode, Target)> {
    let line = read_line(stream, MAX_REQUEST_BYTES).await?;
    if let Some(name) = line.strip_prefix(b"TCP ") {
        let name = std::str::from_utf8(name).ok()?;
        return Some((Mode::Tcp, Target::Tcp(name.to_owned())));
    }
    let text = std::str::from_utf8(line.strip_suffix(b"\r")?).ok()?;
    let mut parts = text.split(' ');
    let (Some("CONNECT"), Some(authority), Some("HTTP/1.1" | "HTTP/1.0"), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return None;
    };
    let target = parse_authority(authority)?;
    // The headers are read to the blank line and ignored.
    let mut used = line.len() + 1;
    loop {
        let header = read_line(stream, MAX_REQUEST_BYTES.saturating_sub(used)).await?;
        used += header.len() + 1;
        match header.as_slice() {
            b"\r" => return Some((Mode::Connect, Target::Connect(target))),
            [.., b'\r'] => {}
            _ => return None,
        }
    }
}

/// `host:port` or `[ipv6]:port`: a host without user info, a port of digits from 1 to 65535.
fn parse_authority(authority: &str) -> Option<Endpoint> {
    let (host, port) = if authority.starts_with('[') {
        let end = authority.find(']')?;
        (&authority[..=end], authority[end + 1..].strip_prefix(':')?)
    } else {
        // Without brackets a colon is ambiguous (`::1:443`), and parsers that disagree about it are how a check is bypassed.
        let (host, port) = authority.rsplit_once(':')?;
        if host.contains(':') {
            return None;
        }
        (host, port)
    };
    if port.is_empty() || port.len() > 5 || !port.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    Endpoint::new(host, port.parse().ok()?).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_denial_counter_stops_at_the_maximum_instead_of_starting_again() {
        let counter = AtomicU32::new(u32::MAX - 1);

        increment(&counter);
        increment(&counter);
        increment(&counter);

        assert_eq!(counter.load(Ordering::Relaxed), u32::MAX);
    }
}
