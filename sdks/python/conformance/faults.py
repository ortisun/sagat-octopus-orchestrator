"""The faults of a scenario (tests/contract/sdk-conformance/README.md, "Faults") at the SDK's transport hooks:
an ``httpx`` transport for REST and a ``grpc.aio`` client interceptor for gRPC, both fed by ONE plan."""

from __future__ import annotations

import json
from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from typing import Any

import grpc
import grpc.aio
import httpx


@dataclass(frozen=True)
class Fault:
    channel: str
    match: str
    action: str
    times: int | None = None
    after: int = 0
    status: int | None = None
    code: str | None = None
    reason: str | None = None
    retry_after_ms: int | None = None
    body: Any = None

    @staticmethod
    def of(raw: dict[str, Any]) -> Fault:
        return Fault(
            channel=raw["channel"],
            match=raw["match"],
            action=raw["action"],
            times=raw.get("times"),
            after=raw.get("after", 0),
            status=raw.get("status"),
            code=raw.get("code"),
            reason=raw.get("reason"),
            retry_after_ms=raw.get("retryAfterMs"),
            body=raw.get("body"),
        )


class FaultPlan:
    """One plan for every client and worker host of the run.

    A call is counted against EVERY fault that matches it, and the first fault in file order that is due
    answers it; ``applied()`` says, per fault and in file order, how many calls it answered.
    """

    def __init__(self, faults: list[Fault]) -> None:
        self._faults = faults
        self._seen = [0] * len(faults)
        self._answered = [0] * len(faults)

    def applied(self) -> list[int]:
        return list(self._answered)

    def has_grpc(self) -> bool:
        return any(fault.channel == "grpc" for fault in self._faults)

    def next(self, channel: str, key: str) -> Fault | None:
        due: int | None = None
        for index, fault in enumerate(self._faults):
            if fault.channel != channel or not key.startswith(fault.match):
                continue
            count = self._seen[index]
            self._seen[index] = count + 1
            times = float("inf") if fault.times is None else fault.times
            if due is None and fault.after <= count < fault.after + times:
                due = index
        if due is None:
            return None
        self._answered[due] += 1
        return self._faults[due]

    def transport(self, api_path: str, inner: httpx.AsyncBaseTransport | None = None) -> httpx.AsyncBaseTransport:
        return FaultyTransport(self, api_path, inner or httpx.AsyncHTTPTransport())

    def interceptors(self) -> list[grpc.aio.ClientInterceptor]:
        return [FaultyChannel(self), FaultyStreamChannel(self)] if self.has_grpc() else []


class FaultyTransport(httpx.AsyncBaseTransport):
    """What a fault says happens to a REST call; otherwise the real thing."""

    def __init__(self, plan: FaultPlan, api_path: str, inner: httpx.AsyncBaseTransport) -> None:
        self._plan = plan
        self._api_path = api_path
        self._inner = inner

    async def handle_async_request(self, request: httpx.Request) -> httpx.Response:
        path = request.url.path.removeprefix(self._api_path) or "/"
        fault = self._plan.next("http", f"{request.method.upper()} {path}")
        if fault is None:
            return await self._inner.handle_async_request(request)
        if fault.action == "failBefore":
            raise httpx.ConnectError("connection refused", request=request)
        if fault.action == "status":
            headers = {"content-type": "application/json"}
            if fault.retry_after_ms is not None:
                headers["retry-after"] = str(-(-fault.retry_after_ms // 1000))
            return httpx.Response(
                fault.status or 503,
                headers=headers,
                content=json.dumps(fault.body if fault.body is not None else {}).encode(),
            )
        # dropResponse: the call is made, the server applies it, the answer is lost.
        response = await self._inner.handle_async_request(request)
        await response.aread()
        await response.aclose()
        raise httpx.ReadError("connection reset", request=request)

    async def aclose(self) -> None:
        await self._inner.aclose()


class FaultyChannel(grpc.aio.UnaryUnaryClientInterceptor):
    """The interceptor of the worker's ``grpc.aio`` channel."""

    def __init__(self, plan: FaultPlan) -> None:
        self._plan = plan

    @staticmethod
    def _error(
        code: grpc.StatusCode, reason: str | None = None, retry_after_ms: int | None = None
    ) -> grpc.aio.AioRpcError:
        trailers = []
        if reason:
            trailers.append(("x-sagat-error", reason))
        if retry_after_ms is not None:
            trailers.append(("retry-after-ms", str(retry_after_ms)))
        return grpc.aio.AioRpcError(
            code, grpc.aio.Metadata(), grpc.aio.Metadata(*trailers), details="injected", debug_error_string="injected"
        )

    async def intercept_unary_unary(
        self,
        continuation: Callable[[grpc.aio.ClientCallDetails, Any], Awaitable[Any]],
        client_call_details: grpc.aio.ClientCallDetails,
        request: Any,
    ) -> Any:
        method = client_call_details.method
        name = (method.decode() if isinstance(method, bytes) else method).rsplit("/", 1)[-1]
        fault = self._plan.next("grpc", name)
        if fault is None:
            return await continuation(client_call_details, request)
        if fault.action == "failBefore":
            raise self._error(grpc.StatusCode.UNAVAILABLE)
        if fault.action == "status":
            code = next(
                (c for c in grpc.StatusCode if c.name == (fault.code or "UNAVAILABLE")), grpc.StatusCode.UNAVAILABLE
            )
            raise self._error(code, fault.reason, fault.retry_after_ms)
        # dropResponse: the server applies the call; its answer is replaced by a failure to deliver it.
        call = await continuation(client_call_details, request)
        await call
        raise self._error(grpc.StatusCode.UNAVAILABLE)


class FaultyStreamChannel(grpc.aio.UnaryStreamClientInterceptor):
    """Streaming counterpart kept separate because grpc.aio classifies one interceptor by one RPC shape."""

    def __init__(self, plan: FaultPlan) -> None:
        self._plan = plan

    async def intercept_unary_stream(
        self,
        continuation: Callable[[grpc.aio.ClientCallDetails, Any], Awaitable[Any]],
        client_call_details: grpc.aio.ClientCallDetails,
        request: Any,
    ) -> Any:
        method = client_call_details.method
        name = (method.decode() if isinstance(method, bytes) else method).rsplit("/", 1)[-1]
        fault = self._plan.next("grpc", name)
        if fault is None:
            return await continuation(client_call_details, request)

        if fault.action in ("failBefore", "status"):
            code = grpc.StatusCode.UNAVAILABLE
            if fault.action == "status":
                code = next(
                    (item for item in grpc.StatusCode if item.name == (fault.code or "UNAVAILABLE")),
                    grpc.StatusCode.UNAVAILABLE,
                )
            error = FaultyChannel._error(code, fault.reason, fault.retry_after_ms)

            async def rejected() -> Any:
                raise error
                yield  # pragma: no cover - makes this the response async iterator

            return rejected()

        call = await continuation(client_call_details, request)
        if fault.action == "dropStreamAfterFirst":

            async def drop_after_first() -> Any:
                async for response in call:
                    yield response
                    if len(response.tasks) > 0:
                        call.cancel()
                        raise FaultyChannel._error(grpc.StatusCode.UNAVAILABLE)

            return drop_after_first()

        async def drop_response() -> Any:
            async for _response in call:
                pass
            raise FaultyChannel._error(grpc.StatusCode.UNAVAILABLE)
            yield  # pragma: no cover

        return drop_response()
