//! Candidate queries for the scheduler and the transactional outbox.
//!
//! Every comparison uses the database's clock (`now()`), so schedulers on machines with different
//! clocks agree on what is due. It is `now()` and not `clock_timestamp()` because the planner can
//! use a stable function as the bound of an index range and cannot use a volatile one: with
//! `clock_timestamp()` the filter on a deadline is applied to every entry of the index, in order,
//! until the limit is reached, so a pass with nothing due reads every waiting step. With `now()`
//! (the start of the statement, which is when these candidate queries are asked) it reads the
//! entries that are due and stops. The queries only *find* candidates; the authority is the
//! run's row lock, taken by the caller, under which the domain re-checks the condition.

use std::collections::BTreeMap;
use std::time::Duration;

use sagat_application::error::StoreError;
use sagat_application::execution::MAX_RUN_OUTPUT_BYTES;
use sagat_application::maintenance::{
    BatchPurged, EndedChild, LeaseLoss, LostLease, MaintenanceStore, OutboxDue, OutboxMessage,
    OutboxStore, PendingChild, PendingContinuation, RunRef, StepRef, Window,
};
use sagat_domain::execution::{Fence, InstancePath, RunId};
use sagat_domain::identity::TenantId;
use sagat_domain::workflow::{MAX_NODES, NodeId};
use sqlx::postgres::PgRow;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::execution::PgRunStore;
use crate::identity::{col, corrupt, store_error, store_error_without_text};
use crate::pool::BeginSafely as _;

fn micros(duration: Duration) -> i64 {
    i64::try_from(duration.as_micros()).unwrap_or(i64::MAX)
}

fn run_ref(row: &PgRow) -> Result<RunRef, StoreError> {
    Ok(RunRef {
        tenant_id: TenantId::new(col::<Uuid>(row, "tenant_id")?),
        run_id: RunId::new(col::<Uuid>(row, "id")?),
    })
}

fn step_ref(row: &PgRow) -> Result<StepRef, StoreError> {
    let node: String = col(row, "node")?;
    Ok(StepRef {
        tenant_id: TenantId::new(col::<Uuid>(row, "tenant_id")?),
        run_id: RunId::new(col::<Uuid>(row, "run_id")?),
        node: NodeId::parse_stored(&node).map_err(|_| corrupt("step node"))?,
        instance_path: InstancePath::root(),
    })
}

/// A waiter row: a step's names its node; a run's start's has `''` (T022-022).
fn gate_waiter(row: &PgRow) -> Result<sagat_application::flow_control::GateWaiter, StoreError> {
    let node: String = col(row, "node")?;
    Ok(sagat_application::flow_control::GateWaiter {
        tenant_id: TenantId::new(col::<Uuid>(row, "tenant_id")?),
        run_id: RunId::new(col::<Uuid>(row, "run_id")?),
        node: if node.is_empty() {
            None
        } else {
            Some(NodeId::parse_stored(&node).map_err(|_| corrupt("waiter node"))?)
        },
    })
}

/// Whether `error` is PostgreSQL's `undefined_table` (42P01): a schema that predates the tables a pass reads (a rollback,
/// or a rollout whose migration has not run). Such a pass has nothing to do, and must not fail every tick.
fn absent_table(error: &sqlx::Error) -> bool {
    error
        .as_database_error()
        .and_then(sqlx::error::DatabaseError::code)
        .is_some_and(|code| code == "42P01")
}

fn addressed_step_ref(row: &PgRow) -> Result<StepRef, StoreError> {
    let mut step = step_ref(row)?;
    let nodes: Vec<String> = col(row, "instance_path_nodes")?;
    let indices: Vec<i32> = col(row, "instance_path_indices")?;
    if nodes.len() != indices.len() {
        return Err(corrupt("step instance path"));
    }
    for (node, index) in nodes.into_iter().zip(indices) {
        let node = NodeId::parse_stored(&node).map_err(|_| corrupt("scope node"))?;
        let index = u32::try_from(index).map_err(|_| corrupt("scope index"))?;
        step.instance_path = step
            .instance_path
            .child(node, index)
            .map_err(|_| corrupt("step instance path"))?;
    }
    Ok(step)
}

/// The messages of the rows a claim returned, in the order they were written.
fn outbox_messages(rows: &[sqlx::postgres::PgRow]) -> Result<Vec<OutboxMessage>, StoreError> {
    let mut out = rows
        .iter()
        .map(|row| {
            let payload: serde_json::Value = col(row, "payload")?;
            let attempts: i32 = col(row, "attempt_count")?;
            Ok(OutboxMessage {
                id: col(row, "id")?,
                tenant_id: TenantId::new(col::<Uuid>(row, "tenant_id")?),
                aggregate_id: col(row, "aggregate_id")?,
                event_type: col(row, "event_type")?,
                routing_key: col(row, "routing_key")?,
                payload: serde_json::to_vec(&payload).map_err(|_| corrupt("outbox payload"))?,
                attempt_count: u32::try_from(attempts).map_err(|_| corrupt("attempts"))?,
                created_at: col(row, "created_at")?,
            })
        })
        .collect::<Result<Vec<_>, StoreError>>()?;
    out.sort_by_key(|m| m.created_at);
    Ok(out)
}

/// A window row's key as read: tenant, namespace, pool, scope, group and digest.
type WindowKey = (Uuid, Uuid, String, String, String, Vec<u8>);

/// The window as the three parameters every candidate query takes: how many, the shards to look at
/// and the tenants to leave out.
fn window_params(window: &Window) -> (i64, Vec<i16>, Vec<Uuid>) {
    (
        i64::from(window.limit),
        window.shards.indexes(),
        window.exclude.iter().map(|t| t.as_uuid()).collect(),
    )
}

impl MaintenanceStore for PgRunStore {
    async fn due_gate_keys(
        &self,
        window: &Window,
    ) -> Result<Vec<sagat_application::flow_control::GateKey>, StoreError> {
        let (limit, shards, exclude) = window_params(window);
        // the partial indexes `(key_shard, next_eligible_at) WHERE waiters > 0` of both key tables serve this
        // `now()` is the selection's instant: at or after every hint it selects, at or before every count of the passes
        // that follow (decision D022-066 item 3)
        let rows = sqlx::query(
            "SELECT k.tenant_id, k.namespace_id, k.kind, k.scope, k.group_name, k.key_digest, k.key_shard, \
                    k.next_eligible_at, now() AS selected_at \
               FROM unnest($2::smallint[]) AS sh(s) \
               CROSS JOIN LATERAL ( \
                 (SELECT tenant_id, namespace_id, 'CONCURRENCY' AS kind, permit_scope AS scope, group_name, key_digest, \
                         key_shard, next_eligible_at \
                    FROM concurrency_semaphores \
                   WHERE key_shard = sh.s AND waiters > 0 AND next_eligible_at <= now() \
                     AND pool = 'ORDINARY' AND tenant_id <> ALL($3::uuid[]) \
                   ORDER BY next_eligible_at LIMIT $1) \
                 UNION ALL \
                 (SELECT tenant_id, namespace_id, 'RATE' AS kind, limit_scope AS scope, group_name, key_digest, \
                         key_shard, next_eligible_at \
                    FROM rate_limit_windows \
                   WHERE key_shard = sh.s AND waiters > 0 AND next_eligible_at <= now() \
                     AND pool = 'ORDINARY' AND tenant_id <> ALL($3::uuid[]) \
                   ORDER BY next_eligible_at LIMIT $1) \
               ) k \
              ORDER BY k.next_eligible_at LIMIT $1",
        )
        .bind(limit)
        .bind(shards)
        .bind(exclude)
        .fetch_all(self.pool())
        .await;
        // a schema that predates feature 022 has no key to serve (the scheduler goes on with the schema it finds)
        let rows = match rows {
            Err(error) if absent_table(&error) => return Ok(Vec::new()),
            rows => rows.map_err(store_error)?,
        };
        rows.iter()
            .map(|row| {
                let kind: String = col(row, "kind")?;
                let scope: String = col(row, "scope")?;
                let group: String = col(row, "group_name")?;
                let digest: Vec<u8> = col(row, "key_digest")?;
                let shard: i16 = col(row, "key_shard")?;
                Ok(sagat_application::flow_control::GateKey {
                    tenant_id: TenantId::new(col::<Uuid>(row, "tenant_id")?),
                    namespace_id: sagat_domain::identity::NamespaceId::new(col::<Uuid>(
                        row,
                        "namespace_id",
                    )?),
                    kind: if kind == "RATE" {
                        sagat_domain::workflow::GateKind::RateLimit
                    } else {
                        sagat_domain::workflow::GateKind::Concurrency
                    },
                    level: match scope.as_str() {
                        "RUN" => sagat_domain::workflow::GateLevel::Run,
                        "STEP" => sagat_domain::workflow::GateLevel::Step,
                        _ => return Err(corrupt("key scope")),
                    },
                    group: sagat_domain::workflow::ResolvedGroup::parse(&group)
                        .map_err(|_| corrupt("key group"))?,
                    digest: digest.try_into().map_err(|_| corrupt("key digest"))?,
                    shard: u8::try_from(shard)
                        .ok()
                        .and_then(sagat_domain::sharding::Shard::new)
                        .ok_or_else(|| corrupt("key shard"))?,
                    hint: col(row, "next_eligible_at")?,
                    selected_at: col(row, "selected_at")?,
                })
            })
            .collect()
    }

