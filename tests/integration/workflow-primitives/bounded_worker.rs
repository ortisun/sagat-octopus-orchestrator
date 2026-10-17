//! Worker operations use exact-path state while retaining current authority and report fences.

use std::sync::Arc;

use sagat_application::clock::SystemClock;
use sagat_application::entropy::SystemEntropy;
use sagat_application::error::AppError;
use sagat_application::execution::{DefinitionCache, Engine};
use sagat_application::worker::{Capabilities, WorkerService, WorkerSettings};
use sagat_domain::execution::{ExecutionInstanceId, InstancePath};
use sagat_domain::identity::{Permission, Principal, Role};
use sagat_domain::workflow::NodeId;
use sagat_infra_ir::JsonEvaluator;
use sagat_infra_postgres::PgRunStore;
use sagat_scheduler::{Keeper, KeeperSettings, Placement, Scheduler, SchedulerSettings};
use serde_json::json;
use uuid::Uuid;

use crate::scoped_worker::{Fixture, ScopedWorkers, StructuredJsonIrCodec, fixture};
use crate::support::engine::{activity, document, subject_of, task_ref, world};
use crate::support::eventually;

pub(super) async fn runtime_workers(fixture: &Fixture, connections: u32) -> ScopedWorkers {
    let store = PgRunStore::new(
        fixture.world.db.runtime_pool_with(connections).await,
        DefinitionCache::new(Arc::new(StructuredJsonIrCodec), 8),
    )
    .with_scoped_worker_protocol();
    WorkerService::new(
        store,
        SystemClock,
        Engine::new(JsonEvaluator),
        SystemEntropy,
        WorkerSettings::default(),
    )
    .with_vault(fixture.world.keys.cipher())
}

#[tokio::test]
async fn a_scoped_claim_waiting_for_binding_authority_holds_no_session_row() {
    assert_claim_authority_after_binding_wait(false).await;
}

#[tokio::test]
async fn a_scoped_claim_rechecks_current_role_after_binding_contention() {
    assert_claim_authority_after_binding_wait(true).await;
}

async fn assert_claim_authority_after_binding_wait(revoke_role: bool) {
    let fixture = fixture("bounded-claim-authority").await;
    let workers = Arc::new(runtime_workers(&fixture, 2).await);
    let mut guard = fixture.world.db.pool.begin().await.expect("binding guard");
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
        .bind(format!(
            "binding:{}:{}:{}",
            fixture.world.admin.tenant_id(),
            fixture.world.namespace.id(),
            fixture.session.principal.subject()
        ))
        .execute(&mut *guard)
        .await
        .expect("hold binding replacement authority");
    let caller = workers.clone();
    let principal = fixture.session.principal.clone();
    let worker_id = fixture.session.worker_id;
    let token = fixture.session.token.clone();
    let poll =
        tokio::spawn(async move { caller.poll(&principal, worker_id, &token, 2, None).await });
    eventually(
        "runtime claim waits for current binding authority",
        || async {
            let waiting: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM pg_stat_activity WHERE datname=current_database() \
             AND usename='sagat_app' AND wait_event_type='Lock' \
             AND query LIKE '%pg_advisory_xact_lock_shared%'",
            )
            .fetch_one(&fixture.world.db.pool)
            .await
            .expect("binding wait barrier");
            (waiting == 1).then_some(())
        },
    )
    .await;
    sqlx::query("SET LOCAL lock_timeout='500ms'")
        .execute(&mut *guard)
        .await
        .expect("bounded guard");
    if revoke_role {
        sqlx::query("UPDATE memberships SET role='viewer' WHERE tenant_id=$1 AND subject=$2")
            .bind(fixture.session.principal.tenant_id().as_uuid())
            .bind(fixture.session.principal.subject().as_str())
            .execute(&mut *guard)
            .await
            .expect("revoke the cached principal's current role");
    } else {
        sqlx::query(
            "DELETE FROM worker_bindings WHERE tenant_id=$1 AND namespace_id=$2 AND subject=$3",
        )
        .bind(fixture.world.admin.tenant_id().as_uuid())
        .bind(fixture.world.namespace.id().as_uuid())
        .bind(fixture.session.principal.subject().as_str())
        .execute(&mut *guard)
        .await
        .expect("withdraw bindings");
        sqlx::query("DELETE FROM worker_sessions WHERE tenant_id=$1 AND id=$2")
            .bind(fixture.world.admin.tenant_id().as_uuid())
            .bind(worker_id)
            .execute(&mut *guard)
            .await
            .expect("binding replacement can close its sessions without a lock cycle");
    }
    guard.commit().await.expect("commit authority withdrawal");
    assert!(
        poll.await
            .expect("poll task")
            .expect("authority refusal leaves work ready")
            .is_empty()
    );
    let attempts: i64 =
        sqlx::query_scalar("SELECT sum(attempt)::bigint FROM steps WHERE run_id=$1")
            .bind(fixture.run.as_uuid())
            .fetch_one(&fixture.world.db.pool)
            .await
            .expect("no lease spent");
    assert_eq!(attempts, 0);
}

