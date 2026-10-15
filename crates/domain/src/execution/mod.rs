//! Durable execution model: the run state machine, its journal vocabulary and its commands.
//!
//! Everything here is pure: no clock, no randomness, no storage. Time and jitter are passed in, and
//! the data-dependent decisions (conditions, correlation keys) are resolved by an adapter and
//! returned as journal events (`FR-002-001`, `FR-002-002`; `docs/architecture/execution-semantics.md`).

mod command;
mod error;
mod event;
mod instance;
mod plan;
mod sagas;
mod state;

pub use command::{
    Expiry, Failure, Heartbeat, Progress, Resolution, complete_reconciliation, complete_step,
    expire_attempt, fail_report_for_storage_quota, fail_step, fail_step_for_storage_quota,
    finish_child, fire_timer, heartbeat, lease_step, mark_for_review, pause_run, resolve,
    resolve_review, resume_run, retry_due, retry_review, satisfy_wait, skip_review, start_run,
    stop_for_storage_quota, stop_run, time_out_wait,
};
pub use error::ExecutionError;
pub use event::{
    Cause, ChildOutcome, CorrelationKey, FailureCode, Fence, JoinOutcome, PlannedEntry,
    ReconcileResult, ReviewOutcome, RunEvent, StopReason, SwitchSelection,
};
pub use instance::{
    ExecutionInstanceId, InstanceIdentityError, InstancePath, MAX_ITERATION_INDEX, MAX_SCOPE_DEPTH,
    ScopeSegment,
};
pub use plan::{Evaluation, Plan, plan};
pub use state::{
    RunId, RunState, RunStatus, SagaPhase, SagaState, StepState, StepStatus, WaitingOn,
};
