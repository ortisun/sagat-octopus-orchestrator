//! The flow-control fields of the API (feature 022, release 0.3.0 slice; `contracts/openapi-flow-control.yaml`):
//! `Run.concurrency` and the page of `GET /namespaces/{namespaceId}/concurrency-keys` (T022-026, FR-022-022).
//!
//! A key appears only as its 12-hex label (D1168), and only to a caller who may reveal personal data in the namespace
//! (`keyLabel` is `null` for every other caller, FR-022-057, D022-060); no body here can carry a key's value, because no
//! type it is built from holds one.

use sagat_application::flow_control_keys::{
    ConcurrencyKeyPage, ConcurrencyKeyView, KeyPosition, RunPermit, RunPermitState,
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
            pool: permit.pool.as_str(),
            group: permit.group.as_str().to_owned(),
            key_label: permit.label().map(|label| label.as_str().to_owned()),
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
            key_label: key.label.as_ref().map(|label| label.as_str().to_owned()),
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
    /// whether another page follows; `cursor` seals the position of the page's last key when one does.
    ///
    /// # Errors
    /// What `cursor` returns.
    pub fn of<E>(
        page: &ConcurrencyKeyPage,
        limit: usize,
        cursor: impl FnOnce(&KeyPosition) -> Result<String, E>,
    ) -> Result<Self, E> {
        let shown = &page.items[..page.items.len().min(limit)];
        let next_cursor = match shown.last() {
            Some(last) if page.items.len() > limit => Some(cursor(&last.position)?),
            _ => None,
        };
        Ok(Self {
            queued_runs: page.queued_runs,
            items: shown.iter().map(ConcurrencyKeyBody::from).collect(),
            next_cursor,
        })
    }
}
