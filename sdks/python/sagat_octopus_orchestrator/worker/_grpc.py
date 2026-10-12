"""The typed, authenticated, error-mapped worker RPCs over ``grpc.aio`` (``sdks/SPEC.md`` sections 2 and 6)."""

from __future__ import annotations

from collections.abc import AsyncIterable, AsyncIterator, Sequence
from dataclasses import dataclass, field
from typing import Any, cast
from urllib.parse import urlsplit

import grpc
import grpc.aio

from .._proto import worker_pb2 as pb
from .._proto import worker_pb2_grpc as pb_grpc
from ..config import Secret, TlsOptions, TokenSource, resolve_token
from ..errors import ErrorCode, SagatError
from ..retry import capped_ms, parse_ms
from ..traceparent import is_traceparent

METHODS = (
    "RegisterWorker", "PollTasks", "Heartbeat", "CompleteStep", "FailStep", "ResolveSecrets", "WriteBlob", "ReadBlob"
)


@dataclass(frozen=True)
class GrpcConfig:
    grpc_url: str
    token: TokenSource | Secret = field(repr=False)
    tls: TlsOptions | None = None
    timeout: float = 30.0
    interceptors: Sequence[grpc.aio.ClientInterceptor] = ()


def _metadata_value(error: grpc.aio.AioRpcError, key: str) -> str | None:
    for source in (error.trailing_metadata(), error.initial_metadata()):
        if source is None:
            continue
        for name, value in source:
            if name == key:
                return value if isinstance(value, str) else value.decode("utf-8", "replace")
    return None


def error_of_status(method: str, error: grpc.aio.AioRpcError) -> SagatError:
    """The mapping of a gRPC failure onto the canonical errors (``sdks/SPEC.md`` section 2).

    The ``x-sagat-error`` trailer wins when present; the call's own words (``details``) are never copied
    into the message.
    """
    reason = _metadata_value(error, "x-sagat-error")
    retry_after = _metadata_value(error, "retry-after-ms")
    retry_after_ms = capped_ms(parse_ms(retry_after))
    status = error.code()
    name = status.name

    def safe(code: ErrorCode, message: str) -> SagatError:
        return SagatError(code, message, status=name, reason=reason, retry_after_ms=retry_after_ms)

    if status == grpc.StatusCode.UNAUTHENTICATED:
        return safe("UNAUTHENTICATED", "the worker service refused the credentials or the session")
    if status == grpc.StatusCode.PERMISSION_DENIED:
        if reason == "NOT_BOUND":
            return safe("NOT_BOUND", "the identity is not bound to the namespace and activity")
        if reason == "SECRET_NOT_USABLE":
            return safe("SECRET_NOT_USABLE", "a secret of the step cannot be used")
        return safe("FORBIDDEN", "the identity may not use the worker service")
    if status == grpc.StatusCode.INVALID_ARGUMENT:
        details = (error.details() or "").lower()
        if reason == "UNSUPPORTED_PROTOCOL" or (method == "RegisterWorker" and "unsupported protocol" in details):
            return safe("UNSUPPORTED_PROTOCOL", "the server does not speak this protocol version")
        return safe("VALIDATION_FAILED", "the worker service refused a field")
    if status == grpc.StatusCode.ABORTED:
        if reason == "IDEMPOTENCY_KEY_REUSED":
            return safe("IDEMPOTENCY_KEY_REUSED", "the idempotency key was used for a different report")
        if reason is None or reason == "STALE_ATTEMPT":
            return safe("STALE_ATTEMPT", "the lease is no longer current")
        return safe("CONFLICT", "the request conflicts with the state of the run")
    if status == grpc.StatusCode.FAILED_PRECONDITION:
        return safe("STALE_ATTEMPT", "the lease is no longer current")
    if status == grpc.StatusCode.NOT_FOUND:
        return safe("NOT_FOUND", "the worker service does not know it")
    if status == grpc.StatusCode.RESOURCE_EXHAUSTED:
        return safe(
            "QUOTA_EXCEEDED" if reason == "QUOTA_EXCEEDED" else "RATE_LIMITED",
            "the worker service asks for a slower pace",
        )
    if status in (grpc.StatusCode.UNAVAILABLE, grpc.StatusCode.DEADLINE_EXCEEDED):
        # The server's own UNAVAILABLE (its storage) is told from the lack of an answer by its trailer.
        if reason == "UNAVAILABLE":
            return safe("UNAVAILABLE", "the worker service could not serve the call now")
        return safe("TRANSPORT", "no answer from the worker service")
    if status == grpc.StatusCode.CANCELLED:
        return SagatError("CANCELLED", "the call was cancelled", status=name)
    if status == grpc.StatusCode.DATA_LOSS:
        return safe("PROTOCOL", "the blob stream failed its integrity check")
    return safe("INTERNAL", "the worker service failed")


