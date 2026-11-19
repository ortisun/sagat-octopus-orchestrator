//! The purge of batch jobs dies at any write (`T018-051`, the purge half; `G2317`, `FR-018-033`).
//!
//! **What these tests show, and what they do not.** A call of the purge is one statement, so PostgreSQL gives it atomicity whatever
//! the function says: a connection killed in the middle of it leaves the tables as they were before it, and no test could make that
//! false. What they pin is what the function and the pass add to that. (1) *Every write the function makes is inside the call:* the
//! connection is killed after each of the writing statements of a call in turn (`tripwire.rs`: the items, the job, and what the
//! triggers of the job write, the charge of its content among them), and after each the tables are equal to what they were, so no write
//! of the function is committed apart from the rest (an autonomous commit, a second connection) and none escapes the call. (2) *The
//! state between two calls is a state a reader and the next call can use:* a call that the budget cut short leaves a job that is
//! finished, with the items it has left and no item without its job; the same is shown when the call after it dies. (3) *The pass goes
//! on:* a pass whose second call dies leaves the job as its first call left it, counts the failure, and the next pass finishes the
//! job. The mutants of `verification.md` (q) that these kill are logic (M3 never ends a job, M10 ends one without the room for it), not
//! atomicity. Real PostgreSQL, the function the runtime role runs.
#![allow(clippy::expect_used, clippy::panic)]

use std::collections::BTreeSet;

use sagat_application::maintenance::{BatchPurged, MaintenanceStore};
use sagat_scheduler::BATCH_PURGE_ROWS;
use uuid::Uuid;

use crate::support::schedules::{Fixture, fixture};
use crate::tripwire::{Tripwire, table_digests};

/// A job of the first tenant that finished long ago and whose window ended ten days ago, with `items` items.
async fn over(f: &Fixture, items: i32) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO batch_jobs \
         (tenant_id, namespace_id, id, action, selector, reason, created_by, state, max_items, rate_milli, selected, \
          selection_bound_at, selection_horizon, next_slot_at, created_at, finished_at, expires_at) \
         VALUES ($1, $2, $3, 'CANCEL', '{\"filter\": {}}', 'stop the runs that are stuck', 'alice', 'COMPLETED', 100000, \
                 20000, $4, clock_timestamp() - interval '40 days', pg_current_xact_id(), clock_timestamp(), \
                 clock_timestamp() - interval '40 days', clock_timestamp() - interval '39 days', \
                 clock_timestamp() - interval '10 days')",
    )
    .bind(f.w.admin.tenant_id().as_uuid())
    .bind(f.w.namespace.id().as_uuid())
    .bind(id)
    .bind(items)
    .execute(&f.w.db.pool)
    .await
    .expect("seed the job");
    sqlx::query(
        "INSERT INTO batch_items (tenant_id, job_id, ordinal, run_id, state, updated_at) \
         SELECT $1, $2, g, gen_random_uuid(), 'SUCCEEDED', clock_timestamp() FROM generate_series(1, $3) g",
    )
    .bind(f.w.admin.tenant_id().as_uuid())
    .bind(id)
    .bind(items)
    .execute(&f.w.db.pool)
    .await
    .expect("seed the items");
    id
}

async fn items_of(f: &Fixture, id: Uuid) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM batch_items WHERE job_id = $1")
        .bind(id)
        .fetch_one(&f.w.db.pool)
        .await
        .expect("items")
}

async fn rows(f: &Fixture) -> (i64, i64) {
    let jobs = sqlx::query_scalar("SELECT count(*) FROM batch_jobs")
        .fetch_one(&f.w.db.pool)
        .await
        .expect("jobs");
    let items = sqlx::query_scalar("SELECT count(*) FROM batch_items")
        .fetch_one(&f.w.db.pool)
        .await
        .expect("items");
    (jobs, items)
}

/// What no cut may leave: an item whose job is gone, and a job that is not in a state in which it ended (every job of this suite
/// has ended: a job a cut left without its state would be one the purge changed).
async fn whole(f: &Fixture, what: &str) {
    let orphans: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM batch_items i WHERE NOT EXISTS \
         (SELECT 1 FROM batch_jobs j WHERE j.tenant_id = i.tenant_id AND j.id = i.job_id)",
    )
    .fetch_one(&f.w.db.pool)
    .await
    .expect("orphans");
    assert_eq!(orphans, 0, "{what}: an item without its job");
    let unfinished: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM batch_jobs WHERE state <> 'COMPLETED' OR finished_at IS NULL",
    )
    .fetch_one(&f.w.db.pool)
    .await
    .expect("states");
    assert_eq!(unfinished, 0, "{what}: a job that lost its end");
}

async fn purge(
    f: &Fixture,
    rows: u32,
) -> Result<BatchPurged, sagat_application::error::StoreError> {
    f.w.store.purge_batch_jobs(10, rows).await
}

/// The positions and tables of the writing statements of the calls that ran while the wire was watching.
async fn writes_of(wire: &Tripwire) -> Vec<(usize, String)> {
    let writes = wire.writes().await;
    let transactions: BTreeSet<i64> = writes.iter().map(|(xid, _, _)| *xid).collect();
    assert_eq!(
        transactions.len(),
        1,
        "a call of the purge is one transaction: {writes:?}"
    );
    writes
        .into_iter()
        .map(|(_, position, table)| (position, table))
        .collect()
}

