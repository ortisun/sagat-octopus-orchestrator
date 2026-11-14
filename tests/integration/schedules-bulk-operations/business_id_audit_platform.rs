//! The business-id conflicts the platform meets on its own (`FR-018-044`, `G2086`, review of D2218): a batch `RERUN`
//! item that keeps the id of its run, and the completion of a reset that waited for its source. Each is the platform's
//! decision, not the person's: its record is by the engine and commits with the fenced record of the decision (the item,
//! the failed entry). A replica whose lease moves between the refusal and that record writes nothing, and the replica that
//! records the decision writes the one record. The lease is moved while the refused step waits for it, with held locks
//! seen in `pg_stat_activity`, never with a sleep.
#![allow(clippy::expect_used, clippy::panic)]

use std::time::Duration as StdDuration;

use sagat_application::batch::JobRecord;
use sagat_application::reset_completion::Completed;
use sagat_application::schedules::ShardFence;
use sagat_application::shards::Lease;
use sagat_domain::batch::{Action, Selector};
use sagat_domain::business_ids::{ConflictPolicy, ReusePolicy};
use sagat_domain::execution::RunId;
use sagat_domain::sharding::Shard;
use sqlx::PgPool;
use tokio::sync::oneshot;
use uuid::Uuid;

use crate::batch_executor::{create, due, executor, items, live, select, write};
use crate::business_ids::{business, cancel, key, start};
use crate::cancel_source::{asking, completer, due_of, request};
use crate::firing::Fixture;
use crate::support::bounded::within;

/// `(actor, resource type, resource id, reason)` of every `business_id.conflict` record, oldest first.
async fn conflicts(f: &Fixture) -> Vec<(String, String, String, Option<String>)> {
    sqlx::query_as(
        "SELECT actor, resource_type, resource_id, reason FROM audit_records \
         WHERE action = 'business_id.conflict' ORDER BY created_at, id",
    )
    .fetch_all(&f.w.db.pool)
    .await
    .expect("the audit trail")
}

/// A transaction that holds the row of the binding of `id`, and the id of its backend.
async fn hold_binding(f: &Fixture, id: &str) -> (sqlx::Transaction<'static, sqlx::Postgres>, i32) {
    let mut held = f.w.db.pool.begin().await.expect("begin");
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *held)
        .await
        .expect("pid");
    sqlx::query("SELECT 1 FROM business_ids WHERE business_id = $1 FOR UPDATE")
        .bind(id)
        .execute(&mut *held)
        .await
        .expect("hold the binding");
    (held, pid)
}

