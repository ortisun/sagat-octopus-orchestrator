//! Narrow schema-valid receipt decoder controls for dormant flat OrdinaryLeafV1 Finish.
//!
//! Install as `scoped_leaf_finish_receipts.rs`, a child of
//! `scoped_leaf_finish_native.rs`. These tests use genuine native source certification and Finish.
//! The only owner forgery is a labelled historical-receipt decoder fixture; it does not prove that
//! forged rows are native facts or that the storage boundary resists an arbitrary table owner.

use super::*;
use sqlx::{Postgres, Transaction};

const TOO_LATE_FOR_TIME: &str = "10000-01-01 00:00:00+00";
const LAST_SUPPORTED_CE_YEAR: &str = "9999-12-31 23:59:59+00";
const FIRST_BC_YEAR: &str = "0001-01-01 00:00:00+00 BC";

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReceiptCatalog {
    constraints: Vec<(String, String, String, bool)>,
    triggers: Vec<(String, String)>,
}

#[derive(Debug)]
struct ReceiptTruth {
    fingerprint: Vec<u8>,
    member_kind: String,
    namespace_id: Uuid,
    member_id: Uuid,
    chain_id: Option<Uuid>,
    head_run_id: Option<Uuid>,
    accepted_chain_revision: Option<i64>,
    original_run_id: Option<Uuid>,
    root_owner_node: Option<String>,
    iteration_index: Option<i32>,
    expected_group_revision: i64,
    expected_group_epoch: i64,
    accepted_run_revision: i64,
    resulting_revision: i64,
    resulting_epoch: i64,
    finished_at: OffsetDateTime,
}

async fn receipt_catalog(pool: &PgPool) -> ReceiptCatalog {
    let constraints = sqlx::query_as(
        "SELECT conname::text,contype::text,pg_get_constraintdef(oid,true),convalidated \
         FROM pg_constraint WHERE conrelid='capacity_group_finish_receipts'::regclass \
         AND contype IN ('c','f') ORDER BY conname",
    )
    .fetch_all(pool)
    .await
    .expect("receipt CHECK/FK catalog");
    let triggers = sqlx::query_as(
        "SELECT tgname::text,tgenabled::text FROM pg_trigger \
         WHERE tgrelid='capacity_group_finish_receipts'::regclass AND NOT tgisinternal \
         ORDER BY tgname",
    )
    .fetch_all(pool)
    .await
    .expect("receipt user-trigger catalog");
    ReceiptCatalog {
        constraints,
        triggers,
    }
}

async fn receipt_catalog_tx(tx: &mut Transaction<'_, Postgres>) -> ReceiptCatalog {
    let constraints = sqlx::query_as(
        "SELECT conname::text,contype::text,pg_get_constraintdef(oid,true),convalidated \
         FROM pg_constraint WHERE conrelid='capacity_group_finish_receipts'::regclass \
         AND contype IN ('c','f') ORDER BY conname",
    )
    .fetch_all(&mut **tx)
    .await
    .expect("receipt CHECK/FK catalog in owner fixture transaction");
    let triggers = sqlx::query_as(
        "SELECT tgname::text,tgenabled::text FROM pg_trigger \
         WHERE tgrelid='capacity_group_finish_receipts'::regclass AND NOT tgisinternal \
         ORDER BY tgname",
    )
    .fetch_all(&mut **tx)
    .await
    .expect("receipt user-trigger catalog in owner fixture transaction");
    ReceiptCatalog {
        constraints,
        triggers,
    }
}

fn disabled_append_only_catalog(catalog: &ReceiptCatalog) -> ReceiptCatalog {
    let mut disabled = catalog.clone();
    let append_only = disabled
        .triggers
        .iter_mut()
        .find(|(name, _)| name == "capacity_group_finish_receipts_append_only")
        .expect("append-only receipt trigger exists");
    assert_eq!(append_only.1, "A", "append-only trigger starts ALWAYS");
    append_only.1 = "D".into();
    disabled
}

