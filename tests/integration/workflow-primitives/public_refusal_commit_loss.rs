//! Public scoped-output refusal through a reopened API's supplied one-connection runtime pool.
//! This prerequisite intentionally does not inject or claim a lost COMMIT reply.

use std::{sync::Arc, time::Duration};

use prost::Message as _;
use sagat_api_grpc::pb::{CompleteStepRequest, StepCommandResponse, TaskLease};
use sagat_application::{
    entropy::{SystemEntropy, Token},
    execution::{DefinitionCache, Engine, MAX_RUN_OUTPUT_BYTES, PathExecutionStore},
    worker::WorkerSettings,
};
use sagat_domain::{
    execution::{InstancePath, RunId},
    workflow::NodeId,
};
use sagat_infra_ir::JsonEvaluator;
use sagat_infra_postgres::PgRunStore;
use sagat_scheduler::{Keeper, KeeperSettings, Placement, Scheduler, SchedulerSettings};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use tokio::{
    task::{JoinError, JoinHandle},
    time::{Instant, MissedTickBehavior, interval, sleep_until, timeout},
};

use crate::capacity_finish::commit_proxy::{CommitAckLossProxy, CommitResolution};
use crate::support::api::{Options, start_on_db_with_pool};
use crate::support::worker::{Platform, TENANT, Worker};
use crate::support::{HeldServer, RUNTIME_ROLE, attach_existing_db, hold_server};

#[path = "public_refusal_committed_loss.rs"]
mod committed_loss;

const WORKER: &str = "public-scoped-refusal-worker";
const PROXY_APPLICATION: &str = "sagat-test-capacity-commit-loss";
const OUTPUT_BYTES: usize = 250_000;

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

type PublicScheduler = Scheduler<PgRunStore, JsonEvaluator, SystemEntropy>;

#[derive(PartialEq, Eq, sqlx::FromRow)]
struct LeaseWitness {
    step_id: uuid::Uuid,
    status: String,
    lease_token_hash: Vec<u8>,
    attempt: i32,
    generation: i64,
    leased_by: String,
    lease_expires_at: time::OffsetDateTime,
    updated_at: time::OffsetDateTime,
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

#[derive(sqlx::FromRow)]
struct RefusalState {
    run_status: String,
    stop_reason: Option<String>,
    stop_code: Option<String>,
    scope_lifecycle: String,
    member_lifecycle: String,
    slot_held: bool,
    step_status: String,
    output_absent: bool,
    logical_output_bytes: i64,
    run_version: i64,
    head_seq: i64,
    journal_head: i64,
    checkpoints: i64,
    returns: i64,
    terminal_steps: i64,
    sagas: i64,
    stops: i64,
    successes: i64,
    stop_seq: Option<i64>,
    success_seq: Option<i64>,
    candidate_payloads: i64,
    candidate_event_payloads: i64,
    completion_payloads: i64,
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

fn collection_document(name: &str) -> Value {
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": {"name": name},
        "entrypoint": "lines",
        "bodies": {"line": {"entrypoint": "reserve", "nodes": [
            {"id": "reserve", "type": "activity", "activity": "line.reserve",
             "compensateWith": "line.release",
             "compensationRetry": {"maxAttempts": 2, "initialDelayMs": 100,
                                    "maxDelayMs": 100, "strategy": "fixed", "jitter": false},
             "next": "returned"},
            {"id": "returned", "type": "return", "result": "1"}
        ]}},
        "nodes": [
            {"id": "lines", "type": "forEach", "items": "input.lines", "body": "line",
             "result": "bodyResult", "maxItems": 17, "maxConcurrency": 1, "next": "done"},
            {"id": "done", "type": "end"}
        ]
    })
}

fn json_text(fill: char) -> String {
    let output = format!("\"{}\"", fill.to_string().repeat(OUTPUT_BYTES - 2));
    assert!(output.len() == OUTPUT_BYTES, "fixed JSON output width");
    output
}

async fn next_reserve(scheduler: &PublicScheduler, worker: &mut Worker<'_>) -> TaskLease {
    for _ in 0..12 {
        let report = scheduler.tick().await;
        assert!(report.failed == 0, "ordinary setup scheduler pass succeeds");
        let mut tasks = worker.poll(1).await;
        if !tasks.is_empty() {
            assert!(tasks.len() == 1, "concurrency-one fixture leases one task");
            let task = tasks.remove(0);
            assert!(
                task.activity_name == "line.reserve",
                "fixture leases reserve activity"
            );
            return task;
        }
    }
    panic!("bounded scheduler passes did not produce the next reserve lease");
}

async fn lease_witness(pool: &PgPool, tenant: uuid::Uuid, run: uuid::Uuid) -> LeaseWitness {
    sqlx::query_as(
        "SELECT step_id,status,lease_token_hash,attempt,generation,leased_by,lease_expires_at,updated_at \
         FROM steps WHERE tenant_id=$1 AND run_id=$2 \
           AND instance_path_nodes=ARRAY['lines']::text[] \
           AND instance_path_indices=ARRAY[16]::integer[] AND node='reserve'",
    )
    .bind(tenant)
    .bind(run)
    .fetch_one(pool)
    .await
    .expect("read exact original scoped lease")
}

