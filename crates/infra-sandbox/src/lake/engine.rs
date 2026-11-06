//! The container engine side of the `lake` profile (feature 035, T035-028).
//!
//! [`super::container_config`] says what a lake container is; this says how one is run against a real engine: made,
//! attached, started, read for a bounded time and a bounded number of bytes, killed if it outstays its deadline and
//! always removed, also if the caller goes away. What comes out is the exit status, whether the memory limit killed it
//! and a bounded copy of its two output streams. The production host writes nothing it must not (D035-062), and the
//! runner keeps no more of its standard error than a class name and a line count, so the bound is a ceiling, not a
//! buffer to be read.

use std::collections::HashMap;
use std::pin::Pin;
use std::time::{Duration, Instant};

use bollard::Docker;
use bollard::container::LogOutput;
use bollard::exec::{StartExecOptions, StartExecResults};
use bollard::models::{ContainerCreateBody, ExecConfig, VolumeCreateRequest};
use bollard::query_parameters::CreateImageOptionsBuilder;
use bollard::query_parameters::{
    AttachContainerOptionsBuilder, CreateContainerOptionsBuilder, KillContainerOptions,
    ListContainersOptionsBuilder, RemoveContainerOptionsBuilder, RemoveVolumeOptions,
};
use futures_util::{Stream, StreamExt as _};

use super::{LABEL, OWNER_LABEL};
use crate::docker::{Removal, failed_at, within};

/// How long the engine gets to end a container that was killed.
const KILL_GRACE: Duration = Duration::from_secs(5);

/// Why the engine could not do what was asked. The engine's own words are logged for the operator and never returned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineError {
    /// The engine did not answer, or answered with a failure.
    Unavailable,
}

impl std::fmt::Display for EngineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the container engine is unavailable")
    }
}

impl std::error::Error for EngineError {}

/// How a container ended.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Exit {
    /// The exit status, if the engine reported one.
    pub code: Option<i64>,
    /// Whether the engine says the memory limit killed it.
    pub oom_killed: bool,
    /// Whether it was still running at the deadline and was killed for it.
    pub timed_out: bool,
    /// Standard output, cut at the bound.
    pub stdout: Vec<u8>,
    /// Standard error, cut at the bound.
    pub stderr: Vec<u8>,
}

/// A container engine, reached the way the environment says (`DOCKER_HOST`, or the local socket).
#[derive(Clone)]
pub struct LakeEngine {
    docker: Docker,
}

impl LakeEngine {
    /// Connects to the engine the environment names.
    ///
    /// # Errors
    /// The engine's address cannot be used.
    pub fn connect() -> Result<Self, EngineError> {
        Docker::connect_with_local_defaults()
            .map(|docker| Self { docker })
            .map_err(|error| {
                failed_at("connect")(error);
                EngineError::Unavailable
            })
    }

    /// Creates the work volume: memory-backed and `bytes` large, so that the attempt directories together cannot fill
    /// more than that, owned by `uid` (the lake runner's) with mode 0755. A deployment creates its volume with the same
    /// options in its own manifest; this is for the development stack and for tests.
    ///
    /// # Errors
    /// The engine did not create it.
    pub async fn create_work_volume(
        &self,
        name: &str,
        bytes: u64,
        uid: u32,
    ) -> Result<(), EngineError> {
        let request = VolumeCreateRequest {
            name: Some(name.to_owned()),
            driver: Some("local".to_owned()),
            driver_opts: Some(HashMap::from([
                ("type".to_owned(), "tmpfs".to_owned()),
                ("device".to_owned(), "tmpfs".to_owned()),
                ("o".to_owned(), format!("size={bytes},uid={uid},mode=0755")),
            ])),
            ..VolumeCreateRequest::default()
        };
        match within(self.docker.create_volume(request)).await {
            Some(Ok(_)) => Ok(()),
            Some(Err(error)) => {
                failed_at("create volume")(error);
                Err(EngineError::Unavailable)
            }
            None => {
                failed_at("create volume")("the engine did not answer in time");
                Err(EngineError::Unavailable)
            }
        }
    }

    /// Removes a work volume. A volume still in use is not removed, and that is not an error.
    pub async fn remove_work_volume(&self, name: &str) {
        let _ = within(
            self.docker
                .remove_volume(name, Some(RemoveVolumeOptions { force: true })),
        )
        .await;
    }

