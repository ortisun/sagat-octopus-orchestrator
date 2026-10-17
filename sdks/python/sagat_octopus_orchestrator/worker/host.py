"""The worker host (``sdks/SPEC.md`` section 6): registration, the poll loop, heartbeats, cancellation and reports."""

from __future__ import annotations

import asyncio
import contextlib
import contextvars
import functools
import hashlib
import inspect
import logging
import random
import re
import time
from collections.abc import AsyncIterable, AsyncIterator, Awaitable, Callable, Iterable, Sequence
from concurrent.futures import ThreadPoolExecutor
from dataclasses import dataclass, field
from typing import TYPE_CHECKING, Any, Literal, TypeVar

import grpc.aio

from .._json import decode_json, encode_json, utf16_key
from .._proto import worker_pb2 as pb
from ..config import (
    LOG,
    PROTOCOL_VERSION,
    ProviderFailure,
    Secret,
    TlsOptions,
    TokenSource,
    is_provider,
    log,
    warn_if_plaintext,
)
from ..errors import SagatError
from ..retry import DEFAULT_RETRY, MAX_SERVER_WAIT, RetryPolicy, backoff_delay, server_wait
from ._grpc import GrpcConfig, WorkerRpc
from .activity import (
    ActivityContext,
    ActivityFailure,
    ActivityHandler,
    BlobDescriptor,
    BlobOutput,
    CancellationToken,
    CancelReason,
    ExecutionInstanceIdentity,
    ExecutionScope,
    Secrets,
    TaskKind,
    Verdict,
    is_cancellation,
)

T = TypeVar("T")
Kind = Literal["activity", "compensation", "reconciler"]
WorkerTransport = Literal["auto", "poll", "stream"]

MAX_OUTPUT_BYTES = 256 * 1024
MAX_POLL = 64
MAX_RECEIVE_INPUT_BYTES = 8 * 1024 * 1024
REPORT_ATTEMPTS = 5
_CODE = re.compile(r"[A-Z0-9_]{1,64}")
_SURROGATES = re.compile(r"[\ud800-\udfff]")
SHUTDOWN_GRACE = 2.0
"""Seconds. After ``stop`` has cancelled the tokens of the tasks left, it waits this long for them to react and be
reported before it closes the connection; it never interrupts a handler."""
_VERSION = re.compile(r"1(?:\.([0-9]+))?")
_REPEATABLE = frozenset({"TRANSPORT", "UNAVAILABLE", "RATE_LIMITED"})
_TASK_KINDS: dict[str, TaskKind] = {"COMPENSATION": "COMPENSATION", "RECONCILIATION": "RECONCILIATION"}
_BLOB_SHA256 = re.compile(r"[0-9a-f]{64}")
MAX_BLOB_CHUNK = 256 * 1024


@dataclass(frozen=True)
class WorkerEvent:
    """Something that happened in the host: the hook for metrics (counters of tasks taken, completed, failed,
    stale, heartbeats, polls, reconnects; a histogram of ``duration_ms``)."""

    type: Literal[
        "registered",
        "poll",
        "task.taken",
        "task.completed",
        "task.failed",
        "task.stale",
        "task.dropped",
        "heartbeat",
        "receive.failed",
        "reconnect",
    ]
    task_id: str | None = None
    activity: str | None = None
    attempt: int | None = None
    code: str | None = None
    retryable: bool | None = None
    duration_ms: int | None = None
    reason: str | None = None
    protocol_version: str | None = None
    tasks: int | None = None
    traceparent: str | None = None


@dataclass(frozen=True)
class WorkerConfig:
    """Configuration of ``WorkerHost``. Durations are in seconds."""

    grpc_url: str
    """``http://host:port`` or ``https://host:port`` of the worker service."""
    namespace: str
    token: TokenSource | Secret = field(repr=False)
    """A bearer token or a provider; held as a ``Secret``."""
    worker_name: str
    """One live session per name and identity: registering again under the same name replaces it."""
    max_inflight: int = 10
    """Tasks in flight at once, 1 to 1000."""
    languages: Sequence[str] = ("python",)
    pools: Sequence[str] = ()
    tls: TlsOptions | None = None
    timeout: float = 30.0
    """Deadline of one unary RPC."""
    retry: RetryPolicy = field(default_factory=lambda: DEFAULT_RETRY)
    protocol_version: str | None = None
    """Overrides the newest protocol minor the host offers (tests play an older or a newer SDK with it)."""
    transport: WorkerTransport = "auto"
    """Task receive transport. ``auto`` streams at protocol 1.4+ and polls older servers."""
    heartbeat_interval: float | None = None
    """Overrides the heartbeat interval (otherwise a third of the lease, 0.5 to 10 s)."""
    logger: logging.Logger = LOG
    interceptors: Sequence[grpc.aio.ClientInterceptor] = ()
    """Interceptors of the ``grpc.aio`` channel: the hook of the conformance suite and of tests."""
    on_event: Callable[[WorkerEvent], None] | None = None

    def __post_init__(self) -> None:
        if not isinstance(self.token, Secret):
            object.__setattr__(self, "token", Secret(self.token))


@dataclass
class _Registered:
    kind: Kind
    handler: ActivityHandler
    parse: Callable[[Any], Any] | None
    blocking: bool


