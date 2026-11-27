//! Design 041, task T041-8: the server's fault and security cases at the gRPC boundary. A lost answer goes through the
//! proxy of `tests/support/worker_proxy.rs` (its `lose_append_ack` fault); the cross-tenant audit, the per-tenant
//! tickets and a restart of the API go through the real worker service. Every wait is on a barrier or a held lock,
//! seen in `pg_stat_activity`, never on time.

use std::sync::atomic::Ordering;

use reqwest::Method;
use sagat_api_grpc::pb::{StepLogStopReason, StepLogVerdict, TaskLease};
use sagat_domain::identity::{Role, TenantId};
use serde_json::json;

use crate::rpc::{
    append_batch, batch, blocked_on, foreign_run, hold_the_audit_trail, leased, registered,
    stored_lines,
};
use crate::support::api::Options;
use crate::support::worker::{
    NAMESPACE, Platform, TENANT, Worker, platform, platform_with, single_ir,
};

use crate::worker_proxy as proxy;

/// A crash between the commit and the answer, at the gRPC boundary: the proxy forwards the batch and drops its answer;
/// the worker's resend of the same batch stores nothing more and is answered `CONTINUE` through the batch.
#[tokio::test]
async fn a_lost_answer_through_the_rpc_is_resent_and_stores_nothing_more() {
    let p = platform().await;
    let proxy = proxy::start(&p.api.grpc_url).await;
    let mut worker = p.worker_at(&proxy.url, "wanda").await;
    let mut message = Worker::register_message(&["act.a"], 10);
    message
        .capabilities
        .as_mut()
        .expect("capabilities")
        .protocol_version = "1.7".to_owned();
    let answer = worker
        .client
        .register_worker(worker.req(message))
        .await
        .expect("register through the proxy")
        .into_inner();
    assert_eq!(answer.accepted_protocol_version, "1.7");
    worker.worker_id = answer.worker_id;
    worker.session = answer.session_token;
    let lease = leased(&p, &mut worker).await;
    proxy.faults.lose_append_ack.store(true, Ordering::SeqCst);
    let lost = append_batch(&mut worker, &lease, &[1, 2])
        .await
        .expect_err("the answer is lost");
    assert_eq!(lost.code(), tonic::Code::Unavailable);
    assert_eq!(proxy.faults.lost_appends.load(Ordering::SeqCst), 1);
    assert_eq!(
        stored_lines(&p, &lease.run_id).await,
        2,
        "the batch was committed"
    );
    let resent = append_batch(&mut worker, &lease, &[1, 2])
        .await
        .expect("the resend is answered");
    assert_eq!(resent.verdict, i32::from(StepLogVerdict::Continue));
    assert_eq!(resent.stored_through_sequence, 2);
    assert_eq!(
        stored_lines(&p, &lease.run_id).await,
        2,
        "nothing more is stored"
    );
}

/// The first backend waiting on the holder `pid`, by its process and the start of its statement (`pg_stat_activity`).
async fn first_waiter(p: &Platform, pid: i32) -> Option<(i32, String)> {
    sqlx::query_as(
        "SELECT pid, query_start::text FROM pg_stat_activity \
          WHERE $1 = ANY(pg_blocking_pids(pid)) ORDER BY query_start LIMIT 1",
    )
    .bind(pid)
    .fetch_optional(&p.api.db.pool)
    .await
    .expect("pg_stat_activity")
}

