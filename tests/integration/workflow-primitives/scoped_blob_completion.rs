//! Public scoped blob commits share the report transaction and use current exact-path authority.
use sagat_api_grpc::pb::{TaskLease, WriteBlobHeader, WriteBlobResponse};
use sagat_domain::execution::RunId;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::support::blob_collections::*;
use crate::support::worker::{Platform, Worker};

async fn leases<'a>(
    p: &'a Platform,
    name: &str,
    count: usize,
) -> (Sched, Worker<'a>, Uuid, Vec<TaskLease>) {
    let workflow = p
        .publish(name, &collection_ir(name, "input.items", count as u32))
        .await;
    let run = start_inline(
        p,
        &workflow,
        &json!({"items": (0..count).collect::<Vec<_>>()}),
    )
    .await;
    let scheduler = scheduler(p, None).await;
    let mut worker = scoped_worker(p, name, &["src.line"]).await;
    for _ in 0..count + 3 {
        scheduler.tick().await;
    }
    let tasks = worker.poll(count as i32).await;
    assert_eq!(tasks.len(), count);
    sqlx::query("REVOKE SELECT ON iteration_checkpoints FROM sagat_app")
        .execute(&p.api.db.pool)
        .await
        .expect("deny unrelated history");
    (scheduler, worker, run, tasks)
}

async fn upload(worker: &mut Worker<'_>, task: &TaskLease) -> WriteBlobResponse {
    let bytes = vec![b'x'; 300_000];
    worker
        .write_blob(
            task,
            WriteBlobHeader {
                content_type: "application/octet-stream".into(),
                expected_size: bytes.len() as i64,
                expected_sha256: sha(&bytes),
                ..Default::default()
            },
            &bytes,
            64_000,
        )
        .await
        .expect("upload under the exact scoped lease")
}

async fn state(p: &Platform, blob: &str) -> String {
    sqlx::query_scalar("SELECT state FROM blobs WHERE tenant_id=$1 AND id=$2")
        .bind(p.tenant.as_uuid())
        .bind(blob.parse::<Uuid>().expect("blob UUID"))
        .fetch_one(&p.api.db.pool)
        .await
        .expect("catalog state")
}

async fn finish(p: &Platform, scheduler: &Sched, run: Uuid) {
    sqlx::query("GRANT SELECT ON iteration_checkpoints TO sagat_app")
        .execute(&p.api.db.pool)
        .await
        .expect("restore full replay oracle");
    tick_until_settled(p, scheduler, run, 20).await;
    assert_eq!(status_and_code(p, run).await.0, "SUCCEEDED");
    p.api
        .executions
        .verify_run(p.tenant, RunId::new(run))
        .await
        .expect("full replay");
}

#[tokio::test]
async fn a_scoped_blob_commits_and_replays_without_unrelated_history() {
    let p = platform().await;
    let (scheduler, mut worker, run, tasks) = leases(&p, "scoped-blob-replay", 1).await;
    let task = &tasks[0];
    let blob = upload(&mut worker, task).await;
    let unused = upload(&mut worker, task).await;
    let first = worker
        .complete_with_blob(task, &blob.blob_id)
        .await
        .expect("bounded scoped commit");
    assert!(!first.already_applied);
    assert_eq!(first.step_status, "SUCCEEDED");
    assert_eq!(state(&p, &blob.blob_id).await, "COMMITTED");
    let replay = worker
        .complete_with_blob(task, &blob.blob_id)
        .await
        .expect("receipt replay without restoring history");
    assert!(replay.already_applied);
    let conflict = worker
        .complete_with_blob(task, &unused.blob_id)
        .await
        .expect_err("another blob is a different report for the same key");
    assert_eq!(
        crate::support::worker::machine_code(&conflict),
        "IDEMPOTENCY_KEY_REUSED"
    );
    assert_eq!(state(&p, &unused.blob_id).await, "UPLOADED");
    sqlx::query("UPDATE memberships SET role='viewer' WHERE tenant_id=$1 AND subject=$2")
        .bind(p.tenant.as_uuid())
        .bind(&worker.subject)
        .execute(&p.api.db.pool)
        .await
        .expect("withdraw current worker role");
    let denied = worker
        .complete_with_blob(task, &blob.blob_id)
        .await
        .expect_err("receipt replay still requires current authority");
    assert_eq!(denied.code(), tonic::Code::PermissionDenied);
    sqlx::query("UPDATE memberships SET role='worker' WHERE tenant_id=$1 AND subject=$2")
        .bind(p.tenant.as_uuid())
        .bind(&worker.subject)
        .execute(&p.api.db.pool)
        .await
        .expect("restore fixture worker role");
    finish(&p, &scheduler, run).await;
    assert_eq!(aggregate(&p, run).await[0]["$blob"]["id"], blob.blob_id);
}

