use std::time::{Duration, SystemTime, UNIX_EPOCH};

use reqwest::Method;
use sagat_api_http::RateLimitConfig;
use sagat_application::worker::WorkerSettings;
use sagat_domain::identity::Role;
use serde_json::json;
use tonic::{Code, Status};

use crate::support::api::Options;
use crate::support::worker::{
    TENANT, Worker, chain_ir, machine_code, platform, platform_with, single_ir,
};

fn now_ms() -> i64 {
    i64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_millis(),
    )
    .expect("fits")
}

fn assert_code(result: Result<impl std::fmt::Debug, Status>, code: Code, machine: &str) {
    let status = result.expect_err("the call must be refused");
    assert_eq!(status.code(), code, "{status:?}");
    assert_eq!(machine_code(&status), machine, "{status:?}");
}

#[tokio::test]
async fn a_worker_executes_a_workflow_through_every_rpc() {
    let p = platform().await;
    let wf = p.publish("chain-flow", &chain_ir("chain-flow")).await;
    let run = p.start(&wf, &json!({"orderId": "o-1"})).await;
    let mut w = p.worker("wendy").await;
    w.register(&["act.a", "act.b"]).await;

    let lease = w.one_task().await;
    assert_eq!(lease.activity_name, "act.a");
    assert_eq!(lease.run_id, run);
    assert_eq!(lease.attempt, 1);
    assert!(lease.generation >= 1);
    assert_eq!(lease.task_id, format!("{run}/a/{}", lease.generation));
    assert_eq!(lease.lease_token.len(), 64);
    assert!(lease.expires_at_epoch_ms > now_ms());
    assert!(lease.input_json.contains("o-1"), "{}", lease.input_json);
    assert!(lease.input_blob_ref.is_empty());
    assert!(!lease.idempotency_key.is_empty());
    // The trace of the request that started the run continues into the worker.
    assert!(
        lease.traceparent.starts_with("00-"),
        "{}",
        lease.traceparent
    );
    assert_eq!(lease.traceparent.len(), 55);

    let beat = w
        .client
        .heartbeat(w.req(w.heartbeat_message(&lease)))
        .await
        .expect("heartbeat")
        .into_inner();
    assert!(beat.lease_expires_at_epoch_ms >= lease.expires_at_epoch_ms);
    assert!(!beat.cancel_requested);

    let done = w
        .complete(&lease, &json!({"charged": true}))
        .await
        .expect("complete a");
    assert_eq!(done.step_status, "SUCCEEDED");
    assert_eq!(done.run_status, "RUNNING");
    assert!(!done.already_applied);
    assert!(!done.command_id.is_empty());
    assert_eq!(p.status(&run).await, "RUNNING");

    let next = w.one_task().await;
    assert_eq!(next.activity_name, "act.b");
    assert!(
        next.input_json.contains("charged"),
        "the output of A is visible to B: {}",
        next.input_json
    );
    let finished = w.complete(&next, &json!({})).await.expect("complete b");
    assert_eq!(finished.run_status, "SUCCEEDED");
    assert_eq!(p.status(&run).await, "SUCCEEDED");
}

#[tokio::test]
async fn nothing_to_do_is_an_empty_answer_with_a_retry_hint_not_an_error() {
    let p = platform().await;
    let mut w = p.worker("wendy").await;
    w.register(&["act.a"]).await;
    let request = w.req(w.poll_message(4));
    let answer = w
        .client
        .poll_tasks(request)
        .await
        .expect("poll")
        .into_inner();
    assert!(answer.tasks.is_empty());
    assert_eq!(answer.retry_after_ms, 500);
}

async fn register_stream_worker(worker: &mut Worker<'_>, max_inflight: i32) {
    let mut message = Worker::register_message(&["act.a"], max_inflight);
    message
        .capabilities
        .as_mut()
        .expect("capabilities")
        .protocol_version = "1.4".into();
    let registered = worker
        .client
        .register_worker(worker.req(message))
        .await
        .expect("register 1.4")
        .into_inner();
    assert_eq!(registered.accepted_protocol_version, "1.4");
    worker.worker_id = registered.worker_id;
    worker.session = registered.session_token;
}

#[tokio::test]
async fn a_task_stream_spends_credit_across_successive_frames() {
    let p = platform().await;
    let wf = p
        .publish("stream-waves", &single_ir("stream-waves", None))
        .await;
    let mut w = p.worker("wendy").await;
    register_stream_worker(&mut w, 2).await;
    let response = w
        .client
        .stream_tasks(w.req(w.poll_message(2)))
        .await
        .expect("open stream");
    let mut stream = response.into_inner();

    let first_run = p.start(&wf, &json!({"wave": 1})).await;
    let first = stream
        .message()
        .await
        .expect("first frame")
        .expect("first frame present");
    assert_eq!(first.tasks.len(), 1);
    assert_eq!(first.tasks[0].run_id, first_run);

    let second_run = p.start(&wf, &json!({"wave": 2})).await;
    let second = stream
        .message()
        .await
        .expect("second frame")
        .expect("second frame present");
    assert_eq!(second.tasks.len(), 1);
    assert_eq!(second.tasks[0].run_id, second_run);
    assert!(stream.message().await.expect("filled eof").is_none());
}

