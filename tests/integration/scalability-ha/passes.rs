//! A replica works on the shards it holds and on nothing else; a replica that lost a shard is
//! refused by the fence; the one that is left takes over (FR-010-001; US01, US03; T010-007).

use std::time::Duration;

use sagat_domain::execution::RunId;
use sagat_domain::sharding::{Shard, ShardSet};
use sagat_infra_postgres::PgRunStore;
use sagat_scheduler::{Keeper, Placement, SchedulerSettings};
use serde_json::json;

use crate::support::engine::{World, activity, document, with, world};

pub use crate::support::scale::{TTL, fired, keeper, scheduler, sleepers};

fn shard_of(run: RunId) -> Shard {
    Shard::of(run.as_uuid())
}

/// Two replicas that share the 64 shards between them.
async fn two_replicas(w: &World) -> (Keeper<PgRunStore>, Keeper<PgRunStore>) {
    let (a, b) = (keeper(w, "alpha", TTL), keeper(w, "beta", TTL));
    a.round().await.expect("alpha");
    assert_eq!(a.placement().owned(), ShardSet::all());
    // beta announces and finds nothing to take; alpha now sees two replicas and gives half back;
    // beta takes it.
    b.round().await.expect("beta");
    a.round().await.expect("alpha");
    b.round().await.expect("beta");
    (a, b)
}

#[tokio::test]
async fn two_replicas_split_the_shards_and_each_works_on_its_own() {
    let w = world().await;
    let (a, b) = two_replicas(&w).await;
    let (sa, sb) = (a.placement().owned(), b.placement().owned());
    assert_eq!((sa.len(), sb.len()), (32, 32));
    assert!(
        sa.iter().all(|s| !sb.contains(s)),
        "no shard has two owners"
    );

    let runs = sleepers(&w, "split-brain-free", 60).await;
    let alpha = scheduler(
        &w,
        SchedulerSettings::default(),
        Some(a.placement().clone()),
    );
    let beta = scheduler(
        &w,
        SchedulerSettings::default(),
        Some(b.placement().clone()),
    );

    // Alpha fires exactly the timers of the runs in its shards, and touches no other.
    let report = alpha.tick().await;
    let mine: Vec<RunId> = runs
        .iter()
        .copied()
        .filter(|r| sa.contains(shard_of(*r)))
        .collect();
    assert!(
        !mine.is_empty() && mine.len() < runs.len(),
        "the sample must straddle both replicas ({} of {})",
        mine.len(),
        runs.len()
    );
    assert_eq!(report.timers_fired as usize, mine.len(), "{report:?}");
    assert_eq!(report.fenced, 0);
    for run in &runs {
        assert_eq!(fired(&w, *run).await, sa.contains(shard_of(*run)));
    }

    // Beta takes the rest, and nothing is fired twice.
    let report = beta.tick().await;
    assert_eq!(
        report.timers_fired as usize,
        runs.len() - mine.len(),
        "{report:?}"
    );
    for run in &runs {
        assert!(fired(&w, *run).await);
        let events = w
            .journal(*run)
            .await
            .iter()
            .filter(|e| *e == "timer.fired")
            .count();
        assert_eq!(events, 1, "fired once");
        w.assert_consistent(*run).await;
    }
}

#[tokio::test]
async fn a_replica_that_holds_no_shard_does_nothing() {
    let w = world().await;
    let runs = sleepers(&w, "nobody-home", 6).await;
    // It never ran a round: it holds nothing, and the timers are not its to fire.
    let nobody = Placement::new("lonely");
    let report = scheduler(&w, SchedulerSettings::default(), Some(nobody))
        .tick()
        .await;
    assert!(!report.did_work(), "{report:?}");
    for run in &runs {
        assert!(!fired(&w, *run).await);
    }
}

#[tokio::test]
async fn a_replica_that_lost_its_shards_is_refused_by_the_fence_and_changes_nothing() {
    // The split brain: alpha is paused for longer than its lease (here: made to look so), beta takes
    // the shards, and alpha wakes up still believing it holds them. It must change nothing.
    let w = world().await;
    let alpha_keeper = keeper(&w, "alpha", TTL);
    alpha_keeper.round().await.expect("alpha");
    let runs = sleepers(&w, "fenced-out", 12).await;
    let stale = scheduler(
        &w,
        SchedulerSettings::default(),
        Some(alpha_keeper.placement().clone()),
    );

    // alpha's lease ran out, and beta took every shard.
    sqlx::query("UPDATE scheduler_shards SET expires_at = clock_timestamp() - interval '1 second' WHERE owner = 'alpha'")
        .execute(&w.db.pool)
        .await
        .expect("expire alpha");
    sqlx::query("UPDATE scheduler_members SET expires_at = clock_timestamp() - interval '1 second' WHERE owner = 'alpha'")
        .execute(&w.db.pool)
        .await
        .expect("expire alpha");
    let beta_keeper = keeper(&w, "beta", TTL);
    beta_keeper.round().await.expect("beta");
    assert_eq!(beta_keeper.placement().owned(), ShardSet::all());

    // alpha's snapshot says it still holds everything (its own clock has not run out yet), so it
    // looks at every timer; the fence is what stops it, and says so.
    let report = stale.tick().await;
    assert_eq!(report.timers_fired, 0, "{report:?}");
    assert_eq!(report.fenced as usize, runs.len(), "{report:?}");
    for run in &runs {
        assert!(!fired(&w, *run).await, "the stale replica changed a run");
    }

    // beta, the owner, fires them all.
    let beta = scheduler(
        &w,
        SchedulerSettings::default(),
        Some(beta_keeper.placement().clone()),
    );
    assert_eq!(beta.tick().await.timers_fired as usize, runs.len());
    for run in &runs {
        w.assert_consistent(*run).await;
    }
}