async fn durable_snapshot(
    pool: &PgPool,
    tenant: uuid::Uuid,
    run: uuid::Uuid,
    worker: &str,
) -> Value {
    sqlx::query_scalar(
        r#"SELECT jsonb_build_object(
          'run',to_jsonb(run),
          'version',(SELECT to_jsonb(version_row) FROM workflow_versions version_row
                     WHERE version_row.tenant_id=run.tenant_id AND version_row.id=run.workflow_version_id),
          'scope',(SELECT to_jsonb(scope) FROM iteration_scopes scope
                   WHERE scope.tenant_id=run.tenant_id AND scope.run_id=run.id),
          'items',(SELECT coalesce(jsonb_agg(to_jsonb(item) ORDER BY item.iteration_index),'[]')
                   FROM collection_scope_items item
                   WHERE item.tenant_id=run.tenant_id AND item.run_id=run.id),
          'iterations',(SELECT coalesce(jsonb_agg(to_jsonb(member) ORDER BY member.iteration_index),'[]')
                        FROM collection_iterations member
                        WHERE member.tenant_id=run.tenant_id AND member.run_id=run.id),
          'checkpoints',(SELECT coalesce(jsonb_agg(to_jsonb(point) ORDER BY point.iteration_index),'[]')
                         FROM iteration_checkpoints point
                         WHERE point.tenant_id=run.tenant_id AND point.run_id=run.id),
          'steps',(SELECT coalesce(jsonb_agg(to_jsonb(step)
                    ORDER BY step.instance_path_nodes,step.instance_path_indices,step.node),'[]')
                   FROM steps step WHERE step.tenant_id=run.tenant_id AND step.run_id=run.id),
          'retainedSteps',(SELECT coalesce(jsonb_agg(to_jsonb(step)
                            ORDER BY step.instance_path_indices),'[]')
                           FROM steps step WHERE step.tenant_id=run.tenant_id AND step.run_id=run.id
                             AND step.instance_path_nodes=ARRAY['lines']::text[]
                             AND step.instance_path_indices[1]<16 AND step.node='reserve'),
          'journal',(SELECT coalesce(jsonb_agg(jsonb_build_object(
                    'row',to_jsonb(event),'payload',encode(payload.content,'hex')) ORDER BY event.seq),'[]')
                     FROM run_events event LEFT JOIN payloads payload
                       ON payload.tenant_id=event.tenant_id AND payload.digest=event.payload_digest
                     WHERE event.tenant_id=run.tenant_id AND event.run_id=run.id),
          'usage',(SELECT to_jsonb(usage) FROM run_instance_usage usage
                   WHERE usage.tenant_id=run.tenant_id AND usage.run_id=run.id),
          'iterationUsage',(SELECT to_jsonb(usage) FROM tenant_iteration_usage usage
                            WHERE usage.tenant_id=run.tenant_id),
          'telemetry',(SELECT to_jsonb(head) FROM telemetry_pending head
                       WHERE head.tenant_id=run.tenant_id AND head.run_id=run.id),
          'payloads',(SELECT coalesce(jsonb_agg(to_jsonb(payload) ORDER BY encode(payload.digest,'hex')),'[]')
                      FROM payloads payload WHERE payload.tenant_id=run.tenant_id),
          'retainedUsage',(SELECT to_jsonb(usage) FROM tenant_retained_usage usage
                           WHERE usage.tenant_id=run.tenant_id),
          'receipts',(SELECT coalesce(jsonb_agg(to_jsonb(receipt)
                      ORDER BY receipt.scope,receipt.idempotency_key),'[]')
                      FROM command_dedupe receipt WHERE receipt.tenant_id=run.tenant_id),
          'audits',(SELECT coalesce(jsonb_agg(to_jsonb(audit) ORDER BY audit.id),'[]')
                    FROM audit_records audit WHERE audit.tenant_id=run.tenant_id),
          'outbox',(SELECT coalesce(jsonb_agg(to_jsonb(message) ORDER BY message.id),'[]')
                    FROM event_outbox message WHERE message.tenant_id=run.tenant_id),
          'blobs',(SELECT coalesce(jsonb_agg(to_jsonb(blob) ORDER BY blob.id),'[]')
                   FROM blobs blob WHERE blob.tenant_id=run.tenant_id AND blob.run_id=run.id),
          'sagas',(SELECT coalesce(jsonb_agg(to_jsonb(saga)
                   ORDER BY saga.owner_path_nodes,saga.owner_path_indices,saga.owner_node),'[]')
                   FROM scope_sagas saga WHERE saga.tenant_id=run.tenant_id AND saga.run_id=run.id),
          'sagaEntries',(SELECT coalesce(jsonb_agg(to_jsonb(entry)
                          ORDER BY entry.owner_path_nodes,entry.owner_path_indices,
                                   entry.owner_node,entry.iteration_index,entry.node),'[]')
                         FROM scope_saga_entries entry
                         WHERE entry.tenant_id=run.tenant_id AND entry.run_id=run.id),
          'chains',(SELECT coalesce(jsonb_agg(to_jsonb(segment) ORDER BY segment.segment_index),'[]')
                    FROM chain_segments segment
                    WHERE segment.tenant_id=run.tenant_id AND segment.run_id=run.id),
          'capacityGroups',(SELECT coalesce(jsonb_agg(to_jsonb(g) ORDER BY g.group_id),'[]')
                            FROM capacity_groups g WHERE g.tenant_id=run.tenant_id),
          'capacityMembers',(SELECT coalesce(jsonb_agg(to_jsonb(m) ORDER BY m.group_id,m.member_id),'[]')
                             FROM capacity_group_members m WHERE m.tenant_id=run.tenant_id),
          'capacityReceipts',(SELECT coalesce(jsonb_agg(to_jsonb(r) ORDER BY r.group_id,r.operation_key),'[]')
                              FROM capacity_group_finish_receipts r WHERE r.tenant_id=run.tenant_id),
          'quotaDeltas',(SELECT coalesce(jsonb_agg(to_jsonb(d) ORDER BY d.transaction_id,d.tenant_id),'[]')
                        FROM quota_transaction_deltas d WHERE d.tenant_id=run.tenant_id),
          'authority',jsonb_build_object(
            'memberships',(SELECT coalesce(jsonb_agg(to_jsonb(membership) ORDER BY membership.subject),'[]')
                           FROM memberships membership WHERE membership.tenant_id=run.tenant_id),
            'bindings',(SELECT coalesce(jsonb_agg(to_jsonb(binding)
                        ORDER BY binding.namespace_id,binding.subject,binding.pattern),'[]')
                        FROM worker_bindings binding WHERE binding.tenant_id=run.tenant_id),
            'bindingPools',(SELECT coalesce(jsonb_agg(to_jsonb(grant_row)
                             ORDER BY grant_row.namespace_id,grant_row.subject,grant_row.pool),'[]')
                            FROM worker_binding_pools grant_row WHERE grant_row.tenant_id=run.tenant_id),
            'drains',(SELECT coalesce(jsonb_agg(to_jsonb(drain)
                        ORDER BY drain.namespace_id,drain.subject),'[]')
                      FROM worker_drains drain WHERE drain.tenant_id=run.tenant_id),
            'sessions',(SELECT coalesce(jsonb_agg(to_jsonb(session) ORDER BY session.id),'[]')
                        FROM worker_sessions session
                        WHERE session.tenant_id=run.tenant_id AND session.subject=$3)))
        FROM runs run WHERE run.tenant_id=$1 AND run.id=$2"#,
    )
    .bind(tenant)
    .bind(run)
    .bind(worker)
    .fetch_one(pool)
    .await
    .expect("read complete public refusal snapshot")
}