    /// Makes `body`, runs it for at most `wait`, keeps at most `keep` bytes of each output stream, and removes it.
    ///
    /// # Errors
    /// The engine could not make, attach or start the container (for instance the image is not there).
    pub async fn run(
        &self,
        body: ContainerCreateBody,
        wait: Duration,
        keep: usize,
    ) -> Result<Exit, EngineError> {
        let name = format!("sagat-lake-{}", uuid::Uuid::now_v7());
        // If this future is dropped, the container is still removed, in the background.
        let guard = Removal::new(self.docker.clone(), name.clone());
        let create = CreateContainerOptionsBuilder::default().name(&name).build();
        match within(self.docker.create_container(Some(create), body)).await {
            Some(Ok(_)) => {}
            Some(Err(error)) => {
                failed_at("create")(error);
                return Err(EngineError::Unavailable);
            }
            None => {
                failed_at("create")("the engine did not answer in time");
                return Err(EngineError::Unavailable);
            }
        }
        let attach = AttachContainerOptionsBuilder::default()
            .stream(true)
            .stdout(true)
            .stderr(true)
            .build();
        let mut attached = match self.docker.attach_container(&name, Some(attach)).await {
            Ok(attached) => attached,
            Err(error) => {
                failed_at("attach")(error);
                self.remove(&name).await;
                guard.disarm();
                return Err(EngineError::Unavailable);
            }
        };
        if let Err(error) = self.docker.start_container(&name, None).await {
            failed_at("start")(error);
            self.remove(&name).await;
            guard.disarm();
            return Err(EngineError::Unavailable);
        }
        let mut exit = Exit::default();
        collect(&mut attached.output, wait, keep, &mut exit).await;
        drop(attached);
        if exit.timed_out {
            let _ = within(
                self.docker
                    .kill_container(&name, None::<KillContainerOptions>),
            )
            .await;
        }
        let end = Instant::now() + KILL_GRACE;
        loop {
            match within(self.docker.inspect_container(&name, None)).await {
                Some(Ok(info)) => {
                    let state = info.state.unwrap_or_default();
                    if state.running != Some(true) || Instant::now() >= end {
                        exit.code = state.exit_code;
                        exit.oom_killed = state.oom_killed == Some(true);
                        break;
                    }
                }
                _ => break,
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        self.remove(&name).await;
        guard.disarm();
        Ok(exit)
    }

    /// Makes and starts `body` and leaves it running.
    ///
    /// # Errors
    /// The engine could not make or start the container.
    pub async fn start(&self, body: ContainerCreateBody) -> Result<LakeContainer, EngineError> {
        let name = format!("sagat-lake-{}", uuid::Uuid::now_v7());
        let guard = Removal::new(self.docker.clone(), name.clone());
        let create = CreateContainerOptionsBuilder::default().name(&name).build();
        match within(self.docker.create_container(Some(create), body)).await {
            Some(Ok(_)) => {}
            Some(Err(error)) => {
                failed_at("create")(error);
                return Err(EngineError::Unavailable);
            }
            None => {
                failed_at("create")("the engine did not answer in time");
                return Err(EngineError::Unavailable);
            }
        }
        if let Err(error) = self.docker.start_container(&name, None).await {
            failed_at("start")(error);
            self.remove(&name).await;
            guard.disarm();
            return Err(EngineError::Unavailable);
        }
        Ok(LakeContainer {
            engine: self.clone(),
            name,
            guard,
        })
    }

    /// Makes sure `reference` is on the engine, pulling it if it is not. A deployment pulls its own images by digest;
    /// this is for the development stack and for tests.
    ///
    /// # Errors
    /// The image is not on the engine and could not be pulled.
    pub async fn ensure_image(&self, reference: &str) -> Result<(), EngineError> {
        if matches!(
            within(self.docker.inspect_image(reference)).await,
            Some(Ok(_))
        ) {
            return Ok(());
        }
        let options = if reference.contains('@') {
            CreateImageOptionsBuilder::default()
                .from_image(reference)
                .build()
        } else {
            let (name, tag) = reference
                .rsplit_once(':')
                .filter(|(_, tag)| !tag.contains('/'))
                .unwrap_or((reference, "latest"));
            CreateImageOptionsBuilder::default()
                .from_image(name)
                .tag(tag)
                .build()
        };
        let mut pull = self.docker.create_image(Some(options), None, None);
        while let Some(progress) = pull.next().await {
            if let Err(error) = progress {
                failed_at("pull")(error);
                return Err(EngineError::Unavailable);
            }
        }
        Ok(())
    }

    /// Removes the lake containers of `owner` that are older than `age` and that something left behind. Returns how
    /// many it removed.
    pub async fn sweep(&self, owner: &str, age: Duration) -> usize {
        let filters = HashMap::from([(
            "label".to_owned(),
            vec![format!("{LABEL}=1"), format!("{OWNER_LABEL}={owner}")],
        )]);
        let options = ListContainersOptionsBuilder::default()
            .all(true)
            .filters(&filters)
            .build();
        let Some(Ok(found)) = within(self.docker.list_containers(Some(options))).await else {
            return 0;
        };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let mut removed = 0;
        for container in found {
            let created = u64::try_from(container.created.unwrap_or(0)).unwrap_or(0);
            if now.saturating_sub(created) >= age.as_secs()
                && let Some(id) = container.id
                && self.remove(&id).await
            {
                removed += 1;
            }
        }
        removed
    }

    async fn remove(&self, name: &str) -> bool {
        let options = RemoveContainerOptionsBuilder::default()
            .force(true)
            .v(true)
            .build();
        within(self.docker.remove_container(name, Some(options)))
            .await
            .is_some_and(|removed| removed.is_ok())
    }
}

/// Reads `output` to its end or to the deadline, keeping at most `keep` bytes of each stream.
async fn collect(
    output: &mut Pin<Box<dyn Stream<Item = Result<LogOutput, bollard::errors::Error>> + Send>>,
    wait: Duration,
    keep: usize,
    exit: &mut Exit,
) {
    let deadline = tokio::time::Instant::now() + wait;
    loop {
        match tokio::time::timeout_at(deadline, output.next()).await {
            Err(_) => {
                exit.timed_out = true;
                break;
            }
            Ok(None | Some(Err(_))) => break,
            Ok(Some(Ok(LogOutput::StdOut { message } | LogOutput::Console { message }))) => {
                append(&mut exit.stdout, &message, keep);
            }
            Ok(Some(Ok(LogOutput::StdErr { message }))) => append(&mut exit.stderr, &message, keep),
            Ok(Some(Ok(LogOutput::StdIn { .. }))) => {}
        }
    }
}

/// A container that was started and is left running: the holder of a volume, or an attempt the caller will end itself.
/// It is removed when this value is dropped.
pub struct LakeContainer {
    engine: LakeEngine,
    name: String,
    guard: Removal,
}

impl LakeContainer {
    /// Runs `command` inside the container, as the container's user, for at most `wait`.
    ///
    /// # Errors
    /// The engine could not make or start the command.
    pub async fn exec(
        &self,
        command: Vec<String>,
        wait: Duration,
        keep: usize,
    ) -> Result<Exit, EngineError> {
        let docker = &self.engine.docker;
        let config = ExecConfig {
            cmd: Some(command),
            attach_stdout: Some(true),
            attach_stderr: Some(true),
            ..ExecConfig::default()
        };
        let created = match within(docker.create_exec(&self.name, config)).await {
            Some(Ok(created)) => created,
            Some(Err(error)) => {
                failed_at("create exec")(error);
                return Err(EngineError::Unavailable);
            }
            None => {
                failed_at("create exec")("the engine did not answer in time");
                return Err(EngineError::Unavailable);
            }
        };
        let options = StartExecOptions {
            detach: false,
            tty: false,
            output_capacity: None,
        };
        let StartExecResults::Attached { mut output, .. } = docker
            .start_exec(&created.id, Some(options))
            .await
            .map_err(|error| {
                failed_at("start exec")(error);
                EngineError::Unavailable
            })?
        else {
            return Err(EngineError::Unavailable);
        };
        let mut exit = Exit::default();
        collect(&mut output, wait, keep, &mut exit).await;
        if let Some(Ok(done)) = within(docker.inspect_exec(&created.id)).await {
            exit.code = done.exit_code;
        }
        Ok(exit)
    }

    /// Kills the container and removes it.
    pub async fn stop(self) {
        let _ = within(
            self.engine
                .docker
                .kill_container(&self.name, None::<KillContainerOptions>),
        )
        .await;
        self.engine.remove(&self.name).await;
        self.guard.disarm();
    }
}

/// Appends `chunk` to `into` without letting it grow past `keep`.
fn append(into: &mut Vec<u8>, chunk: &[u8], keep: usize) {
    let room = keep.saturating_sub(into.len());
    into.extend_from_slice(&chunk[..chunk.len().min(room)]);
}
