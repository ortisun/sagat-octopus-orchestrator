//! A full retained-log budget cannot prevent a fenced job from reaching its terminal state.
use super::{built, claim, complete, create, publish};
use crate::support::engine::{ctx, world};
use sagat_application::scripts::TestReport;
use sagat_domain::quota::QuotaResource;
use sagat_domain::scripts::{JobStatus, Language, VersionStatus};

#[tokio::test]
async fn zero_log_budget_still_commits_build_and_test_results_with_truncation_evidence() {
    let w = world().await;
    let script = create(&w, "zero-log").await;
    let version = publish(&w, &script).await;
    w.quotas
        .set(&w.admin, None, QuotaResource::TenantLogBytes, 0, &ctx())
        .await
        .expect("zero logs");
    let job = claim(&w).await.expect("build");
    complete(&w, &job, built(&job, "export default () => 1")).await;
    let stored = w
        .scripts
        .version(
            &w.admin,
            w.namespace.id(),
            script.id,
            version.version.number,
            &ctx(),
        )
        .await
        .expect("version");
    assert_eq!(stored.status, VersionStatus::Ready);
    assert_eq!(stored.build_log.as_deref(), Some(""));
    let truncated: bool =
        sqlx::query_scalar("SELECT build_log_truncated FROM script_versions WHERE id=$1")
            .bind(stored.id.as_uuid())
            .fetch_one(&w.db.pool)
            .await
            .expect("truncation");
    assert!(truncated);
    let trial = w
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
    let leased = w
        .runner
        .claim_test(&w.runner_identity, &[Language::TypeScript])
        .await
        .expect("claim")
        .expect("job");
    let report = TestReport {
        result: Ok(b"{}".to_vec()),
        log: "canary-é".into(),
        log_truncated: false,
        denied: vec![],
        exit_code: Some(0),
        wall_ms: 1,
        start_ms: 1,
        memory_peak_bytes: None,
        inferred_input_schema: None,
        inferred_output_schema: None,
    };
    w.runner
        .complete_test(
            &w.runner_identity,
            trial.id,
            &leased.lease_token,
            report.clone(),
            &ctx(),
        )
        .await
        .expect("complete");
    w.runner
        .complete_test(
            &w.runner_identity,
            trial.id,
            &leased.lease_token,
            report,
            &ctx(),
        )
        .await
        .expect("replay");
    let row: (String, String, bool) =
        sqlx::query_as("SELECT status,log,log_truncated FROM script_test_runs WHERE id=$1")
            .bind(trial.id.as_uuid())
            .fetch_one(&w.db.pool)
            .await
            .expect("row");
    assert_eq!(
        row,
        (JobStatus::Succeeded.as_str().into(), String::new(), true)
    );
    let logs: i64 =
        sqlx::query_scalar("SELECT log_bytes FROM tenant_retained_usage WHERE tenant_id=$1")
            .bind(w.admin.tenant_id().as_uuid())
            .fetch_one(&w.db.pool)
            .await
            .expect("usage");
    assert_eq!(logs, 0);
}

