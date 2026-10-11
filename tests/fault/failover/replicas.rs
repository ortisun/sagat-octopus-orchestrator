//! A replica that dies in the middle of a pass, and one that is held up past its lease.

use std::time::Duration;

use sagat_application::runs::{Lock, RunStore};
use sagat_domain::sharding::ShardSet;
use sagat_scheduler::SchedulerSettings;

use crate::assert_each_fired_once;
use crate::support::engine::world;
use crate::support::scale::{TTL, keeper, scheduler, sleepers};

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn a_replica_that_dies_in_the_middle_of_a_pass_leaves_nothing_half_done() {
    let w = world().await;
    let runs = sleepers(&w, "chaos-crash", 400).await;
    let alpha = keeper(&w, "alpha", TTL);
    alpha.round().await.expect("alpha");
    let doomed = scheduler(
        &w,
        SchedulerSettings {
            concurrency: 8,
            batch: 100,
            ..SchedulerSettings::default()
        },
        Some(alpha.placement().clone()),
    );

    // alpha is killed while it is firing: the pass is dropped where it stands, transactions open.
    let pass = tokio::spawn(async move { doomed.tick().await });
    tokio::time::sleep(Duration::from_millis(120)).await;
    pass.abort();
    let _ = pass.await;
    let done: i64 =
        sqlx::query_scalar("SELECT count(*) FROM run_events WHERE event_type = 'timer.fired'")
            .fetch_one(&w.db.pool)
            .await
            .expect("count");
    assert!(
        done < 400,
        "the pass finished before it was killed ({done}): nothing was tested"
    );

    // Its leases run out; beta takes over and finishes the work.
    sqlx::query("UPDATE scheduler_shards SET expires_at = clock_timestamp() - interval '1 second' WHERE owner = 'alpha'")
        .execute(&w.db.pool)
        .await
        .expect("expire");
    sqlx::query("UPDATE scheduler_members SET expires_at = clock_timestamp() - interval '1 second' WHERE owner = 'alpha'")
        .execute(&w.db.pool)
        .await
        .expect("expire");
    let beta = keeper(&w, "beta", TTL);
    beta.round().await.expect("beta");
    assert_eq!(beta.placement().owned(), ShardSet::all());
    let taker = scheduler(
        &w,
        SchedulerSettings::default(),
        Some(beta.placement().clone()),
    );
    while taker.tick().await.did_work() {}
    assert_each_fired_once(&w, &runs).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn a_replica_held_up_past_its_lease_in_the_middle_of_an_action_is_refused_when_it_goes_on() {
    // The split brain with real timing: alpha has chosen its candidates and is held up (here, by a
    // lock on a run it is about to change); while it waits, its lease runs out and beta takes the
    // shard. When the hold-up ends, alpha goes on with the action it had chosen. It must not commit.
    let w = world().await;
    let runs = sleepers(&w, "chaos-held", 12).await;
    let victim = runs[0];
    let alpha = keeper(&w, "alpha", TTL);
    alpha.round().await.expect("alpha");
    let slow = scheduler(
        &w,
        SchedulerSettings {
            concurrency: 4,
            ..SchedulerSettings::default()
        },
        Some(alpha.placement().clone()),
    );

    // Whoever holds this transaction holds the victim's row lock.
    let lock = w
        .store
        .begin(w.admin.tenant_id(), victim, Lock::Wait)
        .await
        .expect("begin")
        .expect("the run");
    let pass = tokio::spawn(async move { slow.tick().await });
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert!(
        !pass.is_finished(),
        "alpha is waiting for the victim's lock"
    );

    // alpha's lease runs out and beta takes every shard.
    sqlx::query("UPDATE scheduler_shards SET expires_at = clock_timestamp() - interval '1 second' WHERE owner = 'alpha'")
        .execute(&w.db.pool)
        .await
        .expect("expire");
    sqlx::query("UPDATE scheduler_members SET expires_at = clock_timestamp() - interval '1 second' WHERE owner = 'alpha'")
        .execute(&w.db.pool)
        .await
        .expect("expire");
    let beta = keeper(&w, "beta", TTL);
    beta.round().await.expect("beta");
    assert_eq!(beta.placement().owned(), ShardSet::all());

    // The hold-up ends.
    drop(lock);
    let report = tokio::time::timeout(Duration::from_secs(10), pass)
        .await
        .expect("alpha's pass ends")
        .expect("join");
    assert_eq!(
        report.fenced, 1,
        "the action alpha had chosen was refused: {report:?}"
    );
    assert_eq!(
        report.timers_fired, 11,
        "the others were done while alpha still held the shard: {report:?}"
    );
    let taker = scheduler(
        &w,
        SchedulerSettings::default(),
        Some(beta.placement().clone()),
    );
    assert_eq!(
        taker.tick().await.timers_fired,
        1,
        "beta fires the victim's timer"
    );
    assert_each_fired_once(&w, &runs).await;
}
