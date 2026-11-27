"""An in-process HTTP server and an in-process gRPC ``WorkerService`` that the tests script. Offline."""

from __future__ import annotations

import asyncio
import json
import threading
import time
from collections.abc import Awaitable, Callable, Iterator
from dataclasses import dataclass, field
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import Any, cast

import grpc
import grpc.aio
import pytest

from sagat_octopus_orchestrator._proto import worker_pb2 as pb
from sagat_octopus_orchestrator._proto import worker_pb2_grpc as pb_grpc

# ------------------------------------------------------------------------------------------------- HTTP


@dataclass
class Seen:
    method: str
    url: str
    headers: dict[str, str]
    body: Any


@dataclass
class Reply:
    status: int
    body: Any = None
    headers: dict[str, str] = field(default_factory=dict)
    stream: Iterator[bytes] | None = None
    """Chunks written as they come, with no length and the connection closed at the end."""


def reply(status: int, body: Any = None, **headers: str) -> Reply:
    return Reply(status, body, {name.replace("_", "-"): value for name, value in headers.items()})


class FakeHttpServer:
    """A REST API the test scripts: ``handler`` answers every request; ``seen`` records them."""

    def __init__(self) -> None:
        self.seen: list[Seen] = []
        self.handler: Callable[[Seen], Reply] = lambda _seen: reply(404)
        server = self

        class Handler(BaseHTTPRequestHandler):
            protocol_version = "HTTP/1.1"

            def _serve(self) -> None:
                length = int(self.headers.get("content-length") or 0)
                text = self.rfile.read(length).decode("utf-8") if length else ""
                entry = Seen(
                    self.command,
                    self.path,
                    {name.lower(): value for name, value in self.headers.items()},
                    json.loads(text) if text else None,
                )
                server.seen.append(entry)
                answer = server.handler(entry)
                if answer.stream is not None:
                    self.send_response(answer.status)
                    for name, value in {"content-type": "application/json", **answer.headers}.items():
                        self.send_header(name, value)
                    self.send_header("connection", "close")
                    self.end_headers()
                    self.close_connection = True
                    try:
                        for chunk in answer.stream:
                            self.wfile.write(chunk)
                            self.wfile.flush()
                    except OSError:
                        pass  # the client has read what it wants
                    return
                if answer.body is None:
                    payload = b""
                elif isinstance(answer.body, str):
                    payload = answer.body.encode("utf-8")
                else:
                    payload = json.dumps(answer.body).encode("utf-8")
                self.send_response(answer.status)
                headers = {"content-type": "application/json", "x-trace-id": "f" * 32, **answer.headers}
                for name, value in headers.items():
                    self.send_header(name, value)
                self.send_header("content-length", str(len(payload)))
                self.end_headers()
                self.wfile.write(payload)

            do_GET = do_POST = do_PUT = do_DELETE = _serve

            def log_message(self, format: str, *args: Any) -> None:
                pass

        self._server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self._server.daemon_threads = True
        self._thread = threading.Thread(target=self._server.serve_forever, kwargs={"poll_interval": 0.01}, daemon=True)
        self._thread.start()
        self.base = f"http://127.0.0.1:{self._server.server_address[1]}/v1"

    def serve(self, handler: Callable[[Seen], Reply]) -> None:
        self.seen.clear()
        self.handler = handler

    def close(self) -> None:
        self._server.shutdown()
        self._server.server_close()


# ------------------------------------------------------------------------------------------------- gRPC


@dataclass
class Call:
    method: str
    request: Any
    metadata: dict[str, str]
    at: float = 0.0


@dataclass
class Scripted:
    code: grpc.StatusCode
    reason: str | None = None
    retry_after_ms: int | None = None


@dataclass
class Held:
    """An ``AppendStepLogs`` answer held until the test sets ``release``, then answered with ``then`` (a verdict or a
    scripted failure; ``CONTINUE`` when ``None``): the barrier that keeps a batch in flight (AGENTS.md Rule 18)."""

    release: asyncio.Event = field(default_factory=asyncio.Event)
    then: pb.AppendStepLogsResponse | Scripted | None = None


def verdict(name: str, stop_reason: str = "STEP_LOG_STOP_REASON_UNSPECIFIED", retry_after_ms: int = 0) -> Any:
    """An ``AppendStepLogsResponse`` by the names of its enums."""
    return pb.AppendStepLogsResponse(
        verdict=cast(Any, pb.StepLogVerdict.Value(name)),
        stop_reason=cast(Any, pb.StepLogStopReason.Value(stop_reason)),
        retry_after_ms=retry_after_ms,
    )


