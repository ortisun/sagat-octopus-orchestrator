//! Root dispatch must not mistake schema-valid scoped sentinels for executable root waits.
//! Sentinels below are storage fixtures, not a native scoped wait producer or runtime proof.
use crate::support::engine::{World, ctx, document, wait_node, with, world};
use sagat_application::{
    dispatch::{ClaimedEvent, EventDispatchStore},
    events::PublishRequest,
};
use sagat_domain::{
    events::EventId,
    execution::{CorrelationKey, ExecutionInstanceId, InstancePath, RunId},
    workflow::{EventType, NodeId},
};
use sagat_infra_postgres::PgRunStore;
use serde_json::json;
use std::time::Duration;
use uuid::Uuid;

async fn fixture(
    name: &str,
    scoped_name: &str,
) -> (World, PgRunStore, RunId, ClaimedEvent, serde_json::Value) {
    let w = world().await;
    let (workflow, _) = w
        .publish(document(
            name,
            "pay",
            vec![
                with(
                    wait_node("pay", "payment.confirmed", "input.orderId", 300_000, None),
                    "next",
                    json!("done"),
                ),
                crate::support::engine::end_node("done"),
            ],
        ))
        .await;
    let run = w.start(workflow, &json!({"orderId":"order-1"})).await;
    w.assert_consistent(run).await;
    // Clone real wait metadata but give the sentinel a nonroot logical identity. The workflow
    // intentionally has no body: no receiver may treat this row as an authorized root wait.
    for index in 0..2_i32 {
        sqlx::query("INSERT INTO steps SELECT (jsonb_populate_record(NULL::steps,to_jsonb(s)||jsonb_build_object('step_id',$3::uuid,'node',$4::text,'instance_path_nodes',ARRAY['lines']::text[],'instance_path_indices',ARRAY[$5]::integer[],'updated_at',s.updated_at-interval '1 second'))).* FROM steps s WHERE s.tenant_id=$1 AND s.run_id=$2 AND s.node='pay' AND cardinality(s.instance_path_nodes)=0")
            .bind(w.admin.tenant_id().as_uuid()).bind(run.as_uuid()).bind(Uuid::new_v4()).bind(scoped_name).bind(index).execute(&w.db.pool).await.expect("schema-valid scoped selection sentinel");
    }
    w.events
        .publish(
            &w.admin,
            PublishRequest {
                namespace: "payments".into(),
                event_id: Some(EventId::parse("identity-event").expect("event")),
                event_type: EventType::parse("payment.confirmed").expect("type"),
                subject: None,
                correlation_key: Some(CorrelationKey::parse("order-1").expect("key")),
                occurred_at: None,
                data: b"{}".to_vec(),
                dedupe_ttl: None,
            },
            &ctx(),
        )
        .await
        .expect("real accepted event");
    let runtime = w.db.runtime_pool().await;
    let role: String = sqlx::query_scalar("SELECT current_user")
        .fetch_one(&runtime)
        .await
        .expect("matcher role");
    assert_eq!(role, "sagat_app");
    let matcher = PgRunStore::new(runtime, w.cache.clone());
    let mut events = matcher
        .claim_events(1, Duration::from_secs(60))
        .await
        .expect("native claim");
    assert_eq!(events.len(), 1);
    let event = events.remove(0);
    let before = metadata_snapshot(&w, run).await;
    (w, matcher, run, event, before)
}

// Keep the sentinels until the owned database is dropped: source rows are append-only.
// They are intentionally not executable workflow state, so compare untouched persisted metadata
// after the operation rather than deleting them to manufacture a full-runtime replay claim.
async fn metadata_snapshot(w: &World, run: RunId) -> serde_json::Value {
    sqlx::query_scalar("SELECT jsonb_build_object('run',to_jsonb(r),'root',(SELECT to_jsonb(s)-'event_scanned_at' FROM steps s WHERE s.tenant_id=r.tenant_id AND s.run_id=r.id AND s.node='pay' AND cardinality(s.instance_path_nodes)=0),'scoped',(SELECT jsonb_agg(to_jsonb(s) ORDER BY s.instance_path_indices,s.node) FROM steps s WHERE s.tenant_id=r.tenant_id AND s.run_id=r.id AND cardinality(s.instance_path_nodes)>0),'journal',(SELECT count(*) FROM run_events e WHERE e.tenant_id=r.tenant_id AND e.run_id=r.id),'receipts',(SELECT count(*) FROM event_deliveries d WHERE d.tenant_id=r.tenant_id AND d.run_id=r.id)) FROM runs r WHERE r.tenant_id=$1 AND r.id=$2")
        .bind(w.admin.tenant_id().as_uuid()).bind(run.as_uuid()).fetch_one(&w.db.pool).await.expect("exact stored metadata snapshot")
}

