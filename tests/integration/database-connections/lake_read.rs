//! A lake attempt reads the blobs its lease named, and nothing else (feature 035, slice R, step R4, D035-144).
//!
//! `ReadBlob` is the one door of the data plane a managed step was shut out of altogether. It is opened for a lake attempt and only
//! for the blobs the lease of exactly that attempt pinned, so the attacks are the ones a compromised or confused lake runner could try:
//! a blob of another run, of another namespace or tenant, a blob that exists and the lease did not name, a lease that has lapsed, been
//! replaced by another attempt, been cancelled or been given up, another runner on the same lease, and every other managed door (the
//! lease check with no blob and the upload). Whatever the reason, the answer for a blob is the same `NotFound`, and a refusal is audited
//! once for the run without naming a blob.
#![allow(clippy::expect_used, clippy::panic)]
use super::lake::{append, iceberg_in, publish_on_lake, rows_schema, write};
use super::lake_blobs::{committed_output, pin};
use crate::support::engine::{World, ctx, task_ref};
use crate::support::{now, subject};
use sagat_application::{
    blobs::BlobScope,
    clock::SystemClock,
    database::ConnectionService,
    entropy::SystemEntropy,
    error::AppError,
    execution::Engine,
    runs::{RunOperation, RunService, StartRun},
    worker::{Capabilities, ClaimedTask, TaskRef, WorkerService, WorkerSettings},
};
use sagat_domain::{
    audit::AuditAction,
    binding::BindingProfile,
    blobs::BlobId,
    classification::EgressCeiling,
    database::{ConnectionStatus, WriteMode},
    execution::{Fence, RunId},
    identity::{Membership, Principal, Role},
};
use sagat_infra_ir::JsonEvaluator;
use sagat_infra_postgres::PgRunStore;
use serde_json::json;

const ACTIVITY: &str = "sagat.database.iceberg.write";
const TRACE: &str = "0af7651916cd43dd8448eb211c80319c";

type Workers = WorkerService<PgRunStore, SystemClock, JsonEvaluator, SystemEntropy>;

/// The lake runner and the service it calls, for the three questions of the scenario.
struct Gate<'a> {
    workers: &'a Workers,
    runner: &'a Principal,
}

impl Gate<'_> {
    async fn read(&self, task: &ClaimedTask, blob: BlobId) -> Result<BlobScope, AppError> {
        self.workers
            .check_blob_read(self.runner, &task_ref(task), blob, Some(TRACE))
            .await
    }

    async fn audited(&self, task: &ClaimedTask, blob: BlobId) -> Result<(), AppError> {
        self.workers
            .audit_blob_read(self.runner, &task_ref(task), blob, Some(TRACE))
            .await
    }

    /// The namespace's budget of lake attempts is counted by the scenario that follows: the step ends for good.
    async fn release(&self, task: ClaimedTask) {
        let key = format!("lake-read-release-{}-{}", task.run_id, task.fence.attempt);
        let _ = self
            .workers
            .fail(
                self.runner,
                &task_ref(&task),
                "ICEBERG_LIMIT_EXCEEDED",
                false,
                &key,
            )
            .await;
    }
}

/// What a runner asks of a lease that is not its tenant's, as the words of the refusals (a refusal that differs from another is a way to tell
/// a run from a run that does not exist): a read, the audit of a read, and the credentials.
async fn foreign_answers(
    workers: &Workers,
    stranger: &Principal,
    reference: TaskRef<'_>,
    blob: BlobId,
) -> (String, String, String) {
    (
        format!(
            "{:?}",
            workers
                .check_blob_read(stranger, &reference, blob, Some(TRACE))
                .await
                .err()
        ),
        format!(
            "{:?}",
            workers
                .audit_blob_read(stranger, &reference, blob, Some(TRACE))
                .await
                .err()
        ),
        format!(
            "{:?}",
            workers
                .resolve_database(stranger, &reference, None)
                .await
                .err()
        ),
    )
}

