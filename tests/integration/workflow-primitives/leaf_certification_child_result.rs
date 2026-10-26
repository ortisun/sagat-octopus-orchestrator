//! One genuine result-bearing ordinary child composition through the collection terminal path.

use sagat_application::{
    execution::DefinitionCache, leaf_certification::LeafCollectionSuccessStore,
};
use sagat_domain::{execution::RunId, workflow::CancelPolicy};
use sagat_infra_ir::JsonIrCodec;
use sagat_infra_postgres::PgLeafCollectionSuccessStore;
use serde_json::{Value, json};
use sqlx::Row;
use std::sync::Arc;
use uuid::Uuid;

use crate::support::scope_flow::Flow;

fn result_child_document(name: &str) -> Value {
    let mut document = super::super::document(name);
    let end = document["nodes"]
        .as_array_mut()
        .expect("child root nodes")
        .iter_mut()
        .find(|node| node["id"].as_str() == Some("done"))
        .expect("child End");
    end["outputExpr"] = json!("[steps.lines.output[0], steps.later.output[0]][1].amount");
    document
}

fn json_content(bytes: &[u8]) -> Value {
    serde_json::from_slice(bytes).expect("retained JSON")
}

async fn assert_child_result(f: &Flow, child: RunId) -> Vec<u8> {
    let owners = sqlx::query(
        "SELECT scope.owner_node,scope.result_digest,owner.output_digest,payload.content
           FROM iteration_scopes scope LEFT JOIN steps owner
             ON owner.tenant_id=scope.tenant_id AND owner.run_id=scope.run_id
            AND owner.instance_path_nodes=scope.owner_path_nodes
            AND owner.instance_path_indices=scope.owner_path_indices
            AND owner.node=scope.owner_node
           LEFT JOIN payloads payload
             ON payload.tenant_id=owner.tenant_id AND payload.digest=owner.output_digest
          WHERE scope.tenant_id=$1 AND scope.run_id=$2
            AND scope.owner_path_nodes='{}'::text[] AND scope.owner_path_indices='{}'::integer[]
            AND scope.owner_node IN ('lines','later') ORDER BY scope.owner_node",
    )
    .bind(f.tenant.as_uuid())
    .bind(child.as_uuid())
    .fetch_all(&f.api.db.pool)
    .await
    .expect("both actual child collection owner outputs");
    assert_eq!(owners.len(), 2);
    for owner in owners {
        let node: String = owner.get("owner_node");
        assert_eq!(
            owner.get::<Option<Vec<u8>>, _>("output_digest"),
            owner.get::<Option<Vec<u8>>, _>("result_digest"),
            "{node} owner retains its collection aggregate digest"
        );
        let content = owner
            .get::<Option<Vec<u8>>, _>("content")
            .expect("owner aggregate is retained, not only an evaluator overlay");
        let expected = match node.as_str() {
            "lines" => json!([{"accepted": true}]),
            "later" => json!([{"amount": 17}]),
            other => panic!("unexpected collection owner {other}"),
        };
        assert_eq!(json_content(&content), expected);
    }
    let row = sqlx::query(
        "SELECT run.status,run.stop_reason,run.stop_code,scope.lifecycle, \
                aggregate.content AS aggregate_content,ended.status AS end_status, \
                ended.output_digest AS end_digest,end_payload.content AS end_content, \
                run.result_digest AS run_digest,result_payload.content AS run_content, \
                (SELECT count(*) FROM run_events event WHERE event.tenant_id=run.tenant_id \
                  AND event.run_id=run.id AND event.event_type='end.reached' \
                  AND event.node='done' AND event.instance_path_nodes='{}' \
                  AND event.instance_path_indices='{}') AS end_events, \
                (SELECT event.payload_digest FROM run_events event \
                  WHERE event.tenant_id=run.tenant_id AND event.run_id=run.id \
                    AND event.event_type='end.reached' AND event.node='done' \
                    AND event.instance_path_nodes='{}' AND event.instance_path_indices='{}') \
                  AS end_event_digest, \
                (SELECT count(*) FROM run_events event WHERE event.tenant_id=run.tenant_id \
                  AND event.run_id=run.id AND event.event_type='run.finished' \
                  AND event.detail->>'status'='SUCCEEDED') AS finished_events \
           FROM runs run JOIN iteration_scopes scope \
             ON scope.tenant_id=run.tenant_id AND scope.run_id=run.id \
            AND scope.owner_path_nodes='{}' AND scope.owner_path_indices='{}' \
            AND scope.owner_node='later' \
           JOIN payloads aggregate ON aggregate.tenant_id=scope.tenant_id \
            AND aggregate.digest=scope.result_digest \
           JOIN steps ended ON ended.tenant_id=run.tenant_id AND ended.run_id=run.id \
            AND ended.instance_path_nodes='{}' AND ended.instance_path_indices='{}' \
            AND ended.node='done' \
           JOIN payloads end_payload ON end_payload.tenant_id=ended.tenant_id \
            AND end_payload.digest=ended.output_digest \
           JOIN payloads result_payload ON result_payload.tenant_id=run.tenant_id \
            AND result_payload.digest=run.result_digest \
          WHERE run.tenant_id=$1 AND run.id=$2",
    )
    .bind(f.tenant.as_uuid())
    .bind(child.as_uuid())
    .fetch_one(&f.api.db.pool)
    .await
    .expect("actual child collection and End result");

    assert_eq!(row.get::<String, _>("status"), "SUCCEEDED");
    assert_eq!(row.get::<Option<String>, _>("stop_reason"), None);
    assert_eq!(row.get::<Option<String>, _>("stop_code"), None);
    assert_eq!(row.get::<String, _>("lifecycle"), "SUCCEEDED");
    assert_eq!(row.get::<String, _>("end_status"), "SUCCEEDED");
    assert_eq!(
        json_content(&row.get::<Vec<u8>, _>("aggregate_content")),
        json!([{"amount": 17}]),
        "the later collection retains its aggregate"
    );
    assert_eq!(
        json_content(&row.get::<Vec<u8>, _>("end_content")),
        json!(17),
        "the child End evaluates a scalar, not the collection aggregate"
    );
    assert_eq!(
        json_content(&row.get::<Vec<u8>, _>("run_content")),
        json!(17)
    );
    let end_digest = row.get::<Vec<u8>, _>("end_digest");
    let run_digest = row.get::<Vec<u8>, _>("run_digest");
    assert_eq!(end_digest, run_digest);
    assert_eq!(
        row.get::<Option<Vec<u8>>, _>("end_event_digest"),
        Some(run_digest.clone()),
        "EndReached names the retained child Run result"
    );
    assert_eq!(
        (
            row.get::<i64, _>("end_events"),
            row.get::<i64, _>("finished_events")
        ),
        (1, 1)
    );
    run_digest
}

