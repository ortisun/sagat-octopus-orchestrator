//! What a published script did: kept once per step attempt, for as long as the script says, with
//! what it was refused on the trail (`FR-005-007`, US04).

use sagat_application::error::AppError;
use sagat_application::scripts::{DraftChange, ExecutionStatus, NewExecution};
use sagat_application::worker::ClaimedTask;
use sagat_domain::audit::AuditAction;
use sagat_domain::quota::QuotaResource;
use sagat_domain::scripts::{Failure, Retention, activity_of};
use serde_json::json;
use uuid::Uuid;

use super::{digest_of, other_runner, ready};
use crate::support::engine::{World, ctx, document, world};

fn execution(w: &World, digest: &sagat_domain::scripts::Digest) -> NewExecution {
    NewExecution {
        namespace_id: w.namespace.id(),
        digest: digest.clone(),
        run_id: None,
        node: None,
        attempt: None,
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

fn execution_for(
    w: &World,
    digest: &sagat_domain::scripts::Digest,
    task: &ClaimedTask,
) -> NewExecution {
    let mut execution = execution(w, digest);
    execution.run_id = Some(task.run_id.as_uuid());
    execution.node = Some(task.instance.node().as_str().to_owned());
    execution.attempt = Some(task.fence.attempt);
    execution
}

async fn leased_script(
    w: &World,
    digest: &sagat_domain::scripts::Digest,
    name: &str,
) -> ClaimedTask {
    let activity = activity_of(digest);
    let (workflow, _) = w
        .publish(document(
            name,
            "transform",
            vec![json!({
                "id": "transform", "type": "script", "language": "typescript",
                "artifactDigest": digest.as_str()
            })],
        ))
        .await;
    let run = w.start(workflow, &json!({})).await;
    let session = w.session(&[activity.as_str()]).await;
    let task = w.one_task(&session).await;
    assert_eq!(task.run_id, run);
    task
}

#[tokio::test]
async fn an_execution_is_kept_once_per_step_attempt_until_its_retention_ends() {
    let w = world().await;
    let (script, version) = ready(&w, "kept", "bytes").await;
    let digest = digest_of(&version);
    let first = leased_script(&w, &digest, "kept-execution-one").await;
    assert!(
        w.runner
            .record_execution(
                &w.runner_identity,
                execution_for(&w, &digest, &first),
                &ctx()
            )
            .await
            .expect("record")
    );
    assert!(
        !w.runner
            .record_execution(
                &w.runner_identity,
                execution_for(&w, &digest, &first),
                &ctx()
            )
            .await
            .expect("again"),
        "the same attempt reported twice is one record"
    );
    let next = leased_script(&w, &digest, "kept-execution-two").await;
    let next_run = next.run_id;
    assert!(
        w.runner
            .record_execution(
                &w.runner_identity,
                execution_for(&w, &digest, &next),
                &ctx()
            )
            .await
            .expect("another real attempt")
    );

    let listed = w
        .scripts
        .executions(&w.admin, w.namespace.id(), Some(&digest), None, 10, &ctx())
        .await
        .expect("list");
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0].run_id, Some(next_run.as_uuid()), "newest first");
    assert_eq!(listed[0].attempt, Some(1));
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
        .record_execution(&w.runner_identity, execution(&w, &week_digest), &ctx())
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
async fn a_leased_script_pins_retention_before_policy_changes_and_replay_is_free() {
    let w = world().await;
    let (_, version) = ready(&w, "pinned-execution", "bytes").await;
    let digest = digest_of(&version);
    w.quotas
        .set(
            &w.admin,
            Some(w.namespace.id()),
            QuotaResource::ScriptRetentionDays,
            5,
            &ctx(),
        )
        .await
        .expect("five-day policy");
    let task = leased_script(&w, &digest, "pinned-execution-flow").await;
    let detail: serde_json::Value = sqlx::query_scalar(
        "SELECT detail FROM run_events WHERE tenant_id=$1 AND run_id=$2 AND node=$3 \
         AND event_type='step.leased' AND attempt=$4",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(task.run_id.as_uuid())
    .bind(task.instance.node().as_str())
    .bind(i64::from(task.fence.attempt))
    .fetch_one(&w.db.pool)
    .await
    .expect("lease detail");
    assert_eq!(detail["scriptRetentionPolicyVersion"], 1);
    assert_eq!(detail["scriptRetentionDays"], 5);

    w.quotas
        .set(
            &w.admin,
            Some(w.namespace.id()),
            QuotaResource::ScriptRetentionDays,
            0,
            &ctx(),
        )
        .await
        .expect("disable new retention");
    let report = execution_for(&w, &digest, &task);
    assert!(
        w.runner
            .record_execution(&w.runner_identity, report.clone(), &ctx())
            .await
            .expect("the leased pin survives the later policy change")
    );
    let before: (time::OffsetDateTime, time::OffsetDateTime) = sqlx::query_as(
        "SELECT created_at,expires_at FROM script_executions WHERE tenant_id=$1 AND run_id=$2 \
         AND node=$3 AND attempt=$4",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(task.run_id.as_uuid())
    .bind(task.instance.node().as_str())
    .bind(i64::from(task.fence.attempt))
    .fetch_one(&w.db.pool)
    .await
    .expect("execution");
    assert_eq!(before.1 - before.0, time::Duration::days(5));
    let usage_before: (i64, i64) = sqlx::query_as(
        "SELECT content_bytes,log_bytes FROM tenant_retained_usage WHERE tenant_id=$1",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("usage");
    assert!(
        !w.runner
            .record_execution(&w.runner_identity, report, &ctx())
            .await
            .expect("replay")
    );
    let after: (time::OffsetDateTime, time::OffsetDateTime) = sqlx::query_as(
        "SELECT created_at,expires_at FROM script_executions WHERE tenant_id=$1 AND run_id=$2 \
         AND node=$3 AND attempt=$4",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(task.run_id.as_uuid())
    .bind(task.instance.node().as_str())
    .bind(i64::from(task.fence.attempt))
    .fetch_one(&w.db.pool)
    .await
    .expect("execution after replay");
    let usage_after: (i64, i64) = sqlx::query_as(
        "SELECT content_bytes,log_bytes FROM tenant_retained_usage WHERE tenant_id=$1",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("usage after replay");
    assert_eq!(after, before, "replay does not move expiry");
    assert_eq!(
        usage_after, usage_before,
        "replay does not charge retained bytes"
    );

    assert_eq!(
        w.runner
            .record_execution(&w.runner_identity, execution(&w, &digest), &ctx())
            .await,
        Err(AppError::QuotaExceeded("script retention in the namespace")),
        "an unbound record has no immutable pin and obeys the current zero policy"
    );
}

#[tokio::test]
async fn a_bound_report_must_match_the_exact_leased_namespace_digest_run_node_and_attempt() {
    let w = world().await;
    let (_, version) = ready(&w, "bound-report", "bytes").await;
    let digest = digest_of(&version);
    let mut different = super::ts("different-bound-report");
    different.source = sagat_domain::scripts::Source::parse(
        "export default function () { return { different: true }; }\n",
    )
    .expect("different source");
    let different = w
        .scripts
        .create(&w.admin, w.namespace.id(), different, &ctx())
        .await
        .expect("different script");
    let published = w
        .scripts
        .publish(&w.admin, w.namespace.id(), different.id, &ctx())
        .await
        .expect("publish different script");
    let job = super::claim(&w).await.expect("different build");
    super::complete(&w, &job, super::built(&job, "other")).await;
    let other_version = w
        .scripts
        .version(
            &w.admin,
            w.namespace.id(),
            different.id,
            published.version.number,
            &ctx(),
        )
        .await
        .expect("different version");
    let other_digest = digest_of(&other_version);
    let task = leased_script(&w, &digest, "bound-report-flow").await;
    let exact = execution_for(&w, &digest, &task);

    let mut cases = Vec::new();
    let mut namespace = exact.clone();
    namespace.namespace_id = w.other_namespace.id();
    cases.push(("namespace", namespace));
    let mut wrong_digest = exact.clone();
    wrong_digest.digest = other_digest;
    cases.push(("digest", wrong_digest));
    let mut run = exact.clone();
    run.run_id = Some(Uuid::now_v7());
    cases.push(("run", run));
    let mut node = exact.clone();
    node.node = Some("another-node".into());
    cases.push(("node", node));
    let mut attempt = exact.clone();
    attempt.attempt = Some(task.fence.attempt + 1);
    cases.push(("attempt", attempt));
    for (label, report) in cases {
        assert_eq!(
            w.runner
                .record_execution(&w.runner_identity, report, &ctx())
                .await,
            Err(AppError::NotFound),
            "{label}"
        );
    }

    for (label, run_id, node, attempt) in [
        ("run only", exact.run_id, None, None),
        ("run and node", exact.run_id, exact.node.clone(), None),
        ("node and attempt", None, exact.node.clone(), exact.attempt),
        ("attempt only", None, None, exact.attempt),
    ] {
        let mut partial = execution(&w, &digest);
        partial.run_id = run_id;
        partial.node = node;
        partial.attempt = attempt;
        assert_eq!(
            w.runner
                .record_execution(&w.runner_identity, partial, &ctx())
                .await,
            Err(AppError::NotFound),
            "{label}"
        );
    }
}

#[tokio::test]
async fn legacy_leases_are_accepted_but_unknown_or_incomplete_retention_pins_fail_closed() {
    let w = world().await;
    let (_, version) = ready(&w, "lease-format", "bytes").await;
    let digest = digest_of(&version);

    let legacy = leased_script(&w, &digest, "legacy-lease-format").await;
    let unknown = leased_script(&w, &digest, "unknown-marker-lease-format").await;
    let missing = leased_script(&w, &digest, "missing-pin-lease-format").await;
    let invalid = leased_script(&w, &digest, "invalid-pin-lease-format").await;
    // These rows model data written by older/corrupt server versions. Production roles cannot
    // mutate the journal; the test owner briefly disables only that guard to construct history.
    sqlx::query("ALTER TABLE run_events DISABLE TRIGGER run_events_append_only")
        .execute(&w.db.pool)
        .await
        .expect("fixture can model historical lease metadata");
    sqlx::query(
        "UPDATE run_events SET detail = detail - 'scriptRetentionPolicyVersion' \
                                   - 'scriptRetentionDays' \
         WHERE tenant_id=$1 AND run_id=$2 AND node=$3 AND event_type='step.leased' AND attempt=$4",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(legacy.run_id.as_uuid())
    .bind(legacy.instance.node().as_str())
    .bind(i64::from(legacy.fence.attempt))
    .execute(&w.db.pool)
    .await
    .expect("make a legacy lease");
    sqlx::query(
        "UPDATE run_events SET detail = jsonb_set(detail, '{scriptRetentionPolicyVersion}', '2'::jsonb) \
         WHERE tenant_id=$1 AND run_id=$2 AND node=$3 AND event_type='step.leased' AND attempt=$4",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(unknown.run_id.as_uuid())
    .bind(unknown.instance.node().as_str())
    .bind(i64::from(unknown.fence.attempt))
    .execute(&w.db.pool)
    .await
    .expect("unknown marker");
    sqlx::query(
        "UPDATE run_events SET detail = detail - 'scriptRetentionDays' \
         WHERE tenant_id=$1 AND run_id=$2 AND node=$3 AND event_type='step.leased' AND attempt=$4",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(missing.run_id.as_uuid())
    .bind(missing.instance.node().as_str())
    .bind(i64::from(missing.fence.attempt))
    .execute(&w.db.pool)
    .await
    .expect("missing pin");
    sqlx::query(
        "UPDATE run_events SET detail = jsonb_set(detail, '{scriptRetentionDays}', '0'::jsonb) \
         WHERE tenant_id=$1 AND run_id=$2 AND node=$3 AND event_type='step.leased' AND attempt=$4",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(invalid.run_id.as_uuid())
    .bind(invalid.instance.node().as_str())
    .bind(i64::from(invalid.fence.attempt))
    .execute(&w.db.pool)
    .await
    .expect("invalid pin");
    sqlx::query("ALTER TABLE run_events ENABLE TRIGGER run_events_append_only")
        .execute(&w.db.pool)
        .await
        .expect("restore append-only journal guard");

    assert!(
        w.runner
            .record_execution(
                &w.runner_identity,
                execution_for(&w, &digest, &legacy),
                &ctx()
            )
            .await
            .expect("legacy report")
    );
    assert_eq!(
        w.runner
            .record_execution(
                &w.runner_identity,
                execution_for(&w, &digest, &unknown),
                &ctx()
            )
            .await,
        Err(AppError::Internal)
    );
    assert_eq!(
        w.runner
            .record_execution(
                &w.runner_identity,
                execution_for(&w, &digest, &missing),
                &ctx()
            )
            .await,
        Err(AppError::Internal)
    );
    assert_eq!(
        w.runner
            .record_execution(
                &w.runner_identity,
                execution_for(&w, &digest, &invalid),
                &ctx()
            )
            .await,
        Err(AppError::Internal)
    );
}

#[tokio::test]
async fn execution_trace_ids_are_closed_lowercase_nonzero_hex_in_the_service_and_database() {
    let w = world().await;
    let (_, version) = ready(&w, "trace-format", "bytes").await;
    let digest = digest_of(&version);
    for invalid in [
        "a".repeat(33),
        "not-a-trace".to_owned(),
        "A".repeat(32),
        "0".repeat(32),
    ] {
        let mut report = execution(&w, &digest);
        report.trace_id = Some(invalid);
        assert!(matches!(
            w.runner
                .record_execution(&w.runner_identity, report, &ctx())
                .await,
            Err(AppError::NotAllowed(_))
        ));
    }

    w.runner
        .record_execution(&w.runner_identity, execution(&w, &digest), &ctx())
        .await
        .expect("valid report");
    let id: Uuid = sqlx::query_scalar(
        "SELECT id FROM script_executions WHERE tenant_id=$1 ORDER BY created_at DESC LIMIT 1",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("execution id");
    for invalid in ["g".repeat(32), "A".repeat(32), "0".repeat(32)] {
        assert!(
            sqlx::query("UPDATE script_executions SET trace_id=$1 WHERE id=$2")
                .bind(invalid)
                .bind(id)
                .execute(&w.db.pool)
                .await
                .is_err(),
            "the database constraint is the final boundary"
        );
    }
}

#[tokio::test]
async fn what_a_script_was_refused_is_audited_once_however_often_it_asks() {
    let w = world().await;
    let (_, version) = ready(&w, "pushy", "bytes").await;
    let digest = digest_of(&version);
    for _ in 1..=5 {
        let mut asked = execution(&w, &digest);
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
    let mut foreign = execution(&w, &digest);
    foreign.namespace_id = w.other_namespace.id();
    assert_eq!(
        w.runner
            .record_execution(&other_runner(&w), foreign, &ctx())
            .await,
        Err(AppError::NotFound)
    );
    // My runner reports on a namespace that is not mine.
    let mut elsewhere = execution(&w, &digest);
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
            .record_execution(&w.runner_identity, execution(&w, &unknown), &ctx())
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
    let mut loud = execution(&w, &digest);
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
    for _ in 1..=3 {
        w.runner
            .record_execution(&w.runner_identity, execution(&w, &digest), &ctx())
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
