//! Batch jobs over many runs: preview, create, inspect, cancel and list the items, reached from
//! [`Client::batch`]. A job is durable; a request sent again with the same key and body is
//! answered with the first job.
use super::wire::{
    Fields, Moment, body_of, id_segment, namespace_path, push_page, request, summary_debug, with,
};
use super::*;
use crate::Page;
use serde::{Deserialize, Serialize};

/// The runs a batch job selects by their properties. `created_after` and `created_before` are
/// sent in UTC.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchFilter {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub status: Vec<crate::RunStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schedule_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_after: Option<Moment>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_before: Option<Moment>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub business_id_prefix: Option<String>,
}

/// How a batch job selects its runs: by properties, by id, or by a query.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BatchSelector {
    Filter(BatchFilter),
    RunIds(Vec<String>),
    Query(String),
}
impl BatchSelector {
    /// Text that is not an instant is refused before anything is sent.
    fn check(&self) -> Result<(), SagatError> {
        if let Self::Filter(filter) = self {
            if let Some(at) = &filter.created_after {
                at.utc_text("createdAfter")?;
            }
            if let Some(at) = &filter.created_before {
                at.utc_text("createdBefore")?;
            }
        }
        Ok(())
    }
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct BatchSignal {
    pub name: String,
    pub payload: Value,
}
impl std::fmt::Debug for BatchSignal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BatchSignal")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct BatchUpdate {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args: Option<Value>,
}
impl std::fmt::Debug for BatchUpdate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BatchUpdate")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

/// A batch job to create. `action` is `CANCEL`, `SIGNAL`, `RERUN`, `RETRY_STEP`, `PAUSE`, `RESUME`,
/// `MARK_MANUAL_REVIEW`, ...: the set only grows, so it is the text the server knows.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchRequest {
    pub selector: BatchSelector,
    pub action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signal: Option<BatchSignal>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub update: Option<BatchUpdate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub business_id: Option<BusinessIdDisposition>,
    pub reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_items: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Items per second the job admits; the server judges the range (more than 0, at most 200).
    pub items_per_second: Option<f64>,
}
impl BatchRequest {
    pub fn new(
        selector: BatchSelector,
        action: impl Into<String>,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            selector,
            action: action.into(),
            signal: None,
            update: None,
            step_id: None,
            business_id: None,
            reason: reason.into(),
            max_items: None,
            items_per_second: None,
        }
    }
}

/// What a selector matches, up to `max_items`.
#[derive(Clone)]
pub struct BatchPreview {
    pub matched: u64,
    pub capped: bool,
    pub sample: Vec<Uuid>,
    pub raw: Value,
}
summary_debug!(BatchPreview { matched, capped });

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BatchJobCounts {
    pub selected: u64,
    pub succeeded: u64,
    pub skipped: u64,
    pub failed: u64,
    pub cancelled: u64,
    pub deferred: u64,
}

#[derive(Clone)]
pub struct BatchJob {
    pub batch_id: Uuid,
    pub action: String,
    pub state: String,
    pub failure_reason: Option<String>,
    pub created_by: String,
    pub has_payload: bool,
    pub max_items: Option<u64>,
    pub items_per_second: Option<f64>,
    pub counts: BatchJobCounts,
    pub created_at: OffsetDateTime,
    pub started_at: Option<OffsetDateTime>,
    pub finished_at: Option<OffsetDateTime>,
    pub expires_at: Option<OffsetDateTime>,
    pub raw: Value,
}
summary_debug!(BatchJob {
    batch_id,
    action,
    state
});
impl BatchJob {
    fn parse(raw: Value) -> Result<Self, SagatError> {
        Self::read(&raw)
    }
    fn read(raw: &Value) -> Result<Self, SagatError> {
        let f = Fields::of(raw, "a batch job")?;
        let counts = f.object("counts");
        let count = |key: &str| counts.as_ref().map_or(0, |c| c.count(key));
        Ok(Self {
            batch_id: f.uuid("batchId")?,
            action: f.text("action")?,
            state: f.text("state")?,
            failure_reason: f.opt_text("failureReason"),
            created_by: f.text("createdBy")?,
            has_payload: f.flag("hasPayload"),
            max_items: f.opt_u64("maxItems"),
            items_per_second: f.opt_f64("itemsPerSecond"),
            counts: BatchJobCounts {
                selected: count("selected"),
                succeeded: count("succeeded"),
                skipped: count("skipped"),
                failed: count("failed"),
                cancelled: count("cancelled"),
                deferred: count("deferred"),
            },
            created_at: f.instant("createdAt")?,
            started_at: f.opt_instant("startedAt")?,
            finished_at: f.opt_instant("finishedAt")?,
            expires_at: f.opt_instant("expiresAt")?,
            raw: raw.clone(),
        })
    }
}

/// What became of one run of a batch job. `state` and `reason` are kept as the strings they came
/// as: an unknown value is never an error.
#[derive(Clone)]
pub struct BatchItem {
    pub ordinal: u64,
    pub run_id: Uuid,
    pub state: String,
    pub reason: Option<String>,
    pub error_code: Option<String>,
    pub attempts: u64,
    pub new_run_id: Option<Uuid>,
    pub raw: Value,
}
summary_debug!(BatchItem {
    ordinal,
    run_id,
    state
});
impl BatchItem {
    fn read(raw: &Value) -> Result<Self, SagatError> {
        let f = Fields::of(raw, "a batch item")?;
        Ok(Self {
            ordinal: f.at_least("ordinal", 0)?,
            run_id: f.uuid("runId")?,
            state: f.text("state")?,
            reason: f.opt_text("reason"),
            error_code: f.opt_text("errorCode"),
            attempts: f.count("attempts"),
            new_run_id: f.opt_uuid("newRunId")?,
            raw: raw.clone(),
        })
    }
}

