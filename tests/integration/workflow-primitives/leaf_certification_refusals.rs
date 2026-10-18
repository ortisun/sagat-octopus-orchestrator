//! Native refusal evidence. Owner corruption fixtures do not prove producer preservation.

use super::*;
use sagat_domain::{execution::RunId, identity::TenantId};

pub(super) async fn closed(
    name: &str,
) -> (
    Flow,
    PgLeafCollectionSuccessStore,
    CertifyLeafCollectionSuccess,
) {
    closed_with_output(name, json!({"accepted":true})).await
}

pub(super) async fn closed_with_output(
    name: &str,
    output: Value,
) -> (
    Flow,
    PgLeafCollectionSuccessStore,
    CertifyLeafCollectionSuccess,
) {
    let f = Flow::start(name, document(name), json!([{"v":1}])).await;
    sqlx::query("GRANT INSERT ON leaf_collection_success_certifications TO sagat_app")
        .execute(&f.api.db.pool)
        .await
        .expect("test-only dormant activation");
    f.drive().await;
    let tasks = f.poll().await;
    assert_eq!(tasks.len(), 1);
    f.complete(&tasks[0], output).await;
    f.drive().await;
    f.verify().await;
    let c = command(&f).await;
    let store = PgLeafCollectionSuccessStore::new(
        f.api.api_pool.clone(),
        DefinitionCache::new(Arc::new(JsonIrCodec), 8),
    );
    (f, store, c)
}

pub(super) async fn counts(f: &Flow) -> (i64, i64) {
    sqlx::query_as("SELECT (SELECT count(*) FROM leaf_collection_success_certifications WHERE tenant_id=$1 AND run_id=$2),(SELECT count(*) FROM audit_records WHERE tenant_id=$1 AND action='collection.leaf_success_certified')")
        .bind(f.tenant.as_uuid()).bind(f.run.as_uuid()).fetch_one(&f.api.db.pool).await.expect("exact fact and dedicated audit counts")
}

pub(super) async fn refusal_state(f: &Flow) -> (String, String) {
    // Journal UPDATE/DELETE is guarded; count/max detects appended rows without loading detail.
    // Keep this separate from the selected-prefix oracle, which permits later legitimate work.
    let execution: String = sqlx::query_scalar("SELECT md5(jsonb_build_object('run',(SELECT to_jsonb(r) FROM runs r WHERE tenant_id=$1 AND id=$2),'journal',(SELECT jsonb_build_object('count',count(*),'max',max(seq)) FROM run_events WHERE tenant_id=$1 AND run_id=$2))::text)")
        .bind(f.tenant.as_uuid()).bind(f.run.as_uuid()).fetch_one(&f.api.db.pool).await.expect("run and append-only journal boundary fingerprint");
    (selected_sources(f).await, execution)
}

#[tokio::test]
async fn original_qualified_identity_and_creation_cas_refuse_without_metadata_writes() {
    let (f, store, c) = closed("leaf-identity-refusal").await;
    let before = refusal_state(&f).await;
    let i = &c.identity;
    for (tenant, namespace, run) in [
        (TenantId::new(Uuid::new_v4()), i.namespace_id(), i.run_id()),
        (i.tenant_id(), NamespaceId::new(Uuid::new_v4()), i.run_id()),
        (i.tenant_id(), i.namespace_id(), RunId::new(Uuid::new_v4())),
    ] {
        let mut wrong = c.clone();
        wrong.identity = LeafCollectionPathIdentity::new(tenant, namespace, run, i.path().clone())
            .expect("qualified wrong identity");
        wrong.scheduler =
            SchedulerAuthority::for_run(c.scheduler.owner(), run, c.scheduler.lease().generation);
        assert_eq!(
            store.certify_leaf_collection_success(&wrong).await,
            Err(LeafCollectionSuccessError::Missing)
        );
        assert_eq!(counts(&f).await, (0, 0));
        assert_eq!(refusal_state(&f).await, before);
    }
    let mut never_admitted = c.clone();
    never_admitted.identity = LeafCollectionPathIdentity::new(
        i.tenant_id(),
        i.namespace_id(),
        i.run_id(),
        InstancePath::root()
            .child(NodeId::parse("lines").expect("owner"), 1)
            .expect("absent iteration"),
    )
    .expect("depth1 identity");
    assert_eq!(
        store.certify_leaf_collection_success(&never_admitted).await,
        Err(LeafCollectionSuccessError::SourceEvidenceMissing)
    );
    assert_eq!(counts(&f).await, (0, 0));
    assert_eq!(refusal_state(&f).await, before);
    let mut stale = c.clone();
    stale.expected_run_revision -= 1;
    assert_eq!(
        store.certify_leaf_collection_success(&stale).await,
        Err(LeafCollectionSuccessError::StaleRunRevision {
            current: c.expected_run_revision
        })
    );
    assert_eq!(counts(&f).await, (0, 0));
    assert_eq!(refusal_state(&f).await, before);
    let valid = store
        .certify_leaf_collection_success(&c)
        .await
        .expect("positive control actual source");
    assert!(!valid.replayed);
    assert_eq!(counts(&f).await, (1, 1));
    assert_eq!(refusal_state(&f).await, before);
}

