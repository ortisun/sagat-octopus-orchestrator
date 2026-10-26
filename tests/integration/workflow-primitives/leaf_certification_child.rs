//! Characterization of the dormant native certifier on one real ordinary child Run.
//!
//! Install as a child of `leaf_certification_native.rs`. This records behavior already accepted
//! by `PgLeafCollectionSuccessStore`: its OrdinaryLeafV1 source proof does not reject a Run merely
//! because that Run has a ParentLink. It adds no production caller, grant, source guarantee,
//! incoming-parent policy, chain/nested closure, race, or scoped Finish activation.

use super::*;
use reqwest::{Method, StatusCode};
use sagat_api_grpc::pb::TaskLease;
use sagat_application::leaf_certification::LeafCollectionSuccessRecord;
use sagat_application::scoped_leaf_finish::{
    ScopedLeafFinishCommand, ScopedLeafFinishError, ScopedLeafFinishExpectations,
    ScopedLeafFinishStore,
};
use sagat_application::{execution::child_run_id, runs::RunStore};
use sagat_domain::{
    execution::{CapacityGroupId, CapacityMemberId, RunId, RunStatus},
    identity::TenantId,
    workflow::CancelPolicy,
};
use sagat_infra_postgres::PgScopedLeafFinishStore;
use sqlx::PgPool;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct OrdinaryChildSourceSnapshot {
    pub(super) selected_source: String,
    pub(super) certificate: String,
    pub(super) run_and_journal: String,
    pub(super) steps: String,
    pub(super) parent_flags: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct OrdinarySourceRecordProjection {
    scope_id: Uuid,
    body: String,
    return_path_nodes: Vec<String>,
    return_path_indices: Vec<i32>,
    return_node: String,
    checkpoint_result_digest: Vec<u8>,
    aggregate_result_digest: Vec<u8>,
    admitted_sequence: i64,
    returned_sequence: i64,
    checkpoint_sequence: i64,
    finished_sequence: i64,
    owner_routed_sequence: i64,
}

/// Reusable public-runtime boundary for later parent-policy characterization.
///
/// The source `lines/0` is closed and locally quiescent. `later/0/work` has been leased and is
/// deliberately unfinished. `source` is discovered after that lease changed the child Run revision.
pub(super) struct OrdinaryChildSourceBoundary {
    pub(super) flow: Flow,
    pub(super) bootstrap: RunId,
    pub(super) parent: RunId,
    pub(super) child: RunId,
    pub(super) later: TaskLease,
    pub(super) source: CertifyLeafCollectionSuccess,
}

fn terminal_document(name: &str) -> Value {
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": {"name": name},
        "entrypoint": "done",
        "nodes": [{"id": "done", "type": "end"}]
    })
}

async fn start_public_parent_with_policy_and_document(
    f: &Flow,
    label: &str,
    cancel_policy: CancelPolicy,
    child_document: fn(&str) -> Value,
    parent_end_output_expr: Option<&str>,
) -> (RunId, RunId) {
    let suffix = Uuid::now_v7();
    let child_name = format!("ocs-c-{label}-{suffix}");
    crate::support::scope_flow::publish(&f.api, &child_name, child_document(&child_name))
        .await
        .expect("the ordinary two-collection child publishes through the public API");

    let parent_name = format!("ocs-p-{label}-{suffix}");
    let mut child_node =
        json!({"id": "kid", "type": "child", "workflowId": child_name, "next": "done"});
    child_node["cancelPolicy"] = json!(match cancel_policy {
        CancelPolicy::Propagate => "propagate",
        CancelPolicy::Detach => "detach",
        CancelPolicy::RequestCancel => "request_cancel",
    });
    let mut parent_end = json!({"id": "done", "type": "end"});
    if let Some(expression) = parent_end_output_expr {
        parent_end["outputExpr"] = json!(expression);
    }
    let parent_document = json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": {"name": parent_name.clone()},
        "entrypoint": "kid",
        "nodes": [
            child_node,
            parent_end
        ]
    });
    let parent_workflow =
        crate::support::scope_flow::publish(&f.api, &parent_name, parent_document)
            .await
            .expect("the parent publishes through the public API");
    let started = f
        .api
        .as_user(Method::POST, "/runs", "alice", "acme")
        .header("x-octopus-namespace", "payments")
        .header("idempotency-key", Uuid::now_v7().to_string())
        .json(&json!({
            "workflowId": parent_workflow,
            "input": {"lines": [{"v": 1}]}
        }))
        .send()
        .await
        .expect("start the public parent");
    assert_eq!(started.status(), StatusCode::ACCEPTED);
    let parent: RunId = crate::support::api::json_of(started).await["runId"]
        .as_str()
        .expect("public parent Run id")
        .parse()
        .expect("valid public parent Run id");
    let child = child_run_id(parent, &NodeId::parse("kid").expect("parent child node"));
    (parent, child)
}

async fn assert_ordinary_child_context(
    f: &Flow,
    parent: RunId,
    child: RunId,
    cancel_policy: CancelPolicy,
) {
    let parent_record = f
        .api
        .executions
        .get(f.tenant, parent)
        .await
        .expect("read actual public parent")
        .expect("the public parent exists");
    let record = f
        .api
        .executions
        .get(f.tenant, child)
        .await
        .expect("read actual public child")
        .expect("the scheduler created the deterministic child");
    let parent_link = record.info.parent.expect("actual immutable ParentLink");
    assert_eq!(parent_link.run, parent);
    assert_eq!(
        parent_link.node,
        NodeId::parse("kid").expect("parent child node")
    );
    assert_eq!(parent_link.cancel_policy, cancel_policy);
    assert_eq!(record.info.tenant_id, f.tenant);
    assert_eq!(record.info.namespace_id, parent_record.info.namespace_id);
    assert_eq!((record.info.depth, record.info.root), (1, parent));
    assert_eq!(record.status, RunStatus::Running);
    assert_eq!(
        record.chain, None,
        "the ordinary child is not a chain segment"
    );

    let topology: (i64, i64, i64) = sqlx::query_as(
        "SELECT \
         (SELECT count(*) FROM chain_segments WHERE tenant_id=$1 AND run_id=$2),\
         (SELECT count(*) FROM execution_chains WHERE tenant_id=$1 \
          AND (chain_id=$2 OR current_run_id=$2)),\
         (SELECT count(*) FROM child_chain_links WHERE tenant_id=$1 AND child_chain_id=$2)",
    )
    .bind(f.tenant.as_uuid())
    .bind(child.as_uuid())
    .fetch_one(&f.api.db.pool)
    .await
    .expect("exact ordinary-child chain absence");
    assert_eq!(topology, (0, 0, 0));
    let parent_step: (String, Option<String>, Option<Uuid>, bool) = sqlx::query_as(
        "SELECT status,waiting_kind,child_run_id,child_started FROM steps \
         WHERE tenant_id=$1 AND run_id=$2 AND node='kid' \
          AND instance_path_nodes='{}' AND instance_path_indices='{}'",
    )
    .bind(f.tenant.as_uuid())
    .bind(parent.as_uuid())
    .fetch_one(&f.api.db.pool)
    .await
    .expect("public parent waits on its real child step");
    assert_eq!(
        parent_step,
        (
            "WAITING".into(),
            Some("CHILD".into()),
            Some(child.as_uuid()),
            true,
        )
    );
}

