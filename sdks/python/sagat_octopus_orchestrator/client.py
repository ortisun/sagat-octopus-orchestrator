"""The client of the REST API (``sdks/SPEC.md`` section 3)."""

from __future__ import annotations

import asyncio
import hashlib
import random
import re
import time
import uuid
from collections.abc import AsyncIterable, AsyncIterator, Iterable, Mapping
from dataclasses import dataclass, field
from datetime import datetime, timedelta, timezone
from types import TracebackType
from typing import Any, Literal, cast
from urllib.parse import quote

from ._http import HttpRequest, HttpTransport
from ._json import format_instant, parse_instant
from .config import ClientConfig
from .errors import SagatError, invalid
from .traceparent import check_traceparent
from .worker.activity import BlobDescriptor

RunStatus = Literal[
    "RUNNING",
    "WAITING",
    "PAUSED",
    "CANCELLING",
    "COMPENSATING",
    "MANUAL_REVIEW",
    "SUCCEEDED",
    "FAILED",
    "CANCELLED",
    "TIMED_OUT",
    "COMPENSATED",
    "PARTIAL",
    "COMPENSATION_FAILED",
]

TERMINAL_STATUSES: frozenset[str] = frozenset(
    {"SUCCEEDED", "FAILED", "CANCELLED", "TIMED_OUT", "COMPENSATED", "PARTIAL", "COMPENSATION_FAILED"}
)

OperateAction = Literal[
    "PAUSE", "RESUME", "CANCEL", "RETRY_STEP", "RETRY_COMPENSATION", "SKIP_COMPENSATION", "RESOLVE_STEP"
]
ApprovalDecision = Literal["APPROVE", "REJECT"]
ApprovalStatus = Literal["PENDING", "APPROVED", "REJECTED", "EXPIRED", "CANCELLED"]

_UUID = re.compile(r"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}", re.IGNORECASE)
_KEY = re.compile(r"[\x21-\x7e]{8,160}")
_APPROVAL_DECISION_KEY = re.compile(r"[\x21-\x7e]{1,128}")
_APPROVAL_REASON_CODE = re.compile(r"[A-Za-z0-9_.:-]{1,64}")
_DECLARED_OPERATION_NAME = re.compile(r"[A-Za-z0-9][A-Za-z0-9_.:-]{0,127}")
_UPDATE_KEY = re.compile(r"[\x21-\x7e]{1,128}")
_MAX_I64 = (1 << 63) - 1
_NAME_TTL = 60.0


@dataclass(frozen=True)
class RunParent:
    run_id: str
    node: str
    cancel_policy: str


@dataclass(frozen=True)
class Run:
    """A run as the server reports it. ``raw`` keeps the answer as sent, so a field of a newer server is reachable."""

    run_id: str
    workflow_id: str
    version_id: str
    status: str
    """One of ``RunStatus``; a status a newer server adds is passed through as it came."""
    version: int
    """Number of journal events applied; usable as ``expected_run_version``."""
    paused: bool
    created_at: datetime
    started_by: str | None = None
    updated_at: datetime | None = None
    finished_at: datetime | None = None
    deadline: datetime | None = None
    trace_id: str | None = None
    correlation_id: str | None = None
    depth: int = 0
    parent: RunParent | None = None
    raw: Mapping[str, Any] = field(default_factory=dict, repr=False, compare=False)

    @property
    def is_terminal(self) -> bool:
        return self.status in TERMINAL_STATUSES


@dataclass(frozen=True)
class RunPage:
    items: tuple[Run, ...]
    next_cursor: str | None


@dataclass(frozen=True)
class OperationResult:
    operation_id: str
    accepted: bool
    run_status: str | None
    run_version: int | None
    raw: Mapping[str, Any] = field(default_factory=dict, repr=False, compare=False)


@dataclass(frozen=True)
class ApprovalDecisionReceipt:
    receipt_id: str
    approval_id: str
    decision_key: str
    decision: ApprovalDecision
    reason_code: str | None
    actor: str
    decided_at: datetime
    outcome_sequence: int
    replayed: bool
    raw: Mapping[str, Any] = field(default_factory=dict, repr=False, compare=False)


@dataclass(frozen=True)
class ApprovalScope:
    scope_node_id: str
    iteration_index: int


@dataclass(frozen=True)
class ApprovalListItem:
    approval_id: str
    run_id: str
    node_id: str
    instance_path: tuple[ApprovalScope, ...]
    status: ApprovalStatus
    deadline: datetime
    settled_at: datetime | None
    raw: Mapping[str, Any] = field(default_factory=dict, repr=False, compare=False)


@dataclass(frozen=True)
class ApprovalPage:
    items: tuple[ApprovalListItem, ...]
    next_cursor: str | None