async fn begin_owner_receipt_rewrite<'a>(
    f: &'a Flow,
    catalog: &ReceiptCatalog,
) -> Transaction<'a, Postgres> {
    assert_eq!(&receipt_catalog(&f.api.db.pool).await, catalog);
    assert!(
        catalog
            .constraints
            .iter()
            .all(|(_, kind, _, validated)| (kind == "c" || kind == "f") && *validated),
        "every receipt CHECK/FK remains present and validated"
    );
    assert!(
        catalog.triggers.iter().all(|(_, mode)| mode == "A"),
        "all receipt guards start in ALWAYS mode"
    );

    let mut tx = f
        .api
        .db
        .pool
        .begin()
        .await
        .expect("labelled owner receipt decoder fixture");
    sqlx::query(
        "ALTER TABLE capacity_group_finish_receipts \
         DISABLE TRIGGER capacity_group_finish_receipts_append_only",
    )
    .execute(&mut *tx)
    .await
    .expect("disable only the append-only receipt trigger in owner fixture transaction");
    assert_eq!(
        receipt_catalog_tx(&mut tx).await,
        disabled_append_only_catalog(catalog),
        "only the named append-only trigger is disabled; CHECK/FK constraints stay validated"
    );
    tx
}

async fn finish_owner_receipt_rewrite(
    mut tx: Transaction<'_, Postgres>,
    f: &Flow,
    catalog: &ReceiptCatalog,
) {
    sqlx::query(
        "ALTER TABLE capacity_group_finish_receipts \
         ENABLE ALWAYS TRIGGER capacity_group_finish_receipts_append_only",
    )
    .execute(&mut *tx)
    .await
    .expect("restore append-only receipt trigger before owner fixture commit");
    assert_eq!(
        &receipt_catalog_tx(&mut tx).await,
        catalog,
        "append-only is ALWAYS and all CHECK/FK constraints are intact before commit"
    );
    tx.commit()
        .await
        .expect("owner receipt decoder fixture commits with guards restored");
    assert_eq!(
        &receipt_catalog(&f.api.db.pool).await,
        catalog,
        "receipt schema guards remain exact after owner fixture commit"
    );
}

async fn receipt_truth(f: &Flow, command: &ScopedLeafFinishCommand) -> ReceiptTruth {
    let row = sqlx::query(
        "SELECT fingerprint,member_kind,namespace_id,member_id,chain_id,head_run_id,\
         accepted_chain_revision,original_run_id,root_owner_node,iteration_index,\
         expected_group_revision,expected_group_epoch,accepted_run_revision,resulting_revision,\
         resulting_epoch,finished_at FROM capacity_group_finish_receipts \
         WHERE tenant_id=$1 AND group_id=$2 AND operation_key=$3",
    )
    .bind(command.identity.tenant_id().as_uuid())
    .bind(command.group_id.as_uuid())
    .bind(command.operation_key)
    .fetch_one(&f.api.db.pool)
    .await
    .expect("genuine native receipt truth");
    ReceiptTruth {
        fingerprint: row.get("fingerprint"),
        member_kind: row.get("member_kind"),
        namespace_id: row.get("namespace_id"),
        member_id: row.get("member_id"),
        chain_id: row.get("chain_id"),
        head_run_id: row.get("head_run_id"),
        accepted_chain_revision: row.get("accepted_chain_revision"),
        original_run_id: row.get("original_run_id"),
        root_owner_node: row.get("root_owner_node"),
        iteration_index: row.get("iteration_index"),
        expected_group_revision: row.get("expected_group_revision"),
        expected_group_epoch: row.get("expected_group_epoch"),
        accepted_run_revision: row.get("accepted_run_revision"),
        resulting_revision: row.get("resulting_revision"),
        resulting_epoch: row.get("resulting_epoch"),
        finished_at: row.get("finished_at"),
    }
}

async fn owner_rewrite_time_and_fingerprint(
    f: &Flow,
    command: &ScopedLeafFinishCommand,
    catalog: &ReceiptCatalog,
    fingerprint: &[u8; 32],
    timestamp_text: &'static str,
) {
    let mut tx = begin_owner_receipt_rewrite(f, catalog).await;
    let changed = sqlx::query(
        "UPDATE capacity_group_finish_receipts \
         SET fingerprint=$4,finished_at=$5::text::timestamptz \
         WHERE tenant_id=$1 AND group_id=$2 AND operation_key=$3",
    )
    .bind(command.identity.tenant_id().as_uuid())
    .bind(command.group_id.as_uuid())
    .bind(command.operation_key)
    .bind(fingerprint.as_slice())
    .bind(timestamp_text)
    .execute(&mut *tx)
    .await
    .expect("schema-valid owner timestamp/fingerprint decoder fixture");
    assert_eq!(changed.rows_affected(), 1);
    finish_owner_receipt_rewrite(tx, f, catalog).await;
}