async fn assert_closed_lines_source(f: &Flow) {
    let row = sqlx::query(
        "SELECT s.lifecycle AS scope_lifecycle,s.item_count,s.next_admission_index,\
         s.admitted_count,s.completed_count,s.active_count,\
         i.lifecycle AS iteration_lifecycle,i.slot_held,i.remaining_reservations,\
         i.return_node,c.return_instance_id=i.return_instance_id AS exact_return,\
         c.result_digest=i.result_digest AS exact_result,\
         c.journal_sequence=i.checkpoint_sequence AS exact_checkpoint,\
         owner.status AS owner_status,owner.routed AS owner_routed,\
         (SELECT count(*) FROM steps local WHERE local.tenant_id=s.tenant_id \
          AND local.run_id=s.run_id AND local.instance_path_nodes=ARRAY['lines']::text[] \
          AND local.instance_path_indices=ARRAY[0]::integer[]) AS local_steps,\
         (SELECT count(*) FROM steps local WHERE local.tenant_id=s.tenant_id \
          AND local.run_id=s.run_id AND local.instance_path_nodes=ARRAY['lines']::text[] \
          AND local.instance_path_indices=ARRAY[0]::integer[] \
          AND local.status='SUCCEEDED' AND local.routed) AS succeeded_routed_steps,\
         (SELECT count(*) FROM steps local WHERE local.tenant_id=s.tenant_id \
          AND local.run_id=s.run_id AND local.instance_path_nodes=ARRAY['lines']::text[] \
          AND local.instance_path_indices=ARRAY[0]::integer[] AND \
          (local.lease_token_hash IS NOT NULL OR local.lease_expires_at IS NOT NULL \
           OR local.leased_by IS NOT NULL OR local.attempt_deadline_at IS NOT NULL \
           OR local.next_attempt_at IS NOT NULL OR local.waiting_kind IS NOT NULL)) AS local_open_work,\
         (SELECT count(*) FROM scope_sagas saga WHERE saga.tenant_id=s.tenant_id \
          AND saga.run_id=s.run_id AND saga.owner_node='lines' \
          AND saga.owner_path_nodes='{}' AND saga.owner_path_indices='{}') AS local_sagas,\
         (SELECT count(*) FROM scope_saga_entries entry WHERE entry.tenant_id=s.tenant_id \
          AND entry.run_id=s.run_id AND entry.owner_node='lines' \
          AND entry.owner_path_nodes='{}' AND entry.owner_path_indices='{}' \
          AND entry.iteration_index=0) AS local_saga_entries \
         FROM iteration_scopes s \
         JOIN collection_iterations i ON i.tenant_id=s.tenant_id AND i.run_id=s.run_id \
          AND i.scope_id=s.scope_id AND i.iteration_index=0 \
         JOIN iteration_checkpoints c ON c.tenant_id=i.tenant_id AND c.run_id=i.run_id \
          AND c.scope_id=i.scope_id AND c.iteration_index=i.iteration_index \
         JOIN steps owner ON owner.tenant_id=s.tenant_id AND owner.run_id=s.run_id \
          AND owner.node='lines' AND owner.instance_path_nodes='{}' \
          AND owner.instance_path_indices='{}' \
         WHERE s.tenant_id=$1 AND s.run_id=$2 AND s.owner_node='lines' \
          AND s.owner_path_nodes='{}' AND s.owner_path_indices='{}'",
    )
    .bind(f.tenant.as_uuid())
    .bind(f.run.as_uuid())
    .fetch_one(&f.api.db.pool)
    .await
    .expect("exact closed ordinary child source");
    assert_eq!(row.get::<String, _>("scope_lifecycle"), "SUCCEEDED");
    assert_eq!(
        (
            row.get::<i32, _>("item_count"),
            row.get::<i32, _>("next_admission_index"),
            row.get::<i32, _>("admitted_count"),
            row.get::<i32, _>("completed_count"),
            row.get::<i32, _>("active_count"),
        ),
        (1, 1, 1, 1, 0)
    );
    assert_eq!(row.get::<String, _>("iteration_lifecycle"), "CHECKPOINTED");
    assert!(!row.get::<bool, _>("slot_held"));
    assert_eq!(row.get::<i32, _>("remaining_reservations"), 0);
    assert_eq!(row.get::<String, _>("return_node"), "returned");
    assert!(row.get::<bool, _>("exact_return"));
    assert!(row.get::<bool, _>("exact_result"));
    assert!(row.get::<bool, _>("exact_checkpoint"));
    assert_eq!(row.get::<String, _>("owner_status"), "SUCCEEDED");
    assert!(row.get::<bool, _>("owner_routed"));
    assert_eq!(
        (
            row.get::<i64, _>("local_steps"),
            row.get::<i64, _>("succeeded_routed_steps"),
            row.get::<i64, _>("local_open_work"),
            row.get::<i64, _>("local_sagas"),
            row.get::<i64, _>("local_saga_entries"),
        ),
        (2, 2, 0, 0, 0),
        "selected lines/0 is returned, checkpointed, routed and locally quiescent"
    );
}

async fn assert_later_task_pending(f: &Flow, later: &TaskLease) {
    assert_eq!(later.run_id, f.run.to_string());
    let instance = later.instance.as_ref().expect("later scoped task identity");
    assert_eq!(instance.definition_node_id, "work");
    assert_eq!(instance.instance_path.len(), 1);
    assert_eq!(instance.instance_path[0].scope_node_id, "later");
    assert_eq!(instance.instance_path[0].iteration_index, 0);
    let state: (String, String, String, bool, bool) = sqlx::query_as(
        "SELECT scope.lifecycle,iteration.lifecycle,step.status,iteration.slot_held,\
         step.lease_token_hash IS NOT NULL \
         FROM iteration_scopes scope \
         JOIN collection_iterations iteration USING(tenant_id,run_id,scope_id) \
         JOIN steps step ON step.tenant_id=scope.tenant_id AND step.run_id=scope.run_id \
          AND step.node='work' AND step.instance_path_nodes=ARRAY['later']::text[] \
          AND step.instance_path_indices=ARRAY[0]::integer[] \
         WHERE scope.tenant_id=$1 AND scope.run_id=$2 AND scope.owner_node='later' \
          AND scope.owner_path_nodes='{}' AND scope.owner_path_indices='{}' \
          AND iteration.iteration_index=0",
    )
    .bind(f.tenant.as_uuid())
    .bind(f.run.as_uuid())
    .fetch_one(&f.api.db.pool)
    .await
    .expect("actual later child work remains pending");
    assert_eq!(
        state,
        (
            "OPEN".into(),
            "ADMITTED".into(),
            "LEASED".into(),
            true,
            true
        )
    );
}

