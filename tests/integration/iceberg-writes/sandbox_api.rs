//! ADR-0079, rule 14: the client of the lake runner, the server of the sandbox runner and a real container engine, on a real
//! socket. The cases of `apps/sandbox-runner/src/lake/roundtrip.rs` run the client against the real router with an engine in
//! memory, and `lifecycle.rs` runs the engine's calls against a real engine; what only this file shows is the join: the
//! container the API asks for is made by the Docker adapter from the node's configuration, started in the lake profile with a
//! real sub-path mount of a real work volume, labelled as the node reads it back, and ended by the call that follows.
//!
//! The image is the isolation image (`python:3.13-alpine` unless `SAGAT_LAKE_ISOLATION_IMAGE` names another). It has no
//! `host.py`, so the container the API starts ends on its own a moment after it starts, with the interpreter's own error:
//! what holds a container up (the ceiling, the deadline sweep, a kill of a running one) is shown against the engine in
//! `lifecycle.rs` and against the router in memory, not here.
//!
//! The directory the API checks (`SAGAT_LAKE_WORK_ROOT`) is a local one and the volume the container mounts is the engine's own
//! (memory-backed, held by a stand-in for the lake runner): in a deployment they are one volume, here they are two so that the
//! cases run against an engine in another machine too. What is not shown is that the two are the same volume.
//!
//! The node needs to run as a user that is not root and not in the range of the lake containers (40000 and above).

use std::collections::HashMap;
use std::os::unix::fs::MetadataExt as _;
use std::time::Duration;

use sagat_infra_lake::attempt::{LakeSandbox as _, SandboxError, StartRequest};
use sagat_infra_lake::sandbox_api::client::SocketSandbox;
use sagat_infra_lake::sandbox_api::{Class, Reason};
use sagat_infra_sandbox::lake::engine::{LakeContainer, LakeEngine};
use sagat_sandbox_runner::lake::engine::DockerEngine;
use sagat_sandbox_runner::lake::{LakeConfig, SOCKET_NAME, serve};
use tokio::sync::watch;
use tokio::task::JoinHandle;

use crate::isolation::{image, work_volume_with_holder};
use crate::support::bounded::{LIFECYCLE_LIMIT, START_LIMIT, within};

const SLOT: u32 = 5;
const PREPARE_WAIT: Duration = Duration::from_secs(60);

/// A sandbox runner in lake mode on a socket of its own, against the engine of the environment, with the stand-in for the lake
/// runner that holds the work volume.
struct Node {
    engine: LakeEngine,
    holder: LakeContainer,
    volume: String,
    /// The label of the node's containers: its own, so that no case ends another's.
    owner: String,
    work_root: tempfile::TempDir,
    /// The user and the group of the node's process and of the directories it checks.
    uid: u32,
    gid: u32,
    stop: watch::Sender<bool>,
    server: Option<JoinHandle<Result<(), String>>>,
    sandbox: SocketSandbox,
    _socket_dir: tempfile::TempDir,
}

