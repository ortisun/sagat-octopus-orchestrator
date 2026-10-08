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
