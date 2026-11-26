from google.protobuf.internal import containers as _containers
from google.protobuf.internal import enum_type_wrapper as _enum_type_wrapper
from google.protobuf import descriptor as _descriptor
from google.protobuf import message as _message
from collections.abc import Iterable as _Iterable, Mapping as _Mapping
from typing import ClassVar as _ClassVar, Optional as _Optional, Union as _Union

DESCRIPTOR: _descriptor.FileDescriptor

class DatabaseOperationKind(int, metaclass=_enum_type_wrapper.EnumTypeWrapper):
    __slots__ = ()
    DATABASE_OPERATION_UNSPECIFIED: _ClassVar[DatabaseOperationKind]
    DATABASE_QUERY: _ClassVar[DatabaseOperationKind]
    DATABASE_EXECUTE: _ClassVar[DatabaseOperationKind]

class DatabaseTlsMode(int, metaclass=_enum_type_wrapper.EnumTypeWrapper):
    __slots__ = ()
    DATABASE_TLS_UNSPECIFIED: _ClassVar[DatabaseTlsMode]
    DATABASE_TLS_VERIFY_FULL: _ClassVar[DatabaseTlsMode]
    DATABASE_TLS_DEVELOPMENT_PLAINTEXT: _ClassVar[DatabaseTlsMode]

class EgressCeiling(int, metaclass=_enum_type_wrapper.EnumTypeWrapper):
    __slots__ = ()
    EGRESS_CEILING_UNSPECIFIED: _ClassVar[EgressCeiling]
    EGRESS_CEILING_PUBLIC: _ClassVar[EgressCeiling]
    EGRESS_CEILING_INTERNAL: _ClassVar[EgressCeiling]
    EGRESS_CEILING_PII: _ClassVar[EgressCeiling]

class LakeRestAuth(int, metaclass=_enum_type_wrapper.EnumTypeWrapper):
    __slots__ = ()
    LAKE_REST_AUTH_UNSPECIFIED: _ClassVar[LakeRestAuth]
    LAKE_REST_AUTH_OAUTH2: _ClassVar[LakeRestAuth]
    LAKE_REST_AUTH_BEARER: _ClassVar[LakeRestAuth]
    LAKE_REST_AUTH_SIGV4: _ClassVar[LakeRestAuth]

class LakeConditionalCreate(int, metaclass=_enum_type_wrapper.EnumTypeWrapper):
    __slots__ = ()
    LAKE_CONDITIONAL_CREATE_UNKNOWN: _ClassVar[LakeConditionalCreate]
    LAKE_CONDITIONAL_CREATE_SUPPORTED: _ClassVar[LakeConditionalCreate]
    LAKE_CONDITIONAL_CREATE_UNSUPPORTED: _ClassVar[LakeConditionalCreate]

class StepLogLevel(int, metaclass=_enum_type_wrapper.EnumTypeWrapper):
    __slots__ = ()
    STEP_LOG_LEVEL_UNSPECIFIED: _ClassVar[StepLogLevel]
    STEP_LOG_LEVEL_TRACE: _ClassVar[StepLogLevel]
    STEP_LOG_LEVEL_DEBUG: _ClassVar[StepLogLevel]
    STEP_LOG_LEVEL_INFO: _ClassVar[StepLogLevel]
    STEP_LOG_LEVEL_WARN: _ClassVar[StepLogLevel]
    STEP_LOG_LEVEL_ERROR: _ClassVar[StepLogLevel]

class StepLogVerdict(int, metaclass=_enum_type_wrapper.EnumTypeWrapper):
    __slots__ = ()
    STEP_LOG_VERDICT_UNSPECIFIED: _ClassVar[StepLogVerdict]
    STEP_LOG_VERDICT_CONTINUE: _ClassVar[StepLogVerdict]
    STEP_LOG_VERDICT_FINAL_ONLY: _ClassVar[StepLogVerdict]
    STEP_LOG_VERDICT_STOP_ATTEMPT: _ClassVar[StepLogVerdict]
    STEP_LOG_VERDICT_STOP_CAPTURE: _ClassVar[StepLogVerdict]
    STEP_LOG_VERDICT_RETRY_LATER: _ClassVar[StepLogVerdict]

class StepLogStopReason(int, metaclass=_enum_type_wrapper.EnumTypeWrapper):
    __slots__ = ()
    STEP_LOG_STOP_REASON_UNSPECIFIED: _ClassVar[StepLogStopReason]
    STEP_LOG_STOP_REASON_FINAL_RECORDED: _ClassVar[StepLogStopReason]
    STEP_LOG_STOP_REASON_LEASE: _ClassVar[StepLogStopReason]
    STEP_LOG_STOP_REASON_AUTHORITY: _ClassVar[StepLogStopReason]
    STEP_LOG_STOP_REASON_REMOVED: _ClassVar[StepLogStopReason]
    STEP_LOG_STOP_REASON_CAPTURE_OFF: _ClassVar[StepLogStopReason]
    STEP_LOG_STOP_REASON_SESSION: _ClassVar[StepLogStopReason]

class HistoryEventKind(int, metaclass=_enum_type_wrapper.EnumTypeWrapper):
    __slots__ = ()
    HISTORY_EVENT_KIND_UNSPECIFIED: _ClassVar[HistoryEventKind]
    HISTORY_RUN_STARTED: _ClassVar[HistoryEventKind]
    HISTORY_WORKFLOW_TASK_STARTED: _ClassVar[HistoryEventKind]
    HISTORY_WORKFLOW_TASK_COMPLETED: _ClassVar[HistoryEventKind]
    HISTORY_COMMAND_RECORDED: _ClassVar[HistoryEventKind]
    HISTORY_STEP_OUTCOME: _ClassVar[HistoryEventKind]
    HISTORY_TIMER_FIRED: _ClassVar[HistoryEventKind]
    HISTORY_WAIT_OUTCOME: _ClassVar[HistoryEventKind]
    HISTORY_CHILD_FINISHED: _ClassVar[HistoryEventKind]
    HISTORY_APPROVAL_DECIDED: _ClassVar[HistoryEventKind]
    HISTORY_SAGA_OUTCOME: _ClassVar[HistoryEventKind]
    HISTORY_SIGNAL_BUFFERED: _ClassVar[HistoryEventKind]
    HISTORY_UPDATE_ADMITTED: _ClassVar[HistoryEventKind]
    HISTORY_CANCELLATION_REQUESTED: _ClassVar[HistoryEventKind]
    HISTORY_MARKER_RECORDED: _ClassVar[HistoryEventKind]

class HistoryEventOrigin(int, metaclass=_enum_type_wrapper.EnumTypeWrapper):
    __slots__ = ()
    HISTORY_EVENT_ORIGIN_UNSPECIFIED: _ClassVar[HistoryEventOrigin]
    HISTORY_ORIGIN_COMPLETION: _ClassVar[HistoryEventOrigin]
    HISTORY_ORIGIN_EFFECT: _ClassVar[HistoryEventOrigin]
    HISTORY_ORIGIN_ENGINE: _ClassVar[HistoryEventOrigin]
    HISTORY_ORIGIN_EXTERNAL: _ClassVar[HistoryEventOrigin]

class StepOutcome(int, metaclass=_enum_type_wrapper.EnumTypeWrapper):
    __slots__ = ()
    STEP_OUTCOME_UNSPECIFIED: _ClassVar[StepOutcome]
    STEP_OUTCOME_COMPLETED: _ClassVar[StepOutcome]
    STEP_OUTCOME_FAILED: _ClassVar[StepOutcome]
    STEP_OUTCOME_TIMED_OUT: _ClassVar[StepOutcome]
    STEP_OUTCOME_CANCELLED: _ClassVar[StepOutcome]

class CancelOrigin(int, metaclass=_enum_type_wrapper.EnumTypeWrapper):
    __slots__ = ()
    CANCEL_ORIGIN_UNSPECIFIED: _ClassVar[CancelOrigin]
    CANCEL_ORIGIN_REQUEST: _ClassVar[CancelOrigin]
    CANCEL_ORIGIN_WITHOUT_GRACE: _ClassVar[CancelOrigin]
    CANCEL_ORIGIN_FUNCTION_ACKNOWLEDGED: _ClassVar[CancelOrigin]
    CANCEL_ORIGIN_CONTINUED_AFTER_REQUEST: _ClassVar[CancelOrigin]
    CANCEL_ORIGIN_LATCHED_HANDOFF: _ClassVar[CancelOrigin]

class CommandKind(int, metaclass=_enum_type_wrapper.EnumTypeWrapper):
    __slots__ = ()
    COMMAND_KIND_UNSPECIFIED: _ClassVar[CommandKind]
    COMMAND_ACTIVITY: _ClassVar[CommandKind]
    COMMAND_SCRIPT: _ClassVar[CommandKind]
    COMMAND_TIMER_START: _ClassVar[CommandKind]
    COMMAND_TIMER_CANCEL: _ClassVar[CommandKind]
    COMMAND_WAIT_EVENT: _ClassVar[CommandKind]
    COMMAND_CHILD: _ClassVar[CommandKind]
    COMMAND_APPROVAL: _ClassVar[CommandKind]
    COMMAND_STEP_CANCEL: _ClassVar[CommandKind]
    COMMAND_SAGA_OPEN: _ClassVar[CommandKind]
    COMMAND_SAGA_CLOSE: _ClassVar[CommandKind]
    COMMAND_SAGA_ABORT: _ClassVar[CommandKind]
    COMMAND_MARKER_PATCH: _ClassVar[CommandKind]
    COMMAND_MARKER_SIDE_EFFECT: _ClassVar[CommandKind]
    COMMAND_SET_STATE: _ClassVar[CommandKind]
    COMMAND_UPDATE_ACCEPT: _ClassVar[CommandKind]
    COMMAND_UPDATE_REJECT: _ClassVar[CommandKind]
    COMMAND_UPDATE_COMPLETE: _ClassVar[CommandKind]
    COMMAND_CONSUME_SIGNALS: _ClassVar[CommandKind]
    COMMAND_COMPLETE: _ClassVar[CommandKind]
    COMMAND_FAIL: _ClassVar[CommandKind]
    COMMAND_CONTINUE_AS_NEW: _ClassVar[CommandKind]
    COMMAND_ACKNOWLEDGE_CANCELLATION: _ClassVar[CommandKind]

class WorkflowTaskFailureCause(int, metaclass=_enum_type_wrapper.EnumTypeWrapper):
    __slots__ = ()
    WORKFLOW_TASK_FAILURE_CAUSE_UNSPECIFIED: _ClassVar[WorkflowTaskFailureCause]
    CAUSE_NONDETERMINISM: _ClassVar[WorkflowTaskFailureCause]
    CAUSE_CHAIN_MISMATCH: _ClassVar[WorkflowTaskFailureCause]
    CAUSE_NONDETERMINISM_RISK: _ClassVar[WorkflowTaskFailureCause]
    CAUSE_COMMAND_OUTSIDE_ENVELOPE: _ClassVar[WorkflowTaskFailureCause]
    CAUSE_COMMAND_INVALID: _ClassVar[WorkflowTaskFailureCause]
    CAUSE_VALIDATOR_COMMAND: _ClassVar[WorkflowTaskFailureCause]
    CAUSE_WORKFLOW_TASK_FAILED: _ClassVar[WorkflowTaskFailureCause]
    CAUSE_ACTIVATION_DEADLINE: _ClassVar[WorkflowTaskFailureCause]
    CAUSE_WORKFLOW_TASK_TIMED_OUT: _ClassVar[WorkflowTaskFailureCause]
    CAUSE_COMMAND_VALUE_RELEASED: _ClassVar[WorkflowTaskFailureCause]
    CAUSE_COMMAND_VALUE_LIMIT: _ClassVar[WorkflowTaskFailureCause]
    CAUSE_COMMAND_LIMIT: _ClassVar[WorkflowTaskFailureCause]
    CAUSE_OUTSTANDING_OBLIGATIONS: _ClassVar[WorkflowTaskFailureCause]
    CAUSE_UNHANDLED_EVENTS: _ClassVar[WorkflowTaskFailureCause]
    CAUSE_CLOSING_WINDOW_COMMAND: _ClassVar[WorkflowTaskFailureCause]
    CAUSE_HISTORY_LIMIT: _ClassVar[WorkflowTaskFailureCause]
    CAUSE_RETAINED_BYTES_LIMIT: _ClassVar[WorkflowTaskFailureCause]
    CAUSE_STORAGE_QUOTA_EXCEEDED: _ClassVar[WorkflowTaskFailureCause]

class WorkflowTaskOutcome(int, metaclass=_enum_type_wrapper.EnumTypeWrapper):
    __slots__ = ()
    WORKFLOW_TASK_OUTCOME_UNSPECIFIED: _ClassVar[WorkflowTaskOutcome]
    WORKFLOW_TASK_ACCEPTED: _ClassVar[WorkflowTaskOutcome]
    WORKFLOW_TASK_ACCEPTED_DEFERRED: _ClassVar[WorkflowTaskOutcome]
    WORKFLOW_TASK_REVIEW: _ClassVar[WorkflowTaskOutcome]
