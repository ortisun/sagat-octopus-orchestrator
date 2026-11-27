#![doc = include_str!("../README.md")]
//! Async native SDK for Sagat's public REST and worker protocols.
//! The server owns orchestration, durable state, retries and capability authorization.
mod client;
mod error;
mod tls;
mod token;
mod types;
pub use client::{
    ApprovalDecisionRequest, BatchFilter, BatchItem, BatchJob, BatchJobCounts, BatchListFilter,
    BatchOperations, BatchPreview, BatchRequest, BatchSelector, BatchSignal, BatchUpdate,
    BusinessIdBinding, BusinessIdConflictPolicy, BusinessIdDisposition, BusinessIdReusePolicy,
    BusinessIds, CallOptions, CatchupMode, ChainCommand, ChainQueryRequest, ChainUpdateRequest,
    Client, ClientOptions, DstPolicy, ListOptions, Moment, OperationOptions, OverlapPolicy,
    PageOptions, PublishEvent, ReapplyPolicy, ReexecutedStep, Reset, ResetPlan, ResetPoint,
    ResetRequest, RestoredStep, RetryPolicy, RunDeadline, Schedule, ScheduleAction,
    ScheduleActionDefinition, ScheduleActionFilter, ScheduleActionRecord, ScheduleBackfill,
    ScheduleCalendar, ScheduleDescribeOptions, ScheduleDescription, ScheduleInterval,
    ScheduleListFilter, SchedulePolicies, ScheduleRange, ScheduleSpec, ScheduleWrite, Schedules,
    SignalWithStartResult, SourcePolicy, StartOptions, StartedRun, StepOutcome, UpcomingAction,
    UpdateRequest, UpdateWithStartOptions, UpdateWithStartOutcome, UpdateWithStartResult,
    UploadRunInputOptions, VersionPolicy, WaitOptions, WithStartOptions,
};
pub use error::SagatError;
pub use serde_json::{Value, json};
pub use tls::{TlsIdentity, TlsOptions};
pub use token::{TokenProviderError, TokenSource};
pub use tokio_util::sync::CancellationToken;
pub use types::{
    ApprovalDecision, ApprovalDecisionReceipt, ApprovalListItem, ApprovalPage, ApprovalStatus,
    BlobDescriptor, Cause, ChainInspection, ChainLink, ChainRun, ChainRunPage, ChainSegment,
    CompletionKind, EventReceipt, EventStatus, JournalEvent, JournalPage, OperationResult, Page,
    QueryResult, Run, RunPage, RunParent, RunResetFrom, RunSchedule, RunStatus, ScopeSegment,
    SignalResult, UpdateReceipt,
};
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

// Wire types stay private: applications use capability-scoped public contexts.
#[allow(dead_code, clippy::all)]
mod protocol {
    tonic::include_proto!("sagat_octopus_orchestrator.v1");
}

// A step-log batch (feature 041): only counts print, never a message, an exception, a field, a token
// or the identity of the attempt (FR-041-007 (d)).
impl std::fmt::Debug for protocol::AppendStepLogsRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppendStepLogsRequest")
            .field("records", &self.records.len())
            .finish_non_exhaustive()
    }
}
// The last records of an attempt on its report (design 041 section 2a): its counts only.
impl std::fmt::Debug for protocol::StepLogCarry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StepLogCarry")
            .field("records", &self.records.len())
            .field("highest_sequence", &self.highest_sequence)
            .finish()
    }
}
impl std::fmt::Debug for protocol::StepLogRecord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StepLogRecord")
            .field("sequence", &self.sequence)
            .field("fields", &self.fields.len())
            .finish_non_exhaustive()
    }
}
impl std::fmt::Debug for protocol::StepLogField {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("StepLogField { <redacted> }")
    }
}
impl std::fmt::Debug for protocol::step_log_field::Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("StepLogField::Value(<redacted>)")
    }
}

#[cfg(test)]
mod step_log_debug_tests {
    use super::protocol;

    #[test]
    fn the_debug_of_a_step_log_batch_prints_counts_and_no_text() {
        let request = protocol::AppendStepLogsRequest {
            session_token: "canary-session-token".into(),
            lease_token: "canary-lease-token".into(),
            records: vec![protocol::StepLogRecord {
                sequence: 3,
                message: "canary-message".into(),
                exception: "canary-exception".into(),
                fields: vec![protocol::StepLogField {
                    key: "canary-key".into(),
                    value: Some(protocol::step_log_field::Value::StringValue(
                        "canary-value".into(),
                    )),
                }],
                ..Default::default()
            }],
            ..Default::default()
        };
        for text in [
            format!("{request:?}"),
            format!("{request:#?}"),
            format!("{:?}", request.records[0].fields[0].value),
        ] {
            assert!(!text.contains("canary"), "{text}");
        }
        assert!(format!("{request:?}").contains("records: 1"));
    }

    /// A report holding a carry (design 041 section 2a) prints the carry's counts, never a record.
    #[test]
    fn the_debug_of_a_report_that_holds_a_carry_prints_counts_and_no_text() {
        let carry = protocol::StepLogCarry {
            records: vec![protocol::StepLogRecord {
                sequence: 3,
                message: "canary-message".into(),
                ..Default::default()
            }],
            highest_sequence: 5,
        };
        let complete = protocol::CompleteStepRequest {
            step_logs: Some(carry.clone()),
            ..Default::default()
        };
        let fail = protocol::FailStepRequest {
            step_logs: Some(carry),
            ..Default::default()
        };
        for text in [format!("{complete:?}"), format!("{fail:#?}")] {
            assert!(!text.contains("canary"), "{text}");
            assert!(
                text.contains("records: 1") && text.contains("highest_sequence: 5"),
                "{text}"
            );
        }
    }
}

mod worker_rpc;

mod activity;
mod worker;
pub use activity::{
    ActivityContext, ActivityFailure, CancellationReason, ExecutionInstanceIdentity, TaskInfo,
    TaskKind,
};
pub use worker::{WorkerHost, WorkerOptions, WorkerTransport};

mod json;

mod capabilities;
pub use capabilities::{SecretSet, SecretValue};

mod blob;
pub use blob::{BlobOutput, WriteBlobOptions};

pub use capabilities::BlobReader;

pub use activity::Verdict;

mod telemetry;
pub use telemetry::{EventKind, SdkEvent, SdkTransport, Telemetry};

pub mod workflow;
