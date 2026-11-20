//! Feature 022: a world whose store runs the readiness gate of flow control (T022-096), shared by the suites that
//! exercise or measure it.

use std::sync::Arc;

use sagat_application::definitions::DefinitionService;
use sagat_application::flow_control::FlowControlSwitches;
use sagat_application::instances::{Activation, Capability, InstanceKind, InstanceStore};
use sagat_application::keyed_digest::KeyRing;
use sagat_domain::definition::WorkflowId;
use sagat_domain::identity::Subject;
use sagat_infra_ir::JsonIrCodec;
use sagat_infra_postgres::{PgInstanceStore, PgRunStore};
use serde_json::Value;
use uuid::Uuid;

use super::engine::{World, ctx};
use super::instances::{nothing, registration};

const BOTH_ON: FlowControlSwitches = FlowControlSwitches {
    concurrency_keys: true,
    rate_limits: true,
};
const KINDS: [InstanceKind; 2] = [InstanceKind::ControlPlane, InstanceKind::ExecutionEngine];

/// A world whose store runs the readiness gate: both switches on, a digest key ring whose version 1 is active, both
/// capabilities activated for a fleet of processes that list them, and step gates admitted (tests only).
pub async fn gated_world() -> World {
    gated_world_with_keys(RING).await
}

/// The digest key ring of [`gated_world`]'s processes: version 1.
pub const RING: &str = "1:5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a";

/// A ring of versions 1 and 2, for a rotation: version 1 is [`RING`]'s.
pub const RING_TWO: &str = "1:5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a,\
                            2:6b6b6b6b6b6b6b6b6b6b6b6b6b6b6b6b6b6b6b6b6b6b6b6b6b6b6b6b6b6b6b6b";

/// The switches of a process with flow control `on`, or of one without it.
pub const fn switches(on: bool) -> FlowControlSwitches {
    if on {
        BOTH_ON
    } else {
        FlowControlSwitches {
            concurrency_keys: false,
            rate_limits: false,
        }
    }
}

/// `store` running the gates: both switches on, the key ring [`RING`], step gates admitted (tests only).
pub fn gate(store: PgRunStore) -> PgRunStore {
    gate_with_keys(store, RING)
}

/// `store` running the gates with the digest key ring `ring` (`version:hex,...`).
pub fn gate_with_keys(store: PgRunStore, ring: &str) -> PgRunStore {
    store
        .with_flow_control(BOTH_ON)
        .with_flow_control_keys(Arc::new(KeyRing::parse(ring).expect("a key ring")))
        .serving_rate_gates_for_tests()
}

/// [`gated_world`], whose processes hold the digest key ring `ring` (`version:hex,...`); version 1 is activated.
pub async fn gated_world_with_keys(ring: &str) -> World {
    let ring = ring.to_owned();
    let w = super::engine::world_with_store(
        sagat_application::worker::WorkerSettings::default(),
        sagat_application::execution::BreakerPolicy::default(),
        move |store: PgRunStore| gate_with_keys(store, &ring),
    )
    .await;
    activate(&w.db.pool).await;
    w
}

/// Activates digest key version 1 and both capabilities for a fleet of processes that list them, on the database of
/// `pool` (its owner).
pub async fn activate(pool: &sqlx::PgPool) {
    activate_capabilities(
        pool,
        &[
            Capability::FlowControlConcurrencyV1,
            Capability::FlowControlRateV1,
        ],
    )
    .await;
}

/// [`activate`] for the concurrency gate only: what a fleet of this build's real processes can activate (D022-057: none
/// lists the rate gate), so that a control plane or an engine started for real afterwards is not refused at its
/// registration for lacking `flow-control-rate-v1`.
pub async fn activate_concurrency(pool: &sqlx::PgPool) {
    activate_capabilities(pool, &[Capability::FlowControlConcurrencyV1]).await;
}

async fn activate_capabilities(pool: &sqlx::PgPool, capabilities: &[Capability]) {
    let activated: (String, i64) = sqlx::query_as(
        "SELECT out_outcome, out_rows FROM flow_control_key_activate(1::smallint, 'operator')",
    )
    .fetch_one(pool)
    .await
    .expect("the digest key version is activated");
    assert_eq!(activated.0, "ACTIVATED", "{activated:?}");
    let registry = PgInstanceStore::new(pool.clone());
    let actor = Subject::parse("operator:flow-control").expect("subject");
    for kind in KINDS {
        registry
            .register(&registration(Uuid::new_v4(), kind, capabilities))
            .await
            .expect("a process that runs the gates");
    }
    for capability in capabilities {
        registry
            .activate(
                Activation {
                    capability: *capability,
                    kinds: &KINDS,
                    actor: &actor,
                },
                nothing,
            )
            .await
            .expect("activated");
    }
}

