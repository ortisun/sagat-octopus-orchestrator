//! Typed public responses retain the complete additive JSON document.
use crate::SagatError;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use time::{OffsetDateTime, UtcOffset};
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RunStatus {
    Running,
    Waiting,
    Paused,
    Cancelling,
    Compensating,
    ManualReview,
    Succeeded,
    Failed,
    Cancelled,
    TimedOut,
    Compensated,
    Partial,
    CompensationFailed,
    Unknown(String),
}
impl RunStatus {
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Succeeded
                | Self::Failed
                | Self::Cancelled
                | Self::TimedOut
                | Self::Compensated
                | Self::Partial
                | Self::CompensationFailed
        )
    }
    pub fn as_str(&self) -> &str {
        match self {
            Self::Running => "RUNNING",
            Self::Waiting => "WAITING",
            Self::Paused => "PAUSED",
            Self::Cancelling => "CANCELLING",
            Self::Compensating => "COMPENSATING",
            Self::ManualReview => "MANUAL_REVIEW",
            Self::Succeeded => "SUCCEEDED",
            Self::Failed => "FAILED",
            Self::Cancelled => "CANCELLED",
            Self::TimedOut => "TIMED_OUT",
            Self::Compensated => "COMPENSATED",
            Self::Partial => "PARTIAL",
            Self::CompensationFailed => "COMPENSATION_FAILED",
            Self::Unknown(value) => value,
        }
    }
}
impl From<String> for RunStatus {
    fn from(value: String) -> Self {
        match value.as_str() {
            "RUNNING" => Self::Running,
            "WAITING" => Self::Waiting,
            "PAUSED" => Self::Paused,
            "CANCELLING" => Self::Cancelling,
            "COMPENSATING" => Self::Compensating,
            "MANUAL_REVIEW" => Self::ManualReview,
            "SUCCEEDED" => Self::Succeeded,
            "FAILED" => Self::Failed,
            "CANCELLED" => Self::Cancelled,
            "TIMED_OUT" => Self::TimedOut,
            "COMPENSATED" => Self::Compensated,
            "PARTIAL" => Self::Partial,
            "COMPENSATION_FAILED" => Self::CompensationFailed,
            _ => Self::Unknown(value),
        }
    }
}
impl<'de> Deserialize<'de> for RunStatus {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(Self::from)
    }
}
impl Serialize for RunStatus {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CompletionKind {
    Continued,
    Final,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChainSegment {
    pub chain_id: Uuid,
    pub segment_index: u32,
    #[serde(deserialize_with = "required_nullable")]
    pub completion_kind: Option<CompletionKind>,
    #[serde(deserialize_with = "required_nullable")]
    pub successor_run_id: Option<Uuid>,
}
fn required_nullable<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer)
}
/// Scope dimensions are ordered; they are never flattened into a node identifier.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ScopeSegment {
    pub scope_node_id: String,
    pub iteration_index: u32,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunParent {
    pub run_id: Uuid,
    pub node: String,
    pub cancel_policy: String,
}
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Run {
    #[serde(rename = "runId")]
    pub id: Uuid,
    pub workflow_id: Uuid,
    pub version_id: Uuid,
    pub status: RunStatus,
    pub version: u64,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub updated_at: Option<OffsetDateTime>,
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub finished_at: Option<OffsetDateTime>,
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub deadline: Option<OffsetDateTime>,
    pub correlation_id: Option<String>,
    pub trace_id: Option<String>,
    pub started_by: Option<String>,
    pub paused: Option<bool>,
    pub rerun_of: Option<Uuid>,
    #[serde(default)]
    pub depth: u32,
    pub parent: Option<RunParent>,
    pub chain: Option<ChainSegment>,
    #[serde(skip)]
    pub raw: Value,
}
impl std::fmt::Debug for Run {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Run")
            .field("id", &self.id)
            .field("status", &self.status)
            .field("version", &self.version)
            .finish_non_exhaustive()
    }
}
impl Run {
    pub(crate) fn parse(raw: Value) -> Result<Self, SagatError> {
        let mut run: Self = serde_json::from_value(raw.clone()).map_err(|_| protocol())?;
        if run.depth > 8
            || run.version > i64::MAX as u64
            || run
                .chain
                .as_ref()
                .is_some_and(|chain| chain.segment_index > 9999)
        {
            return Err(protocol());
        }
        run.created_at = run.created_at.to_offset(UtcOffset::UTC);
        run.updated_at = run.updated_at.map(|v| v.to_offset(UtcOffset::UTC));
        run.finished_at = run.finished_at.map(|v| v.to_offset(UtcOffset::UTC));
        run.deadline = run.deadline.map(|v| v.to_offset(UtcOffset::UTC));
        run.raw = raw;
        Ok(run)
    }
}
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationResult {
    pub operation_id: Uuid,
    pub accepted: bool,
    pub run_status: Option<RunStatus>,
    pub run_version: Option<u64>,
    pub new_run_id: Option<Uuid>,
    #[serde(skip)]
    pub raw: Value,
}
impl std::fmt::Debug for OperationResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OperationResult")
            .field("operation_id", &self.operation_id)
            .field("accepted", &self.accepted)
            .finish_non_exhaustive()
    }
}
impl OperationResult {
    pub(crate) fn parse(raw: Value) -> Result<Self, SagatError> {
        let mut result: Self = serde_json::from_value(raw.clone()).map_err(|_| protocol())?;
        if result.run_version.is_some_and(|v| v > i64::MAX as u64) {
            return Err(protocol());
        }
        result.raw = raw;
        Ok(result)
    }
}
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SignalResult {
    pub delivered_to: Vec<String>,
    #[serde(skip)]
    pub raw: Value,
}
impl std::fmt::Debug for SignalResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SignalResult")
            .field("delivered_to", &self.delivered_to)
            .finish_non_exhaustive()
    }
}
impl SignalResult {
    pub(crate) fn parse(raw: Value) -> Result<Self, SagatError> {
        let mut result: Self = serde_json::from_value(raw.clone()).map_err(|_| protocol())?;
        result.raw = raw;
        Ok(result)
    }
}
fn protocol() -> SagatError {
    SagatError::new("PROTOCOL", "the response is missing a required typed field")
}