/// A call that takes a job whole (its items and its row, with the charge of what the job held given back) is cut after each write:
/// the tables are as they were, and the call made again removes the job and its items.
#[tokio::test]
async fn a_purge_that_dies_at_any_write_changes_nothing_and_made_again_removes_the_job() {
    let f = fixture().await;
    let wire = Tripwire::install(&f.w.db.pool).await;

    // which writes does a call make? watch a normal one
    over(&f, 3).await;
    wire.watch().await;
    assert_eq!(
        purge(&f, 1_000).await.expect("the probe call"),
        BatchPurged { jobs: 1, items: 3 }
    );
    wire.disarm().await;
    let writes = writes_of(&wire).await;
    let tables: BTreeSet<&str> = writes.iter().map(|(_, t)| t.as_str()).collect();
    for table in ["batch_items", "batch_jobs"] {
        assert!(
            tables.contains(table),
            "the call is expected to write {table}: {writes:?}"
        );
    }
    println!(
        "a call of the purge that takes a job whole makes {} writing statements in one transaction, on {} tables: {tables:?}",
        writes.len(),
        tables.len()
    );

    for (position, table) in &writes {
        let id = over(&f, 3).await;
        let digests = table_digests(&f.w.db.pool).await;

        wire.arm(*position).await;
        let result = purge(&f, 1_000).await;
        wire.disarm().await;

        assert!(
            result.is_err(),
            "cut off at write {position} ({table}), the call answered {result:?}"
        );
        assert_eq!(
            table_digests(&f.w.db.pool).await,
            digests,
            "cut off at write {position} ({table}), a table changed"
        );
        whole(&f, &format!("write {position} ({table})")).await;
        assert_eq!(items_of(&f, id).await, 3, "write {position}");

        assert_eq!(
            purge(&f, 1_000)
                .await
                .unwrap_or_else(|e| panic!("write {position} ({table}), made again: {e:?}")),
            BatchPurged { jobs: 1, items: 3 },
            "write {position} ({table}): the call made again does the work"
        );
        assert_eq!(rows(&f).await, (0, 0), "write {position}");
    }
    assert!(wire.kills().await >= i64::try_from(writes.len()).expect("count"));
}

/// A job the call before it cut short (two items of five left, and its row) is purged by a call that is cut after each of its writes:
/// the job is as the first call left it (finished, with the two items), never an item without its job, and the call made again takes
/// what is left and the row.
#[tokio::test]
async fn a_purge_that_dies_after_a_call_cut_short_the_job_leaves_it_as_that_call_did_and_made_again_finishes_it()
 {
    let f = fixture().await;
    let wire = Tripwire::install(&f.w.db.pool).await;

    // the writes of the call that finishes a job: watch one
    let probe = over(&f, 5).await;
    assert_eq!(
        purge(&f, 3).await.expect("the first call"),
        BatchPurged { jobs: 0, items: 3 }
    );
    wire.watch().await;
    assert_eq!(
        purge(&f, 3).await.expect("the probe call"),
        BatchPurged { jobs: 1, items: 2 }
    );
    wire.disarm().await;
    assert_eq!(items_of(&f, probe).await, 0);
    let writes = writes_of(&wire).await;
    println!(
        "the call that finishes a job makes {} writing statements: {writes:?}",
        writes.len()
    );

    for (position, table) in &writes {
        let id = over(&f, 5).await;
        assert_eq!(
            purge(&f, 3).await.expect("the first call"),
            BatchPurged { jobs: 0, items: 3 }
        );
        assert_eq!(items_of(&f, id).await, 2);
        let digests = table_digests(&f.w.db.pool).await;

        wire.arm(*position).await;
        let result = purge(&f, 3).await;
        wire.disarm().await;

        assert!(
            result.is_err(),
            "cut off at write {position} ({table}), the call answered {result:?}"
        );
        assert_eq!(
            table_digests(&f.w.db.pool).await,
            digests,
            "cut off at write {position} ({table}), a table changed"
        );
        whole(&f, &format!("write {position} ({table})")).await;
        assert_eq!(
            items_of(&f, id).await,
            2,
            "write {position}: the job has the items the first call left"
        );

        assert_eq!(
            purge(&f, 3)
                .await
                .unwrap_or_else(|e| panic!("write {position} ({table}), made again: {e:?}")),
            BatchPurged { jobs: 1, items: 2 },
            "write {position} ({table}): the call made again finishes the job"
        );
        assert_eq!(rows(&f).await, (0, 0), "write {position}");
    }
}

/// The pass: a job of one call's rows and a thousand more. The first call of the pass takes 5,000 items and commits; the second would take
/// the thousand and the job, and its connection is killed at its write of the job (the first transaction whose fourth writing statement
/// writes `batch_jobs`: the first call writes only items). The pass counts the failure and goes on with the rest of the housekeeping; the
/// job is as the first call left it (finished, a thousand items, none orphaned), and the next pass finishes it. Catches: a pass that
/// treats the failure of a call as the end of the job (it would be lost), and a state between calls that a read or the next call cannot use.
#[tokio::test]
async fn a_pass_whose_second_call_dies_leaves_the_job_as_the_first_call_left_it_and_the_next_pass_finishes_it()
 {
    let f = fixture().await;
    let wire = Tripwire::install(&f.w.db.pool).await;
    let id = over(&f, i32::try_from(BATCH_PURGE_ROWS).expect("rows") + 1_000).await;

    wire.arm_on("batch_jobs", 4).await;
    f.w.scheduler.housekeeping().await;
    wire.disarm().await;

    assert_eq!(wire.kills().await, 1, "the second call was killed, once");
    assert_eq!(
        items_of(&f, id).await,
        1_000,
        "the first call committed its 5,000 items and the second nothing"
    );
    whole(&f, "after the pass whose second call died").await;
    assert_eq!(rows(&f).await, (1, 1_000));

    f.w.scheduler.housekeeping().await;

    assert_eq!(rows(&f).await, (0, 0), "the next pass finishes the job");
}