impl Node {
    async fn new() -> Self {
        let engine =
            LakeEngine::connect().expect("a container engine (DOCKER_HOST, or the local socket)");
        within(
            "pull the isolation image",
            START_LIMIT,
            engine.ensure_image(&image()),
        )
        .await
        .expect("the isolation image is on the engine or can be pulled");
        let owner = format!("sandbox-api-{}", uuid::Uuid::now_v7());
        let (volume, holder) = work_volume_with_holder(&engine, &format!("{owner}-holder")).await;

        let work_root = tempfile::tempdir().expect("a directory");
        let metadata = std::fs::metadata(work_root.path()).expect("metadata");
        let (uid, gid) = (metadata.uid(), metadata.gid());
        let socket_dir = tempfile::tempdir().expect("a directory");
        let socket_parent = socket_dir.path().join("lake");

        let values: HashMap<&str, String> = [
            (
                "SAGAT_LAKE_SOCKET_DIR",
                socket_parent.to_string_lossy().into_owned(),
            ),
            ("SAGAT_LAKE_RUNNER_UID", uid.to_string()),
            ("SAGAT_LAKE_RUNNER_GID", gid.to_string()),
            ("SAGAT_LAKE_WORK_VOLUME", volume.clone()),
            (
                "SAGAT_LAKE_WORK_ROOT",
                work_root.path().to_string_lossy().into_owned(),
            ),
            ("SAGAT_LAKE_IMAGE", image()),
            ("SAGAT_LAKE_OWNER", owner.clone()),
            ("SAGAT_LAKE_NODE_MEMORY_MIB", "8192".to_owned()),
            ("SAGAT_LAKE_DRAIN_SECS", "1".to_owned()),
            ("SAGAT_ADMIN_ADDR", "off".to_owned()),
            // the node and the lake runner are one user here, which only a development environment allows
            ("SAGAT_ENV", "dev".to_owned()),
        ]
        .into_iter()
        .collect();
        let lookup = |name: &str| values.get(name).cloned();
        let config = LakeConfig::from_lookup(&lookup).expect("a valid configuration");
        let socket = config.socket_path();

        let (stop, stopped) = watch::channel(false);
        let docker = DockerEngine::connect().expect("the node reaches the engine");
        let server = tokio::spawn(serve::run(config, docker, stopped));
        // the socket exists once the node has made its checks, taken its lock and bound
        within("the node binds its socket", PREPARE_WAIT, async {
            while !socket.exists() {
                if server.is_finished() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await;
        assert!(
            !server.is_finished(),
            "the node stopped before it served: {:?}",
            server.await
        );
        assert_eq!(
            socket.file_name().and_then(|n| n.to_str()),
            Some(SOCKET_NAME)
        );

        Self {
            engine,
            holder,
            volume,
            owner,
            work_root,
            uid,
            gid,
            stop,
            server: Some(server),
            sandbox: SocketSandbox::new(socket).expecting_server(uid),
            _socket_dir: socket_dir,
        }
    }

    /// An attempt the lake runner has prepared: its directory in the volume, for the container to mount, and the directory the
    /// node checks.
    async fn attempt(&self) -> String {
        let id = uuid::Uuid::now_v7().to_string();
        std::fs::create_dir(self.work_root.path().join(&id))
            .expect("the directory the node checks");
        let script = format!(
            "set -e; mkdir /w/{id}; chown {}:{} /w/{id}; chmod 0770 /w/{id}",
            self.uid, self.gid
        );
        let exit = self
            .holder
            .exec(
                vec!["sh".to_owned(), "-c".to_owned(), script],
                PREPARE_WAIT,
                4096,
            )
            .await
            .expect("the preparer runs");
        assert_eq!(exit.code, Some(0), "preparing the attempt: {exit:?}");
        id
    }

    fn request(attempt: &str) -> StartRequest {
        StartRequest {
            attempt_id: attempt.to_owned(),
            slot: SLOT,
            class: Class::Small,
            deadline: Duration::from_secs(120),
        }
    }

    async fn live(&self, attempt: &str) -> Vec<sagat_infra_sandbox::lake::engine::LakeInfo> {
        self.engine
            .list(&self.owner)
            .await
            .expect("the engine lists its containers")
            .into_iter()
            .filter(|info| info.attempt_id == attempt)
            .collect()
    }

    async fn finish(mut self) {
        let _ = self.stop.send(true);
        if let Some(server) = self.server.take() {
            let ended = within("the node drains", LIFECYCLE_LIMIT, server)
                .await
                .expect("the node's task");
            assert_eq!(ended, Ok(()), "the node ended with an error");
        }
        // whatever a case left: the node's own label, at any age
        self.engine.sweep(&self.owner, Duration::ZERO).await;
        self.holder.stop().await;
        self.engine.remove_work_volume(&self.volume).await;
    }
}

/// The whole way: the client asks, the node makes the container from its own configuration, the engine runs it and labels it,
/// the wait says when it is over, and the kill removes it.
#[tokio::test(flavor = "multi_thread")]
async fn an_attempt_is_started_seen_waited_for_and_ended_through_the_client_the_node_and_the_engine()
 {
    let node = Node::new().await;
    let attempt = node.attempt().await;
    let before = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("a clock")
        .as_secs();

    let started = within(
        "start through the API",
        START_LIMIT,
        node.sandbox.start(&Node::request(&attempt)),
    )
    .await;
    assert_eq!(started, Ok(()), "the API did not start the attempt");

    // the container as the engine has it: one, in the slot and the class that were asked for, with the deadline label of the
    // attempt's deadline plus the node's grace
    let ours = node.live(&attempt).await;
    assert_eq!(ours.len(), 1, "containers of the attempt: {ours:?}");
    assert_eq!(ours[0].slot, Some(SLOT));
    assert_eq!(ours[0].class.as_deref(), Some("small"));
    let deadline = ours[0].deadline_epoch_secs.expect("a deadline label");
    assert!(
        deadline >= before + 120 && deadline <= before + 120 + 600,
        "the deadline label {deadline} is not the attempt's 120 s from {before} and a grace"
    );
    assert!(ours[0].memory_bytes.is_some(), "no memory label");

    // this image has no host program, so the container ends by itself; the wait says so and does not before
    within(
        "wait for the container to end",
        LIFECYCLE_LIMIT,
        node.sandbox.exited(&attempt),
    )
    .await;

    let exit = within("kill", LIFECYCLE_LIMIT, node.sandbox.kill(&attempt)).await;
    assert!(exit.ended, "the engine did not confirm the end: {exit:?}");
    assert!(!exit.memory_killed, "{exit:?}");
    assert!(
        node.live(&attempt).await.is_empty(),
        "the container is still there after the kill"
    );
    node.finish().await;
}

/// An attempt whose directory the node cannot see is refused with its reason, and nothing is made.
#[tokio::test(flavor = "multi_thread")]
async fn an_attempt_without_its_directory_is_refused_by_name_and_makes_no_container() {
    let node = Node::new().await;
    let attempt = uuid::Uuid::now_v7().to_string();

    let started = within(
        "start through the API",
        START_LIMIT,
        node.sandbox.start(&Node::request(&attempt)),
    )
    .await;
    assert_eq!(
        started,
        Err(SandboxError::Refused(Reason::Directory)),
        "the attempt had no directory"
    );
    assert!(node.live(&attempt).await.is_empty());
    node.finish().await;
}
