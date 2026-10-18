//! Existing incoming parent-flag writers versus first certification of a genuine ordinary child.
//!
//! This is a characterization test only. It adds no production caller, grant, or source guarantee.

use super::*;
use crate::support::{RUNTIME_ROLE, RUNTIME_ROLE_PASSWORD, db_error};
use sqlx::{
    PgPool, Row,
    postgres::{PgConnectOptions, PgPoolOptions},
};
use std::{future::Future, pin::Pin, time::Duration};
use tokio::time::timeout;

const WAIT: Duration = Duration::from_secs(10);
const CERTIFIER_SQL: &str = "SELECT workflow_version_id,run_version,trace_id FROM runs";
const CERTIFIER_PREDICATE: &str = "WHERE tenant_id=$1 AND namespace_id=$2 AND id=$3 FOR UPDATE";
const SETTLE_SQL: &str = "UPDATE runs SET parent_settled = true";
const REQUEST_SQL: &str = "UPDATE runs SET parent_cancel = true";
const CHILD_PREDICATE: &str = "WHERE tenant_id = $1 AND id = $2 AND parent_run_id = $3";

#[derive(Clone, Copy, Debug)]
struct Scenario {
    label: &'static str,
    policy: CancelPolicy,
    certifier_first: bool,
}

struct ExpectedParentEvent {
    event_type: &'static str,
    node: Option<&'static str>,
    detail: Value,
}

struct ExpectedParentOutcome {
    status: &'static str,
    finished: bool,
    parent_cancel: bool,
    parent_settled: bool,
    kid_status: &'static str,
    waiting_kind: Option<&'static str>,
    cancel_requested: bool,
    routed: bool,
    suffix: Vec<ExpectedParentEvent>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ChildState {
    selected_source: String,
    certificate: String,
    certification_audit: String,
    run_core_and_journal: String,
    steps: String,
    later_step: String,
    run_revision: i64,
    parent_cancel: bool,
    parent_settled: bool,
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

async fn three_pid_activity(pool: &PgPool, pids: [i32; 3]) -> Vec<ActivityWitness> {
    assert!(
        pids[0] != pids[1] && pids[0] != pids[2] && pids[1] != pids[2],
        "the activity witness requires three distinct PIDs",
    );
    let rows = sqlx::query(
        "SELECT pid,datname,usename,state,query,wait_event_type,wait_event,                pg_blocking_pids(pid) AS blockers          FROM pg_stat_activity WHERE pid=ANY($1)          ORDER BY array_position($1::integer[],pid)",
    )
    .bind(pids.to_vec())
    .fetch_all(pool)
    .await
    .expect("exact three-PID activity witness");
    let activity = rows
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
    assert_eq!(activity.len(), 3);
    assert_eq!(
        activity.iter().map(|row| row.pid).collect::<Vec<_>>(),
        pids.to_vec(),
        "the activity witness records the three exact requested PIDs in role order",
    );
    let database = activity[0]
        .datname
        .as_deref()
        .expect("holder/gate database is recorded");
    assert!(
        activity
            .iter()
            .all(|row| row.datname.as_deref() == Some(database)),
        "all three blocked sessions belong to one database",
    );
    activity
}

fn emit_activity_witness(kind: &str, scenario: &str, roles: [&str; 3], rows: &[ActivityWitness]) {
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
            "witness": kind,
            "scenario": scenario,
            "sessions": sessions,
        }),
    );
}

async fn record_queue_activity(
    pool: &PgPool,
    scenario: &str,
    holder: i32,
    leader: i32,
    follower: i32,
    first: (&str, &str),
    second: (&str, &str),
) {
    let rows = three_pid_activity(pool, [holder, leader, follower]).await;
    let held = &rows[0];
    assert!(
        held.usename.as_deref().is_some_and(|role| !role.is_empty()),
        "holder role is recorded",
    );
    assert_eq!(held.state.as_deref(), Some("idle in transaction"));
    assert!(held.query.as_deref().is_some_and(|query| {
        query.contains("SELECT 1 FROM runs WHERE tenant_id=$1 AND id=$2 FOR UPDATE")
    }),);
    assert!(held.blockers.is_empty());

    for (row, operation) in [(&rows[1], first), (&rows[2], second)] {
        assert_eq!(row.usename.as_deref(), Some(RUNTIME_ROLE));
        assert_eq!(row.state.as_deref(), Some("active"));
        assert_eq!(row.wait_event_type.as_deref(), Some("Lock"));
        assert!(
            row.wait_event
                .as_deref()
                .is_some_and(|event| !event.is_empty())
        );
        assert!(
            row.query
                .as_deref()
                .is_some_and(|query| query.contains(operation.0) && query.contains(operation.1)),
        );
    }
    assert!(rows[1].blockers.contains(&holder));
    assert!(rows[2].blockers.contains(&leader));
    emit_activity_witness(
        "ordinary_child_parent_flag_queue",
        scenario,
        ["holder", "leader", "follower"],
        &rows,
    );
}

async fn record_commit_activity(
    pool: &PgPool,
    scenario: &str,
    gate: i32,
    committing: i32,
    parent: i32,
) {
    let rows = three_pid_activity(pool, [gate, committing, parent]).await;
    let held = &rows[0];
    assert!(
        held.usename.as_deref().is_some_and(|role| !role.is_empty()),
        "advisory gate role is recorded",
    );
    assert_eq!(held.state.as_deref(), Some("idle in transaction"));
    assert!(
        held.query
            .as_deref()
            .is_some_and(|query| query.contains("SELECT pg_advisory_xact_lock")),
    );
    assert!(held.blockers.is_empty());

    let commit = &rows[1];
    assert_eq!(commit.usename.as_deref(), Some(RUNTIME_ROLE));
    assert_eq!(commit.state.as_deref(), Some("active"));
    assert_eq!(commit.query.as_deref(), Some("COMMIT"));
    assert_eq!(commit.wait_event_type.as_deref(), Some("Lock"));
    assert!(
        commit
            .wait_event
            .as_deref()
            .is_some_and(|event| !event.is_empty()),
    );
    assert!(commit.blockers.contains(&gate));

    let incoming = &rows[2];
    assert_eq!(incoming.usename.as_deref(), Some(RUNTIME_ROLE));
    assert_eq!(incoming.state.as_deref(), Some("active"));
    assert_eq!(incoming.wait_event_type.as_deref(), Some("Lock"));
    assert!(
        incoming
            .wait_event
            .as_deref()
            .is_some_and(|event| !event.is_empty()),
    );
    assert!(
        incoming
            .query
            .as_deref()
            .is_some_and(|query| { query.contains(SETTLE_SQL) && query.contains(CHILD_PREDICATE) }),
    );
    assert!(incoming.blockers.contains(&committing));
    emit_activity_witness(
        "ordinary_child_parent_flag_commit",
        scenario,
        ["gate", "committer", "parent"],
        &rows,
    );
}

