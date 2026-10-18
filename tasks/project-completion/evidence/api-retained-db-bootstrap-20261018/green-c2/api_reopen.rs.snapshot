//! A current API instance can reopen its owned database and honor durable public worker authority.
use std::time::Duration;

use sagat_application::worker::WorkerSettings;
use serde_json::json;

use crate::support::api::{Options, start_on_db};
use crate::support::worker::{HUMAN, Platform, TENANT, Worker, single_ir};
use crate::support::{RUNTIME_ROLE, attach_existing_db, hold_server};

const WORKER: &str = "retained-db-public-worker";

#[derive(Debug, PartialEq, Eq, sqlx::FromRow)]
struct MembershipIdentity {
    tenant_id: String,
    subject: String,
    role: String,
}

#[derive(Debug, PartialEq, Eq, sqlx::FromRow)]
struct BindingIdentity {
    tenant_id: String,
    namespace_id: String,
    subject: String,
    pattern: String,
    updated_by: String,
}

#[derive(Debug, PartialEq, Eq, sqlx::FromRow)]
struct SessionIdentity {
    tenant_id: String,
    id: String,
    namespace_id: String,
    subject: String,
    worker_name: String,
    activities: Vec<String>,
    languages: Vec<String>,
    pools: Vec<String>,
    max_inflight: i32,
    accepted_protocol_minor: i32,
}

#[derive(Debug, PartialEq, Eq)]
struct IdentityWitness {
    tenant_ids: Vec<String>,
    memberships: Vec<MembershipIdentity>,
    bindings: Vec<BindingIdentity>,
    session_count: i64,
    sessions: Vec<SessionIdentity>,
}

fn runtime_options() -> Options {
    Options {
        runtime_role: true,
        worker: WorkerSettings {
            lease_ttl: Duration::from_secs(300),
            session_ttl: Duration::from_secs(300),
            ..WorkerSettings::default()
        },
        ..Options::default()
    }
}

async fn identity_witness(platform: &Platform) -> IdentityWitness {
    let tenant = platform.tenant.as_uuid();
    let tenant_ids =
        sqlx::query_scalar("SELECT id::text FROM tenants WHERE id=$1 AND slug=$2 ORDER BY id")
            .bind(tenant)
            .bind(TENANT)
            .fetch_all(&platform.api.db.pool)
            .await
            .expect("read retained tenant identity");
    let memberships = sqlx::query_as(
        r#"
        SELECT tenant_id::text AS tenant_id, subject, role
        FROM memberships
        WHERE tenant_id=$1 AND (subject=$2 OR subject=$3)
        ORDER BY subject
        "#,
    )
    .bind(tenant)
    .bind(HUMAN)
    .bind(WORKER)
    .fetch_all(&platform.api.db.pool)
    .await
    .expect("read retained membership identities");
    let bindings = sqlx::query_as(
        r#"
        SELECT tenant_id::text AS tenant_id, namespace_id::text AS namespace_id,
               subject, pattern, updated_by
        FROM worker_bindings
        WHERE tenant_id=$1 AND subject=$2
        ORDER BY namespace_id, pattern, updated_by
        "#,
    )
    .bind(tenant)
    .bind(WORKER)
    .fetch_all(&platform.api.db.pool)
    .await
    .expect("read retained binding identities");
    let sessions = sqlx::query_as(
        r#"
        SELECT tenant_id::text AS tenant_id, id::text AS id,
               namespace_id::text AS namespace_id, subject, worker_name,
               activities, languages, pools, max_inflight, accepted_protocol_minor
        FROM worker_sessions
        WHERE tenant_id=$1 AND subject=$2
        ORDER BY id
        "#,
    )
    .bind(tenant)
    .bind(WORKER)
    .fetch_all(&platform.api.db.pool)
    .await
    .expect("read retained session identities");
    let session_count = sqlx::query_scalar(
        "SELECT count(*) FROM worker_sessions WHERE tenant_id=$1 AND subject=$2",
    )
    .bind(tenant)
    .bind(WORKER)
    .fetch_one(&platform.api.db.pool)
    .await
    .expect("count retained sessions");
    IdentityWitness {
        tenant_ids,
        memberships,
        bindings,
        session_count,
        sessions,
    }
}

async fn assert_pool_roles(platform: &Platform) {
    let api_role: String = sqlx::query_scalar("SELECT current_user::text")
        .fetch_one(&platform.api.api_pool)
        .await
        .expect("API database role");
    let owner_role: String = sqlx::query_scalar("SELECT current_user::text")
        .fetch_one(&platform.api.db.pool)
        .await
        .expect("owner inspection role");
    assert_eq!(api_role, RUNTIME_ROLE);
    assert_ne!(owner_role, RUNTIME_ROLE);
}

