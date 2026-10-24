//! Bounded entry-barrier controls for dormant flat OrdinaryLeafV1 Finish.
//!
//! Install as a child of `scoped_leaf_finish_native.rs`. These tests exercise only the
//! existing native adapter's entry locks, rollback and exact-command serialization. They do not
//! establish a production caller, non-flat source closure or broader capacity lifecycle support.

use super::*;
use crate::support::scope_flow::RunHold;
use tokio::time::timeout;

const ENTRY_WAIT: Duration = Duration::from_secs(10);
const RUN_ENTRY: &str =
    "SELECT workflow_version_id,run_version,parent_run_id IS NULL AS parentless";

async fn assert_native_role(pool: &PgPool) {
    let user: String = sqlx::query_scalar("SELECT current_user")
        .fetch_one(pool)
        .await
        .expect("explicit scoped Finish runtime role");
    assert_eq!(user, "sagat_app");
}

async fn direct_run_waiter(pool: &PgPool) -> i32 {
    loop {
        let waiter: Option<i32> = sqlx::query_scalar(
            "SELECT leader.pid FROM pg_stat_activity leader \
             WHERE leader.datname=current_database() AND leader.usename='sagat_app' \
             AND leader.wait_event_type='Lock' AND position($1 in leader.query)>0 \
             AND EXISTS(SELECT 1 FROM pg_stat_activity holder \
                 WHERE holder.datname=leader.datname \
                 AND holder.pid=ANY(pg_blocking_pids(leader.pid)) \
                 AND position('SELECT 1 FROM runs' in holder.query)>0) \
             LIMIT 1",
        )
        .bind(RUN_ENTRY)
        .fetch_optional(pool)
        .await
        .expect("actual original-Run waiter observation");
        if let Some(waiter) = waiter {
            return waiter;
        }
        tokio::task::yield_now().await;
    }
}

async fn two_run_waiters_reach_the_same_holder(pool: &PgPool) {
    loop {
        // pg_blocking_pids reports both hard blockers and lock-queue soft blockers. Follow that
        // graph so the second exact command may queue behind the first without requiring both
        // native sessions to name RunHold as their immediate blocker.
        let observed: bool = sqlx::query_scalar(
            "WITH RECURSIVE run_waiters AS ( \
                 SELECT pid FROM pg_stat_activity \
                 WHERE datname=current_database() AND usename='sagat_app' \
                 AND wait_event_type='Lock' AND position($1 in query)>0 \
             ), reach(waiter,blocker) AS ( \
                 SELECT waiter.pid,blockers.blocker FROM run_waiters waiter \
                 CROSS JOIN LATERAL unnest(pg_blocking_pids(waiter.pid)) blockers(blocker) \
                 UNION \
                 SELECT reach.waiter,blockers.blocker FROM reach \
                 JOIN pg_stat_activity blocked ON blocked.pid=reach.blocker \
                 CROSS JOIN LATERAL unnest(pg_blocking_pids(blocked.pid)) blockers(blocker) \
             ), rooted AS ( \
                 SELECT DISTINCT reach.waiter FROM reach \
                 JOIN pg_stat_activity holder ON holder.pid=reach.blocker \
                 WHERE holder.datname=current_database() \
                 AND position('SELECT 1 FROM runs' in holder.query)>0 \
             ) SELECT (SELECT count(*) FROM run_waiters)=2 \
                 AND (SELECT count(*) FROM rooted)=2",
        )
        .bind(RUN_ENTRY)
        .fetch_one(pool)
        .await
        .expect("actual duplicate original-Run blocking graph");
        if observed {
            return;
        }
        tokio::task::yield_now().await;
    }
}

async fn native_chain_waiter(pool: &PgPool, gate_pid: i32) {
    loop {
        let observed: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM pg_stat_activity waiter \
             WHERE waiter.datname=current_database() AND waiter.usename='sagat_app' \
             AND waiter.wait_event_type='Lock' \
             AND $1=ANY(pg_blocking_pids(waiter.pid)) \
             AND position('chain_segments' in waiter.query)>0 \
             AND position('execution_chains' in waiter.query)>0)",
        )
        .bind(gate_pid)
        .fetch_one(pool)
        .await
        .expect("actual applicable-chain waiter observation");
        if observed {
            return;
        }
        tokio::task::yield_now().await;
    }
}

async fn assert_select_denied(pool: &PgPool, table: &str, statement: &'static str) {
    let error = sqlx::query(statement)
        .fetch_optional(pool)
        .await
        .expect_err("runtime-role SELECT must be denied by the context-order fixture");
    assert_eq!(
        error
            .as_database_error()
            .and_then(|database| database.code())
            .as_deref(),
        Some("42501"),
        "unexpected {table} denial: {error}"
    );
}