fn receipt_scope(completion: &CompleteStepRequest) -> String {
    let identity = completion
        .instance
        .as_ref()
        .expect("scoped instance identity");
    assert!(
        identity.definition_node_id == "reserve"
            && identity.instance_path.len() == 1
            && identity.instance_path[0].scope_node_id == "lines"
            && identity.instance_path[0].iteration_index == 16,
        "original request carries the exact seventeenth reserve identity"
    );
    let segment = &identity.instance_path[0];
    let node = &identity.definition_node_id;
    let prefix = Token::hash_of(&completion.lease_token)
        .iter()
        .take(8)
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!(
        "step:{}:scoped:{}:{}:{};{}:{}:{prefix}",
        completion.run_id,
        segment.scope_node_id.len(),
        segment.scope_node_id,
        segment.iteration_index,
        node.len(),
        node
    )
}

fn request_hash(completion: &CompleteStepRequest) -> Vec<u8> {
    let attempt = u32::try_from(completion.attempt)
        .expect("nonnegative completion attempt")
        .to_be_bytes();
    let generation = u64::try_from(completion.generation)
        .expect("nonnegative completion generation")
        .to_be_bytes();
    let mut fingerprint = Vec::new();
    for part in [
        b"complete".as_slice(),
        attempt.as_slice(),
        generation.as_slice(),
        completion.output_json.as_bytes(),
    ] {
        fingerprint.extend_from_slice(&(part.len() as u64).to_be_bytes());
        fingerprint.extend_from_slice(part);
    }
    Sha256::digest(fingerprint).to_vec()
}

async fn receipt(pool: &PgPool, tenant: uuid::Uuid, scope: &str, key: &str) -> ReceiptWitness {
    sqlx::query_as(
        "SELECT tenant_id::text AS tenant_id,scope,idempotency_key,request_hash,resource_id,created_at \
         FROM command_dedupe WHERE tenant_id=$1 AND scope=$2 AND idempotency_key=$3",
    )
    .bind(tenant)
    .bind(scope)
    .bind(key)
    .fetch_one(pool)
    .await
    .expect("read exact public refusal receipt")
}

fn positive_advisory_key() -> i64 {
    i64::try_from(uuid::Uuid::new_v4().as_u128() & ((1_u128 << 63) - 1))
        .expect("positive advisory key")
}

async fn install_exact_receipt_gate(
    pool: &PgPool,
    tenant: uuid::Uuid,
    scope: &str,
    idempotency_key: &str,
    advisory_key: i64,
) {
    assert!(
        !scope.contains('\'') && !idempotency_key.contains('\''),
        "generated receipt identity must be safe for fixed test DDL"
    );
    let ddl = format!(
        "CREATE FUNCTION public_refusal_receipt_gate() RETURNS trigger LANGUAGE plpgsql AS $$ \
         BEGIN \
           IF NEW.tenant_id='{tenant}'::uuid AND NEW.scope='{scope}' \
              AND NEW.idempotency_key='{idempotency_key}' THEN \
             PERFORM pg_advisory_xact_lock({advisory_key}); \
           END IF; \
           RETURN NEW; \
         END $$; \
         CREATE TRIGGER public_refusal_receipt_gate BEFORE INSERT ON command_dedupe \
         FOR EACH ROW EXECUTE FUNCTION public_refusal_receipt_gate();"
    );
    sqlx::raw_sql(sqlx::AssertSqlSafe(ddl))
        .execute(pool)
        .await
        .expect("install exact public receipt gate");
}

