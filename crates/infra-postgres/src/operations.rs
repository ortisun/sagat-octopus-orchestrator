//! The monitor's store: what finds the conditions that need a person, keeps the alerts in step with
//! them, and totals the queues for the gauges (FR-008-003, FR-008-005).
//!
//! Every comparison uses `clock_timestamp()` on the database, so monitors on machines with different
//! clocks agree. The detection queries are aggregates that return counts and ages per namespace and
//! read nothing of a payload.

use std::time::Duration;

use sagat_application::error::StoreError;
use sagat_application::operations::{
    AlertReader, AlertRecord, AuditEntry, AuditFilter, AuditStore, Bounds, Candidate, Detection,
    DrainRecord, MonitorStore, Reconciled, Sample, Short, Thresholds, WorkerAdmin, WorkerView,
};
use sagat_domain::identity::{NamespaceId, TenantId};
use sagat_domain::operations::{AlertKind, AlertState, Severity};
use sagat_domain::outbound::OutboundKind;
use sqlx::Row;
use sqlx::postgres::PgRow;
use uuid::Uuid;

use crate::execution::PgRunStore;
use crate::identity::{col, corrupt, store_error};
use crate::pool::BeginSafely;

/// The name of the monitor's lease.
pub const MONITOR_LEASE: &str = "operations-monitor";

/// The audit read. Newest first by the columns of `audit_records_tenant_created_idx`, so that the
/// read of one tenant walks that tenant's records and not the table: ordering by the primary key
/// walks it backwards across every tenant until it has found a page.
pub const READ_AUDIT_SQL: &str = "SELECT id, actor, action, resource_type, resource_id, reason, trace_id, created_at \
     FROM audit_records \
     WHERE tenant_id = $1 \
       AND ($2::text IS NULL OR action = $2) \
       AND ($3::text IS NULL OR resource_type = $3) \
       AND ($4::text IS NULL OR resource_id = $4) \
       AND ($5::text IS NULL OR actor = $5) \
       AND ($6::timestamptz IS NULL OR created_at >= $6) \
       AND ($7::timestamptz IS NULL OR created_at < $7) \
       AND ($8::uuid IS NULL OR (created_at, id) < ( \
            SELECT c.created_at, c.id FROM audit_records c WHERE c.id = $8 AND c.tenant_id = $1)) \
       AND NOT (action = ANY($10)) \
     ORDER BY created_at DESC, id DESC LIMIT $9";

/// The key of the lock that couples a drain with the polls that lease for the identity.
pub(crate) fn drain_lock_key(tenant: TenantId, namespace: NamespaceId, subject: &str) -> String {
    format!("drain:{tenant}:{namespace}:{subject}")
}

fn secs(duration: Duration) -> f64 {
    duration.as_secs_f64()
}

fn candidate_from_row(kind: AlertKind, row: &PgRow) -> Result<Candidate, StoreError> {
    let items: i64 = col(row, "items")?;
    let subject: String = col(row, "subject")?;
    Ok(Candidate {
        tenant_id: TenantId::new(col::<Uuid>(row, "tenant_id")?),
        namespace_id: NamespaceId::new(col::<Uuid>(row, "namespace_id")?),
        kind,
        subject: Candidate::bounded_subject(&subject),
        items: u64::try_from(items).unwrap_or(0),
        oldest_seconds: col::<f64>(row, "oldest")?.max(0.0),
    })
}

