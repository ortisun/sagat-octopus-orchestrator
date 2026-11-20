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

fn gate_with_keys(store: PgRunStore, ring: &str) -> PgRunStore {
    store
        .with_flow_control(BOTH_ON)
        .with_flow_control_keys(Arc::new(KeyRing::parse(ring).expect("a key ring")))
        .admitting_step_gates_for_tests()
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
    let activated: (String, i64) = sqlx::query_as(
        "SELECT out_outcome, out_rows FROM flow_control_key_activate(1::smallint, 'operator')",
    )
    .fetch_one(pool)
    .await
    .expect("the digest key version is activated");
    assert_eq!(activated.0, "ACTIVATED", "{activated:?}");
    let registry = PgInstanceStore::new(pool.clone());
    let actor = Subject::parse("operator:flow-control").expect("subject");
    let both = [
        Capability::FlowControlConcurrencyV1,
        Capability::FlowControlRateV1,
    ];
    for kind in KINDS {
        registry
            .register(&registration(Uuid::new_v4(), kind, &both))
            .await
            .expect("a process that runs both gates");
    }
    for capability in both {
        registry
            .activate(
                Activation {
                    capability,
                    kinds: &KINDS,
                    actor: &actor,
                },
                nothing,
            )
            .await
            .expect("activated");
    }
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
    // a waiter row is never changed (its trigger refuses an update): the test replaces it, in one transaction, by the
    // same row with an earlier deadline (an empty key is pruned only at commit, and no count moves)
    let mut tx = w.db.pool.begin().await.expect("tx");
    for statement in [
        "WITH gone AS (DELETE FROM concurrency_waiters WHERE run_id = $1 AND node = $2 RETURNING *) \
         INSERT INTO concurrency_waiters (tenant_id, namespace_id, run_id, node, permit_scope, attempt, pool, \
                group_name, key_digest, declared_limit, enqueued_at, deadline_at, shard) \
         SELECT tenant_id, namespace_id, run_id, node, permit_scope, attempt, pool, group_name, key_digest, \
                declared_limit, clock_timestamp() - interval '1 hour', clock_timestamp() - interval '1 second', shard \
           FROM gone",
        "WITH gone AS (DELETE FROM rate_limit_waiters WHERE run_id = $1 AND node = $2 RETURNING *) \
         INSERT INTO rate_limit_waiters (tenant_id, namespace_id, run_id, node, attempt, pool, limit_scope, \
                group_name, key_digest, declared_limit, period_ms, enqueued_at, deadline_at, shard) \
         SELECT tenant_id, namespace_id, run_id, node, attempt, pool, limit_scope, group_name, key_digest, \
                declared_limit, period_ms, clock_timestamp() - interval '1 hour', \
                clock_timestamp() - interval '1 second', shard \
           FROM gone",
    ] {
        sqlx::query(statement)
            .bind(run.as_uuid())
            .bind(node)
            .execute(&mut *tx)
            .await
            .expect("waiter aged");
    }
    tx.commit().await.expect("aged");
    sqlx::query(
        "UPDATE steps SET flow_control_gates = ( \
             SELECT jsonb_object_agg(k, CASE WHEN v = 'null'::jsonb OR v->>'phase' <> 'QUEUED' THEN v \
                 ELSE jsonb_set(v, '{deadlineUs}', \
                      to_jsonb((extract(epoch FROM clock_timestamp()) * 1000000)::bigint - 1000000)) END) \
               FROM jsonb_each(flow_control_gates) AS e(k, v)) \
          WHERE run_id = $1 AND node = $2 AND flow_control_gates IS NOT NULL",
    )
    .bind(run.as_uuid())
    .bind(node)
    .execute(&w.db.pool)
    .await
    .expect("projection aged");
}