#[tokio::test]
async fn selected_group_busy_follows_actual_run_entry_and_rolls_back_every_effect() {
    let (f, sources, pool) = certified_flow("scoped-finish-entry-group-busy", 1).await;
    assert_native_role(&pool).await;
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
    let command = finish_command(&sources[0], seeded, 10, 4);
    let source_before = source_execution_snapshot(&f).await;
    let capacity_before = capacity_snapshot(&f, seeded.group).await;

    let mut group_gate = f.api.db.pool.begin().await.expect("selected-G barrier");
    sqlx::query(
        "SELECT group_id FROM capacity_groups \
         WHERE tenant_id=$1 AND group_id=$2 FOR UPDATE",
    )
    .bind(f.tenant.as_uuid())
    .bind(seeded.group.as_uuid())
    .execute(&mut *group_gate)
    .await
    .expect("hold selected G unchanged");
    let run_hold = RunHold::take(&f).await;

    let store = PgScopedLeafFinishStore::new(pool.clone());
    let mut pending = Box::pin(store.finish_scoped_leaf(&command));
    tokio::select! {
        result = pending.as_mut() => panic!("native Finish bypassed held original Run: {result:?}"),
        waiter = timeout(ENTRY_WAIT, direct_run_waiter(&f.api.db.pool)) => {
            waiter.expect("native original-Run entry witness deadline");
        }
    }
    run_hold.release().await;
    assert_eq!(
        timeout(ENTRY_WAIT, pending.as_mut())
            .await
            .expect("selected-G NOWAIT result deadline"),
        Err(ScopedLeafFinishError::Busy)
    );
    drop(pending);

    assert_eq!(source_execution_snapshot(&f).await, source_before);
    assert_eq!(capacity_snapshot(&f, seeded.group).await, capacity_before);
    assert_eq!(finish_counts(&f, seeded.group).await, (0, 0, 0));

    let mut run_probe = f.api.db.pool.begin().await.expect("post-Busy run probe");
    sqlx::query(
        "SELECT id FROM runs WHERE tenant_id=$1 AND namespace_id=$2 AND id=$3 \
         FOR UPDATE NOWAIT",
    )
    .bind(f.tenant.as_uuid())
    .bind(sources[0].identity.namespace_id().as_uuid())
    .bind(f.run.as_uuid())
    .execute(&mut *run_probe)
    .await
    .expect("Busy rollback released the original Run while G remains held");
    run_probe.rollback().await.expect("run probe ends");

    group_gate.rollback().await.expect("release selected G");
    let applied = PgScopedLeafFinishStore::new(pool)
        .finish_scoped_leaf(&command)
        .await
        .expect("unchanged command succeeds after G release");
    assert!(!applied.replayed);
    assert_linked_effect(&f, &command, &applied).await;
    assert_eq!(finish_counts(&f, seeded.group).await, (1, 1, 1));
    assert_eq!(source_execution_snapshot(&f).await, source_before);
}

#[tokio::test]
async fn duplicate_native_callers_queue_on_the_actual_run_and_share_one_original_result() {
    let (f, sources, pool) = certified_flow("scoped-finish-entry-duplicate", 1).await;
    assert_native_role(&pool).await;
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
    let command = finish_command(&sources[0], seeded, 10, 4);
    let source_before = source_execution_snapshot(&f).await;
    let run_hold = RunHold::take(&f).await;

    let first_store = PgScopedLeafFinishStore::new(pool.clone());
    let second_store = PgScopedLeafFinishStore::new(pool);
    let mut first = Box::pin(first_store.finish_scoped_leaf(&command));
    let mut second = Box::pin(second_store.finish_scoped_leaf(&command));

    tokio::select! {
        result = first.as_mut() => panic!("first exact command bypassed held original Run: {result:?}"),
        waiter = timeout(ENTRY_WAIT, direct_run_waiter(&f.api.db.pool)) => {
            waiter.expect("first exact-command Run waiter deadline");
        }
    }
    tokio::select! {
        result = first.as_mut() => panic!("first exact command completed under RunHold: {result:?}"),
        result = second.as_mut() => panic!("second exact command bypassed held original Run: {result:?}"),
        observed = timeout(ENTRY_WAIT, two_run_waiters_reach_the_same_holder(&f.api.db.pool)) => {
            observed.expect("two-call Run blocking graph deadline");
        }
    }
    assert_eq!(finish_counts(&f, seeded.group).await, (0, 0, 0));

    run_hold.release().await;
    let (first_result, second_result) = timeout(ENTRY_WAIT, async {
        tokio::join!(first.as_mut(), second.as_mut())
    })
    .await
    .expect("both exact commands finish after Run release");
    let mut first_result = first_result.expect("first exact-command result");
    let mut second_result = second_result.expect("second exact-command result");
    assert_ne!(
        first_result.replayed, second_result.replayed,
        "one caller applies and the other replays"
    );
    let applied = if first_result.replayed {
        second_result.clone()
    } else {
        first_result.clone()
    };
    first_result.replayed = false;
    second_result.replayed = false;
    assert_eq!(
        first_result, second_result,
        "both callers receive the exact original result"
    );
    assert_linked_effect(&f, &command, &applied).await;
    assert_eq!(finish_counts(&f, seeded.group).await, (1, 1, 1));
    assert_eq!(source_execution_snapshot(&f).await, source_before);
}