async fn ordinary_child_source_boundary_with_parent_end(
    label: &str,
    cancel_policy: CancelPolicy,
    child_document: fn(&str) -> Value,
    parent_end_output_expr: Option<&str>,
) -> OrdinaryChildSourceBoundary {
    let bootstrap_name = format!("ocs-b-{label}-{}", Uuid::now_v7());
    let mut flow = Flow::start(
        &bootstrap_name,
        terminal_document(&bootstrap_name),
        json!([]),
    )
    .await;
    let bootstrap = flow.run;
    flow.drive().await;
    assert_eq!(flow.run_status().await, "SUCCEEDED");
    flow.verify().await;

    // Retain the already acquired placement and worker, and enable the production child pass.
    flow.scheduler = flow
        .scheduler
        .with_children(DefinitionCache::new(Arc::new(JsonIrCodec), 8));
    let (parent, child) = start_public_parent_with_policy_and_document(
        &flow,
        label,
        cancel_policy,
        child_document,
        parent_end_output_expr,
    )
    .await;
    flow.drive().await;
    assert_ordinary_child_context(&flow, parent, child, cancel_policy).await;

    // The bootstrap is terminal, so this worker batch belongs only to the actual child.
    flow.run = child;
    let source_tasks = flow.poll().await;
    assert_eq!(source_tasks.len(), 1);
    let source_task = &source_tasks[0];
    assert_eq!(source_task.run_id, child.to_string());
    let source_instance = source_task
        .instance
        .as_ref()
        .expect("ordinary child source identity");
    assert_eq!(source_instance.definition_node_id, "work");
    assert_eq!(source_instance.instance_path.len(), 1);
    assert_eq!(source_instance.instance_path[0].scope_node_id, "lines");
    assert_eq!(source_instance.instance_path[0].iteration_index, 0);
    let before_close = command(&flow).await;
    let preclose_store = PgLeafCollectionSuccessStore::new(
        flow.api.api_pool.clone(),
        DefinitionCache::new(Arc::new(JsonIrCodec), 8),
    );
    assert_eq!(
        preclose_store
            .certify_leaf_collection_success(&before_close)
            .await,
        Err(LeafCollectionSuccessError::SourceNotClosed)
    );
    assert_eq!(certification_counts(&flow).await, (0, 0));
    flow.complete(source_task, json!({"accepted": true})).await;
    flow.drive().await;

    // Polling is a real StepLeased writer and advances the child revision. Discover the command
    // only after this exact source boundary has been created and the later work has been leased.
    let later_tasks = flow.poll().await;
    assert_eq!(later_tasks.len(), 1);
    let later = later_tasks
        .into_iter()
        .next()
        .expect("one later child task");
    assert_later_task_pending(&flow, &later).await;
    assert_closed_lines_source(&flow).await;
    flow.verify().await;
    let source = command(&flow).await;

    OrdinaryChildSourceBoundary {
        flow,
        bootstrap,
        parent,
        child,
        later,
        source,
    }
}

pub(super) async fn ordinary_child_source_boundary_with_document(
    label: &str,
    cancel_policy: CancelPolicy,
    child_document: fn(&str) -> Value,
) -> OrdinaryChildSourceBoundary {
    ordinary_child_source_boundary_with_parent_end(label, cancel_policy, child_document, None).await
}

pub(super) async fn ordinary_child_source_boundary_with_policy(
    label: &str,
    cancel_policy: CancelPolicy,
) -> OrdinaryChildSourceBoundary {
    ordinary_child_source_boundary_with_document(label, cancel_policy, document).await
}

pub(super) async fn ordinary_child_source_boundary(label: &str) -> OrdinaryChildSourceBoundary {
    ordinary_child_source_boundary_with_policy(label, CancelPolicy::Propagate).await
}

fn record_source_projection(
    record: &LeafCollectionSuccessRecord,
) -> OrdinarySourceRecordProjection {
    let [segment] = record.return_instance.path().segments() else {
        panic!("certified return identity is exactly depth one");
    };
    OrdinarySourceRecordProjection {
        scope_id: record.scope_id.as_uuid(),
        body: record.body.as_str().to_owned(),
        return_path_nodes: vec![segment.scope_node().as_str().to_owned()],
        return_path_indices: vec![
            i32::try_from(segment.iteration_index()).expect("bounded return index"),
        ],
        return_node: record.return_instance.node().as_str().to_owned(),
        checkpoint_result_digest: record.checkpoint_result_digest.as_bytes().to_vec(),
        aggregate_result_digest: record.aggregate_result_digest.as_bytes().to_vec(),
        admitted_sequence: i64::try_from(record.admitted_sequence).expect("admitted sequence"),
        returned_sequence: i64::try_from(record.returned_sequence).expect("returned sequence"),
        checkpoint_sequence: i64::try_from(record.checkpoint_sequence)
            .expect("checkpoint sequence"),
        finished_sequence: i64::try_from(record.finished_sequence).expect("finished sequence"),
        owner_routed_sequence: i64::try_from(record.owner_routed_sequence)
            .expect("routed sequence"),
    }
}

fn lower_hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut hex, byte| {
            use std::fmt::Write as _;
            write!(hex, "{byte:02x}").expect("writing into a String cannot fail");
            hex
        })
}