fn alert_from_row(row: &PgRow) -> Result<AlertRecord, StoreError> {
    let kind: String = col(row, "kind")?;
    let state: String = col(row, "state")?;
    let severity: String = col(row, "severity")?;
    let items: i64 = col(row, "item_count")?;
    let fired: i32 = col(row, "fired_count")?;
    Ok(AlertRecord {
        id: col(row, "id")?,
        tenant_id: TenantId::new(col::<Uuid>(row, "tenant_id")?),
        namespace_id: NamespaceId::new(col::<Uuid>(row, "namespace_id")?),
        kind: kind.parse().map_err(|()| corrupt("alert kind"))?,
        subject: col(row, "subject")?,
        state: state.parse().map_err(|()| corrupt("alert state"))?,
        severity: match severity.as_str() {
            "warning" => Severity::Warning,
            "critical" => Severity::Critical,
            _ => return Err(corrupt("alert severity")),
        },
        first_seen_at: col(row, "first_seen_at")?,
        last_seen_at: col(row, "last_seen_at")?,
        resolved_at: col(row, "resolved_at")?,
        fired_count: u32::try_from(fired).map_err(|_| corrupt("alert count"))?,
        items: u64::try_from(items).unwrap_or(0),
        oldest_seconds: col(row, "oldest_seconds")?,
    })
}

const ALERT_COLUMNS: &str = "id, tenant_id, namespace_id, kind, subject, state, severity, first_seen_at, \
     last_seen_at, resolved_at, fired_count, item_count, oldest_seconds";

/// The body subscribers are sent for a change of an alert: identifiers, the kind, counts, ages and
/// the runbook; never data of a run.
fn event_body(
    alert: &AlertRecord,
    event: OutboundKind,
    runbook_base: &str,
    now: time::OffsetDateTime,
) -> Vec<u8> {
    let rfc3339 = |at: time::OffsetDateTime| {
        at.format(&time::format_description::well_known::Rfc3339)
            .unwrap_or_default()
    };
    serde_json::to_vec(&serde_json::json!({
        "id": format!("alert-{}-{}-{}", alert.id, alert.fired_count, event.as_str()),
        "type": event.as_str(),
        "time": rfc3339(now),
        "data": {
            "tenantId": alert.tenant_id.to_string(),
            "namespaceId": alert.namespace_id.to_string(),
            "alertId": alert.id.to_string(),
            "kind": alert.kind.as_str(),
            "subject": alert.subject,
            "severity": alert.severity.as_str(),
            "state": alert.state.as_str(),
            "firstSeenAt": rfc3339(alert.first_seen_at),
            "items": alert.items,
            "oldestSeconds": alert.oldest_seconds,
            "summary": alert.kind.summary(),
            "runbook": alert.runbook(runbook_base),
        }
    }))
    .unwrap_or_default()
}

impl MonitorStore for PgRunStore {
    async fn acquire_monitor(&self) -> Result<bool, StoreError> {
        self.acquire_lease(MONITOR_LEASE, Duration::from_secs(60))
            .await
    }