async fn owner_restore_time_and_fingerprint(
    f: &Flow,
    command: &ScopedLeafFinishCommand,
    catalog: &ReceiptCatalog,
    truth: &ReceiptTruth,
) {
    let mut tx = begin_owner_receipt_rewrite(f, catalog).await;
    let changed = sqlx::query(
        "UPDATE capacity_group_finish_receipts SET fingerprint=$4,finished_at=$5 \
         WHERE tenant_id=$1 AND group_id=$2 AND operation_key=$3",
    )
    .bind(command.identity.tenant_id().as_uuid())
    .bind(command.group_id.as_uuid())
    .bind(command.operation_key)
    .bind(truth.fingerprint.as_slice())
    .bind(truth.finished_at)
    .execute(&mut *tx)
    .await
    .expect("restore exact native timestamp/fingerprint with OffsetDateTime bind");
    assert_eq!(changed.rows_affected(), 1);
    finish_owner_receipt_rewrite(tx, f, catalog).await;
}

async fn owner_rewrite_revision_pair(
    f: &Flow,
    command: &ScopedLeafFinishCommand,
    catalog: &ReceiptCatalog,
    expected_group_revision: i64,
    resulting_revision: i64,
) {
    let mut tx = begin_owner_receipt_rewrite(f, catalog).await;
    let changed = sqlx::query(
        "UPDATE capacity_group_finish_receipts \
         SET expected_group_revision=$4,resulting_revision=$5 \
         WHERE tenant_id=$1 AND group_id=$2 AND operation_key=$3",
    )
    .bind(command.identity.tenant_id().as_uuid())
    .bind(command.group_id.as_uuid())
    .bind(command.operation_key)
    .bind(expected_group_revision)
    .bind(resulting_revision)
    .execute(&mut *tx)
    .await
    .expect("schema-valid owner expected/result revision decoder fixture");
    assert_eq!(changed.rows_affected(), 1);
    finish_owner_receipt_rewrite(tx, f, catalog).await;
}

async fn owner_rewrite_epoch_pair(
    f: &Flow,
    command: &ScopedLeafFinishCommand,
    catalog: &ReceiptCatalog,
    expected_group_epoch: i64,
    resulting_epoch: i64,
) {
    let mut tx = begin_owner_receipt_rewrite(f, catalog).await;
    let changed = sqlx::query(
        "UPDATE capacity_group_finish_receipts \
         SET expected_group_epoch=$4,resulting_epoch=$5 \
         WHERE tenant_id=$1 AND group_id=$2 AND operation_key=$3",
    )
    .bind(command.identity.tenant_id().as_uuid())
    .bind(command.group_id.as_uuid())
    .bind(command.operation_key)
    .bind(expected_group_epoch)
    .bind(resulting_epoch)
    .execute(&mut *tx)
    .await
    .expect("schema-valid owner expected/result epoch decoder fixture");
    assert_eq!(changed.rows_affected(), 1);
    finish_owner_receipt_rewrite(tx, f, catalog).await;
}

async fn owner_rewrite_accepted_run_revision(
    f: &Flow,
    command: &ScopedLeafFinishCommand,
    catalog: &ReceiptCatalog,
    accepted_run_revision: i64,
) {
    let mut tx = begin_owner_receipt_rewrite(f, catalog).await;
    let changed = sqlx::query(
        "UPDATE capacity_group_finish_receipts SET accepted_run_revision=$4 \
         WHERE tenant_id=$1 AND group_id=$2 AND operation_key=$3",
    )
    .bind(command.identity.tenant_id().as_uuid())
    .bind(command.group_id.as_uuid())
    .bind(command.operation_key)
    .bind(accepted_run_revision)
    .execute(&mut *tx)
    .await
    .expect("schema-valid owner accepted Run revision decoder fixture");
    assert_eq!(changed.rows_affected(), 1);
    finish_owner_receipt_rewrite(tx, f, catalog).await;
}