struct ChildRunHold {
    tx: sqlx::Transaction<'static, sqlx::Postgres>,
    pid: i32,
}

impl ChildRunHold {
    async fn take(f: &Flow, child: RunId) -> Self {
        let mut tx = f.api.db.pool.begin().await.expect("child hold transaction");
        let pid = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *tx)
            .await
            .expect("child holder pid");
        sqlx::query("SELECT 1 FROM runs WHERE tenant_id=$1 AND id=$2 FOR UPDATE")
            .bind(f.tenant.as_uuid())
            .bind(child.as_uuid())
            .execute(&mut *tx)
            .await
            .expect("hold the exact original child Run");
        Self { tx, pid }
    }

    async fn release(self) {
        self.tx.rollback().await.expect("release exact child Run");
    }
}

fn writer_fragment(policy: CancelPolicy) -> &'static str {
    match policy {
        CancelPolicy::Propagate | CancelPolicy::Detach => SETTLE_SQL,
        CancelPolicy::RequestCancel => REQUEST_SQL,
    }
}

async fn child_state(f: &Flow, child: RunId) -> ChildState {
    let original = f.run;
    assert_eq!(
        original, child,
        "the reusable fixture remains bound to its child"
    );
    let selected_source = selected_sources(f).await;
    let certificate = sqlx::query_scalar(
        "SELECT md5(COALESCE((SELECT jsonb_agg(to_jsonb(c) ORDER BY          c.owner_node,c.iteration_index) FROM leaf_collection_success_certifications c          WHERE c.tenant_id=$1 AND c.run_id=$2 AND c.owner_node='lines'           AND c.iteration_index=0),'[]'::jsonb)::text)",
    )
    .bind(f.tenant.as_uuid())
    .bind(child.as_uuid())
    .fetch_one(&f.api.db.pool)
    .await
    .expect("qualified child certificate snapshot");
    let namespace: Uuid =
        sqlx::query_scalar("SELECT namespace_id FROM runs WHERE tenant_id=$1 AND id=$2")
            .bind(f.tenant.as_uuid())
            .bind(child.as_uuid())
            .fetch_one(&f.api.db.pool)
            .await
            .expect("qualified child namespace");
    let certification_audit = sqlx::query_scalar(
        "SELECT md5(COALESCE((SELECT jsonb_agg(to_jsonb(a) ORDER BY a.id)          FROM audit_records a WHERE a.tenant_id=$1           AND a.action='collection.leaf_success_certified'           AND a.resource_type='leaf_collection_success'           AND a.resource_id=concat($2::text,'/',$3::text,'/lines/0')),'[]'::jsonb)::text)",
    )
    .bind(f.tenant.as_uuid())
    .bind(namespace)
    .bind(child.as_uuid())
    .fetch_one(&f.api.db.pool)
    .await
    .expect("qualified child certification audit snapshot");
    let run_core_and_journal = sqlx::query_scalar(
        "SELECT md5(jsonb_build_object(         'run',(SELECT to_jsonb(r)-'parent_cancel'-'parent_settled' FROM runs r           WHERE r.tenant_id=$1 AND r.id=$2),         'journal',(SELECT COALESCE(jsonb_agg(to_jsonb(e) ORDER BY e.seq),'[]'::jsonb)           FROM run_events e WHERE e.tenant_id=$1 AND e.run_id=$2))::text)",
    )
    .bind(f.tenant.as_uuid())
    .bind(child.as_uuid())
    .fetch_one(&f.api.db.pool)
    .await
    .expect("child Run excluding incoming flags and full journal");
    let steps = sqlx::query_scalar(
        "SELECT md5(COALESCE((SELECT jsonb_agg(to_jsonb(s) ORDER BY          s.instance_path_nodes,s.instance_path_indices,s.node) FROM steps s          WHERE s.tenant_id=$1 AND s.run_id=$2),'[]'::jsonb)::text)",
    )
    .bind(f.tenant.as_uuid())
    .bind(child.as_uuid())
    .fetch_one(&f.api.db.pool)
    .await
    .expect("full child steps snapshot");
    let later_step = sqlx::query_scalar(
        "SELECT md5(to_jsonb(s)::text) FROM steps s          WHERE s.tenant_id=$1 AND s.run_id=$2 AND s.node='work'           AND s.instance_path_nodes=ARRAY['later']::text[]           AND s.instance_path_indices=ARRAY[0]::integer[]",
    )
    .bind(f.tenant.as_uuid())
    .bind(child.as_uuid())
    .fetch_one(&f.api.db.pool)
    .await
    .expect("exact leased later step");
    let (run_revision, parent_cancel, parent_settled): (i64, bool, bool) = sqlx::query_as(
        "SELECT run_version,parent_cancel,parent_settled FROM runs          WHERE tenant_id=$1 AND id=$2",
    )
    .bind(f.tenant.as_uuid())
    .bind(child.as_uuid())
    .fetch_one(&f.api.db.pool)
    .await
    .expect("child revision and incoming flags");
    ChildState {
        selected_source,
        certificate,
        certification_audit,
        run_core_and_journal,
        steps,
        later_step,
        run_revision,
        parent_cancel,
        parent_settled,
    }
}

