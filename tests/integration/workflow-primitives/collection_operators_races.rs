//! Races of a chained collection's entry (design 042, T042-48, SC-042-005), forced with a barrier inside the evaluator,
//! never with a sleep: the chain is held while the evaluation is in flight, the other side commits, and the chain is
//! released. The stale entry records nothing; the run is cancelled or entered exactly once.

use sagat_domain::execution::RunId;
use serde_json::json;
use uuid::Uuid;

use crate::collection_operators::{
    Mode, Watched, cancel, chained_ir, drive, gate, owner, replica, scope_of, settle, verified,
};
use crate::support::blob_collections::{
    platform_with_operators, scoped_worker, scopes, start_inline, status_and_code,
};
use crate::support::worker::Platform;
use sagat_infra_postgres::PgRunStore;
use sagat_scheduler::{CollectionAdvance, CollectionAdvanceError, Keeper};

/// Another owner takes the run's shard: a new generation, so every fence of the old lease fails.
pub(crate) async fn steal_shard(p: &Platform, run: Uuid) {
    sqlx::query(
        "UPDATE scheduler_shards SET generation = generation + 1, owner = 'operators-thief', \
                expires_at = clock_timestamp() + interval '1 hour', acquired_at = clock_timestamp() \
         WHERE shard = run_shard($1)",
    )
    .bind(run)
    .execute(&p.api.db.pool)
    .await
    .expect("steal the shard");
}

/// The thief's lease runs out and `keeper` takes the shard again, under a newer generation.
pub(crate) async fn give_back_shard(p: &Platform, run: Uuid, keeper: &Keeper<PgRunStore>) {
    sqlx::query(
        "UPDATE scheduler_shards SET expires_at = clock_timestamp() - interval '1 second' \
         WHERE shard = run_shard($1)",
    )
    .bind(run)
    .execute(&p.api.db.pool)
    .await
    .expect("expire the thief's lease");
    keeper.round().await.expect("take the shard again");
}

async fn chained_run(p: &Platform, name: &str) -> Uuid {
    let workflow = p
        .publish(
            name,
            &chained_ir(
                name,
                "input.lines",
                &json!([{"filter": {"where": "element % 2 == 1"}}]),
                10,
            ),
        )
        .await;
    start_inline(p, &workflow, &json!({"lines": [1, 2, 3, 4, 5]})).await
}

#[tokio::test]
async fn a_cancellation_committed_while_the_chain_is_held_wins_and_the_entry_records_nothing() {
    let p = platform_with_operators().await;
    let run = chained_run(&p, "operators-cancel-first").await;
    let (hold, mut barrier) = gate();
    let evaluator = Watched::new(Mode::Hold(hold));
    let (scheduler, keeper) = replica(&p, evaluator.clone(), None).await;

    let cancel_first = async {
        barrier.held().await;
        cancel(&p, run).await;
        barrier.release();
    };
    let joined_owner = owner();
    let (entry, ()) = tokio::join!(
        scheduler.advance_collection_from_run(p.tenant, RunId::new(run), &joined_owner),
        cancel_first
    );
    assert!(
        !matches!(entry, Ok(CollectionAdvance::Entered)),
        "the entry evaluated before the cancellation must not commit after it: {entry:?}"
    );
    assert_eq!(scopes(&p, run).await, 0, "the stale entry recorded nothing");
    assert_eq!(evaluator.calls(), 1);

    // Another replica settles the run; its evaluator panics if the cancelled run's chain is evaluated again.
    keeper.leave().await;
    drop(scheduler);
    let after = Watched::new(Mode::Panic);
    let (scheduler, _keeper) = replica(&p, after.clone(), None).await;
    settle(&p, &scheduler, run, 20).await;
    assert_eq!(status_and_code(&p, run).await.0, "CANCELLED");
    assert_eq!(after.calls(), 0);
    assert_eq!(scopes(&p, run).await, 0, "cancelled with no scope");
    verified(&p, run).await;
}

