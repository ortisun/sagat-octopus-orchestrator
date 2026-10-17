//! Tenant boundaries of the execution plane (NFR-002-SEC; US04 of feature 001 applied to runs).

use std::time::Duration;

use sagat_application::error::AppError;
use sagat_application::identity::PageRequest;
use sagat_application::runs::{RunOperation, RunStore};
use sagat_application::worker::TaskRef;
use sagat_domain::execution::RunId;
use sagat_domain::identity::{Principal, Role};
use serde_json::json;

use crate::support::engine::{activity, chain, ctx, document, task_ref, wait_node, with, world};
use crate::support::{eventually, subject};

#[tokio::test]
async fn another_tenant_cannot_read_operate_or_signal_a_run_and_the_attempt_is_audited() {
    let w = world().await;
    let (wf, _) = w
        .publish(document(
            "private",
            "w",
            vec![
                with(
                    wait_node("w", "e", "input.k", 60_000, None),
                    "next",
                    json!("a"),
                ),
                activity("a"),
            ],
        ))
        .await;
    let run = w.start(wf, &json!({"k": "secret-key"})).await;
    let intruder = &w.other_admin;

    assert_eq!(
        w.runs.get(intruder, run, &ctx()).await.map(|_| ()),
        Err(AppError::NotFound)
    );
    assert_eq!(
        w.runs
            .journal(intruder, run, &PageRequest::default(), &ctx())
            .await
            .map(|_| ()),
        Err(AppError::NotFound)
    );
    assert_eq!(
        w.runs.graph(intruder, run, &ctx()).await.map(|_| ()),
        Err(AppError::NotFound)
    );
    assert_eq!(
        w.runs
            .operate(
                intruder,
                run,
                RunOperation::Cancel,
                "I should not be able to",
                None,
                "intruder-key-1",
                &ctx()
            )
            .await
            .map(|_| ()),
        Err(AppError::NotFound)
    );
    assert_eq!(
        w.runs
            .signal(intruder, run, "e", b"{}", "intruder-key-2", &ctx())
            .await
            .map(|_| ()),
        Err(AppError::NotFound)
    );
    // Nothing happened to the run.
    assert_eq!(w.journal(run).await.len(), 2);
    w.assert_consistent(run).await;

    // The attempts were recorded, in the intruder's tenant, without anything about the victim.
    let records: Vec<(String, String)> = eventually("cross-tenant denials", || async {
        let rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT resource_id, tenant_id::text FROM audit_records WHERE action = 'access.denied.cross_tenant' AND resource_type = 'run'",
        )
        .fetch_all(&w.db.pool)
        .await
        .expect("audit");
        (!rows.is_empty()).then_some(rows)
    })
    .await;
    assert!(
        records
            .iter()
            .all(|(id, tenant)| *id == run.to_string()
                && *tenant == intruder.tenant_id().to_string())
    );
}

#[tokio::test]
async fn a_run_that_does_not_exist_anywhere_is_indistinguishable_and_leaves_no_trace() {
    let w = world().await;
    let ghost = RunId::generate();
    assert_eq!(
        w.runs.get(&w.admin, ghost, &ctx()).await.map(|_| ()),
        Err(AppError::NotFound)
    );
    tokio::time::sleep(Duration::from_millis(300)).await;
    let denials: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_records WHERE action = 'access.denied.cross_tenant'",
    )
    .fetch_one(&w.db.pool)
    .await
    .expect("count");
    assert_eq!(denials, 0);
}

#[tokio::test]
async fn listings_never_include_another_tenants_runs() {
    let w = world().await;
    let (wf, _) = w.publish(chain("listed")).await;
    let mine = w.start(wf, &json!({})).await;
    let listed = w.runs_of(&Default::default()).await;
    assert_eq!(listed, [mine]);
    // The other tenant's namespace has the same slug, but its list is its own.
    let theirs = w
        .store
        .list(
            w.other_admin.tenant_id(),
            w.other_namespace.id(),
            &Default::default(),
            None,
            50,
        )
        .await
        .expect("list");
    assert!(theirs.is_empty());
    // And asking for a namespace id of another tenant lists nothing rather than leaking.
    let crossed = w
        .store
        .list(
            w.other_admin.tenant_id(),
            w.namespace.id(),
            &Default::default(),
            None,
            50,
        )
        .await
        .expect("list");
    assert!(crossed.is_empty());
}

