//! Actual own-child cancellation, selected-source replay, later drain and compensation.
//!
//! This remains characterization of existing producers and stores. It adds no caller or grant.

use super::*;
use crate::support::{RUNTIME_ROLE, RUNTIME_ROLE_PASSWORD, db_error};
use sagat_application::entropy::Token;
use sagat_scheduler::{CollectionAdvance, Report};
use sqlx::{
    PgPool,
    postgres::{PgConnectOptions, PgPoolOptions},
};
use std::{future::Future, pin::Pin};
use tokio::time::timeout;

const WAIT: Duration = Duration::from_secs(10);
const COMPACT_RUN: (&str, &str) = (
    "SELECT workflow_version_id,run_version,trace_id FROM runs",
    "WHERE tenant_id=$1 AND namespace_id=$2 AND id=$3 FOR UPDATE",
);
const FULL_RUN: (&str, &str) = (
    "SELECT id, tenant_id, namespace_id, workflow_id, workflow_version_id,",
    "FROM runs WHERE tenant_id = $1 AND id = $2 FOR UPDATE",
);

#[derive(Clone, Debug, PartialEq, Eq)]
struct SelectedState {
    source: String,
    certificate: String,
    audit: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct LeaseState {
    step_id: Uuid,
    status: String,
    attempt: i32,
    generation: i64,
    cancel_requested: bool,
    lease_hash: Vec<u8>,
    lease_expires_at: String,
    attempt_deadline_at: Option<String>,
    leased_by: String,
    expires_at_epoch_ms: i64,
    lease_live: bool,
    deadline_live: Option<bool>,
}

#[derive(Debug)]
struct ActivityWitness {
    pid: i32,
    datname: Option<String>,
    usename: Option<String>,
    state: Option<String>,
    query: Option<String>,
    wait_event_type: Option<String>,
    wait_event: Option<String>,
    blockers: Vec<i32>,
}

#[derive(Clone, Copy)]
enum RaceWitness {
    StopThenCertify,
    CertifyThenStop,
}

struct Prepared {
    fixture: OrdinaryChildSourceBoundary,
    after: TaskLease,
    source: CertifyLeafCollectionSuccess,
    selected: SelectedState,
    parent_key: String,
    parent_snapshot: String,
}

struct Certified {
    result: sagat_application::leaf_certification::LeafCollectionSuccessResult,
    successful_command: CertifyLeafCollectionSuccess,
}

fn compensating_child_document(name: &str) -> Value {
    let mut child = document(name);
    child["bodies"]["later_line"] = json!({"entrypoint":"work","nodes":[
        {"id":"work","type":"activity","activity":"line.charge","next":"after",
         "compensateWith":"line.refund"},
        {"id":"after","type":"activity","activity":"line.notify","next":"returned"},
        {"id":"returned","type":"return","result":"steps.after.output"}
    ]});
    child
}

async fn grant_source_writer(f: &Flow) {
    sqlx::query("GRANT INSERT ON leaf_collection_success_certifications TO sagat_app")
        .execute(&f.api.db.pool)
        .await
        .expect("temporary ordinary-child source writer");
    assert_source_insert_privilege(f, true).await;
}

async fn revoke_source_writer(f: &Flow) {
    sqlx::query("REVOKE INSERT ON leaf_collection_success_certifications FROM sagat_app")
        .execute(&f.api.db.pool)
        .await
        .expect("remove temporary ordinary-child source writer");
    assert_source_insert_privilege(f, false).await;
}

async fn selected_state(f: &Flow) -> SelectedState {
    let namespace: Uuid =
        sqlx::query_scalar("SELECT namespace_id FROM runs WHERE tenant_id=$1 AND id=$2")
            .bind(f.tenant.as_uuid())
            .bind(f.run.as_uuid())
            .fetch_one(&f.api.db.pool)
            .await
            .expect("qualified child namespace");
    let certificate = sqlx::query_scalar(
        "SELECT md5(COALESCE((SELECT jsonb_agg(to_jsonb(c) ORDER BY c.owner_node,c.iteration_index) \
         FROM leaf_collection_success_certifications c WHERE c.tenant_id=$1 AND c.run_id=$2 \
          AND c.owner_node='lines' AND c.iteration_index=0),'[]'::jsonb)::text)",
    )
    .bind(f.tenant.as_uuid())
    .bind(f.run.as_uuid())
    .fetch_one(&f.api.db.pool)
    .await
    .expect("qualified selected certificate snapshot");
    let audit = sqlx::query_scalar(
        "SELECT md5(COALESCE((SELECT jsonb_agg(to_jsonb(a) ORDER BY a.id) \
         FROM audit_records a WHERE a.tenant_id=$1 \
          AND a.action='collection.leaf_success_certified' \
          AND a.resource_type='leaf_collection_success' \
          AND a.resource_id=concat($2::text,'/',$3::text,'/lines/0')),'[]'::jsonb)::text)",
    )
    .bind(f.tenant.as_uuid())
    .bind(namespace)
    .bind(f.run.as_uuid())
    .fetch_one(&f.api.db.pool)
    .await
    .expect("qualified selected certification audit snapshot");
    SelectedState {
        source: selected_sources(f).await,
        certificate,
        audit,
    }
}

async fn assert_selected(f: &Flow, expected: &SelectedState, record: &LeafCollectionSuccessRecord) {
    assert_eq!(selected_state(f).await, *expected);
    assert_record_matches_actual_source(f, record).await;
    assert_persisted_certificate_and_audit(f, record).await;
    assert_eq!(certification_counts(f).await, (1, 1));
}

async fn lease_state(f: &Flow) -> LeaseState {
    let row = sqlx::query(
        "SELECT step_id,status,attempt,generation,cancel_requested,lease_token_hash, \
         lease_expires_at::text,attempt_deadline_at::text,leased_by, \
         floor(extract(epoch FROM lease_expires_at)*1000)::bigint AS expires_at_epoch_ms, \
         lease_expires_at>clock_timestamp() AS lease_live, \
         attempt_deadline_at>clock_timestamp() AS deadline_live \
         FROM steps WHERE tenant_id=$1 AND run_id=$2 AND node='after' \
          AND instance_path_nodes=ARRAY['later']::text[] \
          AND instance_path_indices=ARRAY[0]::integer[]",
    )
    .bind(f.tenant.as_uuid())
    .bind(f.run.as_uuid())
    .fetch_one(&f.api.db.pool)
    .await
    .expect("exact later successor lease");
    LeaseState {
        step_id: row.get("step_id"),
        status: row.get("status"),
        attempt: row.get("attempt"),
        generation: row.get("generation"),
        cancel_requested: row.get("cancel_requested"),
        lease_hash: row.get("lease_token_hash"),
        lease_expires_at: row.get("lease_expires_at"),
        attempt_deadline_at: row.get("attempt_deadline_at"),
        leased_by: row.get("leased_by"),
        expires_at_epoch_ms: row.get("expires_at_epoch_ms"),
        lease_live: row.get("lease_live"),
        deadline_live: row.get("deadline_live"),
    }
}

async fn exact_child_snapshot(f: &Flow) -> String {
    sqlx::query_scalar(
        "SELECT md5(jsonb_build_object( \
         'run',(SELECT to_jsonb(r) FROM runs r WHERE r.tenant_id=$1 AND r.id=$2), \
         'journal',(SELECT COALESCE(jsonb_agg(to_jsonb(e) ORDER BY e.seq),'[]'::jsonb) \
          FROM run_events e WHERE e.tenant_id=$1 AND e.run_id=$2), \
         'steps',(SELECT COALESCE(jsonb_agg(to_jsonb(s) ORDER BY s.instance_path_nodes, \
          s.instance_path_indices,s.node),'[]'::jsonb) FROM steps s \
          WHERE s.tenant_id=$1 AND s.run_id=$2), \
         'scopes',(SELECT COALESCE(jsonb_agg(to_jsonb(s) ORDER BY s.owner_node),'[]'::jsonb) \
          FROM iteration_scopes s WHERE s.tenant_id=$1 AND s.run_id=$2), \
         'iterations',(SELECT COALESCE(jsonb_agg(to_jsonb(i) ORDER BY i.scope_id,i.iteration_index), \
          '[]'::jsonb) FROM collection_iterations i WHERE i.tenant_id=$1 AND i.run_id=$2), \
         'sagas',(SELECT COALESCE(jsonb_agg(to_jsonb(s) ORDER BY s.owner_node),'[]'::jsonb) \
          FROM scope_sagas s WHERE s.tenant_id=$1 AND s.run_id=$2), \
         'entries',(SELECT COALESCE(jsonb_agg(to_jsonb(e) ORDER BY e.owner_node,e.iteration_index,e.node), \
          '[]'::jsonb) FROM scope_saga_entries e WHERE e.tenant_id=$1 AND e.run_id=$2), \
         'usage',(SELECT to_jsonb(u) FROM run_instance_usage u WHERE u.tenant_id=$1 AND u.run_id=$2) \
         )::text)",
    )
    .bind(f.tenant.as_uuid())
    .bind(f.run.as_uuid())
    .fetch_one(&f.api.db.pool)
    .await
    .expect("qualified mutable child snapshot")
}

async fn child_run_and_journal_without_parent_flags(f: &Flow) -> String {
    sqlx::query_scalar(
        "SELECT md5(jsonb_build_object( \
         'run',(SELECT to_jsonb(r)-'parent_cancel'-'parent_settled' FROM runs r \
          WHERE r.tenant_id=$1 AND r.id=$2), \
         'journal',(SELECT COALESCE(jsonb_agg(to_jsonb(e) ORDER BY e.seq),'[]'::jsonb) \
          FROM run_events e WHERE e.tenant_id=$1 AND e.run_id=$2))::text)",
    )
    .bind(f.tenant.as_uuid())
    .bind(f.run.as_uuid())
    .fetch_one(&f.api.db.pool)
    .await
    .expect("child Run excluding only incoming flags plus complete journal")
}

async fn parent_snapshot(f: &Flow, parent: RunId, key: &str) -> String {
    sqlx::query_scalar(
        "SELECT md5(jsonb_build_object( \
         'run',(SELECT to_jsonb(r) FROM runs r WHERE r.tenant_id=$1 AND r.id=$2), \
         'journal',(SELECT COALESCE(jsonb_agg(to_jsonb(e) ORDER BY e.seq),'[]'::jsonb) \
          FROM run_events e WHERE e.tenant_id=$1 AND e.run_id=$2), \
         'steps',(SELECT COALESCE(jsonb_agg(to_jsonb(s) ORDER BY s.node),'[]'::jsonb) \
          FROM steps s WHERE s.tenant_id=$1 AND s.run_id=$2), \
         'audit',(SELECT COALESCE(jsonb_agg(to_jsonb(a) ORDER BY a.id),'[]'::jsonb) \
          FROM audit_records a WHERE a.tenant_id=$1 AND a.resource_type='run' \
           AND a.resource_id=$2::text AND a.action='run.cancel_requested'), \
         'dedupe',(SELECT COALESCE(jsonb_agg(to_jsonb(d) ORDER BY d.idempotency_key),'[]'::jsonb) \
          FROM command_dedupe d WHERE d.tenant_id=$1 \
           AND d.scope=concat('run.operate:',$2::text) AND d.idempotency_key=$3), \
         'outbox',(SELECT COALESCE(jsonb_agg(to_jsonb(o) ORDER BY o.id),'[]'::jsonb) \
          FROM event_outbox o WHERE o.tenant_id=$1 AND o.aggregate_id=$2 \
           AND o.event_type='run.finished'))::text)",
    )
    .bind(f.tenant.as_uuid())
    .bind(parent.as_uuid())
    .bind(key)
    .fetch_one(&f.api.db.pool)
    .await
    .expect("exact settled parent snapshot")
}

async fn cancel_parent(f: &Flow, parent: RunId, key: &str) -> String {
    let before: i64 =
        sqlx::query_scalar("SELECT run_version FROM runs WHERE tenant_id=$1 AND id=$2")
            .bind(f.tenant.as_uuid())
            .bind(parent.as_uuid())
            .fetch_one(&f.api.db.pool)
            .await
            .expect("parent revision before public CANCEL");
    let response = f
        .api
        .as_user(
            Method::POST,
            &format!("/runs/{parent}/operations"),
            "alice",
            "acme",
        )
        .header("x-octopus-namespace", "payments")
        .header("idempotency-key", key)
        .json(&json!({"action":"CANCEL","reason":"own child cancellation proof"}))
        .send()
        .await
        .expect("public parent CANCEL response");
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let body = crate::support::api::json_of(response).await;
    let relation = sqlx::query(
        "SELECT p.status,p.finished_at IS NOT NULL AS parent_finished,p.run_version, \
         c.parent_cancel,c.parent_settled,c.stop_reason,c.finished_at IS NOT NULL AS child_finished, \
         kid.status AS kid_status,kid.waiting_kind,kid.cancel_requested,kid.routed, \
         kid.child_run_id,kid.child_started, \
         (SELECT count(*) FROM steps done WHERE done.tenant_id=p.tenant_id \
          AND done.run_id=p.id AND done.node='done') AS done_count \
         FROM runs c JOIN runs p ON p.tenant_id=c.tenant_id AND p.id=c.parent_run_id \
         JOIN steps kid ON kid.tenant_id=p.tenant_id AND kid.run_id=p.id AND kid.node='kid' \
         WHERE c.tenant_id=$1 AND c.id=$2 AND p.id=$3",
    )
    .bind(f.tenant.as_uuid())
    .bind(f.run.as_uuid())
    .bind(parent.as_uuid())
    .fetch_one(&f.api.db.pool)
    .await
    .expect("PROPAGATE parent and child relationship");
    assert_eq!(relation.get::<String, _>("status"), "CANCELLED");
    assert!(relation.get::<bool, _>("parent_finished"));
    assert_eq!(relation.get::<i64, _>("run_version"), before + 3);
    assert!(relation.get::<bool, _>("parent_cancel"));
    assert!(relation.get::<bool, _>("parent_settled"));
    assert_eq!(relation.get::<Option<String>, _>("stop_reason"), None);
    assert!(!relation.get::<bool, _>("child_finished"));
    assert_eq!(relation.get::<String, _>("kid_status"), "CANCELLED");
    assert_eq!(relation.get::<Option<String>, _>("waiting_kind"), None);
    assert!(!relation.get::<bool, _>("cancel_requested"));
    assert!(relation.get::<bool, _>("routed"));
    assert_eq!(
        relation.get::<Option<Uuid>, _>("child_run_id"),
        Some(f.run.as_uuid())
    );
    assert!(relation.get::<bool, _>("child_started"));
    assert_eq!(relation.get::<i64, _>("done_count"), 0);
    assert_eq!(body["runStatus"], "CANCELLED");
    assert_eq!(body["runVersion"], before + 3);

    let suffix: Vec<(i64, String, Option<String>, Value)> = sqlx::query_as(
        "SELECT seq,event_type,node,detail FROM run_events \
         WHERE tenant_id=$1 AND run_id=$2 AND seq>$3 ORDER BY seq",
    )
    .bind(f.tenant.as_uuid())
    .bind(parent.as_uuid())
    .bind(before)
    .fetch_all(&f.api.db.pool)
    .await
    .expect("exact PROPAGATE parent journal suffix");
    assert_eq!(
        suffix,
        vec![
            (
                before + 1,
                "run.stop_requested".into(),
                None,
                json!({"reason":"CANCELLED","code":null})
            ),
            (
                before + 2,
                "step.cancelled".into(),
                Some("kid".into()),
                json!({})
            ),
            (
                before + 3,
                "run.finished".into(),
                None,
                json!({"status":"CANCELLED"})
            ),
        ]
    );
    let evidence = sqlx::query(
        "SELECT \
         (SELECT count(*) FROM audit_records a WHERE a.tenant_id=$1 \
          AND a.action='run.cancel_requested' AND a.resource_type='run' \
          AND a.resource_id=$2::text AND a.reason='own child cancellation proof') AS audit_count, \
         (SELECT max(a.actor) FROM audit_records a WHERE a.tenant_id=$1 \
          AND a.action='run.cancel_requested' AND a.resource_type='run' \
          AND a.resource_id=$2::text AND a.reason='own child cancellation proof') AS audit_actor, \
         COALESCE((SELECT bool_and(a.trace_id ~ '^[0-9a-f]{32}$') FROM audit_records a \
          WHERE a.tenant_id=$1 AND a.action='run.cancel_requested' \
           AND a.resource_type='run' AND a.resource_id=$2::text \
           AND a.reason='own child cancellation proof'),false) AS audit_trace, \
         (SELECT count(*) FROM command_dedupe d WHERE d.tenant_id=$1 \
          AND d.scope=concat('run.operate:',$2::text) AND d.idempotency_key=$3) AS dedupe_count, \
         (SELECT max(d.resource_id) FROM command_dedupe d WHERE d.tenant_id=$1 \
          AND d.scope=concat('run.operate:',$2::text) AND d.idempotency_key=$3) AS dedupe_resource, \
         COALESCE((SELECT bool_and(octet_length(d.request_hash)=32) FROM command_dedupe d \
          WHERE d.tenant_id=$1 AND d.scope=concat('run.operate:',$2::text) \
           AND d.idempotency_key=$3),false) AS dedupe_hash, \
         (SELECT count(*) FROM event_outbox o WHERE o.tenant_id=$1 \
          AND o.aggregate_id=$2 AND o.event_type='run.finished') AS outbox_count, \
         COALESCE((SELECT bool_and(o.payload->>'tenantId'=$1::text \
          AND o.payload->>'runId'=$2::text AND o.payload->>'status'='CANCELLED') \
          FROM event_outbox o WHERE o.tenant_id=$1 AND o.aggregate_id=$2 \
           AND o.event_type='run.finished'),false) AS outbox_payload",
    )
    .bind(f.tenant.as_uuid())
    .bind(parent.as_uuid())
    .bind(key)
    .fetch_one(&f.api.db.pool)
    .await
    .expect("parent audit, dedupe and terminal outbox");
    assert_eq!(evidence.get::<i64, _>("audit_count"), 1);
    assert_eq!(
        evidence.get::<Option<String>, _>("audit_actor").as_deref(),
        Some("alice")
    );
    assert!(evidence.get::<bool, _>("audit_trace"));
    assert_eq!(evidence.get::<i64, _>("dedupe_count"), 1);
    assert_eq!(
        evidence
            .get::<Option<String>, _>("dedupe_resource")
            .as_deref(),
        Some("cancel")
    );
    assert!(evidence.get::<bool, _>("dedupe_hash"));
    assert_eq!(evidence.get::<i64, _>("outbox_count"), 1);
    assert!(evidence.get::<bool, _>("outbox_payload"));
    parent_snapshot(f, parent, key).await
}

async fn prepare(label: &str) -> Prepared {
    let fixture = ordinary_child_source_boundary_with_document(
        label,
        CancelPolicy::Propagate,
        compensating_child_document,
    )
    .await;
    let f = &fixture.flow;
    assert_eq!(f.run, fixture.child);
    let expected_charge = json!({"charged":"known-effect"});
    f.complete(&fixture.later, expected_charge.clone()).await;
    f.drive().await;
    let tasks = f.poll().await;
    assert_eq!(tasks.len(), 1);
    let after = tasks.into_iter().next().expect("one later successor");
    let instance = after.instance.as_ref().expect("scoped successor identity");
    assert_eq!(instance.definition_node_id, "after");
    assert_eq!(instance.instance_path.len(), 1);
    assert_eq!(instance.instance_path[0].scope_node_id, "later");
    assert_eq!(instance.instance_path[0].iteration_index, 0);
    assert_eq!(after.task_kind, "FORWARD");
    assert_eq!(after.activity_name, "line.notify");
    let charge = sqlx::query(
        "SELECT s.step_id,s.status,s.routed,p.content, \
         s.lease_token_hash IS NULL AS token_cleared, \
         s.lease_expires_at IS NULL AS expiry_cleared,s.leased_by IS NULL AS owner_cleared, \
         (SELECT count(*) FROM run_events e WHERE e.tenant_id=s.tenant_id \
          AND e.run_id=s.run_id AND e.event_type='step.completed' AND e.node='work' \
          AND e.instance_path_nodes=ARRAY['later']::text[] \
          AND e.instance_path_indices=ARRAY[0]::integer[]) AS completed_events, \
         (SELECT count(*) FROM run_events e WHERE e.tenant_id=s.tenant_id \
          AND e.run_id=s.run_id AND e.event_type='step.routed' AND e.node='work' \
          AND e.instance_path_nodes=ARRAY['later']::text[] \
          AND e.instance_path_indices=ARRAY[0]::integer[]) AS routed_events \
         FROM steps s JOIN payloads p ON p.tenant_id=s.tenant_id AND p.digest=s.output_digest \
         WHERE s.tenant_id=$1 AND s.run_id=$2 AND s.node='work' \
          AND s.instance_path_nodes=ARRAY['later']::text[] \
          AND s.instance_path_indices=ARRAY[0]::integer[]",
    )
    .bind(f.tenant.as_uuid())
    .bind(f.run.as_uuid())
    .fetch_one(&f.api.db.pool)
    .await
    .expect("known later charge and journal");
    assert_eq!(charge.get::<String, _>("status"), "SUCCEEDED");
    assert!(charge.get::<bool, _>("routed"));
    assert_eq!(
        serde_json::from_slice::<Value>(&charge.get::<Vec<u8>, _>("content"))
            .expect("retained known charge output"),
        expected_charge
    );
    for field in ["token_cleared", "expiry_cleared", "owner_cleared"] {
        assert!(charge.get::<bool, _>(field), "charge invariant {field}");
    }
    assert_eq!(
        (
            charge.get::<i64, _>("completed_events"),
            charge.get::<i64, _>("routed_events")
        ),
        (1, 1),
    );
    let successor_lease = lease_state(f).await;
    assert_eq!(successor_lease.step_id.to_string(), after.step_id);
    assert_eq!(successor_lease.attempt, after.attempt);
    assert_eq!(successor_lease.generation, after.generation);
    assert_eq!(
        successor_lease.lease_hash,
        Token::hash_of(&after.lease_token)
    );
    assert_eq!(
        successor_lease.expires_at_epoch_ms,
        after.expires_at_epoch_ms
    );
    assert!(!successor_lease.leased_by.is_empty());
    assert!(successor_lease.lease_live);
    if successor_lease.attempt_deadline_at.is_some() {
        assert_eq!(successor_lease.deadline_live, Some(true));
    } else {
        assert_eq!(successor_lease.deadline_live, None);
    }
    let later: (String, String, bool, i64, i64) = sqlx::query_as(
        "SELECT s.lifecycle,i.lifecycle,i.slot_held, \
         (SELECT count(*) FROM scope_sagas saga WHERE saga.tenant_id=s.tenant_id \
          AND saga.run_id=s.run_id AND saga.owner_node='later' \
          AND saga.owner_path_nodes='{}' AND saga.owner_path_indices='{}'), \
         (SELECT count(*) FROM steps returned WHERE returned.tenant_id=s.tenant_id \
          AND returned.run_id=s.run_id AND returned.node='returned' \
          AND returned.instance_path_nodes=ARRAY['later']::text[] \
          AND returned.instance_path_indices=ARRAY[0]::integer[]) \
         FROM iteration_scopes s JOIN collection_iterations i USING(tenant_id,run_id,scope_id) \
         WHERE s.tenant_id=$1 AND s.run_id=$2 AND s.owner_node='later' \
          AND s.owner_path_nodes='{}' AND s.owner_path_indices='{}' \
          AND i.iteration_index=0",
    )
    .bind(f.tenant.as_uuid())
    .bind(f.run.as_uuid())
    .fetch_one(&f.api.db.pool)
    .await
    .expect("later effect boundary");
    assert_eq!(later, ("OPEN".into(), "ADMITTED".into(), true, 0, 0));
    assert_closed_lines_source(f).await;
    let source = command(f).await;
    assert_eq!(certification_counts(f).await, (0, 0));
    let selected_before = selected_state(f).await;
    let progress_before_parent = ordinary_child_source_snapshot(f, fixture.parent).await;
    let run_before_parent = child_run_and_journal_without_parent_flags(f).await;
    let before_revision = source.expected_run_revision;
    let before_lease = lease_state(f).await;
    let key = format!("own-child-{label}-{}", Uuid::now_v7());
    let parent_snapshot = cancel_parent(f, fixture.parent, &key).await;
    let child: (i64, Option<String>, bool, bool, bool) = sqlx::query_as(
        "SELECT run_version,stop_reason,finished_at IS NOT NULL,parent_cancel,parent_settled \
         FROM runs WHERE tenant_id=$1 AND id=$2",
    )
    .bind(f.tenant.as_uuid())
    .bind(f.run.as_uuid())
    .fetch_one(&f.api.db.pool)
    .await
    .expect("child flags after parent PROPAGATE");
    assert_eq!(
        child,
        (
            i64::try_from(before_revision).expect("signed source revision"),
            None,
            false,
            true,
            true,
        )
    );
    assert_eq!(lease_state(f).await, before_lease);
    assert_eq!(selected_state(f).await, selected_before);
    let progress_after_parent = ordinary_child_source_snapshot(f, fixture.parent).await;
    assert_eq!(
        child_run_and_journal_without_parent_flags(f).await,
        run_before_parent,
    );
    assert_eq!(
        progress_after_parent.selected_source,
        progress_before_parent.selected_source
    );
    assert_eq!(
        progress_after_parent.certificate,
        progress_before_parent.certificate
    );
    assert_ne!(
        progress_after_parent.run_and_journal,
        progress_before_parent.run_and_journal
    );
    assert_eq!(progress_after_parent.steps, progress_before_parent.steps);
    assert_ne!(
        progress_after_parent.parent_flags,
        progress_before_parent.parent_flags
    );
    assert_eq!(certification_counts(f).await, (0, 0));
    Prepared {
        fixture,
        after,
        source,
        selected: selected_before,
        parent_key: key,
        parent_snapshot,
    }
}

fn advisory_key() -> i64 {
    i64::try_from(
        u64::from_be_bytes(
            Uuid::now_v7().as_bytes()[..8]
                .try_into()
                .expect("eight UUID bytes"),
        ) >> 1,
    )
    .expect("positive advisory key")
}

async fn install_stop_gate(f: &Flow, key: i64) {
    let ddl = format!(
        "CREATE FUNCTION leaf_child_own_stop_probe() RETURNS trigger \
         LANGUAGE plpgsql AS $$ BEGIN \
         IF NEW.tenant_id='{tenant}'::uuid AND NEW.run_id='{run}'::uuid \
            AND NEW.event_type='run.stop_requested' THEN \
           PERFORM pg_advisory_xact_lock({key}); \
         END IF; RETURN NEW; END $$; \
         CREATE CONSTRAINT TRIGGER leaf_child_own_stop_gate AFTER INSERT ON run_events \
         DEFERRABLE INITIALLY DEFERRED FOR EACH ROW \
         EXECUTE FUNCTION leaf_child_own_stop_probe();",
        tenant = f.tenant,
        run = f.run,
    );
    sqlx::raw_sql(sqlx::AssertSqlSafe(ddl))
        .execute(&f.api.db.pool)
        .await
        .expect("exact child stop deferred COMMIT gate");
}

async fn remove_stop_gate(f: &Flow) {
    sqlx::raw_sql(
        "DROP TRIGGER leaf_child_own_stop_gate ON run_events; \
         DROP FUNCTION leaf_child_own_stop_probe();",
    )
    .execute(&f.api.db.pool)
    .await
    .expect("remove exact child stop gate");
}

async fn install_certification_gate(f: &Flow, key: i64) {
    let ddl = format!(
        "CREATE FUNCTION leaf_child_own_cert_probe() RETURNS trigger \
         LANGUAGE plpgsql AS $$ BEGIN \
         IF NEW.tenant_id='{tenant}'::uuid AND NEW.run_id='{run}'::uuid THEN \
           PERFORM pg_advisory_xact_lock({key}); \
         END IF; RETURN NEW; END $$; \
         CREATE CONSTRAINT TRIGGER leaf_child_own_cert_gate \
         AFTER INSERT ON leaf_collection_success_certifications \
         DEFERRABLE INITIALLY DEFERRED FOR EACH ROW \
         EXECUTE FUNCTION leaf_child_own_cert_probe();",
        tenant = f.tenant,
        run = f.run,
    );
    sqlx::raw_sql(sqlx::AssertSqlSafe(ddl))
        .execute(&f.api.db.pool)
        .await
        .expect("exact child certification deferred COMMIT gate");
}

async fn remove_certification_gate(f: &Flow) {
    sqlx::raw_sql(
        "DROP TRIGGER leaf_child_own_cert_gate ON leaf_collection_success_certifications; \
         DROP FUNCTION leaf_child_own_cert_probe();",
    )
    .execute(&f.api.db.pool)
    .await
    .expect("remove exact child certification gate");
}

async fn three_pid_activity(pool: &PgPool, pids: [i32; 3]) -> Vec<ActivityWitness> {
    assert!(pids[0] != pids[1] && pids[0] != pids[2] && pids[1] != pids[2]);
    let rows = sqlx::query(
        "SELECT pid,datname,usename,state,query,wait_event_type,wait_event, \
         pg_blocking_pids(pid) AS blockers FROM pg_stat_activity \
         WHERE pid=ANY($1) ORDER BY array_position($1::integer[],pid)",
    )
    .bind(pids.to_vec())
    .fetch_all(pool)
    .await
    .expect("exact three-session activity witness");
    let rows = rows
        .into_iter()
        .map(|row| ActivityWitness {
            pid: row.get("pid"),
            datname: row.get("datname"),
            usename: row.get("usename"),
            state: row.get("state"),
            query: row.get("query"),
            wait_event_type: row.get("wait_event_type"),
            wait_event: row.get("wait_event"),
            blockers: row.get("blockers"),
        })
        .collect::<Vec<_>>();
    assert_eq!(rows.len(), 3);
    assert_eq!(rows.iter().map(|row| row.pid).collect::<Vec<_>>(), pids);
    let database = rows[0].datname.as_deref().expect("gate database recorded");
    assert!(
        rows.iter()
            .all(|row| row.datname.as_deref() == Some(database))
    );
    rows
}

async fn record_activity(pool: &PgPool, scenario: &str, kind: RaceWitness, pids: [i32; 3]) {
    let rows = three_pid_activity(pool, pids).await;
    let gate = &rows[0];
    assert!(gate.usename.as_deref().is_some_and(|role| !role.is_empty()));
    assert_eq!(gate.state.as_deref(), Some("idle in transaction"));
    assert!(
        gate.query
            .as_deref()
            .is_some_and(|query| query.contains("SELECT pg_advisory_xact_lock"))
    );
    assert!(gate.blockers.is_empty());
    let leader = &rows[1];
    let follower = &rows[2];
    for row in [leader, follower] {
        assert_eq!(row.usename.as_deref(), Some(RUNTIME_ROLE));
        assert_eq!(row.state.as_deref(), Some("active"));
        assert_eq!(row.wait_event_type.as_deref(), Some("Lock"));
        assert!(
            row.wait_event
                .as_deref()
                .is_some_and(|event| !event.is_empty())
        );
    }
    assert_eq!(leader.query.as_deref(), Some("COMMIT"));
    assert!(leader.blockers.contains(&gate.pid));
    let (fragment, predicate) = match kind {
        RaceWitness::StopThenCertify => COMPACT_RUN,
        RaceWitness::CertifyThenStop => FULL_RUN,
    };
    assert!(
        follower
            .query
            .as_deref()
            .is_some_and(|query| { query.contains(fragment) && query.contains(predicate) })
    );
    assert!(follower.blockers.contains(&leader.pid));
    let roles = match kind {
        RaceWitness::StopThenCertify => ["gate", "child_stop", "certifier"],
        RaceWitness::CertifyThenStop => ["gate", "certifier", "child_stop"],
    };
    let sessions = rows
        .iter()
        .zip(roles)
        .map(|(row, role)| {
            json!({
                "role": role,
                "pid": row.pid,
                "datname": row.datname.as_deref(),
                "usename": row.usename.as_deref(),
                "state": row.state.as_deref(),
                "query": row.query.as_deref(),
                "wait_event_type": row.wait_event_type.as_deref(),
                "wait_event": row.wait_event.as_deref(),
                "pg_blocking_pids": &row.blockers,
            })
        })
        .collect::<Vec<_>>();
    eprintln!(
        "{}",
        json!({
            "witness":"ordinary_child_own_cancel_commit_order",
            "scenario":scenario,
            "sessions":sessions,
        })
    );
}

async fn committing_behind_gate(pool: &PgPool, gate: i32, application: Option<&str>) -> i32 {
    loop {
        let pid = sqlx::query_scalar(
            "SELECT committing.pid FROM pg_stat_activity committing \
             JOIN pg_stat_activity held ON held.pid=$1 \
             WHERE committing.datname=current_database() \
              AND held.datname=committing.datname AND committing.usename='sagat_app' \
              AND committing.query='COMMIT' AND committing.wait_event_type='Lock' \
              AND $1=ANY(pg_blocking_pids(committing.pid)) \
              AND position('SELECT pg_advisory_xact_lock' in held.query)>0 \
              AND ($2::text IS NULL OR committing.application_name=$2)",
        )
        .bind(gate)
        .bind(application)
        .fetch_optional(pool)
        .await
        .expect("actual deferred COMMIT waiter");
        if let Some(pid) = pid {
            return pid;
        }
        tokio::task::yield_now().await;
    }
}

async fn query_behind_writer(pool: &PgPool, writer: i32, fragments: (&str, &str)) -> i32 {
    loop {
        let pid = sqlx::query_scalar(
            "SELECT waiter.pid FROM pg_stat_activity waiter \
             JOIN pg_stat_activity leader ON leader.pid=$1 \
             WHERE waiter.datname=current_database() AND leader.datname=waiter.datname \
              AND waiter.usename='sagat_app' AND waiter.wait_event_type='Lock' \
              AND $1=ANY(pg_blocking_pids(waiter.pid)) \
              AND position($2 in waiter.query)>0 AND position($3 in waiter.query)>0",
        )
        .bind(writer)
        .bind(fragments.0)
        .bind(fragments.1)
        .fetch_optional(pool)
        .await
        .expect("exact original-child Run waiter");
        if let Some(pid) = pid {
            return pid;
        }
        tokio::task::yield_now().await;
    }
}

async fn observe_commit<F: Future>(
    operation: Pin<&mut F>,
    pool: &PgPool,
    gate: i32,
    application: Option<&str>,
) -> i32 {
    tokio::select! {
        _ = operation => panic!("operation completed before its actual COMMIT witness"),
        pid = timeout(WAIT, committing_behind_gate(pool, gate, application)) =>
            pid.expect("actual deferred COMMIT witness deadline"),
    }
}

async fn observe_follower<A: Future, B: Future>(
    leader: Pin<&mut A>,
    follower: Pin<&mut B>,
    pool: &PgPool,
    writer: i32,
    fragments: (&str, &str),
) -> i32 {
    tokio::select! {
        _ = leader => panic!("COMMIT leader escaped its advisory gate"),
        _ = follower => panic!("follower escaped the original-child Run writer"),
        pid = timeout(WAIT, query_behind_writer(pool, writer, fragments)) =>
            pid.expect("exact writer/follower witness deadline"),
    }
}

async fn assert_own_guards(f: &Flow) {
    let mut run_probe = f.api.db.pool.begin().await.expect("fresh child Run probe");
    let error =
        sqlx::query("SELECT id FROM runs WHERE tenant_id=$1 AND id=$2 FOR KEY SHARE NOWAIT")
            .bind(f.tenant.as_uuid())
            .bind(f.run.as_uuid())
            .execute(&mut *run_probe)
            .await
            .expect_err("original child FOR UPDATE survives through COMMIT");
    assert_eq!(db_error(error).0, "55P03");
    drop(run_probe);

    let mut exclusive = f
        .api
        .db
        .pool
        .begin()
        .await
        .expect("fresh shard conflict probe");
    let error = sqlx::query(
        "SELECT shard FROM scheduler_shards \
         WHERE shard=run_shard($1) FOR NO KEY UPDATE NOWAIT",
    )
    .bind(f.run.as_uuid())
    .execute(&mut *exclusive)
    .await
    .expect_err("own-child scheduler SHARE survives through COMMIT");
    assert_eq!(db_error(error).0, "55P03");
    drop(exclusive);

    let mut shared = f
        .api
        .db
        .pool
        .begin()
        .await
        .expect("fresh shard SHARE probe");
    sqlx::query("SELECT shard FROM scheduler_shards WHERE shard=run_shard($1) FOR SHARE NOWAIT")
        .bind(f.run.as_uuid())
        .execute(&mut *shared)
        .await
        .expect("own-child scheduler guard remains shareable");
    shared.rollback().await.expect("finish SHARE probe");
}

fn assert_only_child_cancel(report: Report) {
    assert_eq!(report.children_cancelled, 1, "{report:?}");
    let mut remainder = report;
    remainder.children_cancelled = 0;
    assert!(
        !remainder.did_work(),
        "unexpected scheduler mutation: {report:?}"
    );
    assert_eq!((report.failed, report.fenced), (0, 0), "{report:?}");
}

async fn produce_child_cancel(f: &Flow) -> Report {
    for _ in 0..40 {
        let report = f.scheduler.tick().await;
        if report.children_cancelled == 1 {
            assert_only_child_cancel(report);
            return report;
        }
        assert_eq!(report.children_cancelled, 0, "{report:?}");
        assert!(
            !report.did_work(),
            "unexpected pre-stop scheduler mutation: {report:?}"
        );
        assert_eq!((report.failed, report.fenced), (0, 0), "{report:?}");
        tokio::task::yield_now().await;
    }
    panic!("real child-cancel producer was not discovered within its bounded polls")
}

async fn assert_pre_stop_invisible(f: &Flow, revision: u64, lease: &LeaseState) {
    let child: (String, i64, Option<String>, bool) = sqlx::query_as(
        "SELECT status,run_version,stop_reason,finished_at IS NOT NULL \
         FROM runs WHERE tenant_id=$1 AND id=$2",
    )
    .bind(f.tenant.as_uuid())
    .bind(f.run.as_uuid())
    .fetch_one(&f.api.db.pool)
    .await
    .expect("independent pre-COMMIT child state");
    assert_eq!(
        child,
        (
            "RUNNING".into(),
            i64::try_from(revision).expect("signed revision"),
            None,
            false,
        )
    );
    assert_eq!(lease_state(f).await, *lease);
    assert_eq!(certification_counts(f).await, (0, 0));
}

async fn assert_child_stopped(f: &Flow, revision: u64, lease: &LeaseState) {
    let child: (String, i64, Option<String>, bool, bool, bool) = sqlx::query_as(
        "SELECT status,run_version,stop_reason,finished_at IS NOT NULL,parent_cancel,parent_settled \
         FROM runs WHERE tenant_id=$1 AND id=$2",
    )
    .bind(f.tenant.as_uuid())
    .bind(f.run.as_uuid())
    .fetch_one(&f.api.db.pool)
    .await
    .expect("committed own-child stop");
    assert_eq!(
        child,
        (
            "CANCELLING".into(),
            i64::try_from(revision + 1).expect("signed stopped revision"),
            Some("CANCELLED".into()),
            false,
            true,
            true,
        )
    );
    let suffix: Vec<(i64, String, Option<String>, Value)> = sqlx::query_as(
        "SELECT seq,event_type,node,detail FROM run_events \
         WHERE tenant_id=$1 AND run_id=$2 AND seq>$3 ORDER BY seq",
    )
    .bind(f.tenant.as_uuid())
    .bind(f.run.as_uuid())
    .bind(i64::try_from(revision).expect("signed captured revision"))
    .fetch_all(&f.api.db.pool)
    .await
    .expect("exact child stop journal suffix");
    assert_eq!(
        suffix,
        vec![(
            i64::try_from(revision + 1).expect("signed stop sequence"),
            "run.stop_requested".into(),
            None,
            json!({"reason":"CANCELLED","code":null}),
        )]
    );
    assert_eq!(lease_state(f).await, *lease);
    let later: (String, String, bool, i64) = sqlx::query_as(
        "SELECT s.lifecycle,i.lifecycle,i.slot_held, \
         (SELECT count(*) FROM scope_sagas saga WHERE saga.tenant_id=s.tenant_id \
          AND saga.run_id=s.run_id AND saga.owner_node='later' \
          AND saga.owner_path_nodes='{}' AND saga.owner_path_indices='{}') \
         FROM iteration_scopes s JOIN collection_iterations i USING(tenant_id,run_id,scope_id) \
         WHERE s.tenant_id=$1 AND s.run_id=$2 AND s.owner_node='later' \
          AND s.owner_path_nodes='{}' AND s.owner_path_indices='{}' AND i.iteration_index=0",
    )
    .bind(f.tenant.as_uuid())
    .bind(f.run.as_uuid())
    .fetch_one(&f.api.db.pool)
    .await
    .expect("later scope is untouched by own-child stop");
    assert_eq!(later, ("OPEN".into(), "ADMITTED".into(), true, 0));
}

async fn stop_before_first_certification(prepared: &Prepared) -> Certified {
    let f = &prepared.fixture.flow;
    let revision = prepared.source.expected_run_revision;
    let lease = lease_state(f).await;
    grant_source_writer(f).await;
    let key = advisory_key();
    install_stop_gate(f, key).await;
    let mut gate = f
        .api
        .db
        .pool
        .begin()
        .await
        .expect("child stop COMMIT gate owner");
    let gate_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *gate)
        .await
        .expect("child stop gate PID");
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(key)
        .execute(&mut *gate)
        .await
        .expect("hold child stop COMMIT gate");
    let store = PgLeafCollectionSuccessStore::new(
        f.api.api_pool.clone(),
        DefinitionCache::new(Arc::new(JsonIrCodec), 8),
    );
    let producer = produce_child_cancel(f);
    tokio::pin!(producer);
    let stopper = observe_commit(producer.as_mut(), &f.api.db.pool, gate_pid, None).await;
    assert_ne!(stopper, gate_pid);
    let certification = store.certify_leaf_collection_success(&prepared.source);
    tokio::pin!(certification);
    let certifier = observe_follower(
        producer.as_mut(),
        certification.as_mut(),
        &f.api.db.pool,
        stopper,
        COMPACT_RUN,
    )
    .await;
    record_activity(
        &f.api.db.pool,
        "stop-before-certification",
        RaceWitness::StopThenCertify,
        [gate_pid, stopper, certifier],
    )
    .await;
    assert_own_guards(f).await;
    assert_pre_stop_invisible(f, revision, &lease).await;
    assert_eq!(selected_state(f).await, prepared.selected);
    gate.rollback()
        .await
        .expect("release only child stop COMMIT gate");
    let (report, first) = timeout(WAIT, async {
        tokio::join!(producer.as_mut(), certification.as_mut())
    })
    .await
    .expect("child stop and stale certifier complete");
    assert_only_child_cancel(report);
    assert_child_stopped(f, revision, &lease).await;
    assert_eq!(
        first,
        Err(LeafCollectionSuccessError::StaleRunRevision {
            current: revision + 1,
        })
    );
    assert_eq!(certification_counts(f).await, (0, 0));
    assert_eq!(selected_state(f).await, prepared.selected);

