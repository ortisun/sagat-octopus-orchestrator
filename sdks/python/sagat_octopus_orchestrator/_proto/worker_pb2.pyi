from google.protobuf.internal import containers as _containers
from google.protobuf import descriptor as _descriptor
from google.protobuf import message as _message
from collections.abc import Iterable as _Iterable, Mapping as _Mapping
from typing import ClassVar as _ClassVar, Optional as _Optional, Union as _Union

DESCRIPTOR: _descriptor.FileDescriptor

class WorkerCapabilities(_message.Message):
    __slots__ = ("languages", "activities", "pools", "max_inflight", "protocol_version")
    LANGUAGES_FIELD_NUMBER: _ClassVar[int]
    ACTIVITIES_FIELD_NUMBER: _ClassVar[int]
    POOLS_FIELD_NUMBER: _ClassVar[int]
    MAX_INFLIGHT_FIELD_NUMBER: _ClassVar[int]
    PROTOCOL_VERSION_FIELD_NUMBER: _ClassVar[int]
    languages: _containers.RepeatedScalarFieldContainer[str]
    activities: _containers.RepeatedScalarFieldContainer[str]
    pools: _containers.RepeatedScalarFieldContainer[str]
    max_inflight: int
    protocol_version: str
    def __init__(self, languages: _Optional[_Iterable[str]] = ..., activities: _Optional[_Iterable[str]] = ..., pools: _Optional[_Iterable[str]] = ..., max_inflight: _Optional[int] = ..., protocol_version: _Optional[str] = ...) -> None: ...

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
    __slots__ = ("worker_id", "session_token", "accepted_protocol_version")
    WORKER_ID_FIELD_NUMBER: _ClassVar[int]
    SESSION_TOKEN_FIELD_NUMBER: _ClassVar[int]
    ACCEPTED_PROTOCOL_VERSION_FIELD_NUMBER: _ClassVar[int]
    worker_id: str
    session_token: str
    accepted_protocol_version: str
    def __init__(self, worker_id: _Optional[str] = ..., session_token: _Optional[str] = ..., accepted_protocol_version: _Optional[str] = ...) -> None: ...

class PollTasksRequest(_message.Message):
    __slots__ = ("worker_id", "session_token", "max_tasks")
    WORKER_ID_FIELD_NUMBER: _ClassVar[int]
    SESSION_TOKEN_FIELD_NUMBER: _ClassVar[int]
    MAX_TASKS_FIELD_NUMBER: _ClassVar[int]
    worker_id: str
    session_token: str
    max_tasks: int
    def __init__(self, worker_id: _Optional[str] = ..., session_token: _Optional[str] = ..., max_tasks: _Optional[int] = ...) -> None: ...

class TaskLease(_message.Message):
    __slots__ = ("task_id", "run_id", "step_id", "attempt", "generation", "lease_token", "expires_at_epoch_ms", "activity_name", "input_json", "input_blob_ref", "traceparent", "tenant_alias", "idempotency_key", "task_kind", "target_node", "target_idempotency_key")
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
    def __init__(self, task_id: _Optional[str] = ..., run_id: _Optional[str] = ..., step_id: _Optional[str] = ..., attempt: _Optional[int] = ..., generation: _Optional[int] = ..., lease_token: _Optional[str] = ..., expires_at_epoch_ms: _Optional[int] = ..., activity_name: _Optional[str] = ..., input_json: _Optional[str] = ..., input_blob_ref: _Optional[str] = ..., traceparent: _Optional[str] = ..., tenant_alias: _Optional[str] = ..., idempotency_key: _Optional[str] = ..., task_kind: _Optional[str] = ..., target_node: _Optional[str] = ..., target_idempotency_key: _Optional[str] = ...) -> None: ...