/// A single server page, including opaque continuation and additive raw response fields.
#[derive(Clone)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub next_cursor: Option<String>,
    pub raw: Value,
}
impl<T> std::fmt::Debug for Page<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Page")
            .field("items", &self.items.len())
            .field("has_next", &self.next_cursor.is_some())
            .finish_non_exhaustive()
    }
}
pub type RunPage = Page<Run>;
pub type JournalPage = Page<JournalEvent>;
pub(crate) fn parse_page<T>(
    raw: Value,
    parse: impl Fn(Value) -> Result<T, SagatError>,
) -> Result<Page<T>, SagatError> {
    let items = raw
        .get("items")
        .and_then(Value::as_array)
        .filter(|items| items.len() <= 200)
        .ok_or_else(protocol)?;
    let next_cursor = match raw.get("nextCursor") {
        None | Some(Value::Null) => None,
        Some(Value::String(cursor)) => Some(cursor.clone()),
        _ => return Err(protocol()),
    };
    let items = items
        .iter()
        .cloned()
        .map(parse)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Page {
        items,
        next_cursor,
        raw,
    })
}
#[derive(Clone, Debug, Deserialize)]
pub struct Cause {
    pub kind: String,
    pub code: String,
}
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JournalEvent {
    pub seq: u64,
    pub event_type: String,
    #[serde(with = "time::serde::rfc3339")]
    pub occurred_at: OffsetDateTime,
    pub actor: String,
    pub step_id: Option<String>,
    pub attempt: Option<u32>,
    pub trace_id: Option<String>,
    pub payload_ref: Option<String>,
    pub span_id: Option<String>,
    pub cause: Option<Cause>,
    #[serde(skip)]
    pub raw: Value,
}
impl std::fmt::Debug for JournalEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JournalEvent")
            .field("seq", &self.seq)
            .field("event_type", &self.event_type)
            .finish_non_exhaustive()
    }
}
impl JournalEvent {
    pub(crate) fn parse(raw: Value) -> Result<Self, SagatError> {
        let mut event: Self = serde_json::from_value(raw.clone()).map_err(|_| protocol())?;
        if event.seq == 0
            || event.seq > i64::MAX as u64
            || event.attempt.is_some_and(|v| v == 0 || v > i32::MAX as u32)
        {
            return Err(protocol());
        }
        event.occurred_at = event.occurred_at.to_offset(UtcOffset::UTC);
        event.raw = raw;
        Ok(event)
    }
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryResult {
    pub run_id: Uuid,
    pub run_version: u64,
    pub state_revision: u64,
    pub result: Value,
    #[serde(skip)]
    pub raw: Value,
}
impl std::fmt::Debug for QueryResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QueryResult")
            .field("run_version", &self.run_version)
            .field("state_revision", &self.state_revision)
            .finish_non_exhaustive()
    }
}
impl QueryResult {
    pub(crate) fn parse(raw: Value) -> Result<Self, SagatError> {
        if raw.get("result").is_none() {
            return Err(protocol());
        }
        let mut value: Self = serde_json::from_value(raw.clone()).map_err(|_| protocol())?;
        if value.run_version > i64::MAX as u64 || value.state_revision > i64::MAX as u64 {
            return Err(protocol());
        }
        value.raw = raw;
        Ok(value)
    }
}
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateReceipt {
    pub run_id: Uuid,
    pub operation: String,
    pub update_key: String,
    pub status: String,
    pub result: Value,
    pub before_state_revision: u64,
    pub after_state_revision: u64,
    pub run_version: u64,
    pub replayed: bool,
    #[serde(skip)]
    pub raw: Value,
}
impl std::fmt::Debug for UpdateReceipt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UpdateReceipt")
            .field("status", &self.status)
            .field("replayed", &self.replayed)
            .finish_non_exhaustive()
    }
}
impl UpdateReceipt {
    pub(crate) fn parse(raw: Value) -> Result<Self, SagatError> {
        if raw.get("result").is_none() {
            return Err(protocol());
        }
        let mut value: Self = serde_json::from_value(raw.clone()).map_err(|_| protocol())?;
        if value.status != "APPLIED"
            || value.operation.is_empty()
            || value.operation.len() > 128
            || value.update_key.is_empty()
            || value.update_key.len() > 128
            || value.before_state_revision > i64::MAX as u64
            || !(1..=i64::MAX as u64).contains(&value.after_state_revision)
            || !(1..=i64::MAX as u64).contains(&value.run_version)
        {
            return Err(protocol());
        }
        value.raw = raw;
        Ok(value)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum EventStatus {
    Accepted,
    Duplicate,
}
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventReceipt {
    pub event_id: String,
    pub source: String,
    pub status: EventStatus,
    #[serde(skip)]
    pub raw: Value,
}
impl std::fmt::Debug for EventReceipt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EventReceipt")
            .field("status", &self.status)
            .finish_non_exhaustive()
    }
}
impl EventReceipt {
    pub(crate) fn parse(raw: Value) -> Result<Self, SagatError> {
        let mut value: Self = serde_json::from_value(raw.clone()).map_err(|_| protocol())?;
        if value.event_id.is_empty() || value.source.is_empty() {
            return Err(protocol());
        }
        value.raw = raw;
        Ok(value)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ApprovalDecision {
    Approve,
    Reject,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ApprovalStatus {
    Pending,
    Approved,
    Rejected,
    Expired,
    Cancelled,
}
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalListItem {
    pub approval_id: Uuid,
    pub run_id: Uuid,
    pub node_id: String,
    pub instance_path: Vec<ScopeSegment>,
    pub status: ApprovalStatus,
    #[serde(with = "time::serde::rfc3339")]
    pub deadline: OffsetDateTime,
    #[serde(deserialize_with = "nullable_instant")]
    pub settled_at: Option<OffsetDateTime>,
    #[serde(skip)]
    pub raw: Value,
}
fn nullable_instant<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<OffsetDateTime>, D::Error> {
    Option::<String>::deserialize(deserializer)?
        .map(|value| {
            OffsetDateTime::parse(&value, &time::format_description::well_known::Rfc3339)
                .map_err(serde::de::Error::custom)
        })
        .transpose()
}
impl std::fmt::Debug for ApprovalListItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ApprovalListItem")
            .field("status", &self.status)
            .finish_non_exhaustive()
    }
}
pub type ApprovalPage = Page<ApprovalListItem>;
impl ApprovalListItem {
    pub(crate) fn parse(raw: Value) -> Result<Self, SagatError> {
        let mut value: Self = serde_json::from_value(raw.clone()).map_err(|_| protocol())?;
        if value.node_id.is_empty()
            || value.node_id.chars().count() > 64
            || value.instance_path.len() > 8
            || value.instance_path.iter().any(|p| {
                p.scope_node_id.is_empty()
                    || p.scope_node_id.chars().count() > 64
                    || p.iteration_index > 9999
            })
        {
            return Err(protocol());
        }
        value.deadline = value.deadline.to_offset(UtcOffset::UTC);
        value.settled_at = value.settled_at.map(|time| time.to_offset(UtcOffset::UTC));
        value.raw = raw;
        Ok(value)
    }
}
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalDecisionReceipt {
    pub receipt_id: Uuid,
    pub approval_id: Uuid,
    pub decision_key: String,
    pub decision: ApprovalDecision,
    pub reason_code: Option<String>,
    pub actor: String,
    #[serde(with = "time::serde::rfc3339")]
    pub decided_at: OffsetDateTime,
    pub outcome_sequence: u64,
    pub replayed: bool,
    #[serde(skip)]
    pub raw: Value,
}
impl std::fmt::Debug for ApprovalDecisionReceipt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ApprovalDecisionReceipt")
            .field("decision", &self.decision)
            .field("replayed", &self.replayed)
            .finish_non_exhaustive()
    }
}
impl ApprovalDecisionReceipt {
    pub(crate) fn parse(raw: Value) -> Result<Self, SagatError> {
        let mut value: Self = serde_json::from_value(raw.clone()).map_err(|_| protocol())?;
        if value.decision_key.is_empty()
            || value.decision_key.len() > 128
            || value.actor.is_empty()
            || !(1..=i64::MAX as u64).contains(&value.outcome_sequence)
        {
            return Err(protocol());
        }
        value.decided_at = value.decided_at.to_offset(UtcOffset::UTC);
        value.raw = raw;
        Ok(value)
    }
}
