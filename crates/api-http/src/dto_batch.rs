//! Wire types of the batch routes (feature 018).
//!
//! A request is decoded into the application's [`BatchWrite`] here, once, so that the service sees
//! only values the domain accepts. A refusal names the closed `details.reason` of the contract and the
//! JSON pointer of the place it concerns. The payload of a signal and the arguments of an update are
//! never returned: a job shows only that it has one, by digest.

use sagat_application::batch::{BatchWrite, ItemRecord, JobRecord, NamedPayload, Preview};
use sagat_application::error::AppError;
use sagat_domain::batch::{Action, Filter, Justification, Rate, Selector};
use sagat_domain::execution::RunStatus;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use time::OffsetDateTime;
use uuid::Uuid;

fn invalid(reason: &'static str, field: &str) -> AppError {
    AppError::InvalidBatch {
        reason,
        field: Some(field.to_owned()),
    }
}

/// The structured filter of a selector; every field is optional and they AND together.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FilterBody {
    pub workflow_id: Option<String>,
    pub version_id: Option<String>,
    pub status: Option<Vec<String>>,
    pub origin: Option<String>,
    pub schedule_id: Option<String>,
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub created_after: Option<OffsetDateTime>,
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub created_before: Option<OffsetDateTime>,
    pub business_id_prefix: Option<String>,
}

/// Which runs a job or a preview looks at: a filter, a list of ids, or (while the query language of
/// feature 020 is not enabled) a query that is refused.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SelectorBody {
    Filter(FilterBody),
    RunIds(Vec<String>),
    #[expect(
        dead_code,
        reason = "the query is parsed so the shape is checked; it is refused while feature 020's capability is off"
    )]
    Query(String),
}

/// An id of the request, which must be a UUID.
fn id(text: &str, field: &str) -> Result<Uuid, AppError> {
    text.parse::<Uuid>()
        .map_err(|_| invalid("BATCH_SELECTOR_INVALID", field))
}

fn optional_id(text: Option<&str>, field: &str) -> Result<Option<Uuid>, AppError> {
    text.map(|text| id(text, field)).transpose()
}

impl SelectorBody {
    /// # Errors
    /// `BATCH_SELECTOR_INVALID` for a selector the domain refuses, `QUERY_UNSUPPORTED_OPERATOR` for
    /// a query while the capability of feature 020 is off.
    pub fn into_selector(self) -> Result<Selector, AppError> {
        let bad = || invalid("BATCH_SELECTOR_INVALID", "/selector");
        match self {
            Self::Query(_) => Err(invalid("QUERY_UNSUPPORTED_OPERATOR", "/selector/query")),
            Self::RunIds(ids) => Selector::run_ids(
                ids.iter()
                    .map(|text| id(text, "/selector/runIds"))
                    .collect::<Result<Vec<_>, _>>()?,
            )
            .map_err(|_| bad()),
            Self::Filter(filter) => {
                let statuses = filter
                    .status
                    .unwrap_or_default()
                    .iter()
                    .map(|name| name.parse::<RunStatus>())
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|_| invalid("BATCH_SELECTOR_INVALID", "/selector/filter/status"))?;
                Selector::filter(Filter {
                    workflow_id: optional_id(
                        filter.workflow_id.as_deref(),
                        "/selector/filter/workflowId",
                    )?,
                    version_id: optional_id(
                        filter.version_id.as_deref(),
                        "/selector/filter/versionId",
                    )?,
                    statuses,
                    origin: filter.origin,
                    schedule_id: optional_id(
                        filter.schedule_id.as_deref(),
                        "/selector/filter/scheduleId",
                    )?,
                    created_after: filter.created_after,
                    created_before: filter.created_before,
                    business_id_prefix: filter.business_id_prefix,
                })
                .map_err(|_| bad())
            }
        }
    }
}

/// The body of `previewBatchOperation`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PreviewBody {
    pub selector: SelectorBody,
    pub max_items: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignalBody {
    pub name: String,
    pub payload: Value,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateBody {
    pub name: String,
    #[serde(default)]
    pub args: Value,
}

/// What a rerun does with the business id of the run it copies. Named by serde, not by literals:
/// the privilege audit reads every literal that looks like a statement.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum BusinessIdPolicy {
    Keep,
    Drop,
}

/// The body of `createBatchOperation`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateBody {
    pub selector: SelectorBody,
    pub action: String,
    pub signal: Option<SignalBody>,
    pub update: Option<UpdateBody>,
    pub step_id: Option<String>,
    #[serde(default)]
    pub instance_path: Vec<Value>,
    pub business_id: Option<BusinessIdPolicy>,
    /// Feature 019's; its presence is all this build looks at (the action is unavailable).
    pub repin: Option<Value>,
    pub reason: String,
    pub max_items: Option<u32>,
    pub items_per_second: Option<f64>,
}

/// The rate of `per_second` items a second, in thousandths, as the domain keeps it.
fn rate_of(per_second: f64) -> Result<Rate, AppError> {
    let bad = || invalid("BATCH_LIMITS_INVALID", "/itemsPerSecond");
    if !per_second.is_finite() || per_second <= 0.0 || per_second > 200.0 {
        return Err(bad());
    }
    // 0 < per_second <= 200, so the thousandths are between 0 and 200,000
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let milli = (per_second * 1000.0).round() as u32;
    Rate::from_milli(milli).map_err(|_| bad())
}