    async fn gate_key_waiters(
        &self,
        key: &sagat_application::flow_control::GateKey,
        limit: u32,
    ) -> Result<Vec<sagat_application::flow_control::GateWaiter>, StoreError> {
        let statement = match key.kind {
            sagat_domain::workflow::GateKind::Concurrency => {
                concat!(
                    "SELECT w.tenant_id, w.run_id, w.node FROM concurrency_waiters w \
                   JOIN runs r ON r.tenant_id = w.tenant_id AND r.id = w.run_id \
                  WHERE w.tenant_id = $1 AND w.namespace_id = $2 AND w.pool = 'ORDINARY' AND w.permit_scope = $6 \
                    AND w.group_name = $3 AND w.key_digest = $4 AND ",
                    eligible_gate_waiter!("w", "r", "now()"),
                    " \
                  ORDER BY w.enqueued_at, w.run_id, w.node LIMIT $5"
                )
            }
            sagat_domain::workflow::GateKind::RateLimit => {
                concat!(
                    "SELECT w.tenant_id, w.run_id, w.node FROM rate_limit_waiters w \
                   JOIN runs r ON r.tenant_id = w.tenant_id AND r.id = w.run_id \
                  WHERE w.tenant_id = $1 AND w.namespace_id = $2 AND w.pool = 'ORDINARY' AND w.limit_scope = $6 \
                    AND w.group_name = $3 AND w.key_digest = $4 AND ",
                    eligible_gate_waiter!("w", "r", "now()"),
                    " \
                  ORDER BY w.enqueued_at, w.run_id, w.node LIMIT $5"
                )
            }
        };
        sqlx::query(statement)
            .bind(key.tenant_id.as_uuid())
            .bind(key.namespace_id.as_uuid())
            .bind(key.group.as_str())
            .bind(key.digest.as_slice())
            .bind(i64::from(limit))
            .bind(key.level.as_str())
            .fetch_all(self.pool())
            .await
            .map_err(store_error)?
            .iter()
            .map(gate_waiter)
            .collect()
    }

    async fn rest_gate_key(
        &self,
        key: &sagat_application::flow_control::GateKey,
        holder: Option<(&str, sagat_application::shards::Lease)>,
    ) -> Result<(), StoreError> {
        // only the holder of the key's home shard rests it, and only while its hint is the one the pass was selected by:
        // a release that moved the hint since is kept (never late)
        if key.kind == sagat_domain::workflow::GateKind::Concurrency {
            sqlx::query(
                "UPDATE concurrency_semaphores SET next_eligible_at = NULL, updated_at = clock_timestamp() \
                  WHERE tenant_id = $1 AND namespace_id = $2 AND pool = 'ORDINARY' AND permit_scope = $9 \
                    AND group_name = $3 AND key_digest = $4 AND next_eligible_at = $5 \
                    AND ($6::text IS NULL OR EXISTS (SELECT 1 FROM scheduler_shards s WHERE s.shard = $7 \
                         AND s.owner = $6 AND s.generation = $8 AND s.expires_at > clock_timestamp()))",
            )
            .bind(key.tenant_id.as_uuid())
            .bind(key.namespace_id.as_uuid())
            .bind(key.group.as_str())
            .bind(key.digest.as_slice())
            .bind(key.hint)
            .bind(holder.map(|(owner, _)| owner))
            .bind(holder.map(|(_, lease)| i16::from(lease.shard.index())))
            .bind(holder.and_then(|(_, lease)| i64::try_from(lease.generation).ok()))
            .bind(key.level.as_str())
            .execute(self.pool())
            .await
            .map_err(store_error)?;
            return Ok(());
        }
        // a window: locked first if its hint is still the selection's, then, while the key's home shard is held, its hint
        // computed by a second statement (the lease row is only read, a leaf lock, ADR-0050 decision 7), whose
        // snapshot holds every grant committed before the lock was granted (a grant is written only under its window's
        // lock). One statement would re-check its compare-and-set on the row a grant just changed but count from the
        // snapshot it took before that grant committed: a grant whose `least` left the hint unchanged was lost and the
        // window rested with no hint (correctness review Major 1 of the re-review of 2026-11-20, decision D022-066 item 2)
        let mut tx = self.pool().begin_safely().await.map_err(store_error)?;
        let held: Option<i32> = sqlx::query_scalar(
            "SELECT 1 FROM rate_limit_windows \
              WHERE tenant_id = $1 AND namespace_id = $2 AND pool = 'ORDINARY' AND limit_scope = $6 \
                AND group_name = $3 AND key_digest = $4 AND next_eligible_at = $5 \
                FOR UPDATE",
        )
        .bind(key.tenant_id.as_uuid())
        .bind(key.namespace_id.as_uuid())
        .bind(key.group.as_str())
        .bind(key.digest.as_slice())
        .bind(key.hint)
        .bind(key.level.as_str())
        .fetch_optional(&mut *tx)
        .await
        .map_err(store_error)?;
        if held.is_none() {
            tx.rollback().await.map_err(store_error)?;
            return Ok(());
        }
        // the first instant a grant that counted when the pass began may stop counting for one of the key's waiters: each
        // waiter counts over its own window (D022-009), so the instant is taken for every window length its waiters
        // declare; a leased grant ages out at its lease plus that length, an unleased one no earlier than the pass's start
        // plus it (it frees by a return, which sets the hint, or by a lease, whose recording lowers the hint). The pass's
        // start is the instant of the statement that selected the key ($5: at or after its hint, at or before every count
        // the pass made, decision D022-066 item 3): a grant that aged out while the pass ran yields an instant already
        // past, so the key is due again at once rather than never (correctness review Major 3 of 2026-11-20). Early,
        // never late, and no window lock for the lease pass (D022-021, D022-064; plan "The grant pass, by key")
        sqlx::query(
            "UPDATE rate_limit_windows w \
                SET next_eligible_at = ( \
                    SELECT min(coalesce(g.leased_at, $5) + make_interval(secs => p.period_ms / 1000.0)) \
                      FROM rate_limit_grants g \
                     CROSS JOIN (SELECT DISTINCT wt.period_ms FROM rate_limit_waiters wt \
                                  WHERE wt.tenant_id = w.tenant_id AND wt.namespace_id = w.namespace_id \
                                    AND wt.pool = w.pool AND wt.limit_scope = w.limit_scope \
                                    AND wt.group_name = w.group_name AND wt.key_digest = w.key_digest) p \
                     WHERE g.tenant_id = w.tenant_id AND g.namespace_id = w.namespace_id \
                       AND g.pool = w.pool AND g.limit_scope = w.limit_scope \
                       AND g.group_name = w.group_name AND g.key_digest = w.key_digest \
                       AND (g.leased_at IS NULL \
                            OR g.leased_at + make_interval(secs => p.period_ms / 1000.0) > $5)), \
                    updated_at = clock_timestamp() \
              WHERE w.tenant_id = $1 AND w.namespace_id = $2 AND w.pool = 'ORDINARY' AND w.limit_scope = $6 \
                AND w.group_name = $3 AND w.key_digest = $4 \
                AND ($7::text IS NULL OR EXISTS (SELECT 1 FROM scheduler_shards s WHERE s.shard = $8 \
                     AND s.owner = $7 AND s.generation = $9 AND s.expires_at > clock_timestamp()))",
        )
        .bind(key.tenant_id.as_uuid())
        .bind(key.namespace_id.as_uuid())
        .bind(key.group.as_str())
        .bind(key.digest.as_slice())
        .bind(key.selected_at)
        .bind(key.level.as_str())
        .bind(holder.map(|(owner, _)| owner))
        .bind(holder.map(|(_, lease)| i16::from(lease.shard.index())))
        .bind(holder.and_then(|(_, lease)| i64::try_from(lease.generation).ok()))
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        tx.commit().await.map_err(store_error)?;
        Ok(())
    }