#[tokio::test]
async fn current_authority_precedes_canonical_replay_and_fresh_owner_keeps_original_record() {
    let (f, store, mut c) = closed("leaf-authority-refusal").await;
    let created = store
        .certify_leaf_collection_success(&c)
        .await
        .expect("actual source fact");
    let before = refusal_state(&f).await;
    let lease = c.scheduler.lease();
    f.api
        .executions
        .release(c.scheduler.owner(), &[lease])
        .await
        .expect("actual scheduler release");
    c.expected_run_revision = 1;
    assert_eq!(
        store.certify_leaf_collection_success(&c).await,
        Err(LeafCollectionSuccessError::SchedulerFenceLost)
    );
    assert_eq!(counts(&f).await, (1, 1));
    assert_eq!(refusal_state(&f).await, before);
    let fresh = f
        .api
        .executions
        .acquire("leaf-fresh-owner", &[lease.shard], Duration::from_secs(60))
        .await
        .expect("actual scheduler takeover");
    assert_eq!(fresh.len(), 1);
    assert!(fresh[0].generation > lease.generation);
    c.scheduler = SchedulerAuthority::for_run("leaf-fresh-owner", f.run, fresh[0].generation);
    let replay = store
        .certify_leaf_collection_success(&c)
        .await
        .expect("fresh authority reconciles original identity before CAS");
    assert!(replay.replayed);
    assert_eq!(replay.record, created.record);
    assert_eq!(counts(&f).await, (1, 1));
    assert_eq!(refusal_state(&f).await, before);
}

#[tokio::test]
async fn compensating_definition_is_not_an_ordinary_leaf_source() {
    let mut d = document("leaf-shape-refusal");
    d["bodies"]["line"]["nodes"][0]["compensateWith"] = json!("line.release");
    d["bodies"]["line"]["nodes"][0]["compensationRetry"] = crate::support::scope_flow::retry();
    let f = Flow::start("leaf-shape-refusal", d, json!([{"v":1}])).await;
    f.drive().await;
    let store = PgLeafCollectionSuccessStore::new(
        f.api.api_pool.clone(),
        DefinitionCache::new(Arc::new(JsonIrCodec), 8),
    );
    let before = refusal_state(&f).await;
    assert!(matches!(
        store
            .certify_leaf_collection_success(&command(&f).await)
            .await,
        Err(LeafCollectionSuccessError::UnsupportedShape(_))
    ));
    assert_eq!(counts(&f).await, (0, 0));
    assert_eq!(refusal_state(&f).await, before);
}

async fn exact_step_fingerprint(f: &Flow, step: Uuid) -> String {
    sqlx::query_scalar("SELECT md5(to_jsonb(t)::text) FROM steps t WHERE tenant_id=$1 AND run_id=$2 AND step_id=$3")
        .bind(f.tenant.as_uuid()).bind(f.run.as_uuid()).bind(step)
        .fetch_one(&f.api.db.pool).await.expect("retained exact step fingerprint")
}

