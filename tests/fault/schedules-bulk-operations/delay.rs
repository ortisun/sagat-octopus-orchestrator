//! The release of a delayed start dies at any write (`T018-037`, `SC-018-007`, `FR-018-023`): the
//! journal entry that releases the entry, the step it makes ready and the run row are one
//! transaction, so a process killed between two of its statements leaves the run held, exactly as it
//! was, and the next pass releases it once. The connection of the transaction is killed inside
//! PostgreSQL after each of its writing statements in turn (`tripwire.rs`). Real PostgreSQL, the real
//! scheduler.
#![allow(clippy::expect_used, clippy::panic)]

use std::collections::BTreeSet;
use std::time::Duration as StdDuration;

use sagat_application::runs::{StartOptions, StartRun};
use sagat_domain::execution::{RunId, RunStatus};
use time::{Duration, OffsetDateTime};

use crate::run_driven_locks::shard_owner;
use crate::support::engine::ctx;
use crate::support::schedules::{Fixture, fixture};
use crate::tripwire::{Tripwire, table_digests};

/// A delayed run whose instant has already passed by the database's clock.
pub(crate) async fn due_run(f: &Fixture, key: &str) -> RunId {
    f.clock
        .set(OffsetDateTime::now_utc() - Duration::seconds(5));
    f.w.runs
        .start_with_options(
            &f.w.admin,
            &StartRun {
                namespace: "payments",
                workflow: f.workflow,
                version: None,
                input: b"{}",
                correlation_id: None,
                deadline: None,
                key,
            },
            StartOptions {
                start_delay: Some(StdDuration::from_millis(1)),
                ..StartOptions::default()
            },
            &ctx(),
        )
        .await
        .expect("a delayed start")
        .run
        .info
        .run_id
}

async fn journal(f: &Fixture, run: RunId) -> Vec<String> {
    sqlx::query_scalar("SELECT event_type FROM run_events WHERE run_id = $1 ORDER BY seq")
        .bind(run.as_uuid())
        .fetch_all(&f.w.db.pool)
        .await
        .expect("the journal")
}

#[tokio::test]
async fn a_release_that_dies_at_any_write_leaves_the_run_held_and_the_next_pass_releases_it_once() {
    let f = fixture().await;
    let wire = Tripwire::install(&f.w.db.pool).await;
    let passes = shard_owner(&f).await;

    // A first start and release of the tenant make writes that later ones do not (its first counters).
    let warm = due_run(&f, "release-warm-up-key").await;
    passes.tick().await;
    assert_eq!(f.w.run_status(warm).await, RunStatus::Running);

    // Which writes does the release make? Watch a normal one.
    let probe = due_run(&f, "release-probe-key").await;
    wire.watch().await;
    let report = passes.tick().await;
    wire.disarm().await;
    assert_eq!(report.starts_released, 1, "{report:?}");
    let writes = wire.writes().await;
    let transactions: BTreeSet<i64> = writes.iter().map(|(xid, _, _)| *xid).collect();
    assert_eq!(
        transactions.len(),
        1,
        "a release is one transaction: {writes:?}"
    );
    let tables: BTreeSet<&str> = writes.iter().map(|(_, _, t)| t.as_str()).collect();
    for table in ["run_events", "steps", "runs"] {
        assert!(
            tables.contains(table),
            "the release is expected to write {table}: {writes:?}"
        );
    }
    assert_eq!(
        journal(&f, probe).await,
        [
            "run.started",
            "run.start_delayed",
            "run.start_released",
            "step.released"
        ]
    );
    println!(
        "a release makes {} writing statements in one transaction, on {} tables: {tables:?}",
        writes.len(),
        tables.len()
    );

    // Die after each of those writes, then let the next pass go on.
    for (_, position, table) in &writes {
        let run = due_run(&f, &format!("release-dies-at-{position}-key")).await;
        let before = journal(&f, run).await;
        let digests = table_digests(&f.w.db.pool).await;

        wire.arm(*position).await;
        let cut_off = passes.tick().await;
        wire.disarm().await;

        assert_eq!(
            cut_off.starts_released, 0,
            "cut off at write {position} ({table}), nothing was released: {cut_off:?}"
        );
        assert_eq!(
            table_digests(&f.w.db.pool).await,
            digests,
            "cut off at write {position} ({table}), a table changed"
        );
        assert_eq!(journal(&f, run).await, before, "write {position}");
        assert_eq!(f.w.run_status(run).await, RunStatus::Waiting);

        // the next pass releases it, once
        let mut released = 0;
        for _ in 0..4 {
            released += passes.tick().await.starts_released;
        }
        assert_eq!(released, 1, "write {position} ({table}): released once");
        assert_eq!(
            journal(&f, run).await,
            [
                "run.started",
                "run.start_delayed",
                "run.start_released",
                "step.released"
            ],
            "write {position}"
        );
        f.w.store
            .verify_run(f.w.admin.tenant_id(), run)
            .await
            .expect("the projection equals a replay of the journal");
    }
}