/// The cross-tenant audit (decision D041-150's suspicion, recorded for this task): a batch naming another tenant's run
/// is answered `LEASE` while its audit writes still wait on the held trails, so its answer takes no longer than a
/// missing run's (the writes are off the answer's path, as for every worker call that reaches a foreign run); once the
/// trails are released the records are written, outside the batch's rolled-back transaction, and kept: the caller's
/// tenant gets the record a missing run leaves, and the platform's security audit the probe (feature 009, D60;
/// D041-155).
#[tokio::test]
async fn a_batch_for_another_tenants_run_is_answered_before_its_audit_is_written_and_the_audit_is_kept()
 {
    let p = platform().await;
    let (mut worker, _, _) = registered(&p, "wanda", "1.7").await;
    let lease = leased(&p, &mut worker).await;
    let foreign = foreign_run(&p).await;
    let mut message = batch(&worker, &lease, &[1]);
    message.task_id = format!("{foreign}/a/{}", lease.generation);
    message.run_id = foreign.clone();
    let (locker, pid) = hold_the_audit_trail(&p).await;
    let mut client = worker.client.clone();
    let request = worker.req(message);
    let call = tokio::spawn(async move { client.append_step_logs(request).await });
    // The audit write of the batch waits on the held trail; the answer then arrives while that same write still waits.
    // An answer that waited for its audit write would come only once that write gave up (the pool's statement timeout
    // of 10 s), so the write waiting afterwards would be another one, or none.
    let write = crate::support::eventually("the audit write waits on the held trail", || {
        first_waiter(&p, pid)
    })
    .await;
    let answer =
        crate::support::bounded::within("the answer", std::time::Duration::from_secs(30), call)
            .await
            .expect("the call")
            .expect("a verdict")
            .into_inner();
    assert_eq!(
        first_waiter(&p, pid).await,
        Some(write),
        "the answer came while its audit write still waited"
    );
    assert_eq!(answer.verdict, i32::from(StepLogVerdict::StopAttempt));
    assert_eq!(answer.stop_reason, i32::from(StepLogStopReason::Lease));
    // The caller's record and the probe both wait on the held trails.
    blocked_on(&p, pid, 2).await;
    locker.commit().await.expect("release the trails");
    let trail = crate::support::eventually("the caller's record is written", || async {
        let trail = crate::support::trail_of(&p.api.db.pool, p.tenant, "wanda").await;
        trail
            .iter()
            .any(|row| row.0 == "access.denied.not_found")
            .then_some(trail)
    })
    .await;
    assert!(
        trail.iter().all(|row| !row.2.contains(&foreign)),
        "the foreign run is not on the caller's trail: {trail:?}"
    );
    let probes = crate::support::eventually("the probe is written", || async {
        let probes = crate::support::probes_of(&p.api.db.pool, p.tenant).await;
        (!probes.is_empty()).then_some(probes)
    })
    .await;
    assert_eq!(
        probes,
        vec![(
            "wanda".to_owned(),
            "run".to_owned(),
            foreign.clone(),
            Some(p.other_tenant.as_uuid())
        )],
        "filed in the platform's security audit against the caller, with the run's tenant"
    );
    assert_eq!(stored_lines(&p, &lease.run_id).await, 0);
}

