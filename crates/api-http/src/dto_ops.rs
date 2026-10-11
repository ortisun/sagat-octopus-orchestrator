//! The shapes of the operations routes: alerts, and what an operator is told about the platform
//! around their runs (FR-008-003, FR-008-005).

use sagat_application::operations::AlertRecord;
use serde::Serialize;
use time::OffsetDateTime;

/// An operational alert. Counts, ages and names; never data of a run.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AlertBody {
    pub id: String,
    pub namespace_id: String,
    pub kind: &'static str,
    pub subject: String,
    pub state: &'static str,
    pub severity: &'static str,
    pub summary: &'static str,
    /// Where to read what to do about it.
    pub runbook: String,
    #[serde(with = "time::serde::rfc3339")]
    pub first_seen_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub last_seen_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    pub resolved_at: Option<OffsetDateTime>,
    /// How many times it has begun to fire.
    pub fired_count: u32,
    /// How many things are in the condition at the last look.
    pub items: u64,
    /// The age of the oldest of them, in seconds.
    pub oldest_seconds: f64,
}

impl AlertBody {
    #[must_use]
    pub fn of(alert: &AlertRecord, runbook: String) -> Self {
        Self {
            id: alert.id.to_string(),
            namespace_id: alert.namespace_id.to_string(),
            kind: alert.kind.as_str(),
            subject: alert.subject.clone(),
            state: alert.state.as_str(),
            severity: alert.severity.as_str(),
            summary: alert.kind.summary(),
            runbook,
            first_seen_at: alert.first_seen_at,
            last_seen_at: alert.last_seen_at,
            resolved_at: alert.resolved_at,
            fired_count: alert.fired_count,
            items: alert.items,
            oldest_seconds: alert.oldest_seconds,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AlertPageBody {
    pub items: Vec<AlertBody>,
    pub next_cursor: Option<String>,
}

/// A worker identity serving a namespace.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkerBody {
    pub worker_id: String,
    pub subject: String,
    pub name: String,
    pub activities: Vec<String>,
    pub languages: Vec<String>,
    pub pools: Vec<String>,
    pub max_inflight: u32,
    /// Attempts it holds a lease on right now.
    pub in_flight: u32,
    #[serde(with = "time::serde::rfc3339")]
    pub registered_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub expires_at: OffsetDateTime,
    /// Its identity is being drained: it takes no new work, and what it holds goes on.
    pub draining: bool,
}

impl From<&sagat_application::operations::WorkerView> for WorkerBody {
    fn from(w: &sagat_application::operations::WorkerView) -> Self {
        Self {
            worker_id: w.id.to_string(),
            subject: w.subject.clone(),
            name: w.name.clone(),
            activities: w.activities.clone(),
            languages: w.languages.clone(),
            pools: w.pools.clone(),
            max_inflight: w.max_inflight,
            in_flight: w.in_flight,
            registered_at: w.registered_at,
            expires_at: w.expires_at,
            draining: w.draining,
        }
    }
}

/// Who stopped a worker identity from taking work, why and when.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DrainBody {
    pub subject: String,
    pub reason: String,
    pub drained_by: String,
    #[serde(with = "time::serde::rfc3339")]
    pub drained_at: OffsetDateTime,
}

impl From<&sagat_application::operations::DrainRecord> for DrainBody {
    fn from(d: &sagat_application::operations::DrainRecord) -> Self {
        Self {
            subject: d.subject.clone(),
            reason: d.reason.clone(),
            drained_by: d.drained_by.clone(),
            drained_at: d.drained_at,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkerListBody {
    pub items: Vec<WorkerBody>,
    /// The identities being drained in the namespace, whether or not a session of theirs is live.
    pub drains: Vec<DrainBody>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReasonBody {
    pub reason: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UndrainBody {
    /// Whether the identity was being drained.
    pub was_draining: bool,
}

/// One record of the audit trail. Free text an operator wrote is scrubbed of what has the shape of a
/// credential on the way out; the trail itself is kept as it was written.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditRecordBody {
    pub id: String,
    pub actor: String,
    pub action: String,
    pub resource_type: String,
    pub resource_id: String,
    pub reason: Option<String>,
    pub trace_id: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

impl From<&sagat_application::operations::AuditEntry> for AuditRecordBody {
    fn from(e: &sagat_application::operations::AuditEntry) -> Self {
        Self {
            id: e.id.to_string(),
            actor: e.actor.clone(),
            action: e.action.clone(),
            resource_type: e.resource_type.clone(),
            resource_id: e.resource_id.clone(),
            reason: e
                .reason
                .as_deref()
                .map(|r| sagat_telemetry::scrub(r).into_owned()),
            trace_id: e.trace_id.clone(),
            created_at: e.created_at,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditPageBody {
    pub items: Vec<AuditRecordBody>,
    pub next_cursor: Option<String>,
}
