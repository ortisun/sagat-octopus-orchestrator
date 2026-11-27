//! `AcknowledgeTasks` over the real tonic server, identity provider and PostgreSQL store (feature 006, Task 006-049;
//! `stream-completion.md`, "Wire" and "The server's acknowledgement"): the answer in request order, the request's bounds
//! refused before any lock, and every status the call makes carrying the envelope `x-sagat-error`.
//!
//! This build negotiates at most 1.6: a session is raised to 1.8 by SQL (as `as_protocol_1_8` of `lease_withdrawal.rs`
//! does) on a platform judged at the ceiling of 1.8, whose engine gate keeps every requirement of 1.8, so its claim flags
//! the lease.

use std::sync::Arc;
use std::time::Duration;

use sagat_api_grpc::pb::{self, AcknowledgementVerdict, TaskAcknowledgement, TaskLease};
use sagat_application::instances::{Requirement, Requirements, ServingGate};
use serde_json::json;
use tonic::Code;

use crate::support::api::Options;
use crate::support::worker::{Platform, Worker, machine_code, platform_with, single_ir};

fn every_requirement_of_1_8() -> Arc<ServingGate> {
    let gate = ServingGate::opened();
    let both = || vec!["CONTROL_PLANE".to_owned(), "EXECUTION_ENGINE".to_owned()];
    gate.keep(&Requirements {
        epoch: 1,
        required: vec![
            Requirement {
                capability: "lease-withdrawal-v1".to_owned(),
                kinds: both(),
            },
            Requirement {
                capability: "lease-withdrawal-v2".to_owned(),
                kinds: both(),
            },
            Requirement {
                capability: "delivery-acknowledgement-v1".to_owned(),
                kinds: vec!["EXECUTION_ENGINE".to_owned()],
            },
        ],
    });
    Arc::new(gate)
}

async fn platform() -> Platform {
    let mut options = Options {
        hand_back_ceiling: 8,
        ..Options::default()
    };
    options.worker.idle_retry = Duration::from_millis(50);
    options.lease_withdrawal = Some(every_requirement_of_1_8());
    platform_with(options).await
}

async fn at_minor(p: &Platform, subject: &str, minor: i32) {
    let done = sqlx::query(
        "UPDATE worker_sessions SET accepted_protocol_minor = $3 WHERE tenant_id = $1 AND subject = $2",
    )
    .bind(p.tenant.as_uuid())
    .bind(subject)
    .bind(minor)
    .execute(&p.api.db.pool)
    .await
    .expect("raise the minor");
    assert!(done.rows_affected() >= 1, "no session of {subject}");
}

/// A worker of `subject` holding one lease of a new run, its session stored at `minor`.
async fn leased<'a>(p: &'a Platform, subject: &str, minor: i32) -> (Worker<'a>, TaskLease) {
    let name = format!("ack-{subject}");
    let workflow = p.publish(&name, &single_ir(&name, None)).await;
    p.start(&workflow, &json!({})).await;
    let mut worker = p.worker(subject).await;
    worker.register_version(&["act.a"], "1.6").await;
    at_minor(p, subject, minor).await;
    let lease = worker.one_task().await;
    (worker, lease)
}

fn acknowledgement(lease: &TaskLease) -> TaskAcknowledgement {
    TaskAcknowledgement {
        task_id: lease.task_id.clone(),
        run_id: lease.run_id.clone(),
        step_id: lease.step_id.clone(),
        attempt: lease.attempt,
        generation: lease.generation,
        lease_token: lease.lease_token.clone(),
        instance: lease.instance.clone(),
    }
}

async fn acknowledge(
    worker: &mut Worker<'_>,
    tasks: Vec<TaskAcknowledgement>,
) -> Result<pb::AcknowledgeTasksResponse, tonic::Status> {
    let request = worker.req(pb::AcknowledgeTasksRequest {
        worker_id: worker.worker_id.clone(),
        tasks,
    });
    worker
        .client
        .acknowledge_tasks(request)
        .await
        .map(tonic::Response::into_inner)
}

#[tokio::test]
async fn a_flagged_lease_is_acknowledged_and_a_repeat_is_answered_again() {
    let p = platform().await;
    let (mut worker, lease) = leased(&p, "ada", 8).await;
    assert!(lease.acknowledge_before_start, "flagged at its claim");
    for round in ["first", "repeat"] {
        let answer = acknowledge(&mut worker, vec![acknowledgement(&lease)])
            .await
            .expect("answered");
        assert_eq!(answer.results.len(), 1, "{round}");
        let result = &answer.results[0];
        assert_eq!(result.task_id, lease.task_id, "{round}");
        assert_eq!(
            result.verdict,
            AcknowledgementVerdict::Acknowledged as i32,
            "{round}"
        );
        assert!(
            // the time-to-live, and the 2 s the CI's virtual machine can step the database clock back
            (6_001..=32_000).contains(&result.lease_time_left_ms),
            "{round}: {}",
            result.lease_time_left_ms
        );
        assert_eq!(result.attempt_time_left_ms, None, "{round}: no timeoutMs");
    }
}

#[tokio::test]
async fn an_unflagged_lease_is_not_held_and_the_answer_keeps_the_request_order() {
    let p = platform().await;
    let (_, flagged) = leased(&p, "ada", 8).await;
    let (mut worker, unflagged) = leased(&p, "bob", 6).await;
    assert!(!unflagged.acknowledge_before_start);
    // a lease of another identity's session is still named by its token: the token is the only authority
    let answer = acknowledge(
        &mut worker,
        vec![acknowledgement(&unflagged), acknowledgement(&flagged)],
    )
    .await
    .expect("answered");
    let verdicts: Vec<_> = answer
        .results
        .iter()
        .map(|r| (r.task_id.clone(), r.verdict))
        .collect();
    assert_eq!(
        verdicts,
        [
            (
                unflagged.task_id.clone(),
                AcknowledgementVerdict::NotHeld as i32
            ),
            (
                flagged.task_id.clone(),
                AcknowledgementVerdict::Acknowledged as i32
            ),
        ]
    );
}

