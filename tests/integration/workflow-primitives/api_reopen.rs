//! A current API instance can reopen its owned database and honor durable public worker authority.
use std::time::Duration;

use sagat_api_grpc::pb::StepCommandResponse;
use sagat_application::entropy::Token;
use sagat_application::worker::WorkerSettings;
use serde_json::json;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use tokio::{
    task::{JoinError, JoinHandle},
    time::{Instant, MissedTickBehavior, interval, sleep_until, timeout},
};

use crate::capacity_finish::commit_proxy::{CommitAckLossProxy, CommitResolution};
use crate::support::api::{Options, start_on_db, start_on_db_with_pool};
use crate::support::worker::{HUMAN, Platform, TENANT, Worker, single_ir};
use crate::support::{RUNTIME_ROLE, attach_existing_db, hold_server};

const WORKER: &str = "retained-db-public-worker";
const PROXY_APPLICATION: &str = "sagat-test-capacity-commit-loss";

type PublicReport = Result<tonic::Response<StepCommandResponse>, tonic::Status>;

struct PendingReport(Option<JoinHandle<PublicReport>>);

impl PendingReport {
    fn new(task: JoinHandle<PublicReport>) -> Self {
        Self(Some(task))
    }

    async fn join(&mut self) -> Result<PublicReport, JoinError> {
        let joined = self.0.as_mut().expect("pending public report").await;
        self.0 = None;
        joined
    }
}

impl Drop for PendingReport {
    fn drop(&mut self) {
        if let Some(task) = self.0.take() {
            task.abort();
        }
    }
}

fn early_rpc_outcome(joined: Result<PublicReport, JoinError>) -> String {
    match joined {
        Ok(Ok(response)) => format!(
            "rpc_succeeded=true already_applied={}",
            response.get_ref().already_applied
        ),
        Ok(Err(status)) => format!("rpc_succeeded=false tonic_code={:?}", status.code()),
        Err(error) => format!(
            "rpc_succeeded=false join_cancelled={} join_panic={}",
            error.is_cancelled(),
            error.is_panic()
        ),
    }
}

#[derive(sqlx::FromRow)]
struct PublicWriterObservation {
    writer_present: bool,
    database_matches: bool,
    user_matches: bool,
    application_matches: bool,
    active: bool,
    lock_wait: bool,
    blocked_by_exact_gate: bool,
    receipt_insert_marker: bool,
    blocker_lock_marker: bool,
    state: Option<String>,
    wait_event_type: Option<String>,
    wait_event: Option<String>,
    query_age_ms: i64,
    transaction_age_ms: i64,
}

impl std::fmt::Debug for PublicWriterObservation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PublicWriterObservation")
            .field("writer_present", &self.writer_present)
            .field("database_matches", &self.database_matches)
            .field("user_matches", &self.user_matches)
            .field("application_matches", &self.application_matches)
            .field("active", &self.active)
            .field("lock_wait", &self.lock_wait)
            .field("blocked_by_exact_gate", &self.blocked_by_exact_gate)
            .field("receipt_insert_marker", &self.receipt_insert_marker)
            .field("blocker_lock_marker", &self.blocker_lock_marker)
            .field("state", &self.state)
            .field("wait_event_type", &self.wait_event_type)
            .field("wait_event", &self.wait_event)
            .field("query_age_ms", &self.query_age_ms)
            .field("transaction_age_ms", &self.transaction_age_ms)
            .finish()
    }
}

impl PublicWriterObservation {
    fn is_exact_gate_wait(&self) -> bool {
        self.writer_present
            && self.database_matches
            && self.user_matches
            && self.application_matches
            && self.active
            && self.lock_wait
            && self.blocked_by_exact_gate
            && self.receipt_insert_marker
            && self.blocker_lock_marker
    }
}

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

#[derive(PartialEq, Eq, sqlx::FromRow)]
struct ReceiptWitness {
    tenant_id: String,
    scope: String,
    idempotency_key: String,
    request_hash: Vec<u8>,
    resource_id: String,
    created_at: time::OffsetDateTime,
}

