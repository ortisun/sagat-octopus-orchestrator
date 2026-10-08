//! A real NATS server with JetStream, and a TCP proxy in front of it that tests can cut.
#![allow(dead_code, clippy::expect_used, clippy::panic)]

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use testcontainers_modules::nats::{Nats, NatsServerCmd};
use testcontainers_modules::testcontainers::ContainerAsync;
use testcontainers_modules::testcontainers::ImageExt;
use testcontainers_modules::testcontainers::runners::AsyncRunner;
use tokio::io::copy_bidirectional;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;

pub struct TestNats {
    _container: ContainerAsync<Nats>,
    pub addr: SocketAddr,
}

impl TestNats {
    pub fn url(&self) -> String {
        format!("nats://{}", self.addr)
    }
}

/// Starts a NATS server with JetStream enabled.
pub async fn start() -> TestNats {
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
        _container: container,
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