class FakeWorkerService(pb_grpc.WorkerServiceServicer):
    """An in-memory worker service: queued leases, queued failures, a record of every call."""

    def __init__(self) -> None:
        self.calls: list[Call] = []
        self.leases: list[pb.TaskLease] = []
        self.failures: dict[str, list[Scripted]] = {}
        self.always: dict[str, Scripted] = {}
        self.before: dict[str, Callable[[], Awaitable[None]]] = {}
        """Runs when a call of that method arrives, once it is recorded and before it is answered: a test holds a
        call in flight with it until what the test claims has happened (a barrier, never a sleep: AGENTS.md Rule 18)."""
        self.ignore_max_tasks = False
        self.session_token = "session-1"
        self.resolve_delay = 0.0
        self.accepted_version = "1.2"
        self.cancel_requested = False
        self.heartbeat_expires_ms = int(time.time() * 1000) + 60_000
        self.secrets: list[pb.ResolvedSecret] = []
        self.poll_delay = 0.0
        self.poll_retry_after_ms = 50
        self.stream_batches: list[list[pb.TaskLease]] | None = None
        self.stream_failure_after_first: grpc.StatusCode | None = None
        self.hold_stream = False
        self.stream_cancelled = 0
        self.fail_gate: Callable[[], Awaitable[None]] | None = None
        """Awaited by ``FailStep`` after the call is recorded and before it is answered: a test holds failures there."""
        self.step_logs: bool | None = None
        """The ``StepLogPolicy`` of the registration (protocol 1.7): ``None`` sends no policy."""
        self.append_script: list[pb.AppendStepLogsResponse | Scripted | Held] = []
        """The answers of ``AppendStepLogs`` in order; ``CONTINUE`` once it is empty."""
        self.append_answered: list[float] = []
        """When each ``AppendStepLogs`` was answered (``time.monotonic``), a scripted failure included."""
        self.port = 0
        self._server: grpc.aio.Server | None = None

    async def start(self) -> None:
        self._server = grpc.aio.server()
        cast(Any, pb_grpc).add_WorkerServiceServicer_to_server(self, self._server)
        self.port = self._server.add_insecure_port("127.0.0.1:0")
        await self._server.start()

    async def stop(self) -> None:
        if self._server is not None:
            await self._server.stop(None)

    @property
    def url(self) -> str:
        return f"http://127.0.0.1:{self.port}"

    def of(self, method: str) -> list[Call]:
        return [call for call in self.calls if call.method == method]

    async def _enter(self, method: str, request: Any, context: grpc.aio.ServicerContext[Any, Any]) -> None:
        metadata = {key: str(value) for key, value in context.invocation_metadata() or ()}
        self.calls.append(Call(method, request, metadata, time.monotonic()))
        hook = self.before.get(method)
        if hook is not None:
            await hook()
        queue = self.failures.get(method)
        scripted = queue.pop(0) if queue else self.always.get(method)
        if scripted is not None:
            await self._abort(scripted, context)

    @staticmethod
    async def _abort(scripted: Scripted, context: grpc.aio.ServicerContext[Any, Any]) -> None:
        trailers = []
        if scripted.reason:
            trailers.append(("x-sagat-error", scripted.reason))
        if scripted.retry_after_ms is not None:
            trailers.append(("retry-after-ms", str(scripted.retry_after_ms)))
        context.set_trailing_metadata(tuple(trailers))
        await context.abort(scripted.code, "scripted")

    async def RegisterWorker(self, request: Any, context: Any) -> Any:
        await self._enter("RegisterWorker", request, context)
        response = pb.RegisterWorkerResponse(
            worker_id="w-1", session_token=self.session_token, accepted_protocol_version=self.accepted_version
        )
        if self.step_logs is not None:
            response.step_logs.enabled = self.step_logs
        return response

    async def PollTasks(self, request: Any, context: Any) -> Any:
        await self._enter("PollTasks", request, context)
        if self.poll_delay > 0:
            await asyncio.sleep(self.poll_delay)
        # Like the real service: no more than were asked for.
        take = len(self.leases) if self.ignore_max_tasks else request.max_tasks
        tasks, self.leases = self.leases[:take], self.leases[take:]
        return pb.PollTasksResponse(tasks=tasks, retry_after_ms=self.poll_retry_after_ms)

    async def StreamTasks(self, request: Any, context: Any) -> Any:
        await self._enter("StreamTasks", request, context)
        batches = self.stream_batches
        if batches is None:
            take = len(self.leases) if self.ignore_max_tasks else request.max_tasks
            tasks, self.leases = self.leases[:take], self.leases[take:]
            batches = [tasks]
        self.stream_batches = None
        sent = 0
        try:
            for tasks in batches:
                if not tasks:
                    continue
                sent += len(tasks)
                yield pb.PollTasksResponse(tasks=tasks)
                if self.stream_failure_after_first is not None:
                    code, self.stream_failure_after_first = self.stream_failure_after_first, None
                    await asyncio.sleep(0.02)
                    await context.abort(code, "scripted")
            if self.hold_stream:
                await asyncio.Event().wait()
            if sent < request.max_tasks:
                yield pb.PollTasksResponse(retry_after_ms=self.poll_retry_after_ms)
        finally:
            if self.hold_stream or context.cancelled():
                self.stream_cancelled += 1

    async def Heartbeat(self, request: Any, context: Any) -> Any:
        await self._enter("Heartbeat", request, context)
        return pb.HeartbeatResponse(
            lease_expires_at_epoch_ms=self.heartbeat_expires_ms, cancel_requested=self.cancel_requested
        )

    async def CompleteStep(self, request: Any, context: Any) -> Any:
        await self._enter("CompleteStep", request, context)
        return pb.StepCommandResponse(
            command_id="c", already_applied=False, step_status="SUCCEEDED", run_status="RUNNING"
        )

    async def FailStep(self, request: Any, context: Any) -> Any:
        await self._enter("FailStep", request, context)
        if self.fail_gate is not None:
            await self.fail_gate()
        return pb.StepCommandResponse(command_id="c", already_applied=False, step_status="FAILED", run_status="RUNNING")

    async def ResolveSecrets(self, request: Any, context: Any) -> Any:
        await self._enter("ResolveSecrets", request, context)
        if self.resolve_delay > 0:
            await asyncio.sleep(self.resolve_delay)
        return pb.ResolveSecretsResponse(secrets=self.secrets)

    async def AppendStepLogs(self, request: Any, context: Any) -> Any:
        await self._enter("AppendStepLogs", request, context)
        step = self.append_script.pop(0) if self.append_script else None
        if isinstance(step, Held):
            await step.release.wait()
            step = step.then
        self.append_answered.append(time.monotonic())
        if isinstance(step, Scripted):
            await self._abort(step, context)
        return step if step is not None else verdict("STEP_LOG_VERDICT_CONTINUE")