/// The backend that waits for a lock `holder` holds while running a statement on `table`.
async fn blocked_on(f: &Fixture, holder: i32, table: &str, stage: &str) -> i32 {
    within(stage, StdDuration::from_secs(120), async {
        loop {
            let pid: Option<i32> = sqlx::query_scalar(
                "SELECT pid FROM pg_stat_activity \
                 WHERE $1 = ANY(pg_blocking_pids(pid)) AND wait_event_type = 'Lock' \
                   AND strpos(query, $2) > 0 \
                 LIMIT 1",
            )
            .bind(holder)
            .bind(table)
            .fetch_optional(&f.w.db.pool)
            .await
            .expect("look at the backends");
            if let Some(pid) = pid {
                return pid;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
}

/// Gives `shard` to `owner` at `generation`, for an hour.
async fn lease(pool: &PgPool, shard: Shard, owner: &str, generation: i64) {
    sqlx::query(
        "UPDATE scheduler_shards SET owner = $2, generation = $3, \
         expires_at = clock_timestamp() + interval '1 hour' WHERE shard = $1",
    )
    .bind(i16::from(shard.index()))
    .bind(owner)
    .bind(generation)
    .execute(pool)
    .await
    .expect("the lease");
}

fn fence(shard: Shard, owner: &str, generation: u64) -> ShardFence {
    ShardFence {
        owner: owner.to_owned(),
        lease: Lease { shard, generation },
    }
}

/// Another replica takes `shard` over in a transaction that stays open until `release` is sent: the takeover waits for
/// what holds the row in shared mode, and then holds it itself, so a fence read after it waits for its commit.
async fn takeover(
    f: &Fixture,
    shard: Shard,
) -> (i32, oneshot::Sender<()>, tokio::task::JoinHandle<()>) {
    let (pid_tx, pid_rx) = oneshot::channel();
    let (release, released) = oneshot::channel::<()>();
    let pool = f.w.db.pool.clone();
    let task = tokio::spawn(async move {
        let mut tx = pool.begin().await.expect("begin");
        let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *tx)
            .await
            .expect("pid");
        pid_tx.send(pid).expect("send the pid");
        sqlx::query(
            "UPDATE scheduler_shards SET owner = 'replica-new', generation = 12, \
             expires_at = clock_timestamp() + interval '1 hour' WHERE shard = $1",
        )
        .bind(i16::from(shard.index()))
        .execute(&mut *tx)
        .await
        .expect("take the shard over");
        released.await.expect("the signal to commit");
        tx.commit().await.expect("commit the takeover");
    });
    (pid_rx.await.expect("the pid"), release, task)
}

/// A run that held `id` and ended, and an open run that holds it now.
async fn source_and_holder(f: &Fixture, id: &str) -> (RunId, RunId) {
    let source = start(
        f,
        business(id, ConflictPolicy::Fail, ReusePolicy::AllowDuplicate),
        &key(),
    )
    .await
    .expect("the source")
    .run
    .info
    .run_id;
    cancel(f, source).await;
    let holder = start(
        f,
        business(id, ConflictPolicy::Fail, ReusePolicy::AllowDuplicate),
        &key(),
    )
    .await
    .expect("a later start takes the id")
    .run
    .info
    .run_id;
    (source, holder)
}

#[tokio::test]
async fn a_rerun_item_that_finds_its_id_held_is_audited_once_by_the_engine_with_its_record() {
    let f = crate::firing::fixture().await;
    let id = format!("order-{}", Uuid::now_v7());
    let (source, _) = source_and_holder(&f, &id).await;
    live(&f);
    let exec = executor(&f);
    let job: JobRecord = create(
        &f,
        write(
            Action::Rerun,
            Selector::run_ids(vec![source.as_uuid()]).expect("selector"),
        ),
        "conflict-rerun-item",
    )
    .await;
    select(&exec, &job).await;
    let shard = Shard::of(job.id);
    lease(&f.w.db.pool, shard, "replica-old", 11).await;
    f.clock.advance(time::Duration::seconds(1));

    // the old replica's rerun waits for the binding; the shard changes hands; the refusal then waits for the fence
    let (binding, binder) = hold_binding(&f, &id).await;
    let step = tokio::spawn({
        let due = due(&job);
        let stale = fence(shard, "replica-old", 11);
        let exec = executor(&f);
        async move { exec.step(&due, Some(stale)).await }
    });
    blocked_on(
        &f,
        binder,
        "business_ids",
        "the rerun waits for the binding",
    )
    .await;
    let (mover, release, moved) = takeover(&f, shard).await;
    binding.commit().await.expect("release the binding");
    blocked_on(
        &f,
        mover,
        "scheduler_shards",
        "the record of the refusal waits for the fence",
    )
    .await;
    release.send(()).expect("commit the takeover");
    moved.await.expect("the takeover");
    let refused = step.await.expect("join").expect("step");

    assert!(
        matches!(
            refused,
            sagat_application::batch_executor::Stepped::Refused(_)
        ),
        "{refused:?}"
    );
    assert_eq!(items(&f, job.id).await[0].1, "PENDING");
    assert!(
        conflicts(&f).await.is_empty(),
        "a refusal whose record was refused leaves no record: {:?}",
        conflicts(&f).await
    );

    // the new owner decides it, once, with its record
    f.clock.advance(time::Duration::seconds(1));
    exec.step(&due(&job), Some(fence(shard, "replica-new", 12)))
        .await
        .expect("the owner");
    let row = &items(&f, job.id).await[0];
    assert_eq!(
        (row.1.as_str(), row.3.as_deref()),
        ("FAILED", Some("CONFLICT"))
    );
    assert_eq!(
        conflicts(&f).await,
        [(
            "system:engine".to_owned(),
            "run".to_owned(),
            source.to_string(),
            Some(format!("BUSINESS_ID_IN_USE batch {}", job.id)),
        )],
        "one record, by the engine, on the run the item reruns, with the closed reason and the job"
    );
}

#[tokio::test]
async fn a_reset_completion_that_finds_its_id_held_is_audited_once_by_the_engine_with_the_failed_entry()
 {
    let f = crate::firing::fixture().await;
    let id = format!("order-{}", Uuid::now_v7());
    let source = start(
        &f,
        business(&id, ConflictPolicy::Fail, ReusePolicy::AllowDuplicate),
        &key(),
    )
    .await
    .expect("the source")
    .run
    .info
    .run_id;
    let made = request(&f, source, asking(1, true), &key())
        .await
        .expect("the request");
    assert!(f.w.run_status(source).await.is_terminal());
    start(
        &f,
        business(&id, ConflictPolicy::Fail, ReusePolicy::AllowDuplicate),
        &key(),
    )
    .await
    .expect("a later start takes the id");
    let shard = Shard::of(made.reset.id.as_uuid());
    lease(&f.w.db.pool, shard, "replica-old", 11).await;

    let (binding, binder) = hold_binding(&f, &id).await;
    let step = tokio::spawn({
        let completer = completer(&f);
        let due = due_of(&f, made.reset.id);
        let stale = fence(shard, "replica-old", 11);
        async move { completer.step(&due, Some(stale)).await }
    });
    blocked_on(
        &f,
        binder,
        "business_ids",
        "the completion waits for the binding",
    )
    .await;
    let (mover, release, moved) = takeover(&f, shard).await;
    binding.commit().await.expect("release the binding");
    blocked_on(
        &f,
        mover,
        "scheduler_shards",
        "the failure of the entry waits for the fence",
    )
    .await;
    release.send(()).expect("commit the takeover");
    moved.await.expect("the takeover");
    let refused = step.await.expect("join").expect("step");

    assert!(matches!(refused, Completed::Refused(_)), "{refused:?}");
    assert!(
        conflicts(&f).await.is_empty(),
        "a refusal whose record was refused leaves no record: {:?}",
        conflicts(&f).await
    );

    let failed = completer(&f)
        .step(
            &due_of(&f, made.reset.id),
            Some(fence(shard, "replica-new", 12)),
        )
        .await
        .expect("the owner");
    assert_eq!(failed, Completed::Failed("BUSINESS_ID_IN_USE"));
    assert_eq!(
        conflicts(&f).await,
        [(
            "system:engine".to_owned(),
            "run".to_owned(),
            source.to_string(),
            Some(format!("BUSINESS_ID_IN_USE reset {}", made.reset.id)),
        )],
        "one record, by the engine, on the source, with the closed reason and the entry"
    );
}