#[tokio::test]
async fn task_stream_byte_credit_is_cumulative_across_frames() {
    let p = platform().await;
    let wf = p
        .publish("stream-byte-credit", &single_ir("stream-byte-credit", None))
        .await;
    let mut all_runs = Vec::new();
    for index in 0..40 {
        all_runs.push(
            p.start(&wf, &json!({"index": index, "large": "x".repeat(220_000)}))
                .await,
        );
    }
    // Admit both waves before the finite stream window starts. Keep the second wave behind a
    // real run lock so the first frame cannot consume it, without thirty REST calls while the
    // response body is deliberately unread.
    let second_wave = all_runs[10..]
        .iter()
        .map(|run| uuid::Uuid::parse_str(run).expect("run id"))
        .collect::<Vec<_>>();
    let mut wave_guard = p.api.db.pool.begin().await.expect("second-wave barrier");
    sqlx::query("SELECT id FROM runs WHERE tenant_id=$1 AND id=ANY($2) FOR UPDATE")
        .bind(p.tenant.as_uuid())
        .bind(&second_wave)
        .fetch_all(&mut *wave_guard)
        .await
        .expect("hold the second wave");
    let mut w = p.worker("wendy").await;
    register_stream_worker(&mut w, 64).await;
    let mut stream = w
        .client
        .stream_tasks(w.req(w.poll_message(64)))
        .await
        .expect("open stream")
        .into_inner();
    let first = stream
        .message()
        .await
        .expect("first byte frame")
        .expect("first byte frame present");
    assert!(!first.tasks.is_empty());
    assert!(
        first
            .tasks
            .iter()
            .all(|task| all_runs[..10].contains(&task.run_id))
    );

    wave_guard
        .rollback()
        .await
        .expect("release the second wave");
    let mut frames = 1;
    let mut bytes = first
        .tasks
        .iter()
        .map(|task| task.input_json.len())
        .sum::<usize>();
    let mut smallest_task = first
        .tasks
        .iter()
        .map(|task| task.input_json.len())
        .min()
        .expect("first frame has tasks");
    let mut delivered = first.tasks.len();
    let mut terminal = 0;
    while let Some(frame) = stream.message().await.expect("byte-budget frame") {
        if frame.tasks.is_empty() {
            terminal += 1;
            assert!(frame.retry_after_ms > 0);
        } else {
            frames += 1;
            delivered += frame.tasks.len();
            bytes += frame
                .tasks
                .iter()
                .map(|task| task.input_json.len())
                .sum::<usize>();
            smallest_task = smallest_task.min(
                frame
                    .tasks
                    .iter()
                    .map(|task| task.input_json.len())
                    .min()
                    .expect("data frame has tasks"),
            );
        }
    }
    assert!(frames >= 2, "the byte budget was spent across frames");
    assert!(
        bytes <= sagat_application::worker::MAX_POLL_BYTES,
        "the aggregate receive window exceeded its byte credit"
    );
    assert!(
        sagat_application::worker::MAX_POLL_BYTES - bytes < smallest_task,
        "the unused credit must be too small for even the smallest task"
    );
    assert_eq!(terminal, 1, "byte exhaustion has one terminal hint");
    assert!(
        delivered < all_runs.len(),
        "one task must not fit the remainder"
    );

    let run_ids = all_runs
        .iter()
        .map(|run| uuid::Uuid::parse_str(run).expect("run id"))
        .collect::<Vec<_>>();
    let untouched: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM steps WHERE tenant_id = $1 AND run_id = ANY($2) \
         AND node = 'a' AND status = 'READY' AND attempt = 0 AND leased_by IS NULL",
    )
    .bind(p.tenant.as_uuid())
    .bind(&run_ids)
    .fetch_one(&p.api.db.pool)
    .await
    .expect("unclaimed remainder");
    assert!(
        untouched > 0,
        "a task too large for the remainder stays ready"
    );
}

#[tokio::test]
async fn unary_and_stream_claims_share_the_exact_session_limit() {
    let p = platform().await;
    let wf = p
        .publish("mixed-claims", &single_ir("mixed-claims", None))
        .await;
    for index in 0..12 {
        p.start(&wf, &json!({"index": index})).await;
    }
    let mut w = p.worker("wendy").await;
    register_stream_worker(&mut w, 2).await;
    let mut stream_client = w.client.clone();
    let mut poll_client = w.client.clone();
    let stream_request = w.req(w.poll_message(8));
    let poll_request = w.req(w.poll_message(8));
    let (stream, poll) = tokio::join!(
        stream_client.stream_tasks(stream_request),
        poll_client.poll_tasks(poll_request)
    );
    let mut stream = stream.expect("stream call").into_inner();
    let polled = poll.expect("unary poll").into_inner();
    let mut streamed = 0;
    let mut terminal_frames = 0;
    while let Some(frame) = stream.message().await.expect("stream frame") {
        if frame.tasks.is_empty() {
            terminal_frames += 1;
            assert!(frame.retry_after_ms > 0);
        } else {
            streamed += frame.tasks.len();
        }
    }
    assert_eq!(
        streamed + polled.tasks.len(),
        2,
        "mixed transports cannot race past the durable session cap"
    );
    assert_eq!(terminal_frames, 1, "one underfilled terminal hint");
    let active: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM steps WHERE tenant_id = $1 \
         AND status IN ('LEASED', 'RUNNING') AND leased_by = $2",
    )
    .bind(p.tenant.as_uuid())
    .bind(&w.worker_id)
    .fetch_one(&p.api.db.pool)
    .await
    .expect("active leases");
    assert_eq!(active, 2, "no later stream claim overshot the cap");
}