async fn parent_snapshot(f: &Flow, parent: RunId, key: &str) -> String {
    sqlx::query_scalar(
        "SELECT md5(jsonb_build_object(         'run',(SELECT to_jsonb(r) FROM runs r WHERE r.tenant_id=$1 AND r.id=$2),         'journal',(SELECT COALESCE(jsonb_agg(to_jsonb(e) ORDER BY e.seq),'[]'::jsonb)           FROM run_events e WHERE e.tenant_id=$1 AND e.run_id=$2),         'steps',(SELECT COALESCE(jsonb_agg(to_jsonb(s) ORDER BY s.node),'[]'::jsonb)           FROM steps s WHERE s.tenant_id=$1 AND s.run_id=$2),         'audit',(SELECT COALESCE(jsonb_agg(to_jsonb(a) ORDER BY a.id),'[]'::jsonb)           FROM audit_records a WHERE a.tenant_id=$1 AND a.resource_type='run'            AND a.resource_id=$2::text AND a.action='run.cancel_requested'),         'dedupe',(SELECT COALESCE(jsonb_agg(to_jsonb(d) ORDER BY d.idempotency_key),'[]'::jsonb)           FROM command_dedupe d WHERE d.tenant_id=$1            AND d.scope=concat('run.operate:',$2::text) AND d.idempotency_key=$3),         'finished',(SELECT COALESCE(jsonb_agg(to_jsonb(o) ORDER BY o.id),'[]'::jsonb)           FROM event_outbox o WHERE o.tenant_id=$1 AND o.aggregate_id=$2            AND o.event_type='run.finished'))::text)",
    )
    .bind(f.tenant.as_uuid())
    .bind(parent.as_uuid())
    .bind(key)
    .fetch_one(&f.api.db.pool)
    .await
    .expect("qualified parent command snapshot")
}

async fn parent_revision(f: &Flow, parent: RunId) -> i64 {
    sqlx::query_scalar("SELECT run_version FROM runs WHERE tenant_id=$1 AND id=$2")
        .bind(f.tenant.as_uuid())
        .bind(parent.as_uuid())
        .fetch_one(&f.api.db.pool)
        .await
        .expect("parent revision before CANCEL")
}

async fn cancel_parent(f: &Flow, parent: RunId, key: &str) -> (StatusCode, Value) {
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
        .json(&json!({"action": "CANCEL", "reason": "parent flag race proof"}))
        .send()
        .await
        .expect("public parent CANCEL response");
    let status = response.status();
    let body = crate::support::api::json_of(response).await;
    (status, body)
}

async fn first_waiter(
    pool: &PgPool,
    holder: i32,
    fragment: &str,
    predicate: &str,
) -> (i32, String, Vec<i32>) {
    loop {
        let observed: Option<(i32, String, Vec<i32>)> = sqlx::query_as(
            "SELECT waiter.pid,waiter.query,pg_blocking_pids(waiter.pid)              FROM pg_stat_activity waiter JOIN pg_stat_activity held ON held.pid=$1              WHERE waiter.datname=current_database() AND held.datname=waiter.datname               AND waiter.usename='sagat_app' AND waiter.wait_event_type='Lock'               AND $1=ANY(pg_blocking_pids(waiter.pid))               AND position($2 in waiter.query)>0 AND position($3 in waiter.query)>0               AND position('SELECT 1 FROM runs WHERE tenant_id=$1 AND id=$2 FOR UPDATE'                   in held.query)>0",
        )
        .bind(holder)
        .bind(fragment)
        .bind(predicate)
        .fetch_optional(pool)
        .await
        .expect("first exact child-Run waiter");
        if let Some(observed) = observed {
            return observed;
        }
        tokio::task::yield_now().await;
    }
}

async fn ordered_waiters(
    pool: &PgPool,
    holder: i32,
    first: (&str, &str),
    second: (&str, &str),
) -> (i32, i32, String, String, Vec<i32>, Vec<i32>) {
    loop {
        let observed = sqlx::query_as(
            "SELECT leader.pid,follower.pid,leader.query,follower.query,                    pg_blocking_pids(leader.pid),pg_blocking_pids(follower.pid)              FROM pg_stat_activity leader JOIN pg_stat_activity follower                ON leader.pid=ANY(pg_blocking_pids(follower.pid))              JOIN pg_stat_activity held ON held.pid=$1              WHERE leader.datname=current_database() AND follower.datname=leader.datname               AND held.datname=leader.datname               AND leader.usename='sagat_app' AND follower.usename='sagat_app'               AND leader.wait_event_type='Lock' AND follower.wait_event_type='Lock'               AND $1=ANY(pg_blocking_pids(leader.pid))               AND position($2 in leader.query)>0 AND position($3 in leader.query)>0               AND position($4 in follower.query)>0 AND position($5 in follower.query)>0               AND position('SELECT 1 FROM runs WHERE tenant_id=$1 AND id=$2 FOR UPDATE'                   in held.query)>0",
        )
        .bind(holder)
        .bind(first.0)
        .bind(first.1)
        .bind(second.0)
        .bind(second.1)
        .fetch_optional(pool)
        .await
        .expect("exact child-Run queue direction");
        if let Some(observed) = observed {
            return observed;
        }
        tokio::task::yield_now().await;
    }
}

async fn observe_first<F: Future>(
    operation: Pin<&mut F>,
    pool: &PgPool,
    holder: i32,
    fragment: &str,
    predicate: &str,
) -> (i32, String, Vec<i32>) {
    tokio::select! {
        _ = operation => panic!("selected queue leader bypassed child hold"),
        observed = timeout(WAIT, first_waiter(pool, holder, fragment, predicate)) =>
            observed.expect("selected queue leader witness deadline"),
    }
}

async fn observe_order<A: Future, B: Future>(
    first_future: Pin<&mut A>,
    second_future: Pin<&mut B>,
    pool: &PgPool,
    holder: i32,
    first: (&str, &str),
    second: (&str, &str),
) -> (i32, i32, String, String, Vec<i32>, Vec<i32>) {
    tokio::select! {
        _ = first_future => panic!("queue leader completed before exact order witness"),
        _ = second_future => panic!("queue follower completed before exact order witness"),
        observed = timeout(
            WAIT,
            ordered_waiters(pool, holder, first, second),
        ) =>
            observed.expect("two-writer queue direction deadline"),
    }
}