DATABASE_OPERATION_UNSPECIFIED: DatabaseOperationKind
DATABASE_QUERY: DatabaseOperationKind
DATABASE_EXECUTE: DatabaseOperationKind
DATABASE_TLS_UNSPECIFIED: DatabaseTlsMode
DATABASE_TLS_VERIFY_FULL: DatabaseTlsMode
DATABASE_TLS_DEVELOPMENT_PLAINTEXT: DatabaseTlsMode
EGRESS_CEILING_UNSPECIFIED: EgressCeiling
EGRESS_CEILING_PUBLIC: EgressCeiling
EGRESS_CEILING_INTERNAL: EgressCeiling
EGRESS_CEILING_PII: EgressCeiling
LAKE_REST_AUTH_UNSPECIFIED: LakeRestAuth
LAKE_REST_AUTH_OAUTH2: LakeRestAuth
LAKE_REST_AUTH_BEARER: LakeRestAuth
LAKE_REST_AUTH_SIGV4: LakeRestAuth
LAKE_CONDITIONAL_CREATE_UNKNOWN: LakeConditionalCreate
LAKE_CONDITIONAL_CREATE_SUPPORTED: LakeConditionalCreate
LAKE_CONDITIONAL_CREATE_UNSUPPORTED: LakeConditionalCreate
STEP_LOG_LEVEL_UNSPECIFIED: StepLogLevel
STEP_LOG_LEVEL_TRACE: StepLogLevel
STEP_LOG_LEVEL_DEBUG: StepLogLevel
STEP_LOG_LEVEL_INFO: StepLogLevel
STEP_LOG_LEVEL_WARN: StepLogLevel
STEP_LOG_LEVEL_ERROR: StepLogLevel
STEP_LOG_VERDICT_UNSPECIFIED: StepLogVerdict
STEP_LOG_VERDICT_CONTINUE: StepLogVerdict
STEP_LOG_VERDICT_FINAL_ONLY: StepLogVerdict
STEP_LOG_VERDICT_STOP_ATTEMPT: StepLogVerdict
STEP_LOG_VERDICT_STOP_CAPTURE: StepLogVerdict
STEP_LOG_VERDICT_RETRY_LATER: StepLogVerdict
STEP_LOG_STOP_REASON_UNSPECIFIED: StepLogStopReason
STEP_LOG_STOP_REASON_FINAL_RECORDED: StepLogStopReason
STEP_LOG_STOP_REASON_LEASE: StepLogStopReason
STEP_LOG_STOP_REASON_AUTHORITY: StepLogStopReason
STEP_LOG_STOP_REASON_REMOVED: StepLogStopReason
STEP_LOG_STOP_REASON_CAPTURE_OFF: StepLogStopReason
STEP_LOG_STOP_REASON_SESSION: StepLogStopReason
HISTORY_EVENT_KIND_UNSPECIFIED: HistoryEventKind
HISTORY_RUN_STARTED: HistoryEventKind
HISTORY_WORKFLOW_TASK_STARTED: HistoryEventKind
HISTORY_WORKFLOW_TASK_COMPLETED: HistoryEventKind
HISTORY_COMMAND_RECORDED: HistoryEventKind
HISTORY_STEP_OUTCOME: HistoryEventKind
HISTORY_TIMER_FIRED: HistoryEventKind
HISTORY_WAIT_OUTCOME: HistoryEventKind
HISTORY_CHILD_FINISHED: HistoryEventKind
HISTORY_APPROVAL_DECIDED: HistoryEventKind
HISTORY_SAGA_OUTCOME: HistoryEventKind
HISTORY_SIGNAL_BUFFERED: HistoryEventKind
HISTORY_UPDATE_ADMITTED: HistoryEventKind
HISTORY_CANCELLATION_REQUESTED: HistoryEventKind
HISTORY_MARKER_RECORDED: HistoryEventKind
HISTORY_EVENT_ORIGIN_UNSPECIFIED: HistoryEventOrigin
HISTORY_ORIGIN_COMPLETION: HistoryEventOrigin
HISTORY_ORIGIN_EFFECT: HistoryEventOrigin
HISTORY_ORIGIN_ENGINE: HistoryEventOrigin
HISTORY_ORIGIN_EXTERNAL: HistoryEventOrigin
STEP_OUTCOME_UNSPECIFIED: StepOutcome
STEP_OUTCOME_COMPLETED: StepOutcome
STEP_OUTCOME_FAILED: StepOutcome
STEP_OUTCOME_TIMED_OUT: StepOutcome
STEP_OUTCOME_CANCELLED: StepOutcome
CANCEL_ORIGIN_UNSPECIFIED: CancelOrigin
CANCEL_ORIGIN_REQUEST: CancelOrigin
CANCEL_ORIGIN_WITHOUT_GRACE: CancelOrigin
CANCEL_ORIGIN_FUNCTION_ACKNOWLEDGED: CancelOrigin
CANCEL_ORIGIN_CONTINUED_AFTER_REQUEST: CancelOrigin
CANCEL_ORIGIN_LATCHED_HANDOFF: CancelOrigin
COMMAND_KIND_UNSPECIFIED: CommandKind
COMMAND_ACTIVITY: CommandKind
COMMAND_SCRIPT: CommandKind
COMMAND_TIMER_START: CommandKind
COMMAND_TIMER_CANCEL: CommandKind
COMMAND_WAIT_EVENT: CommandKind
COMMAND_CHILD: CommandKind
COMMAND_APPROVAL: CommandKind
COMMAND_STEP_CANCEL: CommandKind
COMMAND_SAGA_OPEN: CommandKind
COMMAND_SAGA_CLOSE: CommandKind
COMMAND_SAGA_ABORT: CommandKind
COMMAND_MARKER_PATCH: CommandKind
COMMAND_MARKER_SIDE_EFFECT: CommandKind
COMMAND_SET_STATE: CommandKind
COMMAND_UPDATE_ACCEPT: CommandKind
COMMAND_UPDATE_REJECT: CommandKind
COMMAND_UPDATE_COMPLETE: CommandKind
COMMAND_CONSUME_SIGNALS: CommandKind
COMMAND_COMPLETE: CommandKind
COMMAND_FAIL: CommandKind
COMMAND_CONTINUE_AS_NEW: CommandKind
COMMAND_ACKNOWLEDGE_CANCELLATION: CommandKind
WORKFLOW_TASK_FAILURE_CAUSE_UNSPECIFIED: WorkflowTaskFailureCause
CAUSE_NONDETERMINISM: WorkflowTaskFailureCause
CAUSE_CHAIN_MISMATCH: WorkflowTaskFailureCause
CAUSE_NONDETERMINISM_RISK: WorkflowTaskFailureCause
CAUSE_COMMAND_OUTSIDE_ENVELOPE: WorkflowTaskFailureCause
CAUSE_COMMAND_INVALID: WorkflowTaskFailureCause
CAUSE_VALIDATOR_COMMAND: WorkflowTaskFailureCause
CAUSE_WORKFLOW_TASK_FAILED: WorkflowTaskFailureCause
CAUSE_ACTIVATION_DEADLINE: WorkflowTaskFailureCause
CAUSE_WORKFLOW_TASK_TIMED_OUT: WorkflowTaskFailureCause
CAUSE_COMMAND_VALUE_RELEASED: WorkflowTaskFailureCause
CAUSE_COMMAND_VALUE_LIMIT: WorkflowTaskFailureCause
CAUSE_COMMAND_LIMIT: WorkflowTaskFailureCause
CAUSE_OUTSTANDING_OBLIGATIONS: WorkflowTaskFailureCause
CAUSE_UNHANDLED_EVENTS: WorkflowTaskFailureCause
CAUSE_CLOSING_WINDOW_COMMAND: WorkflowTaskFailureCause
CAUSE_HISTORY_LIMIT: WorkflowTaskFailureCause
CAUSE_RETAINED_BYTES_LIMIT: WorkflowTaskFailureCause
CAUSE_STORAGE_QUOTA_EXCEEDED: WorkflowTaskFailureCause
WORKFLOW_TASK_OUTCOME_UNSPECIFIED: WorkflowTaskOutcome
WORKFLOW_TASK_ACCEPTED: WorkflowTaskOutcome
WORKFLOW_TASK_ACCEPTED_DEFERRED: WorkflowTaskOutcome
WORKFLOW_TASK_REVIEW: WorkflowTaskOutcome

class WorkerCapabilities(_message.Message):
    __slots__ = ("languages", "activities", "pools", "max_inflight", "protocol_version", "workflow_types", "offers_workflow_as_code")
    LANGUAGES_FIELD_NUMBER: _ClassVar[int]
    ACTIVITIES_FIELD_NUMBER: _ClassVar[int]
    POOLS_FIELD_NUMBER: _ClassVar[int]
    MAX_INFLIGHT_FIELD_NUMBER: _ClassVar[int]
    PROTOCOL_VERSION_FIELD_NUMBER: _ClassVar[int]
    WORKFLOW_TYPES_FIELD_NUMBER: _ClassVar[int]
    OFFERS_WORKFLOW_AS_CODE_FIELD_NUMBER: _ClassVar[int]
    languages: _containers.RepeatedScalarFieldContainer[str]
    activities: _containers.RepeatedScalarFieldContainer[str]
    pools: _containers.RepeatedScalarFieldContainer[str]
    max_inflight: int
    protocol_version: str
    workflow_types: _containers.RepeatedCompositeFieldContainer[WorkflowTypeOffer]
    offers_workflow_as_code: bool
    def __init__(self, languages: _Optional[_Iterable[str]] = ..., activities: _Optional[_Iterable[str]] = ..., pools: _Optional[_Iterable[str]] = ..., max_inflight: _Optional[int] = ..., protocol_version: _Optional[str] = ..., workflow_types: _Optional[_Iterable[_Union[WorkflowTypeOffer, _Mapping]]] = ..., offers_workflow_as_code: _Optional[bool] = ...) -> None: ...

class WorkflowTypeOffer(_message.Message):
    __slots__ = ("workflow_type", "code_digest")
    WORKFLOW_TYPE_FIELD_NUMBER: _ClassVar[int]
    CODE_DIGEST_FIELD_NUMBER: _ClassVar[int]
    workflow_type: str
    code_digest: str
    def __init__(self, workflow_type: _Optional[str] = ..., code_digest: _Optional[str] = ...) -> None: ...

class RegisterWorkerRequest(_message.Message):
    __slots__ = ("worker_name", "namespace", "capabilities")
    WORKER_NAME_FIELD_NUMBER: _ClassVar[int]
    NAMESPACE_FIELD_NUMBER: _ClassVar[int]
    CAPABILITIES_FIELD_NUMBER: _ClassVar[int]
    worker_name: str
    namespace: str
    capabilities: WorkerCapabilities
    def __init__(self, worker_name: _Optional[str] = ..., namespace: _Optional[str] = ..., capabilities: _Optional[_Union[WorkerCapabilities, _Mapping]] = ...) -> None: ...

class RegisterWorkerResponse(_message.Message):
    __slots__ = ("worker_id", "session_token", "accepted_protocol_version", "step_logs")
    WORKER_ID_FIELD_NUMBER: _ClassVar[int]
    SESSION_TOKEN_FIELD_NUMBER: _ClassVar[int]
    ACCEPTED_PROTOCOL_VERSION_FIELD_NUMBER: _ClassVar[int]
    STEP_LOGS_FIELD_NUMBER: _ClassVar[int]
    worker_id: str
    session_token: str
    accepted_protocol_version: str
    step_logs: StepLogPolicy
    def __init__(self, worker_id: _Optional[str] = ..., session_token: _Optional[str] = ..., accepted_protocol_version: _Optional[str] = ..., step_logs: _Optional[_Union[StepLogPolicy, _Mapping]] = ...) -> None: ...

class PollTasksRequest(_message.Message):
    __slots__ = ("worker_id", "session_token", "max_tasks")
    WORKER_ID_FIELD_NUMBER: _ClassVar[int]
    SESSION_TOKEN_FIELD_NUMBER: _ClassVar[int]
    MAX_TASKS_FIELD_NUMBER: _ClassVar[int]
    worker_id: str
    session_token: str
    max_tasks: int
    def __init__(self, worker_id: _Optional[str] = ..., session_token: _Optional[str] = ..., max_tasks: _Optional[int] = ...) -> None: ...

class ScopePathSegment(_message.Message):
    __slots__ = ("scope_node_id", "iteration_index")
    SCOPE_NODE_ID_FIELD_NUMBER: _ClassVar[int]
    ITERATION_INDEX_FIELD_NUMBER: _ClassVar[int]
    scope_node_id: str
    iteration_index: int
    def __init__(self, scope_node_id: _Optional[str] = ..., iteration_index: _Optional[int] = ...) -> None: ...

class ExecutionInstanceIdentity(_message.Message):
    __slots__ = ("instance_path", "definition_node_id")
    INSTANCE_PATH_FIELD_NUMBER: _ClassVar[int]
    DEFINITION_NODE_ID_FIELD_NUMBER: _ClassVar[int]
    instance_path: _containers.RepeatedCompositeFieldContainer[ScopePathSegment]
    definition_node_id: str
    def __init__(self, instance_path: _Optional[_Iterable[_Union[ScopePathSegment, _Mapping]]] = ..., definition_node_id: _Optional[str] = ...) -> None: ...