    async fn record_rate_leases(&self, window: &Window) -> Result<u64, StoreError> {
        let (limit, shards, exclude) = window_params(window);
        // the root step's journal is read through its instance (`steps.step_id` is the `instance_id` of its events,
        // `run_events_instance_idx`), and only for a step whose projection says the attempt was leased (security review
        // Major 2 of 2026-11-20: a grant no worker leased costs no journal probe); a lease counts only when it is the
        // granted attempt's and was journaled after the grant (D022-029); the instant is the lease's, or the grant's when
        // the lease's transaction read its clock first (a later instant only keeps a slot longer: under-delivery, never an
        // early grant, plan "Failure and recovery")
        let recorded = sqlx::query(
            "UPDATE rate_limit_grants g SET leased_at = greatest(d.leased_at, g.granted_at) \
               FROM (SELECT c.tenant_id, c.run_id, c.node, c.attempt, c.leased_at \
                       FROM unnest($2::smallint[]) AS sh(s) \
                      CROSS JOIN LATERAL ( \
                        SELECT u.tenant_id, u.run_id, u.node, u.attempt, l.occurred_at AS leased_at \
                          FROM rate_limit_grants u \
                          JOIN steps st ON st.tenant_id = u.tenant_id AND st.run_id = u.run_id \
                                       AND st.instance_path_nodes = '{}' AND st.instance_path_indices = '{}' \
                                       AND st.node = u.node AND st.attempt >= u.attempt \
                         CROSS JOIN LATERAL ( \
                           SELECT e.occurred_at FROM run_events e \
                            WHERE e.tenant_id = u.tenant_id AND e.run_id = u.run_id AND e.instance_id = st.step_id \
                              AND e.event_type = 'step.leased' AND e.attempt = u.attempt \
                              AND e.seq > (SELECT max(gr.seq) FROM run_events gr \
                                            WHERE gr.tenant_id = u.tenant_id AND gr.run_id = u.run_id \
                                              AND gr.instance_id = st.step_id AND gr.event_type = 'rate_limit.granted' \
                                              AND gr.attempt = u.attempt) \
                            ORDER BY e.seq LIMIT 1) l \
                         WHERE u.shard = sh.s AND u.leased_at IS NULL AND u.limit_scope = 'STEP' \
                           AND u.tenant_id <> ALL($3::uuid[]) \
                         ORDER BY u.granted_at LIMIT $1) c) d \
              WHERE g.tenant_id = d.tenant_id AND g.run_id = d.run_id AND g.node = d.node AND g.attempt = d.attempt \
                AND g.leased_at IS NULL \
             RETURNING g.tenant_id, g.namespace_id, g.pool, g.limit_scope, g.group_name, g.key_digest, g.leased_at",
        )
        .bind(limit)
        .bind(shards)
        .bind(exclude)
        .fetch_all(self.pool())
        .await;
        let recorded = match recorded {
            // a schema that predates feature 022 has no grant to record
            Err(error) if absent_table(&error) => return Ok(0),
            recorded => recorded.map_err(store_error)?,
        };
        // the windows of the recorded slots wake no later than the first instant a slot may stop counting for one of
        // their waiters (the lease plus the shortest window they wait over): the rest rule saw these grants unleased and
        // waited from its own instant, which a lease journaled earlier makes late (correctness review minor 2, architecture
        // review on D022-064 item 5). After the grants' commit, one window a statement, so no transaction holds a grant
        // row and waits for a window, nor two windows
        let mut earliest: BTreeMap<WindowKey, OffsetDateTime> = BTreeMap::new();
        for row in &recorded {
            let key = (
                col::<Uuid>(row, "tenant_id")?,
                col::<Uuid>(row, "namespace_id")?,
                col::<String>(row, "pool")?,
                col::<String>(row, "limit_scope")?,
                col::<String>(row, "group_name")?,
                col::<Vec<u8>>(row, "key_digest")?,
            );
            let leased: OffsetDateTime = col(row, "leased_at")?;
            earliest
                .entry(key)
                .and_modify(|at| *at = (*at).min(leased))
                .or_insert(leased);
        }
        for ((tenant, namespace, pool, scope, group, digest), leased) in earliest {
            sqlx::query(
                "UPDATE rate_limit_windows w \
                    SET next_eligible_at = least(w.next_eligible_at, \
                            $7 + make_interval(secs => (SELECT min(x.period_ms) FROM rate_limit_waiters x \
                                                         WHERE x.tenant_id = w.tenant_id AND x.namespace_id = w.namespace_id \
                                                           AND x.pool = w.pool AND x.limit_scope = w.limit_scope \
                                                           AND x.group_name = w.group_name \
                                                           AND x.key_digest = w.key_digest) / 1000.0)), \
                        updated_at = clock_timestamp() \
                  WHERE w.tenant_id = $1 AND w.namespace_id = $2 AND w.pool = $3 AND w.limit_scope = $4 \
                    AND w.group_name = $5 AND w.key_digest = $6 AND w.waiters > 0",
            )
            .bind(tenant)
            .bind(namespace)
            .bind(pool)
            .bind(scope)
            .bind(group)
            .bind(digest)
            .bind(leased)
            .execute(self.pool())
            .await
            .map_err(store_error)?;
        }
        Ok(u64::try_from(recorded.len()).unwrap_or(u64::MAX))
    }

    async fn purge_rate_grants(&self, keys: u32, limit: u32) -> Result<u64, StoreError> {
        let now: OffsetDateTime = sqlx::query_scalar("SELECT clock_timestamp()")
            .fetch_one(self.pool())
            .await
            .map_err(store_error)?;
        self.purge_rate_grants_as_of(now, keys, limit).await
    }

    async fn due_gate_waits(
        &self,
        window: &Window,
    ) -> Result<Vec<sagat_application::flow_control::GateWaiter>, StoreError> {
        let (limit, shards, exclude) = window_params(window);
        // a step queued for both gates has one deadline, the earlier, on both rows: `UNION` names it once
        let rows = sqlx::query(
            "SELECT t.tenant_id, t.run_id, t.node FROM unnest($2::smallint[]) AS sh(s) \
             CROSS JOIN LATERAL ( \
                 (SELECT tenant_id, run_id, node, deadline_at FROM concurrency_waiters \
                   WHERE shard = sh.s AND deadline_at <= now() \
                     AND tenant_id <> ALL($3::uuid[]) ORDER BY deadline_at LIMIT $1) \
                 UNION \
                 (SELECT tenant_id, run_id, node, deadline_at FROM rate_limit_waiters \
                   WHERE shard = sh.s AND deadline_at <= now() \
                     AND tenant_id <> ALL($3::uuid[]) ORDER BY deadline_at LIMIT $1) \
             ) t ORDER BY t.deadline_at LIMIT $1",
        )
        .bind(limit)
        .bind(shards)
        .bind(exclude)
        .fetch_all(self.pool())
        .await;
        // a schema that predates feature 022 has no waiter to time out
        match rows {
            Err(error) if absent_table(&error) => Ok(Vec::new()),
            rows => rows.map_err(store_error)?.iter().map(gate_waiter).collect(),
        }
    }

    /// Boundaries that can be handed off now. A run that still holds open work is not a candidate: it
    /// would sit at the head of the oldest-first list and keep the others from being examined
    /// (D296). The store proves quiescence again under the chain and run locks.
    async fn continuation_candidates(
        &self,
        window: &Window,
    ) -> Result<Vec<PendingContinuation>, StoreError> {
        let (limit, shards, exclude) = window_params(window);
        sqlx::query(
            "SELECT candidate.tenant_id,candidate.namespace_id,candidate.chain_id, \
                    candidate.run_id,candidate.node \
             FROM unnest($2::smallint[]) AS sh(s) \
             CROSS JOIN LATERAL ( \
               SELECT step.tenant_id,chain.namespace_id,chain.chain_id,step.run_id,step.node, \
                      step.updated_at \
               FROM steps step JOIN runs run \
                 ON run.tenant_id=step.tenant_id AND run.id=step.run_id \
               JOIN execution_chains chain \
                 ON chain.tenant_id=step.tenant_id AND chain.current_run_id=step.run_id \
               WHERE run_shard(step.run_id)=sh.s AND run.status='RUNNING' \
                 AND step.status='PENDING' AND cardinality(step.instance_path_nodes)=0 \
                 AND chain.cancel_requested=false AND chain.final_outcome IS NULL \
                 AND step.tenant_id <> ALL($3::uuid[]) \
                 AND NOT EXISTS ( \
                   SELECT 1 FROM steps open \
                   WHERE open.tenant_id=step.tenant_id AND open.run_id=step.run_id \
                     AND (open.node<>step.node OR cardinality(open.instance_path_nodes)>0) \
                     AND (open.status<>'SUCCEEDED' OR open.routed=false)) \
                 AND NOT EXISTS ( \
                   SELECT 1 FROM run_sagas saga WHERE saga.tenant_id=step.tenant_id \
                     AND saga.run_id=step.run_id AND saga.phase<>'DONE') \
                 AND NOT EXISTS ( \
                   SELECT 1 FROM approval_requests approval \
                   WHERE approval.tenant_id=step.tenant_id AND approval.run_id=step.run_id \
                     AND approval.status='PENDING') \
                 AND NOT EXISTS ( \
                   SELECT 1 FROM runs child WHERE child.tenant_id=step.tenant_id \
                     AND child.parent_run_id=step.run_id AND child.parent_settled=false) \
                 AND NOT EXISTS ( \
                   SELECT 1 FROM child_chain_links link WHERE link.tenant_id=step.tenant_id \
                     AND link.parent_run_id=step.run_id \
                     AND (link.final_outcome IS NULL OR link.settled=false)) \
               ORDER BY step.updated_at LIMIT $1 \
             ) candidate ORDER BY candidate.updated_at LIMIT $1",
        )
        .bind(limit)
        .bind(shards)
        .bind(exclude)
        .fetch_all(self.pool())
        .await
        .map_err(store_error)?
        .iter()
        .map(|row| {
            let node: String = col(row, "node")?;
            Ok(PendingContinuation {
                tenant_id: TenantId::new(col::<Uuid>(row, "tenant_id")?),
                namespace_id: sagat_domain::identity::NamespaceId::new(col::<Uuid>(
                    row,
                    "namespace_id",
                )?),
                chain_id: RunId::new(col::<Uuid>(row, "chain_id")?),
                run_id: RunId::new(col::<Uuid>(row, "run_id")?),
                node: NodeId::parse_stored(&node).map_err(|_| corrupt("continuation node"))?,
            })
        })
        .collect()
    }

