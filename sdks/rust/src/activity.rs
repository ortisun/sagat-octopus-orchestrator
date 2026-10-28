//! Attempt-local application data; wire authorization stays private to the host.
use crate::{ScopeSegment, Value};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicI64, AtomicU8, Ordering},
};
use tokio_util::sync::CancellationToken;
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExecutionInstanceIdentity {
    pub instance_path: Vec<ScopeSegment>,
    pub definition_node_id: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskKind {
    Forward,
    Compensation,
    Reconciliation,
}
#[derive(Clone)]
pub struct TaskInfo {
    pub activity_name: String,
    pub task_id: String,
    pub run_id: String,
    pub step_id: String,
    pub node: String,
    pub attempt: i32,
    pub generation: i64,
    pub idempotency_key: String,
    pub traceparent: String,
    pub task_kind: TaskKind,
    pub target_node: String,
    pub target_idempotency_key: String,
    pub instance: Option<ExecutionInstanceIdentity>,
    pub target_instance: Option<ExecutionInstanceIdentity>,
    pub steps: Value,
    pub item: Option<Value>,
    pub index: Option<u64>,
}
impl std::fmt::Debug for TaskInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TaskInfo")
            .field("activity_name", &self.activity_name)
            .field("attempt", &self.attempt)
            .field("task_kind", &self.task_kind)
            .finish_non_exhaustive()
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CancellationReason {
    RunCancelled,
    LeaseLost,
    Shutdown,
}
#[derive(Clone)]
pub struct ActivityContext {
    pub(crate) capabilities: Option<Arc<crate::capabilities::Capabilities>>,
    pub task: Arc<TaskInfo>,
    pub cancellation: CancellationToken,
    pub(crate) reason: Arc<AtomicU8>,
    pub(crate) lost: Arc<AtomicBool>,
    pub(crate) expires: Arc<AtomicI64>,
}
impl std::fmt::Debug for ActivityContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ActivityContext")
            .field("task", &self.task)
            .field("cancelled", &self.cancellation.is_cancelled())
            .finish_non_exhaustive()
    }
}
impl ActivityContext {
    pub async fn read_blob(
        &self,
        descriptor: &crate::BlobDescriptor,
    ) -> Result<crate::BlobReader, crate::SagatError> {
        self.capabilities
            .as_ref()
            .ok_or_else(|| {
                crate::SagatError::new("VALIDATION_FAILED", "no active attempt capability")
            })?
            .read_blob(descriptor)
            .await
    }

    pub async fn write_blob<R: tokio::io::AsyncRead + Unpin + Send + 'static>(
        &self,
        source: R,
        options: crate::WriteBlobOptions,
    ) -> Result<crate::BlobOutput, crate::SagatError> {
        self.capabilities
            .as_ref()
            .ok_or_else(|| {
                crate::SagatError::new("VALIDATION_FAILED", "no active attempt capability")
            })?
            .write_blob(source, options)
            .await
    }

    pub async fn secrets(&self) -> Result<crate::SecretSet, crate::SagatError> {
        self.capabilities
            .as_ref()
            .ok_or_else(|| {
                crate::SagatError::new("VALIDATION_FAILED", "no active attempt capability")
            })?
            .secrets()
            .await
    }

    pub fn deadline_epoch_ms(&self) -> i64 {
        self.expires.load(Ordering::Acquire)
    }
    pub fn cancellation_reason(&self) -> Option<CancellationReason> {
        match self.reason.load(Ordering::Acquire) {
            1 => Some(CancellationReason::RunCancelled),
            2 => Some(CancellationReason::LeaseLost),
            3 => Some(CancellationReason::Shutdown),
            _ => None,
        }
    }
    pub(crate) fn cancel(&self, reason: CancellationReason) {
        let code = match reason {
            CancellationReason::RunCancelled => 1,
            CancellationReason::LeaseLost => {
                self.lost.store(true, Ordering::Release);
                2
            }
            CancellationReason::Shutdown => 3,
        };
        let _ = self
            .reason
            .compare_exchange(0, code, Ordering::AcqRel, Ordering::Acquire);
        self.cancellation.cancel();
    }
}
/// Only explicitly safe messages are sent to the platform; panics use a fixed message.
#[derive(Clone)]
pub struct ActivityFailure {
    pub code: String,
    pub safe_message: String,
    pub retryable: bool,
    pub(crate) cancelled: bool,
}
impl std::fmt::Debug for ActivityFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ActivityFailure")
            .field("code", &self.code)
            .field("retryable", &self.retryable)
            .finish_non_exhaustive()
    }
}
impl ActivityFailure {
    pub fn permanent(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            safe_message: message.into(),
            retryable: false,
            cancelled: false,
        }
    }
    pub fn retryable(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            retryable: true,
            ..Self::permanent(code, message)
        }
    }
    pub fn cancelled() -> Self {
        Self {
            cancelled: true,
            ..Self::permanent("CANCELLED", "the activity was cancelled")
        }
    }
}

impl From<crate::SagatError> for ActivityFailure {
    fn from(_: crate::SagatError) -> Self {
        Self::retryable("ACTIVITY_ERROR", "SagatError")
    }
}

/// A typed reconciliation answer; only Applied can carry disclosed output.
#[derive(Clone, serde::Serialize)]
#[serde(tag = "effect", rename_all = "snake_case")]
pub enum Verdict {
    Applied {
        #[serde(skip_serializing_if = "Option::is_none")]
        output: Option<Value>,
    },
    NotApplied,
    Unknown,
}
impl Verdict {
    pub fn applied(output: Option<Value>) -> Self {
        Self::Applied { output }
    }
    pub fn not_applied() -> Self {
        Self::NotApplied
    }
    pub fn unknown() -> Self {
        Self::Unknown
    }
}
impl std::fmt::Debug for Verdict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Applied { .. } => "Verdict::Applied",
            Self::NotApplied => "Verdict::NotApplied",
            Self::Unknown => "Verdict::Unknown",
        })
    }
}
