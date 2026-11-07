//! Serving the lake sandbox API (feature 035, ADR-0079 rules 3, 4, 8 and 13).
//!
//! The socket lives in a directory this process owns, mode 0750, which the lake runner's group may search and not write; the
//! socket is 0660 in that group. The directory is what keeps the socket from everyone else from the moment it is bound, before
//! it has its mode, and, because the group cannot write in it, what keeps the lake runner from swapping the socket for a link
//! while this process is changing its mode. So the directory is checked, not trusted: it must be a real directory (not a link),
//! owned by this user, and hold nothing but the socket and the lock. An exclusive lock on the lock file keeps one process to
//! the socket: a second one is refused, instead of taking a live socket from the first and having its own deleted when the
//! first exits.
//!
//! The listener serves one user, the lake runner's; anyone else is dropped at accept.
//!
//! On the stop signal the node drains: a start is refused from that moment, a wait and a kill are answered for the drain time,
//! and then the listener stops and its connections finish within a grace. The containers are not touched: they end by their own
//! deadline label, and the next process reads them from the engine. (An orchestrator must give the process at least the drain
//! time and the grace before it kills it.)

use std::fs::{File, OpenOptions};
use std::io;
use std::os::unix::fs::{
    FileTypeExt as _, MetadataExt as _, OpenOptionsExt as _, PermissionsExt as _, chown,
};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rustix::fs::{FlockOperation, flock};
use sagat_api_http::{ServerConfig, serve, serve_unix};
use sagat_control_plane::config::Environment;
use tokio::net::UnixListener;
use tokio::sync::watch;

use super::admission::Admission;
use super::api::router;
use super::config::{LOCK_NAME, LakeConfig, SOCKET_NAME};
use super::engine::Engine;

/// The sub-path mount, which is what keeps a container to its own attempt directory, came with API 1.45.
const MIN_API: (u32, u32) = (1, 45);

/// A bound socket, and the lock that keeps one process to it for as long as this lives.
pub struct Bound {
    pub listener: UnixListener,
    _lock: File,
}

fn refuse(kind: io::ErrorKind, reason: &str) -> io::Error {
    io::Error::new(kind, reason.to_owned())
}

/// Makes the socket directory what the API needs it to be, takes the lock, takes the stale socket of a process that died, and
/// binds.
///
/// # Errors
/// The directory is not one this process may use as it is (a link, not its own, holding something else), the lock is held by
/// another process, or the directory or the socket could not be made as required.
pub fn prepare_socket(config: &LakeConfig) -> io::Result<Bound> {
    let directory = &config.socket_dir;
    match std::fs::symlink_metadata(directory) {
        Ok(found) if found.file_type().is_symlink() => {
            return Err(refuse(
                io::ErrorKind::InvalidInput,
                "the lake socket directory is a link",
            ));
        }
        Ok(found) if !found.is_dir() => {
            return Err(refuse(
                io::ErrorKind::InvalidInput,
                "the lake socket directory is not a directory",
            ));
        }
        Ok(found) => {
            // the owner decides who may replace what is in it
            if found.uid() != rustix::process::geteuid().as_raw() {
                return Err(refuse(
                    io::ErrorKind::PermissionDenied,
                    "the lake socket directory is not owned by the user of this process",
                ));
            }
            for entry in std::fs::read_dir(directory)? {
                let name = entry?.file_name();
                if name != SOCKET_NAME && name != LOCK_NAME {
                    return Err(refuse(
                        io::ErrorKind::InvalidInput,
                        "the lake socket directory holds something that is not the socket",
                    ));
                }
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            std::fs::create_dir_all(directory)?
        }
        Err(error) => return Err(error),
    }
    std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o750))?;
    chown(directory, None, Some(config.group))?;
    // the group may search the directory and may not write in it
    if std::fs::metadata(directory)?.permissions().mode() & 0o777 != 0o750 {
        return Err(refuse(
            io::ErrorKind::PermissionDenied,
            "the lake socket directory could not be made 0750",
        ));
    }
    let lock = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .mode(0o600)
        .open(directory.join(LOCK_NAME))?;
    flock(&lock, FlockOperation::NonBlockingLockExclusive).map_err(|_| {
        refuse(
            io::ErrorKind::WouldBlock,
            "another lake sandbox runner is serving this socket",
        )
    })?;
    // the lock is ours: whatever socket is here belongs to a process that is gone
    let path = config.socket_path();
    match std::fs::symlink_metadata(&path) {
        Ok(found) if found.file_type().is_socket() => std::fs::remove_file(&path)?,
        Ok(_) => {
            return Err(refuse(
                io::ErrorKind::AlreadyExists,
                "something that is not a socket is where the lake socket goes",
            ));
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let listener = UnixListener::bind(&path)?;
    // the directory has kept everyone out until now; this is what the lake runner's group may do in it
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o660))?;
    chown(&path, None, Some(config.group))?;
    Ok(Bound {
        listener,
        _lock: lock,
    })
}

