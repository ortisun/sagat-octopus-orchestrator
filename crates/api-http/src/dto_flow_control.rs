//! The flow-control fields of the API (feature 022, release 0.3.0 slice; `contracts/openapi-flow-control.yaml`):
//! `Run.concurrency` and the page of `GET /namespaces/{namespaceId}/concurrency-keys` (T022-026, FR-022-022).
//!
//! A key appears only as its 12-hex label (D1168); no body here can carry a key's value, because no type it is built
//! from holds one.

use sagat_application::flow_control_keys::{
    ConcurrencyKeyPage, ConcurrencyKeyView, RunPermit, RunPermitState,
};
use serde::Serialize;
use time::OffsetDateTime;

/// `Run.concurrency`: the run-level permit the run holds or waits for.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunConcurrencyBody {
    pub state: &'static str,
    pub pool: &'static str,
    pub group: String,
    pub key_label: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub since: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    pub queue_deadline: Option<OffsetDateTime>,
}

impl From<&RunPermit> for RunConcurrencyBody {
    fn from(permit: &RunPermit) -> Self {
        Self {
            state: permit.state.as_str(),
            pool: permit.key.pool.as_str(),
            group: permit.key.group.as_str().to_owned(),
            key_label: Some(permit.key.key_label.as_str().to_owned()),
            since: permit.since,
            queue_deadline: match permit.state {
                RunPermitState::Holding => None,
                RunPermitState::Waiting { deadline } => Some(deadline),
            },
        }
    }
}

/// One item of `ConcurrencyKeyPage`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConcurrencyKeyBody {
    pub pool: &'static str,
    pub scope: &'static str,
    pub group: String,
    pub key_label: Option<String>,
    pub holders: u32,
    pub waiters: u32,
    #[serde(with = "time::serde::rfc3339::option")]
    pub oldest_wait_since: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339::option")]
    pub oldest_holder_since: Option<OffsetDateTime>,
}

impl From<&ConcurrencyKeyView> for ConcurrencyKeyBody {
    fn from(key: &ConcurrencyKeyView) -> Self {
        Self {
            pool: key.pool.as_str(),
            scope: key.scope.as_str(),
            group: key.group.as_str().to_owned(),
            key_label: Some(key.label.as_str().to_owned()),
            holders: key.holders,
            waiters: key.waiters,
            oldest_wait_since: key.oldest_wait_since,
            oldest_holder_since: key.oldest_holder_since,
        }
    }
}

/// `ConcurrencyKeyPage`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConcurrencyKeyPageBody {
    pub queued_runs: u64,
    pub items: Vec<ConcurrencyKeyBody>,
    pub next_cursor: Option<String>,
}

impl ConcurrencyKeyPageBody {
    /// The page of at most `limit` keys out of `page`, which the store read with one more than `limit` to know
    /// whether another page follows.
    #[must_use]
    pub fn of(page: &ConcurrencyKeyPage, limit: usize) -> Self {
        let shown = &page.items[..page.items.len().min(limit)];
        Self {
            queued_runs: page.queued_runs,
            items: shown.iter().map(ConcurrencyKeyBody::from).collect(),
            next_cursor: if page.items.len() > limit {
                shown.last().map(|key| key.position.encode())
            } else {
                None
            },
        }
    }
}
