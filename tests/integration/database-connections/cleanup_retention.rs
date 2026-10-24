//! Current/grace versions and live/manual-review pins survive the operator cleanup path.
use super::{
    cleanup::{age_operations, runtime_store},
    support::engine::{Session, ctx, document, task_ref, world},
};
use sagat_application::{
    clock::SystemClock,
    database::{ConnectionService, ConnectionWrite},
    managed_secrets::{ManagedSecretService, ManagedSecretWrite},
    runs::{ReviewAction, RunOperation},
    secret_cleanup::{SecretCleanupService, SecretCleanupStore},
    secret_provider::SecretProvider,
    worker::Capabilities,
};
use sagat_domain::{
    database::{
        ConnectionConfig, ConnectionStatus, DatabaseLimits, MANAGED_DATABASE_ACTIVITY, TlsPolicy,
    },
    execution::ReviewOutcome,
    identity::{Role, Slug},
    secrets::{SecretName, SecretValue},
};
use sagat_infra_ir::JsonDatabaseResultValidator;
use sagat_infra_secrets::database::JsonDatabaseCredentials;
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

pub async fn run(provider: Arc<dyn SecretProvider>) {
    let mut w = world().await;
    w.workers = w
        .workers
        .with_secret_provider(provider.clone())
        .with_database_content(
            Arc::new(JsonDatabaseCredentials),
            Arc::new(JsonDatabaseResultValidator),
        );
    let store = runtime_store(&w).await;
    let cleanup = SecretCleanupService::new(store.clone(), provider.clone());
    let ns = w.namespace.id();
    let secrets = ManagedSecretService::new(
        w.store.clone(),
        SystemClock,
        provider.clone(),
        SecretValue::new(vec![42; 32]).unwrap(),
    )
    .unwrap();
    let rotation = |expected_version| {
        ManagedSecretWrite { name:"retained-credential", expected_version, request_id:Uuid::new_v4(), value: SecretValue::new(serde_json::to_vec(&json!({"username":"reader", "password":format!("retention-password-{expected_version}")})).unwrap()).unwrap() }
    };
    secrets
        .put(&w.admin, ns, rotation(0), &ctx())
        .await
        .unwrap();
    ConnectionService::new(w.store.clone(), SystemClock)
        .put(
            &w.admin,
            ns,
            ConnectionWrite {
                name: Slug::parse("retained-db").unwrap(),
                expected_revision: 0,
                request_id: Uuid::new_v4(),
                status: ConnectionStatus::Active,
                config: ConnectionConfig::postgres(
                    "db.example.com",
                    5432,
                    "orders",
                    TlsPolicy::VerifyFull { ca_pem: None },
                    SecretName::parse("retained-credential").unwrap(),
                    DatabaseLimits::default(),
                )
                .unwrap(),
            },
            &ctx(),
        )
        .await
        .unwrap();
    let definition = |name, operation, statement| {
        document(
            name,
            "db",
            vec![
                json!({"id":"db","type":"database","connectionRef":{"name":"retained-db"},"operation":operation,"statement":statement}),
            ],
        )
    };
    let (query, _) = w
        .publish(definition("retained-query", "query", "SELECT 1"))
        .await;
    let (write, _) = w
        .publish(definition(
            "retained-write",
            "execute",
            "INSERT INTO orders VALUES (1)",
        ))
        .await;
    let machine = w.member("cleanup-machine", Role::DatabaseRunner).await;
    w.bind_everything(&machine).await;
    let registration = w
        .workers
        .register(
            &machine,
            "payments",
            "cleanup-machine",
            &Capabilities {
                activities: vec![MANAGED_DATABASE_ACTIVITY.into()],
                languages: vec![],
                pools: vec![],
                max_inflight: 4,
                protocol_version: "1.5".into(),
            },
        )
        .await
        .unwrap();
    let session = Session {
        principal: machine.clone(),
        worker_id: registration.worker_id,
        token: registration.session_token.expose().to_owned(),
    };
    let live = w.start(query, &json!({})).await;
    let review = w.start(write, &json!({})).await;
    let terminal = w.start(query, &json!({})).await;
    let tasks = w.poll(&session, 4).await;
    assert_eq!(tasks.len(), 3);
    for task in &tasks {
        w.workers
            .resolve_database(&machine, &task_ref(task), None)
            .await
            .unwrap();
    }
    let pending = tasks.iter().find(|task| task.run_id == review).unwrap();
    w.fail_with(&session, pending, "OUTCOME_UNKNOWN", false)
        .await
        .unwrap();
    assert_eq!(w.run_status(review).await.as_str(), "MANUAL_REVIEW");
    let done = tasks.iter().find(|task| task.run_id == terminal).unwrap();
    w.complete(&session,done,&json!({"columns":[{"name":"value","type":"integer"}],"rows":[[{"type":"integer","value":"1"}]],"affectedRows":"1","truncated":false})).await.unwrap();
    for version in 1..8 {
        secrets
            .put(&w.admin, ns, rotation(version), &ctx())
            .await
            .unwrap();
    }
    age_operations(&w.db.pool).await;
    let result = cleanup.run_batch(32).await.unwrap();
    assert_eq!(
        result.retired, 2,
        "only versions 2 and 3 are outside the retained five and unpinned"
    );
    assert_eq!(
        result.pins_released, 0,
        "recent terminal pin has a 24 hour grace period"
    );
    let age_terminal = || async {
        sqlx::query("UPDATE runs SET finished_at=clock_timestamp()-interval '25 hours' WHERE finished_at IS NOT NULL").execute(&w.db.pool).await.unwrap();
    };
    age_terminal().await;
    let result = cleanup.run_batch(32).await.unwrap();
    assert_eq!((result.pins_released, result.retired), (1, 0));
    assert!(
        store
            .provider_cleanup_candidates(32)
            .await
            .unwrap()
            .is_empty()
    );
    let pinned: i64 = sqlx::query_scalar("SELECT count(*) FROM step_database_credentials")
        .fetch_one(&w.db.pool)
        .await
        .unwrap();
    assert_eq!(
        pinned, 2,
        "live and manual-review attempts keep the old provider object"
    );
    w.operate_run(live, RunOperation::Cancel).await;
    // Cancellation waits for the leased executor to drain/expire; until then its pin remains live.
    w.age_lease(live, "db").await;
    w.scheduler.tick().await;
    assert_eq!(w.run_status(live).await.as_str(), "CANCELLED");
    w.review(
        review,
        "db",
        ReviewAction::Resolve(ReviewOutcome::Succeeded),
        "database independently reconciled",
    )
    .await
    .unwrap();
    age_terminal().await;
    let result = cleanup.run_batch(32).await.unwrap();
    assert_eq!((result.pins_released, result.retired), (2, 1));
    let retained: i64 = sqlx::query_scalar("SELECT count(*) FROM secret_provider_operations")
        .fetch_one(&w.db.pool)
        .await
        .unwrap();
    assert_eq!(
        retained, 5,
        "current and four grace versions remain readable"
    );
    let references: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM secret_provider_operations")
        .fetch_all(&w.db.pool)
        .await
        .unwrap();
    for operation in references {
        assert!(
            provider
                .read(&sagat_application::secret_provider::ProviderReference {
                    tenant: w.admin.tenant_id(),
                    namespace: ns,
                    operation
                })
                .await
                .unwrap()
                .is_some()
        );
    }
}