#[tokio::test]
async fn task_stream_credit_is_validated_and_old_sessions_cannot_lease() {
    let p = platform().await;
    let wf = p
        .publish("old-stream", &single_ir("old-stream", None))
        .await;
    p.start(&wf, &json!({})).await;
    let mut old = p.worker("old-worker").await;
    old.register(&["act.a"]).await;
    let refused = old.client.stream_tasks(old.req(old.poll_message(1))).await;
    assert_code(refused, Code::InvalidArgument, "UNSUPPORTED_PROTOCOL");
    assert_eq!(old.poll(1).await.len(), 1, "stream refusal leased nothing");

    let mut current = p.worker("current-worker").await;
    register_stream_worker(&mut current, 4).await;
    for credit in [0, 65, -1] {
        let status = current
            .client
            .stream_tasks(current.req(current.poll_message(credit)))
            .await
            .expect_err("invalid stream credit");
        assert_eq!(status.code(), Code::InvalidArgument);
    }
}

#[tokio::test]
async fn task_stream_lifetime_admission_survives_headers_and_is_released_on_drop() {
    let p = platform().await;
    let wf = p
        .publish("stalled-streams", &single_ir("stalled-streams", None))
        .await;
    for index in 0..2 {
        p.start(&wf, &json!({"index": index, "large": "x".repeat(180_000)}))
            .await;
    }
    let mut w = p.worker("wendy").await;
    register_stream_worker(&mut w, 4).await;
    let mut first_client = p.connect("wendy", TENANT).await;
    first_client.worker_id.clone_from(&w.worker_id);
    first_client.session.clone_from(&w.session);
    first_client.client = sagat_api_grpc::pb::worker_service_client::WorkerServiceClient::new(
        tonic::transport::Endpoint::from_shared(p.api.grpc_url.clone())
            .expect("gRPC endpoint")
            .initial_stream_window_size(0_u32)
            .connect()
            .await
            .expect("zero-window connection"),
    );
    let mut second_client = p.connect("wendy", TENANT).await;
    second_client.worker_id.clone_from(&w.worker_id);
    second_client.session.clone_from(&w.session);
    second_client.client = sagat_api_grpc::pb::worker_service_client::WorkerServiceClient::new(
        tonic::transport::Endpoint::from_shared(p.api.grpc_url.clone())
            .expect("gRPC endpoint")
            .initial_stream_window_size(0_u32)
            .connect()
            .await
            .expect("zero-window connection"),
    );
    let mut refused_client = p.connect("wendy", TENANT).await;
    refused_client.worker_id.clone_from(&w.worker_id);
    refused_client.session.clone_from(&w.session);

    let first = first_client
        .client
        .stream_tasks(first_client.req(first_client.poll_message(1)))
        .await
        .expect("first stream");
    let second = second_client
        .client
        .stream_tasks(second_client.req(second_client.poll_message(1)))
        .await
        .expect("second stream");
    let refused = refused_client
        .client
        .stream_tasks(refused_client.req(refused_client.poll_message(1)))
        .await
        .expect_err("the subject cap is held past response headers");
    assert_eq!(refused.code(), Code::ResourceExhausted);

    // Neither response body is read or dropped, and these two connections advertise a zero stream
    // window, so both server sends stop on flow-control capacity. The connection watchdog,
    // rather than another body poll, must tear them down and release the body-owned tickets.
    tokio::time::sleep(Duration::from_millis(5_100)).await;
    p.start(&wf, &json!({"index": 3, "large": "x".repeat(180_000)}))
        .await;
    let mut replacement_client = p.connect("wendy", TENANT).await;
    replacement_client.worker_id.clone_from(&w.worker_id);
    replacement_client.session.clone_from(&w.session);
    let replacement = replacement_client
        .client
        .stream_tasks(replacement_client.req(replacement_client.poll_message(1)))
        .await
        .expect("the transport watchdog releases the stalled bodies");
    let mut replacement = replacement.into_inner();
    let frame = replacement
        .message()
        .await
        .expect("replacement frame")
        .expect("replacement frame present");
    assert_eq!(frame.tasks.len(), 1);
    assert!(
        replacement
            .message()
            .await
            .expect("replacement eof")
            .is_none()
    );

    // Prepare delayed work outside either response window. A real run lock prevents its claim
    // until the old watchdog's deadline passes, without spending the remaining window on REST.
    let last_run = p
        .start(&wf, &json!({"index": 4, "large": "x".repeat(180_000)}))
        .await;
    let mut last_guard = p.api.db.pool.begin().await.expect("survivor work barrier");
    sqlx::query("SELECT id FROM runs WHERE tenant_id=$1 AND id=$2 FOR UPDATE")
        .bind(p.tenant.as_uuid())
        .bind(uuid::Uuid::parse_str(&last_run).expect("run id"))
        .fetch_one(&mut *last_guard)
        .await
        .expect("hold survivor work");

    // A normally consumed body cancels its watchdog. A second stream on the same HTTP/2
    // connection remains alive past the first stream's former deadline and receives later work.
    tokio::time::sleep(Duration::from_secs(1)).await;
    let mut survivor = replacement_client
        .client
        .stream_tasks(replacement_client.req(replacement_client.poll_message(1)))
        .await
        .expect("open stream on the same connection")
        .into_inner();
    tokio::time::sleep(Duration::from_millis(4_100)).await;
    last_guard.rollback().await.expect("release survivor work");
    let frame = survivor
        .message()
        .await
        .expect("surviving stream")
        .expect("surviving frame");
    assert_eq!(frame.tasks.len(), 1);
    assert_eq!(frame.tasks[0].run_id, last_run);
    drop((first, second));
}

