//! Exact-path capability checks must not restore the run's unrelated iteration history.
use sagat_application::error::AppError;
use sagat_domain::identity::{Permission, Principal, Role, TenantId};
use uuid::Uuid;

use crate::{bounded_worker::runtime_workers, scoped_worker::fixture, support::engine::task_ref};

#[tokio::test]
async fn scoped_secret_delivery_needs_no_unrelated_iteration_history() {
    let fixture = fixture("bounded-secret-capability").await;
    let workers = runtime_workers(&fixture, 2).await;
    let task = workers
        .poll(
            &fixture.session.principal,
            fixture.session.worker_id,
            &fixture.session.token,
            1,
            None,
        )
        .await
        .expect("claim")
        .pop()
        .expect("one scoped lease");
    sqlx::query("REVOKE SELECT ON iteration_checkpoints FROM sagat_app")
        .execute(&fixture.world.db.pool)
        .await
        .expect("deny unrelated history");
    let resolved = workers
        .resolve_secrets(&fixture.session.principal, &task_ref(&task), None)
        .await
        .expect("resolve only this path's declared secrets");
    assert_eq!(resolved.len(), 1);
    assert_eq!(resolved[0].name.as_str(), "scoped-key");
    assert_eq!(resolved[0].value.expose(), b"scoped-secret-value");
    sqlx::query("GRANT SELECT ON iteration_checkpoints TO sagat_app")
        .execute(&fixture.world.db.pool)
        .await
        .expect("restore replay oracle");
    fixture
        .store
        .verify_run(fixture.world.admin.tenant_id(), fixture.run)
        .await
        .expect("audited capability keeps journal and projections consistent");
}

#[tokio::test]
async fn scoped_blob_capabilities_need_no_unrelated_iteration_history() {
    let fixture = fixture("bounded-blob-capability").await;
    let workers = runtime_workers(&fixture, 2).await;
    let task = workers
        .poll(
            &fixture.session.principal,
            fixture.session.worker_id,
            &fixture.session.token,
            1,
            None,
        )
        .await
        .expect("claim")
        .pop()
        .expect("one scoped lease");
    sqlx::query("REVOKE SELECT ON iteration_checkpoints FROM sagat_app")
        .execute(&fixture.world.db.pool)
        .await
        .expect("deny unrelated history");
    let scope = workers
        .check_lease(&fixture.session.principal, &task_ref(&task))
        .await
        .expect("a read capability needs only the current exact path");
    assert_eq!(scope.access.tenant_id, fixture.world.admin.tenant_id());
    assert_eq!(scope.access.namespace_id, fixture.world.namespace.id());
    assert_eq!(scope.run_id, task.run_id);
    assert_eq!(Some(scope.step_id), task_ref(&task).step_id);
    assert_eq!(scope.attempt, task.fence.attempt);
    assert_eq!(scope.generation, task.fence.generation);
    assert_eq!(
        workers
            .check_blob_upload(&fixture.session.principal, &task_ref(&task))
            .await,
        Err(AppError::NotAllowed(
            "blob outputs with declared secrets require content scanning"
        ))
    );
    assert_eq!(
        workers
            .blob_report_scope(&fixture.session.principal, &task_ref(&task))
            .await
            .expect("scope lookup is bounded too"),
        scope
    );
    let mut forged = task_ref(&task);
    forged.step_id = Some(Uuid::now_v7());
    assert_eq!(
        workers
            .check_lease(&fixture.session.principal, &forged)
            .await,
        Err(AppError::StaleAttempt)
    );
    forged = task_ref(&task);
    forged.fence.generation += 1;
    assert_eq!(
        workers
            .check_lease(&fixture.session.principal, &forged)
            .await,
        Err(AppError::StaleAttempt)
    );
    let foreign = Principal::new(
        TenantId::new(Uuid::now_v7()),
        fixture.session.principal.subject().clone(),
        Role::Worker,
    );
    assert!(
        workers
            .check_lease(&foreign, &task_ref(&task))
            .await
            .is_err(),
        "another tenant cannot obtain the scope"
    );
    sqlx::query("UPDATE memberships SET role='viewer' WHERE tenant_id=$1 AND subject=$2")
        .bind(fixture.session.principal.tenant_id().as_uuid())
        .bind(fixture.session.principal.subject().as_str())
        .execute(&fixture.world.db.pool)
        .await
        .expect("withdraw current role while retaining the cached principal");
    assert_eq!(
        workers
            .check_lease(&fixture.session.principal, &task_ref(&task))
            .await,
        Err(AppError::Forbidden {
            permission: Permission::WorkerExecute
        })
    );
    assert_eq!(
        workers
            .blob_report_scope(&fixture.session.principal, &task_ref(&task))
            .await,
        Err(AppError::Forbidden {
            permission: Permission::WorkerExecute
        })
    );
}

#[tokio::test]
async fn scoped_secret_audits_distinguish_instances_and_collapse_only_their_own_retries() {
    let fixture = fixture("bounded-secret-audit-identity").await;
    let workers = runtime_workers(&fixture, 2).await;
    let tasks = workers
        .poll(
            &fixture.session.principal,
            fixture.session.worker_id,
            &fixture.session.token,
            2,
            None,
        )
        .await
        .expect("claim two distinct instances of node a");
    assert_eq!(tasks.len(), 2);
    for task in &tasks {
        for _ in 0..2 {
            assert_eq!(
                workers
                    .resolve_secrets(&fixture.session.principal, &task_ref(task), None)
                    .await
                    .expect("repeated authorized resolution")
                    .len(),
                1
            );
        }
    }
    let reasons: Vec<String> = sqlx::query_scalar(
        "SELECT reason FROM audit_records WHERE tenant_id=$1 AND action='secret.resolved'",
    )
    .bind(fixture.session.principal.tenant_id().as_uuid())
    .fetch_all(&fixture.world.db.pool)
    .await
    .expect("committed delivery audit");
    assert_eq!(
        reasons.len(),
        2,
        "distinct scoped deliveries cannot collapse together"
    );
    for task in &tasks {
        let identity = task_ref(task)
            .step_id
            .expect("scoped step UUID")
            .to_string();
        assert!(reasons.iter().any(|reason| reason.contains(&identity)));
    }
    assert!(
        reasons
            .iter()
            .all(|reason| !reason.contains("scoped-secret-value"))
    );
}
