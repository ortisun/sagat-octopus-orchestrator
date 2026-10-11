//! Test runs: a draft tried in the sandbox without publishing it, with its own quota (`FR-005-005`,
//! US01).

use sagat_application::error::AppError;
use sagat_application::scripts::{DraftChange, TestReport};
use sagat_domain::audit::AuditAction;
use sagat_domain::identity::Permission;
use sagat_domain::quota::QuotaResource;
use sagat_domain::scripts::{Failure, JobStatus, Language, Source};

use super::{create, developer, publish, viewer};
use crate::support::engine::{ctx, world};

fn report(result: Result<&[u8], Failure>) -> TestReport {
    TestReport {
        result: result.map(<[u8]>::to_vec),
        log: "adding\n".to_owned(),
        log_truncated: false,
        denied: Vec::new(),
        exit_code: Some(0),
        wall_ms: 120,
        start_ms: 80,
        memory_peak_bytes: Some(30_000_000),
        inferred_input_schema: Some(br#"{"type":"object"}"#.to_vec()),
        inferred_output_schema: Some(br#"{"type":"object"}"#.to_vec()),
    }
}

#[tokio::test]
async fn a_draft_is_tried_and_the_author_gets_the_result_the_log_and_the_inferred_schemas() {
    let w = world().await;
    let script = create(&w, "try-it").await;
    let queued = w
        .scripts
        .test(
            &w.admin,
            w.namespace.id(),
            script.id,
            br#"{"n":21}"#.to_vec(),
            &ctx(),
        )
        .await
        .expect("ask for a test run");
    assert_eq!(queued.status, JobStatus::Queued);
    assert_eq!(
        queued.draft, script.draft,
        "the draft as it was when it was asked for"
    );

    let taken = w
        .runner
        .claim_test(&w.runner_identity, &[Language::TypeScript])
        .await
        .expect("claim")
        .expect("a test run is waiting");
    assert_eq!(taken.run.id, queued.id);
    assert_eq!(taken.run.input, br#"{"n":21}"#);
    assert!(
        w.runner
            .claim_test(&w.runner_identity, &[Language::TypeScript])
            .await
            .expect("claim")
            .is_none(),
        "held by one runner"
    );
    assert_eq!(
        w.scripts
            .test_run(&w.admin, w.namespace.id(), script.id, queued.id, &ctx())
            .await
            .expect("get")
            .status,
        JobStatus::Running
    );

    w.runner
        .complete_test(
            &w.runner_identity,
            queued.id,
            &taken.lease_token,
            report(Ok(br#"{"double":42}"#)),
            &ctx(),
        )
        .await
        .expect("complete");
    let done = w
        .scripts
        .test_run(&w.admin, w.namespace.id(), script.id, queued.id, &ctx())
        .await
        .expect("get");
    assert_eq!(done.status, JobStatus::Succeeded);
    assert_eq!(done.result.as_deref(), Some(br#"{"double":42}"#.as_slice()));
    assert_eq!(done.log, "adding\n");
    assert_eq!(done.wall_ms, Some(120));
    assert_eq!(done.start_ms, Some(80));
    assert_eq!(done.memory_peak_bytes, Some(30_000_000));
    assert!(done.inferred_input_schema.is_some() && done.inferred_output_schema.is_some());
    assert!(done.finished_at.is_some());
    // The script was not published by being tried.
    let versions = w
        .scripts
        .versions(&w.admin, w.namespace.id(), script.id, None, 10, &ctx())
        .await
        .expect("versions");
    assert!(versions.is_empty());

    let runs = w
        .scripts
        .test_runs(&w.admin, w.namespace.id(), script.id, 10, &ctx())
        .await
        .expect("list");
    assert_eq!(runs.len(), 1);
}

#[tokio::test]
async fn a_test_run_that_fails_says_why_and_a_completion_is_accepted_once() {
    let w = world().await;
    let script = create(&w, "fails").await;
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
        .claim_test(&w.runner_identity, &[Language::TypeScript])
        .await
        .expect("claim")
        .expect("run");
    let mut failed = report(Err(Failure::Timeout));
    failed.denied = vec!["fetch http://169.254.169.254".to_owned()];
    failed.inferred_input_schema = None;
    failed.inferred_output_schema = None;
    w.runner
        .complete_test(
            &w.runner_identity,
            queued.id,
            &taken.lease_token,
            failed.clone(),
            &ctx(),
        )
        .await
        .expect("complete");
    // The answer was lost, the runner says it again: accepted, and nothing is written twice.
    w.runner
        .complete_test(
            &w.runner_identity,
            queued.id,
            &taken.lease_token,
            failed,
            &ctx(),
        )
        .await
        .expect("again");
    let done = w
        .scripts
        .test_run(&w.admin, w.namespace.id(), script.id, queued.id, &ctx())
        .await
        .expect("get");
    assert_eq!(done.status, JobStatus::Failed);
    assert_eq!(done.failure, Some(Failure::Timeout));
    assert_eq!(done.result, None);
    assert_eq!(done.denied, vec!["fetch http://169.254.169.254".to_owned()]);
    let violations: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_records WHERE tenant_id = $1 AND action = $2",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(AuditAction::ScriptViolation.as_str())
    .fetch_one(&w.db.pool)
    .await
    .expect("count");
    assert_eq!(
        violations, 1,
        "what the script was refused is on the trail, once"
    );
}

#[tokio::test]
async fn a_test_run_nobody_takes_fails_as_unavailable_and_gives_back_its_place() {
    let w = world().await;
    w.quotas
        .set(
            &w.admin,
            Some(w.namespace.id()),
            QuotaResource::ScriptJobs,
            1,
            &ctx(),
        )
        .await
        .expect("limit");
    let script = create(&w, "abandoned").await;
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
    assert!(matches!(
        w.scripts
            .test(
                &w.admin,
                w.namespace.id(),
                script.id,
                b"{}".to_vec(),
                &ctx()
            )
            .await,
        Err(AppError::QuotaExceeded(_))
    ));
    sqlx::query(
        "UPDATE script_test_runs SET deadline_at = clock_timestamp() - interval '1 second'",
    )
    .execute(&w.db.pool)
    .await
    .expect("age");
    let read = w
        .scripts
        .test_run(&w.admin, w.namespace.id(), script.id, queued.id, &ctx())
        .await
        .expect("get");
    assert_eq!(read.status, JobStatus::Failed);
    assert_eq!(read.failure, Some(Failure::Unavailable));
    w.scripts
        .test(
            &w.admin,
            w.namespace.id(),
            script.id,
            b"{}".to_vec(),
            &ctx(),
        )
        .await
        .expect("there is room again");
    assert!(
        w.runner
            .claim_test(&w.runner_identity, &[Language::TypeScript])
            .await
            .expect("claim")
            .is_some(),
        "the new one is offered, the abandoned one is not"
    );
}

#[tokio::test]
async fn a_test_run_whose_runner_vanishes_is_taken_again_then_given_up() {
    let w = world().await;
    let script = create(&w, "vanishing-test").await;
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
    let first = w
        .runner
        .claim_test(&w.runner_identity, &[Language::TypeScript])
        .await
        .expect("claim")
        .expect("run");
    sqlx::query(
        "UPDATE script_test_runs SET lease_expires_at = clock_timestamp() - interval '1 second'",
    )
    .execute(&w.db.pool)
    .await
    .expect("age");
    let second = w
        .runner
        .claim_test(&w.runner_identity, &[Language::TypeScript])
        .await
        .expect("claim")
        .expect("again");
    assert_eq!(second.run.id, queued.id);
    assert_eq!(
        w.runner
            .complete_test(
                &w.runner_identity,
                queued.id,
                &first.lease_token,
                report(Ok(b"{}")),
                &ctx()
            )
            .await,
        Err(AppError::StaleAttempt)
    );
    for _ in 0..2 {
        sqlx::query("UPDATE script_test_runs SET lease_expires_at = clock_timestamp() - interval '1 second' WHERE status = 'RUNNING'")
            .execute(&w.db.pool)
            .await
            .expect("age");
        let _ = w
            .runner
            .claim_test(&w.runner_identity, &[Language::TypeScript])
            .await
            .expect("claim");
    }
    let end = w
        .scripts
        .test_run(&w.admin, w.namespace.id(), script.id, queued.id, &ctx())
        .await
        .expect("get");
    assert_eq!(end.status, JobStatus::Failed);
    assert_eq!(end.failure, Some(Failure::Unavailable));
}

#[tokio::test]
async fn trying_a_draft_needs_write_permission_and_stays_in_the_tenant() {
    let w = world().await;
    let script = create(&w, "guarded-test").await;
    assert_eq!(
        w.scripts
            .test(
                &viewer(&w),
                w.namespace.id(),
                script.id,
                b"{}".to_vec(),
                &ctx()
            )
            .await
            .map(|_| ()),
        Err(AppError::Forbidden {
            permission: Permission::ScriptWrite
        })
    );
    assert!(
        w.scripts
            .test(
                &developer(&w),
                w.namespace.id(),
                script.id,
                b"{}".to_vec(),
                &ctx()
            )
            .await
            .is_ok()
    );
    assert_eq!(
        w.scripts
            .test(
                &w.other_admin,
                w.namespace.id(),
                script.id,
                b"{}".to_vec(),
                &ctx()
            )
            .await
            .map(|_| ()),
        Err(AppError::NotFound)
    );
    let too_big = vec![b' '; 240 * 1024 + 1];
    assert!(matches!(
        w.scripts
            .test(&w.admin, w.namespace.id(), script.id, too_big, &ctx())
            .await,
        Err(AppError::PayloadTooLarge(_))
    ));
    // Neither the builder of another tenant nor an ordinary worker takes it.
    assert!(
        w.runner
            .claim_test(&super::other_runner(&w), &[Language::TypeScript])
            .await
            .expect("claim")
            .is_none()
    );
    assert!(matches!(
        w.runner
            .claim_test(&w.worker, &[Language::TypeScript])
            .await,
        Err(AppError::Forbidden { .. })
    ));
}

#[tokio::test]
async fn a_test_run_takes_the_draft_as_it_was_not_as_it_became() {
    let w = world().await;
    let script = create(&w, "snapshot").await;
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
    w.scripts
        .update(
            &w.admin,
            w.namespace.id(),
            script.id,
            DraftChange {
                source: Some(
                    Source::parse("export default () => 'edited after'\n").expect("source"),
                ),
                ..DraftChange::default()
            },
            1,
            &ctx(),
        )
        .await
        .expect("edit");
    let taken = w
        .runner
        .claim_test(&w.runner_identity, &[Language::TypeScript])
        .await
        .expect("claim")
        .expect("run");
    assert_eq!(taken.run.id, queued.id);
    assert_eq!(taken.run.draft.source, script.draft.source);
    // Publishing does not take a test run's place or hold its result.
    publish(
        &w,
        &w.scripts
            .get(&w.admin, w.namespace.id(), script.id, &ctx())
            .await
            .expect("get"),
    )
    .await;
}

#[tokio::test]
async fn a_test_run_has_no_secrets_of_the_namespace() {
    // A test run reports what it was refused, and the platform never hands a test run a secret:
    // the secrets a production script declares are resolved only for a lease on a step of a run.
    // Here the report of a script that asked is what is audited and shown.
    let w = world().await;
    let script = create(&w, "no-secrets").await;
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
        .claim_test(&w.runner_identity, &[Language::TypeScript])
        .await
        .expect("claim")
        .expect("run");
    let mut asked = report(Err(Failure::Denied));
    asked.denied = vec!["secret payments-api-key".to_owned()];
    w.runner
        .complete_test(
            &w.runner_identity,
            queued.id,
            &taken.lease_token,
            asked,
            &ctx(),
        )
        .await
        .expect("complete");
    let reason: String =
        sqlx::query_scalar("SELECT reason FROM audit_records WHERE tenant_id = $1 AND action = $2")
            .bind(w.admin.tenant_id().as_uuid())
            .bind(AuditAction::ScriptViolation.as_str())
            .fetch_one(&w.db.pool)
            .await
            .expect("audit");
    assert_eq!(
        reason, "secret payments-api-key",
        "the name that was asked for, never a value"
    );
}

/// Asks for a test run of `script` and has it reported as finished with `denied`.
async fn tried_and_refused(
    w: &crate::support::engine::World,
    script: &sagat_application::scripts::ScriptRecord,
    denied: Vec<String>,
) {
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
        .claim_test(&w.runner_identity, &[Language::TypeScript])
        .await
        .expect("claim")
        .expect("run");
    let mut asked = report(Err(Failure::Denied));
    asked.denied = denied;
    w.runner
        .complete_test(
            &w.runner_identity,
            queued.id,
            &taken.lease_token,
            asked,
            &ctx(),
        )
        .await
        .expect("complete");
}

#[tokio::test]
async fn trying_a_script_over_and_over_audits_what_it_was_refused_once_and_bounds_the_trail() {
    let w = world().await;
    let script = create(&w, "again-and-again").await;
    for _ in 0..3 {
        tried_and_refused(&w, &script, vec!["fetch http://169.254.169.254".to_owned()]).await;
    }
    let rows: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_records WHERE tenant_id = $1 AND action = $2",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(AuditAction::ScriptViolation.as_str())
    .fetch_one(&w.db.pool)
    .await
    .expect("count");
    assert_eq!(
        rows, 1,
        "the same refusal of the same script is one entry, however often it is tried"
    );
    let resource: String = sqlx::query_scalar(
        "SELECT resource_id FROM audit_records WHERE tenant_id = $1 AND action = $2",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(AuditAction::ScriptViolation.as_str())
    .fetch_one(&w.db.pool)
    .await
    .expect("resource");
    assert_eq!(
        resource,
        script.id.to_string(),
        "against the script, not the run"
    );

    // Refusals the script chooses the words of cannot grow the trail without end either: at most a
    // hundred different ones are kept for one script, and each is short.
    for batch in 0..5 {
        let names: Vec<String> = (0..32)
            .map(|i| format!("secret n{batch}-{i}-{}", "x".repeat(500)))
            .collect();
        tried_and_refused(&w, &script, names).await;
    }
    let (rows, longest): (i64, Option<i32>) = sqlx::query_as(
        "SELECT count(*), max(length(reason)) FROM audit_records WHERE tenant_id = $1 AND action = $2",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(AuditAction::ScriptViolation.as_str())
    .fetch_one(&w.db.pool)
    .await
    .expect("count");
    assert_eq!(rows, 100, "the cap holds");
    assert!(longest.is_some_and(|n| n <= 200), "{longest:?}");
}

#[tokio::test]
async fn only_the_latest_finished_test_runs_of_a_script_are_kept() {
    let w = world().await;
    let script = create(&w, "chatty").await;
    sqlx::query(
        "INSERT INTO script_test_runs (id, tenant_id, namespace_id, script_id, language, status, \
             source, policy, retention_days, input, failure, finished_at, created_by, created_at, \
             deadline_at) \
         SELECT gen_random_uuid(), s.tenant_id, s.namespace_id, s.id, s.language, 'FAILED', 'x', \
                s.policy, s.retention_days, '{}', 'SCRIPT_ERROR', now(), 'test', now(), now() \
         FROM scripts s, generate_series(1, 105) n WHERE s.id = $1",
    )
    .bind(script.id.as_uuid())
    .execute(&w.db.pool)
    .await
    .expect("seed the runs");
    w.scripts
        .test(
            &w.admin,
            w.namespace.id(),
            script.id,
            b"{}".to_vec(),
            &ctx(),
        )
        .await
        .expect("another one");
    let finished: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM script_test_runs WHERE script_id = $1 AND status = 'FAILED'",
    )
    .bind(script.id.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("count");
    assert_eq!(finished, 100, "the oldest finished runs went");
    let queued: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM script_test_runs WHERE script_id = $1 AND status = 'QUEUED'",
    )
    .bind(script.id.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("count");
    assert_eq!(queued, 1, "and the new one is waiting");
}
