//! The engine's background loops: the scheduler, the housekeeping and the outbox dispatcher. Each
//! runs until the shared stop flag is set; each is safe to run in several replicas at once (every
//! claim is a `SKIP LOCKED` or a re-checked command under the run's row lock).

use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use sagat_application::entropy::{Entropy, SystemEntropy};
use sagat_control_plane::stopped;
use sagat_infra_ir::JsonEvaluator;
use sagat_infra_nats::NatsPublisher;
use sagat_infra_postgres::PgRunStore;
use sagat_scheduler::{
    DeliveryRound, DeliveryWorker, Dispatched, EventDispatcher, EventRound, Keeper, Monitor,
    MonitorRound, OutboxDispatcher, ProjectionRound, Report, Round, Scheduler, TelemetryProjector,
};
use tokio::sync::watch;

pub type EngineScheduler = Scheduler<PgRunStore, JsonEvaluator, SystemEntropy>;
pub type EngineDispatcher = OutboxDispatcher<PgRunStore, NatsPublisher, SystemEntropy>;
pub type EngineEvents = EventDispatcher<PgRunStore, JsonEvaluator, SystemEntropy>;
pub type EngineProjector = TelemetryProjector<PgRunStore>;
pub type EngineMonitor = Monitor<PgRunStore>;
pub type EngineKeeper = Keeper<PgRunStore>;
pub type EngineDeliveries = DeliveryWorker<
    PgRunStore,
    sagat_infra_connectors::webhook::GuardedSender<PgRunStore>,
    SystemEntropy,
    sagat_infra_connectors::publish::Publisher<
        sagat_infra_connectors::guard::PolicyGuard<PgRunStore>,
        PgRunStore,
    >,
>;

/// Liveness of the scheduler loop: it must keep ticking, even when a tick finds nothing (or the
/// database is down and every pass fails fast).
#[derive(Debug)]
pub struct Health {
    started: Instant,
    process_failed: AtomicBool,
    last_tick_ms: AtomicU64,
    stale_after: Duration,
    last_round_ms: AtomicU64,
    keeper_stale_after: Duration,
}

impl Health {
    #[must_use]
    pub fn new(stale_after: Duration) -> Self {
        Self {
            started: Instant::now(),
            process_failed: AtomicBool::new(false),
            last_tick_ms: AtomicU64::new(0),
            stale_after,
            last_round_ms: AtomicU64::new(0),
            keeper_stale_after: stale_after,
        }
    }

    /// How long the shard keeper may go without finishing a round (a round that fails counts: the
    /// loop is alive and the database is what is down) before `/livez` reports it.
    #[must_use]
    pub const fn with_keeper_stale_after(mut self, stale_after: Duration) -> Self {
        self.keeper_stale_after = stale_after;
        self
    }

    /// The keeper finished a round, well or badly.
    pub fn keeper_rounded(&self) {
        self.last_round_ms.store(self.now_ms(), Ordering::Relaxed);
    }

    /// Marks a critical task failure. This is sticky: a process whose task set changed underneath
    /// it must be replaced, even if the remaining loops are still ticking while they drain.
    pub fn fail_process(&self) {
        self.process_failed.store(true, Ordering::Release);
    }

    /// Whether every critical task is still expected to be running.
    #[must_use]
    pub fn process_alive(&self) -> bool {
        !self.process_failed.load(Ordering::Acquire)
    }

    /// Whether the keeper's loop is running (or has only just started): a replica whose loop ended
    /// would go on holding shards it never renews, and nothing else would say so.
    #[must_use]
    pub fn keeper_alive(&self) -> bool {
        if !self.process_alive() {
            return false;
        }
        let age = self
            .now_ms()
            .saturating_sub(self.last_round_ms.load(Ordering::Relaxed));
        u128::from(age) < self.keeper_stale_after.as_millis()
    }