    // The queue shape is: for every shard the replica owns, the oldest `limit` due items of
    // that shard (the first entries of an index whose leading column is the shard, so a shard with
    // nothing due costs one probe), then the oldest `limit` of all of them. The oldest `limit` of the
    // owned shards are among the oldest `limit` of each, so the answer is exactly what one query
    // over all of them would give. Collection discovery also filters semantic scope/history facts;
    // its output limits do not bound the number of scanned rows or imply a shard-first index.
    async fn collection_candidates(&self, window: &Window) -> Result<Vec<StepRef>, StoreError> {
        let (limit, shards, exclude) = window_params(window);
        let output_limit =
            i64::try_from(MAX_RUN_OUTPUT_BYTES).map_err(|_| corrupt("run output byte limit"))?;
        let root_limit = i64::try_from(MAX_NODES).map_err(|_| corrupt("root collection limit"))?;
        // Lengths only nominate a dormant root owner. The frontier's repeatable-read inspection
        // and the locked native command verify the legacy evidence and effective total again.
        // Keep fitting legacy histories out of the limited queue; a missing join is not zero bytes.
        sqlx::query(
            "SELECT candidate.tenant_id,candidate.run_id,candidate.node, \
                    candidate.instance_path_nodes,candidate.instance_path_indices \
             FROM unnest($2::smallint[]) AS sh(s) \
             CROSS JOIN LATERAL ( \
               SELECT work.tenant_id,work.run_id,work.node,work.instance_path_nodes, \
                      work.instance_path_indices,work.updated_at \
               FROM ( \
                 SELECT step.tenant_id,step.run_id,step.node,step.instance_path_nodes, \
                        step.instance_path_indices,step.updated_at \
                 FROM steps step JOIN runs run \
                   ON run.tenant_id=step.tenant_id AND run.id=step.run_id \
                 JOIN workflow_versions version \
                   ON version.tenant_id=run.tenant_id AND version.id=run.workflow_version_id \
                 WHERE run_shard(step.run_id)=sh.s AND run.status='RUNNING' \
                   AND step.status='PENDING' AND cardinality(step.instance_path_nodes)=0 \
                   AND step.tenant_id <> ALL($3::uuid[]) \
                   AND EXISTS ( \
                     SELECT 1 FROM jsonb_array_elements( \
                       (convert_from(version.document,'UTF8')::jsonb)->'nodes') declared \
                     WHERE declared->>'id'=step.node AND declared->>'type'='forEach') \
                 UNION ALL \
                 SELECT scope.tenant_id,scope.run_id,scope.owner_node, \
                        scope.owner_path_nodes,scope.owner_path_indices,scope.updated_at \
                 FROM iteration_scopes scope JOIN runs run \
                   ON run.tenant_id=scope.tenant_id AND run.id=scope.run_id \
                 LEFT JOIN run_instance_usage usage \
                   ON usage.tenant_id=run.tenant_id AND usage.run_id=run.id \
                 WHERE run_shard(scope.run_id)=sh.s \
                   AND run.status IN ('RUNNING','WAITING','CANCELLING','COMPENSATING') \
                   AND scope.kind='COLLECTION' \
                   AND scope.tenant_id <> ALL($3::uuid[]) \
                   AND ( \
                     scope.lifecycle='DRAINING' \
                     OR (scope.lifecycle='OPEN' AND ( \
                       run.status='CANCELLING' \
                       OR (scope.owner_path_nodes='{}'::text[] \
                         AND scope.owner_path_indices='{}'::integer[] \
                         AND (usage.logical_output_bytes IS NULL \
                           OR usage.logical_output_bytes < 0 \
                           OR EXISTS ( \
                             SELECT 1 FROM ( \
                               SELECT prior.owner_node,prior.namespace_id,prior.result_digest \
                               FROM iteration_scopes prior \
                               WHERE prior.tenant_id=scope.tenant_id \
                                 AND prior.run_id=scope.run_id \
                                 AND prior.owner_path_nodes='{}'::text[] \
                                 AND prior.owner_path_indices='{}'::integer[] \
                                 AND prior.lifecycle='SUCCEEDED' \
                               ORDER BY prior.owner_node LIMIT ($5 + 1) \
                             ) completed \
                             LEFT JOIN steps owner \
                               ON owner.tenant_id=scope.tenant_id \
                              AND owner.run_id=scope.run_id \
                              AND owner.instance_path_nodes='{}'::text[] \
                              AND owner.instance_path_indices='{}'::integer[] \
                              AND owner.node=completed.owner_node \
                             LEFT JOIN payloads payload \
                               ON payload.tenant_id=scope.tenant_id \
                              AND payload.digest=completed.result_digest \
                             HAVING count(*) > $5 \
                               OR bool_or( \
                                 completed.namespace_id IS DISTINCT FROM run.namespace_id \
                                 OR completed.result_digest IS NULL \
                                 OR octet_length(completed.result_digest) <> 32 \
                                 OR owner.step_id IS NULL \
                                 OR owner.namespace_id IS DISTINCT FROM run.namespace_id \
                                 OR owner.kind IS DISTINCT FROM 'FORWARD' \
                                 OR owner.node_kind NOT IN ('forEach','legacyControl') \
                                 OR owner.status IS DISTINCT FROM 'SUCCEEDED' \
                                 OR owner.routed IS DISTINCT FROM true \
                                 OR (owner.output_digest IS NOT NULL \
                                   AND owner.output_digest IS DISTINCT FROM completed.result_digest) \
                                 OR payload.digest IS NULL) \
                               OR coalesce(sum(octet_length(payload.content)::bigint) \
                                 FILTER (WHERE owner.output_digest IS NULL),0) \
                                 > $4::numeric - usage.logical_output_bytes::numeric))) \
                       OR (scope.next_admission_index < scope.item_count \
                         AND scope.active_count < scope.max_concurrency) \
                       OR scope.completed_count = scope.item_count \
                       OR EXISTS ( \
                         SELECT 1 FROM collection_iterations member JOIN steps returned \
                           ON returned.tenant_id=member.tenant_id \
                          AND returned.run_id=member.run_id \
                          AND returned.instance_path_nodes=member.entry_path_nodes \
                          AND returned.instance_path_indices=member.entry_path_indices \
                         WHERE member.tenant_id=scope.tenant_id \
                           AND member.run_id=scope.run_id AND member.scope_id=scope.scope_id \
                           AND member.lifecycle='ADMITTED' \
                           AND (returned.status IN ('PENDING','FAILED') OR (returned.status='SUCCEEDED' AND NOT returned.routed))))) \
                   ) \
               ) work ORDER BY work.updated_at LIMIT $1 \
             ) candidate ORDER BY candidate.updated_at LIMIT $1",
        )
        .bind(limit)
        .bind(shards)
        .bind(exclude)
        .bind(output_limit)
        .bind(root_limit)
        .fetch_all(self.pool())
        .await
        .map_err(store_error)?
        .iter()
        .map(addressed_step_ref)
        .collect()
    }