#[tokio::test]
async fn exhausted_content_finishes_build_and_test_as_nonretryable_and_replays() {
    let w = world().await;
    let script = create(&w, "content-full").await;
    let version = publish(&w, &script).await;
    let job = claim(&w).await.expect("build");
    let trial = w
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
    let leased = w
        .runner
        .claim_test(&w.runner_identity, &[Language::TypeScript])
        .await
        .expect("claim")
        .expect("test job");
    w.quotas
        .set(&w.admin, None, QuotaResource::TenantContentBytes, 0, &ctx())
        .await
        .expect("no growth");
    w.quotas
        .set(&w.admin, None, QuotaResource::TenantLogBytes, 0, &ctx())
        .await
        .expect("no logs");
    sqlx::query("UPDATE quota_accounting_control SET content_enforcement_enabled=true, log_enforcement_enabled=true")
        .execute(&w.db.pool).await.expect("enable for proof");
    let report = built(&job, "export default () => 'not retained'");
    complete(&w, &job, report.clone()).await;
    complete(&w, &job, report).await;
    let stored = w
        .scripts
        .version(
            &w.admin,
            w.namespace.id(),
            script.id,
            version.version.number,
            &ctx(),
        )
        .await
        .expect("version");
    assert_eq!(stored.status, VersionStatus::Failed);
    assert_eq!(
        stored.failure_code,
        Some(sagat_domain::scripts::Failure::StorageQuotaExceeded)
    );
    assert!(stored.artifact.is_none());
    let artifacts: i64 = sqlx::query_scalar("SELECT count(*) FROM script_artifacts")
        .fetch_one(&w.db.pool)
        .await
        .expect("artifacts");
    assert_eq!(artifacts, 0);
    let report = TestReport {
        result: Ok(br#"{"canary":"rejected"}"#.to_vec()),
        log: "unretained".into(),
        log_truncated: false,
        denied: vec![],
        exit_code: Some(0),
        wall_ms: 1,
        start_ms: 1,
        memory_peak_bytes: None,
        inferred_input_schema: Some(b"{}".to_vec()),
        inferred_output_schema: Some(b"{}".to_vec()),
    };
    w.runner
        .complete_test(
            &w.runner_identity,
            trial.id,
            &leased.lease_token,
            report.clone(),
            &ctx(),
        )
        .await
        .expect("terminal fallback");
    w.runner
        .complete_test(
            &w.runner_identity,
            trial.id,
            &leased.lease_token,
            report,
            &ctx(),
        )
        .await
        .expect("replay");
    let row: (String, String, Option<Vec<u8>>, Option<Vec<u8>>) = sqlx::query_as(
        "SELECT status,failure,result,inferred_output_schema FROM script_test_runs WHERE id=$1",
    )
    .bind(trial.id.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("trial");
    assert_eq!(
        row,
        ("FAILED".into(), "STORAGE_QUOTA_EXCEEDED".into(), None, None)
    );
}

#[tokio::test]
async fn build_logs_use_an_exact_utf8_prefix_and_replay_does_not_charge_twice() {
    use sagat_application::scripts::BuildReport;
    let w = world().await;
    let script = create(&w, "utf8-log").await;
    let version = publish(&w, &script).await;
    let job = claim(&w).await.expect("build");
    w.quotas
        .set(&w.admin, None, QuotaResource::TenantLogBytes, 3, &ctx())
        .await
        .expect("three bytes");
    let mut report = built(&job, "bytes");
    if let BuildReport::Built { log, .. } = &mut report {
        *log = "aéz".into();
    }
    complete(&w, &job, report.clone()).await;
    complete(&w, &job, report).await;
    let stored = w
        .scripts
        .version(
            &w.admin,
            w.namespace.id(),
            script.id,
            version.version.number,
            &ctx(),
        )
        .await
        .expect("version");
    assert_eq!(stored.build_log.as_deref(), Some("aé"));
    assert!(stored.build_log_truncated);
    let logs: i64 =
        sqlx::query_scalar("SELECT log_bytes FROM tenant_retained_usage WHERE tenant_id=$1")
            .bind(w.admin.tenant_id().as_uuid())
            .fetch_one(&w.db.pool)
            .await
            .expect("usage");
    assert_eq!(logs, 3);
    let failed = create(&w, "no-diagnostic-space").await;
    let failed = w
        .scripts
        .update(
            &w.admin,
            w.namespace.id(),
            failed.id,
            sagat_application::scripts::DraftChange {
                source: Some(
                    sagat_domain::scripts::Source::parse("export default () => 'different source'")
                        .expect("source"),
                ),
                ..Default::default()
            },
            failed.revision,
            &ctx(),
        )
        .await
        .expect("distinct build");
    let published = publish(&w, &failed).await;
    let job = claim(&w).await.expect("build");
    complete(
        &w,
        &job,
        BuildReport::Failed {
            kind: sagat_application::scripts::BuildFailureKind::Rejected,
            message: "secret-free compiler detail".into(),
            log: "extra diagnostic".into(),
        },
    )
    .await;
    let stored = w
        .scripts
        .version(
            &w.admin,
            w.namespace.id(),
            failed.id,
            published.version.number,
            &ctx(),
        )
        .await
        .expect("failed version");
    assert_eq!(stored.failure.as_deref(), Some(""));
    assert_eq!(stored.build_log.as_deref(), Some(""));
    assert_eq!(
        stored.failure_code,
        Some(sagat_domain::scripts::Failure::BuildRejected)
    );
    assert!(stored.build_log_truncated);
}

#[tokio::test]
async fn cached_and_abandoned_builds_terminalize_at_zero_log_capacity() {
    let w = world().await;
    let (_, original) = super::ready(&w, "original-cache", "bytes").await;
    w.quotas
        .set(&w.admin, None, QuotaResource::TenantLogBytes, 0, &ctx())
        .await
        .expect("zero");
    let duplicate = create(&w, "cache-without-log").await;
    let published = publish(&w, &duplicate).await;
    assert!(
        claim(&w).await.is_none(),
        "claim finalizes the cached build"
    );
    let cached = w
        .scripts
        .version(
            &w.admin,
            w.namespace.id(),
            duplicate.id,
            published.version.number,
            &ctx(),
        )
        .await
        .expect("cached version");
    assert_eq!(cached.status, VersionStatus::Ready);
    assert_eq!(cached.artifact, original.artifact);
    assert_eq!(cached.build_log.as_deref(), Some(""));
    assert!(cached.build_log_truncated);
    let abandoned = w
        .scripts
        .create(
            &w.admin,
            w.namespace.id(),
            super::spec(
                "abandoned",
                Language::TypeScript,
                "export default () => 999",
            ),
            &ctx(),
        )
        .await
        .expect("create");
    let abandoned = publish(&w, &abandoned).await;
    let job = claim(&w).await.expect("claim");
    sqlx::query(
        "UPDATE script_build_jobs SET attempts=3, lease_expires_at='2000-01-01' WHERE id=$1",
    )
    .bind(job.job.as_uuid())
    .execute(&w.db.pool)
    .await
    .expect("expired final attempt");
    assert!(claim(&w).await.is_none());
    let row: (String, String, bool, String) = sqlx::query_as(
        "SELECT status,failure,build_log_truncated,failure_code FROM script_versions WHERE id=$1",
    )
    .bind(abandoned.version.id.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("recovered");
    assert_eq!(
        row,
        (
            "FAILED".into(),
            "".into(),
            true,
            "SCRIPT_SANDBOX_UNAVAILABLE".into()
        )
    );
}

#[tokio::test]
async fn concurrent_builds_share_the_last_log_byte_without_losing_terminal_results() {
    let w = world().await;
    for (name, source) in [
        ("log-a", "export default () => 100"),
        ("log-b", "export default () => 200"),
    ] {
        let script = w
            .scripts
            .create(
                &w.admin,
                w.namespace.id(),
                super::spec(name, Language::TypeScript, source),
                &ctx(),
            )
            .await
            .expect("create");
        publish(&w, &script).await;
    }
    let first = claim(&w).await.expect("first");
    let second = claim(&w).await.expect("second");
    w.quotas
        .set(&w.admin, None, QuotaResource::TenantLogBytes, 1, &ctx())
        .await
        .expect("one byte");
    let ((), ()) = tokio::join!(
        complete(&w, &first, built(&first, "a")),
        complete(&w, &second, built(&second, "b"))
    );
    let row: (i64,i64,i64) = sqlx::query_as("SELECT count(*),sum(octet_length(build_log))::bigint,count(*) FILTER (WHERE build_log_truncated) FROM script_versions WHERE status='READY'").fetch_one(&w.db.pool).await.expect("results");
    assert_eq!(row, (2, 1, 2));
}

#[tokio::test]
async fn application_build_log_bound_is_reported_as_truncation() {
    let w = world().await;
    let script = create(&w, "bounded-build-log").await;
    let version = publish(&w, &script).await;
    let job = claim(&w).await.expect("job");
    let mut report = built(&job, "bytes");
    if let sagat_application::scripts::BuildReport::Built { log, .. } = &mut report {
        *log = "x".repeat(65537);
    }
    complete(&w, &job, report).await;
    let row: (i32, bool) = sqlx::query_as(
        "SELECT octet_length(build_log),build_log_truncated FROM script_versions WHERE id=$1",
    )
    .bind(version.version.id.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("log");
    assert_eq!(row, (65536, true));
}

#[tokio::test]
async fn an_audit_failure_rolls_back_quota_terminalization_before_the_same_token_retries() {
    let w = world().await;
    let script = create(&w, "fallback-audit").await;
    let version = publish(&w, &script).await;
    let build = claim(&w).await.expect("build");
    let trial = w
        .scripts
        .test(
            &w.admin,
            w.namespace.id(),
            script.id,
            b"{}".to_vec(),
            &ctx(),
        )
        .await
        .expect("trial");
    let test = w
        .runner
        .claim_test(&w.runner_identity, &[Language::TypeScript])
        .await
        .expect("claim")
        .expect("test");
    w.quotas
        .set(&w.admin, None, QuotaResource::TenantContentBytes, 0, &ctx())
        .await
        .expect("full");
    sqlx::query("UPDATE quota_accounting_control SET content_enforcement_enabled=true")
        .execute(&w.db.pool)
        .await
        .expect("enforce");
    // Only the fallback audit is refused: the original artifact/result first reaches the real
    // deferred content quota check and rolls back, then the new failure transaction is tested.
    sqlx::query("ALTER TABLE audit_records ADD CONSTRAINT reject_quota_terminal_audit CHECK (reason IS NULL OR reason NOT LIKE '%STORAGE_QUOTA_EXCEEDED%') NOT VALID").execute(&w.db.pool).await.expect("audit fault");
    assert!(
        w.runner
            .complete_build(
                &w.runner_identity,
                build.job,
                &build.lease_token,
                built(&build, "new bytes"),
                &ctx()
            )
            .await
            .is_err()
    );
    let report = TestReport {
        result: Ok(b"{\"new\":true}".to_vec()),
        log: String::new(),
        log_truncated: false,
        denied: vec![],
        exit_code: Some(0),
        wall_ms: 1,
        start_ms: 1,
        memory_peak_bytes: None,
        inferred_input_schema: None,
        inferred_output_schema: None,
    };
    assert!(
        w.runner
            .complete_test(
                &w.runner_identity,
                trial.id,
                &test.lease_token,
                report.clone(),
                &ctx()
            )
            .await
            .is_err()
    );
    let status: String = sqlx::query_scalar("SELECT status FROM script_versions WHERE id=$1")
        .bind(version.version.id.as_uuid())
        .fetch_one(&w.db.pool)
        .await
        .expect("version status");
    assert_eq!(status, "BUILDING");
    let status: String = sqlx::query_scalar("SELECT status FROM script_test_runs WHERE id=$1")
        .bind(trial.id.as_uuid())
        .fetch_one(&w.db.pool)
        .await
        .expect("test status");
    assert_eq!(status, "RUNNING");
    let artifacts: i64 = sqlx::query_scalar("SELECT count(*) FROM script_artifacts")
        .fetch_one(&w.db.pool)
        .await
        .expect("artifacts");
    assert_eq!(artifacts, 0);
    sqlx::query("ALTER TABLE audit_records DROP CONSTRAINT reject_quota_terminal_audit")
        .execute(&w.db.pool)
        .await
        .expect("restore audit");
    complete(&w, &build, built(&build, "new bytes")).await;
    w.runner
        .complete_test(
            &w.runner_identity,
            trial.id,
            &test.lease_token,
            report,
            &ctx(),
        )
        .await
        .expect("retry terminalizes");
}

#[tokio::test]
async fn replacement_lease_between_quota_refusal_and_fallback_fences_the_old_test_result() {
    use sagat_application::entropy::Token;
    use sagat_application::scripts::{Finished, RunnerStore};
    use sagat_domain::scripts::Failure;
    let w = world().await;
    let script = create(&w, "replacement-fallback").await;
    let trial = w
        .scripts
        .test(
            &w.admin,
            w.namespace.id(),
            script.id,
            b"{}".to_vec(),
            &ctx(),
        )
        .await
        .expect("trial");
    let old = w
        .runner
        .claim_test(&w.runner_identity, &[Language::TypeScript])
        .await
        .expect("claim")
        .expect("old lease");
    w.quotas
        .set(&w.admin, None, QuotaResource::TenantContentBytes, 0, &ctx())
        .await
        .expect("full");
    sqlx::query("UPDATE quota_accounting_control SET content_enforcement_enabled=true")
        .execute(&w.db.pool)
        .await
        .expect("enforce");
    let mut report = TestReport {
        result: Ok(b"{\"new\":true}".to_vec()),
        log: String::new(),
        log_truncated: false,
        denied: vec![],
        exit_code: Some(0),
        wall_ms: 1,
        start_ms: 1,
        memory_peak_bytes: None,
        inferred_input_schema: None,
        inferred_output_schema: None,
    };
    // Drive the same two store calls used by complete_test with a deterministic takeover between
    // them, after the failed transaction released its locks and before fallback opens a new one.
    let before = w
        .store
        .finish_test(
            w.admin.tenant_id(),
            trial.id,
            &Token::hash_of(&old.lease_token),
            &report,
            time::OffsetDateTime::now_utc(),
            &[],
        )
        .await;
    assert!(matches!(
        before,
        Err(sagat_application::error::StoreError::QuotaExceeded { .. })
    ));
    sqlx::query("UPDATE script_test_runs SET lease_expires_at='2000-01-01' WHERE id=$1")
        .bind(trial.id.as_uuid())
        .execute(&w.db.pool)
        .await
        .expect("expire");
    let replacement = w
        .runner
        .claim_test(&w.runner_identity, &[Language::TypeScript])
        .await
        .expect("reclaim")
        .expect("replacement");
    report.result = Err(Failure::StorageQuotaExceeded);
    let result = w
        .store
        .finish_test(
            w.admin.tenant_id(),
            trial.id,
            &Token::hash_of(&old.lease_token),
            &report,
            time::OffsetDateTime::now_utc(),
            &[],
        )
        .await
        .expect("old fallback");
    assert_eq!(result, Finished::Stale);
    report.result = Err(Failure::Unavailable);
    w.runner
        .complete_test(
            &w.runner_identity,
            trial.id,
            &replacement.lease_token,
            report.clone(),
            &ctx(),
        )
        .await
        .expect("winner");
    report.result = Err(Failure::StorageQuotaExceeded);
    let result = w
        .store
        .finish_test(
            w.admin.tenant_id(),
            trial.id,
            &Token::hash_of(&old.lease_token),
            &report,
            time::OffsetDateTime::now_utc(),
            &[],
        )
        .await
        .expect("old retry");
    assert_eq!(result, Finished::Stale);
    let code: String = sqlx::query_scalar("SELECT failure FROM script_test_runs WHERE id=$1")
        .bind(trial.id.as_uuid())
        .fetch_one(&w.db.pool)
        .await
        .expect("winner preserved");
    assert_eq!(code, Failure::Unavailable.code());
}