#[tokio::test]
async fn a_scoped_blob_commit_rolls_back_with_its_failed_journal_write() {
    let p = platform().await;
    let (scheduler, mut worker, run, tasks) = leases(&p, "scoped-blob-rollback", 1).await;
    let task = &tasks[0];
    let blob = upload(&mut worker, task).await;
    sqlx::raw_sql(
        "CREATE FUNCTION fail_scoped_blob_journal() RETURNS trigger LANGUAGE plpgsql AS $$ \
        BEGIN RAISE EXCEPTION 'injected scoped completion journal failure'; RETURN NEW; END $$; \
        CREATE TRIGGER fail_scoped_blob_journal BEFORE INSERT ON run_events FOR EACH ROW \
        WHEN (NEW.event_type='step.completed') EXECUTE FUNCTION fail_scoped_blob_journal();",
    )
    .execute(&p.api.db.pool)
    .await
    .expect("fault at the actual commit boundary");
    let refused = worker
        .complete_with_blob(task, &blob.blob_id)
        .await
        .expect_err("journal failure");
    assert_eq!(refused.code(), tonic::Code::Internal);
    assert_eq!(state(&p, &blob.blob_id).await, "UPLOADED");
    let status: String =
        sqlx::query_scalar("SELECT status FROM steps WHERE tenant_id=$1 AND step_id=$2")
            .bind(p.tenant.as_uuid())
            .bind(task.step_id.parse::<Uuid>().expect("step UUID"))
            .fetch_one(&p.api.db.pool)
            .await
            .expect("lease status");
    assert_eq!(status, "LEASED");
    sqlx::raw_sql("DROP TRIGGER fail_scoped_blob_journal ON run_events; DROP FUNCTION fail_scoped_blob_journal();")
        .execute(&p.api.db.pool).await.expect("restore journal writes");
    let committed = worker
        .complete_with_blob(task, &blob.blob_id)
        .await
        .expect("same-key retry applies after rollback");
    assert!(!committed.already_applied);
    assert_eq!(state(&p, &blob.blob_id).await, "COMMITTED");
    assert!(
        worker
            .complete_with_blob(task, &blob.blob_id)
            .await
            .expect("replay")
            .already_applied
    );
    finish(&p, &scheduler, run).await;
}

#[tokio::test]
async fn a_scoped_blob_cannot_be_committed_by_another_iteration() {
    let p = platform().await;
    let (scheduler, mut worker, run, tasks) = leases(&p, "scoped-blob-provenance", 2).await;
    let blob = upload(&mut worker, &tasks[0]).await;
    let refused = worker
        .complete_with_blob(&tasks[1], &blob.blob_id)
        .await
        .expect_err("another iteration is not the owner");
    assert_eq!(refused.code(), tonic::Code::NotFound);
    assert_eq!(state(&p, &blob.blob_id).await, "UPLOADED");
    worker
        .complete_with_blob(&tasks[0], &blob.blob_id)
        .await
        .expect("owning iteration");
    worker
        .complete(&tasks[1], &json!(2))
        .await
        .expect("other iteration stays usable");
    finish(&p, &scheduler, run).await;
    let index = tasks[0]
        .instance
        .as_ref()
        .expect("scoped identity")
        .instance_path[0]
        .iteration_index as usize;
    let result: Value = aggregate(&p, run).await;
    assert_eq!(result[index]["$blob"]["id"], blob.blob_id);
    assert_eq!(result[1 - index], 2);
}