    async fn due_timers(&self, window: &Window) -> Result<Vec<StepRef>, StoreError> {
        let (limit, shards, exclude) = window_params(window);
        sqlx::query(
            "SELECT t.tenant_id, t.run_id, t.node,t.instance_path_nodes,t.instance_path_indices FROM unnest($2::smallint[]) AS sh(s) \
             CROSS JOIN LATERAL ( \
                 SELECT tenant_id, run_id, node, due_at,instance_path_nodes,instance_path_indices FROM steps \
                 WHERE run_shard(run_id) = sh.s AND status = 'WAITING' AND waiting_kind = 'TIMER' \
                   AND due_at <= now() AND tenant_id <> ALL($3::uuid[]) \
                 ORDER BY due_at LIMIT $1) t \
             ORDER BY t.due_at LIMIT $1",
        )
        .bind(limit)
        .bind(shards)
        .bind(exclude)
        .fetch_all(self.pool())
        .await
        .map_err(store_error)?
        .iter()
        .map(addressed_step_ref)
        .collect()
    }

    async fn due_wait_deadlines(&self, window: &Window) -> Result<Vec<StepRef>, StoreError> {
        let (limit, shards, exclude) = window_params(window);
        sqlx::query(
            "SELECT t.tenant_id, t.run_id, t.node,t.instance_path_nodes,t.instance_path_indices FROM unnest($2::smallint[]) AS sh(s) \
             CROSS JOIN LATERAL ( \
                 SELECT tenant_id, run_id, node, due_at,instance_path_nodes,instance_path_indices FROM steps \
                 WHERE run_shard(run_id) = sh.s AND status = 'WAITING' AND waiting_kind = 'EVENT' \
                   AND due_at <= now() AND tenant_id <> ALL($3::uuid[]) \
                 ORDER BY due_at LIMIT $1) t \
             ORDER BY t.due_at LIMIT $1",
        )
        .bind(limit)
        .bind(shards)
        .bind(exclude)
        .fetch_all(self.pool())
        .await
        .map_err(store_error)?
        .iter()
        .map(addressed_step_ref)
        .collect()
    }

    async fn due_retries(&self, window: &Window) -> Result<Vec<StepRef>, StoreError> {
        let (limit, shards, exclude) = window_params(window);
        sqlx::query(
            "SELECT t.tenant_id, t.run_id, t.node, t.instance_path_nodes, \
                    t.instance_path_indices FROM unnest($2::smallint[]) AS sh(s) \
             CROSS JOIN LATERAL ( \
                 SELECT tenant_id, run_id, node, instance_path_nodes, instance_path_indices, \
                        next_attempt_at FROM steps \
                 WHERE run_shard(run_id) = sh.s AND status = 'RETRY_WAIT' \
                   AND ($4 OR cardinality(instance_path_nodes) = 0) \
                   AND next_attempt_at <= now() AND tenant_id <> ALL($3::uuid[]) \
                 ORDER BY next_attempt_at LIMIT $1) t \
             ORDER BY t.next_attempt_at LIMIT $1",
        )
        .bind(limit)
        .bind(shards)
        .bind(exclude)
        .bind(self.scoped_worker_protocol)
        .fetch_all(self.pool())
        .await
        .map_err(store_error)?
        .iter()
        .map(addressed_step_ref)
        .collect()
    }

    async fn lost_leases(&self, window: &Window) -> Result<Vec<LostLease>, StoreError> {
        let (limit, shards, exclude) = window_params(window);
        // Two index scans (one per partial index) rather than one query with OR.
        let rows = sqlx::query(
            "(SELECT t.tenant_id, t.run_id, t.node, t.instance_path_nodes, \
                    t.instance_path_indices, t.attempt, t.generation, t.cause \
                FROM unnest($2::smallint[]) AS sh(s) \
                CROSS JOIN LATERAL ( \
                    SELECT tenant_id, run_id, node, instance_path_nodes, instance_path_indices, \
                           attempt, generation, 'TIMED_OUT' AS cause, \
                           attempt_deadline_at AS at FROM steps \
                    WHERE run_shard(run_id) = sh.s AND status IN ('LEASED', 'RUNNING') \
                      AND ($4 OR cardinality(instance_path_nodes) = 0) \
                      AND attempt_deadline_at <= now() AND tenant_id <> ALL($3::uuid[]) \
                    ORDER BY attempt_deadline_at LIMIT $1) t \
              ORDER BY t.at LIMIT $1) \
             UNION ALL \
             (SELECT t.tenant_id, t.run_id, t.node, t.instance_path_nodes, \
                     t.instance_path_indices, t.attempt, t.generation, t.cause \
                FROM unnest($2::smallint[]) AS sh(s) \
                CROSS JOIN LATERAL ( \
                    SELECT tenant_id, run_id, node, instance_path_nodes, instance_path_indices, \
                           attempt, generation, 'EXPIRED' AS cause, \
                           lease_expires_at AS at FROM steps \
                    WHERE run_shard(run_id) = sh.s AND status IN ('LEASED', 'RUNNING') \
                      AND ($4 OR cardinality(instance_path_nodes) = 0) \
                      AND lease_expires_at <= now() AND tenant_id <> ALL($3::uuid[]) \
                    ORDER BY lease_expires_at LIMIT $1) t \
              ORDER BY t.at LIMIT $1)",
        )
        .bind(limit)
        .bind(shards)
        .bind(exclude)
        .bind(self.scoped_worker_protocol)
        .fetch_all(self.pool())
        .await
        .map_err(store_error)?;
        let mut out: Vec<LostLease> = Vec::with_capacity(rows.len());
        for row in &rows {
            let step = addressed_step_ref(row)?;
            let attempt: i32 = col(row, "attempt")?;
            let generation: i64 = col(row, "generation")?;
            let fence = Fence {
                attempt: u32::try_from(attempt).map_err(|_| corrupt("attempt"))?,
                generation: u64::try_from(generation).map_err(|_| corrupt("generation"))?,
            };
            let cause: String = col(row, "cause")?;
            // The timeout row comes first, so a step that is both reports the timeout.
            if out.iter().any(|l| l.step == step && l.fence == fence) {
                continue;
            }
            out.push(LostLease {
                step,
                fence,
                cause: if cause == "TIMED_OUT" {
                    LeaseLoss::TimedOut
                } else {
                    LeaseLoss::Expired
                },
            });
        }
        out.truncate(window.limit as usize);
        Ok(out)
    }

    async fn due_start_releases(&self, window: &Window) -> Result<Vec<RunRef>, StoreError> {
        let (limit, shards, exclude) = window_params(window);
        // feature 022 (D022-054): a release a flow-control ceiling refused is held back until its retry instant
        const DUE: &str = "SELECT t.tenant_id, t.id FROM unnest($2::smallint[]) AS sh(s) \
             CROSS JOIN LATERAL ( \
                 SELECT tenant_id, id, start_delayed_until FROM runs \
                 WHERE run_shard(id) = sh.s AND start_delayed_until IS NOT NULL \
                   AND status = 'WAITING' AND start_delayed_until <= now() \
                   AND tenant_id <> ALL($3::uuid[]) \
                   AND NOT EXISTS (SELECT 1 FROM start_release_deferrals d \
                                    WHERE d.tenant_id = runs.tenant_id AND d.run_id = runs.id \
                                      AND d.retry_at > now()) \
                 ORDER BY start_delayed_until LIMIT $1) t \
             ORDER BY t.start_delayed_until LIMIT $1";
        // a schema that predates the deferrals (a rollback, or a rollout whose migration has not run) holds none back
        const DUE_WITHOUT_DEFERRALS: &str = "SELECT t.tenant_id, t.id FROM unnest($2::smallint[]) AS sh(s) \
             CROSS JOIN LATERAL ( \
                 SELECT tenant_id, id, start_delayed_until FROM runs \
                 WHERE run_shard(id) = sh.s AND start_delayed_until IS NOT NULL \
                   AND status = 'WAITING' AND start_delayed_until <= now() \
                   AND tenant_id <> ALL($3::uuid[]) \
                 ORDER BY start_delayed_until LIMIT $1) t \
             ORDER BY t.start_delayed_until LIMIT $1";
        let code = |error: &sqlx::Error| match error {
            sqlx::Error::Database(database) => database.code().map(|code| code.into_owned()),
            _ => None,
        };
        let mut rows = sqlx::query(DUE)
            .bind(limit)
            .bind(&shards)
            .bind(&exclude)
            .fetch_all(self.pool())
            .await;
        if let Err(error) = &rows
            && code(error).as_deref() == Some("42P01")
        {
            rows = sqlx::query(DUE_WITHOUT_DEFERRALS)
                .bind(limit)
                .bind(&shards)
                .bind(&exclude)
                .fetch_all(self.pool())
                .await;
        }
        // A schema that predates the delay (a rollback, or a rollout whose migration has not run yet)
        // has no delayed start to release: that is not a failure of the pass.
        rows.or_else(|error| match code(&error).as_deref() {
            Some("42703") => Ok(Vec::new()),
            _ => Err(error),
        })
        .map_err(store_error)?
        .iter()
        .map(run_ref)
        .collect()
    }

