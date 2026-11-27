//! Task T041-7: `AppendStepLogs` and the registration policy through the worker gRPC API, against a real PostgreSQL.
//! The ingest's rules are tested by calling `StepLogIngest` directly in the other modules; these are the handler's
//! part: protocol 1.7's negotiation, the policy of `RegisterWorkerResponse`, the wire mapping of a batch and its
//! verdict, the audit of an `AUTHORITY` answer, which only the handler writes (decision D041-146 (2)), and what the
//! security review of the handler added: the batches' own rate budgets (decision D041-150), P1 before the ticket, a
//! ticket kept while its call waits for its audit, and a batch naming another tenant's run.

use reqwest::Method;
use sagat_api_grpc::pb::{
    AppendStepLogsRequest, AppendStepLogsResponse, StepLogField, StepLogLevel, StepLogRecord,
    StepLogStopReason, StepLogVerdict, TaskLease, step_log_field,
};
use sagat_api_http::RateLimitConfig;
use sagat_domain::identity::Role;
use serde_json::json;
use tonic::Code;

use crate::support::api::Options;
use crate::support::worker::{
    NAMESPACE, Platform, TENANT, Worker, platform, platform_with, single_ir,
};

const ACTIVITY: &str = "act.a";
const DENIED: &str = "access.denied.permission";

/// A worker of acme registered at `version`, bound to every activity; the policy it was told.
async fn registered<'a>(
    p: &'a Platform,
    sub: &str,
    version: &str,
) -> (Worker<'a>, String, Option<bool>) {
    let mut worker = p.worker(sub).await;
    let mut message = Worker::register_message(&[ACTIVITY], 10);
    message
        .capabilities
        .as_mut()
        .expect("capabilities")
        .protocol_version = version.to_owned();
    let request = worker.req(message);
    let answer = worker
        .client
        .register_worker(request)
        .await
        .expect("register")
        .into_inner();
    worker.worker_id = answer.worker_id;
    worker.session = answer.session_token;
    let policy = answer.step_logs.map(|policy| policy.enabled);
    (worker, answer.accepted_protocol_version, policy)
}

/// A run of a one-activity workflow and its step leased to `worker`.
async fn leased(p: &Platform, worker: &mut Worker<'_>) -> TaskLease {
    let name = format!("logs-rpc-{}", uuid::Uuid::now_v7().simple());
    let workflow = p.publish(&name, &single_ir(&name, None)).await;
    p.start(&workflow, &json!({})).await;
    worker.one_task().await
}

fn batch(worker: &Worker<'_>, lease: &TaskLease, sequences: &[u64]) -> AppendStepLogsRequest {
    AppendStepLogsRequest {
        worker_id: worker.worker_id.clone(),
        session_token: worker.session.clone(),
        task_id: lease.task_id.clone(),
        generation: lease.generation,
        lease_token: lease.lease_token.clone(),
        run_id: lease.run_id.clone(),
        step_id: lease.step_id.clone(),
        attempt: lease.attempt,
        instance: lease.instance.clone(),
        records: sequences
            .iter()
            .map(|&sequence| StepLogRecord {
                sequence,
                worker_time_epoch_ms: 1_700_000_000_000,
                level: StepLogLevel::Info.into(),
                message: format!("line {sequence}"),
                fields: vec![StepLogField {
                    key: "n".into(),
                    value: Some(step_log_field::Value::IntValue(
                        i64::try_from(sequence).expect("small"),
                    )),
                }],
                exception: String::new(),
                truncated: false,
            })
            .collect(),
    }
}

async fn append_batch(
    worker: &mut Worker<'_>,
    lease: &TaskLease,
    sequences: &[u64],
) -> Result<AppendStepLogsResponse, tonic::Status> {
    let message = batch(worker, lease, sequences);
    append(worker, message).await
}