class PollTasksResponse(_message.Message):
    __slots__ = ("tasks", "retry_after_ms")
    TASKS_FIELD_NUMBER: _ClassVar[int]
    RETRY_AFTER_MS_FIELD_NUMBER: _ClassVar[int]
    tasks: _containers.RepeatedCompositeFieldContainer[TaskLease]
    retry_after_ms: int
    def __init__(self, tasks: _Optional[_Iterable[_Union[TaskLease, _Mapping]]] = ..., retry_after_ms: _Optional[int] = ...) -> None: ...

class HeartbeatRequest(_message.Message):
    __slots__ = ("task_id", "worker_id", "generation", "lease_token", "run_id", "step_id", "attempt")
    TASK_ID_FIELD_NUMBER: _ClassVar[int]
    WORKER_ID_FIELD_NUMBER: _ClassVar[int]
    GENERATION_FIELD_NUMBER: _ClassVar[int]
    LEASE_TOKEN_FIELD_NUMBER: _ClassVar[int]
    RUN_ID_FIELD_NUMBER: _ClassVar[int]
    STEP_ID_FIELD_NUMBER: _ClassVar[int]
    ATTEMPT_FIELD_NUMBER: _ClassVar[int]
    task_id: str
    worker_id: str
    generation: int
    lease_token: str
    run_id: str
    step_id: str
    attempt: int
    def __init__(self, task_id: _Optional[str] = ..., worker_id: _Optional[str] = ..., generation: _Optional[int] = ..., lease_token: _Optional[str] = ..., run_id: _Optional[str] = ..., step_id: _Optional[str] = ..., attempt: _Optional[int] = ...) -> None: ...

class HeartbeatResponse(_message.Message):
    __slots__ = ("lease_expires_at_epoch_ms", "cancel_requested")
    LEASE_EXPIRES_AT_EPOCH_MS_FIELD_NUMBER: _ClassVar[int]
    CANCEL_REQUESTED_FIELD_NUMBER: _ClassVar[int]
    lease_expires_at_epoch_ms: int
    cancel_requested: bool
    def __init__(self, lease_expires_at_epoch_ms: _Optional[int] = ..., cancel_requested: _Optional[bool] = ...) -> None: ...

class CompleteStepRequest(_message.Message):
    __slots__ = ("task_id", "run_id", "step_id", "attempt", "generation", "lease_token", "idempotency_key", "output_json", "output_blob_ref", "output_sha256", "traceparent")
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
    def __init__(self, task_id: _Optional[str] = ..., run_id: _Optional[str] = ..., step_id: _Optional[str] = ..., attempt: _Optional[int] = ..., generation: _Optional[int] = ..., lease_token: _Optional[str] = ..., idempotency_key: _Optional[str] = ..., output_json: _Optional[str] = ..., output_blob_ref: _Optional[str] = ..., output_sha256: _Optional[str] = ..., traceparent: _Optional[str] = ...) -> None: ...

class FailStepRequest(_message.Message):
    __slots__ = ("task_id", "run_id", "step_id", "attempt", "generation", "lease_token", "idempotency_key", "error_code", "safe_message", "retryable", "traceparent")
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
    def __init__(self, task_id: _Optional[str] = ..., run_id: _Optional[str] = ..., step_id: _Optional[str] = ..., attempt: _Optional[int] = ..., generation: _Optional[int] = ..., lease_token: _Optional[str] = ..., idempotency_key: _Optional[str] = ..., error_code: _Optional[str] = ..., safe_message: _Optional[str] = ..., retryable: _Optional[bool] = ..., traceparent: _Optional[str] = ...) -> None: ...

