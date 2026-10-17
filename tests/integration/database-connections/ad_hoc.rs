use crate::support::engine::{World, ctx, document};
use sagat_application::{
    clock::SystemClock,
    database::{ConnectionService, ConnectionWrite},
    definitions::DefinitionStore,
    error::AppError,
    runs::{AdHocMode, RunOperation, StartDatabaseExecution},
};
use sagat_domain::{
    database::{ConnectionConfig, ConnectionStatus, DatabaseLimits, TlsPolicy},
    identity::Slug,
    secrets::SecretName,
};
use serde_json::json;
use uuid::Uuid;

pub async fn run(w: &World) {
    let catalog = ConnectionService::new(w.store.clone(), SystemClock);
    catalog
        .put(
            &w.admin,
            w.namespace.id(),
            ConnectionWrite {
                name: Slug::parse("ad-hoc-db").unwrap(),
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
            },
            &ctx(),
        )
        .await
        .unwrap();
    let bytes = serde_json::to_vec(&document(
        "database-ad-hoc",
        "query",
        vec![json!({
            "id":"query", "type":"database", "connectionRef":{"name":"ad-hoc-db"},
            "operation":"query", "statement":"SELECT 1::bigint"
        })],
    ))
    .unwrap();
    let request = StartDatabaseExecution {
        namespace: w.namespace.id(),
        mode: AdHocMode::Step,
        document: &bytes,
        input: b"{}",
        timeout_ms: None,
        key: "ad-hoc-create-001",
    };
    let first = w
        .runs
        .start_database(&w.admin, &request, &ctx())
        .await
        .unwrap();
    assert_eq!(first.info.origin.unwrap().as_str(), "AD_HOC_STEP");
    let replay = w
        .runs
        .start_database(&w.admin, &request, &ctx())
        .await
        .unwrap();
    assert_eq!(first.info.run_id, replay.info.run_id);
    let second = w
        .runs
        .start_database(
            &w.admin,
            &StartDatabaseExecution {
                key: "ad-hoc-create-002",
                ..request.clone()
            },
            &ctx(),
        )
        .await
        .unwrap();
    assert_ne!(first.info.run_id, second.info.run_id);
    assert_eq!(first.info.version_id, second.info.version_id);
    assert!(
        w.store
            .get_workflow(w.admin.tenant_id(), first.info.workflow_id)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        w.store
            .get_version_by_id(w.admin.tenant_id(), first.info.version_id)
            .await
            .unwrap()
            .is_none()
    );
    assert!(matches!(
        w.runs
            .start_database(&w.other_admin, &request, &ctx())
            .await,
        Err(AppError::NotFound)
    ));
    assert!(matches!(
        w.runs
            .start_database(
                &w.admin,
                &StartDatabaseExecution {
                    input: b"{\"changed\":true}",
                    ..request.clone()
                },
                &ctx()
            )
            .await,
        Err(AppError::IdempotencyKeyReused)
    ));
    w.operate(first.info.run_id, RunOperation::Cancel)
        .await
        .unwrap();
    let rerun = w
        .runs
        .rerun(
            &w.admin,
            first.info.run_id,
            "repeat the database check",
            "ad-hoc-rerun-001",
            &ctx(),
        )
        .await
        .unwrap();
    assert_eq!(rerun.info.origin, first.info.origin);
    assert_eq!(rerun.info.rerun_of, Some(first.info.run_id));
    assert_eq!(rerun.info.version_id, first.info.version_id);
    deeper_admission(w, &request, &first).await;
    rollback_before_commit(w, &request).await;
    w.store
        .verify_run(w.admin.tenant_id(), first.info.run_id)
        .await
        .unwrap();
}

