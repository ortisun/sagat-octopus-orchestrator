//! One-shot local PostgreSQL wire relay for losing an actual dispatched COMMIT response.
//! Coupled to pinned SQLx0.9's Q/COMMIT path. No TLS/COPY/pipelining/cancel proxy support.
//! Never logs frame bodies; authentication passes transparently to the disposable test server.

use sqlx::{
    PgPool,
    postgres::{PgConnectOptions, PgPoolOptions, PgSslMode},
};
use std::{
    io,
    sync::{
        Arc,
        atomic::{AtomicU8, AtomicU64, Ordering},
    },
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::oneshot,
    task::JoinHandle,
    time::{Duration, timeout},
};

const FORWARDING: u8 = 0;
const ARMED: u8 = 1;
const COMMIT_SENT: u8 = 2;
const MAX_FRAME: usize = 1024 * 1024;
const RELAY_RUNNING: u8 = 0;
const RELAY_COMPLETED: u8 = 1;
const RELAY_IO_ERROR: u8 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CommitResolution {
    Committed,
    Aborted23514,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RelayProgress {
    pub frontend_frames: u64,
    pub backend_frames: u64,
    pub frontend_last_tag: Option<char>,
    pub backend_last_tag: Option<char>,
    pub terminal: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RelayTermination {
    pub completed_ok: bool,
    pub io_error_kind: Option<io::ErrorKind>,
    pub join_cancelled: bool,
    pub join_panic: bool,
}

#[derive(Default)]
struct RelayDiagnostics {
    frontend_frames: AtomicU64,
    backend_frames: AtomicU64,
    frontend_last_tag: AtomicU8,
    backend_last_tag: AtomicU8,
    terminal: AtomicU8,
}

impl RelayDiagnostics {
    fn snapshot(&self) -> RelayProgress {
        let tag = |value| match value {
            0 => None,
            value => Some(char::from(value)),
        };
        let terminal = match self.terminal.load(Ordering::Acquire) {
            RELAY_RUNNING => "running",
            RELAY_COMPLETED => "completed",
            RELAY_IO_ERROR => "io_error",
            _ => "invalid",
        };
        RelayProgress {
            frontend_frames: self.frontend_frames.load(Ordering::Acquire),
            backend_frames: self.backend_frames.load(Ordering::Acquire),
            frontend_last_tag: tag(self.frontend_last_tag.load(Ordering::Acquire)),
            backend_last_tag: tag(self.backend_last_tag.load(Ordering::Acquire)),
            terminal,
        }
    }
}

pub(crate) struct CommitAckLossProxy {
    pub pool: PgPool,
    state: Arc<AtomicU8>,
    sent: Option<oneshot::Receiver<()>>,
    lose_ack: Option<oneshot::Sender<()>>,
    disconnected: Option<oneshot::Receiver<()>>,
    resolved: Option<oneshot::Receiver<CommitResolution>>,
    diagnostics: Arc<RelayDiagnostics>,
    task: Option<JoinHandle<io::Result<()>>>,
}
impl CommitAckLossProxy {
    pub async fn start(db: &crate::support::TestDb, expected: CommitResolution) -> Self {
        let upstream: PgConnectOptions = db
            .url_for(
                crate::support::RUNTIME_ROLE,
                crate::support::RUNTIME_ROLE_PASSWORD,
            )
            .parse()
            .expect("runtime PostgreSQL options");
        assert!(
            upstream.get_socket().is_none(),
            "test relay requires local TCP"
        );
        let host = upstream.get_host().to_owned();
        let port = upstream.get_port();
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("local one-shot listener");
        let proxy_port = listener.local_addr().expect("listener address").port();
        let state = Arc::new(AtomicU8::new(FORWARDING));
        let (sent_tx, sent) = oneshot::channel();
        let (lose_ack, lose_rx) = oneshot::channel();
        let (closed_tx, disconnected) = oneshot::channel();
        let (resolved_tx, resolved) = oneshot::channel();
        let diagnostics = Arc::new(RelayDiagnostics::default());
        let task_state = Arc::clone(&state);
        let task_diagnostics = Arc::clone(&diagnostics);
        let task = tokio::spawn(async move {
            let result = relay(
                listener,
                host,
                port,
                expected,
                task_state,
                Controls {
                    sent: sent_tx,
                    lose: lose_rx,
                    closed: closed_tx,
                    resolved: resolved_tx,
                },
                Arc::clone(&task_diagnostics),
            )
            .await;
            task_diagnostics.terminal.store(
                if result.is_ok() {
                    RELAY_COMPLETED
                } else {
                    RELAY_IO_ERROR
                },
                Ordering::Release,
            );
            result
        });
        let options = upstream
            .host("127.0.0.1")
            .port(proxy_port)
            .ssl_mode(PgSslMode::Disable)
            .application_name("sagat-test-capacity-commit-loss");
        let pool = PgPoolOptions::new()
            .min_connections(0)
            .max_connections(1)
            .test_before_acquire(false)
            .connect_lazy_with(options);
        // Own the abort-on-drop proxy before the first startup await. Cancellation during
        // connect/authentication drops Self and aborts the already-spawned relay as well.
        let proxy = Self {
            pool,
            state,
            sent: Some(sent),
            lose_ack: Some(lose_ack),
            disconnected: Some(disconnected),
            resolved: Some(resolved),
            diagnostics,
            task: Some(task),
        };
        match timeout(Duration::from_secs(5), proxy.pool.acquire()).await {
            Ok(Ok(connection)) => drop(connection),
            _ => panic!("authenticated single proxied runtime connection failed"),
        }
        proxy
    }
    pub fn arm(&self) {
        assert_eq!(
            self.state
                .compare_exchange(FORWARDING, ARMED, Ordering::AcqRel, Ordering::Acquire),
            Ok(FORWARDING)
        );
    }
    pub async fn wait_sent(&mut self) {
        timeout(
            Duration::from_secs(5),
            self.sent.take().expect("one commit observation"),
        )
        .await
        .expect("COMMIT dispatch deadline")
        .expect("relay forwarded COMMIT");
    }
    /// Close only the client reply direction, retaining the upstream transaction until resolved.
    pub async fn lose_reply(&mut self) {
        self.lose_ack
            .take()
            .expect("one response loss")
            .send(())
            .expect("live relay");
        timeout(
            Duration::from_secs(5),
            self.disconnected.take().expect("disconnect observation"),
        )
        .await
        .expect("client EOF deadline")
        .expect("reply socket closed");
    }
    pub async fn resolution(&mut self) -> CommitResolution {
        timeout(
            Duration::from_secs(5),
            self.resolved.take().expect("one server resolution"),
        )
        .await
        .expect("server resolution deadline")
        .expect("observed backend idle after result")
    }
    pub fn progress(&self) -> RelayProgress {
        self.diagnostics.snapshot()
    }
    pub async fn wait_termination(&mut self) -> RelayTermination {
        let joined = self.task.as_mut().expect("relay task").await;
        self.task = None;
        match joined {
            Ok(Ok(())) => RelayTermination {
                completed_ok: true,
                io_error_kind: None,
                join_cancelled: false,
                join_panic: false,
            },
            Ok(Err(error)) => RelayTermination {
                completed_ok: false,
                io_error_kind: Some(error.kind()),
                join_cancelled: false,
                join_panic: false,
            },
            Err(error) => RelayTermination {
                completed_ok: false,
                io_error_kind: None,
                join_cancelled: error.is_cancelled(),
                join_panic: error.is_panic(),
            },
        }
    }
    pub async fn close(mut self) {
        timeout(Duration::from_secs(5), self.pool.close())
            .await
            .expect("proxy pool closes");
        let termination = timeout(Duration::from_secs(5), self.wait_termination())
            .await
            .expect("relay ends");
        assert_eq!(
            termination,
            RelayTermination {
                completed_ok: true,
                io_error_kind: None,
                join_cancelled: false,
                join_panic: false,
            },
            "valid protocol resolution"
        );
    }
}
impl Drop for CommitAckLossProxy {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}
struct Controls {
    sent: oneshot::Sender<()>,
    lose: oneshot::Receiver<()>,
    closed: oneshot::Sender<()>,
    resolved: oneshot::Sender<CommitResolution>,
}
struct Frame {
    tag: u8,
    wire: Vec<u8>,
}
impl Frame {
    fn body(&self) -> &[u8] {
        &self.wire[5..]
    }
}
fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
async fn startup<R: AsyncRead + Unpin>(reader: &mut R) -> io::Result<Vec<u8>> {
    let mut header = [0_u8; 4];
    reader.read_exact(&mut header).await?;
    let length =
        usize::try_from(u32::from_be_bytes(header)).map_err(|_| invalid("startup length"))?;
    if !(8..=MAX_FRAME).contains(&length) {
        return Err(invalid("startup frame bound"));
    }
    let mut wire = vec![0; length];
    wire[..4].copy_from_slice(&header);
    reader.read_exact(&mut wire[4..]).await?;
    if wire[4..8] != 196608_u32.to_be_bytes() {
        return Err(invalid("only protocol3 without TLS negotiation"));
    }
    Ok(wire)
}
async fn frame<R: AsyncRead + Unpin>(reader: &mut R) -> io::Result<Frame> {
    let mut header = [0_u8; 5];
    reader.read_exact(&mut header).await?;
    let length = usize::try_from(u32::from_be_bytes(
        header[1..].try_into().expect("four-byte length"),
    ))
    .map_err(|_| invalid("frame length"))?;
    if !(4..=MAX_FRAME).contains(&length) {
        return Err(invalid("tagged frame bound"));
    }
    let mut wire = vec![0; length + 1];
    wire[..5].copy_from_slice(&header);
    reader.read_exact(&mut wire[5..]).await?;
    Ok(Frame {
        tag: header[0],
        wire,
    })
}
fn error_field(mut body: &[u8], field: u8) -> Option<&[u8]> {
    while let Some((&tag, rest)) = body.split_first() {
        if tag == 0 {
            return None;
        }
        let end = rest.iter().position(|byte| *byte == 0)?;
        if tag == field {
            return Some(&rest[..end]);
        }
        body = rest.get(end + 1..)?;
    }
    None
}
async fn relay(
    listener: TcpListener,
    host: String,
    port: u16,
    expected: CommitResolution,
    state: Arc<AtomicU8>,
    control: Controls,
    diagnostics: Arc<RelayDiagnostics>,
) -> io::Result<()> {
    let (mut client, _) = listener.accept().await?;
    // This proxy models commit-reply loss, not packetization latency between frames.
    client.set_nodelay(true)?;
    let mut upstream = TcpStream::connect((host.as_str(), port)).await?;
    upstream.set_nodelay(true)?;
    upstream.write_all(&startup(&mut client).await?).await?;
    let (mut client_read, mut client_write) = client.into_split();
    let (mut server_read, mut server_write) = upstream.into_split();
    let client_state = Arc::clone(&state);
    let Controls {
        sent,
        mut lose,
        closed,
        resolved,
    } = control;
    let frontend_diagnostics = Arc::clone(&diagnostics);
    let frontend = async move {
        loop {
            let request = frame(&mut client_read).await?;
            frontend_diagnostics
                .frontend_frames
                .fetch_add(1, Ordering::AcqRel);
            frontend_diagnostics
                .frontend_last_tag
                .store(request.tag, Ordering::Release);
            if request.tag == b'Q'
                && request.body() == b"COMMIT\0"
                && client_state
                    .compare_exchange(ARMED, COMMIT_SENT, Ordering::AcqRel, Ordering::Acquire)
                    .is_ok()
            {
                server_write.write_all(&request.wire).await?;
                server_write.flush().await?;
                let _ = sent.send(());
                // Ignore later client-side rollback cleanup; only this dispatched COMMIT is tested.
                return std::future::pending::<io::Result<()>>().await;
            }
            server_write.write_all(&request.wire).await?;
        }
    };
    let backend = async move {
        let mut closed = Some(closed);
        let mut resolved = Some(resolved);
        let mut disconnected = false;
        let mut outcome = None;
        let mut withheld = 0;
        loop {
            // Keep this read future pinned when the control signal wins. No partially read
            // protocol header/body is discarded by cancellation of read_exact.
            let response = {
                let next = frame(&mut server_read);
                tokio::pin!(next);
                loop {
                    tokio::select! {
                        result=&mut next=>break result?,
                        request=&mut lose,if !disconnected=>{
                            request.map_err(|_|invalid("missing disconnect request"))?;
                            if state.load(Ordering::Acquire)!=COMMIT_SENT{return Err(invalid("disconnect before COMMIT dispatch"));}
                            client_write.shutdown().await?;disconnected=true;
                            let _=closed.take().expect("one reply shutdown").send(());
                        }
                    }
                }
            };
            diagnostics.backend_frames.fetch_add(1, Ordering::AcqRel);
            diagnostics
                .backend_last_tag
                .store(response.tag, Ordering::Release);
            if state.load(Ordering::Acquire) != COMMIT_SENT {
                client_write.write_all(&response.wire).await?;
                continue;
            }
            withheld += 1;
            if withheld > 8 {
                return Err(invalid("post-COMMIT frame bound"));
            }
            match response.tag {
                b'C' if response.body() == b"COMMIT\0" && outcome.is_none() => {
                    outcome = Some(CommitResolution::Committed)
                }
                b'E' if outcome.is_none()
                    && error_field(response.body(), b'C') == Some(b"23514".as_slice())
                    && error_field(response.body(), b'n')
                        == Some(b"capacity_finish_proxy_forced_abort".as_slice()) =>
                {
                    outcome = Some(CommitResolution::Aborted23514)
                }
                b'Z' if response.body() == b"I" && outcome == Some(expected) && disconnected => {
                    let _ = resolved
                        .take()
                        .expect("one outcome")
                        .send(outcome.expect("validated backend resolution"));
                    return Ok(());
                }
                b'N' => {}
                _ => return Err(invalid("unexpected post-COMMIT resolution frame")),
            }
        }
    };
    tokio::select! { result=frontend=>result,result=backend=>result }
}
