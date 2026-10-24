//! Live capability checks, with the real provider deliberately stopped between read and recheck.
use super::{
    ReadBarrier,
    support::engine::{Session, ctx, document, task_ref, world},
};
use sagat_application::{
    clock::SystemClock,
    database::{ConnectionService, ConnectionWrite},
    error::AppError,
    managed_secrets::{ManagedSecretService, ManagedSecretWrite},
    secret_provider::SecretProvider,
    worker::{Capabilities, StepAck},
};
use sagat_domain::{
    database::{
        ConnectionConfig, ConnectionStatus, DatabaseLimits, MANAGED_DATABASE_ACTIVITY, TlsPolicy,
    },
    identity::{Role, Slug},
    quota::QuotaResource,
    secrets::{SecretName, SecretValue},
};
use serde_json::json;
use std::{sync::Arc, time::Duration};
use uuid::Uuid;

pub async fn run(provider: Arc<dyn SecretProvider>) {
    let mut w = world().await;
    w.workers = w.workers.with_secret_provider(provider.clone());
    let ns = w.namespace.id();
    let secrets = ManagedSecretService::new(
        w.store.clone(),
        SystemClock,
        provider.clone(),
        SecretValue::new(vec![42; 32]).unwrap(),
    )
    .unwrap();
    let rotate = |expected_version| ManagedSecretWrite {
        name: "executor-credentials",
        expected_version,
        request_id: Uuid::new_v4(),
        value: SecretValue::new(format!("executor-canary-{expected_version:08}").into_bytes())
            .unwrap(),
    };
    let initial = rotate(0);
    let initial_request = initial.request_id;
    let pool_names = vec!["database-production".to_owned()];
    let created = secrets
        .put_with_pools(&w.admin, ns, initial, Some(&pool_names), &ctx())
        .await
        .unwrap();
    assert_eq!(created.pools, Some(pool_names.clone()));
    let mut changed = rotate(0);
    changed.request_id = initial_request;
    assert!(
        matches!(
            secrets
                .put_with_pools(
                    &w.admin,
                    ns,
                    changed,
                    Some(&["another-pool".to_owned()]),
                    &ctx()
                )
                .await,
            Err(AppError::IdempotencyKeyReused)
        ),
        "a replay cannot replace the bound pool restriction"
    );
    let recovered = secrets
        .reconcile_request(&w.admin, ns, initial_request, &ctx())
        .await
        .unwrap();
    assert_eq!(recovered.pools, Some(pool_names));
    let catalog = ConnectionService::new(w.store.clone(), SystemClock);
    let mut write = ConnectionWrite {
        name: Slug::parse("executor-db").unwrap(),
        request_id: Uuid::new_v4(),
        expected_revision: 0,
        status: ConnectionStatus::Active,
        config: ConnectionConfig::postgres(
            "db.example.com",
            5432,
            "app",
            TlsPolicy::VerifyFull { ca_pem: None },
            SecretName::parse("executor-credentials").unwrap(),
            DatabaseLimits {
                pool_size: 1,
                ..DatabaseLimits::default()
            },
        )
        .unwrap(),
    };
    catalog
        .put(&w.admin, ns, write.clone(), &ctx())
        .await
        .unwrap();
    let publisher = w.member("database-author", Role::Developer).await;
    w.secrets
        .grant(
            &w.admin,
            ns,
            "executor-credentials",
            publisher.subject().as_str(),
            &ctx(),
        )
        .await
        .unwrap();
    let (workflow, _) = w
        .try_publish_as(
            &publisher,
            document(
                "executor-flow",
                "database",
                vec![json!({
                    "id":"database","type":"database","connectionRef":{"name":"executor-db"},
                    "operation":"query","statement":"SELECT 1"
                })],
            ),
        )
        .await
        .unwrap();
    for key in ["executor-run-1", "executor-run-2"] {
        w.start_with(workflow, &json!({}), key, None).await.unwrap();
    }
    let runner = w.member("database-machine", Role::DatabaseRunner).await;
    let other = w
        .member("other-database-machine", Role::DatabaseRunner)
        .await;
    let caps = Capabilities {
        activities: vec![MANAGED_DATABASE_ACTIVITY.into()],
        languages: vec![],
        pools: vec![],
        max_inflight: 32,
        protocol_version: "1.6".into(),
    };
    for principal in [&w.admin, &w.worker, &w.runner_identity] {
        w.bind_everything(principal).await;
        assert!(matches!(
            w.workers
                .register(principal, "payments", "forged-database", &caps)
                .await,
            Err(AppError::NotBound)
        ));
    }
    let ordinary = Capabilities {
        activities: vec!["ordinary".into()],
        protocol_version: "1.6".into(),
        ..caps.clone()
    };
    let ordinary_registration = w
        .workers
        .register(
            &w.worker,
            "payments",
            "ordinary-protocol-isolation",
            &ordinary,
        )
        .await
        .unwrap();
    assert_eq!(
        ordinary_registration.accepted_protocol_version, "1.4",
        "database capability support does not enable scoped execution on a default store"
    );
    w.bind_everything(&runner).await;
    w.bind_everything(&other).await;
    let old = Capabilities {
        protocol_version: "1.4".into(),
        ..caps.clone()
    };
    assert!(matches!(
        w.workers
            .register(&runner, "payments", "old-database", &old)
            .await,
        Err(AppError::NotBound)
    ));
    let mixed = Capabilities {
        activities: vec![MANAGED_DATABASE_ACTIVITY.into(), "ordinary".into()],
        ..caps.clone()
    };
    assert!(matches!(
        w.workers
            .register(&runner, "payments", "mixed-database", &mixed)
            .await,
        Err(AppError::NotBound)
    ));
    sqlx::query("UPDATE memberships SET role='worker' WHERE tenant_id=$1 AND subject=$2")
        .bind(runner.tenant_id().as_uuid())
        .bind(runner.subject().as_str())
        .execute(&w.db.pool)
        .await
        .unwrap();
    assert!(matches!(
        w.workers
            .register(&runner, "payments", "stale-database", &caps)
            .await,
        Err(AppError::NotBound)
    ));
    sqlx::query("UPDATE memberships SET role='database-runner' WHERE tenant_id=$1 AND subject=$2")
        .bind(runner.tenant_id().as_uuid())
        .bind(runner.subject().as_str())
        .execute(&w.db.pool)
        .await
        .unwrap();
    let registered = w
        .workers
        .register(&runner, "payments", "database-machine", &caps)
        .await
        .unwrap();
    assert_eq!(registered.accepted_protocol_version, "1.6");
    let session = Session {
        principal: runner.clone(),
        worker_id: registered.worker_id,
        token: registered.session_token.expose().to_owned(),
    };
    let other_registered = w
        .workers
        .register(&other, "payments", "other-database-machine", &caps)
        .await
        .unwrap();
    let other_session = Session {
        principal: other.clone(),
        worker_id: other_registered.worker_id,
        token: other_registered.session_token.expose().to_owned(),
    };
    // Imported/local pool restrictions must apply to managed credentials too, even when the
    // managed node has no ordinary `secrets` declaration.
    let credential_pools = vec!["database-production".to_owned()];
    w.secrets
        .set_pools(
            &w.admin,
            ns,
            "executor-credentials",
            &credential_pools,
            &ctx(),
        )
        .await
        .unwrap();
    assert!(
        w.poll(&session, 32).await.is_empty(),
        "restricted credential cannot be leased without a pool grant"
    );
    w.bindings
        .bind_with_pools(
            &w.admin,
            ns,
            runner.subject().as_str(),
            &["*".to_owned()],
            Some(&credential_pools),
            &ctx(),
        )
        .await
        .unwrap();
    let tasks = w.poll(&session, 32).await;
    assert_eq!(
        tasks.len(),
        1,
        "pool concurrency is reserved in the lease transaction"
    );
    assert!(w.poll(&session, 32).await.is_empty());
    assert!(
        w.poll(&other_session, 32).await.is_empty(),
        "the connection pool bound spans worker sessions and identities"
    );
    let task = &tasks[0];
    let session = Box::pin(session_expiry_after_wait(&w, session, task, &caps)).await;
    let reference = task_ref(task);
    assert!(
        w.workers
            .resolve_database(&other, &reference, None)
            .await
            .is_err(),
        "a different bound machine cannot borrow the token"
    );
    assert!(
        w.workers
            .resolve_database(&w.admin, &reference, None)
            .await
            .is_err(),
        "human admin cannot borrow the token"
    );
    let mut wrong = task_ref(task);
    wrong.fence.attempt += 1;
    assert!(matches!(
        w.workers.resolve_database(&runner, &wrong, None).await,
        Err(AppError::StaleAttempt)
    ));
    sqlx::raw_sql("CREATE FUNCTION test_reject_database_audit() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.action='secret.resolved' THEN RAISE EXCEPTION 'audit unavailable'; END IF; RETURN NEW; END $$; CREATE TRIGGER test_reject_database_audit BEFORE INSERT ON audit_records FOR EACH ROW EXECUTE FUNCTION test_reject_database_audit();").execute(&w.db.pool).await.unwrap();
    assert!(
        w.workers
            .resolve_database(&runner, &reference, None)
            .await
            .is_err(),
        "a credential cannot leave before its audit commits"
    );
    let uncommitted: i64 =
        sqlx::query_scalar("SELECT count(*) FROM step_database_credentials WHERE tenant_id=$1")
            .bind(runner.tenant_id().as_uuid())
            .fetch_one(&w.db.pool)
            .await
            .unwrap();
    assert_eq!(
        uncommitted, 0,
        "audit failure rolls the credential pin back"
    );
    sqlx::raw_sql("DROP TRIGGER test_reject_database_audit ON audit_records; DROP FUNCTION test_reject_database_audit();").execute(&w.db.pool).await.unwrap();
    let first = w
        .workers
        .resolve_database(&runner, &reference, None)
        .await
        .unwrap();
    assert_eq!(first.credential.value.expose(), b"executor-canary-00000000");
    assert_eq!(first.connection.revision, 1);
    assert!(!format!("{first:?}").contains("executor-canary"));
    assert!(
        w.complete(
            &session,
            task,
            &json!({"password":"executor-canary-00000000"})
        )
        .await
        .is_err(),
        "managed result storage stays closed until pin-based scanning ships"
    );
    assert!(
        w.fail_with(&session, task, "executor-canary-00000000", false)
            .await
            .is_err(),
        "failure codes cannot bypass the pending result gate"
    );
    assert!(
        w.workers
            .check_blob_upload(&runner, &reference)
            .await
            .is_err(),
        "blob uploads cannot bypass the gate"
    );
    assert!(
        w.workers.check_lease(&runner, &reference).await.is_err(),
        "managed leases cannot fetch unrelated run blobs"
    );
    for version in 1..=6 {
        secrets
            .put(&w.admin, ns, rotate(version), &ctx())
            .await
            .unwrap();
    }
    let retained: i64 =
        sqlx::query_scalar("SELECT count(*) FROM secret_versions WHERE tenant_id=$1")
            .bind(w.admin.tenant_id().as_uuid())
            .fetch_one(&w.db.pool)
            .await
            .unwrap();
    assert_eq!(retained, 5);
    let again = w
        .workers
        .resolve_database(&runner, &reference, None)
        .await
        .unwrap();
    assert_eq!(again.credential.version, 1);
    assert_eq!(
        again.credential.value.expose(),
        first.credential.value.expose(),
        "the original attempt pin survives the logical retention window"
    );
    let pins: i64 =
        sqlx::query_scalar("SELECT count(*) FROM step_database_credentials WHERE tenant_id=$1")
            .bind(w.admin.tenant_id().as_uuid())
            .fetch_one(&w.db.pool)
            .await
            .unwrap();
    assert_eq!(pins, 1);
    let pinned: Uuid = sqlx::query_scalar(
        "SELECT provider_operation FROM step_database_credentials WHERE tenant_id=$1",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .unwrap();
    assert!(
        sqlx::query("DELETE FROM secret_provider_operations WHERE id=$1")
            .bind(pinned)
            .execute(&w.db.pool)
            .await
            .is_err()
    );

    assert!(sqlx::query("UPDATE step_database_credentials SET secret_version=secret_version+1 WHERE tenant_id=$1")
        .bind(runner.tenant_id().as_uuid()).execute(&w.db.pool).await.is_err(),"credential evidence is immutable");

    // Rotation commits while the provider is paused: no catalog/run lock crosses external I/O.
    let barrier = Arc::new(ReadBarrier {
        inner: provider.clone(),
        entered: tokio::sync::Barrier::new(2),
        resume: tokio::sync::Barrier::new(2),
    });
    w.workers = w.workers.with_secret_provider(barrier.clone());
    // Rebinding while provider I/O is paused must finish, and the final read must withhold
    // a value after its pool grant is removed. The timeout also guards the lock order.
    let (result, ()) = tokio::time::timeout(Duration::from_secs(10), async {
        tokio::join!(
            w.workers.resolve_database(&runner, &reference, None),
            async {
                barrier.entered.wait().await;
                tokio::time::timeout(
                    Duration::from_secs(2),
                    w.bindings.bind_with_pools(
                        &w.admin,
                        ns,
                        runner.subject().as_str(),
                        &["*".to_owned()],
                        Some(&[]),
                        &ctx(),
                    ),
                )
                .await
                .unwrap()
                .unwrap();
                barrier.resume.wait().await;
            }
        )
    })
    .await
    .unwrap();
    assert!(
        result.is_err(),
        "pool revocation discards a credential already read"
    );
    w.bindings
        .bind_with_pools(
            &w.admin,
            ns,
            runner.subject().as_str(),
            &["*".to_owned()],
            Some(&credential_pools),
            &ctx(),
        )
        .await
        .unwrap();
    let (result, ()) = tokio::time::timeout(Duration::from_secs(10), async {
        tokio::join!(
            w.workers.resolve_database(&runner, &reference, None),
            async {
                barrier.entered.wait().await;
                tokio::time::timeout(
                    Duration::from_secs(2),
                    secrets.put(&w.admin, ns, rotate(7), &ctx()),
                )
                .await
                .unwrap()
                .unwrap();
                barrier.resume.wait().await;
            }
        )
    })
    .await
    .unwrap();
    assert!(matches!(result, Err(AppError::Unavailable)));
    let (result, ()) = tokio::time::timeout(Duration::from_secs(10), async {
        tokio::join!(
            w.workers.resolve_database(&runner, &reference, None),
            async {
                barrier.entered.wait().await;
                tokio::time::timeout(
                    Duration::from_secs(2),
                    w.secrets.revoke(
                        &w.admin,
                        ns,
                        "executor-credentials",
                        publisher.subject().as_str(),
                        &ctx(),
                    ),
                )
                .await
                .unwrap()
                .unwrap();
                barrier.resume.wait().await;
            }
        )
    })
    .await
    .unwrap();
    assert!(
        result.is_err(),
        "revoking the publisher ACL discards a credential already read"
    );
    w.secrets
        .grant(
            &w.admin,
            ns,
            "executor-credentials",
            publisher.subject().as_str(),
            &ctx(),
        )
        .await
        .unwrap();
    // Connection disable during I/O also commits immediately and invalidates the response.
    write.expected_revision = 1;
    write.request_id = Uuid::new_v4();
    write.status = ConnectionStatus::Paused;
    let (result, ()) = tokio::time::timeout(Duration::from_secs(10), async {
        tokio::join!(
            w.workers.resolve_database(&runner, &reference, None),
            async {
                barrier.entered.wait().await;
                tokio::time::timeout(
                    Duration::from_secs(2),
                    catalog.put(&w.admin, ns, write.clone(), &ctx()),
                )
                .await
                .unwrap()
                .unwrap();
                barrier.resume.wait().await;
            }
        )
    })
    .await
    .unwrap();
    assert!(result.is_err());
    write.expected_revision = 2;
    write.request_id = Uuid::new_v4();
    write.status = ConnectionStatus::Active;
    catalog.put(&w.admin, ns, write, &ctx()).await.unwrap();
    // The machine's role is checked again, even though its authenticated Principal is stale.
    let (result, ()) = tokio::time::timeout(Duration::from_secs(10), async {
        tokio::join!(
            w.workers.resolve_database(&runner, &reference, None),
            async {
                barrier.entered.wait().await;
                tokio::time::timeout(
                    Duration::from_secs(2),
                    sqlx::query(
                        "UPDATE memberships SET role='worker' WHERE tenant_id=$1 AND subject=$2",
                    )
                    .bind(runner.tenant_id().as_uuid())
                    .bind(runner.subject().as_str())
                    .execute(&w.db.pool),
                )
                .await
                .unwrap()
                .unwrap();
                barrier.resume.wait().await;
            }
        )
    })
    .await
    .unwrap();
    assert!(matches!(result, Err(AppError::NotBound)));
    w.workers = w.workers.with_secret_provider(provider.clone());
    sqlx::query("UPDATE memberships SET role='database-runner' WHERE tenant_id=$1 AND subject=$2")
        .bind(runner.tenant_id().as_uuid())
        .bind(runner.subject().as_str())
        .execute(&w.db.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE steps SET lease_expires_at=clock_timestamp()-interval '1 second' WHERE tenant_id=$1 AND run_id=$2 AND node='database'")
        .bind(runner.tenant_id().as_uuid()).bind(task.run_id.as_uuid()).execute(&w.db.pool).await.unwrap();
    assert!(matches!(
        w.workers.resolve_database(&runner, &reference, None).await,
        Err(AppError::StaleAttempt)
    ));
    sqlx::query("UPDATE steps SET lease_expires_at=clock_timestamp()+interval '1 minute' WHERE tenant_id=$1 AND run_id=$2 AND node='database'")
        .bind(runner.tenant_id().as_uuid()).bind(task.run_id.as_uuid()).execute(&w.db.pool).await.unwrap();
    w.workers = w.workers.with_secret_provider(barrier.clone());
    let (result, ()) = tokio::time::timeout(Duration::from_secs(10), async {
        tokio::join!(
            w.workers.resolve_database(&runner, &reference, None),
            async {
                barrier.entered.wait().await;
                tokio::time::timeout(
                    Duration::from_secs(2),
                    w.operate(task.run_id, sagat_application::runs::RunOperation::Cancel),
                )
                .await
                .unwrap()
                .unwrap();
                barrier.resume.wait().await;
            }
        )
    })
    .await
    .unwrap();
    assert!(
        matches!(result, Err(AppError::StaleAttempt)),
        "a cancelling run cannot receive a new capability"
    );
    // Finishing the cancelled attempt releases capacity for the other admitted run.
    w.workers = w.workers.with_secret_provider(provider);
    let applied = w
        .fail_with(&session, task, "WORKER_ERROR", false)
        .await
        .expect("persist managed failure receipt");
    assert!(!applied.already_applied);
    let deleted = sqlx::query("DELETE FROM worker_sessions WHERE tenant_id=$1 AND id=$2")
        .bind(session.principal.tenant_id().as_uuid())
        .bind(session.worker_id)
        .execute(&w.db.pool)
        .await
        .expect("clear completed managed session")
        .rows_affected();
    assert_eq!(deleted, 1);
    managed_receipt_replay_waits_for_binding_before_secrets(&w, &session, task, &applied).await;
    let registered = w
        .workers
        .register(&session.principal, "payments", "database-machine", &caps)
        .await
        .expect("public registration after session-cleared receipt replay");
    assert_eq!(registered.accepted_protocol_version, "1.6");
    let session = Session {
        principal: session.principal.clone(),
        worker_id: registered.worker_id,
        token: registered.session_token.expose().to_owned(),
    };
    let next = w.poll(&session, 1).await;
    assert_eq!(next.len(), 1);
    let next_ref = task_ref(&next[0]);
    let credential = w
        .workers
        .resolve_database(&runner, &next_ref, None)
        .await
        .unwrap();
    assert_eq!(credential.credential.version, 8);
    w.secrets
        .delete(&w.admin, ns, "executor-credentials", &ctx())
        .await
        .unwrap();
    assert!(
        w.workers
            .resolve_database(&runner, &next_ref, None)
            .await
            .is_err()
    );
    secrets.put(&w.admin, ns, rotate(0), &ctx()).await.unwrap();
    w.secrets
        .grant(
            &w.admin,
            ns,
            "executor-credentials",
            publisher.subject().as_str(),
            &ctx(),
        )
        .await
        .unwrap();
    assert!(
        matches!(
            w.workers.resolve_database(&runner, &next_ref, None).await,
            Err(AppError::NotFound)
        ),
        "recreating a logical name must not reauthorize a pin from the deleted secret identity"
    );
    let audits: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_records WHERE tenant_id=$1 AND action='secret.resolved'",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .unwrap();
    assert_eq!(
        audits, 2,
        "identical successful disclosures retain one audit per attempt/version, as the existing bounded audit policy requires"
    );
}

pub async fn grpc(provider: Arc<dyn SecretProvider>) {
    use crate::support::{
        api::Options,
        worker::{Worker, platform_with},
    };
    use reqwest::Method;
    use sagat_api_grpc::pb::database_value::Value as Wire;
    let p = platform_with(Options {
        secret_provider: Some(provider),
        ..Default::default()
    })
    .await;
    let ns = p.namespace_id().await;
    let (status, _) = p.rest(Method::PUT,&format!("/namespaces/{ns}/secrets/pg-credential"),Some(&json!({
        "value":"GRPC_CREDENTIAL_CANARY", "requestId":Uuid::new_v4().to_string(), "expectedVersion":0,"requiredProvider":"openbao"
    }))).await;
    assert_eq!(status, 200);
    let (status, body) = p
        .rest(
            Method::PUT,
            &format!("/namespaces/{ns}/connections/warehouse"),
            Some(&json!({
                "requestId":Uuid::new_v4().to_string(),"expectedRevision":0,"status":"ACTIVE","config":{
                    "driver":"postgresql","host":"db.example.com","port":5432,"database":"test",
                    "tls":{"mode":"verify-full"},"secretName":"pg-credential"
                }
            })),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let mut workflow: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../contract/database-connections/typed-parameters.workflow.json"
    ))
    .unwrap();
    workflow["metadata"]["name"] = json!("typed-database-wire");
    workflow["nodes"][0]["connectionRef"] = json!({"name":"warehouse"});
    let workflow_id = p.publish("typed-database-wire", &workflow).await;
    p.start(&workflow_id, &json!({"orderId":"9223372036854775807"}))
        .await;
    p.grant(p.tenant, "database-grpc", Role::DatabaseRunner)
        .await;
    p.bind_everything("database-grpc").await;
    let mut worker = p.connect("database-grpc", "acme").await;
    worker
        .register_version(&[MANAGED_DATABASE_ACTIVITY], "1.6")
        .await;
    let lease = worker.one_task().await;
    assert_eq!(
        lease.input_json, "{}",
        "the capability owns parameter evaluation; run context stays server-side"
    );
    let request = worker.req(Worker::resolve_secrets_message(&lease));
    let response = worker
        .client
        .resolve_database(request)
        .await
        .unwrap()
        .into_inner();
    assert_eq!(response.connection_revision, 1);
    assert_eq!(response.namespace_id, ns);
    assert_eq!(response.tenant_id, p.tenant.to_string());
    assert_eq!(response.connection_name, "warehouse");
    assert_eq!(response.parameters.len(), 11);
    assert_eq!(
        response.parameters[2].value,
        Some(Wire::IntegerValue("9223372036854775807".into()))
    );
    assert_eq!(
        response.parameters[3].value,
        Some(Wire::DecimalValue("12345678901234567890.123456789".into()))
    );
    assert_eq!(
        response.parameters[8].value,
        Some(Wire::BinaryValue(vec![0, 255]))
    );
    assert_eq!(
        response.parameters[10].value,
        Some(Wire::IntegerValue("9223372036854775807".into()))
    );
    assert_eq!(
        response.credential.as_ref().unwrap().value,
        b"GRPC_CREDENTIAL_CANARY"
    );
    assert!(!format!("{response:?}").contains("GRPC_CREDENTIAL_CANARY"));
    assert!(!format!("{response:?}").contains("SELECT"));
    // Cross-tenant and ordinary machine identities receive no capability even with the lease.
    let mut ordinary = p.worker("ordinary-grpc").await;
    let request = ordinary.req(Worker::resolve_secrets_message(&lease));
    assert_eq!(
        ordinary
            .client
            .resolve_database(request)
            .await
            .unwrap_err()
            .code(),
        tonic::Code::PermissionDenied
    );
    p.grant(
        p.other_tenant,
        "foreign-database-grpc",
        Role::DatabaseRunner,
    )
    .await;
    let mut foreign = p.connect("foreign-database-grpc", "globex").await;
    let request = foreign.req(Worker::resolve_secrets_message(&lease));
    assert_eq!(
        foreign
            .client
            .resolve_database(request)
            .await
            .unwrap_err()
            .code(),
        tonic::Code::Aborted
    );
}

async fn session_expiry_after_wait(
    w: &super::support::engine::World,
    session: Session,
    task: &sagat_application::worker::ClaimedTask,
    caps: &Capabilities,
) -> Session {
    let tenant = w.admin.tenant_id().as_uuid();
    let expires_at: time::OffsetDateTime = sqlx::query_scalar(
        "UPDATE worker_sessions SET expires_at=clock_timestamp()+interval '3 seconds' \
         WHERE tenant_id=$1 AND id=$2 RETURNING expires_at",
    )
    .bind(tenant)
    .bind(session.worker_id)
    .fetch_one(&w.db.pool)
    .await
    .expect("short managed session expiry");
    let effects = || async {
        sqlx::query_as::<_,(i64,i64,i64,i64,i64,time::OffsetDateTime,time::OffsetDateTime)>(
            "SELECT r.run_version, \
             (SELECT count(*) FROM run_events WHERE tenant_id=$1 AND run_id=$2), \
             (SELECT count(*) FROM event_outbox WHERE tenant_id=$1 AND aggregate_id=$2), \
             (SELECT count(*) FROM audit_records WHERE tenant_id=$1), \
             (SELECT count(*) FROM step_database_credentials WHERE tenant_id=$1 AND run_id=$2), \
             w.expires_at,s.lease_expires_at FROM runs r JOIN steps s ON s.tenant_id=r.tenant_id AND s.run_id=r.id \
             JOIN worker_sessions w ON w.tenant_id=r.tenant_id AND w.id=$3 \
             WHERE r.tenant_id=$1 AND r.id=$2 AND s.node=$4",
        ).bind(tenant).bind(task.run_id.as_uuid()).bind(session.worker_id).bind(task.instance.node().as_str())
        .fetch_one(&w.db.pool).await.expect("atomic managed heartbeat effects")
    };
    let before = effects().await;
    let mut guard = w.db.pool.begin().await.expect("managed session guard");
    let backend: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *guard)
        .await
        .expect("session guard backend");
    sqlx::query("SELECT id FROM worker_sessions WHERE tenant_id=$1 AND id=$2 FOR UPDATE")
        .bind(tenant)
        .bind(session.worker_id)
        .execute(&mut *guard)
        .await
        .expect("hold unchanged managed session");
    let reference = task_ref(task);
    let mut heartbeat = Box::pin(w.workers.heartbeat(&session.principal, &reference));
    tokio::select! {
        _ = &mut heartbeat => panic!("managed heartbeat must wait on its session"),
        () = super::support::eventually("actual managed owner session SHARE wait", || async {
            sqlx::query_scalar::<_,bool>(
                "SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE datname=current_database() \
                 AND $1=ANY(pg_blocking_pids(pid)) AND query LIKE '%JOIN worker_sessions w%' \
                 AND (NOT $2 OR usename='sagat_app'))",
            ).bind(backend).bind(super::support::as_runtime_role()).fetch_one(&w.db.pool).await
            .expect("actual app managed session guard").then_some(())
        }) => {}
    }
    assert!(
        sqlx::query_scalar::<_, bool>("SELECT clock_timestamp()<$1")
            .bind(expires_at)
            .fetch_one(&w.db.pool)
            .await
            .expect("session live at actual wait")
    );
    super::support::eventually("managed session expires under unchanged guard", || async {
        sqlx::query_scalar::<_, bool>("SELECT clock_timestamp()>=$1")
            .bind(expires_at)
            .fetch_one(&w.db.pool)
            .await
            .expect("database expiry")
            .then_some(())
    })
    .await;
    guard
        .rollback()
        .await
        .expect("release unchanged expired managed session");
    let result = heartbeat.await;
    let after = effects().await;
    assert!(
        matches!(result, Err(AppError::NotBound)),
        "expired managed session must be refused; accepted={}, lease_extended={}",
        result.is_ok(),
        after.6 > before.6
    );
    assert_eq!(
        after, before,
        "expired managed authority cannot renew or create journal/outbox/audit/pin effects"
    );
    w.store
        .verify_run(w.admin.tenant_id(), task.run_id)
        .await
        .expect("managed journal remains consistent");

    // A supported reconnect keeps the session id and existing lease accounting, replacing its
    // token; no owner SQL revives expired authority for this positive control.
    let registered = w
        .workers
        .register(&session.principal, "payments", "database-machine", caps)
        .await
        .expect("fresh managed reconnect");
    assert_eq!(registered.worker_id, session.worker_id);
    let fresh = Session {
        principal: session.principal.clone(),
        worker_id: registered.worker_id,
        token: registered.session_token.expose().to_owned(),
    };
    assert!(
        w.workers
            .heartbeat(&fresh.principal, &reference)
            .await
            .expect("fresh current managed owner can heartbeat")
            .lease_expires_at
            .is_some()
    );
    w.store
        .verify_run(w.admin.tenant_id(), task.run_id)
        .await
        .expect("fresh heartbeat journal matches projection");
    fresh
}

