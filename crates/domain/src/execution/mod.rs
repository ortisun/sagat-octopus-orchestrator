//! Durable execution model: the run state machine, its journal vocabulary and its commands.
//!
//! Everything here is pure: no clock, no randomness, no storage. Time and jitter are passed in, and
//! the data-dependent decisions (conditions, correlation keys) are resolved by an adapter and
//! returned as journal events (`FR-002-001`, `FR-002-002`; `docs/architecture/execution-semantics.md`).

mod approval;
mod command;
mod continuation;
mod error;
mod event;
mod instance;
mod operations;
mod plan;
mod sagas;
mod state;

pub use approval::{
    ApprovalDecision, ApprovalDecisionCommand, ApprovalDecisionReceipt, ApprovalDecisionResolution,
    ApprovalError, ApprovalPolicy, ApprovalReasonCode, ApprovalReceiptId, ApprovalRequest,
    ApprovalRequestId, ApprovalStatus, ApprovalTransition, DecisionKey,
    MAX_APPROVAL_POLICY_ENTRIES, MAX_APPROVAL_REASON_CODE_BYTES, MAX_DECISION_KEY_BYTES,
};
pub use command::{
    AddressedProgress, Expiry, Failure, Heartbeat, InstanceHeartbeat, Progress, Resolution,
    complete_instance, complete_reconciliation, complete_step, expire_attempt, expire_instance,
    fail_instance, fail_report_for_storage_quota, fail_step, fail_step_for_storage_quota,
    finish_child, fire_timer, heartbeat, heartbeat_instance, lease_instance, lease_step,
    mark_for_review, pause_run, resolve, resolve_review, resume_run, retry_due, retry_due_instance,
    retry_review, satisfy_wait, skip_review, start_run, stop_for_storage_quota, stop_run,
    time_out_wait,
};
pub use continuation::{
    CancelResolution, ChainError, ChainFinalOutcome, ChainHandoff, ChainPolicy, ChainScope,
    ChildChainLink, ChildObservation, ContinuationProposal, ContinuationTransfer, ExecutionChain,
    FinalizationResolution, MAX_CHAIN_SEGMENTS, Quiescence, QuiescenceBlocker, Segment,
    SegmentCompletionKind, SegmentIndex,
};
pub use error::ExecutionError;
pub use event::{
    AddressedRunEvent, Cause, ChildOutcome, CollectionDrainReason, CollectionOutcome,
    CorrelationKey, EventAddressError, FailureCode, Fence, JoinOutcome, LoopContinuation,
    LoopDrainReason, LoopKind, LoopOutcome, PlannedEntry, ReconcileResult, ReviewOutcome, RunEvent,
    StopReason, SwitchSelection,
};
pub use instance::{
    ExecutionInstanceId, InstanceIdentityError, InstancePath, MAX_ITERATION_INDEX, MAX_SCOPE_DEPTH,
    ScopeSegment,
};
pub use operations::{
    DeclaredOperationsState, MAX_OPERATION_NAME_BYTES, MAX_OPERATION_VALUE_BYTES,
    MAX_UPDATE_KEY_BYTES, OperationAddress, OperationError, OperationScope, OperationValue,
    QueryResponse, QuerySnapshot, ResultDisclosure, UpdateEvaluation, UpdateKey, UpdateReceipt,
    UpdateReceiptStatus, UpdateRejection, UpdateRequest, UpdateResolution,
};
pub use plan::{AddressedPlan, Evaluation, Plan, plan, plan_addressed};
pub use state::{
    CollectionIterationRestore, CollectionPhase, CollectionResult, CollectionState, LoopCheckpoint,
    LoopIterationRestore, LoopPhase, LoopState, RunChainState, RunId, RunState, RunStatus,
    SagaPhase, SagaState, StepState, StepStatus, WaitingOn,
};