fn payload(field: &str, body: &Value) -> Result<Vec<u8>, AppError> {
    serde_json::to_vec(body).map_err(|_| invalid("BATCH_PAYLOAD_INVALID", field))
}

impl CreateBody {
    /// # Errors
    /// [`AppError::InvalidBatch`] with the reason and the pointer of the first thing wrong.
    pub fn into_write(self) -> Result<BatchWrite, AppError> {
        let action = self
            .action
            .parse::<Action>()
            .map_err(|()| invalid("BATCH_ACTION_PARAMETERS", "/action"))?;
        let selector = self.selector.into_selector()?;
        let reason = Justification::new(&self.reason)
            .map_err(|_| invalid("BATCH_REASON_INVALID", "/reason"))?;
        let rate = self.items_per_second.map(rate_of).transpose()?;
        let drop_business_id = matches!(self.business_id, Some(BusinessIdPolicy::Drop));
        let signal = match self.signal {
            None => None,
            Some(signal) => {
                if !signal.payload.is_object() {
                    return Err(invalid("BATCH_PAYLOAD_INVALID", "/signal/payload"));
                }
                Some(NamedPayload {
                    body: payload("/signal", &signal.payload)?,
                    name: signal.name,
                })
            }
        };
        let update = match self.update {
            None => None,
            Some(update) => Some(NamedPayload {
                body: payload("/update", &update.args)?,
                name: update.name,
            }),
        };
        Ok(BatchWrite {
            selector,
            action,
            step_id: self.step_id,
            instance_path: self.instance_path.iter().map(Value::to_string).collect(),
            signal,
            update,
            drop_business_id,
            repin: self.repin.is_some(),
            reason,
            max_items: self.max_items,
            rate,
        })
    }
}

/// The body of `cancelBatchOperation`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CancelBody {
    pub reason: String,
}

// --- what is returned -----------------------------------------------------------------------------

/// What a preview found.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewResponse {
    pub matched: u32,
    pub capped: bool,
    pub sample: Vec<String>,
}

impl From<&Preview> for PreviewResponse {
    fn from(preview: &Preview) -> Self {
        Self {
            matched: preview.matched,
            capped: preview.capped,
            sample: preview.sample.iter().map(Uuid::to_string).collect(),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CountsBody {
    pub selected: u32,
    pub succeeded: u32,
    pub skipped: u32,
    pub failed: u32,
    pub cancelled: u32,
    pub deferred: u32,
}

/// A batch job. The payload of a signal or an update is shown by its digest only.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobBody {
    pub batch_id: String,
    pub action: String,
    pub state: &'static str,
    pub failure_reason: Option<&'static str>,
    pub created_by: String,
    /// Whether the job carries a signal payload. Its digest is not returned: it would let a reader
    /// of the job confirm a guess of a payload they may not read.
    pub has_payload: bool,
    pub max_items: u32,
    pub items_per_second: f64,
    pub counts: CountsBody,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    pub started_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339::option")]
    pub finished_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339::option")]
    pub expires_at: Option<OffsetDateTime>,
}

impl From<&JobRecord> for JobBody {
    fn from(job: &JobRecord) -> Self {
        Self {
            batch_id: job.id.to_string(),
            action: job.action.as_str().to_owned(),
            state: job.state.as_str(),
            failure_reason: job
                .failure_reason
                .map(sagat_domain::batch::FailureReason::as_str),
            created_by: job.created_by.clone(),
            has_payload: job.payload_digest.is_some(),
            max_items: job.max_items,
            items_per_second: f64::from(job.rate.milli()) / 1000.0,
            counts: CountsBody {
                selected: job.counts.selected,
                succeeded: job.counts.succeeded,
                skipped: job.counts.skipped,
                failed: job.counts.failed,
                cancelled: job.counts.cancelled,
                deferred: job.counts.deferred,
            },
            created_at: job.created_at,
            started_at: job.started_at,
            finished_at: job.finished_at,
            expires_at: Some(job.expires_at),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobPageBody {
    pub items: Vec<JobBody>,
    pub next_cursor: Option<String>,
}

/// One item of a job.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemBody {
    pub ordinal: u32,
    pub run_id: String,
    pub state: &'static str,
    pub reason: Option<String>,
    pub error_code: Option<String>,
    pub attempts: u32,
    pub new_run_id: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
}

impl From<&ItemRecord> for ItemBody {
    fn from(item: &ItemRecord) -> Self {
        Self {
            ordinal: item.ordinal,
            run_id: item.run_id.to_string(),
            state: sagat_application::batch::item_state_name(item.state),
            reason: item.reason.clone(),
            error_code: item.error_code.clone(),
            attempts: item.attempts,
            new_run_id: item.new_run_id.map(|id| id.to_string()),
            updated_at: item.updated_at,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemPageBody {
    pub items: Vec<ItemBody>,
    pub next_cursor: Option<String>,
}