async fn append(
    worker: &mut Worker<'_>,
    message: AppendStepLogsRequest,
) -> Result<AppendStepLogsResponse, tonic::Status> {
    let request = worker.req(message);
    worker
        .client
        .append_step_logs(request)
        .await
        .map(tonic::Response::into_inner)
}

async fn stored_lines(p: &Platform, run: &str) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM step_log_lines WHERE run_id = $1::uuid AND kind = 'line'",
    )
    .bind(run)
    .fetch_one(&p.api.db.pool)
    .await
    .expect("count the stored lines")
}

/// `(actor, reason, resource_id)` of the permission denials of acme.
async fn denials(p: &Platform) -> Vec<(String, Option<String>, String)> {
    sqlx::query_as(
        "SELECT actor, reason, resource_id FROM audit_records \
         WHERE tenant_id = $1 AND action = $2 ORDER BY created_at, id",
    )
    .bind(p.tenant.as_uuid())
    .bind(DENIED)
    .fetch_all(&p.api.db.pool)
    .await
    .expect("the denial records")
}

/// Design section 1: the server offers 1.7 to an ordinary worker and tells it the step-log policy; below 1.7 no policy
/// is sent; capture turned off in the namespace is told as `enabled = false`.
#[tokio::test]
async fn a_1_7_registration_is_told_the_step_log_policy_and_an_older_one_none() {
    let p = platform().await;
    let (_, accepted, policy) = registered(&p, "wanda", "1.999").await;
    assert_eq!((accepted.as_str(), policy), ("1.7", Some(true)));
    let (_, accepted, policy) = registered(&p, "wilma", "1.6").await;
    assert_eq!(
        (accepted.as_str(), policy),
        ("1.6", None),
        "below 1.7 the field is absent"
    );
    let namespace = p.namespace_id().await;
    let (status, body) = p
        .rest(
            Method::PUT,
            &format!("/namespaces/{namespace}/quotas/step_log_attempt_records"),
            Some(&json!({"limit": 0})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let (_, accepted, policy) = registered(&p, "willa", "1.7").await;
    assert_eq!(
        (accepted.as_str(), policy),
        ("1.7", Some(false)),
        "capture off in the namespace"
    );
}

/// `SAGAT_STEP_LOGS_ENABLED=false` (P2): the registration is told capture is off, and a batch is answered
/// `STOP_CAPTURE` (`CAPTURE_OFF`) and stores nothing.
#[tokio::test]
async fn with_the_deployment_switch_off_capture_is_off_at_registration_and_for_a_batch() {
    let p = platform_with(Options {
        step_logs_enabled: false,
        ..Options::default()
    })
    .await;
    let (mut worker, accepted, policy) = registered(&p, "wanda", "1.7").await;
    assert_eq!((accepted.as_str(), policy), ("1.7", Some(false)));
    let lease = leased(&p, &mut worker).await;
    let answer = append_batch(&mut worker, &lease, &[1, 2])
        .await
        .expect("a verdict");
    assert_eq!(answer.verdict, i32::from(StepLogVerdict::StopCapture));
    assert_eq!(answer.stop_reason, i32::from(StepLogStopReason::CaptureOff));
    assert_eq!(answer.stored_through_sequence, 0);
    assert_eq!(stored_lines(&p, &lease.run_id).await, 0);
}

/// A batch through the RPC is stored and answered `CONTINUE` with the stream's highest sequence; its resend stores
/// nothing more; a malformed batch is `INVALID_ARGUMENT`.
#[tokio::test]
async fn a_batch_through_the_rpc_is_stored_once_and_a_malformed_one_is_refused() {
    let p = platform().await;
    let (mut worker, accepted, _) = registered(&p, "wanda", "1.7").await;
    assert_eq!(accepted, "1.7");
    let lease = leased(&p, &mut worker).await;
    for _ in 0..2 {
        let answer = append_batch(&mut worker, &lease, &[1, 2, 3])
            .await
            .expect("a verdict");
        assert_eq!(answer.verdict, i32::from(StepLogVerdict::Continue));
        assert_eq!(
            answer.stop_reason,
            i32::from(StepLogStopReason::Unspecified)
        );
        assert_eq!(answer.stored_through_sequence, 3);
        assert_eq!(answer.retry_after_ms, 0);
    }
    assert_eq!(stored_lines(&p, &lease.run_id).await, 3, "stored once");
    let empty = append_batch(&mut worker, &lease, &[])
        .await
        .expect_err("no record");
    assert_eq!(empty.code(), Code::InvalidArgument, "{empty:?}");
    let mut without_step = batch(&worker, &lease, &[4]);
    without_step.step_id.clear();
    let refused = append(&mut worker, without_step)
        .await
        .expect_err("no step id");
    assert_eq!(refused.code(), Code::InvalidArgument, "{refused:?}");
    assert_eq!(stored_lines(&p, &lease.run_id).await, 3);
}

/// Decision D041-146 (2): an `AUTHORITY` answer is a verdict, not an error, and the handler audits it itself, once a
/// batch, as the refusal a report would meet (`NOT_BOUND`), naming the RPC; nothing is stored.
#[tokio::test]
async fn an_authority_answer_is_audited_by_the_handler_and_stores_nothing() {
    let p = platform().await;
    let (mut worker, _, _) = registered(&p, "wanda", "1.7").await;
    let lease = leased(&p, &mut worker).await;
    sqlx::query(
        "DELETE FROM worker_bindings WHERE tenant_id = $1 AND subject = 'wanda' \
         AND namespace_id = (SELECT id FROM namespaces WHERE tenant_id = $1 AND slug = $2)",
    )
    .bind(p.tenant.as_uuid())
    .bind(NAMESPACE)
    .execute(&p.api.db.pool)
    .await
    .expect("remove the binding");
    assert!(denials(&p).await.is_empty(), "no denial yet");
    let answer = append_batch(&mut worker, &lease, &[1])
        .await
        .expect("a verdict, not an error");
    assert_eq!(answer.verdict, i32::from(StepLogVerdict::StopAttempt));
    assert_eq!(answer.stop_reason, i32::from(StepLogStopReason::Authority));
    assert_eq!(answer.stored_through_sequence, 0);
    let rows = denials(&p).await;
    assert_eq!(
        rows,
        vec![(
            "wanda".to_owned(),
            Some("refused: NOT_BOUND".to_owned()),
            "grpc WorkerService/AppendStepLogs".to_owned()
        )],
        "the handler wrote the record before it answered"
    );
    assert_eq!(stored_lines(&p, &lease.run_id).await, 0);
}

async fn remove_binding(p: &Platform, sub: &str) {
    sqlx::query(
        "DELETE FROM worker_bindings WHERE tenant_id = $1 AND subject = $2 \
         AND namespace_id = (SELECT id FROM namespaces WHERE tenant_id = $1 AND slug = $3)",
    )
    .bind(p.tenant.as_uuid())
    .bind(sub)
    .bind(NAMESPACE)
    .execute(&p.api.db.pool)
    .await
    .expect("remove the binding");
}

/// A transaction that holds `audit_records` in `SHARE` mode, so that every audit insert waits for it, and its backend.
async fn hold_the_audit_trail(p: &Platform) -> (sqlx::Transaction<'static, sqlx::Postgres>, i32) {
    let mut locker = p.api.db.pool.begin().await.expect("begin");
    sqlx::query("LOCK TABLE audit_records IN SHARE MODE")
        .execute(&mut *locker)
        .await
        .expect("hold the audit trail");
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *locker)
        .await
        .expect("the holder's backend");
    (locker, pid)
}

/// Waits until `count` backends wait on the holder `pid` (seen in `pg_stat_activity`, never by time).
async fn blocked_on(p: &Platform, pid: i32, count: i64) {
    crate::support::eventually("audit writes waiting on the held trail", || async {
        let waiting: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pg_stat_activity WHERE $1 = ANY(pg_blocking_pids(pid))",
        )
        .bind(pid)
        .fetch_one(&p.api.db.pool)
        .await
        .expect("the waiting backends");
        (waiting >= count).then_some(())
    })
    .await;
}