async fn assert_parent_result(f: &Flow, parent: RunId, child: RunId, child_result_digest: &[u8]) {
    let row = sqlx::query(
        "SELECT parent.status,parent.stop_reason,parent.stop_code, \
                parent.result_digest AS run_digest,run_payload.content AS run_content, \
                child.parent_settled,child.parent_cancel, \
                kid.status AS kid_status,kid.waiting_kind,kid.routed AS kid_routed, \
                kid.child_run_id,kid.child_started,kid.output_digest AS kid_digest, \
                kid_payload.content AS kid_content,ended.status AS end_status, \
                ended.output_digest AS end_digest,end_payload.content AS end_content, \
                (SELECT count(*) FROM run_events event WHERE event.tenant_id=parent.tenant_id \
                  AND event.run_id=parent.id AND event.event_type='child.finished' \
                  AND event.node='kid' AND event.instance_path_nodes='{}' \
                  AND event.instance_path_indices='{}') AS child_finished_events, \
                (SELECT event.payload_digest FROM run_events event \
                  WHERE event.tenant_id=parent.tenant_id AND event.run_id=parent.id \
                    AND event.event_type='child.finished' AND event.node='kid' \
                    AND event.instance_path_nodes='{}' AND event.instance_path_indices='{}') \
                  AS child_finished_digest, \
                (SELECT event.detail FROM run_events event \
                  WHERE event.tenant_id=parent.tenant_id AND event.run_id=parent.id \
                    AND event.event_type='child.finished' AND event.node='kid' \
                    AND event.instance_path_nodes='{}' AND event.instance_path_indices='{}') \
                  AS child_finished_detail, \
                (SELECT count(*) FROM run_events event WHERE event.tenant_id=parent.tenant_id \
                  AND event.run_id=parent.id AND event.event_type='end.reached' \
                  AND event.node='done' AND event.instance_path_nodes='{}' \
                  AND event.instance_path_indices='{}') AS end_events, \
                (SELECT event.payload_digest FROM run_events event \
                  WHERE event.tenant_id=parent.tenant_id AND event.run_id=parent.id \
                    AND event.event_type='end.reached' AND event.node='done' \
                    AND event.instance_path_nodes='{}' AND event.instance_path_indices='{}') \
                  AS end_event_digest, \
                (SELECT count(*) FROM run_events event WHERE event.tenant_id=parent.tenant_id \
                  AND event.run_id=parent.id AND event.event_type='run.finished' \
                  AND event.detail->>'status'='SUCCEEDED') AS finished_events \
           FROM runs parent JOIN runs child \
             ON child.tenant_id=parent.tenant_id AND child.id=$3 AND child.parent_run_id=parent.id \
           JOIN steps kid ON kid.tenant_id=parent.tenant_id AND kid.run_id=parent.id \
            AND kid.instance_path_nodes='{}' AND kid.instance_path_indices='{}' AND kid.node='kid' \
           JOIN payloads kid_payload ON kid_payload.tenant_id=kid.tenant_id \
            AND kid_payload.digest=kid.output_digest \
           JOIN steps ended ON ended.tenant_id=parent.tenant_id AND ended.run_id=parent.id \
            AND ended.instance_path_nodes='{}' AND ended.instance_path_indices='{}' \
            AND ended.node='done' \
           JOIN payloads end_payload ON end_payload.tenant_id=ended.tenant_id \
            AND end_payload.digest=ended.output_digest \
           JOIN payloads run_payload ON run_payload.tenant_id=parent.tenant_id \
            AND run_payload.digest=parent.result_digest \
          WHERE parent.tenant_id=$1 AND parent.id=$2",
    )
    .bind(f.tenant.as_uuid())
    .bind(parent.as_uuid())
    .bind(child.as_uuid())
    .fetch_one(&f.api.db.pool)
    .await
    .expect("actual parent child-result fanout and End result");

    assert_eq!(row.get::<String, _>("status"), "SUCCEEDED");
    assert_eq!(row.get::<Option<String>, _>("stop_reason"), None);
    assert_eq!(row.get::<Option<String>, _>("stop_code"), None);
    assert!(row.get::<bool, _>("parent_settled"));
    assert!(!row.get::<bool, _>("parent_cancel"));
    assert_eq!(row.get::<String, _>("kid_status"), "SUCCEEDED");
    assert_eq!(row.get::<Option<String>, _>("waiting_kind"), None);
    assert!(row.get::<bool, _>("kid_routed"));
    assert_eq!(
        row.get::<Option<Uuid>, _>("child_run_id"),
        Some(child.as_uuid())
    );
    assert!(row.get::<bool, _>("child_started"));
    assert_eq!(row.get::<String, _>("end_status"), "SUCCEEDED");

    let kid_digest = row.get::<Vec<u8>, _>("kid_digest");
    let end_digest = row.get::<Vec<u8>, _>("end_digest");
    let run_digest = row.get::<Vec<u8>, _>("run_digest");
    assert_eq!(kid_digest.as_slice(), child_result_digest);
    assert_eq!(end_digest.as_slice(), kid_digest.as_slice());
    assert_eq!(run_digest.as_slice(), kid_digest.as_slice());
    for column in ["kid_content", "end_content", "run_content"] {
        assert_eq!(json_content(&row.get::<Vec<u8>, _>(column)), json!(17));
    }
    assert_eq!(row.get::<i64, _>("child_finished_events"), 1);
    assert_eq!(
        row.get::<Option<Vec<u8>>, _>("child_finished_digest"),
        Some(kid_digest.clone()),
        "the one child.finished event carries the child's actual result"
    );
    assert_eq!(
        row.get::<Option<Value>, _>("child_finished_detail"),
        Some(json!({"child": child.to_string(), "outcome": null}))
    );
    assert_eq!(
        row.get::<Option<Vec<u8>>, _>("end_event_digest"),
        Some(kid_digest),
        "the parent EndReached names the propagated result"
    );
    assert_eq!(
        (
            row.get::<i64, _>("end_events"),
            row.get::<i64, _>("finished_events")
        ),
        (1, 1)
    );
}

