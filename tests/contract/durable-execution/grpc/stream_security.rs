//! Security and cancellation contracts for the finite worker task stream, over the real tonic
//! server, identity provider and PostgreSQL store.

use std::time::Duration;

use reqwest::Method;
use sagat_api_grpc::pb::PollTasksResponse;
use sagat_domain::identity::Role;
use serde_json::json;
use tonic::{Code, Status, Streaming};
use uuid::Uuid;

use crate::support::api::Options;
use crate::support::worker::{
    NAMESPACE, Platform, TENANT, Worker, machine_code, platform_with, single_ir,
};

const CLAIM_WAIT: Duration = Duration::from_millis(150);
const STATUS_WAIT: Duration = Duration::from_secs(3);

async fn fast_platform() -> Platform {
    let mut options = Options::default();
    options.worker.idle_retry = Duration::from_millis(50);
    platform_with(options).await
}

async fn register_stream_worker(worker: &mut Worker<'_>, namespace: &str, activities: &[&str]) {
    let mut message = Worker::register_message(activities, 8);
    message.namespace = namespace.to_owned();
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
    assert_eq!(registered.accepted_protocol_version, "1.4");
    worker.worker_id = registered.worker_id;
    worker.session = registered.session_token;
}

async fn open_stream(worker: &mut Worker<'_>, credit: i32) -> Streaming<PollTasksResponse> {
    worker
        .client
        .stream_tasks(worker.req(worker.poll_message(credit)))
        .await
        .expect("open task stream")
        .into_inner()
}

async fn stream_status(stream: &mut Streaming<PollTasksResponse>) -> Status {
    tokio::time::timeout(STATUS_WAIT, stream.message())
        .await
        .expect("the authority refusal must end the waiting stream promptly")
        .expect_err("the stream must end with a status")
}

async fn assert_unleased(p: &Platform, run: &str) {
    let row: (String, i32, Option<String>) = sqlx::query_as(
        "SELECT status, attempt, leased_by FROM steps WHERE tenant_id = $1 AND run_id = $2 AND node = 'a'",
    )
    .bind(p.tenant.as_uuid())
    .bind(Uuid::parse_str(run).expect("run id"))
    .fetch_one(&p.api.db.pool)
    .await
    .expect("step projection");
    assert_eq!(row, ("READY".into(), 0, None), "run {run} was leased");
}

async fn bind(p: &Platform, namespace: &str, subject: &str, activities: &[&str]) {
    let (status, body) = p
        .rest(
            Method::PUT,
            &format!("/namespaces/{namespace}/worker-bindings/{subject}"),
            Some(&json!({"activities": activities})),
        )
        .await;
    assert_eq!(status, 200, "bind {subject}: {body}");
}

#[tokio::test]
async fn foreign_tenant_token_session_and_namespace_never_lease_ready_work() {
    let p = fast_platform().await;
    let workflow = p
        .publish(
            "stream-foreign-authority",
            &single_ir("stream-foreign-authority", None),
        )
        .await;
    let run = p.start(&workflow, &json!({})).await;

    let mut owner = p.worker("wendy").await;
    register_stream_worker(&mut owner, NAMESPACE, &["act.a"]).await;

    let workspace = p.api.create_workspace("alice", TENANT, "isolated").await;
    let isolated_namespace = p
        .api
        .create_namespace("alice", TENANT, &workspace, "isolated", "dev")
        .await;
    p.grant(p.tenant, "nora", Role::Worker).await;
    bind(&p, &isolated_namespace, "nora", &["act.a"]).await;
    let mut isolated = p.connect("nora", TENANT).await;
    register_stream_worker(&mut isolated, "isolated", &["act.a"]).await;

    // A valid bearer belonging to another subject cannot use the owner's session.
    let nora_token = p.api.idp.token("nora", TENANT).build();
    let request = owner.req_with_token(owner.poll_message(1), &nora_token);
    let status = owner
        .client
        .stream_tasks(request)
        .await
        .expect_err("foreign subject token");
    assert_eq!(status.code(), Code::Unauthenticated);

    // Nor can the owner use a session opened by the other subject and namespace.
    let request = owner.req(isolated.poll_message(1));
    let status = owner
        .client
        .stream_tasks(request)
        .await
        .expect_err("foreign session");
    assert_eq!(status.code(), Code::Unauthenticated);

    // The bearer is valid for another tenant, but it has no authority over acme's session.
    p.grant(p.other_tenant, "gina", Role::Worker).await;
    let globex_token = p.api.idp.token("gina", "globex").build();
    let request = owner.req_with_token(owner.poll_message(1), &globex_token);
    let status = owner
        .client
        .stream_tasks(request)
        .await
        .expect_err("foreign tenant token");
    assert_eq!(status.code(), Code::Unauthenticated);

    // A completely valid stream in another namespace remains idle while payments has ready work.
    let mut foreign_namespace_stream = open_stream(&mut isolated, 1).await;
    let frame = tokio::time::timeout(CLAIM_WAIT * 2, foreign_namespace_stream.message()).await;
    assert!(frame.is_err(), "a foreign namespace received a frame");
    drop(foreign_namespace_stream);
    assert_unleased(&p, &run).await;

    // Refusals and cancellation did not poison recovery for the rightful session.
    let mut rightful = open_stream(&mut owner, 1).await;
    let frame = tokio::time::timeout(STATUS_WAIT, rightful.message())
        .await
        .expect("rightful delivery")
        .expect("stream read")
        .expect("task frame");
    assert_eq!(frame.tasks.len(), 1);
    assert_eq!(frame.tasks[0].run_id, run);
}