class TaskLease(_message.Message):
    __slots__ = ("task_id", "run_id", "step_id", "attempt", "generation", "lease_token", "expires_at_epoch_ms", "activity_name", "input_json", "input_blob_ref", "traceparent", "tenant_alias", "idempotency_key", "task_kind", "target_node", "target_idempotency_key", "instance", "target_instance", "workflow_task", "query_task", "replay_check_task")
    TASK_ID_FIELD_NUMBER: _ClassVar[int]
    RUN_ID_FIELD_NUMBER: _ClassVar[int]
    STEP_ID_FIELD_NUMBER: _ClassVar[int]
    ATTEMPT_FIELD_NUMBER: _ClassVar[int]
    GENERATION_FIELD_NUMBER: _ClassVar[int]
    LEASE_TOKEN_FIELD_NUMBER: _ClassVar[int]
    EXPIRES_AT_EPOCH_MS_FIELD_NUMBER: _ClassVar[int]
    ACTIVITY_NAME_FIELD_NUMBER: _ClassVar[int]
    INPUT_JSON_FIELD_NUMBER: _ClassVar[int]
    INPUT_BLOB_REF_FIELD_NUMBER: _ClassVar[int]
    TRACEPARENT_FIELD_NUMBER: _ClassVar[int]
    TENANT_ALIAS_FIELD_NUMBER: _ClassVar[int]
    IDEMPOTENCY_KEY_FIELD_NUMBER: _ClassVar[int]
    TASK_KIND_FIELD_NUMBER: _ClassVar[int]
    TARGET_NODE_FIELD_NUMBER: _ClassVar[int]
    TARGET_IDEMPOTENCY_KEY_FIELD_NUMBER: _ClassVar[int]
    INSTANCE_FIELD_NUMBER: _ClassVar[int]
    TARGET_INSTANCE_FIELD_NUMBER: _ClassVar[int]
    WORKFLOW_TASK_FIELD_NUMBER: _ClassVar[int]
    QUERY_TASK_FIELD_NUMBER: _ClassVar[int]
    REPLAY_CHECK_TASK_FIELD_NUMBER: _ClassVar[int]
    task_id: str
    run_id: str
    step_id: str
    attempt: int
    generation: int
    lease_token: str
    expires_at_epoch_ms: int
    activity_name: str
    input_json: str
    input_blob_ref: str
    traceparent: str
    tenant_alias: str
    idempotency_key: str
    task_kind: str
    target_node: str
    target_idempotency_key: str
    instance: ExecutionInstanceIdentity
    target_instance: ExecutionInstanceIdentity
    workflow_task: WorkflowTaskLease
    query_task: QueryTask
    replay_check_task: ReplayCheckTask
    def __init__(self, task_id: _Optional[str] = ..., run_id: _Optional[str] = ..., step_id: _Optional[str] = ..., attempt: _Optional[int] = ..., generation: _Optional[int] = ..., lease_token: _Optional[str] = ..., expires_at_epoch_ms: _Optional[int] = ..., activity_name: _Optional[str] = ..., input_json: _Optional[str] = ..., input_blob_ref: _Optional[str] = ..., traceparent: _Optional[str] = ..., tenant_alias: _Optional[str] = ..., idempotency_key: _Optional[str] = ..., task_kind: _Optional[str] = ..., target_node: _Optional[str] = ..., target_idempotency_key: _Optional[str] = ..., instance: _Optional[_Union[ExecutionInstanceIdentity, _Mapping]] = ..., target_instance: _Optional[_Union[ExecutionInstanceIdentity, _Mapping]] = ..., workflow_task: _Optional[_Union[WorkflowTaskLease, _Mapping]] = ..., query_task: _Optional[_Union[QueryTask, _Mapping]] = ..., replay_check_task: _Optional[_Union[ReplayCheckTask, _Mapping]] = ...) -> None: ...

class PollTasksResponse(_message.Message):
    __slots__ = ("tasks", "retry_after_ms")
    TASKS_FIELD_NUMBER: _ClassVar[int]
    RETRY_AFTER_MS_FIELD_NUMBER: _ClassVar[int]
    tasks: _containers.RepeatedCompositeFieldContainer[TaskLease]
    retry_after_ms: int
    def __init__(self, tasks: _Optional[_Iterable[_Union[TaskLease, _Mapping]]] = ..., retry_after_ms: _Optional[int] = ...) -> None: ...

class HeartbeatRequest(_message.Message):
    __slots__ = ("task_id", "worker_id", "generation", "lease_token", "run_id", "step_id", "attempt", "instance")
    TASK_ID_FIELD_NUMBER: _ClassVar[int]
    WORKER_ID_FIELD_NUMBER: _ClassVar[int]
    GENERATION_FIELD_NUMBER: _ClassVar[int]
    LEASE_TOKEN_FIELD_NUMBER: _ClassVar[int]
    RUN_ID_FIELD_NUMBER: _ClassVar[int]
    STEP_ID_FIELD_NUMBER: _ClassVar[int]
    ATTEMPT_FIELD_NUMBER: _ClassVar[int]
    INSTANCE_FIELD_NUMBER: _ClassVar[int]
    task_id: str
    worker_id: str
    generation: int
    lease_token: str
    run_id: str
    step_id: str
    attempt: int
    instance: ExecutionInstanceIdentity
    def __init__(self, task_id: _Optional[str] = ..., worker_id: _Optional[str] = ..., generation: _Optional[int] = ..., lease_token: _Optional[str] = ..., run_id: _Optional[str] = ..., step_id: _Optional[str] = ..., attempt: _Optional[int] = ..., instance: _Optional[_Union[ExecutionInstanceIdentity, _Mapping]] = ...) -> None: ...

class HeartbeatResponse(_message.Message):
    __slots__ = ("lease_expires_at_epoch_ms", "cancel_requested")
    LEASE_EXPIRES_AT_EPOCH_MS_FIELD_NUMBER: _ClassVar[int]
    CANCEL_REQUESTED_FIELD_NUMBER: _ClassVar[int]
    lease_expires_at_epoch_ms: int
    cancel_requested: bool
    def __init__(self, lease_expires_at_epoch_ms: _Optional[int] = ..., cancel_requested: _Optional[bool] = ...) -> None: ...

class CompleteStepRequest(_message.Message):
    __slots__ = ("task_id", "run_id", "step_id", "attempt", "generation", "lease_token", "idempotency_key", "output_json", "output_blob_ref", "output_sha256", "traceparent", "instance", "accept_workflow_task")
    TASK_ID_FIELD_NUMBER: _ClassVar[int]
    RUN_ID_FIELD_NUMBER: _ClassVar[int]
    STEP_ID_FIELD_NUMBER: _ClassVar[int]
    ATTEMPT_FIELD_NUMBER: _ClassVar[int]
    GENERATION_FIELD_NUMBER: _ClassVar[int]
    LEASE_TOKEN_FIELD_NUMBER: _ClassVar[int]
    IDEMPOTENCY_KEY_FIELD_NUMBER: _ClassVar[int]
    OUTPUT_JSON_FIELD_NUMBER: _ClassVar[int]
    OUTPUT_BLOB_REF_FIELD_NUMBER: _ClassVar[int]
    OUTPUT_SHA256_FIELD_NUMBER: _ClassVar[int]
    TRACEPARENT_FIELD_NUMBER: _ClassVar[int]
    INSTANCE_FIELD_NUMBER: _ClassVar[int]
    ACCEPT_WORKFLOW_TASK_FIELD_NUMBER: _ClassVar[int]
    task_id: str
    run_id: str
    step_id: str
    attempt: int
    generation: int
    lease_token: str
    idempotency_key: str
    output_json: str
    output_blob_ref: str
    output_sha256: str
    traceparent: str
    instance: ExecutionInstanceIdentity
    accept_workflow_task: bool
    def __init__(self, task_id: _Optional[str] = ..., run_id: _Optional[str] = ..., step_id: _Optional[str] = ..., attempt: _Optional[int] = ..., generation: _Optional[int] = ..., lease_token: _Optional[str] = ..., idempotency_key: _Optional[str] = ..., output_json: _Optional[str] = ..., output_blob_ref: _Optional[str] = ..., output_sha256: _Optional[str] = ..., traceparent: _Optional[str] = ..., instance: _Optional[_Union[ExecutionInstanceIdentity, _Mapping]] = ..., accept_workflow_task: _Optional[bool] = ...) -> None: ...

class FailStepRequest(_message.Message):
    __slots__ = ("task_id", "run_id", "step_id", "attempt", "generation", "lease_token", "idempotency_key", "error_code", "safe_message", "retryable", "traceparent", "instance")
    TASK_ID_FIELD_NUMBER: _ClassVar[int]
    RUN_ID_FIELD_NUMBER: _ClassVar[int]
    STEP_ID_FIELD_NUMBER: _ClassVar[int]
    ATTEMPT_FIELD_NUMBER: _ClassVar[int]
    GENERATION_FIELD_NUMBER: _ClassVar[int]
    LEASE_TOKEN_FIELD_NUMBER: _ClassVar[int]
    IDEMPOTENCY_KEY_FIELD_NUMBER: _ClassVar[int]
    ERROR_CODE_FIELD_NUMBER: _ClassVar[int]
    SAFE_MESSAGE_FIELD_NUMBER: _ClassVar[int]
    RETRYABLE_FIELD_NUMBER: _ClassVar[int]
    TRACEPARENT_FIELD_NUMBER: _ClassVar[int]
    INSTANCE_FIELD_NUMBER: _ClassVar[int]
    task_id: str
    run_id: str
    step_id: str
    attempt: int
    generation: int
    lease_token: str
    idempotency_key: str
    error_code: str
    safe_message: str
    retryable: bool
    traceparent: str
    instance: ExecutionInstanceIdentity
    def __init__(self, task_id: _Optional[str] = ..., run_id: _Optional[str] = ..., step_id: _Optional[str] = ..., attempt: _Optional[int] = ..., generation: _Optional[int] = ..., lease_token: _Optional[str] = ..., idempotency_key: _Optional[str] = ..., error_code: _Optional[str] = ..., safe_message: _Optional[str] = ..., retryable: _Optional[bool] = ..., traceparent: _Optional[str] = ..., instance: _Optional[_Union[ExecutionInstanceIdentity, _Mapping]] = ...) -> None: ...

class ResolveSecretsRequest(_message.Message):
    __slots__ = ("task_id", "run_id", "step_id", "attempt", "generation", "lease_token", "traceparent", "instance")
    TASK_ID_FIELD_NUMBER: _ClassVar[int]
    RUN_ID_FIELD_NUMBER: _ClassVar[int]
    STEP_ID_FIELD_NUMBER: _ClassVar[int]
    ATTEMPT_FIELD_NUMBER: _ClassVar[int]
    GENERATION_FIELD_NUMBER: _ClassVar[int]
    LEASE_TOKEN_FIELD_NUMBER: _ClassVar[int]
    TRACEPARENT_FIELD_NUMBER: _ClassVar[int]
    INSTANCE_FIELD_NUMBER: _ClassVar[int]
    task_id: str
    run_id: str
    step_id: str
    attempt: int
    generation: int
    lease_token: str
    traceparent: str
    instance: ExecutionInstanceIdentity
    def __init__(self, task_id: _Optional[str] = ..., run_id: _Optional[str] = ..., step_id: _Optional[str] = ..., attempt: _Optional[int] = ..., generation: _Optional[int] = ..., lease_token: _Optional[str] = ..., traceparent: _Optional[str] = ..., instance: _Optional[_Union[ExecutionInstanceIdentity, _Mapping]] = ...) -> None: ...

class ResolvedSecret(_message.Message):
    __slots__ = ("name", "value", "version")
    NAME_FIELD_NUMBER: _ClassVar[int]
    VALUE_FIELD_NUMBER: _ClassVar[int]
    VERSION_FIELD_NUMBER: _ClassVar[int]
    name: str
    value: bytes
    version: int
    def __init__(self, name: _Optional[str] = ..., value: _Optional[bytes] = ..., version: _Optional[int] = ...) -> None: ...

class ResolveSecretsResponse(_message.Message):
    __slots__ = ("secrets",)
    SECRETS_FIELD_NUMBER: _ClassVar[int]
    secrets: _containers.RepeatedCompositeFieldContainer[ResolvedSecret]
    def __init__(self, secrets: _Optional[_Iterable[_Union[ResolvedSecret, _Mapping]]] = ...) -> None: ...

class WriteBlobRequest(_message.Message):
    __slots__ = ("header", "chunk")
    HEADER_FIELD_NUMBER: _ClassVar[int]
    CHUNK_FIELD_NUMBER: _ClassVar[int]
    header: WriteBlobHeader
    chunk: bytes
    def __init__(self, header: _Optional[_Union[WriteBlobHeader, _Mapping]] = ..., chunk: _Optional[bytes] = ...) -> None: ...

class WriteBlobHeader(_message.Message):
    __slots__ = ("task_id", "run_id", "step_id", "attempt", "generation", "lease_token", "content_type", "expected_size", "expected_sha256", "instance")
    TASK_ID_FIELD_NUMBER: _ClassVar[int]
    RUN_ID_FIELD_NUMBER: _ClassVar[int]
    STEP_ID_FIELD_NUMBER: _ClassVar[int]
    ATTEMPT_FIELD_NUMBER: _ClassVar[int]
    GENERATION_FIELD_NUMBER: _ClassVar[int]
    LEASE_TOKEN_FIELD_NUMBER: _ClassVar[int]
    CONTENT_TYPE_FIELD_NUMBER: _ClassVar[int]
    EXPECTED_SIZE_FIELD_NUMBER: _ClassVar[int]
    EXPECTED_SHA256_FIELD_NUMBER: _ClassVar[int]
    INSTANCE_FIELD_NUMBER: _ClassVar[int]
    task_id: str
    run_id: str
    step_id: str
    attempt: int
    generation: int
    lease_token: str
    content_type: str
    expected_size: int
    expected_sha256: str
    instance: ExecutionInstanceIdentity
    def __init__(self, task_id: _Optional[str] = ..., run_id: _Optional[str] = ..., step_id: _Optional[str] = ..., attempt: _Optional[int] = ..., generation: _Optional[int] = ..., lease_token: _Optional[str] = ..., content_type: _Optional[str] = ..., expected_size: _Optional[int] = ..., expected_sha256: _Optional[str] = ..., instance: _Optional[_Union[ExecutionInstanceIdentity, _Mapping]] = ...) -> None: ...

class WriteBlobResponse(_message.Message):
    __slots__ = ("blob_id", "size", "sha256", "content_type")
    BLOB_ID_FIELD_NUMBER: _ClassVar[int]
    SIZE_FIELD_NUMBER: _ClassVar[int]
    SHA256_FIELD_NUMBER: _ClassVar[int]
    CONTENT_TYPE_FIELD_NUMBER: _ClassVar[int]
    blob_id: str
    size: int
    sha256: str
    content_type: str
    def __init__(self, blob_id: _Optional[str] = ..., size: _Optional[int] = ..., sha256: _Optional[str] = ..., content_type: _Optional[str] = ...) -> None: ...