#[tokio::test]
async fn an_underfilled_task_stream_ends_with_one_positive_retry_hint() {
    let p = platform().await;
    let mut w = p.worker("wendy").await;
    register_stream_worker(&mut w, 4).await;
    let mut stream = w
        .client
        .stream_tasks(w.req(w.poll_message(2)))
        .await
        .expect("open stream")
        .into_inner();
    let terminal = stream
        .message()
        .await
        .expect("terminal frame")
        .expect("terminal frame present");
    assert!(terminal.tasks.is_empty());
    assert!(terminal.retry_after_ms > 0);
    assert!(stream.message().await.expect("normal eof").is_none());
}

#[tokio::test]
async fn cancelling_while_a_claim_waits_rolls_back_before_the_fence_opens() {
    let p = platform().await;
    let wf = p
        .publish("cancel-mid-claim", &single_ir("cancel-mid-claim", None))
        .await;
    let run = p.start(&wf, &json!({})).await;
    let mut w = p.worker("wendy").await;
    register_stream_worker(&mut w, 4).await;

    let namespace = p.namespace_id().await;
    let drain_key = format!("drain:{}:{namespace}:wendy", p.tenant);
    let mut fence = p.api.db.pool.begin().await.expect("begin drain fence");
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(&drain_key)
        .execute(&mut *fence)
        .await
        .expect("hold drain fence");
    let fence_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *fence)
        .await
        .expect("fence backend");

    let mut cancelling = p.connect("wendy", TENANT).await;
    cancelling.worker_id.clone_from(&w.worker_id);
    cancelling.session.clone_from(&w.session);
    let stream = cancelling
        .client
        .stream_tasks(cancelling.req(cancelling.poll_message(1)))
        .await
        .expect("open stream")
        .into_inner();
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let blocked: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM pg_stat_activity waiter \
                 WHERE waiter.datname = current_database() \
                   AND $1 = ANY(pg_blocking_pids(waiter.pid)))",
            )
            .bind(fence_pid)
            .fetch_one(&p.api.db.pool)
            .await
            .expect("observe claim wait");
            if blocked {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("claim reached the drain fence");

    drop(stream);
    drop(cancelling);
    fence.commit().await.expect("release drain fence");
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let blocked: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM pg_stat_activity waiter \
                 WHERE waiter.datname = current_database() \
                   AND $1 = ANY(pg_blocking_pids(waiter.pid)))",
            )
            .bind(fence_pid)
            .fetch_one(&p.api.db.pool)
            .await
            .expect("observe cancellation");
            if !blocked {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("cancelled claim left the database wait");

    let projection: (String, i32, Option<String>) = sqlx::query_as(
        "SELECT status, attempt, leased_by FROM steps \
         WHERE tenant_id = $1 AND run_id = $2 AND node = 'a'",
    )
    .bind(p.tenant.as_uuid())
    .bind(uuid::Uuid::parse_str(&run).expect("run id"))
    .fetch_one(&p.api.db.pool)
    .await
    .expect("step projection");
    assert_eq!(projection, ("READY".into(), 0, None));

    let mut recovery = w
        .client
        .stream_tasks(w.req(w.poll_message(1)))
        .await
        .expect("recovery stream")
        .into_inner();
    let frame = recovery
        .message()
        .await
        .expect("recovery read")
        .expect("recovery frame");
    assert_eq!(frame.tasks.len(), 1);
    assert_eq!(frame.tasks[0].run_id, run);
}

#[tokio::test]
async fn an_activity_without_a_result_completes_with_an_empty_output() {
    let p = platform().await;
    let wf = p.publish("chain-flow", &chain_ir("chain-flow")).await;
    p.start(&wf, &json!({})).await;
    let mut w = p.worker("wendy").await;
    w.register(&["act.a", "act.b"]).await;
    let lease = w.one_task().await;
    let done = w
        .client
        .complete_step(w.req(Worker::complete_message(&lease, "")))
        .await
        .expect("an empty output means no value")
        .into_inner();
    assert_eq!(done.step_status, "SUCCEEDED");
    // The next step sees it as JSON null, not as garbage.
    let next = w.one_task().await;
    assert!(
        next.input_json.contains("\"output\":null"),
        "{}",
        next.input_json
    );
}

#[tokio::test]
async fn a_completion_is_idempotent_and_a_conflicting_replay_is_refused() {
    let p = platform().await;
    let wf = p.publish("single", &single_ir("single", None)).await;
    p.start(&wf, &json!({})).await;
    let mut w = p.worker("wendy").await;
    w.register(&["act.a"]).await;
    let lease = w.one_task().await;

    let first = w.complete(&lease, &json!({"v": 1})).await.expect("first");
    let replay = w.complete(&lease, &json!({"v": 1})).await.expect("replay");
    assert!(replay.already_applied);
    assert_eq!(replay.command_id, first.command_id);
    assert_eq!(replay.step_status, first.step_status);
    assert_eq!(replay.run_status, first.run_status);

    // The same key for a different result is a client bug, never silently accepted.
    assert_code(
        w.complete(&lease, &json!({"v": 2})).await,
        Code::Aborted,
        "IDEMPOTENCY_KEY_REUSED",
    );
    // A new key on a finished attempt is a stale report.
    let mut late = Worker::complete_message(&lease, "{\"v\":1}");
    late.idempotency_key = "another-key-after-finish".into();
    let result = w.client.complete_step(w.req(late)).await;
    assert_code(result, Code::Aborted, "STALE_ATTEMPT");
}

