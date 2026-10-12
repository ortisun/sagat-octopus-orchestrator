//! Durable supervisor denials (D51, FR-009-004/005/008): an acknowledged category is in the
//! append-only audit trail, tied to the current fenced attempt and bounded independently of the
//! destination an untrusted script supplied.

use sagat_application::error::AppError;
use sagat_application::scripts::{
    BuildReport, ExecutionStatus, IncidentCategory, IncidentTarget, NewExecution, PolicyRequest,
    ScriptSpec,
};
use sagat_application::worker::ClaimedTask;
use sagat_domain::audit::AuditAction;
use sagat_domain::identity::{Principal, Role, Slug};
use sagat_domain::scripts::{Failure, Language, Source, activity_of, digest_of_activity};
use serde_json::json;

use crate::support::engine::{
    World, ctx, document, end_node, parallel_node, subject_of, with, world,
};

async fn leased_production() -> (World, crate::support::engine::Session, ClaimedTask) {
    let w = world().await;
    let script = w
        .scripts
        .create(
            &w.admin,
            w.namespace.id(),
            ScriptSpec {
                name: Slug::parse("incident-script").expect("slug"),
                language: Language::TypeScript,
                source: Source::parse("export default () => 1").expect("source"),
                lockfile: None,
                input_schema: None,
                output_schema: None,
                policy: PolicyRequest::default(),
                retention: None,
            },
            &ctx(),
        )
        .await
        .expect("create script");
    let published = w
        .scripts
        .publish(&w.admin, w.namespace.id(), script.id, &ctx())
        .await
        .expect("publish script");
    let toolchain = "node@sha256:aa;shim=1".to_owned();
    let build = w
        .runner
        .claim_build(
            &w.runner_identity,
            &[(Language::TypeScript, toolchain.clone())],
            &ctx(),
        )
        .await
        .expect("claim build")
        .expect("build lease");
    let artifact = serde_json::to_vec(&json!({
        "v": 1,
        "language": build.language.as_str(),
        "policy": build.policy.canonical(),
        "inputSchema": null,
        "outputSchema": null,
        "entry": "main.mjs",
        "files": {"main.mjs": "export default () => 1"}
    }))
    .expect("artifact");
    w.runner
        .complete_build(
            &w.runner_identity,
            build.job,
            &build.lease_token,
            BuildReport::Built {
                artifact,
                sbom: br#"{"bomFormat":"CycloneDX"}"#.to_vec(),
                toolchain,
                log: "built".to_owned(),
            },
            &ctx(),
        )
        .await
        .expect("complete build");
    let version = w
        .scripts
        .version(
            &w.admin,
            w.namespace.id(),
            script.id,
            published.version.number,
            &ctx(),
        )
        .await
        .expect("ready version");
    let activity = activity_of(version.artifact.as_ref().expect("artifact digest"));
    let (workflow, _) = w
        .publish(document(
            "incident",
            "script",
            vec![json!({"id": "script", "type": "activity", "activity": activity})],
        ))
        .await;
    w.start(workflow, &json!({})).await;
    let session = w
        .session_as(&w.runner_identity, &[activity.as_str()], 1)
        .await;
    let task = w.one_task(&session).await;
    (w, session, task)
}

fn production<'a>(w: &World, task: &'a ClaimedTask) -> IncidentTarget<'a> {
    production_with(
        w,
        task,
        task.fence.attempt,
        task.fence.generation,
        task.lease_token.expose(),
    )
}

fn production_with<'a>(
    w: &World,
    task: &'a ClaimedTask,
    attempt: u32,
    generation: u64,
    lease_token: &'a str,
) -> IncidentTarget<'a> {
    IncidentTarget::Production {
        namespace_id: w.namespace.id(),
        run_id: task.run_id,
        step_id: task.step_id,
        attempt,
        generation,
        lease_token,
    }
}

