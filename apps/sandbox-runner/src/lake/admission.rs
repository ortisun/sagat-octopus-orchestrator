//! Admission: what a start, a wait and a kill do (feature 035, ADR-0079 rules 5 to 10 and 13).
//!
//! **The state is the engine's.** What is running, in which slot, in which class, with which memory limit and until when is
//! read from the labels on the containers each time it is needed, so that a restarted process admits as the last one would have.
//! The only state here is the reservations of starts that are inside the engine and not yet visible in its list, and the
//! per-attempt locks.
//!
//! A start is checked and reserved under one lock that is held across the engine's list and not across its launch, so that two
//! starts cannot both see room for one; it is refused, not clamped, when it asks for more than the node gives (a class the node
//! does not have, a deadline above its ceiling); and it is idempotent for an attempt that already has a container, so a retry
//! after a lost answer gets the same answer and no second container. A start and a kill of one attempt never run at once.
//!
//! **A start does not die with its caller.** It runs in a task of its own, so that a lake runner that gives up (its own
//! deadline, a cancel) and closes the connection does not leave the engine half-way through a container; and the reservation is
//! held by a guard, so that a start that is dropped or that panics gives its place back.

use std::collections::HashMap;
use std::os::unix::fs::MetadataExt as _;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use sagat_infra_lake::sandbox_api::{
    KillOutcome, Reason, StartAnswer, StartBody, WAIT_HOLD, WaitState,
};
use sagat_infra_sandbox::lake::engine::{EngineError, LakeInfo, Sweep, Wait};
use sagat_infra_sandbox::lake::{AttemptMount, LakeLimits, LakeSpec};
use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard, Semaphore};

use super::config::{Budget, LakeConfig};
use super::engine::Engine;

/// The clock, in seconds since the epoch. It is the node's own and never a request's.
pub type Clock = Arc<dyn Fn() -> u64 + Send + Sync>;

/// How long a start waits for its turn to be checked. The engine's list under the lock has its own limit; a start that has
/// waited this long for the lock behind a wedged engine is told so, and does not queue for ever.
const ADMIT_WAIT: Duration = if cfg!(test) {
    Duration::from_millis(300)
} else {
    Duration::from_secs(5)
};

/// A start that is inside the engine and not yet in its list.
struct Reservation {
    slot: u32,
    memory_bytes: u64,
}

type Reserved = Arc<Mutex<HashMap<String, Reservation>>>;

/// A place held for a start. It is given back when this is dropped: when the start ends, when it is dropped half-way, when it
/// panics.
struct Held {
    reserved: Reserved,
    id: String,
}

impl Drop for Held {
    fn drop(&mut self) {
        self.reserved
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&self.id);
    }
}

pub struct Admission<E> {
    engine: E,
    config: LakeConfig,
    clock: Clock,
    /// Held across a list of the engine and the check that follows it, never across the launch.
    admit: AsyncMutex<()>,
    reserved: Reserved,
    attempts: Mutex<HashMap<String, Arc<AsyncMutex<()>>>>,
    draining: AtomicBool,
    waiters: Semaphore,
}

/// Holds the lock of one attempt, and takes its entry out of the map when nobody else is waiting for it.
struct Serial<'a> {
    attempts: &'a Mutex<HashMap<String, Arc<AsyncMutex<()>>>>,
    id: String,
    lock: Arc<AsyncMutex<()>>,
    guard: Option<OwnedMutexGuard<()>>,
}

impl Drop for Serial<'_> {
    fn drop(&mut self) {
        let mut attempts = self.attempts.lock().unwrap_or_else(PoisonError::into_inner);
        self.guard.take();
        // the map's handle and ours: nobody has cloned it to wait for the lock
        if Arc::strong_count(&self.lock) == 2 {
            attempts.remove(&self.id);
        }
    }
}

impl<E: Engine> Admission<E> {
    pub fn new(engine: E, config: LakeConfig, clock: Clock) -> Self {
        let waiters = Semaphore::new(config.ceiling);
        Self {
            engine,
            config,
            clock,
            admit: AsyncMutex::new(()),
            reserved: Arc::new(Mutex::new(HashMap::new())),
            attempts: Mutex::new(HashMap::new()),
            draining: AtomicBool::new(false),
            waiters,
        }
    }

    /// From now on a start is `Unavailable`; a wait and a kill are answered.
    pub fn drain(&self) {
        self.draining.store(true, Ordering::SeqCst);
    }

    #[must_use]
    pub fn draining(&self) -> bool {
        self.draining.load(Ordering::SeqCst)
    }

    #[must_use]
    pub const fn config(&self) -> &LakeConfig {
        &self.config
    }

    /// The engine, for the cases that look at what was asked of it.
    #[cfg(test)]
    pub(crate) const fn engine(&self) -> &E {
        &self.engine
    }

    /// How many starts are holding a place right now.
    #[cfg(test)]
    pub(crate) fn places_held(&self) -> usize {
        self.reserved
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }

    async fn serial(&self, attempt_id: &str) -> Serial<'_> {
        let lock = {
            let mut attempts = self.attempts.lock().unwrap_or_else(PoisonError::into_inner);
            Arc::clone(attempts.entry(attempt_id.to_owned()).or_default())
        };
        let guard = Arc::clone(&lock).lock_owned().await;
        Serial {
            attempts: &self.attempts,
            id: attempt_id.to_owned(),
            lock,
            guard: Some(guard),
        }
    }

    /// The attempt directory is there, is a directory (not a link) and is the lake runner's. **Advisory**: a swap after this
    /// check is possible, and every directory in the volume is the lake runner's own; what confines the container is the
    /// engine's resolution of the sub-path inside the volume (ADR-0079, rule 10), which is why the engine's API is checked
    /// when the node starts.
    async fn directory_ok(&self, attempt_id: &str) -> bool {
        match tokio::fs::symlink_metadata(self.config.work_root.join(attempt_id)).await {
            Ok(metadata) => metadata.file_type().is_dir() && metadata.uid() == self.config.peer_uid,
            Err(_) => false,
        }
    }

    /// The memory of the largest class the node has: what a container is charged when nothing says what it was given.
    fn largest_memory(&self) -> u64 {
        self.config
            .large
            .map_or(self.config.small.memory_bytes, |large| {
                large.memory_bytes.max(self.config.small.memory_bytes)
            })
    }

    /// What a live container holds: **the limit it was created with**, read from its label, and not what this node's
    /// settings say now (they may have changed since). A container with no such label is charged as the largest class, so the
    /// count errs toward refusing.
    fn memory_of(&self, container: &LakeInfo) -> u64 {
        container
            .memory_bytes
            .unwrap_or_else(|| self.largest_memory())
    }

    /// How many ended containers are kept at most. They are kept for a time so that what the engine said about them can be
    /// read, and a peer chooses their deadline: without a cap, containers that run for no time at all would pile up in the
    /// engine for as long as that is.
    const fn kept_cap(&self) -> usize {
        self.config.ceiling.saturating_mul(2)
    }

    fn spec(&self, attempt_id: &str, body: &StartBody, budget: Budget) -> LakeSpec {
        // the label is the backstop: the attempt's own deadline rounded up, plus the time the lake runner has to end the
        // container first, so that the label never ends a host that is still within its attempt
        let seconds = body.deadline_ms.div_ceil(1000);
        let deadline = (self.clock)()
            .saturating_add(seconds)
            .saturating_add(self.config.deadline_grace.as_secs());
        LakeSpec {
            image: self.config.image.clone(),
            attempt_id: attempt_id.to_owned(),
            slot: body.slot,
            group: self.config.group,
            mount: AttemptMount {
                volume: self.config.work_volume.clone(),
            },
            limits: LakeLimits {
                memory_bytes: budget.memory_bytes,
                processes: budget.processes,
                cpu_millis: budget.cpu_millis,
                work_bytes: budget.work_bytes,
                tmp_bytes: budget.tmp_bytes,
            },
            owner: self.config.owner.clone(),
            class: body.class.as_str().to_owned(),
            deadline_epoch_secs: deadline,
        }
    }

    /// Starts the attempt's container, or says why not. `attempt_id` has been checked by the caller.
    ///
    /// The work is a task of its own: a caller that gives up and a connection that closes do not stop it half-way.
    pub async fn start(self: &Arc<Self>, attempt_id: &str, body: StartBody) -> StartAnswer {
        let admission = Arc::clone(self);
        let id = attempt_id.to_owned();
        let answer = match tokio::spawn(async move { admission.start_inner(&id, body).await }).await
        {
            Ok(answer) => answer,
            Err(error) => {
                tracing::error!(
                    panicked = error.is_panic(),
                    "a lake start ended without an answer"
                );
                StartAnswer::Unavailable
            }
        };
        let outcome = match answer {
            StartAnswer::Started => "started",
            StartAnswer::Refused(reason) => reason.as_str(),
            StartAnswer::Unavailable => "unavailable",
        };
        metrics::counter!("lake_sandbox_starts_total", "outcome" => outcome).increment(1);
        answer
    }

    async fn start_inner(&self, attempt_id: &str, body: StartBody) -> StartAnswer {
        if self.draining() {
            return StartAnswer::Unavailable;
        }
        if let Err(reason) = body.validate() {
            return StartAnswer::Refused(reason);
        }
        let Some(budget) = self.config.budget(body.class) else {
            return StartAnswer::Refused(Reason::Class);
        };
        if Duration::from_millis(body.deadline_ms) > self.config.max_attempt {
            return StartAnswer::Refused(Reason::Deadline);
        }
        if !self.directory_ok(attempt_id).await {
            return StartAnswer::Refused(Reason::Directory);
        }
        let _serial = self.serial(attempt_id).await;
        let held = {
            let Ok(_admitting) = tokio::time::timeout(ADMIT_WAIT, self.admit.lock()).await else {
                return StartAnswer::Unavailable;
            };
            let live = match self.engine.list(&self.config.owner).await {
                Ok(live) => live,
                Err(_) => return StartAnswer::Unavailable,
            };
            let alive_count = live.iter().filter(|c| c.running).count();
            metrics::gauge!("lake_sandbox_containers_running").set(alive_count as f64);
            // A container of the attempt is there already, running or ended and kept: a retry after a lost answer gets the
            // same answer. One made for another slot or class is not this start's.
            if let Some(existing) = live.iter().find(|c| c.attempt_id == attempt_id) {
                return if existing.slot == Some(body.slot)
                    && existing.class.as_deref() == Some(body.class.as_str())
                {
                    StartAnswer::Started
                } else {
                    StartAnswer::Refused(Reason::Slot)
                };
            }
            // ended containers are kept, but not without bound
            if live.len() - alive_count >= self.kept_cap() {
                return StartAnswer::Refused(Reason::Ceiling);
            }
            let mut reserved = self.reserved.lock().unwrap_or_else(PoisonError::into_inner);
            // a start whose container is already in the list is counted once, as the container
            let (mut pending, mut pending_memory, mut slot_taken) = (0_usize, 0_u64, false);
            for (id, reservation) in reserved.iter() {
                if live.iter().any(|c| c.attempt_id == *id) {
                    continue;
                }
                pending += 1;
                pending_memory = pending_memory.saturating_add(reservation.memory_bytes);
                slot_taken |= reservation.slot == body.slot;
            }
            if slot_taken || live.iter().any(|c| c.running && c.slot == Some(body.slot)) {
                return StartAnswer::Refused(Reason::Slot);
            }
            if alive_count + pending >= self.config.ceiling {
                return StartAnswer::Refused(Reason::Ceiling);
            }
            let held: u64 = live
                .iter()
                .filter(|c| c.running)
                .map(|c| self.memory_of(c))
                .fold(pending_memory, u64::saturating_add);
            if held.saturating_add(budget.memory_bytes) > self.config.node_memory_bytes {
                return StartAnswer::Refused(Reason::Ceiling);
            }
            reserved.insert(
                attempt_id.to_owned(),
                Reservation {
                    slot: body.slot,
                    memory_bytes: budget.memory_bytes,
                },
            );
            Held {
                reserved: Arc::clone(&self.reserved),
                id: attempt_id.to_owned(),
            }
        };
        let launched = self
            .engine
            .launch(self.spec(attempt_id, &body, budget))
            .await;
        drop(held);
        match launched {
            Ok(()) => StartAnswer::Started,
            // The engine has a container by the name of the attempt that the list did not show. It is this start's only if it
            // was made for this slot and class; a start that cannot tell is told to try again.
            Err(EngineError::Exists) => match self.engine.list(&self.config.owner).await {
                Ok(live)
                    if live.iter().any(|c| {
                        c.attempt_id == attempt_id
                            && c.slot == Some(body.slot)
                            && c.class.as_deref() == Some(body.class.as_str())
                    }) =>
                {
                    StartAnswer::Started
                }
                _ => StartAnswer::Unavailable,
            },
            Err(EngineError::Invalid) => {
                tracing::error!("the lake profile refused a container this node built");
                StartAnswer::Unavailable
            }
            Err(EngineError::Unavailable) => StartAnswer::Unavailable,
        }
    }

    /// Holds while the attempt's container runs. `None` is "ask again": the engine did not answer, or there are as many waits
    /// as the ceiling already.
    pub async fn wait(&self, attempt_id: &str) -> Option<WaitState> {
        let _permit = self.waiters.try_acquire().ok()?;
        match self
            .engine
            .wait(&self.config.owner, attempt_id, WAIT_HOLD)
            .await
        {
            Ok(Wait::Running) => Some(WaitState::Running),
            Ok(Wait::Ended) => Some(WaitState::Ended),
            Err(_) => None,
        }
    }

    /// Ends the attempt's container. `ended` is what the engine confirmed, nothing more.
    pub async fn kill(&self, attempt_id: &str) -> KillOutcome {
        let _serial = self.serial(attempt_id).await;
        let ended = self.engine.end(&self.config.owner, attempt_id).await;
        metrics::counter!(
            "lake_sandbox_kills_total",
            "confirmed" => if ended.confirmed { "yes" } else { "no" }
        )
        .increment(1);
        KillOutcome {
            ended: ended.confirmed,
            memory_killed: ended.memory_killed,
        }
    }

    /// One pass of the sweeps: what is past its deadline label, and, by age, whatever a wrong or missing label left.
    pub async fn sweep_once(&self) -> Sweep {
        let sweep = self
            .engine
            .sweep_expired(&self.config.owner, (self.clock)(), self.config.retain)
            .await;
        let longest = self
            .config
            .max_attempt
            .saturating_add(self.config.deadline_grace)
            .saturating_add(self.config.retain);
        let by_age = self.engine.sweep(&self.config.owner, longest).await;
        metrics::counter!("lake_sandbox_swept_total", "kind" => "deadline")
            .increment(sweep.removed as u64);
        metrics::counter!("lake_sandbox_swept_total", "kind" => "age").increment(by_age as u64);
        // the saturation signal is read here too, so that it falls when containers end and nobody starts another
        if let Ok(live) = self.engine.list(&self.config.owner).await {
            metrics::gauge!("lake_sandbox_containers_running")
                .set(live.iter().filter(|c| c.running).count() as f64);
        }
        if !sweep.listed {
            tracing::warn!(
                "the lake sweep could not list the containers: the engine did not answer"
            );
        } else if sweep.failed > 0 || sweep.unlabelled > 0 {
            tracing::warn!(
                failed = sweep.failed,
                unlabelled = sweep.unlabelled,
                "the sweep left lake containers it could not end or could not read a deadline of"
            );
        }
        sweep
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::atomic::AtomicU64;

    use sagat_infra_lake::sandbox_api::Class;

    use super::super::engine::fake::Fake;
    use super::*;

    const NOW: u64 = 1_800_000_000;
    const MIB: u64 = 1024 * 1024;

    struct Rig {
        admission: Arc<Admission<Fake>>,
        clock: Arc<AtomicU64>,
        directory: tempfile::TempDir,
    }

    fn env(
        directory: &tempfile::TempDir,
        extra: &[(&'static str, &'static str)],
    ) -> HashMap<String, String> {
        let uid = std::fs::metadata(directory.path()).expect("metadata").uid();
        let mut env: HashMap<String, String> = [
            ("SAGAT_LAKE_SOCKET_DIR", "/run/lake".to_owned()),
            ("SAGAT_LAKE_RUNNER_UID", uid.to_string()),
            ("SAGAT_LAKE_RUNNER_GID", "10001".to_owned()),
            ("SAGAT_LAKE_WORK_VOLUME", "sagat-lake-work".to_owned()),
            (
                "SAGAT_LAKE_WORK_ROOT",
                directory.path().to_string_lossy().into_owned(),
            ),
            ("SAGAT_LAKE_IMAGE", "sagat/lake@sha256:abc".to_owned()),
            ("SAGAT_LAKE_OWNER", "lake-runner".to_owned()),
            ("SAGAT_LAKE_NODE_MEMORY_MIB", "8192".to_owned()),
            ("SAGAT_LAKE_SMALL_MEMORY_MIB", "2048".to_owned()),
            ("SAGAT_LAKE_CEILING", "4".to_owned()),
            ("SAGAT_LAKE_DEADLINE_GRACE_SECS", "30".to_owned()),
            ("SAGAT_LAKE_MAX_ATTEMPT_SECS", "3600".to_owned()),
        ]
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value))
        .collect();
        for (key, value) in extra {
            env.insert((*key).to_owned(), (*value).to_owned());
        }
        env
    }

    fn rig(extra: &[(&'static str, &'static str)]) -> Rig {
        let directory = tempfile::tempdir().expect("a directory");
        let env = env(&directory, extra);
        let lookup = |name: &str| env.get(name).cloned();
        let config = LakeConfig::from_lookup(&lookup).expect("a valid configuration");
        let clock = Arc::new(AtomicU64::new(NOW));
        let reader = Arc::clone(&clock);
        let admission = Arc::new(Admission::new(
            Fake::new(),
            config,
            Arc::new(move || reader.load(Ordering::SeqCst)),
        ));
        Rig {
            admission,
            clock,
            directory,
        }
    }

    fn body(slot: u32, class: Class, deadline_ms: u64) -> StartBody {
        StartBody {
            slot,
            class,
            deadline_ms,
        }
    }

    /// A container as a previous process left it.
    fn left_behind(attempt: &str, slot: u32, class: &str, memory_mib: u64) -> LakeSpec {
        LakeSpec {
            image: "x".into(),
            attempt_id: attempt.into(),
            slot,
            group: 1,
            mount: AttemptMount { volume: "v".into() },
            limits: LakeLimits {
                memory_bytes: memory_mib * MIB,
                processes: 1,
                cpu_millis: 1,
                work_bytes: 1,
                tmp_bytes: 1,
            },
            owner: "lake-runner".into(),
            class: class.into(),
            deadline_epoch_secs: NOW + 1000,
        }
    }

    impl Rig {
        /// The attempt directory the lake runner leaves in the work volume.
        fn directory_of(&self, attempt: &str) {
            let path = self.directory.path().join(attempt);
            if !path.exists() {
                std::fs::create_dir(path).expect("an attempt directory");
            }
        }

        async fn start(
            &self,
            attempt: &str,
            slot: u32,
            class: Class,
            deadline_ms: u64,
        ) -> StartAnswer {
            self.directory_of(attempt);
            self.admission
                .start(attempt, body(slot, class, deadline_ms))
                .await
        }

        fn reservations(&self) -> usize {
            self.admission
                .reserved
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .len()
        }
    }

    #[tokio::test]
    async fn a_start_makes_the_container_the_node_describes_and_nothing_the_request_chose() {
        let rig = rig(&[]);
        assert_eq!(
            rig.start("a1", 7, Class::Small, 60_000).await,
            StartAnswer::Started
        );
        let specs = rig.admission.engine.specs();
        assert_eq!(specs.len(), 1);
        let spec = &specs[0];
        assert_eq!(spec.image, "sagat/lake@sha256:abc");
        assert_eq!(spec.attempt_id, "a1");
        assert_eq!(spec.slot, 7);
        assert_eq!(spec.group, 10_001);
        assert_eq!(spec.mount.volume, "sagat-lake-work");
        assert_eq!(spec.owner, "lake-runner");
        assert_eq!(spec.class, "small");
        assert_eq!(spec.limits.memory_bytes, 2048 * MIB);
        // the attempt's 60 s rounded up, plus the 30 s the lake runner has to end it first
        assert_eq!(spec.deadline_epoch_secs, NOW + 60 + 30);
    }

    /// A deadline is rounded up to a whole second: a label that ended a container a second before its attempt would report a
    /// host that was still writing as one that went silent.
    #[tokio::test]
    async fn the_deadline_label_is_rounded_up_and_never_before_the_attempts_own() {
        for (ms, seconds) in [
            (1, 1),
            (999, 1),
            (1000, 1),
            (1001, 2),
            (60_000, 60),
            (60_001, 61),
        ] {
            let rig = rig(&[]);
            assert_eq!(
                rig.start("a1", 0, Class::Small, ms).await,
                StartAnswer::Started
            );
            assert_eq!(
                rig.admission.engine.specs()[0].deadline_epoch_secs,
                NOW + seconds + 30,
                "{ms} ms"
            );
        }
    }

    #[tokio::test]
    async fn a_retry_of_a_start_gets_the_same_answer_and_no_second_container() {
        let rig = rig(&[]);
        assert_eq!(
            rig.start("a1", 7, Class::Small, 60_000).await,
            StartAnswer::Started
        );
        assert_eq!(
            rig.start("a1", 7, Class::Small, 60_000).await,
            StartAnswer::Started
        );
        assert_eq!(rig.admission.engine.launches.load(Ordering::SeqCst), 1);
        assert_eq!(rig.admission.engine.specs().len(), 1);
        // the container ended by itself and is kept: the retry still gets the answer it would have got
        rig.admission.engine.finish("a1");
        assert_eq!(
            rig.start("a1", 7, Class::Small, 60_000).await,
            StartAnswer::Started
        );
        assert_eq!(rig.admission.engine.launches.load(Ordering::SeqCst), 1);
    }

    /// The attempt has a container made for another slot or class: that is not this start's, and the relay of this start
    /// would be waiting for another user.
    #[tokio::test]
    async fn an_attempt_that_already_has_a_container_for_another_slot_or_class_is_refused() {
        let rig = rig(&[("SAGAT_LAKE_LARGE_MEMORY_MIB", "4096")]);
        assert_eq!(
            rig.start("a1", 7, Class::Small, 60_000).await,
            StartAnswer::Started
        );
        assert_eq!(
            rig.start("a1", 8, Class::Small, 60_000).await,
            StartAnswer::Refused(Reason::Slot)
        );
        assert_eq!(
            rig.start("a1", 7, Class::Large, 60_000).await,
            StartAnswer::Refused(Reason::Slot)
        );
        assert_eq!(rig.admission.engine.specs().len(), 1);
    }

    #[tokio::test]
    async fn a_slot_in_use_by_another_attempt_is_refused_until_that_container_has_ended() {
        let rig = rig(&[]);
        assert_eq!(
            rig.start("a1", 7, Class::Small, 60_000).await,
            StartAnswer::Started
        );
        assert_eq!(
            rig.start("a2", 7, Class::Small, 60_000).await,
            StartAnswer::Refused(Reason::Slot)
        );
        rig.admission.engine.finish("a1");
        assert_eq!(
            rig.start("a2", 7, Class::Small, 60_000).await,
            StartAnswer::Started
        );
    }

    #[tokio::test]
    async fn the_ceiling_counts_the_containers_that_run_and_not_the_ones_that_ended() {
        let rig = rig(&[("SAGAT_LAKE_CEILING", "2")]);
        assert_eq!(
            rig.start("a1", 1, Class::Small, 60_000).await,
            StartAnswer::Started
        );
        assert_eq!(
            rig.start("a2", 2, Class::Small, 60_000).await,
            StartAnswer::Started
        );
        assert_eq!(
            rig.start("a3", 3, Class::Small, 60_000).await,
            StartAnswer::Refused(Reason::Ceiling)
        );
        rig.admission.engine.finish("a1");
        assert_eq!(
            rig.start("a3", 3, Class::Small, 60_000).await,
            StartAnswer::Started
        );
    }

    /// Ended containers are kept for the facts they hold, and a peer chooses how long; they are not kept without bound, or
    /// containers that run for no time would fill the engine.
    #[tokio::test]
    async fn the_ended_containers_that_are_kept_are_capped_and_a_kill_makes_room() {
        let rig = rig(&[("SAGAT_LAKE_CEILING", "2")]);
        for index in 0..4_u32 {
            rig.admission.engine.seed(
                left_behind(&format!("k{index}"), 100 + index, "small", 2048),
                false,
            );
        }
        assert_eq!(rig.admission.engine.alive(), 0);
        // four are kept, which is twice the ceiling: no fifth
        assert_eq!(
            rig.start("a1", 1, Class::Small, 60_000).await,
            StartAnswer::Refused(Reason::Ceiling)
        );
        // a retry for one of them is still answered: it is not a new container
        assert_eq!(
            rig.start("k0", 100, Class::Small, 60_000).await,
            StartAnswer::Started
        );
        assert_eq!(rig.admission.engine.launches.load(Ordering::SeqCst), 0);
        // ending one makes room
        let _ = rig.admission.kill("k1").await;
        assert_eq!(
            rig.start("a1", 1, Class::Small, 60_000).await,
            StartAnswer::Started
        );
    }

    #[tokio::test]
    async fn the_memory_of_the_node_is_a_ceiling_of_its_own() {
        // room for three small containers by count, and for one by memory
        let rig = rig(&[
            ("SAGAT_LAKE_CEILING", "3"),
            ("SAGAT_LAKE_NODE_MEMORY_MIB", "3072"),
        ]);
        assert_eq!(
            rig.start("a1", 1, Class::Small, 60_000).await,
            StartAnswer::Started
        );
        assert_eq!(
            rig.start("a2", 2, Class::Small, 60_000).await,
            StartAnswer::Refused(Reason::Ceiling)
        );
    }

    /// A container is charged for the memory it was created with, which its label says, and not for what this node's settings
    /// say now: the settings of a node change, and a container made before is still holding what it was given.
    #[tokio::test]
    async fn a_container_is_charged_the_memory_it_was_made_with_and_not_the_settings_of_today() {
        let rig = rig(&[("SAGAT_LAKE_NODE_MEMORY_MIB", "8192")]);
        // made when the node had a large class of 6 GiB; the node has none now
        rig.admission
            .engine
            .seed(left_behind("old", 9, "large", 6144), true);
        // 6144 held: a small (2048) still fits, a second one does not
        assert_eq!(
            rig.start("a1", 1, Class::Small, 60_000).await,
            StartAnswer::Started
        );
        assert_eq!(
            rig.start("a2", 2, Class::Small, 60_000).await,
            StartAnswer::Refused(Reason::Ceiling)
        );
    }

    /// A container whose label does not say charges as the largest class: the count errs toward refusing.
    #[tokio::test]
    async fn a_container_with_no_memory_label_counts_as_the_largest_class() {
        let rig = rig(&[
            ("SAGAT_LAKE_LARGE_MEMORY_MIB", "6144"),
            ("SAGAT_LAKE_NODE_MEMORY_MIB", "8192"),
        ]);
        rig.admission
            .engine
            .seed_without_memory_label(left_behind("old", 9, "small", 1), true);
        // charged 6144 (the largest class), not the 1 MiB it was made with
        assert_eq!(
            rig.start("a1", 1, Class::Small, 60_000).await,
            StartAnswer::Started
        );
        assert_eq!(
            rig.start("a2", 2, Class::Small, 60_000).await,
            StartAnswer::Refused(Reason::Ceiling)
        );
    }

    #[tokio::test]
    async fn what_the_node_does_not_have_or_does_not_allow_is_refused_and_not_cut() {
        let rig = rig(&[]);
        // no large class on this node
        assert_eq!(
            rig.start("a1", 1, Class::Large, 60_000).await,
            StartAnswer::Refused(Reason::Class)
        );
        // above the ceiling of an attempt (3600 s)
        assert_eq!(
            rig.start("a2", 2, Class::Small, 3_600_001).await,
            StartAnswer::Refused(Reason::Deadline)
        );
        assert_eq!(
            rig.start("a3", 3, Class::Small, 3_600_000).await,
            StartAnswer::Started
        );
        // what no node accepts
        assert_eq!(
            rig.start("a4", 4, Class::Small, 0).await,
            StartAnswer::Refused(Reason::Deadline)
        );
        assert_eq!(
            rig.start("a5", 4096, Class::Small, 60_000).await,
            StartAnswer::Refused(Reason::Slot)
        );
        assert_eq!(rig.admission.engine.specs().len(), 1);
    }

    #[tokio::test]
    async fn the_attempt_directory_must_be_there_be_a_directory_and_be_the_lake_runners() {
        let rig = rig(&[]);
        let request = body(1, Class::Small, 60_000);
        // missing
        assert_eq!(
            rig.admission.start("missing", request).await,
            StartAnswer::Refused(Reason::Directory)
        );
        // a file
        std::fs::write(rig.directory.path().join("afile"), b"x").expect("a file");
        assert_eq!(
            rig.admission.start("afile", request).await,
            StartAnswer::Refused(Reason::Directory)
        );
        // a link, also to a directory that is there
        std::os::unix::fs::symlink(rig.directory.path(), rig.directory.path().join("alink"))
            .expect("a link");
        assert_eq!(
            rig.admission.start("alink", request).await,
            StartAnswer::Refused(Reason::Directory)
        );
        assert_eq!(rig.admission.engine.launches.load(Ordering::SeqCst), 0);
        rig.directory_of("fine");
        assert_eq!(
            rig.admission.start("fine", request).await,
            StartAnswer::Started
        );
    }

    #[tokio::test]
    async fn a_directory_that_belongs_to_another_user_is_refused() {
        let directory = tempfile::tempdir().expect("a directory");
        let uid = std::fs::metadata(directory.path()).expect("metadata").uid();
        let mut env = env(&directory, &[]);
        // the lake runner is somebody else
        env.insert("SAGAT_LAKE_RUNNER_UID".to_owned(), (uid + 1).to_string());
        let lookup = |name: &str| env.get(name).cloned();
        let config = LakeConfig::from_lookup(&lookup).expect("valid");
        let admission = Arc::new(Admission::new(Fake::new(), config, Arc::new(|| NOW)));
        std::fs::create_dir(directory.path().join("a1")).expect("a directory");
        assert_eq!(
            admission.start("a1", body(1, Class::Small, 60_000)).await,
            StartAnswer::Refused(Reason::Directory)
        );
    }

    /// Two starts cannot both see room for one: the second finds the first's reservation while the first is still inside the
    /// engine, before its container is in the list.
    #[tokio::test]
    async fn two_starts_at_once_cannot_both_take_the_last_place() {
        let rig = rig(&[("SAGAT_LAKE_CEILING", "1")]);
        rig.admission.engine.gate_launches();
        rig.directory_of("a1");
        rig.directory_of("a2");
        let first = {
            let admission = Arc::clone(&rig.admission);
            tokio::spawn(async move { admission.start("a1", body(1, Class::Small, 60_000)).await })
        };
        rig.admission.engine.wait_entered().await; // the first is inside the engine, with its reservation
        let second = rig
            .admission
            .start("a2", body(2, Class::Small, 60_000))
            .await;
        assert_eq!(second, StartAnswer::Refused(Reason::Ceiling));
        rig.admission.engine.ungate_launches();
        assert_eq!(first.await.expect("the first"), StartAnswer::Started);
        assert_eq!(rig.admission.engine.launches.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn two_starts_at_once_cannot_both_take_the_same_slot() {
        let rig = rig(&[]);
        rig.admission.engine.gate_launches();
        rig.directory_of("a1");
        rig.directory_of("a2");
        let first = {
            let admission = Arc::clone(&rig.admission);
            tokio::spawn(async move { admission.start("a1", body(5, Class::Small, 60_000)).await })
        };
        rig.admission.engine.wait_entered().await;
        let second = rig
            .admission
            .start("a2", body(5, Class::Small, 60_000))
            .await;
        assert_eq!(second, StartAnswer::Refused(Reason::Slot));
        rig.admission.engine.ungate_launches();
        assert_eq!(first.await.expect("the first"), StartAnswer::Started);
    }

    /// A reservation and the container it made are one place, not two, in the moment between the container being in the list
    /// and the reservation being given back.
    #[tokio::test]
    async fn a_reservation_and_its_own_container_are_counted_once() {
        let rig = rig(&[("SAGAT_LAKE_CEILING", "2")]);
        rig.admission
            .engine
            .seed(left_behind("a1", 1, "small", 2048), true);
        rig.admission
            .reserved
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(
                "a1".to_owned(),
                Reservation {
                    slot: 1,
                    memory_bytes: 2048 * MIB,
                },
            );
        // one place is taken, as the container; counted twice it would be two of two
        assert_eq!(
            rig.start("a2", 2, Class::Small, 60_000).await,
            StartAnswer::Started
        );
    }

    /// A start that fails in the engine gives its place back.
    #[tokio::test]
    async fn a_start_the_engine_could_not_make_does_not_keep_its_place() {
        let rig = rig(&[("SAGAT_LAKE_CEILING", "1")]);
        rig.admission
            .engine
            .set_launch_fails(Some(EngineError::Unavailable));
        assert_eq!(
            rig.start("a1", 1, Class::Small, 60_000).await,
            StartAnswer::Unavailable
        );
        assert_eq!(rig.reservations(), 0);
        rig.admission.engine.set_launch_fails(None);
        assert_eq!(
            rig.start("a2", 2, Class::Small, 60_000).await,
            StartAnswer::Started
        );
    }

    /// A start that is dropped while it is inside the engine gives its place back: the reservation is a guard and not a line
    /// after the launch that a cancelled future never reaches.
    #[tokio::test]
    async fn a_start_that_is_dropped_inside_the_engine_gives_its_place_back() {
        let rig = rig(&[("SAGAT_LAKE_CEILING", "1")]);
        rig.admission.engine.gate_launches();
        rig.directory_of("a1");
        let mut start = Box::pin(
            rig.admission
                .start_inner("a1", body(1, Class::Small, 60_000)),
        );
        tokio::select! {
            _ = &mut start => panic!("the start finished while the launch was held"),
            () = rig.admission.engine.wait_entered() => {}
        }
        assert_eq!(
            rig.reservations(),
            1,
            "the place was not held while the launch ran"
        );
        drop(start);
        assert_eq!(rig.reservations(), 0, "a dropped start kept its place");
        rig.admission.engine.ungate_launches();
        assert_eq!(
            rig.start("a2", 2, Class::Small, 60_000).await,
            StartAnswer::Started
        );
    }

    /// A start does not die with its caller: the lake runner that gives up and closes the connection does not leave the engine
    /// half-way through a container, and a kill that follows finds what it made.
    #[tokio::test]
    async fn a_start_runs_to_its_end_when_the_caller_goes_away() {
        let rig = rig(&[]);
        rig.admission.engine.gate_launches();
        rig.directory_of("a1");
        let mut caller = Box::pin(rig.admission.start("a1", body(1, Class::Small, 60_000)));
        tokio::select! {
            _ = &mut caller => panic!("the start finished while the launch was held"),
            () = rig.admission.engine.wait_entered() => {}
        }
        drop(caller); // the connection closed
        rig.admission.engine.ungate_launches();
        // the container is made all the same, and a kill ends it
        let killed = rig.admission.kill("a1").await;
        assert_eq!(rig.admission.engine.launches_made(), 1);
        assert!(killed.ended);
        assert!(rig.admission.engine.ended().contains(&"a1".to_owned()));
        assert_eq!(rig.admission.engine.alive(), 0);
    }

    /// A launch that panics gives its place back too, and is an answer to try again.
    #[tokio::test]
    async fn a_launch_that_panics_gives_its_place_back() {
        let rig = rig(&[("SAGAT_LAKE_CEILING", "1")]);
        rig.admission.engine.set_launch_panics(true);
        assert_eq!(
            rig.start("a1", 1, Class::Small, 60_000).await,
            StartAnswer::Unavailable
        );
        assert_eq!(rig.reservations(), 0);
        rig.admission.engine.set_launch_panics(false);
        assert_eq!(
            rig.start("a2", 2, Class::Small, 60_000).await,
            StartAnswer::Started
        );
    }

    /// A start that waits for its turn behind a wedged engine is told so after a while, and does not queue for ever.
    #[tokio::test]
    async fn a_start_does_not_wait_for_ever_for_its_turn_to_be_checked() {
        let rig = rig(&[]);
        rig.directory_of("a1");
        let _held = rig.admission.admit.lock().await;
        // bounded here too: a start that waited for ever would otherwise hang the suite instead of failing it
        let answer = tokio::time::timeout(
            Duration::from_secs(5),
            rig.admission.start("a1", body(1, Class::Small, 60_000)),
        )
        .await
        .expect("the start gave up waiting for its turn");
        assert_eq!(answer, StartAnswer::Unavailable);
        assert_eq!(rig.reservations(), 0);
    }

    /// When the engine says a container is there that the list did not show, it is this start's only if it is for this slot and
    /// class; a start that cannot tell is told to try again, never `started`.
    #[tokio::test]
    async fn a_container_the_engine_has_that_the_list_did_not_show_is_not_taken_for_this_start() {
        let rig = rig(&[]);
        rig.admission
            .engine
            .set_launch_fails(Some(EngineError::Exists));
        assert_eq!(
            rig.start("a1", 1, Class::Small, 60_000).await,
            StartAnswer::Unavailable
        );
        assert_eq!(rig.reservations(), 0);
    }

    #[tokio::test]
    async fn an_engine_that_cannot_list_is_unavailable_and_nothing_is_started() {
        let rig = rig(&[]);
        rig.admission.engine.set_list_fails(true);
        assert_eq!(
            rig.start("a1", 1, Class::Small, 60_000).await,
            StartAnswer::Unavailable
        );
        assert_eq!(rig.admission.engine.launches.load(Ordering::SeqCst), 0);
    }

    /// The state is the engine's: a process that starts after another died admits as it would have, because it reads the same
    /// labels; and it can wait for and end what the last one made.
    #[tokio::test]
    async fn a_restarted_process_admits_from_what_the_engine_has_and_ends_what_the_last_one_made() {
        let rig = rig(&[("SAGAT_LAKE_CEILING", "1")]);
        rig.admission
            .engine
            .seed(left_behind("left-behind", 3, "small", 2048), true);
        assert_eq!(
            rig.start("a1", 1, Class::Small, 60_000).await,
            StartAnswer::Refused(Reason::Ceiling)
        );
        assert_eq!(
            rig.start("a2", 3, Class::Small, 60_000).await,
            StartAnswer::Refused(Reason::Slot)
        );
        assert_eq!(
            rig.admission.wait("left-behind").await,
            Some(WaitState::Running)
        );
        rig.admission.engine.mark_memory_killed("left-behind");
        assert_eq!(
            rig.admission.kill("left-behind").await,
            KillOutcome {
                ended: true,
                memory_killed: true
            }
        );
        assert_eq!(
            rig.start("a1", 1, Class::Small, 60_000).await,
            StartAnswer::Started
        );
    }

    #[tokio::test]
    async fn a_node_that_is_draining_starts_nothing_and_still_answers_waits_and_kills() {
        let rig = rig(&[]);
        assert_eq!(
            rig.start("a1", 1, Class::Small, 60_000).await,
            StartAnswer::Started
        );
        rig.admission.drain();
        assert!(rig.admission.draining());
        assert_eq!(
            rig.start("a2", 2, Class::Small, 60_000).await,
            StartAnswer::Unavailable
        );
        assert_eq!(rig.admission.wait("a1").await, Some(WaitState::Running));
        assert_eq!(
            rig.admission.kill("a1").await,
            KillOutcome {
                ended: true,
                memory_killed: false
            }
        );
    }

    #[tokio::test]
    async fn a_wait_says_what_the_container_does_and_asks_again_when_the_engine_does_not_answer() {
        let rig = rig(&[]);
        assert_eq!(
            rig.start("a1", 1, Class::Small, 60_000).await,
            StartAnswer::Started
        );
        assert_eq!(rig.admission.wait("a1").await, Some(WaitState::Running));
        rig.admission.engine.finish("a1");
        assert_eq!(rig.admission.wait("a1").await, Some(WaitState::Ended));
        // there is none: it ended
        assert_eq!(rig.admission.wait("never").await, Some(WaitState::Ended));
        rig.admission.engine.set_list_fails(true);
        assert_eq!(rig.admission.wait("a1").await, None);
    }

    /// Waits are as many as the ceiling and no more: a peer cannot hold a task and a descriptor per request for ever.
    #[tokio::test]
    async fn waits_are_as_many_as_the_ceiling_and_the_next_is_told_to_ask_again() {
        let rig = rig(&[("SAGAT_LAKE_CEILING", "1")]);
        rig.admission.engine.gate_waits();
        let held = {
            let admission = Arc::clone(&rig.admission);
            tokio::spawn(async move { admission.wait("a1").await })
        };
        rig.admission.engine.wait_entered().await;
        assert_eq!(rig.admission.wait("a2").await, None);
        rig.admission.engine.ungate_waits();
        assert_eq!(held.await.expect("the held wait"), Some(WaitState::Ended));
        // the place is free again
        assert_eq!(rig.admission.wait("a2").await, Some(WaitState::Ended));
    }

    #[tokio::test]
    async fn a_kill_says_only_what_the_engine_confirmed() {
        let rig = rig(&[]);
        assert_eq!(
            rig.start("a1", 1, Class::Small, 60_000).await,
            StartAnswer::Started
        );
        rig.admission.engine.mark_memory_killed("a1");
        assert_eq!(
            rig.admission.kill("a1").await,
            KillOutcome {
                ended: true,
                memory_killed: true
            }
        );
        // nothing to end, and the engine answered
        assert_eq!(
            rig.admission.kill("a1").await,
            KillOutcome {
                ended: true,
                memory_killed: false
            }
        );
        // the engine refuses the removal: not confirmed
        assert_eq!(
            rig.start("a2", 2, Class::Small, 60_000).await,
            StartAnswer::Started
        );
        rig.admission.engine.set_end_refuses(true);
        assert_eq!(
            rig.admission.kill("a2").await,
            KillOutcome {
                ended: false,
                memory_killed: false
            }
        );
        // the engine cannot be asked: not confirmed
        rig.admission.engine.set_end_refuses(false);
        rig.admission.engine.set_list_fails(true);
        assert_eq!(
            rig.admission.kill("a2").await,
            KillOutcome {
                ended: false,
                memory_killed: false
            }
        );
    }

    /// A kill of an attempt whose start is inside the engine waits for the start, and then ends what it made: it does not
    /// run first and leave the container the start is about to create.
    #[tokio::test]
    async fn a_kill_waits_for_the_start_of_its_attempt_and_ends_what_it_made() {
        let rig = rig(&[]);
        rig.admission.engine.gate_launches();
        rig.directory_of("a1");
        let start = {
            let admission = Arc::clone(&rig.admission);
            tokio::spawn(async move { admission.start("a1", body(1, Class::Small, 60_000)).await })
        };
        rig.admission.engine.wait_entered().await;
        let kill = {
            let admission = Arc::clone(&rig.admission);
            tokio::spawn(async move { admission.kill("a1").await })
        };
        // the kill has every chance to run: if it were not held back it would end nothing and the container would be made
        for _ in 0..50 {
            tokio::task::yield_now().await;
        }
        rig.admission.engine.ungate_launches();
        assert_eq!(start.await.expect("the start"), StartAnswer::Started);
        assert_eq!(
            kill.await.expect("the kill"),
            KillOutcome {
                ended: true,
                memory_killed: false
            }
        );
        assert_eq!(
            rig.admission.engine.alive(),
            0,
            "the kill ran before the start made its container"
        );
    }

    /// The per-attempt locks do not pile up: an entry is dropped when nobody waits for it.
    #[tokio::test]
    async fn the_locks_of_attempts_that_are_over_are_not_kept() {
        let rig = rig(&[]);
        for index in 0..20 {
            let attempt = format!("a{index}");
            let _ = rig.start(&attempt, index, Class::Small, 60_000).await;
            let _ = rig.admission.kill(&attempt).await;
        }
        assert!(
            rig.admission
                .attempts
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .is_empty()
        );
    }

    #[tokio::test]
    async fn the_sweep_ends_what_is_past_its_deadline_by_the_nodes_own_clock() {
        let rig = rig(&[]);
        assert_eq!(
            rig.start("a1", 1, Class::Small, 60_000).await,
            StartAnswer::Started
        );
        let before = rig.admission.sweep_once().await;
        assert_eq!(before.removed, 0);
        // past the label: the attempt's 60 s and the 30 s of grace
        rig.clock.store(NOW + 91, Ordering::SeqCst);
        let after = rig.admission.sweep_once().await;
        assert_eq!(after.removed, 1);
        assert_eq!(rig.admission.engine.alive(), 0);
    }
}