/// The check the listener makes on each connection: the lake runner's user and nobody else.
fn only(peer: u32) -> impl Fn(u32) -> bool + Send + Sync + 'static {
    move |uid| uid == peer
}

fn now_epoch_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

/// `/livez`: the process answers.
fn live() -> axum::Router {
    axum::Router::new().route("/livez", axum::routing::get(|| async { "ok" }))
}

/// What is checked before anything is bound, so that a node that cannot work does not start: the engine speaks an API that has
/// sub-path mounts, the work volume is where this process says it is, and the lake runner is not this process's own user
/// (nothing would be separated; a development machine, with one user, is the exception).
async fn check<E: Engine>(config: &LakeConfig, engine: &E) -> Result<(), String> {
    match engine.api_version().await {
        Ok(version) if version >= MIN_API => {}
        Ok((major, minor)) => {
            return Err(format!(
                "the container engine speaks API {major}.{minor}; the sub-path mounts that keep a container to its attempt directory need {}.{}",
                MIN_API.0, MIN_API.1
            ));
        }
        Err(_) => {
            return Err("the container engine could not be asked which API it speaks".to_owned());
        }
    }
    match tokio::fs::metadata(&config.work_root).await {
        Ok(found) if found.is_dir() => {}
        _ => return Err("SAGAT_LAKE_WORK_ROOT is not a directory this process can see".to_owned()),
    }
    if config.environment != Environment::Dev
        && config.peer_uid == rustix::process::geteuid().as_raw()
    {
        return Err(
            "SAGAT_LAKE_RUNNER_UID is the user of this process: the socket would separate nothing outside SAGAT_ENV=dev"
                .to_owned(),
        );
    }
    Ok(())
}