    let fresh = command(f).await;
    assert_eq!(fresh.expected_run_revision, revision + 1);
    let created = store
        .certify_leaf_collection_success(&fresh)
        .await
        .expect("fresh own-child CAS creates canonical source certificate");
    assert!(!created.replayed);
    assert_eq!(created.record.identity, prepared.source.identity);
    assert_eq!(created.record.certified_run_revision, revision + 1);
    assert_record_matches_actual_source(f, &created.record).await;
    assert_persisted_certificate_and_audit(f, &created.record).await;
    assert_eq!(certification_counts(f).await, (1, 1));
    let replay = store
        .certify_leaf_collection_success(&fresh)
        .await
        .expect("fresh creation command replays");
    assert!(replay.replayed);
    assert_eq!(replay.record, created.record);
    let mut original_current = prepared.source.clone();
    original_current.scheduler = fresh.scheduler.clone();
    let replay = store
        .certify_leaf_collection_success(&original_current)
        .await
        .expect("original stale CAS identity replays after canonical creation");
    assert!(replay.replayed);
    assert_eq!(replay.record, created.record);
    remove_stop_gate(f).await;
    revoke_source_writer(f).await;
    Certified {
        result: created,
        successful_command: fresh,
    }
}

async fn certification_before_stop(prepared: &Prepared) -> Certified {
    let f = &prepared.fixture.flow;
    let revision = prepared.source.expected_run_revision;
    let lease = lease_state(f).await;
    grant_source_writer(f).await;
    let key = advisory_key();
    install_certification_gate(f, key).await;
    let mut gate = f
        .api
        .db
        .pool
        .begin()
        .await
        .expect("certification COMMIT gate owner");
    let gate_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *gate)
        .await
        .expect("certification gate PID");
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(key)
        .execute(&mut *gate)
        .await
        .expect("hold certification COMMIT gate");
    let options: PgConnectOptions = f
        .api
        .db
        .url_for(RUNTIME_ROLE, RUNTIME_ROLE_PASSWORD)
        .parse()
        .expect("runtime database URL");
    let application = format!("sagat-child-own-{}", Uuid::now_v7());
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect_with(options.application_name(&application))
        .await
        .expect("isolated one-connection child certifier pool");
    let role: String = sqlx::query_scalar("SELECT current_user")
        .fetch_one(&pool)
        .await
        .expect("isolated certifier role");
    assert_eq!(role, RUNTIME_ROLE);
    let created = {
        let store = PgLeafCollectionSuccessStore::new(
            pool.clone(),
            DefinitionCache::new(Arc::new(JsonIrCodec), 8),
        );
        let certification = store.certify_leaf_collection_success(&prepared.source);
        tokio::pin!(certification);
        let certifier = observe_commit(
            certification.as_mut(),
            &f.api.db.pool,
            gate_pid,
            Some(&application),
        )
        .await;
        assert_ne!(certifier, gate_pid);
        let producer = produce_child_cancel(f);
        tokio::pin!(producer);
        let stopper = observe_follower(
            certification.as_mut(),
            producer.as_mut(),
            &f.api.db.pool,
            certifier,
            FULL_RUN,
        )
        .await;
        record_activity(
            &f.api.db.pool,
            "certification-before-stop",
            RaceWitness::CertifyThenStop,
            [gate_pid, certifier, stopper],
        )
        .await;
        assert_own_guards(f).await;
        assert_pre_stop_invisible(f, revision, &lease).await;
        assert_eq!(selected_state(f).await, prepared.selected);
        gate.rollback()
            .await
            .expect("release only certification COMMIT gate");
        let (created, report) = timeout(WAIT, async {
            tokio::join!(certification.as_mut(), producer.as_mut())
        })
        .await
        .expect("certifier and child stop complete");
        let created = created.expect("first captured CAS certifies before child stop");
        assert!(!created.replayed);
        assert_eq!(created.record.identity, prepared.source.identity);
        assert_eq!(created.record.certified_run_revision, revision);
        assert_only_child_cancel(report);
        assert_child_stopped(f, revision, &lease).await;
        assert_record_matches_actual_source(f, &created.record).await;
        assert_persisted_certificate_and_audit(f, &created.record).await;
        assert_eq!(certification_counts(f).await, (1, 1));
        let current = command(f).await;
        assert_eq!(current.expected_run_revision, revision + 1);
        let mut original_current = prepared.source.clone();
        original_current.scheduler = current.scheduler;
        let replay = store
            .certify_leaf_collection_success(&original_current)
            .await
            .expect("original certified CAS replays after child stop");
        assert!(replay.replayed);
        assert_eq!(replay.record, created.record);
        created
    };
    remove_certification_gate(f).await;
    revoke_source_writer(f).await;
    pool.close().await;
    Certified {
        result: created,
        successful_command: prepared.source.clone(),
    }
}