class WorkerRpc:
    """The six calls of ``WorkerService``. Created inside the running event loop (``grpc.aio`` binds to it)."""

    def __init__(self, config: GrpcConfig) -> None:
        self._config = config
        self._closed = False
        url = urlsplit(config.grpc_url)
        if url.scheme not in ("http", "https") or not url.hostname or not url.port:
            raise SagatError("VALIDATION_FAILED", "grpc_url is http(s)://host:port")
        target = f"{url.hostname}:{url.port}" if ":" not in url.hostname else f"[{url.hostname}]:{url.port}"
        options: list[tuple[str, Any]] = [
            ("grpc.keepalive_time_ms", 120_000),
            ("grpc.max_receive_message_length", 16 * 1024 * 1024),
        ]
        interceptors = list(config.interceptors) or None
        if url.scheme == "https":
            tls = config.tls or TlsOptions()
            if tls.server_name:
                # The name the certificate is verified against instead of the URL's host: verified, not skipped.
                options.append(("grpc.ssl_target_name_override", tls.server_name))
            credentials = grpc.ssl_channel_credentials(
                root_certificates=TlsOptions.as_bytes(tls.ca),
                private_key=tls.key_bytes(),
                certificate_chain=TlsOptions.as_bytes(tls.cert),
            )
            self._channel = grpc.aio.secure_channel(target, credentials, options=options, interceptors=interceptors)
        else:
            self._channel = grpc.aio.insecure_channel(target, options=options, interceptors=interceptors)
        self._stub: Any = cast(Any, pb_grpc).WorkerServiceStub(self._channel)

    async def close(self) -> None:
        self._closed = True
        await self._channel.close(grace=None)

    async def _call(self, method: str, request: Any, traceparent: str | None = None) -> Any:
        if self._closed:
            raise SagatError("CANCELLED", "the connection to the worker service is closed")
        token = await resolve_token(self._config.token)
        metadata = [("authorization", f"Bearer {token}")]
        if traceparent and is_traceparent(traceparent):
            # The lease's own trace context goes back as it came; one that is not valid is left out rather than
            # allowed to fail a report.
            metadata.append(("traceparent", traceparent))
        try:
            return await getattr(self._stub, method)(request, metadata=metadata, timeout=self._config.timeout)
        except grpc.aio.AioRpcError as error:
            raise error_of_status(method, error) from None
        except grpc.aio.BaseError:
            raise SagatError("CANCELLED", "the connection to the worker service is closed") from None

    async def _metadata(self, traceparent: str | None) -> list[tuple[str, str]]:
        if self._closed:
            raise SagatError("CANCELLED", "the connection to the worker service is closed")
        token = await resolve_token(self._config.token)
        metadata = [("authorization", f"Bearer {token}")]
        if traceparent and is_traceparent(traceparent):
            metadata.append(("traceparent", traceparent))
        return metadata

    async def register_worker(self, request: pb.RegisterWorkerRequest) -> pb.RegisterWorkerResponse:
        response: pb.RegisterWorkerResponse = await self._call("RegisterWorker", request)
        return response

    async def poll_tasks(self, request: pb.PollTasksRequest) -> pb.PollTasksResponse:
        response: pb.PollTasksResponse = await self._call("PollTasks", request)
        return response

    async def heartbeat(self, request: pb.HeartbeatRequest, traceparent: str | None = None) -> pb.HeartbeatResponse:
        response: pb.HeartbeatResponse = await self._call("Heartbeat", request, traceparent)
        return response

    async def complete_step(
        self, request: pb.CompleteStepRequest, traceparent: str | None = None
    ) -> pb.StepCommandResponse:
        response: pb.StepCommandResponse = await self._call("CompleteStep", request, traceparent)
        return response

    async def fail_step(self, request: pb.FailStepRequest, traceparent: str | None = None) -> pb.StepCommandResponse:
        response: pb.StepCommandResponse = await self._call("FailStep", request, traceparent)
        return response

    async def resolve_secrets(
        self, request: pb.ResolveSecretsRequest, traceparent: str | None = None
    ) -> pb.ResolveSecretsResponse:
        response: pb.ResolveSecretsResponse = await self._call("ResolveSecrets", request, traceparent)
        return response

    async def write_blob(
        self, requests: AsyncIterable[pb.WriteBlobRequest], traceparent: str | None = None
    ) -> pb.WriteBlobResponse:
        try:
            response: pb.WriteBlobResponse = await self._stub.WriteBlob(
                requests, metadata=await self._metadata(traceparent), timeout=self._config.timeout
            )
            return response
        except grpc.aio.AioRpcError as error:
            raise error_of_status("WriteBlob", error) from None
        except grpc.aio.BaseError:
            raise SagatError("CANCELLED", "the connection to the worker service is closed") from None

    async def read_blob(
        self, request: pb.ReadBlobRequest, traceparent: str | None = None
    ) -> AsyncIterator[pb.ReadBlobResponse]:
        call = self._stub.ReadBlob(
            request, metadata=await self._metadata(traceparent), timeout=self._config.timeout
        )
        try:
            async for response in call:
                yield response
        except grpc.aio.AioRpcError as error:
            raise error_of_status("ReadBlob", error) from None
        except grpc.aio.BaseError:
            raise SagatError("CANCELLED", "the connection to the worker service is closed") from None
        finally:
            call.cancel()