@dataclass(eq=False)
class _Task:
    """One lease being worked on."""

    lease: pb.TaskLease = field(repr=False)
    protocol_minor: int = 0
    token: CancellationToken = field(default_factory=CancellationToken)
    deadline_ms: int = 0
    lost: bool = False
    runner: asyncio.Task[None] | None = None
    beat: asyncio.Task[None] | None = None
    lease_token: Secret = field(default_factory=lambda: Secret(""), repr=False)
    rpc: WorkerRpc | None = field(default=None, repr=False)
    """The connection the task was leased on: its reports go there, or nowhere if the host has stopped since."""
    secrets: Secrets | None = None
    finished: bool = False
    """True once the handler has returned: its secrets are gone and ``ctx.secrets()`` raises."""
    secrets_lock: asyncio.Lock = field(default_factory=asyncio.Lock)


if TYPE_CHECKING:
    _AdapterBase = logging.LoggerAdapter[logging.Logger]
else:
    _AdapterBase = logging.LoggerAdapter


class _TaskLog(_AdapterBase):
    """The logger of an activity: every line carries the task's identifiers, and nothing else of it."""

    def process(self, msg: Any, kwargs: Any) -> tuple[Any, Any]:
        fields = dict(self.extra or {})
        kwargs["extra"] = {"sagat": fields}
        return f"{msg} " + " ".join(f"{key}={value}" for key, value in fields.items()), kwargs


def _minor_of(version: str) -> int:
    match = _VERSION.fullmatch(version)
    return int(match[1] or 0) if match else 0


def _scoped_instance(lease: pb.TaskLease, minor: int) -> dict[str, pb.ExecutionInstanceIdentity]:
    if minor >= 5 and lease.HasField("instance"):
        return {"instance": lease.instance}
    return {}


def _activity_instance(lease: pb.TaskLease, minor: int) -> ExecutionInstanceIdentity | None:
    if minor < 5 or not lease.HasField("instance"):
        return None
    return ExecutionInstanceIdentity(
        instance_path=tuple(
            ExecutionScope(scope_node_id=segment.scope_node_id, iteration_index=segment.iteration_index)
            for segment in lease.instance.instance_path
        ),
        definition_node_id=lease.instance.definition_node_id,
    )


def _is_async_callable(handler: Callable[..., Any]) -> bool:
    return inspect.iscoroutinefunction(handler) or inspect.iscoroutinefunction(type(handler).__call__)