class ReadBlobRequest(_message.Message):
    __slots__ = ("task_id", "run_id", "step_id", "attempt", "generation", "lease_token", "blob_id", "instance", "read_token")
    TASK_ID_FIELD_NUMBER: _ClassVar[int]
    RUN_ID_FIELD_NUMBER: _ClassVar[int]
    STEP_ID_FIELD_NUMBER: _ClassVar[int]
    ATTEMPT_FIELD_NUMBER: _ClassVar[int]
    GENERATION_FIELD_NUMBER: _ClassVar[int]
    LEASE_TOKEN_FIELD_NUMBER: _ClassVar[int]
    BLOB_ID_FIELD_NUMBER: _ClassVar[int]
    INSTANCE_FIELD_NUMBER: _ClassVar[int]
    READ_TOKEN_FIELD_NUMBER: _ClassVar[int]
    task_id: str
    run_id: str
    step_id: str
    attempt: int
    generation: int
    lease_token: str
    blob_id: str
    instance: ExecutionInstanceIdentity
    read_token: str
    def __init__(self, task_id: _Optional[str] = ..., run_id: _Optional[str] = ..., step_id: _Optional[str] = ..., attempt: _Optional[int] = ..., generation: _Optional[int] = ..., lease_token: _Optional[str] = ..., blob_id: _Optional[str] = ..., instance: _Optional[_Union[ExecutionInstanceIdentity, _Mapping]] = ..., read_token: _Optional[str] = ...) -> None: ...

class ReadBlobResponse(_message.Message):
    __slots__ = ("header", "chunk")
    HEADER_FIELD_NUMBER: _ClassVar[int]
    CHUNK_FIELD_NUMBER: _ClassVar[int]
    header: ReadBlobHeader
    chunk: bytes
    def __init__(self, header: _Optional[_Union[ReadBlobHeader, _Mapping]] = ..., chunk: _Optional[bytes] = ...) -> None: ...

class ReadBlobHeader(_message.Message):
    __slots__ = ("blob_id", "size", "sha256", "content_type")
    BLOB_ID_FIELD_NUMBER: _ClassVar[int]
    SIZE_FIELD_NUMBER: _ClassVar[int]
    SHA256_FIELD_NUMBER: _ClassVar[int]
    CONTENT_TYPE_FIELD_NUMBER: _ClassVar[int]
    blob_id: str
    size: int
    sha256: str
    content_type: str
    def __init__(self, blob_id: _Optional[str] = ..., size: _Optional[int] = ..., sha256: _Optional[str] = ..., content_type: _Optional[str] = ...) -> None: ...

class StepCommandResponse(_message.Message):
    __slots__ = ("command_id", "already_applied", "step_status", "run_status", "workflow_task")
    COMMAND_ID_FIELD_NUMBER: _ClassVar[int]
    ALREADY_APPLIED_FIELD_NUMBER: _ClassVar[int]
    STEP_STATUS_FIELD_NUMBER: _ClassVar[int]
    RUN_STATUS_FIELD_NUMBER: _ClassVar[int]
    WORKFLOW_TASK_FIELD_NUMBER: _ClassVar[int]
    command_id: str
    already_applied: bool
    step_status: str
    run_status: str
    workflow_task: TaskLease
    def __init__(self, command_id: _Optional[str] = ..., already_applied: _Optional[bool] = ..., step_status: _Optional[str] = ..., run_status: _Optional[str] = ..., workflow_task: _Optional[_Union[TaskLease, _Mapping]] = ...) -> None: ...

class DatabaseValue(_message.Message):
    __slots__ = ("null_value", "boolean_value", "integer_value", "decimal_value", "float_value", "text_value", "timestamp_value", "date_value", "binary_value", "json_value")
    NULL_VALUE_FIELD_NUMBER: _ClassVar[int]
    BOOLEAN_VALUE_FIELD_NUMBER: _ClassVar[int]
    INTEGER_VALUE_FIELD_NUMBER: _ClassVar[int]
    DECIMAL_VALUE_FIELD_NUMBER: _ClassVar[int]
    FLOAT_VALUE_FIELD_NUMBER: _ClassVar[int]
    TEXT_VALUE_FIELD_NUMBER: _ClassVar[int]
    TIMESTAMP_VALUE_FIELD_NUMBER: _ClassVar[int]
    DATE_VALUE_FIELD_NUMBER: _ClassVar[int]
    BINARY_VALUE_FIELD_NUMBER: _ClassVar[int]
    JSON_VALUE_FIELD_NUMBER: _ClassVar[int]
    null_value: bool
    boolean_value: bool
    integer_value: str
    decimal_value: str
    float_value: float
    text_value: str
    timestamp_value: str
    date_value: str
    binary_value: bytes
    json_value: str
    def __init__(self, null_value: _Optional[bool] = ..., boolean_value: _Optional[bool] = ..., integer_value: _Optional[str] = ..., decimal_value: _Optional[str] = ..., float_value: _Optional[float] = ..., text_value: _Optional[str] = ..., timestamp_value: _Optional[str] = ..., date_value: _Optional[str] = ..., binary_value: _Optional[bytes] = ..., json_value: _Optional[str] = ...) -> None: ...

class PostgresConnection(_message.Message):
    __slots__ = ("host", "port", "database", "tls", "ca_pem")
    HOST_FIELD_NUMBER: _ClassVar[int]
    PORT_FIELD_NUMBER: _ClassVar[int]
    DATABASE_FIELD_NUMBER: _ClassVar[int]
    TLS_FIELD_NUMBER: _ClassVar[int]
    CA_PEM_FIELD_NUMBER: _ClassVar[int]
    host: str
    port: int
    database: str
    tls: DatabaseTlsMode
    ca_pem: str
    def __init__(self, host: _Optional[str] = ..., port: _Optional[int] = ..., database: _Optional[str] = ..., tls: _Optional[_Union[DatabaseTlsMode, str]] = ..., ca_pem: _Optional[str] = ...) -> None: ...

class DatabaseLimits(_message.Message):
    __slots__ = ("max_statement_bytes", "max_parameters", "max_parameter_bytes", "max_rows", "max_result_bytes", "timeout_ms", "connect_timeout_ms", "pool_size", "idle_timeout_ms")
    MAX_STATEMENT_BYTES_FIELD_NUMBER: _ClassVar[int]
    MAX_PARAMETERS_FIELD_NUMBER: _ClassVar[int]
    MAX_PARAMETER_BYTES_FIELD_NUMBER: _ClassVar[int]
    MAX_ROWS_FIELD_NUMBER: _ClassVar[int]
    MAX_RESULT_BYTES_FIELD_NUMBER: _ClassVar[int]
    TIMEOUT_MS_FIELD_NUMBER: _ClassVar[int]
    CONNECT_TIMEOUT_MS_FIELD_NUMBER: _ClassVar[int]
    POOL_SIZE_FIELD_NUMBER: _ClassVar[int]
    IDLE_TIMEOUT_MS_FIELD_NUMBER: _ClassVar[int]
    max_statement_bytes: int
    max_parameters: int
    max_parameter_bytes: int
    max_rows: int
    max_result_bytes: int
    timeout_ms: int
    connect_timeout_ms: int
    pool_size: int
    idle_timeout_ms: int
    def __init__(self, max_statement_bytes: _Optional[int] = ..., max_parameters: _Optional[int] = ..., max_parameter_bytes: _Optional[int] = ..., max_rows: _Optional[int] = ..., max_result_bytes: _Optional[int] = ..., timeout_ms: _Optional[int] = ..., connect_timeout_ms: _Optional[int] = ..., pool_size: _Optional[int] = ..., idle_timeout_ms: _Optional[int] = ...) -> None: ...

class LakeLimits(_message.Message):
    __slots__ = ("max_record_bytes", "max_records", "timeout_ms")
    MAX_RECORD_BYTES_FIELD_NUMBER: _ClassVar[int]
    MAX_RECORDS_FIELD_NUMBER: _ClassVar[int]
    TIMEOUT_MS_FIELD_NUMBER: _ClassVar[int]
    max_record_bytes: int
    max_records: int
    timeout_ms: int
    def __init__(self, max_record_bytes: _Optional[int] = ..., max_records: _Optional[int] = ..., timeout_ms: _Optional[int] = ...) -> None: ...

class IcebergRestCatalog(_message.Message):
    __slots__ = ("uri", "warehouse", "prefix", "auth", "token_uri", "scope", "region", "signing_name", "vended_credentials", "remote_signing")
    URI_FIELD_NUMBER: _ClassVar[int]
    WAREHOUSE_FIELD_NUMBER: _ClassVar[int]
    PREFIX_FIELD_NUMBER: _ClassVar[int]
    AUTH_FIELD_NUMBER: _ClassVar[int]
    TOKEN_URI_FIELD_NUMBER: _ClassVar[int]
    SCOPE_FIELD_NUMBER: _ClassVar[int]
    REGION_FIELD_NUMBER: _ClassVar[int]
    SIGNING_NAME_FIELD_NUMBER: _ClassVar[int]
    VENDED_CREDENTIALS_FIELD_NUMBER: _ClassVar[int]
    REMOTE_SIGNING_FIELD_NUMBER: _ClassVar[int]
    uri: str
    warehouse: str
    prefix: str
    auth: LakeRestAuth
    token_uri: str
    scope: str
    region: str
    signing_name: str
    vended_credentials: bool
    remote_signing: bool
    def __init__(self, uri: _Optional[str] = ..., warehouse: _Optional[str] = ..., prefix: _Optional[str] = ..., auth: _Optional[_Union[LakeRestAuth, str]] = ..., token_uri: _Optional[str] = ..., scope: _Optional[str] = ..., region: _Optional[str] = ..., signing_name: _Optional[str] = ..., vended_credentials: _Optional[bool] = ..., remote_signing: _Optional[bool] = ...) -> None: ...

class IcebergGlueCatalog(_message.Message):
    __slots__ = ("region", "catalog_id", "role_arn", "external_id")
    REGION_FIELD_NUMBER: _ClassVar[int]
    CATALOG_ID_FIELD_NUMBER: _ClassVar[int]
    ROLE_ARN_FIELD_NUMBER: _ClassVar[int]
    EXTERNAL_ID_FIELD_NUMBER: _ClassVar[int]
    region: str
    catalog_id: str
    role_arn: str
    external_id: str
    def __init__(self, region: _Optional[str] = ..., catalog_id: _Optional[str] = ..., role_arn: _Optional[str] = ..., external_id: _Optional[str] = ...) -> None: ...

class IcebergHiveCatalog(_message.Message):
    __slots__ = ("uris", "tls_required")
    URIS_FIELD_NUMBER: _ClassVar[int]
    TLS_REQUIRED_FIELD_NUMBER: _ClassVar[int]
    uris: _containers.RepeatedScalarFieldContainer[str]
    tls_required: bool
    def __init__(self, uris: _Optional[_Iterable[str]] = ..., tls_required: _Optional[bool] = ...) -> None: ...

class IcebergJdbcCatalog(_message.Message):
    __slots__ = ("host", "port", "database", "tls", "ca_pem", "engine")
    HOST_FIELD_NUMBER: _ClassVar[int]
    PORT_FIELD_NUMBER: _ClassVar[int]
    DATABASE_FIELD_NUMBER: _ClassVar[int]
    TLS_FIELD_NUMBER: _ClassVar[int]
    CA_PEM_FIELD_NUMBER: _ClassVar[int]
    ENGINE_FIELD_NUMBER: _ClassVar[int]
    host: str
    port: int
    database: str
    tls: DatabaseTlsMode
    ca_pem: str
    engine: str
    def __init__(self, host: _Optional[str] = ..., port: _Optional[int] = ..., database: _Optional[str] = ..., tls: _Optional[_Union[DatabaseTlsMode, str]] = ..., ca_pem: _Optional[str] = ..., engine: _Optional[str] = ...) -> None: ...

class LakeS3Storage(_message.Message):
    __slots__ = ("region", "endpoint", "path_style", "role_arn", "external_id")
    REGION_FIELD_NUMBER: _ClassVar[int]
    ENDPOINT_FIELD_NUMBER: _ClassVar[int]
    PATH_STYLE_FIELD_NUMBER: _ClassVar[int]
    ROLE_ARN_FIELD_NUMBER: _ClassVar[int]
    EXTERNAL_ID_FIELD_NUMBER: _ClassVar[int]
    region: str
    endpoint: str
    path_style: bool
    role_arn: str
    external_id: str
    def __init__(self, region: _Optional[str] = ..., endpoint: _Optional[str] = ..., path_style: _Optional[bool] = ..., role_arn: _Optional[str] = ..., external_id: _Optional[str] = ...) -> None: ...

class LakeGcsStorage(_message.Message):
    __slots__ = ("project", "endpoint")
    PROJECT_FIELD_NUMBER: _ClassVar[int]
    ENDPOINT_FIELD_NUMBER: _ClassVar[int]
    project: str
    endpoint: str
    def __init__(self, project: _Optional[str] = ..., endpoint: _Optional[str] = ...) -> None: ...

class LakeAdlsStorage(_message.Message):
    __slots__ = ("account", "endpoint_suffix")
    ACCOUNT_FIELD_NUMBER: _ClassVar[int]
    ENDPOINT_SUFFIX_FIELD_NUMBER: _ClassVar[int]
    account: str
    endpoint_suffix: str
    def __init__(self, account: _Optional[str] = ..., endpoint_suffix: _Optional[str] = ...) -> None: ...

class LakeStorage(_message.Message):
    __slots__ = ("s3", "gcs", "adls", "vended_credentials")
    S3_FIELD_NUMBER: _ClassVar[int]
    GCS_FIELD_NUMBER: _ClassVar[int]
    ADLS_FIELD_NUMBER: _ClassVar[int]
    VENDED_CREDENTIALS_FIELD_NUMBER: _ClassVar[int]
    s3: LakeS3Storage
    gcs: LakeGcsStorage
    adls: LakeAdlsStorage
    vended_credentials: bool
    def __init__(self, s3: _Optional[_Union[LakeS3Storage, _Mapping]] = ..., gcs: _Optional[_Union[LakeGcsStorage, _Mapping]] = ..., adls: _Optional[_Union[LakeAdlsStorage, _Mapping]] = ..., vended_credentials: _Optional[bool] = ...) -> None: ...

