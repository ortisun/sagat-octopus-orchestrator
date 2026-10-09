//! Guarded outbound HTTP (`FR-009-008`): the only way integrations (webhook delivery, HTTP
//! activities) reach the network.
//!
//! The policy lives in the domain ([`EgressPolicy`]); this crate makes it impossible to bypass:
//!
//! * the destination is checked *before* anything is resolved;
//! * the DNS answer is checked address by address *inside the connector's resolver*, so the
//!   addresses that passed are the very ones the socket connects to (no second lookup to rebind),
//!   and one forbidden address among the answers refuses the whole answer;
//! * an address literal never reaches the resolver, so it is checked up front;
//! * redirects are never followed by the HTTP client; the few this crate follows (for `GET` and
//!   `HEAD` only) go through the same two checks on every hop;
//! * proxies from the environment are ignored, because a proxy would make the connection to
//!   somewhere the policy never saw;
//! * time and size are bounded.
//!
//! A refusal is an [`EgressError::Denied`] carrying a [`Denial`] (a category, never the address) so
//! the caller can audit it.

use std::future::Future;
use std::net::{IpAddr, SocketAddr};
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use reqwest::redirect::Policy;
use reqwest::{Method, StatusCode, Url};
use sagat_domain::egress::{Denial, Destination, EgressPolicy};

/// What decides whether a connection may be made. [`EgressPolicy`] is the production
/// implementation; tests of this crate substitute one that lets them reach a local server.
pub trait Guard: Send + Sync + 'static {
    /// # Errors
    /// The [`Denial`] for a destination that may not be reached.
    fn check_destination(&self, destination: &Destination<'_>) -> Result<(), Denial>;

    /// # Errors
    /// The [`Denial`] for an address that may not be connected to.
    fn check_address(&self, address: IpAddr) -> Result<(), Denial>;
}

impl Guard for EgressPolicy {
    fn check_destination(&self, destination: &Destination<'_>) -> Result<(), Denial> {
        Self::check_destination(self, destination)
    }

    fn check_address(&self, address: IpAddr) -> Result<(), Denial> {
        Self::check_address(self, address)
    }
}

/// Resolves a host name to addresses. The system resolver by default.
pub trait NameResolver: Send + Sync + 'static {
    fn lookup<'a>(
        &'a self,
        host: &'a str,
    ) -> Pin<Box<dyn Future<Output = std::io::Result<Vec<IpAddr>>> + Send + 'a>>;
}

/// The operating system's resolver.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemResolver;

impl NameResolver for SystemResolver {
    fn lookup<'a>(
        &'a self,
        host: &'a str,
    ) -> Pin<Box<dyn Future<Output = std::io::Result<Vec<IpAddr>>> + Send + 'a>> {
        Box::pin(async move {
            Ok(tokio::net::lookup_host((host, 0))
                .await?
                .map(|a| a.ip())
                .collect())
        })
    }
}

/// Limits of one client.
#[derive(Debug, Clone, Copy)]
pub struct EgressLimits {
    pub connect_timeout: Duration,
    pub total_timeout: Duration,
    /// Largest response body read; more is [`EgressError::ResponseTooLarge`].
    pub max_response_bytes: usize,
    /// Redirects followed for `GET` and `HEAD`; every other method never follows one.
    pub max_redirects: u8,
}

impl Default for EgressLimits {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(5),
            total_timeout: Duration::from_secs(15),
            max_response_bytes: 1024 * 1024,
            max_redirects: 0,
        }
    }
}

/// Why a request was not made or did not complete.
#[derive(Debug, thiserror::Error)]
pub enum EgressError {
    /// The policy refused. Safe to audit: it names a category, never an address.
    #[error("egress denied: {}", .0.code())]
    Denied(Denial),
    #[error("the URL is not acceptable")]
    InvalidUrl,
    #[error("the connection or the request failed")]
    Transport,
    #[error("the request timed out")]
    Timeout,
    #[error("the response is larger than allowed")]
    ResponseTooLarge,
    #[error("too many redirects")]
    TooManyRedirects,
}