@dataclass(frozen=True)
class QueryResult:
    run_id: str
    run_version: int
    state_revision: int
    result: Any
    raw: Mapping[str, Any] = field(default_factory=dict, repr=False, compare=False)


@dataclass(frozen=True)
class UpdateReceipt:
    run_id: str
    operation: str
    update_key: str
    status: Literal["APPLIED"]
    result: Any
    before_state_revision: int
    after_state_revision: int
    run_version: int
    replayed: bool
    raw: Mapping[str, Any] = field(default_factory=dict, repr=False, compare=False)


@dataclass(frozen=True)
class SignalResult:
    delivered_to: tuple[str, ...]


@dataclass(frozen=True)
class ChainSegment:
    index: int
    run_id: str
    predecessor_run_id: str | None
    successor_run_id: str | None


@dataclass(frozen=True)
class ChainInspection:
    chain_id: str
    current_run_id: str
    revision: int
    cancel_requested: bool
    final_outcome: str | None
    segments: tuple[ChainSegment, ...]
    raw: Mapping[str, Any] = field(default_factory=dict, repr=False, compare=False)


@dataclass(frozen=True)
class EventAccepted:
    event_id: str
    source: str
    status: Literal["ACCEPTED", "DUPLICATE"] | str


@dataclass(frozen=True)
class JournalEvent:
    """One event of a run's journal. ``raw`` is the answer as sent."""

    raw: Mapping[str, Any]

    @property
    def event_type(self) -> str | None:
        value = self.raw.get("eventType")
        return value if isinstance(value, str) else None

    @property
    def step_id(self) -> str | None:
        value = self.raw.get("stepId")
        return value if isinstance(value, str) else None

    @property
    def attempt(self) -> int | None:
        value = self.raw.get("attempt")
        return value if isinstance(value, int) and not isinstance(value, bool) else None


@dataclass(frozen=True)
class JournalPage:
    items: tuple[JournalEvent, ...]
    next_cursor: str | None


# ------------------------------------------------------------------------------------------------ parsing


def _object(value: Any, what: str) -> Mapping[str, Any]:
    if not isinstance(value, dict):
        raise SagatError("PROTOCOL", f"{what} is not an object")
    return value


def _str(raw: Mapping[str, Any], key: str) -> str:
    value = raw.get(key)
    if not isinstance(value, str):
        raise SagatError("PROTOCOL", f"{key} is missing from the answer")
    return value


def _opt_str(raw: Mapping[str, Any], key: str) -> str | None:
    value = raw.get(key)
    return value if isinstance(value, str) else None


def _int(raw: Mapping[str, Any], key: str, fallback: int) -> int:
    value = raw.get(key)
    return value if isinstance(value, int) and not isinstance(value, bool) else fallback


def to_run(value: Any) -> Run:
    """A ``Run`` from the server's JSON."""
    raw = _object(value, "a run")
    created = parse_instant(raw.get("createdAt"))
    if created is None:
        raise SagatError("PROTOCOL", "createdAt is missing from the answer")
    parent = raw.get("parent")
    return Run(
        run_id=_str(raw, "runId"),
        workflow_id=_str(raw, "workflowId"),
        version_id=_str(raw, "versionId"),
        status=_str(raw, "status"),
        version=_int(raw, "version", 0),
        paused=raw.get("paused") is True,
        started_by=_opt_str(raw, "startedBy"),
        created_at=created,
        updated_at=parse_instant(raw.get("updatedAt")),
        finished_at=parse_instant(raw.get("finishedAt")),
        deadline=parse_instant(raw.get("deadline")),
        trace_id=_opt_str(raw, "traceId"),
        correlation_id=_opt_str(raw, "correlationId"),
        depth=_int(raw, "depth", 0),
        parent=(
            RunParent(str(parent.get("runId")), str(parent.get("node")), str(parent.get("cancelPolicy")))
            if isinstance(parent, dict)
            else None
        ),
        raw=raw,
    )


def _to_page(body: Any) -> RunPage:
    raw = _object(body, "a page")
    items = raw.get("items")
    if not isinstance(items, list):
        raise SagatError("PROTOCOL", "items is missing from the page")
    return RunPage(tuple(to_run(item) for item in items), _opt_str(raw, "nextCursor"))


def _run_id(value: str) -> str:
    """A run id is a UUID: nothing else is ever put in the path of a call about a run."""
    if not isinstance(value, str) or not _UUID.fullmatch(value):
        raise invalid("a run id is a UUID")
    return value


def _namespace_id(value: str) -> str:
    if not isinstance(value, str) or not _UUID.fullmatch(value):
        raise invalid("a namespace id is a UUID")
    return value.lower()