struct ExpectedEvent {
    event_type: &'static str,
    node: Option<&'static str>,
    path_nodes: &'static [&'static str],
    path_indices: &'static [i32],
    detail: Value,
}

async fn run_revision(f: &Flow) -> i64 {
    sqlx::query_scalar("SELECT run_version FROM runs WHERE tenant_id=$1 AND id=$2")
        .bind(f.tenant.as_uuid())
        .bind(f.run.as_uuid())
        .fetch_one(&f.api.db.pool)
        .await
        .expect("current child revision")
}

async fn assert_suffix(f: &Flow, before: i64, expected: &[ExpectedEvent]) -> i64 {
    let rows = sqlx::query(
        "SELECT seq,event_type,node,COALESCE(instance_path_nodes,'{}'::text[]) AS path_nodes, \
         COALESCE(instance_path_indices,'{}'::integer[]) AS path_indices,detail \
         FROM run_events WHERE tenant_id=$1 AND run_id=$2 AND seq>$3 ORDER BY seq",
    )
    .bind(f.tenant.as_uuid())
    .bind(f.run.as_uuid())
    .bind(before)
    .fetch_all(&f.api.db.pool)
    .await
    .expect("exact child journal suffix");
    assert_eq!(
        rows.len(),
        expected.len(),
        "actual addressed event types after {before}: {:?}",
        rows.iter()
            .map(|row| row.get::<String, _>("event_type"))
            .collect::<Vec<_>>()
    );
    for (offset, (row, expected)) in rows.iter().zip(expected).enumerate() {
        assert_eq!(
            row.get::<i64, _>("seq"),
            before + i64::try_from(offset).expect("bounded event offset") + 1,
        );
        assert_eq!(row.get::<String, _>("event_type"), expected.event_type);
        assert_eq!(
            row.get::<Option<String>, _>("node").as_deref(),
            expected.node
        );
        assert_eq!(
            row.get::<Vec<String>, _>("path_nodes"),
            expected
                .path_nodes
                .iter()
                .map(|part| (*part).to_owned())
                .collect::<Vec<_>>(),
        );
        assert_eq!(
            row.get::<Vec<i32>, _>("path_indices"),
            expected.path_indices.to_vec(),
        );
        assert_eq!(row.get::<Value, _>("detail"), expected.detail);
    }
    let after = run_revision(f).await;
    assert_eq!(
        after,
        before + i64::try_from(expected.len()).expect("bounded journal suffix")
    );
    after
}

