//! Worker bindings (FR-009-002, gap G31): a worker identity serves only what its binding names,
//! in the namespace it names; removing or narrowing a binding takes effect at once.

use sagat_application::error::AppError;
use sagat_application::worker::Capabilities;
use sagat_domain::execution::StepStatus;
use sagat_domain::identity::{NamespaceId, Principal, Role};
use serde_json::json;

use crate::support::engine::{World, activity, ctx, document, subject_of, task_ref, world};

async fn worker(w: &World, name: &str) -> Principal {
    w.member(name, Role::Worker).await
}

fn caps(activities: &[&str]) -> Capabilities {
    Capabilities {
        activities: activities.iter().map(|a| (*a).to_owned()).collect(),
        languages: vec![],
        pools: vec![],
        max_inflight: 10,
        protocol_version: "1".into(),
    }
}

async fn bind(w: &World, who: &str, namespace: NamespaceId, patterns: &[&str]) {
    let patterns: Vec<String> = patterns.iter().map(|p| (*p).to_owned()).collect();
    w.bindings
        .bind(&w.admin, namespace, who, &patterns, &ctx())
        .await
        .expect("bind");
}

async fn register(
    w: &World,
    who: &Principal,
    namespace: &str,
    name: &str,
    activities: &[&str],
) -> Result<sagat_application::worker::Registered, AppError> {
    w.workers
        .register(who, namespace, name, &caps(activities))
        .await
}

// ------------------------------------------------------------- fail closed

#[tokio::test]
async fn a_worker_without_a_binding_is_refused_and_the_attempt_is_audited() {
    let w = world().await;
    let stranger = worker(&w, "stranger").await;
    let refused = register(&w, &stranger, "payments", "first", &["act.a"]).await;
    assert_eq!(refused.map(|_| ()), Err(AppError::NotBound));

    let (action, reason, kind): (String, String, String) = {
        let tenant = w.admin.tenant_id();
        crate::support::eventually("the refusal is audited", || async {
            sqlx::query_as::<_, (String, String, String)>(
                "SELECT action, reason, resource_type FROM audit_records \
                 WHERE tenant_id = $1 AND action = 'worker.denied'",
            )
            .bind(tenant.as_uuid())
            .fetch_optional(&w.db.pool)
            .await
            .expect("audit")
        })
        .await
    };
    assert_eq!(
        (action.as_str(), reason.as_str(), kind.as_str()),
        (
            "worker.denied",
            "not bound to activity act.a",
            "worker_binding"
        )
    );
}

#[tokio::test]
async fn a_binding_covers_exactly_the_activities_it_names() {
    let w = world().await;
    let who = worker(&w, "scoped").await;
    bind(&w, "scoped", w.namespace.id(), &["payments.*", "audit.log"]).await;

    register(
        &w,
        &who,
        "payments",
        "ok",
        &["payments.charge", "audit.log"],
    )
    .await
    .expect("covered");
    for (what, activities) in [
        (
            "another activity of a bound prefix's sibling",
            vec!["audit.purge"],
        ),
        ("the bare prefix", vec!["payments"]),
        ("a look-alike prefix", vec!["paymentsx.charge"]),
        (
            "one covered and one not",
            vec!["payments.charge", "other.thing"],
        ),
    ] {
        let refused = register(&w, &who, "payments", "bad", &activities).await;
        assert_eq!(refused.map(|_| ()), Err(AppError::NotBound), "{what}");
    }
}

#[tokio::test]
async fn a_binding_belongs_to_one_namespace() {
    let w = world().await;
    let billing = w.add_namespace("billing").await;
    let who = worker(&w, "narrow").await;
    bind(&w, "narrow", w.namespace.id(), &["*"]).await;

    register(&w, &who, "payments", "here", &["act.a"])
        .await
        .expect("bound in payments");
    let elsewhere = register(&w, &who, "billing", "there", &["act.a"]).await;
    assert_eq!(
        elsewhere.map(|_| ()),
        Err(AppError::NotBound),
        "the same identity and activity in another namespace is another decision"
    );
    bind(&w, "narrow", billing.id(), &["act.a"]).await;
    register(&w, &who, "billing", "there", &["act.a"])
        .await
        .expect("now bound in billing too");
}

