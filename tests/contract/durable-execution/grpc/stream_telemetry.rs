//! Bounded task-stream telemetry against the real tonic server and PostgreSQL store.

use std::time::Duration;

use reqwest::Method;
use sagat_domain::identity::Role;
use sagat_telemetry::{Metrics, TraceContext};
use serde_json::json;
use tonic::Code;

use crate::support::api::{Options, json_of};
use crate::support::logs;
use crate::support::worker::{NAMESPACE, TENANT, Worker, platform_with, single_ir};

const PAYLOAD_CANARY: &str = "stream-payload-canary-99f30c8d";
const IDENTITY_CANARY: &str = "stream-identity-canary-843b7e12";
const TOKEN_CANARY: &str = "stream-token-canary-521bd490";

fn total(page: &str, name: &str, labels: &[(&str, &str)]) -> f64 {
    page.lines()
        .filter(|line| {
            line.starts_with(name)
                && matches!(line[name.len()..].chars().next(), Some('{' | ' '))
                && labels
                    .iter()
                    .all(|(key, value)| line.contains(&format!("{key}=\"{value}\"")))
        })
        .filter_map(|line| line.rsplit(' ').next()?.parse::<f64>().ok())
        .sum()
}

async fn register_stream_worker(worker: &mut Worker<'_>) {
    let mut message = Worker::register_message(&["act.a"], 8);
    message
        .capabilities
        .as_mut()
        .expect("capabilities")
        .protocol_version = "1.4".into();
    let registered = worker
        .client
        .register_worker(worker.req(message))
        .await
        .expect("register protocol 1.4")
        .into_inner();
    worker.worker_id = registered.worker_id;
    worker.session = registered.session_token;
}

