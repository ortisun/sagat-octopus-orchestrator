//! Bounded source-before-receipt and metadata-only controls for dormant scoped Finish.
//!
//! This file is drafted as a child of `scoped_leaf_finish_native.rs`. The owner-only
//! mutations below are labelled disposable fault fixtures. They restore every immutable guard
//! to ENABLE ALWAYS before the runtime-role consumer observes the fixture; they do not claim that
//! native code can corrupt retained source evidence.

use super::*;

async fn certificate_pin(f: &Flow, source: &CertifyLeafCollectionSuccess) -> (Uuid, i64) {
    let [segment] = source.identity.path().segments() else {
        panic!("source fixture is exactly depth one");
    };
    sqlx::query_as(
        "SELECT definition_version_id,certified_run_revision \
         FROM leaf_collection_success_certifications \
         WHERE tenant_id=$1 AND namespace_id=$2 AND run_id=$3 \
         AND owner_node=$4 AND iteration_index=$5",
    )
    .bind(source.identity.tenant_id().as_uuid())
    .bind(source.identity.namespace_id().as_uuid())
    .bind(source.identity.run_id().as_uuid())
    .bind(segment.scope_node().as_str())
    .bind(i32::try_from(segment.iteration_index()).expect("bounded ordinary index"))
    .fetch_one(&f.api.db.pool)
    .await
    .expect("exact genuine certificate pin")
}

async fn definitions_snapshot(f: &Flow) -> String {
    sqlx::query_scalar(
        "SELECT md5(COALESCE(jsonb_agg(to_jsonb(v) ORDER BY id),'[]'::jsonb)::text) \
         FROM workflow_versions v WHERE tenant_id=$1",
    )
    .bind(f.tenant.as_uuid())
    .fetch_one(&f.api.db.pool)
    .await
    .expect("owner definition snapshot")
}

async fn set_certificate_pin_fault(
    f: &Flow,
    source: &CertifyLeafCollectionSuccess,
    definition: Uuid,
    certified_revision: i64,
) {
    // LABELLED OWNER IMMUTABLE-GUARD FAULT FIXTURE: this is not a native write path. Restore the
    // append-only trigger to ALWAYS in the same transaction before the consumer can see the row.
    let [segment] = source.identity.path().segments() else {
        panic!("source fixture is exactly depth one");
    };
    let mut tx = f.api.db.pool.begin().await.expect("certificate fault tx");
    sqlx::query(
        "ALTER TABLE leaf_collection_success_certifications \
         DISABLE TRIGGER leaf_success_append_only",
    )
    .execute(&mut *tx)
    .await
    .expect("disable owner-only certificate guard for fault fixture");
    let changed = sqlx::query(
        "UPDATE leaf_collection_success_certifications \
         SET definition_version_id=$6,certified_run_revision=$7 \
         WHERE tenant_id=$1 AND namespace_id=$2 AND run_id=$3 \
         AND owner_node=$4 AND iteration_index=$5",
    )
    .bind(source.identity.tenant_id().as_uuid())
    .bind(source.identity.namespace_id().as_uuid())
    .bind(source.identity.run_id().as_uuid())
    .bind(segment.scope_node().as_str())
    .bind(i32::try_from(segment.iteration_index()).expect("bounded ordinary index"))
    .bind(definition)
    .bind(certified_revision)
    .execute(&mut *tx)
    .await
    .expect("install schema-valid certificate pin fault")
    .rows_affected();
    assert_eq!(changed, 1);
    sqlx::query(
        "ALTER TABLE leaf_collection_success_certifications \
         ENABLE ALWAYS TRIGGER leaf_success_append_only",
    )
    .execute(&mut *tx)
    .await
    .expect("restore certificate guard ALWAYS");
    tx.commit()
        .await
        .expect("certificate fault fixture commits");
}

async fn assert_certificate_guard_always(f: &Flow) {
    let enabled: bool = sqlx::query_scalar(
        "SELECT tgenabled='A' FROM pg_trigger \
         WHERE tgrelid='leaf_collection_success_certifications'::regclass \
         AND tgname='leaf_success_append_only'",
    )
    .fetch_one(&f.api.db.pool)
    .await
    .expect("certificate append-only trigger metadata");
    assert!(
        enabled,
        "owner fixture must restore the certificate guard ALWAYS"
    );
}