    async fn defer_start_release(
        &self,
        run: RunRef,
        reason: &'static str,
    ) -> Result<u32, StoreError> {
        // the backoff of 018's deferred items (plan row 4): 5 s, doubling, at most 5 min
        let deferrals: i32 = sqlx::query_scalar(
            "INSERT INTO start_release_deferrals AS d \
                    (tenant_id, namespace_id, run_id, deferrals, first_deferred_at, retry_at, reason) \
             SELECT r.tenant_id, r.namespace_id, r.id, 1, now(), now() + interval '5 seconds', $3 \
               FROM runs r WHERE r.tenant_id = $1 AND r.id = $2 \
             ON CONFLICT (tenant_id, run_id) DO UPDATE SET \
                 deferrals = d.deferrals + 1, \
                 retry_at = now() + least(interval '5 seconds' * power(2, least(d.deferrals, 10)), \
                                          interval '5 minutes'), \
                 reason = EXCLUDED.reason \
             RETURNING deferrals",
        )
        .bind(run.tenant_id.as_uuid())
        .bind(run.run_id.as_uuid())
        .bind(reason)
        .fetch_optional(self.pool())
        .await
        .map_err(store_error)?
        .unwrap_or(0);
        Ok(u32::try_from(deferrals).unwrap_or(0))
    }

    async fn overdue_runs(&self, window: &Window) -> Result<Vec<RunRef>, StoreError> {
        let (limit, shards, exclude) = window_params(window);
        sqlx::query(
            "SELECT t.tenant_id, t.id FROM unnest($2::smallint[]) AS sh(s) \
             CROSS JOIN LATERAL ( \
                 SELECT tenant_id, id, deadline_at FROM runs \
                 WHERE run_shard(id) = sh.s AND finished_at IS NULL AND stop_reason IS NULL \
                   AND deadline_at <= now() AND tenant_id <> ALL($3::uuid[]) \
                 ORDER BY deadline_at LIMIT $1) t \
             ORDER BY t.deadline_at LIMIT $1",
        )
        .bind(limit)
        .bind(shards)
        .bind(exclude)
        .fetch_all(self.pool())
        .await
        .map_err(store_error)?
        .iter()
        .map(run_ref)
        .collect()
    }

    async fn stopped_runs(&self, window: &Window) -> Result<Vec<RunRef>, StoreError> {
        let (limit, shards, exclude) = window_params(window);
        sqlx::query(
            "SELECT t.tenant_id, t.id FROM unnest($2::smallint[]) AS sh(s) \
             CROSS JOIN LATERAL ( \
                 SELECT run.tenant_id, run.id, run.updated_at FROM runs run \
                 WHERE run_shard(run.id) = sh.s AND run.status = 'CANCELLING' \
                   AND run.finished_at IS NULL AND run.stop_reason IS NOT NULL AND NOT run.paused \
                   AND run.tenant_id <> ALL($3::uuid[]) \
                   AND NOT EXISTS ( \
                     SELECT 1 FROM steps held \
                     WHERE held.tenant_id = run.tenant_id AND held.run_id = run.id \
                       AND held.status IN ('LEASED','RUNNING')) \
                 ORDER BY run.updated_at LIMIT $1) t \
             ORDER BY t.updated_at LIMIT $1",
        )
        .bind(limit)
        .bind(shards)
        .bind(exclude)
        .fetch_all(self.pool())
        .await
        .map_err(store_error)?
        .iter()
        .map(run_ref)
        .collect()
    }

    async fn stale_hints(
        &self,
        older_than: Duration,
        window: &Window,
    ) -> Result<Vec<StepRef>, StoreError> {
        let (limit, shards, exclude) = window_params(window);
        sqlx::query(
            "SELECT t.tenant_id, t.run_id, t.node, t.instance_path_nodes, \
                    t.instance_path_indices \
             FROM unnest($2::smallint[]) AS sh(s) \
             CROSS JOIN LATERAL ( \
                 SELECT tenant_id, run_id, node, instance_path_nodes, instance_path_indices, \
                        last_hinted_at \
                 FROM steps \
                 WHERE run_shard(run_id) = sh.s AND status = 'READY' \
                   AND ($5 OR cardinality(instance_path_nodes) = 0) \
                   AND hint_count < 5 \
                   AND last_hinted_at <= now() - $4 * interval '1 microsecond' \
                   AND tenant_id <> ALL($3::uuid[]) \
                 ORDER BY last_hinted_at LIMIT $1) t \
             ORDER BY t.last_hinted_at LIMIT $1",
        )
        .bind(limit)
        .bind(shards)
        .bind(exclude)
        .bind(micros(older_than))
        .bind(self.scoped_worker_protocol)
        .fetch_all(self.pool())
        .await
        .map_err(store_error)?
        .iter()
        .map(addressed_step_ref)
        .collect()
    }

    async fn pending_children(&self, window: &Window) -> Result<Vec<PendingChild>, StoreError> {
        let (limit, shards, exclude) = window_params(window);
        sqlx::query(
            "SELECT t.tenant_id, t.run_id, t.node, t.child_run_id, t.child_input, \
                    clock_timestamp() AS db_now \
             FROM unnest($2::smallint[]) AS sh(s) \
             CROSS JOIN LATERAL ( \
                 SELECT tenant_id, run_id, node, child_run_id, child_input, updated_at FROM steps \
                 WHERE run_shard(run_id) = sh.s AND status = 'WAITING' AND waiting_kind = 'CHILD' \
                   AND cardinality(instance_path_nodes) = 0 \
                   AND child_started = false AND child_input IS NOT NULL \
                   AND tenant_id <> ALL($3::uuid[]) \
                 ORDER BY updated_at LIMIT $1) t \
             ORDER BY t.updated_at LIMIT $1",
        )
        .bind(limit)
        .bind(shards)
        .bind(exclude)
        .fetch_all(self.pool())
        .await
        .map_err(store_error)?
        .iter()
        .map(|row| {
            Ok(PendingChild {
                step: step_ref(row)?,
                child: RunId::new(col::<Uuid>(row, "child_run_id")?),
                input: col(row, "child_input")?,
                as_of: col(row, "db_now")?,
            })
        })
        .collect()
    }