/// The environment of a real control plane or `execution-engine` (`EngineConfig::from_lookup`) that serves the
/// concurrency gate with [`RING`] (D022-057): the switch on and the digest key file, which lives as long as the test
/// binary. Such a process lists `flow-control-concurrency-v1`; pair it with [`activate_concurrency`].
///
/// # Panics
/// When the file cannot be written.
pub fn process_env() -> [(&'static str, String); 2] {
    static FILE: std::sync::OnceLock<tempfile::NamedTempFile> = std::sync::OnceLock::new();
    let file = FILE.get_or_init(|| {
        use std::io::Write as _;
        let mut file = tempfile::NamedTempFile::new().expect("a key file");
        file.write_all(RING.as_bytes()).expect("the ring written");
        file
    });
    [
        (
            "SAGAT_FLOW_CONTROL_CONCURRENCY_KEYS_ENABLED",
            "true".to_owned(),
        ),
        (
            "SAGAT_FLOW_CONTROL_DIGEST_KEYS_FILE",
            file.path().to_str().expect("a path").to_owned(),
        ),
    ]
}

/// Publishes `doc` with the switches on (a control plane of the fleet).
pub async fn publish(w: &World, doc: Value) -> WorkflowId {
    publish_as(w, &w.admin, doc).await
}

/// [`publish`], by `principal` in its tenant's `payments` namespace.
pub async fn publish_as(
    w: &World,
    principal: &sagat_domain::identity::Principal,
    doc: Value,
) -> WorkflowId {
    try_publish_as(w, principal, doc).await.expect("published")
}

/// [`publish_as`], or why the definition service refused the document.
pub async fn try_publish_as(
    w: &World,
    principal: &sagat_domain::identity::Principal,
    doc: Value,
) -> Result<WorkflowId, sagat_application::error::AppError> {
    let defs = DefinitionService::new(
        w.store.clone(),
        sagat_application::clock::SystemClock,
        JsonIrCodec,
    )
    .with_flow_control(BOTH_ON);
    let name = doc["metadata"]["name"].as_str().expect("name").to_owned();
    let workflow = defs
        .create_workflow(
            principal,
            "payments",
            &name,
            &serde_json::to_vec(&doc).expect("json"),
            &format!("create-{}", Uuid::now_v7()),
            &ctx(),
        )
        .await?
        .workflow
        .id();
    defs.publish(
        principal,
        workflow,
        1,
        None,
        &format!("publish-{}", Uuid::now_v7()),
        &ctx(),
    )
    .await?;
    Ok(workflow)
}

/// Moves the queue deadline of the waiting step `node` of `run` one second into the past, in its waiter rows and in
/// its gates' projection, as the time it waited had passed (the journal keeps the deadline it was queued with; the
/// projection agrees with it again once the wait has ended).
pub async fn age_gate_deadline(w: &World, run: sagat_domain::execution::RunId, node: &str) {
    set_gate_deadline(&w.db.pool, run, node, -1_000_000).await;
}

/// Sets the queue deadline of the waiting step `node` of `run` to `offset_us` microseconds from the database clock now,
/// in its waiter rows and its gates' projection, with one instant; returns that deadline in microseconds since the
/// epoch. A waiter row is never changed (its trigger refuses an update): it is replaced, in one transaction, by the same
/// row with that deadline (an empty key is pruned only at commit, and no count moves).
pub async fn set_gate_deadline(
    pool: &sqlx::PgPool,
    run: sagat_domain::execution::RunId,
    node: &str,
    offset_us: i64,
) -> i64 {
    let mut tx = pool.begin().await.expect("tx");
    let deadline: i64 =
        sqlx::query_scalar("SELECT (extract(epoch FROM clock_timestamp()) * 1000000)::bigint + $1")
            .bind(offset_us)
            .fetch_one(&mut *tx)
            .await
            .expect("clock");
    for statement in [
        "WITH gone AS (DELETE FROM concurrency_waiters WHERE run_id = $1 AND node = $2 RETURNING *) \
         INSERT INTO concurrency_waiters (tenant_id, namespace_id, run_id, node, permit_scope, attempt, pool, \
                group_name, key_digest, declared_limit, enqueued_at, deadline_at, shard) \
         SELECT tenant_id, namespace_id, run_id, node, permit_scope, attempt, pool, group_name, key_digest, \
                declared_limit, clock_timestamp() - interval '1 hour', timestamptz 'epoch' + $3 * interval '1 microsecond', \
                shard \
           FROM gone",
        "WITH gone AS (DELETE FROM rate_limit_waiters WHERE run_id = $1 AND node = $2 RETURNING *) \
         INSERT INTO rate_limit_waiters (tenant_id, namespace_id, run_id, node, attempt, pool, limit_scope, \
                group_name, key_digest, declared_limit, period_ms, enqueued_at, deadline_at, shard) \
         SELECT tenant_id, namespace_id, run_id, node, attempt, pool, limit_scope, group_name, key_digest, \
                declared_limit, period_ms, clock_timestamp() - interval '1 hour', \
                timestamptz 'epoch' + $3 * interval '1 microsecond', shard \
           FROM gone",
    ] {
        sqlx::query(statement)
            .bind(run.as_uuid())
            .bind(node)
            .bind(deadline)
            .execute(&mut *tx)
            .await
            .expect("waiter deadline set");
    }
    sqlx::query(
        "UPDATE steps SET flow_control_gates = ( \
             SELECT jsonb_object_agg(k, CASE WHEN v = 'null'::jsonb OR v->>'phase' <> 'QUEUED' THEN v \
                 ELSE jsonb_set(v, '{deadlineUs}', to_jsonb($3::bigint)) END) \
               FROM jsonb_each(flow_control_gates) AS e(k, v)) \
          WHERE run_id = $1 AND node = $2 AND flow_control_gates IS NOT NULL",
    )
    .bind(run.as_uuid())
    .bind(node)
    .bind(deadline)
    .execute(&mut *tx)
    .await
    .expect("projection deadline set");
    tx.commit().await.expect("deadline set");
    deadline
}

/// Sets the queue deadline of the queued start of `run` to `offset_us` microseconds from the database clock now, in
/// its waiter row and its run's gates' projection, with one instant; returns it in microseconds since the epoch.
pub async fn set_start_deadline(
    pool: &sqlx::PgPool,
    run: sagat_domain::execution::RunId,
    offset_us: i64,
) -> i64 {
    let mut tx = pool.begin().await.expect("tx");
    let deadline: i64 =
        sqlx::query_scalar("SELECT (extract(epoch FROM clock_timestamp()) * 1000000)::bigint + $1")
            .bind(offset_us)
            .fetch_one(&mut *tx)
            .await
            .expect("clock");
    sqlx::query(
        "WITH gone AS (DELETE FROM concurrency_waiters WHERE run_id = $1 AND permit_scope = 'RUN' RETURNING *) \
         INSERT INTO concurrency_waiters (tenant_id, namespace_id, run_id, node, permit_scope, attempt, pool, \
                group_name, key_digest, declared_limit, enqueued_at, deadline_at, shard) \
         SELECT tenant_id, namespace_id, run_id, node, permit_scope, attempt, pool, group_name, key_digest, \
                declared_limit, clock_timestamp() - interval '1 hour', timestamptz 'epoch' + $2 * interval '1 microsecond', \
                shard FROM gone",
    )
    .bind(run.as_uuid())
    .bind(deadline)
    .execute(&mut *tx)
    .await
    .expect("waiter deadline set");
    sqlx::query(
        "UPDATE runs SET flow_control_gates = jsonb_set(flow_control_gates, '{concurrency,deadlineUs}', to_jsonb($2)) \
          WHERE id = $1 AND flow_control_gates -> 'concurrency' ->> 'phase' = 'QUEUED'",
    )
    .bind(run.as_uuid())
    .bind(deadline)
    .execute(&mut *tx)
    .await
    .expect("projection deadline set");
    tx.commit().await.expect("deadline set");
    deadline
}

/// Moves the queued start of `run` one second past its deadline (see [`set_start_deadline`]).
pub async fn age_start_deadline(w: &World, run: sagat_domain::execution::RunId) {
    set_start_deadline(&w.db.pool, run, -1_000_000).await;
}

/// Returns once the database clock is past `deadline_us` (microseconds since the epoch): a bounded condition on the
/// clock the gates decide with, not a sleep standing in for an order. Fails by name after two minutes.
pub async fn until_past(pool: &sqlx::PgPool, deadline_us: i64) {
    super::bounded::within(
        "the database clock passes the deadline".to_owned(),
        std::time::Duration::from_secs(120),
        async {
            loop {
                let past: bool = sqlx::query_scalar(
                    "SELECT (extract(epoch FROM clock_timestamp()) * 1000000)::bigint > $1",
                )
                .bind(deadline_us)
                .fetch_one(pool)
                .await
                .expect("clock");
                if past {
                    return;
                }
                tokio::task::yield_now().await;
            }
        },
    )
    .await;
}

/// Microseconds since the epoch on the database clock now.
pub async fn database_now_us(pool: &sqlx::PgPool) -> i64 {
    sqlx::query_scalar("SELECT (extract(epoch FROM clock_timestamp()) * 1000000)::bigint")
        .fetch_one(pool)
        .await
        .expect("clock")
}

/// Invariants 1 and 2 of `data-model.md` across the runs of `tenant` (T022-027; FR-022-020): in one snapshot, every
/// semaphore's counters equal its holder and waiter rows; every holder row is a permit its run's or step's gates hold
/// (`HELD`, same attempt, pool, group and label) and every waiter row one they wait for (`QUEUED`, same attempt and
/// deadline); and every permit the gates hold or wait for has its row. The gates' projections are a replay of the
/// journal (what `verify_run` checks), so this ties the counters, the rows and the open `concurrency.acquired` and
/// `concurrency.queued` events (restored ones excluded by the replay) together. Returns the runs the rows name, for the
/// caller to verify their journals. Also invariant 12 (no waiter of a terminal run), FR-022-020 (no empty key row) and
/// D022-058 (a key's oldest wait is its waiters'). A schema without the flow-control tables has nothing to check; one
/// with them whose columns the checks cannot read is an error.
pub async fn key_consistency(
    pool: &sqlx::PgPool,
    tenant: sagat_domain::identity::TenantId,
) -> Result<Vec<sagat_domain::execution::RunId>, String> {
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    // What the schema has is decided once, by name (keys review 2026-11-20, architecture M2): a schema rolled back
    // before the gates has nothing to check, and a projection or column the checks name that is missing from a schema
    // that has the gates is a failure, never a silent pass.
    let (tables, run_gates, step_gates, oldest_wait): (bool, bool, bool, bool) = sqlx::query_as(
        "SELECT to_regclass('public.concurrency_semaphores') IS NOT NULL, \
                EXISTS (SELECT 1 FROM pg_attribute WHERE attrelid = 'public.runs'::regclass \
                         AND attname = 'flow_control_gates' AND NOT attisdropped), \
                EXISTS (SELECT 1 FROM pg_attribute WHERE attrelid = 'public.steps'::regclass \
                         AND attname = 'flow_control_gates' AND NOT attisdropped), \
                to_regclass('public.concurrency_semaphores') IS NOT NULL \
                AND EXISTS (SELECT 1 FROM pg_attribute WHERE attrelid = to_regclass('public.concurrency_semaphores') \
                             AND attname = 'oldest_wait_at' AND NOT attisdropped)",
    )
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| e.to_string())?;
    // which of them the schema must have is decided by the migrations applied, never by the columns found (review of
    // the fixes, correctness m9): a column a migration added that is gone is a failure
    let applied: i64 =
        sqlx::query_scalar("SELECT COALESCE(max(version), 0) FROM _sqlx_migrations WHERE success")
            .fetch_one(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
    let must = |since: i64, present: bool, what: &str| -> Result<(), String> {
        if applied >= since && !present {
            Err(format!(
                "feature 022's consistency check: {what} is missing from a schema at {applied}"
            ))
        } else {
            Ok(())
        }
    };
    must(20_270_130_000_000, tables, "concurrency_semaphores")?;
    must(20_270_130_000_600, step_gates, "steps.flow_control_gates")?;
    must(20_270_203_000_000, run_gates, "runs.flow_control_gates")?;
    must(
        20_270_205_000_000,
        oldest_wait,
        "concurrency_semaphores.oldest_wait_at",
    )?;
    if !tables {
        return Ok(Vec::new());
    }
    // a run's own gates exist from `flow_control_run_gates` on; a schema between the two has only steps' gates.
    // Steps inside an instance path (a loop body, a collection item) are not filed under gates in this slice (G1160):
    // their permits are not compared here.
    let gates: String = format!(
        "{}SELECT s.tenant_id, s.run_id, s.node, 'STEP' AS scope, s.flow_control_gates -> 'concurrency' AS g \
           FROM steps s WHERE s.tenant_id = $1 AND s.flow_control_gates IS NOT NULL \
            AND cardinality(s.instance_path_nodes) = 0",
        if run_gates {
            "SELECT r.tenant_id, r.id AS run_id, '' AS node, 'RUN' AS scope, r.flow_control_gates -> 'concurrency' AS g \
               FROM runs r WHERE r.tenant_id = $1 AND r.flow_control_gates IS NOT NULL \
             UNION ALL "
        } else {
            ""
        }
    );
    let gates = gates.as_str();
    let mut checks: Vec<(&str, String)> = vec![
        (
            "a key's counters differ from its rows",
            "SELECT s.group_name || ':' || encode(substr(s.key_digest, 1, 6), 'hex') || ' holders ' || s.holders \
                    || '/' || (SELECT count(*) FROM concurrency_holders h WHERE h.tenant_id = s.tenant_id \
                        AND h.namespace_id = s.namespace_id AND h.pool = s.pool AND h.holder_scope = s.permit_scope \
                        AND h.group_name = s.group_name AND h.key_digest = s.key_digest) \
                    || ' waiters ' || s.waiters || '/' || (SELECT count(*) FROM concurrency_waiters w \
                      WHERE w.tenant_id = s.tenant_id AND w.namespace_id = s.namespace_id AND w.pool = s.pool \
                        AND w.permit_scope = s.permit_scope AND w.group_name = s.group_name \
                        AND w.key_digest = s.key_digest) \
               FROM concurrency_semaphores s WHERE s.tenant_id = $1 \
                AND (s.holders <> (SELECT count(*) FROM concurrency_holders h WHERE h.tenant_id = s.tenant_id \
                        AND h.namespace_id = s.namespace_id AND h.pool = s.pool AND h.holder_scope = s.permit_scope \
                        AND h.group_name = s.group_name AND h.key_digest = s.key_digest) \
                  OR s.waiters <> (SELECT count(*) FROM concurrency_waiters w WHERE w.tenant_id = s.tenant_id \
                        AND w.namespace_id = s.namespace_id AND w.pool = s.pool AND w.permit_scope = s.permit_scope \
                        AND w.group_name = s.group_name AND w.key_digest = s.key_digest))"
                .to_owned(),
        ),
        (
            // FR-022-020: the transaction that empties a key deletes it
            "an empty key row",
            "SELECT s.group_name || ':' || encode(substr(s.key_digest, 1, 6), 'hex') FROM concurrency_semaphores s \
              WHERE s.tenant_id = $1 AND s.holders = 0 AND s.waiters = 0"
                .to_owned(),
        ),
        (
            // invariant 12, the rate-limit waiters (review of the fixes, correctness m8)
            "a rate-limit waiter row of a terminal run",
            "SELECT w.run_id::text || '/' || w.node || ' ' || r.status FROM rate_limit_waiters w \
               JOIN runs r ON r.tenant_id = w.tenant_id AND r.id = w.run_id \
              WHERE w.tenant_id = $1 AND r.status IN ('SUCCEEDED', 'FAILED', 'CANCELLED', 'TIMED_OUT', 'COMPENSATED', \
                                                      'PARTIAL', 'COMPENSATION_FAILED')"
                .to_owned(),
        ),
        (
            // invariant 12: every waiter row belongs to a run that is not terminal
            "a waiter row of a terminal run",
            "SELECT w.run_id::text || '/' || w.node || ' ' || r.status FROM concurrency_waiters w \
               JOIN runs r ON r.tenant_id = w.tenant_id AND r.id = w.run_id \
              WHERE w.tenant_id = $1 AND r.status IN ('SUCCEEDED', 'FAILED', 'CANCELLED', 'TIMED_OUT', 'COMPENSATED', \
                                                      'PARTIAL', 'COMPENSATION_FAILED')"
                .to_owned(),
        ),
        (
            "a holder row whose gates hold no such permit",
            format!(
                "SELECT h.run_id::text || '/' || h.node FROM concurrency_holders h WHERE h.tenant_id = $1 \
                   AND NOT EXISTS (SELECT 1 FROM ({gates}) p WHERE p.run_id = h.run_id AND p.node = h.node \
                       AND p.scope = h.holder_scope AND p.g ->> 'phase' = 'HELD' \
                       AND (p.g ->> 'attempt')::int = h.attempt AND p.g ->> 'pool' = h.pool \
                       AND p.g ->> 'group' = h.group_name AND p.g ->> 'key' = encode(substr(h.key_digest, 1, 6), 'hex'))"
            ),
        ),
        (
            "a waiter row whose gates wait for no such permit",
            format!(
                "SELECT w.run_id::text || '/' || w.node FROM concurrency_waiters w WHERE w.tenant_id = $1 \
                   AND NOT EXISTS (SELECT 1 FROM ({gates}) p WHERE p.run_id = w.run_id AND p.node = w.node \
                       AND p.scope = w.permit_scope AND p.g ->> 'phase' = 'QUEUED' \
                       AND (p.g ->> 'attempt')::int = w.attempt AND p.g ->> 'pool' = w.pool \
                       AND p.g ->> 'group' = w.group_name AND p.g ->> 'key' = encode(substr(w.key_digest, 1, 6), 'hex') \
                       AND (p.g ->> 'deadlineUs')::bigint = (extract(epoch FROM w.deadline_at) * 1000000)::bigint)"
            ),
        ),
        (
            "a permit the gates hold without its holder row",
            format!(
                "SELECT p.run_id::text || '/' || p.node FROM ({gates}) p WHERE p.g ->> 'phase' = 'HELD' \
                   AND NOT EXISTS (SELECT 1 FROM concurrency_holders h WHERE h.tenant_id = p.tenant_id \
                       AND h.run_id = p.run_id AND h.node = p.node AND h.holder_scope = p.scope)"
            ),
        ),
        (
            "a permit the gates wait for without its waiter row",
            format!(
                "SELECT p.run_id::text || '/' || p.node FROM ({gates}) p WHERE p.g ->> 'phase' = 'QUEUED' \
                   AND NOT EXISTS (SELECT 1 FROM concurrency_waiters w WHERE w.tenant_id = p.tenant_id \
                       AND w.run_id = p.run_id AND w.node = p.node AND w.permit_scope = p.scope)"
            ),
        ),
    ];
    if oldest_wait {
        checks.push((
            // D022-058: the oldest wait the listing orders by is its waiters' oldest
            "a key's oldest wait differs from its waiters'",
            "SELECT s.group_name || ':' || encode(substr(s.key_digest, 1, 6), 'hex') FROM concurrency_semaphores s \
              WHERE s.tenant_id = $1 AND s.oldest_wait_at IS DISTINCT FROM (SELECT min(w.enqueued_at) \
                    FROM concurrency_waiters w WHERE w.tenant_id = s.tenant_id AND w.namespace_id = s.namespace_id \
                     AND w.pool = s.pool AND w.permit_scope = s.permit_scope AND w.group_name = s.group_name \
                     AND w.key_digest = s.key_digest)"
                .to_owned(),
        ));
    }
    for (what, statement) in &checks {
        // built from constants only, no value of a request or a row; any error (a renamed column included) fails
        let found: Vec<String> = sqlx::query_scalar(sqlx::AssertSqlSafe(statement.clone()))
            .bind(tenant.as_uuid())
            .fetch_all(&mut *tx)
            .await
            .map_err(|e| {
                format!("feature 022's consistency check \"{what}\" could not run: {e}")
            })?;
        if !found.is_empty() {
            return Err(format!(
                "invariants 1 and 2 of feature 022: {what}: {found:?}"
            ));
        }
    }
    let runs: Vec<Uuid> = sqlx::query_scalar(
        "SELECT run_id FROM concurrency_holders WHERE tenant_id = $1 \
         UNION SELECT run_id FROM concurrency_waiters WHERE tenant_id = $1",
    )
    .bind(tenant.as_uuid())
    .fetch_all(&mut *tx)
    .await
    .map_err(|e| e.to_string())?;
    Ok(runs
        .into_iter()
        .map(sagat_domain::execution::RunId::new)
        .collect())
}
