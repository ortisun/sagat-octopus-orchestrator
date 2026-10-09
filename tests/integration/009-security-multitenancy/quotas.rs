//! Quotas (FR-009-006, US04): a hard limit judged against an exact count, taken in the transaction
//! that creates the thing counted, so racing replicas cannot overshoot it and one tenant's limit is
//! never another's.

use sagat_application::error::AppError;
use sagat_application::identity::RequestContext;
use sagat_application::quotas::StoredLimit;
use sagat_application::runs::RunOperation;
use sagat_domain::identity::{Principal, Role};
use sagat_domain::quota::{MAX_LIMIT, QuotaResource, QuotaSource};
use serde_json::json;

use crate::support::engine::{World, activity, child_node, ctx, document, subject_of, world};

async fn set(w: &World, resource: QuotaResource, value: u64) {
    w.quotas
        .set(&w.admin, None, resource, value, &ctx())
        .await
        .expect("set a tenant-wide quota");
}

async fn set_in_namespace(w: &World, resource: QuotaResource, value: u64) {
    w.quotas
        .set(&w.admin, Some(w.namespace.id()), resource, value, &ctx())
        .await
        .expect("set a namespace quota");
}

fn waiting_flow(name: &str) -> serde_json::Value {
    document(name, "a", vec![activity("a")])
}

// ------------------------------------------------------------- runs

#[tokio::test]
async fn a_namespace_cannot_start_more_runs_than_its_quota_and_a_finished_run_gives_its_place_back()
{
    let w = world().await;
    let (wf, _) = w.publish(waiting_flow("limited")).await;
    set_in_namespace(&w, QuotaResource::ActiveRuns, 3).await;

    let mut runs = Vec::new();
    for _ in 0..3 {
        runs.push(w.start(wf, &json!({})).await);
    }
    let refused = w.start_with(wf, &json!({}), "start-over-quota", None).await;
    assert_eq!(
        refused,
        Err(AppError::QuotaExceeded("active runs in the namespace"))
    );
    let counted: i64 = sqlx::query_scalar("SELECT count(*) FROM runs")
        .fetch_one(&w.db.pool)
        .await
        .expect("count");
    assert_eq!(counted, 3, "the refused run left nothing behind");

    // The view shows the exact use and where the limit comes from.
    let view = w
        .quotas
        .list(&w.admin, Some(w.namespace.id()), &ctx())
        .await
        .expect("list");
    let active = view
        .iter()
        .find(|q| q.resource == QuotaResource::ActiveRuns)
        .expect("active runs");
    assert_eq!(
        (active.limit, active.source, active.used),
        (3, QuotaSource::Namespace, Some(3))
    );

    // A run that ends frees its place at once.
    w.operate_run(runs[0], RunOperation::Cancel).await;
    w.start_with(wf, &json!({}), "start-after-cancel", None)
        .await
        .expect("a place was freed");
}

#[tokio::test]
async fn a_zero_quota_refuses_everything_and_removing_it_restores_the_default() {
    let w = world().await;
    let (wf, _) = w.publish(waiting_flow("zero")).await;
    set_in_namespace(&w, QuotaResource::ActiveRuns, 0).await;
    assert_eq!(
        w.start_with(wf, &json!({}), "start-with-zero", None).await,
        Err(AppError::QuotaExceeded("active runs in the namespace"))
    );
    let view = w
        .quotas
        .clear(
            &w.admin,
            Some(w.namespace.id()),
            QuotaResource::ActiveRuns,
            &ctx(),
        )
        .await
        .expect("clear");
    assert_eq!(
        (view.limit, view.source),
        (
            QuotaResource::ActiveRuns.default_limit(),
            QuotaSource::Default
        )
    );
    w.start(wf, &json!({})).await;
}

#[tokio::test]
async fn the_quota_of_one_tenant_never_affects_another() {
    let w = world().await;
    let (mine, _) = w.publish(waiting_flow("mine")).await;
    let theirs = w.publish_as_other_tenant(waiting_flow("theirs")).await;
    set_in_namespace(&w, QuotaResource::ActiveRuns, 1).await;
    w.start(mine, &json!({})).await;
    assert!(matches!(
        w.start_with(mine, &json!({}), "start-second", None).await,
        Err(AppError::QuotaExceeded(_))
    ));

    // The other tenant is at the platform default, whatever the first one did, and cannot even see
    // or change the first one's settings.
    for _ in 0..5 {
        w.start_as_other_tenant(theirs, &json!({}))
            .await
            .expect("the other tenant is unaffected");
    }
    let foreign = w
        .quotas
        .set(
            &w.other_admin,
            Some(w.namespace.id()),
            QuotaResource::ActiveRuns,
            1_000,
            &ctx(),
        )
        .await;
    assert_eq!(foreign.map(|_| ()), Err(AppError::NotFound));
    let still: Vec<StoredLimit> = {
        use sagat_application::quotas::QuotaStore;
        w.store
            .settings(w.admin.tenant_id(), Some(w.namespace.id()))
            .await
            .expect("settings")
    };
    assert_eq!(
        still,
        vec![StoredLimit {
            resource: QuotaResource::ActiveRuns,
            value: 1
        }],
        "the foreign attempt changed nothing"
    );
}