#[tokio::test]
async fn an_entry_committed_before_the_cancellation_is_entered_once_and_then_cancelled() {
    let p = platform_with_operators().await;
    let run = chained_run(&p, "operators-entry-first").await;
    let (hold, mut barrier) = gate();
    let evaluator = Watched::new(Mode::Hold(hold));
    let (scheduler, keeper) = replica(&p, evaluator.clone(), None).await;

    let release = async {
        barrier.held().await;
        barrier.release();
    };
    let joined_owner = owner();
    let (entry, ()) = tokio::join!(
        scheduler.advance_collection_from_run(p.tenant, RunId::new(run), &joined_owner),
        release
    );
    assert!(matches!(entry, Ok(CollectionAdvance::Entered)), "{entry:?}");
    // The entry has committed: only now is the cancellation sent.
    cancel(&p, run).await;
    keeper.leave().await;
    drop(scheduler);
    let after = Watched::new(Mode::Panic);
    let (scheduler, _keeper) = replica(&p, after.clone(), None).await;
    settle(&p, &scheduler, run, 20).await;
    assert_eq!(status_and_code(&p, run).await.0, "CANCELLED");
    assert_eq!(scopes(&p, run).await, 1, "entered exactly once");
    assert_eq!(scope_of(&p, run).await.0, 3);
    assert_eq!(
        evaluator.calls(),
        1,
        "the drain does not evaluate the chain"
    );
    verified(&p, run).await;
}

#[tokio::test]
async fn a_shard_lease_taken_while_the_chain_is_held_discards_the_evaluation() {
    let p = platform_with_operators().await;
    let run = chained_run(&p, "operators-lease").await;
    let (hold, mut barrier) = gate();
    let evaluator = Watched::new(Mode::Hold(hold));
    let (scheduler, keeper) = replica(&p, evaluator.clone(), None).await;

    let take_the_lease = async {
        barrier.held().await;
        steal_shard(&p, run).await;
        barrier.release();
    };
    let joined_owner = owner();
    let (stale, ()) = tokio::join!(
        scheduler.advance_collection_from_run(p.tenant, RunId::new(run), &joined_owner),
        take_the_lease
    );
    assert!(
        matches!(
            stale,
            Err(CollectionAdvanceError::FenceUnavailable
                | CollectionAdvanceError::Primitive(
                    sagat_application::primitives::PrimitiveStoreError::SchedulerFenceLost
                ))
        ),
        "the entry of a lost lease is refused: {stale:?}"
    );
    assert_eq!(scopes(&p, run).await, 0, "the stale entry recorded nothing");

    // The next owner evaluates once more and records once.
    give_back_shard(&p, run, &keeper).await;
    let release = async {
        barrier.held().await;
        barrier.release();
    };
    let joined_owner = owner();
    let (entry, ()) = tokio::join!(
        scheduler.advance_collection_from_run(p.tenant, RunId::new(run), &joined_owner),
        release
    );
    assert!(matches!(entry, Ok(CollectionAdvance::Entered)), "{entry:?}");
    assert_eq!(evaluator.calls(), 2);
    assert_eq!(scopes(&p, run).await, 1);
    assert_eq!(scope_of(&p, run).await, (3, b"[1,3,5]".to_vec()));

    // The rest of the run never calls the evaluator's chain again.
    let after = Watched::new(Mode::Panic);
    keeper.leave().await;
    drop(scheduler);
    let (scheduler, _keeper) = replica(&p, after.clone(), None).await;
    let mut worker = scoped_worker(&p, "operators-lease-worker", &["src.line"]).await;
    let seen = drive(&p, &scheduler, &mut worker, run).await;
    assert_eq!(seen.len(), 3);
    assert_eq!(
        status_and_code(&p, run).await,
        ("SUCCEEDED".to_owned(), None)
    );
    assert_eq!(after.calls(), 0);
    verified(&p, run).await;
}