async fn remove_exact_receipt_gate(pool: &PgPool) {
    sqlx::raw_sql(
        "DROP TRIGGER public_refusal_receipt_gate ON command_dedupe; \
         DROP FUNCTION public_refusal_receipt_gate();",
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

async fn close_pool(pool: PgPool, message: &'static str) {
    assert!(
        timeout(Duration::from_secs(5), pool.close()).await.is_ok(),
        "{message}"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn ordinary_public_scoped_refusal_reopens_on_supplied_runtime_pool() {
    assert!(
        timeout(
            Duration::from_secs(90),
            ordinary_public_scoped_refusal_scenario()
        )
        .await
        .is_ok(),
        "public scoped refusal supplied-pool scenario deadline"
    );
}

// Owns the original capability and the reopened API; a borrowing Worker is created by each test.
// Do not derive Debug: the saved command/session carry private execution capabilities.
struct ReopenedRefusalFixture {
    _server: HeldServer,
    reopened: Platform,
    proxy: CommitAckLossProxy,
    completion: CompleteStepRequest,
    worker_id: String,
    session: String,
    before: Value,
    lease: LeaseWitness,
    backend_pid: i32,
}

async fn reopened_refusal_fixture(resolution: CommitResolution) -> ReopenedRefusalFixture {
    let _server = hold_server();
    let original = crate::support::worker::platform_with(runtime_options()).await;
    let namespace = original.namespace_id().await;
    let (quota_status, _) = original
        .rest(
            reqwest::Method::PUT,
            "/quotas/iteration_concurrency",
            Some(&json!({"limit": 1})),
        )
        .await;
    assert!(
        quota_status == 200,
        "public iteration quota configuration succeeds"
    );

    let name = format!("public-refusal-reopen-{}", uuid::Uuid::now_v7());
    let workflow = original.publish(&name, &collection_document(&name)).await;
    let run = original
        .start(
            &workflow,
            &json!({"lines": (0_u32..17).collect::<Vec<_>>()}),
        )
        .await;
    let run_uuid = uuid::Uuid::parse_str(&run).expect("public run id");

    let placement = Placement::new(format!("{name}-scheduler"));
    let keeper = Keeper::new(
        original.api.executions.clone(),
        placement.clone(),
        KeeperSettings {
            ttl: Duration::from_secs(60),
        },
    );
    assert!(
        keeper.round().await.is_ok(),
        "setup keeper acquires scheduler shards"
    );
    let scheduler = Scheduler::new(
        original.api.executions.clone(),
        Engine::new(JsonEvaluator),
        SystemEntropy,
        SchedulerSettings::default(),
    )
    .with_children(DefinitionCache::new(
        Arc::new(sagat_infra_ir::JsonIrCodec),
        64,
    ))
    .with_placement(placement.clone());

    let mut worker = original.worker(WORKER).await;
    worker
        .register_version(&["line.reserve", "line.release"], "1.5")
        .await;
    let retained = json_text('x');
    for index in 0..16 {
        let task = next_reserve(&scheduler, &mut worker).await;
        let identity = task.instance.as_ref().expect("retained scoped identity");
        assert!(
            identity.instance_path.len() == 1
                && identity.instance_path[0].iteration_index == index
                && identity.definition_node_id == "reserve",
            "setup completes the expected retained member"
        );
        let request = worker.req(Worker::complete_message(&task, &retained));
        let response = worker.client.complete_step(request).await;
        let response = match response {
            Ok(response) => response.into_inner(),
            Err(_) => panic!("retained public completion succeeds"),
        };
        assert!(
            !response.already_applied && response.step_status == "SUCCEEDED",
            "retained public completion is newly accepted"
        );
    }
    let original_lease = next_reserve(&scheduler, &mut worker).await;
    let candidate = json_text('y');
    let mut completion = Worker::complete_message(&original_lease, &candidate);
    completion.idempotency_key = format!("public-refusal-{}", uuid::Uuid::now_v7());
    completion.traceparent = sagat_telemetry::traceparent(
        sagat_telemetry::TraceId::generate(),
        sagat_telemetry::SpanId::generate(),
        true,
    );
    assert!(
        completion.encoded_len() < 256 * 1024,
        "complete original public request stays below the report message ceiling"
    );

    let tenant = original.tenant;
    let other_tenant = original.other_tenant;
    let tenant_uuid = tenant.as_uuid();
    let worker_id = worker.worker_id.clone();
    let session = worker.session.clone();
    let expected_session_hash = Token::hash_of(&session);
    let original_lease_witness = lease_witness(&original.api.db.pool, tenant_uuid, run_uuid).await;
    assert!(
        original_lease_witness.step_id.to_string() == completion.step_id
            && original_lease_witness.status == "LEASED"
            && original_lease_witness.lease_token_hash == Token::hash_of(&completion.lease_token)
            && original_lease_witness.attempt == completion.attempt
            && original_lease_witness.generation == completion.generation
            && original_lease_witness.leased_by == worker_id,
        "original public request exactly matches its durable live lease"
    );
    let session_hash: Vec<u8> = sqlx::query_scalar(
        "SELECT token_hash FROM worker_sessions \
         WHERE tenant_id=$1 AND namespace_id=$2 AND id=$3 AND subject=$4",
    )
    .bind(tenant_uuid)
    .bind(uuid::Uuid::parse_str(&namespace).expect("namespace id"))
    .bind(uuid::Uuid::parse_str(&worker_id).expect("worker id"))
    .bind(WORKER)
    .fetch_one(&original.api.db.pool)
    .await
    .expect("original durable session token hash");
    assert!(
        session_hash == expected_session_hash,
        "original session hash matches saved capability"
    );

    let candidate_digest = Sha256::digest(candidate.as_bytes()).to_vec();
    let premise: (i64, i64, i64, i64, i64, i64, String, String, bool, i64, i64) = sqlx::query_as(
        r#"SELECT usage.logical_output_bytes,
          (SELECT coalesce(sum(octet_length(payload.content)),0)::bigint
           FROM steps retained_step JOIN payloads payload
             ON payload.tenant_id=retained_step.tenant_id AND payload.digest=retained_step.output_digest
           WHERE retained_step.tenant_id=run.tenant_id AND retained_step.run_id=run.id
             AND retained_step.instance_path_nodes=ARRAY['lines']::text[]
             AND retained_step.node='reserve' AND retained_step.status='SUCCEEDED'),
          (SELECT count(*) FROM payloads WHERE tenant_id=run.tenant_id AND digest=$3),
          (SELECT count(*) FROM collection_scope_items
           WHERE tenant_id=run.tenant_id AND run_id=run.id),
          (SELECT count(*) FROM iteration_checkpoints
           WHERE tenant_id=run.tenant_id AND run_id=run.id),
          (SELECT count(*) FROM steps
           WHERE tenant_id=run.tenant_id AND run_id=run.id
             AND instance_path_nodes=ARRAY['lines']::text[]
             AND node='reserve' AND status='SUCCEEDED' AND output_digest IS NOT NULL),
          scope.lifecycle,member.lifecycle,member.slot_held,
          (SELECT count(*) FROM chain_segments
           WHERE tenant_id=run.tenant_id AND run_id=run.id),
          (SELECT count(*) FROM scope_sagas
           WHERE tenant_id=run.tenant_id AND run_id=run.id)
        FROM runs run JOIN run_instance_usage usage
          ON usage.tenant_id=run.tenant_id AND usage.run_id=run.id
        JOIN iteration_scopes scope
          ON scope.tenant_id=run.tenant_id AND scope.run_id=run.id
        JOIN collection_iterations member
          ON member.tenant_id=run.tenant_id AND member.run_id=run.id
         AND member.scope_id=scope.scope_id AND member.iteration_index=16
        WHERE run.tenant_id=$1 AND run.id=$2 AND run.parent_run_id IS NULL AND run.depth=0"#,
    )
    .bind(tenant_uuid)
    .bind(run_uuid)
    .bind(&candidate_digest)
    .fetch_one(&original.api.db.pool)
    .await
    .expect("genuine public refusal premise");
    assert!(
        premise
            == (
                4_000_000,
                4_000_000,
                0,
                17,
                16,
                16,
                "OPEN".into(),
                "ADMITTED".into(),
                true,
                0,
                0,
            ),
        "sixteen retained outputs and the leased seventeenth member form the exact refusal premise"
    );
    assert!(
        4_000_000_usize + candidate.len() > MAX_RUN_OUTPUT_BYTES,
        "candidate crosses the retained run-output ceiling"
    );
    let before = durable_snapshot(&original.api.db.pool, tenant_uuid, run_uuid, WORKER).await;
    assert!(
        before["version"].is_object()
            && before["version"]["id"] == before["run"]["workflow_version_id"]
            && before["version"]["document"].is_string(),
        "snapshot contains the complete pinned published version and document"
    );
    assert!(
        before["iterationUsage"]["active_iterations"] == 1,
        "the original seventeenth member holds exactly one tenant iteration slot"
    );

    for key in [
        "capacityGroups",
        "capacityMembers",
        "capacityReceipts",
        "quotaDeltas",
    ] {
        assert!(
            before[key].as_array().is_some_and(Vec::is_empty),
            "ordinary fixture has no capacity groups or committed quota delta residue"
        );
    }

    let database_name = original.api.db.name.clone();
    let admin_url = original.api.db.admin_url.clone();
    let old_runtime_pool = original.api.api_pool.clone();
    let old_owner_pool = original.api.db.pool.clone();
    drop(worker);
    keeper.leave().await;
    let departed: (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM scheduler_shards WHERE owner=$1 AND expires_at>clock_timestamp()), \
                (SELECT count(*) FROM scheduler_members WHERE owner=$1)",
    )
    .bind(placement.owner())
    .fetch_one(&original.api.db.pool)
    .await
    .expect("observe clean scheduler departure");
    assert!(
        departed == (0, 0) && placement.snapshot().leases().is_empty(),
        "setup keeper releases all active shards and withdraws membership"
    );
    drop(scheduler);
    drop(keeper);
    drop(original);
    close_pool(
        old_runtime_pool,
        "setup runtime pool closes within its deadline",
    )
    .await;
    close_pool(
        old_owner_pool,
        "setup owner pool closes within its deadline",
    )
    .await;

    let db = attach_existing_db(&admin_url, &database_name).await;
    let proxy = CommitAckLossProxy::start(&db, resolution).await;
    assert!(
        proxy.pool.options().get_max_connections() == 1,
        "supplied pool has one connection"
    );
    sqlx::query("SET statement_timeout='30s'")
        .execute(&proxy.pool)
        .await
        .expect("set supplied statement timeout");
    sqlx::query("SET idle_in_transaction_session_timeout='30s'")
        .execute(&proxy.pool)
        .await
        .expect("set supplied idle transaction timeout");

    let api = start_on_db_with_pool(db, runtime_options(), proxy.pool.clone()).await;
    let reopened = Platform {
        api,
        tenant,
        other_tenant,
    };
    let backend: (String, String, i32, String, bool, bool) = sqlx::query_as(
        "SELECT current_database()::text,current_user::text,pg_backend_pid(), \
                current_setting('application_name'),current_setting('statement_timeout')='30s', \
                current_setting('idle_in_transaction_session_timeout')='30s'",
    )
    .fetch_one(&reopened.api.api_pool)
    .await
    .expect("capture supplied scoped-report backend");
    assert!(
        backend.0 == database_name
            && backend.1 == RUNTIME_ROLE
            && backend.2 > 0
            && backend.3 == PROXY_APPLICATION
            && backend.4
            && backend.5,
        "reopened public API uses the exact bounded sagat_app proxy connection"
    );
    let reopened_before =
        durable_snapshot(&reopened.api.db.pool, tenant_uuid, run_uuid, WORKER).await;
    assert!(
        reopened_before == before,
        "reopen performs no durable execution or authority write"
    );
    let reopened_lease = lease_witness(&reopened.api.db.pool, tenant_uuid, run_uuid).await;
    assert!(
        reopened_lease == original_lease_witness,
        "reopen preserves the exact original lease row"
    );
    let live: bool = sqlx::query_scalar(
        "SELECT lease_expires_at>clock_timestamp() FROM steps \
         WHERE tenant_id=$1 AND run_id=$2 AND step_id=$3",
    )
    .bind(tenant_uuid)
    .bind(run_uuid)
    .bind(reopened_lease.step_id)
    .fetch_one(&reopened.api.db.pool)
    .await
    .expect("check current original lease deadline");
    assert!(live, "original lease remains live without expiry rewriting");
    let session_live: bool = sqlx::query_scalar(
        "SELECT expires_at>clock_timestamp() FROM worker_sessions WHERE tenant_id=$1 AND id=$2",
    )
    .bind(tenant_uuid)
    .bind(uuid::Uuid::parse_str(&worker_id).expect("original worker id"))
    .fetch_one(&reopened.api.db.pool)
    .await
    .expect("check original session deadline");
    assert!(
        session_live,
        "the original session remains live without expiry rewriting"
    );

    ReopenedRefusalFixture {
        _server,
        reopened,
        proxy,
        completion,
        worker_id,
        session,
        before,
        lease: reopened_lease,
        backend_pid: backend.2,
    }
}

// Reads committed state independently of any delivered acknowledgement.
async fn inspect_committed_refusal(
    reopened: &Platform,
    completion: &CompleteStepRequest,
    before: &Value,
    reopened_lease: &LeaseWitness,
) -> (RefusalState, ReceiptWitness, Value) {
    let tenant = reopened.tenant;
    let tenant_uuid = tenant.as_uuid();
    let run_uuid = uuid::Uuid::parse_str(&completion.run_id).expect("original run id");
    let report_scope = receipt_scope(completion);
    let candidate_digest = Sha256::digest(completion.output_json.as_bytes()).to_vec();
    let post: RefusalState = sqlx::query_as(
        r#"SELECT run.status AS run_status,run.stop_reason,run.stop_code,
          scope.lifecycle AS scope_lifecycle,member.lifecycle AS member_lifecycle,
          member.slot_held,step.status AS step_status,step.output_digest IS NULL AS output_absent,
          usage.logical_output_bytes,run.run_version,head.head_seq,
          (SELECT max(seq) FROM run_events WHERE tenant_id=run.tenant_id AND run_id=run.id) AS journal_head,
          (SELECT count(*) FROM iteration_checkpoints
           WHERE tenant_id=run.tenant_id AND run_id=run.id) AS checkpoints,
          (SELECT count(*) FROM steps WHERE tenant_id=run.tenant_id AND run_id=run.id
             AND instance_path_nodes=ARRAY['lines']::text[]
             AND instance_path_indices=ARRAY[16]::integer[] AND node='returned') AS returns,
          (SELECT count(*) FROM steps WHERE tenant_id=run.tenant_id AND run_id=run.id
             AND cardinality(instance_path_nodes)=0 AND node='done') AS terminal_steps,
          (SELECT count(*) FROM scope_sagas
           WHERE tenant_id=run.tenant_id AND run_id=run.id) AS sagas,
          (SELECT count(*) FROM run_events WHERE tenant_id=run.tenant_id AND run_id=run.id
             AND event_type='run.stop_requested' AND detail->>'reason'='FAILED'
             AND detail->>'code'='RUN_DATA_LIMIT') AS stops,
          (SELECT count(*) FROM run_events WHERE tenant_id=run.tenant_id AND run_id=run.id
             AND event_type='step.completed' AND node='reserve'
             AND instance_path_nodes=ARRAY['lines']::text[]
             AND instance_path_indices=ARRAY[16]::integer[]) AS successes,
          (SELECT min(seq) FROM run_events WHERE tenant_id=run.tenant_id AND run_id=run.id
             AND event_type='run.stop_requested' AND detail->>'code'='RUN_DATA_LIMIT') AS stop_seq,
          (SELECT min(seq) FROM run_events WHERE tenant_id=run.tenant_id AND run_id=run.id
             AND event_type='step.completed' AND node='reserve'
             AND instance_path_nodes=ARRAY['lines']::text[]
             AND instance_path_indices=ARRAY[16]::integer[]) AS success_seq,
          (SELECT count(*) FROM payloads
           WHERE tenant_id=run.tenant_id AND digest=$4) AS candidate_payloads,
          (SELECT count(*) FROM run_events WHERE tenant_id=run.tenant_id AND run_id=run.id
             AND payload_digest=$4) AS candidate_event_payloads,
          (SELECT count(*) FROM run_events WHERE tenant_id=run.tenant_id AND run_id=run.id
             AND event_type='step.completed' AND node='reserve'
             AND instance_path_nodes=ARRAY['lines']::text[]
             AND instance_path_indices=ARRAY[16]::integer[]
             AND payload_digest IS NOT NULL) AS completion_payloads
        FROM runs run JOIN iteration_scopes scope
          ON scope.tenant_id=run.tenant_id AND scope.run_id=run.id
        JOIN collection_iterations member
          ON member.tenant_id=run.tenant_id AND member.run_id=run.id
         AND member.scope_id=scope.scope_id AND member.iteration_index=16
        JOIN steps step ON step.tenant_id=run.tenant_id AND step.run_id=run.id
         AND step.step_id=$3
        JOIN run_instance_usage usage ON usage.tenant_id=run.tenant_id AND usage.run_id=run.id
        JOIN telemetry_pending head ON head.tenant_id=run.tenant_id AND head.run_id=run.id
        WHERE run.tenant_id=$1 AND run.id=$2"#,
    )
    .bind(tenant_uuid)
    .bind(run_uuid)
    .bind(reopened_lease.step_id)
    .bind(&candidate_digest)
    .fetch_one(&reopened.api.db.pool)
    .await
    .expect("read committed public refusal state");
    assert!(
        post.run_status == "CANCELLING"
            && post.stop_reason.as_deref() == Some("FAILED")
            && post.stop_code.as_deref() == Some("RUN_DATA_LIMIT")
            && post.scope_lifecycle == "OPEN"
            && post.member_lifecycle == "ADMITTED"
            && post.slot_held
            && post.step_status == "SUCCEEDED"
            && post.output_absent
            && post.logical_output_bytes == 4_000_000
            && post.run_version == post.head_seq
            && post.run_version == post.journal_head
            && post.checkpoints == 16
            && post.returns == 0
            && post.terminal_steps == 0
            && post.sagas == 0
            && post.stops == 1
            && post.successes == 1
            && post.candidate_payloads == 0
            && post.candidate_event_payloads == 0
            && post.completion_payloads == 0,
        "first report persists only the bounded successful-refusal state"
    );
    assert!(
        post.stop_seq.is_some() && post.success_seq.is_some() && post.stop_seq < post.success_seq,
        "the one RUN_DATA_LIMIT stop precedes the genuine scoped success"
    );
    let reported: (bool, i32, i64) = sqlx::query_as(
        "SELECT reported_lease_token_hash=$4,attempt,generation FROM steps \
         WHERE tenant_id=$1 AND run_id=$2 AND step_id=$3",
    )
    .bind(tenant_uuid)
    .bind(run_uuid)
    .bind(reopened_lease.step_id)
    .bind(Token::hash_of(&completion.lease_token))
    .fetch_one(&reopened.api.db.pool)
    .await
    .expect("read exact reported lease fence");
    assert!(
        reported == (true, completion.attempt, completion.generation),
        "successful metadata records the exact original reported fence"
    );

    let expected_hash = request_hash(completion);
    let original_receipt = receipt(
        &reopened.api.db.pool,
        tenant_uuid,
        &report_scope,
        &completion.idempotency_key,
    )
    .await;
    assert!(
        original_receipt.tenant_id == tenant.to_string()
            && original_receipt.scope == report_scope
            && original_receipt.idempotency_key == completion.idempotency_key
            && original_receipt.request_hash == expected_hash
            && original_receipt.resource_id == format!("SUCCEEDED|CANCELLING|{}", post.run_version),
        "receipt is the canonical full original public command identity"
    );
    let audit: (i64, bool, bool, bool) = sqlx::query_as(
        "SELECT count(*),bool_and(actor='system:engine'), \
                bool_and(reason='successful scoped output refused by run data limit'), \
                bool_and(audit.trace_id IS NOT DISTINCT FROM $3::text) \
         FROM audit_records audit JOIN runs run \
           ON run.tenant_id=audit.tenant_id AND run.id=$2 \
         WHERE audit.tenant_id=$1 AND audit.resource_type='run' \
           AND audit.resource_id=$2::text AND audit.action='run.data_limit_exceeded'",
    )
    .bind(tenant_uuid)
    .bind(run_uuid)
    .bind(before["run"]["trace_id"].as_str())
    .fetch_one(&reopened.api.db.pool)
    .await
    .expect("read exact refusal audit");
    assert!(
        audit == (1, true, true, true),
        "one truthful refusal audit carries the run trace"
    );

    let after_first = durable_snapshot(&reopened.api.db.pool, tenant_uuid, run_uuid, WORKER).await;
    assert!(
        after_first["run"]["trace_id"] == before["run"]["trace_id"],
        "committed refusal preserves the original Run trace"
    );
    for key in [
        "retainedSteps",
        "payloads",
        "retainedUsage",
        "iterationUsage",
        "iterations",
        "chains",
        "sagaEntries",
        "capacityGroups",
        "capacityMembers",
        "capacityReceipts",
        "quotaDeltas",
    ] {
        assert!(
            before[key] == after_first[key],
            "report preserves retained outputs, payload bytes, reservations and source metadata"
        );
    }
    assert!(
        before["items"] == after_first["items"],
        "collection source identities stay unchanged"
    );
    assert!(
        before["checkpoints"] == after_first["checkpoints"],
        "sixteen source checkpoints stay unchanged"
    );
    assert!(
        before["scope"] == after_first["scope"],
        "report does not drain or close the scope"
    );
    assert!(
        before["authority"] == after_first["authority"],
        "report preserves durable worker authority"
    );
    assert!(
        before["version"] == after_first["version"],
        "report preserves the immutable pinned version"
    );
    assert!(
        before["outbox"] == after_first["outbox"],
        "report appends exactly zero outbox rows"
    );
    assert!(
        after_first["blobs"].as_array().is_some_and(Vec::is_empty),
        "candidate creates no blob row"
    );
    assert!(
        after_first["sagas"].as_array().is_some_and(Vec::is_empty),
        "candidate creates no scope saga"
    );

    (post, original_receipt, after_first)
}

async fn ordinary_public_scoped_refusal_scenario() {
    let ReopenedRefusalFixture {
        _server,
        reopened,
        mut proxy,
        completion,
        worker_id,
        session,
        before,
        lease: reopened_lease,
        backend_pid,
    } = reopened_refusal_fixture(CommitResolution::Committed).await;
    let database_name = reopened.api.db.name.clone();
    let tenant = reopened.tenant;
    let tenant_uuid = tenant.as_uuid();
    let run = completion.run_id.clone();
    let run_uuid = uuid::Uuid::parse_str(&run).expect("original run id");
    let run_id = RunId::new(run_uuid);

    let lookups_before = reopened.api.authentication_lookups();
    let mut resumed = reopened.connect(WORKER, TENANT).await;
    resumed.worker_id = worker_id;
    resumed.session = session;
    let report_scope = receipt_scope(&completion);
    let advisory_key = positive_advisory_key();
    install_exact_receipt_gate(
        &reopened.api.db.pool,
        tenant_uuid,
        &report_scope,
        &completion.idempotency_key,
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
        .expect("exact receipt gate backend");
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(advisory_key)
        .execute(&mut *gate)
        .await
        .expect("hold exact receipt gate");
    let first_request = resumed.req(completion.clone());
    let mut report_client = resumed.client.clone();
    let mut reporting = PendingReport::new(tokio::spawn(async move {
        report_client.complete_step(first_request).await
    }));
    let observed = wait_for_exact_public_writer(
        &reopened.api.db.pool,
        backend_pid,
        gate_pid,
        &database_name,
        &mut reporting,
        &mut proxy,
    )
    .await;
    assert!(
        observed == backend_pid,
        "exact supplied backend executes the public refusal receipt insert"
    );
    gate.rollback().await.expect("release exact receipt gate");
    let first = match timeout(Duration::from_secs(5), reporting.join()).await {
        Ok(Ok(Ok(response))) => response.into_inner(),
        _ => panic!(
            "original public scoped completion returns an acknowledgement within its deadline"
        ),
    };
    remove_exact_receipt_gate(&reopened.api.db.pool).await;
    assert!(
        !first.already_applied
            && first.step_status == "SUCCEEDED"
            && first.run_status == "CANCELLING",
        "first original report is the genuine successful-output refusal"
    );
    assert!(
        reopened.api.authentication_lookups() > lookups_before,
        "reopened API performs current authentication for the report"
    );

    let (post, original_receipt, after_first) =
        inspect_committed_refusal(&reopened, &completion, &before, &reopened_lease).await;
    assert!(
        first.command_id == format!("{run}:{}", post.run_version),
        "acknowledgement names the actual committed head"
    );

    let replay_lookups_before = reopened.api.authentication_lookups();
    let replay_request = resumed.req(completion.clone());
    let replay = resumed.client.complete_step(replay_request).await;
    let replay = match replay {
        Ok(response) => response.into_inner(),
        Err(_) => panic!("exact original public command replays"),
    };
    assert!(
        replay.already_applied
            && replay.command_id == first.command_id
            && replay.step_status == first.step_status
            && replay.run_status == first.run_status,
        "exact original report replays the original acknowledgement"
    );
    assert!(
        reopened.api.authentication_lookups() > replay_lookups_before,
        "replay also performs current authentication"
    );
    let replay_receipt = receipt(
        &reopened.api.db.pool,
        tenant_uuid,
        &report_scope,
        &completion.idempotency_key,
    )
    .await;
    assert!(
        replay_receipt == original_receipt,
        "replay preserves the complete original receipt row"
    );
    let after_replay = durable_snapshot(&reopened.api.db.pool, tenant_uuid, run_uuid, WORKER).await;
    assert!(
        after_replay == after_first,
        "replay leaves execution, payload, quota, audit, outbox, source, and authority bytes unchanged"
    );

    let cold_store = PgRunStore::new(
        reopened.api.db.pool.clone(),
        DefinitionCache::new(Arc::new(sagat_infra_ir::JsonIrCodec), 8),
    )
    .with_scoped_worker_protocol();
    assert!(
        cold_store.verify_run(tenant, run_id).await.is_ok(),
        "fresh direct store verifies the full committed refusal journal"
    );
    let path = InstancePath::root()
        .child(NodeId::parse("lines").expect("owner node"), 16)
        .expect("bounded original path");
    assert!(
        matches!(
            cold_store.load_path(tenant, run_id, path).await,
            Ok(Some(_))
        ),
        "fresh direct store reads the exact committed seventeenth path"
    );

    drop(cold_store);
    drop(resumed);
    let runtime_pool = reopened.api.api_pool.clone();
    let owner_pool = reopened.api.db.pool.clone();
    drop(reopened);
    close_pool(
        runtime_pool,
        "reopened runtime pool closes within its deadline",
    )
    .await;
    close_pool(owner_pool, "reopened owner pool closes within its deadline").await;
    drop(proxy);
}