    async fn detect(
        &self,
        kind: AlertKind,
        thresholds: &Thresholds,
        bounds: &Bounds,
    ) -> Result<Detection, StoreError> {
        // The groups of the kind: a tenant, a namespace, what it is about, how many, how old.
        let (groups, uses_threshold) = match kind {
            AlertKind::OutboxStuck => (
                "SELECT g.tenant_id, g.ns AS namespace_id, ''::text AS subject, g.items, g.oldest \
                 FROM (SELECT o.tenant_id, o.ns, count(*) AS items, \
                              extract(epoch FROM clock_timestamp() - min(o.created_at))::float8 AS oldest \
                       FROM (SELECT tenant_id, created_at, \
                                    CASE WHEN payload->>'namespaceId' ~ '^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$' \
                                         THEN (payload->>'namespaceId')::uuid END AS ns \
                             FROM event_outbox \
                             WHERE delivered_at IS NULL \
                               AND created_at < clock_timestamp() - make_interval(secs => $1)) o \
                       JOIN namespaces n ON n.tenant_id = o.tenant_id AND n.id = o.ns \
                       GROUP BY o.tenant_id, o.ns) g",
                true,
            ),
            AlertKind::LeaseNotReaped => (
                "SELECT tenant_id, namespace_id, ''::text AS subject, count(*) AS items, \
                        extract(epoch FROM clock_timestamp() - min(lease_expires_at))::float8 AS oldest \
                 FROM steps \
                 WHERE status IN ('LEASED', 'RUNNING') \
                   AND lease_expires_at < clock_timestamp() - make_interval(secs => $1) \
                 GROUP BY tenant_id, namespace_id",
                true,
            ),
            // A step is "unclaimed" when a worker that served it would be given it: the conditions
            // are those of the claim (`CLAIMABLE`). A run that is paused, a forward step of a run
            // that is stopping, a compensation behind an open breaker are held back on purpose.
            AlertKind::StepUnclaimed => (
                "SELECT st.tenant_id, st.namespace_id, coalesce(st.activity, '') AS subject, count(*) AS items, \
                        extract(epoch FROM clock_timestamp() - min(st.ready_at))::float8 AS oldest \
                 FROM steps st \
                 JOIN runs r ON r.tenant_id = st.tenant_id AND r.id = st.run_id \
                 WHERE st.status = 'READY' \
                   AND st.ready_at < clock_timestamp() - make_interval(secs => $1) \
                   AND NOT r.paused \
                   AND CASE WHEN st.kind = 'COMPENSATION' \
                            THEN NOT EXISTS (SELECT 1 FROM compensation_breakers b \
                                 WHERE b.tenant_id = st.tenant_id AND b.namespace_id = st.namespace_id \
                                   AND b.activity = st.activity AND b.opened_until > clock_timestamp()) \
                            ELSE (r.stop_reason IS NULL OR st.kind <> 'FORWARD') END \
                 GROUP BY st.tenant_id, st.namespace_id, coalesce(st.activity, '')",
                true,
            ),
            AlertKind::ReviewPending => (
                "SELECT tenant_id, namespace_id, ''::text AS subject, count(*) AS items, \
                        extract(epoch FROM clock_timestamp() - min(updated_at))::float8 AS oldest \
                 FROM runs \
                 WHERE status = 'MANUAL_REVIEW' \
                   AND updated_at < clock_timestamp() - make_interval(secs => $1) \
                 GROUP BY tenant_id, namespace_id",
                true,
            ),
            AlertKind::DeadItems => (
                "SELECT tenant_id, namespace_id, subject, items, oldest FROM ( \
                     SELECT tenant_id, namespace_id, 'events'::text AS subject, count(*) AS items, \
                            extract(epoch FROM clock_timestamp() - min(received_at))::float8 AS oldest \
                     FROM inbox_events WHERE status = 'DEAD' GROUP BY tenant_id, namespace_id \
                     UNION ALL \
                     SELECT tenant_id, namespace_id, 'webhooks', count(*), \
                            extract(epoch FROM clock_timestamp() - min(created_at))::float8 \
                     FROM webhook_deliveries WHERE status = 'DEAD' GROUP BY tenant_id, namespace_id \
                     UNION ALL \
                     SELECT tenant_id, namespace_id, 'connectors', count(*), \
                            extract(epoch FROM clock_timestamp() - min(received_at))::float8 \
                     FROM connector_dead_letters GROUP BY tenant_id, namespace_id) d",
                false,
            ),
        };
        // The bounds are enforced where the rows are ranked: the oldest first within a namespace and
        // within a tenant, one row more than allowed so that it is known when a look stopped short,
        // and then the oldest of what is left across everyone. One tenant with thousands of groups
        // cannot fill the look and hide the others'.
        let first = if uses_threshold { 2 } else { 1 };
        let sql = format!(
            "SELECT tenant_id, namespace_id, subject, items, oldest, nr FROM ( \
                 SELECT d.*, row_number() OVER (PARTITION BY tenant_id ORDER BY oldest DESC) AS tr FROM ( \
                     SELECT g.*, row_number() OVER (PARTITION BY tenant_id, namespace_id ORDER BY oldest DESC) AS nr \
                     FROM ({groups}) g) d \
                 WHERE nr <= ${ns} + 1) t \
             WHERE tr <= ${tenant} + 1 \
             ORDER BY oldest DESC LIMIT ${total} + 1",
            ns = first,
            tenant = first + 1,
            total = first + 2,
        );
        let query = sqlx::query(sqlx::AssertSqlSafe(sql));
        let query = if uses_threshold {
            query.bind(secs(thresholds.of(kind)))
        } else {
            query
        };
        let rows = query
            .bind(i64::from(bounds.per_namespace))
            .bind(i64::from(bounds.per_tenant))
            .bind(i64::from(bounds.total))
            .fetch_all(self.pool())
            .await
            .map_err(store_error)?;

        let mut detection = Detection::default();
        let mut per_tenant: std::collections::HashMap<TenantId, u32> =
            std::collections::HashMap::new();
        let cut = |detection: &mut Detection, short: Short| {
            if !detection.short.contains(&short) {
                detection.short.push(short);
            }
        };
        // Rows come oldest first.
        for row in &rows {
            let candidate = candidate_from_row(kind, row)?;
            let rank: i64 = col(row, "nr")?;
            if rank > i64::from(bounds.per_namespace) {
                cut(
                    &mut detection,
                    Short::Namespace(candidate.tenant_id, candidate.namespace_id),
                );
                continue;
            }
            let held = per_tenant.entry(candidate.tenant_id).or_default();
            if *held >= bounds.per_tenant {
                cut(&mut detection, Short::Tenant(candidate.tenant_id));
                continue;
            }
            if detection.seen.len() >= bounds.total as usize {
                cut(&mut detection, Short::Everything);
                break;
            }
            *held += 1;
            detection.seen.push(candidate);
        }
        // The query itself stopped at its limit: there may be more that it did not return.
        if rows.len() > bounds.total as usize {
            cut(&mut detection, Short::Everything);
        }
        Ok(detection)
    }