#[tokio::test]
async fn a_report_without_the_current_lease_is_stale_and_changes_nothing() {
    let p = platform().await;
    let wf = p.publish("single", &single_ir("single", None)).await;
    let run = p.start(&wf, &json!({})).await;
    let mut w = p.worker("wendy").await;
    w.register(&["act.a"]).await;
    let lease = w.one_task().await;

    let mut wrong_token = Worker::complete_message(&lease, "{}");
    wrong_token.lease_token = "0".repeat(64);
    assert_code(
        w.client.complete_step(w.req(wrong_token)).await,
        Code::Aborted,
        "STALE_ATTEMPT",
    );

    let mut future = Worker::complete_message(&lease, "{}");
    future.generation = lease.generation + 1;
    future.task_id = format!("{run}/a/{}", future.generation);
    assert_code(
        w.client.complete_step(w.req(future)).await,
        Code::Aborted,
        "STALE_ATTEMPT",
    );

    let mut other_step = Worker::complete_message(&lease, "{}");
    other_step.step_id = "019b5d6e-0000-7000-8000-000000000000".into();
    assert_code(
        w.client.complete_step(w.req(other_step)).await,
        Code::Aborted,
        "STALE_ATTEMPT",
    );

    let mut beat = w.heartbeat_message(&lease);
    beat.lease_token = "f".repeat(64);
    assert_code(
        w.client.heartbeat(w.req(beat)).await,
        Code::Aborted,
        "STALE_ATTEMPT",
    );

    // None of that touched the attempt: the rightful holder still completes it.
    assert_eq!(p.status(&run).await, "RUNNING");
    let done = w
        .complete(&lease, &json!({}))
        .await
        .expect("rightful holder");
    assert_eq!(done.run_status, "SUCCEEDED");
}

#[tokio::test]
async fn a_failure_is_routed_by_the_engine_and_unknown_codes_are_normalised() {
    let p = platform().await;
    let wf = p.publish("single", &single_ir("single", None)).await;
    let run = p.start(&wf, &json!({})).await;
    let mut w = p.worker("wendy").await;
    w.register(&["act.a"]).await;

    // Retryable with attempts left: the step backs off, the run goes on.
    let lease = w.one_task().await;
    let retry = w.fail(&lease, true).await.expect("fail");
    assert_eq!(retry.step_status, "RETRY_WAIT");
    assert_eq!(retry.run_status, "RUNNING");

    // Not retryable: the step and the run fail now.
    p.elapse_retry_delay(&run, "a").await;
    p.tick().await;
    let second = w.one_task().await;
    assert_eq!(second.attempt, 2);
    assert!(second.generation > lease.generation);
    let mut message = Worker::fail_message(&second, "not a valid CODE!", false);
    message.safe_message = String::new();
    let failed = w
        .client
        .fail_step(w.req(message))
        .await
        .expect("a malformed error code is normalised, not rejected")
        .into_inner();
    assert_eq!(failed.step_status, "FAILED");
    assert_eq!(failed.run_status, "FAILED");
    assert_eq!(p.status(&run).await, "FAILED");
}

#[tokio::test]
async fn a_cancellation_reaches_the_worker_through_the_heartbeat() {
    let p = platform().await;
    let wf = p.publish("single", &single_ir("single", None)).await;
    let run = p.start(&wf, &json!({})).await;
    let mut w = p.worker("wendy").await;
    w.register(&["act.a"]).await;
    let lease = w.one_task().await;

    let (status, _) = p
        .rest(
            Method::POST,
            &format!("/runs/{run}/operations"),
            Some(&json!({"action": "CANCEL", "reason": "the customer changed their mind"})),
        )
        .await;
    assert_eq!(status, 202);

    let beat = w
        .client
        .heartbeat(w.req(w.heartbeat_message(&lease)))
        .await
        .expect("heartbeat")
        .into_inner();
    assert!(beat.cancel_requested, "the worker must be told to stop");
}

/// G141 (owner decision 2026-10-16): a heartbeat that was already on its way when the same lease
/// reported is answered without an error, with no lease deadline (`0`) and no cancellation, so the
/// worker stops beating and the call is not counted as a failure. Only the lease that reported gets
/// this answer: a lost lease and a forged token are still stale attempts.
#[tokio::test]
async fn a_heartbeat_after_its_own_report_is_answered_without_an_error() {
    let p = platform().await;
    let wf = p.publish("single", &single_ir("single", None)).await;
    p.start(&wf, &json!({})).await;
    let mut w = p.worker("wendy").await;
    w.register(&["act.a"]).await;
    let lease = w.one_task().await;
    w.complete(&lease, &json!({})).await.expect("complete");

    let beat = w
        .client
        .heartbeat(w.req(w.heartbeat_message(&lease)))
        .await
        .expect("a late beat of the reporting lease is not an error")
        .into_inner();
    assert_eq!(
        beat.lease_expires_at_epoch_ms, 0,
        "no deadline: the attempt is over"
    );
    assert!(!beat.cancel_requested);

    let mut forged = w.heartbeat_message(&lease);
    forged.lease_token = "forged-lease-token".into();
    let refused = w.client.heartbeat(w.req(forged)).await.expect_err("forged");
    assert_eq!(machine_code(&refused), "STALE_ATTEMPT");
}

