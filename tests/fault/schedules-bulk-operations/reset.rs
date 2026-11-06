//! A reset dies at any write (`T018-055`, `SC-018-011`, `FR-018-037`): the run, the events of the prefix it
//! restores, the steps and outputs they project, the `run.reset` row that settles it, the entry of the
//! register, the idempotency receipt, the audit record and the outbox notification are one transaction, so a
//! process killed between two of its statements leaves nothing behind (not a run without its history, not an
//! entry without its run), and the same request sent again, which is what a client that lost the answer does,
//! makes exactly one run and one entry. The connection of the transaction is killed inside PostgreSQL after
//! each of its writing statements in turn (`tripwire.rs`). Real PostgreSQL, the real run service.
#![allow(clippy::expect_used, clippy::panic)]

use std::collections::BTreeSet;

use sagat_application::error::AppError;
use sagat_application::runs::{RerunBusinessId, ResetMade, ResetRequest};
use sagat_domain::execution::{ResetReapply, RunId, RunStatus};
use serde_json::json;
use uuid::Uuid;

use crate::support::engine::{activity, ctx, document, with};
use crate::support::schedules::{Fixture, fixture};
use crate::tripwire::{Tripwire, table_digests};

/// A run of three activities, one after the other, that a worker completed.
async fn ended_chain(f: &Fixture) -> RunId {
    let workflow =
        f.w.publish(document(
            &format!("reset-fault-{}", Uuid::now_v7()),
            "a",
            vec![
                with(activity("a"), "next", json!("b")),
                with(activity("b"), "next", json!("c")),
                activity("c"),
            ],
        ))
        .await
        .0;
    let run = f.w.start(workflow, &json!({"k": 1})).await;
    let session = f.w.session(&["act.a", "act.b", "act.c"]).await;
    for node in ["a", "b", "c"] {
        let task = f.w.one_task_of(&session, &format!("act.{node}")).await;
        f.w.complete(&session, &task, &json!({ "done": node }))
            .await
            .expect("the step completes");
    }
    assert_eq!(f.w.run_status(run).await, RunStatus::Succeeded);
    run
}

async fn reset(
    f: &Fixture,
    source: RunId,
    sequence: u64,
    key: &str,
) -> Result<ResetMade, AppError> {
    f.w.runs
        .reset(
            &f.w.admin,
            source,
            ResetRequest {
                sequence,
                reason: "the third step used a wrong price list",
                acknowledge_reexecution: true,
                business: RerunBusinessId::Keep,
                reapply: ResetReapply::None,
            },
            key,
            &ctx(),
        )
        .await
}

async fn completions(f: &Fixture, run: RunId) -> Vec<u64> {
    sqlx::query_scalar::<_, i64>(
        "SELECT seq FROM run_events WHERE run_id = $1 AND event_type = 'step.completed' ORDER BY seq",
    )
    .bind(run.as_uuid())
    .fetch_all(&f.w.db.pool)
    .await
    .expect("the completions")
    .into_iter()
    .map(|seq| u64::try_from(seq).expect("small"))
    .collect()
}

async fn made_from(f: &Fixture, source: RunId) -> Vec<Uuid> {
    sqlx::query_scalar("SELECT id FROM runs WHERE rerun_of = $1 ORDER BY id")
        .bind(source.as_uuid())
        .fetch_all(&f.w.db.pool)
        .await
        .expect("the runs")
}

async fn entries(f: &Fixture, source: RunId) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM run_resets WHERE source_run_id = $1")
        .bind(source.as_uuid())
        .fetch_one(&f.w.db.pool)
        .await
        .expect("the register")
}

async fn types(f: &Fixture, run: RunId) -> Vec<String> {
    sqlx::query_scalar("SELECT event_type FROM run_events WHERE run_id = $1 ORDER BY seq")
        .bind(run.as_uuid())
        .fetch_all(&f.w.db.pool)
        .await
        .expect("the journal")
}