#[tokio::test]
async fn workers_of_another_tenant_never_receive_tasks_and_cannot_use_foreign_sessions() {
    let w = world().await;
    let (wf, _) = w.publish(chain("mine")).await;
    w.start(wf, &json!({})).await;

    // A worker of the other tenant registers for *its* namespace of the same name.
    let outsider = Principal::new(
        w.other_admin.tenant_id(),
        subject("their-worker"),
        Role::Worker,
    );
    sagat_infra_postgres::PgIdentityStore::new(w.db.pool.clone())
        .insert_membership(&sagat_domain::identity::Membership::restore(
            outsider.tenant_id(),
            outsider.subject().clone(),
            Role::Worker,
            crate::support::now(),
        ))
        .await
        .expect("foreign session identity membership");
    let theirs = w.session_as(&outsider, &["act.a", "act.b"], 10).await;
    assert!(
        w.poll(&theirs, 10).await.is_empty(),
        "same slug, different tenant: nothing to claim"
    );

    // Presenting my worker's session id and token as the other tenant fails.
    let mine = w.session(&["act.a", "act.b"]).await;
    let stolen = w
        .workers
        .poll(&outsider, mine.worker_id, &mine.token, 4, None)
        .await;
    assert_eq!(stolen.map(|_| ()), Err(AppError::Unauthenticated));
    // A different identity of the same tenant cannot use it either.
    let colleague = Principal::new(w.admin.tenant_id(), subject("another-worker"), Role::Worker);
    let borrowed = w
        .workers
        .poll(&colleague, mine.worker_id, &mine.token, 4, None)
        .await;
    assert_eq!(borrowed.map(|_| ()), Err(AppError::Unauthenticated));
    // A wrong token for the right identity fails.
    let wrong = w
        .workers
        .poll(&w.worker, mine.worker_id, &"0".repeat(64), 4, None)
        .await;
    assert_eq!(wrong.map(|_| ()), Err(AppError::Unauthenticated));
    // The right one still works.
    assert_eq!(w.poll(&mine, 4).await.len(), 1);
}