    async fn reconcile(
        &self,
        kind: AlertKind,
        detection: &Detection,
        runbook_base: &str,
    ) -> Result<Reconciled, StoreError> {
        let seen = &detection.seen;
        let mut tx = self.pool().begin_safely().await.map_err(store_error)?;
        let started: time::OffsetDateTime = sqlx::query_scalar("SELECT clock_timestamp()")
            .fetch_one(&mut *tx)
            .await
            .map_err(store_error)?;
        let mut out = Reconciled::default();
        for candidate in seen.iter().filter(|c| c.kind == kind) {
            let previous: Option<String> = sqlx::query_scalar(
                "SELECT state FROM operational_alerts \
                 WHERE tenant_id = $1 AND namespace_id = $2 AND kind = $3 AND subject = $4 FOR UPDATE",
            )
            .bind(candidate.tenant_id.as_uuid())
            .bind(candidate.namespace_id.as_uuid())
            .bind(kind.as_str())
            .bind(&candidate.subject)
            .fetch_optional(&mut *tx)
            .await
            .map_err(store_error)?;
            let was_firing = previous.as_deref() == Some("FIRING");
            let row = sqlx::query(sqlx::AssertSqlSafe(format!(
                "INSERT INTO operational_alerts \
                   (id, tenant_id, namespace_id, kind, subject, state, severity, first_seen_at, last_seen_at, \
                    fired_count, item_count, oldest_seconds) \
                 VALUES ($1, $2, $3, $4, $5, 'FIRING', $6, clock_timestamp(), clock_timestamp(), 1, $7, $8) \
                 ON CONFLICT (tenant_id, namespace_id, kind, subject) DO UPDATE SET \
                   state = 'FIRING', resolved_at = NULL, last_seen_at = clock_timestamp(), \
                   item_count = EXCLUDED.item_count, oldest_seconds = EXCLUDED.oldest_seconds, \
                   first_seen_at = CASE WHEN operational_alerts.state = 'RESOLVED' \
                                        THEN clock_timestamp() ELSE operational_alerts.first_seen_at END, \
                   fired_count = operational_alerts.fired_count \
                                 + CASE WHEN operational_alerts.state = 'RESOLVED' THEN 1 ELSE 0 END \
                 RETURNING {ALERT_COLUMNS}"
            )))
            .bind(Uuid::now_v7())
            .bind(candidate.tenant_id.as_uuid())
            .bind(candidate.namespace_id.as_uuid())
            .bind(kind.as_str())
            .bind(&candidate.subject)
            .bind(kind.severity().as_str())
            .bind(i64::try_from(candidate.items).unwrap_or(i64::MAX))
            .bind(candidate.oldest_seconds)
            .fetch_one(&mut *tx)
            .await
            .map_err(store_error)?;
            if !was_firing {
                out.fired.push(alert_from_row(&row)?);
            }
        }
        // An alert that was not seen is resolved, except where the look did not see everything: an
        // alert beyond a bound is no less real for not having been returned.
        let everything = detection.short.contains(&Short::Everything);
        let tenants: Vec<Uuid> = detection
            .short
            .iter()
            .filter_map(|s| match s {
                Short::Tenant(t) => Some(t.as_uuid()),
                _ => None,
            })
            .collect();
        let (short_tenants, short_namespaces): (Vec<Uuid>, Vec<Uuid>) = detection
            .short
            .iter()
            .filter_map(|s| match s {
                Short::Namespace(t, n) => Some((t.as_uuid(), n.as_uuid())),
                _ => None,
            })
            .unzip();
        let rows = if everything {
            Vec::new()
        } else {
            sqlx::query(sqlx::AssertSqlSafe(format!(
                "UPDATE operational_alerts a SET state = 'RESOLVED', resolved_at = clock_timestamp() \
                 WHERE a.kind = $1 AND a.state = 'FIRING' AND a.last_seen_at < $2 \
                   AND a.tenant_id <> ALL($3::uuid[]) \
                   AND NOT EXISTS (SELECT 1 FROM unnest($4::uuid[], $5::uuid[]) AS s(tenant_id, namespace_id) \
                                   WHERE s.tenant_id = a.tenant_id AND s.namespace_id = a.namespace_id) \
                 RETURNING {ALERT_COLUMNS}"
            )))
            .bind(kind.as_str())
            .bind(started)
            .bind(&tenants)
            .bind(&short_tenants)
            .bind(&short_namespaces)
            .fetch_all(&mut *tx)
            .await
            .map_err(store_error)?
        };
        for row in &rows {
            out.resolved.push(alert_from_row(row)?);
        }
        out.firing = u64::try_from(
            sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM operational_alerts WHERE kind = $1 AND state = 'FIRING'",
            )
            .bind(kind.as_str())
            .fetch_one(&mut *tx)
            .await
            .map_err(store_error)?,
        )
        .unwrap_or(0);
        // Each change is announced to the namespace's subscribers in the same transaction.
        let now: time::OffsetDateTime = sqlx::query_scalar("SELECT clock_timestamp()")
            .fetch_one(&mut *tx)
            .await
            .map_err(store_error)?;
        for (alerts, event) in [
            (&out.fired, OutboundKind::AlertFired),
            (&out.resolved, OutboundKind::AlertResolved),
        ] {
            for alert in alerts {
                let body = event_body(alert, event, runbook_base, now);
                let event_id = format!(
                    "alert-{}-{}-{}",
                    alert.id,
                    alert.fired_count,
                    event.as_str()
                );
                crate::outbound::fan_out(
                    &mut tx,
                    alert.tenant_id,
                    alert.namespace_id,
                    event,
                    &event_id,
                    &body,
                    now,
                )
                .await?;
            }
        }
        tx.commit().await.map_err(store_error)?;
        Ok(out)
    }

    async fn sample(&self) -> Result<Sample, StoreError> {
        let pool = self.pool();
        let count_and_age = |sql: &'static str| async move {
            let row = sqlx::query(sqlx::AssertSqlSafe(sql))
                .fetch_one(pool)
                .await
                .map_err(store_error)?;
            Ok::<(u64, f64), StoreError>((
                u64::try_from(row.try_get::<i64, _>("n").map_err(store_error)?).unwrap_or(0),
                row.try_get::<f64, _>("oldest")
                    .map_err(store_error)?
                    .max(0.0),
            ))
        };
        let (outbox_backlog, outbox_oldest_seconds) = count_and_age(
            "SELECT count(*) AS n, coalesce(extract(epoch FROM clock_timestamp() - min(created_at)), 0)::float8 AS oldest \
             FROM event_outbox WHERE delivered_at IS NULL",
        )
        .await?;
        let (ready_steps, ready_oldest_seconds) = count_and_age(
            "SELECT count(*) AS n, coalesce(extract(epoch FROM clock_timestamp() - min(ready_at)), 0)::float8 AS oldest \
             FROM steps WHERE status = 'READY'",
        )
        .await?;
        let (inbox_pending, inbox_oldest_seconds) = count_and_age(
            "SELECT count(*) AS n, coalesce(extract(epoch FROM clock_timestamp() - min(received_at)), 0)::float8 AS oldest \
             FROM inbox_events WHERE status = 'PENDING'",
        )
        .await?;
        let leases = sqlx::query(
            "SELECT count(*) AS active, count(*) FILTER (WHERE lease_expires_at < clock_timestamp()) AS expired \
             FROM steps WHERE status IN ('LEASED', 'RUNNING')",
        )
        .fetch_one(pool)
        .await
        .map_err(store_error)?;
        let dead = sqlx::query(
            "SELECT (SELECT count(*) FROM inbox_events WHERE status = 'DEAD') AS events, \
                    (SELECT count(*) FROM webhook_deliveries WHERE status = 'DEAD') AS deliveries, \
                    (SELECT count(*) FROM connector_dead_letters) AS letters, \
                    (SELECT count(*) FROM worker_sessions WHERE expires_at > clock_timestamp()) AS workers",
        )
        .fetch_one(pool)
        .await
        .map_err(store_error)?;
        let runs = sqlx::query(
            "SELECT status, count(*) AS n FROM runs WHERE finished_at IS NULL GROUP BY status",
        )
        .fetch_all(pool)
        .await
        .map_err(store_error)?;
        let to_u64 = |row: &PgRow, name: &str| -> Result<u64, StoreError> {
            Ok(u64::try_from(row.try_get::<i64, _>(name).map_err(store_error)?).unwrap_or(0))
        };
        let mut runs_by_status = Vec::with_capacity(runs.len());
        let mut compensating_runs = 0;
        for row in &runs {
            let status: String = col(row, "status")?;
            let n = to_u64(row, "n")?;
            if status == "COMPENSATING" {
                compensating_runs = n;
            }
            runs_by_status.push((status, n));
        }
        Ok(Sample {
            outbox_backlog,
            outbox_oldest_seconds,
            ready_steps,
            ready_oldest_seconds,
            leases_active: to_u64(&leases, "active")?,
            leases_expired: to_u64(&leases, "expired")?,
            inbox_pending,
            inbox_oldest_seconds,
            dead_events: to_u64(&dead, "events")?,
            dead_deliveries: to_u64(&dead, "deliveries")?,
            dead_letters: to_u64(&dead, "letters")?,
            runs_by_status,
            compensating_runs,
            workers: to_u64(&dead, "workers")?,
        })
    }

    async fn purge_alerts(&self, retention: Duration, limit: u32) -> Result<u64, StoreError> {
        let removed = sqlx::query(
            "DELETE FROM operational_alerts WHERE id IN ( \
                 SELECT id FROM operational_alerts \
                 WHERE state = 'RESOLVED' AND resolved_at < clock_timestamp() - make_interval(secs => $1) \
                 LIMIT $2)",
        )
        .bind(secs(retention))
        .bind(i64::from(limit))
        .execute(self.pool())
        .await
        .map_err(store_error)?;
        Ok(removed.rows_affected())
    }
}