async fn wait_for_open_gauge(metrics: &Metrics, wanted: f64) {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if total(&metrics.render(), "worker_task_streams_open", &[]) == wanted {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("stream gauge settled");
}

#[test]
fn stream_metrics_are_bounded_clean_and_trace_preserving() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    let (telemetry, recorder) = Metrics::detached(Duration::from_secs(60));
    let captured = metrics::with_local_recorder(&recorder, || {
        runtime.block_on(async {
            let mut options = Options::default();
            options.worker.idle_retry = Duration::from_millis(25);
            let p = platform_with(options).await;
            let wf = p
                .publish("stream-telemetry", &single_ir("stream-telemetry", None))
                .await;
            let wanted_trace = "11111111111111111111111111111111";
            let traceparent = format!("00-{wanted_trace}-2222222222222222-01");
            let response = p
                .api
                .as_user(Method::POST, "/runs", "alice", TENANT)
                .header("x-octopus-namespace", NAMESPACE)
                .header("idempotency-key", "stream-telemetry-run")
                .header("traceparent", traceparent)
                .json(&json!({"workflowId": wf, "input": {"secret": PAYLOAD_CANARY}}))
                .send()
                .await
                .expect("start traced run");
            assert_eq!(response.status().as_u16(), 202);
            let run = json_of(response).await["runId"]
                .as_str()
                .expect("run id")
                .to_owned();

            let mut worker = p.worker(IDENTITY_CANARY).await;
            register_stream_worker(&mut worker).await;
            let (log_guard, logs) = logs::capture_at(tracing::Level::TRACE);

            // Filled success: the lease continues the run trace and closes as `credit`.
            let mut success = worker
                .client
                .stream_tasks(worker.req(worker.poll_message(1)))
                .await
                .expect("success stream")
                .into_inner();
            let frame = success
                .message()
                .await
                .expect("success read")
                .expect("success frame");
            assert_eq!(frame.tasks.len(), 1);
            assert_eq!(frame.tasks[0].run_id, run);
            let continued = TraceContext::from_traceparent(Some(&frame.tasks[0].traceparent));
            assert_eq!(continued.trace_id().to_string(), wanted_trace);
            assert!(success.message().await.expect("success eof").is_none());

            // Two unread bodies hold the subject cap past headers; the refusal and both drops must
            // use closed labels and return the gauge to zero.
            let first = worker
                .client
                .stream_tasks(worker.req(worker.poll_message(1)))
                .await
                .expect("first held stream");
            let second = worker
                .client
                .stream_tasks(worker.req(worker.poll_message(1)))
                .await
                .expect("second held stream");
            wait_for_open_gauge(&telemetry, 2.0).await;
            let refused = worker
                .client
                .stream_tasks(worker.req(worker.poll_message(1)))
                .await
                .expect_err("subject cap");
            assert_eq!(refused.code(), Code::ResourceExhausted);
            drop((first, second));
            wait_for_open_gauge(&telemetry, 0.0).await;

            // A valid short-lived bearer opens the body and then closes it as unauthenticated.
            let token = p
                .api
                .idp
                .token(IDENTITY_CANARY, TENANT)
                .issued(0, 1)
                .build();
            let request = worker.req_with_token(worker.poll_message(1), &token);
            let mut expired = worker
                .client
                .stream_tasks(request)
                .await
                .expect("short-lived stream")
                .into_inner();
            let status = expired
                .message()
                .await
                .expect_err("expiry is a trailer error");
            assert_eq!(status.code(), Code::Unauthenticated);
            drop(expired);
            wait_for_open_gauge(&telemetry, 0.0).await;

            // A caller-controlled invalid token also remains absent from logs and labels.
            let request = worker.req_with_token(worker.poll_message(1), TOKEN_CANARY);
            let status = worker
                .client
                .stream_tasks(request)
                .await
                .expect_err("invalid token");
            assert_eq!(status.code(), Code::Unauthenticated);

            drop(log_guard);
            logs.text()
        })
    });

    let page = telemetry.render();
    assert!(
        total(
            &page,
            "worker_task_streams_closed_total",
            &[("reason", "credit")]
        ) >= 1.0
    );
    assert!(
        total(
            &page,
            "worker_task_streams_closed_total",
            &[("reason", "cancelled")]
        ) >= 2.0
    );
    assert!(
        total(
            &page,
            "worker_task_streams_closed_total",
            &[("reason", "unauthenticated")]
        ) >= 1.0
    );
    assert!(
        total(
            &page,
            "worker_task_streams_refused_total",
            &[("scope", "subject")]
        ) >= 1.0
    );
    assert_eq!(total(&page, "worker_task_streams_open", &[]), 0.0);

    let allowed_reasons = [
        "credit",
        "bytes",
        "window",
        "cancelled",
        "unauthenticated",
        "refused",
        "unavailable",
        "internal",
    ];
    for line in page
        .lines()
        .filter(|line| line.starts_with("worker_task_streams_closed_total{"))
    {
        assert!(
            allowed_reasons
                .iter()
                .any(|reason| line.contains(&format!("reason=\"{reason}\""))),
            "unexpected closed label: {line}"
        );
    }
    for canary in [PAYLOAD_CANARY, IDENTITY_CANARY, TOKEN_CANARY] {
        assert!(!page.contains(canary), "metric retained {canary}");
        assert!(
            !captured.contains(canary),
            "log retained {canary}: {captured}"
        );
    }
}

