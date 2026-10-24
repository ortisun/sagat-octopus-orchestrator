//! Real PostgreSQL + OpenBao fixture from main: revision pins, current ACL and inherited authority.
#![allow(clippy::panic)]
use crate::support::engine::{World, child_node, ctx, document};
use sagat_application::{
    clock::SystemClock,
    database::{ConnectionService, ConnectionWrite},
    error::AppError,
    execution::child_run_id,
    runs::StartRun,
};
use sagat_domain::{
    database::{
        ConnectionConfig, ConnectionStatus, DatabaseLimits, MANAGED_DATABASE_ACTIVITY, TlsPolicy,
    },
    execution::RunId,
    identity::{Role, Slug},
    secrets::SecretName,
    workflow::NodeId,
};
use serde_json::{Value, json};
use uuid::Uuid;

fn workflow(name: &str, revision: Option<u32>, operation: &str) -> Value {
    let mut reference = json!({"name":"execution-db"});
    if let Some(revision) = revision {
        reference["revision"] = json!(revision);
    }
    document(
        name,
        "database",
        vec![
            json!({"id":"database","type":"database","connectionRef":reference,
        "operation":operation,"statement":"SELECT 1"}),
        ],
    )
}

async fn revision(w: &World, run: RunId) -> i32 {
    sqlx::query_scalar("SELECT revision FROM run_database_connections WHERE tenant_id=$1 AND namespace_id=$2 AND run_id=$3 AND node_id='database'")
        .bind(w.admin.tenant_id().as_uuid()).bind(w.namespace.id().as_uuid()).bind(run.as_uuid())
        .fetch_one(&w.db.pool).await.unwrap()
}