async fn assert_parent_locked(f: &Flow, parent: RunId) {
    let mut probe = f
        .api
        .db
        .pool
        .begin()
        .await
        .expect("fresh parent lock probe");
    let error =
        sqlx::query("SELECT id FROM runs WHERE tenant_id=$1 AND id=$2 FOR KEY SHARE NOWAIT")
            .bind(f.tenant.as_uuid())
            .bind(parent.as_uuid())
            .execute(&mut *probe)
            .await
            .expect_err("public CANCEL retains its parent Run guard");
    assert_eq!(db_error(error).0, "55P03");
}

async fn grant_source_writer(f: &Flow) {
    sqlx::query("GRANT INSERT ON leaf_collection_success_certifications TO sagat_app")
        .execute(&f.api.db.pool)
        .await
        .expect("temporary child-source writer");
    assert_source_insert_privilege(f, true).await;
}

async fn revoke_source_writer(f: &Flow) {
    sqlx::query("REVOKE INSERT ON leaf_collection_success_certifications FROM sagat_app")
        .execute(&f.api.db.pool)
        .await
        .expect("remove temporary child-source writer");
    assert_source_insert_privilege(f, false).await;
}

async fn assert_parent_outcome(
    f: &Flow,
    parent: RunId,
    child: RunId,
    policy: CancelPolicy,
    key: &str,
    before_revision: i64,
    response: (StatusCode, Value),
) {
    assert_eq!(response.0, StatusCode::ACCEPTED);
    assert_eq!(response.1["accepted"], true);
    let row = sqlx::query(
        "SELECT parent.status,parent.run_version,parent.stop_reason,         parent.finished_at IS NOT NULL AS parent_finished,         child.parent_cancel,child.parent_settled,         kid.status AS kid_status,kid.waiting_kind,kid.cancel_requested,kid.routed,         kid.child_run_id,kid.child_started,         (SELECT count(*) FROM steps done WHERE done.tenant_id=parent.tenant_id           AND done.run_id=parent.id AND done.node='done') AS done_count,         (SELECT count(*) FROM run_events own WHERE own.tenant_id=child.tenant_id           AND own.run_id=child.id AND own.event_type='run.finished') AS child_finished_count          FROM runs parent JOIN runs child ON child.tenant_id=parent.tenant_id           AND child.id=$3 AND child.parent_run_id=parent.id          JOIN steps kid ON kid.tenant_id=parent.tenant_id AND kid.run_id=parent.id           AND kid.node='kid' AND kid.instance_path_nodes='{}'           AND kid.instance_path_indices='{}'          WHERE parent.tenant_id=$1 AND parent.id=$2",
    )
    .bind(f.tenant.as_uuid())
    .bind(parent.as_uuid())
    .bind(child.as_uuid())
    .fetch_one(&f.api.db.pool)
    .await
    .expect("exact parent, child relationship and kid projection");

    let expected = match policy {
        CancelPolicy::Propagate => ExpectedParentOutcome {
            status: "CANCELLED",
            finished: true,
            parent_cancel: true,
            parent_settled: true,
            kid_status: "CANCELLED",
            waiting_kind: None,
            cancel_requested: false,
            routed: true,
            suffix: vec![
                ExpectedParentEvent {
                    event_type: "run.stop_requested",
                    node: None,
                    detail: json!({"reason":"CANCELLED","code":null}),
                },
                ExpectedParentEvent {
                    event_type: "step.cancelled",
                    node: Some("kid"),
                    detail: json!({}),
                },
                ExpectedParentEvent {
                    event_type: "run.finished",
                    node: None,
                    detail: json!({"status":"CANCELLED"}),
                },
            ],
        },
        CancelPolicy::Detach => ExpectedParentOutcome {
            status: "CANCELLED",
            finished: true,
            parent_cancel: false,
            parent_settled: true,
            kid_status: "CANCELLED",
            waiting_kind: None,
            cancel_requested: false,
            routed: true,
            suffix: vec![
                ExpectedParentEvent {
                    event_type: "run.stop_requested",
                    node: None,
                    detail: json!({"reason":"CANCELLED","code":null}),
                },
                ExpectedParentEvent {
                    event_type: "step.cancelled",
                    node: Some("kid"),
                    detail: json!({}),
                },
                ExpectedParentEvent {
                    event_type: "run.finished",
                    node: None,
                    detail: json!({"status":"CANCELLED"}),
                },
            ],
        },
        CancelPolicy::RequestCancel => ExpectedParentOutcome {
            status: "CANCELLING",
            finished: false,
            parent_cancel: true,
            parent_settled: false,
            kid_status: "WAITING",
            waiting_kind: Some("CHILD"),
            cancel_requested: true,
            routed: false,
            suffix: vec![
                ExpectedParentEvent {
                    event_type: "run.stop_requested",
                    node: None,
                    detail: json!({"reason":"CANCELLED","code":null}),
                },
                ExpectedParentEvent {
                    event_type: "step.cancel_requested",
                    node: Some("kid"),
                    detail: json!({}),
                },
            ],
        },
    };
    let after_revision = row.get::<i64, _>("run_version");
    assert_eq!(row.get::<String, _>("status"), expected.status);
    assert_eq!(
        row.get::<Option<String>, _>("stop_reason").as_deref(),
        Some("CANCELLED")
    );
    assert_eq!(row.get::<bool, _>("parent_finished"), expected.finished);
    assert_eq!(row.get::<bool, _>("parent_cancel"), expected.parent_cancel);
    assert_eq!(
        row.get::<bool, _>("parent_settled"),
        expected.parent_settled
    );
    assert_eq!(row.get::<String, _>("kid_status"), expected.kid_status);
    assert_eq!(
        row.get::<Option<String>, _>("waiting_kind").as_deref(),
        expected.waiting_kind,
    );
    assert_eq!(
        row.get::<bool, _>("cancel_requested"),
        expected.cancel_requested
    );
    assert_eq!(row.get::<bool, _>("routed"), expected.routed);
    assert_eq!(
        row.get::<Option<Uuid>, _>("child_run_id"),
        Some(child.as_uuid())
    );
    assert!(row.get::<bool, _>("child_started"));
    assert_eq!(
        row.get::<i64, _>("done_count"),
        0,
        "successor End remains absent"
    );
    assert_eq!(
        row.get::<i64, _>("child_finished_count"),
        0,
        "unfinished child has no run.finished event",
    );
    assert_eq!(
        after_revision,
        before_revision + i64::try_from(expected.suffix.len()).expect("bounded suffix"),
    );
    assert_eq!(response.1["runStatus"], expected.status);
    assert_eq!(response.1["runVersion"], after_revision);

    let events: Vec<(i64, String, Option<String>, Value)> = sqlx::query_as(
        "SELECT seq,event_type,node,detail FROM run_events          WHERE tenant_id=$1 AND run_id=$2 AND seq>$3 ORDER BY seq",
    )
    .bind(f.tenant.as_uuid())
    .bind(parent.as_uuid())
    .bind(before_revision)
    .fetch_all(&f.api.db.pool)
    .await
    .expect("exact parent CANCEL journal suffix");
    assert_eq!(events.len(), expected.suffix.len());
    for (offset, ((sequence, event_type, node, detail), expected)) in
        events.iter().zip(&expected.suffix).enumerate()
    {
        assert_eq!(
            *sequence,
            before_revision + i64::try_from(offset).expect("bounded event offset") + 1,
        );
        assert_eq!(event_type, expected.event_type);
        assert_eq!(node.as_deref(), expected.node);
        assert_eq!(detail, &expected.detail);
    }

    let evidence: (
        i64,
        Option<String>,
        bool,
        i64,
        Option<String>,
        bool,
        i64,
        bool,
    ) = sqlx::query_as(
        "SELECT          (SELECT count(*) FROM audit_records a WHERE a.tenant_id=$1           AND a.action='run.cancel_requested' AND a.resource_type='run'           AND a.resource_id=$2::text AND a.reason='parent flag race proof'),         (SELECT max(a.actor) FROM audit_records a WHERE a.tenant_id=$1           AND a.action='run.cancel_requested' AND a.resource_type='run'           AND a.resource_id=$2::text AND a.reason='parent flag race proof'),         COALESCE((SELECT bool_and(a.trace_id ~ '^[0-9a-f]{32}$')           FROM audit_records a WHERE a.tenant_id=$1            AND a.action='run.cancel_requested' AND a.resource_type='run'            AND a.resource_id=$2::text AND a.reason='parent flag race proof'),false),         (SELECT count(*) FROM command_dedupe d WHERE d.tenant_id=$1           AND d.scope=concat('run.operate:',$2::text) AND d.idempotency_key=$3),         (SELECT max(d.resource_id) FROM command_dedupe d WHERE d.tenant_id=$1           AND d.scope=concat('run.operate:',$2::text) AND d.idempotency_key=$3),         COALESCE((SELECT bool_and(octet_length(d.request_hash)=32)           FROM command_dedupe d WHERE d.tenant_id=$1            AND d.scope=concat('run.operate:',$2::text) AND d.idempotency_key=$3),false),         (SELECT count(*) FROM event_outbox o WHERE o.tenant_id=$1           AND o.aggregate_id=$2 AND o.event_type='run.finished'),         COALESCE((SELECT bool_and(o.payload->>'tenantId'=$1::text           AND o.payload->>'runId'=$2::text AND o.payload->>'status'='CANCELLED')           FROM event_outbox o WHERE o.tenant_id=$1 AND o.aggregate_id=$2            AND o.event_type='run.finished'),true)",
    )
    .bind(f.tenant.as_uuid())
    .bind(parent.as_uuid())
    .bind(key)
    .fetch_one(&f.api.db.pool)
    .await
    .expect("atomic parent audit, idempotency and outbox evidence");
    assert_eq!(
        (evidence.0, evidence.1.as_deref(), evidence.2),
        (1, Some("alice"), true)
    );
    assert_eq!(
        (evidence.3, evidence.4.as_deref(), evidence.5),
        (1, Some("cancel"), true)
    );
    assert_eq!(
        evidence.6,
        i64::from(matches!(
            policy,
            CancelPolicy::Propagate | CancelPolicy::Detach
        )),
    );
    assert!(
        evidence.7,
        "every qualified run.finished payload matches the parent"
    );
}

