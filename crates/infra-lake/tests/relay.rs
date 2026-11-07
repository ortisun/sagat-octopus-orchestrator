//! The relay of the lake runner (feature 035, T035-016, D035-049, D035-061, FR-035-004): the only route out of the
//! sandbox. The container reaches it over a Unix socket and must present the attempt's token. The relay allows the
//! hosts the connection declared and the regional endpoints of its storage type, resolves a name once, checks every
//! address of the answer, connects to the address it checked, and never reads the bytes of the tunnel (TLS is end to end).
//!
//! The resolver and the connector are injected, so a DNS rebinding needs no network and no sleep: the resolver is a
//! script, the connector records the address it was asked for, and time is paused where a timeout is the subject.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::VecDeque;
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::os::unix::fs::MetadataExt;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ipnet::IpNet;
use sagat_infra_lake::protocol::RelayToken;
use sagat_infra_lake::relay::{
    Connector, Endpoint, EndpointPolicy, Mapping, Relay, RelayConfig, RelayConfigError,
    RelayDenials, Resolver, SystemResolver, TcpConnector,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream, duplex};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

const TOKEN: &str = "relay-token-canary-0123456789abcdef";

// -- the doubles ------------------------------------------------------------------------------------------------------

/// A resolver that answers from a script and counts its calls.
#[derive(Clone, Default)]
struct Script {
    answers: Arc<Mutex<VecDeque<io::Result<Vec<IpAddr>>>>>,
    calls: Arc<AtomicUsize>,
    asked: Arc<Mutex<Vec<String>>>,
}

impl Script {
    fn answering(answers: Vec<Vec<&str>>) -> Self {
        let script = Self::default();
        for answer in answers {
            script.answers.lock().unwrap().push_back(Ok(answer
                .into_iter()
                .map(|text| text.parse().unwrap())
                .collect()));
        }
        script
    }

    fn failing() -> Self {
        let script = Self::default();
        script
            .answers
            .lock()
            .unwrap()
            .push_back(Err(io::Error::other("no such host")));
        script
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl Resolver for Script {
    async fn resolve(&self, host: &str, _port: u16) -> io::Result<Vec<IpAddr>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.asked.lock().unwrap().push(host.to_owned());
        self.answers
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| Ok(vec!["127.0.0.1".parse().unwrap()]))
    }
}

/// A connector that records the address it was asked for and hands the test the far end of the connection.
#[derive(Clone)]
struct Recording {
    connected: Arc<Mutex<Vec<SocketAddr>>>,
    remotes: Arc<tokio::sync::Mutex<mpsc::UnboundedReceiver<DuplexStream>>>,
    sender: mpsc::UnboundedSender<DuplexStream>,
}

impl Recording {
    fn new() -> Self {
        let (sender, receiver) = mpsc::unbounded_channel();
        Self {
            connected: Arc::default(),
            remotes: Arc::new(tokio::sync::Mutex::new(receiver)),
            sender,
        }
    }

    fn connected(&self) -> Vec<SocketAddr> {
        self.connected.lock().unwrap().clone()
    }

    async fn remote(&self) -> DuplexStream {
        self.remotes
            .lock()
            .await
            .recv()
            .await
            .expect("the relay connected")
    }
}

impl Connector for Recording {
    type Stream = DuplexStream;

    async fn connect(&self, address: SocketAddr) -> io::Result<DuplexStream> {
        self.connected.lock().unwrap().push(address);
        let (near, far) = duplex(64 * 1024);
        self.sender.send(far).unwrap();
        Ok(near)
    }
}

/// A resolver that never answers: a name server of the tenant's that is slow or black-holed.
struct Hang;

impl Resolver for Hang {
    async fn resolve(&self, _host: &str, _port: u16) -> io::Result<Vec<IpAddr>> {
        std::future::pending().await
    }
}

/// A connector whose connection never completes.
struct HangConnector;

impl Connector for HangConnector {
    type Stream = DuplexStream;

    async fn connect(&self, _address: SocketAddr) -> io::Result<DuplexStream> {
        std::future::pending().await
    }
}

// -- helpers ----------------------------------------------------------------------------------------------------------

fn endpoint(host: &str, port: u16) -> Endpoint {
    Endpoint::new(host, port).unwrap()
}

fn endpoints() -> EndpointPolicy {
    EndpointPolicy::new([
        endpoint("catalog.example", 443),
        endpoint("storage.example", 9000),
    ])
    .with_s3_regional("eu-west-1", &["lake"])
    .unwrap()
}

fn token() -> RelayToken {
    RelayToken::new(TOKEN).unwrap()
}

fn networks(list: &[&str]) -> Vec<IpNet> {
    list.iter().map(|net| net.parse().unwrap()).collect()
}

/// What an operator would allow in these tests: the documentation ranges, where the scripted answers live.
fn public_networks() -> Vec<IpNet> {
    networks(&["203.0.113.0/24", "2001:db8::/32"])
}

/// An operator who allows every address and leaves the exclusions of the policy to say no.
fn everywhere() -> Vec<IpNet> {
    networks(&["0.0.0.0/0", "::/0"])
}

fn config() -> RelayConfig {
    RelayConfig::new(token(), endpoints(), public_networks())
}

type TestRelay = Relay<Script, Recording>;

fn relay(config: RelayConfig, script: &Script, connector: &Recording) -> TestRelay {
    Relay::new(config, script.clone(), connector.clone()).unwrap()
}

/// One connection from the container, served by `handle` on the other end of an in-memory pipe.
struct Session {
    client: DuplexStream,
    served: JoinHandle<()>,
}

impl Session {
    fn open<R: Resolver, C: Connector>(relay: &Relay<R, C>) -> Self {
        let (client, server) = duplex(64 * 1024);
        let relay = relay.clone();
        let served =
            tokio::spawn(async move { relay.handle(server, &CancellationToken::new()).await });
        Self { client, served }
    }

    async fn send(&mut self, bytes: &[u8]) {
        self.client.write_all(bytes).await.unwrap();
    }

    async fn authenticate(&mut self) {
        self.send(format!("SAGAT-RELAY/1 {TOKEN}\n").as_bytes())
            .await;
    }

    /// What the relay answered until it closed the connection.
    async fn everything(mut self) -> String {
        let mut answer = Vec::new();
        self.client.read_to_end(&mut answer).await.unwrap();
        self.served.await.unwrap();
        String::from_utf8_lossy(&answer).into_owned()
    }