async fn deeper_admission(
    w: &World,
    request: &StartDatabaseExecution<'_>,
    original: &sagat_application::execution::RunRecord,
) {
    use sagat_domain::{identity::Role, quota::QuotaResource};
    let developer = w.member("ad-hoc-developer", Role::Developer).await;
    let operator = w.member("ad-hoc-operator", Role::Operator).await;
    assert!(matches!(
        w.runs.start_database(&operator, request, &ctx()).await,
        Err(AppError::Forbidden { .. })
    ));
    assert!(matches!(
        w.runs.start_database(&developer, request, &ctx()).await,
        Err(AppError::IdempotencyKeyReused)
    ));
    let fresh = StartDatabaseExecution {
        key: "ad-hoc-developer-first",
        ..request.clone()
    };
    assert!(matches!(
        w.runs.start_database(&developer, &fresh, &ctx()).await,
        Err(AppError::NotFound)
    ));
    w.secrets
        .grant(
            &w.admin,
            w.namespace.id(),
            "db-credentials",
            developer.subject().as_str(),
            &ctx(),
        )
        .await
        .unwrap();
    let dev_run = w
        .runs
        .start_database(&developer, &fresh, &ctx())
        .await
        .unwrap();
    assert_ne!(
        dev_run.info.version_id, original.info.version_id,
        "different actors retain their own publisher"
    );
    let mut write: serde_json::Value = serde_json::from_slice(request.document).unwrap();
    write["nodes"][0]["operation"] = json!("execute");
    write["nodes"][0]["statement"] = json!("INSERT INTO orders(id) VALUES(1)");
    let write_bytes = serde_json::to_vec(&write).unwrap();
    assert!(matches!(
        w.runs
            .start_database(
                &developer,
                &StartDatabaseExecution {
                    key: "ad-hoc-write-denied",
                    document: &write_bytes,
                    ..request.clone()
                },
                &ctx()
            )
            .await,
        Err(AppError::Forbidden { .. })
    ));
    wait_denials(
        &w.db.pool,
        w.admin.tenant_id(),
        &[operator.subject().as_str(), developer.subject().as_str()],
        2,
    )
    .await;
    w.secrets
        .revoke(
            &w.admin,
            w.namespace.id(),
            "db-credentials",
            developer.subject().as_str(),
            &ctx(),
        )
        .await
        .unwrap();
    assert!(
        matches!(
            w.runs.start_database(&developer, &fresh, &ctx()).await,
            Err(AppError::NotFound)
        ),
        "receipt replay rechecks current ACL"
    );

    // Use the cached admin principal after an actual role change. A plain query remains authorized,
    // so the rerun refusal specifically proves the current RunOperate gate.
    w.secrets
        .grant(
            &w.admin,
            w.namespace.id(),
            "db-credentials",
            w.admin.subject().as_str(),
            &ctx(),
        )
        .await
        .unwrap();
    sqlx::query("UPDATE memberships SET role='developer' WHERE tenant_id=$1 AND subject=$2")
        .bind(w.admin.tenant_id().as_uuid())
        .bind(w.admin.subject().as_str())
        .execute(&w.db.pool)
        .await
        .unwrap();
    w.runs
        .start_database(
            &w.admin,
            &StartDatabaseExecution {
                key: "ad-hoc-downgraded-query",
                ..request.clone()
            },
            &ctx(),
        )
        .await
        .unwrap();
    for key in ["ad-hoc-rerun-001", "ad-hoc-revoked-rerun"] {
        assert!(matches!(
            w.runs
                .rerun(
                    &w.admin,
                    original.info.run_id,
                    "repeat the database check",
                    key,
                    &ctx()
                )
                .await,
            Err(AppError::NotFound)
        ));
    }
    sqlx::query("UPDATE memberships SET role='admin' WHERE tenant_id=$1 AND subject=$2")
        .bind(w.admin.tenant_id().as_uuid())
        .bind(w.admin.subject().as_str())
        .execute(&w.db.pool)
        .await
        .unwrap();
    let other = w.member("ad-hoc-other-admin", Role::Admin).await;
    assert!(matches!(
        w.runs
            .rerun(
                &other,
                original.info.run_id,
                "not my private definition",
                "ad-hoc-other-rerun",
                &ctx()
            )
            .await,
        Err(AppError::NotFound)
    ));

    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM workflows WHERE tenant_id=$1 AND namespace_id=$2")
            .bind(w.admin.tenant_id().as_uuid())
            .bind(w.namespace.id().as_uuid())
            .fetch_one(&w.db.pool)
            .await
            .unwrap();
    w.quotas
        .set(
            &w.admin,
            Some(w.namespace.id()),
            QuotaResource::Workflows,
            count as u64,
            &ctx(),
        )
        .await
        .unwrap();
    w.runs
        .start_database(
            &w.admin,
            &StartDatabaseExecution {
                key: "ad-hoc-full-quota-reuse",
                ..request.clone()
            },
            &ctx(),
        )
        .await
        .unwrap();
    assert!(matches!(
        w.runs
            .start_database(
                &w.admin,
                &StartDatabaseExecution {
                    key: "ad-hoc-full-quota-new",
                    document: &write_bytes,
                    ..request.clone()
                },
                &ctx()
            )
            .await,
        Err(AppError::QuotaExceeded { .. })
    ));
    w.quotas
        .set(
            &w.admin,
            Some(w.namespace.id()),
            QuotaResource::Workflows,
            1000,
            &ctx(),
        )
        .await
        .unwrap();
    let parallel = StartDatabaseExecution {
        key: "ad-hoc-concurrent",
        document: &write_bytes,
        ..request.clone()
    };
    let context = ctx();
    let (a, b) = tokio::join!(
        w.runs.start_database(&w.admin, &parallel, &context),
        w.runs.start_database(&w.admin, &parallel, &context)
    );
    assert_eq!(a.unwrap().info.run_id, b.unwrap().info.run_id);
    // A generic published-run start cannot address the retained version, even by its owner.
    assert!(matches!(
        w.runs
            .start(
                &w.admin,
                &sagat_application::runs::StartRun {
                    namespace: "payments",
                    workflow: original.info.workflow_id,
                    version: Some(original.info.version_id),
                    input: b"{}",
                    correlation_id: None,
                    deadline: None,
                    key: "ad-hoc-normal-start-denied"
                },
                &ctx()
            )
            .await,
        Err(AppError::NotFound)
    ));
    for doc in [
        document(
            "bad-ad-hoc",
            "task",
            vec![json!({"id":"task","type":"activity","activity":"ordinary"})],
        ),
        document(
            "bad-ad-hoc",
            "timer",
            vec![json!({"id":"timer","type":"timer","durationMs":100})],
        ),
    ] {
        let bytes = serde_json::to_vec(&doc).unwrap();
        assert!(matches!(
            w.runs
                .start_database(
                    &w.admin,
                    &StartDatabaseExecution {
                        key: "ad-hoc-unsupported",
                        document: &bytes,
                        ..request.clone()
                    },
                    &ctx()
                )
                .await,
            Err(AppError::NotAllowed(_))
        ));
    }
    // Database guards hold even for direct SQL through the owner connection.
    assert!(
        sqlx::query("UPDATE workflows SET internal=false WHERE id=$1")
            .bind(original.info.workflow_id.as_uuid())
            .execute(&w.db.pool)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("UPDATE runs SET origin='CONNECTION_PROBE' WHERE id=$1")
            .bind(original.info.run_id.as_uuid())
            .execute(&w.db.pool)
            .await
            .is_err()
    );
}

