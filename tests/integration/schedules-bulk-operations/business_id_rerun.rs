//! The business identifier of a rerun (`T018-035`, `FR-018-040`): `KEEP` (the default) binds the
//! identifier of the run it repeats to the new run unless another open execution holds it, `DROP`
//! starts without one. Alone and in a batch job. Real PostgreSQL, the real run service and executor.
#![allow(clippy::expect_used, clippy::panic)]

use sagat_application::batch::BatchWrite;
use sagat_application::error::AppError;
use sagat_application::execution::RunRecord;
use sagat_application::runs::{RerunBusinessId, StartOptions};
use sagat_domain::batch::{Action, JobState, Selector};
use sagat_domain::business_ids::{ConflictPolicy, ReusePolicy};
use sagat_domain::execution::RunId;
use uuid::Uuid;

use crate::batch_executor::{create, executor, items, job, live, select, settle, write};
use crate::business_ids::{business, cancel, holder_of, key, runs_of, start};
use crate::firing::{Fixture, fixture};
use crate::support::engine::ctx;

const WHY: &str = "start it again after the fix";

/// A run started under `id` and ended, for the rerun to repeat.
async fn ended_under(f: &Fixture, id: &str) -> RunId {
    let started = start(
        f,
        business(id, ConflictPolicy::Fail, ReusePolicy::AllowDuplicate),
        &key(),
    )
    .await
    .expect("the start");
    cancel(f, started.run.info.run_id).await;
    started.run.info.run_id
}

async fn rerun(
    f: &Fixture,
    source: RunId,
    business: RerunBusinessId,
    key: &str,
) -> Result<RunRecord, AppError> {
    f.w.runs
        .rerun_with(&f.w.admin, source, WHY, key, business, &ctx())
        .await
}

fn id_of(run: &RunRecord) -> Option<&str> {
    run.info.business_id.as_ref().map(|id| id.as_str())
}

#[tokio::test]
async fn a_rerun_keeps_the_identifier_by_default_and_takes_the_binding_from_the_run_it_repeats() {
    let f = fixture().await;
    let source = ended_under(&f, "order-2001").await;

    let again =
        f.w.runs
            .rerun(&f.w.admin, source, WHY, &key(), &ctx())
            .await
            .expect("the rerun");

    assert_eq!(id_of(&again), Some("order-2001"));
    assert_eq!(again.info.rerun_of, Some(source));
    assert_eq!(
        holder_of(&f, "order-2001").await,
        Some(again.info.run_id.as_uuid()),
        "the new run holds it, and holds it open"
    );
    assert_eq!(runs_of(&f, "order-2001").await.len(), 2);
}

#[tokio::test]
async fn a_rerun_that_keeps_the_identifier_is_refused_while_another_open_run_holds_it() {
    let f = fixture().await;
    let source = ended_under(&f, "order-2002").await;
    // the identifier moved on: a start took it over from the ended run
    let current = start(
        &f,
        business(
            "order-2002",
            ConflictPolicy::Fail,
            ReusePolicy::AllowDuplicate,
        ),
        &key(),
    )
    .await
    .expect("the later start")
    .run
    .info
    .run_id;

    let refused = rerun(&f, source, RerunBusinessId::Keep, &key())
        .await
        .expect_err("held by another open run");

    assert_eq!(
        refused,
        AppError::BusinessIdConflict {
            reason: "BUSINESS_ID_IN_USE",
            holder: current,
            reusable_after: None
        }
    );
    assert_eq!(
        runs_of(&f, "order-2002").await.len(),
        2,
        "the refused rerun created nothing"
    );
    assert_eq!(holder_of(&f, "order-2002").await, Some(current.as_uuid()));
}

#[tokio::test]
async fn a_rerun_that_drops_the_identifier_starts_without_one_and_leaves_the_binding_alone() {
    let f = fixture().await;
    let source = ended_under(&f, "order-2003").await;

    let again = rerun(&f, source, RerunBusinessId::Drop, &key())
        .await
        .expect("the rerun");

    assert_eq!(id_of(&again), None);
    assert_eq!(
        holder_of(&f, "order-2003").await,
        Some(source.as_uuid()),
        "the ended run still remembers it"
    );
    assert_eq!(runs_of(&f, "order-2003").await, [source.as_uuid()]);
}

#[tokio::test]
async fn a_rerun_that_drops_the_identifier_is_not_refused_when_another_run_holds_it() {
    let f = fixture().await;
    let source = ended_under(&f, "order-2004").await;
    let current = start(
        &f,
        business(
            "order-2004",
            ConflictPolicy::Fail,
            ReusePolicy::AllowDuplicate,
        ),
        &key(),
    )
    .await
    .expect("the later start")
    .run
    .info
    .run_id;

    let again = rerun(&f, source, RerunBusinessId::Drop, &key())
        .await
        .expect("dropping needs nothing from the binding");

    assert_eq!(id_of(&again), None);
    assert_eq!(holder_of(&f, "order-2004").await, Some(current.as_uuid()));
}

#[tokio::test]
async fn a_rerun_of_a_run_without_an_identifier_is_what_it_always_was() {
    let f = fixture().await;
    let started = start(&f, StartOptions::default(), &key())
        .await
        .expect("the start");
    cancel(&f, started.run.info.run_id).await;

    for choice in [RerunBusinessId::Keep, RerunBusinessId::Drop] {
        let again = rerun(&f, started.run.info.run_id, choice, &key())
            .await
            .expect("the rerun");
        assert_eq!(id_of(&again), None, "{choice:?}");
        cancel(&f, again.info.run_id).await;
    }
}