def _name(value: str, what: str) -> str:
    """A name (a signal, an event type, a workflow) is not empty and is not the path segment ``.`` or ``..``."""
    if not isinstance(value, str) or value in ("", ".", ".."):
        raise invalid(f"{what} is a name: not empty, not '.' and not '..'")
    return value


def _declared_operation_name(value: str) -> str:
    if not isinstance(value, str) or not _DECLARED_OPERATION_NAME.fullmatch(value):
        raise invalid("an operation name is 1 to 128 permitted characters and starts with a letter or digit")
    return value


def _revision(raw: Mapping[str, Any], key: str, minimum: int) -> int:
    value = raw.get(key)
    if not isinstance(value, int) or isinstance(value, bool) or not minimum <= value <= _MAX_I64:
        raise SagatError("PROTOCOL", f"{key} is invalid in the answer")
    return value


def _seconds(value: float | timedelta) -> float:
    return value.total_seconds() if isinstance(value, timedelta) else float(value)


# ------------------------------------------------------------------------------------------------- client


class SagatClient:
    """The asyncio client of the REST API.

    One client holds one ``httpx.AsyncClient``: create it inside the running event loop, use it from that
    loop and ``await client.aclose()`` (or ``async with``) when done.
    """

    def __init__(self, config: ClientConfig) -> None:
        if not config.api_url or not config.namespace:
            raise invalid("api_url and namespace are required")
        self._config = config
        self._http = HttpTransport(config)
        self._names: dict[str, tuple[str, float]] = {}

    async def aclose(self) -> None:
        await self._http.aclose()

    async def __aenter__(self) -> SagatClient:
        return self

    async def __aexit__(
        self, exc_type: type[BaseException] | None, exc: BaseException | None, tb: TracebackType | None
    ) -> None:
        await self.aclose()

    # ------------------------------------------------------------------------------------------ helpers

    def _trace(self, explicit: str | None = None) -> str | None:
        value = (
            explicit if explicit is not None else (self._config.trace_context() if self._config.trace_context else None)
        )
        return None if value is None else check_traceparent(value)

    @staticmethod
    def _key(given: str | None) -> str:
        key = given if given is not None else str(uuid.uuid4())
        if not _KEY.fullmatch(key):
            raise invalid("an idempotency key is 8 to 160 printable ASCII characters")
        return key

    # --------------------------------------------------------------------------------------------- calls

    async def resolve_workflow(self, workflow: str) -> str:
        """The id of a workflow given by id or by name (the workflow of that name in the namespace)."""
        _name(workflow, "a workflow")
        if _UUID.fullmatch(workflow):
            return workflow.lower()
        cached = self._names.get(workflow)
        if cached and time.monotonic() - cached[1] < _NAME_TTL:
            return cached[0]
        cursor: str | None = None
        while True:
            response = await self._http.request(
                HttpRequest("GET", "/workflows", query={"limit": 200, "cursor": cursor}, traceparent=self._trace())
            )
            page = _object(response.body, "a page")
            items = page.get("items")
            for item in items if isinstance(items, list) else ():
                draft = _object(item, "a workflow")
                if draft.get("name") == workflow:
                    found = _str(draft, "id")
                    self._names[workflow] = (found, time.monotonic())
                    return found
            cursor = _opt_str(page, "nextCursor")
            if not cursor:
                break
        raise SagatError(
            "NOT_FOUND", f"no workflow is named {workflow!r} in this namespace", status=404, reason="WORKFLOW_NOT_FOUND"
        )

    async def start(
        self,
        workflow: str,
        input: Mapping[str, Any] | None = None,
        *,
        version_id: str | None = None,
        idempotency_key: str | None = None,
        correlation_id: str | None = None,
        deadline: datetime | None = None,
        deadline_in: float | timedelta | None = None,
        traceparent: str | None = None,
    ) -> Run:
        """Starts a run of a published workflow, given by id or by name.

        ``idempotency_key`` is 8 to 160 printable ASCII characters; a fresh one is made per call when
        absent and reused by every retry of that call. ``deadline`` is an aware instant; ``deadline_in`` is
        a duration from now (seconds or a ``timedelta``), computed once before the first attempt.
        """
        if deadline is not None and deadline_in is not None:
            raise invalid("give a deadline or a deadline_in, not both")
        if input is not None and not isinstance(input, dict):
            raise invalid("the input of a run is a JSON object (a dict)")
        key = self._key(idempotency_key)
        trace = self._trace(traceparent)
        if deadline_in is not None:
            deadline = datetime.now(timezone.utc) + timedelta(seconds=_seconds(deadline_in))
        workflow_id = await self.resolve_workflow(workflow)
        body: dict[str, Any] = {"workflowId": workflow_id, "input": input if input is not None else {}}
        if version_id:
            body["versionId"] = version_id
        if correlation_id is not None:
            body["correlationId"] = correlation_id
        if deadline is not None:
            body["deadline"] = format_instant(deadline)
        response = await self._http.request(
            HttpRequest("POST", "/runs", body=body, idempotency_key=key, traceparent=trace)
        )
        return to_run(response.body)

    async def upload_run_input(
        self,
        source: AsyncIterable[bytes] | Iterable[bytes],
        *,
        content_type: str = "application/octet-stream",
        expected_size: int | None = None,
        expected_sha256: str | None = None,
    ) -> BlobDescriptor:
        """Streams a staged run input once and verifies the returned descriptor locally."""
        if expected_size is not None and (
            isinstance(expected_size, bool) or not isinstance(expected_size, int) or expected_size < 0
        ):
            raise invalid("blob expected_size is a non-negative integer")
        if expected_sha256 is not None and not re.fullmatch(r"[0-9a-f]{64}", expected_sha256):
            raise invalid("blob expected_sha256 is lowercase SHA-256 hex")
        digest = hashlib.sha256()
        size = 0
        source_eof = False

        async def measured() -> AsyncIterator[bytes]:
            nonlocal size, source_eof
            if isinstance(source, AsyncIterable):
                iterator: Any = source
                async for chunk in iterator:
                    if not isinstance(chunk, bytes):
                        raise invalid("a blob source yields bytes")
                    size += len(chunk)
                    digest.update(chunk)
                    yield chunk
            else:
                for chunk in source:
                    if not isinstance(chunk, bytes):
                        raise invalid("a blob source yields bytes")
                    size += len(chunk)
                    digest.update(chunk)
                    yield chunk
            source_eof = True

        headers: dict[str, str] = {}
        if expected_size is not None:
            headers["x-octopus-blob-size"] = str(expected_size)
        if expected_sha256 is not None:
            headers["x-octopus-blob-sha256"] = expected_sha256
        response = await self._http.request(
            HttpRequest(
                "POST",
                "/run-input-blobs",
                stream=measured(),
                content_type=content_type,
                headers=headers,
                traceparent=self._trace(),
                safe_to_repeat=False,
            )
        )
        raw = _object(response.body, "a blob descriptor")
        descriptor = BlobDescriptor(
            _str(raw, "id"), _int(raw, "size", -1), _str(raw, "sha256"), _str(raw, "contentType")
        )
        if (
            not source_eof
            or not _UUID.fullmatch(descriptor.id)
            or descriptor.size != size
            or descriptor.sha256 != digest.hexdigest()
            or not descriptor.content_type
        ):
            raise SagatError("PROTOCOL", "the run-input descriptor does not match the uploaded bytes")
        return descriptor

    async def start_with_blob(
        self,
        workflow: str,
        descriptor: BlobDescriptor,
        *,
        version_id: str | None = None,
        idempotency_key: str | None = None,
        correlation_id: str | None = None,
        deadline: datetime | None = None,
        deadline_in: float | timedelta | None = None,
        traceparent: str | None = None,
    ) -> Run:
        """Atomically starts a run with a descriptor returned by :meth:`upload_run_input`."""
        if (
            not isinstance(descriptor, BlobDescriptor)
            or not _UUID.fullmatch(descriptor.id)
            or descriptor.size < 0
            or not re.fullmatch(r"[0-9a-f]{64}", descriptor.sha256)
            or not descriptor.content_type
        ):
            raise invalid("a run-input blob descriptor is malformed")
        if deadline is not None and deadline_in is not None:
            raise invalid("give a deadline or a deadline_in, not both")
        key = self._key(idempotency_key)
        trace = self._trace(traceparent)
        if deadline_in is not None:
            deadline = datetime.now(timezone.utc) + timedelta(seconds=_seconds(deadline_in))
        workflow_id = await self.resolve_workflow(workflow)
        body: dict[str, Any] = {"workflowId": workflow_id, "inputBlobRef": descriptor.id}
        if version_id:
            body["versionId"] = version_id
        if correlation_id is not None:
            body["correlationId"] = correlation_id
        if deadline is not None:
            body["deadline"] = format_instant(deadline)
        response = await self._http.request(
            HttpRequest("POST", "/runs", body=body, idempotency_key=key, traceparent=trace)
        )
        return to_run(response.body)

    async def get(self, run_id: str) -> Run:
        response = await self._http.request(
            HttpRequest("GET", f"/runs/{quote(_run_id(run_id), safe='')}", traceparent=self._trace())
        )
        return to_run(response.body)

    async def list(
        self,
        *,
        workflow_id: str | None = None,
        status: str | None = None,
        limit: int | None = None,
        cursor: str | None = None,
    ) -> RunPage:
        """One page of runs, newest first, as the server gave it."""
        response = await self._http.request(
            HttpRequest(
                "GET",
                "/runs",
                query={"workflowId": workflow_id, "status": status, "limit": limit, "cursor": cursor},
                traceparent=self._trace(),
            )
        )
        return _to_page(response.body)

    async def list_all(
        self, *, workflow_id: str | None = None, status: str | None = None, limit: int | None = None
    ) -> AsyncIterator[Run]:
        """Every run that matches, page after page, in the order the server gave."""
        cursor: str | None = None
        while True:
            page = await self.list(workflow_id=workflow_id, status=status, limit=limit, cursor=cursor)
            for item in page.items:
                yield item
            cursor = page.next_cursor
            if not cursor:
                return

    async def operate(
        self,
        run_id: str,
        action: OperateAction,
        reason: str,
        *,
        step_id: str | None = None,
        outcome: Literal["SUCCEEDED", "FAILED"] | None = None,
        error_code: str | None = None,
        expected_run_version: int | None = None,
        idempotency_key: str | None = None,
        traceparent: str | None = None,
    ) -> OperationResult:
        """An operation on a run: ``PAUSE``, ``RESUME``, ``CANCEL`` and the saga decisions."""
        path = f"/runs/{quote(_run_id(run_id), safe='')}/operations"
        body: dict[str, Any] = {"action": action, "reason": reason}
        if step_id is not None:
            body["stepId"] = step_id
        if outcome is not None:
            body["outcome"] = outcome
        if error_code is not None:
            body["errorCode"] = error_code
        if expected_run_version is not None:
            body["expectedRunVersion"] = expected_run_version
        response = await self._http.request(
            HttpRequest(
                "POST",
                path,
                body=body,
                idempotency_key=self._key(idempotency_key),
                traceparent=self._trace(traceparent),
            )
        )
        raw = _object(response.body, "an operation result")
        run_version = raw.get("runVersion")
        return OperationResult(
            operation_id=_str(raw, "operationId"),
            accepted=raw.get("accepted") is True,
            run_status=_opt_str(raw, "runStatus"),
            run_version=run_version if isinstance(run_version, int) and not isinstance(run_version, bool) else None,
            raw=raw,
        )

    async def cancel(
        self,
        run_id: str,
        reason: str,
        *,
        expected_run_version: int | None = None,
        idempotency_key: str | None = None,
        traceparent: str | None = None,
    ) -> OperationResult:
        """Cancels a run (the server's reason has 8 to 2000 characters)."""
        return await self.operate(
            run_id,
            "CANCEL",
            reason,
            expected_run_version=expected_run_version,
            idempotency_key=idempotency_key,
            traceparent=traceparent,
        )

    async def signal(
        self,
        run_id: str,
        name: str,
        payload: Mapping[str, Any] | None = None,
        *,
        idempotency_key: str | None = None,
        traceparent: str | None = None,
    ) -> SignalResult:
        """Sends a signal to the steps of a run that wait for it; a signal nobody waits for is a ``CONFLICT``."""
        path = f"/runs/{quote(_run_id(run_id), safe='')}/signals/{quote(_name(name, 'a signal name'), safe='')}"
        response = await self._http.request(
            HttpRequest(
                "POST",
                path,
                body=payload if payload is not None else {},
                idempotency_key=self._key(idempotency_key),
                traceparent=self._trace(traceparent),
            )
        )
        raw = _object(response.body, "a signal result")
        delivered = raw.get("deliveredTo")
        return SignalResult(tuple(str(item) for item in delivered) if isinstance(delivered, list) else ())

    async def inspect_chain(
        self, namespace_id: str, chain_id: str, *, traceparent: str | None = None
    ) -> ChainInspection:
        """Reads one committed chain head and lineage snapshot."""
        path = f"/namespaces/{quote(_namespace_id(namespace_id), safe='')}/chains/{quote(_run_id(chain_id), safe='')}"
        response = await self._http.request(HttpRequest("GET", path, traceparent=self._trace(traceparent)))
        raw = _object(response.body, "a chain inspection")
        values = raw.get("segments")
        if not isinstance(values, list) or not values:
            raise SagatError("PROTOCOL", "segments is invalid in the answer")
        segments: list[ChainSegment] = []
        for value in values:
            segment = _object(value, "a chain segment")
            index = _revision(segment, "index", 0)
            if index > 65_535:
                raise SagatError("PROTOCOL", "index is invalid in the answer")
            segments.append(
                ChainSegment(
                    index,
                    _str(segment, "runId"),
                    _opt_str(segment, "predecessorRunId"),
                    _opt_str(segment, "successorRunId"),
                )
            )
        outcome = _opt_str(raw, "finalOutcome")
        if outcome is not None and outcome not in TERMINAL_STATUSES:
            raise SagatError("PROTOCOL", "finalOutcome is invalid in the answer")
        return ChainInspection(
            _str(raw, "chainId"),
            _str(raw, "currentRunId"),
            _revision(raw, "revision", 0),
            raw.get("cancelRequested") is True,
            outcome,
            tuple(segments),
            raw,
        )

    async def signal_chain(
        self,
        namespace_id: str,
        chain_id: str,
        name: str,
        payload: Any,
        *,
        expected_run_id: str,
        idempotency_key: str | None = None,
        traceparent: str | None = None,
    ) -> SignalResult:
        """Signals a root wait on the expected current chain head."""
        path = (
            f"/namespaces/{quote(_namespace_id(namespace_id), safe='')}/chains/{quote(_run_id(chain_id), safe='')}"
            f"/signals/{quote(_name(name, 'a signal name'), safe='')}"
        )
        response = await self._http.request(
            HttpRequest(
                "POST",
                path,
                body={"expectedRunId": _run_id(expected_run_id), "payload": payload},
                idempotency_key=self._key(idempotency_key),
                traceparent=self._trace(traceparent),
            )
        )
        raw = _object(response.body, "a chain signal result")
        delivered = raw.get("deliveredTo")
        return SignalResult(tuple(str(item) for item in delivered) if isinstance(delivered, list) else ())

    async def cancel_chain(
        self,
        namespace_id: str,
        chain_id: str,
        reason: str,
        *,
        expected_run_id: str,
        idempotency_key: str | None = None,
        traceparent: str | None = None,
    ) -> None:
        """Latches cancellation and cancels the expected current chain head."""
        if not isinstance(reason, str) or not 1 <= len(reason) <= 512:
            raise invalid("a chain cancellation reason is 1 to 512 characters")
        path = (
            f"/namespaces/{quote(_namespace_id(namespace_id), safe='')}/chains/"
            f"{quote(_run_id(chain_id), safe='')}/cancel"
        )
        await self._http.request(
            HttpRequest(
                "POST",
                path,
                body={"expectedRunId": _run_id(expected_run_id), "reason": reason},
                idempotency_key=self._key(idempotency_key),
                traceparent=self._trace(traceparent),
            )
        )

    async def decide_approval(
        self,
        approval_id: str,
        decision: ApprovalDecision,
        *,
        decision_key: str,
        reason_code: str | None = None,
        traceparent: str | None = None,
    ) -> ApprovalDecisionReceipt:
        """Records an eligible actor's durable approval decision."""
        if not isinstance(approval_id, str) or not _UUID.fullmatch(approval_id):
            raise invalid("an approval id is a UUID")
        if decision not in ("APPROVE", "REJECT"):
            raise invalid("an approval decision is APPROVE or REJECT")
        if not isinstance(decision_key, str) or not _APPROVAL_DECISION_KEY.fullmatch(decision_key):
            raise invalid("an approval decision key is 1 to 128 printable ASCII characters")
        if reason_code is not None and (
            not isinstance(reason_code, str) or not _APPROVAL_REASON_CODE.fullmatch(reason_code)
        ):
            raise invalid("an approval reason code is 1 to 64 letters, digits, dots, underscores, colons or hyphens")
        body: dict[str, Any] = {"decisionKey": decision_key, "decision": decision}
        if reason_code is not None:
            body["reasonCode"] = reason_code
        response = await self._http.request(
            HttpRequest(
                "POST",
                f"/approvals/{quote(approval_id.lower(), safe='')}/decisions",
                body=body,
                traceparent=self._trace(traceparent),
            )
        )
        raw = _object(response.body, "an approval decision receipt")
        decided_at = parse_instant(raw.get("decidedAt"))
        if decided_at is None:
            raise SagatError("PROTOCOL", "decidedAt is missing from the answer")
        received_decision = _str(raw, "decision")
        if received_decision not in ("APPROVE", "REJECT"):
            raise SagatError("PROTOCOL", "decision is invalid in the answer")
        outcome_sequence = raw.get("outcomeSequence")
        if not isinstance(outcome_sequence, int) or isinstance(outcome_sequence, bool) or outcome_sequence < 1:
            raise SagatError("PROTOCOL", "outcomeSequence is invalid in the answer")
        return ApprovalDecisionReceipt(
            receipt_id=_str(raw, "receiptId"),
            approval_id=_str(raw, "approvalId"),
            decision_key=_str(raw, "decisionKey"),
            decision=cast(ApprovalDecision, received_decision),
            reason_code=_opt_str(raw, "reasonCode"),
            actor=_str(raw, "actor"),
            decided_at=decided_at,
            outcome_sequence=outcome_sequence,
            replayed=raw.get("replayed") is True,
            raw=raw,
        )

    async def list_approvals(
        self,
        run_id: str,
        *,
        cursor: str | None = None,
        limit: int | None = None,
        traceparent: str | None = None,
    ) -> ApprovalPage:
        """One bounded page of approval requests in immutable opening order."""
        response = await self._http.request(
            HttpRequest(
                "GET",
                f"/runs/{quote(_run_id(run_id), safe='')}/approvals",
                query={"cursor": cursor, "limit": limit},
                traceparent=self._trace(traceparent),
            )
        )
        raw = _object(response.body, "an approval page")
        values = raw.get("items")
        if not isinstance(values, list) or len(values) > 200:
            raise SagatError("PROTOCOL", "items is invalid in the answer")
        items: list[ApprovalListItem] = []
        for value in values:
            item = _object(value, "an approval")
            status = _str(item, "status")
            if status not in ("PENDING", "APPROVED", "REJECTED", "EXPIRED", "CANCELLED"):
                raise SagatError("PROTOCOL", "status is invalid in the answer")
            deadline = parse_instant(item.get("deadline"))
            if deadline is None:
                raise SagatError("PROTOCOL", "deadline is missing from the answer")
            settled_at = parse_instant(item.get("settledAt"))
            path = item.get("instancePath")
            if not isinstance(path, list) or len(path) > 8:
                raise SagatError("PROTOCOL", "instancePath is invalid in the answer")
            scopes: list[ApprovalScope] = []
            for value in path:
                scope = _object(value, "an approval scope")
                index = _revision(scope, "iterationIndex", 0)
                if index > 9_999:
                    raise SagatError("PROTOCOL", "iterationIndex is invalid in the answer")
                scopes.append(ApprovalScope(_str(scope, "scopeNodeId"), index))
            items.append(
                ApprovalListItem(
                    approval_id=_str(item, "approvalId"),
                    run_id=_str(item, "runId"),
                    node_id=_str(item, "nodeId"),
                    instance_path=tuple(scopes),
                    status=cast(ApprovalStatus, status),
                    deadline=deadline,
                    settled_at=settled_at,
                    raw=item,
                )
            )
        return ApprovalPage(tuple(items), _opt_str(raw, "nextCursor"))

    async def list_all_approvals(
        self, run_id: str, *, limit: int | None = None, traceparent: str | None = None
    ) -> AsyncIterator[ApprovalListItem]:
        """Every approval request for a run, page after page, in immutable opening order."""
        cursor: str | None = None
        while True:
            page = await self.list_approvals(run_id, cursor=cursor, limit=limit, traceparent=traceparent)
            for item in page.items:
                yield item
            cursor = page.next_cursor
            if not cursor:
                return

    async def query(
        self,
        run_id: str,
        name: str,
        args: Any = None,
        *,
        traceparent: str | None = None,
    ) -> QueryResult:
        """Evaluates a declared side-effect-free query over one committed state snapshot."""
        path = f"/runs/{quote(_run_id(run_id), safe='')}/queries/{quote(_declared_operation_name(name), safe='')}"
        response = await self._http.request(
            HttpRequest("POST", path, body={"args": args}, traceparent=self._trace(traceparent))
        )
        raw = _object(response.body, "a query result")
        if "result" not in raw:
            raise SagatError("PROTOCOL", "result is missing from the answer")
        return QueryResult(
            run_id=_str(raw, "runId"),
            run_version=_revision(raw, "runVersion", 0),
            state_revision=_revision(raw, "stateRevision", 0),
            result=raw["result"],
            raw=raw,
        )

    async def query_chain(
        self,
        namespace_id: str,
        chain_id: str,
        name: str,
        args: Any = None,
        *,
        expected_run_id: str | None = None,
        traceparent: str | None = None,
    ) -> QueryResult:
        """Evaluates a declared query against the chain head selected under the server's chain lock."""
        path = (
            f"/namespaces/{quote(_namespace_id(namespace_id), safe='')}/chains/{quote(_run_id(chain_id), safe='')}"
            f"/queries/{quote(_declared_operation_name(name), safe='')}"
        )
        body: dict[str, Any] = {"args": args}
        if expected_run_id is not None:
            body["expectedRunId"] = _run_id(expected_run_id)
        response = await self._http.request(HttpRequest("POST", path, body=body, traceparent=self._trace(traceparent)))
        raw = _object(response.body, "a query result")
        if "result" not in raw:
            raise SagatError("PROTOCOL", "result is missing from the answer")
        return QueryResult(
            run_id=_str(raw, "runId"),
            run_version=_revision(raw, "runVersion", 0),
            state_revision=_revision(raw, "stateRevision", 0),
            result=raw["result"],
            raw=raw,
        )

    async def update(
        self,
        run_id: str,
        name: str,
        args: Any,
        *,
        update_key: str,
        expected_state_revision: int | None = None,
        traceparent: str | None = None,
    ) -> UpdateReceipt:
        """Applies or replays a declared durable state update."""
        path = f"/runs/{quote(_run_id(run_id), safe='')}/updates/{quote(_declared_operation_name(name), safe='')}"
        if not isinstance(update_key, str) or not _UPDATE_KEY.fullmatch(update_key):
            raise invalid("an update key is 1 to 128 printable ASCII characters")
        if expected_state_revision is not None and (
            not isinstance(expected_state_revision, int)
            or isinstance(expected_state_revision, bool)
            or not 0 <= expected_state_revision <= _MAX_I64
        ):
            raise invalid("expected state revision is non-negative")
        body: dict[str, Any] = {"updateKey": update_key, "args": args}
        if expected_state_revision is not None:
            body["expectedStateRevision"] = expected_state_revision
        response = await self._http.request(HttpRequest("POST", path, body=body, traceparent=self._trace(traceparent)))
        raw = _object(response.body, "an update receipt")
        if raw.get("status") != "APPLIED":
            raise SagatError("PROTOCOL", "status is invalid in the answer")
        if "result" not in raw:
            raise SagatError("PROTOCOL", "result is missing from the answer")
        return UpdateReceipt(
            run_id=_str(raw, "runId"),
            operation=_str(raw, "operation"),
            update_key=_str(raw, "updateKey"),
            status="APPLIED",
            result=raw["result"],
            before_state_revision=_revision(raw, "beforeStateRevision", 0),
            after_state_revision=_revision(raw, "afterStateRevision", 1),
            run_version=_revision(raw, "runVersion", 1),
            replayed=raw.get("replayed") is True,
            raw=raw,
        )

    async def publish_event(
        self,
        event_type: str,
        data: Mapping[str, Any] | None = None,
        *,
        event_id: str | None = None,
        subject: str | None = None,
        occurred_at: datetime | None = None,
        correlation_key: str | None = None,
        dedupe_ttl_seconds: int | None = None,
        traceparent: str | None = None,
    ) -> EventAccepted:
        """Publishes an event to the inbox of the namespace, once per event id for the deduplication window.

        Without an ``event_id`` one is made per call and kept across its retries, so a repetition is answered
        ``DUPLICATE`` and never admitted twice.
        """
        body: dict[str, Any] = {
            "eventId": event_id if event_id is not None else str(uuid.uuid4()),
            "type": _name(event_type, "an event type"),
            "data": data if data is not None else {},
        }
        if subject is not None:
            body["subject"] = subject
        if occurred_at is not None:
            body["occurredAt"] = format_instant(occurred_at)
        if correlation_key is not None:
            body["correlationKey"] = correlation_key
        if dedupe_ttl_seconds is not None:
            body["dedupeTtlSeconds"] = dedupe_ttl_seconds
        response = await self._http.request(
            HttpRequest("POST", "/events", body=body, traceparent=self._trace(traceparent))
        )
        raw = _object(response.body, "an event answer")
        return EventAccepted(_str(raw, "eventId"), _str(raw, "source"), _str(raw, "status"))

    async def events(self, run_id: str, *, cursor: str | None = None, limit: int | None = None) -> JournalPage:
        """The journal of a run, a page at a time."""
        path = f"/runs/{quote(_run_id(run_id), safe='')}/events"
        response = await self._http.request(
            HttpRequest(
                "GET",
                path,
                query={"cursor": cursor, "limit": limit},
                traceparent=self._trace(),
            )
        )
        raw = _object(response.body, "a page")
        items = raw.get("items")
        return JournalPage(
            tuple(JournalEvent(_object(item, "an event")) for item in items) if isinstance(items, list) else (),
            _opt_str(raw, "nextCursor"),
        )

    async def wait_for_completion(self, run_id: str, *, timeout: float = 60.0, poll_interval: float = 0.5) -> Run:
        """Polls ``get`` every ``poll_interval`` seconds (+-20 % jitter) until the run is terminal or ``timeout``.

        Cancel the awaiting task to stop waiting.
        """
        end = time.monotonic() + timeout
        while True:
            run = await self.get(run_id)
            if run.is_terminal:
                return run
            if time.monotonic() >= end:
                raise SagatError("TRANSPORT", f"the run did not finish within {timeout:g} s", reason="WAIT_TIMEOUT")
            await asyncio.sleep(poll_interval * (0.8 + random.random() * 0.4))