pub async fn http(api: &crate::support::api::TestApi, ns: &str, mut connection: serde_json::Value) {
    use reqwest::Method;
    use sagat_application::identity::IdentityStore;
    use sagat_domain::identity::{Membership, Role};
    let tenant = api
        .owner_store
        .find_tenant_by_slug(&Slug::parse("bao-http").unwrap())
        .await
        .unwrap()
        .unwrap()
        .id();
    api.owner_store
        .insert_membership(&Membership::restore(
            tenant,
            crate::support::subject("probe-operator"),
            Role::Operator,
            crate::support::now(),
        ))
        .await
        .unwrap();
    let path = format!("/namespaces/{ns}/connections/orders-db/probe");
    let denied = api
        .as_user(Method::POST, &path, "probe-operator", "bao-http")
        .header("Idempotency-Key", "probe-role-denied")
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(denied.status(), 403);
    wait_denials(&api.db.pool, tenant, &["probe-operator"], 1).await;
    let paused = api
        .as_user(Method::POST, &path, "owner", "bao-http")
        .header("Idempotency-Key", "probe-paused-001")
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(paused.status(), 404);
    assert_eq!(paused.headers()["cache-control"], "private, no-store");
    assert!(
        api.get_as(&path, "owner", "bao-http")
            .await
            .json::<serde_json::Value>()
            .await
            .unwrap()
            .is_null()
    );
    connection["status"] = json!("ACTIVE");
    connection["expectedRevision"] = json!(1);
    connection["requestId"] = json!(Uuid::new_v4().to_string());
    assert_eq!(
        api.as_user(
            Method::PUT,
            &format!("/namespaces/{ns}/connections/orders-db"),
            "owner",
            "bao-http"
        )
        .json(&connection)
        .send()
        .await
        .unwrap()
        .status(),
        200
    );
    let contract = crate::support::contract::OpenApi::load();
    let mut run = serde_json::Value::Null;
    for _ in 0..2 {
        let response = api
            .as_user(Method::POST, &path, "owner", "bao-http")
            .header("Idempotency-Key", "probe-active-001")
            .json(&json!({}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 201);
        let value: serde_json::Value = response.json().await.unwrap();
        assert_eq!(value["origin"], "CONNECTION_PROBE");
        if !run.is_null() {
            assert_eq!(run["runId"], value["runId"]);
        }
        contract
            .validate_body(
                "POST",
                "/namespaces/{namespaceId}/connections/{connectionName}/probe",
                201,
                &value,
            )
            .unwrap()
            .unwrap();
        run = value;
    }
    let metadata: serde_json::Value = api
        .get_as(&path, "owner", "bao-http")
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(metadata["revision"], 2);
    assert_eq!(metadata["run"]["runId"], run["runId"]);
    assert_ne!(
        metadata["run"]["status"], "SUCCEEDED",
        "admission is not successful connectivity"
    );
    contract
        .validate_body(
            "GET",
            "/namespaces/{namespaceId}/connections/{connectionName}/probe",
            200,
            &metadata,
        )
        .unwrap()
        .unwrap();
    assert_eq!(
        api.as_user(Method::POST, &path, "owner", "bao-http")
            .header("Idempotency-Key", "probe-forgery-001")
            .json(&json!({"statement":"SELECT secret"}))
            .send()
            .await
            .unwrap()
            .status(),
        422
    );
    let hidden = format!("/workflows/{}", run["workflowId"].as_str().unwrap());
    assert_eq!(api.get_as(&hidden, "owner", "bao-http").await.status(), 404);
    let listing: serde_json::Value = api
        .as_user(
            Method::GET,
            "/runs?origin=CONNECTION_PROBE",
            "owner",
            "bao-http",
        )
        .header("X-Octopus-Namespace", "database")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(listing["items"].as_array().unwrap().len(), 1);
    let execution = json!({"mode":"step", "definition": document("http-ad-hoc", "query", vec![json!({"id":"query", "type":"database", "connectionRef":{"name":"orders-db"}, "operation":"query", "statement":"SELECT 1::bigint"})])});
    let endpoint = format!("/namespaces/{ns}/database-executions");
    let response = api
        .as_user(Method::POST, &endpoint, "owner", "bao-http")
        .header("Idempotency-Key", "http-ad-hoc-001")
        .json(&execution)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 201);
    let value = response.json::<serde_json::Value>().await.unwrap();
    assert_eq!(value["origin"], "AD_HOC_STEP");
    let retained: Vec<u8> =
        sqlx::query_scalar("SELECT document FROM workflow_versions WHERE tenant_id=$1 AND id=$2")
            .bind(tenant.as_uuid())
            .bind(run["versionId"].as_str().unwrap().parse::<Uuid>().unwrap())
            .fetch_one(&api.db.pool)
            .await
            .unwrap();
    let same_definition = json!({"mode":"step", "definition":serde_json::from_slice::<serde_json::Value>(&retained).unwrap(), "timeoutMs":30000});
    let changed_origin = api
        .as_user(Method::POST, &endpoint, "owner", "bao-http")
        .header("Idempotency-Key", "probe-active-001")
        .json(&same_definition)
        .send()
        .await
        .unwrap();
    assert_eq!(changed_origin.status(), 409);
    let ad_hoc = api
        .as_user(Method::POST, &endpoint, "owner", "bao-http")
        .header("Idempotency-Key", "probe-as-ad-hoc-001")
        .json(&same_definition)
        .send()
        .await
        .unwrap();
    assert_eq!(ad_hoc.status(), 201);
    let ad_hoc: serde_json::Value = ad_hoc.json().await.unwrap();
    assert_eq!(
        ad_hoc["versionId"], run["versionId"],
        "definition deduplication excludes run origin"
    );
    assert_eq!(ad_hoc["origin"], "AD_HOC_STEP");
    let unchanged: serde_json::Value = api
        .get_as(&path, "owner", "bao-http")
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(
        unchanged["run"]["runId"], run["runId"],
        "ad hoc must never relabel a probe"
    );
    contract
        .validate_body(
            "POST",
            "/namespaces/{namespaceId}/database-executions",
            201,
            &value,
        )
        .unwrap()
        .unwrap();
    let mut forged = execution;
    forged["origin"] = json!("CONNECTION_PROBE");
    assert_eq!(
        api.as_user(Method::POST, &endpoint, "owner", "bao-http")
            .header("Idempotency-Key", "http-ad-hoc-forged")
            .json(&forged)
            .send()
            .await
            .unwrap()
            .status(),
        422
    );
    connection["expectedRevision"] = json!(2);
    connection["requestId"] = json!(Uuid::new_v4().to_string());
    assert_eq!(
        api.as_user(
            Method::PUT,
            &format!("/namespaces/{ns}/connections/orders-db"),
            "owner",
            "bao-http"
        )
        .json(&connection)
        .send()
        .await
        .unwrap()
        .status(),
        200
    );
    let metadata: serde_json::Value = api
        .get_as(&path, "owner", "bao-http")
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(
        metadata["revision"], 2,
        "editing does not move probe evidence to revision 3"
    );
}