    async fn read_exactly(&mut self, length: usize) -> Vec<u8> {
        let mut bytes = vec![0; length];
        self.client.read_exact(&mut bytes).await.unwrap();
        bytes
    }
}

fn connect(target: &str) -> String {
    format!("CONNECT {target} HTTP/1.1\r\nHost: {target}\r\nProxy-Connection: keep-alive\r\n\r\n")
}

const ESTABLISHED: &str = "HTTP/1.1 200 Connection Established\r\n\r\n";

async fn denied<R: Resolver, C: Connector>(relay: &Relay<R, C>, request: &str) -> String {
    let mut session = Session::open(relay);
    session.authenticate().await;
    session.send(request.as_bytes()).await;
    session.everything().await
}

fn expect_counts(
    denials: RelayDenials,
    endpoint: u32,
    address: u32,
    resolution: u32,
    token: u32,
    unsupported: u32,
) {
    assert_eq!(
        (
            denials.endpoint_not_declared,
            denials.address_forbidden,
            denials.resolution_failed,
            denials.token_refused,
            denials.request_unsupported,
        ),
        (endpoint, address, resolution, token, unsupported),
        "(endpoint, address, resolution, token, unsupported) of {denials:?}"
    );
    assert_eq!(
        denials.total(),
        endpoint + address + resolution + token + unsupported
    );
}

// -- the token --------------------------------------------------------------------------------------------------------

#[tokio::test]
async fn a_connection_without_the_attempts_token_is_closed_in_silence_and_counted() {
    let script = Script::default();
    let connector = Recording::new();
    let relay = relay(config(), &script, &connector);
    let prefaces: Vec<(&str, Vec<u8>)> = vec![
        ("a wrong token", b"SAGAT-RELAY/1 not-the-token\n".to_vec()),
        (
            "a prefix of the token",
            format!("SAGAT-RELAY/1 {}\n", &TOKEN[..TOKEN.len() - 1]).into_bytes(),
        ),
        (
            "the token and more",
            format!("SAGAT-RELAY/1 {TOKEN}x\n").into_bytes(),
        ),
        ("no token", b"SAGAT-RELAY/1 \n".to_vec()),
        (
            "another protocol",
            format!("SAGAT-RELAY/2 {TOKEN}\n").into_bytes(),
        ),
        ("no protocol", format!("{TOKEN}\n").into_bytes()),
        (
            "an HTTP request",
            connect("catalog.example:443").into_bytes(),
        ),
        ("a preface that never ends", vec![b'a'; 300]),
        ("nothing at all", Vec::new()),
    ];
    let count = prefaces.len() as u32;

    for (name, preface) in prefaces {
        let mut session = Session::open(&relay);
        session.send(&preface).await;
        session.client.shutdown().await.unwrap();
        assert_eq!(session.everything().await, "", "{name}");
    }

    expect_counts(relay.denials(), 0, 0, 0, count, 0);
    assert_eq!((script.calls(), connector.connected().len()), (0, 0));
}

#[tokio::test]
async fn a_preface_above_the_limit_is_refused_without_waiting_for_its_end() {
    let script = Script::default();
    let connector = Recording::new();
    let relay = relay(config(), &script, &connector);
    let mut session = Session::open(&relay);

    // 300 bytes and no newline, and the client keeps the connection open: the relay must not wait for a line that
    // never ends or buffer it
    session.send(&[b'a'; 300]).await;

    tokio::time::timeout(Duration::from_secs(5), session.served)
        .await
        .expect("the relay gave up on the preface")
        .unwrap();
    expect_counts(relay.denials(), 0, 0, 0, 1, 0);
}

#[tokio::test(start_paused = true)]
async fn a_client_that_never_finishes_the_handshake_is_cut_off_and_counted() {
    let script = Script::default();
    let connector = Recording::new();
    let relay = relay(
        config().with_handshake_timeout(Duration::from_secs(5)),
        &script,
        &connector,
    );
    let mut session = Session::open(&relay);
    session.send(b"SAGAT-RELAY/1 relay-tok").await;

    tokio::time::advance(Duration::from_secs(6)).await;

    assert_eq!(session.everything().await, "");
    expect_counts(relay.denials(), 0, 0, 0, 1, 0);

    // a client that authenticates and then goes quiet before the request is cut off too
    let mut slow = Session::open(&relay);
    slow.authenticate().await;
    slow.send(b"CONNECT catalog.example:443 HTT").await;
    tokio::time::advance(Duration::from_secs(6)).await;
    slow.everything().await;
    assert_eq!(relay.denials().total(), 2);
}

// -- the endpoints ----------------------------------------------------------------------------------------------------

#[tokio::test]
async fn a_connect_to_a_host_the_connection_did_not_declare_is_refused_before_anything_is_resolved()
{
    let script = Script::default();
    let connector = Recording::new();
    let relay = relay(config(), &script, &connector);

    let answer = denied(&relay, &connect("attacker.example:443")).await;

    assert!(answer.starts_with("HTTP/1.1 403 "), "{answer}");
    assert!(
        !answer.contains("attacker"),
        "the answer does not repeat the target: {answer}"
    );
    expect_counts(relay.denials(), 1, 0, 0, 0, 0);
    assert_eq!((script.calls(), connector.connected().len()), (0, 0));
}

#[test]
fn only_the_declared_and_the_regional_endpoints_are_allowed_and_nothing_that_looks_like_them() {
    let policy = endpoints();
    let allowed = [
        ("catalog.example", 443),
        ("storage.example", 9000),
        ("s3.eu-west-1.amazonaws.com", 443),
        ("s3.dualstack.eu-west-1.amazonaws.com", 443),
        ("lake.s3.eu-west-1.amazonaws.com", 443),
        ("lake.s3.dualstack.eu-west-1.amazonaws.com", 443),
    ];
    for (host, port) in allowed {
        assert!(policy.allows(host, port), "{host}:{port}");
    }
    let refused = [
        ("catalog.example", 80),
        ("catalog.example", 9000),
        ("storage.example", 443),
        ("sub.catalog.example", 443),
        ("catalog.example.evil.example", 443),
        ("evilcatalog.example", 443),
        ("catalog.example", 0),
        ("s3.eu-west-2.amazonaws.com", 443),
        ("s3.eu-west-1.amazonaws.com", 80),
        ("s3.eu-west-1.amazonaws.com.evil.example", 443),
        ("evil-s3.eu-west-1.amazonaws.com", 443),
        ("other.s3.eu-west-1.amazonaws.com", 443),
        ("lake.s3.eu-west-2.amazonaws.com", 443),
        ("lake2.s3.eu-west-1.amazonaws.com", 443),
        ("s3.amazonaws.com", 443),
        ("amazonaws.com", 443),
        ("sts.eu-west-1.amazonaws.com", 443),
        ("169.254.169.254", 80),
        ("127.0.0.1", 9000),
        ("", 443),
        ("catalog.example..", 443),
        ("Attacker.Example.", 443),
        ("CATALOG.EXAMPLE.EVIL.EXAMPLE", 443),
        (".catalog.example", 443),
    ];
    for (host, port) in refused {
        assert!(!policy.allows(host, port), "{host}:{port}");
    }
}