async fn owner_rebind_to_existing_scoped_member(
    f: &Flow,
    command: &ScopedLeafFinishCommand,
    catalog: &ReceiptCatalog,
    member: CapacityMemberId,
    identity: &LeafCollectionPathIdentity,
) {
    let [segment] = identity.path().segments() else {
        panic!("owner fixture peer is one ordinary segment");
    };
    let mut tx = begin_owner_receipt_rewrite(f, catalog).await;
    let changed = sqlx::query(
        "UPDATE capacity_group_finish_receipts SET \
         member_kind='SCOPED_PATH',namespace_id=$4,member_id=$5,chain_id=NULL,head_run_id=NULL,\
         accepted_chain_revision=NULL,original_run_id=$6,root_owner_node=$7,iteration_index=$8 \
         WHERE tenant_id=$1 AND group_id=$2 AND operation_key=$3",
    )
    .bind(command.identity.tenant_id().as_uuid())
    .bind(command.group_id.as_uuid())
    .bind(command.operation_key)
    .bind(identity.namespace_id().as_uuid())
    .bind(member.as_uuid())
    .bind(identity.run_id().as_uuid())
    .bind(segment.scope_node().as_str())
    .bind(i32::try_from(segment.iteration_index()).expect("bounded peer index"))
    .execute(&mut *tx)
    .await
    .expect("coherent owner rebinding to an existing nonselected scoped member");
    assert_eq!(changed.rows_affected(), 1);
    finish_owner_receipt_rewrite(tx, f, catalog).await;
}

async fn owner_restore_receipt_truth(
    f: &Flow,
    command: &ScopedLeafFinishCommand,
    catalog: &ReceiptCatalog,
    truth: &ReceiptTruth,
) {
    let mut tx = begin_owner_receipt_rewrite(f, catalog).await;
    let changed = sqlx::query(
        "UPDATE capacity_group_finish_receipts SET \
         fingerprint=$4,member_kind=$5,namespace_id=$6,member_id=$7,chain_id=$8,head_run_id=$9,\
         accepted_chain_revision=$10,original_run_id=$11,root_owner_node=$12,iteration_index=$13,\
         expected_group_revision=$14,expected_group_epoch=$15,accepted_run_revision=$16,\
         resulting_revision=$17,resulting_epoch=$18,finished_at=$19 \
         WHERE tenant_id=$1 AND group_id=$2 AND operation_key=$3",
    )
    .bind(command.identity.tenant_id().as_uuid())
    .bind(command.group_id.as_uuid())
    .bind(command.operation_key)
    .bind(truth.fingerprint.as_slice())
    .bind(truth.member_kind.as_str())
    .bind(truth.namespace_id)
    .bind(truth.member_id)
    .bind(truth.chain_id)
    .bind(truth.head_run_id)
    .bind(truth.accepted_chain_revision)
    .bind(truth.original_run_id)
    .bind(truth.root_owner_node.as_deref())
    .bind(truth.iteration_index)
    .bind(truth.expected_group_revision)
    .bind(truth.expected_group_epoch)
    .bind(truth.accepted_run_revision)
    .bind(truth.resulting_revision)
    .bind(truth.resulting_epoch)
    .bind(truth.finished_at)
    .execute(&mut *tx)
    .await
    .expect("restore exact genuine native receipt fields");
    assert_eq!(changed.rows_affected(), 1);
    finish_owner_receipt_rewrite(tx, f, catalog).await;
}

async fn assert_native_refusal_is_atomic(
    f: &Flow,
    store: &PgScopedLeafFinishStore,
    command: &ScopedLeafFinishCommand,
    source_before: &SourceExecutionSnapshot,
    expected: ScopedLeafFinishError,
) {
    let capacity_before = capacity_snapshot(f, command.group_id).await;
    assert_eq!(store.finish_scoped_leaf(command).await, Err(expected));
    assert_eq!(
        capacity_snapshot(f, command.group_id).await,
        capacity_before,
        "native refusal changes no receipt, G, member, audit, or hint"
    );
    assert_eq!(
        &source_execution_snapshot(f).await,
        source_before,
        "native refusal changes no source, certificate, Run, journal, step, or topology"
    );
}