async fn assert_record_matches_actual_source(f: &Flow, record: &LeafCollectionSuccessRecord) {
    let source = sqlx::query(
        "SELECT s.scope_id,i.return_path_nodes,i.return_path_indices,i.return_node,\
         i.return_instance_id,c.return_instance_id AS checkpoint_return_instance_id,\
         returned.step_id AS returned_step_id,i.item_digest,i.admitted_sequence,\
         i.checkpoint_sequence AS iteration_checkpoint_sequence,\
         c.journal_sequence AS checkpoint_sequence,\
         c.result_digest AS checkpoint_result_digest,s.result_digest AS aggregate_result_digest,\
         s.finish_sequence,owner.step_id AS owner_instance_id \
         FROM iteration_scopes s \
         JOIN collection_iterations i ON i.tenant_id=s.tenant_id AND i.run_id=s.run_id \
          AND i.scope_id=s.scope_id AND i.iteration_index=0 \
         JOIN iteration_checkpoints c ON c.tenant_id=i.tenant_id AND c.run_id=i.run_id \
          AND c.scope_id=i.scope_id AND c.iteration_index=i.iteration_index \
         JOIN steps owner ON owner.tenant_id=s.tenant_id AND owner.run_id=s.run_id \
          AND owner.node='lines' AND owner.kind='FORWARD' AND owner.instance_path_nodes='{}' \
          AND owner.instance_path_indices='{}' \
         JOIN steps returned ON returned.tenant_id=i.tenant_id AND returned.run_id=i.run_id \
          AND returned.node=i.return_node AND returned.kind='FORWARD' \
          AND returned.instance_path_nodes=i.return_path_nodes \
          AND returned.instance_path_indices=i.return_path_indices \
         WHERE s.tenant_id=$1 AND s.run_id=$2 AND s.namespace_id=$3 AND s.owner_node='lines' \
          AND s.owner_path_nodes='{}' AND s.owner_path_indices='{}'",
    )
    .bind(record.identity.tenant_id().as_uuid())
    .bind(record.identity.run_id().as_uuid())
    .bind(record.identity.namespace_id().as_uuid())
    .fetch_one(&f.api.db.pool)
    .await
    .expect("independent actual lines/0 source projection");
    let return_instance_id = source.get::<Uuid, _>("return_instance_id");
    assert_eq!(
        (
            source.get::<Uuid, _>("checkpoint_return_instance_id"),
            source.get::<Uuid, _>("returned_step_id"),
        ),
        (return_instance_id, return_instance_id),
        "actual iteration, checkpoint and returned step retain one instance identity"
    );
    assert_eq!(
        source.get::<i64, _>("iteration_checkpoint_sequence"),
        source.get::<i64, _>("checkpoint_sequence"),
        "actual iteration and checkpoint retain one journal reference"
    );

    let document: Vec<u8> = sqlx::query_scalar(
        "SELECT v.document FROM runs r JOIN workflow_versions v \
         ON v.tenant_id=r.tenant_id AND v.id=r.workflow_version_id \
         WHERE r.tenant_id=$1 AND r.namespace_id=$2 AND r.id=$3",
    )
    .bind(record.identity.tenant_id().as_uuid())
    .bind(record.identity.namespace_id().as_uuid())
    .bind(record.identity.run_id().as_uuid())
    .fetch_one(&f.api.db.pool)
    .await
    .expect("actual pinned child definition document");
    let document: Value = serde_json::from_slice(&document).expect("published definition JSON");
    let body = document["nodes"]
        .as_array()
        .and_then(|nodes| {
            nodes
                .iter()
                .find(|node| node["id"].as_str() == Some("lines"))
        })
        .and_then(|node| node["body"].as_str())
        .expect("actual lines owner names its ordinary body")
        .to_owned();

    let item_digest = source.get::<Vec<u8>, _>("item_digest");
    let checkpoint_digest = source.get::<Vec<u8>, _>("checkpoint_result_digest");
    let aggregate_digest = source.get::<Vec<u8>, _>("aggregate_result_digest");
    let owner_instance_id = source.get::<Uuid, _>("owner_instance_id");
    let events = sqlx::query(
        "SELECT seq,event_type,node,instance_id,instance_path_nodes,instance_path_indices,detail \
         FROM run_events WHERE tenant_id=$1 AND run_id=$2 AND node='lines' \
          AND event_type=ANY($3::text[]) ORDER BY seq",
    )
    .bind(record.identity.tenant_id().as_uuid())
    .bind(record.identity.run_id().as_uuid())
    .bind(vec![
        "collection.iteration_admitted".to_owned(),
        "collection.iteration_returned".to_owned(),
        "collection.iteration_checkpointed".to_owned(),
        "collection.finished".to_owned(),
        "step.routed".to_owned(),
    ])
    .fetch_all(&f.api.db.pool)
    .await
    .expect("five independently selected lines events");
    assert_eq!(events.len(), 5);
    let mut admitted = None;
    let mut returned = None;
    let mut checkpointed = None;
    let mut finished = None;
    let mut routed = None;
    for event in events {
        assert_eq!(event.get::<String, _>("node"), "lines");
        assert_eq!(
            event.get::<Option<Uuid>, _>("instance_id"),
            Some(owner_instance_id)
        );
        assert_eq!(
            event.get::<Option<Vec<String>>, _>("instance_path_nodes"),
            Some(Vec::<String>::new())
        );
        assert_eq!(
            event.get::<Option<Vec<i32>>, _>("instance_path_indices"),
            Some(Vec::<i32>::new())
        );
        let sequence = event.get::<i64, _>("seq");
        let detail = event.get::<Value, _>("detail");
        match event.get::<String, _>("event_type").as_str() {
            "collection.iteration_admitted" => {
                assert_eq!(
                    detail,
                    json!({
                        "index": 0,
                        "entry": {"path": [{"scope": "lines", "index": 0}], "node": "work"},
                        "item_digest": lower_hex(&item_digest),
                    })
                );
                admitted = Some(sequence);
            }
            "collection.iteration_returned" => {
                assert_eq!(
                    detail,
                    json!({
                        "index": 0,
                        "return_instance": {
                            "path": [{"scope": "lines", "index": 0}], "node": "returned"
                        },
                    })
                );
                returned = Some(sequence);
            }
            "collection.iteration_checkpointed" => {
                assert_eq!(
                    detail,
                    json!({
                        "index": 0,
                        "return_instance": {
                            "path": [{"scope": "lines", "index": 0}], "node": "returned"
                        },
                        "result_digest": lower_hex(&checkpoint_digest),
                    })
                );
                checkpointed = Some(sequence);
            }
            "collection.finished" => {
                assert_eq!(
                    detail,
                    json!({
                        "outcome": "succeeded",
                        "result_digest": lower_hex(&aggregate_digest),
                    })
                );
                finished = Some(sequence);
            }
            "step.routed" => {
                assert_eq!(detail, json!({"activated": ["later"]}));
                routed = Some(sequence);
            }
            other => panic!("unexpected selected source event {other}"),
        }
    }
    let admitted = admitted.expect("actual admitted event");
    let returned = returned.expect("actual returned event");
    let checkpointed = checkpointed.expect("actual checkpointed event");
    let finished = finished.expect("actual finished event");
    let routed = routed.expect("actual routed event");
    assert_eq!(admitted, source.get::<i64, _>("admitted_sequence"));
    assert_eq!(checkpointed, source.get::<i64, _>("checkpoint_sequence"));
    assert_eq!(finished, source.get::<i64, _>("finish_sequence"));
    assert_eq!(returned.checked_add(1), Some(checkpointed));
    assert_eq!(finished.checked_add(1), Some(routed));

    let actual = OrdinarySourceRecordProjection {
        scope_id: source.get("scope_id"),
        body,
        return_path_nodes: source.get("return_path_nodes"),
        return_path_indices: source.get("return_path_indices"),
        return_node: source.get("return_node"),
        checkpoint_result_digest: checkpoint_digest,
        aggregate_result_digest: aggregate_digest,
        admitted_sequence: admitted,
        returned_sequence: returned,
        checkpoint_sequence: checkpointed,
        finished_sequence: finished,
        owner_routed_sequence: routed,
    };
    assert_eq!(record_source_projection(record), actual);

    // A pure oracle detector: the comparison notices a bad record without changing retained data.
    let mut mismatched = record.clone();
    let mut mismatched_digest = *mismatched.checkpoint_result_digest.as_bytes();
    mismatched_digest[0] ^= 0xff;
    mismatched.checkpoint_result_digest = sagat_domain::workflow::Digest::new(mismatched_digest);
    assert_ne!(record_source_projection(&mismatched), actual);
}

