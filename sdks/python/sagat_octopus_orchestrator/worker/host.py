"""The worker host (``sdks/SPEC.md`` section 6): registration, the poll loop, heartbeats, cancellation and reports."""

from __future__ import annotations

import asyncio
import contextlib
import contextvars
import functools
import inspect
import logging
import random
import re
import time
from collections.abc import Awaitable, Callable, Sequence
from concurrent.futures import ThreadPoolExecutor
from dataclasses import dataclass, field
from typing import TYPE_CHECKING, Any, Literal, TypeVar

import grpc.aio

from .._json import decode_json, encode_json, utf16_key
from .._proto import worker_pb2 as pb
from ..config import (
    LOG,
    PROTOCOL_VERSION,
    TlsOptions,
    TokenSource,
    log,
    warn_if_plaintext,
)
from ..errors import SagatError
from ..retry import DEFAULT_RETRY, RetryPolicy, backoff_delay
from ._grpc import GrpcConfig, WorkerRpc
from .activity import (
    ActivityContext,
    ActivityFailure,
    ActivityHandler,
    CancellationToken,
    CancelReason,
    Secrets,
    TaskKind,
    Verdict,
    is_cancellation,
)

T = TypeVar("T")
Kind = Literal["activity", "compensation", "reconciler"]

MAX_OUTPUT_BYTES = 256 * 1024
MAX_POLL = 64
REPORT_ATTEMPTS = 5
_CODE = re.compile(r"[A-Z0-9_]{1,64}")
_VERSION = re.compile(r"^1(?:\.(\d+))?$")
_REPEATABLE = frozenset({"TRANSPORT", "UNAVAILABLE", "RATE_LIMITED"})
_TASK_KINDS: dict[str, TaskKind] = {"COMPENSATION": "COMPENSATION", "RECONCILIATION": "RECONCILIATION"}


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


@dataclass(frozen=True)
class WorkerConfig:
    """Configuration of ``WorkerHost``. Durations are in seconds."""

    grpc_url: str
    """``http://host:port`` or ``https://host:port`` of the worker service."""
    namespace: str
    token: TokenSource
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
    heartbeat_interval: float | None = None
    """Overrides the heartbeat interval (otherwise a third of the lease, 0.5 to 10 s)."""
    logger: logging.Logger = LOG
    interceptors: Sequence[grpc.aio.ClientInterceptor] = ()
    """Interceptors of the ``grpc.aio`` channel: the hook of the conformance suite and of tests."""
    on_event: Callable[[WorkerEvent], None] | None = None


@dataclass
class _Registered:
    kind: Kind
    handler: ActivityHandler
    parse: Callable[[Any], Any] | None
    blocking: bool