    fn now_ms(&self) -> u64 {
        u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    pub fn ticked(&self) {
        self.last_tick_ms.store(self.now_ms(), Ordering::Relaxed);
    }

    /// Whether the scheduler loop ticked recently (or has only just started).
    #[must_use]
    pub fn scheduler_alive(&self) -> bool {
        if !self.process_alive() {
            return false;
        }
        let age = self
            .now_ms()
            .saturating_sub(self.last_tick_ms.load(Ordering::Relaxed));
        u128::from(age) < self.stale_after.as_millis()
    }
}

/// `interval` ±20 %, so replicas started together do not tick in lockstep.
fn jittered(interval: Duration, entropy: &impl Entropy) -> Duration {
    interval.mul_f64(0.8 + 0.4 * entropy.unit().clamp(0.0, 1.0))
}

/// Sleeps `pause`, or returns `true` early when the stop flag is set.
async fn stop_during(pause: Duration, stop: &watch::Receiver<bool>) -> bool {
    tokio::select! {
        () = tokio::time::sleep(pause) => false,
        () = stopped(stop.clone()) => true,
    }
}

/// Runs the scheduler passes until stopped. A tick that did work is followed immediately by the
/// next, so a backlog of due timers drains at full speed; an idle tick is followed by a pause.
pub async fn run_scheduler<F, Fut>(
    mut tick: F,
    interval: Duration,
    health: Arc<Health>,
    stop: watch::Receiver<bool>,
) where
    F: FnMut() -> Fut,
    Fut: Future<Output = Report>,
{
    while !*stop.borrow() {
        let report = tick().await;
        health.ticked();
        if report.did_work() {
            tokio::task::yield_now().await;
            continue;
        }
        if stop_during(jittered(interval, &SystemEntropy), &stop).await {
            break;
        }
    }
    tracing::info!("scheduler stopped");
}

/// Keeps the replica's shard leases every `every` until stopped, and gives them back when it stops
/// so that the others take them at once (`leave`). A round that fails (the database is down) is
/// retried at the next interval: the snapshot the passes read expires by itself in the meantime.
pub async fn run_keeper<F, Fut, L, LeaveFut>(
    mut round: F,
    every: Duration,
    leave: L,
    stop: watch::Receiver<bool>,
) where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<Round, sagat_application::error::StoreError>>,
    L: FnOnce() -> LeaveFut,
    LeaveFut: Future<Output = ()>,
{
    while !*stop.borrow() {
        match round().await {
            Ok(done) => tracing::debug!(
                replicas = done.replicas,
                held = done.held,
                acquired = done.acquired,
                released = done.released,
                lost = done.lost,
                "shard leases kept"
            ),
            Err(error) => {
                tracing::warn!(
                    %error,
                    runbook = "docs/runbooks/scaling.md#keeper-failing",
                    "could not keep the shard leases; the passes stop by themselves when they run out"
                );
                metrics::counter!("scheduler_keeper_failures_total").increment(1);
            }
        }
        if stop_during(jittered(every, &SystemEntropy), &stop).await {
            break;
        }
    }
    leave().await;
    tracing::info!("shard keeper stopped");
}

/// Purges expired records now and then.
pub async fn run_housekeeping<F, Fut>(mut sweep: F, every: Duration, stop: watch::Receiver<bool>)
where
    F: FnMut() -> Fut,
    Fut: Future<Output = ()>,
{
    while !*stop.borrow() {
        sweep().await;
        if stop_during(jittered(every, &SystemEntropy), &stop).await {
            break;
        }
    }
}

/// Publishes the outbox until stopped. A full round is followed immediately by the next; an idle
/// or failing round by a pause (a failing message already carries its own backoff in the database).
pub async fn run_dispatcher<F, Fut>(
    mut round: F,
    batch: u32,
    poll: Duration,
    stop: watch::Receiver<bool>,
) where
    F: FnMut() -> Fut,
    Fut: Future<Output = Dispatched>,
{
    while !*stop.borrow() {
        let round = round().await;
        // A round never claims more than `batch`, so publishing a full batch means no failures.
        if round.published >= batch {
            tokio::task::yield_now().await;
            continue;
        }
        if stop_during(jittered(poll, &SystemEntropy), &stop).await {
            break;
        }
    }
    tracing::info!("outbox dispatcher stopped");
}

/// Dispatches admitted events until stopped. A round that did something is followed at once by the
/// next, so a backlog drains at full speed; an idle one by a pause.
pub async fn run_events<F, Fut>(mut round: F, poll: Duration, stop: watch::Receiver<bool>)
where
    F: FnMut() -> Fut,
    Fut: Future<Output = EventRound>,
{
    while !*stop.borrow() {
        if round().await.did_work() {
            tokio::task::yield_now().await;
            continue;
        }
        if stop_during(jittered(poll, &SystemEntropy), &stop).await {
            break;
        }
    }
    tracing::info!("event dispatcher stopped");
}

/// Sends outbound webhooks until stopped: the same rhythm as the event dispatcher.
pub async fn run_deliveries<F, Fut>(mut round: F, poll: Duration, stop: watch::Receiver<bool>)
where
    F: FnMut() -> Fut,
    Fut: Future<Output = DeliveryRound>,
{
    while !*stop.borrow() {
        if round().await.did_work() {
            tokio::task::yield_now().await;
            continue;
        }
        if stop_during(jittered(poll, &SystemEntropy), &stop).await {
            break;
        }
    }
    tracing::info!("delivery worker stopped");
}

/// Projects the journal into spans and measurements until stopped. A round that took runs is
/// followed at once by the next, so a backlog drains at full speed; an idle one by a pause, and a
/// round that failed (the collector or the database is down) by a pause that doubles up to
/// thirty seconds, so an outage is not a storm of attempts.
pub async fn run_telemetry<F, Fut>(mut round: F, poll: Duration, stop: watch::Receiver<bool>)
where
    F: FnMut() -> Fut,
    Fut: Future<Output = ProjectionRound>,
{
    const LONGEST_PAUSE: Duration = Duration::from_secs(30);
    let mut failing = poll;
    while !*stop.borrow() {
        let done = round().await;
        let pause = if done.failed {
            failing = (failing * 2).min(LONGEST_PAUSE);
            failing
        } else {
            failing = poll;
            if done.did_work() {
                tokio::task::yield_now().await;
                continue;
            }
            poll
        };
        if stop_during(jittered(pause, &SystemEntropy), &stop).await {
            break;
        }
    }
    tracing::info!("telemetry projector stopped");
}

/// Looks for conditions that need a person, and samples the queues, every `every` until stopped.
pub async fn run_monitor<F, Fut>(mut round: F, every: Duration, stop: watch::Receiver<bool>)
where
    F: FnMut() -> Fut,
    Fut: Future<Output = MonitorRound>,
{
    while !*stop.borrow() {
        round().await;
        if stop_during(jittered(every, &SystemEntropy), &stop).await {
            break;
        }
    }
    tracing::info!("monitor stopped");
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixed(f64);

    impl Entropy for Fixed {
        fn unit(&self) -> f64 {
            self.0
        }

        fn token(&self) -> [u8; 32] {
            [1; 32]
        }
    }

    use std::sync::atomic::AtomicUsize;

    /// Runs `loop_under_test` for `for_how_long`, then stops it and returns how often it called
    /// its work function.
    async fn calls_during<L, Fut>(
        for_how_long: Duration,
        make: impl FnOnce(Arc<AtomicUsize>, watch::Receiver<bool>) -> L,
    ) -> usize
    where
        L: Future<Output = Fut> + Send + 'static,
        Fut: Send + 'static,
    {
        let calls = Arc::new(AtomicUsize::new(0));
        let (stop_tx, stop_rx) = watch::channel(false);
        let handle = tokio::spawn(make(Arc::clone(&calls), stop_rx));
        tokio::time::sleep(for_how_long).await;
        stop_tx.send(true).expect("stop");
        tokio::time::timeout(Duration::from_secs(2), handle)
            .await
            .expect("the loop stops promptly")
            .expect("no panic");
        calls.load(Ordering::SeqCst)
    }

    #[tokio::test]
    async fn an_idle_scheduler_pauses_between_ticks_instead_of_spinning() {
        let ticks = calls_during(Duration::from_millis(500), |calls, stop| {
            run_scheduler(
                move || {
                    calls.fetch_add(1, Ordering::SeqCst);
                    std::future::ready(Report::default())
                },
                Duration::from_millis(100),
                Arc::new(Health::new(Duration::from_secs(30))),
                stop,
            )
        })
        .await;
        // 100 ms ±20 % over 500 ms is about five ticks; a loop that does not pause makes millions.
        assert!((3..=8).contains(&ticks), "{ticks} ticks in 500 ms");
    }

    #[tokio::test]
    async fn a_scheduler_that_found_work_ticks_again_at_once_until_it_runs_dry() {
        let ticks = calls_during(Duration::from_millis(400), |calls, stop| {
            run_scheduler(
                move || {
                    let n = calls.fetch_add(1, Ordering::SeqCst);
                    std::future::ready(Report {
                        timers_fired: u32::from(n < 20),
                        ..Report::default()
                    })
                },
                Duration::from_millis(200),
                Arc::new(Health::new(Duration::from_secs(30))),
                stop,
            )
        })
        .await;
        // Twenty busy ticks back to back, then one or two idle pauses of ~200 ms.
        assert!((21..=26).contains(&ticks), "{ticks} ticks");
    }

    #[tokio::test]
    async fn every_tick_refreshes_the_liveness_even_when_idle() {
        let health = Arc::new(Health::new(Duration::from_millis(150)));
        let probe = Arc::clone(&health);
        let handle_health = Arc::clone(&health);
        let calls = calls_during(Duration::from_millis(600), move |calls, stop| {
            run_scheduler(
                move || {
                    calls.fetch_add(1, Ordering::SeqCst);
                    std::future::ready(Report::default())
                },
                Duration::from_millis(50),
                handle_health,
                stop,
            )
        })
        .await;
        assert!(calls >= 5);
        assert!(probe.scheduler_alive(), "ticks within the last 150 ms");
    }

    #[tokio::test]
    async fn the_dispatcher_pauses_when_idle_or_failing_and_continues_only_after_a_full_clean_round()
     {
        let idle = calls_during(Duration::from_millis(400), |calls, stop| {
            run_dispatcher(
                move || {
                    calls.fetch_add(1, Ordering::SeqCst);
                    std::future::ready(Dispatched::default())
                },
                10,
                Duration::from_millis(100),
                stop,
            )
        })
        .await;
        assert!((2..=6).contains(&idle), "idle: {idle} rounds in 400 ms");

        // A round that publishes a full batch is followed at once by the next one…
        let draining = calls_during(Duration::from_millis(300), |calls, stop| {
            run_dispatcher(
                move || {
                    let n = calls.fetch_add(1, Ordering::SeqCst);
                    std::future::ready(Dispatched {
                        published: if n < 15 { 10 } else { 0 },
                        failed: 0,
                    })
                },
                10,
                Duration::from_millis(200),
                stop,
            )
        })
        .await;
        assert!((16..=18).contains(&draining), "draining: {draining} rounds");

        // …but a full round with failures backs off instead of hammering a broker that is down.
        let failing = calls_during(Duration::from_millis(400), |calls, stop| {
            run_dispatcher(
                move || {
                    calls.fetch_add(1, Ordering::SeqCst);
                    std::future::ready(Dispatched {
                        published: 9,
                        failed: 1,
                    })
                },
                10,
                Duration::from_millis(100),
                stop,
            )
        })
        .await;
        assert!(
            (2..=6).contains(&failing),
            "failing: {failing} rounds in 400 ms"
        );
    }

    #[tokio::test]
    async fn the_keeper_runs_its_rounds_survives_a_failing_one_and_leaves_once_when_stopped() {
        let left = Arc::new(AtomicUsize::new(0));
        let probe = Arc::clone(&left);
        let rounds = calls_during(Duration::from_millis(400), move |calls, stop| {
            run_keeper(
                move || {
                    let n = calls.fetch_add(1, Ordering::SeqCst);
                    std::future::ready(if n == 1 {
                        Err(sagat_application::error::StoreError::Internal {
                            detail: "the database is down".into(),
                        })
                    } else {
                        Ok(Round::default())
                    })
                },
                Duration::from_millis(100),
                move || {
                    probe.fetch_add(1, Ordering::SeqCst);
                    std::future::ready(())
                },
                stop,
            )
        })
        .await;
        // 100 ms ±20 % over 400 ms: about four rounds, the failing one among them.
        assert!((2..=6).contains(&rounds), "{rounds} rounds in 400 ms");
        assert_eq!(left.load(Ordering::SeqCst), 1, "it gave the shards back");
    }

    #[tokio::test]
    async fn housekeeping_runs_at_once_and_then_only_every_interval() {
        let sweeps = calls_during(Duration::from_millis(500), |calls, stop| {
            run_housekeeping(
                move || {
                    calls.fetch_add(1, Ordering::SeqCst);
                    std::future::ready(())
                },
                Duration::from_millis(200),
                stop,
            )
        })
        .await;
        assert!((2..=4).contains(&sweeps), "{sweeps} sweeps in 500 ms");
    }

    #[test]
    fn jitter_stays_within_twenty_percent() {
        let base = Duration::from_millis(1000);
        assert_eq!(jittered(base, &Fixed(0.0)), Duration::from_millis(800));
        assert_eq!(jittered(base, &Fixed(0.5)), Duration::from_millis(1000));
        assert_eq!(jittered(base, &Fixed(1.0)), Duration::from_millis(1200));
        assert_eq!(
            jittered(base, &Fixed(9.0)),
            Duration::from_millis(1200),
            "clamped"
        );
    }

    #[test]
    fn the_keeper_is_alive_until_it_stops_rounding_whatever_its_rounds_found() {
        let health =
            Health::new(Duration::from_secs(30)).with_keeper_stale_after(Duration::from_millis(60));
        assert!(health.keeper_alive(), "just started");
        std::thread::sleep(Duration::from_millis(90));
        assert!(!health.keeper_alive(), "no round for longer than the limit");
        health.keeper_rounded();
        assert!(
            health.keeper_alive(),
            "a round revives it, a failing one too"
        );
        assert!(health.scheduler_alive(), "the scheduler's limit is its own");
    }

    #[test]
    fn the_scheduler_is_alive_until_it_stops_ticking() {
        let health = Health::new(Duration::from_millis(60));
        assert!(health.scheduler_alive(), "just started");
        std::thread::sleep(Duration::from_millis(90));
        assert!(
            !health.scheduler_alive(),
            "no tick for longer than the limit"
        );
        health.ticked();
        assert!(health.scheduler_alive(), "a tick revives it");
    }
}