async fn remove_certificate_fault(f: &Flow, source: &CertifyLeafCollectionSuccess) -> Value {
    // LABELLED OWNER IMMUTABLE-GUARD FAULT FIXTURE: actual row removal is possible only through
    // owner DDL in this disposable database. The runtime role retains neither DELETE nor INSERT.
    let [segment] = source.identity.path().segments() else {
        panic!("source fixture is exactly depth one");
    };
    let mut tx = f.api.db.pool.begin().await.expect("certificate removal tx");
    sqlx::query(
        "ALTER TABLE leaf_collection_success_certifications \
         DISABLE TRIGGER leaf_success_append_only",
    )
    .execute(&mut *tx)
    .await
    .expect("disable owner-only certificate guard for removal fixture");
    let saved: Value = sqlx::query_scalar(
        "DELETE FROM leaf_collection_success_certifications c \
         WHERE tenant_id=$1 AND namespace_id=$2 AND run_id=$3 \
         AND owner_node=$4 AND iteration_index=$5 RETURNING to_jsonb(c)",
    )
    .bind(source.identity.tenant_id().as_uuid())
    .bind(source.identity.namespace_id().as_uuid())
    .bind(source.identity.run_id().as_uuid())
    .bind(segment.scope_node().as_str())
    .bind(i32::try_from(segment.iteration_index()).expect("bounded ordinary index"))
    .fetch_one(&mut *tx)
    .await
    .expect("remove exact genuine certificate fixture");
    sqlx::query(
        "ALTER TABLE leaf_collection_success_certifications \
         ENABLE ALWAYS TRIGGER leaf_success_append_only",
    )
    .execute(&mut *tx)
    .await
    .expect("restore certificate guard ALWAYS after removal");
    tx.commit()
        .await
        .expect("certificate removal fixture commits");
    saved
}

async fn restore_certificate_fixture(f: &Flow, saved: Value) {
    let mut tx = f.api.db.pool.begin().await.expect("certificate restore tx");
    let inserted = sqlx::query(
        "INSERT INTO leaf_collection_success_certifications \
         SELECT restored.* FROM jsonb_populate_record(\
         NULL::leaf_collection_success_certifications,$1::jsonb) AS restored",
    )
    .bind(saved)
    .execute(&mut *tx)
    .await
    .expect("restore exact genuine certificate")
    .rows_affected();
    assert_eq!(inserted, 1);
    // INSERT does not require disabling the append-only guard. Reassert ALWAYS transactionally.
    sqlx::query(
        "ALTER TABLE leaf_collection_success_certifications \
         ENABLE ALWAYS TRIGGER leaf_success_append_only",
    )
    .execute(&mut *tx)
    .await
    .expect("retain certificate guard ALWAYS after restore");
    tx.commit().await.expect("certificate restore commits");
}

async fn assert_select_denied(pool: &PgPool, table: &str, statement: &'static str) {
    let error = sqlx::query(statement)
        .fetch_optional(pool)
        .await
        .expect_err("runtime source/history SELECT must be denied");
    let code = error
        .as_database_error()
        .and_then(|database| database.code())
        .map(|code| code.into_owned());
    assert_eq!(code.as_deref(), Some("42501"), "{table}: {error}");
}

