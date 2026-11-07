//! The router of the lake sandbox API (feature 035, ADR-0079 rules 3 to 5 and 11).
//!
//! Three calls, and nothing else. **Every** response is a closed body from `sagat_infra_lake::sandbox_api` — the answers, and
//! also the refusals that are not answers: a route nobody has (404), a method not allowed (405), a body too large (413), a body
//! that is not one the API reads (400), headers too large (431). No extractor of the framework writes text: the body is read
//! here, by hand, through the bound, and a path that cannot be read is a closed rejection too. Nothing the engine said, and
//! nothing the peer sent, is echoed.

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::rejection::PathRejection;
use axum::extract::{Path, Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::Response;
use axum::routing::{get, post};
use sagat_infra_lake::sandbox_api::{
    MAX_BODY_BYTES, MAX_HEADER_BYTES, Plain, StartBody, attempt_id_ok, kill_body, plain_body,
    start_body, wait_body,
};

use super::admission::Admission;
use super::engine::Engine;

/// How long a request may take to send its body once its head is in. `header_read_timeout` covers the head only; a peer that
/// sends the head and then nothing would otherwise hold one of the few connections for as long as it liked.
const BODY_READ: Duration = if cfg!(test) {
    Duration::from_millis(700)
} else {
    Duration::from_secs(5)
};

fn json(status: u16, body: Vec<u8>) -> Response {
    let mut response = Response::new(Body::from(body));
    *response.status_mut() =
        StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    response
}

/// A refusal that is not an answer to the question: a closed body, with the status that says why.
fn rejected(status: StatusCode) -> Response {
    json(status.as_u16(), plain_body(Plain::Rejected).1)
}

fn unavailable() -> Response {
    let (status, body) = plain_body(Plain::Unavailable);
    json(status, body)
}

/// What the headers of a request add up to: names and values, as hyper parsed them.
fn header_bytes(headers: &HeaderMap) -> usize {
    headers
        .iter()
        .map(|(name, value)| name.as_str().len() + value.len())
        .sum()
}

async fn bounded_headers(request: Request, next: Next) -> Response {
    if header_bytes(request.headers()) > MAX_HEADER_BYTES {
        return rejected(StatusCode::REQUEST_HEADER_FIELDS_TOO_LARGE);
    }
    next.run(request).await
}

async fn not_found() -> Response {
    rejected(StatusCode::NOT_FOUND)
}

async fn method_not_allowed() -> Response {
    rejected(StatusCode::METHOD_NOT_ALLOWED)
}

/// The attempt id of the path, if it is one. A path that cannot be read as text is not.
fn attempt(path: Result<Path<String>, PathRejection>) -> Option<String> {
    let Path(id) = path.ok()?;
    attempt_id_ok(&id).then_some(id)
}

async fn start<E: Engine>(
    State(admission): State<Arc<Admission<E>>>,
    path: Result<Path<String>, PathRejection>,
    request: Request,
) -> Response {
    let Some(id) = attempt(path) else {
        return rejected(StatusCode::BAD_REQUEST);
    };
    // the bound is on the bytes read, whatever the request says it will send
    let declared = request
        .headers()
        .get(header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<usize>().ok());
    if declared.is_some_and(|length| length > MAX_BODY_BYTES) {
        return rejected(StatusCode::PAYLOAD_TOO_LARGE);
    }
    let bytes = match tokio::time::timeout(BODY_READ, to_bytes(request.into_body(), MAX_BODY_BYTES))
        .await
    {
        Ok(Ok(bytes)) => bytes,
        Ok(Err(_)) => return rejected(StatusCode::PAYLOAD_TOO_LARGE),
        Err(_) => return rejected(StatusCode::REQUEST_TIMEOUT),
    };
    let Ok(body) = StartBody::parse(&bytes) else {
        return rejected(StatusCode::BAD_REQUEST);
    };
    let (status, body) = start_body(admission.start(&id, body).await);
    json(status, body)
}

async fn wait<E: Engine>(
    State(admission): State<Arc<Admission<E>>>,
    path: Result<Path<String>, PathRejection>,
) -> Response {
    let Some(id) = attempt(path) else {
        return rejected(StatusCode::BAD_REQUEST);
    };
    match admission.wait(&id).await {
        Some(state) => {
            let (status, body) = wait_body(state);
            json(status, body)
        }
        None => unavailable(),
    }
}

async fn kill<E: Engine>(
    State(admission): State<Arc<Admission<E>>>,
    path: Result<Path<String>, PathRejection>,
) -> Response {
    let Some(id) = attempt(path) else {
        return rejected(StatusCode::BAD_REQUEST);
    };
    let outcome = admission.kill(&id).await;
    let (status, body) = kill_body(outcome.ended, outcome.memory_killed);
    json(status, body)
}

/// The API, over `admission`.
pub fn router<E: Engine>(admission: Arc<Admission<E>>) -> Router {
    Router::new()
        .route("/v1/attempts/{id}", post(start::<E>).delete(kill::<E>))
        .route("/v1/attempts/{id}/wait", get(wait::<E>))
        .fallback(not_found)
        .method_not_allowed_fallback(method_not_allowed)
        .layer(middleware::from_fn(bounded_headers))
        .with_state(admission)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::os::unix::fs::MetadataExt as _;
    use std::path::{Path as FsPath, PathBuf};
    use std::time::Duration;

    use sagat_api_http::{ServerConfig, serve_unix};
    use sagat_infra_lake::sandbox_api::{
        KillOutcome, Reason, StartOutcome, WaitOutcome, read_kill, read_start, read_wait,
    };
    use serde_json::Value;
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    use tokio::net::{UnixListener, UnixStream};
    use tokio::sync::oneshot;

    use super::super::config::LakeConfig;
    use super::super::engine::fake::Fake;
    use super::*;

    const NOW: u64 = 1_800_000_000;
    const BOUND: Duration = Duration::from_secs(10);

    /// A status, a body and the head of an answer.
    type Reply = (u16, Vec<u8>, String);

    struct Server {
        socket: PathBuf,
        work: tempfile::TempDir,
        _socket_dir: tempfile::TempDir,
        admission: Arc<Admission<Fake>>,
        stop: Option<oneshot::Sender<()>>,
    }

    impl Server {
        fn start(extra: &[(&str, &str)]) -> Self {
            let work = tempfile::tempdir().expect("a work directory");
            let socket_dir = tempfile::tempdir().expect("a socket directory");
            let uid = std::fs::metadata(work.path()).expect("metadata").uid();
            let mut env: HashMap<String, String> = [
                (
                    "SAGAT_LAKE_SOCKET_DIR",
                    socket_dir.path().to_string_lossy().into_owned(),
                ),
                ("SAGAT_LAKE_RUNNER_UID", uid.to_string()),
                ("SAGAT_LAKE_RUNNER_GID", "10001".to_owned()),
                ("SAGAT_LAKE_WORK_VOLUME", "sagat-lake-work".to_owned()),
                (
                    "SAGAT_LAKE_WORK_ROOT",
                    work.path().to_string_lossy().into_owned(),
                ),
                ("SAGAT_LAKE_IMAGE", "sagat/lake@sha256:abc".to_owned()),
                ("SAGAT_LAKE_OWNER", "lake-runner".to_owned()),
                ("SAGAT_LAKE_NODE_MEMORY_MIB", "16384".to_owned()),
                ("SAGAT_LAKE_LARGE_MEMORY_MIB", "4096".to_owned()),
            ]
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect();
            for (key, value) in extra {
                env.insert((*key).to_owned(), (*value).to_owned());
            }
            let lookup = |name: &str| env.get(name).cloned();
            let config = LakeConfig::from_lookup(&lookup).expect("a valid configuration");
            let socket = config.socket_path();
            let admission = Arc::new(Admission::new(Fake::new(), config, Arc::new(|| NOW)));
            let listener = UnixListener::bind(&socket).expect("a socket");
            let (stop, stopped) = oneshot::channel::<()>();
            tokio::spawn(serve_unix(
                listener,
                move |peer| peer == uid,
                router(Arc::clone(&admission)),
                ServerConfig {
                    header_read_timeout: Duration::from_secs(5),
                    max_connections: 16,
                    write_stall_timeout: Duration::from_secs(5),
                    shutdown_grace: Duration::from_millis(300),
                },
                async move {
                    let _ = stopped.await;
                },
            ));
            Self {
                socket,
                work,
                _socket_dir: socket_dir,
                admission,
                stop: Some(stop),
            }
        }

        fn directory_of(&self, attempt: &str) {
            let path = self.work.path().join(attempt);
            if !path.exists() {
                std::fs::create_dir(path).expect("an attempt directory");
            }
        }

        /// One request on its own connection: the status and the body.
        async fn send(&self, request: &[u8]) -> (u16, Vec<u8>, String) {
            let mut stream = UnixStream::connect(&self.socket)
                .await
                .expect("a connection");
            // A server that refuses a request without reading all of it closes with bytes unread; on Linux the stream then ends
            // with a reset *after* the answer, on macOS with a plain end. What was sent before it stands, in both.
            let _ = stream.write_all(request).await;
            let mut got = Vec::new();
            match tokio::time::timeout(BOUND, stream.read_to_end(&mut got)).await {
                Ok(Ok(_)) => {}
                Ok(Err(error)) if error.kind() == std::io::ErrorKind::ConnectionReset => {}
                Ok(Err(error)) => panic!("a read: {error}"),
                Err(_) => panic!("the server did not answer"),
            }
            let split = got
                .windows(4)
                .position(|window| window == b"\r\n\r\n")
                .expect("a head");
            let head = String::from_utf8_lossy(&got[..split]).into_owned();
            let status = head
                .split(' ')
                .nth(1)
                .and_then(|status| status.parse().ok())
                .expect("a status");
            (status, got[split + 4..].to_vec(), head)
        }

        async fn post(&self, attempt: &str, body: &str) -> (u16, Vec<u8>, String) {
            let request = format!(
                "POST /v1/attempts/{attempt} HTTP/1.1\r\nHost: node\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            );
            self.send(request.as_bytes()).await
        }

        async fn get(&self, path: &str) -> (u16, Vec<u8>, String) {
            self.send(
                format!("GET {path} HTTP/1.1\r\nHost: node\r\nConnection: close\r\n\r\n")
                    .as_bytes(),
            )
            .await
        }

        async fn delete(&self, attempt: &str) -> (u16, Vec<u8>, String) {
            self.send(
                format!(
                    "DELETE /v1/attempts/{attempt} HTTP/1.1\r\nHost: node\r\nConnection: close\r\n\r\n"
                )
                .as_bytes(),
            )
            .await
        }
    }

    impl Drop for Server {
        fn drop(&mut self) {
            if let Some(stop) = self.stop.take() {
                let _ = stop.send(());
            }
        }
    }

    /// One answer off a connection that stays open: the head, then as many bytes as its length says.
    async fn read_response(stream: &mut UnixStream) -> (u16, Vec<u8>) {
        let mut head = Vec::new();
        let mut byte = [0_u8; 1];
        while !head.ends_with(b"\r\n\r\n") {
            tokio::time::timeout(BOUND, stream.read_exact(&mut byte))
                .await
                .expect("the answer arrives")
                .expect("a read");
            head.push(byte[0]);
        }
        let text = String::from_utf8_lossy(&head).into_owned();
        let status = text
            .split(' ')
            .nth(1)
            .and_then(|status| status.parse().ok())
            .expect("a status");
        let length: usize = text
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse().ok())
                    .flatten()
            })
            .expect("a length");
        let mut body = vec![0_u8; length];
        tokio::time::timeout(BOUND, stream.read_exact(&mut body))
            .await
            .expect("the body arrives")
            .expect("a read");
        (status, body)
    }

    /// Looks, for a few seconds, until `condition` holds: for a state that a task of the server reaches in its own time.
    async fn eventually(condition: impl Fn() -> bool) -> bool {
        for _ in 0..200 {
            if condition() {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        false
    }

    const SMALL: &str = r#"{"slot":3,"class":"small","deadline_ms":60000}"#;

    fn is_closed_body(body: &[u8]) -> bool {
        let text = String::from_utf8_lossy(body);
        text == r#"{"result":"rejected"}"# || text == r#"{"result":"unavailable"}"#
    }

    fn corpus() -> Value {
        let path = FsPath::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/contract/iceberg-writes/fixtures/sandbox-api/cases.json");
        serde_json::from_str(&std::fs::read_to_string(path).expect("the corpus")).expect("JSON")
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_start_a_wait_and_a_kill_are_answered_in_the_words_of_the_client() {
        let server = Server::start(&[]);
        server.directory_of("a1");
        let (status, body, head) = server.post("a1", SMALL).await;
        assert_eq!(read_start(status, &body), StartOutcome::Started, "{head}");
        assert!(
            head.to_ascii_lowercase()
                .contains("content-type: application/json"),
            "{head}"
        );

        let (status, body, _) = server.get("/v1/attempts/a1/wait").await;
        assert_eq!(read_wait(status, &body), WaitOutcome::Running);
        server.admission.engine().finish("a1");
        let (status, body, _) = server.get("/v1/attempts/a1/wait").await;
        assert_eq!(read_wait(status, &body), WaitOutcome::Ended);

        server.admission.engine().mark_memory_killed("a1");
        let (status, body, _) = server.delete("a1").await;
        assert_eq!(
            read_kill(status, &body),
            KillOutcome {
                ended: true,
                memory_killed: true
            }
        );
    }

    /// The server reads the requests of the corpus the way the corpus says, and a client reads what it answers: the two ends
    /// are held to one file.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_server_answers_the_requests_of_the_corpus_as_the_corpus_says() {
        for case in corpus()["start_requests"].as_array().expect("cases") {
            let name = case["name"].as_str().expect("a name");
            let body = case["body"].as_str().expect("a body");
            let expect = case["expect"].as_str().expect("an expectation");
            let server = Server::start(&[]);
            server.directory_of("a1");
            let (status, answer, _) = server.post("a1", body).await;
            match expect {
                "ok" => assert_eq!(read_start(status, &answer), StartOutcome::Started, "{name}"),
                "rejected" => {
                    assert_eq!(status, 400, "{name}");
                    assert_eq!(answer, br#"{"result":"rejected"}"#, "{name}");
                }
                refused => {
                    let reason =
                        Reason::parse(refused.trim_start_matches("refused:")).expect("a reason");
                    assert_eq!(
                        read_start(status, &answer),
                        StartOutcome::Refused(reason),
                        "{name}"
                    );
                }
            }
        }
    }

    /// Every refusal that is not an answer is a closed body: nothing the peer sent comes back, and nothing of the framework's.
    #[tokio::test(flavor = "multi_thread")]
    async fn every_refusal_that_is_not_an_answer_is_a_closed_body() {
        let server = Server::start(&[]);
        server.directory_of("a1");
        let big = "x".repeat(MAX_BODY_BYTES + 1);
        let cases: Vec<(&str, Reply, u16)> = vec![
            ("a route nobody has", server.get("/v1/other").await, 404),
            ("the root", server.get("/").await, 404),
            ("a version nobody has", server.get("/v2/attempts/a1/wait").await, 404),
            ("an empty attempt", server.get("/v1/attempts//wait").await, 400),
            ("a method not allowed on a start", server.get("/v1/attempts/a1").await, 405),
            (
                "a method not allowed on a wait",
                server
                    .send(b"POST /v1/attempts/a1/wait HTTP/1.1\r\nHost: node\r\nConnection: close\r\nContent-Length: 0\r\n\r\n")
                    .await,
                405,
            ),
            ("a put", server.send(b"PUT /v1/attempts/a1 HTTP/1.1\r\nHost: node\r\nConnection: close\r\nContent-Length: 0\r\n\r\n").await, 405),
            ("a body over the bound, declared", server.post("a1", &big).await, 413),
            ("an attempt in capitals", server.post("A1", SMALL).await, 400),
            ("an attempt too long", server.post(&"a".repeat(61), SMALL).await, 400),
            ("an attempt that starts with a hyphen", server.post("-a1", SMALL).await, 400),
            ("a wait for an attempt that is not one", server.get("/v1/attempts/ZZ/wait").await, 400),
            ("a kill of an attempt that is not one", server.delete("not-hex").await, 400),
            ("an attempt that is not text", server.get("/v1/attempts/%ff/wait").await, 400),
            ("a body that is not an object", server.post("a1", "[3]").await, 400),
        ];
        for (name, (status, body, _), expected) in cases {
            assert_eq!(status, expected, "{name}");
            assert!(
                is_closed_body(&body),
                "{name}: {}",
                String::from_utf8_lossy(&body)
            );
        }
    }

    /// A body that does not say how long it is (chunked) is read through the bound too: the declared length is not the only
    /// way to send a lot.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_chunked_body_over_the_bound_is_refused_and_makes_nothing() {
        let server = Server::start(&[]);
        server.directory_of("a1");
        let chunk = "x".repeat(512);
        let mut body = String::new();
        for _ in 0..8 {
            body.push_str(&format!("{:x}\r\n{chunk}\r\n", chunk.len()));
        }
        body.push_str("0\r\n\r\n");
        let request = format!(
            "POST /v1/attempts/a1 HTTP/1.1\r\nHost: node\r\nConnection: close\r\nTransfer-Encoding: chunked\r\n\r\n{body}"
        );
        let (status, answer, _) = server.send(request.as_bytes()).await;
        assert_eq!(status, 413);
        assert!(is_closed_body(&answer));
        assert_eq!(server.admission.engine().launches_made(), 0);
    }

    /// A body is as long as its length says and no longer: what follows is not part of it, and the request is judged on the bytes
    /// it declared.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_body_is_what_its_length_says_and_no_more() {
        let server = Server::start(&[]);
        server.directory_of("a1");
        let padding = " ".repeat(4 * MAX_BODY_BYTES);
        let request = format!(
            "POST /v1/attempts/a1 HTTP/1.1\r\nHost: node\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{SMALL}{padding}",
            SMALL.len()
        );
        let (status, answer, _) = server.send(request.as_bytes()).await;
        assert_eq!(read_start(status, &answer), StartOutcome::Started);
        assert_eq!(server.admission.engine().launches_made(), 1);
    }

    /// A peer that sends the head and then nothing is told to stop, within a time: it does not keep one of the few connections
    /// for as long as it likes.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_peer_that_sends_the_head_of_a_start_and_no_body_is_told_to_stop_waiting() {
        let server = Server::start(&[]);
        server.directory_of("a1");
        let request = b"POST /v1/attempts/a1 HTTP/1.1\r\nHost: node\r\nConnection: close\r\nContent-Length: 40\r\n\r\n";
        let (status, body, _) = server.send(request).await;
        assert_eq!(status, 408);
        assert!(is_closed_body(&body));
        assert_eq!(server.admission.engine().launches_made(), 0);
    }

    /// One connection serves more than one request: the answers are bounded by their own length, not by the connection closing.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_connection_that_is_kept_serves_one_request_after_another() {
        let server = Server::start(&[]);
        let mut stream = UnixStream::connect(&server.socket)
            .await
            .expect("a connection");
        for _ in 0..3 {
            stream
                .write_all(b"GET /v1/attempts/a1/wait HTTP/1.1\r\nHost: node\r\n\r\n")
                .await
                .expect("a request");
            let (status, body) = read_response(&mut stream).await;
            assert_eq!(read_wait(status, &body), WaitOutcome::Ended);
        }
    }

    /// What the lake runner does when it gives up on a start: it closes the connection. The start goes on to its end all the
    /// same, so that the container the lake runner will ask to kill is there to be killed.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_client_that_closes_while_its_start_is_inside_the_engine_does_not_stop_the_start() {
        let server = Server::start(&[]);
        server.directory_of("a1");
        server.admission.engine().gate_launches();
        let mut stream = UnixStream::connect(&server.socket)
            .await
            .expect("a connection");
        let request = format!(
            "POST /v1/attempts/a1 HTTP/1.1\r\nHost: node\r\nContent-Length: {}\r\n\r\n{SMALL}",
            SMALL.len()
        );
        stream
            .write_all(request.as_bytes())
            .await
            .expect("a request");
        server.admission.engine().wait_entered().await;
        drop(stream); // the lake runner gave up
        server.admission.engine().ungate_launches();
        assert!(
            eventually(|| server.admission.engine().alive() == 1).await,
            "the start was dropped with the connection and made no container"
        );
    }

    /// And the place the dropped start held is not kept for ever: a node whose lake runner times out a few times is not a node
    /// that refuses everything until it is restarted.
    #[tokio::test(flavor = "multi_thread")]
    async fn starts_whose_clients_gave_up_do_not_keep_their_places() {
        let server = Server::start(&[("SAGAT_LAKE_CEILING", "1")]);
        server.directory_of("a1");
        server.directory_of("a2");
        server.admission.engine().gate_launches();
        server.admission.engine().set_launch_fails(Some(
            sagat_infra_sandbox::lake::engine::EngineError::Unavailable,
        ));
        let mut stream = UnixStream::connect(&server.socket)
            .await
            .expect("a connection");
        let request = format!(
            "POST /v1/attempts/a1 HTTP/1.1\r\nHost: node\r\nContent-Length: {}\r\n\r\n{SMALL}",
            SMALL.len()
        );
        stream
            .write_all(request.as_bytes())
            .await
            .expect("a request");
        server.admission.engine().wait_entered().await;
        drop(stream);
        assert_eq!(
            server.admission.places_held(),
            1,
            "the start did not hold its place while it was inside the engine"
        );
        server.admission.engine().ungate_launches();
        // the start fails in the engine, with nobody on the other end, and lets go of its place
        assert!(
            eventually(|| server.admission.places_held() == 0).await,
            "the place of a start whose client gave up was kept"
        );
        server.admission.engine().set_launch_fails(None);
        // the one place of the node is free again, once the start that failed has let go of it
        let mut started = false;
        for _ in 0..200 {
            let (status, body, _) = server
                .post("a2", r#"{"slot":4,"class":"small","deadline_ms":60000}"#)
                .await;
            if read_start(status, &body) == StartOutcome::Started {
                started = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        assert!(
            started,
            "the place of a start whose client gave up was never given back"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn headers_over_the_bound_are_refused_with_a_closed_body() {
        let server = Server::start(&[]);
        let padding = "a".repeat(MAX_HEADER_BYTES + 100);
        let request = format!(
            "GET /v1/attempts/a1/wait HTTP/1.1\r\nHost: node\r\nX-Padding: {padding}\r\nConnection: close\r\n\r\n"
        );
        let (status, body, _) = server.send(request.as_bytes()).await;
        assert_eq!(status, 431);
        assert!(is_closed_body(&body));
        // and a request with headers within it is served
        let request = "GET /v1/attempts/a1/wait HTTP/1.1\r\nHost: node\r\ntraceparent: 00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01\r\nConnection: close\r\n\r\n";
        let (status, body, _) = server.send(request.as_bytes()).await;
        assert_eq!(read_wait(status, &body), WaitOutcome::Ended);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_engine_that_cannot_be_asked_is_unavailable_and_says_nothing_of_why() {
        let server = Server::start(&[]);
        server.directory_of("a1");
        server.admission.engine().set_list_fails(true);
        let (status, body, _) = server.post("a1", SMALL).await;
        assert_eq!(status, 503);
        assert_eq!(body, br#"{"result":"unavailable"}"#);
        let (status, body, _) = server.get("/v1/attempts/a1/wait").await;
        assert_eq!(status, 503);
        assert_eq!(read_wait(status, &body), WaitOutcome::Unavailable);
        // a kill the engine cannot confirm is "not ended", with the status of an answer
        let (status, body, _) = server.delete("a1").await;
        assert_eq!(
            read_kill(status, &body),
            KillOutcome {
                ended: false,
                memory_killed: false
            }
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_draining_node_refuses_starts_and_still_answers_the_rest() {
        let server = Server::start(&[]);
        server.directory_of("a1");
        let (status, body, _) = server.post("a1", SMALL).await;
        assert_eq!(read_start(status, &body), StartOutcome::Started);
        server.admission.drain();
        let (status, body, _) = server.post("a1", SMALL).await;
        assert_eq!(status, 503, "{}", String::from_utf8_lossy(&body));
        let (status, body, _) = server.get("/v1/attempts/a1/wait").await;
        assert_eq!(read_wait(status, &body), WaitOutcome::Running);
        let (status, body, _) = server.delete("a1").await;
        assert_eq!(
            read_kill(status, &body),
            KillOutcome {
                ended: true,
                memory_killed: false
            }
        );
    }

    /// What was refused made no container, whatever the refusal.
    #[tokio::test(flavor = "multi_thread")]
    async fn nothing_that_was_refused_made_a_container() {
        let server = Server::start(&[]);
        server.directory_of("a1");
        for (attempt, body) in [
            (
                "a1",
                r#"{"slot":3,"class":"small","deadline_ms":60000,"image":"x"}"#,
            ),
            ("a1", r#"{"slot":4096,"class":"small","deadline_ms":60000}"#),
            ("a1", r#"{"slot":3,"class":"small","deadline_ms":0}"#),
            ("a1", r#"{"slot":3,"class":"small","deadline_ms":99999999}"#),
            ("A1", SMALL),
        ] {
            let _ = server.post(attempt, body).await;
        }
        assert_eq!(server.admission.engine().launches_made(), 0);
    }
}
