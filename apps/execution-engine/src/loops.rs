//! The engine's background loops: the scheduler, the housekeeping and the outbox dispatcher. Each
//! runs until the shared stop flag is set; each is safe to run in several replicas at once (every
//! claim is a `SKIP LOCKED` or a re-checked command under the run's row lock).

use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use sagat_application::entropy::{Entropy, SystemEntropy};
use sagat_control_plane::stopped;
use sagat_infra_ir::JsonEvaluator;
use sagat_infra_nats::NatsPublisher;
use sagat_infra_postgres::PgRunStore;
use sagat_scheduler::{Dispatched, OutboxDispatcher, Report, Scheduler};
use tokio::sync::watch;

pub type EngineScheduler = Scheduler<PgRunStore, JsonEvaluator, SystemEntropy>;
pub type EngineDispatcher = OutboxDispatcher<PgRunStore, NatsPublisher, SystemEntropy>;

/// Liveness of the scheduler loop: it must keep ticking, even when a tick finds nothing (or the
/// database is down and every pass fails fast).
#[derive(Debug)]
pub struct Health {
    started: Instant,
    last_tick_ms: AtomicU64,
    stale_after: Duration,
}

impl Health {
    #[must_use]
    pub fn new(stale_after: Duration) -> Self {
        Self {
            started: Instant::now(),
            last_tick_ms: AtomicU64::new(0),
            stale_after,
        }
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