#[tokio::test]
async fn outstanding_reservation_refuses_and_creation_cas_precedes_source_corruption() {
    let (f, store, c) = closed("leaf-reserve-refusal").await;
    let reserved: i32 = sqlx::query_scalar("UPDATE collection_iterations i SET remaining_reservations=1 FROM iteration_scopes s WHERE i.tenant_id=$1 AND i.run_id=$2 AND i.iteration_index=0 AND s.tenant_id=i.tenant_id AND s.run_id=i.run_id AND s.scope_id=i.scope_id AND s.owner_path_nodes='{}' AND s.owner_path_indices='{}' AND s.owner_node='lines' RETURNING i.reserved_instances")
        .bind(f.tenant.as_uuid()).bind(f.run.as_uuid()).fetch_one(&f.api.db.pool).await.expect("schema-valid synthetic reservation contradiction");
    assert!(reserved >= 1);
    let before = refusal_state(&f).await;
    let mut stale = c.clone();
    stale.expected_run_revision -= 1;
    assert_eq!(
        store.certify_leaf_collection_success(&stale).await,
        Err(LeafCollectionSuccessError::StaleRunRevision {
            current: c.expected_run_revision
        })
    );
    assert_eq!(counts(&f).await, (0, 0));
    assert_eq!(refusal_state(&f).await, before);
    assert_eq!(
        store.certify_leaf_collection_success(&c).await,
        Err(LeafCollectionSuccessError::ContradictorySource)
    );
    assert_eq!(counts(&f).await, (0, 0));
    assert_eq!(refusal_state(&f).await, before);
}

#[tokio::test]
async fn complete_local_step_set_and_bounded_metadata_are_required() {
    for (name, sql, expected) in [
        (
            "oversize",
            "UPDATE steps SET activity=repeat('x',129) WHERE tenant_id=$1 AND run_id=$2 AND instance_path_nodes=ARRAY['lines']::text[] AND instance_path_indices=ARRAY[0]::integer[] AND node='returned'",
            LeafCollectionSuccessError::ContradictorySource,
        ),
        (
            "unrouted",
            "UPDATE steps SET routed=false WHERE tenant_id=$1 AND run_id=$2 AND instance_path_nodes=ARRAY['lines']::text[] AND instance_path_indices=ARRAY[0]::integer[] AND node='work'",
            LeafCollectionSuccessError::ContradictorySource,
        ),
        (
            "unexpected",
            "UPDATE steps SET node='unexpected' WHERE tenant_id=$1 AND run_id=$2 AND instance_path_nodes=ARRAY['lines']::text[] AND instance_path_indices=ARRAY[0]::integer[] AND node='returned'",
            LeafCollectionSuccessError::ContradictorySource,
        ),
        (
            "missing",
            "UPDATE steps SET instance_path_indices=ARRAY[1]::integer[] WHERE tenant_id=$1 AND run_id=$2 AND instance_path_nodes=ARRAY['lines']::text[] AND instance_path_indices=ARRAY[0]::integer[] AND node='returned'",
            LeafCollectionSuccessError::SourceEvidenceMissing,
        ),
    ] {
        let (f, store, c) = closed(&format!("leaf-step-{name}")).await;
        let node = if name == "unrouted" {
            "work"
        } else {
            "returned"
        };
        let step: Uuid = sqlx::query_scalar("SELECT step_id FROM steps WHERE tenant_id=$1 AND run_id=$2 AND instance_path_nodes=ARRAY['lines']::text[] AND instance_path_indices=ARRAY[0]::integer[] AND node=$3")
            .bind(f.tenant.as_uuid()).bind(f.run.as_uuid()).bind(node)
            .fetch_one(&f.api.db.pool).await.expect("original exact selected step");
        if name == "oversize" {
            let empty: bool = sqlx::query_scalar(
                "SELECT activity IS NULL FROM steps WHERE tenant_id=$1 AND step_id=$2",
            )
            .bind(f.tenant.as_uuid())
            .bind(step)
            .fetch_one(&f.api.db.pool)
            .await
            .expect("return expects no activity");
            assert!(
                empty,
                "exercise NULL-compatible projection, not a non-null activity mismatch"
            );
        }
        // Mutations run as the disposable owner. No guard or constraint is disabled.
        let changed = sqlx::query(sql)
            .bind(f.tenant.as_uuid())
            .bind(f.run.as_uuid())
            .execute(&f.api.db.pool)
            .await
            .expect("schema-valid synthetic step contradiction");
        assert_eq!(changed.rows_affected(), 1, "{name}");
        if name == "missing" {
            let keys: (i64, i64) = sqlx::query_as("SELECT count(*) FILTER(WHERE instance_path_indices=ARRAY[0]::integer[]),count(*) FILTER(WHERE instance_path_indices=ARRAY[1]::integer[]) FROM steps WHERE tenant_id=$1 AND run_id=$2 AND node='returned' AND instance_path_nodes=ARRAY['lines']::text[]")
                .bind(f.tenant.as_uuid()).bind(f.run.as_uuid()).fetch_one(&f.api.db.pool).await.expect("old key absent and relocated retained row present");
            assert_eq!(keys, (0, 1));
        }
        let before = refusal_state(&f).await;
        let retained = exact_step_fingerprint(&f, step).await;
        assert_eq!(
            store.certify_leaf_collection_success(&c).await,
            Err(expected),
            "{name}"
        );
        assert_eq!(counts(&f).await, (0, 0), "{name}");
        assert_eq!(refusal_state(&f).await, before, "{name}");
        assert_eq!(exact_step_fingerprint(&f, step).await, retained, "{name}");
    }
}