/// The security review of task T041-7, Minor 2: a batch answered `AUTHORITY` keeps its tenant's ticket while it waits
/// for its audit, so the next batch of that tenant finds no ticket and is told to retry at once (a ticket refusal,
/// through the RPC), instead of entering the ingest beside it.
#[tokio::test]
async fn a_batch_waiting_for_its_audit_keeps_its_ticket_and_the_next_one_is_told_to_retry() {
    let p = platform_with(Options {
        step_log_tenant_calls: 1,
        ..Options::default()
    })
    .await;
    let (mut worker, _, _) = registered(&p, "wanda", "1.7").await;
    let lease = leased(&p, &mut worker).await;
    remove_binding(&p, "wanda").await;
    let (first, second) = (
        worker.req(batch(&worker, &lease, &[1])),
        worker.req(batch(&worker, &lease, &[2])),
    );
    let (mut a_client, mut b_client) = (worker.client.clone(), worker.client.clone());
    let (locker, pid) = hold_the_audit_trail(&p).await;
    let held = async {
        blocked_on(&p, pid, 1).await;
        let call = b_client.append_step_logs(second);
        tokio::pin!(call);
        // Answered while the first batch's audit still waits, or seen waiting for an audit of its own (before the
        // fix: the second batch entered the ingest, was answered `AUTHORITY` and waited for its audit).
        let early = tokio::select! {
            answer = &mut call => Some(answer),
            () = blocked_on(&p, pid, 2) => None,
        };
        locker.commit().await.expect("release the trail");
        early
    };
    let (first, next) = tokio::join!(a_client.append_step_logs(first), held);
    let first = first.expect("a verdict").into_inner();
    assert_eq!(first.stop_reason, i32::from(StepLogStopReason::Authority));
    let next = next
        .expect("answered while the first batch's audit waited, never audited itself")
        .expect("a verdict")
        .into_inner();
    assert_eq!(next.verdict, i32::from(StepLogVerdict::RetryLater));
    assert_eq!(next.retry_after_ms, 200);
    assert_eq!(next.stored_through_sequence, 0);
    assert_eq!(
        denials(&p).await.len(),
        1,
        "only the first batch reached the ingest"
    );
}