async fn managed_receipt_replay_waits_for_binding_before_secrets(
    w: &super::support::engine::World,
    session: &Session,
    task: &sagat_application::worker::ClaimedTask,
    applied: &StepAck,
) {
    let tenant = session.principal.tenant_id();
    let namespace = w.namespace.id();
    let binding_key = format!(
        "binding:{tenant}:{namespace}:{}",
        session.principal.subject()
    );
    let secrets_key = format!("quota:{}:{tenant}:{namespace}", QuotaResource::Secrets);
    let checkpoint = || async {
        sqlx::query_as::<_, (i64, i64, i64, i64, i64)>(
            "SELECT r.run_version, \
             (SELECT count(*) FROM run_events WHERE tenant_id=$1 AND run_id=$2), \
             (SELECT count(*) FROM event_outbox WHERE tenant_id=$1 AND aggregate_id=$2), \
             (SELECT count(*) FROM audit_records WHERE tenant_id=$1), \
             (SELECT count(*) FROM step_database_credentials WHERE tenant_id=$1 AND run_id=$2) \
             FROM runs r WHERE tenant_id=$1 AND id=$2",
        )
        .bind(tenant.as_uuid())
        .bind(task.run_id.as_uuid())
        .fetch_one(&w.db.pool)
        .await
        .expect("atomic managed replay effects")
    };
    let before = checkpoint().await;
    // Exact native prefix barrier around real public replay. It is not a reproduction of the
    // candidate three-party registration/claim/replay deadlock.
    let mut binding_guard = w.db.pool.begin().await.expect("managed binding guard");
    let binding_backend: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *binding_guard)
        .await
        .expect("binding guard backend");
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
        .bind(&binding_key)
        .execute(&mut *binding_guard)
        .await
        .expect("hold exact binding");
    let mut replay = Box::pin(w.fail_with(session, task, "WORKER_ERROR", false));
    tokio::select! {
        _ = &mut replay => panic!("managed replay bypassed held binding authority"),
        () = super::support::eventually("actual managed replay waits on binding", || async {
            sqlx::query_scalar::<_,bool>(
                "SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE datname=current_database() \
                 AND $1=ANY(pg_blocking_pids(pid)) AND query LIKE '%pg_advisory_xact_lock_shared%' \
                 AND (NOT $2 OR usename='sagat_app'))",
            ).bind(binding_backend).bind(super::support::as_runtime_role()).fetch_one(&w.db.pool).await
            .expect("actual app replay binding wait").then_some(())
        }) => {}
    }
    let mut probe = w.db.pool.begin().await.expect("namespace secrets probe");
    let available: bool =
        sqlx::query_scalar("SELECT pg_try_advisory_xact_lock(hashtextextended($1,0))")
            .bind(&secrets_key)
            .fetch_one(&mut *probe)
            .await
            .expect("probe namespace Secrets X");
    probe.rollback().await.expect("release secrets probe");
    assert!(
        available,
        "managed receipt replay retained Secrets X while waiting for binding S"
    );
    binding_guard
        .rollback()
        .await
        .expect("release binding authority");
    let replayed = tokio::time::timeout(Duration::from_secs(5), replay)
        .await
        .expect("bounded receipt replay")
        .expect("current authority permits durable receipt");
    assert!(replayed.already_applied);
    assert_eq!(replayed.command_id, applied.command_id);
    assert_eq!(replayed.step_status, applied.step_status);
    assert_eq!(replayed.run_status, applied.run_status);
    assert_eq!(
        checkpoint().await,
        before,
        "accepted replay cannot change execution/audit/pins"
    );
    w.store
        .verify_run(tenant, task.run_id)
        .await
        .expect("managed replay journal matches projection");
}