/// Serves the API until `stop` says so, then drains.
///
/// # Errors
/// The node cannot work as configured, the socket could not be prepared, or the listener failed.
pub async fn run<E: Engine>(
    config: LakeConfig,
    engine: E,
    stop: watch::Receiver<bool>,
) -> Result<(), String> {
    check(&config, &engine).await?;
    // bound first: whatever can fail before the socket exists fails before the socket exists
    let admin = match config.admin_addr {
        Some(address) => Some(
            tokio::net::TcpListener::bind(address)
                .await
                .map_err(|error| format!("the admin listener could not be bound: {error}"))?,
        ),
        None => None,
    };
    let bound = prepare_socket(&config)
        .map_err(|error| format!("the lake socket could not be prepared: {error}"))?;
    metrics::gauge!("lake_sandbox_ceiling").set(config.ceiling as f64);
    let (peer, ceiling, drain, sweep_every, path) = (
        config.peer_uid,
        config.ceiling,
        config.drain,
        config.sweep_interval,
        config.socket_path(),
    );
    let admission = Arc::new(Admission::new(engine, config, Arc::new(now_epoch_secs)));
    // what a process that died left behind is ended before the first start is taken
    admission.sweep_once().await;
    let sweeper = {
        let admission = Arc::clone(&admission);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(sweep_every).await;
                admission.sweep_once().await;
            }
        })
    };
    let admin = admin.map(|listener| {
        tokio::spawn(serve(
            listener,
            sagat_control_plane::admin_router_with(live()),
            ServerConfig::default(),
            sagat_control_plane::stopped(stop.clone()),
        ))
    });
    let stopping = {
        let admission = Arc::clone(&admission);
        let stopped = sagat_control_plane::stopped(stop);
        async move {
            stopped.await;
            admission.drain();
            tracing::info!(
                drain_secs = drain.as_secs(),
                "lake sandbox: draining; starts are refused"
            );
            tokio::time::sleep(drain).await;
        }
    };
    let served = serve_unix(
        bound.listener,
        only(peer),
        router(Arc::clone(&admission)),
        ServerConfig {
            header_read_timeout: Duration::from_secs(5),
            max_connections: ceiling.saturating_mul(4).saturating_add(8),
            write_stall_timeout: Duration::from_secs(10),
            shutdown_grace: Duration::from_secs(10),
        },
        stopping,
    )
    .await;
    sweeper.abort();
    if let Some(admin) = admin {
        match admin.await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => tracing::warn!(%error, "the admin listener ended with an error"),
            Err(error) => tracing::warn!(%error, "the admin listener task failed"),
        }
    }
    // the lock held by `bound` is still ours: this socket is ours to take away
    let _ = std::fs::remove_file(&path);
    served.map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use sagat_infra_lake::sandbox_api::{StartOutcome, read_start};
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    use tokio::net::UnixStream;

    use super::super::engine::fake::Fake;
    use super::*;

    struct Env {
        socket_dir: tempfile::TempDir,
        work: tempfile::TempDir,
        config: LakeConfig,
    }

    fn env_with(extra: &[(&str, &str)]) -> Env {
        let socket_dir = tempfile::tempdir().expect("a directory");
        let work = tempfile::tempdir().expect("a directory");
        let metadata = std::fs::metadata(work.path()).expect("metadata");
        let (uid, gid) = (metadata.uid(), metadata.gid());
        // a group this process belongs to: it may give the directory and the socket to it
        let mut values: HashMap<String, String> = [
            (
                "SAGAT_LAKE_SOCKET_DIR",
                socket_dir
                    .path()
                    .join("lake")
                    .to_string_lossy()
                    .into_owned(),
            ),
            ("SAGAT_LAKE_RUNNER_UID", uid.to_string()),
            ("SAGAT_LAKE_RUNNER_GID", gid.to_string()),
            ("SAGAT_LAKE_WORK_VOLUME", "sagat-lake-work".to_owned()),
            (
                "SAGAT_LAKE_WORK_ROOT",
                work.path().to_string_lossy().into_owned(),
            ),
            ("SAGAT_LAKE_IMAGE", "sagat/lake@sha256:abc".to_owned()),
            ("SAGAT_LAKE_OWNER", "lake-runner".to_owned()),
            ("SAGAT_LAKE_NODE_MEMORY_MIB", "8192".to_owned()),
            ("SAGAT_LAKE_DRAIN_SECS", "1".to_owned()),
            ("SAGAT_ADMIN_ADDR", "off".to_owned()),
            // one user on this machine: the lake runner is the user of the process, which only a development environment allows
            ("SAGAT_ENV", "dev".to_owned()),
        ]
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value))
        .collect();
        for (key, value) in extra {
            values.insert((*key).to_owned(), (*value).to_owned());
        }
        let lookup = |name: &str| values.get(name).cloned();
        let config = LakeConfig::from_lookup(&lookup).expect("a valid configuration");
        Env {
            socket_dir,
            work,
            config,
        }
    }

    fn env() -> Env {
        env_with(&[])
    }

    #[tokio::test]
    async fn the_directory_is_the_owners_and_the_groups_to_search_and_the_socket_the_groups_to_use()
    {
        let env = env();
        let bound = prepare_socket(&env.config).expect("a socket");
        let directory = std::fs::metadata(&env.config.socket_dir).expect("the directory");
        assert_eq!(directory.permissions().mode() & 0o777, 0o750);
        let socket = std::fs::metadata(env.config.socket_path()).expect("the socket");
        assert_eq!(socket.permissions().mode() & 0o777, 0o660);
        assert!(socket.file_type().is_socket());
        assert_eq!(socket.gid(), env.config.group);
        assert_eq!(directory.gid(), env.config.group);
        // and the lock is not the group's
        let lock = std::fs::metadata(env.config.socket_dir.join(LOCK_NAME)).expect("the lock");
        assert_eq!(lock.permissions().mode() & 0o777, 0o600);
        drop(bound);
    }

    /// A process that died leaves its socket; the next one takes the path.
    #[tokio::test]
    async fn a_stale_socket_of_a_process_that_died_is_taken_over() {
        let env = env();
        drop(prepare_socket(&env.config).expect("a first socket"));
        assert!(env.config.socket_path().exists());
        prepare_socket(&env.config).expect("a second socket over the stale one");
    }

    /// One process to a socket: a second does not take a live one from the first, and so cannot be left unreachable when the
    /// first exits and takes its socket away.
    #[tokio::test]
    async fn a_second_process_is_refused_while_the_first_holds_the_socket() {
        let env = env();
        let first = prepare_socket(&env.config).expect("a first socket");
        let refused = prepare_socket(&env.config).err().expect("a refusal");
        assert_eq!(refused.kind(), io::ErrorKind::WouldBlock, "{refused}");
        assert!(
            env.config.socket_path().exists(),
            "the live socket was taken"
        );
        drop(first);
        prepare_socket(&env.config).expect("the lock is free when its holder has gone");
    }

    /// Something that is not a socket where the socket goes is not removed: that would be deleting what someone put there.
    #[tokio::test]
    async fn a_file_that_is_not_a_socket_is_not_removed() {
        let env = env();
        std::fs::create_dir_all(&env.config.socket_dir).expect("the directory");
        std::fs::write(env.config.socket_path(), b"precious").expect("a file");
        let error = prepare_socket(&env.config).err().expect("a refusal");
        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(
            std::fs::read(env.config.socket_path()).expect("the file"),
            b"precious"
        );
    }

    /// The directory is used as it is only if it is a real directory of this user that holds nothing but the socket and the
    /// lock: otherwise its mode and its group would be changed on something that was never meant for it.
    #[tokio::test]
    async fn a_directory_that_is_a_link_or_holds_something_else_is_refused_untouched() {
        let env = env();
        // holds something else
        std::fs::create_dir_all(&env.config.socket_dir).expect("the directory");
        std::fs::write(env.config.socket_dir.join("other.txt"), b"x").expect("a file");
        std::fs::set_permissions(
            &env.config.socket_dir,
            std::fs::Permissions::from_mode(0o700),
        )
        .expect("a mode");
        let error = prepare_socket(&env.config).err().expect("a refusal");
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput, "{error}");
        assert_eq!(
            std::fs::metadata(&env.config.socket_dir)
                .expect("metadata")
                .permissions()
                .mode()
                & 0o777,
            0o700,
            "its mode was changed"
        );

        // a link, also to a directory that is there and empty
        let target = env.socket_dir.path().join("target");
        std::fs::create_dir(&target).expect("a target");
        let mode_of = |path: &std::path::Path| {
            std::fs::metadata(path)
                .expect("metadata")
                .permissions()
                .mode()
                & 0o777
        };
        let before = mode_of(&target);
        let link = env.socket_dir.path().join("link");
        std::os::unix::fs::symlink(&target, &link).expect("a link");
        let mut through_a_link = env.config.clone();
        through_a_link.socket_dir = link;
        let error = prepare_socket(&through_a_link).err().expect("a refusal");
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput, "{error}");
        assert_eq!(mode_of(&target), before, "the target of a link was changed");
        assert!(
            std::fs::read_dir(&target)
                .expect("the target")
                .next()
                .is_none(),
            "something was made in the target of a link"
        );
    }

    /// A directory of someone else is not made the lake's: its owner would still decide what is in it.
    #[tokio::test]
    async fn a_directory_that_belongs_to_another_user_is_refused() {
        if rustix::process::geteuid().is_root() {
            return; // root owns /usr and so does this process: nothing to show
        }
        let mut config = env().config;
        config.socket_dir = "/usr".into();
        let error = prepare_socket(&config).err().expect("a refusal");
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied, "{error}");
    }

    #[test]
    fn the_listener_serves_the_lake_runners_user_and_nobody_else() {
        let check = only(10_001);
        assert!(check(10_001));
        for other in [0, 1, 10_000, 10_002, 65_534, u32::MAX] {
            assert!(!check(other), "{other}");
        }
    }

    async fn post(path: &std::path::Path, attempt: &str) -> (u16, Vec<u8>) {
        let body = r#"{"slot":3,"class":"small","deadline_ms":60000}"#;
        let request = format!(
            "POST /v1/attempts/{attempt} HTTP/1.1\r\nHost: node\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        );
        let mut stream = UnixStream::connect(path).await.expect("a connection");
        stream
            .write_all(request.as_bytes())
            .await
            .expect("a request");
        let mut got = Vec::new();
        tokio::time::timeout(Duration::from_secs(10), stream.read_to_end(&mut got))
            .await
            .expect("an answer")
            .expect("a read");
        let split = got
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .expect("a head");
        let head = String::from_utf8_lossy(&got[..split]).into_owned();
        let status = head
            .split(' ')
            .nth(1)
            .and_then(|s| s.parse().ok())
            .expect("a status");
        (status, got[split + 4..].to_vec())
    }

    /// The whole of it over the real socket: it starts, it serves the lake runner's user, it drains and stops on the signal,
    /// and it takes its socket away.
    #[tokio::test(flavor = "multi_thread")]
    async fn it_serves_over_its_socket_stops_on_the_signal_and_takes_the_socket_away() {
        let env = env();
        std::fs::create_dir(env.work.path().join("a1")).expect("an attempt directory");
        let path = env.config.socket_path();
        let (stop, stopped) = watch::channel(false);
        let task = tokio::spawn(run(env.config.clone(), Fake::new(), stopped));
        // the socket appears when the listener is bound
        for _ in 0..200 {
            if path.exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        let (status, body) = post(&path, "a1").await;
        assert_eq!(read_start(status, &body), StartOutcome::Started);

        stop.send(true).expect("the signal");
        // from the signal on, a start is refused: this looks until it sees that, and gives up after a few seconds
        let mut refused = false;
        for _ in 0..200 {
            let (status, _) = post(&path, "a1").await;
            if status == 503 {
                refused = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        assert!(refused, "the node kept starting after the stop signal");
        let ended = tokio::time::timeout(Duration::from_secs(30), task)
            .await
            .expect("it stops after the drain")
            .expect("the task");
        assert_eq!(ended, Ok(()));
        assert!(!path.exists(), "the socket was left behind");
        let _ = env.socket_dir;
    }

    #[tokio::test]
    async fn an_engine_older_than_the_sub_path_mount_is_refused_before_anything_is_bound() {
        let env = env();
        let engine = Fake::new();
        engine.set_api_version(Some((1, 44)));
        let (_stop, stopped) = watch::channel(false);
        // bounded: a node that accepted the engine would serve for ever, and the case would hang instead of failing
        let error = tokio::time::timeout(
            Duration::from_secs(5),
            run(env.config.clone(), engine, stopped),
        )
        .await
        .expect("the node started on an engine without sub-path mounts")
        .expect_err("a refusal");
        assert!(error.contains("1.44") && error.contains("1.45"), "{error}");
        assert!(!env.config.socket_dir.exists(), "something was made");
        // and 1.45 itself is enough
        let engine = Fake::new();
        engine.set_api_version(Some((1, 45)));
        assert_eq!(check(&env.config, &engine).await, Ok(()));
    }

    #[tokio::test]
    async fn a_work_volume_that_is_not_there_is_refused_at_the_start() {
        let mut env = env();
        env.config.work_root = env.work.path().join("nothing-here");
        let error = check(&env.config, &Fake::new())
            .await
            .expect_err("a refusal");
        assert!(error.contains("SAGAT_LAKE_WORK_ROOT"), "{error}");
    }

    /// Outside development the lake runner is another user than this process: the socket would separate nothing otherwise.
    #[tokio::test]
    async fn outside_development_the_lake_runner_is_not_the_user_of_this_process() {
        let prod = env_with(&[("SAGAT_ENV", "prod")]);
        let error = check(&prod.config, &Fake::new())
            .await
            .expect_err("a refusal");
        assert!(error.contains("SAGAT_LAKE_RUNNER_UID"), "{error}");
        // in development, with one user, it is allowed
        assert_eq!(check(&env().config, &Fake::new()).await, Ok(()));
    }

    /// An admin listener that cannot be bound fails the start before there is a socket: a node that did not start leaves
    /// nothing listening that nobody serves.
    #[tokio::test]
    async fn an_admin_listener_that_cannot_be_bound_leaves_no_socket_behind() {
        let taken = std::net::TcpListener::bind("127.0.0.1:0").expect("a port");
        let address = taken.local_addr().expect("an address").to_string();
        let env = env_with(&[("SAGAT_ADMIN_ADDR", address.as_str())]);
        let (_stop, stopped) = watch::channel(false);
        let error = run(env.config.clone(), Fake::new(), stopped)
            .await
            .expect_err("a refusal");
        assert!(error.contains("admin"), "{error}");
        assert!(!env.config.socket_path().exists());
        assert!(!env.config.socket_dir.exists());
    }
}
