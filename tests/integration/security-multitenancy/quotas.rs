//! Quotas (FR-009-006, US04): a hard limit judged against an exact count, taken in the transaction
//! that creates the thing counted, so racing replicas cannot overshoot it and one tenant's limit is
//! never another's.

use sagat_application::error::AppError;
use sagat_application::identity::RequestContext;
use sagat_application::quotas::{QuotaLookup, StoredLimit};
use sagat_application::runs::RunOperation;
use sagat_domain::identity::{NamespaceId, Principal, Role};
use sagat_domain::quota::{MAX_LIMIT, QuotaResource, QuotaScope, QuotaSource};
use sagat_domain::workflow::ActivityName;
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

#[tokio::test]
async fn activity_limits_are_authorized_ceiling_checked_tenant_bound_and_audited() {
    let w = world().await;
    let activity = ActivityName::parse("payment.capture").expect("activity");
    assert!(matches!(
        w.quotas
            .set_activity_limit(&w.worker, w.namespace.id(), activity.clone(), 2, &ctx())
            .await,
        Err(AppError::Forbidden { .. })
    ));
    assert!(matches!(
        w.quotas
            .set_activity_limit(
                &w.admin,
                w.namespace.id(),
                activity.clone(),
                MAX_LIMIT + 1,
                &ctx()
            )
            .await,
        Err(AppError::NotAllowed(_))
    ));
    assert_eq!(
        w.quotas
            .set_activity_limit(
                &w.admin,
                w.other_namespace.id(),
                activity.clone(),
                2,
                &ctx()
            )
            .await,
        Err(AppError::NotFound)
    );
    let view = w
        .quotas
        .set_activity_limit(&w.admin, w.namespace.id(), activity.clone(), 2, &ctx())
        .await
        .expect("set activity quota");
    assert_eq!((view.limit, view.used, view.overridden), (2, 0, true));
    let listed = w
        .quotas
        .list_activity_limits(&w.admin, w.namespace.id(), &ctx())
        .await
        .expect("list activity quotas");
    assert_eq!(listed, [view]);
    w.quotas
        .grant_ceiling(
            w.admin.tenant_id(),
            QuotaResource::ActivityConcurrency,
            Some(1),
        )
        .await
        .expect("lower ceiling");
    let limited = w
        .quotas
        .list_activity_limits(&w.admin, w.namespace.id(), &ctx())
        .await
        .expect("list lowered activity quota");
    assert_eq!((limited[0].limit, limited[0].ceiling), (1, 1));
    let audit: (String, String) = sqlx::query_as(
        "SELECT action, reason FROM audit_records WHERE tenant_id = $1 \
         AND action = 'quota.changed' AND reason LIKE 'activity payment.capture%'",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("audit");
    assert_eq!(
        audit,
        (
            "quota.changed".into(),
            "activity payment.capture concurrency = 2".into()
        )
    );
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
    assert_eq!(
        code.as_deref(),
        Some("CHILD_QUOTA_EXCEEDED"),
        "a full namespace is not the size limit of a tree"
    );
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

#[tokio::test]
async fn worker_registration_reconnect_and_session_renewal_survive_a_full_content_quota() {
    let w = world().await;
    set(&w, QuotaResource::TenantContentBytes, 0).await;
    sqlx::query("UPDATE quota_accounting_control SET content_enforcement_enabled = true")
        .execute(&w.db.pool)
        .await
        .expect("enable content enforcement");
    let caps = sagat_application::worker::Capabilities {
        activities: vec!["act.a".into()],
        languages: vec!["rust".into()],
        pools: vec!["primary".into()],
        max_inflight: 4,
        protocol_version: "1".into(),
    };
    let first = w
        .workers
        .register(&w.worker, "payments", "quota-safe", &caps)
        .await
        .expect("register at zero content allowance");
    let again = w
        .workers
        .register(&w.worker, "payments", "quota-safe", &caps)
        .await
        .expect("reconnect at zero content allowance");
    assert_eq!(again.worker_id, first.worker_id);
    assert!(
        w.workers
            .poll(
                &w.worker,
                again.worker_id,
                again.session_token.expose(),
                1,
                Some("quota-safe-session-renewal"),
            )
            .await
            .expect("renew session at zero content allowance")
            .is_empty()
    );
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
            (QuotaResource::ActivityConcurrency, false),
            (QuotaResource::Secrets, false),
            (QuotaResource::EventEndpoints, false),
            (QuotaResource::WebhookSubscriptions, false),
            (QuotaResource::BrokerSubscriptions, false),
            (QuotaResource::Connectors, false),
            (QuotaResource::Scripts, false),
            (QuotaResource::ScriptJobs, false),
            (QuotaResource::TenantContentBytes, true),
            (QuotaResource::TenantLogBytes, true),
            (QuotaResource::ApiRequestsPerSecond, false),
            (QuotaResource::EventsPerSecond, false),
            (QuotaResource::EventRetentionSeconds, false),
            (QuotaResource::ScriptRetentionDays, false),
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
            (QuotaResource::ActivityConcurrency, Some(0)),
            (QuotaResource::Secrets, Some(0)),
            (QuotaResource::EventEndpoints, Some(0)),
            (QuotaResource::WebhookSubscriptions, Some(0)),
            (QuotaResource::BrokerSubscriptions, Some(0)),
            (QuotaResource::Connectors, Some(0)),
            (QuotaResource::Scripts, Some(0)),
            (QuotaResource::ScriptJobs, Some(0)),
            (QuotaResource::ApiRequestsPerSecond, None),
            (QuotaResource::EventsPerSecond, None),
            (QuotaResource::EventRetentionSeconds, None),
            (QuotaResource::ScriptRetentionDays, None),
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

// ------------------------------------------------------------- who may raise a limit

/// The namespace a limit of `resource` is set for: none for a resource counted across the tenant.
fn level_of(w: &World, resource: QuotaResource) -> Option<NamespaceId> {
    (resource.scope() == QuotaScope::Namespace).then(|| w.namespace.id())
}

/// The limit the stores enforce for `tenant` in `namespace`.
async fn enforced(
    w: &World,
    tenant: sagat_domain::identity::TenantId,
    namespace: NamespaceId,
    resource: QuotaResource,
) -> u64 {
    w.store
        .effective_limit(tenant, Some(namespace), resource)
        .await
        .expect("limit")
}

async fn grant(w: &World, resource: QuotaResource, value: Option<u64>) {
    w.quotas
        .grant_ceiling(w.admin.tenant_id(), resource, value)
        .await
        .expect("grant a ceiling");
}

#[tokio::test]
async fn a_tenant_administrator_cannot_raise_any_limit_above_the_ceiling_the_platform_grants() {
    let w = world().await;
    for resource in QuotaResource::ALL {
        let level = level_of(&w, resource);
        let ceiling = resource.default_limit();

        if ceiling < resource.maximum() {
            let refused = w
                .quotas
                .set(&w.admin, level, resource, ceiling + 1, &ctx())
                .await;
            assert!(
                matches!(refused, Err(AppError::NotAllowed(reason)) if reason.contains("ceiling")),
                "{resource}: {refused:?}"
            );
            // The resource-specific maximum still cannot bypass a lower platform ceiling.
            let refused = w
                .quotas
                .set(&w.admin, level, resource, resource.maximum(), &ctx())
                .await;
            assert!(
                matches!(refused, Err(AppError::NotAllowed(_))),
                "{resource}: {refused:?}"
            );
        }

        // The ceiling itself is allowed, and the view says where the room ends.
        let view = w
            .quotas
            .set(&w.admin, level, resource, ceiling, &ctx())
            .await
            .expect("at the ceiling");
        assert_eq!((view.limit, view.ceiling), (ceiling, ceiling), "{resource}");
    }
    // Only the values at the ceiling were stored: one row per resource.
    let stored: Vec<(String, i64)> =
        sqlx::query_as("SELECT resource, max_value FROM quota_limits ORDER BY resource")
            .fetch_all(&w.db.pool)
            .await
            .expect("rows");
    let expected: Vec<(String, i64)> = {
        let mut rows: Vec<_> = QuotaResource::ALL
            .iter()
            .map(|r| {
                (
                    r.as_str().to_owned(),
                    i64::try_from(r.default_limit()).expect("small"),
                )
            })
            .collect();
        rows.sort();
        rows
    };
    assert_eq!(stored, expected);
}

#[tokio::test]
async fn an_operator_grants_a_ceiling_to_one_tenant_and_the_administrator_can_then_use_it() {
    let w = world().await;
    let resource = QuotaResource::ActiveRuns;
    let beyond = resource.default_limit() + 5_000;
    let refused = w
        .quotas
        .set(&w.admin, Some(w.namespace.id()), resource, beyond, &ctx())
        .await;
    assert!(
        matches!(refused, Err(AppError::NotAllowed(_))),
        "{refused:?}"
    );

    grant(&w, resource, Some(beyond + 5_000)).await;
    let view = w
        .quotas
        .set(&w.admin, Some(w.namespace.id()), resource, beyond, &ctx())
        .await
        .expect("within the granted ceiling");
    assert_eq!(
        (view.limit, view.ceiling, view.source),
        (beyond, beyond + 5_000, QuotaSource::Namespace)
    );
    // The grant does not reach past what it says.
    let refused = w
        .quotas
        .set(
            &w.admin,
            Some(w.namespace.id()),
            resource,
            beyond + 5_001,
            &ctx(),
        )
        .await;
    assert!(
        matches!(refused, Err(AppError::NotAllowed(_))),
        "{refused:?}"
    );

    // It belongs to the tenant it was granted to.
    let other = w
        .quotas
        .set(
            &w.other_admin,
            Some(w.other_namespace.id()),
            resource,
            beyond,
            &ctx(),
        )
        .await;
    assert!(matches!(other, Err(AppError::NotAllowed(_))), "{other:?}");
    let other_view = w
        .quotas
        .list(&w.other_admin, Some(w.other_namespace.id()), &ctx())
        .await
        .expect("list");
    let other_active = other_view
        .iter()
        .find(|q| q.resource == resource)
        .expect("active runs");
    assert_eq!(other_active.ceiling, resource.default_limit());
    // What is enforced follows the same rule as what is shown: the grant reaches only its tenant.
    assert_eq!(
        enforced(&w, w.admin.tenant_id(), w.namespace.id(), resource).await,
        beyond,
        "the granted tenant is judged by the limit it set"
    );
    assert_eq!(
        enforced(
            &w,
            w.other_admin.tenant_id(),
            w.other_namespace.id(),
            resource
        )
        .await,
        resource.default_limit(),
        "another tenant is judged by the default"
    );

    // The grant is on record, as the operator's.
    let (actor, reason): (String, String) = sqlx::query_as(
        "SELECT actor, reason FROM audit_records \
         WHERE tenant_id = $1 AND action = 'quota.changed' AND reason LIKE 'ceiling%'",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("audit");
    assert_eq!(actor, "system:operator");
    assert_eq!(
        reason,
        format!("ceiling of active_runs = {}", beyond + 5_000)
    );
}

#[tokio::test]
async fn withdrawing_or_lowering_a_ceiling_clamps_what_is_enforced_at_once() {
    let w = world().await;
    let (wf, _) = w.publish(waiting_flow("clamped")).await;
    let resource = QuotaResource::ActiveRuns;

    // The administrator is comfortable under a generous grant...
    grant(&w, resource, Some(500)).await;
    set_in_namespace(&w, resource, 400).await;

    // ...which the operator then lowers below what was set: the stored 400 no longer rules.
    grant(&w, resource, Some(2)).await;
    let view = w
        .quotas
        .list(&w.admin, Some(w.namespace.id()), &ctx())
        .await
        .expect("list");
    let active = view.iter().find(|q| q.resource == resource).expect("row");
    assert_eq!(
        (active.limit, active.ceiling, active.source),
        (2, 2, QuotaSource::Namespace),
        "the view tells the truth about what is in force"
    );
    w.start(wf, &json!({})).await;
    w.start(wf, &json!({})).await;
    assert_eq!(
        w.start_with(wf, &json!({}), "start-over-ceiling", None)
            .await,
        Err(AppError::QuotaExceeded("active runs in the namespace")),
        "the third run is refused although the tenant set 400"
    );

    // The same holds for the setting of the whole tenant.
    let tenant_resource = QuotaResource::Workflows;
    grant(&w, tenant_resource, Some(500)).await;
    set(&w, tenant_resource, 400).await;
    grant(&w, tenant_resource, Some(7)).await;
    let tenant_view = w.quotas.list(&w.admin, None, &ctx()).await.expect("list");
    let workflows = tenant_view
        .iter()
        .find(|q| q.resource == tenant_resource)
        .expect("row");
    assert_eq!(
        (workflows.limit, workflows.source),
        (7, QuotaSource::Tenant),
        "a lowered ceiling also bounds what was set for the whole tenant"
    );
    assert_eq!(
        w.store
            .effective_limit(w.admin.tenant_id(), Some(w.namespace.id()), tenant_resource)
            .await
            .expect("limit"),
        7,
        "and it is what the stores enforce"
    );

    // With nothing set, the granted ceiling is the room the tenant has.
    w.quotas
        .clear(&w.admin, Some(w.namespace.id()), resource, &ctx())
        .await
        .expect("clear");
    assert_eq!(
        w.start_with(wf, &json!({}), "start-at-ceiling", None).await,
        Err(AppError::QuotaExceeded("active runs in the namespace"))
    );
    grant(&w, resource, Some(3)).await;
    w.start_with(wf, &json!({}), "start-after-grant", None)
        .await
        .expect("one more place was granted");

    // Withdrawn, the platform default applies again.
    grant(&w, resource, None).await;
    let view = w
        .quotas
        .list(&w.admin, Some(w.namespace.id()), &ctx())
        .await
        .expect("list");
    let active = view.iter().find(|q| q.resource == resource).expect("row");
    assert_eq!(
        (active.limit, active.ceiling, active.source),
        (
            resource.default_limit(),
            resource.default_limit(),
            QuotaSource::Default
        )
    );
}

#[tokio::test]
async fn a_ceiling_that_is_out_of_range_or_for_a_tenant_that_does_not_exist_changes_nothing() {
    let w = world().await;
    let too_large = w
        .quotas
        .grant_ceiling(
            w.admin.tenant_id(),
            QuotaResource::Workflows,
            Some(MAX_LIMIT + 1),
        )
        .await;
    assert!(
        matches!(too_large, Err(AppError::NotAllowed(_))),
        "{too_large:?}"
    );
    let nobody = w
        .quotas
        .grant_ceiling(
            sagat_domain::identity::TenantId::generate(),
            QuotaResource::Workflows,
            Some(10),
        )
        .await;
    assert_eq!(nobody, Err(AppError::NotFound));
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM quota_ceilings")
        .fetch_one(&w.db.pool)
        .await
        .expect("count");
    let audits: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_records WHERE action = 'quota.changed' AND reason LIKE 'ceiling%'",
    )
    .fetch_one(&w.db.pool)
    .await
    .expect("count");
    assert_eq!((rows, audits), (0, 0));
}

#[tokio::test]
async fn the_ceiling_command_grants_withdraws_and_refuses_the_nonsense() {
    use sagat_control_plane::config::{CeilingConfig, Config};

    let w = world().await;
    let env = |name: &str| match name {
        "DATABASE_URL" => Some(w.db.url.clone()),
        "SAGAT_ENV" => Some("dev".to_owned()),
        _ => None,
    };
    let config = Config::from_lookup(&env).expect("config");
    let request = |tenant: String, resource: &str, value: Option<u64>| CeilingConfig {
        tenant_id: tenant,
        resource: resource.to_owned(),
        value,
    };
    let tenant = w.admin.tenant_id();

    sagat_control_plane::grant_quota_ceiling(
        &config,
        &request(tenant.to_string(), "worker_sessions", Some(5_000)),
    )
    .await
    .expect("grant");
    let granted: i64 = sqlx::query_scalar(
        "SELECT max_value FROM quota_ceilings WHERE tenant_id = $1 AND resource = 'worker_sessions'",
    )
    .bind(tenant.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("row");
    assert_eq!(granted, 5_000);
    let view = w.quotas.list(&w.admin, None, &ctx()).await.expect("list");
    let sessions = view
        .iter()
        .find(|q| q.resource == QuotaResource::WorkerSessions)
        .expect("row");
    assert_eq!(
        sessions.ceiling, 5_000,
        "the API shows what the command granted"
    );

    sagat_control_plane::grant_quota_ceiling(
        &config,
        &request(tenant.to_string(), "worker_sessions", None),
    )
    .await
    .expect("withdraw");
    let left: i64 = sqlx::query_scalar("SELECT count(*) FROM quota_ceilings")
        .fetch_one(&w.db.pool)
        .await
        .expect("count");
    assert_eq!(left, 0);

    for (bad, what) in [
        (
            request("not-a-tenant".into(), "workflows", Some(1)),
            "tenant id",
        ),
        (request(tenant.to_string(), "builds", Some(1)), "resource"),
        (
            request(tenant.to_string(), "workflows", Some(MAX_LIMIT + 1)),
            "range",
        ),
    ] {
        let refused = sagat_control_plane::grant_quota_ceiling(&config, &bad).await;
        assert!(
            matches!(refused, Err(sagat_control_plane::RunError::Config(_))),
            "{what}: {refused:?}"
        );
    }
    let missing = sagat_control_plane::grant_quota_ceiling(
        &config,
        &request(
            sagat_domain::identity::TenantId::generate().to_string(),
            "workflows",
            Some(1),
        ),
    )
    .await;
    assert!(
        matches!(missing, Err(sagat_control_plane::RunError::Refused(_))),
        "{missing:?}"
    );
}