#[tokio::test]
async fn saturated_stream_admission_preserves_lease_calls_and_another_tenant() {
    let mut options = Options::default();
    options.worker.idle_retry = Duration::from_millis(25);
    let p = platform_with(options).await;

    let acme_workflow = p
        .publish("stream-cap-acme", &single_ir("stream-cap-acme", None))
        .await;
    let acme_run = p.start(&acme_workflow, &json!({})).await;
    let mut acme = p.worker("saturated-worker").await;
    register_stream_worker(&mut acme).await;
    let lease = acme.one_task().await;
    assert_eq!(lease.run_id, acme_run);

    // Prepare another tenant's ready task before holding the stream slots, keeping the saturation
    // interval entirely inside the finite five-second receive window.
    let workspace = p.api.create_workspace("bob", "globex", "platform").await;
    let namespace = p
        .api
        .create_namespace("bob", "globex", &workspace, NAMESPACE, "dev")
        .await;
    p.grant(p.other_tenant, "other-worker", Role::Worker).await;
    let response = p
        .api
        .as_user(
            Method::PUT,
            &format!("/namespaces/{namespace}/worker-bindings/other-worker"),
            "bob",
            "globex",
        )
        .json(&json!({"activities": ["act.a"]}))
        .send()
        .await
        .expect("bind other tenant worker");
    assert_eq!(response.status().as_u16(), 200);
    let response = p
        .api
        .as_user(Method::POST, "/workflows", "bob", "globex")
        .header("x-octopus-namespace", NAMESPACE)
        .header("idempotency-key", "other-stream-workflow")
        .json(&json!({
            "name": "stream-cap-globex",
            "ir": single_ir("stream-cap-globex", None)
        }))
        .send()
        .await
        .expect("create other workflow");
    assert_eq!(response.status().as_u16(), 201);
    let other_workflow = json_of(response).await["id"]
        .as_str()
        .expect("workflow id")
        .to_owned();
    let response = p
        .api
        .as_user(
            Method::POST,
            &format!("/workflows/{other_workflow}/publish"),
            "bob",
            "globex",
        )
        .header("x-octopus-namespace", NAMESPACE)
        .header("idempotency-key", "publish-other-stream-workflow")
        .json(&json!({"draftRevision": 1}))
        .send()
        .await
        .expect("publish other workflow");
    assert_eq!(response.status().as_u16(), 201);
    let response = p
        .api
        .as_user(Method::POST, "/runs", "bob", "globex")
        .header("x-octopus-namespace", NAMESPACE)
        .header("idempotency-key", "other-stream-run")
        .json(&json!({"workflowId": other_workflow, "input": {}}))
        .send()
        .await
        .expect("start other run");
    assert_eq!(response.status().as_u16(), 202);
    let other_run = json_of(response).await["runId"]
        .as_str()
        .expect("other run id")
        .to_owned();
    let mut other = p.connect("other-worker", "globex").await;
    register_stream_worker(&mut other).await;

    let held_first = acme
        .client
        .stream_tasks(acme.req(acme.poll_message(1)))
        .await
        .expect("first held stream");
    let held_second = acme
        .client
        .stream_tasks(acme.req(acme.poll_message(1)))
        .await
        .expect("second held stream");
    let refused = acme
        .client
        .stream_tasks(acme.req(acme.poll_message(1)))
        .await
        .expect_err("subject stream cap remains exact");
    assert_eq!(refused.code(), Code::ResourceExhausted);

    let heartbeat = acme
        .client
        .heartbeat(acme.req(acme.heartbeat_message(&lease)))
        .await
        .expect("heartbeat reserve remains available")
        .into_inner();
    assert!(!heartbeat.cancel_requested);
    let completed = acme
        .complete(&lease, &json!({"done": true}))
        .await
        .expect("result reserve remains available");
    assert_eq!(completed.run_status, "SUCCEEDED");

    let mut other_stream = other
        .client
        .stream_tasks(other.req(other.poll_message(1)))
        .await
        .expect("other tenant stream remains available")
        .into_inner();
    let frame = other_stream
        .message()
        .await
        .expect("other tenant read")
        .expect("other tenant frame");
    assert_eq!(frame.tasks.len(), 1);
    assert_eq!(frame.tasks[0].run_id, other_run);
    let completed = other
        .complete(&frame.tasks[0], &json!({"done": true}))
        .await
        .expect("other tenant result");
    assert_eq!(completed.run_status, "SUCCEEDED");

    let still_refused = acme
        .client
        .stream_tasks(acme.req(acme.poll_message(1)))
        .await
        .expect_err("held bodies still own the original subject cap");
    assert_eq!(still_refused.code(), Code::ResourceExhausted);
    drop((held_first, held_second));
}