    async fn run_result(
        &self,
        tenant: TenantId,
        run: RunId,
    ) -> Result<Option<Vec<u8>>, StoreError> {
        sqlx::query_scalar(
            "SELECT p.content FROM runs r \
             JOIN payloads p ON p.tenant_id = r.tenant_id AND p.digest = r.result_digest \
             WHERE r.tenant_id = $1 AND r.id = $2",
        )
        .bind(tenant.as_uuid())
        .bind(run.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(store_error)
    }

    async fn ended_children(&self, window: &Window) -> Result<Vec<EndedChild>, StoreError> {
        let (limit, shards, exclude) = window_params(window);
        // The child that ended is told to its parent: the shard is the parent's. A child chain is
        // told once, by its final outcome, through its link (the runs of a chain are created
        // settled); the parent step must still be waiting for that chain.
        sqlx::query(
            "SELECT t.tenant_id, t.child, t.parent_run_id, t.parent_node, t.status \
             FROM unnest($2::smallint[]) AS sh(s) \
             CROSS JOIN LATERAL ( \
                 SELECT * FROM ( \
                     SELECT tenant_id, id AS child, parent_run_id, parent_node, status, \
                            finished_at FROM runs \
                     WHERE run_shard(parent_run_id) = sh.s AND parent_run_id IS NOT NULL \
                       AND parent_settled = false AND finished_at IS NOT NULL \
                       AND tenant_id <> ALL($3::uuid[]) \
                     ORDER BY finished_at LIMIT $1) plain \
                 UNION ALL \
                 SELECT * FROM ( \
                     SELECT link.tenant_id, link.child_chain_id AS child, link.parent_run_id, \
                            first.parent_node, link.final_outcome AS status, \
                            link.updated_at AS finished_at \
                     FROM child_chain_links link \
                     JOIN runs first ON first.tenant_id = link.tenant_id \
                                    AND first.id = link.child_chain_id \
                     WHERE run_shard(link.parent_run_id) = sh.s \
                       AND link.final_outcome IS NOT NULL AND link.settled = false \
                       AND link.tenant_id <> ALL($3::uuid[]) \
                       AND EXISTS ( \
                           SELECT 1 FROM steps parent_step \
                           WHERE parent_step.tenant_id = link.tenant_id \
                             AND parent_step.run_id = link.parent_run_id \
                             AND parent_step.step_id = link.parent_instance_id \
                             AND parent_step.status = 'WAITING' \
                             AND parent_step.waiting_kind = 'CHILD' \
                             AND parent_step.child_run_id = link.child_chain_id) \
                     ORDER BY link.updated_at LIMIT $1) chained \
                 ORDER BY finished_at LIMIT $1) t \
             ORDER BY t.finished_at LIMIT $1",
        )
        .bind(limit)
        .bind(shards)
        .bind(exclude)
        .fetch_all(self.pool())
        .await
        .map_err(store_error)?
        .iter()
        .map(|row| {
            let node: String = col(row, "parent_node")?;
            let status: String = col(row, "status")?;
            Ok(EndedChild {
                tenant_id: TenantId::new(col::<Uuid>(row, "tenant_id")?),
                child: RunId::new(col::<Uuid>(row, "child")?),
                parent: RunId::new(col::<Uuid>(row, "parent_run_id")?),
                node: NodeId::parse(&node).map_err(|_| corrupt("parent node"))?,
                status: status.parse().map_err(|_| corrupt("run status"))?,
            })
        })
        .collect()
    }

    async fn children_to_cancel(&self, window: &Window) -> Result<Vec<RunRef>, StoreError> {
        let (limit, shards, exclude) = window_params(window);
        // A run the parent asked to stop, and the current head of a child chain whose link the
        // parent latched (the head is the only run of the chain that is still unfinished).
        sqlx::query(
            "SELECT t.tenant_id, t.id FROM unnest($2::smallint[]) AS sh(s) \
             CROSS JOIN LATERAL ( \
                 SELECT tenant_id, id FROM ( \
                     SELECT tenant_id, id FROM runs \
                     WHERE run_shard(id) = sh.s AND parent_cancel AND finished_at IS NULL \
                       AND stop_reason IS NULL AND tenant_id <> ALL($3::uuid[]) \
                     UNION ALL \
                     SELECT run.tenant_id, run.id FROM child_chain_links link \
                     JOIN execution_chains chain ON chain.tenant_id = link.tenant_id \
                                                AND chain.chain_id = link.child_chain_id \
                     JOIN runs run ON run.tenant_id = chain.tenant_id \
                                  AND run.id = chain.current_run_id \
                     WHERE link.cancel_requested AND run_shard(run.id) = sh.s \
                       AND chain.final_outcome IS NULL AND run.finished_at IS NULL \
                       AND run.stop_reason IS NULL AND link.tenant_id <> ALL($3::uuid[])) due \
                 ORDER BY id LIMIT $1) t \
             ORDER BY t.id LIMIT $1",
        )
        .bind(limit)
        .bind(shards)
        .bind(exclude)
        .fetch_all(self.pool())
        .await
        .map_err(store_error)?
        .iter()
        .map(run_ref)
        .collect()
    }

    async fn purge_idempotency(&self, older_than: Duration) -> Result<u64, StoreError> {
        sqlx::query(
            "DELETE FROM command_dedupe WHERE created_at < now() - $1 * interval '1 microsecond'",
        )
        .bind(micros(older_than))
        .execute(self.pool())
        .await
        .map(|r| r.rows_affected())
        .map_err(store_error)
    }

    async fn purge_outbox(&self, older_than: Duration, limit: u32) -> Result<u64, StoreError> {
        sqlx::query(
            "DELETE FROM event_outbox WHERE id IN ( \
                 SELECT id FROM event_outbox \
                 WHERE delivered_at IS NOT NULL \
                   AND delivered_at < now() - $1 * interval '1 microsecond' \
                 ORDER BY delivered_at LIMIT $2)",
        )
        .bind(micros(older_than))
        .bind(i64::from(limit))
        .execute(self.pool())
        .await
        .map(|r| r.rows_affected())
        .map_err(store_error)
    }

    async fn purge_script_records(&self, limit: u32) -> Result<u64, StoreError> {
        self.purge_script_records_impl(limit).await
    }

    async fn purge_schedule_actions(&self, schedules: u32, rows: u32) -> Result<u64, StoreError> {
        // the runtime role may not delete an action; it may run this bounded function, which keeps the
        // last 1,000 and the last 30 days of each schedule and never a waiting action
        let removed: i64 = sqlx::query_scalar("SELECT schedule_purge_actions($1, $2)")
            .bind(i32::try_from(schedules).unwrap_or(i32::MAX))
            .bind(i32::try_from(rows).unwrap_or(i32::MAX))
            .fetch_one(self.pool())
            .await
            .map_err(store_error)?;
        Ok(u64::try_from(removed).unwrap_or(0))
    }

    async fn purge_business_ids(&self, rows: u32) -> Result<u64, StoreError> {
        // the runtime role may not delete a binding; it may run this bounded function, which removes
        // only those whose holder closed and whose window passed, or whose holder is gone
        let removed: i64 = sqlx::query_scalar("SELECT business_ids_purge($1)")
            .bind(i32::try_from(rows).unwrap_or(i32::MAX))
            .fetch_one(self.pool())
            .await
            .map_err(store_error)?;
        Ok(u64::try_from(removed).unwrap_or(0))
    }

    async fn purge_batch_jobs(&self, jobs: u32, rows: u32) -> Result<BatchPurged, StoreError> {
        // the runtime role may not delete a job or an item; it may run this bounded function, which removes
        // only a job that has finished and whose retention has passed, with its items, and never more than it
        // is given. The error is read without its text: the statement works over the selector and the reason
        // of a job, which are customer text, and the message of a database error can quote a value.
        let (removed_jobs, removed_items): (i64, i64) =
            sqlx::query_as("SELECT jobs_removed, items_removed FROM batch_jobs_purge($1, $2)")
                .bind(i32::try_from(jobs).unwrap_or(i32::MAX))
                .bind(i32::try_from(rows).unwrap_or(i32::MAX))
                .fetch_one(self.pool())
                .await
                .map_err(store_error_without_text)?;
        Ok(BatchPurged {
            jobs: u64::try_from(removed_jobs).unwrap_or(0),
            items: u64::try_from(removed_items).unwrap_or(0),
        })
    }

    async fn purge_sessions(&self) -> Result<u64, StoreError> {
        // Kept for an hour after expiry so a late report still finds its session's identity.
        sqlx::query("DELETE FROM worker_sessions WHERE expires_at < now() - interval '1 hour'")
            .execute(self.pool())
            .await
            .map(|r| r.rows_affected())
            .map_err(store_error)
    }
}

impl PgRunStore {
    /// Bounded by `limit` per table, so a backlog never holds a long statement.
    pub(crate) async fn purge_script_records_impl(&self, limit: u32) -> Result<u64, StoreError> {
        let executions = sqlx::query(
            "DELETE FROM script_executions WHERE id IN ( \
                 SELECT id FROM script_executions WHERE expires_at < now() \
                 ORDER BY expires_at LIMIT $1)",
        )
        .bind(i64::from(limit))
        .execute(self.pool())
        .await
        .map_err(store_error)?
        .rows_affected();
        let test_runs = sqlx::query(
            "DELETE FROM script_test_runs WHERE id IN ( \
                 SELECT id FROM script_test_runs \
                 WHERE finished_at IS NOT NULL \
                   AND finished_at + make_interval(days => retention_days) < now() \
                 ORDER BY finished_at LIMIT $1)",
        )
        .bind(i64::from(limit))
        .execute(self.pool())
        .await
        .map_err(store_error)?
        .rows_affected();
        Ok(executions + test_runs)
    }
}

impl OutboxStore for PgRunStore {
    async fn claim_outbox(
        &self,
        limit: u32,
        visibility: Duration,
    ) -> Result<Vec<OutboxMessage>, StoreError> {
        let rows = sqlx::query(
            "WITH due AS ( \
                 SELECT id FROM event_outbox \
                 WHERE delivered_at IS NULL AND next_attempt_at <= now() \
                 ORDER BY next_attempt_at LIMIT $1 FOR UPDATE SKIP LOCKED) \
             UPDATE event_outbox o \
                SET next_attempt_at = clock_timestamp() + $2 * interval '1 microsecond', \
                    attempt_count = o.attempt_count + 1 \
               FROM due WHERE o.id = due.id \
             RETURNING o.id, o.tenant_id, o.aggregate_id, o.event_type, o.routing_key, o.payload, \
                       o.attempt_count, o.created_at",
        )
        .bind(i64::from(limit))
        .bind(micros(visibility))
        .fetch_all(self.pool())
        .await
        .map_err(store_error)?;
        outbox_messages(&rows)
    }