impl AlertReader for PgRunStore {
    async fn list_alerts(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        state: Option<AlertState>,
        before: Option<Uuid>,
        limit: u32,
    ) -> Result<Vec<AlertRecord>, StoreError> {
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "SELECT {ALERT_COLUMNS} FROM operational_alerts \
             WHERE tenant_id = $1 AND namespace_id = $2 \
               AND ($3::text IS NULL OR state = $3) AND ($4::uuid IS NULL OR id < $4) \
             ORDER BY id DESC LIMIT $5"
        )))
        .bind(tenant.as_uuid())
        .bind(namespace.as_uuid())
        .bind(state.map(AlertState::as_str))
        .bind(before)
        .bind(i64::from(limit))
        .fetch_all(self.pool())
        .await
        .map_err(store_error)?
        .iter()
        .map(alert_from_row)
        .collect()
    }
}

impl WorkerAdmin for PgRunStore {
    async fn list_workers(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        limit: u32,
    ) -> Result<Vec<WorkerView>, StoreError> {
        sqlx::query(
            "SELECT s.id, s.subject, s.worker_name, s.activities, s.languages, s.pools, s.max_inflight, \
                    s.created_at, s.expires_at, \
                    (SELECT count(*) FROM steps st WHERE st.tenant_id = s.tenant_id \
                        AND st.status IN ('LEASED', 'RUNNING') AND st.leased_by = s.id::text) AS in_flight, \
                    EXISTS (SELECT 1 FROM worker_drains d WHERE d.tenant_id = s.tenant_id \
                        AND d.namespace_id = s.namespace_id AND d.subject = s.subject) AS draining \
             FROM worker_sessions s \
             WHERE s.tenant_id = $1 AND s.namespace_id = $2 AND s.expires_at > clock_timestamp() \
             ORDER BY s.created_at DESC, s.id LIMIT $3",
        )
        .bind(tenant.as_uuid())
        .bind(namespace.as_uuid())
        .bind(i64::from(limit))
        .fetch_all(self.pool())
        .await
        .map_err(store_error)?
        .iter()
        .map(|row| {
            let max: i32 = col(row, "max_inflight")?;
            let held: i64 = col(row, "in_flight")?;
            Ok(WorkerView {
                id: col(row, "id")?,
                subject: col(row, "subject")?,
                name: col(row, "worker_name")?,
                activities: col(row, "activities")?,
                languages: col(row, "languages")?,
                pools: col(row, "pools")?,
                max_inflight: u32::try_from(max).map_err(|_| corrupt("max inflight"))?,
                in_flight: u32::try_from(held).unwrap_or(u32::MAX),
                registered_at: col(row, "created_at")?,
                expires_at: col(row, "expires_at")?,
                draining: col(row, "draining")?,
            })
        })
        .collect()
    }