#[tokio::test]
async fn bindings_of_one_tenant_are_invisible_and_unchangeable_by_another() {
    let w = world().await;
    bind(&w, "someone", w.namespace.id(), &["*"]).await;

    // The other tenant's administrator names this tenant's namespace.
    let listed = w
        .bindings
        .list(&w.other_admin, w.namespace.id(), &ctx())
        .await;
    assert_eq!(listed.map(|_| ()), Err(AppError::NotFound));
    let changed = w
        .bindings
        .bind(
            &w.other_admin,
            w.namespace.id(),
            "someone",
            &["*".to_owned()],
            &ctx(),
        )
        .await;
    assert_eq!(changed.map(|_| ()), Err(AppError::NotFound));
    let removed = w
        .bindings
        .unbind(&w.other_admin, w.namespace.id(), "someone", &ctx())
        .await;
    assert_eq!(removed.map(|_| ()), Err(AppError::NotFound));

    // The probes are recorded against the tenant that made them, naming the namespace asked for.
    let probes = crate::support::eventually("the probes are audited", || async {
        let rows: Vec<String> = sqlx::query_scalar(
            "SELECT resource_id FROM audit_records WHERE tenant_id = $1 \
             AND action = 'access.denied.cross_tenant' AND resource_type = 'namespace'",
        )
        .bind(w.other_admin.tenant_id().as_uuid())
        .fetch_all(&w.db.pool)
        .await
        .expect("audit");
        (!rows.is_empty()).then_some(rows)
    })
    .await;
    assert_eq!(probes, vec![w.namespace.id().to_string()]);

    // And a worker with the same name in the other tenant has no binding of its own.
    let namesake = Principal::new(
        w.other_admin.tenant_id(),
        subject_of("someone"),
        Role::Worker,
    );
    let refused = w
        .workers
        .register(&namesake, "payments", "x", &caps(&["act.a"]))
        .await;
    assert_eq!(refused.map(|_| ()), Err(AppError::NotBound));
    let rows: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM worker_bindings WHERE tenant_id = $1 AND subject = 'someone'",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("count");
    assert_eq!(rows, 1, "the foreign attempts changed nothing");
}

#[tokio::test]
async fn each_binding_operation_audits_a_probe_of_a_foreign_namespace_on_its_own() {
    // Identical probes are collapsed into one audit row, so each operation gets a world of its own
    // to prove that it records the probe itself and does not rely on another one having done it.
    for operation in ["list", "bind", "unbind"] {
        let w = world().await;
        let ns = w.namespace.id();
        let result = match operation {
            "list" => w
                .bindings
                .list(&w.other_admin, ns, &ctx())
                .await
                .map(|_| ()),
            "bind" => w
                .bindings
                .bind(&w.other_admin, ns, "someone", &["*".to_owned()], &ctx())
                .await
                .map(|_| ()),
            _ => w
                .bindings
                .unbind(&w.other_admin, ns, "someone", &ctx())
                .await
                .map(|_| ()),
        };
        assert_eq!(result, Err(AppError::NotFound), "{operation}");
        let tenant = w.other_admin.tenant_id();
        let audited = crate::support::eventually(operation, || async {
            let rows: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM audit_records WHERE tenant_id = $1 \
                 AND action = 'access.denied.cross_tenant' AND resource_type = 'namespace'",
            )
            .bind(tenant.as_uuid())
            .fetch_one(&w.db.pool)
            .await
            .expect("count");
            (rows > 0).then_some(rows)
        })
        .await;
        assert_eq!(audited, 1, "{operation}");
    }
}

// ------------------------------------------------------------- revocation