#[tokio::test]
async fn collection_child_end_result_reaches_its_parent_end() {
    // `result` keeps every generated workflow and worker slug within the 63-byte bound.
    let fixture = super::ordinary_child_source_boundary_with_parent_end(
        "result",
        CancelPolicy::Propagate,
        result_child_document,
        Some("steps.kid.output"),
    )
    .await;
    assert_eq!(fixture.flow.run, fixture.child);
    let before = super::ordinary_child_source_snapshot(&fixture.flow, fixture.parent).await;

    // Disposable certification activation only; it is revoked before later work or fanout.
    sqlx::query("GRANT INSERT ON leaf_collection_success_certifications TO sagat_app")
        .execute(&fixture.flow.api.db.pool)
        .await
        .expect("temporary ordinary-child source writer");
    super::assert_source_insert_privilege(&fixture.flow, true).await;
    let store = PgLeafCollectionSuccessStore::new(
        fixture.flow.api.api_pool.clone(),
        DefinitionCache::new(Arc::new(JsonIrCodec), 8),
    );
    let certified = store
        .certify_leaf_collection_success(&fixture.source)
        .await
        .expect("certify the genuine selected child source");
    assert!(!certified.replayed);
    assert_eq!(certified.record.identity, fixture.source.identity);
    super::assert_record_matches_actual_source(&fixture.flow, &certified.record).await;
    super::assert_persisted_certificate_and_audit(&fixture.flow, &certified.record).await;
    assert_eq!(super::certification_counts(&fixture.flow).await, (1, 1));
    let after_certification =
        super::ordinary_child_source_snapshot(&fixture.flow, fixture.parent).await;
    assert_eq!(after_certification.selected_source, before.selected_source);
    assert_ne!(after_certification.certificate, before.certificate);
    assert_eq!(after_certification.run_and_journal, before.run_and_journal);
    assert_eq!(after_certification.steps, before.steps);
    assert_eq!(after_certification.parent_flags, before.parent_flags);

    sqlx::query("REVOKE INSERT ON leaf_collection_success_certifications FROM sagat_app")
        .execute(&fixture.flow.api.db.pool)
        .await
        .expect("remove the temporary ordinary-child source writer");
    super::assert_source_insert_privilege(&fixture.flow, false).await;

    fixture
        .flow
        .complete(&fixture.later, json!({"amount": 17}))
        .await;
    fixture.flow.drive().await;
    let child_result_digest = assert_child_result(&fixture.flow, fixture.child).await;
    assert_parent_result(
        &fixture.flow,
        fixture.parent,
        fixture.child,
        &child_result_digest,
    )
    .await;

    let terminal = super::ordinary_child_source_snapshot(&fixture.flow, fixture.parent).await;
    assert_eq!(
        terminal.selected_source,
        after_certification.selected_source
    );
    assert_eq!(terminal.certificate, after_certification.certificate);
    assert_ne!(
        terminal.run_and_journal, after_certification.run_and_journal,
        "real later completion and terminal result advance the child Run and journal"
    );

    // Rediscover the current own-child revision/fence, but retain the exact original source key.
    let mut current = super::super::command(&fixture.flow).await;
    current.identity = fixture.source.identity.clone();
    let cold_pool = fixture.flow.api.db.runtime_pool_with(2).await;
    let cold_role: String = sqlx::query_scalar("SELECT current_user")
        .fetch_one(&cold_pool)
        .await
        .expect("cold native role");
    assert_eq!(cold_role, "sagat_app");
    let cold_store = PgLeafCollectionSuccessStore::new(
        cold_pool.clone(),
        DefinitionCache::new(Arc::new(JsonIrCodec), 8),
    );
    let before_replay = super::ordinary_child_source_snapshot(&fixture.flow, fixture.parent).await;
    let replay = cold_store
        .certify_leaf_collection_success(&current)
        .await
        .expect("cold original-source record replay after result fanout");
    assert!(replay.replayed);
    assert_eq!(replay.record, certified.record);
    assert_eq!(super::certification_counts(&fixture.flow).await, (1, 1));
    assert_eq!(
        super::ordinary_child_source_snapshot(&fixture.flow, fixture.parent).await,
        before_replay,
        "cold canonical replay changes no child, source, certificate or parent projection"
    );
    drop(cold_store);
    cold_pool.close().await;

    fixture.flow.verify().await;
    fixture
        .flow
        .api
        .executions
        .verify_run(fixture.flow.tenant, fixture.parent)
        .await
        .expect("the real parent journal rebuilds with its propagated result");
}