#[tokio::test]
async fn when_a_replica_dies_the_other_takes_its_shards_and_its_work() {
    let w = world().await;
    let (a, b) = two_replicas(&w).await;
    let runs = sleepers(&w, "failover", 40).await;
    let beta = scheduler(
        &w,
        SchedulerSettings::default(),
        Some(b.placement().clone()),
    );

    // alpha is gone: no more rounds from it. Its lease and its announcement run out.
    for table in ["scheduler_shards", "scheduler_members"] {
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "UPDATE {table} SET expires_at = clock_timestamp() - interval '1 second' WHERE owner = 'alpha'"
        )))
        .execute(&w.db.pool)
        .await
        .expect("expire alpha");
    }
    drop(a);

    // Before beta's next round it still looks only at its own half.
    let own = beta.tick().await;
    let half = runs
        .iter()
        .filter(|r| b.placement().owned().contains(shard_of(**r)))
        .count();
    assert_eq!(own.timers_fired as usize, half);

    // One round later it holds everything and fires the rest.
    let round = b.round().await.expect("beta");
    assert_eq!(round.replicas, 1);
    assert_eq!(b.placement().owned(), ShardSet::all());
    let rest = beta.tick().await;
    assert_eq!(rest.timers_fired as usize, runs.len() - half, "{rest:?}");
    for run in &runs {
        assert!(fired(&w, *run).await);
        w.assert_consistent(*run).await;
    }
}

#[tokio::test]
async fn the_shards_of_a_stopped_replica_are_taken_within_the_lease_and_not_before() {
    // The one test with real time: the guarantee is "within the lease", so measure it.
    let w = world().await;
    let ttl = Duration::from_secs(3);
    let a = keeper(&w, "alpha", ttl);
    let b = keeper(&w, "beta", ttl);
    a.round().await.expect("alpha");
    b.round().await.expect("beta");
    a.round().await.expect("alpha");
    b.round().await.expect("beta");
    assert_eq!(
        a.placement().owned().len() + b.placement().owned().len(),
        64
    );
    let before = b.placement().owned().len();

    // alpha stops renewing. beta keeps trying every interval.
    let stopped = std::time::Instant::now();
    let mut took_over = None;
    while stopped.elapsed() < ttl * 3 {
        b.round().await.expect("beta");
        if b.placement().owned().len() == 64 {
            took_over = Some(stopped.elapsed());
            break;
        }
        tokio::time::sleep(ttl / 6).await;
    }
    let took_over = took_over.expect("beta took over the shards of alpha");
    assert!(before < 64);
    assert!(
        took_over >= ttl - Duration::from_millis(500),
        "not before the lease ran out: {took_over:?}"
    );
    assert!(
        took_over <= ttl + ttl / 2,
        "within the lease and one interval: {took_over:?}"
    );
}

#[tokio::test]
async fn a_replica_that_leaves_cleanly_hands_its_shards_over_without_waiting() {
    let w = world().await;
    let (a, b) = two_replicas(&w).await;
    a.leave().await;
    assert!(a.placement().owned().is_empty());
    let started = std::time::Instant::now();
    b.round().await.expect("beta");
    assert_eq!(b.placement().owned(), ShardSet::all());
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "no lease had to run out"
    );
}

#[tokio::test]
async fn children_and_parents_are_worked_by_the_owner_of_the_parent() {
    // A child that ended is reported to its parent step, and that is an action on the parent: the
    // replica that holds the parent's shard does it, whoever holds the child's.
    let w = world().await;
    let (a, b) = two_replicas(&w).await;
    let kid = document(
        "shard-kid",
        "kid",
        vec![json!({ "id": "kid", "type": "activity", "activity": "act.kid" })],
    );
    w.publish(kid).await;
    let mother = document(
        "shard-mother",
        "c",
        vec![
            with(
                json!({ "id": "c", "type": "child", "workflowId": "shard-kid" }),
                "next",
                json!("after"),
            ),
            activity("after"),
        ],
    );
    let (wf, _) = w.publish(mother).await;
    let world = &w;
    let runs = futures_util::future::join_all(
        (0..30).map(|i| async move { world.start(wf, &json!({ "i": i })).await }),
    )
    .await;
    let alpha = scheduler(
        &w,
        SchedulerSettings::default(),
        Some(a.placement().clone()),
    );
    let beta = scheduler(
        &w,
        SchedulerSettings::default(),
        Some(b.placement().clone()),
    );

    // Each replica starts the children of the parents in its shards, and only those.
    let (ra, rb) = (alpha.tick().await, beta.tick().await);
    assert_eq!(
        (ra.children_started + rb.children_started) as usize,
        runs.len(),
        "{ra:?} {rb:?}"
    );
    let owned_by_alpha = runs
        .iter()
        .filter(|r| a.placement().owned().contains(shard_of(**r)))
        .count();
    assert_eq!(ra.children_started as usize, owned_by_alpha);
    assert_eq!(ra.fenced + rb.fenced, 0);
}