#[tokio::test]
async fn applicable_chain_precedes_run_and_context_refusal_precedes_source_or_receipt_reads() {
    let (f, sources, pool) = certified_flow("scoped-finish-entry-chain-context", 1).await;
    assert_native_role(&pool).await;
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
    let command = finish_command(&sources[0], seeded, 10, 4);

    // Genuine flat source certification is complete before this labelled owner-only context
    // fixture. The membership proves only the entry prefix and subsequent context refusal; it is
    // not evidence that the earlier certificate closes a chain or another non-flat source.
    seed_flat_chain_membership(&f).await;
    let source_before = source_execution_snapshot(&f).await;
    let capacity_before = capacity_snapshot(&f, seeded.group).await;

    sqlx::query(
        "REVOKE SELECT ON leaf_collection_success_certifications,\
         capacity_group_finish_receipts FROM sagat_app",
    )
    .execute(&f.api.db.pool)
    .await
    .expect("test-only context-before-read restrictions");
    let privileges: (bool, bool) = sqlx::query_as(
        "SELECT \
         has_table_privilege(current_user,'leaf_collection_success_certifications','SELECT'),\
         has_table_privilege(current_user,'capacity_group_finish_receipts','SELECT')",
    )
    .fetch_one(&pool)
    .await
    .expect("effective context fixture privileges");
    assert_eq!(privileges, (false, false));
    assert_select_denied(
        &pool,
        "leaf_collection_success_certifications",
        "SELECT * FROM leaf_collection_success_certifications LIMIT 0",
    )
    .await;
    assert_select_denied(
        &pool,
        "capacity_group_finish_receipts",
        "SELECT * FROM capacity_group_finish_receipts LIMIT 0",
    )
    .await;

    // Current authority is consumed before flat-context and source/receipt reads.
    let mut stale = command.clone();
    stale.scheduler = SchedulerAuthority::for_run(
        "stale-scoped-entry-owner",
        f.run,
        sources[0].scheduler.lease().generation,
    );
    assert_eq!(
        PgScopedLeafFinishStore::new(pool.clone())
            .finish_scoped_leaf(&stale)
            .await,
        Err(ScopedLeafFinishError::SchedulerFenceLost)
    );
    assert_eq!(source_execution_snapshot(&f).await, source_before);
    assert_eq!(capacity_snapshot(&f, seeded.group).await, capacity_before);

    let mut chain_gate = f
        .api
        .db
        .pool
        .begin()
        .await
        .expect("applicable-chain barrier");
    let gate_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *chain_gate)
        .await
        .expect("chain gate backend");
    sqlx::query(
        "SELECT chain_id FROM execution_chains \
         WHERE tenant_id=$1 AND namespace_id=$2 AND chain_id=$3 FOR UPDATE",
    )
    .bind(f.tenant.as_uuid())
    .bind(sources[0].identity.namespace_id().as_uuid())
    .bind(f.run.as_uuid())
    .execute(&mut *chain_gate)
    .await
    .expect("hold explicitly labelled member chain");

    let store = PgScopedLeafFinishStore::new(pool);
    let mut pending = Box::pin(store.finish_scoped_leaf(&command));
    tokio::select! {
        result = pending.as_mut() => panic!("native Finish bypassed applicable chain: {result:?}"),
        observed = timeout(ENTRY_WAIT, native_chain_waiter(&f.api.db.pool, gate_pid)) => {
            observed.expect("native applicable-chain wait deadline");
        }
    }
    let mut run_probe = f.api.db.pool.begin().await.expect("chain-prefix Run probe");
    sqlx::query(
        "SELECT id FROM runs WHERE tenant_id=$1 AND namespace_id=$2 AND id=$3 \
         FOR UPDATE NOWAIT",
    )
    .bind(f.tenant.as_uuid())
    .bind(sources[0].identity.namespace_id().as_uuid())
    .bind(f.run.as_uuid())
    .execute(&mut *run_probe)
    .await
    .expect("chain waiter has not entered its original Run");
    run_probe
        .rollback()
        .await
        .expect("chain-prefix Run probe ends");

    chain_gate
        .rollback()
        .await
        .expect("release applicable chain");
    assert_eq!(
        timeout(ENTRY_WAIT, pending.as_mut())
            .await
            .expect("context refusal deadline"),
        Err(ScopedLeafFinishError::UnsupportedRunContext),
        "context refusal must occur without either denied SELECT"
    );
    assert_eq!(source_execution_snapshot(&f).await, source_before);
    assert_eq!(capacity_snapshot(&f, seeded.group).await, capacity_before);
    assert_eq!(finish_counts(&f, seeded.group).await, (0, 0, 0));
}