async fn assert_persisted_certificate_and_audit(f: &Flow, record: &LeafCollectionSuccessRecord) {
    let [segment] = record.identity.path().segments() else {
        panic!("certified ordinary source is exactly depth one");
    };
    let persisted: (Uuid, i64, Uuid) = sqlx::query_as(
        "SELECT definition_version_id,certified_run_revision,audit_id \
         FROM leaf_collection_success_certifications \
         WHERE tenant_id=$1 AND namespace_id=$2 AND run_id=$3 \
          AND owner_node=$4 AND iteration_index=$5",
    )
    .bind(record.identity.tenant_id().as_uuid())
    .bind(record.identity.namespace_id().as_uuid())
    .bind(record.identity.run_id().as_uuid())
    .bind(segment.scope_node().as_str())
    .bind(i32::try_from(segment.iteration_index()).expect("bounded source index"))
    .fetch_one(&f.api.db.pool)
    .await
    .expect("exact persisted child certificate pin");
    assert_eq!(persisted.0, record.definition_version.as_uuid());
    assert_eq!(
        persisted.1,
        i64::try_from(record.certified_run_revision).expect("signed certified revision")
    );
    assert_eq!(persisted.2, record.audit_id.as_uuid());
    let audit: (String, String, String, String, String) = sqlx::query_as(
        "SELECT actor,action,resource_type,resource_id,reason FROM audit_records \
         WHERE tenant_id=$1 AND id=$2",
    )
    .bind(record.identity.tenant_id().as_uuid())
    .bind(record.audit_id.as_uuid())
    .fetch_one(&f.api.db.pool)
    .await
    .expect("truthful child source audit");
    assert_eq!(
        audit,
        (
            "system:engine".into(),
            "collection.leaf_success_certified".into(),
            "leaf_collection_success".into(),
            format!(
                "{}/{}/{}/{}",
                record.identity.namespace_id(),
                record.identity.run_id(),
                segment.scope_node(),
                segment.iteration_index()
            ),
            "ordinary leaf collection path success certified".into(),
        )
    );
}

pub(super) async fn ordinary_child_source_snapshot(
    f: &Flow,
    parent: RunId,
) -> OrdinaryChildSourceSnapshot {
    let selected_source = selected_sources(f).await;
    let certificate: String = sqlx::query_scalar(
        "SELECT md5(COALESCE((SELECT jsonb_agg(to_jsonb(c) ORDER BY c.owner_node,c.iteration_index) \
         FROM leaf_collection_success_certifications c WHERE c.tenant_id=$1 AND c.run_id=$2 \
          AND c.owner_node='lines' AND c.iteration_index=0),'[]'::jsonb)::text)",
    )
    .bind(f.tenant.as_uuid())
    .bind(f.run.as_uuid())
    .fetch_one(&f.api.db.pool)
    .await
    .expect("exact selected child certificate snapshot");
    let run_and_journal: String = sqlx::query_scalar(
        "SELECT md5(jsonb_build_object(\
         'run',(SELECT to_jsonb(r) FROM runs r WHERE r.tenant_id=$1 AND r.id=$2),\
         'journal',(SELECT COALESCE(jsonb_agg(to_jsonb(e) ORDER BY e.seq),'[]'::jsonb) \
          FROM run_events e WHERE e.tenant_id=$1 AND e.run_id=$2))::text)",
    )
    .bind(f.tenant.as_uuid())
    .bind(f.run.as_uuid())
    .fetch_one(&f.api.db.pool)
    .await
    .expect("exact child Run and journal snapshot");
    let steps: String = sqlx::query_scalar(
        "SELECT md5(COALESCE((SELECT jsonb_agg(to_jsonb(s) ORDER BY \
         s.instance_path_nodes,s.instance_path_indices,s.node) FROM steps s \
         WHERE s.tenant_id=$1 AND s.run_id=$2),'[]'::jsonb)::text)",
    )
    .bind(f.tenant.as_uuid())
    .bind(f.run.as_uuid())
    .fetch_one(&f.api.db.pool)
    .await
    .expect("exact child steps snapshot");
    let parent_flags: String = sqlx::query_scalar(
        "SELECT md5(jsonb_build_object(\
         'child',(SELECT jsonb_build_object(\
          'parent_run_id',r.parent_run_id,'parent_node',r.parent_node,\
          'cancel_policy',r.cancel_policy,'depth',r.depth,'root_run_id',r.root_run_id,\
          'parent_cancel',r.parent_cancel,'parent_settled',r.parent_settled) \
          FROM runs r WHERE r.tenant_id=$1 AND r.id=$2),\
         'parent',(SELECT jsonb_build_object('status',r.status,'run_version',r.run_version) \
          FROM runs r WHERE r.tenant_id=$1 AND r.id=$3),\
         'parent_step',(SELECT jsonb_build_object(\
          'status',s.status,'routed',s.routed,'cancel_requested',s.cancel_requested,\
          'child_run_id',s.child_run_id,'child_started',s.child_started) \
          FROM steps s WHERE s.tenant_id=$1 AND s.run_id=$3 AND s.node='kid' \
           AND s.instance_path_nodes='{}' AND s.instance_path_indices='{}'))::text)",
    )
    .bind(f.tenant.as_uuid())
    .bind(f.run.as_uuid())
    .bind(parent.as_uuid())
    .fetch_one(&f.api.db.pool)
    .await
    .expect("exact child and public-parent flags snapshot");
    OrdinaryChildSourceSnapshot {
        selected_source,
        certificate,
        run_and_journal,
        steps,
        parent_flags,
    }
}

