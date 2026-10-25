//! Real worker progress after a public DETACH cancellation; no whole-child/fanout claim.

use super::*;

async fn settled_parent(f: &Flow, parent: RunId) -> String {
    sqlx::query_scalar(
        "SELECT md5(jsonb_build_object(\
         'run',(SELECT to_jsonb(r) FROM runs r WHERE r.tenant_id=$1 AND r.id=$2),\
         'steps',(SELECT jsonb_agg(to_jsonb(s) ORDER BY s.node) FROM steps s \
          WHERE s.tenant_id=$1 AND s.run_id=$2),\
         'journal',(SELECT jsonb_agg(to_jsonb(e) ORDER BY e.seq) FROM run_events e \
          WHERE e.tenant_id=$1 AND e.run_id=$2),\
         'audit',(SELECT jsonb_agg(to_jsonb(a) ORDER BY a.id) FROM audit_records a \
          WHERE a.tenant_id=$1 AND a.resource_type='run' AND a.resource_id=$2::text),\
         'outbox',(SELECT jsonb_agg(to_jsonb(o) ORDER BY o.id) FROM event_outbox o \
          WHERE o.tenant_id=$1 AND o.aggregate_id=$2))::text)",
    )
    .bind(f.tenant.as_uuid())
    .bind(parent.as_uuid())
    .fetch_one(&f.api.db.pool)
    .await
    .expect("exact settled parent snapshot")
}

