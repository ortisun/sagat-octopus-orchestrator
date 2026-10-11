//! The database under fault: connections killed over and over under load, and a replica cut off
//! from it altogether.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::{Duration, Instant};

use sagat_application::entropy::SystemEntropy;
use sagat_application::execution::{DefinitionCache, Engine};
use sagat_infra_ir::{JsonEvaluator, JsonIrCodec};
use sagat_infra_postgres::{PgRunStore, PoolConfig, connect};
use sagat_scheduler::{Keeper, KeeperSettings, Placement, Scheduler, SchedulerSettings};

use crate::assert_each_fired_once;
use crate::support::engine::{World, world};
use crate::support::nats::proxy_to;
use crate::support::scale::{fired, sleepers};

fn engine_scheduler(
    store: PgRunStore,
    w: &World,
    placement: Placement,
    batch: u32,
) -> sagat_scheduler::Scheduler<PgRunStore, JsonEvaluator, SystemEntropy> {
    Scheduler::new(
        store,
        Engine::new(JsonEvaluator),
        SystemEntropy,
        SchedulerSettings {
            batch,
            concurrency: 6,
            ..SchedulerSettings::default()
        },
    )
    .with_children(w.cache.clone())
    .with_placement(placement)
}

async fn eventually_all_fired(
    w: &World,
    runs: &[sagat_domain::execution::RunId],
    within: Duration,
) {
    let deadline = Instant::now() + within;
    loop {
        let mut all = true;
        for run in runs {
            if !fired(w, *run).await {
                all = false;
                break;
            }
        }
        if all {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "not every timer fired within {within:?}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn killing_every_database_connection_again_and_again_under_load_loses_nothing_and_fires_nothing_twice()
 {
    let w = world().await;
    let runs = sleepers(&w, "chaos-kill", 300).await;

    // Two replicas, each with a pool of its own, sharing the shards.
    let (ra, rb) = (w.replica_with_pool(12).await, w.replica_with_pool(12).await);
    let ttl = KeeperSettings {
        ttl: Duration::from_secs(30),
    };
    let (ka, kb) = (
        Arc::new(Keeper::new(ra.store.clone(), Placement::new("alpha"), ttl)),
        Arc::new(Keeper::new(rb.store.clone(), Placement::new("beta"), ttl)),
    );
    for _ in 0..3 {
        let _ = ka.round().await;
        let _ = kb.round().await;
    }
    let sa = Arc::new(engine_scheduler(
        ra.store.clone(),
        &w,
        ka.placement().clone(),
        40,
    ));
    let sb = Arc::new(engine_scheduler(
        rb.store.clone(),
        &w,
        kb.placement().clone(),
        40,
    ));

    let running = Arc::new(AtomicBool::new(true));
    let killing = Arc::new(AtomicBool::new(true));
    let failed = Arc::new(AtomicU32::new(0));
    let kill = {
        let (pool, killing) = (w.db.pool.clone(), Arc::clone(&killing));
        tokio::spawn(async move {
            while killing.load(Ordering::SeqCst) {
                let _ = sqlx::query(
                    "SELECT pg_terminate_backend(pid) FROM pg_stat_activity \
                     WHERE datname = current_database() AND pid <> pg_backend_pid()",
                )
                .execute(&pool)
                .await;
                tokio::time::sleep(Duration::from_millis(120)).await;
            }
        })
    };
    let mut loops = Vec::new();
    for scheduler in [Arc::clone(&sa), Arc::clone(&sb)] {
        let (running, failed) = (Arc::clone(&running), Arc::clone(&failed));
        loops.push(tokio::spawn(async move {
            while running.load(Ordering::SeqCst) {
                let report = scheduler.tick().await;
                failed.fetch_add(report.failed, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }));
    }
    for keeper in [Arc::clone(&ka), Arc::clone(&kb)] {
        let running = Arc::clone(&running);
        loops.push(tokio::spawn(async move {
            while running.load(Ordering::SeqCst) {
                let _ = keeper.round().await;
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        }));
    }

    // A few seconds of chaos, then the fault ends and the platform finishes by itself.
    tokio::time::sleep(Duration::from_secs(4)).await;
    killing.store(false, Ordering::SeqCst);
    kill.await.expect("killer");
    eventually_all_fired(&w, &runs, Duration::from_secs(60)).await;
    running.store(false, Ordering::SeqCst);
    for task in loops {
        task.await.expect("loop");
    }

    assert!(
        failed.load(Ordering::SeqCst) > 0,
        "the faults never reached an action: the test proved nothing"
    );
    assert_each_fired_once(&w, &runs).await;
}

/// `host:port` of the database of a world.
fn address_of(url: &str) -> String {
    let after_at = url.split('@').nth(1).expect("a url with credentials");
    after_at.split('/').next().expect("a host").to_owned()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn a_replica_cut_off_from_the_database_stops_by_itself_and_rejoins_when_it_returns() {
    let w = world().await;
    let runs = sleepers(&w, "chaos-cut", 40).await;

    // alpha reaches the database through a proxy that can be cut.
    let target = address_of(&w.db.url).parse().expect("an address");
    let proxy = proxy_to(target).await;
    let url =
        w.db.url
            .replacen(&address_of(&w.db.url), &proxy.addr.to_string(), 1);
    let mut config = PoolConfig::new(url);
    config.max_connections = 8;
    config.acquire_timeout = Duration::from_secs(1);
    let pool = connect(&config).await.expect("a pool through the proxy");
    let cache = DefinitionCache::new(Arc::new(JsonIrCodec), 16);
    let through = PgRunStore::new(pool, cache);
    let lease = Duration::from_secs(3);
    let alpha = Keeper::new(
        through.clone(),
        Placement::new("alpha"),
        KeeperSettings { ttl: lease },
    );
    alpha.round().await.expect("alpha takes the shards");
    assert_eq!(alpha.placement().owned().len(), 64);
    let first_generation = alpha
        .placement()
        .snapshot()
        .leases()
        .first()
        .expect("a lease")
        .generation;
    let scheduler_alpha = engine_scheduler(through.clone(), &w, alpha.placement().clone(), 200);

    // The cut. alpha keeps trying and keeps failing.
    proxy.block();
    let cut = Instant::now();
    while cut.elapsed() < lease + Duration::from_millis(500) {
        assert!(
            alpha.round().await.is_err(),
            "a round cannot succeed without the database"
        );
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    // Without a single call to the database, alpha holds nothing any more: it stopped on its own.
    assert!(
        alpha.placement().owned().is_empty(),
        "a replica that cannot renew its leases must stop working on the shards"
    );
    let report = scheduler_alpha.tick().await;
    assert!(!report.did_work(), "{report:?}");

    // beta, with the database, finds the leases run out, takes everything and fires the timers.
    let beta = Keeper::new(
        w.store.clone(),
        Placement::new("beta"),
        KeeperSettings { ttl: lease },
    );
    beta.round().await.expect("beta");
    assert_eq!(beta.placement().owned().len(), 64);
    let scheduler_beta = engine_scheduler(w.store.clone(), &w, beta.placement().clone(), 200);
    while scheduler_beta.tick().await.did_work() {}
    eventually_all_fired(&w, &runs, Duration::from_secs(10)).await;

    // The network comes back: alpha announces itself, the two share the shards, and alpha's leases
    // are new ones (their generation is past the one it lost).
    proxy.unblock();
    for _ in 0..6 {
        let _ = alpha.round().await;
        let _ = beta.round().await;
    }
    let mine = alpha.placement().owned();
    assert!(!mine.is_empty(), "alpha did not rejoin");
    assert!(mine.len() + beta.placement().owned().len() == 64);
    let rejoined = alpha
        .placement()
        .snapshot()
        .leases()
        .iter()
        .map(|l| l.generation)
        .min()
        .expect("a lease");
    assert!(
        rejoined > first_generation,
        "{rejoined} against {first_generation}"
    );
    assert_each_fired_once(&w, &runs).await;
}