/// Invariants 1 and 2 of `data-model.md` across the runs of `tenant` (T022-027; FR-022-020): in one snapshot, every
/// semaphore's counters equal its holder and waiter rows; every holder row is a permit its run's or step's gates hold
/// (`HELD`, same attempt, pool, group and label) and every waiter row one they wait for (`QUEUED`, same attempt and
/// deadline); and every permit the gates hold or wait for has its row. The gates' projections are a replay of the
/// journal (what `verify_run` checks), so this ties the counters, the rows and the open `concurrency.acquired` and
/// `concurrency.queued` events (restored ones excluded by the replay) together. Returns the runs the rows name, for the
/// caller to verify their journals; a schema without the flow-control tables or projections has nothing to check.
pub async fn key_consistency(
    pool: &sqlx::PgPool,
    tenant: sagat_domain::identity::TenantId,
) -> Result<Vec<sagat_domain::execution::RunId>, String> {
    const GATES: &str = "SELECT r.tenant_id, r.id AS run_id, '' AS node, 'RUN' AS scope, r.flow_control_gates -> 'concurrency' AS g \
           FROM runs r WHERE r.tenant_id = $1 AND r.flow_control_gates IS NOT NULL \
         UNION ALL \
         SELECT s.tenant_id, s.run_id, s.node, 'STEP', s.flow_control_gates -> 'concurrency' \
           FROM steps s WHERE s.tenant_id = $1 AND s.flow_control_gates IS NOT NULL \
            AND cardinality(s.instance_path_nodes) = 0";
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    let checks: [(&str, String); 5] = [
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
            "a holder row whose gates hold no such permit",
            format!(
                "SELECT h.run_id::text || '/' || h.node FROM concurrency_holders h WHERE h.tenant_id = $1 \
                   AND NOT EXISTS (SELECT 1 FROM ({GATES}) p WHERE p.run_id = h.run_id AND p.node = h.node \
                       AND p.scope = h.holder_scope AND p.g ->> 'phase' = 'HELD' \
                       AND (p.g ->> 'attempt')::int = h.attempt AND p.g ->> 'pool' = h.pool \
                       AND p.g ->> 'group' = h.group_name AND p.g ->> 'key' = encode(substr(h.key_digest, 1, 6), 'hex'))"
            ),
        ),
        (
            "a waiter row whose gates wait for no such permit",
            format!(
                "SELECT w.run_id::text || '/' || w.node FROM concurrency_waiters w WHERE w.tenant_id = $1 \
                   AND NOT EXISTS (SELECT 1 FROM ({GATES}) p WHERE p.run_id = w.run_id AND p.node = w.node \
                       AND p.scope = w.permit_scope AND p.g ->> 'phase' = 'QUEUED' \
                       AND (p.g ->> 'attempt')::int = w.attempt AND p.g ->> 'pool' = w.pool \
                       AND p.g ->> 'group' = w.group_name AND p.g ->> 'key' = encode(substr(w.key_digest, 1, 6), 'hex') \
                       AND (p.g ->> 'deadlineUs')::bigint = (extract(epoch FROM w.deadline_at) * 1000000)::bigint)"
            ),
        ),
        (
            "a permit the gates hold without its holder row",
            format!(
                "SELECT p.run_id::text || '/' || p.node FROM ({GATES}) p WHERE p.g ->> 'phase' = 'HELD' \
                   AND NOT EXISTS (SELECT 1 FROM concurrency_holders h WHERE h.tenant_id = p.tenant_id \
                       AND h.run_id = p.run_id AND h.node = p.node AND h.holder_scope = p.scope)"
            ),
        ),
        (
            "a permit the gates wait for without its waiter row",
            format!(
                "SELECT p.run_id::text || '/' || p.node FROM ({GATES}) p WHERE p.g ->> 'phase' = 'QUEUED' \
                   AND NOT EXISTS (SELECT 1 FROM concurrency_waiters w WHERE w.tenant_id = p.tenant_id \
                       AND w.run_id = p.run_id AND w.node = p.node AND w.permit_scope = p.scope)"
            ),
        ),
    ];
    for (what, statement) in &checks {
        // built from constants only, no value of a request or a row
        let found: Vec<String> = match sqlx::query_scalar(sqlx::AssertSqlSafe(statement.clone()))
            .bind(tenant.as_uuid())
            .fetch_all(&mut *tx)
            .await
        {
            Ok(found) => found,
            // a schema that predates the gates (a rollback under test) has no permit to check
            Err(sqlx::Error::Database(e))
                if matches!(e.code().as_deref(), Some("42P01" | "42703")) =>
            {
                return Ok(Vec::new());
            }
            Err(e) => return Err(e.to_string()),
        };
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
