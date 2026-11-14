//! The membership of a control-plane or engine process in the instance-capability registry (`FR-018-052`).
//!
//! A process builds a [`Member`] when it has its pool, hands the member's gate to whatever must obey it (the API's
//! admission, the engine's passes and probes), registers **before** it serves its first request or starts its first
//! pass ([`join`]), and runs the heartbeat it returns until it stops; the process, not the heartbeat, leaves
//! ([`Member::leave`]) once it has drained, so that an activation in the grace window still counts an engine that is
//! still working. What the registry decides and the fail mode are in [`sagat_application::instances`].
//!
//! The registry has a connection of its own ([`registry_pool_config`]): the pool of the process can be saturated by the
//! very load that makes the process slow, and a heartbeat that waits for a connection would silence a healthy process.

use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant};

use sagat_application::entropy::{Entropy, SystemEntropy};
use sagat_application::error::StoreError;
use sagat_application::instances::{
    Capability, Counting, InstanceKind, InstanceStore, LIVENESS, Member, RegistrySettings,
};
use sagat_infra_postgres::{PgInstanceStore, PgPool, PoolConfig};
use tokio::sync::watch;
use uuid::Uuid;

use crate::config::DatabaseConfig;

/// The version a process announces: the version of the binary.
pub const BINARY_VERSION: &str = env!("CARGO_PKG_VERSION");

/// What the control plane's binary can do. A constant of the code, not of its settings. Empty until a feature
/// whose control plane must honour a requirement ships (029's `access-narrowing-v1`, 031's `payload-encoding-v1`).
pub const CAPABILITIES: &[Capability] = &[];

/// A new random instance id: two processes never share one, and a restart is a new instance.
#[must_use]
pub fn new_instance_id() -> Uuid {
    Uuid::new_v4()
}

/// The pool of the registry: one connection, of its own, with the statement timeout of the process. It costs the database
/// one connection per process and spares the heartbeat the contention of the pool that serves requests.
#[must_use]
pub fn registry_pool_config(db: &DatabaseConfig) -> PoolConfig {
    let mut config = PoolConfig::new(db.url.clone());
    config.max_connections = 1;
    config.statement_timeout = db.statement_timeout;
    config
}

/// A member of the registry over `pool` (the registry's own, see [`registry_pool_config`]), with a closed gate.
/// `capabilities` is what the process can do.
#[must_use]
pub fn member(
    pool: &PgPool,
    kind: InstanceKind,
    capabilities: &[Capability],
    scheduler_owner: Option<String>,
) -> Arc<Member<PgInstanceStore>> {
    Arc::new(Member::new(
        PgInstanceStore::new(pool.clone()),
        new_instance_id(),
        kind,
        BINARY_VERSION,
        capabilities,
        scheduler_owner,
    ))
}

/// What one beat came to, as far as the log is concerned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Beating {
    Answered,
    FailedQuietly,
    FailedLoudly,
}

/// How long the registry has been silent, so that a failing heartbeat is said loudly once it is longer than the
/// liveness window: from then on the process may no longer be counted live by an activation.
#[derive(Debug, Clone, Copy)]
pub struct Silence {
    last_answer: Instant,
}

impl Silence {
    #[must_use]
    pub const fn since(at: Instant) -> Self {
        Self { last_answer: at }
    }

    /// What a beat that finished at `at` came to: an answer (the silence starts again from `at`), a failure the process
    /// has no reason yet to call serious, or a failure after the registry has been silent for longer than the liveness
    /// window.
    pub fn record(&mut self, answered: bool, at: Instant) -> Beating {
        if answered {
            self.last_answer = at;
            Beating::Answered
        } else if self.beyond_liveness(at) {
            Beating::FailedLoudly
        } else {
            Beating::FailedQuietly
        }
    }

    /// Whether the registry has been silent for longer than a process stays live.
    #[must_use]
    pub fn beyond_liveness(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.last_answer) > LIVENESS
    }

    #[must_use]
    pub fn silent_for(&self, now: Instant) -> Duration {
        now.saturating_duration_since(self.last_answer)
    }
}