#[tokio::test]
async fn a_reset_that_dies_at_any_write_leaves_nothing_and_sent_again_makes_one_run_and_one_entry()
{
    let f = fixture().await;
    let source = ended_chain(&f).await;
    let completed = completions(&f, source).await;
    // after the second step: two steps are kept, the third runs again
    let point = completed[1];
    let kept = usize::try_from(point).expect("small");
    let source_types = types(&f, source).await;
    let wire = Tripwire::install(&f.w.db.pool).await;

    // A first reset of a tenant makes writes that later ones do not (its first counters); one is made first
    // so that the probe shows the writes of an ordinary request.
    reset(&f, source, point, "reset-warm-up-key")
        .await
        .expect("the warm-up reset");

    wire.watch().await;
    let probe = reset(&f, source, point, "reset-probe-key")
        .await
        .expect("the probe reset");
    wire.disarm().await;
    let writes = wire.writes().await;
    let transactions: BTreeSet<i64> = writes.iter().map(|(xid, _, _)| *xid).collect();
    assert_eq!(
        transactions.len(),
        1,
        "a reset is one transaction: {writes:?}"
    );
    let tables: BTreeSet<&str> = writes.iter().map(|(_, _, t)| t.as_str()).collect();
    for table in [
        "runs",
        "run_events",
        "steps",
        "run_resets",
        "audit_records",
        "command_dedupe",
        "event_outbox",
    ] {
        assert!(
            tables.contains(table),
            "a reset is expected to write {table}: {writes:?}"
        );
    }
    // the history it made is the source's up to the point, then the reset
    assert_eq!(
        types(&f, probe.run.info.run_id).await[..kept],
        source_types[..kept]
    );
    println!(
        "a reset makes {} writing statements in one transaction, on {} tables: {tables:?}",
        writes.len(),
        tables.len()
    );

    for (_, position, table) in &writes {
        let key = format!("reset-dies-at-{position}-key");
        let runs = made_from(&f, source).await;
        let before = entries(&f, source).await;
        let digests = table_digests(&f.w.db.pool).await;

        wire.arm(*position).await;
        let result = reset(&f, source, point, &key).await;
        wire.disarm().await;

        assert!(
            result.is_err(),
            "cut off at write {position} ({table}), the request answered {result:?}"
        );
        assert_eq!(
            table_digests(&f.w.db.pool).await,
            digests,
            "cut off at write {position} ({table}), a table changed"
        );
        assert_eq!(made_from(&f, source).await, runs, "write {position}");
        assert_eq!(entries(&f, source).await, before, "write {position}");

        // sent again: one run, one entry, and a history that is the source's
        let again = reset(&f, source, point, &key)
            .await
            .unwrap_or_else(|error| panic!("write {position} ({table}), sent again: {error:?}"));
        let run = again.run.info.run_id;
        assert_eq!(
            made_from(&f, source).await.len(),
            runs.len() + 1,
            "write {position}: one run"
        );
        assert_eq!(
            entries(&f, source).await,
            before + 1,
            "write {position}: one entry"
        );
        assert_eq!(again.reset.new_run, Some(run), "write {position}");
        assert_eq!(types(&f, run).await[..kept], source_types[..kept]);
        f.w.store
            .verify_run(f.w.admin.tenant_id(), run)
            .await
            .expect("the projection equals a replay of the journal");

        // and a third time is the answer of the first, not another reset
        let third = reset(&f, source, point, &key)
            .await
            .expect("a replay of the request");
        assert_eq!(third.reset.id, again.reset.id, "write {position}");
        assert_eq!(third.run.info.run_id, run, "write {position}");
        assert_eq!(
            made_from(&f, source).await.len(),
            runs.len() + 1,
            "write {position}: still one run"
        );
        assert_eq!(entries(&f, source).await, before + 1, "write {position}");
    }
}