async fn assert_source_insert_privilege(f: &Flow, expected: bool) {
    let effective: (String, bool, bool, bool, bool) = sqlx::query_as(
        "SELECT current_user::text,\
         has_table_privilege(current_user,'leaf_collection_success_certifications','INSERT'),\
         has_any_column_privilege(current_user,'leaf_collection_success_certifications','UPDATE'),\
         has_table_privilege(current_user,'leaf_collection_success_certifications','DELETE'),\
         has_table_privilege(current_user,'leaf_collection_success_certifications','TRUNCATE')",
    )
    .fetch_one(&f.api.api_pool)
    .await
    .expect("effective native source privilege probe");
    assert_eq!(
        effective,
        ("sagat_app".into(), expected, false, false, false)
    );
}

async fn command_for_run(f: &mut Flow, run: RunId) -> CertifyLeafCollectionSuccess {
    let original = f.run;
    f.run = run;
    let discovered = command(f).await;
    f.run = original;
    discovered
}

fn qualified_identity(
    source: &LeafCollectionPathIdentity,
    tenant: TenantId,
    namespace: NamespaceId,
    run: RunId,
    index: u32,
) -> LeafCollectionPathIdentity {
    LeafCollectionPathIdentity::new(
        tenant,
        namespace,
        run,
        InstancePath::root()
            .child(NodeId::parse("lines").expect("ordinary root owner"), index)
            .expect("depth-one ordinary source"),
    )
    .unwrap_or_else(|error| panic!("qualified identity beside {:?}: {error:?}", source.path()))
}

async fn seed_single_scoped_member(
    f: &Flow,
    source: &CertifyLeafCollectionSuccess,
) -> (CapacityGroupId, CapacityMemberId) {
    let group = CapacityGroupId::generate();
    let member = CapacityMemberId::generate();
    let [segment] = source.identity.path().segments() else {
        panic!("ordinary source identity is exactly depth one");
    };
    let mut tx = f.api.db.pool.begin().await.expect("owner registry fixture");
    sqlx::query(
        "INSERT INTO capacity_groups(tenant_id,group_id,revision,epoch,active_members,\
         possession,holder_member_id) VALUES($1,$2,1,1,1,'VACANT',NULL)",
    )
    .bind(f.tenant.as_uuid())
    .bind(group.as_uuid())
    .execute(&mut *tx)
    .await
    .expect("one valid vacant capacity header");
    sqlx::query(
        "INSERT INTO capacity_group_members(tenant_id,group_id,member_id,namespace_id,\
         member_kind,chain_id,original_run_id,root_owner_node,iteration_index,state) \
         VALUES($1,$2,$3,$4,'SCOPED_PATH',NULL,$5,$6,$7,'ACTIVE')",
    )
    .bind(f.tenant.as_uuid())
    .bind(group.as_uuid())
    .bind(member.as_uuid())
    .bind(source.identity.namespace_id().as_uuid())
    .bind(source.identity.run_id().as_uuid())
    .bind(segment.scope_node().as_str())
    .bind(i32::try_from(segment.iteration_index()).expect("bounded source index"))
    .execute(&mut *tx)
    .await
    .expect("one exact owner-seeded scoped member");
    tx.commit()
        .await
        .expect("valid one-member registry fixture");
    (group, member)
}

async fn capacity_snapshot(f: &Flow, group: CapacityGroupId) -> Value {
    sqlx::query_scalar(
        "SELECT jsonb_build_object(\
         'group',(SELECT to_jsonb(g) FROM capacity_groups g \
          WHERE tenant_id=$1 AND group_id=$2),\
         'members',(SELECT COALESCE(jsonb_agg(to_jsonb(m) ORDER BY member_id),'[]'::jsonb) \
          FROM capacity_group_members m WHERE tenant_id=$1 AND group_id=$2),\
         'receipts',(SELECT COALESCE(jsonb_agg(to_jsonb(r) ORDER BY operation_key),'[]'::jsonb) \
          FROM capacity_group_finish_receipts r WHERE tenant_id=$1 AND group_id=$2),\
         'audits',(SELECT COALESCE(jsonb_agg(to_jsonb(a) ORDER BY id),'[]'::jsonb) \
          FROM audit_records a WHERE tenant_id=$1 AND action='capacity.member.finished' \
           AND resource_id=$2::text),\
         'hints',(SELECT COALESCE(jsonb_agg(to_jsonb(o) ORDER BY id),'[]'::jsonb) \
          FROM event_outbox o WHERE tenant_id=$1 AND aggregate_id=$2 \
           AND event_type='capacity.scoped_member.finished'))",
    )
    .bind(f.tenant.as_uuid())
    .bind(group.as_uuid())
    .fetch_one(&f.api.db.pool)
    .await
    .expect("exact owner capacity snapshot")
}

async fn assert_select_denied(pool: &PgPool, table: &str, statement: &'static str) {
    let error = sqlx::query(statement)
        .fetch_optional(pool)
        .await
        .expect_err("runtime-role source or receipt read must be denied");
    assert_eq!(
        error
            .as_database_error()
            .and_then(|database| database.code())
            .as_deref(),
        Some("42501"),
        "unexpected {table} denial: {error}"
    );
}

async fn certification_counts(f: &Flow) -> (i64, i64) {
    sqlx::query_as(
        "SELECT \
         (SELECT count(*) FROM leaf_collection_success_certifications \
          WHERE tenant_id=$1 AND namespace_id=$2 AND run_id=$3 \
           AND owner_node='lines' AND iteration_index=0),\
         (SELECT count(*) FROM audit_records WHERE tenant_id=$1 \
          AND action='collection.leaf_success_certified' \
          AND resource_type='leaf_collection_success' \
          AND resource_id=concat($2::text,'/',$3::text,'/lines/0'))",
    )
    .bind(f.tenant.as_uuid())
    .bind(
        sqlx::query_scalar::<_, Uuid>("SELECT namespace_id FROM runs WHERE tenant_id=$1 AND id=$2")
            .bind(f.tenant.as_uuid())
            .bind(f.run.as_uuid())
            .fetch_one(&f.api.db.pool)
            .await
            .expect("qualified child namespace"),
    )
    .bind(f.run.as_uuid())
    .fetch_one(&f.api.db.pool)
    .await
    .expect("one exact source fact and truthful audit")
}