#[test]
fn the_regional_endpoints_follow_the_region_and_the_buckets_of_the_connection() {
    let china = EndpointPolicy::new([])
        .with_s3_regional("cn-north-1", &["lake"])
        .unwrap();
    assert!(china.allows("s3.cn-north-1.amazonaws.com.cn", 443));
    assert!(china.allows("lake.s3.cn-north-1.amazonaws.com.cn", 443));
    assert!(!china.allows("s3.cn-north-1.amazonaws.com", 443));

    // a bucket with a dot cannot be addressed as a virtual host over TLS, so only the path style is allowed for it
    let dotted = EndpointPolicy::new([])
        .with_s3_regional("us-east-1", &["my.bucket"])
        .unwrap();
    assert!(dotted.allows("s3.us-east-1.amazonaws.com", 443));
    assert!(!dotted.allows("my.bucket.s3.us-east-1.amazonaws.com", 443));

    for bad in [
        "",
        "EU-WEST-1",
        "eu west 1",
        "eu-west-1.evil.example",
        "eu_west_1",
        "../x",
        "x",
        &"a".repeat(33),
    ] {
        assert!(
            EndpointPolicy::new([])
                .with_s3_regional(bad, &["lake"])
                .is_err(),
            "{bad:?}"
        );
    }
    for bad in [
        "",
        "Lake",
        "a",
        "lake/../x",
        "lake bucket",
        "my..bucket",
        "-lake",
        "lake-",
    ] {
        assert!(
            EndpointPolicy::new([])
                .with_s3_regional("eu-west-1", &[bad])
                .is_err(),
            "{bad:?}"
        );
    }
}

#[test]
fn an_endpoint_is_a_name_or_an_address_and_is_normalized() {
    assert_eq!(
        endpoint("Catalog.Example.", 443),
        endpoint("catalog.example", 443)
    );
    assert_eq!(
        endpoint("[2001:db8:0:0:0:0:0:1]", 443),
        endpoint("2001:db8::1", 443)
    );
    for bad in [
        "",
        " ",
        "a b",
        "a..b",
        "-a.example",
        "a-.example",
        "a_b.example",
        "user@host",
        "host:443",
        "a/b",
        "ü.example",
        &"a".repeat(64),
        "http://x",
    ] {
        assert!(Endpoint::new(bad, 443).is_err(), "{bad:?}");
    }
    assert!(Endpoint::new("catalog.example", 0).is_err());
}

#[tokio::test]
async fn a_name_is_matched_in_lower_case_without_its_trailing_dot() {
    let script = Script::answering(vec![vec!["203.0.113.7"]]);
    let connector = Recording::new();
    let relay = relay(config(), &script, &connector);
    let mut session = Session::open(&relay);
    session.authenticate().await;

    session
        .send(connect("CATALOG.Example.:443").as_bytes())
        .await;

    assert_eq!(
        session.read_exactly(ESTABLISHED.len()).await,
        ESTABLISHED.as_bytes()
    );
    assert_eq!(
        script.asked.lock().unwrap().as_slice(),
        ["catalog.example."]
    );
}

// -- tunnels ----------------------------------------------------------------------------------------------------------

#[tokio::test]
async fn a_tunnel_carries_the_bytes_in_both_directions_untouched() {
    let script = Script::answering(vec![vec!["203.0.113.7"]]);
    let connector = Recording::new();
    let relay = relay(config(), &script, &connector);
    let mut session = Session::open(&relay);
    session.authenticate().await;
    session
        .send(connect("catalog.example:443").as_bytes())
        .await;
    assert_eq!(
        session.read_exactly(ESTABLISHED.len()).await,
        ESTABLISHED.as_bytes()
    );
    let mut remote = connector.remote().await;

    // the bytes of a TLS ClientHello, then binary that no HTTP or TLS parser would accept: the relay reads none of it
    let mut hello = vec![
        0x16, 0x03, 0x01, 0x02, 0x00, 0x01, 0x00, 0x01, 0xfc, 0x03, 0x03,
    ];
    hello.extend((0..4096_u32).map(|n| (n.wrapping_mul(2_654_435_761) >> 13) as u8));
    session.send(&hello).await;
    let mut received = vec![0; hello.len()];
    remote.read_exact(&mut received).await.unwrap();
    assert_eq!(received, hello);

    let reply: Vec<u8> = (0..=255).rev().collect();
    remote.write_all(&reply).await.unwrap();
    assert_eq!(session.read_exactly(reply.len()).await, reply);

    // when the far side closes, the container sees the end of the stream
    drop(remote);
    let mut rest = Vec::new();
    session.client.read_to_end(&mut rest).await.unwrap();
    assert!(rest.is_empty());
    // a tunnel ends when both directions have: the container closes its own side too
    session.client.shutdown().await.unwrap();
    session.served.await.unwrap();
    assert_eq!(
        connector.connected(),
        vec!["203.0.113.7:443".parse::<SocketAddr>().unwrap()]
    );
    expect_counts(relay.denials(), 0, 0, 0, 0, 0);
}

#[tokio::test]
async fn bytes_sent_right_behind_the_request_are_not_lost() {
    let script = Script::answering(vec![vec!["203.0.113.7"]]);
    let connector = Recording::new();
    let relay = relay(config(), &script, &connector);
    let mut session = Session::open(&relay);
    session.authenticate().await;

    // a client that does not wait for the 200 before it starts the TLS handshake
    let mut pipelined = connect("catalog.example:443").into_bytes();
    pipelined.extend_from_slice(b"\x16\x03\x01early-bytes");
    session.send(&pipelined).await;

    let mut remote = connector.remote().await;
    let early: &[u8] = b"\x16\x03\x01early-bytes";
    let mut received = vec![0; early.len()];
    remote.read_exact(&mut received).await.unwrap();
    assert_eq!(received, early);
}