/// A request to send.
#[derive(Debug, Clone)]
pub struct EgressRequest {
    pub method: Method,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

/// What came back.
#[derive(Debug, Clone)]
pub struct EgressResponse {
    pub status: StatusCode,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

/// The guard applied to the connector's name resolution.
struct GuardedResolver {
    guard: Arc<dyn Guard>,
    names: Arc<dyn NameResolver>,
}

/// Marks an error that came from the policy, so the client can report a denial instead of a
/// transport failure.
#[derive(Debug, thiserror::Error)]
#[error("egress denied: {}", .0.code())]
struct Refused(Denial);

impl Resolve for GuardedResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let guard = Arc::clone(&self.guard);
        let names = Arc::clone(&self.names);
        Box::pin(async move {
            let host = name.as_str().to_owned();
            let answers = names.lookup(&host).await?;
            if answers.is_empty() {
                return Err(Refused(Denial::NoAddress).into());
            }
            // One forbidden address refuses the whole answer: an attacker who controls the zone
            // would otherwise mix good and bad addresses and hope for the bad one.
            for address in &answers {
                guard.check_address(*address).map_err(Refused)?;
            }
            let addrs: Addrs = Box::new(answers.into_iter().map(|a| SocketAddr::new(a, 0)));
            Ok(addrs)
        })
    }
}

/// An HTTP client that can only go where the policy says.
pub struct EgressClient {
    client: reqwest::Client,
    guard: Arc<dyn Guard>,
    limits: EgressLimits,
}

impl std::fmt::Debug for EgressClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EgressClient").finish_non_exhaustive()
    }
}

impl EgressClient {
    /// A client for `policy`, resolving names with the operating system.
    ///
    /// # Errors
    /// [`EgressError::Transport`] if the TLS stack cannot be built.
    pub fn new(policy: EgressPolicy, limits: EgressLimits) -> Result<Self, EgressError> {
        Self::with(Arc::new(policy), Arc::new(SystemResolver), limits)
    }

    /// A client with an explicit guard and resolver.
    ///
    /// # Errors
    /// [`EgressError::Transport`] if the TLS stack cannot be built.
    pub fn with(
        guard: Arc<dyn Guard>,
        names: Arc<dyn NameResolver>,
        limits: EgressLimits,
    ) -> Result<Self, EgressError> {
        install_crypto_provider();
        let client = reqwest::Client::builder()
            .dns_resolver(Arc::new(GuardedResolver {
                guard: Arc::clone(&guard),
                names,
            }))
            // The HTTP client never follows a redirect itself: `send` does, hop by hop.
            .redirect(Policy::none())
            // A proxy taken from the environment would carry the connection somewhere the policy
            // never inspected.
            .no_proxy()
            .connect_timeout(limits.connect_timeout)
            .timeout(limits.total_timeout)
            .build()
            .map_err(|_| EgressError::Transport)?;
        Ok(Self {
            client,
            guard,
            limits,
        })
    }

    /// Sends `request`, enforcing the policy on the destination and on every redirect hop.
    ///
    /// # Errors
    /// [`EgressError`]; [`EgressError::Denied`] when the policy refused.
    pub async fn send(&self, request: EgressRequest) -> Result<EgressResponse, EgressError> {
        let follows = matches!(request.method, Method::GET | Method::HEAD);
        let mut url = Url::parse(&request.url).map_err(|_| EgressError::InvalidUrl)?;
        let mut hops = 0u8;
        loop {
            self.check(&url)?;
            let mut builder = self.client.request(request.method.clone(), url.clone());
            for (name, value) in &request.headers {
                builder = builder.header(name, value);
            }
            if !request.body.is_empty() {
                builder = builder.body(request.body.clone());
            }
            let mut response = builder.send().await.map_err(classify)?;
            let status = response.status();
            if follows && status.is_redirection() && self.limits.max_redirects > 0 {
                let target = response
                    .headers()
                    .get(reqwest::header::LOCATION)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|location| url.join(location).ok());
                if let Some(target) = target {
                    hops += 1;
                    if hops > self.limits.max_redirects {
                        return Err(EgressError::TooManyRedirects);
                    }
                    url = target;
                    continue;
                }
            }
            let headers = response
                .headers()
                .iter()
                .filter_map(|(k, v)| {
                    v.to_str()
                        .ok()
                        .map(|v| (k.as_str().to_owned(), v.to_owned()))
                })
                .collect();
            let mut body = Vec::new();
            while let Some(chunk) = response.chunk().await.map_err(classify)? {
                if body.len() + chunk.len() > self.limits.max_response_bytes {
                    return Err(EgressError::ResponseTooLarge);
                }
                body.extend_from_slice(&chunk);
            }
            return Ok(EgressResponse {
                status,
                headers,
                body,
            });
        }
    }

    /// The first stage of the policy, on the URL as the client will use it.
    fn check(&self, url: &Url) -> Result<(), EgressError> {
        // Credentials in a URL are an exfiltration and confusion vector ("https://allowed@evil").
        if !url.username().is_empty() || url.password().is_some() || url.fragment().is_some() {
            return Err(EgressError::InvalidUrl);
        }
        let host = url.host_str().ok_or(EgressError::InvalidUrl)?;
        let port = url.port_or_known_default().ok_or(EgressError::InvalidUrl)?;
        self.guard
            .check_destination(&Destination {
                scheme: url.scheme(),
                host,
                port,
            })
            .map_err(EgressError::Denied)
    }
}

