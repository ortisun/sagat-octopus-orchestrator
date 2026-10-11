//! A real NATS server with JetStream, and a TCP proxy in front of it that tests can cut.
#![allow(dead_code, clippy::expect_used, clippy::panic)]

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use async_nats::jetstream::consumer::{AckPolicy, pull};
use futures_util::StreamExt;
use sagat_infra_nats::NatsPublisher;
use testcontainers_modules::nats::{Nats, NatsServerCmd};
use testcontainers_modules::testcontainers::ContainerAsync;
use testcontainers_modules::testcontainers::ImageExt;
use testcontainers_modules::testcontainers::runners::AsyncRunner;
use tokio::io::copy_bidirectional;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;

pub struct TestNats {
    _server: Backend,
    pub addr: SocketAddr,
}

/// What runs the server: a container, or (when `SAGAT_TEST_NATS_BIN` names a `nats-server`) a child
/// process of the test, on a port it chose and with a store of its own, killed with the test. The
/// second is for a machine that cannot run containers; it is the same server and the same flags.
enum Backend {
    Container(#[allow(dead_code)] Box<ContainerAsync<Nats>>),
    Process {
        child: std::process::Child,
        _store: std::path::PathBuf,
    },
}

impl Drop for Backend {
    fn drop(&mut self) {
        if let Backend::Process { child, _store } = self {
            let _ = child.kill();
            let _ = child.wait();
            let _ = std::fs::remove_dir_all(_store);
        }
    }
}

impl TestNats {
    pub fn url(&self) -> String {
        format!("nats://{}", self.addr)
    }
}

/// Starts a NATS server with JetStream enabled.
pub async fn start() -> TestNats {
    if let Some(binary) = std::env::var_os("SAGAT_TEST_NATS_BIN") {
        return start_process(&binary);
    }
    let container = Nats::default()
        .with_cmd(&NatsServerCmd::default().with_jetstream())
        .start()
        .await
        .expect("start NATS (is Docker running?)");
    let port = container
        .get_host_port_ipv4(4222)
        .await
        .expect("mapped NATS port");
    TestNats {
        _server: Backend::Container(Box::new(container)),
        addr: SocketAddr::from(([127, 0, 0, 1], port)),
    }
}

/// `nats-server -js` on a free port, with its own store; returns when it accepts connections.
fn start_process(binary: &std::ffi::OsStr) -> TestNats {
    use std::io::{BufRead, BufReader};
    use std::process::{Command, Stdio};

    let store = std::env::temp_dir().join(format!("sagat-nats-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&store).expect("a store for NATS");
    let mut child = Command::new(binary)
        .args(["-js", "-a", "127.0.0.1", "-p", "-1", "-sd"])
        .arg(&store)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start nats-server (SAGAT_TEST_NATS_BIN)");
    // The server logs the port it chose, then "Server is ready" once JetStream is up.
    let mut lines = BufReader::new(child.stderr.take().expect("stderr")).lines();
    let (mut port, mut ready) = (None, false);
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    while !ready && std::time::Instant::now() < deadline {
        let Some(Ok(line)) = lines.next() else { break };
        if let Some(rest) = line.split("Listening for client connections on ").nth(1) {
            port = rest
                .rsplit(':')
                .next()
                .and_then(|p| p.trim().parse::<u16>().ok());
        }
        ready = line.contains("Server is ready");
    }
    let port = port
        .filter(|_| ready)
        .expect("nats-server did not become ready");
    // Keep reading so the pipe never fills and blocks the server.
    std::thread::spawn(move || for _ in lines {});
    TestNats {
        _server: Backend::Process {
            child,
            _store: store,
        },
        addr: SocketAddr::from(([127, 0, 0, 1], port)),
    }
}

/// A TCP forwarder that can sever every connection and refuse new ones, to make "the broker went
/// away" and "the broker came back" happen on demand without touching the container.
pub struct Proxy {
    pub addr: SocketAddr,
    blocked: Arc<AtomicBool>,
    sever: watch::Sender<u64>,
}

impl Proxy {
    pub fn url(&self) -> String {
        format!("nats://{}", self.addr)
    }

    /// While blocked, new connections are closed at once and existing ones are cut.
    pub fn block(&self) {
        self.blocked.store(true, Ordering::SeqCst);
        self.sever.send_modify(|generation| *generation += 1);
    }

    pub fn unblock(&self) {
        self.blocked.store(false, Ordering::SeqCst);
    }
}

pub async fn proxy_to(target: SocketAddr) -> Proxy {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind proxy");
    let addr = listener.local_addr().expect("proxy addr");
    let blocked = Arc::new(AtomicBool::new(false));
    let (sever, severed) = watch::channel(0u64);
    let gate = Arc::clone(&blocked);
    tokio::spawn(async move {
        loop {
            let Ok((mut inbound, _)) = listener.accept().await else {
                return;
            };
            if gate.load(Ordering::SeqCst) {
                drop(inbound);
                continue;
            }
            let mut cut = severed.clone();
            // A clone inherits the version its original last saw; only cuts made from now on count.
            cut.borrow_and_update();
            tokio::spawn(async move {
                let Ok(mut outbound) = TcpStream::connect(target).await else {
                    return;
                };
                tokio::select! {
                    _ = copy_bidirectional(&mut inbound, &mut outbound) => {}
                    _ = cut.changed() => {}
                }
            });
        }
    });
    Proxy {
        addr,
        blocked,
        sever,
    }
}

/// Everything currently in the stream on `filter`, without acknowledging a thing.
pub async fn drain(
    publisher: &NatsPublisher,
    filter: &str,
) -> Vec<(String, Vec<u8>, Option<String>)> {
    let stream = publisher.stream().await.expect("stream");
    let consumer = stream
        .create_consumer(pull::Config {
            filter_subject: filter.to_owned(),
            ack_policy: AckPolicy::None,
            ..Default::default()
        })
        .await
        .expect("consumer");
    let mut batch = consumer
        .fetch()
        .max_messages(100)
        .expires(Duration::from_millis(800))
        .messages()
        .await
        .expect("fetch");
    let mut out = Vec::new();
    while let Some(message) = batch.next().await {
        let message = message.expect("message");
        let id = message
            .headers
            .as_ref()
            .and_then(|h| h.get("Nats-Msg-Id"))
            .map(ToString::to_string);
        out.push((message.subject.to_string(), message.payload.to_vec(), id));
    }
    out
}
