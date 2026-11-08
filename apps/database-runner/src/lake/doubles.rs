//! The doubles of the lake modules' tests: a sandbox that "starts" a scripted host on the real relay socket, a resolver and a
//! connector that reach nothing, and the receipt and failure lines a host sends. They are shared by the tests of the driver and of
//! the shell that calls it (feature 035, T035-058, D035-143).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io;
use std::net::{IpAddr, SocketAddr};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use sagat_infra_lake::attempt::{ContainerExit, LakeSandbox, SandboxError, Signals, StartRequest};
use sagat_infra_lake::relay::{Connector, Resolver};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader, DuplexStream};
use tokio::net::UnixStream;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use super::prepare::{BlobError, Blobs};

// -- the doubles ---------------------------------------------------------------------------------------------------

#[derive(Clone)]
pub(crate) struct NoDns;

impl Resolver for NoDns {
    async fn resolve(&self, _host: &str, _port: u16) -> io::Result<Vec<IpAddr>> {
        Err(io::Error::other("no name server in this test"))
    }
}

#[derive(Clone)]
pub(crate) struct NoNetwork;

impl Connector for NoNetwork {
    type Stream = DuplexStream;

    async fn connect(&self, _address: SocketAddr) -> io::Result<DuplexStream> {
        Err(io::Error::other("no network in this test"))
    }
}

/// A source that never answers: a `ReadBlob` that stalls.
pub(crate) struct Stalled;

impl Blobs for Stalled {
    type Reader = io::Cursor<Vec<u8>>;

    async fn open(&self, _blob: sagat_domain::blobs::BlobId) -> Result<Self::Reader, BlobError> {
        std::future::pending().await
    }
}

pub(crate) fn by_blob(document: &mut Value) {
    document["records"] =
        json!({"blobs": [{"id": "0192f5aa-0000-7000-8000-00000000000a", "format": "ndjson"}]});
}

pub(crate) struct NoBlobs;

impl Blobs for NoBlobs {
    type Reader = io::Cursor<Vec<u8>>;

    async fn open(&self, _blob: sagat_domain::blobs::BlobId) -> Result<Self::Reader, BlobError> {
        Err(BlobError)
    }
}

/// What the scripted host does once the sandbox "starts" it.
#[derive(Clone)]
pub(crate) enum Script {
    /// Connects, reads the request, sends this many beats and then this line, and closes.
    Answer { beats: usize, line: Vec<u8> },
    /// Is started and never connects.
    NeverConnects,
}

pub(crate) struct Scripted {
    pub(crate) work: std::path::PathBuf,
    pub(crate) script: Script,
    pub(crate) start_error: Option<SandboxError>,
    pub(crate) kill_unconfirmed: bool,
    pub(crate) asked: Mutex<Vec<StartRequest>>,
    pub(crate) kills: AtomicUsize,
    pub(crate) host: Mutex<Option<JoinHandle<()>>>,
    pub(crate) request: Arc<Mutex<Vec<u8>>>,
}

impl Scripted {
    pub(crate) fn new(work: &Path, script: Script) -> Self {
        Self {
            work: work.to_path_buf(),
            script,
            start_error: None,
            kill_unconfirmed: false,
            asked: Mutex::new(Vec::new()),
            kills: AtomicUsize::new(0),
            host: Mutex::new(None),
            request: Arc::default(),
        }
    }

    pub(crate) fn asked(&self) -> Vec<StartRequest> {
        self.asked.lock().unwrap().clone()
    }
}

/// The host at the wire, as `crates/infra-lake/tests/attempt.rs` plays it: the token from the file in the attempt
/// directory, the relay socket, `CONTROL`, the request line, then one ending.
pub(crate) async fn host(
    work: std::path::PathBuf,
    id: String,
    script: Script,
    request: Arc<Mutex<Vec<u8>>>,
) {
    let Script::Answer { beats, line } = script else {
        return std::future::pending().await;
    };
    let directory = work.join(&id);
    let token = std::fs::read_to_string(directory.join("relay.token")).unwrap();
    let mut stream = BufReader::new(
        UnixStream::connect(directory.join("relay.sock"))
            .await
            .unwrap(),
    );
    stream
        .get_mut()
        .write_all(format!("SAGAT-RELAY/1 {}\nCONTROL\n", token.trim_end()).as_bytes())
        .await
        .unwrap();
    let mut asked = Vec::new();
    stream.read_until(b'\n', &mut asked).await.unwrap();
    *request.lock().unwrap() = asked;
    for _ in 0..beats {
        stream
            .get_mut()
            .write_all(b"{\"heartbeat\":true}\n")
            .await
            .unwrap();
    }
    stream.get_mut().write_all(&line).await.unwrap();
    stream.get_mut().shutdown().await.unwrap();
}

impl LakeSandbox for Scripted {
    async fn start(&self, request: &StartRequest) -> Result<(), SandboxError> {
        self.asked.lock().unwrap().push(request.clone());
        if let Some(error) = self.start_error {
            return Err(error);
        }
        let task = tokio::spawn(host(
            self.work.clone(),
            request.attempt_id.clone(),
            self.script.clone(),
            Arc::clone(&self.request),
        ));
        *self.host.lock().unwrap() = Some(task);
        Ok(())
    }

    async fn exited(&self, _attempt_id: &str) {
        std::future::pending().await
    }

    async fn kill(&self, _attempt_id: &str) -> ContainerExit {
        self.kills.fetch_add(1, Ordering::SeqCst);
        if let Some(task) = self.host.lock().unwrap().take() {
            task.abort();
        }
        ContainerExit {
            memory_killed: false,
            ended: !self.kill_unconfirmed,
        }
    }
}

// -- the fixture ---------------------------------------------------------------------------------------------------

pub(crate) fn receipt_line() -> Vec<u8> {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/contract/iceberg-writes/fixtures/receipts/iceberg-committed.json");
    let receipt: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let mut line = serde_json::to_vec(&json!({"receipt": receipt, "relayDenials": 0})).unwrap();
    line.push(b'\n');
    line
}

/// The receipt fixture after `edit`, as the line the host sends.
pub(crate) fn receipt_line_with(edit: impl FnOnce(&mut Value)) -> Vec<u8> {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/contract/iceberg-writes/fixtures/receipts/iceberg-committed.json");
    let mut receipt: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    edit(&mut receipt);
    let mut line = serde_json::to_vec(&json!({"receipt": receipt, "relayDenials": 0})).unwrap();
    line.push(b'\n');
    line
}

pub(crate) fn failure_line(code: &str, column: &str) -> Vec<u8> {
    let mut line = serde_json::to_vec(
        &json!({"failure": {"code": code, "column": column}, "relayDenials": 0}),
    )
    .unwrap();
    line.push(b'\n');
    line
}

pub(crate) fn answer(line: Vec<u8>) -> Script {
    Script::Answer { beats: 0, line }
}

pub(crate) struct World {
    // a short path: a Unix socket's path is limited to about a hundred bytes
    pub(crate) work: tempfile::TempDir,
}

pub(crate) fn world() -> World {
    World {
        work: tempfile::Builder::new()
            .prefix("dr")
            .tempdir_in("/tmp")
            .unwrap(),
    }
}

pub(crate) fn signals() -> Signals {
    Signals {
        cancelled: CancellationToken::new(),
        lease_lost: CancellationToken::new(),
    }
}

pub(crate) fn entries(directory: &Path) -> Vec<String> {
    std::fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect()
}