async fn root_marked(w: &World, run: RunId) -> bool {
    sqlx::query_scalar("SELECT event_scanned_at IS NOT NULL FROM steps WHERE tenant_id=$1 AND run_id=$2 AND cardinality(instance_path_nodes)=0 AND node='pay'")
        .bind(w.admin.tenant_id().as_uuid()).bind(run.as_uuid()).fetch_one(&w.db.pool).await.expect("root marker")
}

#[tokio::test]
async fn event_discovery_pages_root_and_scoped_candidates_by_complete_identity() {
    let (w, matcher, run, event, before) = fixture("root-event-page-identity", "aaa").await;
    let first = matcher
        .waits_for_event(&event, None, 1)
        .await
        .expect("first root page");
    assert_eq!(first.len(), 1);
    assert_eq!(
        first[0].instance.node().as_str(),
        "pay",
        "scoped aaa must not consume the root page"
    );
    let next = matcher
        .waits_for_event(&event, Some((run, &first[0].instance)), 1)
        .await
        .expect("after root cursor");
    assert_eq!(next.len(), 1);
    assert_eq!(next[0].instance.node().as_str(), "aaa");
    assert!(!next[0].instance.path().is_root());
    let last = matcher
        .waits_for_event(&event, Some((run, &next[0].instance)), 1)
        .await
        .expect("second same-name scoped page");
    assert_eq!(last.len(), 1);
    assert_ne!(last[0].instance, next[0].instance);
    assert!(
        matcher
            .waits_for_event(&event, Some((run, &last[0].instance)), 1)
            .await
            .expect("finished candidate pages")
            .is_empty()
    );
    let unscanned = matcher
        .unscanned_waits(None, 1)
        .await
        .expect("qualified lookback discovery");
    assert_eq!(unscanned.len(), 1, "lookback remains bounded");
    assert_eq!(unscanned[0].instance.node().as_str(), "pay");
    assert!(unscanned[0].instance.path().is_root());
    assert_eq!(metadata_snapshot(&w, run).await, before);
}

#[tokio::test]
async fn root_scan_markers_refuse_siblings_foreign_scope_and_changed_bindings() {
    let (w, matcher, run, event, before) = fixture("root-event-marker-identity", "pay").await;
    let waits = matcher
        .waits_for_event(&event, None, 32)
        .await
        .expect("root candidate");
    let wait = waits.first().expect("one root candidate").clone();
    matcher
        .mark_wait_scanned(&wait)
        .await
        .expect("mark exact root");
    assert!(root_marked(&w, run).await, "a current root wait is marked");
    let scoped:i64=sqlx::query_scalar("SELECT count(*) FROM steps WHERE tenant_id=$1 AND run_id=$2 AND cardinality(instance_path_nodes)>0 AND event_scanned_at IS NOT NULL")
        .bind(w.admin.tenant_id().as_uuid()).bind(run.as_uuid()).fetch_one(&w.db.pool).await.expect("sibling scan markers");
    assert_eq!(
        scoped, 0,
        "root scan may not mark same-name scoped siblings"
    );
    sqlx::query("UPDATE steps SET event_scanned_at=NULL WHERE tenant_id=$1 AND run_id=$2 AND cardinality(instance_path_nodes)=0")
        .bind(w.admin.tenant_id().as_uuid()).bind(run.as_uuid()).execute(&w.db.pool).await.expect("reset owned optimization marker");
    let mut foreign = wait.clone();
    foreign.tenant_id = w.other_admin.tenant_id();
    matcher
        .mark_wait_scanned(&foreign)
        .await
        .expect("foreign tenant is inert");
    foreign = wait.clone();
    foreign.namespace_id = w.other_namespace.id();
    matcher
        .mark_wait_scanned(&foreign)
        .await
        .expect("foreign namespace is inert");
    assert!(
        !root_marked(&w, run).await,
        "foreign selection cannot mark the root"
    );
    // Each field changes independently, so removing any binding predicate is observable.
    for mutation in [
        "UPDATE steps SET correlation_key='changed-binding' WHERE tenant_id=$1 AND run_id=$2 AND cardinality(instance_path_nodes)=0 AND node='pay'",
        "UPDATE steps SET event_type='payment.other' WHERE tenant_id=$1 AND run_id=$2 AND cardinality(instance_path_nodes)=0 AND node='pay'",
        "UPDATE steps SET event_since=event_since + interval '1 second' WHERE tenant_id=$1 AND run_id=$2 AND cardinality(instance_path_nodes)=0 AND node='pay'",
    ] {
        sqlx::query(sqlx::AssertSqlSafe(mutation))
            .bind(w.admin.tenant_id().as_uuid())
            .bind(run.as_uuid())
            .execute(&w.db.pool)
            .await
            .expect("deliberate stale binding fixture");
        matcher
            .mark_wait_scanned(&wait)
            .await
            .expect("stale binding is inert");
        assert!(
            !root_marked(&w, run).await,
            "an old scan cannot conceal a changed binding"
        );
        sqlx::query("UPDATE steps SET correlation_key=$3,event_type=$4,event_since=$5 WHERE tenant_id=$1 AND run_id=$2 AND cardinality(instance_path_nodes)=0 AND node='pay'")
            .bind(w.admin.tenant_id().as_uuid()).bind(run.as_uuid()).bind(wait.correlation_key.as_str()).bind(wait.event_type.as_str()).bind(wait.since).execute(&w.db.pool).await.expect("restore owned binding fixture");
        assert_eq!(metadata_snapshot(&w, run).await, before);
    }
    assert_eq!(metadata_snapshot(&w, run).await, before);
}