#[tokio::test]
async fn detached_child_completes_real_later_work_and_replays_original_source() {
    let fixture =
        ordinary_child_source_boundary_with_policy("detach-work", CancelPolicy::Detach).await;
    let f = &fixture.flow;
    assert_eq!(f.run, fixture.child);
    sqlx::query("GRANT INSERT ON leaf_collection_success_certifications TO sagat_app")
        .execute(&f.api.db.pool)
        .await
        .expect("disposable dormant source grant");
    assert_source_insert_privilege(f, true).await;
    let store = PgLeafCollectionSuccessStore::new(
        f.api.api_pool.clone(),
        DefinitionCache::new(Arc::new(JsonIrCodec), 8),
    );
    let created = store
        .certify_leaf_collection_success(&fixture.source)
        .await
        .expect("genuine child source before detachment");
    assert!(!created.replayed);
    assert_eq!(created.record.identity, fixture.source.identity);
    assert_eq!(
        created.record.certified_run_revision,
        fixture.source.expected_run_revision
    );
    assert_record_matches_actual_source(f, &created.record).await;
    assert_persisted_certificate_and_audit(f, &created.record).await;
    assert_eq!(certification_counts(f).await, (1, 1));
    let certified = ordinary_child_source_snapshot(f, fixture.parent).await;
    sqlx::query("REVOKE INSERT ON leaf_collection_success_certifications FROM sagat_app")
        .execute(&f.api.db.pool)
        .await
        .expect("remove disposable source grant before all replay");
    assert_source_insert_privilege(f, false).await;

    let response = f
        .api
        .as_user(
            Method::POST,
            &format!("/runs/{}/operations", fixture.parent),
            "alice",
            "acme",
        )
        .header("x-octopus-namespace", "payments")
        .header("idempotency-key", Uuid::now_v7().to_string())
        .json(&json!({"action":"CANCEL","reason":"detached child progress proof"}))
        .send()
        .await
        .expect("public DETACH parent cancellation");
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let response = crate::support::api::json_of(response).await;
    assert_eq!(response["runStatus"], "CANCELLED");
    let relation: (String, bool, String, bool, bool, bool, Option<String>, bool) = sqlx::query_as(
        "SELECT p.status,p.finished_at IS NOT NULL,k.status,k.routed,\
             c.parent_cancel,c.parent_settled,c.stop_reason,c.finished_at IS NOT NULL \
             FROM runs c JOIN runs p ON p.tenant_id=c.tenant_id AND p.id=c.parent_run_id \
             JOIN steps k ON k.tenant_id=p.tenant_id AND k.run_id=p.id AND k.node='kid' \
             WHERE c.tenant_id=$1 AND c.id=$2 AND p.id=$3",
    )
    .bind(f.tenant.as_uuid())
    .bind(fixture.child.as_uuid())
    .bind(fixture.parent.as_uuid())
    .fetch_one(&f.api.db.pool)
    .await
    .expect("actual detached relationship before child progress");
    assert_eq!(
        relation,
        (
            "CANCELLED".into(),
            true,
            "CANCELLED".into(),
            true,
            false,
            true,
            None,
            false,
        )
    );
    let detached = ordinary_child_source_snapshot(f, fixture.parent).await;
    assert_eq!(detached.selected_source, certified.selected_source);
    assert_eq!(detached.certificate, certified.certificate);
    assert_eq!(detached.steps, certified.steps);
    assert_ne!(detached.parent_flags, certified.parent_flags);
    let parent = settled_parent(f, fixture.parent).await;
    let before = command(f).await;
    assert_eq!(
        before.expected_run_revision,
        fixture.source.expected_run_revision
    );
    assert_later_task_pending(f, &fixture.later).await;

    // Only a real worker report runs here. No scheduler tick may finish the child or notify P.
    let expected_output = json!({"detachedWork":"accepted"});
    f.complete(&fixture.later, expected_output.clone()).await;
    let current = command(f).await;
    assert!(current.expected_run_revision > before.expected_run_revision);
    let work = sqlx::query(
        "SELECT s.step_id,s.status,p.content,s.lease_token_hash IS NULL AS token_cleared,\
         s.lease_expires_at IS NULL AS expiry_cleared,s.leased_by IS NULL AS owner_cleared,\
         r.finished_at IS NULL AS unfinished,r.stop_reason,r.parent_cancel,r.parent_settled \
         FROM steps s JOIN runs r ON r.tenant_id=s.tenant_id AND r.id=s.run_id \
         JOIN payloads p ON p.tenant_id=s.tenant_id AND p.digest=s.output_digest \
         WHERE s.tenant_id=$1 AND s.run_id=$2 AND s.node='work' \
          AND s.instance_path_nodes=ARRAY['later']::text[] \
          AND s.instance_path_indices=ARRAY[0]::integer[]",
    )
    .bind(f.tenant.as_uuid())
    .bind(fixture.child.as_uuid())
    .fetch_one(&f.api.db.pool)
    .await
    .expect("genuine later completion with retained output");
    assert_eq!(
        work.get::<Uuid, _>("step_id").to_string(),
        fixture.later.step_id
    );
    assert_eq!(work.get::<String, _>("status"), "SUCCEEDED");
    assert_eq!(
        serde_json::from_slice::<Value>(&work.get::<Vec<u8>, _>("content"))
            .expect("actual retained later output"),
        expected_output
    );
    for field in [
        "token_cleared",
        "expiry_cleared",
        "owner_cleared",
        "unfinished",
    ] {
        assert!(
            work.get::<bool, _>(field),
            "actual completion invariant: {field}"
        );
    }
    assert_eq!(work.get::<Option<String>, _>("stop_reason"), None);
    assert!(!work.get::<bool, _>("parent_cancel"));
    assert!(work.get::<bool, _>("parent_settled"));
    let notifications: (i64, i64) = sqlx::query_as(
        "SELECT \
         (SELECT count(*) FROM run_events WHERE tenant_id=$1 AND run_id=$2 \
          AND event_type='run.finished'),\
         (SELECT count(*) FROM run_events WHERE tenant_id=$1 AND run_id=$3 \
          AND event_type='child.finished')",
    )
    .bind(f.tenant.as_uuid())
    .bind(fixture.child.as_uuid())
    .bind(fixture.parent.as_uuid())
    .fetch_one(&f.api.db.pool)
    .await
    .expect("no terminal/fanout overshoot");
    assert_eq!(notifications, (0, 0));
    let progressed = ordinary_child_source_snapshot(f, fixture.parent).await;
    assert_eq!(progressed.selected_source, detached.selected_source);
    assert_eq!(progressed.certificate, detached.certificate);
    assert_eq!(progressed.parent_flags, detached.parent_flags);
    assert_ne!(progressed.run_and_journal, detached.run_and_journal);
    assert_ne!(progressed.steps, detached.steps);
    assert_eq!(settled_parent(f, fixture.parent).await, parent);
    assert_record_matches_actual_source(f, &created.record).await;

    // Preserve the original creation CAS; only current own-child scheduler authority is refreshed.
    let mut original = fixture.source.clone();
    original.scheduler = current.scheduler;
    let pool = f.api.db.runtime_pool_with(2).await;
    let role: String = sqlx::query_scalar("SELECT current_user")
        .fetch_one(&pool)
        .await
        .expect("effective cold replay role");
    assert_eq!(role, "sagat_app");
    let cold = PgLeafCollectionSuccessStore::new(
        pool.clone(),
        DefinitionCache::new(Arc::new(JsonIrCodec), 8),
    );
    let replay = cold
        .certify_leaf_collection_success(&original)
        .await
        .expect("cold original-record replay after detached child progress");
    assert!(replay.replayed);
    assert_eq!(replay.record, created.record);
    assert_eq!(certification_counts(f).await, (1, 1));
    assert_eq!(
        ordinary_child_source_snapshot(f, fixture.parent).await,
        progressed
    );
    assert_eq!(settled_parent(f, fixture.parent).await, parent);
    f.verify().await;
    f.api
        .executions
        .verify_run(f.tenant, fixture.parent)
        .await
        .expect("settled parent journal remains valid after detached progress");
    drop(cold);
    pool.close().await;
}