async fn assert_public_parent_settled(f: &Flow, parent: RunId) {
    let settled: (String, bool, bool, String, String, bool, bool) = sqlx::query_as(
        "SELECT child.status,child.parent_settled,child.parent_cancel,parent.status,\
         step.status,step.routed,step.child_started \
         FROM runs child JOIN runs parent ON parent.tenant_id=child.tenant_id AND parent.id=$3 \
         JOIN steps step ON step.tenant_id=parent.tenant_id AND step.run_id=parent.id \
          AND step.node='kid' AND step.instance_path_nodes='{}' AND step.instance_path_indices='{}' \
         WHERE child.tenant_id=$1 AND child.id=$2",
    )
    .bind(f.tenant.as_uuid())
    .bind(f.run.as_uuid())
    .bind(parent.as_uuid())
    .fetch_one(&f.api.db.pool)
    .await
    .expect("actual public parent settlement");
    assert_eq!(
        settled,
        (
            "SUCCEEDED".into(),
            true,
            false,
            "SUCCEEDED".into(),
            "SUCCEEDED".into(),
            true,
            true,
        )
    );
}

#[tokio::test]
async fn native_certifier_characterizes_a_real_ordinary_unchained_child_source() {
    let mut fixture = ordinary_child_source_boundary("baseline").await;
    assert_eq!(fixture.flow.run, fixture.child);
    assert_ne!(fixture.bootstrap, fixture.parent);
    assert_ne!(fixture.bootstrap, fixture.child);
    assert_ne!(fixture.parent, fixture.child);
    assert_eq!(certification_counts(&fixture.flow).await, (0, 0));
    let before = ordinary_child_source_snapshot(&fixture.flow, fixture.parent).await;

    // Disposable activation only. The released migration remains SELECT-only for sagat_app.
    sqlx::query("GRANT INSERT ON leaf_collection_success_certifications TO sagat_app")
        .execute(&fixture.flow.api.db.pool)
        .await
        .expect("temporary ordinary-child source writer");
    assert_source_insert_privilege(&fixture.flow, true).await;
    let store = PgLeafCollectionSuccessStore::new(
        fixture.flow.api.api_pool.clone(),
        DefinitionCache::new(Arc::new(JsonIrCodec), 8),
    );
    let created = store
        .certify_leaf_collection_success(&fixture.source)
        .await
        .expect("existing native certifier accepts the genuine ordinary child source");
    assert!(!created.replayed);
    assert_eq!(created.record.identity, fixture.source.identity);
    assert_eq!(
        created.record.definition_version,
        sqlx::query_scalar::<_, Uuid>(
            "SELECT workflow_version_id FROM runs WHERE tenant_id=$1 AND namespace_id=$2 AND id=$3",
        )
        .bind(fixture.source.identity.tenant_id().as_uuid())
        .bind(fixture.source.identity.namespace_id().as_uuid())
        .bind(fixture.source.identity.run_id().as_uuid())
        .fetch_one(&fixture.flow.api.db.pool)
        .await
        .map(sagat_domain::definition::WorkflowVersionId::new)
        .expect("actual pinned child definition")
    );
    assert_eq!(
        created.record.certified_run_revision, fixture.source.expected_run_revision,
        "the retained certificate records the revision discovered after the real later lease"
    );
    assert_record_matches_actual_source(&fixture.flow, &created.record).await;
    assert_persisted_certificate_and_audit(&fixture.flow, &created.record).await;
    assert_eq!(certification_counts(&fixture.flow).await, (1, 1));
    let after = ordinary_child_source_snapshot(&fixture.flow, fixture.parent).await;
    assert_eq!(after.selected_source, before.selected_source);
    assert_ne!(after.certificate, before.certificate);
    assert_eq!(after.run_and_journal, before.run_and_journal);
    assert_eq!(after.steps, before.steps);
    assert_eq!(after.parent_flags, before.parent_flags);

    sqlx::query("REVOKE INSERT ON leaf_collection_success_certifications FROM sagat_app")
        .execute(&fixture.flow.api.db.pool)
        .await
        .expect("remove the temporary ordinary-child source writer");
    assert_source_insert_privilege(&fixture.flow, false).await;
    let fresh = command(&fixture.flow).await;
    assert_eq!(fresh.identity, fixture.source.identity);
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
    let mut stale_authority = fresh.clone();
    stale_authority.scheduler = SchedulerAuthority::for_run(
        format!("stale-ordinary-child-{}", Uuid::now_v7()),
        fixture.child,
        fresh.scheduler.lease().generation,
    );
    assert_eq!(
        cold_store
            .certify_leaf_collection_success(&stale_authority)
            .await,
        Err(LeafCollectionSuccessError::SchedulerFenceLost),
        "current own-child authority precedes canonical replay"
    );
    let replay = cold_store
        .certify_leaf_collection_success(&fresh)
        .await
        .expect("cold pool/cache/store reconstructs the canonical child certificate");
    assert!(replay.replayed);
    assert_eq!(replay.record, created.record);
    assert_eq!(certification_counts(&fixture.flow).await, (1, 1));

    let mut wrong_tenant = fresh.clone();
    wrong_tenant.identity = qualified_identity(
        &fresh.identity,
        TenantId::new(Uuid::now_v7()),
        fresh.identity.namespace_id(),
        fresh.identity.run_id(),
        0,
    );
    assert_eq!(
        cold_store
            .certify_leaf_collection_success(&wrong_tenant)
            .await,
        Err(LeafCollectionSuccessError::Missing)
    );
    let mut wrong_namespace = fresh.clone();
    wrong_namespace.identity = qualified_identity(
        &fresh.identity,
        fresh.identity.tenant_id(),
        NamespaceId::new(Uuid::now_v7()),
        fresh.identity.run_id(),
        0,
    );
    assert_eq!(
        cold_store
            .certify_leaf_collection_success(&wrong_namespace)
            .await,
        Err(LeafCollectionSuccessError::Missing)
    );
    let mut neighboring_index = fresh.clone();
    neighboring_index.identity = qualified_identity(
        &fresh.identity,
        fresh.identity.tenant_id(),
        fresh.identity.namespace_id(),
        fresh.identity.run_id(),
        1,
    );
    assert_eq!(
        cold_store
            .certify_leaf_collection_success(&neighboring_index)
            .await,
        Err(LeafCollectionSuccessError::SourceEvidenceMissing)
    );
    let parent_source = command_for_run(&mut fixture.flow, fixture.parent).await;
    assert!(matches!(
        cold_store
            .certify_leaf_collection_success(&parent_source)
            .await,
        Err(LeafCollectionSuccessError::UnsupportedShape(_))
    ));
    assert_eq!(
        ordinary_child_source_snapshot(&fixture.flow, fixture.parent).await,
        after,
        "stale and misqualified attempts cannot reuse or mutate the child source"
    );
    assert_eq!(certification_counts(&fixture.flow).await, (1, 1));
    drop(cold_store);
    cold_pool.close().await;

    fixture
        .flow
        .complete(&fixture.later, json!({"charged": true}))
        .await;
    fixture.flow.drive().await;
    assert_eq!(fixture.flow.run_status().await, "SUCCEEDED");
    assert_public_parent_settled(&fixture.flow, fixture.parent).await;
    let terminal = ordinary_child_source_snapshot(&fixture.flow, fixture.parent).await;
    assert_eq!(terminal.selected_source, after.selected_source);
    assert_eq!(terminal.certificate, after.certificate);
    assert_ne!(
        terminal.run_and_journal, after.run_and_journal,
        "actual later completion and child termination change the child Run/journal oracle"
    );
    assert_ne!(
        terminal.steps, after.steps,
        "actual later completion changes the child step oracle"
    );
    assert_ne!(
        terminal.parent_flags, after.parent_flags,
        "actual child settlement changes the parent and relationship oracle"
    );
    assert_eq!(certification_counts(&fixture.flow).await, (1, 1));

    // Rediscover only current authority/revision after legitimate later work. The retained old
    // canonical identity must return the exact original certificate; no stale-revision premise is
    // made for parent flag writes that need not append the child's journal.
    let mut current = command(&fixture.flow).await;
    current.identity = fixture.source.identity.clone();
    let terminal_replay = store
        .certify_leaf_collection_success(&current)
        .await
        .expect("old child source identity replays under current authority");
    assert!(terminal_replay.replayed);
    assert_eq!(terminal_replay.record, created.record);
    assert_eq!(certification_counts(&fixture.flow).await, (1, 1));
    assert_eq!(
        ordinary_child_source_snapshot(&fixture.flow, fixture.parent).await,
        terminal,
        "terminal canonical replay is metadata-only across the entire exact snapshot"
    );
    fixture.flow.verify().await;
    fixture
        .flow
        .api
        .executions
        .verify_run(fixture.flow.tenant, fixture.parent)
        .await
        .expect("the public parent journal rebuilds after successful settlement");
}