#[tokio::test]
async fn a_mixed_answer_is_connected_to_on_its_ipv4_address() {
    let script = Script::answering(vec![vec!["2001:db8::1", "203.0.113.7"]]);
    let connector = Recording::new();
    let relay = relay(config(), &script, &connector);
    let mut session = Session::open(&relay);
    session.authenticate().await;

    session
        .send(connect("catalog.example:443").as_bytes())
        .await;

    assert_eq!(
        session.read_exactly(ESTABLISHED.len()).await,
        ESTABLISHED.as_bytes()
    );
    assert_eq!(
        connector.connected(),
        vec!["203.0.113.7:443".parse::<SocketAddr>().unwrap()]
    );
}

#[tokio::test]
async fn the_relay_connects_to_the_address_it_checked_and_resolves_only_once() {
    // the first answer is public; a second resolution would hand the relay the loopback address (DNS rebinding)
    let script = Script::answering(vec![
        vec!["203.0.113.7"],
        vec!["127.0.0.1"],
        vec!["169.254.169.254"],
    ]);
    let connector = Recording::new();
    let relay = relay(config(), &script, &connector);
    let mut session = Session::open(&relay);
    session.authenticate().await;
    session
        .send(connect("catalog.example:443").as_bytes())
        .await;
    assert_eq!(
        session.read_exactly(ESTABLISHED.len()).await,
        ESTABLISHED.as_bytes()
    );

    assert_eq!(script.calls(), 1);
    assert_eq!(
        connector.connected(),
        vec!["203.0.113.7:443".parse::<SocketAddr>().unwrap()]
    );
}

#[tokio::test]
async fn an_answer_with_one_forbidden_address_is_refused_as_a_whole() {
    let script = Script::answering(vec![vec!["203.0.113.7", "169.254.169.254"]]);
    let connector = Recording::new();
    let relay = relay(config(), &script, &connector);

    let answer = denied(&relay, &connect("catalog.example:443")).await;

    assert!(answer.starts_with("HTTP/1.1 403 "), "{answer}");
    expect_counts(relay.denials(), 0, 1, 0, 0, 0);
    assert_eq!(connector.connected().len(), 0);
}

#[tokio::test]
async fn loopback_link_local_metadata_and_platform_state_addresses_are_refused_for_a_declared_name()
{
    let platform_state: ipnet::IpNet = "10.20.0.0/16".parse().unwrap();
    let forbidden = [
        "127.0.0.1",
        "127.0.0.2",
        "::1",
        "169.254.169.254",
        "169.254.0.5",
        "fe80::1",
        "fd00:ec2::254",
        "fd20:ce::254",
        "100.100.100.200",
        "168.63.129.16",
        "0.0.0.0",
        "224.0.0.1",
        "2002::1",
        "10.20.1.2",
        "::ffff:10.20.1.2",
        "::ffff:169.254.169.254",
        "::ffff:127.0.0.1",
    ];
    for address in forbidden {
        let script = Script::answering(vec![vec![address]]);
        let connector = Recording::new();
        let relay = relay(
            RelayConfig::new(token(), endpoints(), everywhere()).with_deny([platform_state]),
            &script,
            &connector,
        );

        let answer = denied(&relay, &connect("catalog.example:443")).await;

        assert!(answer.starts_with("HTTP/1.1 403 "), "{address}: {answer}");
        expect_counts(relay.denials(), 0, 1, 0, 0, 0);
        assert_eq!(connector.connected().len(), 0, "{address}");
    }
}

#[tokio::test]
async fn development_allows_loopback_and_still_refuses_the_metadata_services() {
    for (address, allowed) in [
        ("127.0.0.1", true),
        ("::1", true),
        ("169.254.169.254", false),
        ("fd00:ec2::254", false),
    ] {
        let script = Script::answering(vec![vec![address]]);
        let connector = Recording::new();
        let relay = relay(
            RelayConfig::new(token(), endpoints(), everywhere()).development(true),
            &script,
            &connector,
        );

        let mut session = Session::open(&relay);
        session.authenticate().await;
        session
            .send(connect("catalog.example:443").as_bytes())
            .await;
        // "HTTP/1.1 200" and "HTTP/1.1 403" have the same length; an allowed tunnel stays open, so the answer is read by length
        let status = session.read_exactly(12).await;

        assert_eq!(
            status == b"HTTP/1.1 200",
            allowed,
            "{address}: {}",
            String::from_utf8_lossy(&status)
        );
        assert_eq!(
            connector.connected().len(),
            usize::from(allowed),
            "{address}"
        );
    }
}

#[tokio::test]
async fn a_name_that_does_not_resolve_is_refused_and_counted() {
    for script in [Script::failing(), Script::answering(vec![vec![]])] {
        let connector = Recording::new();
        let relay = relay(config(), &script, &connector);

        let answer = denied(&relay, &connect("catalog.example:443")).await;

        assert!(answer.starts_with("HTTP/1.1 502 "), "{answer}");
        expect_counts(relay.denials(), 0, 0, 1, 0, 0);
        assert_eq!(connector.connected().len(), 0);
    }
}

#[tokio::test]
async fn an_address_literal_needs_to_be_declared_and_is_not_resolved() {
    let script = Script::default();
    let connector = Recording::new();
    let policy = EndpointPolicy::new([endpoint("127.0.0.1", 9000), endpoint("2001:db8::1", 443)]);
    let relay = relay(
        RelayConfig::new(token(), policy, everywhere()).development(true),
        &script,
        &connector,
    );

    // not declared
    let answer = denied(&relay, &connect("169.254.169.254:80")).await;
    assert!(answer.starts_with("HTTP/1.1 403 "), "{answer}");
    expect_counts(relay.denials(), 1, 0, 0, 0, 0);

    // declared: checked against the policy and connected to, with no resolution
    let mut session = Session::open(&relay);
    session.authenticate().await;
    session.send(connect("127.0.0.1:9000").as_bytes()).await;
    assert_eq!(
        session.read_exactly(ESTABLISHED.len()).await,
        ESTABLISHED.as_bytes()
    );
    let mut v6 = Session::open(&relay);
    v6.authenticate().await;
    v6.send(connect("[2001:db8::1]:443").as_bytes()).await;
    assert_eq!(
        v6.read_exactly(ESTABLISHED.len()).await,
        ESTABLISHED.as_bytes()
    );
    assert_eq!(script.calls(), 0);
    assert_eq!(
        connector.connected(),
        vec![
            "127.0.0.1:9000".parse::<SocketAddr>().unwrap(),
            "[2001:db8::1]:443".parse().unwrap()
        ]
    );
}