async fn assert_child_preserved_and_certified(
    f: &Flow,
    child: RunId,
    source: &CertifyLeafCollectionSuccess,
    before: &ChildState,
    policy: CancelPolicy,
    created: &sagat_application::leaf_certification::LeafCollectionSuccessResult,
) -> ChildState {
    assert!(!created.replayed);
    assert_eq!(created.record.identity, source.identity);
    assert_eq!(
        created.record.certified_run_revision, source.expected_run_revision,
        "the first certifier used the post-lease CAS captured before parent flags",
    );
    assert_record_matches_actual_source(f, &created.record).await;
    assert_persisted_certificate_and_audit(f, &created.record).await;
    assert_eq!(certification_counts(f).await, (1, 1));

    let after = child_state(f, child).await;
    assert_eq!(after.selected_source, before.selected_source);
    assert_ne!(after.certificate, before.certificate);
    assert_ne!(after.certification_audit, before.certification_audit);
    assert_eq!(after.run_core_and_journal, before.run_core_and_journal);
    assert_eq!(after.steps, before.steps);
    assert_eq!(after.later_step, before.later_step);
    assert_eq!(after.run_revision, before.run_revision);
    assert_eq!(
        (after.parent_cancel, after.parent_settled),
        match policy {
            CancelPolicy::Propagate => (true, true),
            CancelPolicy::Detach => (false, true),
            CancelPolicy::RequestCancel => (true, false),
        },
    );
    after
}

async fn cold_replay(
    f: &Flow,
    source: &CertifyLeafCollectionSuccess,
    created: &sagat_application::leaf_certification::LeafCollectionSuccessResult,
    after: &ChildState,
) {
    let pool = f.api.db.runtime_pool_with(2).await;
    let role: String = sqlx::query_scalar("SELECT current_user")
        .fetch_one(&pool)
        .await
        .expect("cold replay role");
    assert_eq!(role, RUNTIME_ROLE);
    let store = PgLeafCollectionSuccessStore::new(
        pool.clone(),
        DefinitionCache::new(Arc::new(JsonIrCodec), 8),
    );
    let replay = store
        .certify_leaf_collection_success(source)
        .await
        .expect("cold store/cache replays exact child certificate");
    assert!(replay.replayed);
    assert_eq!(replay.record, created.record);
    assert_eq!(certification_counts(f).await, (1, 1));
    assert_eq!(
        child_state(f, source.identity.run_id()).await,
        after.clone(),
        "canonical replay changes no selected source, execution, flags or metadata",
    );
    drop(store);
    pool.close().await;
}