/// The security review of task T041-7, Minor 3: P1 comes before the ticket. A runner token is refused
/// `PERMISSION_DENIED` and audited even while its tenant's tickets are all taken, never told to retry.
#[tokio::test]
async fn a_runner_token_is_refused_and_audited_even_when_its_tenants_tickets_are_taken() {
    let p = platform_with(Options {
        step_log_tenant_calls: 1,
        ..Options::default()
    })
    .await;
    let (mut worker, _, _) = registered(&p, "wanda", "1.7").await;
    let lease = leased(&p, &mut worker).await;
    remove_binding(&p, "wanda").await;
    p.grant(p.tenant, "rita", Role::Runner).await;
    let runner = p.connect("rita", TENANT).await;
    let mut runner_client = runner.client.clone();
    let refused = runner.req(batch(&worker, &lease, &[1]));
    let first = worker.req(batch(&worker, &lease, &[1]));
    let mut a_client = worker.client.clone();
    let (locker, pid) = hold_the_audit_trail(&p).await;
    let held = async {
        blocked_on(&p, pid, 1).await;
        let call = runner_client.append_step_logs(refused);
        tokio::pin!(call);
        // Answered while the trail is still held only if it never reached its own audit (before the fix: RETRY_LATER).
        let early = tokio::select! {
            answer = &mut call => Some(answer),
            () = blocked_on(&p, pid, 2) => None,
        };
        locker.commit().await.expect("release the trail");
        match early {
            Some(answer) => answer,
            None => call.await,
        }
    };
    let (first, refused) = tokio::join!(a_client.append_step_logs(first), held);
    assert_eq!(
        first.expect("a verdict").into_inner().stop_reason,
        i32::from(StepLogStopReason::Authority)
    );
    let status = refused.expect_err("a runner token is refused");
    assert_eq!(status.code(), Code::PermissionDenied, "{status:?}");
    let rita = crate::support::eventually("the runner's denial is recorded", || async {
        let rows: Vec<_> = denials(&p)
            .await
            .into_iter()
            .filter(|(actor, _, _)| actor == "rita")
            .collect();
        (!rows.is_empty()).then_some(rows)
    })
    .await;
    assert_eq!(
        rita,
        vec![(
            "rita".to_owned(),
            Some("missing permission worker:execute".to_owned()),
            "grpc WorkerService/AppendStepLogs".to_owned()
        )]
    );
}