#[derive(PartialEq, Eq, sqlx::FromRow)]
struct SessionTokenWitness {
    tenant_id: String,
    id: String,
    token_hash: Vec<u8>,
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

async fn session_token_witness(platform: &Platform, session_id: &str) -> SessionTokenWitness {
    sqlx::query_as(
        "SELECT tenant_id::text AS tenant_id,id::text AS id,token_hash \
         FROM worker_sessions WHERE tenant_id=$1 AND id=$2 AND subject=$3",
    )
    .bind(platform.tenant.as_uuid())
    .bind(uuid::Uuid::parse_str(session_id).expect("worker session id"))
    .bind(WORKER)
    .fetch_one(&platform.api.db.pool)
    .await
    .expect("read exact durable worker session token hash")
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

fn positive_advisory_key() -> i64 {
    i64::try_from(uuid::Uuid::new_v4().as_u128() & ((1_u128 << 63) - 1))
        .expect("positive advisory key")
}

fn completion_request_hash(completion: &sagat_api_grpc::pb::CompleteStepRequest) -> Vec<u8> {
    let attempt = u32::try_from(completion.attempt)
        .expect("public completion attempt must be nonnegative")
        .to_be_bytes();
    let generation = u64::try_from(completion.generation)
        .expect("public completion generation must be nonnegative")
        .to_be_bytes();
    let parts: [&[u8]; 4] = [
        b"complete",
        &attempt,
        &generation,
        completion.output_json.as_bytes(),
    ];
    let mut fingerprint = Vec::new();
    for part in parts {
        fingerprint.extend_from_slice(&(part.len() as u64).to_be_bytes());
        fingerprint.extend_from_slice(part);
    }
    Sha256::digest(fingerprint).to_vec()
}

async fn exact_root_receipt_scope(
    platform: &Platform,
    completion: &sagat_api_grpc::pb::CompleteStepRequest,
) -> String {
    let token_prefix: String = sqlx::query_scalar(
        "SELECT encode(substring(lease_token_hash FROM 1 FOR 8),'hex') FROM steps \
         WHERE tenant_id=$1 AND run_id=$2 AND step_id=$3 AND node='a' \
           AND status='LEASED' AND attempt=$4 AND generation=$5",
    )
    .bind(platform.tenant.as_uuid())
    .bind(uuid::Uuid::parse_str(&completion.run_id).expect("run id"))
    .bind(uuid::Uuid::parse_str(&completion.step_id).expect("step id"))
    .bind(completion.attempt)
    .bind(completion.generation)
    .fetch_one(&platform.api.db.pool)
    .await
    .expect("read exact original lease hash prefix");
    assert_eq!(token_prefix.len(), 16, "lease hash prefix width");
    format!("step:{}:a:{token_prefix}", completion.run_id)
}

async fn install_exact_receipt_gate(
    pool: &PgPool,
    tenant: sagat_domain::identity::TenantId,
    scope: &str,
    idempotency_key: &str,
    advisory_key: i64,
) {
    assert!(
        !scope.contains('\'') && !idempotency_key.contains('\''),
        "generated receipt identity must be safe for fixed test DDL"
    );
    let ddl = format!(
        "CREATE FUNCTION api_reopen_public_receipt_gate() RETURNS trigger LANGUAGE plpgsql AS $$ \
         BEGIN \
           IF NEW.tenant_id='{tenant}'::uuid AND NEW.scope='{scope}' \
              AND NEW.idempotency_key='{idempotency_key}' THEN \
             PERFORM pg_advisory_xact_lock({advisory_key}); \
           END IF; \
           RETURN NEW; \
         END $$; \
         CREATE TRIGGER api_reopen_public_receipt_gate BEFORE INSERT ON command_dedupe \
         FOR EACH ROW EXECUTE FUNCTION api_reopen_public_receipt_gate();"
    );
    sqlx::raw_sql(sqlx::AssertSqlSafe(ddl))
        .execute(pool)
        .await
        .expect("install exact public receipt gate");
}

async fn remove_exact_receipt_gate(pool: &PgPool) {
    sqlx::raw_sql(
        "DROP TRIGGER api_reopen_public_receipt_gate ON command_dedupe; \
         DROP FUNCTION api_reopen_public_receipt_gate();",
    )
    .execute(pool)
    .await
    .expect("remove exact public receipt gate");
}

async fn wait_for_exact_public_writer(
    pool: &PgPool,
    backend_pid: i32,
    gate_pid: i32,
    database_name: &str,
    reporting: &mut PendingReport,
    proxy: &mut CommitAckLossProxy,
) -> i32 {
    let started = Instant::now();
    let deadline = started + Duration::from_secs(5);
    let mut poll = interval(Duration::from_millis(50));
    poll.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut last = None;
    loop {
        tokio::select! {
            joined = reporting.join() => panic!(
                "public RPC ended before the exact receipt gate: {:?}",
                early_rpc_outcome(joined)
            ),
            termination = proxy.wait_termination() => panic!(
                "relay ended before the exact receipt gate: termination={termination:?} progress={:?}",
                proxy.progress()
            ),
            _ = sleep_until(deadline) => panic!(
                "timed out waiting for: captured supplied-pool backend executes the exact public receipt insert; observer_elapsed_ms={} writer={last:?} relay={:?}",
                started.elapsed().as_millis().min(5_000),
                proxy.progress()
            ),
            _ = poll.tick() => {}
        }

        let observation = sqlx::query_as::<_, PublicWriterObservation>(
            "SELECT writer.pid IS NOT NULL AS writer_present, \
                    coalesce(writer.datname=$3 AND blocker.datname=writer.datname,false) AS database_matches, \
                    coalesce(writer.usename='sagat_app',false) AS user_matches, \
                    coalesce(writer.application_name=$4,false) AS application_matches, \
                    coalesce(writer.state='active',false) AS active, \
                    coalesce(writer.wait_event_type='Lock',false) AS lock_wait, \
                    coalesce($2=ANY(pg_blocking_pids(writer.pid)),false) AS blocked_by_exact_gate, \
                    coalesce(position('INSERT INTO command_dedupe' in writer.query)>0,false) AS receipt_insert_marker, \
                    coalesce(position('pg_advisory_xact_lock' in blocker.query)>0,false) AS blocker_lock_marker, \
                    writer.state,writer.wait_event_type,writer.wait_event, \
                    LEAST(30000,GREATEST(0,coalesce((extract(epoch FROM clock_timestamp()-writer.query_start)*1000)::bigint,0))) AS query_age_ms, \
                    LEAST(30000,GREATEST(0,coalesce((extract(epoch FROM clock_timestamp()-writer.xact_start)*1000)::bigint,0))) AS transaction_age_ms \
             FROM (SELECT 1) seed \
             LEFT JOIN pg_stat_activity writer ON writer.pid=$1 \
             LEFT JOIN pg_stat_activity blocker ON blocker.pid=$2",
        )
        .bind(backend_pid)
        .bind(gate_pid)
        .bind(database_name)
        .bind(PROXY_APPLICATION)
        .fetch_one(pool);
        tokio::pin!(observation);
        let observed = tokio::select! {
            joined = reporting.join() => panic!(
                "public RPC ended before the exact receipt gate: {:?}",
                early_rpc_outcome(joined)
            ),
            termination = proxy.wait_termination() => panic!(
                "relay ended before the exact receipt gate: termination={termination:?} progress={:?}",
                proxy.progress()
            ),
            _ = sleep_until(deadline) => panic!(
                "timed out waiting for: captured supplied-pool backend executes the exact public receipt insert; observer_elapsed_ms={} writer={last:?} relay={:?}",
                started.elapsed().as_millis().min(5_000),
                proxy.progress()
            ),
            observed = &mut observation => observed.expect("observe exact public receipt writer"),
        };
        if observed.is_exact_gate_wait() {
            return backend_pid;
        }
        last = Some(observed);
    }
}

async fn receipt_witness(
    pool: &PgPool,
    tenant: sagat_domain::identity::TenantId,
    scope: &str,
    idempotency_key: &str,
) -> ReceiptWitness {
    sqlx::query_as(
        "SELECT tenant_id::text AS tenant_id,scope,idempotency_key,request_hash,resource_id,created_at \
         FROM command_dedupe WHERE tenant_id=$1 AND scope=$2 AND idempotency_key=$3",
    )
    .bind(tenant.as_uuid())
    .bind(scope)
    .bind(idempotency_key)
    .fetch_one(pool)
    .await
    .expect("read exact public completion receipt")
}

async fn persisted_step_output(
    pool: &PgPool,
    tenant: sagat_domain::identity::TenantId,
    run_id: &str,
    step_id: &str,
) -> (i32, Vec<u8>) {
    sqlx::query_as(
        "SELECT octet_length(payload.content),payload.content \
         FROM steps step JOIN payloads payload \
           ON payload.tenant_id=step.tenant_id AND payload.digest=step.output_digest \
         WHERE step.tenant_id=$1 AND step.run_id=$2 AND step.step_id=$3 \
           AND step.status='SUCCEEDED'",
    )
    .bind(tenant.as_uuid())
    .bind(uuid::Uuid::parse_str(run_id).expect("completed run id"))
    .bind(uuid::Uuid::parse_str(step_id).expect("completed step id"))
    .fetch_one(pool)
    .await
    .expect("read exact retained public completion output")
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

#[tokio::test(flavor = "current_thread")]
async fn supplied_single_connection_pool_backs_actual_public_completion_after_reopen() {
    timeout(
        Duration::from_secs(90),
        supplied_single_connection_pool_scenario(),
    )
    .await
    .expect("supplied-pool public completion scenario deadline");
}

async fn supplied_single_connection_pool_scenario() {
    let _server = hold_server();
    let original = crate::support::worker::platform_with(runtime_options()).await;
    assert_pool_roles(&original).await;

    let namespace_id = original.namespace_id().await;
    let workflow = original
        .publish(
            "retained-db-supplied-pool",
            &single_ir("retained-db-supplied-pool", None),
        )
        .await;
    let run = original
        .start(&workflow, &json!({"request":"supplied-pool"}))
        .await;
    let scheduled = original.tick().await;
    assert_eq!(scheduled.failed, 0, "schedule supplied-pool task");

    let output = format!("\"{}\"", "x".repeat(249_998));
    assert_eq!(output.len(), 250_000, "benign completion byte count");
    let (worker_id, session, completion, identity_before) = {
        let mut worker = original.worker(WORKER).await;
        worker.register_version(&["act.a"], "1.5").await;
        let lease = worker.one_task().await;
        let completion = Worker::complete_message(&lease, &output);
        let identity = identity_witness(&original).await;
        (
            worker.worker_id.clone(),
            worker.session.clone(),
            completion,
            identity,
        )
    };
    let expected_session_token_hash = Token::hash_of(&session);
    let original_session_token = session_token_witness(&original, &worker_id).await;
    assert!(
        original_session_token.tenant_id == original.tenant.to_string()
            && original_session_token.id == worker_id
            && original_session_token.token_hash == expected_session_token_hash,
        "registered worker session must persist the exact original token hash"
    );
    let receipt_scope = exact_root_receipt_scope(&original, &completion).await;
    let receipt_key = completion.idempotency_key.clone();
    let expected_receipt_hash = completion_request_hash(&completion);

    let database_name = original.api.db.name.clone();
    let admin_url = original.api.db.admin_url.clone();
    let tenant = original.tenant;
    let other_tenant = original.other_tenant;
    drop(original);

    let db = attach_existing_db(&admin_url, &database_name).await;
    let mut proxy = CommitAckLossProxy::start(&db, CommitResolution::Committed).await;
    assert_eq!(
        proxy.pool.options().get_max_connections(),
        1,
        "supplied proxy pool connection bound"
    );
    sqlx::query("SET statement_timeout='30s'")
        .execute(&proxy.pool)
        .await
        .expect("set supplied-pool statement timeout");
    sqlx::query("SET idle_in_transaction_session_timeout='30s'")
        .execute(&proxy.pool)
        .await
        .expect("set supplied-pool idle transaction timeout");

    let api = start_on_db_with_pool(db, runtime_options(), proxy.pool.clone()).await;
    let reopened = Platform {
        api,
        tenant,
        other_tenant,
    };
    assert_pool_roles(&reopened).await;
    assert_eq!(reopened.api.db.name, database_name, "exact owned database");
    assert_eq!(
        reopened.namespace_id().await,
        namespace_id,
        "durable namespace"
    );
    assert_eq!(
        reopened.status(&run).await,
        "RUNNING",
        "durable original run"
    );
    assert_eq!(
        identity_witness(&reopened).await,
        identity_before,
        "reopen must preserve the exact worker authority"
    );
    assert!(
        session_token_witness(&reopened, &worker_id).await == original_session_token,
        "reopen must preserve the exact durable worker session token hash"
    );

    let (
        actual_database,
        actual_user,
        backend_pid,
        application_name,
        statement_bounded,
        idle_bounded,
    ): (String, String, i32, String, bool, bool) = sqlx::query_as(
        "SELECT current_database()::text,current_user::text,pg_backend_pid(), \
                current_setting('application_name'), \
                current_setting('statement_timeout')='30s', \
                current_setting('idle_in_transaction_session_timeout')='30s'",
    )
    .fetch_one(&reopened.api.api_pool)
    .await
    .expect("capture supplied public backend");
    assert_eq!(actual_database, database_name, "supplied database identity");
    assert_eq!(actual_user, RUNTIME_ROLE, "supplied runtime role");
    assert_eq!(
        application_name, PROXY_APPLICATION,
        "supplied relay identity"
    );
    assert!(statement_bounded, "caller configured statement timeout");
    assert!(idle_bounded, "caller configured idle transaction timeout");

    let advisory_key = positive_advisory_key();
    install_exact_receipt_gate(
        &reopened.api.db.pool,
        reopened.tenant,
        &receipt_scope,
        &receipt_key,
        advisory_key,
    )
    .await;
    let mut gate = reopened
        .api
        .db
        .pool
        .begin()
        .await
        .expect("begin exact receipt gate");
    let gate_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *gate)
        .await
        .expect("receipt gate backend pid");
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(advisory_key)
        .execute(&mut *gate)
        .await
        .expect("hold exact receipt gate");

    let lookups_before = reopened.api.authentication_lookups();
    let mut resumed = reopened.connect(WORKER, TENANT).await;
    resumed.worker_id = worker_id.clone();
    resumed.session = session;
    let replay_completion = completion.clone();
    let mut report_client = resumed.client.clone();
    let report_request = resumed.req(completion);
    let mut reporting = PendingReport::new(tokio::spawn(async move {
        report_client.complete_step(report_request).await
    }));

    let observed_pid = wait_for_exact_public_writer(
        &reopened.api.db.pool,
        backend_pid,
        gate_pid,
        &database_name,
        &mut reporting,
        &mut proxy,
    )
    .await;
    assert_eq!(
        observed_pid, backend_pid,
        "the captured supplied backend must execute the public completion"
    );
    gate.rollback().await.expect("release exact receipt gate");

    let completed = timeout(Duration::from_secs(5), reporting.join())
        .await
        .expect("public completion deadline")
        .expect("public completion task")
        .expect("genuine public completion")
        .into_inner();
    assert!(
        !completed.already_applied,
        "first public completion applies"
    );
    assert_eq!(completed.step_status, "SUCCEEDED", "completed step status");
    assert!(
        reopened.api.authentication_lookups() > lookups_before,
        "public completion must perform current authentication"
    );
    let (persisted_output_bytes, persisted_output) = persisted_step_output(
        &reopened.api.db.pool,
        reopened.tenant,
        &replay_completion.run_id,
        &replay_completion.step_id,
    )
    .await;
    assert!(
        persisted_output_bytes == 250_000 && persisted_output.as_slice() == output.as_bytes(),
        "public completion must retain the exact original 250000-byte output"
    );

    let original_receipt = receipt_witness(
        &reopened.api.db.pool,
        reopened.tenant,
        &receipt_scope,
        &receipt_key,
    )
    .await;
    assert_eq!(
        original_receipt.request_hash.len(),
        32,
        "receipt hash width"
    );
    assert!(
        original_receipt.request_hash == expected_receipt_hash,
        "receipt hash must match the canonical original completion request"
    );
    let (_, version) = completed
        .command_id
        .rsplit_once(':')
        .expect("versioned command id");
    let expected_resource = format!(
        "{}|{}|{version}",
        completed.step_status, completed.run_status
    );
    assert_eq!(
        original_receipt.resource_id, expected_resource,
        "receipt stores the genuine public result"
    );

    let replay_request = resumed.req(replay_completion);
    let replayed = resumed
        .client
        .complete_step(replay_request)
        .await
        .expect("replay exact original completion")
        .into_inner();
    assert!(replayed.already_applied, "exact public completion replays");
    assert_eq!(
        replayed.command_id, completed.command_id,
        "stable command id"
    );
    assert_eq!(
        replayed.step_status, completed.step_status,
        "stable step status"
    );
    assert_eq!(
        replayed.run_status, completed.run_status,
        "stable run status"
    );
    let replay_receipt = receipt_witness(
        &reopened.api.db.pool,
        reopened.tenant,
        &receipt_scope,
        &receipt_key,
    )
    .await;
    assert!(
        replay_receipt == original_receipt,
        "replay must preserve the exact original receipt"
    );
    assert_eq!(
        identity_witness(&reopened).await,
        identity_before,
        "completion and replay must preserve worker authority"
    );
    assert!(
        session_token_witness(&reopened, &worker_id).await == original_session_token,
        "completion and replay must preserve the exact durable worker session token hash"
    );

    remove_exact_receipt_gate(&reopened.api.db.pool).await;
    let settled = reopened.tick().await;
    assert_eq!(settled.failed, 0, "settle supplied-pool run");
    assert_eq!(
        reopened.status(&run).await,
        "SUCCEEDED",
        "settled run status"
    );

    // This proxy remains deliberately unarmed. Its close helper requires a resolved injected fault,
    // so first drop every API/client pool reference, then close the supplied pool under a deadline,
    // and finally let the existing abort-on-drop guard own the ordinary EOF relay task.
    drop(resumed);
    drop(reopened);
    timeout(Duration::from_secs(5), proxy.pool.close())
        .await
        .expect("supplied pool close deadline");
    drop(proxy);
}