#[tokio::test]
async fn a_later_path_failure_still_delivers_every_committed_lease() {
    let fixture = fixture("bounded-worker-partial-claim").await;
    let workers = runtime_workers(&fixture, 2).await;
    sqlx::raw_sql(
        "CREATE FUNCTION reject_later_path_lease() RETURNS trigger LANGUAGE plpgsql AS $$ \
         BEGIN \
           IF NEW.status='LEASED' AND EXISTS (SELECT 1 FROM steps \
             WHERE tenant_id=NEW.tenant_id AND run_id=NEW.run_id AND status='LEASED' \
               AND instance_path_indices<>NEW.instance_path_indices) THEN \
             RAISE EXCEPTION 'injected later path lease failure'; \
           END IF; \
           RETURN NEW; \
         END $$; \
         CREATE TRIGGER reject_later_path_lease BEFORE UPDATE ON steps \
           FOR EACH ROW EXECUTE FUNCTION reject_later_path_lease(); \
         REVOKE SELECT ON iteration_checkpoints FROM sagat_app;",
    )
    .execute(&fixture.world.db.pool)
    .await
    .expect("inject a failure only after an earlier path committed");
    let tasks = workers
        .poll(
            &fixture.session.principal,
            fixture.session.worker_id,
            &fixture.session.token,
            2,
            None,
        )
        .await
        .expect("the already committed lease token must reach its worker");
    assert_eq!(tasks.len(), 1);
    assert!(tasks[0].idempotency_key.starts_with("scope:"));
    let rows: Vec<(String, i32)> = sqlx::query_as(
        "SELECT status,attempt FROM steps WHERE run_id=$1 AND node='a' \
         AND cardinality(instance_path_nodes)=1 ORDER BY instance_path_indices",
    )
    .bind(fixture.run.as_uuid())
    .fetch_all(&fixture.world.db.pool)
    .await
    .expect("both independent path projections");
    assert_eq!(
        rows.iter()
            .filter(|(status, attempt)| status == "LEASED" && *attempt == 1)
            .count(),
        1
    );
    assert_eq!(
        rows.iter()
            .filter(|(status, attempt)| status == "READY" && *attempt == 0)
            .count(),
        1
    );
    workers
        .heartbeat(&fixture.session.principal, &task_ref(&tasks[0]))
        .await
        .expect("returned token really owns its committed lease");
    sqlx::query("GRANT SELECT ON iteration_checkpoints TO sagat_app")
        .execute(&fixture.world.db.pool)
        .await
        .expect("restore replay oracle access");
    fixture
        .store
        .verify_run(fixture.world.admin.tenant_id(), fixture.run)
        .await
        .expect("later rollback leaves the earlier journal/projection intact");
}

