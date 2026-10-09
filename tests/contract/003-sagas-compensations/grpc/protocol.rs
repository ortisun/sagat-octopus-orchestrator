use prost::Message;
use reqwest::Method;
use sagat_api_grpc::pb::TaskLease;
use serde_json::{Value, json};

use crate::support::worker::{Platform, Worker, machine_code, platform};

fn saga_ir(name: &str) -> Value {
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1", "kind": "Workflow",
        "metadata": {"name": name}, "entrypoint": "tx",
        "nodes": [
            {"id": "tx", "type": "saga", "begin": "reserve"},
            {"id": "reserve", "type": "activity", "activity": "limit.reserve",
             "compensateWith": "limit.release", "next": "submit"},
            {"id": "submit", "type": "activity", "activity": "payment.submit",
             "compensateWith": "payment.return", "reconcileWith": "payment.lookup"}
        ]
    })
}

const ACTIVITIES: &[&str] = &[
    "limit.reserve",
    "payment.submit",
    "limit.release",
    "payment.return",
    "payment.lookup",
];

/// A run whose `submit` was declined, so `reserve` is being released.
async fn compensating(p: &Platform, w: &mut Worker<'_>) -> (String, TaskLease) {
    let wf = p.publish("settlement", &saga_ir("settlement")).await;
    let run = p.start(&wf, &json!({})).await;
    w.register(ACTIVITIES).await;
    let reserve = w.one_task().await;
    w.complete(&reserve, &json!({"reservation": "r-1"}))
        .await
        .expect("reserve");
    let submit = w.one_task().await;
    w.fail(&submit, false).await.expect("declined");
    (run, reserve)
}

#[tokio::test]
async fn a_compensation_arrives_as_a_task_that_says_what_it_undoes() {
    let p = platform().await;
    let mut w = p.worker("wendy").await;
    let (run, reserve) = compensating(&p, &mut w).await;
    assert_eq!(reserve.task_kind, "FORWARD");
    assert!(reserve.target_node.is_empty() && reserve.target_idempotency_key.is_empty());

    let undo = w.one_task().await;
    assert_eq!(undo.activity_name, "limit.release");
    assert_eq!(undo.task_kind, "COMPENSATION");
    assert_eq!(undo.target_node, "reserve");
    assert_eq!(undo.target_idempotency_key, reserve.idempotency_key);
    assert_eq!(undo.task_id, format!("{run}/reserve~c/{}", undo.generation));
    assert_eq!(undo.attempt, 1);
    assert_eq!(undo.lease_token.len(), 64);
    assert!(undo.idempotency_key.starts_with("compensation:"));
    assert_ne!(undo.idempotency_key, reserve.idempotency_key);
    assert!(
        undo.traceparent.starts_with("00-"),
        "the run's trace continues into the compensation"
    );

    // Every RPC works on it exactly as on forward work.
    let beat = w
        .client
        .heartbeat(w.req(w.heartbeat_message(&undo)))
        .await
        .expect("heartbeat")
        .into_inner();
    assert!(!beat.cancel_requested);
    w.complete(&undo, &json!({})).await.expect("released");
    assert_eq!(p.status(&run).await, "COMPENSATED");
}

#[tokio::test]
async fn a_lookup_arrives_as_a_task_and_is_answered_with_a_verdict() {
    let p = platform().await;
    let wf = p.publish("settlement", &saga_ir("settlement")).await;
    let run = p.start(&wf, &json!({})).await;
    let mut w = p.worker("wendy").await;
    w.register(ACTIVITIES).await;
    let reserve = w.one_task().await;
    w.complete(&reserve, &json!({})).await.expect("reserve");
    let submit = w.one_task().await;
    // Not retryable, not a plain failure: the worker reports that it does not know.
    w.fail_with(&submit, "OUTCOME_UNKNOWN", false)
        .await
        .expect("reported");

    let lookup = w.one_task().await;
    assert_eq!(lookup.activity_name, "payment.lookup");
    assert_eq!(lookup.task_kind, "RECONCILIATION");
    assert_eq!(lookup.target_node, "submit");
    assert_eq!(lookup.target_idempotency_key, submit.idempotency_key);
    assert_eq!(
        lookup.task_id,
        format!("{run}/submit~r/{}", lookup.generation)
    );
    assert!(lookup.idempotency_key.starts_with("reconciliation:"));
    w.complete(&lookup, &json!({"effect": "not_applied"}))
        .await
        .expect("answer");
}

#[tokio::test]
async fn cancelling_a_run_does_not_cancel_the_compensation_that_is_undoing_it() {
    let p = platform().await;
    let mut w = p.worker("wendy").await;
    let (run, _) = compensating(&p, &mut w).await;
    let undo = w.one_task().await;
    let (status, body) = p
        .rest(
            Method::POST,
            &format!("/runs/{run}/operations"),
            Some(&json!({"action": "CANCEL", "reason": "cancelled while releasing"})),
        )
        .await;
    assert_eq!(status, 202, "{body}");
    let beat = w
        .client
        .heartbeat(w.req(w.heartbeat_message(&undo)))
        .await
        .expect("heartbeat")
        .into_inner();
    assert!(
        !beat.cancel_requested,
        "undoing what was done is not work to abandon"
    );
    w.complete(&undo, &json!({})).await.expect("released");
    assert_eq!(p.status(&run).await, "CANCELLED");
}

