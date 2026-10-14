"""An in-process HTTP server and an in-process gRPC ``WorkerService`` that the tests script. Offline."""

from __future__ import annotations

import asyncio
import json
import threading
import time
from collections.abc import Callable, Iterator
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


class FakeWorkerService(pb_grpc.WorkerServiceServicer):
    """An in-memory worker service: queued leases, queued failures, a record of every call."""

    def __init__(self) -> None:
        self.calls: list[Call] = []
        self.leases: list[pb.TaskLease] = []
        self.failures: dict[str, list[Scripted]] = {}
        self.always: dict[str, Scripted] = {}
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
        queue = self.failures.get(method)
        scripted = queue.pop(0) if queue else self.always.get(method)
        if scripted is not None:
            trailers = []
            if scripted.reason:
                trailers.append(("x-sagat-error", scripted.reason))
            if scripted.retry_after_ms is not None:
                trailers.append(("retry-after-ms", str(scripted.retry_after_ms)))
            context.set_trailing_metadata(tuple(trailers))
            await context.abort(scripted.code, "scripted")

    async def RegisterWorker(self, request: Any, context: Any) -> Any:
        await self._enter("RegisterWorker", request, context)
        return pb.RegisterWorkerResponse(
            worker_id="w-1", session_token=self.session_token, accepted_protocol_version=self.accepted_version
        )

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
        return pb.StepCommandResponse(command_id="c", already_applied=False, step_status="FAILED", run_status="RUNNING")

    async def ResolveSecrets(self, request: Any, context: Any) -> Any:
        await self._enter("ResolveSecrets", request, context)
        if self.resolve_delay > 0:
            await asyncio.sleep(self.resolve_delay)
        return pb.ResolveSecretsResponse(secrets=self.secrets)


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


async def until(condition: Callable[[], bool], what: str, seconds: float = 5.0) -> None:
    """Waits for ``condition``; the test fails, saying what it waited for, if it does not come."""
    end = asyncio.get_running_loop().time() + seconds
    while not condition():
        if asyncio.get_running_loop().time() > end:
            pytest.fail(f"timed out waiting for {what}")
        await asyncio.sleep(0.01)