class IcebergConnection(_message.Message):
    __slots__ = ("rest", "glue", "hive", "jdbc", "storage", "warehouse_locations", "allowed_modes", "egress_ceiling", "limits", "credentials")
    class CredentialsEntry(_message.Message):
        __slots__ = ("key", "value")
        KEY_FIELD_NUMBER: _ClassVar[int]
        VALUE_FIELD_NUMBER: _ClassVar[int]
        key: str
        value: bytes
        def __init__(self, key: _Optional[str] = ..., value: _Optional[bytes] = ...) -> None: ...
    REST_FIELD_NUMBER: _ClassVar[int]
    GLUE_FIELD_NUMBER: _ClassVar[int]
    HIVE_FIELD_NUMBER: _ClassVar[int]
    JDBC_FIELD_NUMBER: _ClassVar[int]
    STORAGE_FIELD_NUMBER: _ClassVar[int]
    WAREHOUSE_LOCATIONS_FIELD_NUMBER: _ClassVar[int]
    ALLOWED_MODES_FIELD_NUMBER: _ClassVar[int]
    EGRESS_CEILING_FIELD_NUMBER: _ClassVar[int]
    LIMITS_FIELD_NUMBER: _ClassVar[int]
    CREDENTIALS_FIELD_NUMBER: _ClassVar[int]
    rest: IcebergRestCatalog
    glue: IcebergGlueCatalog
    hive: IcebergHiveCatalog
    jdbc: IcebergJdbcCatalog
    storage: LakeStorage
    warehouse_locations: _containers.RepeatedScalarFieldContainer[str]
    allowed_modes: _containers.RepeatedScalarFieldContainer[str]
    egress_ceiling: EgressCeiling
    limits: LakeLimits
    credentials: _containers.ScalarMap[str, bytes]
    def __init__(self, rest: _Optional[_Union[IcebergRestCatalog, _Mapping]] = ..., glue: _Optional[_Union[IcebergGlueCatalog, _Mapping]] = ..., hive: _Optional[_Union[IcebergHiveCatalog, _Mapping]] = ..., jdbc: _Optional[_Union[IcebergJdbcCatalog, _Mapping]] = ..., storage: _Optional[_Union[LakeStorage, _Mapping]] = ..., warehouse_locations: _Optional[_Iterable[str]] = ..., allowed_modes: _Optional[_Iterable[str]] = ..., egress_ceiling: _Optional[_Union[EgressCeiling, str]] = ..., limits: _Optional[_Union[LakeLimits, _Mapping]] = ..., credentials: _Optional[_Mapping[str, bytes]] = ...) -> None: ...

class ObjectStorageConnection(_message.Message):
    __slots__ = ("storage", "prefix", "formats", "conditional_create", "egress_ceiling", "limits", "credentials")
    class CredentialsEntry(_message.Message):
        __slots__ = ("key", "value")
        KEY_FIELD_NUMBER: _ClassVar[int]
        VALUE_FIELD_NUMBER: _ClassVar[int]
        key: str
        value: bytes
        def __init__(self, key: _Optional[str] = ..., value: _Optional[bytes] = ...) -> None: ...
    STORAGE_FIELD_NUMBER: _ClassVar[int]
    PREFIX_FIELD_NUMBER: _ClassVar[int]
    FORMATS_FIELD_NUMBER: _ClassVar[int]
    CONDITIONAL_CREATE_FIELD_NUMBER: _ClassVar[int]
    EGRESS_CEILING_FIELD_NUMBER: _ClassVar[int]
    LIMITS_FIELD_NUMBER: _ClassVar[int]
    CREDENTIALS_FIELD_NUMBER: _ClassVar[int]
    storage: LakeStorage
    prefix: str
    formats: _containers.RepeatedScalarFieldContainer[str]
    conditional_create: LakeConditionalCreate
    egress_ceiling: EgressCeiling
    limits: LakeLimits
    credentials: _containers.ScalarMap[str, bytes]
    def __init__(self, storage: _Optional[_Union[LakeStorage, _Mapping]] = ..., prefix: _Optional[str] = ..., formats: _Optional[_Iterable[str]] = ..., conditional_create: _Optional[_Union[LakeConditionalCreate, str]] = ..., egress_ceiling: _Optional[_Union[EgressCeiling, str]] = ..., limits: _Optional[_Union[LakeLimits, _Mapping]] = ..., credentials: _Optional[_Mapping[str, bytes]] = ...) -> None: ...

class ClickHouseConnection(_message.Message):
    __slots__ = ("url", "database", "ca_pem", "egress_ceiling", "limits", "credentials")
    class CredentialsEntry(_message.Message):
        __slots__ = ("key", "value")
        KEY_FIELD_NUMBER: _ClassVar[int]
        VALUE_FIELD_NUMBER: _ClassVar[int]
        key: str
        value: bytes
        def __init__(self, key: _Optional[str] = ..., value: _Optional[bytes] = ...) -> None: ...
    URL_FIELD_NUMBER: _ClassVar[int]
    DATABASE_FIELD_NUMBER: _ClassVar[int]
    CA_PEM_FIELD_NUMBER: _ClassVar[int]
    EGRESS_CEILING_FIELD_NUMBER: _ClassVar[int]
    LIMITS_FIELD_NUMBER: _ClassVar[int]
    CREDENTIALS_FIELD_NUMBER: _ClassVar[int]
    url: str
    database: str
    ca_pem: str
    egress_ceiling: EgressCeiling
    limits: LakeLimits
    credentials: _containers.ScalarMap[str, bytes]
    def __init__(self, url: _Optional[str] = ..., database: _Optional[str] = ..., ca_pem: _Optional[str] = ..., egress_ceiling: _Optional[_Union[EgressCeiling, str]] = ..., limits: _Optional[_Union[LakeLimits, _Mapping]] = ..., credentials: _Optional[_Mapping[str, bytes]] = ...) -> None: ...

class ResolvedDatabase(_message.Message):
    __slots__ = ("connection_id", "connection_revision", "postgres", "operation", "statement", "parameters", "limits", "credential", "lease_expires_at_epoch_ms", "namespace_id", "connection_name", "tenant_id", "iceberg", "object_storage", "clickhouse")
    CONNECTION_ID_FIELD_NUMBER: _ClassVar[int]
    CONNECTION_REVISION_FIELD_NUMBER: _ClassVar[int]
    POSTGRES_FIELD_NUMBER: _ClassVar[int]
    OPERATION_FIELD_NUMBER: _ClassVar[int]
    STATEMENT_FIELD_NUMBER: _ClassVar[int]
    PARAMETERS_FIELD_NUMBER: _ClassVar[int]
    LIMITS_FIELD_NUMBER: _ClassVar[int]
    CREDENTIAL_FIELD_NUMBER: _ClassVar[int]
    LEASE_EXPIRES_AT_EPOCH_MS_FIELD_NUMBER: _ClassVar[int]
    NAMESPACE_ID_FIELD_NUMBER: _ClassVar[int]
    CONNECTION_NAME_FIELD_NUMBER: _ClassVar[int]
    TENANT_ID_FIELD_NUMBER: _ClassVar[int]
    ICEBERG_FIELD_NUMBER: _ClassVar[int]
    OBJECT_STORAGE_FIELD_NUMBER: _ClassVar[int]
    CLICKHOUSE_FIELD_NUMBER: _ClassVar[int]
    connection_id: str
    connection_revision: int
    postgres: PostgresConnection
    operation: DatabaseOperationKind
    statement: str
    parameters: _containers.RepeatedCompositeFieldContainer[DatabaseValue]
    limits: DatabaseLimits
    credential: ResolvedSecret
    lease_expires_at_epoch_ms: int
    namespace_id: str
    connection_name: str
    tenant_id: str
    iceberg: IcebergConnection
    object_storage: ObjectStorageConnection
    clickhouse: ClickHouseConnection
    def __init__(self, connection_id: _Optional[str] = ..., connection_revision: _Optional[int] = ..., postgres: _Optional[_Union[PostgresConnection, _Mapping]] = ..., operation: _Optional[_Union[DatabaseOperationKind, str]] = ..., statement: _Optional[str] = ..., parameters: _Optional[_Iterable[_Union[DatabaseValue, _Mapping]]] = ..., limits: _Optional[_Union[DatabaseLimits, _Mapping]] = ..., credential: _Optional[_Union[ResolvedSecret, _Mapping]] = ..., lease_expires_at_epoch_ms: _Optional[int] = ..., namespace_id: _Optional[str] = ..., connection_name: _Optional[str] = ..., tenant_id: _Optional[str] = ..., iceberg: _Optional[_Union[IcebergConnection, _Mapping]] = ..., object_storage: _Optional[_Union[ObjectStorageConnection, _Mapping]] = ..., clickhouse: _Optional[_Union[ClickHouseConnection, _Mapping]] = ...) -> None: ...

class AppendStepLogsRequest(_message.Message):
    __slots__ = ("worker_id", "session_token", "task_id", "generation", "lease_token", "run_id", "step_id", "attempt", "instance", "records", "final_batch", "highest_sequence")
    WORKER_ID_FIELD_NUMBER: _ClassVar[int]
    SESSION_TOKEN_FIELD_NUMBER: _ClassVar[int]
    TASK_ID_FIELD_NUMBER: _ClassVar[int]
    GENERATION_FIELD_NUMBER: _ClassVar[int]
    LEASE_TOKEN_FIELD_NUMBER: _ClassVar[int]
    RUN_ID_FIELD_NUMBER: _ClassVar[int]
    STEP_ID_FIELD_NUMBER: _ClassVar[int]
    ATTEMPT_FIELD_NUMBER: _ClassVar[int]
    INSTANCE_FIELD_NUMBER: _ClassVar[int]
    RECORDS_FIELD_NUMBER: _ClassVar[int]
    FINAL_BATCH_FIELD_NUMBER: _ClassVar[int]
    HIGHEST_SEQUENCE_FIELD_NUMBER: _ClassVar[int]
    worker_id: str
    session_token: str
    task_id: str
    generation: int
    lease_token: str
    run_id: str
    step_id: str
    attempt: int
    instance: ExecutionInstanceIdentity
    records: _containers.RepeatedCompositeFieldContainer[StepLogRecord]
    final_batch: bool
    highest_sequence: int
    def __init__(self, worker_id: _Optional[str] = ..., session_token: _Optional[str] = ..., task_id: _Optional[str] = ..., generation: _Optional[int] = ..., lease_token: _Optional[str] = ..., run_id: _Optional[str] = ..., step_id: _Optional[str] = ..., attempt: _Optional[int] = ..., instance: _Optional[_Union[ExecutionInstanceIdentity, _Mapping]] = ..., records: _Optional[_Iterable[_Union[StepLogRecord, _Mapping]]] = ..., final_batch: _Optional[bool] = ..., highest_sequence: _Optional[int] = ...) -> None: ...

class StepLogRecord(_message.Message):
    __slots__ = ("sequence", "worker_time_epoch_ms", "level", "message", "fields", "exception", "truncated")
    SEQUENCE_FIELD_NUMBER: _ClassVar[int]
    WORKER_TIME_EPOCH_MS_FIELD_NUMBER: _ClassVar[int]
    LEVEL_FIELD_NUMBER: _ClassVar[int]
    MESSAGE_FIELD_NUMBER: _ClassVar[int]
    FIELDS_FIELD_NUMBER: _ClassVar[int]
    EXCEPTION_FIELD_NUMBER: _ClassVar[int]
    TRUNCATED_FIELD_NUMBER: _ClassVar[int]
    sequence: int
    worker_time_epoch_ms: int
    level: StepLogLevel
    message: str
    fields: _containers.RepeatedCompositeFieldContainer[StepLogField]
    exception: str
    truncated: bool
    def __init__(self, sequence: _Optional[int] = ..., worker_time_epoch_ms: _Optional[int] = ..., level: _Optional[_Union[StepLogLevel, str]] = ..., message: _Optional[str] = ..., fields: _Optional[_Iterable[_Union[StepLogField, _Mapping]]] = ..., exception: _Optional[str] = ..., truncated: _Optional[bool] = ...) -> None: ...

class StepLogField(_message.Message):
    __slots__ = ("key", "string_value", "int_value", "double_value", "bool_value")
    KEY_FIELD_NUMBER: _ClassVar[int]
    STRING_VALUE_FIELD_NUMBER: _ClassVar[int]
    INT_VALUE_FIELD_NUMBER: _ClassVar[int]
    DOUBLE_VALUE_FIELD_NUMBER: _ClassVar[int]
    BOOL_VALUE_FIELD_NUMBER: _ClassVar[int]
    key: str
    string_value: str
    int_value: int
    double_value: float
    bool_value: bool
    def __init__(self, key: _Optional[str] = ..., string_value: _Optional[str] = ..., int_value: _Optional[int] = ..., double_value: _Optional[float] = ..., bool_value: _Optional[bool] = ...) -> None: ...

class AppendStepLogsResponse(_message.Message):
    __slots__ = ("verdict", "stop_reason", "stored_through_sequence", "retry_after_ms")
    VERDICT_FIELD_NUMBER: _ClassVar[int]
    STOP_REASON_FIELD_NUMBER: _ClassVar[int]
    STORED_THROUGH_SEQUENCE_FIELD_NUMBER: _ClassVar[int]
    RETRY_AFTER_MS_FIELD_NUMBER: _ClassVar[int]
    verdict: StepLogVerdict
    stop_reason: StepLogStopReason
    stored_through_sequence: int
    retry_after_ms: int
    def __init__(self, verdict: _Optional[_Union[StepLogVerdict, str]] = ..., stop_reason: _Optional[_Union[StepLogStopReason, str]] = ..., stored_through_sequence: _Optional[int] = ..., retry_after_ms: _Optional[int] = ...) -> None: ...