    async fn list_drains(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
    ) -> Result<Vec<DrainRecord>, StoreError> {
        sqlx::query(
            "SELECT subject, reason, drained_by, drained_at FROM worker_drains \
             WHERE tenant_id = $1 AND namespace_id = $2 ORDER BY drained_at DESC LIMIT 500",
        )
        .bind(tenant.as_uuid())
        .bind(namespace.as_uuid())
        .fetch_all(self.pool())
        .await
        .map_err(store_error)?
        .iter()
        .map(|row| {
            Ok(DrainRecord {
                subject: col(row, "subject")?,
                reason: col(row, "reason")?,
                drained_by: col(row, "drained_by")?,
                drained_at: col(row, "drained_at")?,
            })
        })
        .collect()
    }

    async fn drain_worker(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        record: &DrainRecord,
        audit: &sagat_domain::audit::AuditRecord,
    ) -> Result<(), StoreError> {
        let mut tx = self.pool().begin_safely().await.map_err(store_error)?;
        // Waits for the polls that are deciding whether to lease for this identity, and keeps the
        // later ones out until it commits: none is handed work after this call returns.
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
            .bind(drain_lock_key(tenant, namespace, &record.subject))
            .execute(&mut *tx)
            .await
            .map_err(store_error)?;
        sqlx::query(
            "INSERT INTO worker_drains (tenant_id, namespace_id, subject, reason, drained_by) \
             VALUES ($1, $2, $3, $4, $5) \
             ON CONFLICT (tenant_id, namespace_id, subject) DO UPDATE \
               SET reason = EXCLUDED.reason, drained_by = EXCLUDED.drained_by, drained_at = clock_timestamp()",
        )
        .bind(tenant.as_uuid())
        .bind(namespace.as_uuid())
        .bind(&record.subject)
        .bind(&record.reason)
        .bind(&record.drained_by)
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        crate::identity::insert_audit(&mut tx, audit).await?;
        tx.commit().await.map_err(store_error)
    }