async fn rollback_before_commit(w: &World, request: &StartDatabaseExecution<'_>) {
    async fn counts(w: &World) -> Vec<i64> {
        sqlx::query_scalar("SELECT ARRAY[(SELECT count(*) FROM workflows),(SELECT count(*) FROM workflow_versions),(SELECT count(*) FROM private_execution_definitions),(SELECT count(*) FROM runs),(SELECT count(*) FROM run_database_connections),(SELECT count(*) FROM steps),(SELECT count(*) FROM run_events),(SELECT count(*) FROM event_outbox),(SELECT count(*) FROM command_dedupe),(SELECT count(*) FROM audit_records)]")
            .fetch_one(&w.db.pool).await.unwrap()
    }
    let before = counts(w).await;
    sqlx::raw_sql("CREATE FUNCTION test_private_commit_failure() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'injected before commit' USING ERRCODE='check_violation'; END $$; CREATE CONSTRAINT TRIGGER test_private_commit_failure AFTER INSERT ON runs DEFERRABLE INITIALLY DEFERRED FOR EACH ROW WHEN (NEW.origin IS NOT NULL) EXECUTE FUNCTION test_private_commit_failure();")
        .execute(&w.db.pool).await.unwrap();
    let mut document: serde_json::Value = serde_json::from_slice(request.document).unwrap();
    document["nodes"][0]["statement"] = json!("SELECT 789::bigint");
    let bytes = serde_json::to_vec(&document).unwrap();
    assert!(
        w.runs
            .start_database(
                &w.admin,
                &StartDatabaseExecution {
                    document: &bytes,
                    key: "ad-hoc-crash-before-commit",
                    ..request.clone()
                },
                &ctx()
            )
            .await
            .is_err()
    );
    assert_eq!(
        counts(w).await,
        before,
        "retention, pins, initial events/tasks/outbox/audit and receipt roll back together"
    );
    sqlx::raw_sql("DROP TRIGGER test_private_commit_failure ON runs; DROP FUNCTION test_private_commit_failure();").execute(&w.db.pool).await.unwrap();
    let accepted = w
        .runs
        .start_database(
            &w.admin,
            &StartDatabaseExecution {
                document: &bytes,
                key: "ad-hoc-crash-before-commit",
                ..request.clone()
            },
            &ctx(),
        )
        .await
        .unwrap();
    w.store
        .verify_run(w.admin.tenant_id(), accepted.info.run_id)
        .await
        .unwrap();
}

async fn wait_denials(
    pool: &sqlx::PgPool,
    tenant: sagat_domain::identity::TenantId,
    actors: &[&str],
    minimum: i64,
) {
    // Denial audits use the existing bounded asynchronous writer, unlike committed run events.
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            let count: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_records WHERE tenant_id=$1 AND action='database.denied' AND actor=ANY($2)")
                .bind(tenant.as_uuid()).bind(actors).fetch_one(pool).await.unwrap();
            if count >= minimum { break; }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    }).await.expect("bounded database denial audit must arrive");
}