// -- tcp mappings -----------------------------------------------------------------------------------------------------

fn with_mappings() -> RelayConfig {
    config().with_mappings([
        Mapping::new("metastore", endpoint("hive.example", 9083)),
        Mapping::new("catalog-db", endpoint("db.example", 5432)),
    ])
}

#[tokio::test]
async fn a_tcp_mapping_reaches_its_declared_destination_with_the_same_checks() {
    let script = Script::answering(vec![vec!["203.0.113.9"]]);
    let connector = Recording::new();
    let relay = relay(with_mappings(), &script, &connector);
    let mut session = Session::open(&relay);
    session.authenticate().await;
    session.send(b"TCP metastore\n").await;

    assert_eq!(session.read_exactly(3).await, b"OK\n");
    let mut remote = connector.remote().await;
    session.send(b"\x80\x01\x00\x01thrift").await;
    let mut received = vec![0; 10];
    remote.read_exact(&mut received).await.unwrap();
    assert_eq!(received, b"\x80\x01\x00\x01thrift");
    assert_eq!(script.asked.lock().unwrap().as_slice(), ["hive.example."]);
    assert_eq!(
        connector.connected(),
        vec!["203.0.113.9:9083".parse::<SocketAddr>().unwrap()]
    );
}

#[tokio::test]
async fn a_tcp_mapping_that_is_unknown_or_resolves_to_a_forbidden_address_is_refused() {
    let script = Script::default();
    let connector = Recording::new();
    let relay = relay(with_mappings(), &script, &connector);
    for name in [
        "unknown",
        "Metastore",
        "../metastore",
        "metastore ",
        "",
        "metastore\r",
        "a".repeat(200).as_str(),
    ] {
        let answer = denied(&relay, &format!("TCP {name}\n")).await;
        assert!(
            answer == "DENIED\n" || answer.starts_with("HTTP/1.1 400"),
            "{name:?}: {answer:?}"
        );
    }
    assert_eq!(connector.connected().len(), 0);
    assert_eq!(script.calls(), 0);

    let rebinding = Script::answering(vec![vec!["10.0.0.5", "169.254.169.254"]]);
    let relay = self::relay(with_mappings(), &rebinding, &connector);
    assert_eq!(denied(&relay, "TCP catalog-db\n").await, "DENIED\n");
    expect_counts(relay.denials(), 0, 1, 0, 0, 0);
}

// -- requests the relay does not serve --------------------------------------------------------------------------------

#[tokio::test]
async fn anything_but_connect_and_a_mapping_is_unsupported() {
    let script = Script::default();
    let connector = Recording::new();
    let relay = relay(with_mappings(), &script, &connector);
    let cases: Vec<(&str, String)> = vec![
        (
            "a GET",
            "GET http://catalog.example/ HTTP/1.1\r\nHost: catalog.example\r\n\r\n".to_owned(),
        ),
        ("a POST", "POST / HTTP/1.1\r\n\r\n".to_owned()),
        (
            "an HTTP/2 preface",
            "PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n".to_owned(),
        ),
        (
            "an absolute-form target",
            "CONNECT http://catalog.example:443/ HTTP/1.1\r\n\r\n".to_owned(),
        ),
        (
            "a target with user info",
            "CONNECT user@catalog.example:443 HTTP/1.1\r\n\r\n".to_owned(),
        ),
        (
            "a target without a port",
            "CONNECT catalog.example HTTP/1.1\r\n\r\n".to_owned(),
        ),
        (
            "a port that is not a number",
            "CONNECT catalog.example:https HTTP/1.1\r\n\r\n".to_owned(),
        ),
        (
            "a port of zero",
            "CONNECT catalog.example:0 HTTP/1.1\r\n\r\n".to_owned(),
        ),
        (
            "a port above 65535",
            "CONNECT catalog.example:65536 HTTP/1.1\r\n\r\n".to_owned(),
        ),
        (
            "a port with a sign",
            "CONNECT catalog.example:+443 HTTP/1.1\r\n\r\n".to_owned(),
        ),
        (
            "an IPv6 authority without brackets",
            "CONNECT ::1:443 HTTP/1.1\r\n\r\n".to_owned(),
        ),
        (
            "a longer IPv6 authority without brackets",
            "CONNECT 2001:db8::1:443 HTTP/1.1\r\n\r\n".to_owned(),
        ),
        (
            "an unclosed IPv6 literal",
            "CONNECT [2001:db8::1:443 HTTP/1.1\r\n\r\n".to_owned(),
        ),
        (
            "a space in the target",
            "CONNECT catalog.example :443 HTTP/1.1\r\n\r\n".to_owned(),
        ),
        (
            "no HTTP version",
            "CONNECT catalog.example:443\r\n\r\n".to_owned(),
        ),
        (
            "another HTTP version",
            "CONNECT catalog.example:443 HTTP/3\r\n\r\n".to_owned(),
        ),
        (
            "a lower-case method",
            "connect catalog.example:443 HTTP/1.1\r\n\r\n".to_owned(),
        ),
        (
            "a header section above the limit",
            format!(
                "CONNECT catalog.example:443 HTTP/1.1\r\nX: {}\r\n\r\n",
                "a".repeat(9000)
            ),
        ),
        (
            "a request line above the limit",
            format!("CONNECT {}:443 HTTP/1.1\r\n\r\n", "a".repeat(9000)),
        ),
        (
            "the control word with something after it",
            "CONTROL now\n".to_owned(),
        ),
        ("the control word in lower case", "control\n".to_owned()),
        (
            "the control word ended like an HTTP line",
            "CONTROL\r\n".to_owned(),
        ),
        ("an empty request", "\n".to_owned()),
        (
            "a request line ended by a bare newline",
            "CONNECT catalog.example:443 HTTP/1.1\n\r\n".to_owned(),
        ),
        (
            "a header ended by a bare newline",
            "CONNECT catalog.example:443 HTTP/1.1\r\nHost: x\n\r\n".to_owned(),
        ),
    ];
    let count = cases.len() as u32;

    for (name, request) in cases {
        let answer = denied(&relay, &request).await;
        assert!(
            answer.starts_with("HTTP/1.1 400 ") || answer == "DENIED\n",
            "{name}: {answer:?}"
        );
    }

    expect_counts(relay.denials(), 0, 0, 0, 0, count);
    assert_eq!((script.calls(), connector.connected().len()), (0, 0));
}