async fn assert_after_cancelled(f: &Flow, step: Uuid) {
    let row = sqlx::query(
        "SELECT status,cancel_requested,routed,lease_token_hash IS NULL AS token_cleared, \
         lease_expires_at IS NULL AS expiry_cleared,leased_by IS NULL AS owner_cleared, \
         attempt_deadline_at IS NULL AS deadline_cleared,next_attempt_at IS NULL AS no_retry, \
         failure_code,step_id, \
         (SELECT count(*) FROM steps returned WHERE returned.tenant_id=$1 \
          AND returned.run_id=$2 AND returned.node='returned' \
          AND returned.instance_path_nodes=ARRAY['later']::text[] \
          AND returned.instance_path_indices=ARRAY[0]::integer[]) AS returned_count \
         FROM steps WHERE tenant_id=$1 AND run_id=$2 AND node='after' \
          AND instance_path_nodes=ARRAY['later']::text[] \
          AND instance_path_indices=ARRAY[0]::integer[]",
    )
    .bind(f.tenant.as_uuid())
    .bind(f.run.as_uuid())
    .fetch_one(&f.api.db.pool)
    .await
    .expect("acknowledged exact later successor cancellation");
    assert_eq!(row.get::<Uuid, _>("step_id"), step);
    assert_eq!(row.get::<String, _>("status"), "CANCELLED");
    assert!(!row.get::<bool, _>("cancel_requested"));
    assert!(row.get::<bool, _>("routed"));
    for field in [
        "token_cleared",
        "expiry_cleared",
        "owner_cleared",
        "deadline_cleared",
        "no_retry",
    ] {
        assert!(row.get::<bool, _>(field), "cancel acknowledgement: {field}");
    }
    assert_eq!(row.get::<Option<String>, _>("failure_code"), None);
    assert_eq!(row.get::<i64, _>("returned_count"), 0);
}