#[tokio::test]
async fn native_scoped_finish_validates_certificate_pin_before_matching_receipt() {
    let (f, sources, pool) = certified_flow("scoped-finish-source-before-receipt", 2).await;
    let source = &sources[0];
    let seeded = seed_group(
        &f,
        vec![active_member(
            CapacityMemberId::generate(),
            source.identity.clone(),
        )],
        10,
        4,
        Some(0),
    )
    .await;
    let command = finish_command(source, seeded, 10, 4);
    let store = PgScopedLeafFinishStore::new(pool);
    let source_before = source_execution_snapshot(&f).await;
    let definitions_before = definitions_snapshot(&f).await;

    let applied = store
        .finish_scoped_leaf(&command)
        .await
        .expect("genuine source applies before fault fixtures");
    assert!(!applied.replayed);
    assert_linked_effect(&f, &command, &applied).await;
    let capacity_after_apply = capacity_snapshot(&f, seeded.group).await;
    assert_eq!(finish_counts(&f, seeded.group).await, (1, 1, 1));
    assert_eq!(source_execution_snapshot(&f).await, source_before);

    let (original_definition, original_revision) = certificate_pin(&f, source).await;
    let wrong_definition = Uuid::new_v4(); // UUID pin has no version-table FK; no fake definition needed.
    set_certificate_pin_fault(&f, source, wrong_definition, original_revision).await;
    assert_certificate_guard_always(&f).await;
    let wrong_pin_source = source_execution_snapshot(&f).await;
    let wrong_definition_result = store.finish_scoped_leaf(&command).await;
    assert_eq!(source_execution_snapshot(&f).await, wrong_pin_source);
    assert_eq!(
        capacity_snapshot(&f, seeded.group).await,
        capacity_after_apply
    );
    set_certificate_pin_fault(&f, source, original_definition, original_revision).await;

    let future_revision = original_revision
        .checked_add(1)
        .expect("bounded native run revision");
    set_certificate_pin_fault(&f, source, original_definition, future_revision).await;
    assert_certificate_guard_always(&f).await;
    let mut conflicting = command.clone();
    conflicting.expected.group_revision += 1;
    assert_ne!(
        conflicting
            .fingerprint()
            .expect("conflicting valid command"),
        command.fingerprint().expect("original valid command")
    );
    let future_source = source_execution_snapshot(&f).await;
    let future_revision_result = store.finish_scoped_leaf(&conflicting).await;
    assert_eq!(source_execution_snapshot(&f).await, future_source);
    assert_eq!(
        capacity_snapshot(&f, seeded.group).await,
        capacity_after_apply
    );
    set_certificate_pin_fault(&f, source, original_definition, original_revision).await;

    assert_eq!(
        wrong_definition_result,
        Err(ScopedLeafFinishError::ContradictorySource),
        "actual definition pin is validated before a matching receipt"
    );
    assert_eq!(
        future_revision_result,
        Err(ScopedLeafFinishError::ContradictorySource),
        "future certification is validated before a different-fingerprint receipt"
    );
    assert_certificate_guard_always(&f).await;
    assert_eq!(source_execution_snapshot(&f).await, source_before);
    assert_eq!(definitions_snapshot(&f).await, definitions_before);
    assert_eq!(
        capacity_snapshot(&f, seeded.group).await,
        capacity_after_apply
    );
    assert_eq!(finish_counts(&f, seeded.group).await, (1, 1, 1));

    assert_eq!(
        store.finish_scoped_leaf(&conflicting).await,
        Err(ScopedLeafFinishError::FingerprintConflict),
        "restored source allows receipt fingerprint comparison"
    );
    let mut replayed = store
        .finish_scoped_leaf(&command)
        .await
        .expect("restored genuine source replays original receipt");
    assert!(replayed.replayed);
    replayed.replayed = false;
    assert_eq!(replayed, applied);
    assert_eq!(
        capacity_snapshot(&f, seeded.group).await,
        capacity_after_apply
    );
    assert_eq!(finish_counts(&f, seeded.group).await, (1, 1, 1));
    f.verify().await;
}

#[tokio::test]
async fn native_scoped_finish_requires_retained_certificate_before_receipt_and_after_fence() {
    let (f, sources, pool) = certified_flow("scoped-finish-source-missing", 1).await;
    let source = &sources[0];
    let seeded = seed_group(
        &f,
        vec![active_member(
            CapacityMemberId::generate(),
            source.identity.clone(),
        )],
        10,
        4,
        Some(0),
    )
    .await;
    let command = finish_command(source, seeded, 10, 4);
    let store = PgScopedLeafFinishStore::new(pool);
    let applied = store
        .finish_scoped_leaf(&command)
        .await
        .expect("genuine source applies before missing-source fixture");
    assert_linked_effect(&f, &command, &applied).await;
    let source_before = source_execution_snapshot(&f).await;
    let capacity_after_apply = capacity_snapshot(&f, seeded.group).await;

    let saved = remove_certificate_fault(&f, source).await;
    assert_certificate_guard_always(&f).await;
    let missing_source = source_execution_snapshot(&f).await;
    let missing_result = store.finish_scoped_leaf(&command).await;
    assert_eq!(source_execution_snapshot(&f).await, missing_source);
    assert_eq!(
        capacity_snapshot(&f, seeded.group).await,
        capacity_after_apply
    );
    let mut stale = command.clone();
    stale.scheduler = SchedulerAuthority::for_run(
        "stale-source-precedence-owner",
        f.run,
        source.scheduler.lease().generation,
    );
    let stale_result = store.finish_scoped_leaf(&stale).await;
    assert_eq!(source_execution_snapshot(&f).await, missing_source);
    assert_eq!(
        capacity_snapshot(&f, seeded.group).await,
        capacity_after_apply
    );
    restore_certificate_fixture(&f, saved).await;

    assert_eq!(
        missing_result,
        Err(ScopedLeafFinishError::SourceEvidenceMissing),
        "actual certificate absence precedes the matching receipt"
    );
    assert_eq!(
        stale_result,
        Err(ScopedLeafFinishError::SchedulerFenceLost),
        "current scheduler authority still precedes missing source"
    );
    assert_certificate_guard_always(&f).await;
    assert_eq!(source_execution_snapshot(&f).await, source_before);
    assert_eq!(
        capacity_snapshot(&f, seeded.group).await,
        capacity_after_apply
    );
    assert_eq!(finish_counts(&f, seeded.group).await, (1, 1, 1));

    let mut replayed = store
        .finish_scoped_leaf(&command)
        .await
        .expect("exactly restored certificate permits replay");
    assert!(replayed.replayed);
    replayed.replayed = false;
    assert_eq!(replayed, applied);
    assert_eq!(
        capacity_snapshot(&f, seeded.group).await,
        capacity_after_apply
    );
    assert_eq!(finish_counts(&f, seeded.group).await, (1, 1, 1));
    f.verify().await;
}

