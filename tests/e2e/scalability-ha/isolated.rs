//! An engine in a runtime of its own, which can be killed the way a process is: every task of it
//! stops where it is, nothing runs a destructor that gives a shard back, and the connections of its
//! pool close. (`RunningEngine` stops cleanly, which is a different thing to test.)

use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::Duration;

use sagat_control_plane::config::Config;
use sagat_execution_engine::EngineConfig;

pub struct IsolatedEngine {
    pub grpc_url: String,
    pub admin: SocketAddr,
    kill: Option<std::sync::mpsc::Sender<Kill>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

enum Kill {
    Crash,
    Stop,
}

impl IsolatedEngine {
    pub async fn start(env: HashMap<String, String>) -> Self {
        let (address_tx, address_rx) = std::sync::mpsc::channel::<(String, SocketAddr)>();
        let (kill_tx, kill_rx) = std::sync::mpsc::channel::<Kill>();
        let thread = std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .expect("a runtime for the engine");
            let lookup = move |name: &str| env.get(name).cloned();
            let config = Config::from_lookup(&lookup).expect("config");
            let engine = EngineConfig::from_lookup(&lookup, config.environment).expect("engine");
            let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
            let served = runtime.spawn(async move {
                let grpc = tokio::net::TcpListener::bind("127.0.0.1:0")
                    .await
                    .expect("grpc");
                let admin = tokio::net::TcpListener::bind("127.0.0.1:0")
                    .await
                    .expect("admin");
                let url = format!("http://{}", grpc.local_addr().expect("addr"));
                address_tx
                    .send((url, admin.local_addr().expect("addr")))
                    .expect("report the addresses");
                sagat_execution_engine::serve(&config, &engine, grpc, Some(admin), async move {
                    let _ = stop_rx.await;
                })
                .await
            });
            match kill_rx.recv() {
                Ok(Kill::Stop) => {
                    let _ = stop_tx.send(());
                    let _ = runtime.block_on(served);
                    drop(runtime);
                }
                // A crash: the runtime is dropped with its tasks where they stand.
                Ok(Kill::Crash) | Err(_) => runtime.shutdown_background(),
            }
        });
        let (grpc_url, admin) = tokio::task::spawn_blocking(move || address_rx.recv())
            .await
            .expect("join")
            .expect("the engine reported its addresses");
        let engine = Self {
            grpc_url,
            admin,
            kill: Some(kill_tx),
            thread: Some(thread),
        };
        for _ in 0..200 {
            if engine.alive().await {
                return engine;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("the engine never became alive");
    }

    async fn alive(&self) -> bool {
        reqwest::get(format!("http://{}/livez", self.admin))
            .await
            .is_ok_and(|r| r.status().is_success())
    }

    /// Like `kill -9`: nothing is given back and nothing is told.
    pub async fn crash(&mut self) {
        if let Some(kill) = self.kill.take() {
            let _ = kill.send(Kill::Crash);
        }
        self.join().await;
    }

    /// Like SIGTERM: the loops stop, the shards are given back, the calls are drained.
    pub async fn stop(&mut self) {
        if let Some(kill) = self.kill.take() {
            let _ = kill.send(Kill::Stop);
        }
        self.join().await;
    }

    async fn join(&mut self) {
        if let Some(thread) = self.thread.take() {
            tokio::task::spawn_blocking(move || thread.join())
                .await
                .expect("join")
                .expect("the engine thread did not panic");
        }
    }
}

impl Drop for IsolatedEngine {
    fn drop(&mut self) {
        if let Some(kill) = self.kill.take() {
            let _ = kill.send(Kill::Crash);
        }
    }
}