#[tokio::test]
async fn scoped_claims_need_no_unrelated_iteration_history() {
    let fixture = fixture("bounded-worker-claim").await;
    let workers = runtime_workers(&fixture, 2).await;
    sqlx::query("REVOKE SELECT ON iteration_checkpoints FROM sagat_app")
        .execute(&fixture.world.db.pool)
        .await
        .expect("withhold unrelated iteration history before discovery");
    let tasks = workers
        .poll(
            &fixture.session.principal,
            fixture.session.worker_id,
            &fixture.session.token,
            1,
            None,
        )
        .await
        .expect("claim requires only the addressed current path");
    assert_eq!(tasks.len(), 1);
    assert!(!tasks[0].instance.path().is_root());
    assert_eq!(tasks[0].fence.attempt, 1);
    let started: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM run_events WHERE run_id=$1 AND event_type='step.leased'",
    )
    .bind(fixture.run.as_uuid())
    .fetch_one(&fixture.world.db.pool)
    .await
    .expect("persisted lease");
    assert_eq!(started, 1);
    sqlx::query("GRANT SELECT ON iteration_checkpoints TO sagat_app")
        .execute(&fixture.world.db.pool)
        .await
        .expect("restore replay oracle access");
    fixture
        .store
        .verify_run(fixture.world.admin.tenant_id(), fixture.run)
        .await
        .expect("bounded claim agrees with full journal replay");
}

#[tokio::test]
async fn a_scoped_heartbeat_uses_its_path_and_preserves_late_report_and_binding_fences() {
    let fixture = fixture("bounded-worker-heartbeat").await;
    let workers = runtime_workers(&fixture, 8).await;
    let task = workers
        .poll(
            &fixture.session.principal,
            fixture.session.worker_id,
            &fixture.session.token,
            1,
            None,
        )
        .await
        .expect("initial scoped claim")
        .pop()
        .expect("one scoped lease");
    sqlx::query("REVOKE SELECT ON iteration_checkpoints FROM sagat_app")
        .execute(&fixture.world.db.pool)
        .await
        .expect("withhold unrelated iteration history");
    let beat = workers
        .heartbeat(&fixture.session.principal, &task_ref(&task))
        .await
        .expect("heartbeat needs only its exact path");
    assert!(!beat.cancel_requested);
    assert!(
        beat.lease_expires_at
            .is_some_and(|at| at >= task.expires_at)
    );
    let next = workers
        .heartbeat(&fixture.session.principal, &task_ref(&task))
        .await
        .expect("repeat heartbeat");
    assert!(next.lease_expires_at >= beat.lease_expires_at);
    let started: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM run_events WHERE run_id=$1 AND event_type='step.started'",
    )
    .bind(fixture.run.as_uuid())
    .fetch_one(&fixture.world.db.pool)
    .await
    .expect("heartbeat journal");
    assert_eq!(started, 1, "renewal must not duplicate the started event");
    let mut wrong = task_ref(&task);
    wrong.step_id = Some(Uuid::now_v7());
    assert_eq!(
        workers.heartbeat(&fixture.session.principal, &wrong).await,
        Err(AppError::StaleAttempt)
    );
    let mut stale = task_ref(&task);
    stale.fence.generation += 1;
    assert_eq!(
        workers.heartbeat(&fixture.session.principal, &stale).await,
        Err(AppError::StaleAttempt)
    );
    let missing = ExecutionInstanceId::in_path(
        InstancePath::root()
            .child(NodeId::parse("items").expect("scope"), 999)
            .expect("path"),
        task.instance.node().clone(),
    );
    let mut misdirected = task_ref(&task);
    misdirected.instance = &missing;
    assert_eq!(
        workers
            .heartbeat(&fixture.session.principal, &misdirected)
            .await,
        Err(AppError::StaleAttempt)
    );
    let outsider = Principal::new(
        fixture.world.other_admin.tenant_id(),
        subject_of("foreign-heartbeat"),
        Role::Worker,
    );
    assert_eq!(
        workers.heartbeat(&outsider, &task_ref(&task)).await,
        Err(AppError::StaleAttempt)
    );
    // Completion has not been converted by this heartbeat slice; give its existing oracle access.
    sqlx::query("GRANT SELECT ON iteration_checkpoints TO sagat_app")
        .execute(&fixture.world.db.pool)
        .await
        .expect("allow the existing completion path");
    workers
        .complete(
            &fixture.session.principal,
            &task_ref(&task),
            b"{}",
            "bounded-heartbeat-complete",
        )
        .await
        .expect("complete the original lease");
    sqlx::query("REVOKE SELECT ON iteration_checkpoints FROM sagat_app")
        .execute(&fixture.world.db.pool)
        .await
        .expect("late heartbeat still needs only its path");
    let late = workers
        .heartbeat(&fixture.session.principal, &task_ref(&task))
        .await
        .expect("a heartbeat already in flight when this lease reported is harmless");
    assert!(!late.cancel_requested);
    assert_eq!(late.lease_expires_at, None);
    sqlx::query("UPDATE memberships SET role='viewer' WHERE tenant_id=$1 AND subject=$2")
        .bind(fixture.session.principal.tenant_id().as_uuid())
        .bind(fixture.session.principal.subject().as_str())
        .execute(&fixture.world.db.pool)
        .await
        .expect("revoke the late reporter's role");
    assert_eq!(
        workers
            .heartbeat(&fixture.session.principal, &task_ref(&task))
            .await,
        Err(AppError::Forbidden {
            permission: Permission::WorkerExecute
        })
    );
    sqlx::query("UPDATE memberships SET role='worker' WHERE tenant_id=$1 AND subject=$2")
        .bind(fixture.session.principal.tenant_id().as_uuid())
        .bind(fixture.session.principal.subject().as_str())
        .execute(&fixture.world.db.pool)
        .await
        .expect("restore role before independent binding revocation");
    fixture
        .world
        .bindings
        .unbind(
            &fixture.world.admin,
            fixture.world.namespace.id(),
            fixture.session.principal.subject().as_str(),
            &crate::support::engine::ctx(),
        )
        .await
        .expect("revoke the binding");
    assert_eq!(
        workers
            .heartbeat(&fixture.session.principal, &task_ref(&task))
            .await,
        Err(AppError::NotBound),
        "a late heartbeat never bypasses current binding authority"
    );
    sqlx::query("GRANT SELECT ON iteration_checkpoints TO sagat_app")
        .execute(&fixture.world.db.pool)
        .await
        .expect("restore the ordinary history grant");
    fixture
        .store
        .verify_run(fixture.world.admin.tenant_id(), fixture.run)
        .await
        .expect("heartbeat and completion projections agree with the journal");
}

