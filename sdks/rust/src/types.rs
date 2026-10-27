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
#[serde(rename_all = "camelCase", deny_unknown_fields)]
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
