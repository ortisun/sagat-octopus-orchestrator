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
use bollard::models::{
    ContainerCreateBody, ContainerSummary, ContainerSummaryStateEnum, ExecConfig,
    VolumeCreateRequest,
};
use bollard::query_parameters::CreateImageOptionsBuilder;
use bollard::query_parameters::{
    AttachContainerOptionsBuilder, CreateContainerOptionsBuilder, KillContainerOptions,
    ListContainersOptionsBuilder, RemoveContainerOptionsBuilder, RemoveVolumeOptions,
};
use futures_util::{Stream, StreamExt as _};

use super::{
    ATTEMPT_LABEL, CLASS_LABEL, DEADLINE_LABEL, LABEL, MAX_SLOTS, MEMORY_LABEL, OWNER_LABEL,
    SLOT_LABEL, attempt_id_ok, class_ok, deadline_ok, owner_ok,
};
use crate::docker::{Removal, failed_at, within};

/// How long the engine gets to end a container that was killed.
const KILL_GRACE: Duration = Duration::from_secs(5);
/// How often a wait asks the engine whether a container is still there. Every poll is a scan of the engine's containers,
/// and a caller may have as many waiters as there are containers, so this is not made smaller than it needs to be.
const POLL: Duration = Duration::from_millis(500);
/// The longest a single wait holds, whatever it is asked for.
const MAX_HOLD: Duration = Duration::from_secs(60);
/// How many containers a sweep ends at once: one that the engine is slow to remove must not hold up the others past their
/// deadlines.
const SWEEP_PARALLEL: usize = 4;

/// Why the engine could not do what was asked. The engine's own words are logged for the operator and never returned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineError {
    /// The engine did not answer, or answered with a failure.
    Unavailable,
    /// The engine already has a container of this name: for [`LakeEngine::launch`], one of this attempt. It is not touched.
    Exists,
    /// The description is not a lake container: it lacks the labels of the profile, or they are not what the profile
    /// writes. The engine was not asked.
    Invalid,
}

impl std::fmt::Display for EngineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Unavailable => "the container engine is unavailable",
            Self::Exists => "a container of this attempt is already there",
            Self::Invalid => "the description is not a lake container",
        })
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

/// A lake container, as the labels on it say (ADR-0079, rule 7): the state of the sandbox runner is the engine's, so that a
/// restarted process finds what the last one left. A label that is missing or not a number is `None`: such a container was
/// not made by [`super::container_config`] and the caller decides what to do with it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LakeInfo {
    /// The engine's id of the container: the handle for the calls that follow.
    pub name: String,
    pub attempt_id: String,
    pub slot: Option<u32>,
    pub class: Option<String>,
    pub deadline_epoch_secs: Option<u64>,
    /// The memory limit the container was created with. What a restarted process adds up is this, and not what its own
    /// settings say now.
    pub memory_bytes: Option<u64>,
    /// Not ended: created, running, paused or restarting. A state the engine did not name counts as not ended, so that
    /// nothing is ever reported as over that was not seen to be.
    pub running: bool,
}

/// What a wait saw.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wait {
    /// There is a container of the attempt and it has not ended.
    Running,
    /// There is none, or every one has ended.
    Ended,
}

/// What ending an attempt's container did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EndedContainer {
    /// The engine showed a container of the attempt.
    pub existed: bool,
    /// The engine says the memory limit killed it: read **before** the container is removed, because removal loses it.
    pub memory_killed: bool,
    /// The container is gone, or the engine showed there was none. `false` when the engine could not be asked or did not
    /// confirm: a container that holds the credentials may still be up, and the report says so.
    pub confirmed: bool,
}