#[tokio::test]
async fn a_heartbeat_of_a_lost_lease_stays_a_stale_attempt_after_another_worker_reports() {
    let p = platform().await;
    let retry = json!({"maxAttempts": 2, "strategy": "fixed", "initialDelayMs": 1, "maxDelayMs": 1, "jitter": false});
    let wf = p
        .publish("retrying", &single_ir("retrying", Some(retry)))
        .await;
    let run = p.start(&wf, &json!({})).await;
    let mut lost = p.worker("wendy").await;
    lost.register(&["act.a"]).await;
    let first = lost.one_task().await;
    p.expire_lease(&run, "a").await;
    p.tick().await;
    p.elapse_retry_delay(&run, "a").await;
    p.tick().await;
    let mut rescuer = p.worker("rhea").await;
    rescuer.register(&["act.a"]).await;
    let second = rescuer.one_task().await;
    assert!(second.generation > first.generation);
    rescuer
        .complete(&second, &json!({}))
        .await
        .expect("rescuer completes");

    let refused = lost
        .client
        .heartbeat(lost.req(lost.heartbeat_message(&first)))
        .await
        .expect_err("the lost lease never reported");
    assert_eq!(machine_code(&refused), "STALE_ATTEMPT");
}

#[tokio::test]
async fn max_inflight_bounds_what_one_session_can_hold() {
    let p = platform().await;
    let wf = p.publish("single", &single_ir("single", None)).await;
    p.start(&wf, &json!({})).await;
    p.start(&wf, &json!({})).await;
    let mut w = p.worker("wendy").await;
    let request = w.req(Worker::register_message(&["act.a"], 1));
    let registered = w
        .client
        .register_worker(request)
        .await
        .expect("register")
        .into_inner();
    w.worker_id = registered.worker_id;
    w.session = registered.session_token;

    let first = w.poll(8).await;
    assert_eq!(first.len(), 1, "one slot, one task");
    assert!(w.poll(8).await.is_empty(), "the slot is taken");
    w.complete(&first[0], &json!({})).await.expect("complete");
    assert_eq!(w.poll(8).await.len(), 1, "the slot is free again");
}

// ------------------------------------------------------------------------- authentication

fn tokens_leak(status: &Status, token: &str) -> bool {
    let signature = token.rsplit('.').next().unwrap_or_default();
    status.message().contains(signature)
        || status
            .metadata()
            .iter()
            .any(|entry| format!("{entry:?}").contains(signature))
}

#[tokio::test]
async fn every_rpc_requires_a_valid_bearer_token_from_a_known_member() {
    let p = platform().await;
    let mut w = p.worker("wendy").await;
    w.register(&["act.a"]).await;

    // No credentials, a wrong scheme, garbage, an expired token, a stranger.
    let register = || Worker::register_message(&["act.a"], 10);
    let no_header = tonic::Request::new(register());
    assert_eq!(
        w.client
            .register_worker(no_header)
            .await
            .expect_err("refused")
            .code(),
        Code::Unauthenticated
    );

    let mut basic = tonic::Request::new(register());
    basic.metadata_mut().insert(
        "authorization",
        "Basic d2VuZHk6cHc=".parse().expect("ascii"),
    );
    assert_eq!(
        w.client
            .register_worker(basic)
            .await
            .expect_err("refused")
            .code(),
        Code::Unauthenticated
    );

    let garbage = w.req_with_token(register(), "not.a.jwt");
    let status = w
        .client
        .register_worker(garbage)
        .await
        .expect_err("refused");
    assert_eq!(status.code(), Code::Unauthenticated);
    assert_eq!(machine_code(&status), "UNAUTHENTICATED");

    let expired = p.api.idp.token("wendy", TENANT).issued(7200, 60).build();
    let status = w
        .client
        .register_worker(w.req_with_token(register(), &expired))
        .await
        .expect_err("refused");
    assert_eq!(status.code(), Code::Unauthenticated);
    assert!(!tokens_leak(&status, &expired));

    let stranger = p.api.idp.token("mallory", TENANT).build();
    let status = w
        .client
        .register_worker(w.req_with_token(register(), &stranger))
        .await
        .expect_err("a valid token of a non-member is refused");
    assert_eq!(status.code(), Code::Unauthenticated);
    assert!(!tokens_leak(&status, &stranger));

    // Two credentials are ambiguous, so neither is trusted.
    let good = p.api.idp.token("wendy", TENANT).build();
    let mut twice = w.req_with_token(register(), &good);
    twice.metadata_mut().append(
        "authorization",
        format!("Bearer {good}").parse().expect("ascii"),
    );
    assert_eq!(
        w.client
            .register_worker(twice)
            .await
            .expect_err("refused")
            .code(),
        Code::Unauthenticated
    );

    // The other four RPCs are protected the same way.
    let lease = {
        let wf = p.publish("single", &single_ir("single", None)).await;
        p.start(&wf, &json!({})).await;
        w.one_task().await
    };
    assert_eq!(
        w.client
            .poll_tasks(tonic::Request::new(w.poll_message(1)))
            .await
            .expect_err("refused")
            .code(),
        Code::Unauthenticated
    );
    assert_eq!(
        w.client
            .heartbeat(tonic::Request::new(w.heartbeat_message(&lease)))
            .await
            .expect_err("refused")
            .code(),
        Code::Unauthenticated
    );
    assert_eq!(
        w.client
            .complete_step(tonic::Request::new(Worker::complete_message(&lease, "{}")))
            .await
            .expect_err("refused")
            .code(),
        Code::Unauthenticated
    );
    assert_eq!(
        w.client
            .fail_step(tonic::Request::new(Worker::fail_message(
                &lease, "X", false
            )))
            .await
            .expect_err("refused")
            .code(),
        Code::Unauthenticated
    );
}