#[tokio::test]
async fn removing_a_binding_closes_the_session_releases_the_lease_and_stops_the_worker() {
    let w = world().await;
    let (wf, _) = w
        .publish(document(
            "charge",
            "c",
            vec![json!({"id": "c", "type": "activity", "activity": "payments.charge"})],
        ))
        .await;
    let run = w.start(wf, &json!({})).await;
    let who = worker(&w, "revoked").await;
    bind(&w, "revoked", w.namespace.id(), &["payments.*"]).await;
    let registered = register(&w, &who, "payments", "r1", &["payments.charge"])
        .await
        .expect("register");
    let tasks = w
        .workers
        .poll(
            &who,
            registered.worker_id,
            registered.session_token.expose(),
            4,
            None,
        )
        .await
        .expect("poll");
    assert_eq!(tasks.len(), 1);
    let task = &tasks[0];
    w.workers
        .heartbeat(&who, &task_ref(task))
        .await
        .expect("a bound worker heartbeats");

    let revoked = w
        .bindings
        .unbind(&w.admin, w.namespace.id(), "revoked", &ctx())
        .await
        .expect("unbind");
    assert_eq!((revoked.sessions, revoked.leases), (1, 1));

    // At once: the session is gone, and the lease it held can neither be renewed nor reported on.
    let polled = w
        .workers
        .poll(
            &who,
            registered.worker_id,
            registered.session_token.expose(),
            4,
            None,
        )
        .await;
    assert_eq!(polled.map(|_| ()), Err(AppError::Unauthenticated));
    let beat = w.workers.heartbeat(&who, &task_ref(task)).await;
    assert_eq!(beat.map(|_| ()), Err(AppError::NotBound));
    let done = w
        .workers
        .complete(&who, &task_ref(task), b"{}", "revoked-complete-key")
        .await;
    assert_eq!(done.map(|_| ()), Err(AppError::NotBound));

    // The scheduler reclaims the released attempt and a bound worker takes it over.
    let report = w.scheduler.tick().await;
    assert_eq!(report.leases_expired, 1, "{report:?}");
    w.age_retry(run, "c").await;
    w.scheduler.tick().await;
    let session = w.session(&["payments.charge"]).await;
    let again = w.one_task(&session).await;
    assert_eq!(again.run_id, run);
    assert!(
        again.fence.generation > task.fence.generation,
        "a new grant"
    );
    w.complete(&session, &again, &json!({"ok": true}))
        .await
        .expect("the work is finished by a bound worker");
    assert_eq!(w.step_status(run, "c").await, Some(StepStatus::Succeeded));
    w.assert_consistent(run).await;
}

#[tokio::test]
async fn narrowing_a_binding_closes_only_the_sessions_it_no_longer_covers() {
    let w = world().await;
    let who = worker(&w, "narrowed").await;
    bind(&w, "narrowed", w.namespace.id(), &["*"]).await;
    let keeps = register(&w, &who, "payments", "keeps", &["payments.charge"])
        .await
        .expect("keeps");
    let loses = register(&w, &who, "payments", "loses", &["audit.log"])
        .await
        .expect("loses");

    let view = w
        .bindings
        .bind(
            &w.admin,
            w.namespace.id(),
            "narrowed",
            &["payments.*".to_owned()],
            &ctx(),
        )
        .await
        .expect("narrow");
    assert_eq!(view.revoked.sessions, 1);
    assert!(
        w.workers
            .poll(&who, keeps.worker_id, keeps.session_token.expose(), 1, None)
            .await
            .is_ok(),
        "still covered"
    );
    assert_eq!(
        w.workers
            .poll(&who, loses.worker_id, loses.session_token.expose(), 1, None)
            .await
            .map(|_| ()),
        Err(AppError::Unauthenticated)
    );

    // Widening closes nothing.
    let view = w
        .bindings
        .bind(
            &w.admin,
            w.namespace.id(),
            "narrowed",
            &["*".to_owned()],
            &ctx(),
        )
        .await
        .expect("widen");
    assert_eq!(view.revoked.sessions, 0);
}