async fn assert_exact_replay_at(
    f: &Flow,
    store: &PgScopedLeafFinishStore,
    command: &ScopedLeafFinishCommand,
    source_before: &SourceExecutionSnapshot,
    applied: &ScopedLeafFinishResult,
    finished_at: OffsetDateTime,
) {
    let capacity_before = capacity_snapshot(f, command.group_id).await;
    let replay = store
        .finish_scoped_leaf(command)
        .await
        .expect("schema-valid stored timestamp replays");
    let mut expected = applied.clone();
    expected.finished_at = finished_at;
    expected.replayed = true;
    assert_eq!(
        replay, expected,
        "replay returns the exact stored timestamp"
    );
    assert_eq!(
        capacity_snapshot(f, command.group_id).await,
        capacity_before
    );
    assert_eq!(&source_execution_snapshot(f).await, source_before);
}

async fn postgres_timestamp(pool: &PgPool, timestamp_text: &'static str) -> OffsetDateTime {
    sqlx::query_scalar("SELECT $1::text::timestamptz")
        .bind(timestamp_text)
        .fetch_one(pool)
        .await
        .expect("schema-valid timestamp is representable by SQLx/time")
}

#[tokio::test]
async fn native_receipt_checks_fingerprint_before_guarded_timestamp_and_replays_ce_bc_controls() {
    let (f, sources, pool) = certified_flow("scoped-finish-native-receipt-time", 1).await;
    let selected = CapacityMemberId::generate();
    let seeded = seed_group(
        &f,
        vec![active_member(selected, sources[0].identity.clone())],
        10,
        4,
        Some(0),
    )
    .await;
    let command = finish_command(&sources[0], seeded, 10, 4);
    let source_before = source_execution_snapshot(&f).await;
    let store = PgScopedLeafFinishStore::new(pool);
    let applied = store
        .finish_scoped_leaf(&command)
        .await
        .expect("genuine certificate and native Finish create the receipt");
    assert!(!applied.replayed);
    assert_linked_effect(&f, &command, &applied).await;
    assert_eq!(finish_counts(&f, seeded.group).await, (1, 1, 1));
    assert_eq!(source_execution_snapshot(&f).await, source_before);

    let truth = receipt_truth(&f, &command).await;
    assert_eq!(truth.fingerprint.len(), 32);
    let catalog = receipt_catalog(&f.api.db.pool).await;
    let changed_fingerprint = [0x5au8; 32];
    assert_ne!(truth.fingerprint.as_slice(), changed_fingerprint.as_slice());

    owner_rewrite_time_and_fingerprint(
        &f,
        &command,
        &catalog,
        &changed_fingerprint,
        TOO_LATE_FOR_TIME,
    )
    .await;
    assert_native_refusal_is_atomic(
        &f,
        &store,
        &command,
        &source_before,
        ScopedLeafFinishError::FingerprintConflict,
    )
    .await;

    let native_fingerprint: [u8; 32] = truth
        .fingerprint
        .as_slice()
        .try_into()
        .expect("genuine native fingerprint is exactly 32 bytes");
    owner_rewrite_time_and_fingerprint(
        &f,
        &command,
        &catalog,
        &native_fingerprint,
        TOO_LATE_FOR_TIME,
    )
    .await;
    assert_native_refusal_is_atomic(
        &f,
        &store,
        &command,
        &source_before,
        ScopedLeafFinishError::ContradictoryProjection,
    )
    .await;

    for timestamp_text in [LAST_SUPPORTED_CE_YEAR, FIRST_BC_YEAR] {
        owner_rewrite_time_and_fingerprint(
            &f,
            &command,
            &catalog,
            &native_fingerprint,
            timestamp_text,
        )
        .await;
        let expected = postgres_timestamp(&f.api.db.pool, timestamp_text).await;
        assert_exact_replay_at(&f, &store, &command, &source_before, &applied, expected).await;
    }

    owner_restore_time_and_fingerprint(&f, &command, &catalog, &truth).await;
    assert_exact_replay_at(
        &f,
        &store,
        &command,
        &source_before,
        &applied,
        truth.finished_at,
    )
    .await;
    assert_eq!(finish_counts(&f, seeded.group).await, (1, 1, 1));
    f.verify().await;
}