/// What a deadline sweep did. The counts are what an operator's gauge reads: "nothing was due" and "the engine is down" or
/// "the engine refused to remove a container that is past its deadline" are different things.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Sweep {
    /// The engine listed the owner's containers. `false`: nothing was looked at.
    pub listed: bool,
    /// Containers that were due and are confirmed gone.
    pub removed: usize,
    /// Containers that were due and that the engine did not confirm gone.
    pub failed: usize,
    /// Containers of the owner with no deadline label, or one that is not a deadline: not this sweep's to end, and
    /// [`LakeEngine::sweep`] takes them by age.
    pub unlabelled: usize,
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
                self.remove_disarming(&name, guard).await;
                return Err(EngineError::Unavailable);
            }
        };
        match within(self.docker.start_container(&name, None)).await {
            Some(Ok(())) => {}
            Some(Err(error)) => {
                failed_at("start")(error);
                self.remove_disarming(&name, guard).await;
                return Err(EngineError::Unavailable);
            }
            None => {
                failed_at("start")("the engine did not answer in time");
                self.remove_disarming(&name, guard).await;
                return Err(EngineError::Unavailable);
            }
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
        self.remove_disarming(&name, guard).await;
        Ok(exit)
    }

    /// Makes and starts `body` and leaves it running.
    ///
    /// # Errors
    /// The engine could not make or start the container.
    pub async fn start(&self, body: ContainerCreateBody) -> Result<LakeContainer, EngineError> {
        self.start_named(body, format!("sagat-lake-{}", uuid::Uuid::now_v7()), true)
            .await
    }

    /// Creates and starts `body` as `name`. With `guard_first` the removal of the container is armed before the create, so
    /// that a create that lands after the caller has gone is still removed; that is only right for a name nobody else can
    /// have (a fresh id). A name made of an attempt may be someone's, and a guard armed before the create would remove the
    /// first container of the attempt when a second is refused: there it is armed after.
    async fn start_named(
        &self,
        body: ContainerCreateBody,
        name: String,
        guard_first: bool,
    ) -> Result<LakeContainer, EngineError> {
        let early = guard_first.then(|| Removal::new(self.docker.clone(), name.clone()));
        self.create(&name, body).await?;
        let guard = early.unwrap_or_else(|| Removal::new(self.docker.clone(), name.clone()));
        match within(self.docker.start_container(&name, None)).await {
            Some(Ok(())) => {}
            Some(Err(error)) => {
                failed_at("start")(error);
                self.remove_disarming(&name, guard).await;
                return Err(EngineError::Unavailable);
            }
            None => {
                failed_at("start")("the engine did not answer in time");
                self.remove_disarming(&name, guard).await;
                return Err(EngineError::Unavailable);
            }
        }
        Ok(LakeContainer {
            engine: self.clone(),
            name,
            guard,
        })
    }

    /// Creates the container `name`. A name the engine already has is [`EngineError::Exists`]: that container is not this
    /// call's, and nothing here touches it.
    async fn create(&self, name: &str, body: ContainerCreateBody) -> Result<(), EngineError> {
        let options = CreateContainerOptionsBuilder::default().name(name).build();
        match within(self.docker.create_container(Some(options), body)).await {
            Some(Ok(_)) => Ok(()),
            Some(Err(error)) if conflict(&error) => Err(EngineError::Exists),
            Some(Err(error)) => {
                failed_at("create")(error);
                Err(EngineError::Unavailable)
            }
            None => {
                failed_at("create")("the engine did not answer in time");
                Err(EngineError::Unavailable)
            }
        }
    }

    /// Removes the container, and lets go of its guard only if the engine confirmed it: a removal the engine refused is
    /// tried again, in the background, when the guard drops.
    async fn remove_disarming(&self, name: &str, guard: Removal) {
        if self.remove(name).await {
            guard.disarm();
        }
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

    /// The API version the engine speaks (its highest). The sub-path mount of a lake container, which is the only thing that
    /// keeps a container to its own attempt directory, needs 1.45 or later; an engine older than that must not be used.
    ///
    /// # Errors
    /// The engine did not answer or did not say.
    pub async fn api_version(&self) -> Result<(u32, u32), EngineError> {
        match within(self.docker.version()).await {
            Some(Ok(version)) => version
                .api_version
                .as_deref()
                .and_then(parse_api_version)
                .ok_or(EngineError::Unavailable),
            Some(Err(error)) => {
                failed_at("version")(error);
                Err(EngineError::Unavailable)
            }
            None => {
                failed_at("version")("the engine did not answer in time");
                Err(EngineError::Unavailable)
            }
        }
    }

    /// A client of the engine at a Unix socket. The address is not tried here: every call to an engine that is not there is
    /// an [`EngineError::Unavailable`], which is how the cases that need an unreachable engine make one.
    ///
    /// # Errors
    /// The client could not be made.
    pub fn connect_unix(path: &str) -> Result<Self, EngineError> {
        Docker::connect_with_unix(path, 5, bollard::API_DEFAULT_VERSION)
            .map(|docker| Self { docker })
            .map_err(|error| {
                failed_at("connect")(error);
                EngineError::Unavailable
            })
    }

    /// Makes and starts `body` and holds nothing: the container is found again by its labels, and ended by
    /// [`Self::end`], the deadline sweep or [`Self::sweep`]. A start that fails removes what it made, and one the engine
    /// refuses to remove is removed again in the background.
    ///
    /// The container is named after its attempt, so that a second launch of the same attempt is refused by the engine
    /// itself (`Exists`), also when two arrive at once, and the first container is not touched. The body must carry the
    /// labels of the profile as [`super::container_config`] writes them, or nothing would find, count or sweep it.
    ///
    /// # Errors
    /// [`EngineError::Invalid`] for a body that is not a lake container, [`EngineError::Exists`] for an attempt that has
    /// one, [`EngineError::Unavailable`] when the engine could not make or start it.
    pub async fn launch(&self, body: ContainerCreateBody) -> Result<(), EngineError> {
        let name = profile_name(&body).ok_or(EngineError::Invalid)?;
        self.start_named(body, name, false).await?.detach();
        Ok(())
    }

    /// The lake containers of `owner`, ended or not. An owner the profile refuses has none, and the engine is not asked.
    ///
    /// # Errors
    /// The engine did not answer.
    pub async fn list(&self, owner: &str) -> Result<Vec<LakeInfo>, EngineError> {
        if !owner_ok(owner) {
            return Ok(Vec::new());
        }
        self.labelled(vec![format!("{LABEL}=1"), format!("{OWNER_LABEL}={owner}")])
            .await
    }

    /// The containers of one attempt of `owner`, ended or not. Another owner's are not shown: one process ends only what
    /// it made. An owner or an attempt id the profile refuses has none, and the engine is not asked: the two are put in
    /// a label filter, and what goes there is exactly what [`super::container_config`] can have written.
    ///
    /// # Errors
    /// The engine did not answer.
    pub async fn find(&self, owner: &str, attempt_id: &str) -> Result<Vec<LakeInfo>, EngineError> {
        if !owner_ok(owner) || !attempt_id_ok(attempt_id) {
            return Ok(Vec::new());
        }
        self.labelled(vec![
            format!("{LABEL}=1"),
            format!("{OWNER_LABEL}={owner}"),
            format!("{ATTEMPT_LABEL}={attempt_id}"),
        ])
        .await
    }

    /// Holds for at most `hold` (and never more than a minute) while the attempt's container runs. [`Wait::Ended`] when
    /// there is none or it has ended.
    ///
    /// # Errors
    /// The engine did not answer: that is not an ending, and the caller asks again.
    pub async fn wait(
        &self,
        owner: &str,
        attempt_id: &str,
        hold: Duration,
    ) -> Result<Wait, EngineError> {
        let end = Instant::now() + hold.min(MAX_HOLD);
        loop {
            let found = self.find(owner, attempt_id).await?;
            if found.iter().all(|container| !container.running) {
                return Ok(Wait::Ended);
            }
            let now = Instant::now();
            if now >= end {
                return Ok(Wait::Running);
            }
            tokio::time::sleep(POLL.min(end - now)).await;
        }
    }

    /// Ends the attempt's container: kills it, reads whether the memory limit did, and removes it. Nothing to end is
    /// `existed: false, confirmed: true` only when the engine answered; an engine that could not be asked is
    /// `confirmed: false`.
    ///
    /// The work is a task of its own: a caller that gives up (the lake runner bounds its kill by a time) does not stop
    /// the container being removed halfway, with the credentials still in it.
    pub async fn end(&self, owner: &str, attempt_id: &str) -> EndedContainer {
        // no container can carry an owner or an attempt the profile refuses: there is nothing to end
        if !owner_ok(owner) || !attempt_id_ok(attempt_id) {
            return EndedContainer {
                existed: false,
                memory_killed: false,
                confirmed: true,
            };
        }
        let (engine, owner, attempt_id) = (self.clone(), owner.to_owned(), attempt_id.to_owned());
        tokio::spawn(async move { engine.end_found(&owner, &attempt_id).await })
            .await
            .unwrap_or(EndedContainer {
                existed: false,
                memory_killed: false,
                confirmed: false,
            })
    }

    async fn end_found(&self, owner: &str, attempt_id: &str) -> EndedContainer {
        let Ok(found) = self.find(owner, attempt_id).await else {
            return EndedContainer {
                existed: false,
                memory_killed: false,
                confirmed: false,
            };
        };
        let mut ended = EndedContainer {
            existed: !found.is_empty(),
            memory_killed: false,
            confirmed: true,
        };
        for container in found {
            let (memory_killed, gone) = self.end_one(&container.name).await;
            ended.memory_killed |= memory_killed;
            ended.confirmed &= gone;
        }
        ended
    }

    /// Ends what is past its deadline label, whatever the process that set it did: a container still running at its
    /// deadline is killed, and one that has ended is kept for `retain` after its deadline (so that what the engine said
    /// about it can still be read) and then removed. `now_epoch_secs` is the caller's clock, never a request's. Up to four
    /// containers are ended at once, and like [`Self::end`] the work is a task of its own.
    ///
    /// A container with no deadline label that is a deadline is counted in [`Sweep::unlabelled`] and left to
    /// [`Self::sweep`], which goes by age: **a caller runs both**, the second with the longest lifetime a container may
    /// have, so that a label that names a deadline beyond it does not lengthen a life.
    pub async fn sweep_expired(&self, owner: &str, now_epoch_secs: u64, retain: Duration) -> Sweep {
        if !owner_ok(owner) {
            return Sweep {
                listed: true,
                ..Sweep::default()
            };
        }
        let (engine, owner) = (self.clone(), owner.to_owned());
        tokio::spawn(async move { engine.sweep_found(&owner, now_epoch_secs, retain).await })
            .await
            .unwrap_or_default()
    }

    async fn sweep_found(&self, owner: &str, now_epoch_secs: u64, retain: Duration) -> Sweep {
        let Ok(all) = self.list(owner).await else {
            return Sweep::default();
        };
        let mut sweep = Sweep {
            listed: true,
            ..Sweep::default()
        };
        let mut due = Vec::new();
        for container in all {
            let Some(deadline) = container.deadline_epoch_secs else {
                sweep.unlabelled += 1;
                continue;
            };
            let is_due = if container.running {
                now_epoch_secs >= deadline
            } else {
                now_epoch_secs >= deadline.saturating_add(retain.as_secs())
            };
            if is_due {
                due.push(container.name);
            }
        }
        let outcomes: Vec<bool> = futures_util::stream::iter(due)
            .map(|id| {
                let engine = self.clone();
                async move { engine.end_one(&id).await.1 }
            })
            .buffer_unordered(SWEEP_PARALLEL)
            .collect()
            .await;
        for gone in outcomes {
            if gone {
                sweep.removed += 1;
            } else {
                sweep.failed += 1;
            }
        }
        sweep
    }

    async fn labelled(&self, labels: Vec<String>) -> Result<Vec<LakeInfo>, EngineError> {
        let filters = HashMap::from([("label".to_owned(), labels)]);
        let options = ListContainersOptionsBuilder::default()
            .all(true)
            .filters(&filters)
            .build();
        match within(self.docker.list_containers(Some(options))).await {
            Some(Ok(found)) => Ok(found.into_iter().filter_map(info_of).collect()),
            Some(Err(error)) => {
                failed_at("list")(error);
                Err(EngineError::Unavailable)
            }
            None => {
                failed_at("list")("the engine did not answer in time");
                Err(EngineError::Unavailable)
            }
        }
    }

    /// Kills one container, reads the memory flag while it is still there, and removes it. Returns the flag and whether the
    /// container is gone (removed, or the engine said there is none).
    async fn end_one(&self, id: &str) -> (bool, bool) {
        // An error here is "not running" or "not there", and the forced removal below ends the first anyway.
        let _ = within(self.docker.kill_container(id, None::<KillContainerOptions>)).await;
        let end = Instant::now() + KILL_GRACE;
        let mut memory_killed = false;
        loop {
            match within(self.docker.inspect_container(id, None)).await {
                Some(Ok(info)) => {
                    let state = info.state.unwrap_or_default();
                    memory_killed |= state.oom_killed == Some(true);
                    if state.running != Some(true) || Instant::now() >= end {
                        break;
                    }
                }
                Some(Err(error)) if not_found(&error) => return (memory_killed, true),
                // the engine failed: try the removal, whose answer says whether the container is gone
                _ => break,
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let options = RemoveContainerOptionsBuilder::default()
            .force(true)
            .v(true)
            .build();
        match within(self.docker.remove_container(id, Some(options))).await {
            Some(Ok(())) => (memory_killed, true),
            Some(Err(error)) if not_found(&error) => (memory_killed, true),
            Some(Err(error)) => {
                failed_at("remove")(error);
                (memory_killed, false)
            }
            None => {
                failed_at("remove")("the engine did not answer in time");
                (memory_killed, false)
            }
        }
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

    /// Lets go of the container and leaves it running: nothing removes it when this value is dropped any more.
    pub fn detach(self) {
        self.guard.disarm();
    }

    /// Kills the container and removes it.
    pub async fn stop(self) {
        let _ = within(
            self.engine
                .docker
                .kill_container(&self.name, None::<KillContainerOptions>),
        )
        .await;
        if self.engine.remove(&self.name).await {
            self.guard.disarm();
        }
    }
}

/// Whether the engine said the container (or whatever the call named) is not there.
fn not_found(error: &bollard::errors::Error) -> bool {
    matches!(
        error,
        bollard::errors::Error::DockerResponseServerError {
            status_code: 404,
            ..
        }
    )
}

/// `1.45` as `(1, 45)`. Anything else (a patch number, text, nothing) is not a version this reads.
fn parse_api_version(text: &str) -> Option<(u32, u32)> {
    let (major, minor) = text.split_once('.')?;
    Some((major.parse().ok()?, minor.parse().ok()?))
}

/// Whether the engine refused the name because it has a container by it.
fn conflict(error: &bollard::errors::Error) -> bool {
    matches!(
        error,
        bollard::errors::Error::DockerResponseServerError {
            status_code: 409,
            ..
        }
    )
}

/// The name of the container of a lake attempt, or `None` for a body that does not carry the labels the profile writes
/// (every one of them, and each one valid): such a container would not be found, counted or swept.
fn profile_name(body: &ContainerCreateBody) -> Option<String> {
    let labels = body.labels.as_ref()?;
    let attempt = labels.get(ATTEMPT_LABEL)?;
    let number = |label: &str| labels.get(label)?.parse::<u64>().ok();
    (labels.get(LABEL).map(String::as_str) == Some("1")
        && owner_ok(labels.get(OWNER_LABEL)?)
        && attempt_id_ok(attempt)
        && number(SLOT_LABEL).is_some_and(|slot| slot < u64::from(MAX_SLOTS))
        && class_ok(labels.get(CLASS_LABEL)?)
        && number(DEADLINE_LABEL).is_some_and(deadline_ok)
        && number(MEMORY_LABEL).is_some_and(|memory| memory > 0))
    .then(|| format!("sagat-lake-attempt-{attempt}"))
}

/// The labels of one container of the list, read the way [`super::container_config`] wrote them.
fn info_of(container: ContainerSummary) -> Option<LakeInfo> {
    let name = container.id?;
    let labels = container.labels.unwrap_or_default();
    let ended = matches!(
        container.state,
        Some(
            ContainerSummaryStateEnum::EXITED
                | ContainerSummaryStateEnum::DEAD
                | ContainerSummaryStateEnum::REMOVING
        )
    );
    Some(LakeInfo {
        name,
        attempt_id: labels.get(ATTEMPT_LABEL).cloned().unwrap_or_default(),
        slot: labels.get(SLOT_LABEL).and_then(|slot| slot.parse().ok()),
        class: labels.get(CLASS_LABEL).cloned(),
        deadline_epoch_secs: labels
            .get(DEADLINE_LABEL)
            .and_then(|deadline| deadline.parse().ok())
            .filter(|deadline| deadline_ok(*deadline)),
        memory_bytes: labels
            .get(MEMORY_LABEL)
            .and_then(|memory| memory.parse().ok())
            .filter(|memory| *memory > 0),
        running: !ended,
    })
}

/// Appends `chunk` to `into` without letting it grow past `keep`.
fn append(into: &mut Vec<u8>, chunk: &[u8], keep: usize) {
    let room = keep.saturating_sub(into.len());
    into.extend_from_slice(&chunk[..chunk.len().min(room)]);
}

#[cfg(test)]
mod label_tests {
    use super::*;

    const VALID: [(&str, &str); 7] = [
        ("sagat.lake", "1"),
        ("sagat.lake.owner", "lake-runner-1"),
        ("sagat.lake.attempt", "0198f2c4-7b1e-7c3a-9d52-3f4a6b8c0d11"),
        ("sagat.lake.slot", "3"),
        ("sagat.lake.class", "small"),
        ("sagat.lake.deadline", "1800000000"),
        ("sagat.lake.memory", "2147483648"),
    ];

    fn labels(changes: &[(&str, Option<&str>)]) -> HashMap<String, String> {
        let mut map: HashMap<String, String> = VALID
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect();
        for (key, value) in changes {
            match value {
                Some(value) => map.insert((*key).to_owned(), (*value).to_owned()),
                None => map.remove(*key),
            };
        }
        map
    }

    fn summary(
        state: Option<ContainerSummaryStateEnum>,
        labels: HashMap<String, String>,
    ) -> ContainerSummary {
        ContainerSummary {
            id: Some("abc123".to_owned()),
            state,
            labels: Some(labels),
            ..ContainerSummary::default()
        }
    }

    fn body(labels: HashMap<String, String>) -> ContainerCreateBody {
        ContainerCreateBody {
            labels: Some(labels),
            ..ContainerCreateBody::default()
        }
    }

    /// Only a state the engine named as over counts as over: one it did not name, or a state that is not yet an ending, is a
    /// container that may still hold the credentials.
    #[test]
    fn only_exited_dead_and_removing_containers_are_ended() {
        use ContainerSummaryStateEnum::{
            CREATED, DEAD, EMPTY, EXITED, PAUSED, REMOVING, RESTARTING, RUNNING,
        };
        for (state, running) in [
            (Some(CREATED), true),
            (Some(RUNNING), true),
            (Some(PAUSED), true),
            (Some(RESTARTING), true),
            (Some(EMPTY), true),
            (None, true),
            (Some(EXITED), false),
            (Some(DEAD), false),
            (Some(REMOVING), false),
        ] {
            let info = info_of(summary(state, labels(&[]))).expect("a container");
            assert_eq!(info.running, running, "{state:?}");
        }
    }

    #[test]
    fn the_labels_the_profile_wrote_are_read_back() {
        let info = info_of(summary(
            Some(ContainerSummaryStateEnum::RUNNING),
            labels(&[]),
        ))
        .expect("a container");
        assert_eq!(info.name, "abc123");
        assert_eq!(info.attempt_id, "0198f2c4-7b1e-7c3a-9d52-3f4a6b8c0d11");
        assert_eq!(info.slot, Some(3));
        assert_eq!(info.class.as_deref(), Some("small"));
        assert_eq!(info.deadline_epoch_secs, Some(1_800_000_000));
        assert_eq!(info.memory_bytes, Some(2_147_483_648));
    }

    /// A label that is not what the profile writes is `None`, never a number made up: a deadline that is zero or that no
    /// clock reaches would otherwise end a container at once, or never.
    #[test]
    fn a_label_that_is_missing_or_not_what_the_profile_writes_is_none() {
        for bad in ["x", "-1", "", "4294967296", "3.5", " 3"] {
            let info = info_of(summary(None, labels(&[("sagat.lake.slot", Some(bad))])))
                .expect("a container");
            assert_eq!(info.slot, None, "slot {bad:?}");
        }
        for bad in [
            "0",
            "4102444801",
            "-5",
            "18446744073709551616",
            "soon",
            "",
            "18446744073709551615",
        ] {
            let info = info_of(summary(None, labels(&[("sagat.lake.deadline", Some(bad))])))
                .expect("a container");
            assert_eq!(info.deadline_epoch_secs, None, "deadline {bad:?}");
        }
        let info = info_of(summary(
            None,
            labels(&[
                ("sagat.lake.attempt", None),
                ("sagat.lake.class", None),
                ("sagat.lake.slot", None),
                ("sagat.lake.deadline", None),
            ]),
        ))
        .expect("a container");
        assert_eq!(
            (
                info.attempt_id.as_str(),
                info.class,
                info.slot,
                info.deadline_epoch_secs
            ),
            ("", None, None, None)
        );
        // the last second a deadline may name is read, the one after is not
        let latest = info_of(summary(
            None,
            labels(&[("sagat.lake.deadline", Some("4102444800"))]),
        ))
        .expect("a container");
        assert_eq!(latest.deadline_epoch_secs, Some(4_102_444_800));
    }

    #[test]
    fn a_memory_label_that_is_not_a_limit_is_none() {
        for bad in ["0", "-1", "", "lots", "1.5", "18446744073709551616"] {
            let info = info_of(summary(None, labels(&[("sagat.lake.memory", Some(bad))])))
                .expect("a container");
            assert_eq!(info.memory_bytes, None, "{bad:?}");
        }
        let info =
            info_of(summary(None, labels(&[("sagat.lake.memory", None)]))).expect("a container");
        assert_eq!(info.memory_bytes, None);
    }

    #[test]
    fn an_api_version_is_a_major_and_a_minor_and_nothing_else() {
        assert_eq!(parse_api_version("1.45"), Some((1, 45)));
        assert_eq!(parse_api_version("1.52"), Some((1, 52)));
        assert_eq!(parse_api_version("2.0"), Some((2, 0)));
        for bad in [
            "", "1", "1.", ".45", "1.45.1", "v1.45", "one.two", "1.-4", " 1.45",
        ] {
            assert_eq!(parse_api_version(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn a_summary_without_an_id_is_not_a_container_to_act_on() {
        let mut nameless = summary(None, labels(&[]));
        nameless.id = None;
        assert!(info_of(nameless).is_none());
    }

    #[test]
    fn a_launch_needs_every_label_of_the_profile_and_is_named_after_its_attempt() {
        assert_eq!(
            profile_name(&body(labels(&[]))).as_deref(),
            Some("sagat-lake-attempt-0198f2c4-7b1e-7c3a-9d52-3f4a6b8c0d11")
        );
        assert_eq!(profile_name(&ContainerCreateBody::default()), None);
        for key in [
            "sagat.lake",
            "sagat.lake.owner",
            "sagat.lake.attempt",
            "sagat.lake.slot",
            "sagat.lake.class",
            "sagat.lake.deadline",
            "sagat.lake.memory",
        ] {
            assert_eq!(
                profile_name(&body(labels(&[(key, None)]))),
                None,
                "without {key}"
            );
        }
    }

    #[test]
    fn a_launch_with_a_label_the_profile_would_refuse_is_refused() {
        let long_attempt = "a".repeat(61);
        let sixty = "a".repeat(60);
        for (key, bad) in [
            ("sagat.lake", "0"),
            ("sagat.lake.owner", ""),
            ("sagat.lake.owner", "has space"),
            ("sagat.lake.owner", "line\nbreak"),
            ("sagat.lake.attempt", ""),
            ("sagat.lake.attempt", "-abc"),
            ("sagat.lake.attempt", "ABC"),
            ("sagat.lake.attempt", "../x"),
            ("sagat.lake.attempt", long_attempt.as_str()),
            ("sagat.lake.slot", "4096"),
            ("sagat.lake.slot", "-1"),
            ("sagat.lake.class", "Small"),
            ("sagat.lake.class", ""),
            ("sagat.lake.deadline", "0"),
            ("sagat.lake.deadline", "4102444801"),
            ("sagat.lake.deadline", "later"),
            ("sagat.lake.memory", "0"),
            ("sagat.lake.memory", "-1"),
            ("sagat.lake.memory", "lots"),
        ] {
            assert_eq!(
                profile_name(&body(labels(&[(key, Some(bad))]))),
                None,
                "{key}={bad:?}"
            );
        }
        // the longest attempt id and the last slot are accepted
        assert!(
            profile_name(&body(labels(&[
                ("sagat.lake.attempt", Some(sixty.as_str())),
                ("sagat.lake.slot", Some("4095")),
            ])))
            .is_some()
        );
    }
}