#[tokio::test]
async fn only_the_worker_role_may_speak_the_worker_protocol() {
    let p = platform().await;
    p.grant(p.tenant, "vera", Role::Viewer).await;
    p.grant(p.tenant, "olga", Role::Operator).await;
    p.grant(p.tenant, "dave", Role::Developer).await;
    // (An administrator holds every permission, `worker:execute` included, by the role model.)
    for who in ["vera", "olga", "dave"] {
        let mut client = p.connect(who, TENANT).await;
        let request = client.req(Worker::register_message(&["act.a"], 10));
        assert_code(
            client.client.register_worker(request).await,
            Code::PermissionDenied,
            "FORBIDDEN",
        );
        client.worker_id = "019b5d6e-0000-7000-8000-000000000000".into();
        client.session = "x".repeat(64);
        let request = client.req(client.poll_message(1));
        assert_eq!(
            client
                .client
                .poll_tasks(request)
                .await
                .expect_err("refused")
                .code(),
            Code::PermissionDenied,
            "{who}"
        );
    }
}

#[tokio::test]
async fn a_worker_of_another_tenant_cannot_see_touch_or_reuse_anything() {
    let p = platform().await;
    let wf = p.publish("single", &single_ir("single", None)).await;
    let run = p.start(&wf, &json!({})).await;
    let mut wendy = p.worker("wendy").await;
    wendy.register(&["act.a"]).await;
    let lease = wendy.one_task().await;

    // A worker of the other tenant (who has no such namespace).
    p.grant(p.other_tenant, "gina", Role::Worker).await;
    let mut gina = p.connect("gina", "globex").await;
    let status = gina
        .client
        .register_worker(gina.req(Worker::register_message(&["act.a"], 10)))
        .await
        .expect_err("the namespace does not exist for globex");
    assert_eq!(status.code(), Code::NotFound);

    // Her attempts on acme's attempt look exactly like attempts on something that never existed.
    let complete = gina
        .client
        .complete_step(gina.req(Worker::complete_message(&lease, "{}")))
        .await;
    assert_code(complete, Code::Aborted, "STALE_ATTEMPT");
    let beat = gina
        .client
        .heartbeat(gina.req(wendy.heartbeat_message(&lease)))
        .await;
    assert_code(beat, Code::Aborted, "STALE_ATTEMPT");

    // A session token is bound to the identity that opened it, even inside one tenant.
    p.grant(p.tenant, "wanda", Role::Worker).await;
    let mut wanda = p.connect("wanda", TENANT).await;
    wanda.worker_id = wendy.worker_id.clone();
    wanda.session = wendy.session.clone();
    let stolen = wanda
        .client
        .poll_tasks(wanda.req(wanda.poll_message(1)))
        .await;
    assert_eq!(stolen.expect_err("refused").code(), Code::Unauthenticated);

    // And none of it moved the run.
    assert_eq!(p.status(&run).await, "RUNNING");
    wendy
        .complete(&lease, &json!({}))
        .await
        .expect("the owner still can");
}

// ------------------------------------------------------------------------------ validation