    async fn due_outbox(&self, window: &Window) -> Result<Vec<OutboxDue>, StoreError> {
        let (limit, shards, exclude) = window_params(window);
        sqlx::query(
            "SELECT t.id, t.tenant_id FROM unnest($2::smallint[]) AS sh(s) \
             CROSS JOIN LATERAL ( \
                 SELECT id, tenant_id, next_attempt_at FROM event_outbox \
                 WHERE run_shard(aggregate_id) = sh.s AND delivered_at IS NULL \
                   AND next_attempt_at <= now() AND tenant_id <> ALL($3::uuid[]) \
                 ORDER BY next_attempt_at LIMIT $1) t \
             ORDER BY t.next_attempt_at LIMIT $1",
        )
        .bind(limit)
        .bind(shards)
        .bind(exclude)
        .fetch_all(self.pool())
        .await
        .map_err(store_error)?
        .iter()
        .map(|row| {
            Ok(OutboxDue {
                id: col(row, "id")?,
                tenant_id: TenantId::new(col::<Uuid>(row, "tenant_id")?),
            })
        })
        .collect()
    }

    async fn claim_outbox_ids(
        &self,
        ids: &[Uuid],
        visibility: Duration,
    ) -> Result<Vec<OutboxMessage>, StoreError> {
        // The conditions are asked again here: the message may have been delivered, or claimed by
        // another dispatcher, since it was looked at. A message locked by one is skipped.
        let rows = sqlx::query(
            "WITH due AS ( \
                 SELECT id FROM event_outbox \
                 WHERE id = ANY($1) AND delivered_at IS NULL AND next_attempt_at <= now() \
                 ORDER BY next_attempt_at FOR UPDATE SKIP LOCKED) \
             UPDATE event_outbox o \
                SET next_attempt_at = clock_timestamp() + $2 * interval '1 microsecond', \
                    attempt_count = o.attempt_count + 1 \
               FROM due WHERE o.id = due.id \
             RETURNING o.id, o.tenant_id, o.aggregate_id, o.event_type, o.routing_key, o.payload, \
                       o.attempt_count, o.created_at",
        )
        .bind(ids)
        .bind(micros(visibility))
        .fetch_all(self.pool())
        .await
        .map_err(store_error)?;
        outbox_messages(&rows)
    }

    async fn mark_delivered(&self, ids: &[Uuid]) -> Result<(), StoreError> {
        sqlx::query("UPDATE event_outbox SET delivered_at = clock_timestamp() WHERE id = ANY($1) AND delivered_at IS NULL")
            .bind(ids)
            .execute(self.pool())
            .await
            .map_err(store_error)?;
        Ok(())
    }

    async fn mark_failed(
        &self,
        id: Uuid,
        code: &str,
        retry_in: Duration,
    ) -> Result<(), StoreError> {
        sqlx::query(
            "UPDATE event_outbox \
                SET next_attempt_at = clock_timestamp() + $3 * interval '1 microsecond', last_error_code = $2 \
              WHERE id = $1 AND delivered_at IS NULL",
        )
        .bind(id)
        .bind(code)
        .bind(micros(retry_in))
        .execute(self.pool())
        .await
        .map_err(store_error)?;
        Ok(())
    }

    async fn outbox_backlog(&self) -> Result<u64, StoreError> {
        let n: i64 =
            sqlx::query_scalar("SELECT count(*) FROM event_outbox WHERE delivered_at IS NULL")
                .fetch_one(self.pool())
                .await
                .map_err(store_error)?;
        Ok(u64::try_from(n).unwrap_or(0))
    }
}

impl PgRunStore {
    /// [`MaintenanceStore::purge_rate_grants`] as of the database instant `now` (feature 022, transaction T25): a grant is deleted
    /// when `leased_at + max_period_ms + 1 minute < now`, so one leased exactly that long before `now` is kept. Public so
    /// that a test can place its rows on either side of the boundary; the pass passes the database clock.
    ///
    /// # Errors
    /// A store error.
    #[doc(hidden)]
    pub async fn purge_rate_grants_as_of(
        &self,
        now: OffsetDateTime,
        keys: u32,
        limit: u32,
    ) -> Result<u64, StoreError> {
        let candidates = sqlx::query(concat!(
            "SELECT DISTINCT g.tenant_id, g.namespace_id, g.pool, g.limit_scope, g.group_name, g.key_digest \
               FROM rate_limit_grants g \
               JOIN rate_limit_windows w ON w.tenant_id = g.tenant_id AND w.namespace_id = g.namespace_id \
                AND w.pool = g.pool AND w.limit_scope = g.limit_scope AND w.group_name = g.group_name \
                AND w.key_digest = g.key_digest \
              WHERE ",
            purgeable_grant!("g", "w", "$1"),
            " LIMIT $2"
        ))
        .bind(now)
        .bind(i64::from(keys))
        .fetch_all(self.pool())
        .await;
        let candidates = match candidates {
            Err(error) if absent_table(&error) => return Ok(0),
            rows => rows.map_err(store_error)?,
        };
        let mut deleted = 0u64;
        for row in &candidates {
            let tenant: Uuid = col(row, "tenant_id")?;
            let namespace: Uuid = col(row, "namespace_id")?;
            let pool: String = col(row, "pool")?;
            let scope: String = col(row, "limit_scope")?;
            let group: String = col(row, "group_name")?;
            let digest: Vec<u8> = col(row, "key_digest")?;
            let mut tx = self.pool().begin_safely().await.map_err(store_error)?;
            // the window first, as every transaction that changes a grant of the key (plan transaction T25; D022-038)
            let window: Option<i32> = sqlx::query_scalar(
                "SELECT max_period_ms FROM rate_limit_windows WHERE tenant_id = $1 AND namespace_id = $2 AND pool = $3 \
                 AND limit_scope = $4 AND group_name = $5 AND key_digest = $6 FOR UPDATE SKIP LOCKED",
            )
            .bind(tenant)
            .bind(namespace)
            .bind(&pool)
            .bind(&scope)
            .bind(&group)
            .bind(&digest)
            .fetch_optional(&mut *tx)
            .await
            .map_err(store_error)?;
            // a key another transaction holds (a decision, another replica's purge) is left to a later call
            let Some(_) = window else {
                tx.rollback().await.map_err(store_error)?;
                continue;
            };
            let gone = sqlx::query(concat!(
                "DELETE FROM rate_limit_grants g USING rate_limit_windows w \
                  WHERE w.tenant_id = $1 AND w.namespace_id = $2 AND w.pool = $3 AND w.limit_scope = $4 \
                    AND w.group_name = $5 AND w.key_digest = $6 \
                    AND g.tenant_id = w.tenant_id AND g.namespace_id = w.namespace_id AND g.pool = w.pool \
                    AND g.limit_scope = w.limit_scope AND g.group_name = w.group_name AND g.key_digest = w.key_digest \
                    AND (g.tenant_id, g.run_id, g.node, g.attempt) IN ( \
                        SELECT o.tenant_id, o.run_id, o.node, o.attempt FROM rate_limit_grants o \
                         WHERE o.tenant_id = w.tenant_id AND o.namespace_id = w.namespace_id AND o.pool = w.pool \
                           AND o.limit_scope = w.limit_scope AND o.group_name = w.group_name \
                           AND o.key_digest = w.key_digest AND ",
                purgeable_grant!("o", "w", "$7"),
                " LIMIT $8)"
            ))
            .bind(tenant)
            .bind(namespace)
            .bind(&pool)
            .bind(&scope)
            .bind(&group)
            .bind(&digest)
            .bind(now)
            .bind(i64::from(limit))
            .execute(&mut *tx)
            .await
            .map_err(store_error)?
            .rows_affected();
            tx.commit().await.map_err(store_error)?;
            deleted += gone;
        }
        Ok(deleted)
    }
}

#[cfg(test)]
mod tests {
    use sagat_application::batch::MIN_RETENTION;

    /// The purge of batch jobs keeps a job for `MIN_RETENTION` after it ended, and the number is written twice: in the constant the
    /// service stamps windows with and in the SQL of `batch_jobs_purge`. The integration suites that prove it run on a release, not on
    /// every pull request, so this one reads the text of the migration: a change of either side fails here (D2326, G2322).
    #[test]
    fn the_sql_of_the_purge_keeps_a_job_for_the_least_retention_the_service_stamps() {
        let sql = include_str!("../migrations/20270119000000_batch_purge.up.sql");
        let wanted = format!(
            "j.finished_at < now() - interval '{} days'",
            MIN_RETENTION.whole_days()
        );
        assert_eq!(
            MIN_RETENTION.whole_seconds() % 86_400,
            0,
            "the SQL names whole days"
        );
        assert!(
            sql.contains(&wanted),
            "the migration does not hold `{wanted}` for MIN_RETENTION"
        );
    }
}