#[tokio::test]
async fn an_uploaded_scoped_result_can_be_reported_after_cancellation() {
    let p = platform().await;
    let (scheduler, mut worker, run, tasks) = leases(&p, "scoped-blob-cancel-report", 1).await;
    let task = &tasks[0];
    let blob = upload(&mut worker, task).await;
    sqlx::query("GRANT SELECT ON iteration_checkpoints TO sagat_app")
        .execute(&p.api.db.pool)
        .await
        .expect("normal operator cancellation");
    let (status, body) = p
        .rest(
            reqwest::Method::POST,
            &format!("/runs/{run}/operations"),
            Some(&json!({"action":"CANCEL", "reason":"stop after the effect produced its output"})),
        )
        .await;
    assert_eq!(status, 202, "{body}");
    for _ in 0..4 {
        scheduler.tick().await;
    }
    let request = worker.req(worker.heartbeat_message(task));
    assert!(
        worker
            .client
            .heartbeat(request)
            .await
            .expect("current attempt sees cancellation")
            .into_inner()
            .cancel_requested
    );
    sqlx::query("REVOKE SELECT ON iteration_checkpoints FROM sagat_app")
        .execute(&p.api.db.pool)
        .await
        .expect("deny unrelated history again");
    let ack = worker
        .complete_with_blob(task, &blob.blob_id)
        .await
        .expect("a known produced result is recorded during drain, like an inline result");
    assert_eq!(ack.step_status, "SUCCEEDED");
    assert_eq!(state(&p, &blob.blob_id).await, "COMMITTED");
    sqlx::query("GRANT SELECT ON iteration_checkpoints TO sagat_app")
        .execute(&p.api.db.pool)
        .await
        .expect("full drain/replay oracle");
    tick_until_settled(&p, &scheduler, run, 30).await;
    assert_eq!(status_and_code(&p, run).await.0, "CANCELLED");
    p.api
        .executions
        .verify_run(p.tenant, RunId::new(run))
        .await
        .expect("cancelled run replay");
}

#[tokio::test]
async fn a_scoped_blob_report_rejects_a_non_holder_before_catalog_lookup() {
    let p = platform().await;
    let (scheduler, mut worker, run, tasks) = leases(&p, "scoped-blob-authority", 1).await;
    let task = &tasks[0];
    let blob = upload(&mut worker, task).await;
    let mut forged = task.clone();
    forged.lease_token = "not-the-lease-holder".into();
    for id in [blob.blob_id.clone(), Uuid::now_v7().to_string()] {
        let mut message = Worker::complete_message(&forged, "");
        message.output_blob_ref = id;
        message.output_sha256 = "00".repeat(32);
        let request = worker.req(message);
        let denied = worker
            .client
            .complete_step(request)
            .await
            .expect_err("lease before catalog");
        assert_eq!(
            crate::support::worker::machine_code(&denied),
            "STALE_ATTEMPT"
        );
    }
    assert_eq!(state(&p, &blob.blob_id).await, "UPLOADED");
    worker
        .complete_with_blob(task, &blob.blob_id)
        .await
        .expect("actual holder");
    finish(&p, &scheduler, run).await;
}

