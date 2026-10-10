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
    assert_eq!(listed[0].attempt, Some(2), "newest first");
    let kept = &listed[1];
    assert_eq!(kept.wall_ms, 150);
    // A listing is small: the log is in the record.
    let record = w
        .scripts
        .execution(&w.admin, w.namespace.id(), kept.id, &ctx())
        .await
        .expect("the record");
    assert_eq!(record.execution.log, "hello\n");
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
    super::complete(&w, &job, super::built(&job, "week")).await;
    let week_digest = sagat_application::sandbox::digest_of(&super::artifact(&job, "week"));
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
    let record = w
        .scripts
        .execution(&w.admin, w.namespace.id(), listed[0].id, &ctx())
        .await
        .expect("the record");
    assert_eq!(record.execution.log.len(), 1024 * 1024);
}

#[tokio::test]
async fn records_are_kept_until_their_retention_ends_and_not_a_moment_longer() {
    use sagat_application::maintenance::MaintenanceStore as _;
    let w = world().await;
    let (_, version) = ready(&w, "short-lived", "bytes").await;
    let digest = digest_of(&version);
    for attempt in 1..=3 {
        w.runner
            .record_execution(
                &w.runner_identity,
                execution(&w, &digest, Uuid::now_v7(), attempt),
                &ctx(),
            )
            .await
            .expect("record");
    }
    // Two of the three reach the end of their retention.
    sqlx::query(
        "UPDATE script_executions SET expires_at = clock_timestamp() - interval '1 second' \
         WHERE id IN (SELECT id FROM script_executions ORDER BY id LIMIT 2)",
    )
    .execute(&w.db.pool)
    .await
    .expect("age");
    // A test run finished longer ago than its script keeps records, and one that finished today.
    let script = super::create(&w, "tested").await;
    for _ in 0..2 {
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
            .expect("test");
        let taken = w
            .runner
            .claim_test(
                &w.runner_identity,
                &[sagat_domain::scripts::Language::TypeScript],
            )
            .await
            .expect("claim")
            .expect("run");
        w.runner
            .complete_test(
                &w.runner_identity,
                queued.id,
                &taken.lease_token,
                sagat_application::scripts::TestReport {
                    result: Ok(b"{}".to_vec()),
                    log: String::new(),
                    log_truncated: false,
                    denied: Vec::new(),
                    exit_code: Some(0),
                    wall_ms: 1,
                    start_ms: 1,
                    memory_peak_bytes: None,
                    inferred_input_schema: None,
                    inferred_output_schema: None,
                },
                &ctx(),
            )
            .await
            .expect("complete");
    }
    sqlx::query(
        "UPDATE script_test_runs SET finished_at = clock_timestamp() - interval '15 days' \
         WHERE id IN (SELECT id FROM script_test_runs ORDER BY id LIMIT 1)",
    )
    .execute(&w.db.pool)
    .await
    .expect("age");

    assert_eq!(
        w.store.purge_script_records(100).await.expect("purge"),
        3,
        "two executions and one test run"
    );
    let left: i64 = sqlx::query_scalar("SELECT count(*) FROM script_executions")
        .fetch_one(&w.db.pool)
        .await
        .expect("count");
    assert_eq!(left, 1);
    let runs: i64 = sqlx::query_scalar("SELECT count(*) FROM script_test_runs")
        .fetch_one(&w.db.pool)
        .await
        .expect("count");
    assert_eq!(runs, 1);
    assert_eq!(
        w.store
            .purge_script_records(100)
            .await
            .expect("purge again"),
        0,
        "nothing else is due"
    );
    // The artifact and the version outlive every record of what ran.
    assert!(w.runner.artifact(&w.runner_identity, &digest).await.is_ok());
}
