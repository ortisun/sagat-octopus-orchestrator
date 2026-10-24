//! Post-wait scheduler-expiry controls for dormant flat OrdinaryLeafV1 Finish.
//!
//! Install as a child of `scoped_leaf_finish_native.rs`. This proves only that the existing
//! native adapter samples database time after an unchanged scheduler-shard row wait, for first
//! application and exact receipt replay.

use super::*;
use sagat_application::shards::ShardStore;
use tokio::time::timeout;

const SHARD_WAIT_DEADLINE: Duration = Duration::from_secs(10);
const SHORT_LEASE: Duration = Duration::from_secs(3);
const FRESH_LEASE: Duration = Duration::from_secs(600);

async fn actual_shard_waiter(pool: &PgPool, gate_pid: i32) -> i32 {
    loop {
        let waiter: Option<i32> = sqlx::query_scalar(
            "SELECT waiter.pid FROM pg_stat_activity waiter \
             WHERE waiter.datname=current_database() AND waiter.usename='sagat_app' \
             AND waiter.wait_event_type='Lock' \
             AND $1=ANY(pg_blocking_pids(waiter.pid)) \
             AND position('FROM scheduler_shards' in waiter.query)>0 \
             LIMIT 1",
        )
        .bind(gate_pid)
        .fetch_optional(pool)
        .await
        .expect("actual native scheduler-shard waiter observation");
        if let Some(waiter) = waiter {
            return waiter;
        }
        tokio::task::yield_now().await;
    }
}

async fn database_clock_reaches(pool: &PgPool, expiry: OffsetDateTime) {
    loop {
        let expired: bool = sqlx::query_scalar("SELECT clock_timestamp()>=$1")
            .bind(expiry)
            .fetch_one(pool)
            .await
            .expect("database expiry clock");
        if expired {
            return;
        }
        tokio::task::yield_now().await;
    }
}

async fn assert_nowait_blocked(
    pool: &PgPool,
    statement: &'static str,
    tenant: Uuid,
    resource: Uuid,
    what: &str,
) {
    let error = sqlx::query(statement)
        .bind(tenant)
        .bind(resource)
        .execute(pool)
        .await
        .expect_err("native transaction must retain the guarded tuple during shard wait");
    assert_eq!(
        error
            .as_database_error()
            .and_then(|database| database.code())
            .as_deref(),
        Some("55P03"),
        "unexpected {what} lock result: {error}"
    );
}