// -- the control conversation (D035-110) ------------------------------------------------------------------------------

/// The relay serves proxy requests and hands one connection, the host's `CONTROL`, to the attempt driver. The driver
/// holds the receiver from the start; the first authenticated `CONTROL` is sent through it and is the only one.
#[tokio::test]
async fn the_first_control_request_is_handed_to_the_driver_with_the_connection_open() {
    let relay = relay(config(), &Script::default(), &Recording::new());
    let control = relay.take_control().unwrap();

    let mut session = Session::open(&relay);
    session.authenticate().await;
    session.send(b"CONTROL\n").await;
    let mut driver = control.await.unwrap();

    // the relay's own task is over, and the conversation goes on between the host and the driver both ways
    (&mut session.served).await.unwrap();
    driver.write_all(b"request\n").await.unwrap();
    assert_eq!(session.read_exactly(8).await, b"request\n");
    session.send(b"{\"heartbeat\":true}\n").await;
    let mut heard = vec![0; 19];
    driver.read_exact(&mut heard).await.unwrap();
    assert_eq!(heard, b"{\"heartbeat\":true}\n");
    expect_counts(relay.denials(), 0, 0, 0, 0, 0);
}

#[tokio::test]
async fn a_second_control_request_is_refused_and_counted() {
    let relay = relay(config(), &Script::default(), &Recording::new());
    let control = relay.take_control().unwrap();
    let mut first = Session::open(&relay);
    first.authenticate().await;
    first.send(b"CONTROL\n").await;
    let _driver = control.await.unwrap();

    let mut second = Session::open(&relay);
    second.authenticate().await;
    second.send(b"CONTROL\n").await;
    // closed with nothing said: a second host process learns nothing
    assert_eq!(second.everything().await, "");
    expect_counts(relay.denials(), 0, 0, 0, 0, 1);
}

#[tokio::test]
async fn a_control_request_without_the_token_is_not_handed_over() {
    let relay = relay(config(), &Script::default(), &Recording::new());
    let mut control = relay.take_control().unwrap();

    let mut session = Session::open(&relay);
    session
        .send(b"SAGAT-RELAY/1 not-the-token-not-the-token-not-the-token\nCONTROL\n")
        .await;
    assert_eq!(session.everything().await, "");

    // still waiting for a host that has the token: the sender was not used
    assert!(matches!(
        control.try_recv(),
        Err(tokio::sync::oneshot::error::TryRecvError::Empty)
    ));
    expect_counts(relay.denials(), 0, 0, 0, 1, 0);
}

#[tokio::test]
async fn the_driver_takes_the_control_receiver_once() {
    let relay = relay(config(), &Script::default(), &Recording::new());
    assert!(relay.take_control().is_some());
    assert!(relay.take_control().is_none());
}

#[tokio::test]
async fn a_host_that_connects_after_the_driver_stopped_waiting_is_closed_not_counted() {
    let relay = relay(config(), &Script::default(), &Recording::new());
    drop(relay.take_control().unwrap());

    let mut session = Session::open(&relay);
    session.authenticate().await;
    session.send(b"CONTROL\n").await;
    assert_eq!(session.everything().await, "");
    // the attempt is over; that is not the sandbox misbehaving
    expect_counts(relay.denials(), 0, 0, 0, 0, 0);
}

#[tokio::test]
async fn the_tunnels_keep_working_beside_the_control_conversation() {
    let script = Script::answering(vec![vec!["203.0.113.7"]]);
    let connector = Recording::new();
    let relay = relay(config(), &script, &connector);
    let control = relay.take_control().unwrap();

    let mut host = Session::open(&relay);
    host.authenticate().await;
    host.send(b"CONTROL\n").await;
    let _driver = control.await.unwrap();

    let mut forwarder = Session::open(&relay);
    forwarder.authenticate().await;
    forwarder
        .send(connect("catalog.example:443").as_bytes())
        .await;
    assert_eq!(
        forwarder.read_exactly(ESTABLISHED.len()).await,
        ESTABLISHED.as_bytes()
    );
    assert_eq!(connector.connected().len(), 1);
}

// -- the real socket --------------------------------------------------------------------------------------------------

#[tokio::test]
async fn over_a_unix_socket_a_tunnel_reaches_a_real_tcp_server_and_shutdown_closes_it() {
    let echo = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = echo.local_addr().unwrap().port();
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = echo.accept().await else {
                return;
            };
            tokio::spawn(async move {
                let (mut read, mut write) = stream.split();
                let _ = tokio::io::copy(&mut read, &mut write).await;
            });
        }
    });
    let directory = tempfile::tempdir().unwrap();
    let own_uid = std::fs::metadata(directory.path()).unwrap().uid();
    let path = directory.path().join("relay.sock");
    let listener = tokio::net::UnixListener::bind(&path).unwrap();
    let policy = EndpointPolicy::new([endpoint("fixture.example", port)]);
    let config = RelayConfig::new(token(), policy, everywhere())
        .development(true)
        .with_peer_uid(own_uid);
    let script = Script::answering(vec![vec!["127.0.0.1"]]);
    let relay = Relay::new(config, script, TcpConnector::new(Duration::from_secs(5))).unwrap();
    let shutdown = CancellationToken::new();
    let server = tokio::spawn({
        let relay = relay.clone();
        let shutdown = shutdown.clone();
        async move { relay.serve(listener, shutdown).await }
    });

    let mut stream = tokio::net::UnixStream::connect(&path).await.unwrap();
    stream
        .write_all(format!("SAGAT-RELAY/1 {TOKEN}\n").as_bytes())
        .await
        .unwrap();
    stream
        .write_all(connect(&format!("fixture.example:{port}")).as_bytes())
        .await
        .unwrap();
    let mut established = vec![0; ESTABLISHED.len()];
    stream.read_exact(&mut established).await.unwrap();
    assert_eq!(established, ESTABLISHED.as_bytes());
    stream
        .write_all(b"\x16\x03\x01over the real socket")
        .await
        .unwrap();
    let mut echoed = vec![0; b"\x16\x03\x01over the real socket".len()];
    stream.read_exact(&mut echoed).await.unwrap();
    assert_eq!(echoed, b"\x16\x03\x01over the real socket");

    // a wrong token on the same socket is refused and counted
    let mut wrong = tokio::net::UnixStream::connect(&path).await.unwrap();
    wrong.write_all(b"SAGAT-RELAY/1 wrong\n").await.unwrap();
    let mut nothing = Vec::new();
    wrong.read_to_end(&mut nothing).await.unwrap();
    assert!(nothing.is_empty());
    assert_eq!(relay.denials().token_refused, 1);

    // shutdown ends the serve loop and the open tunnel
    shutdown.cancel();
    server.await.unwrap();
    let mut rest = Vec::new();
    stream.read_to_end(&mut rest).await.unwrap();
    assert!(rest.is_empty());
}