pub async fn run(
    w: &World,
    secrets: &sagat_application::managed_secrets::ManagedSecretService<
        sagat_infra_postgres::PgRunStore,
        SystemClock,
    >,
) {
    let ns = w.namespace.id();
    assert!(matches!(
        w.try_publish_as(&w.admin, workflow("missing-database", None, "query"))
            .await,
        Err(AppError::NotFound)
    ));
    let catalog = ConnectionService::new(w.store.clone(), SystemClock);
    let mut write = ConnectionWrite {
        name: Slug::parse("execution-db").unwrap(),
        expected_revision: 0,
        request_id: Uuid::new_v4(),
        status: ConnectionStatus::Active,
        config: ConnectionConfig::postgres(
            "db.example.com",
            5432,
            "orders",
            TlsPolicy::VerifyFull { ca_pem: None },
            SecretName::parse("db-credentials").unwrap(),
            DatabaseLimits::default(),
        )
        .unwrap(),
    };
    catalog
        .put(&w.admin, ns, write.clone(), &ctx())
        .await
        .unwrap();
    let developer = w.member("database-publisher", Role::Developer).await;
    assert!(matches!(
        w.try_publish_as(&developer, workflow("database-no-acl", None, "query"))
            .await,
        Err(AppError::NotFound)
    ));
    w.secrets
        .grant(
            &w.admin,
            ns,
            "db-credentials",
            developer.subject().as_str(),
            &ctx(),
        )
        .await
        .unwrap();
    assert!(matches!(
        w.try_publish_as(
            &developer,
            workflow("database-write-denied", None, "execute")
        )
        .await,
        Err(AppError::NotFound)
    ));
    let (flow, _) = w
        .try_publish_as(&developer, workflow("database-admission", None, "query"))
        .await
        .unwrap();
    promotion(w, secrets, flow, &write).await;
    let run = w
        .start_with(flow, &json!({}), "database-admission-start", None)
        .await
        .unwrap();
    assert_eq!(revision(w, run).await, 1);
    rollback_and_acl_fence(w, run, developer.subject().as_str()).await;
    w.bind_everything(&w.worker).await;
    assert!(
        matches!(
            w.workers
                .register(
                    &w.worker,
                    "payments",
                    "ordinary-database-worker",
                    &sagat_application::worker::Capabilities {
                        activities: vec![MANAGED_DATABASE_ACTIVITY.into()],
                        languages: vec![],
                        pools: vec![],
                        max_inflight: 1,
                        protocol_version: "1.6".into()
                    }
                )
                .await,
            Err(AppError::NotBound)
        ),
        "ordinary wildcard worker cannot register a managed activity"
    );

    write.expected_revision = 1;
    write.request_id = Uuid::new_v4();
    catalog
        .put(&w.admin, ns, write.clone(), &ctx())
        .await
        .unwrap();
    let next = w.start(flow, &json!({})).await;
    assert_eq!(revision(w, next).await, 2);
    let input = json!({});
    let (first, second) = tokio::join!(
        w.start_with(flow, &input, "database-concurrent-start", None),
        w.start_with(flow, &input, "database-concurrent-start", None)
    );
    assert_eq!(
        first.unwrap(),
        second.unwrap(),
        "concurrent starts share one run and its bindings"
    );
    w.operate(next, sagat_application::runs::RunOperation::Cancel)
        .await
        .unwrap();
    let rerun = w
        .runs
        .rerun(
            &w.admin,
            next,
            "verify database rerun admission",
            "database-rerun",
            &ctx(),
        )
        .await
        .unwrap();
    assert_eq!(revision(w, rerun.info.run_id).await, 2);
    assert_eq!(
        w.start_with(flow, &json!({}), "database-admission-start", None)
            .await
            .unwrap(),
        run
    );
    assert_eq!(
        revision(w, run).await,
        1,
        "receipt replay keeps the original binding"
    );
    let (fixed, _) = w
        .publish(workflow("database-fixed", Some(1), "query"))
        .await;
    assert_eq!(revision(w, w.start(fixed, &json!({})).await).await, 1);
    let mut triggered = workflow("database-trigger", None, "query");
    triggered["triggers"] = json!([{"kind":"event","eventType":"database.run"}]);
    let (trigger_flow, _) = w.try_publish_as(&developer, triggered).await.unwrap();
    trigger(w, "database-event-1").await;
    let triggered_run: Uuid =
        sqlx::query_scalar("SELECT id FROM runs WHERE tenant_id=$1 AND workflow_id=$2")
            .bind(w.admin.tenant_id().as_uuid())
            .bind(trigger_flow.as_uuid())
            .fetch_one(&w.db.pool)
            .await
            .unwrap();
    assert_eq!(revision(w, RunId::new(triggered_run)).await, 2);

    let (parent, _) = w
        .publish(document(
            "database-parent",
            "child",
            vec![child_node("child", "database-admission", None)],
        ))
        .await;
    let parent_run = w.start(parent, &json!({})).await;
    w.scheduler.tick().await;
    let child = child_run_id(parent_run, &NodeId::parse("child").unwrap());
    assert_eq!(
        revision(w, child).await,
        2,
        "child admission uses the same revision fence"
    );

    write.expected_revision = 2;
    write.request_id = Uuid::new_v4();
    write.status = ConnectionStatus::Paused;
    catalog
        .put(&w.admin, ns, write.clone(), &ctx())
        .await
        .unwrap();
    trigger(w, "database-event-paused").await;
    let trigger_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM runs WHERE tenant_id=$1 AND workflow_id=$2")
            .bind(w.admin.tenant_id().as_uuid())
            .bind(trigger_flow.as_uuid())
            .fetch_one(&w.db.pool)
            .await
            .unwrap();
    assert_eq!(
        trigger_count, 1,
        "paused connection prevents event admission"
    );
    assert!(matches!(
        w.start_with(flow, &json!({}), "database-paused", None)
            .await,
        Err(AppError::NotFound)
    ));
    assert!(
        matches!(
            w.start_with(flow, &json!({}), "database-admission-start", None)
                .await,
            Err(AppError::NotFound)
        ),
        "receipt rechecks current pause"
    );
    let paused_parent = w.start(parent, &json!({})).await;
    w.scheduler.tick().await;
    assert!(
        w.run_record(child_run_id(
            paused_parent,
            &NodeId::parse("child").unwrap()
        ))
        .await
        .is_none()
    );
    assert_eq!(
        w.step_status(paused_parent, "child").await,
        Some(sagat_domain::execution::StepStatus::Failed)
    );
    write.expected_revision = 3;
    write.request_id = Uuid::new_v4();
    write.status = ConnectionStatus::Active;
    catalog
        .put(&w.admin, ns, write.clone(), &ctx())
        .await
        .unwrap();

    w.secrets
        .revoke(
            &w.admin,
            ns,
            "db-credentials",
            developer.subject().as_str(),
            &ctx(),
        )
        .await
        .unwrap();
    assert!(
        matches!(
            w.start_with(flow, &json!({}), "database-revoked", None)
                .await,
            Err(AppError::NotFound)
        ),
        "admin caller does not bypass publisher revocation"
    );
    w.secrets
        .grant(
            &w.admin,
            ns,
            "db-credentials",
            developer.subject().as_str(),
            &ctx(),
        )
        .await
        .unwrap();
    let caller = w.member("database-caller", Role::Developer).await;
    assert!(
        matches!(
            w.runs
                .start(
                    &caller,
                    &StartRun {
                        namespace: "payments",
                        workflow: flow,
                        version: None,
                        input: b"{}",
                        correlation_id: None,
                        deadline: None,
                        key: "database-caller-denied"
                    },
                    &ctx()
                )
                .await,
            Err(AppError::NotFound)
        ),
        "caller also needs the secret ACL"
    );

    sqlx::query("UPDATE memberships SET role='viewer' WHERE tenant_id=$1 AND subject=$2")
        .bind(w.admin.tenant_id().as_uuid())
        .bind(developer.subject().as_str())
        .execute(&w.db.pool)
        .await
        .unwrap();
    assert!(matches!(
        w.start_with(flow, &json!({}), "database-demoted", None)
            .await,
        Err(AppError::NotFound)
    ));
    assert!(
        w.try_publish_as(
            &developer,
            workflow("database-stale-principal", None, "query")
        )
        .await
        .is_err()
    );
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM runs WHERE tenant_id=$1 AND workflow_id=$2")
            .bind(w.admin.tenant_id().as_uuid())
            .bind(flow.as_uuid())
            .fetch_one(&w.db.pool)
            .await
            .unwrap();
    assert_eq!(
        count, 5,
        "three direct runs, one rerun and one child; refusals create no runs"
    );
    let denials: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_records WHERE tenant_id=$1 AND action='database.denied'",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .unwrap();
    assert!(
        denials >= 4,
        "publication, start and trigger refusals are audited"
    );
}