#[tokio::test]
async fn concurrent_scoped_heartbeats_finish_with_two_connections_without_history() {
    let fixture = fixture("bounded-heartbeat-small-pool").await;
    let workers = Arc::new(runtime_workers(&fixture, 2).await);
    let task = workers
        .poll(
            &fixture.session.principal,
            fixture.session.worker_id,
            &fixture.session.token,
            1,
            None,
        )
        .await
        .expect("initial scoped lease")
        .pop()
        .expect("one task");
    sqlx::query("REVOKE SELECT ON iteration_checkpoints FROM sagat_app")
        .execute(&fixture.world.db.pool)
        .await
        .expect("withhold unrelated history");
    let mut guard = fixture.world.db.pool.begin().await.expect("run lock guard");
    sqlx::query("SELECT id FROM runs WHERE id=$1 FOR UPDATE")
        .bind(fixture.run.as_uuid())
        .execute(&mut *guard)
        .await
        .expect("hold run");
    let mut calls = Vec::new();
    for _ in 0..12 {
        let workers = workers.clone();
        let principal = fixture.session.principal.clone();
        let task = task.clone();
        calls.push(tokio::spawn(async move {
            workers.heartbeat(&principal, &task_ref(&task)).await
        }));
    }
    eventually(
        "both runtime connections wait for the same run lock",
        || async {
            let count: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM pg_stat_activity WHERE datname=current_database() \
             AND usename='sagat_app' AND state='active' AND wait_event_type='Lock' \
             AND query LIKE '%FROM runs%'",
            )
            .fetch_one(&fixture.world.db.pool)
            .await
            .expect("two-connection barrier");
            (count >= 2).then_some(())
        },
    )
    .await;
    guard.rollback().await.expect("release run");
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        for call in calls {
            let beat = call
                .await
                .expect("heartbeat task")
                .expect("every heartbeat finishes");
            assert!(!beat.cancel_requested);
            assert!(beat.lease_expires_at.is_some());
        }
    })
    .await
    .expect("a held transaction must never wait for a second pool connection");
    let started: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM run_events WHERE run_id=$1 AND event_type='step.started'",
    )
    .bind(fixture.run.as_uuid())
    .fetch_one(&fixture.world.db.pool)
    .await
    .expect("started events");
    assert_eq!(started, 1);
    sqlx::query("GRANT SELECT ON iteration_checkpoints TO sagat_app")
        .execute(&fixture.world.db.pool)
        .await
        .expect("restore history grant");
    fixture
        .store
        .verify_run(fixture.world.admin.tenant_id(), fixture.run)
        .await
        .expect("concurrent heartbeats preserve replay");
}