@dataclass(eq=False)
class _Task:
    """One lease being worked on."""

    lease: pb.TaskLease
    token: CancellationToken = field(default_factory=CancellationToken)
    deadline_ms: int = 0
    lost: bool = False
    runner: asyncio.Task[None] | None = None
    handler_task: asyncio.Future[Any] | None = None
    in_thread: bool = False
    """True while a blocking handler runs in its thread: it can only be asked to stop, through the token."""
    secrets: Secrets | None = None
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
    match = _VERSION.match(version)
    return int(match[1] or 0) if match else 0


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
        self._config = config
        self._log = config.logger
        self._handlers: dict[str, _Registered] = {}
        self._rpc: WorkerRpc | None = None
        self._executor: ThreadPoolExecutor | None = None
        self._worker_id = ""
        self._session = ""
        self._accepted = ""
        self._running = False
        self._stopping = False
        self._loop_task: asyncio.Task[None] | None = None
        self._wake: asyncio.Event | None = None
        self._inflight: set[_Task] = set()

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
        """Opens the connection, registers with the server and starts the loop; returns once registered."""
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
        """Stops polling, waits up to ``timeout`` seconds for the tasks in flight, then cancels the ones left
        (reason ``shutdown``). A task that did not finish keeps its lease until it lapses: the server leases
        it again. The host may be started again."""
        if not self._running:
            return
        self._stopping = True
        assert self._wake is not None
        assert self._loop_task is not None
        self._wake.set()
        # The poll in progress ends and no other starts; what it leased is in flight from here on.
        await asyncio.wait({self._loop_task}, timeout=timeout)
        if not self._loop_task.done():
            self._loop_task.cancel()
            await asyncio.gather(self._loop_task, return_exceptions=True)
        pending = {task.runner for task in self._inflight if task.runner is not None}
        if pending:
            _, late = await asyncio.wait(pending, timeout=timeout)
            if late:
                for task in list(self._inflight):
                    self._cancel_activity(task, "shutdown")
                _, stuck = await asyncio.wait(late, timeout=2.0)
                for runner in stuck:
                    runner.cancel()
                await asyncio.gather(*stuck, return_exceptions=True)
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
                if error.code == "UNAUTHENTICATED" and not refreshed and callable(self._config.token):
                    refreshed = True
                    continue
                if error.code not in _REPEATABLE or not error.retryable or n + 1 >= self._config.retry.max_attempts:
                    raise
                await asyncio.sleep(max(backoff_delay(self._config.retry, n), (error.retry_after_ms or 0) / 1000))
                n += 1
                continue
            self._worker_id = response.worker_id
            self._session = response.session_token
            self._accepted = response.accepted_protocol_version
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
        max_inflight = self._config.max_inflight
        while not self._stopping:
            try:
                free = max_inflight - len(self._inflight)
                if free <= 0:
                    await self._idle(0.05)
                    continue
                response = await self._rpc.poll_tasks(
                    pb.PollTasksRequest(
                        worker_id=self._worker_id, session_token=self._session, max_tasks=min(free, MAX_POLL)
                    )
                )
                failures = 0
                self._emit(WorkerEvent("poll", tasks=len(response.tasks)))
                for lease in response.tasks:
                    self._dispatch(lease)
                if not response.tasks:
                    wait_ms = max(50, response.retry_after_ms if response.retry_after_ms > 0 else 1000)
                    await self._idle(wait_ms / 1000 * (0.8 + random.random() * 0.4))
            except SagatError as error:
                if self._stopping:
                    return
                if error.code == "UNAUTHENTICATED":
                    log(self._log, logging.WARNING, "the session is gone: registering again")
                    self._emit(WorkerEvent("reconnect"))
                    try:
                        await self._register()
                        continue
                    except SagatError as again:
                        log(self._log, logging.ERROR, "registering again failed", code=again.code)
                if error.code == "RATE_LIMITED":
                    await self._idle(max((error.retry_after_ms or 0) / 1000, 0.1))
                    continue
                self._emit(WorkerEvent("reconnect"))
                policy = RetryPolicy(self._config.retry.max_attempts, self._config.retry.initial_delay, 30.0)
                pause = min(30.0, max(backoff_delay(policy, failures), (error.retry_after_ms or 0) / 1000))
                failures += 1
                await self._idle(pause)
            except Exception as error:  # a bug of ours must not stop the host
                if self._stopping:
                    return
                log(self._log, logging.ERROR, "the poll loop failed", error=type(error).__name__)
                await self._idle(1.0)

    # ---------------------------------------------------------------------------------------------------- tasks

    def _dispatch(self, lease: pb.TaskLease) -> None:
        task = _Task(lease, deadline_ms=lease.expires_at_epoch_ms)
        self._inflight.add(task)
        runner = asyncio.create_task(self._execute(task), name=f"sagat-task-{lease.task_id}")
        task.runner = runner
        runner.add_done_callback(functools.partial(self._finished, task))
        self._emit(
            WorkerEvent("task.taken", task_id=lease.task_id, activity=lease.activity_name, attempt=lease.attempt)
        )

    def _finished(self, task: _Task, done: asyncio.Future[None]) -> None:
        self._inflight.discard(task)
        if not done.cancelled() and done.exception() is not None:  # a bug of ours: say so, once, without the text
            log(self._log, logging.ERROR, "a task ended with an unexpected error", task=task.lease.task_id,
                error=type(done.exception()).__name__)  # fmt: skip
        if self._wake is not None:
            self._wake.set()

    def _cancel_activity(self, task: _Task, reason: CancelReason) -> None:
        """Cancels the activity: the token first, then, for an ``async`` handler, its task (reason as message)."""
        if not task.token.cancel(reason):
            return
        handler_task = task.handler_task
        if handler_task is not None and not handler_task.done() and not task.in_thread:
            handler_task.cancel(reason)

    async def _execute(self, task: _Task) -> None:
        lease = task.lease
        started = time.monotonic()
        registered = self._handlers.get(lease.activity_name)
        kind: TaskKind = _TASK_KINDS.get(lease.task_kind, "FORWARD")
        minor = _minor_of(self._accepted)

        if registered is None:
            await self._fail(lease, "NO_HANDLER", "no handler is registered for the activity", False, started)
            return

        document: dict[str, Any] = {}
        try:
            parsed = decode_json(lease.input_json) if lease.input_json else {}
            if isinstance(parsed, dict):
                document = parsed
        except ValueError:
            await self._fail(lease, "INPUT_INVALID", "the step document is not JSON", False, started)
            return
        run_input: Any = document.get("input")
        try:
            if registered.parse is not None:
                run_input = registered.parse(run_input)
        except Exception:
            await self._fail(lease, "INPUT_INVALID", "the input does not have the type of the handler", False, started)
            return
        steps: dict[str, Any] = {}
        raw_steps = document.get("steps")
        for node, value in raw_steps.items() if isinstance(raw_steps, dict) else ():
            if isinstance(value, dict) and "output" in value:
                steps[node] = value["output"]

        node_id = document.get("node")
        context = ActivityContext(
            activity_name=lease.activity_name,
            task_id=lease.task_id,
            run_id=lease.run_id,
            node=node_id if isinstance(node_id, str) and node_id else lease.step_id,
            step_id=lease.step_id,
            attempt=lease.attempt,
            generation=lease.generation,
            idempotency_key=lease.idempotency_key,
            traceparent=lease.traceparent,
            task_kind=kind,
            target_node=lease.target_node,
            target_idempotency_key=lease.target_idempotency_key,
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
        )

        # The heartbeat starts before the handler and runs until the report is made.
        heartbeat = asyncio.create_task(self._beat(task, minor), name=f"sagat-heartbeat-{lease.task_id}")
        try:
            handler_task = asyncio.ensure_future(self._invoke(task, registered, run_input, context))
            task.handler_task = handler_task
            try:
                await asyncio.wait({handler_task})
            except asyncio.CancelledError:  # the runner itself is cancelled (a forced stop)
                handler_task.cancel()
                raise
            if task.secrets is not None:  # the values are dropped when the handler returns
                task.secrets._drop()
            await self._conclude(task, registered, handler_task, started)
        finally:
            heartbeat.cancel()
            with contextlib.suppress(asyncio.CancelledError, Exception):
                await heartbeat
            if task.secrets is not None:
                task.secrets._drop()

    async def _invoke(self, task: _Task, registered: _Registered, run_input: Any, context: ActivityContext) -> Any:
        if registered.blocking:
            call = contextvars.copy_context()
            task.in_thread = True
            try:
                result = await asyncio.get_running_loop().run_in_executor(
                    self._executor, lambda: call.run(registered.handler, run_input, context)
                )
            finally:
                task.in_thread = False
        else:
            result = registered.handler(run_input, context)
        if inspect.isawaitable(result):
            if task.token.is_cancelled and (current := asyncio.current_task()) is not None:
                current.cancel(task.token.reason)  # cancelled while the thread ran: the awaitable never gets going
            result = await result
        return result

    async def _conclude(
        self, task: _Task, registered: _Registered, handler_task: asyncio.Future[Any], started: float
    ) -> None:
        """Turns what the handler did into the one report of the attempt (``sdks/SPEC.md`` section 6.5)."""
        lease = task.lease
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
            await self._fail(lease, "CANCELLED", "the activity was cancelled", False, started)
            return
        if handler_task.cancelled():
            await self._fail(lease, "ACTIVITY_ERROR", "CancelledError", True, started)
            return
        if error is not None:
            if isinstance(error, ActivityFailure):
                await self._fail(lease, error.code, error.message, error.is_retryable, started)
            else:
                # Only the type name: the text of an ordinary error may hold the data it was handed.
                await self._fail(lease, "ACTIVITY_ERROR", type(error).__name__, True, started)
            return

        result = handler_task.result()
        if lease.task_kind == "RECONCILIATION" and registered.kind == "reconciler" and not isinstance(result, Verdict):
            await self._fail(
                lease,
                "RECONCILIATION_ANSWER_INVALID",
                "a reconciler answers applied, not_applied or unknown",
                False,
                started,
            )
            return
        if isinstance(result, Verdict):
            result = result.to_json_value()
        try:
            output = encode_json(result)
        except SagatError as invalid:
            await self._fail(lease, "OUTPUT_INVALID", invalid.message, False, started)
            return
        if len(output.encode("utf-8")) > MAX_OUTPUT_BYTES:
            await self._fail(lease, "PAYLOAD_TOO_LARGE", "the output is over 256 KiB", False, started)
            return
        await self._complete(lease, output, started)

    # --------------------------------------------------------------------------------------------- heartbeat

    def _interval(self, task: _Task) -> float:
        if self._config.heartbeat_interval:
            return self._config.heartbeat_interval
        left = max(0.0, task.deadline_ms / 1000 - time.time())
        return min(10.0, max(0.5, left / 3))

    async def _beat(self, task: _Task, minor: int) -> None:
        assert self._rpc is not None
        lease = task.lease
        while True:
            await asyncio.sleep(self._interval(task))
            try:
                answer = await self._rpc.heartbeat(
                    pb.HeartbeatRequest(
                        task_id=lease.task_id,
                        worker_id=self._worker_id,
                        generation=lease.generation,
                        lease_token=lease.lease_token,
                        # The step identity came with 1.1.
                        run_id=lease.run_id if minor >= 1 else "",
                        step_id=lease.step_id if minor >= 1 else "",
                        attempt=lease.attempt if minor >= 1 else 0,
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
                if error.code in _REPEATABLE and error.retryable and n + 1 < REPORT_ATTEMPTS:
                    await asyncio.sleep(max(backoff_delay(self._config.retry, n), (error.retry_after_ms or 0) / 1000))
                    n += 1
                    continue
                raise

    async def _complete(self, lease: pb.TaskLease, output: str, started: float) -> None:
        assert self._rpc is not None
        rpc = self._rpc
        request = pb.CompleteStepRequest(
            task_id=lease.task_id,
            run_id=lease.run_id,
            step_id=lease.step_id,
            attempt=lease.attempt,
            generation=lease.generation,
            lease_token=lease.lease_token,
            idempotency_key=f"done-{lease.task_id}",
            output_json=output,
            traceparent=lease.traceparent,
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

    async def _fail(self, lease: pb.TaskLease, code: str, message: str, retryable: bool, started: float) -> None:
        assert self._rpc is not None
        rpc = self._rpc
        safe_code = code if _CODE.fullmatch(code) else "WORKER_ERROR"
        request = pb.FailStepRequest(
            task_id=lease.task_id,
            run_id=lease.run_id,
            step_id=lease.step_id,
            attempt=lease.attempt,
            generation=lease.generation,
            lease_token=lease.lease_token,
            idempotency_key=f"failed-{lease.task_id}",
            error_code=safe_code,
            safe_message=message[:512],
            retryable=retryable,
            traceparent=lease.traceparent,
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
            if task.secrets is None:
                task.secrets = await self._resolve_secrets(task.lease, minor)
            return task.secrets

    async def _resolve_secrets(self, lease: pb.TaskLease, minor: int) -> Secrets:
        if minor < 2:
            raise SagatError(
                "UNSUPPORTED_PROTOCOL", "the server speaks a protocol older than 1.2, which has no ResolveSecrets"
            )
        assert self._rpc is not None
        answer = await self._rpc.resolve_secrets(
            pb.ResolveSecretsRequest(
                task_id=lease.task_id,
                run_id=lease.run_id,
                step_id=lease.step_id,
                attempt=lease.attempt,
                generation=lease.generation,
                lease_token=lease.lease_token,
                traceparent=lease.traceparent,
            ),
            lease.traceparent or None,
        )
        return Secrets({secret.name: bytes(secret.value) for secret in answer.secrets})