class StepLogPolicy(_message.Message):
    __slots__ = ("enabled",)
    ENABLED_FIELD_NUMBER: _ClassVar[int]
    enabled: bool
    def __init__(self, enabled: _Optional[bool] = ...) -> None: ...

class WorkflowTaskLease(_message.Message):
    __slots__ = ("run_id", "attempt", "generation", "lease_token", "expires_at_epoch_ms", "seed", "started_at_us", "window_start_seq", "gated_entries", "state_max_bytes", "first_page", "visible_count", "continue_as_new_suggested", "history_limit_reached", "traceparent", "workflow_type", "code_digest")
    RUN_ID_FIELD_NUMBER: _ClassVar[int]
    ATTEMPT_FIELD_NUMBER: _ClassVar[int]
    GENERATION_FIELD_NUMBER: _ClassVar[int]
    LEASE_TOKEN_FIELD_NUMBER: _ClassVar[int]
    EXPIRES_AT_EPOCH_MS_FIELD_NUMBER: _ClassVar[int]
    SEED_FIELD_NUMBER: _ClassVar[int]
    STARTED_AT_US_FIELD_NUMBER: _ClassVar[int]
    WINDOW_START_SEQ_FIELD_NUMBER: _ClassVar[int]
    GATED_ENTRIES_FIELD_NUMBER: _ClassVar[int]
    STATE_MAX_BYTES_FIELD_NUMBER: _ClassVar[int]
    FIRST_PAGE_FIELD_NUMBER: _ClassVar[int]
    VISIBLE_COUNT_FIELD_NUMBER: _ClassVar[int]
    CONTINUE_AS_NEW_SUGGESTED_FIELD_NUMBER: _ClassVar[int]
    HISTORY_LIMIT_REACHED_FIELD_NUMBER: _ClassVar[int]
    TRACEPARENT_FIELD_NUMBER: _ClassVar[int]
    WORKFLOW_TYPE_FIELD_NUMBER: _ClassVar[int]
    CODE_DIGEST_FIELD_NUMBER: _ClassVar[int]
    run_id: str
    attempt: int
    generation: int
    lease_token: str
    expires_at_epoch_ms: int
    seed: bytes
    started_at_us: int
    window_start_seq: int
    gated_entries: _containers.RepeatedScalarFieldContainer[str]
    state_max_bytes: int
    first_page: _containers.RepeatedCompositeFieldContainer[HistoryEvent]
    visible_count: int
    continue_as_new_suggested: bool
    history_limit_reached: bool
    traceparent: str
    workflow_type: str
    code_digest: str
    def __init__(self, run_id: _Optional[str] = ..., attempt: _Optional[int] = ..., generation: _Optional[int] = ..., lease_token: _Optional[str] = ..., expires_at_epoch_ms: _Optional[int] = ..., seed: _Optional[bytes] = ..., started_at_us: _Optional[int] = ..., window_start_seq: _Optional[int] = ..., gated_entries: _Optional[_Iterable[str]] = ..., state_max_bytes: _Optional[int] = ..., first_page: _Optional[_Iterable[_Union[HistoryEvent, _Mapping]]] = ..., visible_count: _Optional[int] = ..., continue_as_new_suggested: _Optional[bool] = ..., history_limit_reached: _Optional[bool] = ..., traceparent: _Optional[str] = ..., workflow_type: _Optional[str] = ..., code_digest: _Optional[str] = ...) -> None: ...

class HistoryPayload(_message.Message):
    __slots__ = ("json", "blob", "placeholder")
    JSON_FIELD_NUMBER: _ClassVar[int]
    BLOB_FIELD_NUMBER: _ClassVar[int]
    PLACEHOLDER_FIELD_NUMBER: _ClassVar[int]
    json: bytes
    blob: HistoryBlobRef
    placeholder: str
    def __init__(self, json: _Optional[bytes] = ..., blob: _Optional[_Union[HistoryBlobRef, _Mapping]] = ..., placeholder: _Optional[str] = ...) -> None: ...

class HistoryBlobRef(_message.Message):
    __slots__ = ("blob_id", "size", "sha256", "content_type")
    BLOB_ID_FIELD_NUMBER: _ClassVar[int]
    SIZE_FIELD_NUMBER: _ClassVar[int]
    SHA256_FIELD_NUMBER: _ClassVar[int]
    CONTENT_TYPE_FIELD_NUMBER: _ClassVar[int]
    blob_id: str
    size: int
    sha256: str
    content_type: str
    def __init__(self, blob_id: _Optional[str] = ..., size: _Optional[int] = ..., sha256: _Optional[str] = ..., content_type: _Optional[str] = ...) -> None: ...

class HistoryEvent(_message.Message):
    __slots__ = ("seq", "kind", "origin", "command_number", "at_us", "run_started", "workflow_task_started", "workflow_task_completed", "command_record", "step_outcome", "timer_fired", "wait_outcome", "child_finished", "approval_decided", "saga_outcome", "signal_buffered", "update_admitted", "cancellation_requested", "marker_recorded")
    SEQ_FIELD_NUMBER: _ClassVar[int]
    KIND_FIELD_NUMBER: _ClassVar[int]
    ORIGIN_FIELD_NUMBER: _ClassVar[int]
    COMMAND_NUMBER_FIELD_NUMBER: _ClassVar[int]
    AT_US_FIELD_NUMBER: _ClassVar[int]
    RUN_STARTED_FIELD_NUMBER: _ClassVar[int]
    WORKFLOW_TASK_STARTED_FIELD_NUMBER: _ClassVar[int]
    WORKFLOW_TASK_COMPLETED_FIELD_NUMBER: _ClassVar[int]
    COMMAND_RECORD_FIELD_NUMBER: _ClassVar[int]
    STEP_OUTCOME_FIELD_NUMBER: _ClassVar[int]
    TIMER_FIRED_FIELD_NUMBER: _ClassVar[int]
    WAIT_OUTCOME_FIELD_NUMBER: _ClassVar[int]
    CHILD_FINISHED_FIELD_NUMBER: _ClassVar[int]
    APPROVAL_DECIDED_FIELD_NUMBER: _ClassVar[int]
    SAGA_OUTCOME_FIELD_NUMBER: _ClassVar[int]
    SIGNAL_BUFFERED_FIELD_NUMBER: _ClassVar[int]
    UPDATE_ADMITTED_FIELD_NUMBER: _ClassVar[int]
    CANCELLATION_REQUESTED_FIELD_NUMBER: _ClassVar[int]
    MARKER_RECORDED_FIELD_NUMBER: _ClassVar[int]
    seq: int
    kind: HistoryEventKind
    origin: HistoryEventOrigin
    command_number: int
    at_us: int
    run_started: HistoryRunStarted
    workflow_task_started: HistoryWorkflowTaskStarted
    workflow_task_completed: HistoryWorkflowTaskCompleted
    command_record: HistoryCommandRecord
    step_outcome: HistoryStepOutcome
    timer_fired: HistoryTimerFired
    wait_outcome: HistoryWaitOutcome
    child_finished: HistoryChildFinished
    approval_decided: HistoryApprovalDecided
    saga_outcome: HistorySagaOutcome
    signal_buffered: HistorySignalBuffered
    update_admitted: HistoryUpdateAdmitted
    cancellation_requested: HistoryCancellationRequested
    marker_recorded: HistoryMarkerRecorded
    def __init__(self, seq: _Optional[int] = ..., kind: _Optional[_Union[HistoryEventKind, str]] = ..., origin: _Optional[_Union[HistoryEventOrigin, str]] = ..., command_number: _Optional[int] = ..., at_us: _Optional[int] = ..., run_started: _Optional[_Union[HistoryRunStarted, _Mapping]] = ..., workflow_task_started: _Optional[_Union[HistoryWorkflowTaskStarted, _Mapping]] = ..., workflow_task_completed: _Optional[_Union[HistoryWorkflowTaskCompleted, _Mapping]] = ..., command_record: _Optional[_Union[HistoryCommandRecord, _Mapping]] = ..., step_outcome: _Optional[_Union[HistoryStepOutcome, _Mapping]] = ..., timer_fired: _Optional[_Union[HistoryTimerFired, _Mapping]] = ..., wait_outcome: _Optional[_Union[HistoryWaitOutcome, _Mapping]] = ..., child_finished: _Optional[_Union[HistoryChildFinished, _Mapping]] = ..., approval_decided: _Optional[_Union[HistoryApprovalDecided, _Mapping]] = ..., saga_outcome: _Optional[_Union[HistorySagaOutcome, _Mapping]] = ..., signal_buffered: _Optional[_Union[HistorySignalBuffered, _Mapping]] = ..., update_admitted: _Optional[_Union[HistoryUpdateAdmitted, _Mapping]] = ..., cancellation_requested: _Optional[_Union[HistoryCancellationRequested, _Mapping]] = ..., marker_recorded: _Optional[_Union[HistoryMarkerRecorded, _Mapping]] = ...) -> None: ...

class HistoryRunStarted(_message.Message):
    __slots__ = ("input", "carried_state", "predecessor_run_id")
    INPUT_FIELD_NUMBER: _ClassVar[int]
    CARRIED_STATE_FIELD_NUMBER: _ClassVar[int]
    PREDECESSOR_RUN_ID_FIELD_NUMBER: _ClassVar[int]
    input: HistoryPayload
    carried_state: HistoryPayload
    predecessor_run_id: str
    def __init__(self, input: _Optional[_Union[HistoryPayload, _Mapping]] = ..., carried_state: _Optional[_Union[HistoryPayload, _Mapping]] = ..., predecessor_run_id: _Optional[str] = ...) -> None: ...

class HistoryWorkflowTaskStarted(_message.Message):
    __slots__ = ("continue_as_new_suggested", "history_limit_reached")
    CONTINUE_AS_NEW_SUGGESTED_FIELD_NUMBER: _ClassVar[int]
    HISTORY_LIMIT_REACHED_FIELD_NUMBER: _ClassVar[int]
    continue_as_new_suggested: bool
    history_limit_reached: bool
    def __init__(self, continue_as_new_suggested: _Optional[bool] = ..., history_limit_reached: _Optional[bool] = ...) -> None: ...

class HistoryWorkflowTaskCompleted(_message.Message):
    __slots__ = ("deferred", "deferred_kind", "deferred_target", "deferral_cause")
    DEFERRED_FIELD_NUMBER: _ClassVar[int]
    DEFERRED_KIND_FIELD_NUMBER: _ClassVar[int]
    DEFERRED_TARGET_FIELD_NUMBER: _ClassVar[int]
    DEFERRAL_CAUSE_FIELD_NUMBER: _ClassVar[int]
    deferred: bool
    deferred_kind: CommandKind
    deferred_target: str
    deferral_cause: WorkflowTaskFailureCause
    def __init__(self, deferred: _Optional[bool] = ..., deferred_kind: _Optional[_Union[CommandKind, str]] = ..., deferred_target: _Optional[str] = ..., deferral_cause: _Optional[_Union[WorkflowTaskFailureCause, str]] = ...) -> None: ...

class HistoryCommandRecord(_message.Message):
    __slots__ = ("kind", "target", "node", "noop")
    KIND_FIELD_NUMBER: _ClassVar[int]
    TARGET_FIELD_NUMBER: _ClassVar[int]
    NODE_FIELD_NUMBER: _ClassVar[int]
    NOOP_FIELD_NUMBER: _ClassVar[int]
    kind: CommandKind
    target: str
    node: str
    noop: bool
    def __init__(self, kind: _Optional[_Union[CommandKind, str]] = ..., target: _Optional[str] = ..., node: _Optional[str] = ..., noop: _Optional[bool] = ...) -> None: ...

class HistoryStepOutcome(_message.Message):
    __slots__ = ("node", "outcome", "output", "error_code", "retryable")
    NODE_FIELD_NUMBER: _ClassVar[int]
    OUTCOME_FIELD_NUMBER: _ClassVar[int]
    OUTPUT_FIELD_NUMBER: _ClassVar[int]
    ERROR_CODE_FIELD_NUMBER: _ClassVar[int]
    RETRYABLE_FIELD_NUMBER: _ClassVar[int]
    node: str
    outcome: StepOutcome
    output: HistoryPayload
    error_code: str
    retryable: bool
    def __init__(self, node: _Optional[str] = ..., outcome: _Optional[_Union[StepOutcome, str]] = ..., output: _Optional[_Union[HistoryPayload, _Mapping]] = ..., error_code: _Optional[str] = ..., retryable: _Optional[bool] = ...) -> None: ...

class HistoryTimerFired(_message.Message):
    __slots__ = ("node",)
    NODE_FIELD_NUMBER: _ClassVar[int]
    node: str
    def __init__(self, node: _Optional[str] = ...) -> None: ...

class HistoryWaitOutcome(_message.Message):
    __slots__ = ("node", "timed_out", "payload")
    NODE_FIELD_NUMBER: _ClassVar[int]
    TIMED_OUT_FIELD_NUMBER: _ClassVar[int]
    PAYLOAD_FIELD_NUMBER: _ClassVar[int]
    node: str
    timed_out: bool
    payload: HistoryPayload
    def __init__(self, node: _Optional[str] = ..., timed_out: _Optional[bool] = ..., payload: _Optional[_Union[HistoryPayload, _Mapping]] = ...) -> None: ...

class HistoryChildFinished(_message.Message):
    __slots__ = ("node", "child_run_id", "status", "output", "error_code")
    NODE_FIELD_NUMBER: _ClassVar[int]
    CHILD_RUN_ID_FIELD_NUMBER: _ClassVar[int]
    STATUS_FIELD_NUMBER: _ClassVar[int]
    OUTPUT_FIELD_NUMBER: _ClassVar[int]
    ERROR_CODE_FIELD_NUMBER: _ClassVar[int]
    node: str
    child_run_id: str
    status: str
    output: HistoryPayload
    error_code: str
    def __init__(self, node: _Optional[str] = ..., child_run_id: _Optional[str] = ..., status: _Optional[str] = ..., output: _Optional[_Union[HistoryPayload, _Mapping]] = ..., error_code: _Optional[str] = ...) -> None: ...