async fn promotion(
    w: &World,
    secrets: &sagat_application::managed_secrets::ManagedSecretService<
        sagat_infra_postgres::PgRunStore,
        SystemClock,
    >,
    source: sagat_domain::definition::WorkflowId,
    connection: &ConnectionWrite,
) {
    let target = w
        .add_namespace_in(
            "database-staging",
            sagat_domain::identity::Environment::Staging,
        )
        .await;
    assert!(
        matches!(
            w.publishing
                .promote(
                    &w.admin,
                    source,
                    1,
                    target.id(),
                    "database promotion validation",
                    false,
                    "database-promotion",
                    &ctx()
                )
                .await,
            Err(AppError::NotFound)
        ),
        "a source connection grants no authority in a different namespace"
    );
    secrets
        .put(
            &w.admin,
            target.id(),
            sagat_application::managed_secrets::ManagedSecretWrite {
                name: "db-credentials",
                value: sagat_domain::secrets::SecretValue::new(b"staging-canary".to_vec()).unwrap(),
                expected_version: 0,
                request_id: Uuid::new_v4(),
            },
            &ctx(),
        )
        .await
        .unwrap();
    let catalog = ConnectionService::new(w.store.clone(), SystemClock);
    let mut write = connection.clone();
    write.request_id = Uuid::new_v4();
    catalog
        .put(&w.admin, target.id(), write.clone(), &ctx())
        .await
        .unwrap();
    w.publishing
        .promote(
            &w.admin,
            source,
            1,
            target.id(),
            "database promotion validation",
            false,
            "database-promotion",
            &ctx(),
        )
        .await
        .unwrap();
    write.expected_revision = 1;
    write.request_id = Uuid::new_v4();
    write.status = ConnectionStatus::Paused;
    catalog
        .put(&w.admin, target.id(), write, &ctx())
        .await
        .unwrap();
    assert!(
        matches!(
            w.publishing
                .promote(
                    &w.admin,
                    source,
                    1,
                    target.id(),
                    "database promotion validation",
                    false,
                    "database-promotion",
                    &ctx()
                )
                .await,
            Err(AppError::NotFound)
        ),
        "AlreadyActive also rechecks target authority"
    );
}