async fn assert_expiry_after_unchanged_shard_wait(replaying: bool) {
    let case = if replaying { "replay" } else { "apply" };
    let name = format!("scoped-finish-shard-expiry-{case}");
    let (f, sources, pool) = certified_flow(&name, 1).await;
    let runtime_user: String = sqlx::query_scalar("SELECT current_user")
        .fetch_one(&pool)
        .await
        .expect("explicit scoped Finish runtime role");
    assert_eq!(runtime_user, "sagat_app");

    let seeded = seed_group(
        &f,
        vec![active_member(
            CapacityMemberId::generate(),
            sources[0].identity.clone(),
        )],
        10,
        4,
        Some(0),
    )
    .await;
    let mut command = finish_command(&sources[0], seeded, 10, 4);
    let logical_fingerprint = command.fingerprint().expect("valid exact logical command");
    let store = PgScopedLeafFinishStore::new(pool);
    let original = if replaying {
        let applied = store
            .finish_scoped_leaf(&command)
            .await
            .expect("create the exact original receipt before replay barrier");
        assert!(!applied.replayed);
        assert_linked_effect(&f, &command, &applied).await;
        Some(applied)
    } else {
        None
    };

    let source_before = source_execution_snapshot(&f).await;
    let capacity_before = capacity_snapshot(&f, seeded.group).await;
    let counts_before = finish_counts(&f, seeded.group).await;
    assert_eq!(counts_before, if replaying { (1, 1, 1) } else { (0, 0, 0) });

    // Exercise the real renewable current authority API. This deliberately shortens the fixture's
    // earlier ten-minute lease without changing owner or generation.
    let lease = command.scheduler.lease();
    let renewed = f
        .api
        .executions
        .renew(command.scheduler.owner(), &[lease], SHORT_LEASE)
        .await
        .expect("short current scheduler authority");
    assert_eq!(renewed, vec![lease]);

    // The owner locks, but does not update, the exact scheduler row. The native transaction has
    // already locked its original Run and selected G when it reaches the blocked FOR SHARE.
    let mut shard_gate = f
        .api
        .db
        .pool
        .begin()
        .await
        .expect("unchanged shard-row barrier");
    let expiry: OffsetDateTime = sqlx::query_scalar(
        "SELECT expires_at FROM scheduler_shards \
         WHERE shard=run_shard($1) FOR UPDATE",
    )
    .bind(f.run.as_uuid())
    .fetch_one(&mut *shard_gate)
    .await
    .expect("hold renewable shard tuple unchanged");
    let gate_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *shard_gate)
        .await
        .expect("shard gate backend");

    let mut pending = Box::pin(store.finish_scoped_leaf(&command));
    tokio::select! {
        result = pending.as_mut() => panic!("native Finish bypassed unchanged shard row: {result:?}"),
        waiter = timeout(
            SHARD_WAIT_DEADLINE,
            actual_shard_waiter(&f.api.db.pool, gate_pid),
        ) => {
            waiter.expect("native shard-row wait deadline");
        }
    }
    let still_live: bool = sqlx::query_scalar("SELECT clock_timestamp()<$1")
        .bind(expiry)
        .fetch_one(&f.api.db.pool)
        .await
        .expect("live-at-wait database clock");
    assert!(
        still_live,
        "the actual unchanged-row wait must begin before the committed lease expires"
    );

    assert_nowait_blocked(
        &f.api.db.pool,
        "SELECT id FROM runs WHERE tenant_id=$1 AND id=$2 FOR UPDATE NOWAIT",
        f.tenant.as_uuid(),
        f.run.as_uuid(),
        "original Run",
    )
    .await;
    assert_nowait_blocked(
        &f.api.db.pool,
        "SELECT group_id FROM capacity_groups \
         WHERE tenant_id=$1 AND group_id=$2 FOR UPDATE NOWAIT",
        f.tenant.as_uuid(),
        seeded.group.as_uuid(),
        "selected G",
    )
    .await;

    timeout(
        SHARD_WAIT_DEADLINE,
        database_clock_reaches(&f.api.db.pool, expiry),
    )
    .await
    .expect("database clock expiry deadline");
    shard_gate
        .rollback()
        .await
        .expect("release unchanged shard tuple after expiry");
    assert_eq!(
        timeout(SHARD_WAIT_DEADLINE, pending.as_mut())
            .await
            .expect("post-wait fence result deadline"),
        Err(ScopedLeafFinishError::SchedulerFenceLost)
    );
    drop(pending);

    assert_eq!(source_execution_snapshot(&f).await, source_before);
    assert_eq!(capacity_snapshot(&f, seeded.group).await, capacity_before);
    assert_eq!(finish_counts(&f, seeded.group).await, counts_before);

    // The expired generation is taken through the real shard API. Scheduler-row renewal/take is
    // outside the source and capacity snapshots; the exact logical command remains unchanged.
    let fresh_owner = format!("scoped-finish-shard-fresh-{case}");
    let acquired = f
        .api
        .executions
        .acquire(&fresh_owner, &[lease.shard], FRESH_LEASE)
        .await
        .expect("take expired shard as fresh current authority");
    assert_eq!(acquired.len(), 1);
    assert_eq!(acquired[0].shard, lease.shard);
    assert!(acquired[0].generation > lease.generation);
    command.scheduler = SchedulerAuthority::for_run(fresh_owner, f.run, acquired[0].generation);
    assert_eq!(
        command.fingerprint().expect("valid renewed command"),
        logical_fingerprint,
        "renewable scheduler authority is outside the exact logical-operation fingerprint"
    );

    let recovered = store
        .finish_scoped_leaf(&command)
        .await
        .expect("same exact command succeeds with fresh current authority");
    assert_eq!(recovered.replayed, replaying);
    if let Some(mut original) = original {
        let mut replay = recovered.clone();
        original.replayed = false;
        replay.replayed = false;
        assert_eq!(
            replay, original,
            "fresh authority returns the original receipt result"
        );
        assert_eq!(capacity_snapshot(&f, seeded.group).await, capacity_before);
    } else {
        assert_linked_effect(&f, &command, &recovered).await;
    }
    assert_eq!(finish_counts(&f, seeded.group).await, (1, 1, 1));
    assert_eq!(source_execution_snapshot(&f).await, source_before);
}

#[tokio::test]
async fn native_first_apply_revalidates_expiry_after_unchanged_shard_wait() {
    assert_expiry_after_unchanged_shard_wait(false).await;
}

#[tokio::test]
async fn native_exact_replay_revalidates_expiry_after_unchanged_shard_wait() {
    assert_expiry_after_unchanged_shard_wait(true).await;
}