#[tokio::test]
async fn role_and_membership_removal_end_waiting_streams_before_another_lease() {
    let p = fast_platform().await;
    let mut demoted = p.worker("demoted").await;
    let mut removed = p.worker("removed").await;
    register_stream_worker(&mut demoted, NAMESPACE, &["act.a"]).await;
    register_stream_worker(&mut removed, NAMESPACE, &["act.a"]).await;
    let mut demoted_stream = open_stream(&mut demoted, 2).await;
    let mut removed_stream = open_stream(&mut removed, 2).await;
    tokio::time::sleep(CLAIM_WAIT).await;

    sqlx::query(
        "UPDATE memberships SET role = 'viewer' WHERE tenant_id = $1 AND subject = 'demoted'",
    )
    .bind(p.tenant.as_uuid())
    .execute(&p.api.db.pool)
    .await
    .expect("demote worker");
    sqlx::query("DELETE FROM memberships WHERE tenant_id = $1 AND subject = 'removed'")
        .bind(p.tenant.as_uuid())
        .execute(&p.api.db.pool)
        .await
        .expect("remove worker membership");

    let workflow = p
        .publish(
            "stream-membership-change",
            &single_ir("stream-membership-change", None),
        )
        .await;
    let first = p.start(&workflow, &json!({"case": "role"})).await;
    let second = p.start(&workflow, &json!({"case": "membership"})).await;

    let status = stream_status(&mut demoted_stream).await;
    assert_eq!(status.code(), Code::PermissionDenied, "{status:?}");
    assert_eq!(machine_code(&status), "FORBIDDEN");
    let status = stream_status(&mut removed_stream).await;
    assert_eq!(status.code(), Code::Unauthenticated, "{status:?}");
    assert_eq!(machine_code(&status), "UNAUTHENTICATED");
    assert_unleased(&p, &first).await;
    assert_unleased(&p, &second).await;
}

#[tokio::test]
async fn narrowed_and_revoked_bindings_end_waiting_streams_before_another_lease() {
    let p = fast_platform().await;
    let namespace = p.namespace_id().await;
    let mut narrowed = p.worker("narrowed").await;
    let mut revoked = p.worker("revoked").await;
    register_stream_worker(&mut narrowed, NAMESPACE, &["act.a"]).await;
    register_stream_worker(&mut revoked, NAMESPACE, &["act.a"]).await;
    let mut narrowed_stream = open_stream(&mut narrowed, 2).await;
    let mut revoked_stream = open_stream(&mut revoked, 2).await;
    tokio::time::sleep(CLAIM_WAIT).await;

    bind(&p, &namespace, "narrowed", &["other.*"]).await;
    let (status, body) = p
        .rest(
            Method::DELETE,
            &format!("/namespaces/{namespace}/worker-bindings/revoked"),
            None,
        )
        .await;
    assert_eq!(status, 200, "unbind revoked: {body}");

    let workflow = p
        .publish(
            "stream-binding-change",
            &single_ir("stream-binding-change", None),
        )
        .await;
    let first = p.start(&workflow, &json!({"case": "narrowed"})).await;
    let second = p.start(&workflow, &json!({"case": "revoked"})).await;

    for (label, stream) in [
        ("narrowed", &mut narrowed_stream),
        ("revoked", &mut revoked_stream),
    ] {
        let status = stream_status(stream).await;
        assert_eq!(status.code(), Code::Unauthenticated, "{label}: {status:?}");
        assert_eq!(machine_code(&status), "UNAUTHENTICATED", "{label}");
    }
    assert_unleased(&p, &first).await;
    assert_unleased(&p, &second).await;
}

#[tokio::test]
async fn bearer_expiry_ends_a_waiting_stream_and_suppresses_later_work() {
    let p = fast_platform().await;
    let mut worker = p.worker("short-lived").await;
    register_stream_worker(&mut worker, NAMESPACE, &["act.a"]).await;
    let token = p.api.idp.token("short-lived", TENANT).issued(0, 2).build();
    let request = worker.req_with_token(worker.poll_message(1), &token);
    let mut stream = worker
        .client
        .stream_tasks(request)
        .await
        .expect("open before token expiry")
        .into_inner();
    tokio::time::sleep(Duration::from_millis(2_200)).await;

    let workflow = p
        .publish(
            "stream-token-expiry",
            &single_ir("stream-token-expiry", None),
        )
        .await;
    let run = p.start(&workflow, &json!({})).await;
    let status = stream_status(&mut stream).await;
    assert_eq!(status.code(), Code::Unauthenticated, "{status:?}");
    assert_eq!(machine_code(&status), "UNAUTHENTICATED");
    assert_unleased(&p, &run).await;
}

#[tokio::test]
async fn cancelling_an_idle_stream_stops_its_claims_and_a_new_stream_recovers() {
    let p = fast_platform().await;
    let mut worker = p.worker("cancelled").await;
    register_stream_worker(&mut worker, NAMESPACE, &["act.a"]).await;
    let stream = open_stream(&mut worker, 1).await;
    tokio::time::sleep(CLAIM_WAIT).await;
    drop(stream);
    tokio::time::sleep(CLAIM_WAIT * 2).await;

    let workflow = p
        .publish(
            "stream-cancel-recovery",
            &single_ir("stream-cancel-recovery", None),
        )
        .await;
    let run = p.start(&workflow, &json!({})).await;
    tokio::time::sleep(CLAIM_WAIT * 2).await;
    assert_unleased(&p, &run).await;

    let mut replacement = open_stream(&mut worker, 1).await;
    let frame = tokio::time::timeout(STATUS_WAIT, replacement.message())
        .await
        .expect("replacement delivery")
        .expect("stream read")
        .expect("task frame");
    assert_eq!(frame.tasks.len(), 1);
    assert_eq!(frame.tasks[0].run_id, run);
}