    async fn undrain_worker(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        subject: &str,
        audit: &sagat_domain::audit::AuditRecord,
    ) -> Result<bool, StoreError> {
        let mut tx = self.pool().begin_safely().await.map_err(store_error)?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
            .bind(drain_lock_key(tenant, namespace, subject))
            .execute(&mut *tx)
            .await
            .map_err(store_error)?;
        let removed = sqlx::query(
            "DELETE FROM worker_drains WHERE tenant_id = $1 AND namespace_id = $2 AND subject = $3",
        )
        .bind(tenant.as_uuid())
        .bind(namespace.as_uuid())
        .bind(subject)
        .execute(&mut *tx)
        .await
        .map_err(store_error)?
        .rows_affected()
            > 0;
        if removed {
            crate::identity::insert_audit(&mut tx, audit).await?;
        }
        tx.commit().await.map_err(store_error)?;
        Ok(removed)
    }
}

impl AuditStore for PgRunStore {
    async fn read_audit(
        &self,
        tenant: TenantId,
        filter: &AuditFilter,
        before: Option<Uuid>,
        limit: u32,
        reading: &sagat_domain::audit::AuditRecord,
    ) -> Result<Vec<AuditEntry>, StoreError> {
        let mut tx = self.pool().begin_safely().await.map_err(store_error)?;
        let rows = sqlx::query(READ_AUDIT_SQL)
            .bind(tenant.as_uuid())
            .bind(filter.action.as_deref())
            .bind(filter.resource_type.as_deref())
            .bind(filter.resource_id.as_deref())
            .bind(filter.actor.as_deref())
            .bind(filter.from)
            .bind(filter.to)
            .bind(before)
            .bind(i64::from(limit))
            .bind(
                sagat_domain::audit::AuditAction::WITHHELD_FROM_TENANT
                    .iter()
                    .map(|action| action.as_str())
                    .collect::<Vec<_>>(),
            )
            .fetch_all(&mut *tx)
            .await
            .map_err(store_error)?;
        // The reading is on the trail in the same transaction that reads it (and so is not in what it
        // read: the next reading sees it).
        crate::identity::insert_audit(&mut tx, reading).await?;
        tx.commit().await.map_err(store_error)?;
        rows.iter()
            .map(|row| {
                Ok(AuditEntry {
                    id: col(row, "id")?,
                    actor: col(row, "actor")?,
                    action: col(row, "action")?,
                    resource_type: col(row, "resource_type")?,
                    resource_id: col(row, "resource_id")?,
                    reason: col(row, "reason")?,
                    trace_id: col(row, "trace_id")?,
                    created_at: col(row, "created_at")?,
                })
            })
            .collect()
    }
}
