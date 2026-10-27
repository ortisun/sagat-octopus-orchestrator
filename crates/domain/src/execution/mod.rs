//! Durable execution model: the run state machine, its journal vocabulary and its commands.
//!
//! Everything here is pure: no clock, no randomness, no storage. Time and jitter are passed in, and
//! the data-dependent decisions (conditions, correlation keys) are resolved by an adapter and
//! returned as journal events (`FR-002-001`, `FR-002-002`; `docs/architecture/execution-semantics.md`).

mod approval;
mod capacity;
mod command;
mod continuation;
mod error;
mod event;
mod instance;
mod operations;
mod path_state;
mod plan;
mod sagas;
mod scoped_controls;
mod scoped_event_waits;
mod scoped_sagas;
mod scoped_timers;
mod state;

pub use approval::{
    ApprovalDecision, ApprovalDecisionCommand, ApprovalDecisionReceipt, ApprovalDecisionResolution,
    ApprovalError, ApprovalPolicy, ApprovalReasonCode, ApprovalReceiptId, ApprovalRequest,
    ApprovalRequestId, ApprovalStatus, ApprovalTransition, DecisionKey,
    MAX_APPROVAL_POLICY_ENTRIES, MAX_APPROVAL_REASON_CODE_BYTES, MAX_DECISION_KEY_BYTES,
};
pub use capacity::{
    CapacityGroup, CapacityGroupError, CapacityGroupId, CapacityMemberId, CapacityMemberSnapshot,
    CapacityMembership, CapacityPossession,
};
pub use command::{
    AddressedProgress, Expiry, Failure, Heartbeat, InstanceHeartbeat, Progress, Resolution,
    ScopedOutputRefusalSuccess, advance_scope_compensation, complete_instance,
    complete_path_instance, complete_reconciliation, complete_reconciliation_instance,
    complete_reconciliation_path_instance, complete_step, expire_attempt, expire_instance,
    expire_path_instance, fail_instance, fail_path_instance, fail_report_for_storage_quota,
    fail_step, fail_step_for_storage_quota, fail_unentered_loop, finish_child, fire_timer,
    heartbeat, heartbeat_instance, heartbeat_path_instance, lease_instance, lease_path_instance,
    lease_step, mark_for_review, mark_for_review_instance, mark_for_review_path_instance,
    pause_run, prepare_scoped_output_refusal, release_pending, resolve, resolve_review,
    resolve_review_instance, resolve_review_path_instance, resume_run, retry_due,
    retry_due_instance, retry_due_path_instance, retry_review, retry_review_instance,
    retry_review_path_instance, satisfy_wait, skip_review, skip_review_instance,
    skip_review_path_instance, start_run, stop_for_storage_quota, stop_run, time_out_wait,
};
pub use continuation::{
    CancelResolution, ChainError, ChainFinalOutcome, ChainHandoff, ChainPolicy, ChainScope,
    ChildChainLink, ChildObservation, ContinuationProposal, ContinuationTransfer, ExecutionChain,
    FinalizationResolution, MAX_CHAIN_SEGMENTS, MAX_TREE_RUNS, Quiescence, QuiescenceBlocker,
    Segment, SegmentCompletionKind, SegmentIndex,
};
pub use error::ExecutionError;
pub use event::{
    AddressedRunEvent, Cause, ChildOutcome, CollectionDrainReason, CollectionOutcome,
    CorrelationKey, EventAddressError, FailureCode, Fence, JoinOutcome, LoopContinuation,
    LoopDrainReason, LoopKind, LoopOutcome, PlannedEntry, ReconcileResult, ReviewOutcome, RunEvent,
    ScopedPlannedEntry, StopReason, SwitchSelection,
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
pub use path_state::{
    AncestorExecutionFence, GlobalExecutionFence, PathCollectionState, PathExecutionState,
    PathLoopState, PathScopeSagaState,
};
pub use plan::{AddressedPlan, Evaluation, Plan, plan, plan_addressed, plan_path};
pub use scoped_controls::{resolve_control_instance, resolve_control_path_instance};
pub use scoped_event_waits::{
    satisfy_event_wait_instance, satisfy_event_wait_path_instance, scoped_event_wait_since,
    start_event_wait_instance, start_event_wait_path_instance, time_out_event_wait_instance,
    time_out_event_wait_path_instance,
};
pub use scoped_sagas::{plan_scope_compensation, scope_compensation_candidate};
pub use scoped_timers::{
    fire_timer_instance, fire_timer_path_instance, release_pending_instance_path,
    release_pending_path, start_timer_instance, start_timer_path_instance,
};
pub use state::{
    CollectionIterationRestore, CollectionPhase, CollectionResult, CollectionState, LoopCheckpoint,
    LoopIterationRestore, LoopPhase, LoopState, RunChainState, RunId, RunState, RunStatus,
    RunWorkSummary, SagaPhase, SagaState, ScopeSagaState, StepState, StepStatus, WaitingOn,
    scope_final_status,
};