#[tokio::test]
async fn a_malformed_request_is_refused_before_any_lock_with_the_envelope() {
    let p = platform().await;
    let (mut worker, lease) = leased(&p, "ada", 8).await;
    let one = acknowledgement(&lease);
    let mut attempt_zero = one.clone();
    attempt_zero.attempt = 0;
    let mut other_id = one.clone();
    other_id.task_id = format!("{}/a/{}", lease.run_id, lease.generation + 1);
    other_id.generation = lease.generation + 1;
    for (case, tasks) in [
        ("no task", vec![]),
        ("65 tasks", vec![one.clone(); 65]),
        ("a repeated task_id", vec![one.clone(), one.clone()]),
        ("an attempt below 1", vec![attempt_zero]),
        ("one lease named twice", vec![one.clone(), other_id]),
    ] {
        let status = acknowledge(&mut worker, tasks).await.expect_err(case);
        assert_eq!(status.code(), Code::InvalidArgument, "{case}: {status:?}");
        assert_eq!(machine_code(&status), "INVALID_ARGUMENT", "{case}");
    }
    let mut malformed_worker = one.clone();
    malformed_worker.task_id = lease.task_id.clone();
    let request = worker.req(pb::AcknowledgeTasksRequest {
        worker_id: "not-a-uuid".to_owned(),
        tasks: vec![malformed_worker],
    });
    let status = worker
        .client
        .acknowledge_tasks(request)
        .await
        .expect_err("worker_id is validated");
    assert_eq!(status.code(), Code::InvalidArgument);
    // the lease was never written by any of them
    let acknowledged: bool = sqlx::query_scalar(
        "SELECT lease_acknowledged_at IS NOT NULL FROM steps WHERE tenant_id = $1 AND run_id = $2::uuid AND node = 'a'",
    )
    .bind(p.tenant.as_uuid())
    .bind(&lease.run_id)
    .fetch_one(&p.api.db.pool)
    .await
    .expect("the row");
    assert!(!acknowledged);
}

#[tokio::test]
async fn an_unauthenticated_call_is_refused_with_the_envelope() {
    let p = platform().await;
    let (mut worker, lease) = leased(&p, "ada", 8).await;
    let request = tonic::Request::new(pb::AcknowledgeTasksRequest {
        worker_id: worker.worker_id.clone(),
        tasks: vec![acknowledgement(&lease)],
    });
    let status = worker
        .client
        .acknowledge_tasks(request)
        .await
        .expect_err("no credentials");
    assert_eq!(status.code(), Code::Unauthenticated);
    assert!(!machine_code(&status).is_empty(), "the envelope");
}

/// The `access.denied.permission` records of `subject`, waited for a bounded time: the audit of a denial is written off
/// the answer path.
async fn denials_of(p: &Platform, subject: &str, expected: usize) -> Vec<String> {
    let read = || async {
        sqlx::query_scalar::<_, String>(
            "SELECT COALESCE(reason, '') FROM audit_records \
              WHERE tenant_id = $1 AND actor = $2 AND action = 'access.denied.permission' ORDER BY created_at",
        )
        .bind(p.tenant.as_uuid())
        .bind(subject)
        .fetch_all(&p.api.db.pool)
        .await
        .expect("the audit trail")
    };
    let waited = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let found = read().await;
            if found.len() >= expected {
                return found;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await;
    // a few more moments for a second record that must not come
    let found = waited.unwrap_or_else(|_| panic!("{expected} denial records of {subject}"));
    tokio::task::yield_now().await;
    assert_eq!(read().await, found);
    found
}

#[tokio::test]
async fn a_narrowed_binding_is_answered_not_held_and_audited_once() {
    let p = platform().await;
    let (mut worker, lease) = leased(&p, "ada", 8).await;
    sqlx::query(
        "UPDATE worker_bindings SET pattern = 'other.*' WHERE tenant_id = $1 AND subject = 'ada'",
    )
    .bind(p.tenant.as_uuid())
    .execute(&p.api.db.pool)
    .await
    .expect("narrow the binding");
    let answer = acknowledge(&mut worker, vec![acknowledgement(&lease)])
        .await
        .expect("answered");
    assert_eq!(
        answer.results[0].verdict,
        AcknowledgementVerdict::NotHeld as i32
    );
    let denials = denials_of(&p, "ada", 1).await;
    assert_eq!(denials.len(), 1, "{denials:?}");
    assert!(denials[0].contains("NOT_BOUND"), "{denials:?}");
}

#[tokio::test]
async fn an_identity_without_worker_execute_is_refused_and_audited() {
    let p = platform().await;
    let (mut worker, lease) = leased(&p, "ada", 8).await;
    // still a member, now of a role that executes no work: authenticated, refused by the permission
    sqlx::query("UPDATE memberships SET role = 'viewer' WHERE tenant_id = $1 AND subject = 'ada'")
        .bind(p.tenant.as_uuid())
        .execute(&p.api.db.pool)
        .await
        .expect("change the role");
    let status = acknowledge(&mut worker, vec![acknowledgement(&lease)])
        .await
        .expect_err("refused");
    assert_eq!(status.code(), Code::PermissionDenied, "{status:?}");
    assert!(!machine_code(&status).is_empty(), "the envelope");
    let denials = denials_of(&p, "ada", 1).await;
    assert_eq!(denials.len(), 1, "{denials:?}");
}
