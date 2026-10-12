//! The fault scenarios of the four stories of feature 010 at the level of what a person sees
//! (T010-012, T010-017, T010-022, T010-027; US01..US04; NFR-010-SEC, NFR-010-DUR, SC-010-004):
//! duplicates between replicas of different versions, the race between a deadline and a cancel,
//! what the runtime role may do to the tables of the shards, and that what the keeper logs when it
//! fails names the cause and leaks neither the address nor the credentials of the database.
#![allow(clippy::expect_used, clippy::panic)]

#[path = "../../support/mod.rs"]
mod support;

use std::sync::Arc;
use std::time::Duration;

use sagat_application::runs::RunOperation;
use sagat_domain::execution::RunStatus;
use sagat_infra_postgres::{PgRunStore, PoolConfig, connect};
use sagat_scheduler::{Keeper, KeeperSettings, Placement, SchedulerSettings};
use serde_json::json;
use support::engine::{chain, world};
use support::scale::{TTL, fired, keeper, scheduler, sleepers};
use support::{RUNTIME_ROLE, RUNTIME_ROLE_PASSWORD};

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn replicas_of_different_versions_at_once_apply_each_transition_exactly_once() {
    // During a rolling upgrade, engines that know the shards and engines that do not work on the
    // same rows: redundant, never wrong.
    let w = world().await;
    let runs = sleepers(&w, "mixed-versions", 200).await;
    let keepers: Vec<_> = (0..4)
        .map(|i| keeper(&w, &format!("new-{i}"), TTL))
        .collect();
    for _ in 0..8 {
        for k in &keepers {
            k.round().await.expect("round");
        }
    }
    let mut schedulers: Vec<_> = keepers
        .iter()
        .map(|k| {
            scheduler(
                &w,
                SchedulerSettings::default(),
                Some(k.placement().clone()),
            )
        })
        .collect();
    // Two of the version before feature 010: no placement, every shard, no fence.
    schedulers.push(scheduler(&w, SchedulerSettings::default(), None));
    schedulers.push(scheduler(&w, SchedulerSettings::default(), None));

    let reports = futures_util::future::join_all(schedulers.iter().map(|s| async move {
        let mut fired = 0;
        while let report = s.tick().await
            && report.did_work()
        {
            fired += report.timers_fired;
        }
        fired
    }))
    .await;
    assert_eq!(
        reports.iter().sum::<u32>() as usize,
        runs.len(),
        "{reports:?}"
    );
    let counts: Vec<i64> = sqlx::query_scalar(
        "SELECT count(*) FROM run_events WHERE event_type = 'timer.fired' GROUP BY run_id",
    )
    .fetch_all(&w.db.pool)
    .await
    .expect("count");
    assert_eq!(counts.len(), runs.len());
    assert!(counts.iter().all(|n| *n == 1), "a timer fired twice");
    for run in &runs {
        w.assert_consistent(*run).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn a_deadline_and_a_cancel_at_the_same_instant_leave_one_outcome_with_every_replica_running()
{
    let w = world().await;
    let (wf, _) = w.publish(chain("deadline-vs-cancel")).await;
    let keepers: Vec<_> = (0..2).map(|i| keeper(&w, &format!("r-{i}"), TTL)).collect();
    for _ in 0..4 {
        for k in &keepers {
            k.round().await.expect("round");
        }
    }
    let schedulers: Vec<_> = keepers
        .iter()
        .map(|k| {
            scheduler(
                &w,
                SchedulerSettings::default(),
                Some(k.placement().clone()),
            )
        })
        .collect();

    // Thirty runs, each with a deadline that has passed and a cancel arriving as the scheduler looks.
    let mut runs = Vec::new();
    for i in 0..30 {
        let deadline = time::OffsetDateTime::now_utc() + time::Duration::hours(1);
        let run = w
            .start_with(
                wf,
                &json!({}),
                &format!("race-key-{i:04}-xxxx"),
                Some(deadline),
            )
            .await
            .expect("start");
        w.age_run_deadline(run).await;
        runs.push(run);
    }
    let world = &w;
    let cancels = futures_util::future::join_all(runs.iter().map(|run| async move {
        let _ = world.operate(*run, RunOperation::Cancel).await;
    }));
    let ticks = futures_util::future::join_all(schedulers.iter().map(|s| async move {
        for _ in 0..3 {
            s.tick().await;
        }
    }));
    tokio::join!(cancels, ticks);

    for run in &runs {
        let status = w.run_status(*run).await;
        assert!(
            matches!(
                status,
                RunStatus::Cancelled | RunStatus::TimedOut | RunStatus::Cancelling
            ),
            "{status:?}"
        );
        // One terminal event, whoever won.
        let ended: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM run_events WHERE run_id = $1 AND event_type IN ('run.cancelled', 'run.timed_out')",
        )
        .bind(run.as_uuid())
        .fetch_one(&w.db.pool)
        .await
        .expect("count");
        assert!(ended <= 1, "run {run} ended twice");
        w.assert_consistent(*run).await;
    }
}

#[tokio::test]
async fn the_runtime_role_cannot_rewrite_the_weights_or_remove_a_shard_but_can_take_and_keep_them()
{
    let w = world().await;
    let mut config = PoolConfig::new(w.db.url_for(RUNTIME_ROLE, RUNTIME_ROLE_PASSWORD));
    config.max_connections = 2;
    let pool = connect(&config).await.expect("connect as the runtime role");
    let refused =
        |result: Result<sqlx::postgres::PgQueryResult, sqlx::Error>, what: &str| match result {
            Err(sqlx::Error::Database(e)) => {
                assert_eq!(e.code().as_deref(), Some("42501"), "{what}")
            }
            other => panic!("{what}: expected a permission error, got {other:?}"),
        };
    // The weights are the operator's.
    refused(
        sqlx::query(
            "INSERT INTO tenant_scheduling (tenant_id, weight, updated_at) VALUES ($1, 16, now())",
        )
        .bind(w.admin.tenant_id().as_uuid())
        .execute(&pool)
        .await,
        "insert a weight",
    );
    // The shard rows exist for good; a replica takes and renews them and cannot remove or add one.
    refused(
        sqlx::query("DELETE FROM scheduler_shards")
            .execute(&pool)
            .await,
        "delete shards",
    );
    refused(
        sqlx::query("INSERT INTO scheduler_shards (shard) VALUES (64)")
            .execute(&pool)
            .await,
        "add a shard",
    );
    let store = PgRunStore::new(pool.clone(), w.cache.clone());
    let keeper = Keeper::new(
        store,
        Placement::new("runtime"),
        KeeperSettings { ttl: TTL },
    );
    let round = keeper
        .round()
        .await
        .expect("the runtime role can run a round");
    assert_eq!(round.held, 64);
    assert!(
        sqlx::query("UPDATE scheduler_shards SET owner = owner WHERE shard = 0")
            .execute(&pool)
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn what_the_keeper_logs_when_the_database_is_gone_names_the_cause_and_leaks_nothing() {
    let w = world().await;
    let target =
        w.db.url
            .split('@')
            .nth(1)
            .and_then(|r| r.split('/').next())
            .expect("addr")
            .to_owned();
    let proxy = support::nats::proxy_to(target.parse().expect("an address")).await;
    let url = w.db.url.replacen(&target, &proxy.addr.to_string(), 1);
    let mut config = PoolConfig::new(url);
    config.max_connections = 2;
    config.acquire_timeout = Duration::from_millis(500);
    let pool = connect(&config).await.expect("pool");
    let store = PgRunStore::new(pool, w.cache.clone());
    let keeper = Arc::new(Keeper::new(
        store,
        Placement::new("leaky"),
        KeeperSettings { ttl: TTL },
    ));
    keeper.round().await.expect("round");
    proxy.block();

    let (_guard, logs) = support::logs::capture();
    let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
    let run = tokio::spawn({
        let keeper = Arc::clone(&keeper);
        async move {
            sagat_execution_engine::loops::run_keeper(
                {
                    let keeper = Arc::clone(&keeper);
                    move || {
                        let keeper = Arc::clone(&keeper);
                        async move { keeper.round().await }
                    }
                },
                Duration::from_millis(100),
                || async {},
                stop_rx,
            )
            .await;
        }
    });
    tokio::time::sleep(Duration::from_millis(900)).await;
    stop_tx.send(true).expect("stop");
    run.await.expect("loop");

    let text = logs.text();
    assert!(text.contains("could not keep the shard leases"), "{text}");
    // Neither the credentials nor the address of the database are in what an operator reads.
    for secret in [
        "postgres:postgres",
        "postgres://",
        &proxy.addr.to_string(),
        &target,
    ] {
        assert!(!text.contains(secret), "the log leaks {secret:?}: {text}");
    }
}

#[tokio::test]
async fn the_pass_of_a_replica_with_no_shard_logs_nothing_and_costs_no_query() {
    // A replica that holds nothing is not an error: no log at warning level, no work.
    let w = world().await;
    let runs = sleepers(&w, "quiet", 3).await;
    let (_guard, logs) = support::logs::capture();
    let report = scheduler(
        &w,
        SchedulerSettings::default(),
        Some(Placement::new("nobody")),
    )
    .tick()
    .await;
    assert!(!report.did_work() && report.failed == 0, "{report:?}");
    assert_eq!(logs.text(), "", "an idle replica must not be noisy");
    for run in runs {
        assert!(!fired(&w, run).await);
    }
}