#[tokio::test]
async fn a_late_report_for_a_compensation_is_refused_like_any_stale_attempt() {
    let p = platform().await;
    let mut doomed = p.worker("wendy").await;
    let (run, _) = compensating(&p, &mut doomed).await;
    let first = doomed.one_task().await;
    p.expire_lease(&run, "reserve~c").await;
    p.tick().await;
    p.elapse_retry_delay(&run, "reserve~c").await;
    p.tick().await;
    let mut rescuer = p.worker("rhea").await;
    rescuer.register(ACTIVITIES).await;
    let second = rescuer.one_task().await;
    assert!(second.generation > first.generation);

    let late = doomed
        .complete(&first, &json!({}))
        .await
        .expect_err("stale");
    assert_eq!(machine_code(&late), "STALE_ATTEMPT");
    let late = doomed.fail(&first, true).await.expect_err("stale");
    assert_eq!(machine_code(&late), "STALE_ATTEMPT");
    rescuer
        .complete(&second, &json!({}))
        .await
        .expect("the live attempt counts");
}

#[tokio::test]
async fn a_code_that_is_not_a_code_is_recorded_as_a_worker_error_and_never_as_a_reserved_one() {
    let p = platform().await;
    let mut w = p.worker("wendy").await;
    let (run, _) = compensating(&p, &mut w).await;
    let undo = w.one_task().await;
    // Free text is not a failure code: it is recorded as the generic one, so a worker cannot make
    // the platform believe a reserved condition by how it spells its complaint.
    let ack = w
        .fail_with(&undo, "outcome unknown, please retry", true)
        .await
        .expect("reported");
    assert_eq!(ack.step_status, "RETRY_WAIT");
    let (status, graph) = p
        .rest(Method::GET, &format!("/runs/{run}/graph"), None)
        .await;
    assert_eq!(status, 200, "{graph}");
    let step = graph["nodes"]
        .as_array()
        .expect("nodes")
        .iter()
        .find(|n| n["id"] == "reserve~c")
        .expect("the compensation");
    assert_eq!(step["failureCode"], "WORKER_ERROR");
}

/// What a worker built against protocol 1.0 declares of a lease: fields 1 to 13, nothing else.
#[derive(Clone, PartialEq, Message)]
struct LeaseOfProtocol10 {
    #[prost(string, tag = "1")]
    task_id: String,
    #[prost(string, tag = "2")]
    run_id: String,
    #[prost(string, tag = "3")]
    step_id: String,
    #[prost(int32, tag = "4")]
    attempt: i32,
    #[prost(int64, tag = "5")]
    generation: i64,
    #[prost(string, tag = "6")]
    lease_token: String,
    #[prost(int64, tag = "7")]
    expires_at_epoch_ms: i64,
    #[prost(string, tag = "8")]
    activity_name: String,
    #[prost(string, tag = "9")]
    input_json: String,
    #[prost(string, tag = "10")]
    input_blob_ref: String,
    #[prost(string, tag = "11")]
    traceparent: String,
    #[prost(string, tag = "12")]
    tenant_alias: String,
    #[prost(string, tag = "13")]
    idempotency_key: String,
}

#[tokio::test]
async fn a_worker_built_before_sagas_decodes_every_lease_and_ignores_what_it_does_not_know() {
    let p = platform().await;
    let mut w = p.worker("wendy").await;
    let (_, _) = compensating(&p, &mut w).await;
    let undo = w.one_task().await;
    assert_eq!(undo.task_kind, "COMPENSATION");

    let bytes = undo.encode_to_vec();
    let old =
        LeaseOfProtocol10::decode(bytes.as_slice()).expect("an old worker decodes the new lease");
    assert_eq!(old.task_id, undo.task_id);
    assert_eq!(old.run_id, undo.run_id);
    assert_eq!(old.activity_name, "limit.release");
    assert_eq!(old.lease_token, undo.lease_token);
    assert_eq!(old.idempotency_key, undo.idempotency_key);
    // It can run the compensation as an ordinary activity and report it with what it knows.
    w.complete(&undo, &json!({})).await.expect("done");

    // And the other way: a lease an old platform sent (no new fields) decodes as FORWARD.
    let legacy = LeaseOfProtocol10 {
        task_id: "x".into(),
        activity_name: "act.a".into(),
        ..old
    };
    let decoded =
        TaskLease::decode(legacy.encode_to_vec().as_slice()).expect("new worker decodes old lease");
    assert!(decoded.task_kind.is_empty(), "an empty kind is FORWARD");
    assert!(decoded.target_node.is_empty());
}