#[tokio::test]
async fn malformed_requests_are_invalid_arguments_and_leave_the_attempt_intact() {
    let p = platform().await;
    let wf = p.publish("single", &single_ir("single", None)).await;
    let run = p.start(&wf, &json!({})).await;
    let mut w = p.worker("wendy").await;

    // Registration.
    let mut bad_version = Worker::register_message(&["act.a"], 10);
    bad_version
        .capabilities
        .as_mut()
        .expect("caps")
        .protocol_version = "2".into();
    let mut no_caps = Worker::register_message(&["act.a"], 10);
    no_caps.capabilities = None;
    let no_activities = Worker::register_message(&[], 10);
    let zero = Worker::register_message(&["act.a"], 0);
    let negative = Worker::register_message(&["act.a"], -3);
    let bad_name = Worker::register_message(&["not a name"], 10);
    for (label, message) in [
        ("unsupported version", bad_version),
        ("no capabilities", no_caps),
        ("no activities", no_activities),
        ("zero inflight", zero),
        ("negative inflight", negative),
        ("bad activity name", bad_name),
    ] {
        let status = w
            .client
            .register_worker(w.req(message))
            .await
            .expect_err(label);
        assert_eq!(status.code(), Code::InvalidArgument, "{label}: {status:?}");
    }
    let mut unknown_ns = Worker::register_message(&["act.a"], 10);
    unknown_ns.namespace = "nowhere".into();
    assert_eq!(
        w.client
            .register_worker(w.req(unknown_ns))
            .await
            .expect_err("unknown")
            .code(),
        Code::NotFound
    );

    w.register(&["act.a"]).await;
    // Polling.
    let mut bad_worker = w.poll_message(1);
    bad_worker.worker_id = "nope".into();
    assert_eq!(
        w.client
            .poll_tasks(w.req(bad_worker))
            .await
            .expect_err("bad id")
            .code(),
        Code::InvalidArgument
    );
    assert_eq!(
        w.client
            .poll_tasks(w.req(w.poll_message(-1)))
            .await
            .expect_err("negative")
            .code(),
        Code::InvalidArgument
    );
    let mut bad_session = w.poll_message(1);
    bad_session.session_token = "0".repeat(64);
    assert_eq!(
        w.client
            .poll_tasks(w.req(bad_session))
            .await
            .expect_err("session")
            .code(),
        Code::Unauthenticated
    );

    // Reporting.
    let lease = w.one_task().await;
    let tweak = |edit: &dyn Fn(&mut sagat_api_grpc::pb::CompleteStepRequest)| {
        let mut message = Worker::complete_message(&lease, "{}");
        edit(&mut message);
        message
    };
    let cases = vec![
        ("task id", tweak(&|m| m.task_id = "garbage".into())),
        ("zero attempt", tweak(&|m| m.attempt = 0)),
        ("zero generation", tweak(&|m| m.generation = 0)),
        ("generation vs task id", tweak(&|m| m.generation += 1)),
        (
            "run vs task id",
            tweak(&|m| m.run_id = "019b5d6e-0000-7000-8000-000000000000".into()),
        ),
        ("step id", tweak(&|m| m.step_id = "nope".into())),
        ("no lease token", tweak(&|m| m.lease_token = String::new())),
        (
            "short idempotency key",
            tweak(&|m| m.idempotency_key = "short".into()),
        ),
        (
            "blob output",
            tweak(&|m| m.output_blob_ref = "blob://x".into()),
        ),
        ("not json", tweak(&|m| m.output_json = "{oops".into())),
        (
            "digest mismatch",
            tweak(&|m| m.output_sha256 = "ab".repeat(32)),
        ),
    ];
    for (label, message) in cases {
        let status = w
            .client
            .complete_step(w.req(message))
            .await
            .expect_err(label);
        assert_eq!(status.code(), Code::InvalidArgument, "{label}: {status:?}");
        assert!(
            !status.message().contains(&lease.lease_token),
            "{label} echoed the token"
        );
    }
    let mut too_long = Worker::fail_message(&lease, "X", false);
    too_long.safe_message = "m".repeat(5000);
    assert_eq!(
        w.client
            .fail_step(w.req(too_long))
            .await
            .expect_err("too long")
            .code(),
        Code::InvalidArgument
    );

    // A matching digest is accepted and none of the refusals changed the attempt.
    assert_eq!(p.status(&run).await, "RUNNING");
    let mut ok = Worker::complete_message(&lease, "{\"ok\":true}");
    ok.output_sha256 = {
        // sha256 of {"ok":true}
        "4062edaf750fb8074e7e83e0c9028c94e32468a8b6f1614774328ef045150f93".into()
    };
    let done = w
        .client
        .complete_step(w.req(ok))
        .await
        .expect("accepted")
        .into_inner();
    assert_eq!(done.run_status, "SUCCEEDED");
}

#[tokio::test]
async fn an_oversized_message_is_refused_before_it_is_processed() {
    let p = platform().await;
    let wf = p.publish("single", &single_ir("single", None)).await;
    p.start(&wf, &json!({})).await;
    let mut w = p.worker("wendy").await;
    w.register(&["act.a"]).await;
    let lease = w.one_task().await;
    let huge = format!("\"{}\"", "x".repeat(400 * 1024));
    let status = w
        .client
        .complete_step(w.req(Worker::complete_message(&lease, &huge)))
        .await
        .expect_err("over the message limit");
    assert!(
        matches!(
            status.code(),
            Code::OutOfRange | Code::ResourceExhausted | Code::InvalidArgument
        ),
        "{status:?}"
    );
    // The attempt is untouched.
    w.complete(&lease, &json!({}))
        .await
        .expect("still completable");
}

// ---------------------------------------------------------------------------------- limits

#[tokio::test]
async fn a_subject_over_its_budget_is_told_when_to_come_back() {
    let p = platform_with(Options {
        subject_rate_limit: RateLimitConfig {
            per_second: 0.5,
            burst: 3.0,
        },
        ..Options::default()
    })
    .await;
    let mut w = p.worker("wendy").await;
    // A malformed poll costs budget (authentication happens first) and fails validation.
    let mut seen = Vec::new();
    for _ in 0..10 {
        let mut message = w.poll_message(1);
        message.worker_id = "nope".into();
        let status = w
            .client
            .poll_tasks(w.req(message))
            .await
            .expect_err("refused");
        let limited = status.code() == Code::ResourceExhausted;
        seen.push(status.code());
        if limited {
            assert_eq!(machine_code(&status), "RATE_LIMITED");
            let retry: u64 = status
                .metadata()
                .get("retry-after-ms")
                .expect("retry-after-ms")
                .to_str()
                .expect("ascii")
                .parse()
                .expect("number");
            assert!(retry >= 1000, "never an immediate retry: {retry}");
            break;
        }
    }
    assert_eq!(seen.first(), Some(&Code::InvalidArgument), "{seen:?}");
    assert_eq!(seen.last(), Some(&Code::ResourceExhausted), "{seen:?}");
}

#[tokio::test]
async fn a_session_that_is_not_used_expires() {
    let p = platform_with(Options {
        worker: WorkerSettings {
            session_ttl: Duration::from_secs(1),
            ..WorkerSettings::default()
        },
        ..Options::default()
    })
    .await;
    let mut w = p.worker("wendy").await;
    w.register(&["act.a"]).await;
    assert!(w.poll(1).await.is_empty(), "alive");
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let status = w
        .client
        .poll_tasks(w.req(w.poll_message(1)))
        .await
        .expect_err("expired session");
    assert_eq!(status.code(), Code::Unauthenticated);
}