/// Whether a failure of the registry that has gone on for longer than the liveness window closes the gate. A plain
/// connection failure does not (the process needs the database for everything it does: stopping it would only make an
/// outage longer, and its first beat that succeeds decides); anything else does (a lock or statement timeout on its own
/// row, a permission or function error): the process is invisible to activations and would never learn of an epoch
/// change while the database works.
#[must_use]
pub fn closes_the_gate(error: &StoreError) -> bool {
    !matches!(error, StoreError::DatabaseUnavailable)
}

/// `interval` ±20 %, so processes started together do not beat in lockstep.
#[must_use]
pub fn jittered(interval: Duration, entropy: &impl Entropy) -> Duration {
    interval.mul_f64(0.8 + 0.4 * entropy.unit().clamp(0.0, 1.0))
}

/// One beat and what it means: an answer starts the count of silence again; a failure is a warning with the instance id
/// and, once the registry has been silent for longer than the liveness window, an error, and (unless it is a plain
/// connection failure) the gate is closed. Never fatal. The gauges of the process are set at every try.
async fn beat_once<S: InstanceStore>(
    member: &Member<S>,
    silence: &mut Silence,
    clock: impl Fn() -> Instant,
) -> Beating {
    match member.beat().await {
        Ok(_) => {
            let now = clock();
            member.observe(now);
            silence.record(true, now)
        }
        Err(error) => {
            let now = clock();
            let outcome = silence.record(false, now);
            if outcome == Beating::FailedLoudly {
                tracing::error!(
                    instance = %member.instance(),
                    %error,
                    silent_for_secs = silence.silent_for(now).as_secs(),
                    runbook = "docs/runbooks/instance-registry.md#3-while-it-runs",
                    "the instance registry has not answered for longer than the liveness window: this process may no longer be counted live"
                );
                if closes_the_gate(&error) {
                    member.fail_closed();
                }
            } else {
                tracing::warn!(
                    instance = %member.instance(),
                    %error,
                    "could not say this instance is alive; trying again at the next beat"
                );
            }
            member.observe(now);
            outcome
        }
    }
}

/// While the gate is closed (the process has not registered, or it was refused) the process asks again this often, not
/// at the heartbeat's pace: a closed gate costs the whole process, and the causes that clear by themselves (the migration
/// that was not run yet, a requirement that was removed) should end it soon. One statement a second per such process.
pub const RETRY_WHILE_CLOSED: Duration = Duration::from_secs(1);

/// How long before the next try: the heartbeat's pace while the process serves, and never longer than
/// [`RETRY_WHILE_CLOSED`] while it does not.
#[must_use]
pub fn pace(serving: bool, heartbeat: Duration) -> Duration {
    if serving {
        heartbeat
    } else {
        heartbeat.min(RETRY_WHILE_CLOSED)
    }
}

/// Registers the member, says what came of it, and returns the heartbeat to run. To be called **before** the first
/// request is served or the first pass runs: until it has registered, the gate is closed and the process serves nothing.
/// The caller runs the future under its own supervision (the engine's critical tasks, the control plane's `select`):
/// if it ends, the process must not go on as if it were counted.
///
/// A process that is refused does not exit. Its gate stays closed (every call but liveness is `503`, readiness fails, no
/// pass runs), the reason is in the log and in `instance_registry_refusals_total`, and the heartbeat asks again every
/// second: it serves as soon as the registry lets it, without a restart. That is how a process started before its
/// migration has always behaved (alive, not ready, ready once migrated), and it is how a process that learns of a
/// requirement while it runs behaves.
pub async fn join<S: InstanceStore + 'static>(
    member: &Arc<Member<S>>,
    settings: RegistrySettings,
    stop: watch::Receiver<bool>,
) -> impl Future<Output = ()> + Send + 'static {
    match member.register().await {
        Ok(Counting::Counted) => tracing::info!(
            instance = %member.instance(),
            kind = member.kind().as_str(),
            version = BINARY_VERSION,
            epoch = member.gate().epoch(),
            "registered in the instance registry"
        ),
        Ok(Counting::NotCounted) => tracing::error!(
            instance = %member.instance(),
            kind = member.kind().as_str(),
            reason = sagat_application::instances::REASON_REGISTRY_FULL,
            runbook = "docs/runbooks/instance-registry.md#2-a-process-is-refused",
            "REGISTRY_FULL: the instance registry holds its most live processes; this process serves, but an activation cannot see it, and it asks again at every beat"
        ),
        Err(refusal) => tracing::error!(
            instance = %member.instance(),
            kind = member.kind().as_str(),
            reason = refusal.reason(),
            runbook = "docs/runbooks/instance-registry.md#2-a-process-is-refused",
            "{refusal}; this process serves nothing until the registry lets it, and asks again every second"
        ),
    }
    run_heartbeat(Arc::clone(member), settings, stop)
}