def lease(**overrides: Any) -> pb.TaskLease:
    """A lease for the activity ``charge`` of run ``run-1``, with a step document the host can read."""
    fields: dict[str, Any] = {
        "task_id": "run-1/charge/1",
        "run_id": "run-1",
        "step_id": "charge",
        "attempt": 1,
        "generation": 1,
        "lease_token": "lt",
        "expires_at_epoch_ms": int(time.time() * 1000) + 60_000,
        "activity_name": "charge",
        "input_json": json.dumps(
            {
                "run": {"id": "run-1"},
                "node": "charge",
                "input": {"amount": 5},
                "steps": {"earlier": {"output": {"ok": True}}},
            }
        ),
        "traceparent": "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01",
        "idempotency_key": "run-1:charge",
    }
    fields.update(overrides)
    return pb.TaskLease(**fields)


def record_sends(host: Any, order: list[str]) -> None:
    """Wraps the connection of a started host so that ``order`` reads, in the order the host did it, each heartbeat it
    sent and each report the server answered.

    The side of the host: a call that was sent before the report but reaches the fake after it cannot blur what the test
    reads. Call it right after ``start`` returns, before anything awaits, so the first lease is not taken yet.
    """
    rpc = host._rpc
    assert rpc is not None

    def wrap(name: str, label: str) -> None:
        real = getattr(rpc, name)

        async def call(*args: Any, **kwargs: Any) -> Any:
            order.append(f"sent {label}")
            answer = await real(*args, **kwargs)
            order.append(f"answered {label}")
            return answer

        setattr(rpc, name, call)

    wrap("heartbeat", "Heartbeat")
    wrap("complete_step", "CompleteStep")


async def reached(condition: Callable[[], bool], seconds: float = 3.0) -> bool:
    """Waits for ``condition`` and says whether it came within ``seconds``.

    The barrier of an activity under test: the handler holds its task until what the test claims has happened (three
    heartbeats, a beat after a failure) instead of sleeping for a time it hopes is long enough (AGENTS.md Rule 18). It
    does not fail by itself and gives up before the five seconds of ``until``, so the assertion of the test stays the
    one that says what is missing.
    """
    end = asyncio.get_running_loop().time() + seconds
    while not condition():
        if asyncio.get_running_loop().time() > end:
            return False
        await asyncio.sleep(0.01)
    return True


async def until(condition: Callable[[], bool], what: str, seconds: float = 5.0) -> None:
    """Waits for ``condition``; the test fails, saying what it waited for, if it does not come."""
    if not await reached(condition, seconds):
        pytest.fail(f"timed out waiting for {what}")
