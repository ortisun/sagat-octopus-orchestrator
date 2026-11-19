//! Limits for the stages of a test that can stall without failing: pulling and starting a container,
//! a child process, and a whole scenario that waits on barriers.
//!
//! A test that hangs costs the CI budget (a job has 6 hours by default) and says nothing. Every wait
//! here ends in a panic that names the stage and the limit, so the log shows where it stopped.
//! Limits are generous for a cold runner; a stage that needs more is a defect to explain, not a number
//! to raise. A panic from a stage that only awaits is raised by the runtime the test runs on, so the
//! stage must not block that thread (a barrier, a channel or a socket is fine; `std::thread::sleep`
//! is not).
use std::{
    fmt::Display,
    future::Future,
    sync::{Arc, Mutex},
    time::Duration,
};
use testcontainers_modules::{
    postgres::Postgres,
    testcontainers::{
        ContainerAsync, GenericImage, Image, ImageExt, core::WaitFor, runners::AsyncRunner,
    },
};

/// Image pull plus container start-up (including the wait for its ready message).
pub const START_LIMIT: Duration = Duration::from_secs(300);
/// One scenario from the first container to the last assertion.
pub const LIFECYCLE_LIMIT: Duration = Duration::from_secs(300);
/// A child process that builds or drives a browser (npm, Playwright).
pub const PROCESS_LIMIT: Duration = Duration::from_secs(1200);

/// The OpenBao image every test starts. One place, so a registry or tag change is one edit.
pub const OPENBAO_IMAGE: (&str, &str) = ("openbao/openbao", "2.7.1");
/// The message the development server prints once it listens.
const OPENBAO_READY: &str = "Development mode should NOT be used in production";

/// Runs `work` for at most `limit`; otherwise panics naming `stage`.
pub async fn within<T>(stage: impl Display, limit: Duration, work: impl Future<Output = T>) -> T {
    match tokio::time::timeout(limit, work).await {
        Ok(value) => value,
        Err(_) => panic!("stage `{stage}` did not finish within {limit:?}"),
    }
}

/// The host port Docker published for `internal` of `container`, waiting up to a minute for it to be published.
///
/// Right after a container starts, Docker Desktop can answer that the port is not exposed (`PortNotExposed`) and publish it a
/// moment later. The shared PostgreSQL server of every suite took that answer as final: the first test panicked holding the
/// lock of the server and the other 308 of the run failed with `PoisonError` (2026-11-11, the runtime-role run of the schedules
/// suite; the 129 tests that failed at once on 2026-11-10 were probably the same). A port that never appears still ends in a
/// panic that names the port and the last answer.
pub async fn mapped_port<I: Image>(container: &ContainerAsync<I>, internal: u16) -> u16 {
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    loop {
        match container.get_host_port_ipv4(internal).await {
            Ok(port) => return port,
            Err(_) if std::time::Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
            Err(error) => panic!(
                "the host port of container port {internal} was not published within 60 s: {error}"
            ),
        }
    }
}

/// Starts a development-mode OpenBao with `root_token` and waits for it to listen.
pub async fn start_openbao(root_token: &str) -> ContainerAsync<GenericImage> {
    let (name, tag) = OPENBAO_IMAGE;
    let started = GenericImage::new(name, tag)
        .with_wait_for(WaitFor::message_on_stdout(OPENBAO_READY))
        .with_exposed_port(8200.into())
        .with_env_var("BAO_DEV_ROOT_TOKEN_ID", root_token)
        .with_cmd(["server", "-dev", "-dev-listen-address=0.0.0.0:8200"])
        .start();
    within(format!("pull and start {name}:{tag}"), START_LIMIT, started)
        .await
        .unwrap_or_else(|error| panic!("start {name}:{tag}: {error}"))
}

/// Starts the PostgreSQL 16 a managed connection points at.
///
/// The container is returned once a query works on its published port. On Docker Desktop for macOS that port can share
/// its number with a listener this process already holds on 127.0.0.1, which then answers instead of PostgreSQL
/// (`unexpected response from SSLRequest: 0x48`, D043-171); such a container is replaced by one on another port, at most
/// three times.
pub async fn start_postgres_target() -> ContainerAsync<Postgres> {
    for attempt in 1..=3 {
        let started = Postgres::default().with_tag("16-alpine").start();
        let container = within("pull and start postgres:16-alpine", START_LIMIT, started)
            .await
            .unwrap_or_else(|error| panic!("start postgres:16-alpine: {error}"));
        let port = mapped_port(&container, 5432).await;
        let url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
        match super::wait_until_queryable(&url).await {
            Ok(()) => return container,
            Err(super::AnotherServer) => {
                eprintln!(
                    "the host port {port} of the target PostgreSQL is answered by another server; starting another container ({attempt} of 3)"
                );
                drop(container);
            }
        }
    }
    panic!(
        "three target PostgreSQL containers in a row were published on a port another server answers"
    )
}

/// Runs a child process to its end, killing it (the command must set `kill_on_drop`) at `limit`.
pub async fn exit_status(
    stage: impl Display,
    limit: Duration,
    mut command: tokio::process::Command,
) -> std::process::ExitStatus {
    command.kill_on_drop(true);
    within(&stage, limit, command.status())
        .await
        .unwrap_or_else(|error| panic!("stage `{stage}` could not run: {error}"))
}

/// The scenario step in progress, so a stall names it.
#[derive(Clone, Default)]
pub struct Progress(Arc<Mutex<String>>);

impl Progress {
    /// Records that the scenario entered `stage`.
    pub fn enter(&self, stage: impl Into<String>) {
        if let Ok(mut current) = self.0.lock() {
            *current = stage.into();
        }
    }

    fn current(&self) -> String {
        self.0
            .lock()
            .map_or_else(|_| "unknown".into(), |stage| stage.clone())
    }
}

/// Runs a whole scenario for at most `limit`, passing it the [`Progress`] it reports to.
pub async fn lifecycle<T, F, Fut>(name: &str, limit: Duration, scenario: F) -> T
where
    F: FnOnce(Progress) -> Fut,
    Fut: Future<Output = T>,
{
    let progress = Progress::default();
    progress.enter("start");
    match tokio::time::timeout(limit, scenario(progress.clone())).await {
        Ok(value) => value,
        Err(_) => panic!(
            "scenario `{name}` did not finish within {limit:?}; it was in stage `{}`",
            progress.current()
        ),
    }
}