#[tokio::test(flavor = "current_thread")]
async fn current_api_reopens_owned_database_and_resumes_original_public_worker() {
    let _server = hold_server();
    let original = crate::support::worker::platform_with(runtime_options()).await;
    assert_pool_roles(&original).await;

    let namespace_id = original.namespace_id().await;
    let workflow = original
        .publish(
            "retained-db-current-reopen",
            &single_ir("retained-db-current-reopen", None),
        )
        .await;
    let run = original
        .start(&workflow, &json!({"request":"before-reopen"}))
        .await;
    let scheduled = original.tick().await;
    assert_eq!(scheduled.failed, 0, "schedule original task: {scheduled:?}");

    let (worker_id, session, completion, identity_before) = {
        let mut worker = original.worker(WORKER).await;
        worker.register_version(&["act.a"], "1.5").await;
        let lease = worker.one_task().await;
        let completion = Worker::complete_message(&lease, r#"{"completed":"after-reopen"}"#);
        let identity = identity_witness(&original).await;
        let tenant_id = original.tenant.to_string();
        assert_eq!(identity.tenant_ids, vec![tenant_id.clone()]);
        assert_eq!(
            identity.memberships,
            vec![
                MembershipIdentity {
                    tenant_id: tenant_id.clone(),
                    subject: HUMAN.to_owned(),
                    role: "admin".to_owned(),
                },
                MembershipIdentity {
                    tenant_id: tenant_id.clone(),
                    subject: WORKER.to_owned(),
                    role: "worker".to_owned(),
                },
            ]
        );
        assert_eq!(
            identity.bindings,
            vec![BindingIdentity {
                tenant_id: tenant_id.clone(),
                namespace_id: namespace_id.clone(),
                subject: WORKER.to_owned(),
                pattern: "*".to_owned(),
                updated_by: "test".to_owned(),
            }]
        );
        assert_eq!(identity.session_count, 1);
        assert_eq!(
            identity.sessions,
            vec![SessionIdentity {
                tenant_id,
                id: worker.worker_id.clone(),
                namespace_id: namespace_id.clone(),
                subject: WORKER.to_owned(),
                worker_name: "wire-test-worker".to_owned(),
                activities: vec!["act.a".to_owned()],
                languages: vec!["rust".to_owned()],
                pools: Vec::new(),
                max_inflight: 100,
                accepted_protocol_minor: 5,
            }]
        );
        (
            worker.worker_id.clone(),
            worker.session.clone(),
            completion,
            identity,
        )
    };

    let database_name = original.api.db.name.clone();
    let admin_url = original.api.db.admin_url.clone();
    let tenant = original.tenant;
    let other_tenant = original.other_tenant;
    drop(original);

    let db = attach_existing_db(&admin_url, &database_name).await;
    let api = start_on_db(db, runtime_options()).await;
    let reopened = Platform {
        api,
        tenant,
        other_tenant,
    };
    assert_pool_roles(&reopened).await;
    assert_eq!(
        identity_witness(&reopened).await,
        identity_before,
        "the second API must reopen the exact durable tenant, memberships, binding and sole session"
    );

    let lookups_before = reopened.api.authentication_lookups();
    let reopened_namespace = reopened.namespace_id().await;
    assert_eq!(reopened_namespace, namespace_id);
    assert!(
        reopened.api.authentication_lookups() > lookups_before,
        "the current JWT must pass through current tenant and membership lookup"
    );
    assert_eq!(reopened.api.db.name, database_name);
    assert_eq!(reopened.status(&run).await, "RUNNING");

    let mut resumed = reopened.connect(WORKER, TENANT).await;
    resumed.worker_id = worker_id.clone();
    resumed.session = session;
    assert_eq!(
        identity_witness(&reopened).await,
        identity_before,
        "transport reconnect must not recreate worker authority"
    );
    assert!(
        resumed.poll(1).await.is_empty(),
        "the original session resumes while its original task remains leased"
    );
    assert_eq!(
        identity_witness(&reopened).await,
        identity_before,
        "session use may renew expiry but not change identity or security fields"
    );

    let request = resumed.req(completion);
    let completed = resumed
        .client
        .complete_step(request)
        .await
        .expect("complete the exact original request after API reopen")
        .into_inner();
    assert!(!completed.already_applied);
    assert_eq!(completed.step_status, "SUCCEEDED");
    assert_eq!(
        identity_witness(&reopened).await,
        identity_before,
        "reporting must not register another session or widen a binding"
    );

    let settled = reopened.tick().await;
    assert_eq!(settled.failed, 0, "settle reopened run: {settled:?}");
    assert_eq!(reopened.status(&run).await, "SUCCEEDED");
}