#[tokio::test]
async fn public_parent_flags_and_first_child_certification_serialize_in_both_orders() {
    let scenarios = [
        Scenario {
            label: "pf-p-cf",
            policy: CancelPolicy::Propagate,
            certifier_first: true,
        },
        Scenario {
            label: "pf-p-pf",
            policy: CancelPolicy::Propagate,
            certifier_first: false,
        },
        Scenario {
            label: "pf-d-cf",
            policy: CancelPolicy::Detach,
            certifier_first: true,
        },
        Scenario {
            label: "pf-d-pf",
            policy: CancelPolicy::Detach,
            certifier_first: false,
        },
        Scenario {
            label: "pf-r-cf",
            policy: CancelPolicy::RequestCancel,
            certifier_first: true,
        },
        Scenario {
            label: "pf-r-pf",
            policy: CancelPolicy::RequestCancel,
            certifier_first: false,
        },
    ];

    for scenario in scenarios {
        let fixture =
            ordinary_child_source_boundary_with_policy(scenario.label, scenario.policy).await;
        assert_eq!(fixture.flow.run, fixture.child);
        assert_eq!(fixture.source.identity.run_id(), fixture.child);
        assert_eq!(certification_counts(&fixture.flow).await, (0, 0));
        let before = child_state(&fixture.flow, fixture.child).await;
        assert_eq!(
            (before.parent_cancel, before.parent_settled),
            (false, false)
        );
        assert_eq!(
            before.run_revision,
            i64::try_from(fixture.source.expected_run_revision)
                .expect("captured child revision is signed"),
        );
        let later_step = Uuid::parse_str(&fixture.later.step_id).expect("later lease step id");
        let selected_later_step: Uuid = sqlx::query_scalar(
            "SELECT step_id FROM steps WHERE tenant_id=$1 AND run_id=$2              AND node='work' AND instance_path_nodes=ARRAY['later']::text[]              AND instance_path_indices=ARRAY[0]::integer[] AND status='LEASED'              AND lease_token_hash IS NOT NULL",
        )
        .bind(fixture.flow.tenant.as_uuid())
        .bind(fixture.child.as_uuid())
        .fetch_one(&fixture.flow.api.db.pool)
        .await
        .expect("the snapshot is the exact public worker lease");
        assert_eq!(selected_later_step, later_step);

        let key = format!("parent-flags-{}", Uuid::now_v7());
        let parent_before = parent_snapshot(&fixture.flow, fixture.parent, &key).await;
        let parent_before_revision = parent_revision(&fixture.flow, fixture.parent).await;
        grant_source_writer(&fixture.flow).await;
        let store = PgLeafCollectionSuccessStore::new(
            fixture.flow.api.api_pool.clone(),
            DefinitionCache::new(Arc::new(JsonIrCodec), 8),
        );
        let hold = ChildRunHold::take(&fixture.flow, fixture.child).await;
        let holder = hold.pid;
        let writer = writer_fragment(scenario.policy);

        let certification = store.certify_leaf_collection_success(&fixture.source);
        let cancellation = cancel_parent(&fixture.flow, fixture.parent, &key);
        tokio::pin!(certification, cancellation);

        let first_witness = if scenario.certifier_first {
            observe_first(
                certification.as_mut(),
                &fixture.flow.api.db.pool,
                holder,
                CERTIFIER_SQL,
                CERTIFIER_PREDICATE,
            )
            .await
        } else {
            observe_first(
                cancellation.as_mut(),
                &fixture.flow.api.db.pool,
                holder,
                writer,
                CHILD_PREDICATE,
            )
            .await
        };
        assert_ne!(first_witness.0, holder);
        assert!(first_witness.2.contains(&holder));
        assert!(first_witness.1.contains(if scenario.certifier_first {
            CERTIFIER_SQL
        } else {
            writer
        }),);

        let queue = if scenario.certifier_first {
            observe_order(
                certification.as_mut(),
                cancellation.as_mut(),
                &fixture.flow.api.db.pool,
                holder,
                (CERTIFIER_SQL, CERTIFIER_PREDICATE),
                (writer, CHILD_PREDICATE),
            )
            .await
        } else {
            observe_order(
                cancellation.as_mut(),
                certification.as_mut(),
                &fixture.flow.api.db.pool,
                holder,
                (writer, CHILD_PREDICATE),
                (CERTIFIER_SQL, CERTIFIER_PREDICATE),
            )
            .await
        };
        assert_ne!(queue.0, queue.1);
        assert_ne!(queue.0, holder);
        assert_ne!(queue.1, holder);
        assert!(queue.4.contains(&holder));
        assert!(queue.5.contains(&queue.0));
        assert!(queue.2.contains(if scenario.certifier_first {
            CERTIFIER_SQL
        } else {
            writer
        }));
        assert!(queue.3.contains(if scenario.certifier_first {
            writer
        } else {
            CERTIFIER_SQL
        }));
        let (first_operation, second_operation) = if scenario.certifier_first {
            (
                (CERTIFIER_SQL, CERTIFIER_PREDICATE),
                (writer, CHILD_PREDICATE),
            )
        } else {
            (
                (writer, CHILD_PREDICATE),
                (CERTIFIER_SQL, CERTIFIER_PREDICATE),
            )
        };
        record_queue_activity(
            &fixture.flow.api.db.pool,
            scenario.label,
            holder,
            queue.0,
            queue.1,
            first_operation,
            second_operation,
        )
        .await;

        assert_eq!(
            child_state(&fixture.flow, fixture.child).await,
            before,
            "blocked transactions expose neither flags nor source metadata",
        );
        assert_eq!(certification_counts(&fixture.flow).await, (0, 0));
        assert_eq!(
            parent_snapshot(&fixture.flow, fixture.parent, &key).await,
            parent_before,
            "blocked public CANCEL exposes no parent command delta",
        );
        assert_parent_locked(&fixture.flow, fixture.parent).await;

        hold.release().await;
        let (created, response) = timeout(WAIT, async {
            tokio::join!(certification.as_mut(), cancellation.as_mut())
        })
        .await
        .expect("both child writers finish after exact hold release");
        let created = created.expect("same captured CAS first-certifies child source");
        assert_parent_outcome(
            &fixture.flow,
            fixture.parent,
            fixture.child,
            scenario.policy,
            &key,
            parent_before_revision,
            response,
        )
        .await;
        let after = assert_child_preserved_and_certified(
            &fixture.flow,
            fixture.child,
            &fixture.source,
            &before,
            scenario.policy,
            &created,
        )
        .await;

        revoke_source_writer(&fixture.flow).await;
        cold_replay(&fixture.flow, &fixture.source, &created, &after).await;
        fixture.flow.verify().await;
        fixture
            .flow
            .api
            .executions
            .verify_run(fixture.flow.tenant, fixture.parent)
            .await
            .expect("public parent journal rebuilds after exact CANCEL suffix");
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

async fn install_commit_gate(f: &Flow, key: i64) {
    let ddl = format!(
        "CREATE FUNCTION leaf_child_parent_commit_probe() RETURNS trigger          LANGUAGE plpgsql AS $$ BEGIN PERFORM pg_advisory_xact_lock({key});          RETURN NEW; END $$;          CREATE CONSTRAINT TRIGGER leaf_child_parent_commit_gate          AFTER INSERT ON leaf_collection_success_certifications          DEFERRABLE INITIALLY DEFERRED FOR EACH ROW          EXECUTE FUNCTION leaf_child_parent_commit_probe();",
    );
    sqlx::raw_sql(sqlx::AssertSqlSafe(ddl))
        .execute(&f.api.db.pool)
        .await
        .expect("test-only deferred child certificate COMMIT gate");
}

async fn remove_commit_gate(f: &Flow) {
    sqlx::raw_sql(
        "DROP TRIGGER leaf_child_parent_commit_gate          ON leaf_collection_success_certifications;          DROP FUNCTION leaf_child_parent_commit_probe();",
    )
    .execute(&f.api.db.pool)
    .await
    .expect("remove test-only child certificate COMMIT gate");
}

async fn commit_waiter(pool: &PgPool, app: &str, gate: i32) -> (i32, String, Vec<i32>) {
    loop {
        let observed = sqlx::query_as(
            "SELECT committing.pid,committing.query,pg_blocking_pids(committing.pid)              FROM pg_stat_activity committing JOIN pg_stat_activity held ON held.pid=$2              WHERE committing.datname=current_database() AND held.datname=committing.datname               AND committing.application_name=$1 AND committing.usename='sagat_app'               AND committing.query='COMMIT' AND committing.wait_event_type='Lock'               AND $2=ANY(pg_blocking_pids(committing.pid))               AND position('SELECT pg_advisory_xact_lock' in held.query)>0",
        )
        .bind(app)
        .bind(gate)
        .fetch_optional(pool)
        .await
        .expect("actual child certifier COMMIT witness");
        if let Some(observed) = observed {
            return observed;
        }
        tokio::task::yield_now().await;
    }
}

async fn observe_commit<F: Future>(
    certifier: Pin<&mut F>,
    pool: &PgPool,
    app: &str,
    gate: i32,
) -> (i32, String, Vec<i32>) {
    tokio::select! {
        _ = certifier => panic!("certifier completed before its deferred COMMIT witness"),
        observed = timeout(WAIT, commit_waiter(pool, app, gate)) =>
            observed.expect("actual child certifier COMMIT deadline"),
    }
}

async fn parent_behind_commit(
    pool: &PgPool,
    app: &str,
    gate: i32,
    committing: i32,
) -> (i32, String, Vec<i32>) {
    loop {
        let observed = sqlx::query_as(
            "SELECT parent.pid,parent.query,pg_blocking_pids(parent.pid)              FROM pg_stat_activity parent JOIN pg_stat_activity committing                ON committing.pid=$4              JOIN pg_stat_activity held ON held.pid=$3              WHERE parent.datname=current_database()               AND committing.datname=parent.datname AND held.datname=parent.datname               AND parent.usename='sagat_app' AND parent.wait_event_type='Lock'               AND position($1 in parent.query)>0 AND position($2 in parent.query)>0               AND $4=ANY(pg_blocking_pids(parent.pid))               AND committing.application_name=$5 AND committing.usename='sagat_app'               AND committing.query='COMMIT' AND committing.wait_event_type='Lock'               AND $3=ANY(pg_blocking_pids(committing.pid))",
        )
        .bind(SETTLE_SQL)
        .bind(CHILD_PREDICATE)
        .bind(gate)
        .bind(committing)
        .bind(app)
        .fetch_optional(pool)
        .await
        .expect("parent child-flag UPDATE behind actual certifier COMMIT");
        if let Some(observed) = observed {
            return observed;
        }
        tokio::task::yield_now().await;
    }
}

async fn observe_parent_behind_commit<A: Future, B: Future>(
    certifier: Pin<&mut A>,
    cancellation: Pin<&mut B>,
    pool: &PgPool,
    app: &str,
    gate: i32,
    committing: i32,
) -> (i32, String, Vec<i32>) {
    tokio::select! {
        _ = certifier => panic!("certifier COMMIT escaped advisory gate"),
        _ = cancellation => panic!("parent CANCEL escaped certifier COMMIT"),
        observed = timeout(
            WAIT,
            parent_behind_commit(pool, app, gate, committing),
        ) => observed.expect("actual COMMIT to parent UPDATE chain deadline"),
    }
}

async fn assert_commit_guards(f: &Flow, parent: RunId, child: RunId) {
    let mut child_probe = f
        .api
        .db
        .pool
        .begin()
        .await
        .expect("fresh child guard probe");
    let child_error =
        sqlx::query("SELECT id FROM runs WHERE tenant_id=$1 AND id=$2 FOR KEY SHARE NOWAIT")
            .bind(f.tenant.as_uuid())
            .bind(child.as_uuid())
            .execute(&mut *child_probe)
            .await
            .expect_err("child FOR UPDATE survives through actual COMMIT");
    assert_eq!(db_error(child_error).0, "55P03");
    drop(child_probe);

    let mut exclusive_shard = f
        .api
        .db
        .pool
        .begin()
        .await
        .expect("fresh shard conflict probe");
    let shard_error = sqlx::query(
        "SELECT shard FROM scheduler_shards          WHERE shard=run_shard($1) FOR NO KEY UPDATE NOWAIT",
    )
    .bind(child.as_uuid())
    .execute(&mut *exclusive_shard)
    .await
    .expect_err("child scheduler SHARE survives through actual COMMIT");
    assert_eq!(db_error(shard_error).0, "55P03");
    drop(exclusive_shard);

    let mut shared_shard = f
        .api
        .db
        .pool
        .begin()
        .await
        .expect("fresh shard share probe");
    sqlx::query(
        "SELECT shard FROM scheduler_shards          WHERE shard=run_shard($1) FOR SHARE NOWAIT",
    )
    .bind(child.as_uuid())
    .execute(&mut *shared_shard)
    .await
    .expect("scheduler guard remains shareable");
    shared_shard
        .rollback()
        .await
        .expect("finish scheduler share probe");

    assert_parent_locked(f, parent).await;
}

#[tokio::test]
async fn incoming_parent_flag_waits_for_the_child_certifiers_actual_commit() {
    let fixture =
        ordinary_child_source_boundary_with_policy("pf-commit", CancelPolicy::Propagate).await;
    let before = child_state(&fixture.flow, fixture.child).await;
    assert_eq!(
        (before.parent_cancel, before.parent_settled),
        (false, false)
    );
    assert_eq!(certification_counts(&fixture.flow).await, (0, 0));
    let key = format!("parent-flags-commit-{}", Uuid::now_v7());
    let parent_before = parent_snapshot(&fixture.flow, fixture.parent, &key).await;
    let parent_before_revision = parent_revision(&fixture.flow, fixture.parent).await;

    grant_source_writer(&fixture.flow).await;
    let advisory = advisory_key();
    install_commit_gate(&fixture.flow, advisory).await;
    let mut gate = fixture
        .flow
        .api
        .db
        .pool
        .begin()
        .await
        .expect("deferred COMMIT owner transaction");
    let gate_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *gate)
        .await
        .expect("deferred COMMIT gate pid");
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(advisory)
        .execute(&mut *gate)
        .await
        .expect("hold generated deferred COMMIT gate");

    let options: PgConnectOptions = fixture
        .flow
        .api
        .db
        .url_for(RUNTIME_ROLE, RUNTIME_ROLE_PASSWORD)
        .parse()
        .expect("runtime database URL");
    let app = format!("sagat-child-pf-{}", Uuid::now_v7());
    let commit_pool = PgPoolOptions::new()
        .max_connections(1)
        .connect_with(options.application_name(&app))
        .await
        .expect("isolated one-connection certifier pool");
    let role: String = sqlx::query_scalar("SELECT current_user")
        .fetch_one(&commit_pool)
        .await
        .expect("isolated certifier role");
    assert_eq!(role, RUNTIME_ROLE);
    let store = PgLeafCollectionSuccessStore::new(
        commit_pool.clone(),
        DefinitionCache::new(Arc::new(JsonIrCodec), 8),
    );
    let certification = store.certify_leaf_collection_success(&fixture.source);
    tokio::pin!(certification);
    let commit = observe_commit(
        certification.as_mut(),
        &fixture.flow.api.db.pool,
        &app,
        gate_pid,
    )
    .await;
    let committing_pid = commit.0;
    assert_ne!(committing_pid, gate_pid);
    assert_eq!(commit.1, "COMMIT");
    assert!(commit.2.contains(&gate_pid));
    assert_eq!(certification_counts(&fixture.flow).await, (0, 0));
    assert_eq!(
        child_state(&fixture.flow, fixture.child).await,
        before,
        "uncommitted source fact and audit remain invisible",
    );

    let cancellation = cancel_parent(&fixture.flow, fixture.parent, &key);
    tokio::pin!(cancellation);
    let parent_waiter = observe_parent_behind_commit(
        certification.as_mut(),
        cancellation.as_mut(),
        &fixture.flow.api.db.pool,
        &app,
        gate_pid,
        committing_pid,
    )
    .await;
    assert_ne!(parent_waiter.0, gate_pid);
    assert_ne!(parent_waiter.0, committing_pid);
    assert!(parent_waiter.1.contains(SETTLE_SQL));
    assert!(parent_waiter.1.contains(CHILD_PREDICATE));
    assert!(parent_waiter.2.contains(&committing_pid));
    record_commit_activity(
        &fixture.flow.api.db.pool,
        "pf-commit",
        gate_pid,
        committing_pid,
        parent_waiter.0,
    )
    .await;

    assert_commit_guards(&fixture.flow, fixture.parent, fixture.child).await;
    assert_eq!(certification_counts(&fixture.flow).await, (0, 0));
    assert_eq!(
        child_state(&fixture.flow, fixture.child).await,
        before,
        "the COMMIT/parent chain exposes no child metadata or flags",
    );
    assert_eq!(
        parent_snapshot(&fixture.flow, fixture.parent, &key).await,
        parent_before,
        "the blocked public CANCEL exposes no parent command delta",
    );

    gate.rollback()
        .await
        .expect("release only the deferred advisory gate");
    let (created, response) = timeout(WAIT, async {
        tokio::join!(certification.as_mut(), cancellation.as_mut())
    })
    .await
    .expect("certifier COMMIT and parent CANCEL finish after advisory release");
    let created = created.expect("first child certificate commits");
    assert_parent_outcome(
        &fixture.flow,
        fixture.parent,
        fixture.child,
        CancelPolicy::Propagate,
        &key,
        parent_before_revision,
        response,
    )
    .await;
    let after = assert_child_preserved_and_certified(
        &fixture.flow,
        fixture.child,
        &fixture.source,
        &before,
        CancelPolicy::Propagate,
        &created,
    )
    .await;

    remove_commit_gate(&fixture.flow).await;
    revoke_source_writer(&fixture.flow).await;
    cold_replay(&fixture.flow, &fixture.source, &created, &after).await;
    commit_pool.close().await;
    fixture.flow.verify().await;
    fixture
        .flow
        .api
        .executions
        .verify_run(fixture.flow.tenant, fixture.parent)
        .await
        .expect("public parent journal rebuilds after COMMIT-ordered CANCEL");
}