#[tokio::test]
async fn journal_reference_and_checkpoint_digest_must_match_actual_retained_evidence() {
    for (name, sql) in [
        (
            "journal",
            "UPDATE collection_iterations i SET admitted_sequence=1 FROM iteration_scopes s WHERE i.tenant_id=$1 AND i.run_id=$2 AND i.iteration_index=0 AND s.tenant_id=i.tenant_id AND s.run_id=i.run_id AND s.scope_id=i.scope_id AND s.owner_node='lines' AND s.owner_path_nodes='{}' AND s.owner_path_indices='{}'",
        ),
        (
            "digest",
            "UPDATE iteration_checkpoints c SET result_digest=s.result_digest FROM iteration_scopes s WHERE c.tenant_id=$1 AND c.run_id=$2 AND c.iteration_index=0 AND s.tenant_id=c.tenant_id AND s.run_id=c.run_id AND s.scope_id=c.scope_id AND s.owner_node='lines' AND s.owner_path_nodes='{}' AND s.owner_path_indices='{}' AND c.result_digest<>s.result_digest",
        ),
    ] {
        let (f, store, c) = closed(&format!("leaf-evidence-{name}")).await;
        let changed = sqlx::query(sql)
            .bind(f.tenant.as_uuid())
            .bind(f.run.as_uuid())
            .execute(&f.api.db.pool)
            .await
            .expect("schema-valid synthetic evidence contradiction");
        assert_eq!(changed.rows_affected(), 1, "{name}");
        let before = refusal_state(&f).await;
        assert_eq!(
            store.certify_leaf_collection_success(&c).await,
            Err(LeafCollectionSuccessError::ContradictorySource),
            "{name}"
        );
        assert_eq!(counts(&f).await, (0, 0), "{name}");
        assert_eq!(refusal_state(&f).await, before, "{name}");
    }
}

// Draft insertion for tests/integration/workflow-primitives/leaf_certification_refusals.rs.
// The existing module imports `super::*` and owns `closed`, `counts`, and `refusal_state`.

async fn rewrite_canonical_metadata_for_decode_control(
    f: &Flow,
    definition_version: Uuid,
    certified_run_revision: i64,
) {
    // Owner-only metadata-decoding fixture. Disabling this immutable-row guard does not prove
    // that a native producer can create or preserve either corrupted value.
    let mut tx = f
        .api
        .db
        .pool
        .begin()
        .await
        .expect("canonical metadata owner fixture");
    sqlx::query(
        "ALTER TABLE leaf_collection_success_certifications \
         DISABLE TRIGGER leaf_success_append_only",
    )
    .execute(&mut *tx)
    .await
    .expect("disable canonical append-only guard for metadata-decoding fixture");
    let changed = sqlx::query(
        "UPDATE leaf_collection_success_certifications \
         SET definition_version_id=$3, certified_run_revision=$4 \
         WHERE tenant_id=$1 AND run_id=$2 AND owner_node='lines' AND iteration_index=0",
    )
    .bind(f.tenant.as_uuid())
    .bind(f.run.as_uuid())
    .bind(definition_version)
    .bind(certified_run_revision)
    .execute(&mut *tx)
    .await
    .expect("write schema-valid canonical metadata decode fixture");
    assert_eq!(changed.rows_affected(), 1);
    sqlx::query(
        "ALTER TABLE leaf_collection_success_certifications \
         ENABLE ALWAYS TRIGGER leaf_success_append_only",
    )
    .execute(&mut *tx)
    .await
    .expect("restore canonical append-only guard");
    tx.commit()
        .await
        .expect("commit canonical metadata decode fixture");
    let trigger_mode: String = sqlx::query_scalar(
        "SELECT tgenabled::text FROM pg_trigger \
         WHERE tgrelid='leaf_collection_success_certifications'::regclass \
         AND tgname='leaf_success_append_only'",
    )
    .fetch_one(&f.api.db.pool)
    .await
    .expect("canonical append-only trigger mode");
    assert_eq!(trigger_mode, "A", "restore the production ALWAYS guard");
}