#[tokio::test]
async fn the_system_resolver_resolves_a_literal_and_localhost_and_fails_for_nothing() {
    let resolver = SystemResolver;

    assert_eq!(
        resolver.resolve("127.0.0.1", 80).await.unwrap(),
        vec!["127.0.0.1".parse::<IpAddr>().unwrap()]
    );
    assert!(
        resolver
            .resolve("localhost", 80)
            .await
            .unwrap()
            .iter()
            .any(|address| address.is_loopback())
    );
    assert!(resolver.resolve("no-such-host.invalid", 80).await.is_err());
}

// -- the operator's allowed networks (review of the relay, finding 1) ------------------------------------------------

#[tokio::test]
async fn an_address_outside_the_networks_the_operator_allowed_is_refused_even_when_nothing_else_forbids_it()
 {
    // private, shared, unique-local and public addresses that `prohibited` does not name
    let outside = [
        "10.0.0.5",
        "192.168.1.1",
        "172.16.0.1",
        "100.64.0.1",
        "10.96.0.1",
        "fd12::1",
        "8.8.8.8",
        "2606:4700::1",
    ];
    for address in outside {
        let script = Script::answering(vec![vec![address]]);
        let connector = Recording::new();
        // no `with_deny`: the allowed networks alone keep the sandbox out of the cluster
        let relay = relay(config(), &script, &connector);

        let answer = denied(&relay, &connect("catalog.example:443")).await;

        assert!(answer.starts_with("HTTP/1.1 403 "), "{address}: {answer}");
        expect_counts(relay.denials(), 0, 1, 0, 0, 0);
        assert_eq!(connector.connected().len(), 0, "{address}");
    }
}

#[tokio::test]
async fn a_relay_with_no_allowed_network_reaches_nothing() {
    let script = Script::answering(vec![vec!["203.0.113.7"]]);
    let connector = Recording::new();
    let relay = relay(
        RelayConfig::new(token(), endpoints(), []),
        &script,
        &connector,
    );

    let answer = denied(&relay, &connect("catalog.example:443")).await;

    assert!(answer.starts_with("HTTP/1.1 403 "), "{answer}");
    expect_counts(relay.denials(), 0, 1, 0, 0, 0);
}

// -- the address that is connected to ---------------------------------------------------------------------------------

#[tokio::test]
async fn an_ipv4_mapped_answer_is_connected_to_as_the_ipv4_address_it_was_checked_as() {
    let script = Script::answering(vec![vec!["::ffff:203.0.113.7"]]);
    let connector = Recording::new();
    let relay = relay(
        RelayConfig::new(token(), endpoints(), everywhere()),
        &script,
        &connector,
    );
    let mut session = Session::open(&relay);
    session.authenticate().await;

    session
        .send(connect("catalog.example:443").as_bytes())
        .await;

    assert_eq!(
        session.read_exactly(ESTABLISHED.len()).await,
        ESTABLISHED.as_bytes()
    );
    assert_eq!(
        connector.connected(),
        vec!["203.0.113.7:443".parse::<SocketAddr>().unwrap()]
    );
}

#[tokio::test]
async fn a_tcp_mapping_that_resolves_to_a_mapped_metadata_address_is_refused() {
    for address in [
        "::ffff:169.254.169.254",
        "::ffff:127.0.0.1",
        "fd00:ec2::254",
    ] {
        let script = Script::answering(vec![vec![address]]);
        let connector = Recording::new();
        let relay = relay(with_mappings(), &script, &connector);

        assert_eq!(
            denied(&relay, "TCP metastore\n").await,
            "DENIED\n",
            "{address}"
        );
        expect_counts(relay.denials(), 0, 1, 0, 0, 0);
        assert_eq!(connector.connected().len(), 0, "{address}");
    }
}

// -- time, shutdown and resources (review of the relay, findings 2 and 3) --------------------------------------------

#[tokio::test(start_paused = true)]
async fn a_name_server_that_never_answers_is_given_up_on_and_counted() {
    let connector = Recording::new();
    let relay = Relay::new(
        config().with_admission_timeout(Duration::from_secs(5)),
        Hang,
        connector.clone(),
    )
    .unwrap();
    let mut session = Session::open(&relay);
    session.authenticate().await;
    session
        .send(connect("catalog.example:443").as_bytes())
        .await;

    tokio::time::advance(Duration::from_secs(6)).await;

    let answer = session.everything().await;
    assert!(answer.starts_with("HTTP/1.1 502 "), "{answer}");
    expect_counts(relay.denials(), 0, 0, 1, 0, 0);
    assert_eq!(connector.connected().len(), 0);
}

#[tokio::test(start_paused = true)]
async fn a_destination_that_never_accepts_is_given_up_on_without_a_denial() {
    let script = Script::answering(vec![vec!["203.0.113.7"]]);
    let relay = Relay::new(
        config().with_admission_timeout(Duration::from_secs(5)),
        script,
        HangConnector,
    )
    .unwrap();
    let mut session = Session::open(&relay);
    session.authenticate().await;
    session
        .send(connect("catalog.example:443").as_bytes())
        .await;

    tokio::time::advance(Duration::from_secs(6)).await;

    let answer = session.everything().await;
    assert!(answer.starts_with("HTTP/1.1 502 "), "{answer}");
    expect_counts(relay.denials(), 0, 0, 0, 0, 0);
}

