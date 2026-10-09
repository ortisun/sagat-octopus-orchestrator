//! What a published script did: kept once per step attempt, for as long as the script says, with
//! what it was refused on the trail (`FR-005-007`, US04).

use sagat_application::error::AppError;
use sagat_application::scripts::{DraftChange, ExecutionStatus, NewExecution};
use sagat_domain::audit::AuditAction;
use sagat_domain::scripts::{Failure, Retention};
use uuid::Uuid;

use super::{digest_of, other_runner, ready};
use crate::support::engine::{World, ctx, world};

fn execution(
    w: &World,
    digest: &sagat_domain::scripts::Digest,
    run: Uuid,
    attempt: u32,
) -> NewExecution {
    NewExecution {
        namespace_id: w.namespace.id(),
        digest: digest.clone(),
        run_id: Some(run),
        node: Some("transform".to_owned()),
        attempt: Some(attempt),
        status: ExecutionStatus::Succeeded,
        failure: None,
        exit_code: Some(0),
        log: "hello\n".to_owned(),
        log_truncated: false,
        denied: Vec::new(),
        wall_ms: 150,
        start_ms: 90,
        memory_peak_bytes: Some(40_000_000),
        trace_id: Some("0af7651916cd43dd8448eb211c80319c".to_owned()),
    }
}

#[tokio::test]
async fn an_execution_is_kept_once_per_step_attempt_until_its_retention_ends() {
    let w = world().await;
    let (script, version) = ready(&w, "kept", "bytes").await;
    let digest = digest_of(&version);
    let run = Uuid::now_v7();
    assert!(
        w.runner
            .record_execution(&w.runner_identity, execution(&w, &digest, run, 1), &ctx())
            .await
            .expect("record")
    );
    assert!(
        !w.runner
            .record_execution(&w.runner_identity, execution(&w, &digest, run, 1), &ctx())
            .await
            .expect("again"),
        "the same attempt reported twice is one record"
    );
    assert!(
        w.runner
            .record_execution(&w.runner_identity, execution(&w, &digest, run, 2), &ctx())
            .await
            .expect("next attempt")
    );

    let listed = w
        .scripts
        .executions(&w.admin, w.namespace.id(), Some(&digest), None, 10, &ctx())
        .await
        .expect("list");
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0].execution.attempt, Some(2), "newest first");
    let kept = &listed[1];
    assert_eq!(kept.execution.log, "hello\n");
    assert_eq!(kept.execution.wall_ms, 150);
    // Fourteen days unless the script says otherwise.
    let kept_for = kept.expires_at - kept.created_at;
    assert_eq!(kept_for, time::Duration::days(14));

    // A script that keeps its records for a week.
    let mut week = super::ts("short-memory");
    week.retention = Some(Retention::days(7).expect("retention"));
    week.source =
        sagat_domain::scripts::Source::parse("export default () => 'week'\n").expect("source");
    let week = w
        .scripts
        .create(&w.admin, w.namespace.id(), week, &ctx())
        .await
        .expect("create");
    w.scripts
        .publish(&w.admin, w.namespace.id(), week.id, &ctx())
        .await
        .expect("publish");
    let job = super::claim(&w).await.expect("build");
    super::complete(&w, &job, super::built("week", &super::toolchains()[0].1)).await;
    let week_digest = sagat_application::sandbox::digest_of(&super::artifact("week"));
    w.runner
        .record_execution(
            &w.runner_identity,
            execution(&w, &week_digest, Uuid::now_v7(), 1),
            &ctx(),
        )
        .await
        .expect("record");
    let listed = w
        .scripts
        .executions(
            &w.admin,
            w.namespace.id(),
            Some(&week_digest),
            None,
            10,
            &ctx(),
        )
        .await
        .expect("list");
    assert_eq!(
        listed[0].expires_at - listed[0].created_at,
        time::Duration::days(7)
    );
    let _ = script;
}

#[tokio::test]
async fn what_a_script_was_refused_is_audited_once_however_often_it_asks() {
    let w = world().await;
    let (_, version) = ready(&w, "pushy", "bytes").await;
    let digest = digest_of(&version);
    for attempt in 1..=5 {
        let mut asked = execution(&w, &digest, Uuid::now_v7(), attempt);
        asked.status = ExecutionStatus::Failed;
        asked.failure = Some(Failure::Denied);
        asked.denied = vec![
            "fetch http://169.254.169.254".to_owned(),
            "fetch http://169.254.169.254".to_owned(),
            "secret payments-api-key".to_owned(),
        ];
        w.runner
            .record_execution(&w.runner_identity, asked, &ctx())
            .await
            .expect("record");
    }
    let audited: Vec<String> = sqlx::query_scalar(
        "SELECT reason FROM audit_records WHERE tenant_id = $1 AND action = $2 ORDER BY reason",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(AuditAction::ScriptViolation.as_str())
    .fetch_all(&w.db.pool)
    .await
    .expect("audit");
    assert_eq!(
        audited,
        vec![
            "fetch http://169.254.169.254".to_owned(),
            "secret payments-api-key".to_owned()
        ],
        "fifteen refusals, two entries: the trail grows with what was attempted, not with how often"
    );
}

#[tokio::test]
async fn an_execution_of_an_artifact_the_tenant_does_not_have_is_refused() {
    let w = world().await;
    let (_, version) = ready(&w, "theirs", "bytes").await;
    let digest = digest_of(&version);
    // The other tenant's runner reports on my artifact: nothing.
    let mut foreign = execution(&w, &digest, Uuid::now_v7(), 1);
    foreign.namespace_id = w.other_namespace.id();
    assert_eq!(
        w.runner
            .record_execution(&other_runner(&w), foreign, &ctx())
            .await,
        Err(AppError::NotFound)
    );
    // My runner reports on a namespace that is not mine.
    let mut elsewhere = execution(&w, &digest, Uuid::now_v7(), 1);
    elsewhere.namespace_id = w.other_namespace.id();
    assert_eq!(
        w.runner
            .record_execution(&w.runner_identity, elsewhere, &ctx())
            .await,
        Err(AppError::NotFound)
    );
    // And an unknown digest.
    let unknown = sagat_application::sandbox::digest_of(b"never built");
    assert_eq!(
        w.runner
            .record_execution(
                &w.runner_identity,
                execution(&w, &unknown, Uuid::now_v7(), 1),
                &ctx()
            )
            .await,
        Err(AppError::NotFound)
    );
    let _ = DraftChange::default();
}

#[tokio::test]
async fn an_oversized_log_is_cut_before_it_is_kept() {
    let w = world().await;
    let (_, version) = ready(&w, "chatty", "bytes").await;
    let digest = digest_of(&version);
    let mut loud = execution(&w, &digest, Uuid::now_v7(), 1);
    loud.log = "x".repeat(3 * 1024 * 1024);
    w.runner
        .record_execution(&w.runner_identity, loud, &ctx())
        .await
        .expect("record");
    let listed = w
        .scripts
        .executions(&w.admin, w.namespace.id(), Some(&digest), None, 10, &ctx())
        .await
        .expect("list");
    assert_eq!(listed[0].execution.log.len(), 1024 * 1024);
}