class HistoryApprovalDecided(_message.Message):
    __slots__ = ("node", "decision")
    NODE_FIELD_NUMBER: _ClassVar[int]
    DECISION_FIELD_NUMBER: _ClassVar[int]
    node: str
    decision: str
    def __init__(self, node: _Optional[str] = ..., decision: _Optional[str] = ...) -> None: ...

class HistorySagaOutcome(_message.Message):
    __slots__ = ("scope", "outcome")
    SCOPE_FIELD_NUMBER: _ClassVar[int]
    OUTCOME_FIELD_NUMBER: _ClassVar[int]
    scope: str
    outcome: str
    def __init__(self, scope: _Optional[str] = ..., outcome: _Optional[str] = ...) -> None: ...

class HistorySignalBuffered(_message.Message):
    __slots__ = ("signal_name", "payload")
    SIGNAL_NAME_FIELD_NUMBER: _ClassVar[int]
    PAYLOAD_FIELD_NUMBER: _ClassVar[int]
    signal_name: str
    payload: HistoryPayload
    def __init__(self, signal_name: _Optional[str] = ..., payload: _Optional[_Union[HistoryPayload, _Mapping]] = ...) -> None: ...

class HistoryUpdateAdmitted(_message.Message):
    __slots__ = ("update_name", "args")
    UPDATE_NAME_FIELD_NUMBER: _ClassVar[int]
    ARGS_FIELD_NUMBER: _ClassVar[int]
    update_name: str
    args: HistoryPayload
    def __init__(self, update_name: _Optional[str] = ..., args: _Optional[_Union[HistoryPayload, _Mapping]] = ...) -> None: ...

class HistoryCancellationRequested(_message.Message):
    __slots__ = ("grace_ms", "cancel_origin")
    GRACE_MS_FIELD_NUMBER: _ClassVar[int]
    CANCEL_ORIGIN_FIELD_NUMBER: _ClassVar[int]
    grace_ms: int
    cancel_origin: CancelOrigin
    def __init__(self, grace_ms: _Optional[int] = ..., cancel_origin: _Optional[_Union[CancelOrigin, str]] = ...) -> None: ...

class HistoryMarkerRecorded(_message.Message):
    __slots__ = ("marker_id", "side_effect", "value")
    MARKER_ID_FIELD_NUMBER: _ClassVar[int]
    SIDE_EFFECT_FIELD_NUMBER: _ClassVar[int]
    VALUE_FIELD_NUMBER: _ClassVar[int]
    marker_id: str
    side_effect: bool
    value: HistoryPayload
    def __init__(self, marker_id: _Optional[str] = ..., side_effect: _Optional[bool] = ..., value: _Optional[_Union[HistoryPayload, _Mapping]] = ...) -> None: ...

class CommandValue(_message.Message):
    __slots__ = ("json", "blob_id", "descriptor_sha256", "descriptor_size")
    JSON_FIELD_NUMBER: _ClassVar[int]
    BLOB_ID_FIELD_NUMBER: _ClassVar[int]
    DESCRIPTOR_SHA256_FIELD_NUMBER: _ClassVar[int]
    DESCRIPTOR_SIZE_FIELD_NUMBER: _ClassVar[int]
    json: bytes
    blob_id: str
    descriptor_sha256: str
    descriptor_size: int
    def __init__(self, json: _Optional[bytes] = ..., blob_id: _Optional[str] = ..., descriptor_sha256: _Optional[str] = ..., descriptor_size: _Optional[int] = ...) -> None: ...

class Command(_message.Message):
    __slots__ = ("command_number", "kind", "fragment_json", "value", "saga_scope", "target", "saga_open", "saga_close", "saga_abort", "marker", "set_state", "update_decision", "update_result", "consume_signals", "complete", "fail", "continue_as_new", "acknowledge_cancellation")
    COMMAND_NUMBER_FIELD_NUMBER: _ClassVar[int]
    KIND_FIELD_NUMBER: _ClassVar[int]
    FRAGMENT_JSON_FIELD_NUMBER: _ClassVar[int]
    VALUE_FIELD_NUMBER: _ClassVar[int]
    SAGA_SCOPE_FIELD_NUMBER: _ClassVar[int]
    TARGET_FIELD_NUMBER: _ClassVar[int]
    SAGA_OPEN_FIELD_NUMBER: _ClassVar[int]
    SAGA_CLOSE_FIELD_NUMBER: _ClassVar[int]
    SAGA_ABORT_FIELD_NUMBER: _ClassVar[int]
    MARKER_FIELD_NUMBER: _ClassVar[int]
    SET_STATE_FIELD_NUMBER: _ClassVar[int]
    UPDATE_DECISION_FIELD_NUMBER: _ClassVar[int]
    UPDATE_RESULT_FIELD_NUMBER: _ClassVar[int]
    CONSUME_SIGNALS_FIELD_NUMBER: _ClassVar[int]
    COMPLETE_FIELD_NUMBER: _ClassVar[int]
    FAIL_FIELD_NUMBER: _ClassVar[int]
    CONTINUE_AS_NEW_FIELD_NUMBER: _ClassVar[int]
    ACKNOWLEDGE_CANCELLATION_FIELD_NUMBER: _ClassVar[int]
    command_number: int
    kind: CommandKind
    fragment_json: bytes
    value: CommandValue
    saga_scope: str
    target: TargetCommand
    saga_open: SagaOpenCommand
    saga_close: TargetCommand
    saga_abort: TargetCommand
    marker: MarkerCommand
    set_state: CommandValue
    update_decision: UpdateDecisionCommand
    update_result: UpdateResultCommand
    consume_signals: ConsumeSignalsCommand
    complete: CommandValue
    fail: FailCommand
    continue_as_new: ContinueAsNewCommand
    acknowledge_cancellation: AcknowledgeCancellationCommand
    def __init__(self, command_number: _Optional[int] = ..., kind: _Optional[_Union[CommandKind, str]] = ..., fragment_json: _Optional[bytes] = ..., value: _Optional[_Union[CommandValue, _Mapping]] = ..., saga_scope: _Optional[str] = ..., target: _Optional[_Union[TargetCommand, _Mapping]] = ..., saga_open: _Optional[_Union[SagaOpenCommand, _Mapping]] = ..., saga_close: _Optional[_Union[TargetCommand, _Mapping]] = ..., saga_abort: _Optional[_Union[TargetCommand, _Mapping]] = ..., marker: _Optional[_Union[MarkerCommand, _Mapping]] = ..., set_state: _Optional[_Union[CommandValue, _Mapping]] = ..., update_decision: _Optional[_Union[UpdateDecisionCommand, _Mapping]] = ..., update_result: _Optional[_Union[UpdateResultCommand, _Mapping]] = ..., consume_signals: _Optional[_Union[ConsumeSignalsCommand, _Mapping]] = ..., complete: _Optional[_Union[CommandValue, _Mapping]] = ..., fail: _Optional[_Union[FailCommand, _Mapping]] = ..., continue_as_new: _Optional[_Union[ContinueAsNewCommand, _Mapping]] = ..., acknowledge_cancellation: _Optional[_Union[AcknowledgeCancellationCommand, _Mapping]] = ...) -> None: ...

class TargetCommand(_message.Message):
    __slots__ = ("target",)
    TARGET_FIELD_NUMBER: _ClassVar[int]
    target: str
    def __init__(self, target: _Optional[str] = ...) -> None: ...

class SagaOpenCommand(_message.Message):
    __slots__ = ("scope", "parent_scope", "on_compensation_failure", "cancel_policy")
    SCOPE_FIELD_NUMBER: _ClassVar[int]
    PARENT_SCOPE_FIELD_NUMBER: _ClassVar[int]
    ON_COMPENSATION_FAILURE_FIELD_NUMBER: _ClassVar[int]
    CANCEL_POLICY_FIELD_NUMBER: _ClassVar[int]
    scope: str
    parent_scope: str
    on_compensation_failure: str
    cancel_policy: str
    def __init__(self, scope: _Optional[str] = ..., parent_scope: _Optional[str] = ..., on_compensation_failure: _Optional[str] = ..., cancel_policy: _Optional[str] = ...) -> None: ...

class MarkerCommand(_message.Message):
    __slots__ = ("marker_id", "side_effect_value")
    MARKER_ID_FIELD_NUMBER: _ClassVar[int]
    SIDE_EFFECT_VALUE_FIELD_NUMBER: _ClassVar[int]
    marker_id: str
    side_effect_value: CommandValue
    def __init__(self, marker_id: _Optional[str] = ..., side_effect_value: _Optional[_Union[CommandValue, _Mapping]] = ...) -> None: ...

class UpdateDecisionCommand(_message.Message):
    __slots__ = ("admitted_seq", "accepted", "rejection_reason")
    ADMITTED_SEQ_FIELD_NUMBER: _ClassVar[int]
    ACCEPTED_FIELD_NUMBER: _ClassVar[int]
    REJECTION_REASON_FIELD_NUMBER: _ClassVar[int]
    admitted_seq: int
    accepted: bool
    rejection_reason: str
    def __init__(self, admitted_seq: _Optional[int] = ..., accepted: _Optional[bool] = ..., rejection_reason: _Optional[str] = ...) -> None: ...

class UpdateResultCommand(_message.Message):
    __slots__ = ("admitted_seq", "result", "failure_code")
    ADMITTED_SEQ_FIELD_NUMBER: _ClassVar[int]
    RESULT_FIELD_NUMBER: _ClassVar[int]
    FAILURE_CODE_FIELD_NUMBER: _ClassVar[int]
    admitted_seq: int
    result: CommandValue
    failure_code: str
    def __init__(self, admitted_seq: _Optional[int] = ..., result: _Optional[_Union[CommandValue, _Mapping]] = ..., failure_code: _Optional[str] = ...) -> None: ...

class ConsumeSignalsCommand(_message.Message):
    __slots__ = ("signals",)
    SIGNALS_FIELD_NUMBER: _ClassVar[int]
    signals: _containers.RepeatedCompositeFieldContainer[ConsumedSignals]
    def __init__(self, signals: _Optional[_Iterable[_Union[ConsumedSignals, _Mapping]]] = ...) -> None: ...

class ConsumedSignals(_message.Message):
    __slots__ = ("signal_name", "seqs")
    SIGNAL_NAME_FIELD_NUMBER: _ClassVar[int]
    SEQS_FIELD_NUMBER: _ClassVar[int]
    signal_name: str
    seqs: _containers.RepeatedScalarFieldContainer[int]
    def __init__(self, signal_name: _Optional[str] = ..., seqs: _Optional[_Iterable[int]] = ...) -> None: ...

class FailCommand(_message.Message):
    __slots__ = ("code", "safe_message")
    CODE_FIELD_NUMBER: _ClassVar[int]
    SAFE_MESSAGE_FIELD_NUMBER: _ClassVar[int]
    code: str
    safe_message: str
    def __init__(self, code: _Optional[str] = ..., safe_message: _Optional[str] = ...) -> None: ...

class ContinueAsNewCommand(_message.Message):
    __slots__ = ("input", "state", "active_version")
    INPUT_FIELD_NUMBER: _ClassVar[int]
    STATE_FIELD_NUMBER: _ClassVar[int]
    ACTIVE_VERSION_FIELD_NUMBER: _ClassVar[int]
    input: CommandValue
    state: CommandValue
    active_version: bool
    def __init__(self, input: _Optional[_Union[CommandValue, _Mapping]] = ..., state: _Optional[_Union[CommandValue, _Mapping]] = ..., active_version: _Optional[bool] = ...) -> None: ...

class AcknowledgeCancellationCommand(_message.Message):
    __slots__ = ()
    def __init__(self) -> None: ...

class CompleteWorkflowTaskRequest(_message.Message):
    __slots__ = ("task_id", "run_id", "attempt", "generation", "lease_token", "idempotency_key", "applied_through_seq", "replayed_chain", "commands", "deferred_command", "eager_activity_slots", "behaviour_flags", "traceparent")
    TASK_ID_FIELD_NUMBER: _ClassVar[int]
    RUN_ID_FIELD_NUMBER: _ClassVar[int]
    ATTEMPT_FIELD_NUMBER: _ClassVar[int]
    GENERATION_FIELD_NUMBER: _ClassVar[int]
    LEASE_TOKEN_FIELD_NUMBER: _ClassVar[int]
    IDEMPOTENCY_KEY_FIELD_NUMBER: _ClassVar[int]
    APPLIED_THROUGH_SEQ_FIELD_NUMBER: _ClassVar[int]
    REPLAYED_CHAIN_FIELD_NUMBER: _ClassVar[int]
    COMMANDS_FIELD_NUMBER: _ClassVar[int]
    DEFERRED_COMMAND_FIELD_NUMBER: _ClassVar[int]
    EAGER_ACTIVITY_SLOTS_FIELD_NUMBER: _ClassVar[int]
    BEHAVIOUR_FLAGS_FIELD_NUMBER: _ClassVar[int]
    TRACEPARENT_FIELD_NUMBER: _ClassVar[int]
    task_id: str
    run_id: str
    attempt: int
    generation: int
    lease_token: str
    idempotency_key: str
    applied_through_seq: int
    replayed_chain: bytes
    commands: _containers.RepeatedCompositeFieldContainer[Command]
    deferred_command: Command
    eager_activity_slots: int
    behaviour_flags: _containers.RepeatedScalarFieldContainer[str]
    traceparent: str
    def __init__(self, task_id: _Optional[str] = ..., run_id: _Optional[str] = ..., attempt: _Optional[int] = ..., generation: _Optional[int] = ..., lease_token: _Optional[str] = ..., idempotency_key: _Optional[str] = ..., applied_through_seq: _Optional[int] = ..., replayed_chain: _Optional[bytes] = ..., commands: _Optional[_Iterable[_Union[Command, _Mapping]]] = ..., deferred_command: _Optional[_Union[Command, _Mapping]] = ..., eager_activity_slots: _Optional[int] = ..., behaviour_flags: _Optional[_Iterable[str]] = ..., traceparent: _Optional[str] = ...) -> None: ...