#[tokio::test]
async fn the_same_key_with_the_other_choice_is_another_request_and_the_same_choice_is_a_replay() {
    let f = fixture().await;
    let source = ended_under(&f, "order-2005").await;
    let k = key();

    let first = rerun(&f, source, RerunBusinessId::Keep, &k)
        .await
        .expect("the rerun");
    let replay = rerun(&f, source, RerunBusinessId::Keep, &k)
        .await
        .expect("a retry of the same request");
    assert_eq!(replay.info.run_id, first.info.run_id);
    assert_eq!(
        runs_of(&f, "order-2005").await.len(),
        2,
        "the retry created nothing"
    );

    let other = rerun(&f, source, RerunBusinessId::Drop, &k)
        .await
        .expect_err("the same key asked for something else");
    assert_eq!(other, AppError::IdempotencyKeyReused);

    // and the other way round: a request that dropped it is not answered for one that keeps it
    let source = ended_under(&f, "order-2006").await;
    let k = key();
    let dropped = rerun(&f, source, RerunBusinessId::Drop, &k)
        .await
        .expect("the rerun that drops it");
    let replay = rerun(&f, source, RerunBusinessId::Drop, &k)
        .await
        .expect("a retry of the same request");
    assert_eq!(replay.info.run_id, dropped.info.run_id);
    let kept = rerun(&f, source, RerunBusinessId::Keep, &k)
        .await
        .expect_err("the same key asked for something else");
    assert_eq!(kept, AppError::IdempotencyKeyReused);
}

// --- in a batch job -------------------------------------------------------------------------------

fn rerun_job(source: RunId, drop_business_id: bool) -> BatchWrite {
    BatchWrite {
        drop_business_id,
        ..write(
            Action::Rerun,
            Selector::run_ids(vec![source.as_uuid()]).expect("selector"),
        )
    }
}

async fn new_run_of(f: &Fixture, job: Uuid) -> Option<Uuid> {
    sqlx::query_scalar("SELECT new_run_id FROM batch_items WHERE job_id = $1")
        .bind(job)
        .fetch_one(&f.w.db.pool)
        .await
        .expect("the item")
}

#[tokio::test]
async fn a_batch_rerun_keeps_the_identifier_unless_the_job_says_to_drop_it() {
    let f = fixture().await;
    let kept = ended_under(&f, "batch-keep-1").await;
    let dropped = ended_under(&f, "batch-drop-1").await;
    live(&f);
    let exec = executor(&f);

    let keeping = create(&f, rerun_job(kept, false), "rerun-keeps-the-id").await;
    select(&exec, &keeping).await;
    settle(&exec, &keeping).await;
    let dropping = create(&f, rerun_job(dropped, true), "rerun-drops-the-id").await;
    select(&exec, &dropping).await;
    settle(&exec, &dropping).await;

    for finished in [&keeping, &dropping] {
        let done = job(&f, finished.id).await;
        assert_eq!(done.state, JobState::Completed);
        assert_eq!((done.counts.succeeded, done.counts.failed), (1, 0));
    }
    let kept_new = new_run_of(&f, keeping.id).await.expect("a new run");
    assert_eq!(holder_of(&f, "batch-keep-1").await, Some(kept_new));
    assert_eq!(runs_of(&f, "batch-keep-1").await.len(), 2);
    let dropped_new = new_run_of(&f, dropping.id).await.expect("a new run");
    assert_eq!(holder_of(&f, "batch-drop-1").await, Some(dropped.as_uuid()));
    assert_eq!(runs_of(&f, "batch-drop-1").await, [dropped.as_uuid()]);
    let carried: Option<String> = sqlx::query_scalar("SELECT business_id FROM runs WHERE id = $1")
        .bind(dropped_new)
        .fetch_one(&f.w.db.pool)
        .await
        .expect("the run");
    assert_eq!(carried, None);
}

#[tokio::test]
async fn a_batch_rerun_whose_identifier_is_held_fails_the_item_as_a_conflict_and_creates_nothing() {
    let f = fixture().await;
    let source = ended_under(&f, "batch-held-1").await;
    let current = start(
        &f,
        business(
            "batch-held-1",
            ConflictPolicy::Fail,
            ReusePolicy::AllowDuplicate,
        ),
        &key(),
    )
    .await
    .expect("the later start")
    .run
    .info
    .run_id;
    live(&f);
    let exec = executor(&f);

    let held = create(&f, rerun_job(source, false), "rerun-meets-a-holder").await;
    select(&exec, &held).await;
    settle(&exec, &held).await;

    let done = job(&f, held.id).await;
    assert_eq!(done.state, JobState::Completed);
    assert_eq!((done.counts.failed, done.counts.succeeded), (1, 0));
    let row = &items(&f, held.id).await[0];
    assert_eq!(
        (row.1.as_str(), row.3.as_deref()),
        ("FAILED", Some("CONFLICT")),
        "a held identifier is a conflict the operator can read, not an internal error"
    );
    assert_eq!(holder_of(&f, "batch-held-1").await, Some(current.as_uuid()));
    assert_eq!(runs_of(&f, "batch-held-1").await.len(), 2);
}
