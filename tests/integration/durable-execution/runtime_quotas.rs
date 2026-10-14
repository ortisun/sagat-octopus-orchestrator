//! Runtime decisions at the retained-content and script-retention boundaries (ADR-0027).

use std::time::Duration;

use sagat_application::error::AppError;
use sagat_application::execution::MAX_RUN_OUTPUT_BYTES;
use sagat_application::worker::WorkerSettings;
use sagat_domain::execution::{RunStatus, StepStatus};
use sagat_domain::quota::QuotaResource;
use serde_json::json;

use crate::support::engine::{
    activity, child_node, ctx, document, effect_activities, effect_node, flaky, saga_node,
    task_ref, wait_node, with, world, world_with,
};

const SCRIPT_DIGEST: &str =
    "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

async fn ready_script(w: &crate::support::engine::World, retention_days: i16) -> String {
    sqlx::query(
        "INSERT INTO script_artifacts \
         (tenant_id,digest,language,content,signature,key_id,sbom,toolchain) \
         VALUES ($1,$2,'typescript','\\x01',decode(repeat('ab',64),'hex'),'key-1','\\x7b7d','node@sha256:aa')",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(SCRIPT_DIGEST)
    .execute(&w.db.pool)
    .await
    .expect("script artifact");
    let script = uuid::Uuid::now_v7();
    sqlx::query(
        "INSERT INTO scripts \
         (id,tenant_id,namespace_id,name,language,source,policy,retention_days,created_by) \
         VALUES ($1,$2,$3,'quota-script','typescript','export default 1','v1',$4,'test')",
    )
    .bind(script)
    .bind(w.admin.tenant_id().as_uuid())
    .bind(w.namespace.id().as_uuid())
    .bind(retention_days)
    .execute(&w.db.pool)
    .await
    .expect("script");
    sqlx::query(
        "INSERT INTO script_versions \
         (id,tenant_id,namespace_id,script_id,number,language,status,source,policy,retention_days, \
          content_digest,artifact_digest,created_by,created_at) \
         VALUES ($1,$2,$3,$4,1,'typescript','READY','export default 1','v1',$5,$6,$6,'test',now())",
    )
    .bind(uuid::Uuid::now_v7())
    .bind(w.admin.tenant_id().as_uuid())
    .bind(w.namespace.id().as_uuid())
    .bind(script)
    .bind(retention_days)
    .bind(SCRIPT_DIGEST)
    .execute(&w.db.pool)
    .await
    .expect("READY script version");
    format!("script.sha256-{}", &SCRIPT_DIGEST["sha256:".len()..])
}

async fn set_script_retention(w: &crate::support::engine::World, days: u64) {
    w.quotas
        .set(
            &w.admin,
            Some(w.namespace.id()),
            QuotaResource::ScriptRetentionDays,
            days,
            &ctx(),
        )
        .await
        .expect("set script retention policy");
}

async fn close_content_quota(w: &crate::support::engine::World) {
    let tenant = w.admin.tenant_id().as_uuid();
    let used: i64 = sqlx::query_scalar(
        "SELECT content_bytes + content_reserved_bytes FROM tenant_retained_usage \
         WHERE tenant_id = $1",
    )
    .bind(tenant)
    .fetch_one(&w.db.pool)
    .await
    .expect("retained usage");
    let mut tx = w.db.pool.begin().await.expect("quota policy transaction");
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(format!("quota-policy:tenant_content_bytes:{tenant}"))
        .execute(&mut *tx)
        .await
        .expect("quota policy fence");
    sqlx::query(
        "DELETE FROM quota_limits WHERE tenant_id=$1 AND namespace_id IS NULL \
         AND resource='tenant_content_bytes'",
    )
    .bind(tenant)
    .execute(&mut *tx)
    .await
    .expect("remove prior content quota");
    sqlx::query(
        "INSERT INTO quota_limits (tenant_id, namespace_id, resource, max_value, updated_by) \
         VALUES ($1, NULL, 'tenant_content_bytes', $2, 'runtime-quota-test')",
    )
    .bind(tenant)
    .bind(used)
    .execute(&mut *tx)
    .await
    .expect("close content quota");
    tx.commit().await.expect("quota policy commit");
    sqlx::query("UPDATE quota_accounting_control SET content_enforcement_enabled = true")
        .execute(&w.db.pool)
        .await
        .expect("enable content enforcement");
}

#[tokio::test]
async fn successful_report_at_full_storage_stops_run_without_on_error_and_replays_original() {
    let settings = WorkerSettings {
        lease_ttl: Duration::from_millis(5),
        ..WorkerSettings::default()
    };
    let w = world_with(settings).await;
    let (workflow, _) = w
        .publish(document(
            "storage-terminal-fallback",
            "write",
            vec![
                with(activity("write"), "onError", json!("would-grow")),
                activity("would-grow"),
            ],
        ))
        .await;
    let run = w.start(workflow, &json!({"input": true})).await;
    let session = w.session(&["act.write", "act.would-grow"]).await;
    let task = w.one_task(&session).await;
    close_content_quota(&w).await;
    tokio::time::sleep(Duration::from_millis(10)).await;

    let output = json!({"new-retained-value": "cannot-fit"});
    let ack = w
        .complete(&session, &task, &output)
        .await
        .expect("quota fallback is an accepted terminal report");
    assert_eq!(ack.step_status, StepStatus::Failed);
    assert_eq!(ack.run_status, RunStatus::Failed);
    assert!(!ack.already_applied);
    assert_eq!(w.run_status(run).await, RunStatus::Failed);
    assert_eq!(w.step_status(run, "would-grow").await, None);
    let code: Option<String> = sqlx::query_scalar(
        "SELECT failure_code FROM steps WHERE tenant_id=$1 AND run_id=$2 AND node='write'",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(run.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("failed step");
    assert_eq!(code.as_deref(), Some("STORAGE_QUOTA_EXCEEDED"));

    let replay = w
        .complete(&session, &task, &output)
        .await
        .expect("same original completion replays");
    assert!(replay.already_applied);
    let key = format!("done-{}", task.task_id());
    let different = w
        .workers
        .complete(
            &session.principal,
            &task_ref(&task),
            br#"{"new-retained-value":"different"}"#,
            &key,
        )
        .await;
    assert_eq!(different.map(|_| ()), Err(AppError::IdempotencyKeyReused));
    assert!(w.poll(&session, 4).await.is_empty());
    w.assert_consistent(run).await;
}

#[tokio::test]
async fn completion_reusing_an_existing_payload_digest_succeeds_at_full_storage() {
    let w = world().await;
    let (workflow, _) = w
        .publish(document(
            "existing-payload-at-limit",
            "write",
            vec![activity("write")],
        ))
        .await;
    // This exact document is already the run input payload, so completing with it inserts no new
    // retained payload bytes even though the quota has no remaining byte.
    let shared = json!({"same": "digest"});
    let run = w.start(workflow, &shared).await;
    let session = w.session(&["act.write"]).await;
    let task = w.one_task(&session).await;
    close_content_quota(&w).await;

    let ack = w
        .complete(&session, &task, &shared)
        .await
        .expect("content-addressed reuse needs no new retained bytes");
    assert_eq!(ack.step_status, StepStatus::Succeeded);
    assert_eq!(ack.run_status, RunStatus::Succeeded);
    w.assert_consistent(run).await;
}

#[tokio::test]
async fn failed_report_at_full_storage_stops_before_child_error_route_and_replays_original() {
    let w = world().await;
    w.publish(document("quota-child", "child", vec![activity("child")]))
        .await;
    let mut child = child_node("would-grow", "quota-child", None);
    child["input"] = json!("input.child");
    let (workflow, _) = w
        .publish(document(
            "failed-storage-terminal-fallback",
            "write",
            vec![
                with(flaky("write", 1), "onError", json!("would-grow")),
                child,
            ],
        ))
        .await;
    let run = w
        .start(workflow, &json!({"child": {"new": "retained-value"}}))
        .await;
    let session = w.session(&["act.write"]).await;
    let task = w.one_task(&session).await;
    close_content_quota(&w).await;

    let ack = w
        .fail(&session, &task, false)
        .await
        .expect("known failure has a quota-safe terminal fallback");
    assert_eq!(ack.step_status, StepStatus::Failed);
    assert_eq!(ack.run_status, RunStatus::Failed);
    assert_eq!(w.step_status(run, "would-grow").await, None);
    let (step_code, stop_code): (Option<String>, Option<String>) = sqlx::query_as(
        "SELECT s.failure_code, r.stop_code FROM steps s JOIN runs r \
           ON r.tenant_id=s.tenant_id AND r.id=s.run_id \
         WHERE s.tenant_id=$1 AND s.run_id=$2 AND s.node='write'",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(run.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("terminal codes");
    assert_eq!(step_code.as_deref(), Some("PAYMENT_DECLINED"));
    assert_eq!(stop_code.as_deref(), Some("STORAGE_QUOTA_EXCEEDED"));

    let replay = w
        .fail(&session, &task, false)
        .await
        .expect("same original failure replays");
    assert!(replay.already_applied);
    w.assert_consistent(run).await;
}

#[tokio::test]
async fn secret_output_quota_fallback_preserves_redacted_audit_and_success_effect_evidence() {
    const SECRET: &str = "quota-secret-value-4eC39HqLyjWDarjt";
    let w = world().await;
    w.put_secret("quota-key", SECRET).await;
    w.publish(document(
        "secret-quota-child",
        "child",
        vec![activity("child")],
    ))
    .await;
    let mut write = with(activity("write"), "secrets", json!(["quota-key"]));
    write["onError"] = json!("would-grow");
    let mut child = child_node("would-grow", "secret-quota-child", None);
    child["input"] = json!("input.child");
    let (workflow, _) = w
        .publish(document(
            "secret-storage-terminal-fallback",
            "write",
            vec![write, child],
        ))
        .await;
    let run = w
        .start(workflow, &json!({"child": {"new": "retained-value"}}))
        .await;
    let session = w.session(&["act.write"]).await;
    let task = w.one_task(&session).await;
    close_content_quota(&w).await;

    let ack = w
        .complete(&session, &task, &json!({"echo": SECRET}))
        .await
        .expect("secret rejection has a quota-safe terminal fallback");
    assert_eq!(ack.step_status, StepStatus::Failed);
    assert_eq!(w.step_status(run, "would-grow").await, None);
    let code: Option<String> = sqlx::query_scalar(
        "SELECT failure_code FROM steps WHERE tenant_id=$1 AND run_id=$2 AND node='write'",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(run.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("storage failure code");
    assert_eq!(code.as_deref(), Some("STORAGE_QUOTA_EXCEEDED"));
    let audit: (i64, bool) = sqlx::query_as(
        "SELECT count(*), COALESCE(bool_and(strpos(reason, $2) = 0), false) \
         FROM audit_records WHERE tenant_id=$1 AND action='secret.denied'",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(SECRET)
    .fetch_one(&w.db.pool)
    .await
    .expect("redacted denial audit");
    assert_eq!(audit, (1, true));
    w.assert_consistent(run).await;
}

#[tokio::test]
async fn run_output_limit_rejection_uses_the_success_quota_fallback_when_routing_cannot_fit() {
    let w = world().await;
    w.publish(document(
        "limit-quota-child",
        "child",
        vec![activity("child")],
    ))
    .await;
    let output = json!({"blob": "x".repeat(240_000)});
    let output_len = serde_json::to_vec(&output).expect("output JSON").len();
    let fits = MAX_RUN_OUTPUT_BYTES / output_len;
    let ids: Vec<String> = (0..=fits).map(|i| format!("s{i}")).collect();
    let mut nodes: Vec<serde_json::Value> = ids
        .iter()
        .enumerate()
        .map(|(index, id)| match ids.get(index + 1) {
            Some(next) => with(activity(id), "next", json!(next)),
            None => with(activity(id), "onError", json!("would-grow")),
        })
        .collect();
    let mut child = child_node("would-grow", "limit-quota-child", None);
    child["input"] = json!("input.child");
    nodes.push(child);
    let (workflow, _) = w
        .publish(document("run-limit-storage-fallback", "s0", nodes))
        .await;
    let activities: Vec<String> = ids.iter().map(|id| format!("act.{id}")).collect();
    let session = w
        .session(&activities.iter().map(String::as_str).collect::<Vec<_>>())
        .await;
    let run = w
        .start(workflow, &json!({"child": {"new": "retained-value"}}))
        .await;
    for _ in 0..fits {
        let task = w.one_task(&session).await;
        w.complete(&session, &task, &output)
            .await
            .expect("output fits run budget");
    }
    let crossing = w.one_task(&session).await;
    close_content_quota(&w).await;
    let ack = w
        .complete(&session, &crossing, &output)
        .await
        .expect("run-limit rejection has a quota-safe fallback");
    assert_eq!(ack.step_status, StepStatus::Failed);
    assert_eq!(w.step_status(run, "would-grow").await, None);
    let code: Option<String> = sqlx::query_scalar(
        "SELECT failure_code FROM steps WHERE tenant_id=$1 AND run_id=$2 AND node=$3",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(run.as_uuid())
    .bind(crossing.node.as_str())
    .fetch_one(&w.db.pool)
    .await
    .expect("storage failure code");
    assert_eq!(code.as_deref(), Some("STORAGE_QUOTA_EXCEEDED"));
    let replay = w
        .complete(&session, &crossing, &output)
        .await
        .expect("same crossing completion replays");
    assert!(replay.already_applied);
    w.assert_consistent(run).await;
}

#[tokio::test]
async fn expired_lease_at_full_storage_stops_then_drains_without_child_error_route() {
    let w = world().await;
    w.publish(document(
        "expiry-quota-child",
        "child",
        vec![activity("child")],
    ))
    .await;
    let mut child = child_node("would-grow", "expiry-quota-child", None);
    child["input"] = json!("input.child");
    let (workflow, _) = w
        .publish(document(
            "expiry-storage-terminal-fallback",
            "write",
            vec![
                with(flaky("write", 1), "onError", json!("would-grow")),
                child,
            ],
        ))
        .await;
    let run = w
        .start(workflow, &json!({"child": {"new": "retained-value"}}))
        .await;
    let session = w.session(&["act.write"]).await;
    let _task = w.one_task(&session).await;
    close_content_quota(&w).await;
    w.age_lease(run, "write").await;

    let stopped = w.scheduler.tick().await;
    assert_eq!(stopped.quota_stops, 1, "{stopped:?}");
    assert_eq!(stopped.leases_expired, 0, "the expiry rolled back");
    assert_eq!(w.step_status(run, "write").await, Some(StepStatus::Leased));
    assert_eq!(w.step_status(run, "would-grow").await, None);

    let drained = w.scheduler.tick().await;
    assert_eq!(drained.leases_expired, 1, "{drained:?}");
    assert_eq!(drained.quota_stops, 0, "{drained:?}");
    assert_eq!(
        w.step_status(run, "write").await,
        Some(StepStatus::Cancelled)
    );
    assert_eq!(w.step_status(run, "would-grow").await, None);
    assert_eq!(w.run_status(run).await, RunStatus::Failed);
    let stop_code: Option<String> =
        sqlx::query_scalar("SELECT stop_code FROM runs WHERE tenant_id=$1 AND id=$2")
            .bind(w.admin.tenant_id().as_uuid())
            .bind(run.as_uuid())
            .fetch_one(&w.db.pool)
            .await
            .expect("storage stop code");
    assert_eq!(stop_code.as_deref(), Some("STORAGE_QUOTA_EXCEEDED"));
    w.assert_consistent(run).await;
}

#[tokio::test]
async fn child_creation_content_refusal_is_reported_to_parent_without_an_orphan_run() {
    let w = world().await;
    w.publish(document(
        "content-refused-child",
        "wait",
        vec![wait_node("wait", "child.event", "input.key", 60_000, None)],
    ))
    .await;
    let mut child = child_node("child", "content-refused-child", None);
    child["input"] = json!("input.child");
    let (workflow, _) = w
        .publish(document("content-refused-parent", "child", vec![child]))
        .await;
    let run = w
        .start(workflow, &json!({"child": {"key": "quota-key"}}))
        .await;
    close_content_quota(&w).await;

    let report = w.scheduler.tick().await;
    assert_eq!(report.children_started, 0, "{report:?}");
    assert_eq!(report.children_finished, 1, "{report:?}");
    assert_eq!(w.step_status(run, "child").await, Some(StepStatus::Failed));
    let code: Option<String> = sqlx::query_scalar(
        "SELECT failure_code FROM steps WHERE tenant_id=$1 AND run_id=$2 AND node='child'",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(run.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("child refusal code");
    assert_eq!(code.as_deref(), Some("CHILD_QUOTA_EXCEEDED"));
    let children: i64 =
        sqlx::query_scalar("SELECT count(*) FROM runs WHERE tenant_id=$1 AND parent_run_id=$2")
            .bind(w.admin.tenant_id().as_uuid())
            .bind(run.as_uuid())
            .fetch_one(&w.db.pool)
            .await
            .expect("orphan child count");
    assert_eq!(children, 0);
    w.assert_consistent(run).await;
}

#[tokio::test]
async fn applied_reconciliation_at_full_storage_fails_durably_under_original_report_identity() {
    let w = world().await;
    let (workflow, _) = w
        .publish(document(
            "reconciliation-storage-fallback",
            "effect",
            vec![effect_node("effect", None, true)],
        ))
        .await;
    let activities = effect_activities(&["effect"]);
    let session = w
        .session(&activities.iter().map(String::as_str).collect::<Vec<_>>())
        .await;
    let run = w.start(workflow, &json!({})).await;
    let _lost = w.one_task_of(&session, "act.effect").await;
    w.age_lease(run, "effect").await;
    w.scheduler.tick().await;
    let lookup = w.one_task_of(&session, "lookup.effect").await;
    close_content_quota(&w).await;

    let answer = json!({"effect": "applied", "output": {"provider": "accepted"}});
    let ack = w
        .complete(&session, &lookup, &answer)
        .await
        .expect("quota fallback accepts the reconciliation report");
    assert_eq!(ack.step_status, StepStatus::Failed);
    assert_eq!(
        w.step_status(run, "effect~r").await,
        Some(StepStatus::Failed)
    );
    assert_eq!(
        w.step_status(run, "effect").await,
        Some(StepStatus::Waiting)
    );
    let code: Option<String> = sqlx::query_scalar(
        "SELECT failure_code FROM steps WHERE tenant_id=$1 AND run_id=$2 AND node='effect~r'",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(run.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("reconciliation failure code");
    assert_eq!(code.as_deref(), Some("STORAGE_QUOTA_EXCEEDED"));

    let replay = w
        .complete(&session, &lookup, &answer)
        .await
        .expect("same reconciliation report replays");
    assert!(replay.already_applied);
    w.assert_consistent(run).await;
}

#[tokio::test]
async fn lease_pins_script_obligations_and_zero_withholds_only_new_forward_work() {
    let w = world().await;
    let script_activity = ready_script(&w, 30).await;
    set_script_retention(&w, 5).await;
    let mut charged = activity("charge");
    charged["compensateWith"] = json!(script_activity);
    charged["next"] = json!("fail");
    let (workflow, _) = w
        .publish(document(
            "pinned-script-obligation",
            "saga",
            vec![saga_node("saga", "charge", None), charged, activity("fail")],
        ))
        .await;
    let run = w.start(workflow, &json!({})).await;
    let session = w
        .session(&["act.charge", "act.fail", script_activity.as_str()])
        .await;

    let charge = w.one_task_of(&session, "act.charge").await;
    let forward_detail: serde_json::Value = sqlx::query_scalar(
        "SELECT detail FROM run_events WHERE tenant_id=$1 AND run_id=$2 AND node='charge' \
         AND event_type='step.leased' AND attempt=1",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(run.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("forward lease detail");
    assert_eq!(forward_detail["scriptRetentionPolicyVersion"], 1);
    assert_eq!(forward_detail["compensationScriptRetentionDays"], 5);
    w.complete(&session, &charge, &json!({"charged": true}))
        .await
        .expect("forward complete");

    set_script_retention(&w, 0).await;
    let failing = w.one_task_of(&session, "act.fail").await;
    w.fail(&session, &failing, false)
        .await
        .expect("start compensation");
    let compensation = w.one_task_of(&session, &script_activity).await;
    let inherited: serde_json::Value = sqlx::query_scalar(
        "SELECT detail FROM run_events WHERE tenant_id=$1 AND run_id=$2 AND node=$3 \
         AND event_type='step.leased' AND attempt=1",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(run.as_uuid())
    .bind(compensation.node.as_str())
    .fetch_one(&w.db.pool)
    .await
    .expect("compensation lease detail");
    assert_eq!(inherited["scriptRetentionPolicyVersion"], 1);
    assert_eq!(inherited["scriptRetentionDays"], 5);
    w.complete(&session, &compensation, &json!({}))
        .await
        .expect("already-obligated compensation completes at zero policy");

    let (blocked_workflow, _) = w
        .publish(document(
            "blocked-new-script",
            "script",
            vec![json!({
                "id": "script", "type": "script", "artifactDigest": SCRIPT_DIGEST,
                "language": "typescript"
            })],
        ))
        .await;
    let blocked_run = w.start(blocked_workflow, &json!({})).await;
    assert!(w.poll(&session, 8).await.is_empty());
    assert_eq!(
        w.step_status(blocked_run, "script").await,
        Some(StepStatus::Ready)
    );
    w.assert_consistent(run).await;
    w.assert_consistent(blocked_run).await;
}

#[tokio::test]
async fn reconciliation_script_retention_inherits_after_the_policy_closes() {
    let w = world().await;
    let script_activity = ready_script(&w, 30).await;
    let mut effect = activity("effect");
    effect["reconcileWith"] = json!(script_activity);
    let (workflow, _) = w
        .publish(document(
            "reconciliation-retention-pin",
            "effect",
            vec![effect],
        ))
        .await;
    let session = w.session(&["act.effect", script_activity.as_str()]).await;

    set_script_retention(&w, 7).await;
    let run = w.start(workflow, &json!({})).await;
    let _forward = w.one_task_of(&session, "act.effect").await;
    let forward_detail: serde_json::Value = sqlx::query_scalar(
        "SELECT detail FROM run_events WHERE tenant_id=$1 AND run_id=$2 AND node='effect' \
         AND event_type='step.leased' AND attempt=1",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(run.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("versioned forward lease");
    assert_eq!(forward_detail["scriptRetentionPolicyVersion"], 1);
    assert_eq!(forward_detail["reconciliationScriptRetentionDays"], 7);
    w.age_lease(run, "effect").await;
    w.scheduler.tick().await;
    set_script_retention(&w, 0).await;
    let lookup = w.one_task_of(&session, &script_activity).await;
    let inherited: serde_json::Value = sqlx::query_scalar(
        "SELECT detail FROM run_events WHERE tenant_id=$1 AND run_id=$2 AND node=$3 \
         AND event_type='step.leased' AND attempt=1",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(run.as_uuid())
    .bind(lookup.node.as_str())
    .fetch_one(&w.db.pool)
    .await
    .expect("reconciliation lease detail");
    assert_eq!(inherited["scriptRetentionPolicyVersion"], 1);
    assert_eq!(inherited["scriptRetentionDays"], 7);
}