#[tokio::test]
async fn a_late_scoped_heartbeat_needs_no_history_after_its_collection_and_run_finish() {
    let world = world().await;
    crate::limits::allow_iterations(&world, 1).await;
    let store = PgRunStore::new(world.db.runtime_pool_with(8).await, world.cache.clone())
        .with_scoped_worker_protocol();
    let workers = WorkerService::new(
        store.clone(),
        SystemClock,
        Engine::new(JsonEvaluator),
        SystemEntropy,
        WorkerSettings::default(),
    );
    let placement = Placement::new("completed-scope-heartbeat");
    Keeper::new(
        store.clone(),
        placement.clone(),
        KeeperSettings {
            ttl: std::time::Duration::from_secs(3600),
        },
    )
    .round()
    .await
    .expect("acquire the collection scheduler's shard fences");
    let scheduler = Scheduler::new(
        store.clone(),
        Engine::new(JsonEvaluator),
        SystemEntropy,
        SchedulerSettings::default(),
    )
    .with_placement(placement);
    let (workflow, _) = world
        .publish(json!({
            "apiVersion":"sagat-octopus-orchestrator.io/v1", "kind":"Workflow",
            "metadata":{"name":"completed-scope-heartbeat"}, "entrypoint":"items",
            "limits":{"maxInstances":8,"maxDepth":1},
            "bodies":{"line":{"entrypoint":"work","nodes":[
                {"id":"work","type":"activity","activity":"late.work","next":"returned"},
                {"id":"returned","type":"return","result":"steps.work.output"}
            ]}},
            "nodes":[
                {"id":"items","type":"forEach","items":"input.items","body":"line",
                 "result":"bodyResult","maxItems":1,"maxConcurrency":1,"next":"done"},
                {"id":"done","type":"end"}
            ]
        }))
        .await;
    let session = workers
        .register(
            &world.worker,
            "payments",
            "late-scoped-heartbeat",
            &Capabilities {
                activities: vec!["late.work".into()],
                languages: vec![],
                pools: vec![],
                max_inflight: 1,
                protocol_version: "1.5".into(),
            },
        )
        .await
        .expect("scoped worker");
    let run = world.start(workflow, &json!({"items":[1]})).await;
    let mut task = None;
    for _ in 0..8 {
        let report = scheduler.tick().await;
        assert_eq!(report.failed, 0, "{report:?}");
        assert_eq!(report.fenced, 0, "{report:?}");
        task = workers
            .poll(
                &world.worker,
                session.worker_id,
                session.session_token.expose(),
                1,
                None,
            )
            .await
            .expect("scoped poll")
            .pop();
        if task.is_some() {
            break;
        }
    }
    let task = task.expect("one scoped lease");
    assert!(!task.instance.path().is_root());
    workers
        .complete(
            &world.worker,
            &task_ref(&task),
            b"{}",
            "completed-scope-heartbeat",
        )
        .await
        .expect("complete the member");
    for _ in 0..8 {
        let report = scheduler.tick().await;
        assert_eq!(report.failed, 0, "{report:?}");
        assert_eq!(report.fenced, 0, "{report:?}");
        if world.run_status(run).await.as_str() == "SUCCEEDED" {
            break;
        }
    }
    assert_eq!(world.run_status(run).await.as_str(), "SUCCEEDED");
    sqlx::query("REVOKE SELECT ON iteration_checkpoints FROM sagat_app")
        .execute(&world.db.pool)
        .await
        .expect("withhold closed-scope history");
    let late = workers
        .heartbeat(&world.worker, &task_ref(&task))
        .await
        .expect("a matching late heartbeat remains harmless after terminal drain");
    assert!(!late.cancel_requested);
    assert_eq!(late.lease_expires_at, None);
    sqlx::query("GRANT SELECT ON iteration_checkpoints TO sagat_app")
        .execute(&world.db.pool)
        .await
        .expect("restore ordinary history grant");
    store
        .verify_run(world.admin.tenant_id(), run)
        .await
        .expect("terminal journal replay");
}