#[tokio::test]
async fn path_storage_rejects_a_blob_without_its_matching_output_descriptor() {
    use sagat_application::blobs::{BlobAccess, BlobCommit, BlobScope};
    use sagat_application::entropy::Token;
    use sagat_application::execution::{
        Actor, Effects, PathExecutionStore, PathExecutionTx, ReportedLease,
    };
    use sagat_application::runs::Lock;
    use sagat_domain::blobs::BlobId;
    use sagat_domain::execution::{
        ExecutionInstanceId, Fence, InstancePath, complete_path_instance,
    };
    use sagat_domain::workflow::{Digest, NodeId};
    let p = platform().await;
    let (scheduler, mut worker, run, tasks) = leases(&p, "scoped-blob-output-binding", 1).await;
    let task = &tasks[0];
    let blob = upload(&mut worker, task).await;
    let path = InstancePath::root()
        .child(
            NodeId::parse("items").expect("valid scoped blob fixture"),
            0,
        )
        .expect("valid scoped blob fixture");
    let instance = ExecutionInstanceId::in_path(
        path.clone(),
        NodeId::parse("line").expect("valid scoped blob fixture"),
    );
    let mismatched_type = serde_json::to_vec(&json!({"$blob": {
        "id": blob.blob_id, "size": 300000, "sha256": sha(&vec![b'x'; 300_000]),
        "contentType": "application/json"
    }}))
    .expect("valid scoped blob fixture");
    for (output, content_type) in [
        (b"null".to_vec(), "application/octet-stream"),
        (serde_json::to_vec(&json!({"$blob": {
            "id": Uuid::now_v7().to_string(), "size": 300000, "sha256": sha(&vec![b'x'; 300_000]),
            "contentType": "application/octet-stream"
        }}))
        .expect("valid scoped blob fixture"), "application/octet-stream"),
        (mismatched_type.clone(), "application/octet-stream"),
        (mismatched_type, "application/json"),
    ] {
        let mut tx = p
            .api
            .executions
            .begin_current_path(p.tenant, RunId::new(run), path.clone(), Lock::Wait)
            .await
            .expect("valid scoped blob fixture")
            .expect("valid scoped blob fixture");
        let definition = tx.snapshot().definition.clone();
        let progress = complete_path_instance(
            &definition,
            &mut tx.snapshot().state,
            &instance,
            Fence {
                attempt: task.attempt as u32,
                generation: task.generation as u64,
            },
        )
        .expect("valid scoped blob fixture");
        let mut effects = Effects::by(Actor::Worker(worker.subject.clone()), None);
        effects.reported = Some(ReportedLease {
            instance: instance.clone(),
            token_hash: Token::hash_of(&task.lease_token),
        });
        effects.blobs.push(BlobCommit {
            id: BlobId::new(blob.blob_id.parse().expect("valid scoped blob fixture")),
            size: 300000,
            digest: Digest::from_hex(&sha(&vec![b'x'; 300_000])).expect("valid scoped blob fixture"),
            content_type: sagat_domain::blobs::ContentType::parse(content_type).expect("valid scoped blob fixture"),
            scope: BlobScope {
                access: BlobAccess {
                    tenant_id: p.tenant,
                    namespace_id: tx.snapshot().info.namespace_id,
                },
                run_id: RunId::new(run),
                step_id: task.step_id.parse().expect("valid scoped blob fixture"),
                attempt: task.attempt as u32,
                generation: task.generation as u64,
            },
        });
        let result = tx
            .persist_addressed(&progress.events, &[(instance.clone(), output)], effects)
            .await;
        assert!(
            result.is_err(),
            "a committed blob must be the exact recorded output"
        );
        drop(tx);
        assert_eq!(state(&p, &blob.blob_id).await, "UPLOADED");
    }
    worker
        .complete_with_blob(task, &blob.blob_id)
        .await
        .expect("valid descriptor");
    finish(&p, &scheduler, run).await;
}

#[tokio::test]
async fn a_scoped_blob_quota_recovery_rechecks_lease_expiry() {
    let p = platform().await;
    let (_scheduler, mut worker, run, tasks) = leases(&p, "scoped-blob-quota-expiry", 1).await;
    let task = &tasks[0];
    let blob = upload(&mut worker, task).await;
    sqlx::query(
        "INSERT INTO quota_limits (tenant_id,namespace_id,resource,max_value,updated_by) \
        SELECT tenant_id,NULL,'tenant_content_bytes',content_bytes+content_reserved_bytes,'test' \
        FROM tenant_retained_usage WHERE tenant_id=$1",
    )
    .bind(p.tenant.as_uuid())
    .execute(&p.api.db.pool)
    .await
    .expect("close content quota");
    sqlx::query("UPDATE steps SET lease_expires_at=clock_timestamp()+interval '10 seconds' WHERE tenant_id=$1 AND run_id=$2 AND node='line'")
        .bind(p.tenant.as_uuid()).bind(run).execute(&p.api.db.pool).await.expect("valid scoped blob fixture");
    let mut guard = p
        .api
        .db
        .pool
        .begin()
        .await
        .expect("valid scoped blob fixture");
    let backend: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *guard)
        .await
        .expect("valid scoped blob fixture");
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
        .bind(format!("quota-policy:tenant_content_bytes:{}", p.tenant))
        .execute(&mut *guard)
        .await
        .expect("valid scoped blob fixture");
    let mut message = Worker::complete_message(task, "");
    message.output_blob_ref = blob.blob_id.clone();
    let request = worker.req(message);
    let mut client = worker.client.clone();
    let reporting = tokio::spawn(async move { client.complete_step(request).await });
    crate::support::eventually("completion reached content quota barrier", || async {
        let waiting: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE $1=ANY(pg_blocking_pids(pid)))",
        )
        .bind(backend)
        .fetch_one(&p.api.db.pool)
        .await
        .expect("valid scoped blob fixture");
        waiting.then_some(())
    })
    .await;
    sqlx::query("SELECT pg_sleep(GREATEST(0,EXTRACT(EPOCH FROM lease_expires_at-clock_timestamp()))+0.05) FROM steps WHERE tenant_id=$1 AND run_id=$2 AND node='line'")
        .bind(p.tenant.as_uuid()).bind(run).execute(&p.api.db.pool).await.expect("valid scoped blob fixture");
    guard.commit().await.expect("valid scoped blob fixture");
    let denied = reporting
        .await
        .expect("valid scoped blob fixture")
        .expect_err("quota recovery cannot report after lease expiry");
    assert_eq!(
        crate::support::worker::machine_code(&denied),
        "STALE_ATTEMPT"
    );
    assert_eq!(state(&p, &blob.blob_id).await, "UPLOADED");
    let status: String = sqlx::query_scalar(
        "SELECT status FROM steps WHERE tenant_id=$1 AND run_id=$2 AND node='line'",
    )
    .bind(p.tenant.as_uuid())
    .bind(run)
    .fetch_one(&p.api.db.pool)
    .await
    .expect("valid scoped blob fixture");
    assert_eq!(status, "LEASED");
    sqlx::query("GRANT SELECT ON iteration_checkpoints TO sagat_app")
        .execute(&p.api.db.pool)
        .await
        .expect("valid scoped blob fixture");
    p.api
        .executions
        .verify_run(p.tenant, RunId::new(run))
        .await
        .expect("expired report wrote no state");
}

