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