#[tokio::test]
async fn genuine_ordinary_child_certificate_still_cannot_authorize_scoped_finish() {
    let fixture = ordinary_child_source_boundary("consumer-refusal").await;
    sqlx::query("GRANT INSERT ON leaf_collection_success_certifications TO sagat_app")
        .execute(&fixture.flow.api.db.pool)
        .await
        .expect("temporary ordinary-child source writer");
    assert_source_insert_privilege(&fixture.flow, true).await;
    let source_store = PgLeafCollectionSuccessStore::new(
        fixture.flow.api.api_pool.clone(),
        DefinitionCache::new(Arc::new(JsonIrCodec), 8),
    );
    let created = source_store
        .certify_leaf_collection_success(&fixture.source)
        .await
        .expect("genuine child source certificate");
    assert!(!created.replayed);
    assert_record_matches_actual_source(&fixture.flow, &created.record).await;
    assert_persisted_certificate_and_audit(&fixture.flow, &created.record).await;
    sqlx::query("REVOKE INSERT ON leaf_collection_success_certifications FROM sagat_app")
        .execute(&fixture.flow.api.db.pool)
        .await
        .expect("remove the temporary ordinary-child source writer");
    assert_source_insert_privilege(&fixture.flow, false).await;
    let current = command(&fixture.flow).await;
    let replay = source_store
        .certify_leaf_collection_success(&current)
        .await
        .expect("finish-control fixture reconciles its exact genuine certificate first");
    assert!(replay.replayed);
    assert_eq!(replay.record, created.record);
    assert_eq!(certification_counts(&fixture.flow).await, (1, 1));

    let (group, member) = seed_single_scoped_member(&fixture.flow, &current).await;
    let finish = ScopedLeafFinishCommand {
        operation_key: Uuid::new_v4(),
        group_id: group,
        member_id: member,
        identity: current.identity.clone(),
        expected: ScopedLeafFinishExpectations {
            group_revision: 1,
            group_epoch: 1,
            run_revision: current.expected_run_revision,
        },
        scheduler: current.scheduler.clone(),
    };
    assert_eq!(
        finish
            .fingerprint()
            .expect("valid positive scoped command")
            .len(),
        32
    );
    let source_before = ordinary_child_source_snapshot(&fixture.flow, fixture.parent).await;
    let capacity_before = capacity_snapshot(&fixture.flow, group).await;

    let pool = fixture.flow.api.db.runtime_pool_with(2).await;
    sqlx::query(
        "REVOKE SELECT ON leaf_collection_success_certifications,\
         capacity_group_finish_receipts FROM sagat_app",
    )
    .execute(&fixture.flow.api.db.pool)
    .await
    .expect("test-only context-before-source read denials");
    let privileges: (String, bool, bool, bool) = sqlx::query_as(
        "SELECT current_user::text,\
         has_table_privilege(current_user,'leaf_collection_success_certifications','SELECT'),\
         has_table_privilege(current_user,'leaf_collection_success_certifications','INSERT'),\
         has_table_privilege(current_user,'capacity_group_finish_receipts','SELECT')",
    )
    .fetch_one(&pool)
    .await
    .expect("actual runtime role and effective denied source/receipt reads");
    assert_eq!(privileges, ("sagat_app".into(), false, false, false));
    assert_select_denied(
        &pool,
        "leaf_collection_success_certifications",
        "SELECT * FROM leaf_collection_success_certifications LIMIT 0",
    )
    .await;
    assert_select_denied(
        &pool,
        "capacity_group_finish_receipts",
        "SELECT * FROM capacity_group_finish_receipts LIMIT 0",
    )
    .await;

    assert_eq!(
        PgScopedLeafFinishStore::new(pool.clone())
            .finish_scoped_leaf(&finish)
            .await,
        Err(ScopedLeafFinishError::UnsupportedRunContext),
        "a genuine certificate does not override the scoped consumer's child-context refusal"
    );
    assert_eq!(
        ordinary_child_source_snapshot(&fixture.flow, fixture.parent).await,
        source_before
    );
    assert_eq!(
        capacity_snapshot(&fixture.flow, group).await,
        capacity_before
    );
    assert_eq!(certification_counts(&fixture.flow).await, (1, 1));
    fixture.flow.verify().await;
    pool.close().await;
}

#[path = "leaf_certification_child_parent_flags.rs"]
mod parent_flags;

#[path = "leaf_certification_child_detach.rs"]
mod detached_progress;

#[path = "leaf_certification_child_own_cancel.rs"]
mod own_cancel;

#[path = "leaf_certification_child_result.rs"]
mod result;