class CompleteWorkflowTaskResponse(_message.Message):
    __slots__ = ("outcome", "cause", "already_applied", "eager_activity_leases")
    OUTCOME_FIELD_NUMBER: _ClassVar[int]
    CAUSE_FIELD_NUMBER: _ClassVar[int]
    ALREADY_APPLIED_FIELD_NUMBER: _ClassVar[int]
    EAGER_ACTIVITY_LEASES_FIELD_NUMBER: _ClassVar[int]
    outcome: WorkflowTaskOutcome
    cause: WorkflowTaskFailureCause
    already_applied: bool
    eager_activity_leases: _containers.RepeatedCompositeFieldContainer[TaskLease]
    def __init__(self, outcome: _Optional[_Union[WorkflowTaskOutcome, str]] = ..., cause: _Optional[_Union[WorkflowTaskFailureCause, str]] = ..., already_applied: _Optional[bool] = ..., eager_activity_leases: _Optional[_Iterable[_Union[TaskLease, _Mapping]]] = ...) -> None: ...

class FailWorkflowTaskRequest(_message.Message):
    __slots__ = ("task_id", "run_id", "attempt", "generation", "lease_token", "idempotency_key", "cause", "command_number", "actual_kind", "actual_target", "exception_type", "code_digest", "traceparent")
    TASK_ID_FIELD_NUMBER: _ClassVar[int]
    RUN_ID_FIELD_NUMBER: _ClassVar[int]
    ATTEMPT_FIELD_NUMBER: _ClassVar[int]
    GENERATION_FIELD_NUMBER: _ClassVar[int]
    LEASE_TOKEN_FIELD_NUMBER: _ClassVar[int]
    IDEMPOTENCY_KEY_FIELD_NUMBER: _ClassVar[int]
    CAUSE_FIELD_NUMBER: _ClassVar[int]
    COMMAND_NUMBER_FIELD_NUMBER: _ClassVar[int]
    ACTUAL_KIND_FIELD_NUMBER: _ClassVar[int]
    ACTUAL_TARGET_FIELD_NUMBER: _ClassVar[int]
    EXCEPTION_TYPE_FIELD_NUMBER: _ClassVar[int]
    CODE_DIGEST_FIELD_NUMBER: _ClassVar[int]
    TRACEPARENT_FIELD_NUMBER: _ClassVar[int]
    task_id: str
    run_id: str
    attempt: int
    generation: int
    lease_token: str
    idempotency_key: str
    cause: WorkflowTaskFailureCause
    command_number: int
    actual_kind: CommandKind
    actual_target: str
    exception_type: str
    code_digest: str
    traceparent: str
    def __init__(self, task_id: _Optional[str] = ..., run_id: _Optional[str] = ..., attempt: _Optional[int] = ..., generation: _Optional[int] = ..., lease_token: _Optional[str] = ..., idempotency_key: _Optional[str] = ..., cause: _Optional[_Union[WorkflowTaskFailureCause, str]] = ..., command_number: _Optional[int] = ..., actual_kind: _Optional[_Union[CommandKind, str]] = ..., actual_target: _Optional[str] = ..., exception_type: _Optional[str] = ..., code_digest: _Optional[str] = ..., traceparent: _Optional[str] = ...) -> None: ...

class FailWorkflowTaskResponse(_message.Message):
    __slots__ = ("already_applied",)
    ALREADY_APPLIED_FIELD_NUMBER: _ClassVar[int]
    already_applied: bool
    def __init__(self, already_applied: _Optional[bool] = ...) -> None: ...

class ReadHistoryRequest(_message.Message):
    __slots__ = ("task_id", "run_id", "attempt", "generation", "lease_token", "read_token", "from_seq")
    TASK_ID_FIELD_NUMBER: _ClassVar[int]
    RUN_ID_FIELD_NUMBER: _ClassVar[int]
    ATTEMPT_FIELD_NUMBER: _ClassVar[int]
    GENERATION_FIELD_NUMBER: _ClassVar[int]
    LEASE_TOKEN_FIELD_NUMBER: _ClassVar[int]
    READ_TOKEN_FIELD_NUMBER: _ClassVar[int]
    FROM_SEQ_FIELD_NUMBER: _ClassVar[int]
    task_id: str
    run_id: str
    attempt: int
    generation: int
    lease_token: str
    read_token: str
    from_seq: int
    def __init__(self, task_id: _Optional[str] = ..., run_id: _Optional[str] = ..., attempt: _Optional[int] = ..., generation: _Optional[int] = ..., lease_token: _Optional[str] = ..., read_token: _Optional[str] = ..., from_seq: _Optional[int] = ...) -> None: ...

class ReadHistoryResponse(_message.Message):
    __slots__ = ("events", "last_page", "lease_expires_at_epoch_ms")
    EVENTS_FIELD_NUMBER: _ClassVar[int]
    LAST_PAGE_FIELD_NUMBER: _ClassVar[int]
    LEASE_EXPIRES_AT_EPOCH_MS_FIELD_NUMBER: _ClassVar[int]
    events: _containers.RepeatedCompositeFieldContainer[HistoryEvent]
    last_page: bool
    lease_expires_at_epoch_ms: int
    def __init__(self, events: _Optional[_Iterable[_Union[HistoryEvent, _Mapping]]] = ..., last_page: _Optional[bool] = ..., lease_expires_at_epoch_ms: _Optional[int] = ...) -> None: ...

class QueryTask(_message.Message):
    __slots__ = ("query_id", "run_id", "query_name", "args", "deadline_epoch_ms", "gated_entries", "state_max_bytes", "read_token", "lease_token", "workflow_type", "code_digest", "seed")
    QUERY_ID_FIELD_NUMBER: _ClassVar[int]
    RUN_ID_FIELD_NUMBER: _ClassVar[int]
    QUERY_NAME_FIELD_NUMBER: _ClassVar[int]
    ARGS_FIELD_NUMBER: _ClassVar[int]
    DEADLINE_EPOCH_MS_FIELD_NUMBER: _ClassVar[int]
    GATED_ENTRIES_FIELD_NUMBER: _ClassVar[int]
    STATE_MAX_BYTES_FIELD_NUMBER: _ClassVar[int]
    READ_TOKEN_FIELD_NUMBER: _ClassVar[int]
    LEASE_TOKEN_FIELD_NUMBER: _ClassVar[int]
    WORKFLOW_TYPE_FIELD_NUMBER: _ClassVar[int]
    CODE_DIGEST_FIELD_NUMBER: _ClassVar[int]
    SEED_FIELD_NUMBER: _ClassVar[int]
    query_id: str
    run_id: str
    query_name: str
    args: bytes
    deadline_epoch_ms: int
    gated_entries: _containers.RepeatedScalarFieldContainer[str]
    state_max_bytes: int
    read_token: str
    lease_token: str
    workflow_type: str
    code_digest: str
    seed: bytes
    def __init__(self, query_id: _Optional[str] = ..., run_id: _Optional[str] = ..., query_name: _Optional[str] = ..., args: _Optional[bytes] = ..., deadline_epoch_ms: _Optional[int] = ..., gated_entries: _Optional[_Iterable[str]] = ..., state_max_bytes: _Optional[int] = ..., read_token: _Optional[str] = ..., lease_token: _Optional[str] = ..., workflow_type: _Optional[str] = ..., code_digest: _Optional[str] = ..., seed: _Optional[bytes] = ...) -> None: ...

class RespondWorkerQueryRequest(_message.Message):
    __slots__ = ("query_id", "lease_token", "answer", "committed_through_seq", "failure_code")
    QUERY_ID_FIELD_NUMBER: _ClassVar[int]
    LEASE_TOKEN_FIELD_NUMBER: _ClassVar[int]
    ANSWER_FIELD_NUMBER: _ClassVar[int]
    COMMITTED_THROUGH_SEQ_FIELD_NUMBER: _ClassVar[int]
    FAILURE_CODE_FIELD_NUMBER: _ClassVar[int]
    query_id: str
    lease_token: str
    answer: bytes
    committed_through_seq: int
    failure_code: str
    def __init__(self, query_id: _Optional[str] = ..., lease_token: _Optional[str] = ..., answer: _Optional[bytes] = ..., committed_through_seq: _Optional[int] = ..., failure_code: _Optional[str] = ...) -> None: ...

class RespondWorkerQueryResponse(_message.Message):
    __slots__ = ("accepted",)
    ACCEPTED_FIELD_NUMBER: _ClassVar[int]
    accepted: bool
    def __init__(self, accepted: _Optional[bool] = ...) -> None: ...

class ReplayCheckTask(_message.Message):
    __slots__ = ("items", "gated_entries", "state_max_bytes", "lease_token", "expires_at_epoch_ms", "workflow_type", "code_digest")
    ITEMS_FIELD_NUMBER: _ClassVar[int]
    GATED_ENTRIES_FIELD_NUMBER: _ClassVar[int]
    STATE_MAX_BYTES_FIELD_NUMBER: _ClassVar[int]
    LEASE_TOKEN_FIELD_NUMBER: _ClassVar[int]
    EXPIRES_AT_EPOCH_MS_FIELD_NUMBER: _ClassVar[int]
    WORKFLOW_TYPE_FIELD_NUMBER: _ClassVar[int]
    CODE_DIGEST_FIELD_NUMBER: _ClassVar[int]
    items: _containers.RepeatedCompositeFieldContainer[ReplayCheckItem]
    gated_entries: _containers.RepeatedScalarFieldContainer[str]
    state_max_bytes: int
    lease_token: str
    expires_at_epoch_ms: int
    workflow_type: str
    code_digest: str
    def __init__(self, items: _Optional[_Iterable[_Union[ReplayCheckItem, _Mapping]]] = ..., gated_entries: _Optional[_Iterable[str]] = ..., state_max_bytes: _Optional[int] = ..., lease_token: _Optional[str] = ..., expires_at_epoch_ms: _Optional[int] = ..., workflow_type: _Optional[str] = ..., code_digest: _Optional[str] = ...) -> None: ...

class ReplayCheckItem(_message.Message):
    __slots__ = ("job_id", "ordinal", "run_id", "read_token", "seed")
    JOB_ID_FIELD_NUMBER: _ClassVar[int]
    ORDINAL_FIELD_NUMBER: _ClassVar[int]
    RUN_ID_FIELD_NUMBER: _ClassVar[int]
    READ_TOKEN_FIELD_NUMBER: _ClassVar[int]
    SEED_FIELD_NUMBER: _ClassVar[int]
    job_id: str
    ordinal: int
    run_id: str
    read_token: str
    seed: bytes
    def __init__(self, job_id: _Optional[str] = ..., ordinal: _Optional[int] = ..., run_id: _Optional[str] = ..., read_token: _Optional[str] = ..., seed: _Optional[bytes] = ...) -> None: ...

class RespondReplayCheckRequest(_message.Message):
    __slots__ = ("lease_token", "job_id", "ordinal", "from_command", "body_digests", "last_range", "s_k", "chain", "deferred_body_digest")
    LEASE_TOKEN_FIELD_NUMBER: _ClassVar[int]
    JOB_ID_FIELD_NUMBER: _ClassVar[int]
    ORDINAL_FIELD_NUMBER: _ClassVar[int]
    FROM_COMMAND_FIELD_NUMBER: _ClassVar[int]
    BODY_DIGESTS_FIELD_NUMBER: _ClassVar[int]
    LAST_RANGE_FIELD_NUMBER: _ClassVar[int]
    S_K_FIELD_NUMBER: _ClassVar[int]
    CHAIN_FIELD_NUMBER: _ClassVar[int]
    DEFERRED_BODY_DIGEST_FIELD_NUMBER: _ClassVar[int]
    lease_token: str
    job_id: str
    ordinal: int
    from_command: int
    body_digests: _containers.RepeatedScalarFieldContainer[bytes]
    last_range: bool
    s_k: int
    chain: bytes
    deferred_body_digest: bytes
    def __init__(self, lease_token: _Optional[str] = ..., job_id: _Optional[str] = ..., ordinal: _Optional[int] = ..., from_command: _Optional[int] = ..., body_digests: _Optional[_Iterable[bytes]] = ..., last_range: _Optional[bool] = ..., s_k: _Optional[int] = ..., chain: _Optional[bytes] = ..., deferred_body_digest: _Optional[bytes] = ...) -> None: ...

class RespondReplayCheckResponse(_message.Message):
    __slots__ = ("accepted", "next_from_command")
    ACCEPTED_FIELD_NUMBER: _ClassVar[int]
    NEXT_FROM_COMMAND_FIELD_NUMBER: _ClassVar[int]
    accepted: bool
    next_from_command: int
    def __init__(self, accepted: _Optional[bool] = ..., next_from_command: _Optional[int] = ...) -> None: ...

class ReleaseStickyRequest(_message.Message):
    __slots__ = ("worker_id", "session_token", "run_ids")
    WORKER_ID_FIELD_NUMBER: _ClassVar[int]
    SESSION_TOKEN_FIELD_NUMBER: _ClassVar[int]
    RUN_IDS_FIELD_NUMBER: _ClassVar[int]
    worker_id: str
    session_token: str
    run_ids: _containers.RepeatedScalarFieldContainer[str]
    def __init__(self, worker_id: _Optional[str] = ..., session_token: _Optional[str] = ..., run_ids: _Optional[_Iterable[str]] = ...) -> None: ...

class ReleaseStickyResponse(_message.Message):
    __slots__ = ("released",)
    RELEASED_FIELD_NUMBER: _ClassVar[int]
    released: int
    def __init__(self, released: _Optional[int] = ...) -> None: ...