#[tokio::test]
async fn the_tenant_quota_counts_every_run_of_the_tenant() {
    let w = world().await;
    let (wf, _) = w.publish(waiting_flow("tenant-wide")).await;
    set(&w, QuotaResource::TenantActiveRuns, 2).await;
    let first = w.start(wf, &json!({})).await;
    w.start(wf, &json!({})).await;
    assert_eq!(
        w.start_with(wf, &json!({}), "start-third", None).await,
        Err(AppError::QuotaExceeded("active runs of the tenant"))
    );

    // A run that ends gives its place back to the tenant as well.
    w.operate_run(first, RunOperation::Cancel).await;
    w.start_with(wf, &json!({}), "start-after-end", None)
        .await
        .expect("the tenant has a free place again");
}

#[tokio::test]
async fn replicas_racing_for_the_last_places_never_overshoot_the_quota() {
    let w = world().await;
    let (wf, _) = w.publish(waiting_flow("race")).await;
    set_in_namespace(&w, QuotaResource::ActiveRuns, 5).await;
    let a = std::sync::Arc::new(w.replica().await);
    let b = std::sync::Arc::new(w.replica().await);
    let admin = w.admin.clone();

    let mut attempts = Vec::new();
    for i in 0..30 {
        let replica = if i % 2 == 0 { a.clone() } else { b.clone() };
        let admin = admin.clone();
        attempts.push(tokio::spawn(async move {
            let key = format!("race-start-{i:03}");
            replica
                .runs
                .start(
                    &admin,
                    &sagat_application::runs::StartRun {
                        namespace: "payments",
                        workflow: wf,
                        version: None,
                        input: b"{}",
                        correlation_id: None,
                        deadline: None,
                        key: &key,
                    },
                    &RequestContext::default(),
                )
                .await
                .map(|_| ())
        }));
    }
    let mut accepted = 0;
    let mut refused = 0;
    for attempt in attempts {
        match attempt.await.expect("task") {
            Ok(()) => accepted += 1,
            Err(AppError::QuotaExceeded(_)) => refused += 1,
            Err(other) => panic!("unexpected: {other:?}"),
        }
    }
    assert_eq!(
        (accepted, refused),
        (5, 25),
        "exactly the quota, not one more"
    );
    let stored: i64 = sqlx::query_scalar("SELECT count(*) FROM runs WHERE finished_at IS NULL")
        .fetch_one(&w.db.pool)
        .await
        .expect("count");
    assert_eq!(stored, 5);
}

#[tokio::test]
async fn child_runs_count_against_the_quota_and_the_step_that_wanted_one_fails() {
    let w = world().await;
    let (_kid, _) = w.publish(document("kid", "a", vec![activity("a")])).await;
    let (parent_wf, _) = w
        .publish(document("parent", "c", vec![child_node("c", "kid", None)]))
        .await;
    set_in_namespace(&w, QuotaResource::ActiveRuns, 1).await;
    let parent = w.start(parent_wf, &json!({})).await;

    for _ in 0..4 {
        w.scheduler.tick().await;
    }
    let counted: i64 = sqlx::query_scalar("SELECT count(*) FROM runs")
        .fetch_one(&w.db.pool)
        .await
        .expect("count");
    assert_eq!(counted, 1, "no child was created over the quota");
    let code: Option<String> =
        sqlx::query_scalar("SELECT failure_code FROM steps WHERE run_id = $1 AND node = 'c'")
            .bind(parent.as_uuid())
            .fetch_one(&w.db.pool)
            .await
            .expect("code");
    assert_eq!(code.as_deref(), Some("CHILD_LIMIT_EXCEEDED"));
    assert_eq!(
        w.run_status(parent).await,
        sagat_domain::execution::RunStatus::Failed
    );
}

// ------------------------------------------------------------- workflows and workers