#[tokio::test]
async fn canonical_metadata_reader_refuses_wrong_pin_and_future_revision() {
    let (f, store, c) = closed("leaf-canonical-metadata-refusal").await;
    let created = store
        .certify_leaf_collection_success(&c)
        .await
        .expect("actual source first certification");
    assert!(!created.replayed);

    // Establish one stable native record and live current authority before owner fault injection.
    let stable = store
        .certify_leaf_collection_success(&c)
        .await
        .expect("stable canonical record under current authority");
    assert!(stable.replayed);
    assert_eq!(stable.record, created.record);
    assert_eq!(counts(&f).await, (1, 1));
    let source_before = refusal_state(&f).await;

    let actual_revision: i64 = sqlx::query_scalar(
        "SELECT run_version FROM runs \
         WHERE tenant_id=$1 AND namespace_id=$2 AND id=$3",
    )
    .bind(c.identity.tenant_id().as_uuid())
    .bind(c.identity.namespace_id().as_uuid())
    .bind(c.identity.run_id().as_uuid())
    .fetch_one(&f.api.db.pool)
    .await
    .expect("actual guarded Run revision");
    assert_eq!(
        u64::try_from(actual_revision).expect("positive Run revision"),
        created.record.certified_run_revision
    );

    rewrite_canonical_metadata_for_decode_control(&f, Uuid::new_v4(), actual_revision).await;
    assert_eq!(
        store.certify_leaf_collection_success(&c).await,
        Err(LeafCollectionSuccessError::ContradictorySource),
        "schema-valid stored definition pin must match the actual guarded Run"
    );
    assert_eq!(counts(&f).await, (1, 1));
    assert_eq!(refusal_state(&f).await, source_before);

    let future_revision = actual_revision
        .checked_add(1)
        .expect("fixture revision remains signed");
    rewrite_canonical_metadata_for_decode_control(
        &f,
        created.record.definition_version.as_uuid(),
        future_revision,
    )
    .await;
    assert_eq!(
        store.certify_leaf_collection_success(&c).await,
        Err(LeafCollectionSuccessError::ContradictorySource),
        "stored certified revision cannot exceed the actual guarded Run revision"
    );
    assert_eq!(counts(&f).await, (1, 1));
    assert_eq!(refusal_state(&f).await, source_before);
}

#[tokio::test]
async fn canonical_metadata_reader_qualifies_owner_and_index_before_creation_cas() {
    let (f, store, c) = closed("leaf-canonical-exact-key").await;
    let created = store
        .certify_leaf_collection_success(&c)
        .await
        .expect("actual source first certification");
    assert!(!created.replayed);

    let mut stale = c.clone();
    stale.expected_run_revision = stale
        .expected_run_revision
        .checked_sub(1)
        .expect("closed fixture has a prior signed revision");
    let neighbor = store
        .certify_leaf_collection_success(&stale)
        .await
        .expect("exact neighboring canonical record replays before stale creation CAS");
    assert!(neighbor.replayed);
    assert_eq!(neighbor.record, created.record);

    let source_before = refusal_state(&f).await;
    for (owner, index) in [("missing", 0), ("lines", 1)] {
        let mut absent = stale.clone();
        absent.identity = LeafCollectionPathIdentity::new(
            c.identity.tenant_id(),
            c.identity.namespace_id(),
            c.identity.run_id(),
            InstancePath::root()
                .child(NodeId::parse(owner).expect("bounded owner"), index)
                .expect("depth-one absent key"),
        )
        .expect("qualified absent key");
        assert_eq!(
            store.certify_leaf_collection_success(&absent).await,
            Err(LeafCollectionSuccessError::StaleRunRevision {
                current: c.expected_run_revision
            }),
            "absent {owner}/{index} must miss the neighboring lines/0 record and reach creation CAS"
        );
    }
    assert_eq!(counts(&f).await, (1, 1));
    assert_eq!(refusal_state(&f).await, source_before);
}