async fn continue_through_refund(prepared: &Prepared, certified: &Certified) {
    let f = &prepared.fixture.flow;
    let owner = sagat_domain::execution::ExecutionInstanceId::root(
        NodeId::parse("later").expect("later collection owner"),
    );
    let selected = selected_state(f).await;
    assert_selected(f, &selected, &certified.result.record).await;
    let parent = prepared.parent_snapshot.clone();
    let before_reservations: (i32, i32, i32) = sqlx::query_as(
        "SELECT i.remaining_reservations,u.reserved_instances,u.retired_reservations \
         FROM collection_iterations i JOIN iteration_scopes s USING(tenant_id,run_id,scope_id) \
         JOIN run_instance_usage u ON u.tenant_id=i.tenant_id AND u.run_id=i.run_id \
         WHERE i.tenant_id=$1 AND i.run_id=$2 AND s.owner_node='later' \
          AND s.owner_path_nodes='{}' AND s.owner_path_indices='{}' \
          AND i.iteration_index=0",
    )
    .bind(f.tenant.as_uuid())
    .bind(f.run.as_uuid())
    .fetch_one(&f.api.db.pool)
    .await
    .expect("later reservations before drain");

    let before = run_revision(f).await;
    assert_eq!(
        f.scheduler
            .advance_collection(f.tenant, f.run, &owner)
            .await
            .expect("addressed collection advance"),
        CollectionAdvance::DrainStarted
    );
    let after_start = assert_suffix(
        f,
        before,
        &[
            ExpectedEvent {
                event_type: "collection.drain_started",
                node: Some("later"),
                path_nodes: &[],
                path_indices: &[],
                detail: json!({"reason":"cancelled"}),
            },
            ExpectedEvent {
                event_type: "step.cancel_requested",
                node: Some("after"),
                path_nodes: &["later"],
                path_indices: &[0],
                detail: json!({}),
            },
        ],
    )
    .await;
    let draining: (String, String, bool, String, bool, i64) = sqlx::query_as(
        "SELECT s.lifecycle,i.lifecycle,i.slot_held,step.status,step.cancel_requested, \
         (SELECT count(*) FROM scope_sagas saga WHERE saga.tenant_id=s.tenant_id \
          AND saga.run_id=s.run_id AND saga.owner_node='later' \
          AND saga.owner_path_nodes='{}' AND saga.owner_path_indices='{}') \
         FROM iteration_scopes s JOIN collection_iterations i USING(tenant_id,run_id,scope_id) \
         JOIN steps step ON step.tenant_id=s.tenant_id AND step.run_id=s.run_id \
          AND step.node='after' AND step.instance_path_nodes=ARRAY['later']::text[] \
          AND step.instance_path_indices=ARRAY[0]::integer[] \
         WHERE s.tenant_id=$1 AND s.run_id=$2 AND s.owner_node='later' \
          AND s.owner_path_nodes='{}' AND s.owner_path_indices='{}' AND i.iteration_index=0",
    )
    .bind(f.tenant.as_uuid())
    .bind(f.run.as_uuid())
    .fetch_one(&f.api.db.pool)
    .await
    .expect("started exact later drain");
    assert_eq!(
        draining,
        (
            "DRAINING".into(),
            "ADMITTED".into(),
            true,
            "LEASED".into(),
            true,
            0
        )
    );
    assert_selected(f, &selected, &certified.result.record).await;

    let waiting_snapshot = exact_child_snapshot(f).await;
    assert_eq!(
        f.scheduler
            .advance_collection(f.tenant, f.run, &owner)
            .await
            .expect("addressed collection advance"),
        CollectionAdvance::Waiting
    );
    assert_eq!(run_revision(f).await, after_start);
    assert_eq!(exact_child_snapshot(f).await, waiting_snapshot);
    assert_selected(f, &selected, &certified.result.record).await;

    f.fail(&prepared.after, "CANCELLED", false).await;
    let after_ack = assert_suffix(
        f,
        after_start,
        &[ExpectedEvent {
            event_type: "step.cancelled",
            node: Some("after"),
            path_nodes: &["later"],
            path_indices: &[0],
            detail: json!({}),
        }],
    )
    .await;
    let after_step = Uuid::parse_str(&prepared.after.step_id).expect("successor step UUID");
    assert_after_cancelled(f, after_step).await;
    assert_selected(f, &selected, &certified.result.record).await;

    assert_eq!(
        f.scheduler
            .advance_collection(f.tenant, f.run, &owner)
            .await
            .expect("addressed collection advance"),
        CollectionAdvance::Drained { index: 0 }
    );
    let after_drain = assert_suffix(
        f,
        after_ack,
        &[ExpectedEvent {
            event_type: "collection.iteration_drained",
            node: Some("later"),
            path_nodes: &[],
            path_indices: &[],
            detail: json!({"index":0}),
        }],
    )
    .await;
    let drained = sqlx::query(
        "SELECT s.lifecycle AS scope_lifecycle,i.lifecycle AS iteration_lifecycle, \
         i.slot_held,i.remaining_reservations,s.active_count, \
         u.reserved_instances,u.retired_reservations, \
         (SELECT selected.remaining_reservations FROM collection_iterations selected \
          JOIN iteration_scopes source USING(tenant_id,run_id,scope_id) \
          WHERE selected.tenant_id=i.tenant_id AND selected.run_id=i.run_id \
           AND source.owner_node='lines' AND source.owner_path_nodes='{}' \
          AND source.owner_path_indices='{}' AND selected.iteration_index=0) AS selected_reserve \
         FROM iteration_scopes s JOIN collection_iterations i USING(tenant_id,run_id,scope_id) \
         JOIN run_instance_usage u ON u.tenant_id=i.tenant_id AND u.run_id=i.run_id \
         WHERE s.tenant_id=$1 AND s.run_id=$2 AND s.owner_node='later' \
          AND s.owner_path_nodes='{}' AND s.owner_path_indices='{}' AND i.iteration_index=0",
    )
    .bind(f.tenant.as_uuid())
    .bind(f.run.as_uuid())
    .fetch_one(&f.api.db.pool)
    .await
    .expect("exact drained later iteration and reservations");
    assert_eq!(drained.get::<String, _>("scope_lifecycle"), "DRAINING");
    assert_eq!(drained.get::<String, _>("iteration_lifecycle"), "DRAINED");
    assert!(!drained.get::<bool, _>("slot_held"));
    assert_eq!(drained.get::<i32, _>("remaining_reservations"), 1);
    assert_eq!(drained.get::<i32, _>("active_count"), 0);
    assert_eq!(
        drained.get::<i32, _>("selected_reserve"),
        0,
        "selected reserve remains zero"
    );
    let remaining = drained.get::<i32, _>("remaining_reservations");
    let reserved = drained.get::<i32, _>("reserved_instances");
    let retired = drained.get::<i32, _>("retired_reservations");
    assert!(
        before_reservations.0 > remaining,
        "unused forward reservation released"
    );
    assert_eq!(
        before_reservations.1 - reserved,
        before_reservations.0 - remaining
    );
    assert_eq!(
        retired - before_reservations.2,
        before_reservations.0 - remaining
    );
    assert_selected(f, &selected, &certified.result.record).await;

    assert_eq!(
        f.scheduler
            .advance_collection_from_run(f.tenant, f.run, &owner)
            .await
            .expect("addressed collection advance"),
        CollectionAdvance::Compensated
    );
    let after_release = assert_suffix(
        f,
        after_drain,
        &[
            ExpectedEvent {
                event_type: "scope.compensation_planned",
                node: Some("later"),
                path_nodes: &[],
                path_indices: &[],
                detail: json!({
                    "cause":"RUN_CANCELLED",
                    "entries":[{"instance":{"path":[{"scope":"later","index":0}],"node":"work"},"after":[]}],
                    "irreversible":[],
                }),
            },
            ExpectedEvent {
                event_type: "step.released",
                node: Some("work~c"),
                path_nodes: &["later"],
                path_indices: &[0],
                detail: json!({}),
            },
        ],
    )
    .await;
    let saga = sqlx::query(
        "SELECT phase,cause_code,during_stop,outcome, \
         (SELECT count(*) FROM scope_saga_entries e WHERE e.tenant_id=s.tenant_id \
          AND e.run_id=s.run_id AND e.owner_node='later' AND e.owner_path_nodes='{}' \
          AND e.owner_path_indices='{}' AND e.iteration_index=0 AND e.node='work' \
          AND e.kind='COMPENSATE') AS entry_count, \
         (SELECT count(*) FROM steps c WHERE c.tenant_id=s.tenant_id AND c.run_id=s.run_id \
          AND c.node='work~c' AND c.kind='COMPENSATION' \
          AND c.instance_path_nodes=ARRAY['later']::text[] \
          AND c.instance_path_indices=ARRAY[0]::integer[]) AS compensation_count, \
         (SELECT max(c.status) FROM steps c WHERE c.tenant_id=s.tenant_id AND c.run_id=s.run_id \
          AND c.node='work~c' AND c.kind='COMPENSATION' \
          AND c.instance_path_nodes=ARRAY['later']::text[] \
          AND c.instance_path_indices=ARRAY[0]::integer[]) AS compensation_status \
         FROM scope_sagas s WHERE s.tenant_id=$1 AND s.run_id=$2 \
          AND s.owner_node='later' AND s.owner_path_nodes='{}' AND s.owner_path_indices='{}'",
    )
    .bind(f.tenant.as_uuid())
    .bind(f.run.as_uuid())
    .fetch_one(&f.api.db.pool)
    .await
    .expect("planned exact later compensation");
    assert_eq!(saga.get::<String, _>("phase"), "COMPENSATING");
    assert_eq!(saga.get::<String, _>("cause_code"), "RUN_CANCELLED");
    assert!(saga.get::<bool, _>("during_stop"));
    assert_eq!(saga.get::<Option<String>, _>("outcome"), None);
    assert_eq!(saga.get::<i64, _>("entry_count"), 1);
    assert_eq!(saga.get::<i64, _>("compensation_count"), 1);
    assert_eq!(saga.get::<String, _>("compensation_status"), "READY");
    let selected_sagas: (i64, i64) = sqlx::query_as(
        "SELECT \
         (SELECT count(*) FROM scope_sagas s WHERE s.tenant_id=$1 AND s.run_id=$2 \
          AND s.owner_node='lines' AND s.owner_path_nodes='{}' AND s.owner_path_indices='{}'), \
         (SELECT count(*) FROM scope_saga_entries e WHERE e.tenant_id=$1 AND e.run_id=$2 \
          AND e.owner_node='lines' AND e.owner_path_nodes='{}' \
          AND e.owner_path_indices='{}' AND e.iteration_index=0)",
    )
    .bind(f.tenant.as_uuid())
    .bind(f.run.as_uuid())
    .fetch_one(&f.api.db.pool)
    .await
    .expect("selected source retains saga absence");
    assert_eq!(selected_sagas, (0, 0));
    assert_selected(f, &selected, &certified.result.record).await;

    // The existing domain command plans and releases eligible compensation in one commit.
    // No second scheduler call is needed before the real worker lease.
    let released: String = sqlx::query_scalar(
        "SELECT status FROM steps WHERE tenant_id=$1 AND run_id=$2 AND node='work~c' \
         AND kind='COMPENSATION' AND instance_path_nodes=ARRAY['later']::text[] \
         AND instance_path_indices=ARRAY[0]::integer[]",
    )
    .bind(f.tenant.as_uuid())
    .bind(f.run.as_uuid())
    .fetch_one(&f.api.db.pool)
    .await
    .expect("released exact refund work");
    assert_eq!(released, "READY");
    assert_selected(f, &selected, &certified.result.record).await;

    let tasks = f.poll().await;
    assert_eq!(tasks.len(), 1);
    let refund = &tasks[0];
    let refund_instance = refund.instance.as_ref().expect("refund identity");
    assert_eq!(refund_instance.definition_node_id, "work~c");
    assert_eq!(refund_instance.instance_path.len(), 1);
    assert_eq!(refund_instance.instance_path[0].scope_node_id, "later");
    assert_eq!(refund_instance.instance_path[0].iteration_index, 0);
    assert_eq!(refund.task_kind, "COMPENSATION");
    assert_eq!(refund.activity_name, "line.refund");
    assert!(run_revision(f).await > after_release);
    let refund_output = json!({"refunded":"known-effect"});
    let before_refund_completion = run_revision(f).await;
    f.complete(refund, refund_output).await;
    assert_suffix(
        f,
        before_refund_completion,
        &[
            ExpectedEvent {
                event_type: "step.completed",
                node: Some("work~c"),
                path_nodes: &["later"],
                path_indices: &[0],
                detail: json!({"attempt":refund.attempt,"generation":refund.generation}),
            },
            ExpectedEvent {
                event_type: "step.routed",
                node: Some("work~c"),
                path_nodes: &["later"],
                path_indices: &[0],
                detail: json!({"activated":[]}),
            },
        ],
    )
    .await;
    assert_selected(f, &selected, &certified.result.record).await;

    for _ in 0..4 {
        let phase: String = sqlx::query_scalar(
            "SELECT phase FROM scope_sagas WHERE tenant_id=$1 AND run_id=$2 \
             AND owner_node='later' AND owner_path_nodes='{}' AND owner_path_indices='{}'",
        )
        .bind(f.tenant.as_uuid())
        .bind(f.run.as_uuid())
        .fetch_one(&f.api.db.pool)
        .await
        .expect("inspect later saga before each explicit advance");
        if phase == "DONE" {
            break;
        }
        assert_eq!(phase, "COMPENSATING");
        assert_eq!(
            f.scheduler
                .advance_collection_from_run(f.tenant, f.run, &owner)
                .await
                .expect("addressed collection advance"),
            CollectionAdvance::Compensated
        );
    }
    let final_state = sqlx::query(
        "SELECT run.status,run.stop_reason,run.finished_at IS NULL AS child_unfinished, \
         scope.lifecycle AS scope_lifecycle,iteration.lifecycle AS iteration_lifecycle, \
         iteration.slot_held,saga.phase,saga.outcome, \
         refund.status AS refund_status,refund.routed AS refund_routed, \
         refund.output_digest IS NULL AS output_absent, \
         refund.lease_token_hash IS NULL AS token_cleared, \
         refund.lease_expires_at IS NULL AS expiry_cleared,refund.leased_by IS NULL AS owner_cleared, \
         (SELECT count(*) FROM run_events e WHERE e.tenant_id=run.tenant_id \
          AND e.run_id=run.id AND e.event_type='scope.compensated' AND e.node='later' \
          AND e.instance_path_nodes='{}' AND e.instance_path_indices='{}') AS compensated_events, \
         (SELECT count(*) FROM run_events e WHERE e.tenant_id=run.tenant_id \
          AND e.run_id=run.id AND e.event_type='run.finished') AS child_finished_events, \
         (SELECT count(*) FROM run_events e WHERE e.tenant_id=run.tenant_id \
          AND e.run_id=$3 AND e.event_type='child.finished') AS parent_finished_events \
         FROM runs run JOIN iteration_scopes scope ON scope.tenant_id=run.tenant_id \
          AND scope.run_id=run.id AND scope.owner_node='later' \
          AND scope.owner_path_nodes='{}' AND scope.owner_path_indices='{}' \
         JOIN collection_iterations iteration ON iteration.tenant_id=scope.tenant_id \
          AND iteration.run_id=scope.run_id AND iteration.scope_id=scope.scope_id \
         JOIN scope_sagas saga ON saga.tenant_id=run.tenant_id AND saga.run_id=run.id \
          AND saga.owner_node='later' AND saga.owner_path_nodes='{}' AND saga.owner_path_indices='{}' \
         JOIN steps refund ON refund.tenant_id=run.tenant_id AND refund.run_id=run.id \
          AND refund.node='work~c' AND refund.instance_path_nodes=ARRAY['later']::text[] \
          AND refund.instance_path_indices=ARRAY[0]::integer[] \
         WHERE run.tenant_id=$1 AND run.id=$2 AND iteration.iteration_index=0",
    )
    .bind(f.tenant.as_uuid())
    .bind(f.run.as_uuid())
    .bind(prepared.fixture.parent.as_uuid())
    .fetch_one(&f.api.db.pool)
    .await
    .expect("final unfinished child after real refund");
    assert_eq!(final_state.get::<String, _>("status"), "CANCELLING");
    assert_eq!(
        final_state.get::<Option<String>, _>("stop_reason"),
        Some("CANCELLED".into())
    );
    assert!(final_state.get::<bool, _>("child_unfinished"));
    assert_eq!(final_state.get::<String, _>("scope_lifecycle"), "DRAINING");
    assert_eq!(
        final_state.get::<String, _>("iteration_lifecycle"),
        "DRAINED"
    );
    assert!(!final_state.get::<bool, _>("slot_held"));
    assert_eq!(final_state.get::<String, _>("phase"), "DONE");
    assert_eq!(
        final_state.get::<Option<String>, _>("outcome"),
        Some("COMPENSATED".into())
    );
    assert_eq!(final_state.get::<String, _>("refund_status"), "SUCCEEDED");
    assert!(final_state.get::<bool, _>("refund_routed"));
    assert!(
        final_state.get::<bool, _>("output_absent"),
        "current scoped compensation completion deliberately persists no output",
    );
    for field in ["token_cleared", "expiry_cleared", "owner_cleared"] {
        assert!(
            final_state.get::<bool, _>(field),
            "refund invariant {field}"
        );
    }
    assert_eq!(final_state.get::<i64, _>("compensated_events"), 1);
    assert_eq!(final_state.get::<i64, _>("child_finished_events"), 0);
    assert_eq!(final_state.get::<i64, _>("parent_finished_events"), 0);
    assert_selected(f, &selected, &certified.result.record).await;
    assert_eq!(
        parent_snapshot(f, prepared.fixture.parent, &prepared.parent_key).await,
        parent,
        "settled parent remains unchanged",
    );

    let current = command(f).await;
    let mut replay_command = certified.successful_command.clone();
    replay_command.scheduler = current.scheduler;
    let pool = f.api.db.runtime_pool_with(2).await;
    let role: String = sqlx::query_scalar("SELECT current_user")
        .fetch_one(&pool)
        .await
        .expect("cold replay role");
    assert_eq!(role, RUNTIME_ROLE);
    let cold = PgLeafCollectionSuccessStore::new(
        pool.clone(),
        DefinitionCache::new(Arc::new(JsonIrCodec), 8),
    );
    let before_replay = exact_child_snapshot(f).await;
    let replay = cold
        .certify_leaf_collection_success(&replay_command)
        .await
        .expect("original successful CAS replays after child revision growth");
    assert!(replay.replayed);
    assert_eq!(replay.record, certified.result.record);
    assert_eq!(certification_counts(f).await, (1, 1));
    assert_selected(f, &selected, &certified.result.record).await;
    assert_eq!(exact_child_snapshot(f).await, before_replay);
    drop(cold);
    pool.close().await;
    assert_eq!(
        parent_snapshot(f, prepared.fixture.parent, &prepared.parent_key).await,
        parent,
    );
    f.verify().await;
    f.api
        .executions
        .verify_run(f.tenant, prepared.fixture.parent)
        .await
        .expect("settled parent journal rebuilds at unfinished-child boundary");
}

#[tokio::test]
async fn parent_propagate_child_stop_commits_before_first_source_certification() {
    let prepared = prepare("own-stop-first").await;
    let certified = stop_before_first_certification(&prepared).await;
    continue_through_refund(&prepared, &certified).await;
}

#[tokio::test]
async fn first_child_source_certification_commits_before_parent_propagate_child_stop() {
    let prepared = prepare("own-cert-first").await;
    let certified = certification_before_stop(&prepared).await;
    continue_through_refund(&prepared, &certified).await;
}