pub async fn run(w: &World) {
    let ns = w.namespace.id();
    ConnectionService::new(w.store.clone(), SystemClock)
        .put(
            &w.admin,
            ns,
            write(
                "lake-read",
                0,
                ConnectionStatus::Active,
                iceberg_in(
                    "https://catalog.example/api",
                    "db-credentials",
                    EgressCeiling::Internal,
                    vec![WriteMode::Append],
                ),
            ),
            &ctx(),
        )
        .await
        .expect("the connection");
    let workflow = publish_on_lake(
        w,
        &w.admin,
        append(
            "lake-read-flow",
            "lake-read",
            &["/id", "/total"],
            Some(rows_schema()),
        ),
    )
    .await
    .expect("a workflow on the connection");
    let lake_store = w.store.clone().with_lake_enabled(true);
    let runs = RunService::new(
        lake_store.clone(),
        SystemClock,
        Engine::new(JsonEvaluator),
        w.cache.clone(),
    );
    let workers = WorkerService::new(
        lake_store,
        SystemClock,
        Engine::new(JsonEvaluator),
        SystemEntropy,
        WorkerSettings::default(),
    );
    let runner = w.member("lake-read-runner", Role::DatabaseRunner).await;
    w.bindings
        .bind_with_profile(
            &w.admin,
            ns,
            runner.subject().as_str(),
            &[ACTIVITY.to_owned()],
            None,
            Some(BindingProfile::Lake),
            &ctx(),
        )
        .await
        .expect("bind the lake runner");
    let registered = workers
        .register(
            &runner,
            "payments",
            "lake-read-machine",
            &Capabilities {
                activities: vec![ACTIVITY.to_owned()],
                languages: vec![],
                pools: vec![],
                max_inflight: 32,
                protocol_version: "1.6".into(),
            },
        )
        .await
        .expect("register the lake runner");
    let token = registered.session_token.expose().to_owned();

    // A run of an Iceberg write, leased to the runner. Runs made earlier in the scenario wait too: poll until the one wanted comes.
    let leased = |key: &'static str| {
        let (runs, workers, runner, token) = (&runs, &workers, &runner, &token);
        async move {
            let input =
                serde_json::to_vec(&json!({"rows": [{"id": "r-1", "total": 3}]})).expect("input");
            let run = runs
                .start(
                    &w.admin,
                    &StartRun {
                        namespace: "payments",
                        workflow,
                        version: None,
                        input: &input,
                        correlation_id: None,
                        deadline: None,
                        key,
                    },
                    &ctx(),
                )
                .await
                .expect("a run of an Iceberg write")
                .info
                .run_id;
            for _ in 0..20 {
                let tasks = workers
                    .poll(runner, registered.worker_id, token, 8, Some(TRACE))
                    .await
                    .expect("poll");
                if let Some(task) = tasks.into_iter().find(|task| task.run_id == run) {
                    return task;
                }
            }
            panic!("the lake runner is never handed the step of the run {run}");
        }
    };
    let gate = Gate {
        workers: &workers,
        runner: &runner,
    };
    let tenant = w.admin.tenant_id().as_uuid();
    let audits = |action: AuditAction, run: RunId| async move {
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM audit_records WHERE tenant_id = $1 AND action = $2 AND resource_id LIKE $3",
        )
        .bind(tenant)
        .bind(action.as_str())
        .bind(format!("%{run}%"))
        .fetch_one(&w.db.pool)
        .await
        .expect("audit count")
    };
    let read_audits = |blob: BlobId| async move {
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM audit_records WHERE tenant_id = $1 AND action = $2 AND resource_id LIKE $3",
        )
        .bind(tenant)
        .bind(AuditAction::BlobRead.as_str())
        .bind(format!("%/{blob}"))
        .fetch_one(&w.db.pool)
        .await
        .expect("audit count")
    };

    // ----- the attempt reads what its lease named
    let task = leased("lake-read-start-00001").await;
    let run = task.run_id;
    let (named, unnamed) = (
        committed_output(w, run).await.id(),
        committed_output(w, run).await.id(),
    );
    pin(w, run, "save", task.fence, &[named]).await;
    let scope = gate
        .read(&task, named)
        .await
        .expect("a lake attempt reads the blob its lease named");
    assert_eq!(scope.run_id, run);
    assert_eq!(scope.access.tenant_id, w.admin.tenant_id());
    assert_eq!(scope.access.namespace_id, ns);
    gate.audited(&task, named)
        .await
        .expect("the read is audited before the bytes");
    gate.audited(&task, named).await.expect("and again");
    assert_eq!(
        read_audits(named).await,
        1,
        "the same attempt reading the same blob again is one record"
    );
    assert_eq!(
        audits(AuditAction::DatabaseDenied, run).await,
        0,
        "a healthy read is not a denial"
    );

    // ----- and nothing else: each of these is the same NotFound, whatever the blob is
    let other_run = leased("lake-read-start-00002").await;
    let others = [
        (
            "a blob this run committed and its lease did not name",
            unnamed,
        ),
        (
            "a blob of another run, committed by its step",
            committed_output(w, other_run.run_id).await.id(),
        ),
        ("a blob that does not exist", BlobId::generate()),
    ];
    for (name, blob) in &others {
        let refused = gate.read(&task, *blob).await;
        assert!(
            matches!(refused, Err(AppError::NotFound)),
            "{name}: {refused:?}"
        );
        let refused = gate.audited(&task, *blob).await;
        assert!(
            matches!(refused, Err(AppError::NotFound)),
            "{name}: {refused:?}"
        );
        assert_eq!(
            read_audits(*blob).await,
            0,
            "{name}: nothing is audited as read, no byte was handed over"
        );
    }
    assert_eq!(
        audits(AuditAction::DatabaseDenied, run).await,
        1,
        "the refusals are audited for the run, once, and name no blob"
    );
    // The other run's own attempt reads its own blob only when its own lease pinned it.
    let theirs = committed_output(w, other_run.run_id).await.id();
    assert!(matches!(
        gate.read(&other_run, theirs).await,
        Err(AppError::NotFound)
    ));
    pin(w, other_run.run_id, "save", other_run.fence, &[theirs]).await;
    gate.read(&other_run, theirs)
        .await
        .expect("pinned by its own lease");
    assert!(
        matches!(gate.read(&task, theirs).await, Err(AppError::NotFound)),
        "and a pin of another run's attempt is no leave to this one"
    );

    // ----- every other door of a managed step stays shut
    let lease_only = workers.check_lease(&runner, &task_ref(&task)).await;
    assert!(
        matches!(lease_only, Err(AppError::NotAllowed(_))),
        "the check that names no blob refuses a managed step: {lease_only:?}"
    );
    let upload = workers.check_blob_upload(&runner, &task_ref(&task)).await;
    assert!(
        matches!(upload, Err(AppError::NotAllowed(_))),
        "a lake step cannot upload a blob: {upload:?}"
    );

    // ----- the lease, not the holder of a token alone
    let mut wrong = task_ref(&task);
    wrong.fence = Fence {
        attempt: task.fence.attempt + 1,
        ..task.fence
    };
    assert!(matches!(
        workers
            .check_blob_read(&runner, &wrong, named, Some(TRACE))
            .await,
        Err(AppError::StaleAttempt)
    ));
    let mut wrong = task_ref(&task);
    wrong.fence = Fence {
        generation: task.fence.generation + 1,
        ..task.fence
    };
    assert!(matches!(
        workers
            .check_blob_read(&runner, &wrong, named, Some(TRACE))
            .await,
        Err(AppError::StaleAttempt)
    ));
    let mut wrong = task_ref(&task);
    wrong.lease_token = "not-the-token";
    assert!(matches!(
        workers
            .check_blob_read(&runner, &wrong, named, Some(TRACE))
            .await,
        Err(AppError::StaleAttempt)
    ));
    // Another identity, even a lake runner of the same namespace, is not the owner of the lease; nor is an ordinary worker.
    let sibling = w.member("lake-read-sibling", Role::DatabaseRunner).await;
    let worker = w.member("lake-read-worker", Role::Worker).await;
    for (name, principal) in [("a sibling lake runner", &sibling), ("a worker", &worker)] {
        let refused = workers
            .check_blob_read(principal, &task_ref(&task), named, Some(TRACE))
            .await;
        assert!(
            matches!(refused, Err(AppError::NotBound)),
            "{name}: {refused:?}"
        );
    }

    // ----- another tenant: a lake runner of a tenant that is not this run's, holding every id and token of the lease, is answered as it is for a
    // run that does not exist, whatever it asks (a read, the audit of a read, the credentials), and leaves nothing in this tenant's trail
    let stranger = Principal::new(
        w.other_admin.tenant_id(),
        subject("lake-read-stranger"),
        Role::DatabaseRunner,
    );
    sagat_infra_postgres::PgIdentityStore::new(w.db.pool.clone())
        .insert_membership(&Membership::restore(
            stranger.tenant_id(),
            stranger.subject().clone(),
            Role::DatabaseRunner,
            now(),
        ))
        .await
        .expect("a lake runner of the other tenant");
    let trail = || async {
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM audit_records WHERE tenant_id = $1 AND resource_id LIKE $2",
        )
        .bind(tenant)
        .bind(format!("%{run}%"))
        .fetch_one(&w.db.pool)
        .await
        .expect("audit count")
    };
    let before = trail().await;
    let real = task_ref(&task);
    let mut ghost = task_ref(&task);
    ghost.run_id = RunId::generate();
    let (foreign, absent) = (
        foreign_answers(&workers, &stranger, real, named).await,
        foreign_answers(&workers, &stranger, ghost, named).await,
    );
    for refused in [&foreign.0, &foreign.1, &foreign.2] {
        assert_ne!(
            refused, "None",
            "a runner of another tenant is refused: {foreign:?}"
        );
    }
    assert_eq!(
        foreign, absent,
        "the answer for this tenant's lease is the answer for a run that does not exist"
    );
    assert_eq!(
        trail().await,
        before,
        "another tenant's asking leaves nothing in this tenant's trail"
    );

    // ----- a lease that is over reads nothing: given up, cancelled, lapsed, or replaced by a retry
    // Replaced: the step fails retryably, the next attempt is leased with a fence of its own, and the first attempt's token reads
    // nothing while the second has no pin yet.
    let retried = workers
        .fail(
            &runner,
            &task_ref(&task),
            "CATALOG_UNAVAILABLE",
            true,
            "lake-read-retry-1",
        )
        .await;
    assert!(retried.is_ok(), "{retried:?}");
    assert!(
        matches!(gate.read(&task, named).await, Err(AppError::StaleAttempt)),
        "an attempt that reported reads nothing more"
    );
    w.age_retry(run, "save").await;
    w.scheduler.tick().await;
    let second = {
        let mut found = None;
        for _ in 0..20 {
            let tasks = workers
                .poll(&runner, registered.worker_id, &token, 8, Some(TRACE))
                .await
                .expect("poll");
            if let Some(task) = tasks.into_iter().find(|candidate| candidate.run_id == run) {
                found = Some(task);
                break;
            }
        }
        found.expect("the second attempt is leased")
    };
    assert_eq!(second.fence.attempt, 2);
    assert!(
        matches!(gate.read(&second, named).await, Err(AppError::NotFound)),
        "the pin is the first attempt's: the second attempt has none of its own"
    );
    assert!(
        matches!(gate.read(&task, named).await, Err(AppError::StaleAttempt)),
        "and the first attempt's token reads nothing"
    );
    pin(w, run, "save", second.fence, &[named]).await;
    gate.read(&second, named)
        .await
        .expect("pinned by the lease of the second attempt");

    // Lapsed: the lease is not unexpired.
    w.age_lease(run, "save").await;
    assert!(
        matches!(gate.read(&second, named).await, Err(AppError::StaleAttempt)),
        "a lease that lapsed reads nothing"
    );

    // Cancelled: a run that was told to stop is not given another byte.
    let third = leased("lake-read-start-00003").await;
    let blob = committed_output(w, third.run_id).await.id();
    pin(w, third.run_id, "save", third.fence, &[blob]).await;
    gate.read(&third, blob).await.expect("before the cancel");
    w.operate(third.run_id, RunOperation::Cancel)
        .await
        .expect("cancel the run");
    let after = gate.read(&third, blob).await;
    assert!(
        matches!(after, Err(AppError::StaleAttempt)),
        "a run that was cancelled reads nothing more: {after:?}"
    );

    // The budget of lake attempts is counted by the scenario that follows.
    gate.release(other_run).await;
    gate.release(third).await;
}