#[tokio::test]
async fn a_binding_that_disappears_stops_a_lease_holder_even_without_a_session_event() {
    let w = world().await;
    let (wf, _) = w.publish(document("beat", "a", vec![activity("a")])).await;
    w.start(wf, &json!({})).await;
    let who = worker(&w, "stale-acl").await;
    bind(&w, "stale-acl", w.namespace.id(), &["*"]).await;
    let registered = register(&w, &who, "payments", "s", &["act.a"])
        .await
        .expect("register");
    let tasks = w
        .workers
        .poll(
            &who,
            registered.worker_id,
            registered.session_token.expose(),
            1,
            None,
        )
        .await
        .expect("poll");
    let task = &tasks[0];

    // The row goes away behind the service's back (a restore, an operator with SQL): the decision
    // is taken at use, so the next report is refused all the same.
    sqlx::query("DELETE FROM worker_bindings WHERE subject = 'stale-acl'")
        .execute(&w.db.pool)
        .await
        .expect("delete");
    let beat = w.workers.heartbeat(&who, &task_ref(task)).await;
    assert_eq!(beat.map(|_| ()), Err(AppError::NotBound));
    let failed = w
        .workers
        .fail(
            &who,
            &task_ref(task),
            "SOME_ERROR",
            true,
            "stale-acl-fail-key",
        )
        .await;
    assert_eq!(failed.map(|_| ()), Err(AppError::NotBound));
}

#[tokio::test]
async fn unbinding_an_identity_that_has_no_binding_is_not_found_and_leaves_no_audit() {
    let w = world().await;
    let removed = w
        .bindings
        .unbind(&w.admin, w.namespace.id(), "nobody", &ctx())
        .await;
    assert_eq!(removed.map(|_| ()), Err(AppError::NotFound));
    let audits: i64 =
        sqlx::query_scalar("SELECT count(*) FROM audit_records WHERE action = 'worker.unbound'")
            .fetch_one(&w.db.pool)
            .await
            .expect("count");
    assert_eq!(audits, 0);
}

// ------------------------------------------------------------- administration

#[tokio::test]
async fn only_an_administrator_manages_bindings() {
    let w = world().await;
    let tenant = w.admin.tenant_id();
    for role in [Role::Developer, Role::Operator, Role::Viewer, Role::Worker] {
        let who = Principal::new(tenant, subject_of("someone"), role);
        let ns = w.namespace.id();
        let listed = w.bindings.list(&who, ns, &ctx()).await;
        let bound = w
            .bindings
            .bind(&who, ns, "x", &["*".to_owned()], &ctx())
            .await;
        let unbound = w.bindings.unbind(&who, ns, "x", &ctx()).await;
        assert!(matches!(listed, Err(AppError::Forbidden { .. })), "{role}");
        assert!(matches!(bound, Err(AppError::Forbidden { .. })), "{role}");
        assert!(matches!(unbound, Err(AppError::Forbidden { .. })), "{role}");
    }
}

#[tokio::test]
async fn patterns_and_subjects_are_validated_and_nothing_is_stored_on_refusal() {
    let w = world().await;
    let ns = w.namespace.id();
    let try_bind = |subject: &'static str, patterns: Vec<String>| {
        let w = &w;
        async move {
            w.bindings
                .bind(&w.admin, ns, subject, &patterns, &ctx())
                .await
        }
    };
    for (what, subject, patterns) in [
        ("no patterns", "ok", vec![]),
        (
            "too many",
            "ok",
            (0..65).map(|i| format!("a.n{i}")).collect(),
        ),
        ("a bad pattern", "ok", vec!["has space".to_owned()]),
        ("a double wildcard", "ok", vec!["a.**".to_owned()]),
        ("an empty subject", "", vec!["*".to_owned()]),
        ("a reserved subject", "system:engine", vec!["*".to_owned()]),
    ] {
        let result = try_bind(subject, patterns).await;
        assert!(
            matches!(result, Err(AppError::NotAllowed(_))),
            "{what}: {result:?}"
        );
    }
    // (The world binds its own worker at the start; nothing else may have been stored.)
    let rows: i64 =
        sqlx::query_scalar("SELECT count(*) FROM worker_bindings WHERE subject <> 'worker-1'")
            .fetch_one(&w.db.pool)
            .await
            .expect("count");
    assert_eq!(rows, 0);
}

