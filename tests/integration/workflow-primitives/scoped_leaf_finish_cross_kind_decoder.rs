//! One current-schema decoder control, separate from differing-fingerprint key conflicts.
//! The owner inserts a labelled ChildRoot receipt fixture, never a native finality fact.

use super::*;

#[tokio::test]
async fn exact_scoped_fingerprint_cannot_turn_a_child_root_receipt_into_a_scoped_result() {
    let (f, sources, pool) = certified_flow("scoped-finish-exact-fingerprint-kind", 1).await;
    let alternate = publish_started_run(
        &f,
        "payments",
        sources[0].identity.namespace_id(),
        "equal-fingerprint-child",
    )
    .await;
    let chain_revision = attach_owner_chain_fixture(&f, alternate).await;
    let seeded = seed_group(
        &f,
        vec![active_member(
            CapacityMemberId::generate(),
            sources[0].identity.clone(),
        )],
        6,
        4,
        Some(0),
    )
    .await;
    let child = CapacityMemberId::generate();
    insert_finished_child_root_member(&f, seeded.group, child, alternate).await;
    let command = finish_command(&sources[0], seeded, 6, 4);
    let fingerprint = command.fingerprint().expect("supported flat command");
    // This is a deliberately owner-authored opaque hash, not a claimed SHA256 collision or
    // a native ChildRoot Finish. Every CHECK/FK and ALWAYS trigger stays active throughout.
    insert_child_root_receipt_fixture(&f, &command, child, alternate, chain_revision, fingerprint)
        .await;
    let row = sqlx::query(
        "SELECT member_kind,fingerprint,original_run_id,root_owner_node,iteration_index \
         FROM capacity_group_finish_receipts \
         WHERE tenant_id=$1 AND group_id=$2 AND operation_key=$3",
    )
    .bind(f.tenant.as_uuid())
    .bind(seeded.group.as_uuid())
    .bind(command.operation_key)
    .fetch_one(&f.api.db.pool)
    .await
    .expect("exact matching-fingerprint ChildRoot fixture");
    assert_eq!(row.get::<String, _>("member_kind"), "CHILD_ROOT");
    assert_eq!(row.get::<Vec<u8>, _>("fingerprint"), fingerprint);
    assert_eq!(row.get::<Option<Uuid>, _>("original_run_id"), None);
    assert_eq!(row.get::<Option<String>, _>("root_owner_node"), None);
    assert_eq!(row.get::<Option<i32>, _>("iteration_index"), None);

    let source_before = source_execution_snapshot(&f).await;
    let alternate_before = alternate_run_snapshot(&f, alternate).await;
    let store = PgScopedLeafFinishStore::new(pool);
    assert_refusal_unchanged(
        &f,
        &store,
        &command,
        seeded.group,
        ScopedLeafFinishError::ContradictoryProjection,
    )
    .await;
    assert_eq!(finish_counts(&f, seeded.group).await, (1, 0, 0));

    let mut fresh = command.clone();
    fresh.operation_key = Uuid::new_v4();
    assert_eq!(fresh.fingerprint(), Ok(fingerprint));
    let applied = store
        .finish_scoped_leaf(&fresh)
        .await
        .expect("fresh-key native control consumes the genuine selected source");
    assert!(!applied.replayed);
    assert_eq!(
        (
            applied.revision,
            applied.epoch,
            applied.active_members,
            applied.possession
        ),
        (7, 5, 0, CapacityPossession::Retired)
    );
    assert_linked_effect(&f, &fresh, &applied).await;
    assert_refusal_unchanged(
        &f,
        &store,
        &command,
        seeded.group,
        ScopedLeafFinishError::ContradictoryProjection,
    )
    .await;
    let before_replay = capacity_snapshot(&f, seeded.group).await;
    let replayed = store
        .finish_scoped_leaf(&fresh)
        .await
        .expect("genuine scoped receipt with the same fingerprint replays");
    let mut expected = applied;
    expected.replayed = true;
    assert_eq!(replayed, expected);
    assert_eq!(capacity_snapshot(&f, seeded.group).await, before_replay);
    assert_eq!(source_execution_snapshot(&f).await, source_before);
    assert_eq!(
        alternate_run_snapshot(&f, alternate).await,
        alternate_before
    );
    assert_eq!(finish_counts(&f, seeded.group).await, (2, 1, 1));
    f.verify().await;
}
