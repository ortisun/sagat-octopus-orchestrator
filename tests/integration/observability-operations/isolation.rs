//! The tenant boundary in every read and write this feature added to the store: what a tenant
//! cannot see of another's, and what the store itself refuses when the service in front of it did
//! not (T008-007, NFR-008-SEC).

use std::time::Duration;

use sagat_application::error::AppError;
use sagat_application::operations::{
    AlertReader, AuditFilter, AuditStore, DrainRecord, Thresholds, WorkerAdmin,
};
use sagat_application::telemetry::TelemetryStore;
use sagat_domain::audit::{AuditAction, AuditRecord, ResourceType};
use sagat_scheduler::{Monitor, MonitorSettings};
use serde_json::json;

use crate::support::engine::{World, activity, ctx, document, world};
use crate::support::now;

async fn finished_run(w: &World, name: &str) -> sagat_domain::execution::RunId {
    let (wf, _) = w.publish(document(name, "a", vec![activity("a")])).await;
    let run = w.start(wf, &json!({})).await;
    let session = w.session(&["act.a"]).await;
    let task = w.one_task(&session).await;
    w.complete(&session, &task, &json!({"ok": true}))
        .await
        .expect("complete");
    run
}

#[tokio::test]
async fn the_telemetry_and_the_attempts_of_a_run_are_those_of_its_tenant_only() {
    let w = world().await;
    let run = finished_run(&w, "isolated-telemetry").await;
    let ours = w.admin.tenant_id();
    let theirs = w.other_admin.tenant_id();

    assert!(
        w.store
            .run_telemetry(ours, run)
            .await
            .expect("read")
            .is_some()
    );
    assert!(
        w.store
            .run_telemetry(theirs, run)
            .await
            .expect("read")
            .is_none(),
        "the store does not answer for a run of another tenant"
    );
    assert!(matches!(
        w.runs.telemetry(&w.other_admin, run, &ctx()).await,
        Err(AppError::NotFound)
    ));
    assert!(matches!(
        w.runs.attempts(&w.other_admin, run, &ctx()).await,
        Err(AppError::NotFound)
    ));
    // The same call for a run that exists nowhere gets the same answer.
    let nowhere = sagat_domain::execution::RunId::generate();
    assert!(matches!(
        w.runs.telemetry(&w.other_admin, nowhere, &ctx()).await,
        Err(AppError::NotFound)
    ));
    assert_eq!(
        w.runs
            .attempts(&w.admin, run, &ctx())
            .await
            .expect("attempts")
            .len(),
        1
    );
}

#[tokio::test]
async fn the_audit_trail_read_by_one_tenant_holds_nothing_of_another() {
    let w = world().await;
    let ours = finished_run(&w, "audited-ours").await;
    let (theirs_wf, _) = (
        w.publish_as_other_tenant(document("audited-theirs", "a", vec![activity("a")]))
            .await,
        (),
    );
    let theirs = w
        .start_as_other_tenant(theirs_wf, &json!({}))
        .await
        .expect("start");
    let reading = AuditRecord::by(
        &w.admin,
        AuditAction::AuditRead,
        ResourceType::Tenant,
        w.admin.tenant_id().to_string(),
        None,
        now(),
    );

    let read = |tenant| {
        let w = &w;
        let reading = &reading;
        async move {
            w.store
                .read_audit(tenant, &AuditFilter::default(), None, 200, reading)
                .await
                .expect("audit")
        }
    };
    let ours_seen = read(w.admin.tenant_id()).await;
    let theirs_seen = read(w.other_admin.tenant_id()).await;
    assert!(
        ours_seen.iter().any(|r| r.resource_id == ours.to_string()),
        "the tenant sees what it did"
    );
    assert!(
        !ours_seen
            .iter()
            .any(|r| r.resource_id == theirs.to_string()),
        "and nothing of the other"
    );
    assert!(
        theirs_seen
            .iter()
            .any(|r| r.resource_id == theirs.to_string())
    );
    assert!(
        !theirs_seen
            .iter()
            .any(|r| r.resource_id == ours.to_string()),
        "the other tenant sees none of ours"
    );
    // Filtering by a resource of the other tenant finds nothing: the filter narrows what is
    // visible and cannot widen it.
    let probe = AuditFilter {
        resource_id: Some(theirs.to_string()),
        ..AuditFilter::default()
    };
    assert!(
        w.store
            .read_audit(w.admin.tenant_id(), &probe, None, 50, &reading)
            .await
            .expect("audit")
            .is_empty()
    );
}