#[tokio::test]
async fn a_binding_change_is_audited_with_what_it_grants() {
    let w = world().await;
    bind(
        &w,
        "audited",
        w.namespace.id(),
        &["payments.*", "audit.log"],
    )
    .await;
    w.bindings
        .unbind(&w.admin, w.namespace.id(), "audited", &ctx())
        .await
        .expect("unbind");
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT action, reason FROM audit_records \
         WHERE tenant_id = $1 AND action IN ('worker.bound', 'worker.unbound') \
           AND reason LIKE 'audited%' ORDER BY created_at",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .fetch_all(&w.db.pool)
    .await
    .expect("audit");
    assert_eq!(
        rows,
        vec![
            (
                "worker.bound".to_owned(),
                "audited: payments.*, audit.log".to_owned()
            ),
            ("worker.unbound".to_owned(), "audited".to_owned()),
        ]
    );
    let listed = w
        .bindings
        .list(&w.admin, w.namespace.id(), &ctx())
        .await
        .expect("list");
    assert!(listed.iter().all(|b| b.subject != "audited"), "removed");
}

// ------------------------------------------------------------- resources

#[tokio::test]
async fn a_call_that_holds_a_transaction_never_waits_for_a_second_connection() {
    let w = world().await;
    let (wf, _) = w
        .publish(document("starve", "a", vec![activity("a")]))
        .await;
    w.start(wf, &json!({})).await;
    let session = w.session(&["act.a"]).await;
    let task = w.one_task(&session).await;

    // A replica with a pool of three (and nothing cached yet) and twelve reports on the same lease at
    // once: every call holds a connection for its transaction, so a call that asked the pool for
    // another one (to read a definition nobody has cached, or a binding) would leave the pool
    // waiting on itself until the acquire timeout.
    let replica = std::sync::Arc::new(w.replica_with_pool(3).await);
    let who = w.worker.clone();
    let mut calls = Vec::new();
    for _ in 0..12 {
        let replica = replica.clone();
        let who = who.clone();
        let task = task.clone();
        calls.push(tokio::spawn(async move {
            replica
                .workers
                .heartbeat(&who, &task_ref(&task))
                .await
                .map(|_| ())
        }));
    }
    for call in calls {
        call.await
            .expect("task")
            .expect("every heartbeat is served");
    }
}

// ------------------------------------------------------------- the race with a revocation

#[tokio::test]
async fn the_store_refuses_a_session_that_the_binding_no_longer_covers() {
    use sagat_application::worker::{NewSession, WorkerStore};

    let w = world().await;
    w.member("racer", Role::Worker).await;
    bind(&w, "racer", w.namespace.id(), &["pay.*"]).await;
    let session = |name: &str, activities: &[&str]| NewSession {
        id: uuid::Uuid::now_v7(),
        tenant_id: w.admin.tenant_id(),
        namespace_id: w.namespace.id(),
        subject: subject_of("racer").as_str().to_owned(),
        worker_name: name.to_owned(),
        token_hash: vec![7; 32],
        activities: activities.iter().map(|a| (*a).to_owned()).collect(),
        languages: vec![],
        pools: vec![],
        max_inflight: 10,
        accepted_protocol_minor: 0,
        ttl: std::time::Duration::from_secs(60),
        tenant_limit: 1000,
    };
    let sessions = || async {
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM worker_sessions WHERE tenant_id = $1")
            .bind(w.admin.tenant_id().as_uuid())
            .fetch_one(&w.db.pool)
            .await
            .expect("count")
    };

    // The application looked at an older, wider binding; the store judges the current one.
    let refused = w
        .store
        .create_session(&session("late", &["pay.charge", "other.thing"]))
        .await
        .expect("store");
    assert_eq!(refused, None);
    assert_eq!(sessions().await, 0, "a refused session leaves nothing");

    let accepted = w
        .store
        .create_session(&session("ok", &["pay.charge"]))
        .await
        .expect("store");
    assert!(accepted.is_some());
    assert_eq!(sessions().await, 1);

    // No binding at all is the narrowest binding there is.
    w.bindings
        .unbind(&w.admin, w.namespace.id(), "racer", &ctx())
        .await
        .expect("unbind");
    let refused = w
        .store
        .create_session(&session("after", &["pay.charge"]))
        .await
        .expect("store");
    assert_eq!(refused, None);
    assert_eq!(sessions().await, 0);
}

