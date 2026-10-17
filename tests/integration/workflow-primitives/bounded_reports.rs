//! Worker report restoration and current authority before returning a durable receipt.
use sagat_application::error::AppError;
use sagat_application::worker::ClaimedTask;
use sagat_domain::identity::Permission;
use serde_json::json;
use uuid::Uuid;

use crate::bounded_worker::runtime_workers;
use crate::scoped_worker::fixture;
use crate::support::engine::{Session, World, activity, document, task_ref, world};

#[tokio::test]
async fn scoped_completion_and_its_receipt_need_no_unrelated_iteration_history() {
    assert_scoped_report_without_history(false).await;
}

#[tokio::test]
async fn scoped_failure_and_its_receipt_need_no_unrelated_iteration_history() {
    assert_scoped_report_without_history(true).await;
}

async fn assert_scoped_report_without_history(failed: bool) {
    let fixture = fixture("bounded-scoped-report").await;
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
        .expect("bounded claim")
        .pop()
        .expect("one task");
    sqlx::query("REVOKE SELECT ON iteration_checkpoints FROM sagat_app")
        .execute(&fixture.world.db.pool)
        .await
        .expect("withhold unrelated history");
    let first = if failed {
        workers
            .fail(
                &fixture.session.principal,
                &task_ref(&task),
                "REJECTED",
                false,
                "bounded-report",
            )
            .await
    } else {
        workers
            .complete(
                &fixture.session.principal,
                &task_ref(&task),
                b"{\"ok\":true}",
                "bounded-report",
            )
            .await
    }
    .expect("an exact-path report must not restore unrelated iteration history");
    assert!(!first.already_applied);
    let replay = if failed {
        workers
            .fail(
                &fixture.session.principal,
                &task_ref(&task),
                "REJECTED",
                false,
                "bounded-report",
            )
            .await
    } else {
        workers
            .complete(
                &fixture.session.principal,
                &task_ref(&task),
                b"{\"ok\":true}",
                "bounded-report",
            )
            .await
    }
    .expect("a late receipt needs only its current exact path");
    assert!(replay.already_applied);
    assert_eq!(replay.command_id, first.command_id);
    assert_eq!(replay.step_status, first.step_status);
    assert_eq!(replay.run_status, first.run_status);
    sqlx::query("GRANT SELECT ON iteration_checkpoints TO sagat_app")
        .execute(&fixture.world.db.pool)
        .await
        .expect("restore replay oracle access");
    fixture
        .store
        .verify_run(fixture.world.admin.tenant_id(), fixture.run)
        .await
        .expect("reported journal and projection agree");
}

async fn completed_root() -> (World, Session, ClaimedTask) {
    let world = world().await;
    let (workflow, _) = world
        .publish(document("root-receipt-current", "a", vec![activity("a")]))
        .await;
    world.start(workflow, &json!({})).await;
    let session = world.session(&["act.a"]).await;
    let task = world.one_task(&session).await;
    world
        .workers
        .complete(&session.principal, &task_ref(&task), b"{}", "root-receipt")
        .await
        .expect("original completion");
    (world, session, task)
}

#[tokio::test]
async fn root_receipt_rechecks_current_membership() {
    let (world, session, task) = completed_root().await;
    sqlx::query("UPDATE memberships SET role='viewer' WHERE tenant_id=$1 AND subject=$2")
        .bind(session.principal.tenant_id().as_uuid())
        .bind(session.principal.subject().as_str())
        .execute(&world.db.pool)
        .await
        .expect("revoke current execution role");
    assert_eq!(
        world
            .workers
            .complete(&session.principal, &task_ref(&task), b"{}", "root-receipt")
            .await,
        Err(AppError::Forbidden {
            permission: Permission::WorkerExecute
        })
    );
}

#[tokio::test]
async fn root_receipt_rechecks_current_binding() {
    let (world, session, task) = completed_root().await;
    world
        .bindings
        .unbind(
            &world.admin,
            world.namespace.id(),
            session.principal.subject().as_str(),
            &crate::support::engine::ctx(),
        )
        .await
        .expect("revoke the binding");
    assert_eq!(
        world
            .workers
            .complete(&session.principal, &task_ref(&task), b"{}", "root-receipt")
            .await,
        Err(AppError::NotBound)
    );
}

#[tokio::test]
async fn root_receipt_checks_the_echoed_step_identity() {
    let (world, session, task) = completed_root().await;
    let mut forged = task_ref(&task);
    forged.step_id = Some(Uuid::now_v7());
    assert_eq!(
        world
            .workers
            .complete(&session.principal, &forged, b"{}", "root-receipt")
            .await,
        Err(AppError::StaleAttempt)
    );
}