#[tokio::test]
async fn a_racing_blob_preparation_replays_a_report_committed_after_its_first_lookup() {
    use sagat_application::clock::SystemClock;
    use sagat_application::entropy::SystemEntropy;
    use sagat_application::execution::Engine;
    use sagat_application::worker::{TaskRef, WorkerService, WorkerSettings};
    use sagat_domain::blobs::BlobId;
    use sagat_domain::execution::{ExecutionInstanceId, Fence, InstancePath};
    use sagat_domain::identity::{Principal, Role, Subject};
    use sagat_domain::workflow::NodeId;
    use sagat_infra_ir::JsonEvaluator;
    let p = platform().await;
    let (scheduler, mut worker, run, tasks) = leases(&p, "scoped-blob-preparation-race", 1).await;
    let task = &tasks[0];
    let blob = upload(&mut worker, task).await;
    let service = WorkerService::new(
        p.api.executions.clone(),
        SystemClock,
        Engine::new(JsonEvaluator),
        SystemEntropy,
        WorkerSettings::default(),
    );
    let principal = Principal::new(
        p.tenant,
        Subject::parse(&worker.subject).expect("worker subject"),
        Role::Worker,
    );
    let instance = ExecutionInstanceId::in_path(
        InstancePath::root()
            .child(NodeId::parse("items").expect("owner"), 0)
            .expect("path"),
        NodeId::parse("line").expect("activity"),
    );
    let attempt = TaskRef {
        run_id: RunId::new(run),
        instance: &instance,
        step_id: Some(task.step_id.parse().expect("step UUID")),
        fence: Fence {
            attempt: task.attempt as u32,
            generation: task.generation as u64,
        },
        lease_token: &task.lease_token,
    };
    let id = BlobId::new(blob.blob_id.parse().expect("blob UUID"));
    let key = format!("done-{}", task.task_id);
    assert!(
        service
            .replay_blob_completion(&principal, &attempt, id, "", &key)
            .await
            .expect("initial lookup")
            .is_none()
    );
    let committed = worker
        .complete_with_blob(task, &blob.blob_id)
        .await
        .expect("concurrent identical report commits");
    assert!(!committed.already_applied);
    let late = service
        .prepare_blob_completion(&principal, &attempt, id, "", &key)
        .await
        .expect("atomic preparation after a concurrent commit");
    match late {
        sagat_application::worker::BlobCompletionTarget::Replayed(ack) => {
            assert!(ack.already_applied);
            assert_eq!(
                ack.step_status,
                sagat_domain::execution::StepStatus::Succeeded
            );
        }
        sagat_application::worker::BlobCompletionTarget::Ready(_) => {
            panic!("must return the committed receipt")
        }
    }
    finish(&p, &scheduler, run).await;
}