/// A worker of `tenant` (by its slug and admin) with a leased step of a one-activity run in that tenant's namespace,
/// registered at 1.7; the tenant's workspace and namespace are created on the way, unless `tenant` is acme.
async fn tenant_worker<'a>(
    p: &'a Platform,
    tenant: (TenantId, &str, &str),
    sub: &str,
) -> (Worker<'a>, TaskLease) {
    let (tenant_id, slug, admin) = tenant;
    if slug != TENANT {
        let workspace = p.api.create_workspace(admin, slug, "flood").await;
        p.api
            .create_namespace(admin, slug, &workspace, NAMESPACE, "dev")
            .await;
    }
    p.grant(tenant_id, sub, Role::Worker).await;
    sqlx::query(
        "INSERT INTO worker_bindings (tenant_id, namespace_id, subject, pattern, updated_by) \
         SELECT n.tenant_id, n.id, $2, '*', 'test' FROM namespaces n WHERE n.tenant_id = $1 AND n.slug = $3",
    )
    .bind(tenant_id.as_uuid())
    .bind(sub)
    .bind(NAMESPACE)
    .execute(&p.api.db.pool)
    .await
    .expect("bind the worker");
    let mut worker = p.connect(sub, slug).await;
    let mut message = Worker::register_message(&["act.a"], 10);
    message
        .capabilities
        .as_mut()
        .expect("capabilities")
        .protocol_version = "1.7".to_owned();
    let answer = worker
        .client
        .register_worker(worker.req(message))
        .await
        .expect("register")
        .into_inner();
    worker.worker_id = answer.worker_id;
    worker.session = answer.session_token;
    let call = |method: Method, path: &str, body: serde_json::Value| {
        p.api
            .as_user(method, path, admin, slug)
            .header("x-octopus-namespace", NAMESPACE)
            .header("idempotency-key", format!("key-{}", uuid::Uuid::now_v7()))
            .json(&body)
            .send()
    };
    let name = format!("flood-{}", uuid::Uuid::now_v7().simple());
    let created: serde_json::Value = call(
        Method::POST,
        "/workflows",
        json!({"name": name, "ir": single_ir(&name, None)}),
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
    let started = call(
        Method::POST,
        "/runs",
        json!({"workflowId": workflow, "input": {}}),
    )
    .await
    .expect("start");
    assert!(started.status().is_success(), "{}", started.status());
    let lease = worker.one_task().await;
    (worker, lease)
}

/// Two tenants that flood and a third whose batches are still admitted (design section 2, P3): each tenant holds at
/// most its tickets (2 here, of a Logs class of 5), so two tenants that fill theirs, their batches held in the handler
/// while each waits for the audit of its `AUTHORITY` answer on a held `audit_records`, leave the class room for a third
/// tenant, whose batch is stored at once; a flooding tenant's next batch finds no ticket and is told to retry.
#[tokio::test]
async fn two_flooding_tenants_leave_a_third_tenants_batches_admitted() {
    let p = platform_with(Options {
        step_log_tenant_calls: 2,
        ..Options::default()
    })
    .await;
    let initech = p.api.tenant("initech", "carol").await;
    let (mut quiet, quiet_lease) = tenant_worker(&p, (p.tenant, TENANT, "alice"), "wanda").await;
    let (globex_worker, globex_lease) =
        tenant_worker(&p, (p.other_tenant, "globex", "bob"), "gwen").await;
    let (initech_worker, initech_lease) =
        tenant_worker(&p, (initech, "initech", "carol"), "ivan").await;
    for (worker, slug) in [(&globex_worker, "globex"), (&initech_worker, "initech")] {
        sqlx::query(
            "DELETE FROM worker_bindings WHERE subject = $1 \
             AND tenant_id = (SELECT id FROM tenants WHERE slug = $2)",
        )
        .bind(&worker.subject)
        .bind(slug)
        .execute(&p.api.db.pool)
        .await
        .expect("remove the flooder's binding");
    }
    let (locker, pid) = hold_the_audit_trail(&p).await;
    let mut flooding = Vec::new();
    for (worker, lease) in [
        (&globex_worker, &globex_lease),
        (&initech_worker, &initech_lease),
    ] {
        for sequence in 1..=2 {
            let mut client = worker.client.clone();
            let request = worker.req(batch(worker, lease, &[sequence]));
            flooding.push(tokio::spawn(async move {
                client.append_step_logs(request).await
            }));
        }
    }
    // The four flooding batches hold their tickets, each waiting for its audit on the held trail.
    blocked_on(&p, pid, 4).await;
    for (worker, lease) in [
        (&globex_worker, &globex_lease),
        (&initech_worker, &initech_lease),
    ] {
        let mut client = worker.client.clone();
        let refused = client
            .append_step_logs(worker.req(batch(worker, lease, &[3])))
            .await
            .expect("a verdict")
            .into_inner();
        assert_eq!(
            (refused.verdict, refused.retry_after_ms),
            (i32::from(StepLogVerdict::RetryLater), 200),
            "{} has no ticket left",
            worker.tenant
        );
    }
    let admitted = append_batch(&mut quiet, &quiet_lease, &[1])
        .await
        .expect("the third tenant is admitted");
    assert_eq!(admitted.verdict, i32::from(StepLogVerdict::Continue));
    assert_eq!(admitted.stored_through_sequence, 1);
    assert_eq!(stored_lines(&p, &quiet_lease.run_id).await, 1);
    locker.commit().await.expect("release the trail");
    for call in flooding {
        let answer = call
            .await
            .expect("the call")
            .expect("a verdict")
            .into_inner();
        assert_eq!(answer.stop_reason, i32::from(StepLogStopReason::Authority));
    }
}

/// A restart of the API in the middle of an attempt: a real engine stores the attempt's first batch and stops; a new
/// engine on the same database (no state carried over in memory) stores the next batch, which repeats the first's
/// records and adds one: the repeated ones are not stored again.
#[tokio::test]
async fn a_restart_of_the_api_in_the_middle_of_an_attempt_stores_the_next_batch_once() {
    use crate::support::worker::{RunningEngine, engine_env};
    let p = platform().await;
    let mut first_engine = RunningEngine::start(engine_env(&p.api.db, &p.api.idp, &[])).await;
    let mut worker = p.worker_at(&first_engine.grpc_url, "wanda").await;
    let mut message = Worker::register_message(&["act.a"], 10);
    message
        .capabilities
        .as_mut()
        .expect("capabilities")
        .protocol_version = "1.7".to_owned();
    let answer = worker
        .client
        .register_worker(worker.req(message))
        .await
        .expect("register with the first engine")
        .into_inner();
    assert_eq!(answer.accepted_protocol_version, "1.7");
    worker.worker_id = answer.worker_id;
    worker.session = answer.session_token;
    let lease = leased(&p, &mut worker).await;
    let stored = append_batch(&mut worker, &lease, &[1, 2])
        .await
        .expect("the first engine stores the batch");
    assert_eq!(stored.stored_through_sequence, 2);
    first_engine.finish().await.expect("the first engine stops");
    let second_engine = RunningEngine::start(engine_env(&p.api.db, &p.api.idp, &[])).await;
    let mut after = p
        .connect_at(second_engine.grpc_url.clone(), "wanda", TENANT)
        .await;
    after.worker_id = worker.worker_id.clone();
    after.session = worker.session.clone();
    let next = append_batch(&mut after, &lease, &[1, 2, 3])
        .await
        .expect("the new engine answers");
    assert_eq!(next.verdict, i32::from(StepLogVerdict::Continue));
    assert_eq!(next.stored_through_sequence, 3);
    assert_eq!(stored_lines(&p, &lease.run_id).await, 3, "no duplicate");
    // The stream's counters and the tenant's usage carry across the restart: the usage is exactly the stored rows'
    // charge (their text plus 128 a row), the first engine's quota lease, whose backend is gone, included.
    let (records, last_sequence): (i32, i64) = sqlx::query_as(
        "SELECT records, last_sequence FROM step_log_streams WHERE run_id = $1::uuid",
    )
    .bind(&lease.run_id)
    .fetch_one(&p.api.db.pool)
    .await
    .expect("the stream");
    assert_eq!((records, last_sequence), (3, 3));
    let (usage, charged): (i64, i64) = sqlx::query_as(
        "SELECT sagat_quota_step_log_usage($1), \
                (SELECT sum(octet_length(message) + coalesce(octet_length(exception), 0) \
                            + coalesce(octet_length(fields), 0) + 128)::bigint \
                   FROM step_log_lines WHERE run_id = $2::uuid)",
    )
    .bind(p.tenant.as_uuid())
    .bind(&lease.run_id)
    .fetch_one(&p.api.db.pool)
    .await
    .expect("the usage");
    assert_eq!(usage, charged);
}

/// The server canary through the handler (the security review of task T041-8, finding 1): a connection string whose
/// password is the canary, sent whole by a client that skips its own redaction in the message, the exception, a field
/// key, a field value and alone under a sensitive name. The canary is in neither the wire answer nor the captured
/// `tracing` output of the call (the handler's span and request parsing included), and the stored line is scrubbed.
#[tokio::test]
async fn the_server_canary_reaches_neither_the_answer_nor_the_trace_of_the_rpc() {
    use sagat_api_grpc::pb::{StepLogField, step_log_field};
    let p = platform().await;
    let (mut worker, _, _) = registered(&p, "wanda", "1.7").await;
    let lease = leased(&p, &mut worker).await;
    let canary = format!("cnry{}", uuid::Uuid::now_v7().simple());
    let value = format!("postgres://svc:{canary}@db.internal/app");
    let mut message = batch(&worker, &lease, &[1]);
    let record = &mut message.records[0];
    record.message = format!("connecting with {value}");
    record.exception = format!("ConnectionError: {value} refused");
    record.fields = vec![
        StepLogField {
            key: "target".into(),
            value: Some(step_log_field::Value::StringValue(format!("dsn {value}"))),
        },
        StepLogField {
            key: format!("dsn {value}"),
            value: Some(step_log_field::Value::IntValue(1)),
        },
        StepLogField {
            key: "password".into(),
            value: Some(step_log_field::Value::StringValue(canary.clone())),
        },
    ];
    let (guard, captured) = crate::support::logs::capture_with_spans(tracing::Level::TRACE);
    let answer = crate::rpc::append(&mut worker, message).await;
    drop(guard);
    let answer = answer.expect("a verdict");
    assert_eq!(answer.verdict, i32::from(StepLogVerdict::Continue));
    let stored: String = sqlx::query_scalar(
        "SELECT coalesce(string_agg(l::text, E'\\n'), '') FROM step_log_lines l WHERE run_id = $1::uuid",
    )
    .bind(&lease.run_id)
    .fetch_one(&p.api.db.pool)
    .await
    .expect("the stored line");
    assert!(
        stored.contains("[REDACTED]"),
        "the line is stored, scrubbed"
    );
    for (surface, text) in [
        ("the wire answer", format!("{answer:?}")),
        ("the stored line", stored),
        ("the captured tracing output", captured.text()),
    ] {
        assert!(!text.contains(&canary), "the canary reached {surface}");
    }
}

/// A database error that quotes the bound text, through the handler (the security review of task T041-8, finding 1): a
/// `BEFORE INSERT` trigger on `step_log_lines` raises with `NEW.message`; the call fails, and the text is in neither the
/// status the worker gets nor the captured `tracing` output of the call.
#[tokio::test]
async fn a_database_error_that_quotes_the_bound_text_reaches_neither_the_status_nor_the_trace_of_the_rpc()
 {
    let p = platform().await;
    for statement in [
        "CREATE FUNCTION test_quote_the_line() RETURNS trigger LANGUAGE plpgsql AS \
         $$ BEGIN RAISE EXCEPTION 'refused line: %', NEW.message; END $$",
        "CREATE TRIGGER test_quote_the_line BEFORE INSERT ON step_log_lines \
         FOR EACH ROW EXECUTE FUNCTION test_quote_the_line()",
    ] {
        sqlx::query(statement)
            .execute(&p.api.db.pool)
            .await
            .expect("the quoting trigger");
    }
    let (mut worker, _, _) = registered(&p, "wanda", "1.7").await;
    let lease = leased(&p, &mut worker).await;
    let text = format!("bound-{}", uuid::Uuid::now_v7().simple());
    let mut message = batch(&worker, &lease, &[1]);
    message.records[0].message.clone_from(&text);
    let (guard, captured) = crate::support::logs::capture_with_spans(tracing::Level::TRACE);
    let status = crate::rpc::append(&mut worker, message).await;
    drop(guard);
    let status = status.expect_err("the insert is refused");
    for (surface, shown) in [
        ("the status's message", status.message().to_owned()),
        ("the status's debug form", format!("{status:?}")),
        ("the captured tracing output", captured.text()),
    ] {
        assert!(!shown.contains(&text), "the bound text reached {surface}");
    }
    assert_eq!(stored_lines(&p, &lease.run_id).await, 0);
}