/// A run of globex, the platform's second tenant, whose admin is `bob`.
async fn foreign_run(p: &Platform) -> String {
    let workspace = p.api.create_workspace("bob", "globex", "foreign").await;
    p.api
        .create_namespace("bob", "globex", &workspace, NAMESPACE, "dev")
        .await;
    let call = |method: Method, path: &str, body: serde_json::Value| {
        p.api
            .as_user(method, path, "bob", "globex")
            .header("x-octopus-namespace", NAMESPACE)
            .header("idempotency-key", format!("key-{}", uuid::Uuid::now_v7()))
            .json(&body)
            .send()
    };
    let created: serde_json::Value = call(
        Method::POST,
        "/workflows",
        json!({"name": "foreign", "ir": single_ir("foreign", None)}),
    )
    .await
    .expect("create")
    .json()
    .await
    .expect("json");
    let workflow = created["id"].as_str().expect("a workflow id").to_owned();
    let published = call(
        Method::POST,
        &format!("/workflows/{workflow}/publish"),
        json!({"draftRevision": 1}),
    )
    .await
    .expect("publish");
    assert_eq!(published.status(), 201);
    let run: serde_json::Value = call(
        Method::POST,
        "/runs",
        json!({"workflowId": workflow, "input": {}}),
    )
    .await
    .expect("start")
    .json()
    .await
    .expect("json");
    run["runId"].as_str().expect("a run id").to_owned()
}

/// The security review of task T041-7, Minor 6: a batch naming another tenant's run is answered exactly as one naming
/// a run that does not exist (`STOP_ATTEMPT`, `LEASE`, nothing stored), and only the first is audited as a
/// cross-tenant access.
#[tokio::test]
async fn a_batch_naming_another_tenants_run_is_answered_as_a_missing_run_and_audited() {
    let p = platform().await;
    let (mut worker, _, _) = registered(&p, "wanda", "1.7").await;
    let lease = leased(&p, &mut worker).await;
    let foreign = foreign_run(&p).await;
    let missing = uuid::Uuid::now_v7().to_string();
    let mut answers = Vec::new();
    for run in [&foreign, &missing] {
        let mut message = batch(&worker, &lease, &[1]);
        message.task_id = format!("{run}/a/{}", lease.generation);
        message.run_id = run.clone();
        answers.push(append(&mut worker, message).await.expect("a verdict"));
    }
    assert_eq!(answers[0], answers[1], "the same answer for both");
    assert_eq!(answers[0].verdict, i32::from(StepLogVerdict::StopAttempt));
    assert_eq!(answers[0].stop_reason, i32::from(StepLogStopReason::Lease));
    assert_eq!(answers[0].stored_through_sequence, 0);
    let audited = |run: String| {
        let pool = p.api.db.pool.clone();
        async move {
            sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM audit_records \
                 WHERE action = 'access.denied.cross_tenant' AND resource_id = $1",
            )
            .bind(run)
            .fetch_one(&pool)
            .await
            .expect("the cross-tenant audit")
        }
    };
    crate::support::eventually("the cross-tenant audit", || {
        let count = audited(foreign.clone());
        async move { (count.await == 1).then_some(()) }
    })
    .await;
    assert_eq!(
        audited(missing.clone()).await,
        0,
        "a missing run is not audited"
    );
    assert_eq!(stored_lines(&p, &lease.run_id).await, 0);
}