#[tokio::test]
async fn a_namespace_holds_a_bounded_number_of_workflows() {
    let w = world().await;
    set_in_namespace(&w, QuotaResource::Workflows, 2).await;
    w.publish(waiting_flow("one")).await;
    w.publish(waiting_flow("two")).await;
    let bytes = serde_json::to_vec(&waiting_flow("three")).expect("json");
    let refused = w
        .defs
        .create_workflow(
            &w.admin,
            "payments",
            "three",
            &bytes,
            "create-three-key",
            &ctx(),
        )
        .await;
    assert_eq!(
        refused.map(|_| ()),
        Err(AppError::QuotaExceeded("workflows in the namespace"))
    );
    // The other tenant has its own count.
    w.publish_as_other_tenant(waiting_flow("elsewhere")).await;
}

#[tokio::test]
async fn a_tenant_holds_a_bounded_number_of_worker_sessions_and_a_reconnect_is_not_one_more() {
    let w = world().await;
    set(&w, QuotaResource::WorkerSessions, 2).await;
    let caps = sagat_application::worker::Capabilities {
        activities: vec!["act.a".into()],
        languages: vec![],
        pools: vec![],
        max_inflight: 4,
        protocol_version: "1".into(),
    };
    let other_identity = Principal::new(w.admin.tenant_id(), subject_of("worker-2"), Role::Worker);
    w.bind_everything(&other_identity).await;
    w.workers
        .register(&w.worker, "payments", "first", &caps)
        .await
        .expect("first");
    w.workers
        .register(&other_identity, "payments", "second", &caps)
        .await
        .expect("second");
    let third = w
        .workers
        .register(&w.worker, "payments", "third", &caps)
        .await;
    assert!(
        matches!(third, Err(AppError::QuotaExceeded(_))),
        "{third:?}"
    );
    w.workers
        .register(&w.worker, "payments", "first", &caps)
        .await
        .expect("a reconnect at the limit replaces its own session");
}

// ------------------------------------------------------------- administration

async fn workflows_limit(
    w: &World,
    namespace: Option<sagat_domain::identity::NamespaceId>,
) -> (u64, QuotaSource) {
    w.quotas
        .list(&w.admin, namespace, &ctx())
        .await
        .expect("list")
        .into_iter()
        .find(|q| q.resource == QuotaResource::Workflows)
        .map(|q| (q.limit, q.source))
        .expect("workflows")
}

#[tokio::test]
async fn the_limit_in_force_is_the_namespaces_then_the_tenants_then_the_default() {
    let w = world().await;
    let ns = Some(w.namespace.id());
    assert_eq!(
        workflows_limit(&w, ns).await,
        (
            QuotaResource::Workflows.default_limit(),
            QuotaSource::Default
        )
    );
    set(&w, QuotaResource::Workflows, 50).await;
    assert_eq!(workflows_limit(&w, ns).await, (50, QuotaSource::Tenant));
    set_in_namespace(&w, QuotaResource::Workflows, 7).await;
    assert_eq!(
        workflows_limit(&w, ns).await,
        (7, QuotaSource::Namespace),
        "the namespace wins"
    );
    w.quotas
        .clear(&w.admin, ns, QuotaResource::Workflows, &ctx())
        .await
        .expect("clear");
    assert_eq!(
        workflows_limit(&w, ns).await,
        (50, QuotaSource::Tenant),
        "back to the tenant's"
    );
}

#[tokio::test]
async fn the_tenant_view_has_no_single_usage_for_a_per_namespace_resource() {
    let w = world().await;
    let (wf, _) = w.publish(waiting_flow("usage")).await;
    w.start(wf, &json!({})).await;
    // A run that has ended is not use.
    let ended = w.start(wf, &json!({})).await;
    w.operate_run(ended, RunOperation::Cancel).await;

    // Tenant-wide view: every resource, but a per-namespace one has no single usage there.
    let tenant_view = w.quotas.list(&w.admin, None, &ctx()).await.expect("list");
    let seen: Vec<_> = tenant_view
        .iter()
        .map(|q| (q.resource, q.used.is_some()))
        .collect();
    assert_eq!(
        seen,
        vec![
            (QuotaResource::ActiveRuns, false),
            (QuotaResource::TenantActiveRuns, true),
            (QuotaResource::Workflows, false),
            (QuotaResource::WorkerSessions, true),
        ]
    );
    let used = tenant_view
        .iter()
        .find(|q| q.resource == QuotaResource::TenantActiveRuns)
        .and_then(|q| q.used);
    assert_eq!(used, Some(1), "the exact count of the tenant");

    // Namespace view: only what is counted in a namespace, with its own exact usage.
    let namespace_view = w
        .quotas
        .list(&w.admin, Some(w.namespace.id()), &ctx())
        .await
        .expect("list");
    let seen: Vec<_> = namespace_view
        .iter()
        .map(|q| (q.resource, q.used))
        .collect();
    assert_eq!(
        seen,
        vec![
            (QuotaResource::ActiveRuns, Some(1)),
            (QuotaResource::Workflows, Some(1)),
        ]
    );
}