/// The filters of `list` and `list_items`: a state, and the page.
#[derive(Clone, Debug, Default)]
pub struct BatchListFilter {
    pub state: Option<String>,
    pub limit: Option<u16>,
    pub cursor: Option<String>,
}
impl BatchListFilter {
    fn query(&self) -> Result<Vec<(String, String)>, SagatError> {
        let mut query = Vec::new();
        push_page(&mut query, self.limit, self.cursor.as_deref())?;
        if let Some(state) = &self.state {
            query.push(("state".into(), state.clone()));
        }
        Ok(query)
    }
}

/// Batch jobs of a namespace, reached from [`Client::batch`].
#[derive(Clone, Copy)]
pub struct BatchOperations<'a> {
    client: &'a Client,
}
impl std::fmt::Debug for BatchOperations<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BatchOperations").finish_non_exhaustive()
    }
}
impl Client {
    /// Batch jobs over many runs: preview, create, inspect, cancel and list the items.
    pub fn batch(&self) -> BatchOperations<'_> {
        BatchOperations { client: self }
    }
}

fn path(namespace_id: &str, tail: &[&str]) -> Result<Vec<String>, SagatError> {
    Ok(with(
        &namespace_path(namespace_id)?,
        &[&["batch-operations"], tail].concat(),
    ))
}
fn job_path(namespace_id: &str, batch_id: &str, tail: &[&str]) -> Result<Vec<String>, SagatError> {
    let mut path = path(namespace_id, &[])?;
    path.push(id_segment(batch_id, "a batch id")?);
    path.extend(tail.iter().map(|s| (*s).to_owned()));
    Ok(path)
}

impl BatchOperations<'_> {
    /// Counts what a selector matches, up to `max_items`; changes nothing.
    pub async fn preview(
        &self,
        namespace_id: &str,
        selector: &BatchSelector,
        max_items: Option<u64>,
        call: CallOptions,
    ) -> Result<BatchPreview, SagatError> {
        selector.check()?;
        let mut body = json!({"selector":body_of(selector)?});
        if let Some(max) = max_items {
            body["maxItems"] = json!(max);
        }
        let req = request(
            Method::POST,
            &path(namespace_id, &["preview"])?,
            200,
            Vec::new(),
        )
        .json(&body)?
        .repeatable();
        self.client.execute(req, call).await?.parse(|raw| {
            let f = Fields::of(&raw, "a batch preview")?;
            Ok(BatchPreview {
                matched: f.at_least("matched", 0)?,
                capped: f.flag("capped"),
                sample: f.uuids("sample")?,
                raw: raw.clone(),
            })
        })
    }

    /// Creates a durable job. A request sent again with the same key and body is answered with the
    /// first job.
    pub async fn create(
        &self,
        namespace_id: &str,
        request_body: &BatchRequest,
        call: CallOptions,
    ) -> Result<BatchJob, SagatError> {
        request_body.selector.check()?;
        let body = body_of(request_body)?;
        let req = request(Method::POST, &path(namespace_id, &[])?, 202, Vec::new())
            .json(&body)?
            .keyed(key(&call)?);
        self.client.execute(req, call).await?.parse(BatchJob::parse)
    }

    pub async fn get(
        &self,
        namespace_id: &str,
        batch_id: &str,
        call: CallOptions,
    ) -> Result<BatchJob, SagatError> {
        let req = request(
            Method::GET,
            &job_path(namespace_id, batch_id, &[])?,
            200,
            Vec::new(),
        );
        self.client.execute(req, call).await?.parse(BatchJob::parse)
    }

    pub async fn list(
        &self,
        namespace_id: &str,
        filter: BatchListFilter,
        call: CallOptions,
    ) -> Result<Page<BatchJob>, SagatError> {
        let req = request(Method::GET, &path(namespace_id, &[])?, 200, filter.query()?);
        self.client
            .execute(req, call)
            .await?
            .parse(|raw| wire::page_of(raw, BatchJob::read))
    }

    /// Stops admitting items; what was applied stays applied. A job that already ended is `CONFLICT`.
    pub async fn cancel(
        &self,
        namespace_id: &str,
        batch_id: &str,
        reason: &str,
        call: CallOptions,
    ) -> Result<BatchJob, SagatError> {
        let req = request(
            Method::POST,
            &job_path(namespace_id, batch_id, &["cancel"])?,
            202,
            Vec::new(),
        )
        .json(&json!({"reason":reason}))?
        .keyed(key(&call)?);
        self.client.execute(req, call).await?.parse(BatchJob::parse)
    }

    pub async fn list_items(
        &self,
        namespace_id: &str,
        batch_id: &str,
        filter: BatchListFilter,
        call: CallOptions,
    ) -> Result<Page<BatchItem>, SagatError> {
        let req = request(
            Method::GET,
            &job_path(namespace_id, batch_id, &["items"])?,
            200,
            filter.query()?,
        );
        self.client
            .execute(req, call)
            .await?
            .parse(|raw| wire::page_of(raw, BatchItem::read))
    }
}