/// Decision D041-150 (the security review of task T041-7, Major 1): step-log batches are charged to rate budgets of
/// their own, so a worker whose batches spend their budget still has the budget its heartbeats and reports need; and
/// the batches' budget is enforced.
#[tokio::test]
async fn batches_spend_a_budget_of_their_own_and_never_the_one_heartbeats_need() {
    let p = platform_with(Options {
        subject_rate_limit: RateLimitConfig {
            per_second: 0.01,
            burst: 8.0,
        },
        ..Options::default()
    })
    .await;
    // Two of the worker's eight shared tokens: the registration and the poll.
    let (mut worker, _, _) = registered(&p, "wanda", "1.7").await;
    let lease = leased(&p, &mut worker).await;
    for sequence in 1..=6 {
        let answer = append_batch(&mut worker, &lease, &[sequence])
            .await
            .expect("within the batches' budget");
        assert_eq!(answer.verdict, i32::from(StepLogVerdict::Continue));
    }
    let request = worker.req(worker.heartbeat_message(&lease));
    worker
        .client
        .heartbeat(request)
        .await
        .expect("the heartbeat keeps its budget whatever the batches spent");
    for sequence in 7..=8 {
        append_batch(&mut worker, &lease, &[sequence])
            .await
            .expect("the last two of the batches' eight");
    }
    let shed = append_batch(&mut worker, &lease, &[9])
        .await
        .expect_err("the batches' own budget is spent");
    assert_eq!(shed.code(), Code::ResourceExhausted, "{shed:?}");
}

/// The correctness review of task T041-7, Minor 1: a call refused by P1 is answered without waiting for its audit, so
/// it never holds its place in the Logs class while the audit gate is slow; the record is still written.
#[tokio::test]
async fn a_refused_call_is_answered_without_waiting_for_its_audit_which_is_still_written() {
    let p = platform().await;
    let (mut worker, _, _) = registered(&p, "wanda", "1.7").await;
    let lease = leased(&p, &mut worker).await;
    p.grant(p.tenant, "rita", Role::Runner).await;
    let runner = p.connect("rita", TENANT).await;
    let mut runner_client = runner.client.clone();
    let refused = runner.req(batch(&worker, &lease, &[1]));
    let (locker, pid) = hold_the_audit_trail(&p).await;
    let status = runner_client
        .append_step_logs(refused)
        .await
        .expect_err("a runner token is refused");
    assert_eq!(status.code(), Code::PermissionDenied, "{status:?}");
    // The answer came first and the audit still waits on the held trail (before the fix the answer waited for the
    // audit, which gave up after its budget and no longer waits).
    blocked_on(&p, pid, 1).await;
    locker.commit().await.expect("release the trail");
    let rows = crate::support::eventually("the runner's denial is recorded", || async {
        let rows = denials(&p).await;
        (!rows.is_empty()).then_some(rows)
    })
    .await;
    assert_eq!(
        rows,
        vec![(
            "rita".to_owned(),
            Some("missing permission worker:execute".to_owned()),
            "grpc WorkerService/AppendStepLogs".to_owned()
        )]
    );
}

/// The correctness review of task T041-7, Minor 4: an instance built without the ingest answers `UNIMPLEMENTED`, which
/// a worker treats as an older replica (contracts.md §4).
#[tokio::test]
async fn an_instance_without_the_ingest_answers_unimplemented() {
    let p = platform_with(Options {
        step_logs_served: false,
        ..Options::default()
    })
    .await;
    let (mut worker, _, _) = registered(&p, "wanda", "1.7").await;
    let lease = leased(&p, &mut worker).await;
    let status = append_batch(&mut worker, &lease, &[1])
        .await
        .expect_err("not served");
    assert_eq!(status.code(), Code::Unimplemented, "{status:?}");
}