async fn incident_rows(w: &World) -> Vec<(String, String, String, String)> {
    sqlx::query_as(
        "SELECT actor, resource_type, resource_id, reason FROM audit_records \
         WHERE tenant_id = $1 AND action = $2 ORDER BY reason",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(AuditAction::ScriptViolation.as_str())
    .fetch_all(&w.db.pool)
    .await
    .expect("incident audit rows")
}

#[tokio::test]
async fn duplicate_concurrent_and_lost_ack_retries_commit_one_category() {
    let (w, _session, task) = leased_production().await;
    let a = production(&w, &task);
    let b = production(&w, &task);
    let first_ctx = ctx();
    let concurrent_ctx = ctx();
    let (first, concurrent) = tokio::join!(
        w.runner.record_incident(
            &w.runner_identity,
            &a,
            IncidentCategory::ForbiddenAddress,
            &first_ctx
        ),
        w.runner.record_incident(
            &w.runner_identity,
            &b,
            IncidentCategory::ForbiddenAddress,
            &concurrent_ctx
        )
    );
    first.expect("first acknowledgement follows commit");
    concurrent.expect("a duplicate is an acknowledged no-op");

    // Model a response lost after commit: the client ignores it and retries the identical report.
    w.runner
        .record_incident(
            &w.runner_identity,
            &production(&w, &task),
            IncidentCategory::ForbiddenAddress,
            &ctx(),
        )
        .await
        .expect("lost acknowledgement retry");

    let rows = incident_rows(&w).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].0, "runner-1", "actor comes from authentication");
    assert_eq!(rows[0].1, "run");
    assert!(rows[0].2.contains(&task.run_id.to_string()));
    assert_eq!(rows[0].3, "FORBIDDEN_ADDRESS");
}