#[tokio::test]
async fn an_activity_quota_never_crosses_the_tenant_boundary() {
    let w = world().await;
    sqlx::query(
        "INSERT INTO activity_quota_limits \
         (tenant_id, namespace_id, activity, max_inflight, updated_by) VALUES ($1, $2, 'act.a', 0, 'test')",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(w.namespace.id().as_uuid())
    .execute(&w.db.pool)
    .await
    .expect("activity quota");
    let (mine, _) = w
        .publish(document("capped-mine", "a", vec![activity("a")]))
        .await;
    w.start(mine, &json!({})).await;
    let theirs = w
        .publish_as_other_tenant(document("uncapped-theirs", "a", vec![activity("a")]))
        .await;
    w.start_as_other_tenant(theirs, &json!({}))
        .await
        .expect("other run");

    let mine = w.session(&["act.a"]).await;
    assert!(w.poll(&mine, 1).await.is_empty());
    let outsider = Principal::new(
        w.other_admin.tenant_id(),
        subject("their-worker"),
        Role::Worker,
    );
    sagat_infra_postgres::PgIdentityStore::new(w.db.pool.clone())
        .insert_membership(&sagat_domain::identity::Membership::restore(
            outsider.tenant_id(),
            outsider.subject().clone(),
            Role::Worker,
            crate::support::now(),
        ))
        .await
        .expect("other tenant's worker membership");
    let theirs = w.session_as(&outsider, &["act.a"], 1).await;
    assert_eq!(w.poll(&theirs, 1).await.len(), 1);
}

#[tokio::test]
async fn a_worker_of_another_tenant_cannot_report_on_my_attempt_even_with_its_token() {
    let w = world().await;
    let (wf, _) = w.publish(chain("stolen-lease")).await;
    let run = w.start(wf, &json!({})).await;
    let session = w.session(&["act.a", "act.b"]).await;
    let task = w.one_task(&session).await;

    let outsider = Principal::new(
        w.other_admin.tenant_id(),
        subject("their-worker"),
        Role::Worker,
    );
    let reference: TaskRef<'_> = task_ref(&task);
    assert_eq!(
        w.workers.heartbeat(&outsider, &reference).await.map(|_| ()),
        Err(AppError::StaleAttempt)
    );
    assert_eq!(
        w.workers
            .complete(&outsider, &reference, b"{}", "outsider-key-1")
            .await
            .map(|_| ()),
        Err(AppError::StaleAttempt)
    );
    assert_eq!(
        w.workers
            .fail(&outsider, &reference, "X", false, "outsider-key-2")
            .await
            .map(|_| ()),
        Err(AppError::StaleAttempt)
    );
    w.assert_consistent(run).await;
    w.complete(&session, &task, &json!({}))
        .await
        .expect("the real holder is unaffected");
}

#[tokio::test]
async fn the_database_refuses_cross_tenant_references_in_the_execution_tables() {
    let w = world().await;
    let (wf, _) = w.publish(chain("fk-check")).await;
    let run = w.start(wf, &json!({})).await;
    let theirs = w.other_admin.tenant_id().as_uuid();

    // A step of my run filed under the other tenant.
    let result = sqlx::query(
        "INSERT INTO steps (tenant_id, run_id, node, step_id, namespace_id, status, updated_at) \
         VALUES ($1, $2, 'ghost', gen_random_uuid(), $3, 'PENDING', now())",
    )
    .bind(theirs)
    .bind(run.as_uuid())
    .bind(w.other_namespace.id().as_uuid())
    .execute(&w.db.pool)
    .await
    .expect_err("composite foreign key");
    assert_eq!(crate::support::db_error(result).0, "23503");

    // A journal entry of my run filed under the other tenant.
    let result = sqlx::query(
        "INSERT INTO run_events (tenant_id, run_id, seq, event_type, detail, actor, occurred_at) \
         VALUES ($1, $2, 99, 'x', '{}'::jsonb, 'x', now())",
    )
    .bind(theirs)
    .bind(run.as_uuid())
    .execute(&w.db.pool)
    .await
    .expect_err("composite foreign key");
    assert_eq!(crate::support::db_error(result).0, "23503");

    // A run in my tenant reading the other tenant's input payload.
    let digest: Vec<u8> = sqlx::query_scalar("SELECT input_digest FROM runs WHERE id = $1")
        .bind(run.as_uuid())
        .fetch_one(&w.db.pool)
        .await
        .expect("digest");
    let result = sqlx::query(
        "WITH fresh AS (SELECT gen_random_uuid() AS id) \
         INSERT INTO runs (id, tenant_id, namespace_id, workflow_id, workflow_version_id, status, input_digest, started_by, created_at, updated_at, root_run_id) \
         SELECT fresh.id, $1, $2, workflow_id, workflow_version_id, 'RUNNING', $3, 'x', now(), now(), fresh.id FROM runs, fresh WHERE runs.id = $4",
    )
    .bind(theirs)
    .bind(w.other_namespace.id().as_uuid())
    .bind(&digest)
    .bind(run.as_uuid())
    .execute(&w.db.pool)
    .await
    .expect_err("workflow/version/payload belong to the other tenant");
    assert_eq!(crate::support::db_error(result).0, "23503");
}

#[tokio::test]
async fn roles_gate_every_run_operation() {
    let w = world().await;
    let (wf, _) = w.publish(chain("roles")).await;
    let run = w.start(wf, &json!({})).await;
    let tenant = w.admin.tenant_id();
    let as_role = |role| Principal::new(tenant, subject("someone"), role);

    // Reading: everyone but the machine identity.
    for role in [Role::Admin, Role::Developer, Role::Operator, Role::Viewer] {
        assert!(
            w.runs.get(&as_role(role), run, &ctx()).await.is_ok(),
            "{role}"
        );
    }
    assert!(matches!(
        w.runs.get(&as_role(Role::Worker), run, &ctx()).await,
        Err(AppError::Forbidden { .. })
    ));
    // Operating and signalling: operators and admins only.
    for role in [Role::Developer, Role::Viewer, Role::Worker] {
        let denied = w
            .runs
            .operate(
                &as_role(role),
                run,
                RunOperation::Cancel,
                "not allowed to",
                None,
                "role-key-0001",
                &ctx(),
            )
            .await;
        assert!(matches!(denied, Err(AppError::Forbidden { .. })), "{role}");
        let denied = w
            .runs
            .signal(&as_role(role), run, "e", b"{}", "role-key-0002", &ctx())
            .await;
        assert!(matches!(denied, Err(AppError::Forbidden { .. })), "{role}");
    }
    // Starting: not viewers, not machines.
    for role in [Role::Viewer, Role::Worker] {
        let denied = w
            .runs
            .start(
                &as_role(role),
                &sagat_application::runs::StartRun {
                    namespace: "payments",
                    workflow: wf,
                    version: None,
                    input: b"{}",
                    correlation_id: None,
                    deadline: None,
                    key: "role-key-0003",
                },
                &ctx(),
            )
            .await;
        assert!(matches!(denied, Err(AppError::Forbidden { .. })), "{role}");
    }
    // Machines may do only what machines do.
    assert!(matches!(
        w.workers
            .register(
                &as_role(Role::Viewer),
                "payments",
                "x",
                &sagat_application::worker::Capabilities {
                    activities: vec!["a".into()],
                    languages: vec![],
                    pools: vec![],
                    max_inflight: 1,
                    protocol_version: "1".into()
                }
            )
            .await,
        Err(AppError::Forbidden { .. })
    ));
}

#[tokio::test]
async fn a_workflow_is_never_found_by_name_through_another_tenants_namespace() {
    use sagat_application::definitions::DefinitionStore;
    use serde_json::json;

    let w = world().await;
    let doc = document(
        "shared-name",
        "a",
        vec![json!({ "id": "a", "type": "activity", "activity": "act.a" })],
    );
    w.publish_as_other_tenant(doc.clone()).await;
    w.publish(doc).await;
    let name = crate::support::slug("shared-name");
    let own = w
        .store
        .find_workflow_by_name(w.admin.tenant_id(), w.namespace.id(), &name)
        .await
        .expect("lookup");
    assert!(own.is_some(), "found in its own namespace");
    // Tenant A asking for tenant B's namespace gets nothing, whatever the name.
    let crossed = w
        .store
        .find_workflow_by_name(w.admin.tenant_id(), w.other_namespace.id(), &name)
        .await
        .expect("lookup");
    assert!(crossed.is_none(), "the tenant filter holds on its own");
}