#[tokio::test]
async fn shutdown_ends_a_connection_that_is_still_being_admitted() {
    let relay = Relay::new(config(), Hang, Recording::new()).unwrap();
    let shutdown = CancellationToken::new();
    let (mut client, server) = duplex(64 * 1024);
    let served = tokio::spawn({
        let relay = relay.clone();
        let shutdown = shutdown.clone();
        async move { relay.handle(server, &shutdown).await }
    });
    client
        .write_all(format!("SAGAT-RELAY/1 {TOKEN}\n").as_bytes())
        .await
        .unwrap();
    client
        .write_all(connect("catalog.example:443").as_bytes())
        .await
        .unwrap();

    shutdown.cancel();

    tokio::time::timeout(Duration::from_secs(5), served)
        .await
        .expect("the relay stopped waiting for the name server")
        .unwrap();
}

#[tokio::test]
async fn at_most_the_configured_number_of_connections_are_served_at_once() {
    let script = Script::answering(vec![vec!["203.0.113.7"]; 3]);
    let connector = Recording::new();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("relay.sock");
    let listener = tokio::net::UnixListener::bind(&path).unwrap();
    let relay = relay(config().with_max_connections(2), &script, &connector);
    let shutdown = CancellationToken::new();
    let server = tokio::spawn({
        let relay = relay.clone();
        let shutdown = shutdown.clone();
        async move { relay.serve(listener, shutdown).await }
    });
    let open = |path: std::path::PathBuf| async move {
        let mut stream = tokio::net::UnixStream::connect(path).await.unwrap();
        stream
            .write_all(format!("SAGAT-RELAY/1 {TOKEN}\n").as_bytes())
            .await
            .unwrap();
        stream
            .write_all(connect("catalog.example:443").as_bytes())
            .await
            .unwrap();
        stream
    };
    let established = |mut stream: tokio::net::UnixStream| async move {
        let mut answer = vec![0; ESTABLISHED.len()];
        stream.read_exact(&mut answer).await.unwrap();
        assert_eq!(answer, ESTABLISHED.as_bytes());
        stream
    };

    let first = established(open(path.clone()).await).await;
    let far_first = connector.remote().await;
    let second = established(open(path.clone()).await).await;
    let mut far_second = connector.remote().await;
    assert_eq!(relay.open_connections(), 2);

    // a third client is accepted by the kernel and is not served: a round trip on another tunnel gives the relay every
    // chance to take it
    let third = open(path.clone()).await;
    let mut second = second;
    second.write_all(b"ping").await.unwrap();
    let mut ping = [0; 4];
    far_second.read_exact(&mut ping).await.unwrap();
    assert_eq!(connector.connected().len(), 2);
    assert_eq!(relay.open_connections(), 2);

    // when one ends, the third is served
    drop(first);
    drop(far_first);
    let _third = established(third).await;
    assert_eq!(connector.connected().len(), 3);

    shutdown.cancel();
    server.await.unwrap();
    assert_eq!(relay.open_connections(), 0);
}

// -- who may connect (review of the relay, finding 7) ----------------------------------------------------------------

#[tokio::test]
async fn a_peer_with_another_user_id_is_refused_before_a_byte_of_it_is_read() {
    let directory = tempfile::tempdir().unwrap();
    let own_uid = std::fs::metadata(directory.path()).unwrap().uid();
    let path = directory.path().join("relay.sock");
    let listener = tokio::net::UnixListener::bind(&path).unwrap();
    let script = Script::answering(vec![vec!["203.0.113.7"]]);
    let connector = Recording::new();
    let relay = relay(
        config().with_peer_uid(own_uid.wrapping_add(1)),
        &script,
        &connector,
    );
    let shutdown = CancellationToken::new();
    let server = tokio::spawn({
        let relay = relay.clone();
        let shutdown = shutdown.clone();
        async move { relay.serve(listener, shutdown).await }
    });

    let mut stream = tokio::net::UnixStream::connect(&path).await.unwrap();
    // the token is right and the destination is declared: the user id is what is wrong
    let _ = stream
        .write_all(format!("SAGAT-RELAY/1 {TOKEN}\n").as_bytes())
        .await;
    let _ = stream
        .write_all(connect("catalog.example:443").as_bytes())
        .await;
    let mut answer = Vec::new();
    stream.read_to_end(&mut answer).await.unwrap_or_default();

    assert!(answer.is_empty(), "{answer:?}");
    expect_counts(relay.denials(), 0, 0, 0, 1, 0);
    assert_eq!((script.calls(), connector.connected().len()), (0, 0));
    shutdown.cancel();
    server.await.unwrap();
}

// -- a relay that is configured wrongly is not built -----------------------------------------------------------------

#[test]
fn a_relay_with_a_limit_of_zero_is_not_built() {
    for config in [
        config().with_max_connections(0),
        config().with_handshake_timeout(Duration::ZERO),
        config().with_admission_timeout(Duration::ZERO),
    ] {
        let built = Relay::new(config, Script::default(), Recording::new());
        assert!(matches!(built, Err(RelayConfigError::Limit)));
    }
}

#[test]
fn a_mapping_has_a_lower_case_name_that_is_not_repeated() {
    let long = "a".repeat(33);
    for bad in [
        "",
        "Metastore",
        "meta store",
        "meta_store",
        "../x",
        long.as_str(),
    ] {
        let config = config().with_mappings([Mapping::new(bad, endpoint("hive.example", 9083))]);
        let built = Relay::new(config, Script::default(), Recording::new());
        assert!(matches!(built, Err(RelayConfigError::Mapping)), "{bad:?}");
    }
    let repeated = config().with_mappings([
        Mapping::new("metastore", endpoint("hive.example", 9083)),
        Mapping::new("metastore", endpoint("other.example", 9083)),
    ]);
    assert!(matches!(
        Relay::new(repeated, Script::default(), Recording::new()),
        Err(RelayConfigError::Mapping)
    ));
}

#[test]
fn more_than_256_allowed_networks_are_not_accepted() {
    let many: Vec<IpNet> = (0..=255_u8)
        .map(|n| format!("10.{n}.0.0/16").parse().unwrap())
        .chain(["203.0.113.0/24".parse().unwrap()])
        .collect();
    assert_eq!(many.len(), 257);

    let built = Relay::new(
        RelayConfig::new(token(), endpoints(), many),
        Script::default(),
        Recording::new(),
    );

    assert!(matches!(built, Err(RelayConfigError::Network)));
}