#[tokio::test]
async fn cancelling_before_audit_commit_gives_no_ack_and_retry_commits_once() {
    let (w, _session, task) = leased_production().await;
    let mut blocker = w.db.pool.begin().await.expect("blocking transaction");
    sqlx::query("LOCK TABLE audit_records IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *blocker)
        .await
        .expect("block audit commit");

    let target = production(&w, &task);
    let request_ctx = ctx();
    let unfinished = tokio::time::timeout(
        std::time::Duration::from_millis(100),
        w.runner.record_incident(
            &w.runner_identity,
            &target,
            IncidentCategory::InvalidHost,
            &request_ctx,
        ),
    )
    .await;
    assert!(unfinished.is_err(), "a blocked commit must not acknowledge");
    blocker.rollback().await.expect("release audit table");
    assert!(incident_rows(&w).await.is_empty());

    w.runner
        .record_incident(
            &w.runner_identity,
            &production(&w, &task),
            IncidentCategory::InvalidHost,
            &ctx(),
        )
        .await
        .expect("retry commits");
    assert_eq!(incident_rows(&w).await.len(), 1);
}

#[tokio::test]
async fn categories_are_bounded_per_attempt_and_never_store_destination_canaries() {
    let (w, _session, task) = leased_production().await;
    for category in IncidentCategory::ALL {
        for _ in 0..3 {
            w.runner
                .record_incident(&w.runner_identity, &production(&w, &task), category, &ctx())
                .await
                .expect("category");
        }
    }
    w.runner
        .record_execution(
            &w.runner_identity,
            NewExecution {
                namespace_id: w.namespace.id(),
                digest: digest_of_activity(task.activity.as_str()).expect("script activity"),
                run_id: Some(task.run_id.as_uuid()),
                node: Some(task.node.as_str().to_owned()),
                attempt: Some(task.fence.attempt),
                status: ExecutionStatus::Failed,
                failure: Some(Failure::Denied),
                exit_code: None,
                log: String::new(),
                log_truncated: false,
                denied: vec!["fetch https://secret.example/private/path?query-canary=1".to_owned()],
                wall_ms: 1,
                start_ms: 1,
                memory_peak_bytes: None,
                trace_id: None,
            },
            &ctx(),
        )
        .await
        .expect("terminal diagnostic");
    let rows = incident_rows(&w).await;
    assert_eq!(rows.len(), IncidentCategory::ALL.len());
    let stored = format!("{rows:?}");
    for canary in [
        "secret.example",
        "/private/path",
        "query-canary",
        "authorization-canary",
        "body-canary",
    ] {
        assert!(!stored.contains(canary), "stored attacker data: {canary}");
    }
}

#[tokio::test]
async fn an_incident_on_one_node_does_not_suppress_another_nodes_terminal_audit() {
    let (w, _first_session, first_task) = leased_production().await;
    let activity = first_task.activity.as_str().to_owned();
    let (workflow, _) = w
        .publish(document(
            "incident-two-nodes",
            "fork",
            vec![
                with(
                    parallel_node("fork", &["left", "right"], "all"),
                    "next",
                    json!("done"),
                ),
                json!({"id": "left", "type": "activity", "activity": activity}),
                json!({"id": "right", "type": "activity", "activity": activity}),
                end_node("done"),
            ],
        ))
        .await;
    let run = w.start(workflow, &json!({})).await;
    let session = w
        .session_as(&w.runner_identity, &[activity.as_str()], 2)
        .await;
    let tasks = w.poll(&session, 4).await;
    assert_eq!(tasks.len(), 2);
    let left = tasks
        .iter()
        .find(|task| task.node.as_str() == "left")
        .expect("left");
    let right = tasks
        .iter()
        .find(|task| task.node.as_str() == "right")
        .expect("right");
    assert_eq!((left.run_id, right.run_id), (run, run));
    assert_eq!((left.fence.attempt, right.fence.attempt), (1, 1));

    w.runner
        .record_incident(
            &w.runner_identity,
            &production(&w, left),
            IncidentCategory::NotAllowlisted,
            &ctx(),
        )
        .await
        .expect("left incident");
    let right_destination = "fetch https://right-node.example/canary";
    w.runner
        .record_execution(
            &w.runner_identity,
            NewExecution {
                namespace_id: w.namespace.id(),
                digest: digest_of_activity(right.activity.as_str()).expect("script activity"),
                run_id: Some(right.run_id.as_uuid()),
                node: Some(right.node.as_str().to_owned()),
                attempt: Some(right.fence.attempt),
                status: ExecutionStatus::Failed,
                failure: Some(Failure::Denied),
                exit_code: None,
                log: String::new(),
                log_truncated: false,
                denied: vec![right_destination.to_owned()],
                wall_ms: 1,
                start_ms: 1,
                memory_peak_bytes: None,
                trace_id: None,
            },
            &ctx(),
        )
        .await
        .expect("right terminal report");
    let reasons: Vec<String> = incident_rows(&w)
        .await
        .into_iter()
        .map(|row| row.3)
        .collect();
    assert_eq!(
        reasons,
        vec!["NOT_ALLOWLISTED".to_owned(), right_destination.to_owned()]
    );
}

#[tokio::test]
async fn a_delayed_terminal_report_finds_incident_evidence_after_the_lease_advances() {
    let (w, _old_session, old_task) = leased_production().await;
    w.runner
        .record_incident(
            &w.runner_identity,
            &production(&w, &old_task),
            IncidentCategory::ForbiddenAddress,
            &ctx(),
        )
        .await
        .expect("incident committed before the old lease ended");

    w.age_lease(old_task.run_id, old_task.node.as_str()).await;
    assert_eq!(w.scheduler.tick().await.leases_expired, 1);
    w.age_retry(old_task.run_id, old_task.node.as_str()).await;
    assert_eq!(w.scheduler.tick().await.retries_released, 1);
    let replacement_session = w
        .session_as(&w.runner_identity, &[old_task.activity.as_str()], 1)
        .await;
    let replacement = w.one_task(&replacement_session).await;
    assert_eq!(
        replacement.step_id, old_task.step_id,
        "step identity is stable"
    );
    assert!(replacement.fence.attempt > old_task.fence.attempt);
    assert!(replacement.fence.generation > old_task.fence.generation);

    let destination = "fetch https://late-old-attempt.example/secret-canary";
    w.runner
        .record_execution(
            &w.runner_identity,
            NewExecution {
                namespace_id: w.namespace.id(),
                digest: digest_of_activity(old_task.activity.as_str()).expect("script activity"),
                run_id: Some(old_task.run_id.as_uuid()),
                node: Some(old_task.node.as_str().to_owned()),
                attempt: Some(old_task.fence.attempt),
                status: ExecutionStatus::Failed,
                failure: Some(Failure::Denied),
                exit_code: None,
                log: String::new(),
                log_truncated: false,
                denied: vec![destination.to_owned()],
                wall_ms: 1,
                start_ms: 1,
                memory_peak_bytes: None,
                trace_id: None,
            },
            &ctx(),
        )
        .await
        .expect("delayed terminal diagnostic");

    let reasons: Vec<String> = incident_rows(&w)
        .await
        .into_iter()
        .map(|row| row.3)
        .collect();
    assert_eq!(reasons, vec!["FORBIDDEN_ADDRESS"]);
    assert!(!format!("{reasons:?}").contains("late-old-attempt"));
}

#[tokio::test]
async fn stale_wrong_and_cross_tenant_production_targets_write_nothing() {
    let (w, _session, task) = leased_production().await;
    let other = Principal::new(
        w.other_admin.tenant_id(),
        subject_of("runner-1"),
        Role::Runner,
    );
    let wrong_token = production_with(
        &w,
        &task,
        task.fence.attempt,
        task.fence.generation,
        "wrong-token",
    );
    let wrong_generation = production_with(
        &w,
        &task,
        task.fence.attempt,
        task.fence.generation + 1,
        task.lease_token.expose(),
    );
    let wrong_attempt = production_with(
        &w,
        &task,
        task.fence.attempt + 1,
        task.fence.generation,
        task.lease_token.expose(),
    );
    for (who, target) in [
        (&w.runner_identity, wrong_token),
        (&w.runner_identity, wrong_generation),
        (&w.runner_identity, wrong_attempt),
        (&other, production(&w, &task)),
    ] {
        assert_eq!(
            w.runner
                .record_incident(who, &target, IncidentCategory::NotAllowlisted, &ctx())
                .await,
            Err(AppError::StaleAttempt)
        );
    }
    assert!(incident_rows(&w).await.is_empty());
}

#[tokio::test]
async fn expired_or_revoked_production_leases_cannot_be_attributed() {
    for revoke in [false, true] {
        let (w, _session, task) = leased_production().await;
        if revoke {
            w.bindings
                .unbind(
                    &w.admin,
                    w.namespace.id(),
                    w.runner_identity.subject().as_str(),
                    &ctx(),
                )
                .await
                .expect("revoke binding");
        } else {
            sqlx::query(
                "UPDATE steps SET lease_expires_at = clock_timestamp() - interval '1 second' \
                 WHERE tenant_id = $1 AND run_id = $2 AND step_id = $3",
            )
            .bind(w.admin.tenant_id().as_uuid())
            .bind(task.run_id.as_uuid())
            .bind(task.step_id)
            .execute(&w.db.pool)
            .await
            .expect("expire lease");
        }
        assert_eq!(
            w.runner
                .record_incident(
                    &w.runner_identity,
                    &production(&w, &task),
                    IncidentCategory::EgressDisabled,
                    &ctx(),
                )
                .await,
            Err(AppError::StaleAttempt),
            "revoke={revoke}"
        );
        assert!(incident_rows(&w).await.is_empty());
    }
}

#[tokio::test]
async fn completion_and_incident_serialize_without_stale_attribution() {
    let (w, session, task) = leased_production().await;
    let target = production(&w, &task);
    let incident_ctx = ctx();
    let output = json!({"ok": true});
    let (incident, completion) = tokio::join!(
        w.runner.record_incident(
            &w.runner_identity,
            &target,
            IncidentCategory::NoAddress,
            &incident_ctx
        ),
        w.complete(&session, &task, &output)
    );
    completion.expect("completion wins or follows the incident");
    match incident {
        Ok(()) => assert_eq!(incident_rows(&w).await.len(), 1),
        Err(AppError::StaleAttempt) => assert!(incident_rows(&w).await.is_empty()),
        other => panic!("unexpected incident result: {other:?}"),
    }
}

#[tokio::test]
async fn a_test_run_incident_uses_the_current_test_lease() {
    let w = world().await;
    let script = w
        .scripts
        .create(
            &w.admin,
            w.namespace.id(),
            sagat_application::scripts::ScriptSpec {
                name: Slug::parse("incident-test").expect("slug"),
                language: Language::TypeScript,
                source: Source::parse("export default () => 1").expect("source"),
                lockfile: None,
                input_schema: None,
                output_schema: None,
                policy: sagat_application::scripts::PolicyRequest::default(),
                retention: None,
            },
            &ctx(),
        )
        .await
        .expect("script");
    let queued = w
        .scripts
        .test(
            &w.admin,
            w.namespace.id(),
            script.id,
            b"{}".to_vec(),
            &ctx(),
        )
        .await
        .expect("queue test");
    let lease = w
        .runner
        .claim_test(&w.runner_identity, &[Language::TypeScript])
        .await
        .expect("claim")
        .expect("test lease");
    assert_eq!(lease.run.id, queued.id);
    let target = IncidentTarget::Test {
        namespace_id: w.namespace.id(),
        test_run_id: queued.id,
        lease_token: &lease.lease_token,
    };
    w.runner
        .record_incident(
            &w.runner_identity,
            &target,
            IncidentCategory::UnsupportedScheme,
            &ctx(),
        )
        .await
        .expect("incident committed");
    w.runner
        .record_incident(
            &w.runner_identity,
            &target,
            IncidentCategory::UnsupportedScheme,
            &ctx(),
        )
        .await
        .expect("duplicate");
    let rows = incident_rows(&w).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].1, "script");
    assert!(rows[0].2.contains(&queued.id.to_string()));

    let stale = IncidentTarget::Test {
        namespace_id: w.namespace.id(),
        test_run_id: queued.id,
        lease_token: "wrong-token",
    };
    assert_eq!(
        w.runner
            .record_incident(
                &w.runner_identity,
                &stale,
                IncidentCategory::UnsupportedScheme,
                &ctx(),
            )
            .await,
        Err(AppError::StaleAttempt)
    );
}