#[tokio::test]
async fn native_scoped_finish_applies_and_replays_from_certificate_metadata_only() {
    let (f, sources, pool) = certified_flow("scoped-finish-metadata-only", 1).await;
    let source = &sources[0];
    let seeded = seed_group(
        &f,
        vec![active_member(
            CapacityMemberId::generate(),
            source.identity.clone(),
        )],
        10,
        4,
        Some(0),
    )
    .await;
    let command = finish_command(source, seeded, 10, 4);
    let source_before = source_execution_snapshot(&f).await;
    let capacity_before = capacity_snapshot(&f, seeded.group).await;

    // Revoke only source proof/history/definition reads after genuine native certification. The
    // Finish consumer retains Run/current-authority, certificate, capacity and receipt access.
    sqlx::query(
        "REVOKE SELECT ON iteration_scopes,collection_iterations,iteration_checkpoints,steps,\
         scope_sagas,scope_saga_entries,run_events,workflow_versions,audit_records,\
         payloads,collection_scope_items FROM sagat_app",
    )
    .execute(&f.api.db.pool)
    .await
    .expect("metadata-only runtime-role source restrictions");

    let user: String = sqlx::query_scalar("SELECT current_user")
        .fetch_one(&pool)
        .await
        .expect("explicit scoped Finish runtime role");
    assert_eq!(user, "sagat_app");
    let privileges: (bool, bool, bool, bool) = sqlx::query_as(
        "SELECT \
         has_table_privilege(current_user,'leaf_collection_success_certifications','SELECT'),\
         has_table_privilege(current_user,'leaf_collection_success_certifications','INSERT'),\
         has_table_privilege(current_user,'audit_records','SELECT'),\
         has_table_privilege(current_user,'audit_records','INSERT')",
    )
    .fetch_one(&pool)
    .await
    .expect("effective metadata-only privileges");
    assert_eq!(privileges, (true, false, false, true));

    for (table, statement) in [
        ("payloads", "SELECT * FROM payloads LIMIT 0"),
        (
            "collection_scope_items",
            "SELECT * FROM collection_scope_items LIMIT 0",
        ),
        ("iteration_scopes", "SELECT * FROM iteration_scopes LIMIT 0"),
        (
            "collection_iterations",
            "SELECT * FROM collection_iterations LIMIT 0",
        ),
        (
            "iteration_checkpoints",
            "SELECT * FROM iteration_checkpoints LIMIT 0",
        ),
        ("steps", "SELECT * FROM steps LIMIT 0"),
        ("scope_sagas", "SELECT * FROM scope_sagas LIMIT 0"),
        (
            "scope_saga_entries",
            "SELECT * FROM scope_saga_entries LIMIT 0",
        ),
        ("run_events", "SELECT * FROM run_events LIMIT 0"),
        (
            "workflow_versions",
            "SELECT * FROM workflow_versions LIMIT 0",
        ),
        ("audit_records", "SELECT * FROM audit_records LIMIT 0"),
    ] {
        assert_select_denied(&pool, table, statement).await;
    }

    let store = PgScopedLeafFinishStore::new(pool);
    let applied = store
        .finish_scoped_leaf(&command)
        .await
        .expect("metadata-only genuine source first application");
    assert!(!applied.replayed);
    assert_eq!(
        (
            applied.revision,
            applied.epoch,
            applied.active_members,
            applied.possession,
        ),
        (11, 5, 0, CapacityPossession::Retired)
    );
    assert_ne!(capacity_snapshot(&f, seeded.group).await, capacity_before);
    assert_linked_effect(&f, &command, &applied).await;
    assert_eq!(source_execution_snapshot(&f).await, source_before);
    assert_eq!(finish_counts(&f, seeded.group).await, (1, 1, 1));
    let capacity_after_apply = capacity_snapshot(&f, seeded.group).await;

    let mut replayed = store
        .finish_scoped_leaf(&command)
        .await
        .expect("metadata-only matching receipt replay");
    assert!(replayed.replayed);
    replayed.replayed = false;
    assert_eq!(replayed, applied);
    assert_eq!(source_execution_snapshot(&f).await, source_before);
    assert_eq!(
        capacity_snapshot(&f, seeded.group).await,
        capacity_after_apply
    );
    assert_eq!(finish_counts(&f, seeded.group).await, (1, 1, 1));
    // Do not drive the workflow runtime while source/history SELECT remains revoked. The genuine
    // Flow had already completed, verified and certified before this deterministic privilege probe.
}