#[tokio::test]
async fn a_root_heartbeat_waiting_for_the_run_lock_rechecks_current_membership() {
    let world = world().await;
    let (workflow, _) = world
        .publish(document(
            "heartbeat-role-race",
            "work",
            vec![activity("work")],
        ))
        .await;
    let run = world.start(workflow, &serde_json::json!({})).await;
    let session = world.session(&["act.work"]).await;
    let task = world.one_task(&session).await;
    let workers = Arc::new(WorkerService::new(
        world.store.clone(),
        SystemClock,
        Engine::new(JsonEvaluator),
        SystemEntropy,
        WorkerSettings::default(),
    ));
    let mut guard = world.db.pool.begin().await.expect("run lock guard");
    sqlx::query("SELECT id FROM runs WHERE id=$1 FOR UPDATE")
        .bind(run.as_uuid())
        .execute(&mut *guard)
        .await
        .expect("hold the run before the heartbeat");
    let principal = world.worker.clone();
    let heartbeat_task = task.clone();
    let waiter = tokio::spawn(async move {
        workers
            .heartbeat(&principal, &task_ref(&heartbeat_task))
            .await
    });
    eventually("heartbeat actually waits for the run lock", || async {
        sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE datname=current_database() \
             AND state='active' AND wait_event_type='Lock' AND query LIKE '%FROM runs%')",
        )
        .fetch_one(&world.db.pool)
        .await
        .expect("run lock barrier")
        .then_some(())
    })
    .await;
    sqlx::query("UPDATE memberships SET role='viewer' WHERE tenant_id=$1 AND subject=$2")
        .bind(world.worker.tenant_id().as_uuid())
        .bind(world.worker.subject().as_str())
        .execute(&world.db.pool)
        .await
        .expect("revoke execution while the heartbeat waits");
    guard.rollback().await.expect("release run lock");
    assert_eq!(
        waiter.await.expect("heartbeat waiter"),
        Err(AppError::Forbidden {
            permission: Permission::WorkerExecute
        })
    );
    assert_eq!(
        world
            .workers
            .complete(
                &world.worker,
                &task_ref(&task),
                b"{}",
                "revoked-role-complete"
            )
            .await
            .map(|_| ()),
        Err(AppError::Forbidden {
            permission: Permission::WorkerExecute
        })
    );
    assert_eq!(
        world
            .workers
            .fail(
                &world.worker,
                &task_ref(&task),
                "SAFE_ERROR",
                false,
                "revoked-role-fail"
            )
            .await
            .map(|_| ()),
        Err(AppError::Forbidden {
            permission: Permission::WorkerExecute
        })
    );
    let started: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM run_events WHERE run_id=$1 AND event_type='step.started'",
    )
    .bind(run.as_uuid())
    .fetch_one(&world.db.pool)
    .await
    .expect("refused heartbeat changes no execution state");
    assert_eq!(started, 0);
}