class WorkerHost:
    """The worker host of the SDK: registers activities, takes leases, heartbeats, reports.

    ``async`` handlers run as tasks of the event loop; a plain function is run in a thread. Create the host
    and use it inside the event loop that runs ``start`` and ``stop``.
    """

    def __init__(self, config: WorkerConfig) -> None:
        if not isinstance(config.max_inflight, int) or not 1 <= config.max_inflight <= 1000:
            raise SagatError("VALIDATION_FAILED", "max_inflight is between 1 and 1000")
        if config.transport not in ("auto", "poll", "stream"):
            raise SagatError("VALIDATION_FAILED", "transport is auto, poll or stream")
        self._config = config
        self._log = config.logger
        self._handlers: dict[str, _Registered] = {}
        self._rpc: WorkerRpc | None = None
        self._executor: ThreadPoolExecutor | None = None
        self._worker_id = ""
        self._session = Secret("")
        self._accepted = ""
        self._stream_unavailable = False
        self._running = False
        self._stopping = False
        self._loop_task: asyncio.Task[None] | None = None
        self._wake: asyncio.Event | None = None
        self._inflight: set[_Task] = set()
        self._orphans: set[_Task] = set()
        self._lifecycle = asyncio.Lock()

    @property
    def protocol_version(self) -> str:
        """The version the server agreed to; empty before ``start``."""
        return self._accepted

    # ------------------------------------------------------------------------------------ registration of handlers

    def _add(
        self, kind: Kind, name: str, handler: ActivityHandler, parse: Callable[[Any], Any] | None, blocking: bool | None
    ) -> WorkerHost:
        if self._running:
            raise SagatError("VALIDATION_FAILED", "activities are registered before the host starts")
        if not name:
            raise SagatError("VALIDATION_FAILED", "an activity needs a name")
        if name in self._handlers:
            raise SagatError("VALIDATION_FAILED", f"an activity named {name} is registered already")
        self._handlers[name] = _Registered(
            kind, handler, parse, (not _is_async_callable(handler)) if blocking is None else blocking
        )
        return self

    def register_activity(
        self,
        name: str,
        handler: ActivityHandler,
        *,
        parse: Callable[[Any], Any] | None = None,
        blocking: bool | None = None,
    ) -> WorkerHost:
        """Registers the handler of an activity: ``async def handler(input, ctx)`` or a plain function.

        ``parse`` turns the run's input into the handler's type (raise to refuse: reported as ``INPUT_INVALID``
        and not retried). A plain function runs in a thread; pass ``blocking=False`` to call a quick one in
        the event loop, or ``blocking=True`` for an ``async`` one that you know blocks.
        """
        return self._add("activity", name, handler, parse, blocking)

    def register_compensation(
        self,
        name: str,
        handler: ActivityHandler,
        *,
        parse: Callable[[Any], Any] | None = None,
        blocking: bool | None = None,
    ) -> WorkerHost:
        """Registers the handler of the activity a node names in ``compensate_with``. It must be idempotent."""
        return self._add("compensation", name, handler, parse, blocking)

    def register_reconciler(
        self,
        name: str,
        handler: ActivityHandler,
        *,
        parse: Callable[[Any], Any] | None = None,
        blocking: bool | None = None,
    ) -> WorkerHost:
        """Registers the handler of the activity a node names in ``reconcile_with``; it answers a ``Verdict``."""
        return self._add("reconciler", name, handler, parse, blocking)

    # ---------------------------------------------------------------------------------------------------- lifecycle

    def _emit(self, event: WorkerEvent) -> None:
        if self._config.on_event is not None:
            try:
                self._config.on_event(event)
            except Exception:  # a metrics hook never breaks the host
                log(self._log, logging.ERROR, "the event hook raised", event=event.type)

    async def start(self) -> None:
        """Opens the connection, registers with the server and starts the loop; returns once registered.

        Calls to ``start`` and ``stop`` are serialized: two concurrent ``start`` calls register once."""
        async with self._lifecycle:
            await self._start()

    async def _start(self) -> None:
        if self._running:
            return
        if not self._handlers:
            raise SagatError("VALIDATION_FAILED", "register at least one activity before start")
        if len(self._handlers) > 256:
            raise SagatError("VALIDATION_FAILED", "a worker registers 1 to 256 activities")
        warn_if_plaintext(self._log, self._config.grpc_url, "grpc_url")
        self._rpc = WorkerRpc(
            GrpcConfig(
                grpc_url=self._config.grpc_url,
                token=self._config.token,
                tls=self._config.tls,
                timeout=self._config.timeout,
                interceptors=self._config.interceptors,
            )
        )
        try:
            await self._register()
            if self._config.transport == "stream" and _minor_of(self._accepted) < 4:
                raise SagatError("UNSUPPORTED_PROTOCOL", "task streaming requires worker protocol 1.4")
        except BaseException:
            rpc, self._rpc = self._rpc, None
            await rpc.close()
            raise
        if any(item.blocking for item in self._handlers.values()):
            self._executor = ThreadPoolExecutor(
                max_workers=self._config.max_inflight, thread_name_prefix="sagat-activity"
            )
        self._running = True
        self._stopping = False
        self._wake = asyncio.Event()
        self._loop_task = asyncio.create_task(self._run(), name=f"sagat-worker-{self._config.worker_name}")

    async def stop(self, timeout: float = 30.0) -> None:
        """Stops polling, waits up to ``timeout`` seconds for the tasks in flight, then cancels the tokens of the
        ones left (reason ``shutdown``) and returns. It never interrupts a handler: one that ignores its token runs
        to its end, and is reported only if the connection is still open (it is closed a moment after the tokens are
        cancelled). A task that did not finish keeps its lease until it lapses: the server leases it again. The host
        may be started again."""
        async with self._lifecycle:
            await self._stop(timeout)

    async def _stop(self, timeout: float) -> None:
        if not self._running:
            return
        self._stopping = True
        assert self._wake is not None
        assert self._loop_task is not None
        self._wake.set()
        clock = asyncio.get_running_loop().time
        grace_ends = clock() + timeout
        # The receive owns no handler: cancel it promptly while delivered handlers keep their lifecycle.
        self._loop_task.cancel()
        await asyncio.gather(self._loop_task, return_exceptions=True)
        pending = {task.runner for task in self._inflight if task.runner is not None}
        if pending:
            _, late = await asyncio.wait(pending, timeout=max(0.0, grace_ends - clock()))
            if late:
                for task in list(self._inflight):
                    self._cancel_activity(task, "shutdown")
                await asyncio.wait(late, timeout=SHUTDOWN_GRACE)  # they may react to the token and be reported
        # Whatever is left keeps its lease until it lapses; its heartbeats end with the host.
        for task in list(self._inflight):
            if task.beat is not None:
                task.beat.cancel()
        self._orphans |= self._inflight
        self._inflight = set()
        self._running = False
        rpc, self._rpc = self._rpc, None
        if rpc is not None:
            await rpc.close()
        executor, self._executor = self._executor, None
        if executor is not None:
            executor.shutdown(wait=False, cancel_futures=True)

    async def __aenter__(self) -> WorkerHost:
        await self.start()
        return self

    async def __aexit__(self, *exc: object) -> None:
        await self.stop()

    # ---------------------------------------------------------------------------------------------- registration

    async def _register(self) -> None:
        assert self._rpc is not None
        request = pb.RegisterWorkerRequest(
            worker_name=self._config.worker_name,
            namespace=self._config.namespace,
            capabilities=pb.WorkerCapabilities(
                languages=list(self._config.languages),
                activities=sorted(self._handlers, key=utf16_key),
                pools=list(self._config.pools),
                max_inflight=self._config.max_inflight,
                protocol_version=self._config.protocol_version or PROTOCOL_VERSION,
            ),
        )
        refreshed = False
        n = 0
        while True:
            try:
                response = await self._rpc.register_worker(request)
            except SagatError as error:
                # A token the provider can refresh gets one more chance (SPEC section 5).
                provider_failed = isinstance(error, ProviderFailure)  # nothing was sent: repeating cannot do harm
                if (
                    error.code == "UNAUTHENTICATED"
                    and not provider_failed
                    and not refreshed
                    and is_provider(self._config.token)
                ):
                    refreshed = True
                    continue
                if not provider_failed and (error.code not in _REPEATABLE or not error.retryable):
                    raise
                if n + 1 >= self._config.retry.max_attempts:
                    raise
                await asyncio.sleep(max(backoff_delay(self._config.retry, n), server_wait(error.retry_after_ms)))
                n += 1
                continue
            self._worker_id = response.worker_id
            self._session = Secret(response.session_token)
            self._accepted = response.accepted_protocol_version
            self._stream_unavailable = False
            log(self._log, logging.INFO, "worker registered", worker=self._config.worker_name, protocol=self._accepted)
            self._emit(WorkerEvent("registered", protocol_version=self._accepted))
            return

    # ---------------------------------------------------------------------------------------------------- loop

    async def _idle(self, seconds: float) -> None:
        """The pause between polls: until the time passes, a task finishes or the host stops. A host that is
        stopping does not pause (``stop`` may have run while a poll was in flight, with nothing to wake)."""
        assert self._wake is not None
        if self._stopping:
            return
        with contextlib.suppress(asyncio.TimeoutError):
            await asyncio.wait_for(self._wake.wait(), timeout=seconds)
        self._wake.clear()

    async def _run(self) -> None:
        assert self._rpc is not None
        failures = 0
        refusals = 0  # polls refused as UNAUTHENTICATED in a row
        max_inflight = self._config.max_inflight
        while not self._stopping:
            streaming = False
            meaningful = False
            try:
                free = max_inflight - len(self._inflight)
                if free <= 0:
                    await self._idle(0.05)
                    continue
                asked = min(free, MAX_POLL)
                streaming = self._config.transport == "stream" or (
                    self._config.transport == "auto" and _minor_of(self._accepted) >= 4 and not self._stream_unavailable
                )
                if streaming and self._config.transport == "stream" and _minor_of(self._accepted) < 4:
                    raise SagatError("UNSUPPORTED_PROTOCOL", "task streaming requires worker protocol 1.4")
                if streaming:
                    request = pb.PollTasksRequest(
                        worker_id=self._worker_id, session_token=self._session.reveal(), max_tasks=asked
                    )
                    received = 0
                    input_bytes = 0
                    wait_ms: int | None = None
                    malformed = False
                    task_ids: set[str] = set()
                    stream = self._rpc.stream_tasks(request)
                    try:
                        async for response in stream:
                            if self._stopping:
                                return
                            leases = list(response.tasks)
                            if not leases:
                                wait_ms = response.retry_after_ms if response.retry_after_ms > 0 else 1000
                                malformed = response.retry_after_ms <= 0
                                break
                            if response.retry_after_ms != 0:
                                malformed = True
                            accepted = 0
                            for lease in leases:
                                size = len(lease.input_json.encode("utf-8"))
                                if (
                                    received >= asked
                                    or len(self._inflight) >= max_inflight
                                    or input_bytes + size > MAX_RECEIVE_INPUT_BYTES
                                    or lease.task_id in task_ids
                                ):
                                    malformed = True
                                    break
                                task_ids.add(lease.task_id)
                                received += 1
                                accepted += 1
                                input_bytes += size
                                meaningful = True
                                self._dispatch(lease)
                            self._emit(WorkerEvent("poll", tasks=accepted))
                            if malformed:
                                break
                    finally:
                        await stream.aclose()
                    if meaningful:
                        failures = 0
                    refusals = 0
                    if malformed:
                        log(self._log, logging.WARNING, "the task stream exceeded its bounded receive contract")
                        await self._idle(min(MAX_SERVER_WAIT, max(0.05, (wait_ms or 1000) / 1000)))
                    elif received < asked:
                        # An underfilled EOF without the terminal hint is interrupted, never a busy reopen.
                        pause_ms = max(50, wait_ms or 1000)
                        await self._idle(min(MAX_SERVER_WAIT, pause_ms / 1000 * (0.8 + random.random() * 0.4)))
                    continue
                response = await self._rpc.poll_tasks(
                    pb.PollTasksRequest(
                        worker_id=self._worker_id, session_token=self._session.reveal(), max_tasks=asked
                    )
                )
                if len(response.tasks) > 0:
                    failures = 0
                refusals = 0
                leases = list(response.tasks)
                if len(leases) > asked:
                    # The host runs as many tasks as it asked for; the rest are left to lapse (SPEC section 6.2).
                    log(
                        self._log,
                        logging.WARNING,
                        "the poll answered more tasks than were asked for: the extra leases are left to lapse",
                        asked=asked,
                        got=len(leases),
                        left_to_lapse=len(leases) - asked,
                    )
                    leases = leases[:asked]
                self._emit(WorkerEvent("poll", tasks=len(leases)))
                for lease in leases:
                    self._dispatch(lease)
                if not leases:
                    wait_ms = max(50, response.retry_after_ms if response.retry_after_ms > 0 else 1000)
                    await self._idle(min(MAX_SERVER_WAIT, wait_ms / 1000 * (0.8 + random.random() * 0.4)))
            except SagatError as error:
                if self._stopping:
                    return
                if meaningful:
                    failures = 0
                self._emit(WorkerEvent("receive.failed", code=error.code))
                if streaming and self._config.transport == "auto" and error.status == "UNIMPLEMENTED":
                    self._stream_unavailable = True
                    await self._idle(max(0.1, backoff_delay(self._config.retry, failures)))
                    failures += 1
                    continue
                if isinstance(error, ProviderFailure):
                    # The token provider failed before the poll was sent: the session is not in question.
                    log(self._log, logging.WARNING, "the token provider failed: polling again after a pause")
                    policy = RetryPolicy(self._config.retry.max_attempts, self._config.retry.initial_delay, 30.0)
                    failures += 1
                    await self._idle(min(30.0, backoff_delay(policy, failures - 1)))
                    continue
                if error.code == "UNAUTHENTICATED":
                    log(self._log, logging.WARNING, "the session is gone: registering again")
                    self._emit(WorkerEvent("reconnect"))
                    if refusals:
                        # The poll after the last registration was refused again: a server that registers a worker
                        # and refuses it at once is not hammered (SPEC section 6.2).
                        await self._idle(backoff_delay(self._config.retry, refusals - 1))
                        if self._stopping:
                            return
                    refusals += 1
                    try:
                        await self._register()
                        continue
                    except SagatError as again:
                        log(self._log, logging.ERROR, "registering again failed", code=again.code)
                if error.code == "RATE_LIMITED":
                    await self._idle(max(server_wait(error.retry_after_ms), 0.1))
                    continue
                self._emit(WorkerEvent("reconnect"))
                policy = RetryPolicy(self._config.retry.max_attempts, self._config.retry.initial_delay, 30.0)
                pause = min(30.0, max(backoff_delay(policy, failures), server_wait(error.retry_after_ms)))
                failures += 1
                await self._idle(pause)
            except Exception as error:  # a bug of ours must not stop the host
                if self._stopping:
                    return
                log(self._log, logging.ERROR, "the receive loop failed", error=type(error).__name__)
                await self._idle(1.0)

    # ---------------------------------------------------------------------------------------------------- tasks

    def _dispatch(self, lease: pb.TaskLease) -> None:
        task = _Task(
            lease,
            protocol_minor=_minor_of(self._accepted),
            deadline_ms=lease.expires_at_epoch_ms,
            rpc=self._rpc,
            lease_token=Secret(lease.lease_token),
        )
        lease.lease_token = ""  # the task keeps it behind a Secret: a dump of the task or of its lease shows nothing
        self._inflight.add(task)
        runner = asyncio.create_task(self._execute(task), name=f"sagat-task-{lease.task_id}")
        task.runner = runner
        runner.add_done_callback(functools.partial(self._finished, task))
        self._emit(
            WorkerEvent(
                "task.taken",
                task_id=lease.task_id,
                activity=lease.activity_name,
                attempt=lease.attempt,
                traceparent=lease.traceparent,
            )
        )

    def _finished(self, task: _Task, done: asyncio.Future[None]) -> None:
        self._inflight.discard(task)
        self._orphans.discard(task)
        if not done.cancelled() and done.exception() is not None:  # a bug of ours: say so, once, without the text
            log(self._log, logging.ERROR, "a task ended with an unexpected error", task=task.lease.task_id,
                error=type(done.exception()).__name__)  # fmt: skip
        if self._wake is not None:
            self._wake.set()

    def _cancel_activity(self, task: _Task, reason: CancelReason) -> None:
        """Cancels the token of the activity and does nothing else: a handler is never interrupted (SPEC 6.4)."""
        task.token.cancel(reason)

    async def _execute(self, task: _Task) -> None:
        lease = task.lease
        started = time.monotonic()
        registered = self._handlers.get(lease.activity_name)
        minor = task.protocol_minor
        # task_kind and the target fields came with 1.2: against an older server every task is a forward task and
        # they are not read at all (SPEC 6.1).
        kind: TaskKind = _TASK_KINDS.get(lease.task_kind, "FORWARD") if minor >= 2 else "FORWARD"
        target_node = lease.target_node if minor >= 2 else ""
        target_key = lease.target_idempotency_key if minor >= 2 else ""

        if registered is None:
            await self._fail(task, "NO_HANDLER", "no handler is registered for the activity", False, started)
            return

        document: dict[str, Any] = {}
        try:
            parsed = decode_json(lease.input_json) if lease.input_json else {}
            if isinstance(parsed, dict):
                document = parsed
        except ValueError:
            await self._fail(task, "INPUT_INVALID", "the step document is not JSON", False, started)
            return
        run_input: Any = document.get("input")
        try:
            if registered.parse is not None:
                run_input = registered.parse(run_input)
        except Exception:
            await self._fail(task, "INPUT_INVALID", "the input does not have the type of the handler", False, started)
            return
        steps: dict[str, Any] = {}
        raw_steps = document.get("steps")
        for node, value in raw_steps.items() if isinstance(raw_steps, dict) else ():
            if isinstance(value, dict) and "output" in value:
                steps[node] = value["output"]

        collection_item: Any = None
        collection_index: int | None = None
        scoped = minor >= 5 and lease.HasField("instance") and len(lease.instance.instance_path) > 0
        has_item = "item" in document
        has_index = "index" in document
        if scoped and (has_item or has_index):
            raw_index = document.get("index")
            if (
                not has_item
                or not has_index
                or isinstance(raw_index, bool)
                or not isinstance(raw_index, int)
                or raw_index < 0
                or raw_index > 0xFFFFFFFF
            ):
                await self._fail(task, "INPUT_INVALID", "the collection item context is malformed", False, started)
                return
            collection_item = document["item"]
            collection_index = raw_index

        node_id = document.get("node")
        context = ActivityContext(
            activity_name=lease.activity_name,
            task_id=lease.task_id,
            run_id=lease.run_id,
            node=node_id if isinstance(node_id, str) and node_id else lease.step_id,
            step_id=lease.step_id,
            attempt=lease.attempt,
            generation=lease.generation,
            instance=_activity_instance(lease, minor),
            item=collection_item,
            index=collection_index,
            idempotency_key=lease.idempotency_key,
            traceparent=lease.traceparent,
            task_kind=kind,
            target_node=target_node,
            target_idempotency_key=target_key,
            steps=steps,
            cancellation=task.token,
            log=_TaskLog(
                self._log,
                {
                    "task": lease.task_id,
                    "run": lease.run_id,
                    "node": node_id or lease.step_id,
                    "attempt": lease.attempt,
                },
            ),
            deadline_ms=lambda: task.deadline_ms,
            secrets=lambda: self._secrets_of(task, minor),
            write_blob=lambda source, content_type, expected_size, expected_sha256: self._write_blob(
                task, minor, source, content_type, expected_size, expected_sha256
            ),
            read_blob=lambda descriptor: self._read_blob(task, minor, descriptor),
        )

        # The heartbeat starts before the handler and runs until the report is made.
        heartbeat = asyncio.create_task(self._beat(task, minor), name=f"sagat-heartbeat-{lease.task_id}")
        task.beat = heartbeat
        try:
            handler_task = asyncio.ensure_future(self._invoke(registered, run_input, context))
            try:
                await asyncio.wait({handler_task})
            except asyncio.CancelledError:  # the runner itself is cancelled (a forced stop)
                handler_task.cancel()
                raise
            task.finished = True
            if task.secrets is not None:  # the values are dropped when the handler returns
                task.secrets._drop()
            await self._conclude(task, registered, kind, handler_task, started)
        finally:
            heartbeat.cancel()
            with contextlib.suppress(asyncio.CancelledError, Exception):
                await heartbeat
            if task.secrets is not None:
                task.secrets._drop()

    async def _invoke(self, registered: _Registered, run_input: Any, context: ActivityContext) -> Any:
        if registered.blocking:
            call = contextvars.copy_context()
            result = await asyncio.get_running_loop().run_in_executor(
                self._executor, lambda: call.run(registered.handler, run_input, context)
            )
        else:
            result = registered.handler(run_input, context)
        if inspect.isawaitable(result):
            result = await result
        return result

    async def _conclude(
        self, task: _Task, registered: _Registered, kind: TaskKind, handler_task: asyncio.Future[Any], started: float
    ) -> None:
        """Turns what the handler did into the one report of the attempt (``sdks/SPEC.md`` section 6.5)."""
        lease = task.lease
        minor = task.protocol_minor
        error: BaseException | None = None
        cancelled = handler_task.cancelled()
        if not cancelled:
            error = handler_task.exception()
            cancelled = error is not None and is_cancellation(error)

        if task.lost:
            self._emit(WorkerEvent("task.stale", task_id=lease.task_id))
            log(self._log, logging.WARNING, "the lease was lost: the result is dropped", task=lease.task_id)
            return
        if cancelled and task.token.reason == "shutdown":
            # The host is going away: the step did not finish, so its lease lapses and the server leases it again.
            self._emit(WorkerEvent("task.dropped", task_id=lease.task_id, reason="shutdown"))
            log(
                self._log,
                logging.WARNING,
                "the host stopped before the activity finished: nothing is reported",
                task=lease.task_id,
            )
            return
        if cancelled and task.token.reason == "run-cancelled":
            await self._fail(task, "CANCELLED", "the activity was cancelled", False, started)
            return
        if handler_task.cancelled():
            await self._fail(task, "ACTIVITY_ERROR", "CancelledError", True, started)
            return
        if error is not None:
            if isinstance(error, ActivityFailure):
                await self._fail(task, error.code, error.message, error.is_retryable, started)
            else:
                # Only the type name: the text of an ordinary error may hold the data it was handed.
                await self._fail(task, "ACTIVITY_ERROR", type(error).__name__, True, started)
            return

        result = handler_task.result()
        if kind == "RECONCILIATION" and registered.kind == "reconciler" and not isinstance(result, Verdict):
            await self._fail(
                task,
                "RECONCILIATION_ANSWER_INVALID",
                "a reconciler answers applied, not_applied or unknown",
                False,
                started,
            )
            return
        if isinstance(result, Verdict):
            result = result.to_json_value()
        blob: BlobDescriptor | None = None
        if isinstance(result, BlobOutput):
            if minor < 3:
                await self._fail(
                    task, "UNSUPPORTED_PROTOCOL", "blob output requires worker protocol 1.3", False, started
                )
                return
            try:
                self._validate_blob_descriptor(result.descriptor)
            except SagatError:
                await self._fail(task, "OUTPUT_INVALID", "the blob output descriptor is malformed", False, started)
                return
            blob = result.descriptor
            output = ""
        else:
            try:
                output = encode_json(result)
            except SagatError as invalid:
                await self._fail(task, "OUTPUT_INVALID", invalid.message, False, started)
                return
        if len(output.encode("utf-8")) > MAX_OUTPUT_BYTES:
            await self._fail(task, "PAYLOAD_TOO_LARGE", "the output is over 256 KiB", False, started)
            return
        await self._complete(task, output, started, blob)

    @staticmethod
    def _validate_blob_descriptor(descriptor: BlobDescriptor) -> None:
        if (
            not isinstance(descriptor, BlobDescriptor)
            or not isinstance(descriptor.id, str)
            or not descriptor.id.strip()
            or not isinstance(descriptor.size, int)
            or isinstance(descriptor.size, bool)
            or descriptor.size < 0
            or not isinstance(descriptor.sha256, str)
            or not _BLOB_SHA256.fullmatch(descriptor.sha256)
            or not isinstance(descriptor.content_type, str)
            or not descriptor.content_type.strip()
        ):
            raise SagatError("PROTOCOL", "the blob descriptor is malformed")

    async def _write_blob(
        self,
        task: _Task,
        minor: int,
        source: AsyncIterable[bytes] | Iterable[bytes],
        content_type: str,
        expected_size: int,
        expected_sha256: str,
    ) -> BlobDescriptor:
        if task.finished:
            raise SagatError("VALIDATION_FAILED", "the attempt is over")
        if minor < 3:
            raise SagatError("UNSUPPORTED_PROTOCOL", "blob streaming requires worker protocol 1.3")
        if expected_size < 0 or (expected_sha256 and not _BLOB_SHA256.fullmatch(expected_sha256)):
            raise SagatError("VALIDATION_FAILED", "blob preconditions are malformed")
        rpc = task.rpc
        assert rpc is not None
        lease = task.lease
        digest = hashlib.sha256()
        size = 0

        async def pieces() -> AsyncIterator[pb.WriteBlobRequest]:
            nonlocal size
            yield pb.WriteBlobRequest(
                header=pb.WriteBlobHeader(
                    task_id=lease.task_id,
                    run_id=lease.run_id,
                    step_id=lease.step_id,
                    attempt=lease.attempt,
                    generation=lease.generation,
                    lease_token=task.lease_token.reveal(),
                    content_type=content_type,
                    expected_size=expected_size,
                    expected_sha256=expected_sha256,
                    **_scoped_instance(lease, minor),
                )
            )
            if hasattr(source, "__aiter__"):
                iterator: Any = source.__aiter__()
                async for value in iterator:
                    for chunk in self._blob_chunks(value):
                        size += len(chunk)
                        digest.update(chunk)
                        yield pb.WriteBlobRequest(chunk=chunk)
            else:
                for value in source:
                    for chunk in self._blob_chunks(value):
                        size += len(chunk)
                        digest.update(chunk)
                        yield pb.WriteBlobRequest(chunk=chunk)

        response = await rpc.write_blob(pieces(), lease.traceparent or None)
        descriptor = BlobDescriptor(response.blob_id, response.size, response.sha256, response.content_type)
        self._validate_blob_descriptor(descriptor)
        if descriptor.size != size or descriptor.sha256 != digest.hexdigest():
            raise SagatError("PROTOCOL", "the blob service descriptor does not match the uploaded bytes")
        return descriptor

    @staticmethod
    def _blob_chunks(value: bytes) -> Iterable[bytes]:
        if not isinstance(value, bytes):
            raise SagatError("VALIDATION_FAILED", "a blob source yields bytes")
        for offset in range(0, len(value), MAX_BLOB_CHUNK):
            yield value[offset : offset + MAX_BLOB_CHUNK]

    async def _read_blob(self, task: _Task, minor: int, descriptor: BlobDescriptor) -> AsyncIterator[bytes]:
        if task.finished:
            raise SagatError("VALIDATION_FAILED", "the attempt is over")
        if minor < 3:
            raise SagatError("UNSUPPORTED_PROTOCOL", "blob streaming requires worker protocol 1.3")
        self._validate_blob_descriptor(descriptor)
        rpc = task.rpc
        assert rpc is not None
        lease = task.lease
        request = pb.ReadBlobRequest(
            task_id=lease.task_id,
            run_id=lease.run_id,
            step_id=lease.step_id,
            attempt=lease.attempt,
            generation=lease.generation,
            lease_token=task.lease_token.reveal(),
            blob_id=descriptor.id,
            **_scoped_instance(lease, minor),
        )
        digest = hashlib.sha256()
        size = 0
        header_seen = False
        async for part in rpc.read_blob(request, lease.traceparent or None):
            kind = part.WhichOneof("part")
            if kind == "header":
                header = part.header
                if (
                    header_seen
                    or size
                    or (header.blob_id, header.size, header.sha256, header.content_type)
                    != (descriptor.id, descriptor.size, descriptor.sha256, descriptor.content_type)
                ):
                    raise SagatError("PROTOCOL", "the blob stream header does not match its descriptor")
                header_seen = True
                continue
            if kind != "chunk" or not header_seen or len(part.chunk) > MAX_BLOB_CHUNK:
                raise SagatError("PROTOCOL", "the blob stream has an invalid part order or chunk size")
            size += len(part.chunk)
            if size > descriptor.size:
                raise SagatError("PROTOCOL", "the blob stream is larger than its descriptor")
            digest.update(part.chunk)
            yield part.chunk
        if not header_seen or size != descriptor.size or digest.hexdigest() != descriptor.sha256:
            raise SagatError("PROTOCOL", "the blob stream ended with mismatched integrity metadata")

    # --------------------------------------------------------------------------------------------- heartbeat

    def _interval(self, task: _Task) -> float:
        if self._config.heartbeat_interval:
            return self._config.heartbeat_interval
        left = max(0.0, task.deadline_ms / 1000 - time.time())
        return min(10.0, max(0.5, left / 3))

    async def _beat(self, task: _Task, minor: int) -> None:
        rpc = task.rpc
        assert rpc is not None
        lease = task.lease
        while True:
            await asyncio.sleep(self._interval(task))
            try:
                answer = await rpc.heartbeat(
                    pb.HeartbeatRequest(
                        task_id=lease.task_id,
                        worker_id=self._worker_id,
                        generation=lease.generation,
                        lease_token=task.lease_token.reveal(),
                        # The step identity came with 1.1.
                        run_id=lease.run_id if minor >= 1 else "",
                        step_id=lease.step_id if minor >= 1 else "",
                        attempt=lease.attempt if minor >= 1 else 0,
                        **_scoped_instance(lease, minor),
                    ),
                    lease.traceparent or None,
                )
            except SagatError as error:
                if error.code in ("STALE_ATTEMPT", "CONFLICT"):
                    task.lost = True
                    self._cancel_activity(task, "lease-lost")
                    return
                # Anything else: the next beat tries again; the lease is ours until it runs out.
                log(self._log, logging.WARNING, "a heartbeat failed", task=lease.task_id, code=error.code)
                continue
            except Exception as error:  # a hook of the caller (a token provider...) or a bug of ours: beat again
                # The class only: the text of an exception may carry the credential it was handling.
                log(
                    self._log,
                    logging.ERROR,
                    "a heartbeat failed unexpectedly",
                    task=lease.task_id,
                    error=type(error).__name__,
                )
                continue
            self._emit(WorkerEvent("heartbeat", task_id=lease.task_id))
            if answer.lease_expires_at_epoch_ms > 0:
                task.deadline_ms = answer.lease_expires_at_epoch_ms
            if answer.cancel_requested:
                self._cancel_activity(task, "run-cancelled")

    # ------------------------------------------------------------------------------------------------ reporting

    async def _with_retries(self, call: Callable[[], Awaitable[T]]) -> T | Literal["stale"]:
        """Repeats a report with the same body and key; a stale answer ends it (the lease is gone)."""
        n = 0
        while True:
            try:
                return await call()
            except SagatError as error:
                if error.code == "STALE_ATTEMPT":
                    return "stale"
                # A result that was computed is not lost to a hiccup of the identity provider: the failure of the
                # provider is known by its class, never by a reason a server could send (SPEC section 5).
                repeatable = isinstance(error, ProviderFailure) or (error.code in _REPEATABLE and error.retryable)
                if repeatable and n + 1 < REPORT_ATTEMPTS:
                    await asyncio.sleep(max(backoff_delay(self._config.retry, n), server_wait(error.retry_after_ms)))
                    n += 1
                    continue
                raise

    async def _complete(self, task: _Task, output: str, started: float, blob: BlobDescriptor | None = None) -> None:
        lease = task.lease
        rpc = task.rpc
        assert rpc is not None
        request = pb.CompleteStepRequest(
            task_id=lease.task_id,
            run_id=lease.run_id,
            step_id=lease.step_id,
            attempt=lease.attempt,
            generation=lease.generation,
            lease_token=task.lease_token.reveal(),
            idempotency_key=f"done-{lease.task_id}",
            output_json=output,
            traceparent=lease.traceparent,
            output_blob_ref=blob.id if blob else "",
            output_sha256=blob.sha256 if blob else "",
            **_scoped_instance(lease, task.protocol_minor),
        )
        try:
            answer = await self._with_retries(lambda: rpc.complete_step(request, lease.traceparent or None))
        except SagatError as error:
            self._emit(WorkerEvent("task.dropped", task_id=lease.task_id, reason=error.code))
            log(self._log, logging.ERROR, "the completion could not be reported", task=lease.task_id, code=error.code)
            return
        if answer == "stale":
            self._emit(WorkerEvent("task.stale", task_id=lease.task_id))
            return
        self._emit(
            WorkerEvent("task.completed", task_id=lease.task_id, duration_ms=round((time.monotonic() - started) * 1000))
        )

    async def _fail(self, task: _Task, code: str, message: str, retryable: bool, started: float) -> None:
        lease = task.lease
        rpc = task.rpc
        assert rpc is not None
        safe_code = code if _CODE.fullmatch(code) else "WORKER_ERROR"
        # 512 code points, never inside a character; a lone surrogate (which no UTF-8 can carry) becomes U+FFFD.
        safe_message = _SURROGATES.sub("\ufffd", str(message))[:512]
        request = pb.FailStepRequest(
            task_id=lease.task_id,
            run_id=lease.run_id,
            step_id=lease.step_id,
            attempt=lease.attempt,
            generation=lease.generation,
            lease_token=task.lease_token.reveal(),
            idempotency_key=f"failed-{lease.task_id}",
            error_code=safe_code,
            safe_message=safe_message,
            retryable=retryable,
            traceparent=lease.traceparent,
            **_scoped_instance(lease, task.protocol_minor),
        )
        try:
            answer = await self._with_retries(lambda: rpc.fail_step(request, lease.traceparent or None))
        except SagatError as error:
            self._emit(WorkerEvent("task.dropped", task_id=lease.task_id, reason=error.code))
            log(self._log, logging.ERROR, "the failure could not be reported", task=lease.task_id, code=error.code)
            return
        if answer == "stale":
            self._emit(WorkerEvent("task.stale", task_id=lease.task_id))
            return
        self._emit(
            WorkerEvent(
                "task.failed",
                task_id=lease.task_id,
                code=safe_code,
                retryable=retryable,
                duration_ms=round((time.monotonic() - started) * 1000),
            )
        )

    # --------------------------------------------------------------------------------------------------- secrets

    async def _secrets_of(self, task: _Task, minor: int) -> Secrets:
        """``ctx.secrets()``: one ``ResolveSecrets`` per attempt, on first use."""
        async with task.secrets_lock:
            if task.finished:
                raise SagatError(
                    "VALIDATION_FAILED", "the secrets of this attempt were dropped when the handler returned"
                )
            if task.secrets is None:
                resolved = await self._resolve_secrets(task, minor)
                if task.finished:
                    # The handler returned while the call was in flight (a `ctx.secrets()` it scheduled and did
                    # not await): nobody may read what came back.
                    resolved._drop()
                    raise SagatError(
                        "VALIDATION_FAILED", "the secrets of this attempt were dropped when the handler returned"
                    )
                task.secrets = resolved
            return task.secrets

    async def _resolve_secrets(self, task: _Task, minor: int) -> Secrets:
        lease = task.lease
        if minor < 2:
            raise SagatError(
                "UNSUPPORTED_PROTOCOL", "the server speaks a protocol older than 1.2, which has no ResolveSecrets"
            )
        assert task.rpc is not None
        answer = await task.rpc.resolve_secrets(
            pb.ResolveSecretsRequest(
                task_id=lease.task_id,
                run_id=lease.run_id,
                step_id=lease.step_id,
                attempt=lease.attempt,
                generation=lease.generation,
                lease_token=task.lease_token.reveal(),
                traceparent=lease.traceparent,
                **_scoped_instance(lease, minor),
            ),
            lease.traceparent or None,
        )
        return Secrets({secret.name: secret.value for secret in answer.secrets})