#[tokio::test]
async fn native_receipt_refuses_schema_valid_counter_and_scoped_binding_contradictions() {
    let (f, sources, pool) = certified_flow("scoped-finish-native-receipt-binding", 2).await;
    let selected = CapacityMemberId::generate();
    let peer = CapacityMemberId::generate();
    let seeded = seed_group(
        &f,
        vec![
            active_member(selected, sources[0].identity.clone()),
            active_member(peer, sources[1].identity.clone()),
        ],
        10,
        4,
        Some(0),
    )
    .await;
    let command = finish_command(&sources[0], seeded, 10, 4);
    let source_before = source_execution_snapshot(&f).await;
    let peer_before = nonselected_members(&f, seeded.group, seeded.selected).await;
    let store = PgScopedLeafFinishStore::new(pool);
    let applied = store
        .finish_scoped_leaf(&command)
        .await
        .expect("genuine selected-holder Finish creates a VACANT receipt");
    assert_eq!(
        (
            applied.revision,
            applied.epoch,
            applied.active_members,
            applied.possession,
        ),
        (11, 5, 1, CapacityPossession::Vacant)
    );
    assert_eq!(
        nonselected_members(&f, seeded.group, seeded.selected).await,
        peer_before
    );
    assert_eq!(source_execution_snapshot(&f).await, source_before);
    assert_eq!(finish_counts(&f, seeded.group).await, (1, 1, 1));
    assert_linked_effect(&f, &command, &applied).await;

    let truth = receipt_truth(&f, &command).await;
    let catalog = receipt_catalog(&f.api.db.pool).await;

    owner_rewrite_revision_pair(
        &f,
        &command,
        &catalog,
        truth
            .expected_group_revision
            .checked_add(1)
            .expect("fixture revision"),
        truth
            .resulting_revision
            .checked_add(1)
            .expect("fixture result revision"),
    )
    .await;
    assert_native_refusal_is_atomic(
        &f,
        &store,
        &command,
        &source_before,
        ScopedLeafFinishError::ContradictoryProjection,
    )
    .await;
    owner_restore_receipt_truth(&f, &command, &catalog, &truth).await;

    owner_rewrite_epoch_pair(
        &f,
        &command,
        &catalog,
        truth
            .expected_group_epoch
            .checked_add(1)
            .expect("fixture epoch"),
        truth
            .resulting_epoch
            .checked_add(1)
            .expect("fixture result epoch"),
    )
    .await;
    assert_native_refusal_is_atomic(
        &f,
        &store,
        &command,
        &source_before,
        ScopedLeafFinishError::ContradictoryProjection,
    )
    .await;
    owner_restore_receipt_truth(&f, &command, &catalog, &truth).await;

    owner_rewrite_accepted_run_revision(
        &f,
        &command,
        &catalog,
        truth
            .accepted_run_revision
            .checked_add(1)
            .expect("fixture Run revision"),
    )
    .await;
    assert_native_refusal_is_atomic(
        &f,
        &store,
        &command,
        &source_before,
        ScopedLeafFinishError::ContradictoryProjection,
    )
    .await;
    owner_restore_receipt_truth(&f, &command, &catalog, &truth).await;

    owner_rebind_to_existing_scoped_member(&f, &command, &catalog, peer, &sources[1].identity)
        .await;
    assert_native_refusal_is_atomic(
        &f,
        &store,
        &command,
        &source_before,
        ScopedLeafFinishError::ContradictoryProjection,
    )
    .await;
    owner_restore_receipt_truth(&f, &command, &catalog, &truth).await;

    let capacity_before = capacity_snapshot(&f, seeded.group).await;
    let mut replay = store
        .finish_scoped_leaf(&command)
        .await
        .expect("restored genuine native receipt replays");
    assert!(replay.replayed);
    replay.replayed = false;
    assert_eq!(
        replay, applied,
        "restored receipt returns the exact original result"
    );
    assert_eq!(capacity_snapshot(&f, seeded.group).await, capacity_before);
    assert_eq!(source_execution_snapshot(&f).await, source_before);
    assert_eq!(
        nonselected_members(&f, seeded.group, seeded.selected).await,
        peer_before
    );
    assert_eq!(finish_counts(&f, seeded.group).await, (1, 1, 1));
    assert_eq!(&receipt_catalog(&f.api.db.pool).await, &catalog);
    f.verify().await;
}