/// reqwest is built without a TLS crypto provider (so the choice stays with the binary); this crate
/// needs one to exist, and installs the `ring` provider if the process has not chosen another.
fn install_crypto_provider() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        // An error means the process installed its own provider first, which is fine.
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

/// Maps a transport failure; a refusal raised inside the resolver is a denial, not an outage.
fn classify(error: reqwest::Error) -> EgressError {
    if error.is_timeout() {
        return EgressError::Timeout;
    }
    let mut source: Option<&(dyn std::error::Error + 'static)> = Some(&error);
    while let Some(current) = source {
        if let Some(Refused(denial)) = current.downcast_ref::<Refused>() {
            return EgressError::Denied(*denial);
        }
        source = current.source();
    }
    tracing::debug!(kind = %kind_of(&error), "an outbound request failed");
    EgressError::Transport
}

fn kind_of(error: &reqwest::Error) -> &'static str {
    if error.is_connect() {
        "connect"
    } else if error.is_request() {
        "request"
    } else if error.is_body() || error.is_decode() {
        "body"
    } else {
        "other"
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use sagat_domain::egress::{AddressClass, HostRule};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    use super::*;

    /// Names to addresses, as a hostile (or merely misconfigured) zone would answer.
    #[derive(Default)]
    struct Zone(BTreeMap<&'static str, Vec<IpAddr>>);

    impl NameResolver for Zone {
        fn lookup<'a>(
            &'a self,
            host: &'a str,
        ) -> Pin<Box<dyn Future<Output = std::io::Result<Vec<IpAddr>>> + Send + 'a>> {
            Box::pin(async move { Ok(self.0.get(host).cloned().unwrap_or_default()) })
        }
    }

    fn zone(entries: &[(&'static str, &[&str])]) -> Arc<Zone> {
        Arc::new(Zone(
            entries
                .iter()
                .map(|(name, addrs)| {
                    (
                        *name,
                        addrs.iter().map(|a| a.parse().expect("ip")).collect(),
                    )
                })
                .collect(),
        ))
    }

    /// Lets tests reach a server on this machine: allows the names it is given and every address
    /// except the categories the real policy never allows besides loopback.
    struct Lab {
        names: &'static [&'static str],
    }

    impl Guard for Lab {
        fn check_destination(&self, d: &Destination<'_>) -> Result<(), Denial> {
            if self.names.contains(&d.host) {
                Ok(())
            } else {
                Err(Denial::NotAllowlisted)
            }
        }

        fn check_address(&self, address: IpAddr) -> Result<(), Denial> {
            match sagat_domain::egress::classify(address) {
                Some(AddressClass::Loopback) | None => Ok(()),
                Some(class) => Err(Denial::ForbiddenAddress(class)),
            }
        }
    }

    /// A one-route HTTP server that counts connections and answers `reply(request line)`.
    async fn server(
        reply: impl Fn(&str) -> String + Send + Sync + 'static,
    ) -> (u16, Arc<AtomicUsize>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let hits = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&hits);
        let reply = Arc::new(reply);
        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    return;
                };
                counter.fetch_add(1, Ordering::SeqCst);
                let reply = Arc::clone(&reply);
                tokio::spawn(async move {
                    let mut buffer = vec![0u8; 8192];
                    let n = socket.read(&mut buffer).await.unwrap_or(0);
                    let head = String::from_utf8_lossy(&buffer[..n]).into_owned();
                    let line = head.lines().next().unwrap_or_default().to_owned();
                    let _ = socket.write_all(reply(&line).as_bytes()).await;
                    let _ = socket.shutdown().await;
                });
            }
        });
        (port, hits)
    }

    fn ok(body: &str) -> String {
        format!(
            "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        )
    }

    fn get(url: String) -> EgressRequest {
        EgressRequest {
            method: Method::GET,
            url,
            headers: vec![],
            body: vec![],
        }
    }

    fn lab(names: &'static [&'static str], zone: Arc<Zone>, limits: EgressLimits) -> EgressClient {
        EgressClient::with(Arc::new(Lab { names }), zone, limits).expect("client")
    }

    #[tokio::test]
    async fn a_request_to_an_allowed_destination_goes_to_the_address_that_passed() {
        let (port, hits) = server(|_| ok("hello")).await;
        let client = lab(
            &["good.test"],
            zone(&[("good.test", &["127.0.0.1"])]),
            EgressLimits::default(),
        );
        let response = client
            .send(get(format!("http://good.test:{port}/path")))
            .await
            .expect("answered");
        assert_eq!(
            (response.status, response.body.as_slice()),
            (StatusCode::OK, &b"hello"[..])
        );
        assert_eq!(hits.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn one_forbidden_address_among_the_answers_refuses_the_whole_answer_before_connecting() {
        let (port, hits) = server(|_| ok("secret")).await;
        // The first address would work; the second is private. An attacker mixes them and hopes.
        let client = lab(
            &["rebind.test"],
            zone(&[("rebind.test", &["127.0.0.1", "10.0.0.5"])]),
            EgressLimits::default(),
        );
        let result = client
            .send(get(format!("http://rebind.test:{port}/")))
            .await;
        assert!(
            matches!(
                result,
                Err(EgressError::Denied(Denial::ForbiddenAddress(
                    AddressClass::Private
                )))
            ),
            "{result:?}"
        );
        assert_eq!(hits.load(Ordering::SeqCst), 0, "nothing was connected to");

        let empty = lab(&["void.test"], zone(&[]), EgressLimits::default());
        assert!(matches!(
            empty.send(get(format!("http://void.test:{port}/"))).await,
            Err(EgressError::Denied(Denial::NoAddress))
        ));
    }

    #[tokio::test]
    async fn the_real_policy_refuses_names_that_resolve_into_forbidden_space() {
        let (port, hits) = server(|_| ok("secret")).await;
        let policy = EgressPolicy::new(
            vec![HostRule::parse("*.corp.example").expect("rule")],
            vec![port],
            true,
        )
        .expect("policy");
        let hostile = zone(&[
            ("a.corp.example", &["127.0.0.1"]),
            ("b.corp.example", &["169.254.169.254"]),
            ("c.corp.example", &["10.1.2.3"]),
            ("d.corp.example", &["::ffff:127.0.0.1"]),
            ("e.corp.example", &["fd00:ec2::254"]),
            ("f.corp.example", &["100.100.100.200"]),
        ]);
        let client =
            EgressClient::with(Arc::new(policy), hostile, EgressLimits::default()).expect("client");
        for (host, class) in [
            ("a", AddressClass::Loopback),
            ("b", AddressClass::LinkLocal),
            ("c", AddressClass::Private),
            ("d", AddressClass::Loopback),
            ("e", AddressClass::Metadata),
            ("f", AddressClass::Metadata),
        ] {
            let result = client
                .send(get(format!("http://{host}.corp.example:{port}/")))
                .await;
            match result {
                Err(EgressError::Denied(Denial::ForbiddenAddress(found))) => {
                    assert_eq!(found, class, "{host}");
                }
                other => panic!("{host}: {other:?}"),
            }
        }
        assert_eq!(hits.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn address_literals_and_odd_hosts_are_refused_before_any_resolution_or_connection() {
        let (port, hits) = server(|_| ok("secret")).await;
        let policy = EgressPolicy::new(
            vec![HostRule::parse("api.example.com").expect("rule")],
            vec![port],
            true,
        )
        .expect("policy");
        let client = EgressClient::with(Arc::new(policy), zone(&[]), EgressLimits::default())
            .expect("client");
        for url in [
            format!("http://127.0.0.1:{port}/"),
            format!("http://[::1]:{port}/"),
            format!("http://169.254.169.254:{port}/latest/meta-data"),
            format!("http://10.0.0.1:{port}/"),
            format!("http://2130706433:{port}/"),
            format!("http://0x7f.1:{port}/"),
            format!("http://localhost:{port}/"),
            format!("http://api.example.com:{}/", port.wrapping_add(1)),
            "ftp://api.example.com/".to_owned(),
        ] {
            let result = client.send(get(url.clone())).await;
            assert!(
                matches!(
                    result,
                    Err(EgressError::Denied(_) | EgressError::InvalidUrl)
                ),
                "{url}: {result:?}"
            );
        }
        assert_eq!(
            hits.load(Ordering::SeqCst),
            0,
            "no connection was ever made"
        );
    }

    #[tokio::test]
    async fn credentials_fragments_and_hostless_urls_are_not_acceptable() {
        let client = lab(&["good.test"], zone(&[]), EgressLimits::default());
        for url in [
            "http://user:pw@good.test/",
            "http://user@good.test/",
            "http://good.test/#frag",
            "file:///etc/passwd",
            "not a url",
            "mailto:x@good.test",
        ] {
            assert!(
                matches!(
                    client.send(get(url.to_owned())).await,
                    Err(EgressError::InvalidUrl)
                ),
                "{url}"
            );
        }
    }

    #[tokio::test]
    async fn redirects_are_not_followed_by_default_and_every_hop_is_checked_when_they_are() {
        let (port, hits) = server(move |line| {
            if line.contains("/to-metadata") {
                "HTTP/1.1 302 Found\r\nlocation: http://meta.test/latest\r\ncontent-length: 0\r\nconnection: close\r\n\r\n".to_owned()
            } else if line.contains("/to-good") {
                "HTTP/1.1 302 Found\r\nlocation: /landed\r\ncontent-length: 0\r\nconnection: close\r\n\r\n".to_owned()
            } else if line.contains("/loop") {
                "HTTP/1.1 302 Found\r\nlocation: /loop\r\ncontent-length: 0\r\nconnection: close\r\n\r\n".to_owned()
            } else {
                ok("landed")
            }
        })
        .await;
        let names = zone(&[
            ("good.test", &["127.0.0.1"]),
            ("meta.test", &["169.254.169.254"]),
        ]);

        // Not followed: the redirect itself is the answer.
        let still = lab(
            &["good.test", "meta.test"],
            Arc::clone(&names),
            EgressLimits::default(),
        );
        let response = still
            .send(get(format!("http://good.test:{port}/to-good")))
            .await
            .expect("answered");
        assert_eq!(response.status, StatusCode::FOUND);

        let following = EgressLimits {
            max_redirects: 3,
            ..EgressLimits::default()
        };
        let client = lab(&["good.test", "meta.test"], Arc::clone(&names), following);
        let landed = client
            .send(get(format!("http://good.test:{port}/to-good")))
            .await
            .expect("followed");
        assert_eq!(landed.body, b"landed");

        // A redirect into forbidden space is refused at the hop, with no connection to it.
        let into = client
            .send(get(format!("http://good.test:{port}/to-metadata")))
            .await;
        assert!(
            matches!(
                into,
                Err(EgressError::Denied(Denial::ForbiddenAddress(
                    AddressClass::LinkLocal
                )))
            ),
            "{into:?}"
        );
        // …and a name the policy does not allow is refused before it is even resolved.
        let blocked = lab(&["good.test"], Arc::clone(&names), following);
        assert!(matches!(
            blocked
                .send(get(format!("http://good.test:{port}/to-metadata")))
                .await,
            Err(EgressError::Denied(Denial::NotAllowlisted))
        ));
        assert!(matches!(
            client
                .send(get(format!("http://good.test:{port}/loop")))
                .await,
            Err(EgressError::TooManyRedirects)
        ));
        assert!(hits.load(Ordering::SeqCst) > 0);

        // A POST never follows a redirect: the 302 is returned, nothing is re-sent elsewhere.
        let post = client
            .send(EgressRequest {
                method: Method::POST,
                url: format!("http://good.test:{port}/to-metadata"),
                headers: vec![("content-type".into(), "application/json".into())],
                body: b"{}".to_vec(),
            })
            .await
            .expect("answered");
        assert_eq!(post.status, StatusCode::FOUND);
    }

    #[tokio::test]
    async fn a_response_larger_than_the_limit_is_cut_off() {
        let (port, _) = server(|_| ok(&"x".repeat(4096))).await;
        let limits = EgressLimits {
            max_response_bytes: 1024,
            ..EgressLimits::default()
        };
        let client = lab(
            &["good.test"],
            zone(&[("good.test", &["127.0.0.1"])]),
            limits,
        );
        assert!(matches!(
            client.send(get(format!("http://good.test:{port}/"))).await,
            Err(EgressError::ResponseTooLarge)
        ));
    }

    #[tokio::test]
    async fn a_server_that_never_answers_ends_in_a_timeout() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let port = listener.local_addr().expect("addr").port();
        tokio::spawn(async move {
            let mut held = Vec::new();
            while let Ok((socket, _)) = listener.accept().await {
                held.push(socket); // accepted, never answered
            }
        });
        let limits = EgressLimits {
            total_timeout: Duration::from_millis(300),
            ..EgressLimits::default()
        };
        let client = lab(
            &["slow.test"],
            zone(&[("slow.test", &["127.0.0.1"])]),
            limits,
        );
        assert!(matches!(
            client.send(get(format!("http://slow.test:{port}/"))).await,
            Err(EgressError::Timeout)
        ));
    }
}