#[tokio::test]
async fn only_a_manager_changes_quotas_and_the_roles_that_see_the_tenant_read_them() {
    let w = world().await;
    let tenant = w.admin.tenant_id();
    let as_role = |role| Principal::new(tenant, subject_of("someone"), role);
    for role in [Role::Developer, Role::Operator, Role::Viewer, Role::Worker] {
        let result = w
            .quotas
            .set(&as_role(role), None, QuotaResource::Workflows, 5, &ctx())
            .await;
        assert!(
            matches!(result, Err(AppError::Forbidden { .. })),
            "{role}: {result:?}"
        );
        let cleared = w
            .quotas
            .clear(&as_role(role), None, QuotaResource::Workflows, &ctx())
            .await;
        assert!(matches!(cleared, Err(AppError::Forbidden { .. })), "{role}");
    }
    for role in [Role::Admin, Role::Developer, Role::Operator, Role::Viewer] {
        assert!(
            w.quotas.list(&as_role(role), None, &ctx()).await.is_ok(),
            "{role} can read"
        );
    }
    assert!(
        matches!(
            w.quotas.list(&as_role(Role::Worker), None, &ctx()).await,
            Err(AppError::Forbidden { .. })
        ),
        "a worker identity reads nothing but its tasks"
    );
}

#[tokio::test]
async fn changing_a_quota_is_audited_and_a_foreign_namespace_looks_missing_and_is_audited() {
    let w = world().await;
    set_in_namespace(&w, QuotaResource::ActiveRuns, 9).await;
    let (action, reason, resource): (String, String, String) = sqlx::query_as(
        "SELECT action, reason, resource_type FROM audit_records \
         WHERE tenant_id = $1 AND action = 'quota.changed'",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("audit row");
    assert_eq!(
        (action.as_str(), reason.as_str(), resource.as_str()),
        ("quota.changed", "active_runs = 9", "quota")
    );

    // The other tenant asks about this namespace's quotas.
    let foreign = w
        .quotas
        .list(&w.other_admin, Some(w.namespace.id()), &ctx())
        .await;
    assert_eq!(foreign.map(|_| ()), Err(AppError::NotFound));
    let missing = w
        .quotas
        .list(
            &w.other_admin,
            Some(sagat_domain::identity::NamespaceId::generate()),
            &ctx(),
        )
        .await;
    assert_eq!(
        missing.map(|_| ()),
        Err(AppError::NotFound),
        "indistinguishable"
    );
    let foreign_ns = w.namespace.id().to_string();
    let audited = crate::support::eventually("the cross-tenant access is audited", || async {
        let rows: Vec<String> = sqlx::query_scalar(
            "SELECT resource_id FROM audit_records WHERE tenant_id = $1 \
             AND action = 'access.denied.cross_tenant' AND resource_type = 'namespace'",
        )
        .bind(w.other_admin.tenant_id().as_uuid())
        .fetch_all(&w.db.pool)
        .await
        .expect("rows");
        (!rows.is_empty()).then_some(rows)
    })
    .await;
    assert_eq!(
        audited,
        vec![foreign_ns],
        "the namespace that exists elsewhere is audited, and the one that exists nowhere is not"
    );
}

#[tokio::test]
async fn out_of_range_and_misplaced_values_are_refused_and_stored_nothing() {
    let w = world().await;
    let too_big = w
        .quotas
        .set(
            &w.admin,
            None,
            QuotaResource::Workflows,
            MAX_LIMIT + 1,
            &ctx(),
        )
        .await;
    assert!(
        matches!(too_big, Err(AppError::NotAllowed(_))),
        "{too_big:?}"
    );
    let misplaced = w
        .quotas
        .set(
            &w.admin,
            Some(w.namespace.id()),
            QuotaResource::WorkerSessions,
            5,
            &ctx(),
        )
        .await;
    assert!(
        matches!(misplaced, Err(AppError::NotAllowed(_))),
        "a tenant-wide resource has no namespace level: {misplaced:?}"
    );
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM quota_limits")
        .fetch_one(&w.db.pool)
        .await
        .expect("count");
    assert_eq!(rows, 0);
    // The database says the same on its own (defence in depth).
    let direct = sqlx::query(
        "INSERT INTO quota_limits (tenant_id, namespace_id, resource, max_value, updated_by) \
         VALUES ($1, $2, 'worker_sessions', 5, 'x')",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(w.namespace.id().as_uuid())
    .execute(&w.db.pool)
    .await;
    assert!(
        direct.is_err(),
        "the table refuses a namespace level for a tenant-wide resource"
    );
}