#[tokio::test]
async fn registering_while_a_binding_is_narrowed_never_leaves_a_session_it_does_not_cover() {
    let w = world().await;
    for round in 0..12 {
        let name = format!("racer{round}");
        let who = worker(&w, &name).await;
        bind(&w, &name, w.namespace.id(), &["pay.*", "other.*"]).await;
        let registrations = (0..6).map(|i| {
            let who = &who;
            let w = &w;
            async move {
                // Both outcomes are fine; what matters is what is left behind.
                let _ = register(w, who, "payments", &format!("w{i}"), &["other.thing"]).await;
            }
        });
        let narrowing = bind(&w, &name, w.namespace.id(), &["pay.*"]);
        let ((), _) = tokio::join!(
            async {
                futures_util::future::join_all(registrations).await;
            },
            narrowing
        );
        let leftover: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM worker_sessions \
             WHERE tenant_id = $1 AND subject = $2 AND 'other.thing' = ANY(activities)",
        )
        .bind(w.admin.tenant_id().as_uuid())
        .bind(subject_of(&name).as_str())
        .fetch_one(&w.db.pool)
        .await
        .expect("count");
        assert_eq!(leftover, 0, "round {round}: a session outlived its binding");
    }
}

#[tokio::test]
async fn releasing_the_leases_of_closed_sessions_is_an_index_lookup_and_not_a_scan_of_every_lease()
{
    let w = world().await;
    // The statement `replace_binding` runs while it holds the binding lock and the session rows. The
    // tables are tiny here, so the scan is switched off to see which index the planner would take.
    let mut tx = w.db.pool.begin().await.expect("begin");
    sqlx::query("SET LOCAL enable_seqscan = off")
        .execute(&mut *tx)
        .await
        .expect("set");
    // On tables this small the planner would rather walk either partial index of open steps than
    // look at the leases; the transaction is rolled back, so those competitors are dropped for the
    // question only: is the lookup by session served by `steps_leased_by_idx`?
    sqlx::query("DROP INDEX steps_run_open_idx")
        .execute(&mut *tx)
        .await
        .expect("withhold the run open-work index");
    sqlx::query("DROP INDEX steps_path_live_idx")
        .execute(&mut *tx)
        .await
        .expect("leave the lease index as the only candidate");
    sqlx::query("DROP INDEX steps_run_capacity_lease_idx")
        .execute(&mut *tx)
        .await
        .expect("withhold the capacity lease-probe index for this isolated availability check");
    let plan: Vec<String> = sqlx::query_scalar(
        "EXPLAIN UPDATE steps SET lease_expires_at = clock_timestamp() \
         WHERE tenant_id = $1 AND status IN ('LEASED', 'RUNNING') AND leased_by = ANY($2)",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(vec!["a-session".to_owned()])
    .fetch_all(&mut *tx)
    .await
    .expect("explain");
    let plan = plan.join("\n");
    assert!(plan.contains("steps_leased_by_idx"), "{plan}");
}

#[tokio::test]
async fn session_renewal_post_wait_cannot_revive_an_expired_worker_session() {
    let w = world().await;
    let (workflow, _) = w
        .publish(document("session-renew-postwait", "a", vec![activity("a")]))
        .await;
    let run = w.start(workflow, &json!({})).await;
    let session = w.session(&["act.a"]).await;
    let tenant = w.admin.tenant_id().as_uuid();
    let expires_at: time::OffsetDateTime = sqlx::query_scalar(
        "UPDATE worker_sessions SET expires_at=clock_timestamp()+interval '3 seconds' WHERE tenant_id=$1 AND id=$2 RETURNING expires_at",
    ).bind(tenant).bind(session.worker_id).fetch_one(&w.db.pool).await.expect("short persisted session expiry");
    let mut guard = w.db.pool.begin().await.expect("unchanged session guard");
    let backend: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *guard)
        .await
        .expect("session guard backend");
    sqlx::query("SELECT id FROM worker_sessions WHERE tenant_id=$1 AND id=$2 FOR UPDATE")
        .bind(tenant)
        .bind(session.worker_id)
        .fetch_one(&mut *guard)
        .await
        .expect("hold unchanged session tuple");
    // No bearer token or token hash is part of this independent effects oracle.
    let effects = || async {
        sqlx::query_as::<_,(i64,i64,i64,i64,time::OffsetDateTime)>(
            "SELECT r.run_version, \
             (SELECT count(*) FROM run_events WHERE tenant_id=$1 AND run_id=$2), \
             (SELECT count(*) FROM event_outbox WHERE tenant_id=$1 AND aggregate_id=$2), \
             (SELECT count(*) FROM steps WHERE tenant_id=$1 AND run_id=$2 AND status IN ('LEASED','RUNNING')), \
             s.expires_at FROM runs r JOIN worker_sessions s ON s.tenant_id=r.tenant_id \
             WHERE r.tenant_id=$1 AND r.id=$2 AND s.id=$3",
        ).bind(tenant).bind(run.as_uuid()).bind(session.worker_id).fetch_one(&w.db.pool)
        .await.expect("independent renewal and claim effects")
    };
    let before = effects().await;
    assert_eq!(before.3, 0, "fixture has no leased task before poll");
    let mut poll = Box::pin(w.workers.poll(
        &session.principal,
        session.worker_id,
        &session.token,
        1,
        Some("0af7651916cd43dd8448eb211c80319c"),
    ));
    tokio::select! {
        _ = &mut poll => panic!("poll must wait for the unchanged session row"),
        () = crate::support::eventually("actual app renewal waits on session while live", || async {
            sqlx::query_scalar::<_,bool>(
                "SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE datname=current_database() \
                 AND $1=ANY(pg_blocking_pids(pid)) \
                 AND (query LIKE 'UPDATE worker_sessions SET expires_at%' OR query LIKE '%FROM worker_sessions%') \
                 AND (NOT $2 OR usename='sagat_app'))",
            ).bind(backend).bind(crate::support::as_runtime_role()).fetch_one(&w.db.pool).await
            .expect("real session renewal wait").then_some(())
        }) => {}
    }
    let live: bool = sqlx::query_scalar("SELECT clock_timestamp()<$1")
        .bind(expires_at)
        .fetch_one(&w.db.pool)
        .await
        .expect("lease live at wait");
    assert!(live, "wait must begin before persisted expiry");
    crate::support::eventually(
        "persisted session expires under unchanged row guard",
        || async {
            sqlx::query_scalar::<_, bool>("SELECT clock_timestamp()>=$1")
                .bind(expires_at)
                .fetch_one(&w.db.pool)
                .await
                .expect("database session expiry")
                .then_some(())
        },
    )
    .await;
    guard
        .rollback()
        .await
        .expect("release unchanged expired session");
    let result = poll.await;
    let after = effects().await;
    let outcome = match &result {
        Ok(tasks) => format!("{} leased tasks", tasks.len()),
        Err(error) => format!("{error:?}"),
    };
    assert!(
        matches!(result, Err(AppError::Unauthenticated)),
        "expired renewal must be unauthenticated; outcome={outcome}, expiry_refreshed={}",
        after.4 > expires_at
    );
    assert_eq!(
        after, before,
        "refused renewal cannot lease or write journal/outbox"
    );
    let fresh = w.session(&["act.a"]).await;
    assert_ne!(fresh.worker_id, session.worker_id);
    let tasks = w
        .workers
        .poll(&fresh.principal, fresh.worker_id, &fresh.token, 1, None)
        .await
        .expect("fresh registered session can claim");
    assert_eq!(tasks.len(), 1);
    assert!(
        matches!(
            w.workers
                .poll(
                    &session.principal,
                    session.worker_id,
                    &session.token,
                    1,
                    None
                )
                .await,
            Err(AppError::Unauthenticated)
        ),
        "fresh registration does not revive the old session"
    );
    w.store
        .verify_run(w.admin.tenant_id(), run)
        .await
        .expect("claim journal matches projection");
}