class ResolveSecretsRequest(_message.Message):
    __slots__ = ("task_id", "run_id", "step_id", "attempt", "generation", "lease_token", "traceparent")
    TASK_ID_FIELD_NUMBER: _ClassVar[int]
    RUN_ID_FIELD_NUMBER: _ClassVar[int]
    STEP_ID_FIELD_NUMBER: _ClassVar[int]
    ATTEMPT_FIELD_NUMBER: _ClassVar[int]
    GENERATION_FIELD_NUMBER: _ClassVar[int]
    LEASE_TOKEN_FIELD_NUMBER: _ClassVar[int]
    TRACEPARENT_FIELD_NUMBER: _ClassVar[int]
    task_id: str
    run_id: str
    step_id: str
    attempt: int
    generation: int
    lease_token: str
    traceparent: str
    def __init__(self, task_id: _Optional[str] = ..., run_id: _Optional[str] = ..., step_id: _Optional[str] = ..., attempt: _Optional[int] = ..., generation: _Optional[int] = ..., lease_token: _Optional[str] = ..., traceparent: _Optional[str] = ...) -> None: ...

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
    __slots__ = ("task_id", "run_id", "step_id", "attempt", "generation", "lease_token", "content_type", "expected_size", "expected_sha256")
    TASK_ID_FIELD_NUMBER: _ClassVar[int]
    RUN_ID_FIELD_NUMBER: _ClassVar[int]
    STEP_ID_FIELD_NUMBER: _ClassVar[int]
    ATTEMPT_FIELD_NUMBER: _ClassVar[int]
    GENERATION_FIELD_NUMBER: _ClassVar[int]
    LEASE_TOKEN_FIELD_NUMBER: _ClassVar[int]
    CONTENT_TYPE_FIELD_NUMBER: _ClassVar[int]
    EXPECTED_SIZE_FIELD_NUMBER: _ClassVar[int]
    EXPECTED_SHA256_FIELD_NUMBER: _ClassVar[int]
    task_id: str
    run_id: str
    step_id: str
    attempt: int
    generation: int
    lease_token: str
    content_type: str
    expected_size: int
    expected_sha256: str
    def __init__(self, task_id: _Optional[str] = ..., run_id: _Optional[str] = ..., step_id: _Optional[str] = ..., attempt: _Optional[int] = ..., generation: _Optional[int] = ..., lease_token: _Optional[str] = ..., content_type: _Optional[str] = ..., expected_size: _Optional[int] = ..., expected_sha256: _Optional[str] = ...) -> None: ...

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
    __slots__ = ("task_id", "run_id", "step_id", "attempt", "generation", "lease_token", "blob_id")
    TASK_ID_FIELD_NUMBER: _ClassVar[int]
    RUN_ID_FIELD_NUMBER: _ClassVar[int]
    STEP_ID_FIELD_NUMBER: _ClassVar[int]
    ATTEMPT_FIELD_NUMBER: _ClassVar[int]
    GENERATION_FIELD_NUMBER: _ClassVar[int]
    LEASE_TOKEN_FIELD_NUMBER: _ClassVar[int]
    BLOB_ID_FIELD_NUMBER: _ClassVar[int]
    task_id: str
    run_id: str
    step_id: str
    attempt: int
    generation: int
    lease_token: str
    blob_id: str
    def __init__(self, task_id: _Optional[str] = ..., run_id: _Optional[str] = ..., step_id: _Optional[str] = ..., attempt: _Optional[int] = ..., generation: _Optional[int] = ..., lease_token: _Optional[str] = ..., blob_id: _Optional[str] = ...) -> None: ...

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
    __slots__ = ("command_id", "already_applied", "step_status", "run_status")
    COMMAND_ID_FIELD_NUMBER: _ClassVar[int]
    ALREADY_APPLIED_FIELD_NUMBER: _ClassVar[int]
    STEP_STATUS_FIELD_NUMBER: _ClassVar[int]
    RUN_STATUS_FIELD_NUMBER: _ClassVar[int]
    command_id: str
    already_applied: bool
    step_status: str
    run_status: str
    def __init__(self, command_id: _Optional[str] = ..., already_applied: _Optional[bool] = ..., step_status: _Optional[str] = ..., run_status: _Optional[str] = ...) -> None: ...