/// Beats every `settings.heartbeat` until `stop`. A beat that fails is logged with the instance id and tried again at the
/// next one; it is never fatal. It does **not** leave: the process leaves after it has drained.
pub async fn run_heartbeat<S: InstanceStore>(
    member: Arc<Member<S>>,
    settings: RegistrySettings,
    stop: watch::Receiver<bool>,
) {
    let mut silence = Silence::since(Instant::now());
    while !*stop.borrow() {
        let pause = jittered(
            pace(member.gate().is_open(), settings.heartbeat),
            &SystemEntropy,
        );
        tokio::select! {
            () = tokio::time::sleep(pause) => {}
            () = crate::stopped(stop.clone()) => break,
        }
        beat_once(&member, &mut silence, Instant::now).await;
    }
}

/// Serves until `serving` ends or the heartbeat does. A heartbeat that ends while the process is not stopping (it
/// panicked, or was aborted) is a process that would go on serving with an open gate and a row that ages out, so the
/// process is stopped: `stop` is set, `serving` drains, and the result is an error.
///
/// # Errors
/// What `serving` returned, or the error that says the heartbeat ended.
pub async fn serve_until_the_heartbeat_ends<F>(
    serving: F,
    heartbeat: &mut tokio::task::JoinHandle<()>,
    stop: &watch::Sender<bool>,
) -> std::io::Result<()>
where
    F: Future<Output = std::io::Result<()>>,
{
    let mut serving = std::pin::pin!(serving);
    tokio::select! {
        done = &mut serving => done,
        ended = &mut *heartbeat => {
            // it ended because the process is stopping (the loop leaves its pause when `stop` is set): the server is
            // draining, and its result is the result
            if *stop.borrow() {
                return serving.await;
            }
            tracing::error!(
                ?ended,
                "the heartbeat of the instance registry ended while the process was serving: the process is stopped, because it would go on with an open gate and a row that ages out"
            );
            let _ = stop.send(true);
            serving.await?;
            Err(std::io::Error::other("the instance registry heartbeat ended"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sagat_application::instances::{Heartbeat, Registered, Registration, Requirements};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    struct Fixed(f64);

    impl Entropy for Fixed {
        fn unit(&self) -> f64 {
            self.0
        }

        fn token(&self) -> [u8; 32] {
            [1; 32]
        }
    }

    #[test]
    fn the_jitter_stays_within_twenty_percent() {
        let base = Duration::from_millis(10_000);
        assert_eq!(jittered(base, &Fixed(0.0)), Duration::from_millis(8_000));
        assert_eq!(jittered(base, &Fixed(0.5)), Duration::from_millis(10_000));
        assert_eq!(jittered(base, &Fixed(1.0)), Duration::from_millis(12_000));
        assert_eq!(jittered(base, &Fixed(7.0)), Duration::from_millis(12_000));
        assert_eq!(jittered(base, &Fixed(-3.0)), Duration::from_millis(8_000));
    }

    #[test]
    fn the_registry_is_silent_beyond_the_liveness_window_only_after_thirty_seconds() {
        let start = Instant::now();
        let mut silence = Silence::since(start);
        assert!(!silence.beyond_liveness(start + Duration::from_secs(30)));
        assert!(silence.beyond_liveness(start + Duration::from_secs(31)));
        silence.record(true, start + Duration::from_secs(31));
        assert!(!silence.beyond_liveness(start + Duration::from_secs(40)));
        assert_eq!(
            silence.silent_for(start + Duration::from_secs(40)),
            Duration::from_secs(9)
        );
        assert_eq!(silence.silent_for(start), Duration::ZERO, "never negative");
    }

    #[test]
    fn a_failing_beat_is_loud_only_after_thirty_seconds_without_an_answer_and_an_answer_starts_the_count_again()
     {
        let start = Instant::now();
        let at = |secs| start + Duration::from_secs(secs);
        let mut silence = Silence::since(start);
        assert_eq!(silence.record(true, at(10)), Beating::Answered);
        assert_eq!(
            silence.record(false, at(25)),
            Beating::FailedQuietly,
            "15 s since the answer"
        );
        assert_eq!(
            silence.record(false, at(40)),
            Beating::FailedQuietly,
            "30 s since the answer is not beyond it"
        );
        assert_eq!(silence.record(false, at(41)), Beating::FailedLoudly);
        // an answer starts the count from its own time: at 50 s it was 9 s ago
        assert_eq!(silence.record(true, at(50)), Beating::Answered);
        assert_eq!(silence.record(false, at(59)), Beating::FailedQuietly);
        assert_eq!(silence.record(false, at(81)), Beating::FailedLoudly);
    }

    #[test]
    fn only_a_plain_connection_failure_leaves_the_gate_open_after_the_window() {
        assert!(!closes_the_gate(&StoreError::DatabaseUnavailable));
        for other in [
            StoreError::Unavailable {
                detail: "canceling statement due to lock timeout".into(),
            },
            StoreError::Internal {
                detail: "permission denied for function instance_heartbeat".into(),
            },
            StoreError::DatabaseLimit,
            StoreError::StateConflict("a state"),
        ] {
            assert!(closes_the_gate(&other), "{other:?}");
        }
    }

    #[test]
    fn a_process_that_does_not_serve_asks_again_every_second_and_one_that_does_beats_at_the_heartbeat()
     {
        let beat = Duration::from_secs(10);
        assert_eq!(pace(true, beat), beat);
        assert_eq!(pace(false, beat), Duration::from_secs(1));
        // a heartbeat shorter than the retry is not lengthened by being refused
        assert_eq!(
            pace(false, Duration::from_millis(40)),
            Duration::from_millis(40)
        );
        assert_eq!(
            pace(true, Duration::from_millis(40)),
            Duration::from_millis(40)
        );
    }

    #[test]
    fn the_control_plane_announces_its_own_version_and_a_new_id_per_start() {
        assert_eq!(BINARY_VERSION, env!("CARGO_PKG_VERSION"));
        assert_ne!(new_instance_id(), new_instance_id());
        assert_eq!(new_instance_id().get_version_num(), 4);
    }

    #[test]
    fn the_control_plane_lists_no_capability_until_a_feature_gives_it_one() {
        assert!(CAPABILITIES.is_empty());
    }

    #[test]
    fn the_registry_has_one_connection_of_its_own() {
        let db = DatabaseConfig {
            url: "postgres://u:p@127.0.0.1:1/x".to_owned(),
            max_connections: 40,
            statement_timeout: Duration::from_secs(7),
        };
        let config = registry_pool_config(&db);
        assert_eq!(
            config.max_connections, 1,
            "not a share of the pool of the process"
        );
        assert_eq!(config.statement_timeout, Duration::from_secs(7));
        assert_eq!(config.url, db.url);
    }

    /// What a registry that counts beats and fails on demand remembers.
    struct Counts {
        beats: AtomicUsize,
        failures: usize,
        left: AtomicUsize,
        down: AtomicBool,
        refuse: AtomicBool,
        /// The error a failing beat answers with.
        error: std::sync::Mutex<StoreError>,
    }

    /// A registry that counts beats and fails the first `failures` of them (a local type, because the trait is not).
    #[derive(Clone)]
    struct Counting(Arc<Counts>);

    impl Counting {
        fn new(failures: usize) -> Self {
            Self(Arc::new(Counts {
                beats: AtomicUsize::new(0),
                failures,
                left: AtomicUsize::new(0),
                down: AtomicBool::new(false),
                refuse: AtomicBool::new(false),
                error: std::sync::Mutex::new(StoreError::DatabaseUnavailable),
            }))
        }

        fn fail_with(&self, error: StoreError) {
            *self.0.error.lock().expect("lock") = error;
            self.0.down.store(true, Ordering::SeqCst);
        }
    }

    impl InstanceStore for Counting {
        async fn register(&self, _: &Registration<'_>) -> Result<Registered, StoreError> {
            if self.0.refuse.load(Ordering::SeqCst) {
                return Ok(Registered::CapabilityMissing {
                    epoch: 3,
                    missing: vec!["schedules-v1".to_owned()],
                });
            }
            Ok(Registered::Serving { epoch: 0 })
        }
        async fn heartbeat(
            &self,
            _: Uuid,
            _: &sagat_application::instances::InstanceToken,
        ) -> Result<Heartbeat, StoreError> {
            let n = self.0.beats.fetch_add(1, Ordering::SeqCst);
            if n < self.0.failures || self.0.down.load(Ordering::SeqCst) {
                return Err(self.0.error.lock().expect("lock").clone());
            }
            Ok(Heartbeat {
                found: true,
                epoch: 0,
                live: vec![],
            })
        }
        async fn deregister(
            &self,
            _: Uuid,
            _: &sagat_application::instances::InstanceToken,
        ) -> Result<bool, StoreError> {
            self.0.left.fetch_add(1, Ordering::SeqCst);
            Ok(true)
        }
        async fn epoch(&self) -> Result<i64, StoreError> {
            Ok(0)
        }
        async fn requirements(&self) -> Result<Requirements, StoreError> {
            Ok(Requirements::default())
        }
    }

    fn member_over(store: &Counting, kind: InstanceKind) -> Arc<Member<Counting>> {
        Arc::new(Member::new(
            store.clone(),
            Uuid::new_v4(),
            kind,
            "0.3.0",
            &[],
            None,
        ))
    }

    async fn until(what: &str, mut done: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !done() {
            assert!(Instant::now() < deadline, "timed out waiting for: {what}");
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    }

    #[tokio::test]
    async fn a_connection_failure_beyond_the_window_leaves_the_gate_open_and_any_other_failure_closes_it()
     {
        for (error, stays_open) in [
            (StoreError::DatabaseUnavailable, true),
            (
                StoreError::Unavailable {
                    detail: "lock timeout on its own row".into(),
                },
                false,
            ),
            (
                StoreError::Internal {
                    detail: "permission denied".into(),
                },
                false,
            ),
        ] {
            let store = Counting::new(0);
            let member = member_over(&store, InstanceKind::ExecutionEngine);
            member.register().await.expect("registered");
            store.fail_with(error.clone());
            let start = Instant::now();
            let now = std::cell::Cell::new(start);
            let clock = || now.get();
            let mut silence = Silence::since(start);
            // inside the window: whatever the failure, the gate is as it was
            now.set(start + Duration::from_secs(25));
            assert_eq!(
                beat_once(&member, &mut silence, clock).await,
                Beating::FailedQuietly
            );
            assert!(member.gate().is_open(), "{error:?} inside the window");
            // beyond it
            now.set(start + Duration::from_secs(41));
            assert_eq!(
                beat_once(&member, &mut silence, clock).await,
                Beating::FailedLoudly
            );
            assert_eq!(
                member.gate().is_open(),
                stays_open,
                "{error:?} beyond the window"
            );
            // the next beat that succeeds decides again
            store.0.down.store(false, Ordering::SeqCst);
            now.set(start + Duration::from_secs(45));
            assert_eq!(
                beat_once(&member, &mut silence, clock).await,
                Beating::Answered
            );
            assert!(
                member.gate().is_open(),
                "{error:?}: it serves again once the registry answers"
            );
        }
    }

    #[tokio::test]
    async fn the_loop_counts_the_silence_from_the_last_answer_it_got_and_says_it_loudly_only_past_the_window()
     {
        let store = Counting::new(0);
        let member = member_over(&store, InstanceKind::ExecutionEngine);
        let start = Instant::now();
        let now = std::cell::Cell::new(start);
        let clock = || now.get();
        let mut silence = Silence::since(start);
        let at = |secs: u64| now.set(start + Duration::from_secs(secs));

        at(10);
        assert_eq!(
            beat_once(&member, &mut silence, clock).await,
            Beating::Answered
        );
        store.0.down.store(true, Ordering::SeqCst);
        at(25);
        assert_eq!(
            beat_once(&member, &mut silence, clock).await,
            Beating::FailedQuietly,
            "15 s after the answer of 10 s"
        );
        at(41);
        assert_eq!(
            beat_once(&member, &mut silence, clock).await,
            Beating::FailedLoudly,
            "31 s after it"
        );
        store.0.down.store(false, Ordering::SeqCst);
        at(50);
        assert_eq!(
            beat_once(&member, &mut silence, clock).await,
            Beating::Answered
        );
        store.0.down.store(true, Ordering::SeqCst);
        at(70);
        assert_eq!(
            beat_once(&member, &mut silence, clock).await,
            Beating::FailedQuietly,
            "20 s after the answer of 50 s: an answer starts the count again, whatever came before"
        );
    }

    #[tokio::test]
    async fn the_loop_beats_survives_failing_beats_and_does_not_leave_when_it_is_stopped() {
        let store = Counting::new(3);
        let member = member_over(&store, InstanceKind::ExecutionEngine);
        let (stop_tx, stop_rx) = watch::channel(false);
        let task = tokio::spawn(run_heartbeat(
            Arc::clone(&member),
            RegistrySettings {
                heartbeat: Duration::from_millis(4),
            },
            stop_rx,
        ));
        until("more beats than failures", || {
            store.0.beats.load(Ordering::SeqCst) >= 6
        })
        .await;
        stop_tx.send(true).expect("stop");
        tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .expect("the loop stops with the process")
            .expect("no panic");
        assert_eq!(
            store.0.left.load(Ordering::SeqCst),
            0,
            "leaving is the process's, after it has drained: an activation in the grace window still counts it"
        );
        member.leave().await;
        assert_eq!(store.0.left.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn joining_registers_before_it_returns_and_a_refusal_leaves_the_gate_closed_with_the_loop_to_run()
     {
        for (refuse, serving, epoch) in [(false, true, 0), (true, false, 3)] {
            let store = Counting::new(0);
            store.0.refuse.store(refuse, Ordering::SeqCst);
            let member = member_over(&store, InstanceKind::ExecutionEngine);
            let (stop_tx, stop_rx) = watch::channel(false);
            // a heartbeat of an hour, so that nothing but `join` itself can have decided the gate when it returns
            let heartbeat = join(
                &member,
                RegistrySettings {
                    heartbeat: Duration::from_secs(3600),
                },
                stop_rx,
            )
            .await;
            assert_eq!(member.gate().is_open(), serving, "refuse={refuse}");
            assert_eq!(member.gate().epoch(), epoch, "refuse={refuse}");
            assert_eq!(
                member.gate().missing().is_empty(),
                serving,
                "refuse={refuse}: a refusal names what is missing"
            );
            // the loop is the caller's to run (and to supervise): it asks again whether or not the process was refused
            let task = tokio::spawn(heartbeat);
            tokio::task::yield_now().await;
            assert!(!task.is_finished(), "the heartbeat runs either way");
            stop_tx.send(true).expect("stop");
            tokio::time::timeout(Duration::from_secs(5), task)
                .await
                .expect("the loop stops")
                .expect("no panic");
        }
    }

    #[tokio::test]
    async fn a_loop_whose_gate_is_closed_asks_again_within_two_seconds_whatever_its_heartbeat() {
        let store = Counting::new(0);
        store.0.refuse.store(true, Ordering::SeqCst);
        let member = member_over(&store, InstanceKind::ExecutionEngine);
        let (stop_tx, stop_rx) = watch::channel(false);
        let heartbeat = join(
            &member,
            RegistrySettings {
                heartbeat: Duration::from_secs(3600),
            },
            stop_rx,
        )
        .await;
        assert!(!member.gate().is_open(), "refused");
        let task = tokio::spawn(heartbeat);
        // a closed gate is asked again at the pace of a closed gate (a second, jittered), not at an hour's
        until("the closed process asks again", || {
            store.0.beats.load(Ordering::SeqCst) >= 1
        })
        .await;
        stop_tx.send(true).expect("stop");
        tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .expect("the loop stops")
            .expect("no panic");
    }

    #[tokio::test]
    async fn a_loop_in_the_middle_of_a_long_pause_stops_at_once() {
        let store = Counting::new(0);
        let member = member_over(&store, InstanceKind::ControlPlane);
        let (stop_tx, stop_rx) = watch::channel(false);
        let task = tokio::spawn(run_heartbeat(
            Arc::clone(&member),
            RegistrySettings {
                heartbeat: Duration::from_secs(3600),
            },
            stop_rx,
        ));
        // the loop is in its pause (it has nothing else to do for an hour)
        tokio::task::yield_now().await;
        stop_tx.send(true).expect("stop");
        tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .expect("a process does not wait out its heartbeat to stop")
            .expect("no panic");
        assert_eq!(store.0.beats.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn a_loop_that_is_stopped_before_its_first_beat_never_beats() {
        let store = Counting::new(0);
        let member = member_over(&store, InstanceKind::ControlPlane);
        let (stop_tx, stop_rx) = watch::channel(false);
        stop_tx.send(true).expect("stop");
        run_heartbeat(
            member,
            RegistrySettings {
                heartbeat: Duration::from_secs(3600),
            },
            stop_rx,
        )
        .await;
        assert_eq!(store.0.beats.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn a_process_whose_heartbeat_ended_is_stopped_and_fails_while_one_whose_server_ended_is_not()
     {
        // the heartbeat panics (or is aborted) while the server runs: the server is told to drain, and the result is an error
        let (stop_tx, stop_rx) = watch::channel(false);
        let drained = Arc::new(AtomicBool::new(false));
        let server = {
            let drained = Arc::clone(&drained);
            async move {
                crate::stopped(stop_rx).await;
                drained.store(true, Ordering::SeqCst);
                Ok(())
            }
        };
        let mut dying = tokio::spawn(async { panic!("the heartbeat died") });
        let result = serve_until_the_heartbeat_ends(server, &mut dying, &stop_tx).await;
        assert!(result.is_err(), "{result:?}");
        assert!(
            drained.load(Ordering::SeqCst),
            "the server was told to drain, and did"
        );

        // the heartbeat ends because the process is stopping: not a failure, the server's result is the result
        let (stop_tx, stop_rx) = watch::channel(false);
        let mut leaving = tokio::spawn(crate::stopped(stop_rx.clone()));
        stop_tx.send(true).expect("stop");
        let drained = Arc::new(AtomicBool::new(false));
        let server = {
            let drained = Arc::clone(&drained);
            async move {
                drained.store(true, Ordering::SeqCst);
                Ok(())
            }
        };
        let result = serve_until_the_heartbeat_ends(server, &mut leaving, &stop_tx).await;
        assert!(result.is_ok(), "{result:?}");

        // the heartbeat has ended because the process is stopping while the server is still draining: the heartbeat's
        // end is not a failure, whichever of the two the select meets first
        let (stop_tx, stop_rx) = watch::channel(false);
        let mut leaving = tokio::spawn(crate::stopped(stop_rx.clone()));
        stop_tx.send(true).expect("stop");
        let (release_tx, release_rx) = watch::channel(false);
        let draining = async move {
            crate::stopped(release_rx).await;
            Ok(())
        };
        let releasing = async {
            for _ in 0..16 {
                tokio::task::yield_now().await;
            }
            release_tx.send(true).expect("the server finishes draining");
        };
        let (result, ()) = tokio::join!(
            serve_until_the_heartbeat_ends(draining, &mut leaving, &stop_tx),
            releasing
        );
        assert!(result.is_ok(), "{result:?}");

        // the server ends first (a clean shutdown): its result is the result
        let (stop_tx, _stop_rx) = watch::channel(false);
        let mut beating = tokio::spawn(std::future::pending::<()>());
        let result = serve_until_the_heartbeat_ends(async { Ok(()) }, &mut beating, &stop_tx).await;
        assert!(result.is_ok());
        assert!(!*stop_tx.borrow(), "nothing was stopped");
        beating.abort();
    }
}