#[tokio::test]
async fn alerts_belong_to_the_namespace_and_the_tenant_whose_work_is_stuck() {
    let w = world().await;
    // Each tenant has a step ready that nobody serves.
    let (ours_wf, _) = w
        .publish(document("stuck-ours", "a", vec![activity("a")]))
        .await;
    w.start(ours_wf, &json!({})).await;
    let theirs_wf = w
        .publish_as_other_tenant(document("stuck-theirs", "a", vec![activity("a")]))
        .await;
    w.start_as_other_tenant(theirs_wf, &json!({}))
        .await
        .expect("start");

    let monitor = Monitor::new(
        w.store.clone(),
        MonitorSettings {
            thresholds: Thresholds {
                unclaimed: Duration::ZERO,
                ..Thresholds::default()
            },
            ..MonitorSettings::default()
        },
    );
    assert!(
        monitor.run_once().await.fired >= 2,
        "one for each namespace"
    );

    let list = |tenant, namespace| {
        let w = &w;
        async move {
            w.store
                .list_alerts(tenant, namespace, None, None, 50)
                .await
                .expect("alerts")
        }
    };
    let ours = list(w.admin.tenant_id(), w.namespace.id()).await;
    assert_eq!(ours.len(), 1, "{ours:?}");
    assert!(ours.iter().all(|a| a.tenant_id == w.admin.tenant_id()));
    let theirs = list(w.other_admin.tenant_id(), w.other_namespace.id()).await;
    assert_eq!(theirs.len(), 1, "{theirs:?}");
    assert!(
        theirs
            .iter()
            .all(|a| a.tenant_id == w.other_admin.tenant_id())
    );
    // Asking with one tenant's identity for the other's namespace gives nothing.
    assert!(
        list(w.admin.tenant_id(), w.other_namespace.id())
            .await
            .is_empty()
    );
    assert!(
        list(w.other_admin.tenant_id(), w.namespace.id())
            .await
            .is_empty()
    );
}

#[tokio::test]
async fn a_drain_is_kept_per_tenant_and_namespace_and_the_store_refuses_a_foreign_namespace() {
    let w = world().await;
    let drain = DrainRecord {
        subject: "worker-1".to_owned(),
        reason: "rolling the deploy".to_owned(),
        drained_by: "olivia".to_owned(),
        drained_at: now(),
    };
    let audit = AuditRecord::by(
        &w.admin,
        AuditAction::WorkerDrained,
        ResourceType::Worker,
        "worker-1",
        None,
        now(),
    );
    w.store
        .drain_worker(w.admin.tenant_id(), w.namespace.id(), &drain, &audit)
        .await
        .expect("drain");
    assert_eq!(
        w.store
            .list_drains(w.admin.tenant_id(), w.namespace.id())
            .await
            .expect("drains")
            .len(),
        1
    );
    for (tenant, namespace) in [
        (w.other_admin.tenant_id(), w.other_namespace.id()),
        (w.other_admin.tenant_id(), w.namespace.id()),
        (w.admin.tenant_id(), w.other_namespace.id()),
    ] {
        assert!(
            w.store
                .list_drains(tenant, namespace)
                .await
                .expect("drains")
                .is_empty()
        );
    }
    // The service checks the namespace first; if it did not, the foreign key would say no.
    let foreign = AuditRecord::by(
        &w.other_admin,
        AuditAction::WorkerDrained,
        ResourceType::Worker,
        "worker-1",
        None,
        now(),
    );
    assert!(
        w.store
            .drain_worker(
                w.other_admin.tenant_id(),
                w.namespace.id(),
                &drain,
                &foreign
            )
            .await
            .is_err(),
        "a drain of another tenant's namespace is refused by the store itself"
    );
    // Undraining it from the other tenant frees nothing of ours.
    assert!(
        !w.store
            .undrain_worker(
                w.other_admin.tenant_id(),
                w.namespace.id(),
                "worker-1",
                &foreign
            )
            .await
            .unwrap_or(false)
    );
    assert_eq!(
        w.store
            .list_drains(w.admin.tenant_id(), w.namespace.id())
            .await
            .expect("drains")
            .len(),
        1
    );
}

#[tokio::test]
async fn reading_the_trail_is_recorded_again_once_the_window_of_the_same_reading_has_passed() {
    let w = world().await;
    let read_at = |after: time::Duration| {
        let w = &w;
        async move {
            let reading = AuditRecord::by(
                &w.admin,
                AuditAction::AuditRead,
                ResourceType::Tenant,
                w.admin.tenant_id().to_string(),
                None,
                now() + after,
            )
            .with_reason("everything");
            w.store
                .read_audit(
                    w.admin.tenant_id(),
                    &AuditFilter::default(),
                    None,
                    10,
                    &reading,
                )
                .await
                .expect("audit");
        }
    };
    let recorded = || async {
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM audit_records WHERE action = 'audit.read' AND tenant_id = $1",
        )
        .bind(w.admin.tenant_id().as_uuid())
        .fetch_one(&w.db.pool)
        .await
        .expect("count")
    };
    read_at(time::Duration::ZERO).await;
    read_at(time::Duration::minutes(1)).await;
    assert_eq!(
        recorded().await,
        1,
        "the same reading again within the window adds nothing"
    );
    read_at(time::Duration::minutes(30)).await;
    assert_eq!(
        recorded().await,
        2,
        "half an hour later the same reading is a reading again, and is on the trail"
    );
    read_at(time::Duration::minutes(31)).await;
    assert_eq!(recorded().await, 2);
}