#[tokio::test]
async fn scan_markers_refuse_changed_physical_or_logical_candidate_identity() {
    let (w, matcher, run, event, before) = fixture("root-event-candidate-cas", "pay").await;
    let wait = matcher
        .waits_for_event(&event, None, 1)
        .await
        .expect("candidate")
        .remove(0);
    let mut wrong_id = wait.clone();
    wrong_id.step_id = Uuid::new_v4();
    let mut wrong_path = wait.clone();
    wrong_path.instance = ExecutionInstanceId::in_path(
        InstancePath::root()
            .child(NodeId::parse("lines").expect("scope"), 0)
            .expect("path"),
        wait.instance.node().clone(),
    );
    assert!(
        matcher
            .undelivered_event_for(&wait)
            .await
            .expect("root lookback")
            .is_some()
    );
    assert!(
        matcher
            .undelivered_event_for(&wrong_path)
            .await
            .expect("nonexistent physical/logical scoped identity")
            .is_none()
    );
    let mut marked = Vec::new();
    for candidate in [wrong_id, wrong_path] {
        matcher
            .mark_wait_scanned(&candidate)
            .await
            .expect("stale identity is inert");
        marked.push(root_marked(&w, run).await);
        sqlx::query("UPDATE steps SET event_scanned_at=NULL WHERE tenant_id=$1 AND run_id=$2 AND cardinality(instance_path_nodes)=0")
            .bind(w.admin.tenant_id().as_uuid()).bind(run.as_uuid()).execute(&w.db.pool).await.expect("reset owned marker");
    }
    assert_eq!(
        marked,
        vec![false, false],
        "neither wrong UUID nor wrong path may mark root"
    );
    assert_eq!(metadata_snapshot(&w, run).await, before);
}

#[tokio::test]
async fn event_cursor_uses_complete_database_tuple_across_same_name_scoped_candidates() {
    let (w, matcher, run, event, before) = fixture("root-event-candidate-cursor", "aaa").await;
    let scoped = ExecutionInstanceId::in_path(
        InstancePath::root()
            .child(NodeId::parse("lines").expect("scope"), 0)
            .expect("path"),
        NodeId::parse("aaa").expect("node"),
    );
    // Empty path arrays sort before nonempty ones, irrespective of the local node name.
    let after = matcher
        .waits_for_event(&event, Some((run, &scoped)), 1)
        .await
        .expect("qualified cursor");
    assert_eq!(after.len(), 1);
    let next = ExecutionInstanceId::in_path(
        InstancePath::root()
            .child(NodeId::parse("lines").expect("scope"), 1)
            .expect("path"),
        NodeId::parse("aaa").expect("node"),
    );
    assert_eq!(
        after[0].instance, next,
        "the root is before the scoped cursor; only its next sibling follows"
    );
    assert_eq!(metadata_snapshot(&w, run).await, before);
}