async fn trigger(w: &World, id: &str) {
    w.events
        .publish(
            &w.admin,
            sagat_application::events::PublishRequest {
                namespace: "payments".into(),
                event_id: Some(sagat_domain::events::EventId::parse(id).unwrap()),
                event_type: sagat_domain::workflow::EventType::parse("database.run").unwrap(),
                subject: None,
                correlation_key: None,
                occurred_at: None,
                data: b"{}".to_vec(),
                dedupe_ttl: None,
            },
            &ctx(),
        )
        .await
        .unwrap();
    w.dispatcher.tick().await;
}

async fn rollback_and_acl_fence(w: &World, existing: RunId, publisher: &str) {
    use sagat_application::{
        definitions::{Applied, Idempotency},
        quotas::RunLimits,
        runs::{NewRun, RunStore},
    };
    use sagat_domain::audit::{AuditAction, AuditRecord, ResourceType};
    let mut info = w.run_record(existing).await.unwrap().info;
    info.run_id = RunId::generate();
    info.root = info.run_id;
    let run = info.run_id;
    let new = NewRun {
        retained: None,
        limits: RunLimits::of(&w.store, info.tenant_id, info.namespace_id)
            .await
            .unwrap(),
        definition: w.cache.get(info.version_id).unwrap(),
        input: b"{}".to_vec(),
        input_blob: None,
        audit: AuditRecord::by(
            &w.admin,
            AuditAction::RunStarted,
            ResourceType::Run,
            run.to_string(),
            None,
            time::OffsetDateTime::now_utc(),
        ),
        idempotency: Idempotency {
            scope: "database-test-rollback".into(),
            key: Uuid::new_v4().to_string(),
            fingerprint: vec![1],
        },
        info,
    };
    let held = match w.store.create(new).await.unwrap() {
        Applied::Done(tx) => tx,
        _ => panic!("new run"),
    };
    let request = ctx();
    let revoke = w.secrets.revoke(
        &w.admin,
        w.namespace.id(),
        "db-credentials",
        publisher,
        &request,
    );
    tokio::pin!(revoke);
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), &mut revoke)
            .await
            .is_err(),
        "ACL revocation waits for admission's share lock"
    );
    drop(held); // crash before commit: the run, its receipt and all connection pins disappear.
    revoke.await.unwrap();
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM run_database_connections WHERE tenant_id=$1 AND run_id=$2",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(run.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .unwrap();
    assert_eq!(count, 0);
    assert!(w.run_record(run).await.is_none());
    w.secrets
        .grant(
            &w.admin,
            w.namespace.id(),
            "db-credentials",
            publisher,
            &request,
        )
        .await
        .unwrap();
}
